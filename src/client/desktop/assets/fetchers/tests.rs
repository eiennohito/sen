use super::*;
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::{atomic::AtomicU64, mpsc},
};

struct Cache(PathBuf);
impl Cache {
    fn new() -> (Self, Arc<Mutex<ImageCache>>) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "target/t2/tests/{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let cache = Arc::new(Mutex::new(ImageCache::new(&path, "synthetic").unwrap()));
        (Self(path), cache)
    }
}
impl Drop for Cache {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn avatar(id: usize) -> Asset {
    Asset::avatar(&format!("/test-{id}")).unwrap()
}
fn png() -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]))
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}
fn respond(mut stream: TcpStream, bytes: &[u8]) {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    )
    .unwrap();
    stream.write_all(bytes).unwrap();
}

fn local_server(
    count: usize,
    serve: impl Fn(String, TcpStream) + Send + Sync + 'static,
) -> (http::Transport, mpsc::Receiver<String>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let http = http::Transport::local(format!("http://{}", listener.local_addr().unwrap()));
    let (tx, rx) = mpsc::channel();
    let serve = Arc::new(serve);
    let thread = thread::spawn(move || {
        let mut threads = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while threads.len() < count {
            let stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "asset accept timed out");
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }
                Err(e) => panic!("{e}"),
            };
            let tx = tx.clone();
            let serve = serve.clone();
            threads.push(thread::spawn(move || {
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = BufReader::new(stream);
                let mut headers = String::new();
                loop {
                    let mut line = String::new();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    headers.push_str(&line);
                    if line == "\r\n" {
                        break;
                    }
                }
                tx.send(headers.clone()).unwrap();
                serve(headers, reader.into_inner());
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
    });
    (http, rx, thread)
}

#[derive(Default)]
struct Gate(Mutex<bool>, Condvar);
impl Gate {
    fn wait(&self) {
        let guard = self.0.lock().unwrap();
        let (guard, _) = self
            .1
            .wait_timeout_while(guard, Duration::from_secs(4), |v| !*v)
            .unwrap();
        assert!(*guard, "test server release timed out");
    }
    fn release(&self) {
        *self.0.lock().unwrap() = true;
        self.1.notify_all();
    }
}

#[test]
fn parallel_downloads_overlap() {
    let gate = Arc::new(Gate::default());
    let release = gate.clone();
    let (http, requests, server) = local_server(THREADS, move |_, stream| {
        release.wait();
        thread::sleep(Duration::from_millis(300));
        respond(stream, &png());
    });
    let (_root, cache) = Cache::new();
    let (results, images) = mpsc::channel();
    let fetchers = Fetchers::start(
        http,
        cache,
        Arc::default(),
        Arc::default(),
        Failures::default(),
        |_, _| None,
        move |done, _| {
            results.send(done).unwrap();
        },
    );
    for i in 0..THREADS {
        assert!(fetchers.enqueue(avatar(i), Some("chat".into()), Purpose::Thumbnail));
    }
    for _ in 0..THREADS {
        requests.recv_timeout(Duration::from_secs(1)).unwrap();
    }
    assert!(images.try_recv().is_err());
    let started = Instant::now();
    gate.release();
    for _ in 0..THREADS {
        assert!(
            images
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .result
                .is_ok()
        );
    }
    assert!(
        started.elapsed() < Duration::from_millis(700),
        "downloads serialized: {:?}",
        started.elapsed()
    );
    drop(fetchers);
    server.join().unwrap();
}

#[test]
fn latest_visible_keys_then_current_chat_precede_old_chats() {
    let gates: Arc<Vec<Gate>> = Arc::new((0..THREADS).map(|_| Gate::default()).collect());
    let release = gates.clone();
    let (http, requests, server) = local_server(THREADS + 3, move |headers, stream| {
        for i in 0..THREADS {
            if headers.starts_with(&format!("GET /test-{i}/")) {
                release[i].wait();
            }
        }
        respond(stream, &png());
    });
    let (_root, cache) = Cache::new();
    let priority = Arc::new(Priority::default());
    let (tx, rx) = mpsc::channel();
    let fetchers = Fetchers::start(
        http,
        cache,
        Arc::default(),
        priority.clone(),
        Failures::default(),
        |_, _| None,
        move |done, _| {
            tx.send(done).unwrap();
        },
    );
    for i in 0..THREADS {
        assert!(fetchers.enqueue(avatar(i), None, Purpose::Thumbnail));
    }
    for _ in 0..THREADS {
        requests.recv_timeout(Duration::from_secs(1)).unwrap();
    }
    assert!(fetchers.enqueue(avatar(10), Some("old".into()), Purpose::Thumbnail));
    assert!(fetchers.enqueue(avatar(11), Some("open".into()), Purpose::Thumbnail));
    assert!(fetchers.enqueue(avatar(12), Some("open".into()), Purpose::Thumbnail));
    priority.set("old".into(), [avatar(10).key()]);
    priority.set("open".into(), [avatar(12).key()]);
    gates[0].release();
    for id in [12, 11, 10] {
        let request = requests.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            request.starts_with(&format!("GET /test-{id}/")),
            "{request}"
        );
    }
    for gate in gates.iter() {
        gate.release();
    }
    for _ in 0..THREADS + 3 {
        assert!(
            rx.recv_timeout(Duration::from_secs(2))
                .unwrap()
                .result
                .is_ok()
        );
    }
    server.join().unwrap();
}

