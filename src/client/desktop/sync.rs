//! Cancellable SSE transport. Credentials never enter arguments or diagnostics.
use super::*;
use std::{
    io::{BufRead, BufReader, Read},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncState {
    Connected,
    Reconnecting { attempt: u32 },
    Offline,
    AuthExpired,
}
#[derive(Debug)]
pub enum Event {
    State(SyncState),
    Operation(Value),
    Resync {
        next_revision: Option<String>,
        complete: mpsc::SyncSender<bool>,
    },
    Token(Value),
}
#[derive(Default)]
struct Decoder {
    event: String,
    data: String,
}
impl Decoder {
    fn line(&mut self, line: &str) -> Result<Option<(String, Value)>> {
        if line.is_empty() {
            let name = std::mem::take(&mut self.event);
            let data = std::mem::take(&mut self.data);
            if data.is_empty() {
                return Ok((!name.is_empty()).then_some((name, Value::Null)));
            }
            return Ok(Some((
                if name.is_empty() {
                    "message".into()
                } else {
                    name
                },
                serde_json::from_str(data.trim_end())?,
            )));
        }
        if line.starts_with(':') {
            return Ok(None);
        }
        let (key, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match key {
            "event" => self.event = value.into(),
            "data" => {
                ensure!(
                    self.data.len() + value.len() < 1024 * 1024,
                    "SSE frame too large"
                );
                self.data.push_str(value);
                self.data.push('\n');
            }
            _ => {}
        }
        Ok(None)
    }
}
/// Revision is advanced only after the bounded consumer has accepted the event.
#[derive(Default)]
pub struct Revisions {
    pub local: u64,
}
impl Revisions {
    pub fn accepts(&self, operation: &Value) -> bool {
        number(&operation["revision"]).is_some_and(|r| r > self.local)
    }
    pub fn commit(&mut self, operation: &Value) {
        if let Some(r) = number(&operation["revision"]) {
            self.local = self.local.max(r);
        }
    }
}
fn number(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}
pub fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(2u64.saturating_pow(attempt.min(10)).min(600))
}
fn pause(stop: &AtomicBool, duration: Duration) {
    let end = Instant::now() + duration;
    while !stop.load(Ordering::Relaxed) && Instant::now() < end {
        thread::park_timeout(Duration::from_millis(50));
    }
}

/// Runs on a dedicated thread with its own WASM runtime. `deliver` must return
/// false on cancellation; callers use a bounded channel with cancellable sends.
pub fn receive(
    state_path: &Path,
    wasm_path: &Path,
    stop: &AtomicBool,
    mut deliver: impl FnMut(Event) -> bool,
) -> Result<()> {
    let mut state: Value = serde_json::from_slice(&fs::read(state_path)?)?;
    let mut api = Api::new(&fs::read(wasm_path)?)?;
    let mut revisions = Revisions::default();
    let mut seeded = false;
    let mut refreshed_once = false;
    let mut attempt = 0;
    while !stop.load(Ordering::Relaxed) {
        if !seeded {
            match chat::talk(&mut api, access(&state)?, "getLastOpRevision", json!([])) {
                Ok(revision) => {
                    revisions.local = number(&revision).context("missing operation revision")?;
                    seeded = true;
                }
                Err(error) => {
                    if error
                        .downcast_ref::<RpcError>()
                        .is_some_and(|e| matches!(e.http, 401 | 403))
                    {
                        if refreshed_once
                            || refresh(&mut api, &mut state, state_path, stop, &mut deliver)
                                .is_err()
                        {
                            deliver(Event::State(SyncState::AuthExpired));
                            return Ok(());
                        }
                        refreshed_once = true;
                    } else {
                        attempt += 1;
                        if !deliver(Event::State(retry_state(attempt))) {
                            break;
                        }
                        pause(stop, backoff(attempt));
                    }
                    continue;
                }
            }
        }
        let result = stream(access(&state)?, revisions.local, stop, |name, data| {
            if name == "open" {
                refreshed_once = false;
            }
            dispatch(name, data, &mut revisions, &mut attempt, stop, &mut deliver)
        });
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if matches!(result, Ok(401 | 403)) {
            if refreshed_once
                || refresh(&mut api, &mut state, state_path, stop, &mut deliver).is_err()
            {
                deliver(Event::State(SyncState::AuthExpired));
                return Ok(());
            }
            refreshed_once = true;
            continue;
        }
        attempt += 1;
        if !deliver(Event::State(retry_state(attempt))) {
            break;
        }
        pause(stop, backoff(attempt));
    }
    Ok(())
}
fn retry_state(attempt: u32) -> SyncState {
    if attempt >= 5 {
        SyncState::Offline
    } else {
        SyncState::Reconnecting { attempt }
    }
}
fn refresh(
    api: &mut Api,
    state: &mut Value,
    state_path: &Path,
    stop: &AtomicBool,
    deliver: &mut impl FnMut(Event) -> bool,
) -> Result<()> {
    let token = required(&state["token_v3_issue_result"], "refreshToken")?;
    let value = api.call(
        "/api/auth/tokenRefresh",
        &json!({"refreshToken":token}),
        access(state)?,
        "",
        0,
    )?;
    ensure!(
        value["accessToken"].as_str().is_some_and(|s| !s.is_empty()),
        "refresh missing access token"
    );
    state["token_v3_issue_result"]
        .as_object_mut()
        .context("saved token")?
        .extend(value.as_object().context("refresh token response")?.clone());
    if stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    save_state(state_path, state)?;
    deliver(Event::Token(state["token_v3_issue_result"].clone()));
    Ok(())
}

fn dispatch(
    name: &str,
    data: Value,
    revisions: &mut Revisions,
    attempt: &mut u32,
    stop: &AtomicBool,
    deliver: &mut impl FnMut(Event) -> bool,
) -> Result<bool> {
    match name {
        "message" if revisions.accepts(&data) => {
            if !deliver(Event::Operation(data.clone())) {
                return Ok(false);
            }
            revisions.commit(&data);
        }
        "fullSync" | "partialFullSync" => {
            let next = data["nextRevision"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| number(&data["nextRevision"]).map(|v| v.to_string()));
            let (tx, rx) = mpsc::sync_channel(1);
            if !deliver(Event::Resync {
                next_revision: next.clone(),
                complete: tx,
            }) {
                return Ok(false);
            }
            loop {
                if stop.load(Ordering::Relaxed) {
                    return Ok(false);
                }
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(true) => break,
                    Ok(false) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return Err(anyhow::anyhow!("resync failed"));
                    }
                    Err(_) => {}
                }
            }
            if let Some(next) = next.and_then(|v| v.parse().ok()) {
                revisions.local = next;
            }
            return Ok(false);
        }
        "connInfoRevision" => {
            let (tx, _rx) = mpsc::sync_channel(1);
            if !deliver(Event::Resync {
                next_revision: None,
                complete: tx,
            }) {
                return Ok(false);
            }
        }
        "reconnect" => return Ok(false),
        "talkException" => return Err(anyhow::anyhow!("sync talk exception")),
        "open" => {
            let (tx, rx) = mpsc::sync_channel(1);
            if !deliver(Event::Resync {
                next_revision: None,
                complete: tx,
            }) {
                return Ok(false);
            }
            loop {
                if stop.load(Ordering::Relaxed) {
                    return Ok(false);
                }
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(true) => break,
                    Ok(false) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return Err(anyhow::anyhow!("catch-up failed"));
                    }
                    Err(_) => {}
                }
            }
            *attempt = 0;
            if !deliver(Event::State(SyncState::Connected)) {
                return Ok(false);
            }
        }
        _ => {}
    }
    Ok(true)
}

