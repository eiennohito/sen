use super::{BASE, REQUEST_CANCEL};
use anyhow::{Result, ensure};
use async_executor::{Executor, Task};
use bytes::Bytes;
use futures_lite::{
    future,
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
};
use futures_rustls::{TlsConnector, rustls};
use http_body_util::BodyExt;
use hyper::{Request, body::Incoming, client::conn::http1::SendRequest, http::HeaderMap};
use std::{
    collections::HashMap,
    future::Future,
    io::{self, Read},
    pin::Pin,
    sync::{Arc, Mutex, OnceLock},
    task::{Context, Poll},
    time::{Duration, Instant},
};

const MAX_BODY: usize = 1048576;

#[derive(Debug)]
pub struct CancelToken {
    signal: async_channel::Sender<()>,
    wait: async_channel::Receiver<()>,
}
impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}
impl CancelToken {
    pub fn new() -> Self {
        let (signal, wait) = async_channel::unbounded();
        Self { signal, wait }
    }
    pub fn cancel(&self) {
        self.signal.close();
    }
    pub fn is_cancelled(&self) -> bool {
        self.signal.is_closed()
    }
    pub(crate) fn recv<T>(&self, receiver: &async_channel::Receiver<T>) -> Option<T> {
        future::block_on(future::or(
            async {
                self.cancelled().await;
                None
            },
            async { receiver.recv().await.ok() },
        ))
    }
    pub async fn cancelled(&self) {
        let _ = self.wait.recv().await;
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TransportError {
    #[error("network I/O failed")]
    Io(#[source] io::Error),
    #[error("invalid HTTPS proxy configuration")]
    ProxyConfiguration,
    #[error("request timed out")]
    Timeout,
    #[error("connection failed")]
    ConnectionFailed,
    #[error("HTTP protocol failure")]
    Protocol,
}
fn failure(sent: bool, source: TransportError) -> anyhow::Error {
    super::error::TransportFailure {
        write_started: sent,
        source,
    }
    .into()
}
fn network(
    sent: bool,
    error: impl Into<Box<dyn std::error::Error + Send + Sync>>,
) -> anyhow::Error {
    failure(sent, TransportError::Io(io::Error::other(error)))
}
async fn timeout<T>(
    duration: Duration,
    sent: bool,
    f: impl Future<Output = Result<T>>,
) -> Result<T> {
    future::or(f, async {
        async_io::Timer::after(duration).await;
        Err(failure(sent, TransportError::Timeout))
    })
    .await
}
#[derive(Clone, Default)]
pub(in crate::client) enum Clock {
    #[default]
    Real,
    #[cfg(test)]
    Manual(Arc<test_support::Clock>),
}
impl Clock {
    fn now(&self) -> Instant {
        match self {
            Self::Real => Instant::now(),
            #[cfg(test)]
            Self::Manual(clock) => clock.now(),
        }
    }
    async fn sleep_until(&self, deadline: Instant) {
        match self {
            Self::Real => {
                async_io::Timer::at(deadline).await;
            }
            #[cfg(test)]
            Self::Manual(clock) => clock.sleep_until(deadline).await,
        }
    }
}
async fn expires(clock: &Clock, deadline: Instant, idle: Option<(Duration, &Mutex<Progress>)>) {
    loop {
        let next = idle.map_or(deadline, |(duration, activity)| {
            activity
                .lock()
                .unwrap()
                .last
                .map_or(deadline, |last| last + duration)
        });
        clock.sleep_until(next).await;
        if idle.is_none_or(|(duration, activity)| {
            activity
                .lock()
                .unwrap()
                .last
                .is_none_or(|last| last + duration <= clock.now())
        }) {
            return;
        }
    }
}
async fn cancellable<T>(
    cancel: Option<&CancelToken>,
    f: impl Future<Output = Result<T>>,
) -> Result<T> {
    future::or(
        async {
            if let Some(cancel) = cancel {
                cancel.cancelled().await;
            } else {
                future::pending::<()>().await;
            }
            Err(super::error::Cancelled.into())
        },
        f,
    )
    .await
}

#[derive(Clone)]
struct Config {
    connect: Duration,
    pool_idle: Duration,
    asset_timeout: Duration,
    asset_connect: Duration,
    upload_idle: Duration,
    allow_http: bool,
    asset_hosts: Vec<String>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            pool_idle: Duration::from_secs(15),
            asset_timeout: Duration::from_secs(15),
            asset_connect: Duration::from_secs(5),
            upload_idle: Duration::from_secs(60),
            allow_http: false,
            asset_hosts: [
                "profile.line-scdn.net",
                "stickershop.line-scdn.net",
                "obs.line-apps.com",
                "manager.line-scdn.net",
            ]
            .map(str::to_owned)
            .to_vec(),
        }
    }
}
impl Config {
    fn allowed_asset_url(&self, url: &str, host: Option<&str>) -> bool {
        let prefix = if url.starts_with("https://") {
            "https://"
        } else if self.allow_http && url.starts_with("http://") {
            "http://"
        } else {
            return false;
        };
        let authority = url[prefix.len()..].split('/').next().unwrap_or_default();
        self.asset_hosts.iter().any(|host| host == authority)
            && host.is_none_or(|host| host == authority)
            && !url.contains(['\r', '\n', '\\'])
    }
}
struct Runtime {
    executor: Arc<Executor<'static>>,
}
impl Runtime {
    fn shared() -> Result<&'static Self> {
        static RUNTIME: OnceLock<Runtime> = OnceLock::new();
        static START: Mutex<()> = Mutex::new(());
        let _guard = START.lock().unwrap();
        if let Some(runtime) = RUNTIME.get() {
            return Ok(runtime);
        }
        let executor = Arc::new(Executor::new());
        let ex = executor.clone();
        std::thread::Builder::new()
            .name("sen-http".into())
            .spawn(move || async_io::block_on(ex.run(future::pending::<()>())))?;
        Ok(RUNTIME.get_or_init(|| Self { executor }))
    }
}
#[derive(Default)]
struct Progress {
    last: Option<Instant>,
}
struct Payload {
    bytes: Bytes,
    progress: Arc<Mutex<Progress>>,
    updates: async_channel::Sender<u64>,
    consumed: u64,
    clock: Clock,
}
impl hyper::body::Body for Payload {
    type Data = Bytes;
    type Error = std::convert::Infallible;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<hyper::body::Frame<Bytes>, Self::Error>>> {
        if self.bytes.is_empty() {
            return Poll::Ready(None);
        }
        let n = self.bytes.len().min(64 * 1024);
        let bytes = self.bytes.split_to(n);
        self.consumed += n as u64;
        let _ = self.updates.try_send(self.consumed);
        self.progress.lock().unwrap().last = Some(self.clock.now());
        Poll::Ready(Some(Ok(hyper::body::Frame::data(bytes))))
    }
    fn is_end_stream(&self) -> bool {
        self.bytes.is_empty()
    }
    fn size_hint(&self) -> hyper::body::SizeHint {
        hyper::body::SizeHint::with_exact(self.bytes.len() as u64)
    }
}
type Activity = Arc<Mutex<Progress>>;
struct Connection {
    activity: Activity,
    sender: SendRequest<Payload>,
    _driver: Task<()>,
    idle_since: Instant,
}
type Pool = Arc<Mutex<HashMap<String, Connection>>>;

