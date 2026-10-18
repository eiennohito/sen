//! Pure message attention decisions. Callers supply authoritative unread state.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewLevel {
    NameAndText,
    #[default]
    NameOnly,
    NewMessage,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub enabled: bool,
    pub preview: PreviewLevel,
    pub sound: bool,
    pub mention_exception: bool,
    pub group_events: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Context {
    pub active: bool,
    pub current: bool,
    pub at_latest: bool,
    pub exposed: bool,
    pub replay: bool,
    pub muted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Message,
    Mention,
    GroupEvent,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InApp {
    #[default]
    None,
    List,
    Latest,
    Inline,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Actions {
    pub notify: bool,
    /// Present the authoritative unread badge; never increment a local count.
    pub badge: bool,
    /// Unresolved outgoing failure indicator, separate from unread count.
    pub failure_marker: bool,
    pub mention_indicator: bool,
    pub attention: bool,
    /// Eligible for a daemon sound hint, still subject to the two-second gate.
    pub sound: bool,
    pub in_app: InApp,
}

pub fn decide(preferences: &Preferences, context: Context, event: EventKind) -> Actions {
    if context.replay {
        return Actions::default();
    }
    match event {
        EventKind::Message | EventKind::Mention | EventKind::GroupEvent => {
            let group = event == EventKind::GroupEvent;
            let muted =
                context.muted && !(event == EventKind::Mention && preferences.mention_exception);
            let notify = preferences.enabled
                && !context.active
                && !muted
                && (!group || preferences.group_events);
            Actions {
                notify,
                badge: !(context.active && context.current && context.exposed),
                mention_indicator: event == EventKind::Mention
                    && context.active
                    && !context.current,
                attention: notify,
                sound: notify && preferences.sound,
                in_app: if context.current {
                    if group {
                        InApp::Inline
                    } else if context.at_latest {
                        InApp::None
                    } else {
                        InApp::Latest
                    }
                } else {
                    InApp::List
                },
                ..Actions::default()
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationText {
    pub summary: String,
    pub body: String,
    pub avatar: bool,
}

pub fn redact(level: PreviewLevel, name: &str, text: &str) -> NotificationText {
    let name = if name.is_empty() { "LINE" } else { name };
    match level {
        PreviewLevel::NameAndText => NotificationText {
            summary: name.into(),
            body: text.into(),
            avatar: true,
        },
        PreviewLevel::NameOnly => NotificationText {
            summary: name.into(),
            body: String::new(),
            avatar: false,
        },
        PreviewLevel::NewMessage => NotificationText {
            summary: "New message".into(),
            body: String::new(),
            avatar: false,
        },
    }
}

/// Use a monotonic elapsed clock. Update the caller's per-chat timestamp only
/// after a sound is emitted, so a continuous burst can sound every two seconds.
pub fn sound_due(last_ms: Option<u64>, now_ms: u64) -> bool {
    last_ms.is_none_or(|last| now_ms.saturating_sub(last) >= 2_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn enabled() -> Preferences {
        Preferences {
            enabled: true,
            sound: true,
            group_events: true,
            ..Default::default()
        }
    }
    #[test]
    fn active_message_matrix_keeps_attention_local() {
        for (current, at_latest, exposed, expected, badge) in [
            (true, true, true, InApp::None, false),
            (true, false, false, InApp::Latest, true),
            (false, true, false, InApp::List, true),
            (true, true, false, InApp::None, true),
        ] {
            let result = decide(
                &enabled(),
                Context {
                    active: true,
                    current,
                    at_latest,
                    exposed,
                    ..Default::default()
                },
                EventKind::Message,
            );
            assert_eq!(result.in_app, expected);
            assert_eq!(result.badge, badge);
            assert!(!result.notify && !result.attention && !result.sound);
        }
    }
    #[test]
    fn inactive_messages_notify_unless_muted_or_disabled() {
        let result = decide(&enabled(), Context::default(), EventKind::Message);
        assert!(result.notify && result.badge && result.attention && result.sound);
        for event in [
            EventKind::Message,
            EventKind::Mention,
            EventKind::GroupEvent,
        ] {
            let result = decide(
                &enabled(),
                Context {
                    muted: true,
                    ..Default::default()
                },
                event,
            );
            assert!(result.badge);
            assert!(!result.notify && !result.attention && !result.sound);
        }
        let result = decide(
            &Preferences::default(),
            Context::default(),
            EventKind::Message,
        );
        assert!(result.badge);
        assert!(!result.notify && !result.attention && !result.sound);
    }
    #[test]
    fn mention_exception_and_group_events_are_explicit() {
        let prefs = Preferences {
            mention_exception: true,
            group_events: false,
            ..enabled()
        };
        let muted = Context {
            muted: true,
            ..Default::default()
        };
        assert!(decide(&prefs, muted, EventKind::Mention).notify);
        let active_mention = decide(
            &prefs,
            Context {
                active: true,
                ..Default::default()
            },
            EventKind::Mention,
        );
        assert!(active_mention.mention_indicator && !active_mention.notify);
        assert!(!decide(&prefs, muted, EventKind::Message).notify);
        assert!(!decide(&prefs, Context::default(), EventKind::GroupEvent).notify);
        assert_eq!(
            decide(
                &prefs,
                Context {
                    active: true,
                    current: true,
                    ..Default::default()
                },
                EventKind::GroupEvent
            )
            .in_app,
            InApp::Inline
        );
    }
    #[test]
    fn replay_never_alerts() {
        for event in [
            EventKind::Message,
            EventKind::Mention,
            EventKind::GroupEvent,
        ] {
            assert_eq!(
                decide(
                    &enabled(),
                    Context {
                        replay: true,
                        ..Default::default()
                    },
                    event
                ),
                Actions::default()
            );
        }
    }
    #[test]
    fn privacy_removes_text_and_avatar_together() {
        let full = redact(PreviewLevel::NameAndText, "葵", "Private text 🐈");
        assert_eq!(full.summary, "葵");
        assert_eq!(full.body, "Private text 🐈");
        assert!(full.avatar);
        let name = redact(PreviewLevel::NameOnly, "葵", "Private text 🐈");
        assert_eq!(name.summary, "葵");
        assert!(name.body.is_empty() && !name.avatar);
        let hidden = redact(PreviewLevel::NewMessage, "葵", "Private text 🐈");
        assert_eq!(hidden.summary, "New message");
        assert!(hidden.body.is_empty() && !hidden.avatar);
    }
    #[test]
    fn sound_burst_boundary_and_quiet_persisted_defaults() {
        assert!(sound_due(None, 0));
        assert!(!sound_due(Some(1_000), 2_999));
        assert!(sound_due(Some(1_000), 3_000));
        assert!(!sound_due(Some(1_000), 999));
        let prefs: Preferences = serde_json::from_str("{}").unwrap();
        assert!(!prefs.enabled && !prefs.sound && !prefs.mention_exception && !prefs.group_events);
        assert_eq!(prefs.preview, PreviewLevel::NameOnly);
        assert_eq!(
            serde_json::from_str::<Preferences>(&serde_json::to_string(&enabled()).unwrap())
                .unwrap(),
            enabled()
        );
    }
}
