use super::*;
use crate::client::http::tests::{accept, local_server, read_request};
use std::{io, net::TcpStream};

fn request(reader: &mut BufReader<TcpStream>) {
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert_eq!(
        line,
        "GET /api/operation/receive?localRev=12&version=9.8.7&language=en_US HTTP/1.1\r\n"
    );
    let mut headers = Vec::new();
    loop {
        line.clear();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        if line == "\r\n" {
            break;
        }
        headers.push(line.trim().to_ascii_lowercase());
    }
    assert!(headers.contains(&"cookie: lct=synthetic-token".into()));
    assert!(headers.contains(&"accept: text/event-stream".into()));
    assert!(!headers.iter().any(|header| header.starts_with("x-line-")));
}

fn headers(reader: &mut BufReader<TcpStream>) {
    reader
        .get_mut()
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nConnection: close\r\n\r\n")
        .unwrap();
}

#[test]
fn events_arrive_before_the_server_sends_the_next_line() {
    let (received, wait) = mpsc::channel();
    let (transport, server) = local_server(move |listener| {
        let mut socket = BufReader::new(accept(&listener));
        request(&mut socket);
        socket.get_mut().set_nodelay(true).unwrap();
        // Interim responses must not open the stream or hide the final status.
        socket
            .get_mut()
            .write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
            .unwrap();
        headers(&mut socket);
        for revision in [13, 14] {
            let sent = Instant::now();
            write!(
                socket.get_mut(),
                "data: {{\"revision\":{revision}}}\r\n\r\n"
            )
            .unwrap();
            let delivered = wait.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(
                delivered < sent + Duration::from_millis(90),
                "delivery took {:?}",
                delivered - sent
            );
        }
    });
    let mut seen = Vec::new();
    let status = stream(
        &transport,
        "9.8.7",
        "synthetic-token",
        12,
        &Arc::new(AtomicBool::new(false)),
        PING_TIMEOUT,
        |name, data| {
            seen.push((name.to_owned(), data));
            if name != "open" {
                received.send(Instant::now()).unwrap();
            }
            Ok(true)
        },
    )
    .unwrap();
    assert_eq!(status, 200);
    assert_eq!(
        seen,
        vec![
            ("open".into(), Value::Null),
            ("message".into(), json!({"revision":13})),
            ("message".into(), json!({"revision":14})),
        ]
    );
    server.join().unwrap();
}

#[test]
fn stop_interrupts_silent_headers_body_and_partial_line() {
    for body in [None, Some(""), Some("data: {\"unfinished\":")] {
        let (ready, wait) = mpsc::channel();
        let (transport, server) = local_server(move |listener| {
            let mut socket = BufReader::new(accept(&listener));
            request(&mut socket);
            if let Some(body) = body {
                headers(&mut socket);
                socket.get_mut().write_all(body.as_bytes()).unwrap();
            }
            ready.send(()).unwrap();
            assert_eq!(socket.read(&mut [0]).unwrap(), 0);
        });
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let worker = thread::spawn(move || {
            // Deliberately do not install a thread-local cancellation token.
            stream(
                &transport,
                "9.8.7",
                "synthetic-token",
                12,
                &stop,
                PING_TIMEOUT,
                |_, _| Ok(true),
            )
        });
        wait.recv_timeout(Duration::from_secs(2)).unwrap();
        thread::sleep(Duration::from_millis(20));
        let started = Instant::now();
        signal.store(true, Ordering::Relaxed);
        let result = worker.join().unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(result.unwrap_err().to_string(), "HTTPS request cancelled");
        server.join().unwrap();
    }
}

