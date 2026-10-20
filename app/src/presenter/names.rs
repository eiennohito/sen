//! Name fallbacks are contextual; keep their wording in one place.
use line::{
    controller::AppState,
    store::{ChatId, Friend},
};
use std::borrow::Cow;

pub(crate) const YOU: &str = "You";
pub(crate) const CONVERSATION: &str = "Conversation";
pub(crate) const CONTACT: &str = "Contact";
pub(crate) const NOTIFICATION: &str = "閃";
const MEMBER: &str = "Unknown member";
const FRIEND: &str = "Unnamed friend";
const GROUP: &str = "Unnamed group";
const SENDER: &str = "Unknown sender";

pub(crate) fn nonblank(name: Option<&str>) -> Option<&str> {
    name.filter(|s| !s.trim().is_empty())
}

pub(crate) fn member_name(name: Option<&str>) -> &str {
    nonblank(name).unwrap_or(MEMBER)
}
pub(crate) fn friend_name(friend: &Friend) -> &str {
    if friend.group {
        group_name(friend.name.as_deref())
    } else {
        nonblank(friend.name.as_deref()).unwrap_or(FRIEND)
    }
}
pub(crate) fn group_name(name: Option<&str>) -> &str {
    nonblank(name).unwrap_or(GROUP)
}

pub(crate) fn person<'a>(state: &'a AppState, mid: &str) -> &'a str {
    if mid == state.profile.mid {
        return YOU;
    }
    let contact = state.contact(mid);
    if state.profile_friend(mid) {
        contact.map(friend_name).unwrap_or(FRIEND)
    } else {
        member_name(contact.and_then(|f| f.name.as_deref()))
    }
}

pub(crate) fn chat<'a>(state: &'a AppState, id: &ChatId, fallback: &'a str) -> &'a str {
    nonblank(state.chats.get(id).map(|c| c.chat.title.as_str()))
        .or_else(|| nonblank(state.contact(&id.0).and_then(|c| c.name.as_deref())))
        .unwrap_or(fallback)
}

pub(crate) fn sender<'a>(contact: Option<&'a Friend>, friend: bool, mid: &str) -> Cow<'a, str> {
    match contact {
        Some(s) if friend => Cow::Borrowed(friend_name(s)),
        Some(s) => Cow::Borrowed(member_name(s.name.as_deref())),
        None if mid.is_empty() => Cow::Borrowed(SENDER),
        None => {
            let start = mid.char_indices().rev().nth(5).map_or(0, |(i, _)| i);
            Cow::Owned(format!("Member · {}", &mid[start..]))
        }
    }
}
