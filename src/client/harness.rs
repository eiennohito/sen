use super::e2ee::{Channel, channel, decrypt_body};
use super::*;

pub fn benchmark_history(
    wasm: &[u8],
    version: &str,
    fixture: &Value,
    to: &str,
    from: &str,
) -> Result<f64> {
    let mut api = Api::with_runtime(Runtime::new(wasm, version)?, wasm, version)?;
    let started = Instant::now();
    let h = api.rt.bytes(&decode(required(fixture, "exported_key")?)?);
    let key = api.rt.static_call(Call::Curve25519KeyLoadKey, &[h])?;
    api.rt.release(h)?;
    let ch = channel(
        &mut api.rt,
        key,
        &decode(required(fixture, "peer_public_key")?)?,
    )?;
    api.cache_channel(
        Channel::Direct {
            mid: from.into(),
            own_key_id: 99,
            peer_key_id: 42,
        },
        ch,
    )?;
    let ct = decode(required(fixture, "ciphertext")?)?;
    let mut failed = 0;
    for index in 0..50 {
        let receiver = if index % 10 == 0 { 1000u32 } else { 99 };
        let raw = json!({"to":to,"from":from,"contentType":0,"contentMetadata":{"e2eeVersion":"2"},"chunks":[
            STANDARD.encode(&ct[..16]), STANDARD.encode(&ct[ct.len()-16..]),
            STANDARD.encode(&ct[16..ct.len()-16]), STANDARD.encode(42u32.to_be_bytes()), STANDARD.encode(receiver.to_be_bytes())]});
        match decrypt_body(&mut api, to, &raw) {
            Ok(body) => ensure!(body["text"].is_string(), "missing fixture text"),
            Err(_) => {
                failed += 1;
            }
        }
    }
    ensure!(
        failed == 5 && api.channels.len() == 1,
        "item failures dropped cached channels"
    );
    Ok(started.elapsed().as_secs_f64() * 1000.)
}