#[test]
fn eof_and_reset_return_to_reconnect_without_waiting_for_ping() {
    for (reset, truncated) in [(false, false), (true, false), (false, true), (true, true)] {
        let (close, wait) = mpsc::channel();
        let (closed, disconnected) = mpsc::channel();
        let (transport, server) = local_server(move |listener| {
            let mut socket = BufReader::new(accept(&listener));
            request(&mut socket);
            if truncated {
                socket
                    .get_mut()
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n")
                    .unwrap();
            } else {
                headers(&mut socket);
            }
            wait.recv_timeout(Duration::from_secs(2)).unwrap();
            if reset {
                socket2::SockRef::from(socket.get_ref())
                    .set_linger(Some(Duration::ZERO))
                    .unwrap();
            }
            drop(socket);
            closed.send(Instant::now()).unwrap();
        });
        let result = stream(
            &transport,
            "9.8.7",
            "synthetic-token",
            12,
            &Arc::new(AtomicBool::new(false)),
            PING_TIMEOUT,
            |name, _| {
                assert_eq!(name, "open");
                close.send(()).unwrap();
                Ok(true)
            },
        );
        let closed = disconnected.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(
            closed.elapsed() < Duration::from_millis(100),
            "{:?}",
            closed.elapsed()
        );
        if truncated {
            let error = result.unwrap_err();
            assert_eq!(
                error.downcast_ref::<io::Error>().unwrap().kind(),
                io::ErrorKind::UnexpectedEof
            );
        } else {
            // ureq normalizes resets on a close-delimited body into EOF.
            assert_eq!(result.unwrap(), 200);
        }
        server.join().unwrap();
    }
}

#[test]
fn rejects_an_oversize_unterminated_line_while_server_stays_open() {
    let (transport, server) = local_server(|listener| {
        let mut socket = BufReader::new(accept(&listener));
        request(&mut socket);
        headers(&mut socket);
        let block = [b'x'; 8192];
        for _ in 0..MAX_LINE / block.len() as u64 {
            socket.get_mut().write_all(&block).unwrap();
        }
        socket.get_mut().write_all(b"x").unwrap();
        assert_eq!(socket.read(&mut [0]).unwrap(), 0);
    });
    let error = stream(
        &transport,
        "9.8.7",
        "synthetic-token",
        12,
        &Arc::new(AtomicBool::new(false)),
        PING_TIMEOUT,
        |_, _| Ok(true),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "SSE line too large");
    server.join().unwrap();
}

#[test]
fn statuses_preserve_auth_mapping_and_any_content_type_is_accepted_on_200() {
    for (status, content_type) in [
        (401, "text/event-stream"),
        (403, "text/html"),
        (302, "text/event-stream"),
        (503, "text/event-stream"),
        (204, "text/event-stream"),
        (200, "text/plain"),
        (200, "text/event-streaming"),
        (200, ""),
        (200, "missing"),
    ] {
        let (transport, server) = local_server(move |listener| {
            let mut socket = BufReader::new(accept(&listener));
            request(&mut socket);
            let content_type = if content_type == "missing" {
                String::new()
            } else {
                format!("Content-Type: {content_type}\r\n")
            };
            write!(socket.get_mut(), "HTTP/1.1 {status} Test\r\n{content_type}Location: /api/stolen\r\nConnection: close\r\n\r\n").unwrap();
            assert_eq!(socket.read(&mut [0]).unwrap(), 0);
        });
        let result = stream(
            &transport,
            "9.8.7",
            "synthetic-token",
            12,
            &Arc::new(AtomicBool::new(false)),
            PING_TIMEOUT,
            |name, _| {
                assert_eq!(status, 200);
                assert_eq!(name, "open");
                Ok(false)
            },
        );
        // 401/403 remain statuses, so receive retains its refresh/expiry path.
        if status == 503 {
            assert!(matches!(
                result.unwrap_err().downcast_ref::<crate::client::Error>(),
                Some(crate::client::Error::Server(503))
            ));
        } else {
            assert_eq!(result.unwrap(), status);
        }
        server.join().unwrap();
    }
}

#[test]
fn multiline_frames_are_bounded_without_a_line_queue() {
    let mut decoder = Decoder::default();
    let line = format!("data: {}", "x".repeat(MAX_LINE as usize / 2));
    decoder.line(&line).unwrap();
    assert!(decoder.line(&line).is_err());
    assert!(decoder.data.len() <= MAX_LINE as usize);
}

