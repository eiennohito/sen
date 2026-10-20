use super::*;
use crate::client::http::test_support::{accept, local_server, raw_request_headers};
use std::io::{BufReader, Write};

// Minimal sfnt: three emoji map to one PNG bitmap. No downloaded fixture.
fn font() -> Vec<u8> {
    let mut head = vec![0; 54];
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    let mut hhea = vec![0; 36];
    hhea[34..36].copy_from_slice(&1u16.to_be_bytes());
    let mut cmap = vec![0, 0, 0, 1, 0, 3, 0, 10, 0, 0, 0, 12];
    cmap.extend_from_slice(&[0, 12, 0, 0]);
    cmap.extend_from_slice(&52u32.to_be_bytes());
    cmap.extend_from_slice(&0u32.to_be_bytes());
    cmap.extend_from_slice(&3u32.to_be_bytes());
    for ch in [0x2764u32, 0x1f338, 0x1f600] {
        for value in [ch, ch, 1] {
            cmap.extend_from_slice(&value.to_be_bytes());
        }
    }
    let name_text: Vec<_> = "Test Emoji"
        .encode_utf16()
        .flat_map(u16::to_be_bytes)
        .collect();
    let mut name = vec![0, 0, 0, 1, 0, 18, 0, 3, 0, 1, 4, 9, 0, 1];
    name.extend_from_slice(&(name_text.len() as u16).to_be_bytes());
    name.extend_from_slice(&[0, 0]);
    name.extend(name_text);
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        1,
        1,
        image::Rgba([255, 0, 0, 255]),
    ))
    .write_to(&mut png, image::ImageFormat::Png)
    .unwrap();
    let png = png.into_inner();
    let mut cbdt = vec![0, 3, 0, 0, 1, 1, 0, 1, 1];
    cbdt.extend_from_slice(&(png.len() as u32).to_be_bytes());
    cbdt.extend(png);
    let mut cblc = vec![0; 80];
    cblc[..8].copy_from_slice(&[0, 3, 0, 0, 0, 0, 0, 1]);
    for (offset, value) in [(8, 56u32), (12, 24), (16, 1)] {
        cblc[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }
    cblc[48..64].copy_from_slice(&[0, 1, 0, 1, 1, 1, 32, 1, 0, 1, 0, 1, 0, 0, 0, 8]);
    cblc[64..72].copy_from_slice(&[0, 1, 0, 17, 0, 0, 0, 4]);
    cblc[76..80].copy_from_slice(&((cbdt.len() - 4) as u32).to_be_bytes());
    let mut tables = vec![
        (*b"CBDT", cbdt),
        (*b"CBLC", cblc),
        (*b"cmap", cmap),
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"hmtx", vec![0, 1, 0, 0]),
        (*b"maxp", vec![0, 0, 0x50, 0, 0, 2]),
        (*b"name", name),
    ];
    tables.sort_by_key(|(tag, _)| *tag);
    let mut bytes = vec![0, 1, 0, 0];
    bytes.extend_from_slice(&(tables.len() as u16).to_be_bytes());
    bytes.extend_from_slice(&[0; 6]);
    let mut offset = 12 + tables.len() * 16;
    for (tag, data) in &tables {
        bytes.extend_from_slice(tag);
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&(offset as u32).to_be_bytes());
        bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in tables {
        bytes.extend(data);
        bytes.resize(bytes.len().next_multiple_of(4), 0);
    }
    bytes
}

fn server(
    replies: Vec<(u16, String, Vec<u8>)>,
) -> (Transport, std::thread::JoinHandle<Vec<String>>) {
    local_server(move |listener| {
        replies
            .into_iter()
            .map(|(status, headers, body)| {
                let mut socket = BufReader::new(accept(&listener));
                let request = raw_request_headers(&mut socket).unwrap();
                write!(
                    socket.get_mut(),
                    "HTTP/1.1 {status} Test\r\nConnection: close\r\n{headers}\r\n"
                )
                .unwrap();
                socket.get_mut().write_all(&body).unwrap();
                request
            })
            .collect()
    })
}
fn reply(body: Vec<u8>) -> (u16, String, Vec<u8>) {
    (
        200,
        format!(
            "Content-Length: {}\r\nETag: \"current\"\r\nLast-Modified: Wed, 21 Oct 2026 00:00:00 GMT\r\n",
            body.len()
        ),
        body,
    )
}

