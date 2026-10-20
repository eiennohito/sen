use super::*;
use crate::client::error::media_failure;
use crate::client::http::CancelToken;
use std::{
    sync::Mutex,
    thread::{self, JoinHandle},
    time::Instant,
};

const THREADS: usize = 3;
const QUEUE_LIMIT: usize = 256;
const FAILURE_TTL: Duration = Duration::from_secs(300);

#[derive(Clone, Default)]
pub struct Failures(Arc<Mutex<HashMap<String, Failed>>>, Arc<Mutex<String>>);
impl Failures {
    fn record(&self, key: String, class: Failure, job: Job, replace: bool) {
        let mut failed = self.0.lock().unwrap();
        if !replace && failed.contains_key(&key) {
            return;
        }
        if !failed.contains_key(&key) && failed.len() >= QUEUE_LIMIT {
            let oldest = failed
                .iter()
                .min_by_key(|(_, f)| f.at)
                .map(|(key, _)| key.clone())
                .unwrap();
            failed.remove(&oldest);
        }
        failed.insert(
            key,
            Failed {
                class,
                retry: class.retry_read(),
                at: Instant::now(),
                job,
            },
        );
    }

    pub(in crate::client::desktop) fn scope(&self, account: &str) {
        let mut scope = self.1.lock().unwrap();
        if *scope != account {
            self.0.lock().unwrap().clear();
            *scope = account.into();
        }
    }
}

#[derive(Clone)]
struct Failed {
    class: Failure,
    retry: bool,
    at: Instant,
    job: Job,
}

#[derive(Clone)]
pub enum Purpose {
    Thumbnail,
    Original { request: u64 },
    File { request: u64, name: String },
}

#[derive(Clone)]
pub enum Payload {
    Evicted,
    Image(Arc<Pixels>),
    Original {
        pixels: Arc<Pixels>,
        bytes: Arc<Vec<u8>>,
    },
    File(Arc<Vec<u8>>),
}

pub struct Completion {
    pub network: bool,
    pub key: String,
    pub purpose: Purpose,
    pub result: std::result::Result<Payload, Failure>,
}

#[derive(Clone)]
struct Job {
    bypass: bool,
    asset: Asset,
    purpose: Purpose,
}

struct Entry {
    job: Option<Job>,
    listeners: Vec<Purpose>,
    order: u64,
}

#[derive(Default)]
struct Queue {
    entries: HashMap<String, Entry>,
    evicted: std::collections::VecDeque<Completion>,
    order: u64,
}

impl Entry {
    fn explicit(&self) -> bool {
        self.job.as_ref().is_some_and(|job| job.bypass)
            || self
                .listeners
                .iter()
                .any(|purpose| !matches!(purpose, Purpose::Thumbnail))
    }
}

impl Queue {
    fn next_key(&self) -> Option<String> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.job.is_some())
            .max_by_key(|(_, entry)| (entry.explicit(), entry.order))
            .map(|(key, _)| key.clone())
    }
}

enum Work {
    Evicted(Completion),
    Fetch(String, Job, Option<Failure>),
}

struct Shared {
    queue: Mutex<Queue>,
    ready: async_channel::Sender<()>,
    wait: async_channel::Receiver<()>,
    stop: Arc<CancelToken>,
    global_stop: Arc<CancelToken>,
    failures: Failures,
}

impl Shared {
    fn stopped(&self) -> bool {
        self.stop.is_cancelled() || self.global_stop.is_cancelled()
    }
}

pub struct Fetchers {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
}

