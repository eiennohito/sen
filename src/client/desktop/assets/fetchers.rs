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
pub struct Failures(Arc<Mutex<HashMap<String, Instant>>>);
impl Failures {
    pub fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
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
    pub result: std::result::Result<Payload, ()>,
}

struct Job {
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
        authorize: impl Fn(&Asset, &AtomicBool) -> Option<String> + Send + Sync + 'static,
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
                                            .is_some_and(|t| {
                                                t.elapsed() < Duration::from_secs(300)
                                            });
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
                            let result = if blacklisted && matches!(job.purpose, Purpose::Thumbnail)
                            {
                                Err(())
                            } else {
                                fetch(&http, &cache, &job, || authorize(&job.asset, &shared.stop))
                                    .map_err(|_| ())
                            };
                            if shared.stopped() {
                                return;
                            }
                            let listeners = {
                                let mut queue = shared.queue.lock().unwrap();
                                if !blacklisted
                                    && result.is_err()
                                    && matches!(job.purpose, Purpose::Thumbnail)
                                {
                                    let mut failed = shared.failures.0.lock().unwrap();
                                    if failed.len() >= QUEUE_LIMIT {
                                        failed.clear();
                                    }
                                    failed.insert(key.clone(), Instant::now());
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

    pub fn clear_failures(&self) {
        self.shared.failures.clear();
    }

    pub fn enqueue(&self, asset: Asset, chat: Option<String>, purpose: Purpose) -> bool {
        let mut queue = self.shared.queue.lock().unwrap();
        if self.shared.stopped() {
            return false;
        }
        let key = asset.key();
        if let Some(entry) = queue.entries.get_mut(&key) {
            if !matches!(purpose, Purpose::Thumbnail) {
                if entry.listeners.len() >= QUEUE_LIMIT {
                    return false;
                }
                entry.listeners.push(purpose);
            }
            if let Some(job) = &mut entry.job {
                job.chat = chat;
            }
            return true;
        }
        let job = Job {
            asset,
            chat,
            purpose: purpose.clone(),
        };
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
    if !matches!(job.purpose, Purpose::Thumbnail) {
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
    authorize: impl Fn() -> Option<String>,
) -> Result<Payload> {
    match job.purpose {
        Purpose::Thumbnail => {
            let cached = ImageCache::read_shared(cache, &job.asset)?;
            let pixels = match cached {
                Some(pixels) => pixels,
                None => {
                    let bytes = job.asset.fetch(
                        http,
                        if job.asset.private { authorize() } else { None }.as_deref(),
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
                if job.asset.private { authorize() } else { None }.as_deref(),
            )?;
            Ok(Payload::Image(Arc::new(original_pixels(&bytes)?)))
        }
        Purpose::File { .. } => Ok(Payload::File(Arc::new(job.asset.fetch(
            http,
            if job.asset.private { authorize() } else { None }.as_deref(),
        )?))),
    }
}

#[cfg(test)]
mod tests;
