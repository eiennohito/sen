use super::*;
use crate::attachments::Attachment;

impl Client {
    #[expect(
        clippy::type_complexity,
        reason = "return the concrete upload closure without boxing"
    )]
    pub fn prepare_attachment<F: FnMut() -> Result<String> + Send + 'static>(
        &mut self,
        to: &str,
        mut item: Attachment,
        mut renew: F,
    ) -> Result<
        impl FnOnce(super::NetworkActivity, &mut dyn FnMut(f32)) -> Result<Value> + Send + use<F>,
    > {
        let mut token = self.media_access()?.to_owned();
        let content_type = if item.photo.is_some() { 1 } else { 14 };
        let flow = self.talk("determineMediaMessageFlow", json!([{"chatMid":to}]))?;
        ensure!(
            flow["flowMap"][content_type.to_string()] == 2,
            "unsupported media flow"
        );
        let mut http = self.api.http.assets();
        Ok(move |activity, progress: &mut dyn FnMut(f32)| {
            http.activity = activity;
            let original = item.read()?;
            let mut material = [0_u8; 32];
            crate::fill_random(&mut material)?;
            let encrypted = assets::encrypt_media(original, &material)?;
            let preview = if item.photo.is_some() {
                assets::encrypt_media(std::mem::take(&mut item.preview), &material)?
            } else {
                vec![]
            };
            let sid = if content_type == 1 { "emi" } else { "emf" };
            let mut uuid = [0u8; 16];
            crate::fill_random(&mut uuid)?;
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
            let encrypted_len = encrypted.len();
            let total = (encrypted_len + preview.len()) as f32;
            progress(0.);
            let oid = upload(
                &http,
                &format!("https://obs.line-apps.com/r/talk/{sid}/{request_id}"),
                &params(&name),
                encrypted,
                &mut token,
                &mut renew,
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
                upload(
                    &http,
                    &format!("https://obs.line-apps.com/r/talk/{sid}/{oid}__ud-preview"),
                    &params(&format!("{name}.png")),
                    preview,
                    &mut token,
                    &mut renew,
                    &mut |sent| progress(((encrypted_len as u64 + sent) as f32 / total).min(0.99)),
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
    pub fn finish_attachment(
        &mut self,
        to: &str,
        uploaded: Value,
        seq: u32,
    ) -> Result<MessageInput> {
        self.send_encrypted(
            to,
            uploaded["contentType"].as_u64().context("media type")?,
            &json!({"keyMaterial":uploaded["keyMaterial"], "fileName":uploaded["fileName"]}),
            &uploaded["contentMetadata"],
            seq,
            None,
        )
    }
}

fn upload(
    http: &http::Transport,
    url: &str,
    params: &str,
    bytes: Vec<u8>,
    token: &mut String,
    renew: &mut impl FnMut() -> Result<String>,
    progress: &mut dyn FnMut(u64),
) -> Result<String> {
    let bytes = bytes::Bytes::from(bytes);
    let application = format!("CHROMEOS\t{}\tChrome_OS\t", http.client_version);
    let send = |token: &str, progress: &mut dyn FnMut(u64)| {
        http.upload(
            url,
            &[
                ("x-obs-params", params),
                ("x-line-access", token),
                ("x-line-application", &application),
            ],
            bytes.clone(),
            progress,
        )
    };
    match send(token, progress) {
        Err(error)
            if error
                .downcast_ref::<RpcError>()
                .is_some_and(|e| e.http == 401) =>
        {
            *token = renew()?;
            send(token, progress)
        }
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::http::test_support::{accept, local_server};
    use std::io::{BufRead, BufReader, Read};

    #[test]
    fn unauthorized_upload_renews_and_retries_only_once() -> Result<()> {
        for second_status in [201, 401] {
            let (url_tx, url_rx) = std::sync::mpsc::channel();
            let (http, server) = local_server(move |listener| {
                url_tx
                    .send(format!("http://{}/upload", listener.local_addr().unwrap()))
                    .unwrap();
                for (status, token) in [(401, "stale"), (second_status, "fresh")] {
                    let mut socket = BufReader::new(accept(&listener));
                    let mut headers = String::new();
                    let mut len = 0;
                    loop {
                        let mut line = String::new();
                        socket.read_line(&mut line).unwrap();
                        if line == "\r\n" {
                            break;
                        }
                        if let Some(value) =
                            line.to_ascii_lowercase().strip_prefix("content-length:")
                        {
                            len = value.trim().parse().unwrap();
                        }
                        headers.push_str(&line.to_ascii_lowercase());
                    }
                    assert!(headers.contains(&format!("x-line-access: {token}\r\n")));
                    let mut body = vec![0; len];
                    socket.read_exact(&mut body).unwrap();
                    assert_eq!(body, b"encrypted media");
                    write!(socket.get_mut(), "HTTP/1.1 {status} Test\r\nx-obs-oid: object\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                }
            });
            let mut token = "stale".to_owned();
            let mut renewals = 0;
            let result = upload(
                &http,
                &url_rx.recv().unwrap(),
                "params",
                b"encrypted media".to_vec(),
                &mut token,
                &mut || {
                    renewals += 1;
                    Ok("fresh".into())
                },
                &mut |_| {},
            );
            assert_eq!(result.is_ok(), second_status == 201);
            assert_eq!(renewals, 1);
            assert_eq!(token, "fresh");
            server.join().unwrap();
        }
        Ok(())
    }
}
