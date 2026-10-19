//! Parsed service events. Only known, complete events have displayable content.
use crate::store::Content;

#[derive(Clone, Copy, Debug)]
pub struct Mid<'a>(&'a str);
impl<'a> Mid<'a> {
    pub fn as_str(self) -> &'a str {
        self.0
    }
}

#[derive(Clone, Debug)]
pub enum Event<'a> {
    Rename(Mid<'a>, &'a str),
    Photo(Mid<'a>),
    Join(Mid<'a>),
    Leave(Mid<'a>),
    Invite(Mid<'a>, Vec<Mid<'a>>),
    Remove(Mid<'a>, Mid<'a>),
    Cancel(Mid<'a>, Mid<'a>),
    Link(Mid<'a>, bool),
    AlbumRename(&'a str, &'a str),
    AlbumDelete(&'a str),
    AlbumPhotos(&'a str),
    Limit(Vec<Mid<'a>>),
    Upgrade,
    LeaveChat(&'a str),
}
impl<'a> Event<'a> {
    pub fn mids(&self) -> Vec<Mid<'a>> {
        match self {
            Self::Rename(a, _)
            | Self::Photo(a)
            | Self::Join(a)
            | Self::Leave(a)
            | Self::Link(a, _) => vec![*a],
            Self::Invite(a, others) => std::iter::once(*a).chain(others.iter().copied()).collect(),
            Self::Remove(a, b) | Self::Cancel(a, b) => vec![*a, *b],
            Self::Limit(others) => others.clone(),
            _ => vec![],
        }
    }
    pub fn text(&self, sender: &str, name: impl Fn(&str) -> String) -> String {
        match self {
            Self::Rename(a, title) => format!("{} renamed the group to “{title}”", name(a.0)),
            Self::Photo(a) => format!("{} changed the group photo", name(a.0)),
            Self::Join(a) => format!("{} joined the group", name(a.0)),
            Self::Leave(a) => format!("{} left the group", name(a.0)),
            Self::Invite(a, others) => format!(
                "{} invited {}",
                name(a.0),
                others
                    .iter()
                    .map(|m| name(m.0))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Remove(a, b) => format!("{} removed {} from the group", name(a.0), name(b.0)),
            Self::Cancel(a, b) => {
                format!("{} canceled the invitation for {}", name(a.0), name(b.0))
            }
            Self::Link(a, enabled) => format!(
                "{} {} joining via link or QR code",
                name(a.0),
                if *enabled { "allowed" } else { "disabled" }
            ),
            Self::AlbumRename(a, b) => format!("{} renamed album “{a}” to “{b}”", name(sender)),
            Self::AlbumDelete(a) => format!("{} deleted album “{a}”", name(sender)),
            Self::AlbumPhotos(a) => format!("{} removed photos from “{a}”", name(sender)),
            Self::Limit(others) => format!(
                "{} cannot join any more chats",
                others
                    .iter()
                    .map(|m| name(m.0))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Upgrade => "This chat was upgraded to a group".into(),
            Self::LeaveChat(a) => format!("{a} left the chat"),
        }
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
    }
}
pub fn parse(content: &Content) -> Option<Event<'_>> {
    let Content::System { kind, params, .. } = content else {
        return None;
    };
    let kind = kind.as_deref()?;
    let groups = params
        .as_deref()
        .unwrap_or_default()
        .split('\u{1e}')
        .collect::<Vec<_>>();
    let arg = |i: usize| {
        groups
            .get(i)
            .and_then(|s: &&str| s.split('\u{1f}').next())
            .filter(|s| !s.is_empty())
    };
    let mid = |i| arg(i).map(Mid);
    let people = |i: usize| {
        groups
            .get(i)
            .map(|s| {
                s.split('\u{1f}')
                    .filter(|s| !s.is_empty())
                    .map(Mid)
                    .collect::<Vec<_>>()
            })
            .filter(|s| !s.is_empty())
    };
    Some(match kind {
        "C_PN" => Event::Rename(mid(0)?, arg(1)?),
        "C_PI" => Event::Photo(mid(0)?),
        "C_MJ" | "A_MJ" => Event::Join(mid(0)?),
        "C_ML" | "A_ML" => Event::Leave(mid(0)?),
        "C_GI" | "C_MI" | "A_MC" | "A_MI" => Event::Invite(mid(0)?, people(1)?),
        "C_MR" | "A_MR" => Event::Remove(mid(0)?, mid(1)?),
        "C_IC" | "A_IC" => Event::Cancel(mid(0)?, mid(1)?),
        "C_SP" | "C_SN" => Event::Link(mid(0)?, kind == "C_SP"),
        "BB" => Event::AlbumRename(arg(0)?, arg(1)?),
        "BD" => Event::AlbumDelete(arg(0)?),
        "BO" => Event::AlbumPhotos(arg(0)?),
        "C_OL" => Event::Limit(people(1)?),
        "C_BG" => Event::Upgrade,
        "pseudo_leave_chat" => Event::LeaveChat(arg(0)?),
        _ => {
            static UNKNOWN: std::sync::OnceLock<
                std::sync::Mutex<std::collections::BTreeSet<String>>,
            > = std::sync::OnceLock::new();
            if UNKNOWN
                .get_or_init(Default::default)
                .lock()
                .unwrap()
                .insert(kind.into())
            {
                log::debug!("Hidden chat event: {kind}");
            }
            return None;
        }
    })
}
pub fn displayable(content: &Content) -> bool {
    !matches!(content, Content::System { .. }) || parse(content).is_some()
}
