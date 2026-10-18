use super::{BASE, REQUEST_CANCEL};
use anyhow::{Result, ensure};
use std::{
    cell::RefCell,
    io::{self, Read, Write},
    marker::PhantomData,
    net::{Shutdown, TcpStream},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use ureq::{
    Agent,
    http::{
        HeaderMap,
        header::{HeaderName, HeaderValue},
    },
    unversioned::{
        resolver::Resolver,
        transport::{
            Buffers, ConnectProxyConnector, ConnectionDetails, Connector, LazyBuffers, NextTimeout,
            RustlsConnector, Transport as UreqTransport,
        },
    },
};

const MAX_BODY: usize = 1048576;
const POLL: Duration = Duration::from_millis(50);
static NEXT_ATTEMPT: AtomicU64 = AtomicU64::new(1);
const CONNECT_BUDGET: Duration = Duration::from_secs(10);
// Retire before the assumed >=30s gateway idle window; liveness checks and the
// restricted retry still handle servers that close sooner. No live timing claim.
const POOL_IDLE: Duration = Duration::from_secs(15);

mod dns;
use dns::PollResolver;

// Requests and their body readers stay on the calling worker. Consult the current
// scope on every operation so pooled sockets never retain an earlier token/deadline.
thread_local! {
    static ACTIVE: RefCell<Option<RequestState>> = const { RefCell::new(None) };
}

struct RequestState {
    cancel: Option<Arc<AtomicBool>>,
    deadline: Instant,
    connect_deadline: Instant,
    connecting: bool,
    connector_depth: usize,
    attempt: u64,
    reused: bool,
    body_remaining: usize,
    final_write_started: bool,
    force_fresh: bool,
    pool: bool,
    write_failure: Option<io::ErrorKind>,
    socket: Option<Arc<Socket>>,
    watchdog: Option<Watchdog>,
}

struct Watchdog {
    timeout: Duration,
    remaining: Duration,
}

#[derive(Debug)]
struct PingExpired;
impl std::fmt::Display for PingExpired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SSE ping expired")
    }
}
impl std::error::Error for PingExpired {}

struct RequestScope;

impl RequestScope {
    fn new(timeout: Duration) -> Self {
        ACTIVE.with(|slot| {
            assert!(slot.borrow().is_none(), "nested HTTP request");
            *slot.borrow_mut() = Some(RequestState {
                cancel: REQUEST_CANCEL.with(|slot| slot.borrow().clone()),
                deadline: Instant::now() + timeout,
                connect_deadline: Instant::now() + CONNECT_BUDGET,
                connecting: true,
                connector_depth: 0,
                attempt: 0,
                reused: false,
                body_remaining: 0,
                final_write_started: false,
                force_fresh: false,
                pool: true,
                write_failure: None,
                socket: None,
                watchdog: None,
            });
        });
        Self
    }
}

impl Drop for RequestScope {
    fn drop(&mut self) {
        ACTIVE.with(|slot| {
            let Some(state) = slot.borrow_mut().take() else {
                return;
            };
            // This also closes a socket that reached the pool just before
            // cancellation was observed by the outer body-reading loop.
            let cancelled = state
                .cancel
                .as_ref()
                .is_some_and(|c| c.load(Ordering::Relaxed));
            if let Some(socket) = state.socket
                && (!state.pool || cancelled || Instant::now() >= state.deadline)
            {
                socket.close();
            }
        });
    }
}

fn check_request() -> Result<(), ureq::Error> {
    ACTIVE.with(|slot| {
        let state = slot.borrow();
        let state = state.as_ref().expect("HTTP request scope");
        if state
            .cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
        {
            return Err(
                io::Error::new(io::ErrorKind::ConnectionAborted, "request cancelled").into(),
            );
        }
        let now = Instant::now();
        if state.watchdog.is_none() && now >= state.deadline {
            return Err(ureq::Error::Timeout(ureq::Timeout::Global));
        }
        if state.connecting && now >= state.connect_deadline {
            return Err(ureq::Error::Timeout(ureq::Timeout::Connect));
        }
        Ok(())
    })
}

#[derive(Debug)]
struct Socket {
    stream: TcpStream,
    closed: AtomicBool,
}

impl Socket {
    fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        let _ = self.stream.shutdown(Shutdown::Both);
    }

    fn check(&self) -> Result<(), ureq::Error> {
        check_request().inspect_err(|_| self.close())
    }
}

// The stock CONNECT connector recursively invokes this chain for the proxy.
// Only the outermost connection gets HTTP request-progress tracking; CONNECT
// traffic and TLS records to an HTTPS proxy must not count as origin POST bytes.
#[derive(Debug, Default)]
struct PollConnector {
    proxy: ConnectProxyConnector,
    tcp: PollTcpConnector,
    tls: RustlsConnector,
}