#[derive(Clone)]
struct Proxy {
    url: url::Url,
    exclusions: String,
}
impl Proxy {
    fn parse(value: &str, exclusions: String) -> Result<Self> {
        let invalid = || failure(false, TransportError::ProxyConfiguration);
        let url = url::Url::parse(value).map_err(|_| invalid())?;
        ensure!(
            url.host_str().is_some() && url.port_or_known_default().is_some(),
            failure(false, TransportError::ProxyConfiguration)
        );
        Ok(Self { url, exclusions })
    }
    fn bypass(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        self.exclusions.split(',').map(str::trim).any(|p| {
            let p = p.trim_start_matches('.').to_ascii_lowercase();
            p == "*" || (!p.is_empty() && (host == p || host.ends_with(&format!(".{p}"))))
        })
    }
    fn environment() -> Result<Option<Self>> {
        Self::from_environment(|name| std::env::var_os(name))
    }
    fn from_environment(get: impl Fn(&str) -> Option<std::ffi::OsString>) -> Result<Option<Self>> {
        let value = |names: &[&str]| -> Result<Option<String>> {
            for name in names {
                if let Some(v) = get(name).filter(|v| !v.is_empty()) {
                    return v
                        .into_string()
                        .map(Some)
                        .map_err(|_| failure(false, TransportError::ProxyConfiguration));
                }
            }
            Ok(None)
        };
        value(&["https_proxy", "HTTPS_PROXY", "all_proxy", "ALL_PROXY"])?
            .map(|v| Self::parse(&v, value(&["no_proxy", "NO_PROXY"])?.unwrap_or_default()))
            .transpose()
    }
}

