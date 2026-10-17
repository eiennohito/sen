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
            return;
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
        Some((chat.clone(), entry.pending.take().unwrap()))
    }
    pub fn confirmed(&mut self, chat: &str, message: &str) {
        if let Some(entry) = self.entries.get_mut(chat)
            && entry
                .confirmed
                .as_deref()
                .is_none_or(|id| message_id_cmp(id, message).is_lt())
        {
            entry.confirmed = Some(message.into());
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
