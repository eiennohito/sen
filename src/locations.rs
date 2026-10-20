//! XDG locations for credentials and cache.
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Locations {
    pub config_home: PathBuf,
    pub session: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
}
impl Locations {
    // remove after the first release
    pub fn copy_first_start(&self) -> anyhow::Result<()> {
        let old = self.data.parent().unwrap().join("line-desktop");
        if self.data.join("auth/logged-out").exists()
            || self.data.join("auth/logout-pending").exists()
        {
            return Ok(());
        }
        copy_if_absent(&old.join("session.json"), &self.session)?;
        copy_if_absent(
            &old.join("accounts/last-mid"),
            &self.data.join("auth/last-account"),
        )?;
        Ok(())
    }

    pub(crate) fn account(data: PathBuf, cache: PathBuf) -> Self {
        Self {
            config_home: PathBuf::new(),
            session: data.join("auth/session.json"),
            data,
            cache,
        }
    }

    pub fn fonts(&self) -> PathBuf {
        self.data.join("fonts")
    }

    pub fn config_home() -> anyhow::Result<PathBuf> {
        Self::from_environment()
            .map(|locations| locations.config_home)
            .map_err(anyhow::Error::msg)
    }

    pub fn from_environment() -> Result<Self, String> {
        let get = |key| std::env::var_os(key).map(PathBuf::from);
        let home = get("HOME")
            .filter(|p| p.is_absolute())
            .ok_or("HOME must be an absolute path")?;
        let base = |key, suffix| {
            get(key)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(suffix))
        };
        let data = base("XDG_DATA_HOME", ".local/share").join("sen");
        Ok(Self {
            config_home: base("XDG_CONFIG_HOME", ".config"),
            session: data.join("auth/session.json"),
            data,
            cache: base("XDG_CACHE_HOME", ".cache").join("sen"),
        })
    }
}

// remove after the first release
fn copy_if_absent(old: &std::path::Path, new: &std::path::Path) -> anyhow::Result<()> {
    if !new.exists() && old.exists() {
        crate::client::private_write(new, &std::fs::read(old)?)?;
    }
    Ok(())
}

/// Account IDs are single path segments, using the same alphabet as service IDs.
pub fn account_directory(
    root: &std::path::Path,
    mid: &str,
) -> Result<PathBuf, crate::store::StoreError> {
    if matches!(mid, "auth" | "images" | "fonts" | "clipboard-attachments")
        || mid.is_empty()
        || mid.len() > 256
        || !mid
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Err(crate::store::StoreError::Invalid(
            "invalid account MID".into(),
        ));
    }
    Ok(root.join(mid))
}

pub(crate) fn remove_file(path: &std::path::Path) -> anyhow::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn delete_account(root: &std::path::Path, mid: &str) -> anyhow::Result<()> {
    let path = account_directory(root, mid)?;
    if !path.try_exists()? {
        return Ok(());
    }
    anyhow::ensure!(
        !std::fs::symlink_metadata(&path)?.file_type().is_symlink(),
        "account directory is a symlink"
    );
    anyhow::ensure!(
        path.canonicalize()?.parent() == Some(root.canonicalize()?.as_path()),
        "account directory escapes its root"
    );
    std::fs::remove_dir_all(path)?;
    Ok(())
}

#[cfg(test)]
mod tests;