#[derive(Clone)]
pub(crate) struct Transport {
    pub(in crate::client) client_version: String,
    base: String,
    runtime: &'static Runtime,
    tls: TlsConnector,
    proxy: Option<Proxy>,
    pool: Pool,
    config: Config,
    pub(in crate::client) activity: super::NetworkActivity,
    pub(in crate::client) clock: Clock,
}
trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}
impl Transport {
    pub(super) fn new(client_version: &str) -> Result<Self> {
        let roots = rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        Self::configured(client_version, BASE.into(), Proxy::environment()?, roots)
    }
    fn configured(
        client_version: &str,
        base: String,
        proxy: Option<Proxy>,
        roots: rustls::RootCertStore,
    ) -> Result<Self> {
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("TLS versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
        Ok(Self {
            client_version: client_version.into(),
            base,
            runtime: Runtime::shared()?,
            tls: TlsConnector::from(Arc::new(config)),
            proxy,
            pool: Arc::default(),
            config: Config::default(),
            activity: Default::default(),
            clock: Clock::Real,
        })
    }
    pub(super) fn assets(&self) -> Self {
        Self {
            pool: Arc::default(),
            ..self.clone()
        }
    }
    async fn connect(
        &self,
        url: &url::Url,
        connect_timeout: Duration,
        activity: Activity,
    ) -> Result<Connection> {
        timeout(connect_timeout, false, async {
            let host = url
                .host_str()
                .ok_or_else(|| anyhow::anyhow!("missing HTTP host"))?;
            let port = url
                .port_or_known_default()
                .ok_or_else(|| anyhow::anyhow!("unsupported URL scheme"))?;
            let proxy = self.proxy.as_ref().filter(|p| !p.bypass(host));
            ensure!(
                proxy.is_none() || url.scheme() == "https",
                "proxy requires an HTTPS target"
            );
            ensure!(
                proxy.is_none_or(|p| p.url.scheme() == "http"),
                failure(false, TransportError::ProxyConfiguration)
            );
            let (connect_host, connect_port) = proxy.map_or((host, port), |p| {
                (
                    p.url.host_str().unwrap(),
                    p.url.port_or_known_default().unwrap(),
                )
            });
            let mut tcp = async_net::TcpStream::connect((connect_host, connect_port))
                .await
                .map_err(|e| network(false, e))?;
            tcp.set_nodelay(true).map_err(|e| network(false, e))?;
            if let Some(proxy) = proxy {
                use base64::Engine;
                let mut request =
                    format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n");
                if !proxy.url.username().is_empty() {
                    let mut credentials =
                        percent_encoding::percent_decode_str(proxy.url.username())
                            .collect::<Vec<u8>>();
                    credentials.push(b':');
                    credentials.extend(percent_encoding::percent_decode_str(
                        proxy.url.password().unwrap_or(""),
                    ));
                    request.push_str(&format!(
                        "Proxy-Authorization: Basic {}\r\n",
                        base64::engine::general_purpose::STANDARD.encode(credentials)
                    ));
                }
                request.push_str("\r\n");
                tcp.write_all(request.as_bytes())
                    .await
                    .map_err(|e| network(false, e))?;
                let mut response = Vec::new();
                while !response.ends_with(b"\r\n\r\n") {
                    ensure!(
                        response.len() < 16384,
                        failure(false, TransportError::Protocol)
                    );
                    let mut byte = [0];
                    tcp.read_exact(&mut byte)
                        .await
                        .map_err(|e| network(false, e))?;
                    response.push(byte[0]);
                }
                let status = response.split(|b| *b == b' ').nth(1);
                ensure!(
                    status == Some(b"200".as_slice()),
                    failure(false, TransportError::ConnectionFailed)
                );
            }
            let io: Box<dyn Io> = match url.scheme() {
                "https" => {
                    let name = rustls::pki_types::ServerName::try_from(host.to_owned())
                        .map_err(|e| network(false, e))?;
                    Box::new(
                        self.tls
                            .connect(name, tcp)
                            .await
                            .map_err(|e| network(false, e))?,
                    )
                }
                "http" => Box::new(tcp),
                _ => anyhow::bail!("unsupported URL scheme"),
            };
            let (sender, connection) = hyper::client::conn::http1::handshake(glue::FuturesIo(
                io,
                activity.clone(),
                self.clock.clone(),
            ))
            .await
            .map_err(|e| network(false, e))?;
            let driver = self.runtime.executor.spawn(async move {
                let _ = connection.await;
            });
            Ok(Connection {
                activity,
                sender,
                _driver: driver,
                idle_since: Instant::now(),
            })
        })
        .await
    }
    // All methods share establishment, stale-idle recovery and deadline/cancel races.
    fn send(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Bytes,
        options: SendOptions,
        mut progress: Option<&mut dyn FnMut(u64)>,
    ) -> Result<StreamReader> {
        let SendOptions {
            duration,
            cancel,
            mode,
            connect,
        } = options;
        let stream = matches!(mode, SendMode::Stream);
        let idle = matches!(mode, SendMode::Upload);
        let url = url::Url::parse(url)?;
        ensure!(
            url.scheme() == "https" || (self.config.allow_http && url.scheme() == "http"),
            "HTTPS URL required"
        );
        if cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err(super::error::Cancelled.into());
        }
        let key = url.origin().ascii_serialization();
        let mut builder = Request::builder()
            .method(method)
            .uri(&url[url::Position::BeforePath..]);
        builder = builder.header(
            "host",
            &url[url::Position::BeforeHost..url::Position::AfterPort],
        );
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let cached = if stream {
            None
        } else {
            self.pool.lock().unwrap().remove(&key)
        };
        let activity = cached
            .as_ref()
            .map(|c| c.activity.clone())
            .unwrap_or_default();
        *activity.lock().unwrap() = Progress::default();
        let (updates, progress_updates) = async_channel::unbounded();
        let mut request = builder.body(Payload {
            bytes: body,
            progress: activity.clone(),
            updates,
            consumed: 0,
            clock: self.clock.clone(),
        })?;
        let deadline = self.clock.now() + duration;
        let sent = std::cell::Cell::new(false);
        self.activity.record();
        let send = cancellable(
            cancel.as_deref(),
            future::or(
                async {
                    let reused = cached
                        .as_ref()
                        .is_some_and(|c| c.idle_since.elapsed() < self.config.pool_idle);
                    let mut connection = if reused {
                        cached.unwrap()
                    } else {
                        drop(cached);
                        self.connect(&url, connect, activity.clone()).await?
                    };
                    for retry in 0..2 {
                        if connection.sender.ready().await.is_err() {
                            drop(connection);
                            connection = self.connect(&url, connect, activity.clone()).await?;
                            connection
                                .sender
                                .ready()
                                .await
                                .map_err(|e| network(false, e))?;
                        }
                        // The IO activity belongs to this connection; reset it for this request.
                        connection.activity.lock().unwrap().last = Some(self.clock.now());
                        sent.set(true);
                        let response = connection.sender.try_send_request(request).await;
                        match response {
                            Ok(response) => return Ok((response, connection)),
                            Err(mut error) => {
                                if retry == 0
                                    && let Some(unsent) = error.take_message()
                                {
                                    sent.set(false);
                                    request = unsent;
                                    drop(connection);
                                    connection =
                                        self.connect(&url, connect, activity.clone()).await?;
                                    continue;
                                }
                                let delivered = error.take_message().is_none();
                                return Err(network(delivered, error.into_error()));
                            }
                        }
                    }
                    unreachable!()
                },
                async {
                    expires(
                        &self.clock,
                        deadline,
                        idle.then_some((duration, activity.as_ref())),
                    )
                    .await;
                    Err(failure(sent.get(), TransportError::Timeout))
                },
            ),
        );
        let (response, connection) = future::block_on(async {
            let result = future::or(send, async {
                loop {
                    if let Ok(consumed) = progress_updates.recv().await {
                        if let Some(callback) = progress.as_mut() {
                            callback(consumed);
                        }
                    } else {
                        future::pending::<()>().await;
                    }
                }
            })
            .await;
            while let Ok(consumed) = progress_updates.try_recv() {
                if let Some(callback) = progress.as_mut() {
                    callback(consumed);
                }
            }
            result
        })?;
        let (parts, body) = response.into_parts();
        Ok(StreamReader {
            body,
            eof: false,
            pending: Bytes::new(),
            connection: Some(connection),
            pool: (!stream).then(|| (self.pool.clone(), key)),
            cancel,
            deadline: if stream {
                ReadDeadline::Active(duration)
            } else {
                ReadDeadline::Total(deadline)
            },
            idle: (stream || idle).then_some(duration),
            status: parts.status.as_u16(),
            headers: parts.headers,
            clock: self.clock.clone(),
        })
    }
    pub(super) fn request(
        &self,
        path: &str,
        body: &str,
        headers: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<(u16, Vec<u8>)> {
        ensure!(path.starts_with("/api/"), "invalid API path");
        let mut reader = self.send(
            "POST",
            &format!("{}{path}", self.base),
            headers,
            Bytes::copy_from_slice(body.as_bytes()),
            SendOptions::request(timeout, self.config.connect),
            None,
        )?;
        let status = reader.status;
        Ok((status, reader.capped(MAX_BODY, false)?))
    }
    pub(in crate::client) fn allowed_asset_url(&self, url: &str) -> bool {
        self.config.allowed_asset_url(url, None)
    }
    pub(in crate::client) fn production_asset_url(url: &str, host: Option<&str>) -> bool {
        Config::default().allowed_asset_url(url, host)
    }
    pub(super) fn asset(&self, url: &str, headers: &[(&str, &str)], cap: usize) -> Result<Vec<u8>> {
        ensure!(
            self.allowed_asset_url(url),
            super::desktop::assets::Failure::UnsupportedHost
        );
        let mut reader = self.send(
            "GET",
            url,
            headers,
            Bytes::new(),
            SendOptions::request(self.config.asset_timeout, self.config.asset_connect),
            None,
        )?;
        ensure!(
            reader.status == 200,
            super::desktop::assets::Failure::Http(reader.status)
        );
        reader.capped(cap, true)
    }
    /// Large media: the timeout applies to each idle gap, not the whole body.
    pub(super) fn asset_stream(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        cap: u64,
        mut consume: impl FnMut(&[u8]) -> Result<()>,
    ) -> Result<()> {
        ensure!(
            self.allowed_asset_url(url),
            super::desktop::assets::Failure::UnsupportedHost
        );
        let mut reader = self.send(
            "GET",
            url,
            headers,
            Bytes::new(),
            SendOptions {
                mode: SendMode::Stream,
                ..SendOptions::request(self.config.asset_timeout, self.config.asset_connect)
            },
            None,
        )?;
        ensure!(
            reader.status == 200,
            super::desktop::assets::Failure::Http(reader.status)
        );
        if reader
            .headers
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .is_some_and(|n| n > cap)
        {
            return Err(super::desktop::assets::Failure::TooLarge.into());
        }
        let mut total = 0;
        let mut buffer = vec![0; 64 * 1024];
        loop {
            let result = reader.read(&mut buffer);
            if reader.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
                return Err(super::error::Cancelled.into());
            }
            let n = result?;
            if n == 0 {
                return Ok(());
            }
            total += n as u64;
            ensure!(total <= cap, super::desktop::assets::Failure::TooLarge);
            reader.ping();
            consume(&buffer[..n])?;
        }
    }
    pub(super) fn emoji_font(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        cap: usize,
    ) -> Result<(u16, HeaderMap, Vec<u8>)> {
        let parsed = url::Url::parse(url)?;
        ensure!(
            parsed.scheme() == "https"
                && parsed.host_str() == Some("cdn.jsdelivr.net")
                && parsed.port().is_none()
                && parsed.username().is_empty()
                && parsed.password().is_none(),
            "unsupported emoji font host"
        );
        #[cfg(test)]
        let local_url = self
            .config
            .allow_http
            .then(|| format!("{}{}", self.base, parsed.path()));
        #[cfg(test)]
        let url = local_url.as_deref().unwrap_or(url);
        let mut reader = self.send(
            "GET",
            url,
            headers,
            Bytes::new(),
            SendOptions::request(Duration::from_secs(60), Duration::from_secs(10)),
            None,
        )?;
        ensure!(
            matches!(reader.status, 200 | 304),
            "emoji font HTTP {}",
            reader.status
        );
        Ok((
            reader.status,
            reader.headers.clone(),
            reader.capped(cap, false)?,
        ))
    }
    pub(super) fn upload(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        bytes: Bytes,
        progress: &mut dyn FnMut(u64),
    ) -> Result<String> {
        let mut headers = headers.to_vec();
        headers.push(("content-type", "application/x-www-form-urlencoded"));
        let mut reader = self.send(
            "POST",
            url,
            &headers,
            bytes,
            SendOptions {
                mode: SendMode::Upload,
                ..SendOptions::request(self.config.upload_idle, self.config.connect)
            },
            Some(progress),
        )?;
        ensure!(
            reader.status == 201,
            super::error::RpcError {
                gateway: 0,
                method: "upload".into(),
                http: reader.status,
                code: 0,
                status: 0
            }
        );
        let oid = reader
            .headers
            .get("x-obs-oid")
            .and_then(|h| h.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        reader.capped(MAX_BODY, false)?;
        Ok(oid)
    }
    pub(super) fn stream<T>(
        &self,
        path: &str,
        headers: &[(&str, &str)],
        stop: Arc<CancelToken>,
        timeout: Duration,
        consume: impl FnOnce(u16, &HeaderMap, &mut StreamReader) -> Result<T>,
    ) -> Result<T> {
        ensure!(path.starts_with("/api/"), "invalid API path");
        let mut reader = self.send(
            "GET",
            &format!("{}{path}", self.base),
            headers,
            Bytes::new(),
            SendOptions {
                duration: timeout,
                cancel: Some(stop),
                mode: SendMode::Stream,
                connect: self.config.connect,
            },
            None,
        )?;
        ensure!(
            reader.status < 500,
            super::error::RpcError {
                gateway: 0,
                method: "stream".into(),
                http: reader.status,
                code: 0,
                status: 0
            }
        );
        let headers = reader.headers.clone();
        consume(reader.status, &headers, &mut reader)
    }
    #[cfg(test)]
    pub(in crate::client) fn local(client_version: &str, base: String) -> Self {
        let mut transport =
            Self::configured(client_version, base, None, rustls::RootCertStore::empty()).unwrap();
        transport.config.allow_http = true;
        transport
    }
    #[cfg(test)]
    pub(in crate::client) fn local_assets(base: String) -> Self {
        let mut transport = Self::local("9.8.7", base.clone());
        transport.config.asset_hosts = vec![base.trim_start_matches("http://").to_owned()];
        transport
    }
    #[cfg(test)]
    pub(in crate::client) fn asset_timeout(mut self, timeout: Duration) -> Self {
        self.config.asset_timeout = timeout;
        self
    }
}
struct SendOptions {
    duration: Duration,
    cancel: Option<Arc<CancelToken>>,
    mode: SendMode,
    connect: Duration,
}
enum SendMode {
    Request,
    Stream,
    Upload,
}
impl SendOptions {
    fn request(duration: Duration, connect: Duration) -> Self {
        Self {
            duration,
            cancel: current_cancel(),
            mode: SendMode::Request,
            connect,
        }
    }
}
fn current_cancel() -> Option<Arc<CancelToken>> {
    REQUEST_CANCEL.with(|slot| slot.borrow().clone())
}

enum ReadDeadline {
    Total(Instant),
    Active(Duration),
}
pub(super) struct StreamReader {
    body: Incoming,
    eof: bool,
    pending: Bytes,
    connection: Option<Connection>,
    pool: Option<(Pool, String)>,
    cancel: Option<Arc<CancelToken>>,
    deadline: ReadDeadline,
    idle: Option<Duration>,
    status: u16,
    headers: HeaderMap,
    clock: Clock,
}
impl StreamReader {
    pub(super) fn ping(&mut self) {
        if let (ReadDeadline::Active(remaining), Some(idle)) = (&mut self.deadline, self.idle) {
            *remaining = idle;
        }
    }
    fn capped(&mut self, cap: usize, asset: bool) -> Result<Vec<u8>> {
        let too_large = || {
            if asset {
                super::desktop::assets::Failure::TooLarge.into()
            } else {
                anyhow::anyhow!("HTTPS response exceeds {cap} bytes")
            }
        };
        if self
            .headers
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .is_some_and(|n| n > cap as u64)
        {
            return Err(too_large());
        }
        let mut bytes = Vec::new();
        let mut buffer = [0; 8192];
        loop {
            let result = self.read(&mut buffer[..(cap - bytes.len() + 1).min(8192)]);
            if self.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
                return Err(super::error::Cancelled.into());
            }
            let n = result?;
            if bytes.len() + n > cap {
                return Err(too_large());
            }
            if n == 0 {
                if let Some((pool, key)) = self.pool.take()
                    && let Some(mut connection) = self.connection.take()
                {
                    connection.idle_since = Instant::now();
                    pool.lock().unwrap().insert(key, connection);
                }
                return Ok(bytes);
            }
            bytes.extend_from_slice(&buffer[..n]);
        }
    }
    async fn chunk(&mut self) -> Result<()> {
        let started = self.clock.now();
        let (deadline, upload_idle) = match self.deadline {
            ReadDeadline::Total(deadline) => (deadline, self.idle),
            ReadDeadline::Active(remaining) => (started + remaining, None),
        };
        let frame = cancellable(
            self.cancel.as_deref(),
            future::or(
                async {
                    self.body
                        .frame()
                        .await
                        .transpose()
                        .map_err(|e| network(true, e))
                },
                async {
                    expires(
                        &self.clock,
                        deadline,
                        upload_idle
                            .zip(self.connection.as_ref())
                            .map(|(idle, connection)| (idle, connection.activity.as_ref())),
                    )
                    .await;
                    Err(failure(true, TransportError::Timeout))
                },
            ),
        )
        .await;
        if let ReadDeadline::Active(remaining) = &mut self.deadline {
            let elapsed = self.clock.now() - started;
            *remaining = remaining.saturating_sub(elapsed);
        }
        let frame = frame.inspect_err(|_| {
            self.connection.take();
            self.pool.take();
        })?;
        if let Some(frame) = frame {
            self.pending = frame.into_data().unwrap_or_default();
        } else {
            self.eof = true;
        }
        Ok(())
    }
}
impl Read for StreamReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        loop {
            if !self.pending.is_empty() {
                let n = bytes.len().min(self.pending.len());
                bytes[..n].copy_from_slice(&self.pending.split_to(n));
                return Ok(n);
            }
            if self.eof {
                return Ok(0);
            }
            future::block_on(self.chunk()).map_err(read_error)?;
        }
    }
}
fn read_error(error: anyhow::Error) -> io::Error {
    let kind = if super::error::find_source::<super::error::TransportFailure>(&error)
        .is_some_and(|f| matches!(f.source, TransportError::Timeout))
    {
        io::ErrorKind::TimedOut
    } else if error.chain().any(|e| {
        e.downcast_ref::<hyper::Error>()
            .is_some_and(hyper::Error::is_incomplete_message)
    }) {
        io::ErrorKind::UnexpectedEof
    } else {
        error
            .chain()
            .filter_map(|e| e.downcast_ref::<io::Error>())
            .map(io::Error::kind)
            .find(|k| *k != io::ErrorKind::Other)
            .unwrap_or(io::ErrorKind::Other)
    };
    io::Error::new(
        kind,
        error.reallocate_into_boxed_dyn_error_without_backtrace(),
    )
}

