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
        },
        wasm: vec![],
        state: json!({"token_v3_issue_result":{"accessToken":"synthetic"}}),
        my_mid: "uself".into(),
        account_name: "Self".into(),
        more_chats: false,
        contacts: HashMap::from([("uself".into(), json!({"displayName":"Self"}))]),
        members: HashMap::new(),
        member_contacts: HashMap::new(),
        chat_cursor: None,
        obs_access: None,
        sequence: 0,
        request_seq: 0,
        cache: Arc::new(std::sync::Mutex::new(
            ImageCache::new(&root, "uself").unwrap(),
        )),
        messages: messages::MessageCache::new(&root, &[7; 32]).unwrap(),
        detail_boxes: vec![],
        loaded_chats: vec![],
        live_ids: Default::default(),
    };
    (client, root)
}

fn chats(ids: Vec<String>) -> (&'static str, Reply) {
    (
        "getChats",
        Box::new(move |args| {
            assert_eq!(
                args,
                json!([{"chatMids":["group"],"withMembers":true,"withInvitees":false},0])
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
fn batches_at_100_reuses_contacts_and_members_on_reopen() {
    let (transport, worker) = server(vec![
        chats((0..201).map(|i| format!("u{i:04}")).collect()),
        contacts(100, ""),
        contacts(100, ""),
        contacts(1, ""),
    ]);
    let (mut client, root) = client(transport);
    let (members, failed) = client.members("group").unwrap();
    assert!(!failed);
    assert_eq!(members.len(), 202);
    assert_eq!(members.iter().filter(|m| m.is_self).count(), 1);
    assert!(
        members
            .iter()
            .filter(|m| !m.is_self)
            .all(|m| m.contact.avatar.is_some())
    );
    worker.join().unwrap();
    let (reopened, failed) = client.members("group").unwrap();
    assert!(!failed);
    assert_eq!(reopened.len(), members.len());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failures_are_not_cached_retry_resolves_and_absence_is_cached_without_poisoning_senders() {
    let fail = || {
        (
            "getContactsV2",
            Box::new(|_: Value| (503u16, Value::Null)) as Reply,
        )
    };
    let (transport, worker) = server(vec![
        chats(vec!["uknown".into(), "uabsent".into(), "ufailed".into()]),
        fail(),
        fail(),
        contacts(3, "uabsent"),
        contacts(1, "uabsent"),
    ]);
    let (mut client, root) = client(transport);
    let (members, failed) = client.members("group").unwrap();
    assert!(failed);
    assert_eq!(members.len(), 4);
    assert!(client.member_contacts.is_empty());
    let sender = |id: &str| {
        serde_json::from_value::<Message>(json!({"id":"1","sender_id":id,"body":"Hello","timestamp_ms":1,"outgoing":false,"image":null,"reactions":[],"encrypted":false,"decryption_failed":false})).unwrap()
    };
    let mut history = vec![sender("ufailed")];
    client.resolve_senders("group", &mut history);
    assert_eq!(history[0].sender_name, "Member · failed");
    assert!(client.member_contacts.is_empty());
    let (members, failed) = client.members("group").unwrap();
    assert!(!failed);
    assert_eq!(
        members
            .iter()
            .find(|m| m.contact.id == "ufailed")
            .unwrap()
            .contact
            .name,
        "ufailed"
    );
    assert!(client.member_contacts["uabsent"].is_null());
    let (reopened, failed) = client.members("group").unwrap();
    assert!(!failed);
    assert_eq!(
        reopened
            .iter()
            .filter(|m| m.contact.name == "Unknown member")
            .count(),
        1
    );
    let mut history = vec![sender("uabsent")];
    client.resolve_senders("group", &mut history);
    assert_eq!(history[0].sender_name, "Member · absent");
    worker.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn member_cache_eviction_preserves_primary_contacts() {
    let (transport, worker) = server(vec![contacts(1, "")]);
    let (mut client, root) = client(transport);
    client
        .contacts
        .insert("ufriend".into(), json!({"displayName":"Friend"}));
    client.member_contacts = (0..4096)
        .map(|i| (format!("uold{i}"), Value::Null))
        .collect();
    client.members.insert("group".into(), vec!["unew".into()]);
    assert!(!client.members("group").unwrap().1);
    assert_eq!(client.member_contacts.len(), 4096);
    assert_eq!(client.contacts["ufriend"]["displayName"], "Friend");
    worker.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}
