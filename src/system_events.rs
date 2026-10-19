//! Parsed service events. Only known, complete events have displayable content.
use crate::store::Content;

#[derive(Clone, Debug)]
pub enum Event<'a> {
    Rename(&'a str, &'a str),
    Photo(&'a str),
    Join(&'a str),
    Leave(&'a str),
    Invite(&'a str, Vec<&'a str>),
    Remove(&'a str, &'a str),
    Cancel(&'a str, &'a str),
    Link(&'a str, bool),
    AlbumRename(&'a str, &'a str),
    AlbumDelete(&'a str),
    AlbumPhotos(&'a str),
    Limit(Vec<&'a str>),
    Upgrade,
    LeaveChat(&'a str),
}
impl<'a> Event<'a> {
    pub fn mids(&self) -> Vec<&'a str> {
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
    let people = |i: usize| {
        groups
            .get(i)
            .map(|s| {
                s.split('\u{1f}')
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .filter(|s| !s.is_empty())
    };
    Some(match kind {
        "C_PN" => Event::Rename(arg(0)?, arg(1)?),
        "C_PI" => Event::Photo(arg(0)?),
        "C_MJ" | "A_MJ" => Event::Join(arg(0)?),
        "C_ML" | "A_ML" => Event::Leave(arg(0)?),
        "C_GI" | "C_MI" | "A_MC" | "A_MI" => Event::Invite(arg(0)?, people(1)?),
        "C_MR" | "A_MR" => Event::Remove(arg(0)?, arg(1)?),
        "C_IC" | "A_IC" => Event::Cancel(arg(0)?, arg(1)?),
        "C_SP" | "C_SN" => Event::Link(arg(0)?, kind == "C_SP"),
        "BB" => Event::AlbumRename(arg(0)?, arg(1)?),
        "BD" => Event::AlbumDelete(arg(0)?),
        "BO" => Event::AlbumPhotos(arg(0)?),
        "C_OL" => Event::Limit(people(1)?),
        "C_BG" => Event::Upgrade,
        "pseudo_leave_chat" => Event::LeaveChat(arg(0)?),
        _ => return None,
    })
}
pub fn displayable(content: &Content) -> bool {
    !matches!(content, Content::System { .. }) || parse(content).is_some()
}