#[test]
fn stop_is_checked_for_each_already_buffered_line() {
    let (transport, server) = local_server(|listener| {
        let mut socket = BufReader::new(accept(&listener));
        request(&mut socket);
        headers(&mut socket);
        socket
            .get_mut()
            .write_all(b"data: 1\n\ndata: 2\n\n")
            .unwrap();
    });
    let stop = Arc::new(AtomicBool::new(false));
    let mut messages = Vec::new();
    stream(
        &transport,
        "9.8.7",
        "synthetic-token",
        12,
        &stop,
        PING_TIMEOUT,
        |name, value| {
            if name == "message" {
                messages.push(value);
                stop.store(true, Ordering::Relaxed);
            }
            Ok(true)
        },
    )
    .unwrap();
    assert_eq!(messages, vec![json!(1)]);
    server.join().unwrap();
}

#[test]
fn chunked_stream_accepts_lines_at_the_limit_and_exceeds_the_rpc_body_cap() {
    let (transport, server) = local_server(|listener| {
        let mut socket = BufReader::new(accept(&listener));
        request(&mut socket);
        socket.get_mut().write_all(b"HTTP/1.1 200 OK\r\nContent-Type: TEXT/EVENT-STREAM\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap();
        let comment = format!(":{}\n", "x".repeat(MAX_LINE as usize - 2));
        for chunk in [comment.as_bytes(), comment.as_bytes(), b"data: 1\n\n"] {
            write!(socket.get_mut(), "{:x}\r\n", chunk.len()).unwrap();
            socket.get_mut().write_all(chunk).unwrap();
            socket.get_mut().write_all(b"\r\n").unwrap();
        }
        socket.get_mut().write_all(b"0\r\n\r\n").unwrap();
    });
    let mut messages = Vec::new();
    assert_eq!(
        stream(
            &transport,
            "9.8.7",
            "synthetic-token",
            12,
            &Arc::new(AtomicBool::new(false)),
            PING_TIMEOUT,
            |name, value| {
                if name == "message" {
                    messages.push(value);
                }
                Ok(true)
            }
        )
        .unwrap(),
        200
    );
    assert_eq!(messages, vec![json!(1)]);
    server.join().unwrap();
}

#[test]
fn slow_open_and_delivery_callbacks_do_not_expire_buffered_pings() {
    for slow_event in ["open", "message"] {
        let (blocked, wait) = mpsc::channel();
        let (transport, server) = local_server(move |listener| {
            let mut socket = BufReader::new(accept(&listener));
            request(&mut socket);
            headers(&mut socket);
            if slow_event == "message" {
                socket.get_mut().write_all(b"data: 1\n\n").unwrap();
            }
            wait.recv_timeout(Duration::from_secs(2)).unwrap();
            thread::sleep(Duration::from_millis(30));
            socket.get_mut().write_all(b"event: ping\n\n").unwrap();
            assert_eq!(socket.read(&mut [0]).unwrap(), 0);
        });
        let mut seen = Vec::new();
        let result = stream(
            &transport,
            "9.8.7",
            "synthetic-token",
            12,
            &Arc::new(AtomicBool::new(false)),
            Duration::from_millis(150),
            |name, _| {
                seen.push(name.to_owned());
                if name == slow_event {
                    blocked.send(()).unwrap();
                    thread::sleep(Duration::from_millis(250));
                }
                Ok(name != "ping")
            },
        );
        assert_eq!(result.unwrap(), 200);
        assert_eq!(seen.last().unwrap(), "ping");
        server.join().unwrap();
    }
}

#[test]
fn silence_budget_starts_after_a_slow_callback_returns() {
    let (transport, server) = local_server(|listener| {
        let mut socket = BufReader::new(accept(&listener));
        request(&mut socket);
        headers(&mut socket);
        assert_eq!(socket.read(&mut [0]).unwrap(), 0);
    });
    let mut resumed = None;
    let error = stream(
        &transport,
        "9.8.7",
        "synthetic-token",
        12,
        &Arc::new(AtomicBool::new(false)),
        Duration::from_millis(150),
        |name, _| {
            assert_eq!(name, "open");
            thread::sleep(Duration::from_millis(250));
            resumed = Some(Instant::now());
            Ok(true)
        },
    )
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::TimedOut
    );
    let elapsed = resumed.unwrap().elapsed();
    assert!(elapsed >= Duration::from_millis(150), "{elapsed:?}");
    assert!(elapsed < Duration::from_millis(1200), "{elapsed:?}");
    server.join().unwrap();
}

