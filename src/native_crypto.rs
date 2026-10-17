//! Trial primitives for plaintext keys. Production still uses the WASM runtime.
use aes::{
    Aes256,
    cipher::{Block, BlockDecrypt, BlockEncrypt, KeyInit},
};
use aes_gcm::{
    Aes256Gcm,
    aead::{Aead, Payload},
};
use anyhow::{Result, ensure};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use subtle::{ConditionallySelectable, ConstantTimeEq};
use x25519_dalek::{PublicKey, StaticSecret};

/// RFC 7748 public-key derivation from caller-provided scalar bytes.
pub fn x25519_public_key(scalar: &[u8; 32]) -> [u8; 32] {
    PublicKey::from(&StaticSecret::from(*scalar)).to_bytes()
}

/// RFC 7748 X25519, including canonical all-zero output for low-order points.
pub fn x25519(scalar: &[u8; 32], point: &[u8; 32]) -> [u8; 32] {
    StaticSecret::from(*scalar)
        .diffie_hellman(&PublicKey::from(*point))
        .to_bytes()
}

pub fn derive_key(key: &[u8; 32], prefix: &[u8], suffix: &[u8]) -> Result<[u8; 32]> {
    ensure!(
        !prefix.is_empty() && !suffix.is_empty(),
        "empty key derivation input"
    );
    let mut hash = Sha256::new();
    hash.update(prefix);
    hash.update(key);
    hash.update(suffix);
    Ok(hash.finalize().into())
}

pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).unwrap();
    mac.update(data);
    mac.finalize().into_bytes().into()
}

fn equal(a: &[u8], b: &[u8]) -> bool {
    bool::from(a.ct_eq(b))
}

fn fold(hash: &[u8; 32]) -> [u8; 16] {
    std::array::from_fn(|i| hash[i] ^ hash[i + 16])
}

fn gcm_encrypt(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], data: &[u8]) -> Vec<u8> {
    Aes256Gcm::new(key.into())
        .encrypt(nonce.into(), Payload { msg: data, aad })
        .expect("GCM input exceeds the supported length")
}

fn gcm_decrypt(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    Aes256Gcm::new(key.into())
        .decrypt(nonce.into(), Payload { msg: data, aad })
        .map_err(|_| anyhow::anyhow!("GCM authentication failed"))
}

/// The caller supplies fresh randomness; the trial never generates nonces.
pub fn aes_encrypt(key: &[u8; 32], nonce: &[u8; 12], data: &[u8]) -> Vec<u8> {
    let mut out = nonce.to_vec();
    out.extend_from_slice(&gcm_encrypt(key, nonce, &[], data));
    out
}

pub fn aes_decrypt(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>> {
    ensure!(data.len() > 28, "AES ciphertext too short");
    gcm_decrypt(key, data[..12].try_into().unwrap(), &[], &data[12..])
}

pub struct Channel {
    shared: [u8; 32],
}

pub struct MessageContext<'a> {
    pub to: &'a str,
    pub from: &'a str,
    pub sender_key_id: u32,
    pub receiver_key_id: u32,
    pub content_type: u32,
}

impl MessageContext<'_> {
    fn aad(&self) -> Vec<u8> {
        let mut aad = self.to.as_bytes().to_vec();
        aad.extend_from_slice(self.from.as_bytes());
        for n in [
            self.sender_key_id,
            self.receiver_key_id,
            2,
            self.content_type,
        ] {
            aad.extend_from_slice(&n.to_be_bytes());
        }
        aad
    }
}

impl Channel {
    /// Takes the raw X25519 result, not an exported SecureKey blob.
    pub fn new(shared: [u8; 32]) -> Self {
        Self { shared }
    }

    /// Oracle-compatible channel setup for curve public keys. The oracle accepts
    /// low-order points and encodes field zero as p rather than 32 zero bytes.
    /// Arbitrary twist inputs have different oracle semantics; see crypto-port.md.
    pub fn from_x25519(scalar: &[u8; 32], point: &[u8; 32]) -> Self {
        let shared = x25519(scalar, point);
        let zero = shared.ct_eq(&[0; 32]);
        let mut p = [0xff; 32];
        p[0] = 0xed;
        p[31] = 0x7f;
        Self::new(std::array::from_fn(|i| {
            u8::conditional_select(&shared[i], &p[i], zero)
        }))
    }