impl Fetchers {
    pub(in crate::client::desktop) fn start(
        http: http::Transport,
        cache: Arc<Mutex<ImageCache>>,
        global_stop: Arc<CancelToken>,
        failures: Failures,
        authorize: impl Fn(&Asset, &CancelToken) -> std::result::Result<Option<String>, Failure>
        + Send
        + Sync
        + 'static,
        publish: impl Fn(Completion, &CancelToken) + Send + Sync + 'static,
    ) -> Self {
        let (ready, wait) = async_channel::bounded(THREADS);
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            ready,
            wait,
            stop: Arc::new(CancelToken::new()),
            global_stop,
            failures,
        });
        let publish = Arc::new(publish);
        let authorize = Arc::new(authorize);
        let threads = (0..THREADS)
            .map(|index| {
                let shared = shared.clone();
                let mut http = http.clone();
                let cache = cache.clone();
                let publish = publish.clone();
                let authorize = authorize.clone();
                thread::Builder::new()
                    .name(format!("line-asset-{index}"))
                    .spawn(move || {
                        set_cancellation_token(shared.stop.clone());
                        loop {
                            let work = {
                                let mut queue = shared.queue.lock().unwrap();
                                loop {
                                    if shared.stopped() {
                                        return;
                                    }
                                    if let Some(completion) = queue.evicted.pop_front() {
                                        break Work::Evicted(completion);
                                    }
                                    let key = queue.next_key();
                                    if let Some(key) = key {
                                        let blacklisted = shared
                                            .failures
                                            .0
                                            .lock()
                                            .unwrap()
                                            .get(&key)
                                            .filter(|f| !f.retry || f.at.elapsed() < FAILURE_TTL)
                                            .map(|f| f.class);
                                        let job = queue
                                            .entries
                                            .get_mut(&key)
                                            .unwrap()
                                            .job
                                            .take()
                                            .unwrap();
                                        break Work::Fetch(key, job, blacklisted);
                                    }
                                    drop(queue);
                                    futures_lite::future::block_on(futures_lite::future::or(
                                        async {
                                            let _ = shared.wait.recv().await;
                                        },
                                        futures_lite::future::or(
                                            shared.stop.cancelled(),
                                            shared.global_stop.cancelled(),
                                        ),
                                    ));
                                    queue = shared.queue.lock().unwrap();
                                }
                            };
                            let (key, job, blacklisted) = match work {
                                Work::Evicted(completion) => {
                                    publish(completion, &shared.stop);
                                    continue;
                                }
                                Work::Fetch(key, job, blacklisted) => (key, job, blacklisted),
                            };
                            let activity = crate::client::NetworkActivity::default();
                            http.activity = activity.clone();
                            let result = if let Some(class) = blacklisted.filter(|_| !job.bypass) {
                                Err(class)
                            } else {
                                fetch(&http, &cache, &job, || authorize(&job.asset, &shared.stop))
                                    .map_err(|error| media_failure(&error))
                            };
                            let network = activity.touched();
                            if shared.stopped() {
                                return;
                            }
                            let listeners = {
                                let mut queue = shared.queue.lock().unwrap();
                                match &result {
                                    Err(class) if blacklisted.is_none() || job.bypass => {
                                        shared.failures.record(
                                            key.clone(),
                                            *class,
                                            job.clone(),
                                            true,
                                        );
                                    }
                                    Ok(_) => {
                                        shared.failures.0.lock().unwrap().remove(&key);
                                    }
                                    Err(_) => {}
                                }
                                queue.entries.remove(&key).unwrap().listeners
                            };
                            for purpose in listeners {
                                if shared.stopped() {
                                    return;
                                }
                                publish(
                                    Completion {
                                        network,
                                        key: key.clone(),
                                        purpose,
                                        result: result.clone(),
                                    },
                                    &shared.stop,
                                );
                            }
                        }
                    })
                    .expect("spawn asset fetcher")
            })
            .collect();
        Self { shared, threads }
    }

    pub fn failure(&self, key: &str) -> Option<Failure> {
        self.shared
            .failures
            .0
            .lock()
            .unwrap()
            .get(key)
            .map(|f| f.class)
    }

    pub fn retry(&self, key: &str) -> bool {
        let job = self
            .shared
            .failures
            .0
            .lock()
            .unwrap()
            .get(key)
            .filter(|f| f.retry)
            .map(|f| f.job.clone());
        job.is_some_and(|job| {
            self.enqueue_job(Job {
                bypass: true,
                ..job
            })
        })
    }

    pub fn enqueue(&self, asset: Asset, purpose: Purpose) -> bool {
        let bypass = !matches!(purpose, Purpose::Thumbnail)
            && self
                .shared
                .failures
                .0
                .lock()
                .unwrap()
                .get(&asset.key())
                .is_some_and(|f| f.retry);
        let key = asset.key();
        let job = Job {
            asset,
            purpose,
            bypass,
        };
        let accepted = self.enqueue_job(job.clone());
        if !accepted && self.failure(&key).is_none() {
            self.shared
                .failures
                .record(key, Failure::Network, job, false);
        }
        accepted
    }

    fn enqueue_job(&self, job: Job) -> bool {
        let mut queue = self.shared.queue.lock().unwrap();
        if self.shared.stopped() {
            return false;
        }
        let key = job.asset.key();
        let purpose = job.purpose.clone();
        if let Some(entry) = queue.entries.get_mut(&key) {
            if !matches!(purpose, Purpose::Thumbnail) {
                if entry.listeners.len() >= QUEUE_LIMIT {
                    return false;
                }
                entry.listeners.push(purpose);
            }
            if let Some(pending) = &mut entry.job {
                pending.bypass |= job.bypass;
            }
            return true;
        }
        if queue.entries.len() >= QUEUE_LIMIT {
            let oldest = queue
                .entries
                .iter()
                .filter_map(|(key, entry)| {
                    entry
                        .job
                        .as_ref()
                        .filter(|_| !entry.explicit())
                        .map(|_| (key.clone(), entry.order))
                })
                .min_by_key(|(_, order)| *order);
            let Some((oldest, _)) = oldest else {
                return false;
            };
            let entry = queue.entries.remove(&oldest).unwrap();
            for purpose in entry.listeners {
                queue.evicted.push_back(Completion {
                    network: false,
                    key: oldest.clone(),
                    purpose,
                    result: Ok(Payload::Evicted),
                });
            }
        }
        queue.order += 1;
        let order = queue.order;
        queue.entries.insert(
            key,
            Entry {
                job: Some(job),
                listeners: vec![purpose],
                order,
            },
        );
        let _ = self.shared.ready.try_send(());
        true
    }
}