impl Connector for PollConnector {
    type Out = Box<dyn UreqTransport>;

    fn connect(
        &self,
        details: &ConnectionDetails,
        _: Option<()>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        check_request()?;
        if details.config.proxy().is_some_and(|proxy| {
            !proxy.is_no_proxy(details.uri)
                && !matches!(
                    proxy.protocol(),
                    ureq::ProxyProtocol::Http | ureq::ProxyProtocol::Https
                )
        }) {
            // Never silently bypass an unsupported environment proxy.
            return Err(ureq::Error::InvalidProxyUrl);
        }
        let outermost = ACTIVE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let state = slot.as_mut().unwrap();
            let outermost = state.connector_depth == 0;
            state.connector_depth += 1;
            outermost
        });
        let result = (|| {
            let proxy = self.proxy.connect(details, None::<()>)?;
            let tcp = self.tcp.connect(details, proxy)?;
            let tls = self.tls.connect(details, tcp)?;
            Ok(tls.map(|transport| {
                let inner: Box<dyn UreqTransport> = Box::new(transport);
                if outermost {
                    Box::new(RequestTransport {
                        inner,
                        last_attempt: 0,
                        header_match: 0,
                        headers_done: false,
                    }) as Self::Out
                } else {
                    inner
                }
            }))
        })();
        ACTIVE.with(|slot| slot.borrow_mut().as_mut().unwrap().connector_depth -= 1);
        result
    }
}

#[derive(Debug, Default)]
struct PollTcpConnector;

impl<In: UreqTransport> Connector<In> for PollTcpConnector {
    type Out = Box<dyn UreqTransport>;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        if let Some(transport) = chained {
            return Ok(Some(Box::new(transport)));
        }
        let mut last = None;
        for addr in &details.addrs {
            check_request()?;
            let remaining = ACTIVE.with(|slot| {
                let slot = slot.borrow();
                let state = slot.as_ref().unwrap();
                state
                    .connect_deadline
                    .min(state.deadline)
                    .saturating_duration_since(Instant::now())
            });
            if remaining.is_zero() {
                return Err(ureq::Error::Timeout(ureq::Timeout::Connect));
            }
            match connect_polled(*addr, remaining) {
                Ok(stream) => {
                    stream.set_nodelay(true)?;
                    stream.set_read_timeout(Some(POLL))?;
                    stream.set_write_timeout(Some(POLL))?;
                    let socket = Arc::new(Socket {
                        stream,
                        closed: AtomicBool::new(false),
                    });
                    ACTIVE.with(|slot| {
                        slot.borrow_mut().as_mut().unwrap().socket = Some(socket.clone())
                    });
                    socket.check()?;
                    return Ok(Some(Box::new(PollTransport {
                        socket,
                        buffers: LazyBuffers::new(
                            details.config.input_buffer_size(),
                            details.config.output_buffer_size(),
                        ),
                    })));
                }
                Err(error) => last = Some(error),
            }
        }
        check_request()?;
        Err(last
            .map(ureq::Error::Io)
            .unwrap_or(ureq::Error::ConnectionFailed))
    }
}

fn connect_polled(addr: std::net::SocketAddr, remaining: Duration) -> Result<TcpStream, io::Error> {
    use std::os::fd::AsRawFd;
    let socket = socket2::Socket::new(
        socket2::Domain::for_address(addr),
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )?;
    socket.set_nonblocking(true)?;
    match socket.connect(&addr.into()) {
        Ok(()) => {}
        Err(error)
            if error.kind() == io::ErrorKind::WouldBlock
                || error.raw_os_error() == Some(libc::EINPROGRESS) =>
        {
            let deadline = Instant::now() + remaining;
            loop {
                check_request().map_err(io::Error::other)?;
                if let Some(error) = socket.take_error()? {
                    return Err(error);
                }
                if socket.peer_addr().is_ok() {
                    break;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                let mut poll = libc::pollfd {
                    fd: socket.as_raw_fd(),
                    events: libc::POLLOUT,
                    revents: 0,
                };
                let timeout = POLL.min(remaining).as_millis().max(1) as i32;
                if unsafe { libc::poll(&mut poll, 1, timeout) } < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() != io::ErrorKind::Interrupted {
                        return Err(error);
                    }
                }
            }
        }
        Err(error) => return Err(error),
    }
    socket.set_nonblocking(false)?;
    Ok(socket.into())
}

#[derive(Debug)]
struct RequestTransport {
    inner: Box<dyn UreqTransport>,
    last_attempt: u64,
    header_match: usize,
    headers_done: bool,
}