    fn key(&self, salt: &[u8]) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(self.shared);
        hash.update(salt);
        hash.update(b"Key");
        hash.finalize().into()
    }

    fn iv(&self, salt: &[u8]) -> [u8; 16] {
        let mut hash = Sha256::new();
        hash.update(self.shared);
        hash.update(salt);
        hash.update(b"IV");
        fold(&hash.finalize().into())
    }

    pub fn encrypt_v1(&self, salt: &[u8; 8], data: &[u8]) -> Vec<u8> {
        let aes = Aes256::new((&self.key(salt)).into());
        let padding = 16 - data.len() % 16;
        let mut ciphertext = data.to_vec();
        ciphertext.resize(data.len() + padding, padding as u8);
        let mut previous = self.iv(salt);
        for chunk in ciphertext.as_chunks_mut::<16>().0 {
            let mut block = Block::<Aes256>::clone_from_slice(chunk);
            for (b, p) in block.iter_mut().zip(previous) {
                *b ^= p;
            }
            aes.encrypt_block(&mut block);
            chunk.copy_from_slice(&block);
            previous = block.into();
        }
        let mut mac = Block::<Aes256>::from(fold(&Sha256::digest(&ciphertext).into()));
        aes.encrypt_block(&mut mac);
        let mut out = salt.to_vec();
        out.extend_from_slice(&ciphertext);
        out.extend_from_slice(&mac);
        out
    }

    pub fn decrypt_v1(&self, data: &[u8]) -> Result<Vec<u8>> {
        ensure!(data.len() >= 40 && data.len() % 16 == 8, "invalid V1 frame");
        let salt = &data[..8];
        let (ciphertext, tag) = data[8..].split_at(data.len() - 24);
        let aes = Aes256::new((&self.key(salt)).into());
        let mut mac = Block::<Aes256>::from(fold(&Sha256::digest(ciphertext).into()));
        aes.encrypt_block(&mut mac);
        ensure!(equal(&mac, tag), "V1 authentication failed");
        let mut out = ciphertext.to_vec();
        let mut previous = self.iv(salt);
        for chunk in out.as_chunks_mut::<16>().0 {
            let next = *chunk;
            let mut block = Block::<Aes256>::clone_from_slice(chunk);
            aes.decrypt_block(&mut block);
            for ((b, c), p) in chunk.iter_mut().zip(block).zip(previous) {
                *b = c ^ p;
            }
            previous = next;
        }
        let padding = *out.last().unwrap() as usize;
        ensure!(
            (1..=16).contains(&padding)
                && out[out.len() - padding..]
                    .iter()
                    .all(|b| *b as usize == padding),
            "invalid V1 padding"
        );
        out.truncate(out.len() - padding);
        Ok(out)
    }

    pub fn encrypt_v2(
        &self,
        context: &MessageContext<'_>,
        salt: &[u8; 16],
        sequence: u64,
        random: &[u8; 4],
        data: &[u8],
    ) -> Vec<u8> {
        let mut nonce = [0; 12];
        nonce[..8].copy_from_slice(&sequence.to_be_bytes());
        nonce[8..].copy_from_slice(random);
        let mut out = salt.to_vec();
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&gcm_encrypt(&self.key(salt), &nonce, &context.aad(), data));
        out
    }

    /// Context uses decoded lowercase MIDs; opaque Chrome IDs need the WASM adapter.
    pub fn decrypt_v2(&self, context: &MessageContext<'_>, data: &[u8]) -> Result<Vec<u8>> {
        ensure!(data.len() >= 44, "V2 ciphertext too short");
        gcm_decrypt(
            &self.key(&data[..16]),
            data[16..28].try_into().unwrap(),
            &context.aad(),
            &data[28..],
        )
    }
}

#[cfg(test)]
mod tests;
