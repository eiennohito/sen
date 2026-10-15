use super::*;
pub(super) struct Conversation {
    pub title: &'static str,
    pub subtitle: &'static str,
    pub preview: &'static str,
    pub initial: &'static str,
    pub messages: Vec<MessageRow>,
}

fn message(body: &str, time: &str, outgoing: bool) -> MessageRow {
    MessageRow {
        body: body.into(),
        time: time.into(),
        outgoing,
        ..Default::default()
    }
}

pub(super) fn examples() -> Vec<Conversation> {
    vec![
        Conversation {
            title: "葵",
            subtitle: "Example conversation",
            preview: "また明日、駅の前で。",
            initial: "葵",
            messages: vec![
                message("明日は何時に待ち合わせる？", "10:41", false),
                message("11時でどう？ コーヒーも飲もう。", "10:42", true),
                message("いいね！\nまた明日、駅の前で。", "10:43", false),
            ],
        },
        Conversation {
            title: "Weekend plans",
            subtitle: "Example group · 3 members",
            preview: "The riverside route looks good.",
            initial: "W",
            messages: vec![
                message("Shall we take a walk on Saturday?", "09:18", false),
                message("The riverside route looks good.", "09:20", true),
            ],
        },
        Conversation {
            title: "Notes to self",
            subtitle: "Example conversation",
            preview: "A quiet place for a thought.",
            initial: "N",
            messages: vec![message("A quiet place for a thought.", "Yesterday", true)],
        },
    ]
}
