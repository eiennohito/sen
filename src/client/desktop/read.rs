//! Monotonic, coalesced exposure markers. Eligibility belongs to the UI.
use super::*;
#[derive(Default)]
pub struct Markers {
    entries: HashMap<String, Entry>,
}
struct Entry {
    pending: Option<String>,
    confirmed: Option<String>,
    next: Instant,
}
impl Markers {
    pub fn expose(&mut self, chat: String, message: String, now: Instant) {
        if message.is_empty() {
            return;
        }
        // Bound unattended state independently of the number of conversations.
        if !self.entries.contains_key(&chat) && self.entries.len() >= 256 {
            let oldest = self
                .entries
                .iter()
                .filter(|(_, e)| e.pending.is_none())
                .min_by_key(|(_, e)| e.next)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                self.entries.remove(&id);
            } else {
                return;
            }
        }
        let entry = self.entries.entry(chat).or_insert(Entry {
            pending: None,
            confirmed: None,
            next: now,
        });
        if entry
            .confirmed
            .as_deref()
            .is_some_and(|id| message_id_cmp(id, &message).is_ge())
            || entry
                .pending
                .as_deref()
                .is_some_and(|id| message_id_cmp(id, &message).is_ge())
        {
            return;
        }
        entry.pending = Some(message);
    }
    pub fn wait(&self, now: Instant) -> Option<Duration> {
        self.entries
            .values()
            .filter(|e| e.pending.is_some())
            .map(|e| e.next.saturating_duration_since(now))
            .min()
    }
    pub fn due(&mut self, now: Instant) -> Option<(String, String)> {
        let (chat, entry) = self
            .entries
            .iter_mut()
            .find(|(_, e)| e.pending.is_some() && e.next <= now)?;
        entry.next = now + Duration::from_secs(1);
        Some((chat.clone(), entry.pending.clone().unwrap()))
    }
    pub fn cancel(&mut self, chat: &str) {
        if let Some(entry) = self.entries.get_mut(chat) {
            entry.pending = None;
        }
    }
    pub fn confirmed(&mut self, chat: &str, message: &str) {
        if let Some(entry) = self.entries.get_mut(chat)
            && entry
                .confirmed
                .as_deref()
                .is_none_or(|id| message_id_cmp(id, message).is_lt())
        {
            entry.confirmed = Some(message.into());
            if entry
                .pending
                .as_deref()
                .is_some_and(|id| message_id_cmp(id, message).is_le())
            {
                entry.pending = None;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coalesces_and_never_regresses() {
        let t = Instant::now();
        let mut m = Markers::default();
        m.expose("c".into(), "9".into(), t);
        assert_eq!(m.due(t).unwrap().1, "9");
        m.confirmed("c", "9");
        m.expose("c".into(), "10".into(), t);
        m.expose("c".into(), "11".into(), t);
        m.expose("c".into(), "8".into(), t);
        assert!(m.due(t).is_none());
        assert_eq!(m.due(t + Duration::from_secs(1)).unwrap().1, "11");
        m.confirmed("c", "11");
        m.expose("c".into(), "10".into(), t);
        assert!(m.wait(t).is_none());
    }
}

#[cfg(test)]
mod retry_tests {
    use super::*;
    #[test]
    fn failed_request_remains_pending_and_idle_entries_are_evicted() {
        let t = Instant::now();
        let mut markers = Markers::default();
        markers.expose("c".into(), "10".into(), t);
        assert_eq!(markers.due(t).unwrap().1, "10");
        assert!(markers.due(t).is_none());
        assert_eq!(markers.due(t + Duration::from_secs(1)).unwrap().1, "10");
        markers.confirmed("c", "10");
        assert!(markers.wait(t).is_none());
        for n in 0..600 {
            let chat = format!("chat{n}");
            markers.expose(chat.clone(), "11".into(), t);
            let (id, message) = markers.due(t).unwrap();
            assert_eq!(id, chat);
            markers.confirmed(&id, &message);
        }
        assert_eq!(markers.entries.len(), 256);
    }
}
