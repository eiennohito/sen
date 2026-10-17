use super::{ACTIVE, CONNECT_BUDGET, POLL, check_request};
use std::{
    collections::HashMap,
    fmt, io,
    net::{SocketAddr, ToSocketAddrs},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};
use ureq::{
    config::Config,
    http::Uri,
    unversioned::{
        resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver},
        transport::NextTimeout,
    },
};

const DNS_TTL: Duration = Duration::from_secs(30);
type Lookup = dyn Fn(String) -> io::Result<Vec<SocketAddr>> + Send + Sync;

struct Pending {
    key: String,
    started: Instant,
    abandoned: bool,
    result: mpsc::Receiver<io::Result<Vec<SocketAddr>>>,
}

#[derive(Default)]
struct State {
    cache: HashMap<String, (Instant, ResolvedSocketAddrs)>,
    pending: Option<Pending>,
}

// One outstanding OS lookup per agent, even if multiple requests cancel while
// getaddrinfo is stuck. Keep its receiver for the next request instead of spawning
// another abandoned helper. Successful answers avoid DNS before pooled requests.
pub(super) struct PollResolver {
    lookup: Arc<Lookup>,
    name: &'static str,
    state: Mutex<State>,
}

impl fmt::Debug for PollResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PollResolver").finish_non_exhaustive()
    }
}

impl Default for PollResolver {
    fn default() -> Self {
        Self::new(Arc::new(|address| {
            address.to_socket_addrs().map(|iter| iter.collect())
        }))
    }
}

impl PollResolver {
    pub(super) fn named(name: &'static str) -> Self {
        Self {
            name,
            ..Self::default()
        }
    }

    pub(super) fn new(lookup: Arc<Lookup>) -> Self {
        Self {
            lookup,
            name: "line-rpc-dns",
            state: Mutex::new(State::default()),
        }
    }
}

