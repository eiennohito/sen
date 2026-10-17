//! Cancellable SSE transport. Credentials never enter arguments or diagnostics.
use super::*;
use std::{
    io::{BufRead, BufReader, Read},
    sync::{
        Arc,
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
/// The same stop token is installed for RPCs and passed directly to socket waits.
pub fn receive(
    state_path: &Path,
    wasm_path: &Path,
    stop: &Arc<AtomicBool>,
    mut deliver: impl FnMut(Event) -> bool,
) -> Result<()> {
    set_request_cancel(stop.clone());
    let mut state: Value = serde_json::from_slice(&fs::read(state_path)?)?;
    let mut api = Api::new(&fs::read(wasm_path)?)?;
    // A separate agent keeps the long-lived stream out of the RPC pool.
    let transport = http::Transport::new()?;
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
                        if refreshed_once {
                            deliver(Event::State(SyncState::AuthExpired));
                            return Ok(());
                        }
                        match refresh(&mut api, &mut state, state_path, stop, &mut deliver) {
                            Ok(()) => refreshed_once = true,
                            Err(error) if expired_refresh(&error) => {
                                deliver(Event::State(SyncState::AuthExpired));
                                return Ok(());
                            }
                            Err(_) => {
                                attempt += 1;
                                if !deliver(Event::State(retry_state(attempt))) {
                                    break;
                                }
                                pause(stop, backoff(attempt));
                            }
                        }
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
        let result = stream(
            &transport,
            access(&state)?,
            revisions.local,
            stop,
            PING_TIMEOUT,
            |name, data| {
                if name == "open" {
                    refreshed_once = false;
                }
                dispatch(name, data, &mut revisions, &mut attempt, stop, &mut deliver)
            },
        );
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if result
            .as_ref()
            .err()
            .is_some_and(|error| error.downcast_ref::<ExpiredSession>().is_some())
        {
            deliver(Event::State(SyncState::AuthExpired));
            return Ok(());
        }
        if matches!(result, Ok(401 | 403))
            || result.as_ref().err().is_some_and(|error| {
                error
                    .downcast_ref::<RpcError>()
                    .is_some_and(|e| e.code == 119)
            })
        {
            if refreshed_once {
                deliver(Event::State(SyncState::AuthExpired));
                return Ok(());
            }
            match refresh(&mut api, &mut state, state_path, stop, &mut deliver) {
                Ok(()) => {
                    refreshed_once = true;
                    continue;
                }
                Err(error) if expired_refresh(&error) => {
                    deliver(Event::State(SyncState::AuthExpired));
                    return Ok(());
                }
                Err(_) => {}
            }
        }
        attempt += 1;
        if !deliver(Event::State(retry_state(attempt))) {
            break;
        }
        pause(stop, backoff(attempt));
    }
    Ok(())
}
#[derive(Debug)]
struct ExpiredSession;
impl std::fmt::Display for ExpiredSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Session authorization expired")
    }
}
impl std::error::Error for ExpiredSession {}
fn expired_refresh(error: &anyhow::Error) -> bool {
    error.downcast_ref::<ExpiredSession>().is_some()
        || error.downcast_ref::<RpcError>().is_some_and(|e| {
            ((400..500).contains(&e.http) && !matches!(e.http, 408 | 429)) || e.code == 10201
        })
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
    let token = state["token_v3_issue_result"]["refreshToken"]
        .as_str()
        .filter(|token| !token.is_empty())
        .ok_or(ExpiredSession)?;
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
            *attempt = 0;
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
        "ping" => *attempt = 0,
        "reconnect" => return Ok(false),
        "talkException" => match data["code"].as_u64() {
            Some(1 | 7 | 8) => return Err(ExpiredSession.into()),
            Some(119) => {
                return Err(RpcError {
                    method: "SSE".into(),
                    http: 401,
                    code: 119,
                    status: 0,
                }
                .into());
            }
            _ => return Err(anyhow::anyhow!("sync talk exception")),
        },
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
            if !deliver(Event::State(SyncState::Connected)) {
                return Ok(false);
            }
        }
        _ => {}
    }
    Ok(true)
}

const MAX_LINE: u64 = 1024 * 1024;
const PING_TIMEOUT: Duration = Duration::from_secs(30);

fn stream(
    transport: &http::Transport,
    access: &str,
    revision: u64,
    stop: &Arc<AtomicBool>,
    ping_timeout: Duration,
    mut event: impl FnMut(&str, Value) -> Result<bool>,
) -> Result<u16> {
    // Source uses EventSource withCredentials, not the signed POST mapper.
    transport.stream(
        &format!("/api/operation/receive?localRev={revision}&version={VERSION}&language=en_US"),
        &[
            ("Cookie", &format!("lct={access}")),
            ("Accept", "text/event-stream"),
        ],
        stop.clone(),
        ping_timeout,
        |status, _headers, body| {
            if status != 200 {
                return Ok(status);
            }
            if stop.load(Ordering::Relaxed) || !event("open", Value::Null)? {
                return Ok(status);
            }
            let mut reader = BufReader::new(body);
            let mut decoder = Decoder::default();
            let mut line = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                line.clear();
                let count = reader
                    .by_ref()
                    .take(MAX_LINE + 1)
                    .read_until(b'\n', &mut line)?;
                ensure!(line.len() as u64 <= MAX_LINE, "SSE line too large");
                if count == 0 || stop.load(Ordering::Relaxed) {
                    break;
                }
                let line = String::from_utf8_lossy(&line);
                if let Some((name, data)) = decoder.line(line.trim_end_matches(['\r', '\n']))? {
                    if name == "ping" {
                        reader.get_mut().ping();
                    }
                    if !event(&name, data)? {
                        break;
                    }
                }
            }
            Ok(status)
        },
    )
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

#[cfg(test)]
mod auth_tests {
    use super::*;
    #[test]
    fn refresh_transport_outage_is_not_expired_authorization() {
        for http in [408, 429, 500, 503] {
            assert!(!expired_refresh(
                &RpcError {
                    method: "refresh".into(),
                    http,
                    code: 0,
                    status: 0
                }
                .into()
            ));
        }
        for http in [400, 401, 403] {
            assert!(expired_refresh(
                &RpcError {
                    method: "refresh".into(),
                    http,
                    code: 0,
                    status: 0
                }
                .into()
            ));
        }
        assert!(!expired_refresh(&anyhow::anyhow!("timeout")));
        assert!(expired_refresh(&ExpiredSession.into()));
        let error = dispatch(
            "talkException",
            json!({"code":8}),
            &mut Revisions::default(),
            &mut 0,
            &AtomicBool::new(false),
            &mut |_| true,
        )
        .unwrap_err();
        assert!(error.downcast_ref::<ExpiredSession>().is_some());
    }
}

#[cfg(test)]
mod stream_tests;
