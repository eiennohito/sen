use super::test_client::{Reply, client};
use super::*;

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
    let (mut client, root, worker) = client(vec![
        chats((0..201).map(|i| format!("u{i:04}")).collect()),
        contacts(100, ""),
        contacts(100, ""),
        contacts(2, ""),
    ]);
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
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_member_lookup_can_be_retried_without_hiding_members() {
    let ids = vec!["uknown".into(), "uabsent".into(), "ufailed".into()];
    let (mut client, root, worker) = client(vec![
        chats(ids.clone()),
        (
            "getContactsV2",
            Box::new(|_: Value| (503u16, Value::Null)) as Reply,
        ),
        chats(ids),
        contacts(4, "uabsent"),
    ]);
    let (members, failed) = client.members("group").unwrap();
    assert!(failed);
    assert_eq!(members.len(), 4);
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
