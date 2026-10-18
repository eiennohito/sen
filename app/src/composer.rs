use crate::*;

impl View {
    pub(crate) fn retain_send_requests(&mut self) {
        let mut request_seqs: Vec<_> = self
            .saved
            .outbox
            .iter()
            .filter(|item| item.server_id.is_empty())
            .filter_map(|item| item.request_seq)
            .collect();
        request_seqs.sort_unstable();
        request_seqs.dedup();
        if self.retained_request_seqs.as_ref() == Some(&request_seqs) {
            return;
        }
        // Never prune protocol echo bindings before the corresponding outbox
        // change is durable; a crash must retain exact reconciliation data.
        if !self
            .storage
            .as_ref()
            .is_some_and(|storage| storage.flush(self.saved.clone()))
        {
            return;
        }
        if self.commands.as_ref().is_some_and(|tx| {
            tx.try_send(account::Command::RetainSendRequests {
                request_seqs: request_seqs.clone(),
            })
            .is_ok()
        }) {
            self.retained_request_seqs = Some(request_seqs);
        }
    }

    /// LINE request sequences are positive signed 31-bit integers. Never wrap:
    /// an old unresolved echo must not alias a later submission after restart.
    pub(crate) fn allocate_request_seq(&mut self) -> Option<u32> {
        let retained = self
            .saved
            .outbox
            .iter()
            .filter_map(|item| item.request_seq)
            .max()
            .unwrap_or(0);
        if self.saved.next_request_seq == 0 && retained == 0 {
            use std::io::Read;
            let mut bytes = [0u8; 4];
            std::fs::File::open("/dev/urandom")
                .ok()?
                .read_exact(&mut bytes)
                .ok()?;
            self.saved.next_request_seq = (u32::from_ne_bytes(bytes) & 0x3fff_ffff) + 1;
        }
        let next = self
            .saved
            .next_request_seq
            .max(retained.saturating_add(1))
            .max(1);
        if next > i32::MAX as u32 {
            return None;
        }
        self.saved.next_request_seq = next + 1;
        Some(next)
    }
    pub(crate) fn recover_outgoing(&mut self, ui: &AppWindow, id: &str) {
        let Some(item) = self.saved.outbox.iter().find(|o| {
            o.local_id == id
                && o.chat == self.current
                && o.server_id.is_empty()
                && o.status != "Pending…"
        }) else {
            return;
        };
        if ui.get_draft().is_empty() {
            let text = item.text.clone();
            ui.set_draft(text.clone().into());
            ui.invoke_restore_editor(text.len() as i32, text.len() as i32);
            self.persist(ui);
            ui.invoke_focus_composer();
        } else {
            ui.set_recovery_id(id.into());
            ui.set_recovery_kind("edit".into());
        }
    }
    pub(crate) fn retry_outgoing(&mut self, ui: &AppWindow, id: &str, confirmed: bool) {
        let Some(index) = self.saved.outbox.iter().position(|o| {
            o.local_id == id
                && o.chat == self.current
                && o.server_id.is_empty()
                && o.status != "Pending…"
        }) else {
            return;
        };
        let uncertain = !self.saved.outbox[index].status.starts_with("Not submitted");
        if uncertain && !confirmed {
            ui.set_recovery_id(id.into());
            ui.set_recovery_kind("resend".into());
            return;
        }
        if !self.demo
            && (!ui.get_connected()
                || !ui.get_storage_error().is_empty()
                || self
                    .storage
                    .as_ref()
                    .is_none_or(|storage| !storage.healthy()))
        {
            return;
        }
        let Some(request_seq) = self.allocate_request_seq() else {
            ui.set_storage_error("Couldn’t prepare this message. Try again.".into());
            self.send_availability(ui);
            return;
        };
        let original = self.saved.outbox[index].clone();
        let index = if uncertain {
            if self.saved.outbox.len() >= 256 {
                return;
            }
            self.serial += 1;
            let mut retry = original.clone();
            retry.local_id = format!(
                "local-{}-{}",
                chrono::Utc::now().timestamp_micros(),
                self.serial
            );
            retry.timestamp_ms = now_millis();
            retry.status = "Pending…".into();
            retry.request_seq = Some(request_seq);
            self.saved.outbox.push(retry);
            self.saved.outbox.len() - 1
        } else {
            self.saved.outbox[index].status = "Pending…".into();
            self.saved.outbox[index].request_seq = Some(request_seq);
            index
        };
        if !self.demo
            && !self
                .storage
                .as_ref()
                .is_some_and(|storage| storage.flush(self.saved.clone()))
        {
            if uncertain {
                self.saved.outbox.pop();
            } else {
                self.saved.outbox[index] = original;
            }
            ui.set_storage_error(
                "Could not save the retry. Nothing was submitted. Retry saving first.".into(),
            );
            self.send_availability(ui);
            return;
        }
        let item = &self.saved.outbox[index];
        let command = account::Command::Send {
            id: item.chat.clone(),
            generation: self.generation,
            text: item.text.clone(),
            local_id: item.local_id.clone(),
            request_seq,
        };
        if self
            .commands
            .as_ref()
            .is_none_or(|tx| tx.try_send(command).is_err())
        {
            self.saved.outbox[index].status = "Not submitted · Try again when connected.".into();
        }
        self.render(ui);
        ui.invoke_latest();
        self.persist(ui);
    }
    pub(crate) fn resolve_recovery(&mut self, ui: &AppWindow, action: &str) {
        let id = ui.get_recovery_id().to_string();
        let kind = ui.get_recovery_kind().to_string();
        ui.set_recovery_kind("".into());
        ui.set_recovery_id("".into());
        if action == "cancel" || kind == "resend" {
            ui.invoke_focus_history();
            ui.set_recovery_focus_id(id.clone().into());
            ui.set_recovery_focus_action(kind.clone().into());
        }
        if action == "cancel" {
            return;
        }
        if kind == "resend" && action == "resend" {
            self.retry_outgoing(ui, &id, true);
        } else if kind == "edit"
            && (action == "replace" || action == "append")
            && let Some(item) = self
                .saved
                .outbox
                .iter()
                .find(|o| o.local_id == id && o.chat == self.current)
        {
            let draft = if action == "append" && !ui.get_draft().is_empty() {
                format!("{}\n{}", ui.get_draft(), item.text)
            } else {
                item.text.clone()
            };
            ui.set_draft(draft.clone().into());
            ui.invoke_restore_editor(draft.len() as i32, draft.len() as i32);
            self.persist(ui);
            ui.invoke_focus_composer();
        }
    }
}