impl UreqTransport for RequestTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        // rustls may accept header plaintext while suppressing the underlying
        // flush error. Surface that error before any body plaintext is offered.
        if let Some(error) = take_write_failure() {
            return Err(error);
        }
        ACTIVE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let state = slot.as_mut().unwrap();
            if self.last_attempt != state.attempt {
                state.reused = self.last_attempt != 0;
                self.last_attempt = state.attempt;
                self.header_match = 0;
                self.headers_done = false;
                // DNS and the selected TCP/TLS transport are now ready.
                state.connecting = false;
            }
            // NextTimeout.reason may be Global even during a body write.
            // Count plaintext HTTP bytes above TLS instead of inferring the
            // phase from the timeout. The delimiter may span multiple buffers.
            let output = &self.inner.buffers().output()[..amount];
            let mut body_start = 0;
            if !self.headers_done {
                body_start = output.len();
                for (index, &byte) in output.iter().enumerate() {
                    let delimiter = b"\r\n\r\n";
                    if byte == delimiter[self.header_match] {
                        self.header_match += 1;
                    } else {
                        self.header_match = usize::from(byte == b'\r');
                    }
                    if self.header_match == delimiter.len() {
                        self.headers_done = true;
                        body_start = index + 1;
                        break;
                    }
                }
            }
            let body_bytes = output.len() - body_start;
            // A TLS flush can fail after the final encrypted bytes reached the
            // peer. Disable retry before attempting that final write, not after.
            state.final_write_started |= self.headers_done && body_bytes >= state.body_remaining;
            state.body_remaining = state.body_remaining.saturating_sub(body_bytes);
        });
        let result = self.inner.transmit_output(amount, timeout);
        // Check even after Ok: rustls Stream::write deliberately ignores the
        // socket error from its post-write complete_io call.
        if let Some(error) = take_write_failure() {
            return Err(error);
        }
        result
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        self.inner.await_input(timeout)
    }

    fn is_open(&mut self) -> bool {
        let force_fresh = ACTIVE.with(|slot| {
            let slot = slot.borrow();
            let state = slot.as_ref().unwrap();
            !state.pool || (state.force_fresh && self.last_attempt != state.attempt)
        });
        if force_fresh {
            return false;
        }
        self.inner.is_open()
    }

    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}

#[derive(Debug)]
struct PollTransport {
    socket: Arc<Socket>,
    buffers: LazyBuffers,
}

fn record_write_failure(kind: io::ErrorKind) -> ureq::Error {
    ACTIVE.with(|slot| {
        slot.borrow_mut()
            .as_mut()
            .unwrap()
            .write_failure
            .get_or_insert(kind);
    });
    io::Error::from(kind).into()
}

fn take_write_failure() -> Option<ureq::Error> {
    ACTIVE
        .with(|slot| slot.borrow_mut().as_mut().unwrap().write_failure.take())
        .map(|kind| io::Error::from(kind).into())
}

fn check_phase(deadline: Option<Instant>, timeout: NextTimeout) -> Result<(), ureq::Error> {
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        return Err(ureq::Error::Timeout(timeout.reason));
    }
    Ok(())
}

fn retryable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
    )
}

