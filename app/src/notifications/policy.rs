//! Pure message attention decisions. Callers supply authoritative unread state.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PreviewLevel {
    NameAndText,
    #[default]
    NameOnly,
    NewMessage,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
    pub muted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Message,
    Mention,
    GroupEvent,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Actions {
    pub notify: bool,
    pub sound: bool,
}

pub fn decide(preferences: &Preferences, context: Context, event: EventKind) -> Actions {
    let muted = context.muted && !(event == EventKind::Mention && preferences.mention_exception);
    let notify = preferences.enabled
        && !context.active
        && !muted
        && (event != EventKind::GroupEvent || preferences.group_events);
    Actions {
        notify,
        sound: notify && preferences.sound,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationText {
    pub summary: String,
    pub body: String,
}

pub fn redact(level: PreviewLevel, name: &str, text: &str) -> NotificationText {
    let name = if name.is_empty() { "閃" } else { name };
    match level {
        PreviewLevel::NameAndText => NotificationText {
            summary: name.into(),
            body: {
                let mut chars = text.chars();
                let mut body: String = chars.by_ref().take(1000).collect();
                if chars.next().is_some() {
                    body.pop();
                    body.push('…');
                }
                body
            },
        },
        PreviewLevel::NameOnly => NotificationText {
            summary: name.into(),
            body: String::new(),
        },
        PreviewLevel::NewMessage => NotificationText {
            summary: "New message".into(),
            body: String::new(),
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
    fn notification_and_sound_respect_activity_mutes_and_preferences() {
        let prefs = enabled();
        assert_eq!(
            decide(&prefs, Context::default(), EventKind::Message),
            Actions {
                notify: true,
                sound: true
            }
        );
        assert!(
            !decide(
                &prefs,
                Context {
                    active: true,
                    muted: false
                },
                EventKind::Message
            )
            .notify
        );
        let muted = Context {
            muted: true,
            active: false,
        };
        assert!(!decide(&prefs, muted, EventKind::Mention).notify);
        let prefs = Preferences {
            mention_exception: true,
            group_events: false,
            sound: false,
            ..prefs
        };
        assert_eq!(
            decide(&prefs, muted, EventKind::Mention),
            Actions {
                notify: true,
                sound: false
            }
        );
        assert!(!decide(&prefs, Context::default(), EventKind::GroupEvent).notify);
        assert!(
            !decide(
                &Preferences::default(),
                Context::default(),
                EventKind::Message
            )
            .notify
        );
    }
    #[test]
    fn privacy_removes_text() {
        let full = redact(PreviewLevel::NameAndText, "葵", "Private text 🐈");
        assert_eq!(full.summary, "葵");
        assert_eq!(full.body, "Private text 🐈");
        let name = redact(PreviewLevel::NameOnly, "葵", "Private text 🐈");
        assert_eq!(name.summary, "葵");
        assert!(name.body.is_empty());
        let hidden = redact(PreviewLevel::NewMessage, "葵", "Private text 🐈");
        assert_eq!(hidden.summary, "New message");
        assert!(hidden.body.is_empty());
    }
    #[test]
    fn body_is_bounded_by_characters_including_ellipsis() {
        for length in [0, 999, 1000, 1001, 2000] {
            let input = "🐈".repeat(length);
            let result = redact(PreviewLevel::NameAndText, "Name", &input);
            assert_eq!(result.body.chars().count(), length.min(1000));
            if length > 1000 {
                assert_eq!(result.body, format!("{}…", "🐈".repeat(999)));
            } else {
                assert_eq!(result.body, input);
            }
        }
    }
    #[test]
    fn sound_burst_boundary_and_quiet_persisted_defaults() {
        assert!(sound_due(None, 0));
        assert!(!sound_due(Some(1_000), 2_999));
        assert!(sound_due(Some(1_000), 3_000));
        assert!(!sound_due(Some(1_000), 999));
        let prefs = Preferences::default();
        assert!(!prefs.enabled && !prefs.sound && !prefs.mention_exception && !prefs.group_events);
        assert_eq!(prefs.preview, PreviewLevel::NameOnly);
    }
}
