//! Bounded deferred UI updates while the user holds a list/scrollbar.
use crate::*;
const ORDINARY_LIMIT: usize = 128;
const TOTAL_LIMIT: usize = 5500; // 8*600 retained withdrawals + 2*256 outbox terminals +128 ordinary +16 page outcomes.
fn message_bytes(message: &account::MessageView) -> usize {
    let rich = &message.rich;
    let spans = |spans: &[line_wasm_poc::client::desktop::rich::InlineEmoji]| {
        spans
            .iter()
            .map(|span| {
                span.fallback.len()
                    + span.product_id.len()
                    + span.sticon_id.len()
                    + span.version.as_ref().map_or(0, String::len)
                    + 192
            })
            .sum::<usize>()
    };
    message.body.len()
        + message.id.len()
        + message.sender_id.len()
        + message.sender_name.len()
        + message.sender_key.len()
        + message.image_key.len()
        + std::mem::size_of::<account::MessageView>()
        + 512
        + [
            &rich.file_name,
            &rich.contact_name,
            &rich.system_text,
            &rich.system_key,
            &rich.system_args,
        ]
        .iter()
        .map(|value| value.as_ref().map_or(0, String::len))
        .sum::<usize>()
        + rich
            .reply
            .as_ref()
            .map_or(0, |reply| reply.message_id.len() + 64)
        + spans(&rich.emoji)
        + message
            .emoji
            .iter()
            .map(|emoji| spans(std::slice::from_ref(&emoji.span)) + emoji.image_key.len())
            .sum::<usize>()
        + message
            .reactions
            .iter()
            .map(|reaction| reaction.label.len() + reaction.image_key.len() + 128)
            .sum::<usize>()
}
fn key(event: &account::Update) -> Option<(String, u8)> {
    use account::Update::*;
    Some(match event {
        Chats { .. } => ("chats".into(), 0),
        Withdrawn { chat, message_id } => (format!("withdraw:{chat}:{message_id}"), 4),
        Incoming { chat, .. } => (format!("incoming:{chat}"), 0),
        Messages { id, .. } => (format!("messages:{id}"), 0),
        ChatMeta { chat, .. } => (format!("meta:{chat}"), 0),
        Image { key, .. } | ImageFailed { key } => (format!("image:{key}"), 0),
        Sent { id, local_id, .. } => (format!("send:{id}:{local_id}"), 3),
        SendRejected { id, local_id, .. } => (format!("send:{id}:{local_id}"), 2),
        SendFailed { id, local_id } | SendUncertain { id, local_id } => {
            (format!("send:{id}:{local_id}"), 1)
        }
        OwnEcho {
            chat, request_seq, ..
        } => (format!("echo:{chat}:{request_seq}"), 3),
        Receipt { chat, .. } => (format!("receipt:{chat}"), 0),
        Older { chat, .. } | OlderFailed { chat, .. } => (format!("older:{chat}"), 1),
        _ => return None,
    })
}
impl View {
    pub(crate) fn defer_update(&mut self, ui: &AppWindow, mut event: account::Update) {
        let mut stale = false;
        if let account::Update::Withdrawn { chat, message_id } = &event {
            let retained = if chat == &self.current {
                self.messages.iter().any(|row| row.id == *message_id)
            } else {
                self.history
                    .get(chat)
                    .is_some_and(|history| history.messages.iter().any(|row| row.id == *message_id))
            };
            if !retained {
                return;
            }
        }

        if let account::Update::Incoming {
            chat,
            messages,
            replay: false,
        } = &event
            && chat == &self.current
            && !ui.get_following()
        {
            for message in messages.iter().filter(|message| !message.outgoing) {
                if !self.messages.iter().any(|row| row.id == message.id)
                    && !self.deferred_counted.contains(&message.id)
                {
                    if self.deferred_counted.len() < history_window::RETAINED_ROWS {
                        self.deferred_counted.insert(message.id.clone());
                        ui.set_new_count(ui.get_new_count().saturating_add(1));
                    } else {
                        stale = true;
                    }
                }
            }
        }

        // Critical events can only affect a local pending item. Discarding an
        // unrelated server echo is safe, and prevents untrusted sequence floods.
        if let account::Update::OwnEcho {
            chat, request_seq, ..
        } = &event
            && !self
                .saved
                .outbox
                .iter()
                .any(|item| item.chat == *chat && item.request_seq == Some(*request_seq))
        {
            return;
        }
        let Some((event_key, rank)) = key(&event) else {
            return;
        };
        if let Some(index) = self
            .deferred
            .iter()
            .position(|old| key(old).is_some_and(|old| old.0 == event_key))
        {
            if key(&self.deferred[index]).is_some_and(|old| old.1 > rank) {
                return;
            }
            if let (
                account::Update::Incoming {
                    messages: incoming,
                    replay: incoming_replay,
                    ..
                },
                account::Update::Incoming {
                    messages: old,
                    replay: old_replay,
                    ..
                },
            ) = (&mut event, &mut self.deferred[index])
            {
                *incoming_replay &= *old_replay;
                for message in old.drain(..) {
                    if !incoming.iter().any(|item| item.id == message.id) {
                        incoming.push(message);
                    }
                }
                incoming.sort_by(|a, b| {
                    a.timestamp
                        .cmp(&b.timestamp)
                        .then_with(|| a.id.len().cmp(&b.id.len()))
                        .then_with(|| a.id.cmp(&b.id))
                });
            }
            if let (
                account::Update::Chats {
                    chats: incoming, ..
                },
                account::Update::Chats { chats: old, .. },
            ) = (&mut event, &mut self.deferred[index])
            {
                for chat in old.drain(..) {
                    if !incoming.iter().any(|item| item.id == chat.id) {
                        incoming.push(chat);
                    }
                }
                if incoming.len() > 1000 {
                    incoming.truncate(1000);
                    self.list_retry.set(account::ListRequest::Refresh);
                    ui.set_list_error(
                        "Conversation updates were deferred. Retry to update the list.".into(),
                    );
                    stale = true;
                }
            }
            self.deferred.remove(index);
        }
        if matches!(event, account::Update::Incoming { .. })
            && self
                .deferred
                .iter()
                .filter(|item| matches!(item, account::Update::Incoming { .. }))
                .count()
                >= 8
            && let Some(index) = self
                .deferred
                .iter()
                .position(|item| matches!(item, account::Update::Incoming { .. }))
        {
            self.deferred.remove(index);
            stale = true;
        }
        if let account::Update::Incoming { messages, .. } = &mut event {
            if messages.len() > 50 {
                messages.drain(..messages.len() - 50);
                stale = true;
            }
            let mut bytes: usize = messages.iter().map(message_bytes).sum();
            while bytes > history_window::RETAINED_BYTES && !messages.is_empty() {
                bytes -= message_bytes(&messages[0]);
                messages.remove(0);
                stale = true;
            }
        }
        if rank == 0
            && self
                .deferred
                .iter()
                .filter(|item| key(item).is_some_and(|key| key.1 == 0))
                .count()
                >= ORDINARY_LIMIT
            && let Some(index) = self
                .deferred
                .iter()
                .position(|item| key(item).is_some_and(|key| key.1 == 0))
        {
            self.deferred.remove(index);
            stale = true;
        }
        if self.deferred.len() >= TOTAL_LIMIT {
            if let Some(index) = self
                .deferred
                .iter()
                .position(|item| key(item).is_some_and(|key| key.1 == 0))
            {
                self.deferred.remove(index);
                stale = true;
            } else {
                // More terminal events than the durable outbox can contain is invalid input.
                stale = true;
                if rank == 0 {
                    ui.global::<State>().set_history_needs_refresh(true);
                    return;
                }
                self.deferred.remove(0);
            }
        }
        self.deferred.push(event);
        // Pixel buffers outlive the worker cache while queued. Bound them too.
        while self
            .deferred
            .iter()
            .map(|event| match event {
                account::Update::Image { pixels, .. } => pixels.rgba.len(),
                _ => 0,
            })
            .sum::<usize>()
            > 16 * 1024 * 1024
        {
            let Some(index) = self
                .deferred
                .iter()
                .position(|event| matches!(event, account::Update::Image { .. }))
            else {
                break;
            };
            self.deferred.remove(index);
            stale = true;
        }
        if stale {
            ui.global::<State>().set_history_needs_refresh(true);
        }
    }
}