// Private hyper <-> futures-io adapter, following smol-hyper's MIT/Apache-2.0
// adapter (copyright smol-rs contributors). All wrapped streams are Unpin.
mod glue {
    use super::*;
    pub(super) struct FuturesIo<T>(pub T, pub Activity, pub Clock);
    impl<T: AsyncRead + Unpin> hyper::rt::Read for FuturesIo<T> {
        fn poll_read(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            mut buf: hyper::rt::ReadBufCursor<'_>,
        ) -> Poll<io::Result<()>> {
            // Initialize the offered region before exposing it to futures-io.
            let slice = unsafe {
                let region = buf.as_mut();
                region.as_mut_ptr().write_bytes(0, region.len());
                std::slice::from_raw_parts_mut(region.as_mut_ptr().cast::<u8>(), region.len())
            };
            let this = self.get_mut();
            match Pin::new(&mut this.0).poll_read(cx, slice) {
                Poll::Ready(Ok(n)) => {
                    if n > 0 {
                        this.1.lock().unwrap().last = Some(this.2.now());
                    }
                    unsafe {
                        buf.advance(n);
                    }
                    Poll::Ready(Ok(()))
                }
                Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                Poll::Pending => Poll::Pending,
            }
        }
    }
    impl<T: AsyncWrite + Unpin> hyper::rt::Write for FuturesIo<T> {
        fn poll_write(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            Pin::new(&mut self.get_mut().0).poll_write(cx, bytes)
        }
        fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.get_mut().0).poll_flush(cx)
        }
        fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.get_mut().0).poll_close(cx)
        }
        fn poll_write_vectored(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            bytes: &[io::IoSlice<'_>],
        ) -> Poll<io::Result<usize>> {
            Pin::new(&mut self.get_mut().0).poll_write_vectored(cx, bytes)
        }
    }
}
#[cfg(test)]
mod regression_tests;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tls_tests;