#[test]
fn decoded_ping_resets_the_watchdog_other_events_do_not_and_silence_expires() {
    for name in ["ping", "message"] {
        let (opened, wait) = mpsc::channel();
        let (transport, server) = local_server(move |listener| {
            let mut socket = BufReader::new(accept(&listener));
            request(&mut socket);
            headers(&mut socket);
            wait.recv_timeout(Duration::from_secs(2)).unwrap();
            thread::sleep(Duration::from_millis(150));
            write!(socket.get_mut(), "event: {name}\ndata: null\n\n").unwrap();
            assert_eq!(socket.read(&mut [0]).unwrap(), 0);
        });
        let mut delivered = None;
        let error = stream(
            &transport,
            "9.8.7",
            "synthetic-token",
            12,
            &Arc::new(AtomicBool::new(false)),
            Duration::from_millis(300),
            |event, data| {
                if event == "open" {
                    opened.send(()).unwrap();
                } else {
                    assert_eq!(event, name);
                    assert_eq!(data, Value::Null);
                    delivered = Some(Instant::now());
                }
                Ok(true)
            },
        )
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<io::Error>().unwrap().kind(),
            io::ErrorKind::TimedOut
        );
        let elapsed = delivered.unwrap().elapsed();
        if name == "ping" {
            assert!(elapsed >= Duration::from_millis(300), "{elapsed:?}");
            assert!(elapsed < Duration::from_millis(1200), "{elapsed:?}");
        } else {
            assert!(elapsed < Duration::from_millis(275), "{elapsed:?}");
        }
        server.join().unwrap();
    }
}

#[test]
fn each_stream_attempt_uses_a_fresh_connection_and_closes_it_on_return() {
    let (transport, server) = local_server(|listener| {
        let mut warm = BufReader::new(accept(&listener));
        assert!(read_request(&mut warm));
        warm.get_mut()
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
            .unwrap();
        assert_eq!(warm.read(&mut [0]).unwrap(), 0);
        for _ in 0..2 {
            let mut socket = BufReader::new(accept(&listener));
            request(&mut socket);
            socket
                .get_mut()
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\ndata: 1\n\n")
                .unwrap();
            assert_eq!(socket.read(&mut [0]).unwrap(), 0);
        }
    });
    assert_eq!(
        transport
            .request("/api/test", "{}", &[], Duration::from_millis(400))
            .unwrap(),
        (200, b"{}".to_vec())
    );
    for _ in 0..2 {
        assert_eq!(
            stream(
                &transport,
                "9.8.7",
                "synthetic-token",
                12,
                &Arc::new(AtomicBool::new(false)),
                PING_TIMEOUT,
                |_, _| Ok(true),
            )
            .unwrap(),
            200
        );
    }
    server.join().unwrap();
}

#[test]
fn partial_line_trickle_expires_without_waiting_for_an_empty_poll() {
    let (transport, server) = local_server(|listener| {
        let mut socket = BufReader::new(accept(&listener));
        request(&mut socket);
        socket.get_mut().set_nodelay(true).unwrap();
        headers(&mut socket);
        for _ in 0..50 {
            thread::sleep(Duration::from_millis(40));
            if socket.get_mut().write_all(b"x").is_err() {
                break;
            }
        }
    });
    let started = Instant::now();
    let error = stream(
        &transport,
        "9.8.7",
        "synthetic-token",
        12,
        &Arc::new(AtomicBool::new(false)),
        Duration::from_millis(300),
        |name, _| {
            assert_eq!(name, "open");
            Ok(true)
        },
    )
    .unwrap_err();
    let elapsed = started.elapsed();
    assert_eq!(error.to_string(), "SSE ping expired");
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::TimedOut
    );
    assert!(elapsed >= Duration::from_millis(300), "{elapsed:?}");
    assert!(elapsed < Duration::from_millis(800), "{elapsed:?}");
    server.join().unwrap();
}
