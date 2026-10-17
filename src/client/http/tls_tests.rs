//! Local TLS only. Neither the generated CA nor the reset synchronization is
//! available in production builds; certificate and hostname checks stay enabled.
use super::{tests::accept, *};
use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, IsCa, KeyPair, KeyUsagePurpose};
use rustls::{ServerConfig, ServerConnection, StreamOwned, pki_types::PrivatePkcs8KeyDer};
use std::{
    io::{BufRead, BufReader},
    net::TcpListener,
    sync::{Mutex, atomic::AtomicUsize, mpsc},
    thread,
};

type TlsStream = StreamOwned<ServerConnection, TcpStream>;
const SECOND_BODY: &str = "{\"request\":2}";

fn fixture() -> (Transport, TcpListener, Arc<ServerConfig>) {
    let mut params = CertificateParams::new(Vec::new()).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let ca = CertifiedIssuer::self_signed(params, KeyPair::generate().unwrap()).unwrap();
    let key = KeyPair::generate().unwrap();
    let leaf = CertificateParams::new(vec!["localhost".into()])
        .unwrap()
        .signed_by(&key, &ca)
        .unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut server = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.der().clone()],
            PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
        .unwrap();
    // Late TLS session tickets would correctly make the idle socket peek reject
    // readable bytes. Disable tickets so this test specifically exercises reuse.
    server.send_tls13_tickets = 0;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let root = ureq::tls::Certificate::from_der(ca.der().as_ref()).to_owned();
    let config = Agent::config_builder()
        .proxy(None)
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_connect(Some(CONNECT_BUDGET))
        .max_idle_age(POOL_IDLE)
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(vec![root].into())
                .build(),
        )
        .build();
    let transport = Transport {
        agent: Agent::with_parts(config, PollConnector::default(), PollResolver::default()),
        base: format!(
            "https://localhost:{}",
            listener.local_addr().unwrap().port()
        ),
    };
    (transport, listener, Arc::new(server))
}

fn tls_accept(listener: &TcpListener, config: &Arc<ServerConfig>) -> BufReader<TlsStream> {
    BufReader::new(StreamOwned::new(
        ServerConnection::new(config.clone()).unwrap(),
        accept(listener),
    ))
}

fn read_post(stream: &mut BufReader<TlsStream>) -> Vec<u8> {
    let mut line = String::new();
    stream.read_line(&mut line).unwrap();
    assert_eq!(line, "POST /api/test HTTP/1.1\r\n");
    assert_eq!(stream.get_ref().conn.server_name(), Some("localhost"));
    let mut length = None;
    loop {
        line.clear();
        assert!(stream.read_line(&mut line).unwrap() > 0);
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = Some(value.trim().parse().unwrap());
        }
    }
    let mut body = vec![0; length.unwrap()];
    stream.read_exact(&mut body).unwrap();
    body
}

fn respond(stream: &mut BufReader<TlsStream>) {
    stream
        .get_mut()
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
        .unwrap();
    stream.get_mut().flush().unwrap();
}

fn reset(stream: BufReader<TlsStream>) {
    socket2::SockRef::from(&stream.get_ref().sock)
        .set_linger(Some(Duration::ZERO))
        .unwrap();
    drop(stream);
}

#[derive(Debug)]
struct ResetControl {
    armed: AtomicBool,
    reset: mpsc::Sender<()>,
    acknowledged: Mutex<mpsc::Receiver<()>>,
    connections: AtomicUsize,
    swallowed: AtomicUsize,
}

// This connector preserves the actual production PollTransport ->
// RustlsConnector/RustlsTransport -> RequestTransport stack. The middle hook
// only synchronizes a real server reset after the idle peek, before the write.
#[derive(Debug, Default)]
struct ResetConnector {
    tls: RustlsConnector,
    control: Option<Arc<ResetControl>>,
}
impl Connector for ResetConnector {
    type Out = Box<dyn UreqTransport>;
    fn connect(
        &self,
        details: &ConnectionDetails,
        _: Option<()>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        let control = self.control.as_ref().unwrap().clone();
        control.connections.fetch_add(1, Ordering::Relaxed);
        let inner = PollTcpConnector.connect(details, None::<()>)?.unwrap();
        let socket = ACTIVE.with(|slot| {
            slot.borrow()
                .as_ref()
                .unwrap()
                .socket
                .as_ref()
                .unwrap()
                .clone()
        });
        let tcp = ResetBeforeWrite {
            inner,
            socket,
            control: control.clone(),
        };
        let tls = self.tls.connect(details, Some(tcp))?.unwrap();
        Ok(Some(Box::new(RequestTransport {
            inner: Box::new(ObserveTls {
                inner: Box::new(tls),
                control,
            }),
            last_attempt: 0,
            header_match: 0,
            headers_done: false,
        })))
    }
}

