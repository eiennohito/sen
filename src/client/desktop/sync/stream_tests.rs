use super::*;
use crate::client::http::test_support::{
    Clock, WAIT, accept, local_server, raw_request_headers, read_request,
};
use std::sync::mpsc;
use std::thread;
use std::{io, net::TcpStream};

fn request(reader: &mut BufReader<TcpStream>) {
    let request = raw_request_headers(reader).unwrap();
    let mut lines = request.split_inclusive('\n');
    assert_eq!(
        lines.next().unwrap(),
        "GET /api/operation/receive?localRev=12&version=9.8.7&language=en_US HTTP/1.1\r\n"
    );
    let headers: Vec<_> = lines.map(|line| line.trim().to_ascii_lowercase()).collect();
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
            write!(
                socket.get_mut(),
                "data: {{\"revision\":{revision}}}\r\n\r\n"
            )
            .unwrap();
            wait.recv_timeout(WAIT).unwrap();
        }
    });
    let mut seen = Vec::new();
    let status = stream(
        &transport,
        "9.8.7",
        "synthetic-token",
        12,
        &Arc::new(CancelToken::new()),
        PING_TIMEOUT,
        |name, data| {
            seen.push((name.to_owned(), data));
            if name != "open" {
                received.send(()).unwrap();
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
            match socket.read(&mut [0]) {
                Ok(n) => assert_eq!(n, 0),
                Err(error) => assert_eq!(error.kind(), io::ErrorKind::ConnectionReset),
            }
        });
        let stop = Arc::new(CancelToken::new());
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
        wait.recv_timeout(WAIT).unwrap();
        signal.cancel();
        let result = worker.join().unwrap();
        // Cancellation can also be observed by the stream loop between reads.
        match result {
            Ok(status) => assert_eq!(status, 200),
            Err(error) => assert!(
                crate::client::error::find_source::<crate::client::error::Cancelled>(&error)
                    .is_some()
            ),
        }
        server.join().unwrap();
    }
}

#[test]
fn eof_and_reset_return_to_reconnect_without_waiting_for_ping() {
    for (reset, truncated) in [(false, false), (true, false), (false, true), (true, true)] {
        let (close, wait) = mpsc::channel();
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
            wait.recv_timeout(WAIT).unwrap();
            if reset {
                socket2::SockRef::from(socket.get_ref())
                    .set_linger(Some(Duration::ZERO))
                    .unwrap();
            }
            drop(socket);
        });
        let result = stream(
            &transport,
            "9.8.7",
            "synthetic-token",
            12,
            &Arc::new(CancelToken::new()),
            PING_TIMEOUT,
            |name, _| {
                assert_eq!(name, "open");
                close.send(()).unwrap();
                Ok(true)
            },
        );
        if truncated || reset {
            let error = result.unwrap_err();
            assert_eq!(
                crate::client::error::classify(&error),
                crate::client::error::Failure::Unanswered
            );
            if truncated && !reset {
                assert_eq!(
                    error.downcast_ref::<io::Error>().unwrap().kind(),
                    io::ErrorKind::UnexpectedEof
                );
            }
        } else {
            assert_eq!(result.unwrap(), 200);
        }
        server.join().unwrap();
    }
}

#[test]
fn bad_frame_is_skipped_and_the_next_operation_arrives() {
    let (transport, server) = local_server(|listener| {
        let mut socket = BufReader::new(accept(&listener));
        request(&mut socket);
        headers(&mut socket);
        socket
            .get_mut()
            .write_all(b"data: broken JSON\n\ndata: {\"revision\":13}\n\n")
            .unwrap();
    });
    let mut operations = Vec::new();
    stream(
        &transport,
        "9.8.7",
        "synthetic-token",
        12,
        &Arc::new(CancelToken::new()),
        PING_TIMEOUT,
        |name, data| {
            if name == "message" {
                operations.push(data);
            }
            Ok(true)
        },
    )
    .unwrap();
    assert_eq!(operations, vec![json!({"revision":13})]);
    server.join().unwrap();
}

