//! Fixed XDG locations for session credentials, the crypto module and cache.
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Locations {
    pub session: PathBuf,
    pub cache: PathBuf,
    pub wasm: PathBuf,
}
impl Locations {
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
            session: data.join("session.json"),
            wasm: data.join("ltsm.wasm"),
            cache: base("XDG_CACHE_HOME", ".cache").join("line-desktop"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    #[test]
    fn absolute_xdg_overrides_and_home_fallbacks() {
        let paths = Locations::resolve(|key| match key {
            "HOME" => Some("/home/synthetic".into()),
            "XDG_DATA_HOME" => Some("/data".into()),
            "XDG_CACHE_HOME" => Some("/cache".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(paths.session, Path::new("/data/line-desktop/session.json"));
        assert_eq!(paths.wasm, Path::new("/data/line-desktop/ltsm.wasm"));
        assert_eq!(paths.cache, Path::new("/cache/line-desktop"));
        for invalid in [None, Some(""), Some("relative")] {
            let paths = Locations::resolve(|key| {
                if key == "HOME" {
                    Some("/home/synthetic".into())
                } else {
                    invalid.map(PathBuf::from)
                }
            })
            .unwrap();
            assert_eq!(
                paths.session,
                Path::new("/home/synthetic/.local/share/line-desktop/session.json")
            );
            assert_eq!(
                paths.wasm,
                Path::new("/home/synthetic/.local/share/line-desktop/ltsm.wasm")
            );
            assert_eq!(
                paths.cache,
                Path::new("/home/synthetic/.cache/line-desktop")
            );
        }
    }
    #[test]
    fn home_must_be_absolute() {
        for home in [None, Some(""), Some("relative")] {
            assert!(
                Locations::resolve(|key| {
                    if key == "HOME" {
                        home.map(PathBuf::from)
                    } else {
                        Some("/xdg".into())
                    }
                })
                .is_err()
            );
        }
    }
    #[test]
    fn xdg_paths_are_absolute_and_separate() {
        let paths = Locations::resolve(|key| match key {
            "HOME" => Some("/home/test".into()),
            "XDG_DATA_HOME" => Some("relative".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(
            paths.session,
            Path::new("/home/test/.local/share/line-desktop/session.json")
        );
        assert_eq!(
            paths.wasm,
            Path::new("/home/test/.local/share/line-desktop/ltsm.wasm")
        );
    }
}
