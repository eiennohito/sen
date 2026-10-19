use super::{tests::*, *};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    thread,
};
fn fixture() -> (Transport, TcpListener, Arc<ServerConfig>) {
    let ca = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(ca.cert.der().clone()).unwrap();
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(ca.signing_key.serialize_der()).into();
    let config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![ca.cert.der().clone()], key)
            .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let transport = Transport::configured(
        "test",
        format!(
            "https://localhost:{}",
            listener.local_addr().unwrap().port()
        ),
        None,
        roots,
    )
    .unwrap();
    (transport, listener, Arc::new(config))
}
#[test]
fn streaming_get_uses_verified_tls_directly_and_through_connect() {
    for (proxied, wrong_host) in [(false, false), (true, false), (false, true), (true, true)] {
        let (mut transport, listener, config) = fixture();
        let address = listener.local_addr().unwrap();
        if wrong_host {
            transport.base = format!("https://127.0.0.1:{}", address.port());
        }
        if proxied {
            transport.proxy = Some(
                Proxy::parse(
                    &format!("http://fake-user:fake-password@{address}"),
                    String::new(),
                )
                .unwrap(),
            );
        }
        let (received, wait) = std::sync::mpsc::channel();
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
                let mut authorized = false;
                loop {
                    line.clear();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    assert!(!line.contains("origin-token"));
                    authorized |= line.trim()
                        == "Proxy-Authorization: Basic ZmFrZS11c2VyOmZha2UtcGFzc3dvcmQ=";
                    if line == "\r\n" {
                        break;
                    }
                }
                assert!(authorized);
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
            reader.get_mut().write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 18\r\n\r\ndata: 1\n\n").unwrap();
            reader.get_mut().flush().unwrap();
            wait.recv_timeout(Duration::from_secs(2)).unwrap();
            reader.get_mut().write_all(b"data: 2\n\n").unwrap();
            reader.get_mut().flush().unwrap();
        });
        let result = transport.stream(
            "/api/test",
            &[("Cookie", "lct=origin-token")],
            Arc::new(CancelToken::new()),
            Duration::from_secs(2),
            |status, headers, body| {
                assert_eq!(status, 200);
                assert_eq!(headers["content-type"], "text/event-stream");
                let mut first = [0; 9];
                body.read_exact(&mut first)?;
                assert_eq!(&first, b"data: 1\n\n");
                received.send(()).unwrap();
                let mut bytes = Vec::new();
                body.read_to_end(&mut bytes)?;
                assert_eq!(bytes, b"data: 2\n\n");
                Ok(())
            },
        );
        if wrong_host {
            assert_eq!(
                result.unwrap_err().to_string(),
                "HTTPS transport failed; response not logged"
            );
        } else {
            result.unwrap();
        }
        server.join().unwrap();
    }
}

#[test]
fn no_proxy_bypasses_proxy() {
    let (mut transport, listener, config) = fixture();
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    proxy.set_nonblocking(true).unwrap();
    transport.proxy = Some(
        Proxy::parse(
            &format!("http://{}", proxy.local_addr().unwrap()),
            "localhost,.example.test".into(),
        )
        .unwrap(),
    );
    assert!(transport.proxy.as_ref().unwrap().bypass("sub.example.test"));
    assert!(!transport.proxy.as_ref().unwrap().bypass("badexample.test"));
    let server = thread::spawn(move || {
        let mut socket = BufReader::new(StreamOwned::new(
            ServerConnection::new(config).unwrap(),
            accept(&listener),
        ));
        let mut line = String::new();
        loop {
            line.clear();
            assert!(socket.read_line(&mut line).unwrap() > 0);
            if line == "\r\n" {
                break;
            }
        }
        socket
            .get_mut()
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
            .unwrap();
        socket.get_mut().flush().unwrap();
    });
    transport
        .request("/api/test", "{}", &[], Duration::from_secs(2))
        .unwrap();
    assert_eq!(
        proxy.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    server.join().unwrap();
}
