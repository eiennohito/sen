use super::{
    tests::{accept, local_server, read_request},
    *,
};
use std::{
    io::{BufRead, BufReader},
    net::{SocketAddr, TcpListener},
    sync::{Mutex, atomic::AtomicUsize, mpsc},
    thread,
};
use ureq::unversioned::transport::time::Duration as UreqDuration;

fn stalled_resolver() -> (
    PollResolver,
    mpsc::Receiver<()>,
    mpsc::Sender<()>,
    mpsc::Receiver<()>,
    Arc<AtomicUsize>,
) {
    let (ready, started) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let (done, finished) = mpsc::channel();
    let wait = Mutex::new(wait);
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let resolver = PollResolver::new(Arc::new(move |_| {
        count.fetch_add(1, Ordering::Relaxed);
        ready.send(()).unwrap();
        wait.lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(15))
            .unwrap();
        done.send(()).unwrap();
        Ok(vec!["127.0.0.1:1".parse().unwrap()])
    }));
    (resolver, started, release, finished, calls)
}

#[test]
fn dns_cancellation_is_prompt_and_helpers_are_bounded() {
    let (resolver, started, release, finished, calls) = stalled_resolver();
    let mut transport = Transport::with_parts(None, resolver);
    transport.base = "http://stall.test".into();
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let request = thread::spawn(move || {
        super::super::set_request_cancel(cancel);
        let result = transport.request("/api/test", "{}", &[], 30);
        (transport, result)
    });
    started.recv_timeout(Duration::from_secs(2)).unwrap();
    let now = Instant::now();
    signal.store(true, Ordering::Relaxed);
    let (transport, result) = request.join().unwrap();
    let elapsed = now.elapsed();
    // The OS lookup remains stalled. A subsequent request must wait on that
    // helper and hit its overall deadline, rather than launch another helper.
    let again = transport.request("/api/test", "{}", &[], 1);
    release.send(()).unwrap();
    finished.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(result.unwrap_err().to_string(), "HTTPS request cancelled");
    assert!(elapsed < Duration::from_millis(200), "{elapsed:?}");
    assert_eq!(
        again.unwrap_err().to_string(),
        "HTTPS request timed out; response not logged"
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn dns_lookup_obeys_ten_second_connect_budget() {
    let (resolver, _started, release, finished, _calls) = stalled_resolver();
    let _scope = RequestScope::new(Duration::from_secs(30));
    let config = Agent::config_builder().proxy(None).build();
    let now = Instant::now();
    let result = resolver.resolve(
        &BASE.parse().unwrap(),
        &config,
        NextTimeout {
            after: UreqDuration::from_secs(30),
            reason: ureq::Timeout::Global,
        },
    );
    let elapsed = now.elapsed();
    release.send(()).unwrap();
    finished.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(matches!(
        result,
        Err(ureq::Error::Timeout(ureq::Timeout::Connect))
    ));
    assert!(
        elapsed >= CONNECT_BUDGET - Duration::from_millis(100)
            && elapsed < CONNECT_BUDGET + Duration::from_secs(2),
        "{elapsed:?}"
    );
}

#[test]
fn dns_cache_skips_lookup_for_pooled_requests() {
    let (mut transport, server) = local_server(|listener| {
        let mut stream = BufReader::new(accept(&listener));
        for _ in 0..2 {
            assert!(
                read_request(&mut stream),
                "cached DNS must preserve connection reuse"
            );
            stream
                .get_mut()
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                .unwrap();
        }
    });
    let address: SocketAddr = transport
        .base
        .strip_prefix("http://")
        .unwrap()
        .parse()
        .unwrap();
    transport.base = format!("http://cached.test:{}", address.port());
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let resolver = PollResolver::new(Arc::new(move |_| {
        count.fetch_add(1, Ordering::Relaxed);
        Ok(vec![address])
    }));
    transport.agent = Agent::with_parts(
        transport.agent.config().clone(),
        PollConnector::default(),
        resolver,
    );
    for _ in 0..2 {
        transport.request("/api/test", "{}", &[], 2).unwrap();
    }
    server.join().unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn dns_and_tls_share_one_connect_budget() {
    let (release, wait) = mpsc::channel();
    let (mut transport, server) = local_server(move |listener| {
        let mut stream = accept(&listener);
        let mut hello = [0];
        stream.read_exact(&mut hello).unwrap();
        assert_eq!(hello[0], 22);
        wait.recv_timeout(Duration::from_secs(3)).unwrap();
    });
    let address: SocketAddr = transport
        .base
        .strip_prefix("http://")
        .unwrap()
        .parse()
        .unwrap();
    transport.base = format!("https://slow.test:{}", address.port());
    let resolver = PollResolver::new(Arc::new(move |_| {
        thread::sleep(Duration::from_millis(100));
        Ok(vec![address])
    }));
    transport.agent = Agent::with_parts(
        transport.agent.config().clone(),
        PollConnector::default(),
        resolver,
    );
    let _scope = RequestScope::new(Duration::from_secs(2));
    // Accelerate the shared budget for this DNS+TLS integration check; the
    // separate stalled-DNS test exercises the real production ten seconds.
    ACTIVE.with(|slot| {
        slot.borrow_mut().as_mut().unwrap().connect_deadline =
            Instant::now() + Duration::from_millis(250)
    });
    let now = Instant::now();
    let result = transport.agent.post(&transport.base).send("{}");
    let elapsed = now.elapsed();
    release.send(()).unwrap();
    server.join().unwrap();
    assert!(result.is_err());
    assert!(matches!(
        check_request(),
        Err(ureq::Error::Timeout(ureq::Timeout::Connect))
    ));
    assert!(
        elapsed >= Duration::from_millis(240) && elapsed < Duration::from_millis(750),
        "{elapsed:?}"
    );
}

#[derive(Debug)]
struct CaptureConnector {
    inner: PollConnector,
    sockets: Arc<Mutex<Vec<Arc<Socket>>>>,
}

impl Connector for CaptureConnector {
    type Out = Box<dyn UreqTransport>;
    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<()>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        let result = self.inner.connect(details, chained)?;
        ACTIVE.with(|slot| {
            self.sockets.lock().unwrap().push(
                slot.borrow()
                    .as_ref()
                    .unwrap()
                    .socket
                    .as_ref()
                    .unwrap()
                    .clone(),
            )
        });
        Ok(result)
    }
}

#[test]
fn replaces_idle_connection_closed_by_server() {
    let (close, wait) = mpsc::channel();
    let (closed, observed) = mpsc::channel();
    let (mut transport, server) = local_server(move |listener| {
        let mut stream = BufReader::new(accept(&listener));
        assert!(read_request(&mut stream));
        stream
            .get_mut()
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
            .unwrap();
        wait.recv_timeout(Duration::from_secs(2)).unwrap();
        stream.get_ref().shutdown(Shutdown::Both).unwrap();
        drop(stream);
        closed.send(()).unwrap();
        let mut fresh = BufReader::new(accept(&listener));
        assert!(read_request(&mut fresh));
        fresh
            .get_mut()
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
            .unwrap();
    });
    let sockets = Arc::new(Mutex::new(Vec::new()));
    transport.agent = Agent::with_parts(
        transport.agent.config().clone(),
        CaptureConnector {
            inner: PollConnector::default(),
            sockets: sockets.clone(),
        },
        PollResolver::default(),
    );
    transport.request("/api/test", "{}", &[], 2).unwrap();
    close.send(()).unwrap();
    observed.recv_timeout(Duration::from_secs(2)).unwrap();
    // Wait until FIN is visible, without consuming bytes or relying on a sleep.
    let socket = sockets.lock().unwrap()[0].clone();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match socket.stream.peek(&mut [0]) {
            Ok(0) => break,
            Err(e) if retryable(&e) => assert!(Instant::now() < deadline),
            result => panic!("expected idle EOF: {result:?}"),
        }
    }
    assert_eq!(
        transport.request("/api/test", "{}", &[], 2).unwrap(),
        (200, b"{}".to_vec())
    );
    server.join().unwrap();
    assert_eq!(sockets.lock().unwrap().len(), 2);
}

// Deterministic injection of the kernel write failure that can race a successful
// idle peek. The production RequestTransport and retry loop are unchanged.
#[derive(Debug)]
struct WriteFaultConnector {
    failures: Arc<AtomicUsize>,
    after_body: bool,
}
impl Connector for WriteFaultConnector {
    type Out = Box<dyn UreqTransport>;
    fn connect(
        &self,
        details: &ConnectionDetails,
        _: Option<()>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        let inner = PollTcpConnector.connect(details, None::<()>)?.unwrap();
        Ok(Some(Box::new(RequestTransport {
            inner: Box::new(WriteFault {
                inner,
                failures: self.failures.clone(),
                after_body: self.after_body,
            }),
            last_attempt: 0,
            header_match: 0,
            headers_done: false,
        })))
    }
}
#[derive(Debug)]
struct WriteFault {
    inner: Box<dyn UreqTransport>,
    failures: Arc<AtomicUsize>,
    after_body: bool,
}
impl UreqTransport for WriteFault {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }
    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        let output = &self.inner.buffers().output()[..amount];
        let fail_here = if self.after_body {
            output == b"{}"
        } else {
            output.starts_with(b"POST ")
        };
        if fail_here
            && self
                .failures
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
                .is_ok()
        {
            if self.after_body {
                self.inner.transmit_output(amount, timeout)?;
            }
            return Err(io::Error::from(io::ErrorKind::BrokenPipe).into());
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

#[test]
fn retries_reused_write_failure_once_on_fresh_connection() {
    for failure_count in [1, 2] {
        let (done, wait) = mpsc::channel();
        let (mut transport, server) = local_server(move |listener| {
            let mut stream = BufReader::new(accept(&listener));
            assert!(read_request(&mut stream));
            stream
                .get_mut()
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                .unwrap();
            assert!(
                !read_request(&mut stream),
                "failed attempt must close old socket"
            );
            let mut fresh = BufReader::new(accept(&listener));
            if failure_count == 1 {
                assert!(read_request(&mut fresh));
                fresh
                    .get_mut()
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                    .unwrap();
            } else {
                assert!(!read_request(&mut fresh));
            }
            wait.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                io::ErrorKind::WouldBlock,
                "must not retry twice"
            );
        });
        let failures = Arc::new(AtomicUsize::new(0));
        transport.agent = Agent::with_parts(
            transport.agent.config().clone(),
            WriteFaultConnector {
                failures: failures.clone(),
                after_body: false,
            },
            PollResolver::default(),
        );
        transport.request("/api/test", "{}", &[], 2).unwrap();
        failures.store(failure_count, Ordering::Relaxed);
        let result = transport.request("/api/test", "{}", &[], 2);
        done.send(()).unwrap();
        server.join().unwrap();
        if failure_count == 1 {
            assert_eq!(result.unwrap(), (200, b"{}".to_vec()));
        } else {
            assert_eq!(
                result.unwrap_err().to_string(),
                "HTTPS transport failed; response not logged"
            );
        }
        assert_eq!(failures.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn fresh_write_failure_is_not_retried() {
    let (done, wait) = mpsc::channel();
    let (mut transport, server) = local_server(move |listener| {
        let mut stream = BufReader::new(accept(&listener));
        assert!(!read_request(&mut stream));
        wait.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    });
    transport.agent = Agent::with_parts(
        transport.agent.config().clone(),
        WriteFaultConnector {
            failures: Arc::new(AtomicUsize::new(1)),
            after_body: false,
        },
        PollResolver::default(),
    );
    let result = transport.request("/api/test", "{}", &[], 2);
    done.send(()).unwrap();
    server.join().unwrap();
    assert_eq!(
        result.unwrap_err().to_string(),
        "HTTPS transport failed; response not logged"
    );
}

#[test]
fn failure_after_full_send_is_not_retried() {
    full_send_failure_is_not_retried(false);
}

#[test]
fn final_write_error_is_not_retried() {
    full_send_failure_is_not_retried(true);
}

fn full_send_failure_is_not_retried(inject_write_error: bool) {
    let (done, wait) = mpsc::channel();
    let (mut transport, server) = local_server(move |listener| {
        let mut stream = BufReader::new(accept(&listener));
        assert!(read_request(&mut stream));
        stream
            .get_mut()
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
            .unwrap();
        assert!(
            read_request(&mut stream),
            "second POST must use the pooled socket"
        );
        stream.get_ref().shutdown(Shutdown::Both).unwrap();
        drop(stream);
        wait.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock,
            "a fully sent POST must not be repeated"
        );
    });
    let failures = Arc::new(AtomicUsize::new(0));
    if inject_write_error {
        transport.agent = Agent::with_parts(
            transport.agent.config().clone(),
            WriteFaultConnector {
                failures: failures.clone(),
                after_body: true,
            },
            PollResolver::default(),
        );
    }
    transport.request("/api/test", "{}", &[], 2).unwrap();
    if inject_write_error {
        failures.store(1, Ordering::Relaxed);
    }
    let result = transport.request("/api/test", "{}", &[], 2);
    done.send(()).unwrap();
    server.join().unwrap();
    assert_eq!(
        result.unwrap_err().to_string(),
        "HTTPS transport failed; response not logged"
    );
}

#[test]
fn proxy_connect_keeps_target_tls_and_errors_private() {
    for reject in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let proxy = ureq::Proxy::new(&format!(
            "http://fake-user:fake-password@{}",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let transport = Transport::with_parts(Some(proxy), PollResolver::default());
        let server = thread::spawn(move || {
            let mut stream = BufReader::new(accept(&listener));
            let mut line = String::new();
            stream.read_line(&mut line).unwrap();
            assert_eq!(
                line,
                "CONNECT line-chrome-gw.line-apps.com:443 HTTP/1.1\r\n"
            );
            loop {
                line.clear();
                stream.read_line(&mut line).unwrap();
                assert!(!line.contains("origin-token"));
                if line == "\r\n" {
                    break;
                }
            }
            if reject {
                stream
                    .get_mut()
                    .write_all(b"HTTP/1.1 407 fake-password\r\nContent-Length: 0\r\n\r\n")
                    .unwrap();
            } else {
                stream
                    .get_mut()
                    .write_all(b"HTTP/1.1 200 OK\r\n\r\n")
                    .unwrap();
                let mut hello = [0; 3];
                stream.read_exact(&mut hello).unwrap();
                assert_eq!(
                    &hello[..2],
                    &[22, 3],
                    "CONNECT must be followed by target TLS, never plaintext POST"
                );
            }
        });
        let result = transport.request(
            "/api/test",
            "private-body",
            &[("x-line-access", "origin-token")],
            2,
        );
        server.join().unwrap();
        assert_eq!(
            format!("{:#}", result.unwrap_err()),
            "HTTPS transport failed; response not logged"
        );
    }
}

#[test]
fn environment_proxy_and_no_proxy_are_honoured() {
    const CHILD: &str = "LINE_TEST_PROXY_CHILD";
    if let Ok(expected) = std::env::var(CHILD) {
        let result = Transport::new();
        if expected == "invalid" {
            let Err(error) = result else {
                panic!("invalid proxy must fail closed");
            };
            assert_eq!(format!("{error:#}"), "invalid HTTPS proxy configuration");
        } else {
            let transport = result.unwrap();
            if expected == "direct" {
                assert!(transport.agent.config().proxy().is_none());
            } else {
                let proxy = transport.agent.config().proxy().expect("environment proxy");
                assert_eq!(proxy.host(), expected);
                assert!(proxy.is_no_proxy(&BASE.parse().unwrap()));
            }
        }
        return;
    }
    // Isolate every precedence/error case in a child process. No global env
    // mutation may change the routing of concurrently running loopback tests.
    let cases = [
        (
            "upper.invalid",
            vec![("HTTPS_PROXY", "http://upper.invalid")],
        ),
        (
            "lower.invalid",
            vec![
                ("https_proxy", "https://lower.invalid"),
                ("HTTPS_PROXY", "http://upper.invalid"),
                ("all_proxy", "http://all.invalid"),
            ],
        ),
        (
            "upper.invalid",
            vec![
                ("HTTPS_PROXY", "http://upper.invalid"),
                ("all_proxy", "http://all.invalid"),
            ],
        ),
        (
            "all-lower.invalid",
            vec![
                ("all_proxy", "http://all-lower.invalid"),
                ("ALL_PROXY", "http://all-upper.invalid"),
            ],
        ),
        (
            "all-upper.invalid",
            vec![("ALL_PROXY", "http://all-upper.invalid")],
        ),
        (
            "upper.invalid",
            vec![("https_proxy", ""), ("HTTPS_PROXY", "http://upper.invalid")],
        ),
        (
            "direct",
            vec![
                ("HTTP_PROXY", "http://wrong.invalid"),
                ("http_proxy", "http://wrong.invalid"),
            ],
        ),
        (
            "invalid",
            vec![
                ("https_proxy", "http://fake-user:fake-password@invalid host"),
                ("HTTPS_PROXY", "http://valid.invalid"),
            ],
        ),
        (
            "invalid",
            vec![
                (
                    "HTTPS_PROXY",
                    "socks5://fake-user:fake-password@proxy.invalid",
                ),
                ("ALL_PROXY", "http://valid.invalid"),
            ],
        ),
        (
            "invalid",
            vec![("ALL_PROXY", "unsupported://fake-password@proxy.invalid")],
        ),
        (
            "lower.invalid",
            vec![
                ("https_proxy", "http://lower.invalid"),
                ("no_proxy", "line-chrome-gw.line-apps.com"),
                ("NO_PROXY", "other.invalid"),
            ],
        ),
    ];
    for (expected, variables) in cases {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "client::http::regression_tests::environment_proxy_and_no_proxy_are_honoured",
            ])
            .env(CHILD, expected);
        for name in [
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "http_proxy",
            "ALL_PROXY",
            "all_proxy",
            "NO_PROXY",
            "no_proxy",
        ] {
            command.env_remove(name);
        }
        command.env("NO_PROXY", "line-chrome-gw.line-apps.com");
        let result = command.envs(variables).output().unwrap();
        assert!(
            result.status.success(),
            "proxy environment child failed ({expected})"
        );
    }
}
