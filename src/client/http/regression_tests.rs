use super::{tests::*, *};
use std::{
    io::{BufReader, Write},
    sync::mpsc,
    thread,
};

#[test]
fn replaces_idle_connection_closed_by_server() {
    let (close, wait) = mpsc::channel();
    let (closed, observed) = mpsc::channel();
    let (transport, server) = local_server(move |listener| {
        let mut socket = BufReader::new(accept(&listener));
        assert!(read_request(&mut socket));
        socket
            .get_mut()
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
            .unwrap();
        wait.recv().unwrap();
        socket
            .get_ref()
            .shutdown(std::net::Shutdown::Write)
            .unwrap();
        assert_eq!(socket.read(&mut [0]).unwrap(), 0);
        drop(socket);
        closed.send(()).unwrap();
        let mut socket = BufReader::new(accept(&listener));
        assert!(read_request(&mut socket));
        socket
            .get_mut()
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
            .unwrap();
    });
    transport
        .request("/api/test", "{}", &[], Duration::from_secs(2))
        .unwrap();
    close.send(()).unwrap();
    observed.recv().unwrap();
    assert_eq!(
        transport
            .request("/api/test", "{}", &[], Duration::from_secs(2))
            .unwrap()
            .2,
        b"{}"
    );
    server.join().unwrap();
}
#[test]
fn cancellation_from_another_thread_unblocks_stalled_read() {
    let (ready, wait) = mpsc::channel();
    let (release, finish) = mpsc::channel();
    let (transport, server) = local_server(move |listener| {
        let mut socket = BufReader::new(accept(&listener));
        assert!(read_request(&mut socket));
        socket
            .get_mut()
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n")
            .unwrap();
        ready.send(()).unwrap();
        finish.recv().unwrap();
        match socket.read(&mut [0]) {
            Ok(0) => {}
            Err(e) if e.kind() == io::ErrorKind::ConnectionReset => {}
            other => panic!("connection was not closed: {other:?}"),
        }
    });
    let cancel = Arc::new(CancelToken::new());
    let signal = cancel.clone();
    let (done, result) = mpsc::channel();
    let caller = thread::spawn(move || {
        super::super::set_request_cancel(cancel);
        done.send(transport.request("/api/test", "{}", &[], Duration::from_secs(60)))
            .unwrap();
    });
    wait.recv_timeout(Duration::from_secs(2)).unwrap();
    signal.cancel();
    let error = result
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap_err();
    assert!(
        error
            .chain()
            .any(|e| e.downcast_ref::<super::super::error::Cancelled>().is_some())
    );
    release.send(()).unwrap();
    caller.join().unwrap();
    server.join().unwrap();
}
#[test]
fn connect_refused_is_not_delivered() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let transport = Transport::local("test", format!("http://{}", listener.local_addr().unwrap()));
    drop(listener);
    let error = transport
        .request("/api/test", "{}", &[], Duration::from_secs(2))
        .unwrap_err();
    assert_eq!(
        super::super::error::classify(&error),
        super::super::error::Failure::NotDelivered
    );
}
#[test]
fn stream_idle_timeout_fires() {
    let (release, finish) = mpsc::channel();
    let (transport, server) = local_server(move |listener| {
        let mut socket = BufReader::new(accept(&listener));
        request_headers(&mut socket).unwrap();
        socket
            .get_mut()
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n")
            .unwrap();
        finish.recv().unwrap();
    });
    transport
        .stream(
            "/api/test",
            &[],
            Arc::new(CancelToken::new()),
            Duration::from_millis(100),
            |_, _, reader| {
                assert_eq!(
                    reader.read(&mut [0]).unwrap_err().kind(),
                    io::ErrorKind::TimedOut
                );
                Ok(())
            },
        )
        .unwrap();
    release.send(()).unwrap();
    server.join().unwrap();
}
#[test]
fn body_cap_applies_without_content_length() {
    let (transport, server) = local_server(|listener| {
        let mut socket = BufReader::new(accept(&listener));
        assert!(read_request(&mut socket));
        socket
            .get_mut()
            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n100001\r\n")
            .unwrap();
        let _ = socket.get_mut().write_all(&vec![b'x'; MAX_BODY + 1]);
        let _ = socket.get_mut().write_all(b"\r\n0\r\n\r\n");
    });
    assert!(
        transport
            .request("/api/test", "{}", &[], Duration::from_secs(2))
            .unwrap_err()
            .to_string()
            .contains("exceeds")
    );
    server.join().unwrap();
}

