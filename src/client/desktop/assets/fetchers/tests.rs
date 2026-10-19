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
    let http = http::Transport::local(
        "9.8.7",
        format!("http://{}", listener.local_addr().unwrap()),
    );
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
        |_, _| Ok(None),
        move |done, _| {
            results.send(done).unwrap();
        },
    );
    for i in 0..THREADS {
        assert!(fetchers.enqueue(avatar(i), Some("chat".into()), Purpose::Thumbnail));
    }
    for _ in 0..THREADS {
        requests.recv_timeout(Duration::from_secs(10)).unwrap();
    }
    assert!(images.try_recv().is_err());
    let started = Instant::now();
    gate.release();
    for _ in 0..THREADS {
        assert!(
            images
                .recv_timeout(Duration::from_secs(10))
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
        |_, _| Ok(None),
        move |done, _| {
            tx.send(done).unwrap();
        },
    );
    for i in 0..THREADS {
        assert!(fetchers.enqueue(avatar(i), None, Purpose::Thumbnail));
    }
    for _ in 0..THREADS {
        requests.recv_timeout(Duration::from_secs(10)).unwrap();
    }
    assert!(fetchers.enqueue(avatar(10), Some("old".into()), Purpose::Thumbnail));
    assert!(fetchers.enqueue(avatar(11), Some("open".into()), Purpose::Thumbnail));
    assert!(fetchers.enqueue(avatar(12), Some("open".into()), Purpose::Thumbnail));
    priority.set("old".into(), [avatar(10).key()]);
    priority.set("open".into(), [avatar(12).key()]);
    gates[0].release();
    for id in [12, 11, 10] {
        let request = requests.recv_timeout(Duration::from_secs(10)).unwrap();
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
            rx.recv_timeout(Duration::from_secs(10))
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
        |_, _| Ok(None),
        |_, _| {},
    );
    for i in 0..THREADS {
        for _ in 0..20 {
            assert!(fetchers.enqueue(avatar(i), None, Purpose::Thumbnail));
        }
    }
    for _ in 0..THREADS {
        requests.recv_timeout(Duration::from_secs(10)).unwrap();
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
fn permanent_failure_has_no_expiry_and_cached_private_images_need_no_token() {
    let (http, requests, server) = local_server(1, |_, mut stream| {
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
    fetchers.enqueue(photo.clone(), None, Purpose::Thumbnail);
    assert!(
        rx.recv_timeout(Duration::from_secs(10))
            .unwrap()
            .result
            .is_ok()
    );
    for _ in 0..3 {
        fetchers.enqueue(avatar(1), None, Purpose::Thumbnail);
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(10)).unwrap().result,
            Err(Failure::Http(404))
        ));
        fetchers
            .shared
            .failures
            .0
            .lock()
            .unwrap()
            .get_mut(&avatar(1).key())
            .unwrap()
            .at = Instant::now() - FAILURE_TTL - Duration::from_secs(1);
        fetchers.reconnected();
        assert!(!fetchers.retry(&avatar(1).key()));
    }
    requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(requests.try_recv().is_err());
    server.join().unwrap();
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
            assert!(lower.contains("x-line-application: chromeos\t9.8.7\tchrome_os\t"));
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
        requests.recv_timeout(Duration::from_secs(10)).unwrap();
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
            "media failure: UnsupportedHost"
        );
    }
    assert!(
        assets[3]
            .fetch(&http, Some("token\r\ninjected: yes"))
            .is_err()
    );
    assert!(assets[3].fetch(&http, None).is_err());
    for oid in ["", "../object", "a/b", "a?b", "a%2fb", "日本語"] {
        assert_eq!(
            Asset::photo(&json!({"id":"42","contentMetadata":{"OID":oid}}))
                .unwrap()
                .failure,
            Some(Failure::RejectedPath)
        );
        assert!(
            Asset::attachment(
                &json!({"id":"42","contentType":14,"contentMetadata":{"OID":oid,"ENC_KM":"key"}})
            )
            .unwrap()
            .failure
                == Some(Failure::RejectedPath)
        );
    }
    let (http, requests, server) = local_server(1, |_, mut stream| {
        stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: https://line-chrome-gw.line-apps.com/api/TalkService.do\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    });
    assert!(assets[3].fetch(&http, Some("synthetic")).is_err());
    requests.recv_timeout(Duration::from_secs(10)).unwrap();
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
            "media failure: TooLarge"
        );
        requests.recv_timeout(Duration::from_secs(10)).unwrap();
        server.join().unwrap();
    }
    assert!(pixels(&vec![0; MAX_DOWNLOAD + 1], 64).is_err());
    assert!(pixels(b"not an image", 64).is_err());
    let image = image::RgbaImage::new(8193, 1);
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, ImageFormat::Png).unwrap();
    assert!(pixels(bytes.get_ref(), 64).is_err());
}

