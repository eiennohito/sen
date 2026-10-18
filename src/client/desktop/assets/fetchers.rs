use super::*;
use std::{
    collections::HashSet,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Instant,
};

const THREADS: usize = 3;
const QUEUE_LIMIT: usize = 256;
const FAILURE_TTL: Duration = Duration::from_secs(300);

#[derive(Default)]
pub struct Priority(Mutex<(String, HashSet<String>)>);

impl Priority {
    pub fn set(&self, chat: String, keys: impl IntoIterator<Item = String>) {
        *self.0.lock().unwrap() = (
            chat,
            keys.into_iter().filter(|key| !key.is_empty()).collect(),
        );
    }
}

#[derive(Clone, Default)]
pub struct Failures(Arc<Mutex<HashMap<String, Failed>>>, Arc<Mutex<String>>);
impl Failures {
    fn record(&self, key: String, class: Failure, job: Job, replace: bool) {
        let mut failed = self.0.lock().unwrap();
        if !replace && failed.contains_key(&key) {
            return;
        }
        let retried = failed.get(&key).is_some_and(|f| f.retried);
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
                at: Instant::now(),
                job,
                retried,
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
    at: Instant,
    job: Job,
    retried: bool,
}

#[derive(Clone)]
pub enum Purpose {
    Thumbnail,
    Original { request: u64 },
    File { request: u64, name: String },
}

#[derive(Clone)]
pub enum Payload {
    Image(Arc<Pixels>),
    File(Arc<Vec<u8>>),
}

pub struct Completion {
    pub key: String,
    pub purpose: Purpose,
    pub result: std::result::Result<Payload, Failure>,
}

#[derive(Clone)]
struct Job {
    bypass: bool,
    boost: bool,
    asset: Asset,
    chat: Option<String>,
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
    order: u64,
}

struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
    stop: Arc<AtomicBool>,
    global_stop: Arc<AtomicBool>,
    priority: Arc<Priority>,
    failures: Failures,
}

impl Shared {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed) || self.global_stop.load(Ordering::Relaxed)
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
        global_stop: Arc<AtomicBool>,
        priority: Arc<Priority>,
        failures: Failures,
        authorize: impl Fn(&Asset, &AtomicBool) -> std::result::Result<Option<String>, Failure>
        + Send
        + Sync
        + 'static,
        publish: impl Fn(Completion, &AtomicBool) + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            ready: Condvar::new(),
            stop: Arc::new(AtomicBool::new(false)),
            global_stop,
            priority,
            failures,
        });
        let publish = Arc::new(publish);
        let authorize = Arc::new(authorize);
        let threads = (0..THREADS)
            .map(|index| {
                let shared = shared.clone();
                let http = http.clone();
                let cache = cache.clone();
                let publish = publish.clone();
                let authorize = authorize.clone();
                thread::Builder::new()
                    .name(format!("line-asset-{index}"))
                    .spawn(move || {
                        set_cancellation_token(shared.stop.clone());
                        loop {
                            let (key, job, blacklisted) = {
                                let mut queue = shared.queue.lock().unwrap();
                                loop {
                                    if shared.stopped() {
                                        return;
                                    }
                                    let priority = shared.priority.0.lock().unwrap();
                                    let key = queue
                                        .entries
                                        .iter()
                                        .filter_map(|(key, entry)| {
                                            entry.job.as_ref().map(|job| {
                                                (key, (rank(key, job, &priority), entry.order))
                                            })
                                        })
                                        .max_by_key(|(_, rank)| *rank)
                                        .map(|(key, _)| key.clone());
                                    drop(priority);
                                    if let Some(key) = key {
                                        let blacklisted = shared
                                            .failures
                                            .0
                                            .lock()
                                            .unwrap()
                                            .get(&key)
                                            .filter(|f| {
                                                !f.class.retriable() || f.at.elapsed() < FAILURE_TTL
                                            })
                                            .map(|f| f.class);
                                        let job = queue
                                            .entries
                                            .get_mut(&key)
                                            .unwrap()
                                            .job
                                            .take()
                                            .unwrap();
                                        break (key, job, blacklisted);
                                    }
                                    queue = shared
                                        .ready
                                        .wait_timeout(queue, Duration::from_millis(50))
                                        .unwrap()
                                        .0;
                                }
                            };
                            let result = if let Some(class) = blacklisted.filter(|_| !job.bypass) {
                                Err(class)
                            } else {
                                fetch(&http, &cache, &job, || authorize(&job.asset, &shared.stop))
                                    .map_err(|error| Failure::from_error(&error))
                            };
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
            .filter(|f| f.class.retriable())
            .map(|f| f.job.clone());
        job.is_some_and(|job| {
            self.enqueue_job(Job {
                bypass: true,
                boost: true,
                ..job
            })
        })
    }

    pub fn reconnected(&self) {
        let jobs: Vec<_> = {
            let mut failures = self.shared.failures.0.lock().unwrap();
            failures
                .values_mut()
                .filter_map(|failed| {
                    if failed.class.automatic()
                        && !failed.retried
                        && matches!(failed.job.purpose, Purpose::Thumbnail)
                    {
                        failed.retried = true;
                        Some(Job {
                            bypass: true,
                            boost: false,
                            ..failed.job.clone()
                        })
                    } else {
                        None
                    }
                })
                .collect()
        };
        for job in jobs {
            self.enqueue_job(job);
        }
    }

    pub fn enqueue(&self, asset: Asset, chat: Option<String>, purpose: Purpose) -> bool {
        let bypass = !matches!(purpose, Purpose::Thumbnail)
            && self
                .shared
                .failures
                .0
                .lock()
                .unwrap()
                .get(&asset.key())
                .is_some_and(|f| f.class.retriable());
        let key = asset.key();
        let job = Job {
            asset,
            chat,
            purpose,
            bypass,
            boost: false,
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
                pending.chat = job.chat;
                pending.bypass |= job.bypass;
                pending.boost |= job.boost;
            }
            return true;
        }
        if queue.entries.len() >= QUEUE_LIMIT {
            let priority = self.shared.priority.0.lock().unwrap();
            let oldest = queue
                .entries
                .iter()
                .filter_map(|(key, entry)| {
                    entry
                        .job
                        .as_ref()
                        .filter(|job| matches!(job.purpose, Purpose::Thumbnail))
                        .map(|job| (key.clone(), (rank(key, job, &priority), entry.order)))
                })
                .min_by_key(|(_, rank)| *rank);
            let Some((oldest, (old_rank, _))) = oldest else {
                return false;
            };
            if rank(&key, &job, &priority) < old_rank {
                return false;
            }
            queue.entries.remove(&oldest);
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
        self.shared.ready.notify_one();
        true
    }
}

impl Drop for Fetchers {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.ready.notify_all();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

fn rank(key: &str, job: &Job, priority: &(String, HashSet<String>)) -> u8 {
    if job.boost || !matches!(job.purpose, Purpose::Thumbnail) {
        return 4;
    }
    if priority.1.contains(key) {
        return 3;
    }
    match &job.chat {
        Some(chat) if chat == &priority.0 => 2,
        None => 1,
        Some(_) => 0,
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
            Ok(Payload::Image(if job.asset.kind() == "avatar" {
                Arc::new(circular_avatar(&pixels))
            } else {
                pixels
            }))
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
            Ok(Payload::Image(Arc::new(original_pixels(&bytes)?)))
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
