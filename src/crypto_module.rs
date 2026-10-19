//! Locate the public crypto assets of the installed LINE Chrome extension.
use crate::client::error::{Error, SignInFailure};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const EXTENSION_ID: &str = "ophjlpahpchlmihnnnihgmmeilfjmjjc";

#[derive(Clone, Debug)]
pub struct CryptoModule {
    pub wasm: PathBuf,
    pub version: String,
}

fn version(path: &Path) -> Result<String> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(path.join("manifest.json"))?)?;
    let version = manifest["version"]
        .as_str()
        .context("extension manifest version missing")?;
    anyhow::ensure!(
        !version.is_empty() && version.split('.').all(|n| n.parse::<u32>().is_ok()),
        "invalid extension version"
    );
    Ok(version.into())
}

fn wasm_asset(root: &Path) -> Result<PathBuf> {
    fn references(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::String(s) if s.ends_with(".wasm") => out.push(s.clone()),
            serde_json::Value::Array(a) => a.iter().for_each(|v| references(v, out)),
            serde_json::Value::Object(o) => o.values().for_each(|v| references(v, out)),
            _ => {}
        }
    }
    if let Ok(bytes) = fs::read(root.join("asset-manifest.json")) {
        let manifest = serde_json::from_slice(&bytes)?;
        let mut assets = Vec::new();
        references(&manifest, &mut assets);
        assets.sort_by_key(|s| !s.contains("ltsm"));
        for asset in assets {
            let path = root.join(asset.trim_start_matches('/'));
            if path.is_file() && path.canonicalize()?.starts_with(root.canonicalize()?) {
                return Ok(path);
            }
        }
    }
    // LINE's JS loader references static/js/ltsm.wasm in builds that omit it
    // from asset-manifest.json. Resolve in the extension, never in app data.
    let path = root.join("static/js/ltsm.wasm");
    if path.is_file() {
        return Ok(path);
    }
    bail!(Error::SignIn(SignInFailure::CryptoChanged {
        version: version(root)?,
        missing: "ltsm.wasm".into()
    }))
}

pub fn discover_in(config: &Path) -> Result<CryptoModule> {
    let roots = vec![
        config
            .join("google-chrome*/*/Extensions")
            .join(EXTENSION_ID)
            .join("<ver>"),
        config
            .join("chromium/*/Extensions")
            .join(EXTENSION_ID)
            .join("<ver>"),
    ];
    let mut candidates = Vec::new();
    for browser in fs::read_dir(config).into_iter().flatten().flatten() {
        let name = browser.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("google-chrome") && name != "chromium" {
            continue;
        }
        for profile in fs::read_dir(browser.path()).into_iter().flatten().flatten() {
            let extensions = profile.path().join("Extensions").join(EXTENSION_ID);
            for entry in fs::read_dir(extensions).into_iter().flatten().flatten() {
                if let Ok(ver) = version(&entry.path()) {
                    let numbers: Vec<u32> = ver.split('.').map(|n| n.parse().unwrap()).collect();
                    candidates.push((numbers, entry.path(), ver));
                }
            }
        }
    }
    candidates.sort();
    let (_, root, version) = candidates
        .pop()
        .ok_or(Error::SignIn(SignInFailure::ExtensionMissing(roots)))?;
    Ok(CryptoModule {
        wasm: wasm_asset(&root)?,
        version,
    })
}

/// An empty override means discover the newest installed extension. An override
/// outside an extension needs an explicit version or an installed extension.
pub fn resolve(override_path: &Path, client_version: Option<&str>) -> Result<CryptoModule> {
    let discover = || -> Result<CryptoModule> {
        let home = std::env::var_os("HOME").context("HOME missing")?;
        discover_in(&PathBuf::from(home).join(".config"))
    };
    if override_path.as_os_str().is_empty() {
        return discover();
    }
    let version = match client_version {
        Some(v) if !v.is_empty() => v.to_owned(),
        _ => override_path
            .ancestors()
            .skip(1)
            .find_map(|p| version(p).ok())
            .or_else(|| discover().ok().map(|m| m.version))
            .context("--wasm outside an installed extension requires --client-version")?,
    };
    Ok(CryptoModule {
        wasm: override_path.into(),
        version,
    })
}

