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
        copy_if_absent(&old.join("session.json"), &self.session)?;
        copy_if_absent(
            &old.join("accounts/last-mid"),
            &self.data.join("auth/last-account"),
        )?;
        Ok(())
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
    if mid.is_empty()
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

#[cfg(test)]
mod tests;