#[test]
fn transient_retry_bypasses_failure_and_cached_pixels_and_reconnect_only_retries_once() {
    let (http, requests, server) = local_server(4, |_, mut stream| {
        stream
            .write_all(b"HTTP/1.1 500 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
    });
    let (_root, cache) = Cache::new();
    let (tx, rx) = mpsc::channel();
    let fetchers = Fetchers::start(
        http,
        cache.clone(),
        Arc::default(),
        Arc::default(),
        Failures::default(),
        |_, _| Ok(None),
        move |done, _| {
            tx.send(done).unwrap();
        },
    );
    let asset = avatar(1);
    fetchers.enqueue(asset.clone(), None, Purpose::Thumbnail);
    for step in 0..4 {
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(10)).unwrap().result,
            Err(Failure::Http(500))
        ));
        requests.recv_timeout(Duration::from_secs(10)).unwrap();
        match step {
            0 => fetchers.reconnected(),
            1 => {
                fetchers.reconnected();
                assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
                // A deliberate retry must ignore even a newly cached thumbnail.
                cache.lock().unwrap().store(&asset, &png()).unwrap();
                assert!(fetchers.retry(&asset.key()));
            }
            2 => {
                assert!(fetchers.retry(&asset.key()));
            }
            _ => {}
        }
    }
    fetchers.reconnected();
    assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
    server.join().unwrap();
}

#[test]
fn server_failures_keep_their_classes_through_completions() {
    for (response, expected) in [
        (
            "HTTP/1.1 404 Missing\r\nContent-Length: 0\r\n\r\n",
            Failure::Http(404),
        ),
        (
            "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n",
            Failure::Http(403),
        ),
        (
            "HTTP/1.1 500 Error\r\nContent-Length: 0\r\n\r\n",
            Failure::Http(500),
        ),
        (
            "HTTP/1.1 200 OK\r\nContent-Length: 16777217\r\n\r\n",
            Failure::TooLarge,
        ),
        (
            "HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nbad",
            Failure::Decode,
        ),
    ] {
        let (http, requests, server) = local_server(1, move |_, mut stream| {
            stream.write_all(response.as_bytes()).unwrap();
        });
        let (_root, cache) = Cache::new();
        let (tx, rx) = mpsc::channel();
        let fetchers = Fetchers::start(
            http,
            cache,
            Arc::default(),
            Arc::default(),
            Failures::default(),
            |_, _| Ok(None),
            move |done, _| {
                tx.send(done).unwrap();
            },
        );
        fetchers.enqueue(avatar(1), None, Purpose::Thumbnail);
        assert!(
            matches!(rx.recv_timeout(Duration::from_secs(10)).unwrap().result, Err(class) if class == expected)
        );
        requests.recv_timeout(Duration::from_secs(10)).unwrap();
        server.join().unwrap();
    }
}

#[test]
fn local_timeout_network_and_route_failures_are_typed() {
    for timeout in [false, true] {
        let gate = Arc::new(Gate::default());
        let release = gate.clone();
        let (http, requests, server) = local_server(1, move |_, _stream| {
            if timeout {
                release.wait();
            }
        });
        let http = http.asset_timeout(Duration::from_millis(200));
        let (_root, cache) = Cache::new();
        let (tx, rx) = mpsc::channel();
        let fetchers = Fetchers::start(
            http,
            cache,
            Arc::default(),
            Arc::default(),
            Failures::default(),
            |_, _| Ok(None),
            move |done, _| {
                tx.send(done).unwrap();
            },
        );
        fetchers.enqueue(avatar(1), None, Purpose::Thumbnail);
        let expected = if timeout {
            Failure::Timeout
        } else {
            Failure::Network
        };
        assert!(
            matches!(rx.recv_timeout(Duration::from_secs(10)).unwrap().result, Err(class) if class == expected)
        );
        requests.recv_timeout(Duration::from_secs(10)).unwrap();
        gate.release();
        server.join().unwrap();
    }
    let http = http::Transport::local("9.8.7", "http://127.0.0.1:1".into());
    for (meta, expected) in [
        (json!({"OID":"../no"}), Failure::RejectedPath),
        (
            json!({"DOWNLOAD_URL":"https://unsupported.invalid/image"}),
            Failure::UnsupportedHost,
        ),
    ] {
        let asset = if expected == Failure::UnsupportedHost {
            Asset::attachment(&json!({"id":"42", "contentType":14, "contentMetadata":meta}))
        } else {
            Asset::photo(&json!({"id":"42", "contentMetadata":meta}))
        }
        .unwrap();
        assert_eq!(
            Failure::from_error(&asset.fetch(&http, None).unwrap_err()),
            expected
        );
    }
}

