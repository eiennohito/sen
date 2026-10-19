use super::*;
use crate::client::http::tests::{accept, local_server};
use std::io::{BufRead, BufReader, Read};

type Reply = Box<dyn FnOnce(Value) -> (u16, Value) + Send>;

fn server(steps: Vec<(&'static str, Reply)>) -> (http::Transport, std::thread::JoinHandle<()>) {
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
            let response = json!({"code":0,"data":data}).to_string();
            write!(socket.get_mut(), "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
        }
    })
}

fn client(transport: http::Transport) -> (Client, PathBuf) {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "target/p5-member-tests/{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let client = Client {
        api: Api {
            rt: Runtime::stub().unwrap(),
            http: transport,
            synthetic_signature: true,
            key_cache: HashMap::new(),
            channels: HashMap::new(),
            channel_tick: 0,
            state: json!({"token_v3_issue_result":{"accessToken":"synthetic"}}),
            session_path: Some(root.join("session.json")),
            stream_access: Arc::new(std::sync::RwLock::new("synthetic".into())),
        },
        wasm: vec![],
        my_mid: "uself".into(),
        obs_access: None,
        sequence: 0,
        cache: Arc::new(std::sync::Mutex::new(
            ImageCache::new(&root, "uself").unwrap(),
        )),
        cache_root: root.clone(),
    };
    (client, root)
}

fn chats(ids: Vec<String>) -> (&'static str, Reply) {
    (
        "getChats",
        Box::new(move |args| {
            assert_eq!(
                args,
                json!([{"chatMids":["group"],"withMembers":true,"withInvitees":true},1])
            );
            (200, json!({"chats":[{"chatMid":"group","memberMids":ids}]}))
        }),
    )
}

fn contacts(count: usize, absent: &'static str) -> (&'static str, Reply) {
    (
        "getContactsV2",
        Box::new(move |args| {
            let ids = args[0]["targetUserMids"].as_array().unwrap();
            assert_eq!(ids.len(), count);
            assert_eq!(args[0]["neededContactCalendarEvents"], json!([]));
            assert_eq!(args[1], 0);
            let entries: serde_json::Map<_, _> = ids
                .iter()
                .filter_map(|id| {
                    let id = id.as_str().unwrap();
                    (id != absent).then(|| {
                        (
                            id.to_owned(),
                            json!({"contact":{"displayName":id,"picturePath":"/member-avatar"}}),
                        )
                    })
                })
                .collect();
            (200, json!({"contacts":entries}))
        }),
    )
}

#[test]
fn batches_members_at_100() {
    let (transport, worker) = server(vec![
        chats((0..201).map(|i| format!("u{i:04}")).collect()),
        contacts(100, ""),
        contacts(100, ""),
        contacts(2, ""),
    ]);
    let (mut client, root) = client(transport);
    let members = client.members("group").unwrap();
    assert_eq!(members.len(), 202);
    assert_eq!(members.iter().filter(|m| m.is_self).count(), 1);
    assert!(
        members
            .iter()
            .filter(|m| !m.is_self)
            .all(|m| m.contact.avatar.is_some())
    );
    worker.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_member_lookup_reports_network_failure_and_can_be_retried() {
    let ids = vec!["uknown".into(), "uabsent".into(), "ufailed".into()];
    let (transport, worker) = server(vec![
        chats(ids.clone()),
        (
            "getContactsV2",
            Box::new(|_: Value| (503u16, Value::Null)) as Reply,
        ),
        chats(ids),
        contacts(4, "uabsent"),
    ]);
    let (mut client, root) = client(transport);
    let error = client.members("group").unwrap_err();
    assert_eq!(
        crate::client::error::classify(&error),
        crate::client::error::Failure::Unanswered
    );
    let members = client.members("group").unwrap();
    assert_eq!(
        members
            .iter()
            .find(|m| m.contact.id == "ufailed")
            .unwrap()
            .contact
            .name,
        "ufailed"
    );
    assert_eq!(
        members
            .iter()
            .filter(|m| m.contact.name == "Unknown member")
            .count(),
        1
    );
    worker.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}
