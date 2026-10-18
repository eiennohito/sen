pub mod backend;
pub mod client;
pub mod controller;
pub mod credentials;
pub mod engine;
pub mod locations;
#[cfg(feature = "native-crypto")]
pub mod native_crypto;
pub mod runtime;
pub mod store;
use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use runtime::Runtime;
use sha2::{Digest, Sha256};

pub const TOKEN: &str = "wODdrvWqmdP4Zliay-iF3cz3KZcK0ekrial868apg06TXeCo7A1hIQO0ESElHg6D";
pub const VERSION: &str = "3.7.2";
pub const HASH: &str = "58bb4e189ab9bbd7d72ed415258da36afd50e306cb019bab75329d4f5f1b65b3";
/// Repository path for the development harness and explicit legacy-file selection.
pub const DEFAULT_WASM: &str = "artifacts/line-3.7.2/ltsm.wasm";

pub fn sign(rt: &mut Runtime, path: &str, body: &str, access: &str) -> Result<String> {
    let token = rt.string(TOKEN)?;
    let sk = rt.static_call("SecureKey", "loadToken", &[token])?;
    rt.free_string(token)?;
    let cv = rt.bytes(&Sha256::digest(VERSION.as_bytes()));
    let at = rt.bytes(&Sha256::digest(access.as_bytes()));
    let dk = rt.method("SecureKey", "deriveKey", sk, &[cv, at])?;
    rt.release(cv)?;
    rt.release(at)?;
    let hmac = rt.construct("Hmac", &[dk as u64])?;
    let msg = rt.bytes(format!("{path}{body}").as_bytes());
    let out = rt.method("Hmac", "digest", hmac, &[msg])?;
    rt.release(msg)?;
    let signature = STANDARD.encode(rt.output(out)?);
    rt.destroy("Hmac", hmac)?;
    rt.destroy("SecureKey", dk)?;
    rt.destroy("SecureKey", sk)?;
    Ok(signature)
}

pub mod attachments;
