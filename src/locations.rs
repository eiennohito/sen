//! XDG locations for credentials and cache.
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Locations {
    pub config_home: PathBuf,
    pub session: PathBuf,
    pub cache: PathBuf,
}
impl Locations {
    pub fn config_home() -> anyhow::Result<PathBuf> {
        Self::resolve(|key| std::env::var_os(key).map(PathBuf::from))
            .map(|locations| locations.config_home)
            .map_err(anyhow::Error::msg)
    }

    pub fn from_environment() -> Result<Self, String> {
        Self::resolve(|key| std::env::var_os(key).map(PathBuf::from))
    }
    fn resolve(get: impl Fn(&str) -> Option<PathBuf>) -> Result<Self, String> {
        let home = get("HOME")
            .filter(|p| p.is_absolute())
            .ok_or("HOME must be an absolute path")?;
        let base = |key, suffix| {
            get(key)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(suffix))
        };
        let data = base("XDG_DATA_HOME", ".local/share").join("line-desktop");
        Ok(Self {
            config_home: base("XDG_CONFIG_HOME", ".config"),
            session: data.join("session.json"),
            cache: base("XDG_CACHE_HOME", ".cache").join("line-desktop"),
        })
    }
}