impl Resolver for PollResolver {
    fn resolve(
        &self,
        uri: &Uri,
        _config: &Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        check_request()?;
        let key = uri
            .scheme()
            .zip(uri.authority())
            .and_then(|(scheme, authority)| DefaultResolver::host_and_port(scheme, authority))
            .ok_or(ureq::Error::HostNotFound)?;
        // Literal socket addresses need neither a helper nor a DNS cache entry.
        if let Ok(address) = key.parse::<SocketAddr>() {
            let mut addresses = self.empty();
            addresses.push(address);
            return Ok(addresses);
        }
        let phase_deadline = Instant::now().checked_add(*timeout.after);
        let mut state = self.state.lock().map_err(|_| ureq::Error::HostNotFound)?;
        if let Some(pending) = state.pending.as_mut() {
            // A previous resolve returned while its OS lookup was still running.
            pending.abandoned = true;
        }
        loop {
            check_request()?;
            super::check_phase(phase_deadline, timeout)?;
            state
                .cache
                .retain(|_, (expires, _)| Instant::now() < *expires);
            if let Some((_, addresses)) = state.cache.get(&key) {
                return Ok(addresses.clone());
            }
            if state.pending.is_none() {
                let (send, result) = mpsc::sync_channel(1);
                let lookup = self.lookup.clone();
                let address = key.clone();
                let started = Instant::now();
                thread::Builder::new()
                    .name(self.name.into())
                    .spawn(move || {
                        let _ = send.send(lookup(address));
                    })
                    .map_err(ureq::Error::Io)?;
                state.pending = Some(Pending {
                    key: key.clone(),
                    started,
                    abandoned: false,
                    result,
                });
            }
            let wait = ACTIVE.with(|slot| {
                let slot = slot.borrow();
                let request = slot.as_ref().unwrap();
                POLL.min(
                    request
                        .deadline
                        .min(request.connect_deadline)
                        .saturating_duration_since(Instant::now()),
                )
            });
            let pending = state.pending.as_ref().unwrap();
            match pending.result.recv_timeout(wait) {
                Ok(result) => {
                    let pending = state.pending.take().unwrap();
                    if pending.started.elapsed() >= CONNECT_BUDGET
                        || (pending.abandoned
                            && !result.as_ref().is_ok_and(|values| !values.is_empty()))
                    {
                        // Do not let an abandoned failure poison this request, or
                        // revive an answer older than a full connect budget.
                        continue;
                    }
                    let completed = pending.key;
                    match result {
                        Ok(values) if !values.is_empty() => {
                            let mut addresses = self.empty();
                            for address in values.into_iter().take(16) {
                                addresses.push(address);
                            }
                            // Production has only the gateway and optional proxy host.
                            if state.cache.len() >= 8 {
                                state.cache.clear();
                            }
                            state
                                .cache
                                .insert(completed, (pending.started + DNS_TTL, addresses));
                        }
                        result if completed == key => {
                            return Err(result
                                .err()
                                .map(ureq::Error::Io)
                                .unwrap_or(ureq::Error::HostNotFound));
                        }
                        _ => {}
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let pending = state.pending.take().unwrap();
                    if !pending.abandoned {
                        return Err(ureq::Error::HostNotFound);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{http::RequestScope, set_request_cancel};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[test]
    fn abandoned_dns_results_are_discarded_or_keep_original_ttl() {
        // Exercise a real cancelled resolver call, then advance only its stored
        // lookup timestamp to cover ageing without sleeping for eleven seconds.
        for (failure, age) in [(false, 11), (true, 11), (true, 1), (false, 1)] {
            let (started, ready) = mpsc::channel();
            let (release, wait) = mpsc::channel();
            let wait = Mutex::new(wait);
            let calls = Arc::new(AtomicUsize::new(0));
            let count = calls.clone();
            let old: SocketAddr = "127.0.0.1:1".parse().unwrap();
            let fresh: SocketAddr = "127.0.0.1:2".parse().unwrap();
            let resolver = Arc::new(PollResolver::new(Arc::new(move |_| {
                if count.fetch_add(1, Ordering::Relaxed) == 0 {
                    started.send(()).unwrap();
                    wait.lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(3))
                        .unwrap();
                    if failure {
                        Err(io::Error::from(io::ErrorKind::NotFound))
                    } else {
                        Ok(vec![old])
                    }
                } else {
                    Ok(vec![fresh])
                }
            })));
            let cancelled = Arc::new(AtomicBool::new(false));
            let signal = cancelled.clone();
            let first = resolver.clone();
            let worker = thread::spawn(move || {
                set_request_cancel(cancelled);
                let _scope = RequestScope::new(Duration::from_secs(30));
                first.resolve(
                    &"https://cache.test".parse().unwrap(),
                    &Config::default(),
                    next_timeout(),
                )
            });
            ready.recv_timeout(Duration::from_secs(2)).unwrap();
            signal.store(true, Ordering::Relaxed);
            assert!(
                matches!(worker.join().unwrap(), Err(ureq::Error::Io(error)) if error.kind() == io::ErrorKind::ConnectionAborted)
            );
            let lookup_start = Instant::now() - Duration::from_secs(age);
            resolver
                .state
                .lock()
                .unwrap()
                .pending
                .as_mut()
                .unwrap()
                .started = lookup_start;
            release.send(()).unwrap();
            let _scope = RequestScope::new(Duration::from_secs(30));
            let addresses = resolver
                .resolve(
                    &"https://cache.test".parse().unwrap(),
                    &Config::default(),
                    next_timeout(),
                )
                .unwrap();
            if failure || age > 10 {
                assert_eq!(addresses.as_ref(), &[fresh]);
                assert_eq!(calls.load(Ordering::Relaxed), 2);
            } else {
                assert_eq!(addresses.as_ref(), &[old]);
                assert_eq!(calls.load(Ordering::Relaxed), 1);
                assert_eq!(
                    resolver.state.lock().unwrap().cache["cache.test:443"].0,
                    lookup_start + DNS_TTL
                );
            }
        }
    }

    fn next_timeout() -> NextTimeout {
        NextTimeout {
            after: Duration::from_secs(30).into(),
            reason: ureq::Timeout::Global,
        }
    }
}
