//! Durable local logout. The marker contains identifiers, never a session token.
use crate::{
    locations::{Locations, remove_file},
    store::Store,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub(crate) struct Pending {
    mid: Option<String>,
    key_id: Option<String>,
    delete_data: bool,
}
fn step(clean: &mut bool, name: &str, result: anyhow::Result<()>) {
    if let Err(error) = result {
        *clean = false;
        crate::reporting::log("logout", name, error.as_ref());
    }
}
impl Pending {
    pub(crate) fn begin(
        locations: &Locations,
        mid: Option<String>,
        delete_data: bool,
    ) -> (Self, bool) {
        let mut clean = true;
        let key_id = match crate::credentials::saved_key_id(&locations.session) {
            Ok(id) => id,
            Err(error) => {
                step(&mut clean, "session-key-id", Err(error));
                None
            }
        };
        let pending = Self {
            mid,
            key_id,
            delete_data,
        };
        step(
            &mut clean,
            "pending",
            (|| {
                crate::client::private_write(
                    &locations.data.join("auth/logout-pending"),
                    &serde_json::to_vec(&pending)?,
                )
            })(),
        );
        // This must precede the network call and any keyring prompt.
        pending.detach(locations, &mut clean);
        (pending, clean)
    }
    fn detach(&self, locations: &Locations, clean: &mut bool) {
        // Retained until first-release migration is removed: otherwise an old
        // app's untouched session would be copied back on every subsequent start.
        step(
            clean,
            "migration",
            crate::client::private_write(&locations.data.join("auth/logged-out"), b""),
        );
        step(clean, "session", remove_file(&locations.session));
        step(
            clean,
            "last-account",
            remove_file(&locations.data.join("auth/last-account")),
        );
    }
    pub(crate) fn finish(&self, locations: &Locations, clean: bool) -> bool {
        self.finish_with(locations, clean, crate::credentials::delete_key)
    }
    pub(crate) fn finish_with(
        &self,
        locations: &Locations,
        mut clean: bool,
        delete_key: impl Fn(&str) -> anyhow::Result<()>,
    ) -> bool {
        self.detach(locations, &mut clean);
        if let Some(id) = &self.key_id {
            step(&mut clean, "keyring", delete_key(id));
        }
        if self.delete_data
            && let Some(mid) = &self.mid
        {
            step(
                &mut clean,
                "data",
                (|| {
                    let marker = crate::locations::account_directory(
                        &locations.data.join("auth/forgotten"),
                        mid,
                    )?;
                    crate::client::private_write(&marker, b"")?;
                    crate::locations::delete_account(&locations.data, mid)
                })(),
            );
            // Clipboard names have no account ID. Retain only retry sources still
            // referenced by the remaining stores, immediately after store deletion.
            step(
                &mut clean,
                "clipboard",
                (|| {
                    Store::open_in_memory()?.prune_clipboard(
                        &locations.data.join("clipboard-attachments"),
                        &locations.data,
                    )?;
                    Ok(())
                })(),
            );
            step(&mut clean, "previews", delete_previews(locations, mid));
            step(
                &mut clean,
                "cache",
                crate::locations::delete_account(&locations.cache, mid),
            );
        }
        if clean {
            step(
                &mut clean,
                "pending",
                remove_file(&locations.data.join("auth/logout-pending")),
            );
        }
        clean
    }
}
fn delete_previews(locations: &Locations, mid: &str) -> anyhow::Result<()> {
    let images = locations.data.join("images");
    if images.exists() {
        for entry in std::fs::read_dir(images)? {
            let path = entry?.path();
            if path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("local-previews-"))
            {
                crate::locations::delete_account(&path, mid)?;
            }
        }
    }
    Ok(())
}
pub(crate) fn resume(locations: &Locations) -> Option<bool> {
    let path = locations.data.join("auth/logout-pending");
    if !path.exists() {
        return None;
    }
    let pending = std::fs::read(path)
        .map_err(anyhow::Error::from)
        .and_then(|bytes| Ok(serde_json::from_slice::<Pending>(&bytes)?));
    Some(match pending {
        Ok(pending) => pending.finish(locations, true),
        Err(error) => {
            crate::reporting::log("logout", "pending", error.as_ref());
            false
        }
    })
}