#[test]
fn a_request_possibly_sent_is_never_retried() {
    let (transport, server) = local_server(|listener| {
        let mut socket = BufReader::new(accept(&listener));
        assert!(read_request(&mut socket));
        socket
            .get_mut()
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
            .unwrap();
        assert!(read_request(&mut socket));
        drop(socket);
        listener.set_nonblocking(true).unwrap();
        listener
    });
    transport
        .request("/api/test", "{}", &[], Duration::from_secs(2))
        .unwrap();
    let error = transport
        .request("/api/test", "{}", &[], Duration::from_secs(2))
        .unwrap_err();
    assert_eq!(
        super::super::error::classify(&error),
        super::super::error::Failure::Unanswered
    );
    let listener = server.join().unwrap();
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
}

#[test]
fn unsupported_all_proxy_and_http_targets_never_connect() {
    for scheme in ["socks5", "https", "unsupported"] {
        let target = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let proxy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        proxy.set_nonblocking(true).unwrap();
        let mut transport =
            Transport::local("test", format!("https://{}", target.local_addr().unwrap()));
        transport.proxy = Proxy::from_environment(|name| {
            (name == "ALL_PROXY")
                .then(|| format!("{scheme}://{}", proxy.local_addr().unwrap()).into())
        })
        .unwrap();
        let error = transport
            .request("/api/test", "{}", &[], Duration::from_secs(1))
            .unwrap_err();
        assert_eq!(
            super::super::error::classify(&error),
            super::super::error::Failure::NotDelivered
        );
        assert!(matches!(
            super::super::error::find_source::<super::super::error::TransportFailure>(&error)
                .unwrap()
                .source,
            TransportError::ProxyConfiguration
        ));
        assert_eq!(
            target.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(
            proxy.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }
    let target = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    target.set_nonblocking(true).unwrap();
    let mut transport =
        Transport::local("test", format!("http://{}", target.local_addr().unwrap()));
    transport.config.allow_http = false;
    assert_eq!(
        transport
            .request("/api/test", "{}", &[], Duration::from_secs(1))
            .unwrap_err()
            .to_string(),
        "HTTPS URL required"
    );
    assert_eq!(
        target.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
}

#[test]
fn proxy_environment_prefers_lowercase_and_https_then_all() {
    let variables = [
        ("https_proxy", "http://lower.test"),
        ("HTTPS_PROXY", "http://upper.test"),
        ("all_proxy", "http://all-lower.test"),
        ("ALL_PROXY", "http://all-upper.test"),
        ("no_proxy", "lower.test"),
        ("NO_PROXY", "upper.test"),
    ];
    for (skip, expected) in [
        (0, "lower.test"),
        (1, "upper.test"),
        (2, "all-lower.test"),
        (3, "all-upper.test"),
    ] {
        let proxy = Proxy::from_environment(|name| {
            variables[skip..]
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).into())
        })
        .unwrap()
        .unwrap();
        assert_eq!(proxy.url.host_str(), Some(expected));
        assert_eq!(proxy.exclusions, "lower.test");
    }
}

#[test]
fn precancelled_request_never_connects() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let transport = Transport::local("test", format!("http://{}", listener.local_addr().unwrap()));
    let cancel = Arc::new(CancelToken::default());
    cancel.cancel();
    super::super::set_request_cancel(cancel);
    let error = transport
        .request("/api/test", "{}", &[], Duration::from_secs(1))
        .unwrap_err();
    REQUEST_CANCEL.with(|slot| *slot.borrow_mut() = None);
    assert!(error.is::<super::super::error::Cancelled>());
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
}

