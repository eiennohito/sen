//! LINE client protocol, crypto host and desktop application core.
pub mod backend;
pub mod client;
pub mod controller;
pub mod credentials;
pub mod crypto_module;
pub mod engine;
pub mod locations;
#[cfg(feature = "native-crypto")]
pub mod native_crypto;
pub mod runtime;
pub mod store;
#[cfg(test)]
mod test_support;
use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use runtime::{Call, Runtime};
use sha2::{Digest, Sha256};

pub const TOKEN: &str = "wODdrvWqmdP4Zliay-iF3cz3KZcK0ekrial868apg06TXeCo7A1hIQO0ESElHg6D";

/// Derive and retain the HMAC object for one version/access-token pair.
pub fn signing_key(rt: &mut Runtime, access: &str, client_version: &str) -> Result<u32> {
    let token = rt.string(TOKEN)?;
    let sk = rt.static_call(Call::SecureKeyLoadToken, &[token])?;
    rt.free_string(token)?;
    let cv = rt.bytes(&Sha256::digest(client_version.as_bytes()));
    let at = rt.bytes(&Sha256::digest(access.as_bytes()));
    let dk = rt.method(Call::SecureKeyDeriveKey, sk, &[cv, at])?;
    rt.release(cv)?;
    rt.release(at)?;
    let hmac = rt.construct(Call::HmacNew, &[dk as u64])?;
    rt.destroy("SecureKey", dk)?;
    rt.destroy("SecureKey", sk)?;
    Ok(hmac)
}
pub fn sign(rt: &mut Runtime, hmac: u32, path: &str, body: &str) -> Result<String> {
    let msg = rt.bytes(format!("{path}{body}").as_bytes());
    let out = rt.method(Call::HmacDigest, hmac, &[msg])?;
    rt.release(msg)?;
    Ok(STANDARD.encode(rt.output(out)?))
}

pub mod attachments;
pub mod reporting;