#[derive(Debug)]
struct ResetBeforeWrite {
    inner: Box<dyn UreqTransport>,
    socket: Arc<Socket>,
    control: Arc<ResetControl>,
}
impl UreqTransport for ResetBeforeWrite {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }
    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        if self.control.armed.swap(false, Ordering::Relaxed) {
            self.control.reset.send(()).unwrap();
            self.control
                .acknowledged
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            // Wait for the real RST to reach the client kernel. No fake I/O
            // error is returned: PollTransport performs the failed write.
            loop {
                match self.socket.stream.peek(&mut [0]) {
                    Err(error) if retryable(&error) => assert!(Instant::now() < deadline),
                    Err(error) => {
                        assert_eq!(error.kind(), io::ErrorKind::ConnectionReset);
                        break;
                    }
                    Ok(0) => break,
                    result => panic!("expected reset, got {result:?}"),
                }
            }
        }
        self.inner.transmit_output(amount, timeout)
    }
    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        self.inner.await_input(timeout)
    }
    fn is_open(&mut self) -> bool {
        self.inner.is_open()
    }
}

#[derive(Debug)]
struct ObserveTls {
    inner: Box<dyn UreqTransport>,
    control: Arc<ResetControl>,
}
impl UreqTransport for ObserveTls {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }
    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        let result = self.inner.transmit_output(amount, timeout);
        if result.is_ok()
            && ACTIVE.with(|slot| slot.borrow().as_ref().unwrap().write_failure.is_some())
        {
            self.control.swallowed.fetch_add(1, Ordering::Relaxed);
        }
        result
    }
    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        self.inner.await_input(timeout)
    }
    fn is_open(&mut self) -> bool {
        self.inner.is_open()
    }
    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}

#[test]
fn tls_reused_header_write_failure_retries_once_and_delivers_body_once() {
    let (mut transport, listener, config) = fixture();
    let (reset_now, wait_reset) = mpsc::channel();
    let (acknowledge, acknowledged) = mpsc::channel();
    let (done, wait_done) = mpsc::channel();
    let control = Arc::new(ResetControl {
        armed: AtomicBool::new(false),
        reset: reset_now,
        acknowledged: Mutex::new(acknowledged),
        connections: AtomicUsize::new(0),
        swallowed: AtomicUsize::new(0),
    });
    transport.agent = Agent::with_parts(
        transport.agent.config().clone(),
        ResetConnector {
            tls: RustlsConnector::default(),
            control: Some(control.clone()),
        },
        PollResolver::default(),
    );
    let server = thread::spawn(move || {
        let mut accepts = 0;
        let mut deliveries = 0;
        let mut first = tls_accept(&listener, &config);
        accepts += 1;
        assert_eq!(read_post(&mut first), b"{}");
        respond(&mut first);
        wait_reset.recv_timeout(Duration::from_secs(3)).unwrap();
        reset(first);
        acknowledge.send(()).unwrap();
        let mut fresh = tls_accept(&listener, &config);
        accepts += 1;
        assert_eq!(read_post(&mut fresh), SECOND_BODY.as_bytes());
        deliveries += 1;
        respond(&mut fresh);
        wait_done.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        (accepts, deliveries)
    });
    transport.request("/api/test", "{}", &[], 3).unwrap();
    control.armed.store(true, Ordering::Relaxed);
    let result = transport.request("/api/test", SECOND_BODY, &[], 3);
    done.send(()).unwrap();
    assert_eq!(server.join().unwrap(), (2, 1));
    assert_eq!(result.unwrap(), (200, b"{}".to_vec()));
    assert_eq!(control.connections.load(Ordering::Relaxed), 2);
    assert_eq!(
        control.swallowed.load(Ordering::Relaxed),
        1,
        "must exercise rustls swallowing a real socket write error"
    );
}

#[test]
fn tls_failure_after_full_send_is_not_retried() {
    let (transport, listener, config) = fixture();
    let (done, wait_done) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut accepts = 0;
        let mut deliveries = 0;
        let mut stream = tls_accept(&listener, &config);
        accepts += 1;
        assert_eq!(read_post(&mut stream), b"{}");
        respond(&mut stream);
        assert_eq!(read_post(&mut stream), SECOND_BODY.as_bytes());
        deliveries += 1;
        reset(stream);
        wait_done.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        (accepts, deliveries)
    });
    transport.request("/api/test", "{}", &[], 3).unwrap();
    let result = transport.request("/api/test", SECOND_BODY, &[], 3);
    done.send(()).unwrap();
    assert_eq!(server.join().unwrap(), (1, 1));
    assert_eq!(
        result.unwrap_err().to_string(),
        "HTTPS transport failed; response not logged"
    );
}