#[test]
fn two_hundred_requests_reuse_one_connection() {
    let (transport, server) = local_server(|listener| {
        let mut socket = BufReader::new(accept(&listener));
        for _ in 0..200 {
            assert!(read_request(&mut socket));
            socket
                .get_mut()
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                .unwrap();
        }
        listener.set_nonblocking(true).unwrap();
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    });
    for _ in 0..200 {
        transport
            .request("/api/test", "{}", &[], Duration::from_secs(2))
            .unwrap();
    }
    server.join().unwrap();
}

#[test]
fn stalled_upload_hits_idle_deadline() {
    let (release, wait) = mpsc::channel();
    let (mut transport, server) = local_server(move |listener| {
        let mut socket = BufReader::new(accept(&listener));
        request_headers(&mut socket).unwrap().1.unwrap();
        wait.recv_timeout(Duration::from_secs(3)).unwrap();
    });
    transport.config.upload_idle = Duration::from_millis(100);
    let mut steps = Vec::new();
    let error = transport
        .upload(
            &format!("{}/upload", transport.base),
            &[],
            vec![7; 32 * 1024 * 1024].into(),
            &mut |sent| steps.push(sent),
        )
        .unwrap_err();
    assert!(matches!(
        super::super::error::find_source::<super::super::error::TransportFailure>(&error)
            .unwrap()
            .source,
        TransportError::Timeout
    ));
    assert!(!steps.is_empty());
    assert!(*steps.last().unwrap() < 32 * 1024 * 1024);
    release.send(()).unwrap();
    server.join().unwrap();
}

#[test]
fn asset_body_deadline_is_classified_as_timeout() {
    let (release, wait) = mpsc::channel();
    let (mut transport, server) = local_server(move |listener| {
        let mut socket = BufReader::new(accept(&listener));
        request_headers(&mut socket).unwrap();
        socket
            .get_mut()
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nx")
            .unwrap();
        wait.recv_timeout(Duration::from_secs(3)).unwrap();
    });
    transport.config.asset_hosts = vec![transport.base.trim_start_matches("http://").to_owned()];
    transport.config.asset_timeout = Duration::from_millis(100);
    let error = transport
        .asset(&format!("{}/image", transport.base), &[], 16 * 1024 * 1024)
        .unwrap_err();
    assert_eq!(
        super::super::error::media_failure(&error),
        super::super::desktop::assets::Failure::Timeout
    );
    release.send(()).unwrap();
    server.join().unwrap();
}

#[test]
fn upload_progress_precedes_body_completion() {
    let size = 32 * 1024 * 1024;
    let (first, observed) = mpsc::channel();
    let (release, rest) = mpsc::channel();
    let (transport, server) = local_server(move |listener| {
        let mut socket = BufReader::new(accept(&listener));
        assert_eq!(request_headers(&mut socket).unwrap().1.unwrap(), size);
        socket.read_exact(&mut [0; 65536]).unwrap();
        first.send(()).unwrap();
        rest.recv().unwrap();
        let mut remaining = vec![0; size - 65536];
        socket.read_exact(&mut remaining).unwrap();
        socket
            .get_mut()
            .write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\nX-Obs-Oid: object\r\n\r\n")
            .unwrap();
    });
    let (updates, progress) = mpsc::channel();
    let worker = thread::spawn(move || {
        transport.upload(
            &format!("{}/upload", transport.base),
            &[],
            vec![7; size].into(),
            &mut |n| updates.send(n).unwrap(),
        )
    });
    observed.recv_timeout(Duration::from_secs(5)).unwrap();
    let first = progress.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(first > 0 && first < size as u64);
    release.send(()).unwrap();
    assert_eq!(worker.join().unwrap().unwrap(), "object");
    let steps: Vec<_> = std::iter::once(first).chain(progress).collect();
    assert!(steps.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(steps.last(), Some(&(size as u64)));
    server.join().unwrap();
}
