//! Shared XDG paths and explicit legacy-file selection.
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Locations {
    pub session: PathBuf,
    pub state: PathBuf,
    pub cache: PathBuf,
    pub wasm: PathBuf,
    pub config: PathBuf,
}
#[derive(Serialize, Deserialize)]
struct Existing {
    session: PathBuf,
    wasm: PathBuf,
    state: PathBuf,
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
            state: base("XDG_STATE_HOME", ".local/state").join("line-desktop"),
            cache: base("XDG_CACHE_HOME", ".cache").join("line-desktop"),
            config: base("XDG_CONFIG_HOME", ".config").join("line-desktop/paths.json"),
        })
    }
    pub fn load_existing(&mut self) -> Result<bool, Box<dyn std::error::Error>> {
        if !self.config.exists() {
            return Ok(false);
        }
        let old: Existing = serde_json::from_slice(&fs::read(&self.config)?)?;
        self.session = old.session;
        self.wasm = old.wasm;
        self.state = old.state;
        Ok(true)
    }
    pub fn legacy_available(root: &Path) -> bool {
        root.join("artifacts/auth/session.json").exists() || root.join(crate::DEFAULT_WASM).exists()
    }
    pub fn choose(
        &mut self,
        root: &Path,
        existing: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if existing {
            self.session = root.join("artifacts/auth/session.json");
            self.wasm = root.join(crate::DEFAULT_WASM);
            self.state = root.join("artifacts/auth/desktop-state");
        }
        let value = Existing {
            session: self.session.clone(),
            wasm: self.wasm.clone(),
            state: self.state.clone(),
        };
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(self.config.parent().ok_or("missing config parent")?)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&self.config)?;
        use std::io::Write;
        file.write_all(&serde_json::to_vec(&value)?)?;
        file.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absolute_xdg_overrides_and_home_fallbacks() {
        let paths = Locations::resolve(|key| match key {
            "HOME" => Some("/home/synthetic".into()),
            "XDG_DATA_HOME" => Some("/data".into()),
            "XDG_STATE_HOME" => Some("/state".into()),
            "XDG_CACHE_HOME" => Some("/cache".into()),
            "XDG_CONFIG_HOME" => Some("/config".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(paths.session, Path::new("/data/line-desktop/session.json"));
        assert_eq!(paths.wasm, Path::new("/data/line-desktop/ltsm.wasm"));
        assert_eq!(paths.state, Path::new("/state/line-desktop"));
        assert_eq!(paths.cache, Path::new("/cache/line-desktop"));
        assert_eq!(paths.config, Path::new("/config/line-desktop/paths.json"));
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
                paths.state,
                Path::new("/home/synthetic/.local/state/line-desktop")
            );
            assert_eq!(
                paths.cache,
                Path::new("/home/synthetic/.cache/line-desktop")
            );
            assert_eq!(
                paths.config,
                Path::new("/home/synthetic/.config/line-desktop/paths.json")
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
            "XDG_STATE_HOME" => Some("/state".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(
            paths.session,
            Path::new("/home/test/.local/share/line-desktop/session.json")
        );
        assert_eq!(paths.state, Path::new("/state/line-desktop"));
        assert_eq!(
            paths.wasm,
            Path::new("/home/test/.local/share/line-desktop/ltsm.wasm")
        );
    }
    #[test]
    fn existing_files_require_explicit_choice_and_stay_in_place() {
        let root = std::env::temp_dir().join(format!("line-migration-test-{}", std::process::id()));
        fs::create_dir_all(root.join("artifacts/auth")).unwrap();
        let old = root.join("artifacts/auth/session.json");
        fs::write(&old, b"synthetic sentinel, not credentials").unwrap();
        let mut paths = Locations::resolve(|key| (key == "HOME").then(|| root.clone())).unwrap();
        assert!(Locations::legacy_available(&root));
        assert!(!paths.load_existing().unwrap());
        assert!(!paths.config.exists());
        paths.choose(&root, true).unwrap();
        let mut reloaded = Locations::resolve(|key| (key == "HOME").then(|| root.clone())).unwrap();
        assert!(reloaded.load_existing().unwrap());
        assert_eq!(reloaded.session, old);
        assert_eq!(reloaded.state, root.join("artifacts/auth/desktop-state"));
        assert_eq!(
            fs::read(&old).unwrap(),
            b"synthetic sentinel, not credentials"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn start_fresh_records_defaults_without_touching_existing_files() {
        let root = std::env::temp_dir().join(format!("line-fresh-test-{}", std::process::id()));
        fs::create_dir_all(root.join("artifacts/auth")).unwrap();
        let old = root.join("artifacts/auth/session.json");
        fs::write(&old, b"synthetic old file").unwrap();
        let mut paths = Locations::resolve(|key| (key == "HOME").then(|| root.clone())).unwrap();
        let session = paths.session.clone();
        paths.choose(&root, false).unwrap();
        assert_eq!(paths.session, session);
        assert!(!session.exists());
        assert_eq!(fs::read(old).unwrap(), b"synthetic old file");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&paths.config).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_dir_all(root).unwrap();
    }
}
