//! Cancellable SSE transport. Credentials never enter arguments or diagnostics.
use super::*;
use crate::client::http::CancelToken;
use std::{
    io::{BufRead, BufReader, Read},
    sync::{Arc, mpsc},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncState {
    Connected,
    Reconnecting { attempt: u32 },
}
#[derive(Debug)]
pub enum Event {
    State(SyncState),
    Failure {
        attempt: u32,
        error: crate::client::error::Failure,
    },
    Operation(Value),
    Resync {
        next_revision: Option<String>,
        complete: mpsc::SyncSender<std::result::Result<(), crate::client::error::Failure>>,
    },
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
    Duration::from_secs(2u64.saturating_pow(attempt.clamp(1, 6)).min(60))
}
fn pause(stop: &CancelToken, duration: Duration) {
    async_io::block_on(futures_lite::future::or(stop.cancelled(), async {
        async_io::Timer::after(duration).await;
    }));
}

/// The stream has its own HTTP connection and borrows authorization from the RPC owner.
/// Neither credentials nor a WASM runtime are loaded on the stream thread.
pub fn receive_with_auth(
    client_version: &str,
    revision: u64,
    stop: &Arc<CancelToken>,
    authorize: impl FnMut(bool) -> Result<String>,
    deliver: impl FnMut(Event) -> bool,
) -> Result<()> {
    let transport = http::Transport::new(client_version)?;
    receive_on(
        &transport,
        client_version,
        revision,
        stop,
        authorize,
        deliver,
        pause,
    )
}

fn receive_on(
    transport: &http::Transport,
    client_version: &str,
    revision: u64,
    stop: &Arc<CancelToken>,
    mut authorize: impl FnMut(bool) -> Result<String>,
    mut deliver: impl FnMut(Event) -> bool,
    mut wait: impl FnMut(&CancelToken, Duration),
) -> Result<()> {
    set_request_cancel(stop.clone());
    let mut revisions = Revisions { local: revision };
    let mut refreshed_once = false;
    let mut attempt = 0;
    let mut close_delay = Duration::from_secs(3);
    while !stop.is_cancelled() {
        let mut delivered = false;
        let mut reconnect = false;
        let mut result = stream(
            transport,
            client_version,
            &authorize(false)?,
            revisions.local,
            stop,
            PING_TIMEOUT,
            |name, data| {
                delivered |= name != "open";
                reconnect |= name == "reconnect";
                if matches!(name, "message" | "ping") {
                    refreshed_once = false;
                }
                dispatch(name, data, &mut revisions, &mut attempt, stop, &mut deliver)
            },
        );
        if delivered {
            close_delay = Duration::from_secs(3);
        }
        if stop.is_cancelled() {
            break;
        }
        if crate::client::error::needs_refresh(&result) {
            if refreshed_once {
                deliver(Event::Failure {
                    attempt,
                    error: crate::client::error::Failure::SignedOut(
                        crate::client::error::SignOutReason::TokenRefresh,
                    ),
                });
                return Ok(());
            }
            match authorize(true) {
                Ok(_) => {
                    refreshed_once = true;
                    continue;
                }
                Err(error) => result = Err(error),
            }
        }
        if let Err(error) = result {
            attempt = attempt.saturating_add(1);
            let error = crate::client::error::classify(&error);
            let terminal = matches!(error, crate::client::error::Failure::SignedOut(_));
            if !deliver(Event::Failure { attempt, error }) || terminal {
                break;
            }
            wait(stop, backoff(attempt));
        } else if !reconnect {
            wait(stop, close_delay);
            if !delivered {
                close_delay = (close_delay * 2).min(Duration::from_secs(30));
            }
        }
        if !stop.is_cancelled() && !deliver(Event::State(SyncState::Reconnecting { attempt })) {
            break;
        }
    }
    Ok(())
}

fn dispatch(
    name: &str,
    data: Value,
    revisions: &mut Revisions,
    attempt: &mut u32,
    stop: &CancelToken,
    deliver: &mut impl FnMut(Event) -> bool,
) -> Result<bool> {
    if name == "message"
        && number(&data["revision"]).is_some()
        && !deliver(Event::State(SyncState::Connected))
    {
        return Ok(false);
    }
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
                if stop.is_cancelled() {
                    return Ok(false);
                }
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(Ok(())) => break,
                    Ok(Err(error)) => return Err(error.into()),
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return Err(crate::client::error::Failure::Bug(
                            "resync worker disconnected".into(),
                        )
                        .into());
                    }
                    Err(_) => {}
                }
            }
            if let Some(next) = next.and_then(|v| v.parse().ok()) {
                revisions.local = next;
            }
            return Ok(false);
        }
        "connInfoRevision" if number(&data).is_some_and(|r| r > revisions.local) => {
            let (tx, _rx) = mpsc::sync_channel(1);
            if !deliver(Event::Resync {
                next_revision: None,
                complete: tx,
            }) {
                return Ok(false);
            }
        }
        "ping" => {
            *attempt = 0;
            if !deliver(Event::State(SyncState::Connected)) {
                return Ok(false);
            }
        }
        "reconnect" => return Ok(false),
        "talkException" => {
            return Err(RpcError {
                gateway: 0,
                method: "SSE".into(),
                http: 200,
                code: data["code"].as_i64().context("talk exception code")?,
                status: 0,
            }
            .into());
        }
        "open" if stop.is_cancelled() || !deliver(Event::State(SyncState::Connected)) => {
            return Ok(false);
        }
        _ => {}
    }
    Ok(true)
}