#[test]
fn every_permanent_class_blocks_explicit_and_automatic_retries() {
    let (http, requests, server) = local_server(3, |headers, mut stream| {
        if headers.contains("test-0/") {
            stream
                .write_all(b"HTTP/1.1 404 Missing\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        } else if headers.contains("test-1/") {
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 16777217\r\n\r\n")
                .unwrap();
        } else {
            respond(stream, b"bad image");
        }
    });
    let (_root, cache) = Cache::new();
    let (tx, rx) = mpsc::channel();
    let fetchers = Fetchers::start(
        http,
        cache,
        Arc::default(),
        Arc::default(),
        Failures::default(),
        |_, _| Ok(None),
        move |done, _| {
            tx.send(done).unwrap();
        },
    );
    let mut assets: Vec<_> = (0..3).map(avatar).collect();
    assets.push(Asset::photo(&json!({"id":"42", "contentMetadata":{"OID":"../no"}})).unwrap());
    assets.push(
        Asset::attachment(
            &json!({"id":"43", "contentType":14, "contentMetadata":{"DOWNLOAD_URL":"https://unsupported.invalid"}}),
        )
        .unwrap(),
    );
    for asset in &assets {
        fetchers.enqueue(asset.clone(), None, Purpose::Thumbnail);
        assert!(
            matches!(rx.recv_timeout(Duration::from_secs(10)).unwrap().result, Err(class) if !class.retriable())
        );
        fetchers
            .shared
            .failures
            .0
            .lock()
            .unwrap()
            .get_mut(&asset.key())
            .unwrap()
            .at = Instant::now() - FAILURE_TTL - Duration::from_secs(1);
        fetchers.enqueue(asset.clone(), None, Purpose::Thumbnail);
        assert!(
            matches!(rx.recv_timeout(Duration::from_secs(10)).unwrap().result, Err(class) if !class.retriable())
        );
        assert!(!fetchers.retry(&asset.key()));
    }
    for _ in 0..3 {
        fetchers.reconnected();
    }
    assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
    for _ in 0..3 {
        requests.recv_timeout(Duration::from_secs(10)).unwrap();
    }
    assert!(requests.try_recv().is_err());
    server.join().unwrap();
}

#[test]
fn retained_failures_never_follow_credentials_to_another_account() {
    let failures = Failures::default();
    failures.scope("first-account");
    failures.0.lock().unwrap().insert(
        "image".into(),
        Failed {
            class: Failure::Network,
            at: Instant::now(),
            retried: false,
            job: Job {
                bypass: false,
                boost: false,
                asset: avatar(1),
                chat: None,
                purpose: Purpose::Thumbnail,
            },
        },
    );
    failures.scope("first-account");
    assert_eq!(failures.0.lock().unwrap().len(), 1);
    failures.scope("second-account");
    assert!(failures.0.lock().unwrap().is_empty());
}

#[test]
fn failure_records_evict_at_the_queue_bound() {
    let (_root, cache) = Cache::new();
    let (tx, rx) = mpsc::channel();
    let fetchers = Fetchers::start(
        http::Transport::local("9.8.7", "http://127.0.0.1:1".into()),
        cache,
        Arc::default(),
        Arc::default(),
        Failures::default(),
        |_, _| Ok(None),
        move |done, _| {
            tx.send(done).unwrap();
        },
    );
    let first = Asset::photo(&json!({"id":"0", "contentMetadata":{"OID":"../0"}}))
        .unwrap()
        .key();
    for id in 0..QUEUE_LIMIT + 10 {
        let asset = Asset::photo(
            &json!({"id":id.to_string(), "contentMetadata":{"OID":format!("../{id}")}}),
        )
        .unwrap();
        assert!(fetchers.enqueue(asset, None, Purpose::Thumbnail));
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(10)).unwrap().result,
            Err(Failure::RejectedPath)
        ));
        assert!(fetchers.shared.failures.0.lock().unwrap().len() <= QUEUE_LIMIT);
    }
    assert_eq!(
        fetchers.shared.failures.0.lock().unwrap().len(),
        QUEUE_LIMIT
    );
    assert!(fetchers.failure(&first).is_none());
}

