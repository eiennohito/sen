use super::*;

pub(crate) const WAIT: Duration = Duration::from_secs(10);
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    thread,
};
pub(crate) fn local_server<T: Send + 'static>(
    serve: impl FnOnce(TcpListener) -> T + Send + 'static,
) -> (Transport, thread::JoinHandle<T>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let transport = Transport::local(
        "9.8.7",
        format!("http://{}", listener.local_addr().unwrap()),
    );
    (
        transport,
        thread::spawn(move || {
            let address = listener.local_addr().unwrap();
            let (finished, done) = std::sync::mpsc::channel();
            let watchdog = thread::spawn(move || {
                if matches!(
                    done.recv_timeout(WAIT),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                ) {
                    let _ = TcpStream::connect(address);
                    panic!("test server did not finish");
                }
            });
            let result = serve(listener);
            let _ = finished.send(());
            watchdog.join().unwrap();
            result
        }),
    )
}
pub(crate) fn accept(listener: &TcpListener) -> TcpStream {
    let socket = listener.accept().unwrap().0;
    socket.set_read_timeout(Some(WAIT)).unwrap();
    socket.set_write_timeout(Some(WAIT)).unwrap();
    socket
}
pub(crate) fn request_headers(
    stream: &mut BufReader<TcpStream>,
) -> Option<(String, Option<usize>)> {
    let headers = raw_request_headers(stream)?;
    let request = headers.lines().next().unwrap().to_owned();
    let length = headers.lines().find_map(|line| {
        line.to_ascii_lowercase()
            .strip_prefix("content-length:")
            .map(|value| value.trim().parse().unwrap())
    });
    Some((request, length))
}
pub(crate) fn raw_request_headers(stream: &mut impl BufRead) -> Option<String> {
    let mut headers = String::new();
    loop {
        let mut line = String::new();
        if stream.read_line(&mut line).unwrap() == 0 {
            assert!(headers.is_empty(), "truncated request headers");
            return None;
        }
        headers.push_str(&line);
        if line == "\r\n" {
            return Some(headers);
        }
    }
}

pub(crate) fn request(stream: &mut BufReader<TcpStream>) -> Option<(String, Vec<u8>)> {
    let (request, length) = request_headers(stream)?;
    let mut body = vec![0; length.unwrap_or_default()];
    stream.read_exact(&mut body).unwrap();
    Some((request, body))
}
pub(crate) fn read_request(stream: &mut BufReader<TcpStream>) -> bool {
    let Some((request, body)) = request(stream) else {
        return false;
    };
    assert!(request.starts_with("POST /api/"));
    assert_eq!(body, b"{}");
    true
}
pub(crate) type Reply = Box<dyn FnOnce(serde_json::Value) -> (u16, serde_json::Value) + Send>;
pub(crate) fn server_responses(
    steps: Vec<(&'static str, Reply)>,
) -> (Transport, thread::JoinHandle<()>) {
    local_server(move |listener| {
        for (path, reply) in steps {
            let mut socket = BufReader::new(accept(&listener));
            let (request, body) = request(&mut socket).unwrap();
            assert_eq!(request, format!("POST {path} HTTP/1.1"));
            let (status, data) = reply(serde_json::from_slice(&body).unwrap());
            let body = data.to_string();
            write!(
                socket.get_mut(),
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    })
}

pub(crate) fn asset_server(
    count: usize,
    serve: impl Fn(String, TcpStream) + Send + Sync + 'static,
) -> (
    String,
    Transport,
    std::sync::mpsc::Receiver<String>,
    thread::JoinHandle<()>,
) {
    let (tx, rx) = std::sync::mpsc::channel();
    let serve = Arc::new(serve);
    let (transport, server) = local_server(move |listener| {
        let mut workers = Vec::new();
        for _ in 0..count {
            let stream = accept(&listener);
            let tx = tx.clone();
            let serve = serve.clone();
            workers.push(thread::spawn(move || {
                let mut reader = BufReader::new(stream);
                let headers = raw_request_headers(&mut reader).unwrap();
                tx.send(headers.clone()).unwrap();
                serve(headers, reader.into_inner());
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
    });
    let base = transport.base.clone();
    (base.clone(), Transport::local_assets(base), rx, server)
}

/// Manual time advances only when a test explicitly spends the silence budget.
pub(in crate::client) struct Clock {
    now: Mutex<Instant>,
    tick: async_channel::Sender<()>,
    ticks: async_channel::Receiver<()>,
    waiting: std::sync::mpsc::Sender<Duration>,
}
impl Clock {
    pub(in crate::client) fn new() -> (Arc<Self>, std::sync::mpsc::Receiver<Duration>) {
        let (tick, ticks) = async_channel::unbounded();
        let (waiting, waits) = std::sync::mpsc::channel();
        (
            Arc::new(Self {
                now: Mutex::new(Instant::now()),
                tick,
                ticks,
                waiting,
            }),
            waits,
        )
    }
    pub(in crate::client) fn now(&self) -> Instant {
        *self.now.lock().unwrap()
    }
    pub(in crate::client) fn advance(&self, duration: Duration) {
        *self.now.lock().unwrap() += duration;
        self.tick.try_send(()).unwrap();
    }
    pub(super) async fn sleep_until(&self, deadline: Instant) {
        self.waiting
            .send(deadline.saturating_duration_since(self.now()))
            .unwrap();
        while self.now() < deadline {
            self.ticks.recv().await.unwrap();
        }
    }
}
