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
fn version(root: &Path) -> Result<(String, Vec<u32>)> {
    let path = root.join("manifest.json");
    let load = || -> Result<(String, Vec<u32>)> {
        let manifest: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
        let version = manifest["version"]
            .as_str()
            .context("extension manifest version missing")?;
        let numbers = version
            .split('.')
            .map(str::parse)
            .collect::<std::result::Result<Vec<u32>, _>>()?;
        Ok((version.into(), numbers))
    };
    load().context(Error::SignIn(SignInFailure::CryptoModule(path)))
}
pub fn discover_in(config: &Path) -> Result<CryptoModule> {
    discover_candidates(config).map_err(|error| module_error(error, config))
}
fn module_error(error: anyhow::Error, path: &Path) -> anyhow::Error {
    if error.downcast_ref::<Error>().is_some() {
        error
    } else {
        error.context(Error::SignIn(SignInFailure::CryptoModule(path.into())))
    }
}
fn discover_candidates(config: &Path) -> Result<CryptoModule> {
    let mut candidates = Vec::new();
    let mut invalid = None;
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
                let (ver, numbers) = match version(&entry.path()) {
                    Ok(version) => version,
                    Err(error) => {
                        invalid = Some(error);
                        continue;
                    }
                };
                candidates.push((numbers, entry.path(), ver));
            }
        }
    }
    let (_, root, version) = candidates.into_iter().max().ok_or_else(|| {
        invalid.unwrap_or_else(|| Error::SignIn(SignInFailure::ExtensionMissing).into())
    })?;
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
        let version = match version_override {
            Some(version) => version,
            None => {
                let root = wasm
                    .parent()
                    .and_then(Path::parent)
                    .and_then(Path::parent)
                    .context("--wasm outside an extension requires --client-version")?;
                version(root)?.0
            }
        };
        return Ok(CryptoModule { wasm, version });
    }
    let mut module = discover_in(&crate::locations::Locations::config_home()?)?;
    if let Some(version) = version_override {
        module.version = version;
    }
    Ok(module)
}
impl CryptoModule {
    pub(crate) fn runtime(&self, bytes: &[u8]) -> Result<crate::runtime::Runtime> {
        crate::runtime::Runtime::new(bytes, &self.version)
            .map_err(|error| module_error(error, &self.wasm))
    }
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
    fn override_uses_only_version_directory_and_explicit_version_wins() {
        let f = Fixture::new();
        let root = f.extension("chromium", "Default", "version", "3.7.2");
        let wasm = root.join("static/js/ltsm.wasm");
        assert_eq!(resolve(Some(wasm.clone()), None).unwrap().version, "3.7.2");
        assert_eq!(
            resolve(Some(wasm), Some("9.0".into())).unwrap().version,
            "9.0"
        );
        fs::write(f.0.join("manifest.json"), br#"{"version":"99"}"#).unwrap();
        let outside = f.0.join("nested/static/js/ltsm.wasm");
        assert!(resolve(Some(outside), None).is_err());
    }
    #[test]
    fn newest_manifest_version_across_browsers_and_profiles_wins() {
        let f = Fixture::new();
        f.extension("google-chrome", "Default", "999_0", "3.9.0");
        let newest = f.extension("chromium", "Profile 2", "1_0", "3.10.0");
        f.extension("google-chrome-beta", "Profile 1", "888_0", "3.8.9");
        let broken = f.extension("google-chrome", "Default", "broken", "99.0.0");
        fs::write(broken.join("manifest.json"), b"broken").unwrap();
        let module = discover_in(&f.0).unwrap();
        assert_eq!(module.version, "3.10.0");
        assert_eq!(module.wasm, newest.join("static/js/ltsm.wasm"));
    }
}