#[test]
fn deduplicates_inflight_keys_and_cancellation_joins_stalled_fetchers_promptly() {
    let gate = Arc::new(Gate::default());
    let release = gate.clone();
    let (http, requests, server) = local_server(THREADS, move |_, _stream| {
        release.wait();
    });
    let (_root, cache) = Cache::new();
    let failures = Failures::default();
    let fetchers = Fetchers::start(
        http,
        cache,
        Arc::default(),
        Arc::default(),
        failures.clone(),
        |_, _| None,
        |_, _| {},
    );
    for i in 0..THREADS {
        for _ in 0..20 {
            assert!(fetchers.enqueue(avatar(i), None, Purpose::Thumbnail));
        }
    }
    for _ in 0..THREADS {
        requests.recv_timeout(Duration::from_secs(1)).unwrap();
    }
    assert_eq!(fetchers.shared.queue.lock().unwrap().entries.len(), THREADS);
    for i in 100..100 + QUEUE_LIMIT + 20 {
        fetchers.enqueue(avatar(i), None, Purpose::Thumbnail);
    }
    assert_eq!(
        fetchers.shared.queue.lock().unwrap().entries.len(),
        QUEUE_LIMIT
    );
    let started = Instant::now();
    drop(fetchers);
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "slow join: {:?}",
        started.elapsed()
    );
    assert!(failures.0.lock().unwrap().is_empty());
    gate.release();
    server.join().unwrap();
}