impl UreqTransport for PollTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        let deadline = Instant::now().checked_add(*timeout.after);
        let mut sent = 0;
        while sent < amount {
            self.socket.check()?;
            check_phase(deadline, timeout)?;
            // Preserve partial-write progress across polling timeouts. Retrying
            // write_all here could duplicate request bytes (including on TLS).
            match (&self.socket.stream).write(&self.buffers.output()[sent..amount]) {
                Ok(0) => return Err(record_write_failure(io::ErrorKind::WriteZero)),
                Ok(n) => sent += n,
                Err(error) if retryable(&error) => continue,
                Err(error) => return Err(record_write_failure(error.kind())),
            }
        }
        self.socket.check()
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        let deadline = Instant::now().checked_add(*timeout.after);
        loop {
            self.socket.check()?;
            check_phase(deadline, timeout)?;
            let expired = ACTIVE.with(|slot| {
                slot.borrow()
                    .as_ref()
                    .unwrap()
                    .watchdog
                    .as_ref()
                    .is_some_and(|watchdog| watchdog.remaining.is_zero())
            });
            if expired {
                return Err(io::Error::new(io::ErrorKind::TimedOut, PingExpired).into());
            }
            let started = Instant::now();
            let result = (&self.socket.stream).read(self.buffers.input_append_buf());
            ACTIVE.with(|slot| {
                if let Some(watchdog) = &mut slot.borrow_mut().as_mut().unwrap().watchdog {
                    watchdog.remaining = watchdog.remaining.saturating_sub(started.elapsed());
                }
            });
            match result {
                Ok(n) => {
                    self.socket.check()?;
                    self.buffers.input_appended(n);
                    return Ok(n > 0);
                }
                Err(error) if retryable(&error) => {
                    self.socket.check()?;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    fn is_open(&mut self) -> bool {
        // Called both when returning to and borrowing from the pool.
        ACTIVE.with(|slot| {
            slot.borrow_mut().as_mut().unwrap().socket = Some(self.socket.clone());
        });
        if self.socket.closed.load(Ordering::Relaxed) || self.socket.check().is_err() {
            return false;
        }
        if self.socket.stream.set_nonblocking(true).is_err() {
            return false;
        }
        let mut byte = [0];
        let open = matches!(self.socket.stream.peek(&mut byte), Err(e) if e.kind() == io::ErrorKind::WouldBlock);
        let restored = self.socket.stream.set_nonblocking(false).is_ok();
        open && restored
    }
}

// The RPC target is always HTTPS. Match curl's HTTPS-specific precedence;
// HTTP_PROXY/http_proxy never apply, and a selected invalid value fails closed.
fn environment_proxy() -> Result<Option<ureq::Proxy>> {
    let invalid = || anyhow::anyhow!("invalid HTTPS proxy configuration");
    let mut selected = None;
    for name in ["https_proxy", "HTTPS_PROXY", "all_proxy", "ALL_PROXY"] {
        if let Some(value) = std::env::var_os(name) {
            if value.is_empty() {
                continue;
            }
            selected = Some(value.into_string().map_err(|_| invalid())?);
            break;
        }
    }
    let Some(value) = selected else {
        return Ok(None);
    };
    let parsed = ureq::Proxy::new(&value).map_err(|_| invalid())?;
    ensure!(
        matches!(
            parsed.protocol(),
            ureq::ProxyProtocol::Http | ureq::ProxyProtocol::Https
        ),
        "invalid HTTPS proxy configuration"
    );
    let mut builder = ureq::Proxy::builder(parsed.protocol())
        .host(parsed.host())
        .port(parsed.port());
    if let Some(user) = parsed.username() {
        builder = builder.username(user);
    }
    if let Some(password) = parsed.password() {
        builder = builder.password(password);
    }
    // curl prefers the lower-case exclusion variable as well.
    let no_proxy = std::env::var_os("no_proxy")
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var_os("NO_PROXY").filter(|value| !value.is_empty()));
    if let Some(value) = no_proxy {
        let value = value.into_string().map_err(|_| invalid())?;
        for pattern in value.split(',').map(str::trim) {
            builder = builder.no_proxy(pattern);
        }
    }
    builder.build().map(Some).map_err(|_| invalid())
}

#[derive(Clone)]
pub(super) struct Transport {
    agent: Agent,
    #[cfg(test)]
    base: String,
    #[cfg(test)]
    asset_timeout: Duration,
}

impl Transport {
    pub(super) fn new() -> Result<Self> {
        Ok(Self::with_parts(
            environment_proxy()?,
            PollResolver::default(),
        ))
    }

    pub(super) fn assets(&self) -> Self {
        // A stalled asset lookup must not hold the RPC resolver's lock.
        Self {
            agent: Agent::with_parts(
                self.agent.config().clone(),
                PollConnector::default(),
                PollResolver::named("line-asset-dns"),
            ),
            #[cfg(test)]
            base: self.base.clone(),
            #[cfg(test)]
            asset_timeout: self.asset_timeout,
        }
    }

    fn with_parts(proxy: Option<ureq::Proxy>, resolver: impl Resolver) -> Self {
        let config = Agent::config_builder()
            .https_only(!cfg!(test))
            .proxy(proxy)
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_connect(Some(CONNECT_BUDGET))
            .max_idle_age(POOL_IDLE)
            .build();
        Self {
            agent: Agent::with_parts(config, PollConnector::default(), resolver),
            #[cfg(test)]
            base: BASE.into(),
            #[cfg(test)]
            asset_timeout: Duration::from_secs(15),
        }
    }

    pub(super) fn stream<T>(
        &self,
        path: &str,
        headers: &[(&str, &str)],
        stop: Arc<AtomicBool>,
        timeout: Duration,
        consume: impl FnOnce(u16, &HeaderMap, &mut StreamReader) -> Result<T>,
    ) -> Result<T> {
        let validated = validate(path, headers)?;
        #[cfg(not(test))]
        let base = BASE;
        #[cfg(test)]
        let base = &self.base;
        let _scope = RequestScope::new(timeout);
        ACTIVE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let state = slot.as_mut().unwrap();
            state.cancel = Some(stop);
            state.attempt = NEXT_ATTEMPT.fetch_add(1, Ordering::Relaxed);
            state.pool = false;
        });
        check_request().map_err(transport_error)?;
        let mut request = self.agent.get(format!("{base}{path}"));
        for (name, value) in validated {
            request = request.header(name, value);
        }
        let response = request.call();
        check_request().map_err(transport_error)?;
        let (parts, body) = response.map_err(transport_error)?.into_parts();
        let mut reader = StreamReader {
            body: body.into_reader(),
            _worker: PhantomData,
        };
        ACTIVE.with(|slot| {
            slot.borrow_mut().as_mut().unwrap().watchdog = Some(Watchdog {
                timeout,
                remaining: timeout,
            });
        });
        consume(parts.status.as_u16(), &parts.headers, &mut reader)
    }

    pub(super) fn request(
        &self,
        path: &str,
        body: &str,
        headers: &[(&str, &str)],
        timeout: u64,
    ) -> Result<(u16, Vec<u8>)> {
        let validated = validate(path, headers)?;
        #[cfg(not(test))]
        let base = BASE;
        #[cfg(test)]
        let base = &self.base;
        let _scope = RequestScope::new(Duration::from_secs(timeout));
        let mut response = loop {
            check_request().map_err(transport_error)?;
            ACTIVE.with(|slot| {
                let mut slot = slot.borrow_mut();
                let state = slot.as_mut().unwrap();
                state.attempt = NEXT_ATTEMPT.fetch_add(1, Ordering::Relaxed);
                state.reused = false;
                state.body_remaining = body.len();
                state.final_write_started = false;
                state.write_failure = None;
                state.connecting = true;
            });
            let remaining = ACTIVE.with(|slot| {
                slot.borrow()
                    .as_ref()
                    .unwrap()
                    .deadline
                    .saturating_duration_since(Instant::now())
            });
            let mut request = self
                .agent
                .post(format!("{base}{path}"))
                .config()
                .timeout_global(Some(remaining))
                .build();
            for (name, value) in &validated {
                request = request.header(name.clone(), value.clone());
            }
            let response = request.send(body.as_bytes());
            check_request().map_err(transport_error)?;
            match response {
                Ok(response) => break response,
                Err(error) => {
                    let retry = matches!(&error, ureq::Error::Io(e) if matches!(e.kind(),
                        io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset |
                        io::ErrorKind::ConnectionAborted | io::ErrorKind::NotConnected |
                        io::ErrorKind::UnexpectedEof | io::ErrorKind::WriteZero))
                        && ACTIVE.with(|slot| {
                            let slot = slot.borrow();
                            let state = slot.as_ref().unwrap();
                            state.reused && !state.final_write_started && !state.force_fresh
                        });
                    if !retry {
                        return Err(transport_error(error));
                    }
                    ACTIVE.with(|slot| {
                        let mut slot = slot.borrow_mut();
                        let state = slot.as_mut().unwrap();
                        if let Some(socket) = state.socket.take() {
                            socket.close();
                        }
                        state.force_fresh = true;
                    });
                }
            }
        };
        let status = response.status().as_u16();
        ensure!(
            response
                .headers()
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .is_none_or(|n| n <= MAX_BODY as u64),
            "HTTPS response exceeds 1048576 bytes"
        );
        let mut reader = response.body_mut().as_reader();
        let mut bytes = Vec::new();
        let mut chunk = [0; 8192];
        loop {
            check_request().map_err(transport_error)?;
            let remaining = (MAX_BODY - bytes.len() + 1).min(chunk.len());
            let n = reader.read(&mut chunk[..remaining]);
            check_request().map_err(transport_error)?;
            let n =
                n.map_err(|_| anyhow::anyhow!("HTTPS response read failed; response not logged"))?;
            if n == 0 {
                return Ok((status, bytes));
            }
            ensure!(
                bytes.len() + n <= MAX_BODY,
                "HTTPS response exceeds 1048576 bytes"
            );
            bytes.extend_from_slice(&chunk[..n]);
        }
    }

    pub(super) fn asset(&self, url: &str, headers: &[(&str, &str)], cap: usize) -> Result<Vec<u8>> {
        #[cfg(not(test))]
        let timeout = Duration::from_secs(15);
        #[cfg(test)]
        let timeout = self.asset_timeout;
        let _scope = RequestScope::new(timeout);
        ACTIVE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let state = slot.as_mut().unwrap();
            state.connect_deadline = Instant::now() + Duration::from_secs(5);
            state.attempt = NEXT_ATTEMPT.fetch_add(1, Ordering::Relaxed);
        });
        check_request().map_err(asset_error)?;
        #[cfg(test)]
        let local_url = format!(
            "{}{}",
            self.base,
            url.split_once("://")
                .unwrap()
                .1
                .split_once('/')
                .map(|(_, path)| format!("/{path}"))
                .unwrap_or_default()
        );
        #[cfg(test)]
        let url = if self.base == BASE { url } else { &local_url };
        let mut request = self.agent.get(url);
        for (name, value) in headers {
            ensure!(!value.contains(['\r', '\n']), "invalid media header");
            let mut value = HeaderValue::from_str(value)
                .map_err(|_| anyhow::anyhow!("invalid media header"))?;
            value.set_sensitive(true);
            request = request.header(*name, value);
        }
        let mut response = request.call().map_err(asset_error)?;
        ensure!(
            response.status() == 200,
            super::desktop::assets::Failure::Http(response.status().as_u16())
        );
        ensure!(
            response
                .headers()
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .is_none_or(|n| n <= cap as u64),
            super::desktop::assets::Failure::TooLarge
        );
        let mut reader = response.body_mut().as_reader();
        let mut bytes = Vec::new();
        let mut chunk = [0; 8192];
        loop {
            check_request().map_err(asset_error)?;
            let n = reader.read(&mut chunk[..(cap - bytes.len() + 1).min(8192)]);
            check_request().map_err(asset_error)?;
            let n = n.map_err(|error| asset_error(error.into()))?;
            ensure!(
                bytes.len() + n <= cap,
                super::desktop::assets::Failure::TooLarge
            );
            if n == 0 {
                return Ok(bytes);
            }
            bytes.extend_from_slice(&chunk[..n]);
        }
    }

    #[cfg(test)]
    pub(in crate::client) fn asset_timeout(mut self, timeout: Duration) -> Self {
        self.asset_timeout = timeout;
        self
    }

    #[cfg(test)]
    pub(in crate::client) fn local(base: String) -> Self {
        let mut transport = Self::with_parts(None, PollResolver::default());
        transport.base = base;
        transport
    }
}

