use super::*;
use crate::client::http::tests::{accept, local_server};
use std::io::{BufRead, BufReader, Read};

pub(super) type Reply = Box<dyn FnOnce(Value) -> (u16, Value) + Send>;

pub(super) fn server_responses(
    steps: Vec<(&'static str, Reply)>,
) -> (http::Transport, std::thread::JoinHandle<()>) {
    local_server(move |listener| {
        for (method, reply) in steps {
            let mut socket = BufReader::new(accept(&listener));
            let mut line = String::new();
            socket.read_line(&mut line).unwrap();
            assert_eq!(
                line,
                format!("POST /api/talk/thrift/Talk/TalkService/{method} HTTP/1.1\r\n")
            );
            let mut length = 0;
            loop {
                line.clear();
                assert!(socket.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            socket.read_exact(&mut body).unwrap();
            let (status, data) = reply(serde_json::from_slice(&body).unwrap());
            let response = data.to_string();
            write!(socket.get_mut(), "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
        }
    })
}

pub(super) fn client(api: Api) -> (Client, PathBuf) {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "target/crypto-send-tests/{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let client = Client {
        api,
        stream_access: Arc::new(std::sync::RwLock::new("synthetic".into())),
        state: json!({"token_v3_issue_result":{"accessToken":"synthetic"}}),
        my_mid: "uself".into(),
        obs_access: None,
        sequence: 0,
        cache: Arc::new(std::sync::Mutex::new(
            ImageCache::new(&root, "uself").unwrap(),
        )),
        session_path: root.join("session.json"),
        cache_root: root.clone(),
    };
    (client, root)
}