#[test]
fn statuses_keep_exact_classes_and_200_accepts_any_content_type() {
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
            &Arc::new(CancelToken::new()),
            PING_TIMEOUT,
            |name, _| {
                assert_eq!(status, 200);
                assert_eq!(name, "open");
                Ok(false)
            },
        );
        if status != 200 {
            let failure = crate::client::error::classify(&result.unwrap_err());
            assert_eq!(
                failure,
                crate::client::error::Failure::Refused(crate::client::error::Refusal::Server(
                    crate::client::error::ServerRefusal::Http(status)
                ))
            );
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
    let stop = Arc::new(CancelToken::new());
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
                stop.cancel();
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
            &Arc::new(CancelToken::new()),
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
fn decoded_ping_resets_watchdog_and_callbacks_do_not_spend_silence_budget() {
    for slow_event in ["open", "message"] {
        let (clock, waits) = Clock::new();
        let server_clock = clock.clone();
        let (callback_ready, callback) = mpsc::channel();
        let (resume, callback_resumed) = mpsc::channel();
        let (send_ping, ping) = mpsc::channel();
        let (sent, buffered) = mpsc::channel();
        let (mut transport, server) = local_server(move |listener| {
            let mut socket = BufReader::new(accept(&listener));
            request(&mut socket);
            headers(&mut socket);
            if slow_event == "message" {
                socket.get_mut().write_all(b"data: 1\n\n").unwrap();
            }
            ping.recv_timeout(WAIT).unwrap();
            socket.get_mut().write_all(b"event: ping\n\n").unwrap();
            sent.send(()).unwrap();
            assert_eq!(socket.read(&mut [0]).unwrap(), 0);
        });
        transport.clock = crate::client::http::Clock::Manual(clock.clone());
        let driver = thread::spawn(move || {
            callback.recv_timeout(WAIT).unwrap();
            // Ignore waits for the initial message, before the slow callback.
            while waits.try_recv().is_ok() {}
            resume.send(()).unwrap();
            // The callback has returned, and the reader is spending silence time.
            assert_eq!(waits.recv_timeout(WAIT).unwrap(), PING_TIMEOUT);
            server_clock.advance(Duration::from_secs(20));
            send_ping.send(()).unwrap();
            let remaining = waits.recv_timeout(WAIT).unwrap();
            server_clock.advance(PING_TIMEOUT);
            assert_eq!(remaining, PING_TIMEOUT);
        });
        let mut seen = Vec::new();
        let error = stream(
            &transport,
            "9.8.7",
            "synthetic-token",
            12,
            &Arc::new(CancelToken::new()),
            PING_TIMEOUT,
            |name, data| {
                seen.push(name.to_owned());
                if name == slow_event {
                    clock.advance(Duration::from_secs(100));
                    callback_ready.send(()).unwrap();
                    callback_resumed.recv_timeout(WAIT).unwrap();
                }
                if name == "ping" {
                    assert_eq!(data, Value::Null);
                    buffered.recv_timeout(WAIT).unwrap();
                    // A slow delivery callback must not consume the reset budget.
                    clock.advance(Duration::from_secs(100));
                }
                Ok(true)
            },
        )
        .unwrap_err();
        assert_eq!(
            seen,
            if slow_event == "open" {
                vec!["open", "ping"]
            } else {
                vec!["open", "message", "ping"]
            }
        );
        assert_eq!(
            error.downcast_ref::<io::Error>().unwrap().kind(),
            io::ErrorKind::TimedOut
        );
        driver.join().unwrap();
        server.join().unwrap();
    }
}

#[test]
fn silence_expires_despite_other_events_and_partial_line_trickle() {
    for body in [
        "",
        "event: message\ndata: null\n\n",
        "x",
        "data: {\"unfinished\":",
    ] {
        let (clock, waits) = Clock::new();
        let driver_clock = clock.clone();
        let (write_body, write) = mpsc::channel();
        let (mut transport, server) = local_server(move |listener| {
            let mut socket = BufReader::new(accept(&listener));
            request(&mut socket);
            headers(&mut socket);
            write.recv_timeout(WAIT).unwrap();
            socket.get_mut().write_all(body.as_bytes()).unwrap();
            assert_eq!(socket.read(&mut [0]).unwrap(), 0);
        });
        transport.clock = crate::client::http::Clock::Manual(clock.clone());
        let (reading, read_started) = mpsc::channel();
        let waits = Arc::new(std::sync::Mutex::new(waits));
        let driver_waits = waits.clone();
        let driver = thread::spawn(move || {
            read_started.recv_timeout(WAIT).unwrap();
            let waits = driver_waits.lock().unwrap();
            assert_eq!(waits.recv_timeout(WAIT).unwrap(), PING_TIMEOUT);
            driver_clock.advance(Duration::from_secs(20));
            write_body.send(()).unwrap();
            let remaining = if body.is_empty() {
                Duration::from_secs(10)
            } else {
                waits.recv_timeout(WAIT).unwrap()
            };
            driver_clock.advance(remaining);
            assert_eq!(remaining, Duration::from_secs(10));
        });
        let mut seen = Vec::new();
        let error = stream(
            &transport,
            "9.8.7",
            "synthetic-token",
            12,
            &Arc::new(CancelToken::new()),
            PING_TIMEOUT,
            |name, data| {
                seen.push(name.to_owned());
                if name == "open" {
                    clock.advance(Duration::from_secs(100));
                    while waits.lock().unwrap().try_recv().is_ok() {}
                    reading.send(()).unwrap();
                } else {
                    assert_eq!(name, "message");
                    assert_eq!(data, Value::Null);
                }
                Ok(true)
            },
        )
        .unwrap_err();
        assert_eq!(
            seen,
            if body.contains("event:") {
                vec!["open", "message"]
            } else {
                vec!["open"]
            }
        );
        assert_eq!(
            crate::client::error::classify(&error),
            crate::client::error::Failure::Unanswered
        );
        assert_eq!(
            error.downcast_ref::<io::Error>().unwrap().kind(),
            io::ErrorKind::TimedOut
        );
        driver.join().unwrap();
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
        transport.request("/api/test", "{}", &[], WAIT).unwrap(),
        (200, b"{}".to_vec())
    );
    for _ in 0..2 {
        assert_eq!(
            stream(
                &transport,
                "9.8.7",
                "synthetic-token",
                12,
                &Arc::new(CancelToken::new()),
                PING_TIMEOUT,
                |_, _| Ok(true),
            )
            .unwrap(),
            200
        );
    }
    drop(transport);
    server.join().unwrap();
}

#[test]
fn empty_close_waits_three_seconds_without_resync_and_delivers_next_stream() {
    let (transport, server) = local_server(|listener| {
        {
            let mut first = BufReader::new(accept(&listener));
            request(&mut first);
            headers(&mut first);
        }
        let mut second = BufReader::new(accept(&listener));
        request(&mut second);
        headers(&mut second);
        second
            .get_mut()
            .write_all(b"data: {\"revision\":13}\r\n\r\n")
            .unwrap();
    });
    let stop = Arc::new(CancelToken::new());
    let mut events = Vec::new();
    let mut waits = Vec::new();
    receive_on(
        &transport,
        "9.8.7",
        12,
        &stop,
        |_| Ok("synthetic-token".into()),
        |event| {
            if let Event::Resync { complete, .. } = &event {
                complete.send_blocking(Ok(())).unwrap();
            }
            assert!(
                !matches!(event, Event::Failure { .. }),
                "unexpected stream failure: {event:?}"
            );
            if matches!(event, Event::Operation(_)) {
                stop.cancel();
            }
            events.push(event);
            true
        },
        |_, delay| {
            assert!(
                waits.is_empty(),
                "stream did not resume after the empty close"
            );
            waits.push(delay);
        },
    )
    .unwrap();
    server.join().unwrap();
    assert_eq!(waits, [Duration::from_secs(3)]);
    assert!(
        matches!(events.as_slice(), [Event::State(SyncState::Connected), Event::State(SyncState::Reconnecting), Event::State(SyncState::Connected), Event::State(SyncState::Connected), Event::Operation(value)] if value.data["revision"] == 13)
    );
}

#[test]
fn repeated_empty_closes_back_off_to_cap_and_events_reset_delay() {
    // Empty streams hit the cap; a ping resets it; reconnect skips only its own wait.
    let bodies = [
        "",
        "",
        "",
        "",
        "",
        "",
        "event: ping\n\n",
        "",
        "event: reconnect\n\n",
        "",
    ];
    let (finished, done) = mpsc::channel();
    let (transport, server) = local_server(move |listener| {
        for body in bodies {
            let mut socket = BufReader::new(accept(&listener));
            request(&mut socket);
            headers(&mut socket);
            socket.get_mut().write_all(body.as_bytes()).unwrap();
        }
        finished.send(()).unwrap();
    });
    let stop = Arc::new(CancelToken::new());
    let waits = std::cell::RefCell::new(Vec::new());
    let mut opens_after_waits = Vec::new();
    receive_on(
        &transport,
        "9.8.7",
        12,
        &stop,
        |_| Ok("synthetic-token".into()),
        |event| {
            match event {
                Event::State(SyncState::Connected) => {
                    opens_after_waits.push(waits.borrow().len());
                }
                Event::State(SyncState::Reconnecting) => {}
                _ => panic!("unexpected event: {event:?}"),
            }
            true
        },
        |_, delay| {
            let mut waits = waits.borrow_mut();
            waits.push(delay.as_secs());
            if waits.len() == 9 {
                stop.cancel();
            }
        },
    )
    .unwrap();
    done.recv_timeout(WAIT).unwrap();
    server.join().unwrap();
    assert_eq!(*waits.borrow(), [3, 6, 12, 24, 30, 30, 3, 3, 3]);
    assert_eq!(opens_after_waits, [0, 1, 2, 3, 4, 5, 6, 6, 7, 8, 8]);
}

#[test]
fn repeated_stream_expiry_signs_out_after_one_refresh() -> Result<()> {
    let (transport, server) = local_server(move |listener| {
        for _ in 0..2 {
            let mut socket = BufReader::new(accept(&listener));
            request(&mut socket);
            headers(&mut socket);
            socket
                .get_mut()
                .write_all(b"event: talkException\ndata: {\"code\":119}\n\n")
                .unwrap();
        }
    });
    let mut authorizations = Vec::new();
    let mut failures = Vec::new();
    receive_on(
        &transport,
        "9.8.7",
        12,
        &Arc::new(CancelToken::new()),
        |refresh| {
            authorizations.push(refresh);
            Ok("synthetic-token".into())
        },
        |event| {
            if let Event::Failure { error } = event {
                failures.push(error);
            }
            true
        },
        |_, _| panic!("terminal authentication failure retried"),
    )?;
    server.join().unwrap();
    assert_eq!(authorizations, [false, true, false]);
    assert_eq!(
        failures,
        [crate::client::error::Failure::SignedOut(
            crate::client::error::SignOutReason::TokenRefresh
        )]
    );
    Ok(())
}
