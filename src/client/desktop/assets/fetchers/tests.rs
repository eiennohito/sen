use super::*;
use crate::client::http::test_support::{WAIT, asset_server as local_server};
use std::os::unix::fs::PermissionsExt;
use std::{
    io::Write,
    net::TcpStream,
    sync::{
        Condvar,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
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

fn avatar(base: &str, id: usize) -> Asset {
    let mut asset = Asset::avatar(&format!("/test-{id}")).unwrap();
    asset.url = format!("{base}/test-{id}/preview");
    asset
}
fn png() -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]))
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}
fn local_asset(base: &str, mut asset: Asset) -> Asset {
    let path = url::Url::parse(&asset.url).unwrap();
    asset.url = format!("{base}{}", &path[url::Position::BeforePath..]);
    if let Some(original) = &mut asset.original_url {
        let path = url::Url::parse(original).unwrap();
        *original = format!("{base}{}", &path[url::Position::BeforePath..]);
    }
    asset
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

#[derive(Default)]
struct Gate(Mutex<bool>, Condvar);
impl Gate {
    fn wait(&self) {
        let guard = self.0.lock().unwrap();
        let (guard, _) = self.1.wait_timeout_while(guard, WAIT, |v| !*v).unwrap();
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
    let (base, http, requests, server) = local_server(THREADS, move |_, stream| {
        release.wait();
        respond(stream, &png());
    });
    let (_root, cache) = Cache::new();
    let (results, images) = mpsc::channel();
    let fetchers = Fetchers::start(
        http,
        cache,
        Arc::default(),
        Failures::default(),
        |_, _| Ok(None),
        move |done, _| {
            results.send(done).unwrap();
        },
    );
    for i in 0..THREADS {
        assert!(fetchers.enqueue(avatar(&base, i), Purpose::Thumbnail));
    }
    for _ in 0..THREADS {
        requests.recv_timeout(WAIT).unwrap();
    }
    assert!(images.try_recv().is_err());
    gate.release();
    for _ in 0..THREADS {
        assert!(images.recv_timeout(WAIT).unwrap().result.is_ok());
    }
    drop(fetchers);
    server.join().unwrap();
}

#[test]
fn deduplicates_inflight_keys_and_cancellation_joins_stalled_fetchers_promptly() {
    let gate = Arc::new(Gate::default());
    let release = gate.clone();
    let (base, http, requests, server) = local_server(THREADS, move |_, _stream| {
        release.wait();
    });
    let (_root, cache) = Cache::new();
    let failures = Failures::default();
    let fetchers = Fetchers::start(
        http,
        cache,
        Arc::default(),
        failures.clone(),
        |_, _| Ok(None),
        |_, _| {},
    );
    for i in 0..THREADS {
        for _ in 0..20 {
            assert!(fetchers.enqueue(avatar(&base, i), Purpose::Thumbnail));
        }
    }
    for _ in 0..THREADS {
        requests.recv_timeout(WAIT).unwrap();
    }
    assert_eq!(fetchers.shared.queue.lock().unwrap().entries.len(), THREADS);
    for i in 100..100 + QUEUE_LIMIT + 20 {
        fetchers.enqueue(avatar(&base, i), Purpose::Thumbnail);
    }
    assert_eq!(
        fetchers.shared.queue.lock().unwrap().entries.len(),
        QUEUE_LIMIT
    );
    drop(fetchers);
    assert!(failures.entries.lock().unwrap().is_empty());
    gate.release();
    server.join().unwrap();
}

#[test]
fn permanent_failure_has_no_expiry_and_cached_private_images_need_no_token() {
    let (base, http, requests, server) = local_server(1, |_, mut stream| {
        stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
    });
    let (_root, cache) = Cache::new();
    let photo = local_asset(&base, Asset::photo(&json!({"id":"42"})).unwrap());
    cache.lock().unwrap().store(&photo, &png()).unwrap();
    let (tx, rx) = mpsc::channel();
    let fetchers = Fetchers::start(
        http,
        cache.clone(),
        Arc::default(),
        Failures::default(),
        |_, _| panic!("cache hit must not authorize"),
        move |done, _| {
            tx.send(done).unwrap();
        },
    );
    fetchers.enqueue(photo.clone(), Purpose::Thumbnail);
    let cached = rx.recv_timeout(WAIT).unwrap();
    assert!(cached.result.is_ok());
    assert!(
        !cached.network,
        "cached pixels must not count as a network response"
    );
    for _ in 0..3 {
        fetchers.enqueue(avatar(&base, 1), Purpose::Thumbnail);
        assert!(matches!(
            rx.recv_timeout(WAIT).unwrap().result,
            Err(Failure::Http(404))
        ));
        fetchers
            .shared
            .failures
            .entries
            .lock()
            .unwrap()
            .get_mut(&avatar(&base, 1).key())
            .unwrap()
            .at = Instant::now() - FAILURE_TTL - Duration::from_secs(1);
        assert!(!fetchers.retry(&avatar(&base, 1).key()));
    }
    requests.recv_timeout(WAIT).unwrap();
    assert!(requests.try_recv().is_err());
    server.join().unwrap();
    let mut cache = cache.lock().unwrap();
    assert_eq!(
        fs::metadata(cache.directory())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(cache.read(&photo).unwrap().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn shared_http_preserves_asset_hosts_paths_headers_and_no_redirects() {
    let (base, http, requests, server) = local_server(5, |headers, stream| {
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
        avatar(&base, 1),
        Asset::sticker(&json!({"STKID":"123"})).unwrap(),
        Asset::paid_reaction(&json!({"productId":"abc", "emojiId":"001"})).unwrap(),
        Asset::photo(&json!({"id":"42"})).unwrap(),
        Asset::attachment(&json!({"id":"43", "contentType":14})).unwrap(),
    ]
    .map(|asset| local_asset(&base, asset));
    for asset in &assets {
        assert_eq!(
            asset.fetch(&http, Some("synthetic")).unwrap(),
            b"synthetic body"
        );
    }
    for _ in 0..5 {
        requests.recv_timeout(WAIT).unwrap();
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
    let (base, http, requests, server) = local_server(1, |_, mut stream| {
        stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: https://line-chrome-gw.line-apps.com/api/TalkService.do\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    });
    assert!(
        local_asset(&base, assets[3].clone())
            .fetch(&http, Some("synthetic"))
            .is_err()
    );
    requests.recv_timeout(WAIT).unwrap();
    server.join().unwrap();
}

#[test]
fn download_and_decoder_caps_remain_enforced() {
    for chunked in [false, true] {
        let (base, http, requests, server) = local_server(1, move |_, mut stream| {
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
            avatar(&base, 1).fetch(&http, None).unwrap_err().to_string(),
            "media failure: TooLarge"
        );
        requests.recv_timeout(WAIT).unwrap();
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
fn transient_retry_bypasses_failure_and_cached_pixels() {
    let (base, http, requests, server) = local_server(4, |_, mut stream| {
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
        Failures::default(),
        |_, _| Ok(None),
        move |done, _| {
            tx.send(done).unwrap();
        },
    );
    let asset = avatar(&base, 1);
    fetchers.enqueue(asset.clone(), Purpose::Thumbnail);
    for step in 0..4 {
        assert!(matches!(
            rx.recv_timeout(WAIT).unwrap().result,
            Err(Failure::Http(500))
        ));
        requests.recv_timeout(WAIT).unwrap();
        if step < 3 {
            // A deliberate retry ignores even a newly cached thumbnail.
            cache.lock().unwrap().store(&asset, &png()).unwrap();
            assert!(fetchers.retry(&asset.key()));
        }
    }
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
        let (base, http, requests, server) = local_server(1, move |_, mut stream| {
            stream.write_all(response.as_bytes()).unwrap();
        });
        let (_root, cache) = Cache::new();
        let (tx, rx) = mpsc::channel();
        let fetchers = Fetchers::start(
            http,
            cache,
            Arc::default(),
            Failures::default(),
            |_, _| Ok(None),
            move |done, _| {
                tx.send(done).unwrap();
            },
        );
        fetchers.enqueue(avatar(&base, 1), Purpose::Thumbnail);
        assert!(matches!(rx.recv_timeout(WAIT).unwrap().result, Err(class) if class == expected));
        requests.recv_timeout(WAIT).unwrap();
        server.join().unwrap();
    }
}

#[test]
fn local_timeout_network_and_route_failures_are_typed() {
    for timeout in [false, true] {
        let gate = Arc::new(Gate::default());
        let release = gate.clone();
        let (base, http, requests, server) = local_server(1, move |_, _stream| {
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
            Failures::default(),
            |_, _| Ok(None),
            move |done, _| {
                tx.send(done).unwrap();
            },
        );
        fetchers.enqueue(avatar(&base, 1), Purpose::Thumbnail);
        let expected = if timeout {
            Failure::Timeout
        } else {
            Failure::Network
        };
        assert!(matches!(rx.recv_timeout(WAIT).unwrap().result, Err(class) if class == expected));
        requests.recv_timeout(WAIT).unwrap();
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
            media_failure(&asset.fetch(&http, None).unwrap_err()),
            expected
        );
    }
}

#[test]
fn every_permanent_class_blocks_explicit_retries() {
    let (base, http, requests, server) = local_server(3, |headers, mut stream| {
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
        Failures::default(),
        |_, _| Ok(None),
        move |done, _| {
            tx.send(done).unwrap();
        },
    );
    let mut assets: Vec<_> = (0..3).map(|id| avatar(&base, id)).collect();
    assets.push(Asset::photo(&json!({"id":"42", "contentMetadata":{"OID":"../no"}})).unwrap());
    assets.push(
        Asset::attachment(
            &json!({"id":"43", "contentType":14, "contentMetadata":{"DOWNLOAD_URL":"https://unsupported.invalid"}}),
        )
        .unwrap(),
    );
    for asset in &assets {
        fetchers.enqueue(asset.clone(), Purpose::Thumbnail);
        assert!(matches!(rx.recv_timeout(WAIT).unwrap().result, Err(class) if !class.retry_read()));
        fetchers
            .shared
            .failures
            .entries
            .lock()
            .unwrap()
            .get_mut(&asset.key())
            .unwrap()
            .at = Instant::now() - FAILURE_TTL - Duration::from_secs(1);
        fetchers.enqueue(asset.clone(), Purpose::Thumbnail);
        assert!(matches!(rx.recv_timeout(WAIT).unwrap().result, Err(class) if !class.retry_read()));
        assert!(!fetchers.retry(&asset.key()));
    }
    assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
    for _ in 0..3 {
        requests.recv_timeout(WAIT).unwrap();
    }
    assert!(requests.try_recv().is_err());
    server.join().unwrap();
}

#[test]
fn failure_records_evict_at_the_queue_bound() {
    let (_root, cache) = Cache::new();
    let (tx, rx) = mpsc::channel();
    let fetchers = Fetchers::start(
        http::Transport::local("9.8.7", "http://127.0.0.1:1".into()),
        cache,
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
        assert!(fetchers.enqueue(asset, Purpose::Thumbnail));
        assert!(matches!(
            rx.recv_timeout(WAIT).unwrap().result,
            Err(Failure::RejectedPath)
        ));
        assert!(fetchers.shared.failures.entries.lock().unwrap().len() <= QUEUE_LIMIT);
    }
    assert_eq!(
        fetchers.shared.failures.entries.lock().unwrap().len(),
        QUEUE_LIMIT
    );
    assert!(fetchers.failure(&first).is_none());
}

#[test]
fn transient_expiry_allows_a_new_fetch_without_sync_reconnect() {
    let (base, http, requests, server) = local_server(12, |_, mut stream| {
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
        let asset = avatar(&base, id);
        fetchers.enqueue(asset.clone(), Purpose::Thumbnail);
        assert!(rx.recv_timeout(WAIT).unwrap().result.is_err());
        requests.recv_timeout(WAIT).unwrap();
        {
            let mut failures = fetchers.shared.failures.entries.lock().unwrap();
            let failed = failures.get_mut(&asset.key()).unwrap();
            failed.class = class;
            failed.at = Instant::now() - FAILURE_TTL + Duration::from_secs(1);
        }
        let at = fetchers.shared.failures.entries.lock().unwrap()[&asset.key()].at;
        fetchers.enqueue(asset.clone(), Purpose::Thumbnail);
        assert!(matches!(rx.recv_timeout(WAIT).unwrap().result, Err(found) if found == class));
        assert_eq!(
            fetchers.shared.failures.entries.lock().unwrap()[&asset.key()].at,
            at
        );
        assert!(requests.try_recv().is_err());
        fetchers
            .shared
            .failures
            .entries
            .lock()
            .unwrap()
            .get_mut(&asset.key())
            .unwrap()
            .at = Instant::now() - FAILURE_TTL;
        fetchers.enqueue(asset, Purpose::Thumbnail);
        assert!(matches!(
            rx.recv_timeout(WAIT).unwrap().result,
            Err(Failure::Http(500))
        ));
        requests.recv_timeout(WAIT).unwrap();
    }
    server.join().unwrap();
}

#[test]
fn photo_download_metadata_still_uses_obs_but_file_download_urls_are_refused() {
    let (base, http, requests, server) = local_server(2, |headers, stream| {
        assert!(headers.starts_with("GET /r/talk/m/object/preview "));
        respond(stream, &png());
    });
    for key in ["DOWNLOAD_URL", "DOWNLOAD_URL_RENEWAL"] {
        let meta = json!({"OID":"object", key:"https://unsupported.invalid/image"});
        let asset = local_asset(
            &base,
            Asset::photo(&json!({"id":"42", "contentMetadata":meta})).unwrap(),
        );
        assert!(asset.fetch(&http, Some("synthetic")).is_ok());
        let file = Asset::attachment(&json!({"id":"42", "contentType":14, "contentMetadata":meta}))
            .unwrap();
        assert_eq!(
            media_failure(&file.fetch(&http, Some("synthetic")).unwrap_err()),
            Failure::UnsupportedHost
        );
        requests.recv_timeout(WAIT).unwrap();
    }
    server.join().unwrap();
}

#[test]
fn official_image_urls_fetch_plain_bytes_without_line_headers() {
    let (base, http, requests, server) =
        local_server(2, |_, stream| respond(stream, b"plain image"));
    let asset = Asset::photo(&json!({"id":"99","contentMetadata":{
        "PREVIEW_URL":"https://manager.line-scdn.net/id/clearpreview",
        "DOWNLOAD_URL":"https://manager.line-scdn.net/id/original"}}))
    .unwrap();
    for asset in [
        local_asset(&base, asset.clone()),
        local_asset(&base, asset.original().unwrap()),
    ] {
        assert_eq!(
            asset.fetch(&http, Some("must-not-send")).unwrap(),
            b"plain image"
        );
        let request = requests.recv_timeout(WAIT).unwrap().to_ascii_lowercase();
        assert!(
            !request.contains("x-line-")
                && !request.contains("x-talk-meta")
                && !request.contains("must-not-send")
        );
    }
    server.join().unwrap();
}

// Keep the queue paused so ordering and capacity checks do not depend on worker timing.
fn paused_fetchers() -> Fetchers {
    let (ready, wait) = async_channel::bounded(THREADS);
    Fetchers {
        shared: Arc::new(Shared {
            queue: Mutex::default(),
            ready,
            wait,
            stop: Arc::default(),
            global_stop: Arc::default(),
            failures: Failures::default(),
        }),
        threads: Vec::new(),
    }
}

#[test]
fn explicit_requests_first_then_newest_first() {
    let fetchers = paused_fetchers();
    let assets: Vec<_> = (0..5).map(|i| avatar("http://127.0.0.1", i)).collect();
    for (i, asset) in assets.iter().enumerate() {
        assert!(fetchers.enqueue(
            asset.clone(),
            if i < 2 {
                Purpose::Original
            } else {
                Purpose::Thumbnail
            }
        ));
    }
    // Retry thumbnails are explicit too, even though their purpose stays Thumbnail.
    assert!(fetchers.enqueue_job(Job {
        asset: assets[2].clone(),
        purpose: Purpose::Thumbnail,
        bypass: true
    }));
    let mut queue = fetchers.shared.queue.lock().unwrap();
    for i in [2, 1, 0, 4, 3] {
        let key = queue.next_key().expect("queued job disappeared");
        assert_eq!(
            key,
            assets[i].key(),
            "fetch order must be explicit-first, then newest-first"
        );
        queue.entries.remove(&key);
    }
    assert!(queue.next_key().is_none());
}

#[test]
fn eviction_publishes_completion_and_never_evicts_explicit_jobs() {
    let fetchers = paused_fetchers();
    let retry = avatar("http://127.0.0.1", 0);
    assert!(fetchers.enqueue_job(Job {
        asset: retry.clone(),
        purpose: Purpose::Thumbnail,
        bypass: true
    }));
    let original = avatar("http://127.0.0.1", 1);
    assert!(fetchers.enqueue(original.clone(), Purpose::Original));
    let evicted = avatar("http://127.0.0.1", 2);
    for i in 2..=QUEUE_LIMIT {
        assert!(fetchers.enqueue(avatar("http://127.0.0.1", i), Purpose::Thumbnail));
    }
    let completion = fetchers
        .shared
        .queue
        .lock()
        .unwrap()
        .evicted
        .pop_front()
        .expect("eviction did not queue a completion for the workers to publish");
    assert_eq!(
        completion.key,
        evicted.key(),
        "eviction must choose the oldest non-explicit thumbnail"
    );
    assert!(matches!(completion.result, Ok(Payload::Evicted)));
    assert!(!completion.network);
    let queue = fetchers.shared.queue.lock().unwrap();
    assert!(
        queue.entries.contains_key(&retry.key()),
        "Retry job was evicted"
    );
    assert!(
        queue.entries.contains_key(&original.key()),
        "explicit original job was evicted"
    );
    drop(queue);
    assert!(
        fetchers.enqueue(evicted, Purpose::Thumbnail),
        "evicted thumbnail could not be requested again"
    );
}

#[test]
fn refused_visible_enqueue_retains_a_job_for_explicit_retry() {
    let fetchers = paused_fetchers();
    for i in 0..QUEUE_LIMIT {
        assert!(fetchers.enqueue(avatar("http://127.0.0.1", i), Purpose::Original));
    }
    let asset = avatar("http://127.0.0.1", QUEUE_LIMIT);
    let key = asset.key();
    assert!(!fetchers.enqueue(asset, Purpose::Thumbnail));
    assert_eq!(fetchers.failure(&key), Some(Failure::Network));
    fetchers.shared.queue.lock().unwrap().entries.clear();
    assert!(
        fetchers.retry(&key),
        "refused visible request did not retain a retryable job"
    );
    let queue = fetchers.shared.queue.lock().unwrap();
    let job = queue.entries[&key].job.as_ref().unwrap();
    assert!(
        job.bypass,
        "explicit Retry did not bypass the recorded refusal"
    );
}

#[test]
fn workers_publish_evictions_after_inflight_fetches_finish() {
    let gate = Arc::new(Gate::default());
    let release = gate.clone();
    let (base, http, requests, server) = local_server(THREADS, move |_, stream| {
        release.wait();
        respond(stream, &png());
    });
    let (_root, cache) = Cache::new();
    let (tx, completions) = mpsc::channel();
    let fetchers = Fetchers::start(
        http,
        cache,
        Arc::default(),
        Failures::default(),
        |_, _| Ok(None),
        move |done, _| {
            tx.send(done).unwrap();
        },
    );
    for i in 0..THREADS {
        assert!(fetchers.enqueue(avatar(&base, i), Purpose::Thumbnail));
        requests
            .recv_timeout(WAIT)
            .expect("fetch worker did not reach the local server");
    }
    for i in THREADS..=QUEUE_LIMIT {
        assert!(fetchers.enqueue(avatar(&base, i), Purpose::Thumbnail));
    }
    gate.release();
    let deadline = Instant::now() + WAIT;
    let mut inflight: std::collections::HashSet<_> =
        (0..THREADS).map(|i| avatar(&base, i).key()).collect();
    let mut evicted = false;
    while !evicted || !inflight.is_empty() {
        let completion = completions
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("workers did not finish the inflight fetches and publish the eviction");
        if matches!(completion.result, Ok(Payload::Evicted)) {
            assert_eq!(completion.key, avatar(&base, THREADS).key());
            evicted = true;
        } else if inflight.remove(&completion.key) {
            assert!(matches!(completion.result, Ok(Payload::Image(_))));
        }
    }
    server.join().unwrap();
    drop(fetchers);
}

#[test]
fn idle_workers_wake_for_work_and_both_shutdown_signals() {
    for global in [false, true] {
        let (_root, cache) = Cache::new();
        let stop = Arc::new(CancelToken::new());
        let (published, completions) = mpsc::channel();
        let mut fetchers = Fetchers::start(
            http::Transport::local_assets("http://127.0.0.1".into()),
            cache,
            stop.clone(),
            Failures::default(),
            |_, _| Ok(None),
            move |completion, _| {
                published.send(completion).unwrap();
            },
        );
        let mut asset = avatar("http://127.0.0.1", 0);
        asset.failure = Some(Failure::RejectedPath);
        assert!(fetchers.enqueue(asset, Purpose::Thumbnail));
        assert!(matches!(
            completions.recv_timeout(WAIT).unwrap().result,
            Err(Failure::RejectedPath)
        ));
        let (finished, done) = mpsc::channel();
        let worker = thread::spawn(move || {
            if global {
                stop.cancel();
                for worker in fetchers.threads.drain(..) {
                    worker.join().unwrap();
                }
            }
            drop(fetchers);
            finished.send(()).unwrap();
        });
        done.recv_timeout(WAIT)
            .expect("idle asset workers did not stop");
        worker.join().unwrap();
    }
}

#[test]
fn unauthorized_download_renews_and_retries_only_once() {
    for second_status in [200, 401] {
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = count.clone();
        let (base, http, _requests, server) = local_server(2, move |headers, mut socket| {
            let n = seen.fetch_add(1, Ordering::SeqCst);
            let token = if n == 0 { "stale" } else { "fresh" };
            assert!(
                headers
                    .to_ascii_lowercase()
                    .contains(&format!("x-line-access: {token}\r\n"))
            );
            let status = if n == 0 { 401 } else { second_status };
            write!(
                socket,
                "HTTP/1.1 {status} Test\r\nContent-Length: 4\r\nConnection: close\r\n\r\nfile"
            )
            .unwrap();
        });
        let mut asset = avatar(&base, 1);
        asset.private = true;
        let job = Job {
            bypass: false,
            asset,
            purpose: Purpose::File {
                name: "file".into(),
            },
        };
        let (_root, cache) = Cache::new();
        let requests = std::cell::RefCell::new(Vec::new());
        let result = fetch(&http, &cache, &job, |renew| {
            requests.borrow_mut().push(renew);
            Ok(Some(if renew { "fresh" } else { "stale" }.into()))
        });
        assert_eq!(requests.into_inner(), [false, true]);
        if second_status == 200 {
            assert!(matches!(result.unwrap(), Payload::File(bytes) if *bytes == b"file"));
        } else {
            assert_eq!(
                result.err().unwrap().downcast_ref::<Failure>(),
                Some(&Failure::Http(401))
            );
        }
        server.join().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }
}