#[test]
fn transient_expiry_allows_a_new_fetch_without_sync_reconnect() {
    let (http, requests, server) = local_server(12, |_, mut stream| {
        stream
            .write_all(b"HTTP/1.1 500 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
    });
    let (_root, cache) = Cache::new();
    let (tx, rx) = mpsc::channel();
    let fetchers = Fetchers::start(
        http,
        cache,
        Arc::default(),
        Arc::default(),
        Failures::default(),
        |_, _| Ok(None),
        move |done, _| {
            tx.send(done).unwrap();
        },
    );
    for (id, class) in [
        Failure::Timeout,
        Failure::Network,
        Failure::Http(500),
        Failure::Http(403),
        Failure::Cancelled,
        Failure::Http(429),
    ]
    .into_iter()
    .enumerate()
    {
        let asset = avatar(id);
        fetchers.enqueue(asset.clone(), None, Purpose::Thumbnail);
        assert!(
            rx.recv_timeout(Duration::from_secs(10))
                .unwrap()
                .result
                .is_err()
        );
        requests.recv_timeout(Duration::from_secs(10)).unwrap();
        {
            let mut failures = fetchers.shared.failures.0.lock().unwrap();
            let failed = failures.get_mut(&asset.key()).unwrap();
            failed.class = class;
            failed.at = Instant::now() - FAILURE_TTL + Duration::from_secs(1);
        }
        let at = fetchers.shared.failures.0.lock().unwrap()[&asset.key()].at;
        fetchers.enqueue(asset.clone(), None, Purpose::Thumbnail);
        assert!(
            matches!(rx.recv_timeout(Duration::from_secs(10)).unwrap().result, Err(found) if found == class)
        );
        assert_eq!(
            fetchers.shared.failures.0.lock().unwrap()[&asset.key()].at,
            at
        );
        assert!(requests.try_recv().is_err());
        fetchers
            .shared
            .failures
            .0
            .lock()
            .unwrap()
            .get_mut(&asset.key())
            .unwrap()
            .at = Instant::now() - FAILURE_TTL;
        fetchers.enqueue(asset, None, Purpose::Thumbnail);
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(10)).unwrap().result,
            Err(Failure::Http(500))
        ));
        requests.recv_timeout(Duration::from_secs(10)).unwrap();
    }
    server.join().unwrap();
}

#[test]
fn reconnect_backlog_keeps_normal_rank_and_visible_requests_can_evict_it() {
    let gates: Arc<Vec<Gate>> = Arc::new((0..QUEUE_LIMIT).map(|_| Gate::default()).collect());
    let release = gates.clone();
    let (http, requests, server) = local_server(THREADS + 1, move |headers, stream| {
        if !headers.starts_with("GET /test-9999/") {
            let id: usize = headers
                .split("test-")
                .nth(1)
                .unwrap()
                .split('/')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            release[id].wait();
        } else {
            respond(stream, &png());
        }
    });
    let (_root, cache) = Cache::new();
    let (tx, rx) = mpsc::channel();
    let fetchers = Fetchers::start(
        http,
        cache,
        Arc::default(),
        Arc::default(),
        Failures::default(),
        |_, _| Ok(None),
        move |done, _| {
            let _ = tx.send(done);
        },
    );
    for id in 0..QUEUE_LIMIT {
        let asset = avatar(id);
        fetchers.shared.failures.0.lock().unwrap().insert(
            asset.key(),
            Failed {
                class: Failure::Network,
                at: Instant::now(),
                retried: false,
                job: Job {
                    bypass: false,
                    boost: false,
                    asset,
                    chat: None,
                    purpose: Purpose::Thumbnail,
                },
            },
        );
    }
    fetchers.reconnected();
    let active: Vec<_> = (0..THREADS)
        .map(|_| requests.recv_timeout(Duration::from_secs(10)).unwrap())
        .collect();
    {
        let queue = fetchers.shared.queue.lock().unwrap();
        assert_eq!(queue.entries.len(), QUEUE_LIMIT);
        assert!(
            queue
                .entries
                .values()
                .filter_map(|entry| entry.job.as_ref())
                .all(|job| job.bypass && !job.boost)
        );
    }
    let visible = avatar(9999);
    let visible_key = visible.key();
    fetchers.shared.priority.set("open".into(), [visible.key()]);
    assert!(fetchers.enqueue(visible, Some("open".into()), Purpose::Thumbnail));
    // Release just one active slot, without allowing the other backlog to run.
    let first: usize = active[0]
        .split("test-")
        .nth(1)
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    gates[first].release();
    assert!(
        requests
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .starts_with("GET /test-9999/")
    );
    loop {
        let done = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        if done.key == visible_key {
            assert!(done.result.is_ok());
            break;
        }
    }
    drop(fetchers);
    drop(rx);
    for gate in gates.iter() {
        gate.release();
    }
    server.join().unwrap();
}