impl Drop for Fetchers {
    fn drop(&mut self) {
        self.shared.stop.cancel();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

fn fetch(
    http: &http::Transport,
    cache: &Mutex<ImageCache>,
    job: &Job,
    authorize: impl Fn() -> std::result::Result<Option<String>, Failure>,
) -> Result<Payload> {
    if let Some(failure) = job.asset.failure {
        return Err(failure.into());
    }
    match job.purpose {
        Purpose::Thumbnail => {
            let cached = if job.bypass {
                None
            } else {
                ImageCache::read_shared(cache, &job.asset)?
            };
            let pixels = match cached {
                Some(pixels) => pixels,
                None => {
                    let bytes = job.asset.fetch(
                        http,
                        if job.asset.private {
                            authorize()?
                        } else {
                            None
                        }
                        .as_deref(),
                    )?;
                    ImageCache::store_shared(cache, &job.asset, &bytes)?
                }
            };
            Ok(Payload::Image(pixels))
        }
        Purpose::Original { .. } => {
            let bytes = job.asset.fetch(
                http,
                if job.asset.private {
                    authorize()?
                } else {
                    None
                }
                .as_deref(),
            )?;
            Ok(Payload::Original {
                pixels: Arc::new(original_pixels(&bytes)?),
                bytes: Arc::new(bytes),
            })
        }
        Purpose::File { .. } => Ok(Payload::File(Arc::new(
            job.asset.fetch(
                http,
                if job.asset.private {
                    authorize()?
                } else {
                    None
                }
                .as_deref(),
            )?,
        ))),
    }
}

#[cfg(test)]
mod tests;