const MAX_LINE: u64 = 1024 * 1024;
const PING_TIMEOUT: Duration = Duration::from_secs(30);

#[allow(clippy::too_many_arguments)]
fn stream(
    transport: &http::Transport,
    client_version: &str,
    access: &str,
    revision: u64,
    stop: &Arc<CancelToken>,
    ping_timeout: Duration,
    mut event: impl FnMut(&str, Value) -> Result<bool>,
) -> Result<u16> {
    // Source uses EventSource withCredentials, not the signed POST mapper.
    transport.stream(
        &format!(
            "/api/operation/receive?localRev={revision}&version={client_version}&language=en_US"
        ),
        &[
            ("Cookie", &format!("lct={access}")),
            ("Accept", "text/event-stream"),
        ],
        stop.clone(),
        ping_timeout,
        |status, _headers, body| {
            if status != 200 {
                return Err(RpcError {
                    gateway: 0,
                    method: "SSE".into(),
                    http: status,
                    code: 0,
                    status: 0,
                }
                .into());
            }
            if stop.is_cancelled() || !event("open", Value::Null)? {
                return Ok(status);
            }
            let mut reader = BufReader::new(body);
            let mut decoder = Decoder::default();
            let mut line = Vec::new();
            let mut discarding = false;
            while !stop.is_cancelled() {
                line.clear();
                let count = reader
                    .by_ref()
                    .take(MAX_LINE + 1)
                    .read_until(b'\n', &mut line)?;
                if line.len() as u64 > MAX_LINE {
                    decoder = Decoder::default();
                    discarding = true;
                    continue;
                }
                if count == 0 || stop.is_cancelled() {
                    break;
                }
                let line = String::from_utf8_lossy(&line);
                if discarding {
                    if line.trim_end_matches(['\r', '\n']).is_empty() {
                        discarding = false;
                    }
                    continue;
                }
                let frame = match decoder.line(line.trim_end_matches(['\r', '\n'])) {
                    Ok(frame) => frame,
                    Err(error) => {
                        crate::reporting::log("stream-frame", "decode", error.as_ref());
                        decoder = Decoder::default();
                        discarding = !line.trim_end_matches(['\r', '\n']).is_empty();
                        continue;
                    }
                };
                if let Some((name, data)) = frame {
                    if name == "ping" {
                        reader.get_mut().ping();
                    }
                    match event(&name, data) {
                        Ok(true) => {}
                        Ok(false) => break,
                        Err(error) => return Err(error),
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
        assert_eq!(backoff(30), Duration::from_secs(60));
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    #[test]
    fn fake_transport_orders_replay_catchup_and_named_sync() -> Result<()> {
        let stop = CancelToken::new();
        let mut r = Revisions { local: 10 };
        let mut attempt = 4;
        let mut seen = Vec::new();
        let mut consumer = |event| {
            match event {
                Event::Operation(v) => seen.push(number(&v["revision"]).unwrap()),
                Event::Resync { complete, .. } => {
                    complete.send(Ok(())).unwrap();
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
                        complete.send(Ok(())).unwrap();
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
        let stop = CancelToken::new();
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
                        complete
                            .send(Err(crate::client::error::Failure::Unanswered))
                            .unwrap();
                    }
                    true
                }
            )
            .is_err()
        );
        assert_eq!(r.local, 9);
        stop.cancel();
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
mod stream_tests;

#[cfg(test)]
mod reconnect_tests {
    use super::*;

    #[test]
    fn open_and_unchanged_revision_do_not_request_resync() -> Result<()> {
        let stop = CancelToken::new();
        let mut revisions = Revisions { local: 12 };
        let mut attempt = 0;
        let mut resyncs = 0;
        for (name, data) in [
            ("open", Value::Null),
            ("connInfoRevision", json!(12)),
            ("reconnect", Value::Null),
            ("open", Value::Null),
            ("connInfoRevision", json!("12")),
            ("connInfoRevision", json!(11)),
            ("connInfoRevision", json!(13)),
        ] {
            dispatch(
                name,
                data,
                &mut revisions,
                &mut attempt,
                &stop,
                &mut |event| {
                    if let Event::Resync { .. } = event {
                        resyncs += 1;
                    }
                    true
                },
            )?;
        }
        assert_eq!(resyncs, 1);
        assert_eq!(revisions.local, 12);
        Ok(())
    }
}