// The body and its thread-local request scope must stay on the calling worker.
pub(super) struct StreamReader {
    body: ureq::BodyReader<'static>,
    _worker: PhantomData<Rc<()>>,
}

impl StreamReader {
    pub(super) fn ping(&mut self) {
        ACTIVE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let watchdog = slot.as_mut().unwrap().watchdog.as_mut().unwrap();
            watchdog.remaining = watchdog.timeout;
        });
    }
}

impl Read for StreamReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        check_request().map_err(|e| io::Error::other(transport_error(e)))?;
        let result = self.body.read(bytes);
        check_request().map_err(|e| io::Error::other(transport_error(e)))?;
        result.map_err(|error| {
            if error
                .get_ref()
                .is_some_and(|inner| inner.is::<PingExpired>())
            {
                error
            } else {
                io::Error::new(
                    error.kind(),
                    "HTTPS response read failed; response not logged",
                )
            }
        })
    }
}

fn validate(path: &str, headers: &[(&str, &str)]) -> Result<Vec<(HeaderName, HeaderValue)>> {
    ensure!(
        path.starts_with("/api/") && !path.contains(['\r', '\n']),
        "invalid API path"
    );
    let mut validated = Vec::with_capacity(headers.len());
    for (name, value) in headers {
        ensure!(
            !name.contains(['\r', '\n', ':']) && !value.contains(['\r', '\n']),
            "invalid header"
        );
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| anyhow::anyhow!("invalid header"))?;
        let mut value =
            HeaderValue::from_str(value).map_err(|_| anyhow::anyhow!("invalid header"))?;
        value.set_sensitive(true);
        validated.push((name, value));
    }
    Ok(validated)
}