#[test]
fn photo_download_metadata_still_uses_obs_but_file_download_urls_are_refused() {
    let (http, requests, server) = local_server(2, |headers, stream| {
        assert!(headers.starts_with("GET /r/talk/m/object/preview "));
        respond(stream, &png());
    });
    for key in ["DOWNLOAD_URL", "DOWNLOAD_URL_RENEWAL"] {
        let meta = json!({"OID":"object", key:"https://unsupported.invalid/image"});
        let asset = Asset::photo(&json!({"id":"42", "contentMetadata":meta})).unwrap();
        assert!(asset.fetch(&http, Some("synthetic")).is_ok());
        let file = Asset::attachment(&json!({"id":"42", "contentType":14, "contentMetadata":meta}))
            .unwrap();
        assert_eq!(
            Failure::from_error(&file.fetch(&http, Some("synthetic")).unwrap_err()),
            Failure::UnsupportedHost
        );
        requests.recv_timeout(Duration::from_secs(10)).unwrap();
    }
    server.join().unwrap();
}

#[test]
fn refused_visible_enqueue_retains_a_job_for_explicit_retry() {
    let fetchers = Fetchers {
        shared: Arc::new(Shared {
            queue: Mutex::default(),
            ready: Condvar::new(),
            stop: Arc::default(),
            global_stop: Arc::default(),
            priority: Arc::default(),
            failures: Failures::default(),
        }),
        threads: Vec::new(),
    };
    for id in 0..QUEUE_LIMIT {
        assert!(fetchers.enqueue(avatar(id), None, Purpose::Original { request: id as u64 }));
    }
    let asset = avatar(9999);
    let key = asset.key();
    fetchers.shared.priority.set("open".into(), [key.clone()]);
    assert!(!fetchers.enqueue(asset, Some("open".into()), Purpose::Thumbnail));
    assert_eq!(fetchers.failure(&key), Some(Failure::Network));
    assert!(!fetchers.retry(&key));
    fetchers
        .shared
        .queue
        .lock()
        .unwrap()
        .entries
        .remove(&avatar(0).key());
    assert!(fetchers.retry(&key));
    let queue = fetchers.shared.queue.lock().unwrap();
    let job = queue.entries[&key].job.as_ref().unwrap();
    assert!(job.bypass && job.boost);
}

#[test]
fn official_image_urls_fetch_plain_bytes_without_line_headers() {
    let (http, requests, server) = local_server(2, |_, stream| respond(stream, b"plain image"));
    let asset = Asset::photo(&json!({"id":"99","contentMetadata":{
        "PREVIEW_URL":"https://manager.line-scdn.net/id/clearpreview",
        "DOWNLOAD_URL":"https://manager.line-scdn.net/id/original"}}))
    .unwrap();
    for asset in [asset.clone(), asset.original().unwrap()] {
        assert_eq!(
            asset.fetch(&http, Some("must-not-send")).unwrap(),
            b"plain image"
        );
        let request = requests
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .to_ascii_lowercase();
        assert!(
            !request.contains("x-line-")
                && !request.contains("x-talk-meta")
                && !request.contains("must-not-send")
        );
    }
    server.join().unwrap();
}