impl CryptoModule {
    pub fn read(&self) -> Result<Vec<u8>> {
        fs::read(&self.wasm).context(Error::SignIn(SignInFailure::CryptoModule(
            self.wasm.clone(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "line-extension-discovery-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn extension(
            &self,
            browser: &str,
            profile: &str,
            directory: &str,
            version: &str,
            asset: &str,
        ) -> PathBuf {
            let root = self
                .0
                .join(browser)
                .join(profile)
                .join("Extensions")
                .join(EXTENSION_ID)
                .join(directory);
            fs::create_dir_all(root.join(Path::new(asset).parent().unwrap())).unwrap();
            fs::write(
                root.join("manifest.json"),
                json!({"version": version}).to_string(),
            )
            .unwrap();
            fs::write(
                root.join("asset-manifest.json"),
                json!({"files":{"ltsm.wasm":format!("/{asset}")}}).to_string(),
            )
            .unwrap();
            fs::write(root.join(asset), b"public synthetic WASM").unwrap();
            root
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn newest_manifest_version_across_browsers_and_profiles_wins() {
        let f = Fixture::new();
        f.extension(
            "google-chrome",
            "Default",
            "999_0",
            "3.9.0",
            "static/js/ltsm.wasm",
        );
        let newest = f.extension(
            "chromium",
            "Profile 2",
            "1_0",
            "3.10.0",
            "assets/crypto.123.wasm",
        );
        f.extension(
            "google-chrome-beta",
            "Profile 1",
            "888_0",
            "3.8.9",
            "static/js/ltsm.wasm",
        );
        let module = discover_in(&f.0).unwrap();
        assert_eq!(module.version, "3.10.0");
        assert_eq!(module.wasm, newest.join("assets/crypto.123.wasm"));
    }
    #[test]
    fn missing_extension_lists_search_locations() {
        let f = Fixture::new();
        let error = discover_in(&f.0).unwrap_err();
        let Error::SignIn(reason @ SignInFailure::ExtensionMissing(_)) =
            error.downcast_ref::<Error>().unwrap()
        else {
            panic!("wrong error")
        };
        assert!(
            reason
                .to_string()
                .contains("Install LINE for Chrome — this app uses its crypto module")
        );
        assert!(reason.to_string().contains("google-chrome*/*/Extensions"));
        assert!(reason.to_string().contains("chromium/*/Extensions"));
    }
    #[test]
    fn loader_fallback_and_broken_newest_do_not_select_an_old_build() {
        let f = Fixture::new();
        let old = f.extension(
            "google-chrome",
            "Default",
            "old",
            "3.7.2",
            "static/js/ltsm.wasm",
        );
        fs::remove_file(old.join("asset-manifest.json")).unwrap();
        assert_eq!(
            discover_in(&f.0).unwrap().wasm,
            old.join("static/js/ltsm.wasm")
        );
        let new = f.extension("chromium", "Default", "new", "4.0.0", "static/js/ltsm.wasm");
        fs::remove_file(new.join("static/js/ltsm.wasm")).unwrap();
        assert!(
            matches!(discover_in(&f.0).unwrap_err().downcast_ref::<Error>(), Some(Error::SignIn(SignInFailure::CryptoChanged { version, missing })) if version == "4.0.0" && missing == "ltsm.wasm")
        );
    }
    #[test]
    fn developer_override_uses_explicit_or_adjacent_manifest_version() {
        let f = Fixture::new();
        let extension = f.extension(
            "chromium",
            "Default",
            "test",
            "9.8.7",
            "static/js/ltsm.wasm",
        );
        let path = extension.join("static/js/ltsm.wasm");
        assert_eq!(resolve(&path, None).unwrap().version, "9.8.7");
        assert_eq!(resolve(&path, Some("8.7.6")).unwrap().version, "8.7.6");
    }
}
