//! Public, synthetic crypto oracle fixture shared by tests and the harness.
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const FIXTURE_PATH: &str = "artifacts/line-3.7.2/ltsm.wasm";
pub const FIXTURE_VERSION: &str = "3.7.2";
pub const FIXTURE_SHA256: &str = "58bb4e189ab9bbd7d72ed415258da36afd50e306cb019bab75329d4f5f1b65b3";
pub fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_PATH)
}
pub fn read_fixture(path: &Path) -> Result<Vec<u8>> {
    let bytes = std::fs::read(path)?;
    ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == FIXTURE_SHA256,
        "crypto fixture hash mismatch"
    );
    Ok(bytes)
}
