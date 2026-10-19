//! Locate the public crypto module of the newest installed LINE Chrome extension.
use crate::client::error::{Error, SignInFailure};
use anyhow::{Context, Result};
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
fn version(root: &Path) -> Result<String> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("manifest.json"))?)?;
    let version = manifest["version"]
        .as_str()
        .context("extension manifest version missing")?;
    anyhow::ensure!(
        !version.is_empty() && version.split('.').all(|n| n.parse::<u32>().is_ok()),
        "invalid extension version"
    );
    Ok(version.into())
}
pub fn discover_in(config: &Path) -> Result<CryptoModule> {
    let mut candidates = Vec::new();
    for browser in fs::read_dir(config)? {
        let browser = browser?;
        let name = browser.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("google-chrome") && name != "chromium" {
            continue;
        }
        if !browser.file_type()?.is_dir() {
            continue;
        }
        for profile in fs::read_dir(browser.path())? {
            let profile = profile?;
            if !profile.file_type()?.is_dir() {
                continue;
            }
            let extensions = profile.path().join("Extensions").join(EXTENSION_ID);
            if !extensions.try_exists()? {
                continue;
            }
            for entry in fs::read_dir(extensions)? {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                let ver = version(&entry.path())?;
                let numbers: Vec<u32> = ver
                    .split('.')
                    .map(str::parse)
                    .collect::<std::result::Result<_, _>>()?;
                candidates.push((numbers, entry.path(), ver));
            }
        }
    }
    let (_, root, version) = candidates
        .into_iter()
        .max()
        .ok_or(Error::SignIn(SignInFailure::ExtensionMissing))?;
    let wasm = root.join("static/js/ltsm.wasm");
    if !wasm.is_file() {
        return Err(Error::SignIn(SignInFailure::CryptoChanged {
            version,
            missing: "static/js/ltsm.wasm".into(),
        })
        .into());
    }
    Ok(CryptoModule { wasm, version })
}
pub fn resolve(
    wasm_override: Option<PathBuf>,
    version_override: Option<String>,
) -> Result<CryptoModule> {
    if let Some(wasm) = wasm_override {
        let root = wasm
            .ancestors()
            .skip(1)
            .find(|p| p.join("manifest.json").is_file());
        let version = if let Some(root) = root {
            version(root)?
        } else {
            version_override
                .filter(|v| !v.is_empty())
                .context("--wasm outside an extension requires --client-version")?
        };
        return Ok(CryptoModule { wasm, version });
    }
    discover_in(&crate::locations::Locations::config_home()?)
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
        ) -> PathBuf {
            let root = self
                .0
                .join(browser)
                .join(profile)
                .join("Extensions")
                .join(EXTENSION_ID)
                .join(directory);
            fs::create_dir_all(root.join("static/js")).unwrap();
            fs::write(
                root.join("manifest.json"),
                json!({"version": version}).to_string(),
            )
            .unwrap();
            fs::write(root.join("static/js/ltsm.wasm"), b"public synthetic WASM").unwrap();
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
        f.extension("google-chrome", "Default", "999_0", "3.9.0");
        let newest = f.extension("chromium", "Profile 2", "1_0", "3.10.0");
        f.extension("google-chrome-beta", "Profile 1", "888_0", "3.8.9");
        let module = discover_in(&f.0).unwrap();
        assert_eq!(module.version, "3.10.0");
        assert_eq!(module.wasm, newest.join("static/js/ltsm.wasm"));
    }
}