#[test]
fn absent_font_is_downloaded_validated_and_registered_then_checked_monthly() {
    let directory = crate::test_support::TempDir::new("emoji-download");
    let data = font();
    let mut updated = data.clone();
    let head_record = updated.windows(4).position(|tag| tag == b"head").unwrap();
    let head_offset = u32::from_be_bytes(
        updated[head_record + 8..head_record + 12]
            .try_into()
            .unwrap(),
    ) as usize;
    updated[head_offset + 4..head_offset + 8].copy_from_slice(&2u32.to_be_bytes());
    let (transport, server) = server(vec![
        reply(data.clone()),
        reply(b"SIL OPEN FONT LICENSE Version 1.1".to_vec()),
        (304, "Content-Length: 0\r\n".into(), vec![]),
        reply(updated.clone()),
    ]);
    let downloaded = update(&directory.0, &transport, MONTH).unwrap().unwrap();
    assert_eq!(downloaded, data);
    validate(&downloaded).unwrap(); // Includes successful fontique registration.
    assert!(directory.0.join("OFL.txt").is_file());
    assert!(
        update(&directory.0, &transport, MONTH + 1)
            .unwrap()
            .is_none()
    );
    assert!(
        update(&directory.0, &transport, MONTH * 2)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        std::fs::read(directory.0.join("NotoColorEmoji.ttf")).unwrap(),
        data
    );
    assert_eq!(
        update(&directory.0, &transport, MONTH * 3)
            .unwrap()
            .unwrap(),
        updated
    );
    assert_eq!(
        std::fs::read(directory.0.join("NotoColorEmoji.ttf")).unwrap(),
        updated
    );
    let requests = server.join().unwrap();
    assert!(requests[2].contains("if-none-match: \"current\""));
    assert!(requests[2].contains("if-modified-since:"));
}

#[test]
fn invalid_downloads_are_discarded_and_existing_font_survives() {
    let mut outline = font();
    // An sfnt with a COLR table in place of CBDT is still a parseable font.
    outline[12..16].copy_from_slice(b"COLR");
    for (index, data) in [b"not a font".to_vec(), outline].into_iter().enumerate() {
        let directory = crate::test_support::TempDir::new("emoji-invalid");
        if index == 1 {
            std::fs::write(directory.0.join("NotoColorEmoji.ttf"), font()).unwrap();
        }
        std::fs::write(directory.0.join("OFL.txt"), b"SIL OPEN FONT LICENSE").unwrap();
        let (transport, server) = server(vec![reply(data)]);
        assert!(update(&directory.0, &transport, MONTH).is_err());
        server.join().unwrap();
        let path = directory.0.join("NotoColorEmoji.ttf");
        if index == 0 {
            assert!(!path.exists());
        } else {
            assert_eq!(std::fs::read(path).unwrap(), font());
        }
        assert!(
            std::fs::read_dir(&directory.0).unwrap().all(|e| !e
                .unwrap()
                .path()
                .extension()
                .is_some_and(|e| e == "tmp"))
        );
    }
}

#[test]
fn oversized_responses_and_unlisted_hosts_are_refused() {
    let (transport, server) = server(vec![reply(vec![0; 65])]);
    for url in [
        "http://cdn.jsdelivr.net/font",
        "https://evil.example/font",
        "https://cdn.jsdelivr.net.evil.example/font",
        "https://cdn.jsdelivr.net@evil.example/font",
    ] {
        assert!(transport.emoji_font(url, &[], CAP).is_err());
    }
    assert!(transport.emoji_font(SOURCE, &[], 64).is_err());
    assert_eq!(server.join().unwrap().len(), 1);
}

#[test]
fn startup_download_runs_off_the_calling_ui_thread() {
    let directory = crate::test_support::TempDir::new("emoji-background");
    let (transport, server) = server(vec![
        reply(font()),
        reply(b"SIL OPEN FONT LICENSE".to_vec()),
    ]);
    let ui_thread = std::thread::current().id();
    let (entered, started) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let (registered, ready) = std::sync::mpsc::channel();
    let worker = spawn(
        directory.0.clone(),
        move || {
            entered.send(std::thread::current().id()).unwrap();
            wait.recv_timeout(crate::client::http::test_support::WAIT)
                .unwrap();
            Ok(transport)
        },
        move |data| {
            registered.send(data).unwrap();
        },
    )
    .unwrap();
    let worker_thread = started
        .recv_timeout(crate::client::http::test_support::WAIT)
        .unwrap();
    // Release before asserting, so a mutation cannot strand the HTTP watchdog.
    release.send(()).unwrap();
    assert_eq!(
        ready
            .recv_timeout(crate::client::http::test_support::WAIT)
            .unwrap(),
        font()
    );
    worker.join().unwrap();
    server.join().unwrap();
    assert_ne!(worker_thread, ui_thread);
}
