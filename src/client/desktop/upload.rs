use super::*;
use crate::attachments::{Attachment, FILE_LIMIT};

impl Client {
    pub fn prepare_attachment(
        &mut self,
        to: &str,
        mut item: Attachment,
        seq: u32,
    ) -> Result<impl FnOnce(&mut dyn FnMut(f32)) -> Result<Value> + Send + 'static> {
        ensure!(
            seq > 0 && seq <= 0x7fff_ffff,
            "invalid persisted send sequence"
        );
        let token_response = self.talk("acquireEncryptedAccessToken", json!([2]))?;
        let token = token_response
            .as_str()
            .context("missing encrypted access token")?;
        let token = token
            .split('\x1e')
            .nth(1)
            .and_then(|s| s.split('\x1f').next())
            .filter(|s| !s.is_empty())
            .context("missing media access token")?
            .to_string();
        let content_type = if item.photo.is_some() { 1 } else { 14 };
        let flow = self.talk("determineMediaMessageFlow", json!([{"chatMid":to}]))?;
        ensure!(
            flow["flowMap"][content_type.to_string()] == 2,
            "unsupported media flow"
        );
        let http = self.api.http.assets();
        Ok(move |progress: &mut dyn FnMut(f32)| {
            let original = item.read(FILE_LIMIT)?;
            let mut material = [0_u8; 32];
            use std::io::Read;
            fs::File::open("/dev/urandom")?.read_exact(&mut material)?;
            let encrypted = assets::encrypt_media(original, &material)?;
            let preview = if item.photo.is_some() {
                assets::encrypt_media(std::mem::take(&mut item.preview), &material)?
            } else {
                vec![]
            };
            let sid = if content_type == 1 { "emi" } else { "emf" };
            let mut uuid = [0u8; 16];
            fs::File::open("/dev/urandom")?.read_exact(&mut uuid)?;
            uuid[6] = (uuid[6] & 15) | 0x40;
            uuid[8] = (uuid[8] & 63) | 0x80;
            let hex: String = uuid.iter().map(|b| format!("{b:02x}")).collect();
            let request_id = format!(
                "reqid-{}-{}-{}-{}-{}",
                &hex[..8],
                &hex[8..12],
                &hex[12..16],
                &hex[16..20],
                &hex[20..]
            );
            let name = SystemTime::now()
                .duration_since(UNIX_EPOCH)?
                .as_millis()
                .to_string();
            let params = |name: &str| {
                STANDARD.encode(
                    serde_json::to_vec(&json!({"ver":"2.0","name":name,"type":"file"}))
                        .expect("JSON"),
                )
            };
            let application = format!("CHROMEOS\t{}\tChrome_OS\t", http.client_version);
            let total = (encrypted.len() + preview.len()) as f32;
            progress(0.);
            let oid = http.upload(
                &format!("https://obs.line-apps.com/r/talk/{sid}/{request_id}"),
                &[
                    ("x-obs-params", &params(&name)),
                    ("x-line-access", &token),
                    ("x-line-application", &application),
                ],
                &encrypted,
                &mut |sent| progress((sent as f32 / total).min(0.99)),
            )?;
            ensure!(
                !oid.is_empty()
                    && oid.len() <= 256
                    && oid
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
                "missing or invalid upload object ID"
            );
            if !preview.is_empty() {
                http.upload(
                    &format!("https://obs.line-apps.com/r/talk/{sid}/{oid}__ud-preview"),
                    &[
                        ("x-obs-params", &params(&format!("{name}.png"))),
                        ("x-line-access", &token),
                        ("x-line-application", &application),
                    ],
                    &preview,
                    &mut |sent| {
                        progress(((encrypted.len() as u64 + sent) as f32 / total).min(0.99))
                    },
                )?;
            }
            let mut message = json!({"keyMaterial": STANDARD.encode(material), "fileName": item.name,
            "contentType": content_type, "contentMetadata": {}});
            let meta = &mut message["contentMetadata"];
            meta["SID"] = json!(sid);
            meta["OID"] = json!(oid);
            meta["FILE_SIZE"] = json!(item.size.to_string());
            if let Some(photo) = &item.photo {
                meta["MEDIA_CONTENT_INFO"] = json!(json!({"category":"original", "fileSize":item.size,"extension":photo.extension,"animated":photo.animated,"width":photo.width,"height":photo.height}).to_string());
                meta["MEDIA_THUMB_INFO"] = json!(
                    json!({"width":photo.preview_width,"height":photo.preview_height}).to_string()
                );
            }
            progress(1.);
            Ok(message)
        })
    }
    pub fn finish_attachment(&mut self, to: &str, uploaded: Value, seq: u32) -> Result<Message> {
        self.send_encrypted(
            to,
            uploaded["contentType"].as_u64().context("media type")?,
            &json!({"keyMaterial":uploaded["keyMaterial"], "fileName":uploaded["fileName"]}),
            &uploaded["contentMetadata"],
            seq,
        )
    }
}