#[test]
fn blacklist_lasts_300_seconds_and_cached_private_images_need_no_token() {
    let (http, requests, server) = local_server(2, |_, mut stream| {
        stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
    });
    let (_root, cache) = Cache::new();
    let photo = Asset::photo(&json!({"id":"42"})).unwrap();
    cache.lock().unwrap().store(&photo, &png()).unwrap();
    let (tx, rx) = mpsc::channel();
    let fetchers = Fetchers::start(
        http,
        cache.clone(),
        Arc::default(),
        Arc::default(),
        Failures::default(),
        |_, _| panic!("cache hit must not authorize"),
        move |done, _| {
            tx.send(done).unwrap();
        },
    );
    assert!(fetchers.enqueue(photo.clone(), None, Purpose::Thumbnail));
    assert!(
        rx.recv_timeout(Duration::from_secs(1))
            .unwrap()
            .result
            .is_ok()
    );
    assert!(fetchers.enqueue(avatar(1), None, Purpose::Thumbnail));
    assert!(
        rx.recv_timeout(Duration::from_secs(1))
            .unwrap()
            .result
            .is_err()
    );
    requests.recv_timeout(Duration::from_secs(1)).unwrap();
    let failed = fetchers.shared.failures.0.lock().unwrap()[&avatar(1).key()];
    for i in 1..QUEUE_LIMIT {
        fetchers
            .shared
            .failures
            .0
            .lock()
            .unwrap()
            .insert(format!("failed-{i}"), failed);
    }
    assert!(fetchers.enqueue(avatar(1), None, Purpose::Thumbnail));
    assert!(
        rx.recv_timeout(Duration::from_secs(1))
            .unwrap()
            .result
            .is_err()
    );
    assert_eq!(
        fetchers.shared.failures.0.lock().unwrap()[&avatar(1).key()],
        failed
    );
    assert_eq!(
        fetchers.shared.failures.0.lock().unwrap().len(),
        QUEUE_LIMIT
    );
    fetchers
        .shared
        .failures
        .0
        .lock()
        .unwrap()
        .insert(avatar(1).key(), Instant::now() - Duration::from_secs(299));
    assert!(fetchers.enqueue(avatar(1), None, Purpose::Thumbnail));
    assert!(
        rx.recv_timeout(Duration::from_secs(1))
            .unwrap()
            .result
            .is_err()
    );
    assert!(requests.try_recv().is_err());
    fetchers
        .shared
        .failures
        .0
        .lock()
        .unwrap()
        .insert(avatar(1).key(), Instant::now() - Duration::from_secs(301));
    assert!(fetchers.enqueue(avatar(1), None, Purpose::Thumbnail));
    assert!(
        rx.recv_timeout(Duration::from_secs(1))
            .unwrap()
            .result
            .is_err()
    );
    requests.recv_timeout(Duration::from_secs(1)).unwrap();
    server.join().unwrap();
    fetchers.clear_failures();
    assert!(fetchers.shared.failures.0.lock().unwrap().is_empty());
    let cache = cache.lock().unwrap();
    assert_eq!(
        fs::metadata(cache.directory())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(cache.directory().join(format!("{}.png", photo.key())))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn shared_http_preserves_asset_hosts_paths_headers_and_no_redirects() {
    let (http, requests, server) = local_server(5, |headers, stream| {
        assert!(headers.starts_with("GET "));
        let lower = headers.to_ascii_lowercase();
        assert!(!lower.contains("x-line-locale:"));
        assert!(!lower.contains("content-type:"));
        if headers.contains("/r/talk/") {
            assert!(lower.contains("x-line-access: synthetic\r\n"));
            assert!(lower.contains("x-line-application: chromeos\t"));
            assert!(lower.contains("x-talk-meta:"));
        } else {
            assert!(!lower.contains("x-line-"));
            assert!(!lower.contains("x-talk-meta:"));
        }
        respond(stream, b"synthetic body");
    });
    let assets = [
        avatar(1),
        Asset::sticker(&json!({"STKID":"123"})).unwrap(),
        Asset::paid_reaction(&json!({"productId":"abc", "emojiId":"001"})).unwrap(),
        Asset::photo(&json!({"id":"42"})).unwrap(),
        Asset::attachment(&json!({"id":"43", "contentType":14})).unwrap(),
    ];
    for asset in &assets {
        assert_eq!(
            asset.fetch(&http, Some("synthetic")).unwrap(),
            b"synthetic body"
        );
    }
    for _ in 0..5 {
        requests.recv_timeout(Duration::from_secs(1)).unwrap();
    }
    server.join().unwrap();
    for url in [
        "https://line-chrome-gw.line-apps.com/api/TalkService.do",
        "https://obs.line-apps.com.evil.test/r/talk/m/42",
        "https://obs.line-apps.com@evil.test/r/talk/m/42",
        "https://obs.line-apps.com:443/r/talk/m/42",
        "http://obs.line-apps.com/r/talk/m/42",
        "https://obs.line-apps.com/r/\\talk/m/42",
        "https://obs.line-apps.com/r/talk/m/42\r\n",
    ] {
        let mut asset = assets[3].clone();
        asset.url = url.into();
        assert_eq!(
            asset
                .fetch(&http, Some("synthetic"))
                .unwrap_err()
                .to_string(),
            "unsupported asset host"
        );
    }
    assert!(
        assets[3]
            .fetch(&http, Some("token\r\ninjected: yes"))
            .is_err()
    );
    assert!(assets[3].fetch(&http, None).is_err());
    for oid in ["", "../object", "a/b", "a?b", "a%2fb", "日本語"] {
        assert!(Asset::photo(&json!({"id":"42","contentMetadata":{"OID":oid}})).is_none());
        assert!(
            Asset::attachment(
                &json!({"id":"42","contentType":14,"contentMetadata":{"OID":oid,"ENC_KM":"key"}})
            )
            .is_none()
        );
    }
    let (http, requests, server) = local_server(1, |_, mut stream| {
        stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: https://line-chrome-gw.line-apps.com/api/TalkService.do\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    });
    assert!(assets[3].fetch(&http, Some("synthetic")).is_err());
    requests.recv_timeout(Duration::from_secs(1)).unwrap();
    server.join().unwrap();
}

#[test]
fn download_and_decoder_caps_remain_enforced() {
    for chunked in [false, true] {
        let (http, requests, server) = local_server(1, move |_, mut stream| {
            if chunked {
                write!(stream, "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n", MAX_DOWNLOAD + 1).unwrap();
                let chunk = [0; 8192];
                for _ in 0..MAX_DOWNLOAD / chunk.len() {
                    stream.write_all(&chunk).unwrap();
                }
                let _ = stream.write_all(b"x\r\n0\r\n\r\n");
            } else {
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    MAX_DOWNLOAD + 1
                )
                .unwrap();
            }
        });
        assert_eq!(
            avatar(1).fetch(&http, None).unwrap_err().to_string(),
            "image download too large"
        );
        requests.recv_timeout(Duration::from_secs(1)).unwrap();
        server.join().unwrap();
    }
    assert!(pixels(&vec![0; MAX_DOWNLOAD + 1], 64).is_err());
    assert!(pixels(b"not an image", 64).is_err());
    let image = image::RgbaImage::new(8193, 1);
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, ImageFormat::Png).unwrap();
    assert!(pixels(bytes.get_ref(), 64).is_err());
}