fn asset_error(error: ureq::Error) -> anyhow::Error {
    use super::desktop::assets::Failure;
    if ACTIVE.with(|slot| {
        slot.borrow()
            .as_ref()
            .and_then(|s| s.cancel.as_ref())
            .is_some_and(|c| c.load(Ordering::Relaxed))
    }) {
        Failure::Cancelled
    } else if matches!(error, ureq::Error::Timeout(_))
        || ACTIVE.with(|slot| {
            slot.borrow()
                .as_ref()
                .is_some_and(|s| Instant::now() >= s.deadline)
        })
    {
        Failure::Timeout
    } else {
        Failure::Network
    }
    .into()
}

fn transport_error(error: ureq::Error) -> anyhow::Error {
    use super::desktop::assets::Failure;
    let failure = if ACTIVE.with(|slot| {
        slot.borrow()
            .as_ref()
            .and_then(|state| state.cancel.as_ref())
            .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
    }) {
        Failure::Cancelled
    } else if matches!(error, ureq::Error::Timeout(_)) {
        Failure::Timeout
    } else {
        Failure::Network
    };
    failure.transport_error()
}

#[cfg(test)]
pub(in crate::client) mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::{TcpListener, TcpStream},
        sync::{Arc, atomic::AtomicBool, mpsc},
        thread,
        time::Instant,
    };

    pub(in crate::client) fn local_server<T: Send + 'static>(
        serve: impl FnOnce(TcpListener) -> T + Send + 'static,
    ) -> (Transport, thread::JoinHandle<T>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut transport = Transport::with_parts(None, PollResolver::default());
        transport.base = format!("http://{}", listener.local_addr().unwrap());
        (transport, thread::spawn(move || serve(listener)))
    }

    pub(in crate::client) fn accept(listener: &TcpListener) -> TcpStream {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    stream
                        .set_write_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    return stream;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "server accept timed out");
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("server accept: {error}"),
            }
        }
    }

    pub(in crate::client) fn read_request(stream: &mut BufReader<TcpStream>) -> bool {
        let mut line = String::new();
        if stream.read_line(&mut line).unwrap() == 0 {
            return false;
        }
        assert!(line.starts_with("POST /api/"));
        let mut length = 0;
        loop {
            line.clear();
            assert!(stream.read_line(&mut line).unwrap() > 0);
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        stream.read_exact(&mut body).unwrap();
        assert_eq!(body, b"{}");
        true
    }

    #[test]
    fn local_connect_wakes_before_the_poll_interval() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let _scope = RequestScope::new(Duration::from_secs(1));
        let started = Instant::now();
        let _stream =
            connect_polled(listener.local_addr().unwrap(), Duration::from_secs(1)).unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(25),
            "local connect took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn reuses_one_connection_for_two_requests() {
        let (transport, server) = local_server(|listener| {
            let mut stream = BufReader::new(accept(&listener));
            let mut accepts = 1;
            for _ in 0..2 {
                if !read_request(&mut stream) {
                    stream = BufReader::new(accept(&listener));
                    accepts += 1;
                    assert!(read_request(&mut stream));
                }
                stream
                    .get_mut()
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                    .unwrap();
            }
            accepts
        });
        for _ in 0..2 {
            assert_eq!(
                transport.request("/api/test", "{}", &[], 2).unwrap(),
                (200, b"{}".to_vec())
            );
        }
        assert_eq!(server.join().unwrap(), 1);
    }

    #[test]
    fn does_not_follow_redirects() {
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let location = format!("http://{}/api/stolen", target.local_addr().unwrap());
        let (transport, server) = local_server(move |listener| {
            let mut stream = BufReader::new(accept(&listener));
            assert!(read_request(&mut stream));
            write!(
                stream.get_mut(),
                "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n"
            )
            .unwrap();
        });
        assert_eq!(
            transport
                .request("/api/test", "{}", &[("x-line-access", "secret")], 2)
                .unwrap(),
            (302, vec![])
        );
        server.join().unwrap();
        assert_eq!(
            target.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn caps_declared_and_streamed_bodies() {
        for framing in [
            "Content-Length: 1048577",
            "Transfer-Encoding: chunked",
            "Connection: close",
        ] {
            let (transport, server) = local_server(move |listener| {
                let mut stream = BufReader::new(accept(&listener));
                assert!(read_request(&mut stream));
                write!(stream.get_mut(), "HTTP/1.1 200 OK\r\n{framing}\r\n\r\n").unwrap();
                if framing.starts_with("Content-Length") {
                    return;
                }
                let body = vec![b'x'; MAX_BODY + 1];
                if framing.starts_with("Transfer-Encoding") {
                    write!(stream.get_mut(), "{:x}\r\n", body.len()).unwrap();
                }
                let _ = stream.get_mut().write_all(&body);
            });
            let error = transport.request("/api/test", "{}", &[], 2).unwrap_err();
            assert_eq!(error.to_string(), "HTTPS response exceeds 1048576 bytes");
            server.join().unwrap();
        }
    }

    #[test]
    fn accepts_body_at_the_limit() {
        let (transport, server) = local_server(|listener| {
            let mut stream = BufReader::new(accept(&listener));
            assert!(read_request(&mut stream));
            write!(
                stream.get_mut(),
                "HTTP/1.1 200 OK\r\nContent-Length: {MAX_BODY}\r\n\r\n"
            )
            .unwrap();
            stream.get_mut().write_all(&vec![b'x'; MAX_BODY]).unwrap();
        });
        let (status, bytes) = transport.request("/api/test", "{}", &[], 2).unwrap();
        assert_eq!(status, 200);
        assert_eq!(bytes, vec![b'x'; MAX_BODY]);
        server.join().unwrap();
    }

    #[test]
    fn rejects_invalid_paths_and_headers_without_requests() {
        let mut transport = Transport::new().unwrap();
        transport.base = "http://127.0.0.1:0".into();
        for path in [
            "//other.test/api/",
            "https://other.test/api/",
            "/api/test\r\n",
            "/other",
        ] {
            assert_eq!(
                transport
                    .request(path, "{}", &[], 1)
                    .unwrap_err()
                    .to_string(),
                "invalid API path"
            );
        }
        for header in [
            ("bad:name", "secret"),
            ("name\n", "secret"),
            ("name", "secret\r\nnext"),
        ] {
            let error = transport
                .request("/api/test", "{}", &[header], 1)
                .unwrap_err();
            assert_eq!(format!("{error:#}"), "invalid header");
        }
    }

    #[test]
    fn times_out_stalled_headers_and_body() {
        for headers in [false, true] {
            let (release, wait) = mpsc::channel();
            let (transport, server) = local_server(move |listener| {
                let mut stream = BufReader::new(accept(&listener));
                assert!(read_request(&mut stream));
                if headers {
                    stream
                        .get_mut()
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nx")
                        .unwrap();
                }
                let _ = wait.recv_timeout(Duration::from_secs(3));
            });
            let started = Instant::now();
            let error = transport
                .request("/api/test", "{}", &[("x-line-access", "secret-token")], 1)
                .unwrap_err();
            assert!(!format!("{error:#}").contains("secret-token"));
            assert!(started.elapsed() >= Duration::from_millis(900));
            assert!(started.elapsed() < Duration::from_secs(2));
            release.send(()).unwrap();
            server.join().unwrap();
        }
    }

    #[test]
    fn total_timeout_is_not_reset_by_body_reads() {
        let (transport, server) = local_server(|listener| {
            let mut stream = BufReader::new(accept(&listener));
            assert!(read_request(&mut stream));
            stream
                .get_mut()
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\n\r\n")
                .unwrap();
            for _ in 0..20 {
                if stream.get_mut().write_all(b"x").is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(150));
            }
        });
        let started = Instant::now();
        assert!(transport.request("/api/test", "{}", &[], 1).is_err());
        assert!(started.elapsed() >= Duration::from_millis(900));
        assert!(started.elapsed() < Duration::from_secs(2));
        server.join().unwrap();
    }

    #[test]
    fn precancelled_request_does_not_connect() {
        let mut transport = Transport::new().unwrap();
        transport.base = "http://127.0.0.1:0".into();
        let cancel = Arc::new(AtomicBool::new(true));
        super::super::set_request_cancel(cancel);
        let error = transport.request("/api/test", "{}", &[], 5).unwrap_err();
        REQUEST_CANCEL.with(|slot| *slot.borrow_mut() = None);
        assert_eq!(error.to_string(), "HTTPS request cancelled");
    }

    #[test]
    fn cancellation_aborts_stalled_headers_and_body() {
        for (headers, warmup) in [(false, false), (true, false), (false, true), (true, true)] {
            let (ready, wait) = mpsc::channel();
            let (transport, server) = local_server(move |listener| {
                let mut stream = BufReader::new(accept(&listener));
                assert!(read_request(&mut stream));
                if warmup {
                    stream
                        .get_mut()
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                        .unwrap();
                    assert!(
                        read_request(&mut stream),
                        "warmup connection must be reused"
                    );
                }
                if headers {
                    stream
                        .get_mut()
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nx")
                        .unwrap();
                }
                ready.send(()).unwrap();
                let mut byte = [0];
                assert_eq!(
                    stream.read(&mut byte).unwrap(),
                    0,
                    "cancel must close the request connection while the agent is alive"
                );
                // A later request on the same agent must establish a fresh connection.
                let mut fresh = BufReader::new(accept(&listener));
                assert!(read_request(&mut fresh));
                fresh
                    .get_mut()
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                    .unwrap();
            });
            let cancel = Arc::new(AtomicBool::new(false));
            let signal = cancel.clone();
            let request = thread::spawn(move || {
                if warmup {
                    // The pooled socket starts with a different token and deadline.
                    super::super::set_request_cancel(Arc::new(AtomicBool::new(false)));
                    transport.request("/api/test", "{}", &[], 2).unwrap();
                }
                super::super::set_request_cancel(cancel);
                let result = transport.request("/api/test", "{}", &[], 5);
                (transport, result)
            });
            wait.recv_timeout(Duration::from_secs(2)).unwrap();
            thread::sleep(Duration::from_millis(30));
            let started = Instant::now();
            signal.store(true, Ordering::Relaxed);
            let (transport, result) = request.join().unwrap();
            assert_eq!(result.unwrap_err().to_string(), "HTTPS request cancelled");
            assert!(started.elapsed() < Duration::from_millis(200));
            assert_eq!(
                transport.request("/api/test", "{}", &[], 2).unwrap(),
                (200, b"{}".to_vec())
            );
            server.join().unwrap();
        }
    }
}

#[cfg(test)]
mod regression_tests;

#[cfg(test)]
mod tls_tests;