fn stream(
    access: &str,
    revision: u64,
    stop: &AtomicBool,
    mut event: impl FnMut(&str, Value) -> Result<bool>,
) -> Result<u16> {
    // Source uses EventSource withCredentials, not the signed POST mapper.
    let config = format!(
        "url = {}\nrequest = \"GET\"\nsilent\nshow-error\nno-buffer\ninclude\nconnect-timeout = 10\nheader = {}\nheader = \"Accept: text/event-stream\"\n",
        quoted(&format!(
            "{BASE}/api/operation/receive?localRev={revision}&version={VERSION}&language=en_US"
        )),
        quoted(&format!("Cookie: lct={access}"))
    );
    let mut child = Command::new("curl")
        .args(["--config", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    child
        .stdin
        .take()
        .context("SSE stdin")?
        .write_all(config.as_bytes())?;
    let stdout = child.stdout.take().context("SSE stdout")?;
    let (tx, rx) = mpsc::sync_channel(16);
    let reader = thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            // Bound even a malicious unterminated line.
            let mut line = Vec::new();
            let result = reader
                .by_ref()
                .take(1024 * 1024 + 1)
                .read_until(b'\n', &mut line);
            if line.len() > 1024 * 1024 || !matches!(result, Ok(n) if n > 0) {
                break;
            }
            if tx
                .send(
                    String::from_utf8_lossy(&line)
                        .trim_end_matches(['\r', '\n'])
                        .to_owned(),
                )
                .is_err()
            {
                break;
            }
        }
    });
    let result = (|| {
        let mut decoder = Decoder::default();
        let mut status = 0;
        let mut headers = true;
        let mut ping = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            let line = match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(line) => line,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    ensure!(ping.elapsed() < Duration::from_secs(30), "SSE ping expired");
                    continue;
                }
                Err(_) => break,
            };
            if headers {
                if line.starts_with("HTTP/") {
                    status = line
                        .split_whitespace()
                        .nth(1)
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                }
                if line.is_empty() {
                    if status == 200 {
                        headers = false;
                        if !event("open", Value::Null)? {
                            break;
                        }
                    } else if status != 100 {
                        return Ok(status);
                    }
                }
            } else if let Some((name, data)) = decoder.line(&line)? {
                if name == "ping" {
                    ping = Instant::now();
                }
                if !event(&name, data)? {
                    break;
                }
            }
        }
        Ok(status)
    })();
    let _ = child.kill();
    let _ = child.wait();
    drop(rx);
    let _ = reader.join();
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frames_and_revision_replay() {
        let mut d = Decoder::default();
        assert!(d.line(": comment").unwrap().is_none());
        d.line("data: {\"revision\":\"12\",\"type\":26}").unwrap();
        let (name, v) = d.line("").unwrap().unwrap();
        assert_eq!(name, "message");
        let mut r = Revisions { local: 11 };
        assert!(r.accepts(&v));
        r.commit(&v);
        assert!(!r.accepts(&v));
        assert!(!r.accepts(&json!({"revision":"10"})));
        assert_eq!(backoff(30), Duration::from_secs(600));
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    #[test]
    fn fake_transport_orders_replay_catchup_and_named_sync() -> Result<()> {
        let stop = AtomicBool::new(false);
        let mut r = Revisions { local: 10 };
        let mut attempt = 4;
        let mut seen = Vec::new();
        let mut consumer = |event| {
            match event {
                Event::Operation(v) => seen.push(number(&v["revision"]).unwrap()),
                Event::Resync { complete, .. } => {
                    complete.send(true).unwrap();
                }
                Event::State(s) => assert_eq!(s, SyncState::Connected),
                _ => {}
            }
            true
        };
        for (name, data) in [
            ("open", Value::Null),
            ("message", json!({"revision":"11"})),
            ("message", json!({"revision":"12"})),
            ("reconnect", Value::Null),
            ("message", json!({"revision":"11"})),
            ("message", json!({"revision":"13"})),
        ] {
            dispatch(name, data, &mut r, &mut attempt, &stop, &mut consumer)?;
        }
        assert_eq!(seen, vec![11, 12, 13]);
        assert_eq!(attempt, 0);
        for name in ["fullSync", "partialFullSync"] {
            assert!(!dispatch(
                name,
                json!({"nextRevision":"20"}),
                &mut r,
                &mut attempt,
                &stop,
                &mut |e| {
                    if let Event::Resync { complete, .. } = e {
                        complete.send(true).unwrap();
                    }
                    true
                }
            )?);
            assert_eq!(r.local, 20);
        }
        Ok(())
    }
    #[test]
    fn failed_full_sync_never_advances_and_cancellation_does_not_block() {
        let stop = AtomicBool::new(false);
        let mut r = Revisions { local: 9 };
        let mut attempt = 0;
        assert!(
            dispatch(
                "fullSync",
                json!({"nextRevision":"20"}),
                &mut r,
                &mut attempt,
                &stop,
                &mut |e| {
                    if let Event::Resync { complete, .. } = e {
                        complete.send(false).unwrap();
                    }
                    true
                }
            )
            .is_err()
        );
        assert_eq!(r.local, 9);
        stop.store(true, Ordering::Relaxed);
        let started = Instant::now();
        assert!(
            !dispatch(
                "open",
                Value::Null,
                &mut r,
                &mut attempt,
                &stop,
                &mut |_| true
            )
            .unwrap()
        );
        assert!(started.elapsed() < Duration::from_millis(100));
    }
}