#[test]
fn streaming_get_uses_verified_tls_directly_and_through_connect() {
    for (proxied, wrong_host) in [(false, false), (true, false), (true, true)] {
        let (mut transport, listener, config) = fixture();
        let address = listener.local_addr().unwrap();
        if wrong_host {
            transport.base = format!("https://127.0.0.1:{}", address.port());
        }
        if proxied {
            let proxy =
                ureq::Proxy::new(&format!("http://fake-user:fake-password@{address}")).unwrap();
            let agent_config = Agent::config_builder()
                .proxy(Some(proxy))
                .max_redirects(0)
                .http_status_as_error(false)
                .timeout_connect(Some(CONNECT_BUDGET))
                .tls_config(transport.agent.config().tls_config().clone())
                .build();
            transport.agent = Agent::with_parts(
                agent_config,
                PollConnector::default(),
                PollResolver::default(),
            );
        }
        let base = transport.base.clone();
        let server = thread::spawn(move || {
            let socket = accept(&listener);
            let socket = if proxied {
                let mut reader = BufReader::new(socket);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert_eq!(
                    line,
                    format!(
                        "CONNECT {} HTTP/1.1\r\n",
                        base.trim_start_matches("https://")
                    )
                );
                loop {
                    line.clear();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    assert!(!line.contains("origin-token"));
                    if line == "\r\n" {
                        break;
                    }
                }
                reader
                    .get_mut()
                    .write_all(b"HTTP/1.1 200 OK\r\n\r\n")
                    .unwrap();
                reader.into_inner()
            } else {
                socket
            };
            let mut reader = BufReader::new(StreamOwned::new(
                ServerConnection::new(config).unwrap(),
                socket,
            ));
            let mut line = String::new();
            if wrong_host {
                assert!(reader.read_line(&mut line).is_err());
                return;
            }
            reader.read_line(&mut line).unwrap();
            assert_eq!(line, "GET /api/test HTTP/1.1\r\n");
            assert_eq!(reader.get_ref().conn.server_name(), Some("localhost"));
            let mut cookie = false;
            loop {
                line.clear();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                assert!(
                    !line
                        .to_ascii_lowercase()
                        .starts_with("proxy-authorization:")
                );
                cookie |= line.trim().eq_ignore_ascii_case("cookie: lct=origin-token");
                if line == "\r\n" {
                    break;
                }
            }
            assert!(cookie);
            reader.get_mut().write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 9\r\n\r\ndata: 1\n\n").unwrap();
            reader.get_mut().flush().unwrap();
        });
        let result = transport.stream(
            "/api/test",
            &[("Cookie", "lct=origin-token")],
            Arc::new(AtomicBool::new(false)),
            Duration::from_secs(2),
            |status, headers, body| {
                assert_eq!(status, 200);
                assert_eq!(headers["content-type"], "text/event-stream");
                let mut bytes = Vec::new();
                body.read_to_end(&mut bytes)?;
                assert_eq!(bytes, b"data: 1\n\n");
                Ok(())
            },
        );
        if wrong_host {
            assert_eq!(
                format!("{:#}", result.unwrap_err()),
                "HTTPS transport failed; response not logged"
            );
        } else {
            result.unwrap();
        }
        server.join().unwrap();
    }
}

#[test]
fn buffered_http_and_tls_bytes_survive_a_spent_watchdog_budget() {
    let (mut transport, listener, config) = fixture();
    let agent_config = Agent::config_builder()
        .proxy(None)
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_connect(Some(CONNECT_BUDGET))
        .input_buffer_size(1024)
        .tls_config(transport.agent.config().tls_config().clone())
        .build();
    transport.agent = Agent::with_parts(
        agent_config,
        PollConnector::default(),
        PollResolver::default(),
    );
    let payload = format!(":{}\nevent: ping\n\n", "x".repeat(2048));
    let expected = payload.clone();
    let (ready, wait) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut socket = tls_accept(&listener, &config);
        let mut line = String::new();
        socket.read_line(&mut line).unwrap();
        assert_eq!(line, "GET /api/test HTTP/1.1\r\n");
        loop {
            line.clear();
            assert!(socket.read_line(&mut line).unwrap() > 0);
            if line == "\r\n" {
                break;
            }
        }
        socket
            .get_mut()
            .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
            .unwrap();
        socket.get_mut().flush().unwrap();
        wait.recv_timeout(Duration::from_secs(2)).unwrap();
        socket.get_mut().write_all(payload.as_bytes()).unwrap();
        socket.get_mut().flush().unwrap();
        assert_eq!(socket.get_mut().sock.read(&mut [0]).unwrap(), 0);
    });
    transport
        .stream(
            "/api/test",
            &[],
            Arc::new(AtomicBool::new(false)),
            Duration::from_secs(2),
            |status, _, body| {
                assert_eq!(status, 200);
                ready.send(()).unwrap();
                let mut bytes = vec![0; expected.len()];
                body.read_exact(&mut bytes[..1])?;
                // One TLS record exceeds the HTTP input buffer; exhaust only the
                // budget, leaving bytes buffered in both HTTP and rustls.
                ACTIVE.with(|slot| {
                    let mut slot = slot.borrow_mut();
                    let watchdog = slot.as_mut().unwrap().watchdog.as_mut().unwrap();
                    watchdog.remaining = Duration::ZERO;
                    watchdog.timeout = Duration::from_millis(100);
                });
                body.read_exact(&mut bytes[1..])?;
                assert_eq!(bytes, expected.as_bytes());
                body.ping();
                let error = body.read(&mut [0]).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::TimedOut);
                assert_eq!(error.to_string(), "SSE ping expired");
                Ok(())
            },
        )
        .unwrap();
    server.join().unwrap();
}
