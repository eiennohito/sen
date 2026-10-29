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
pub struct Failures {
    entries: Arc<Mutex<HashMap<String, Failed>>>,
}
impl Failures {
    fn record(&self, key: String, class: Failure, job: Job, replace: bool) {
        let mut failed = self.entries.lock().unwrap();
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

    fn blocked(&self, key: &str) -> Option<Failure> {
        self.entries
            .lock()
            .unwrap()
            .get(key)
            .filter(|f| !f.retry || f.at.elapsed() < FAILURE_TTL)
            .map(|f| f.class)
    }
    fn retry_job(&self, key: &str) -> Option<Job> {
        self.entries
            .lock()
            .unwrap()
            .get(key)
            .filter(|f| f.retry)
            .map(|f| f.job.clone())
    }
    fn clear(&self, key: &str) {
        self.entries.lock().unwrap().remove(key);
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
    Original,
    File { name: String },
    /// A named local copy for the system's default application.
    Media { name: String },
}

#[derive(Clone)]
pub enum Payload {
    Evicted,
    Image(PathBuf),
    Original { path: PathBuf, source: PathBuf },
    File(Arc<Vec<u8>>),
    Media(PathBuf),
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
    threads: Mutex<Vec<JoinHandle<()>>>,
}

impl Fetchers {
    pub fn stop_and_join(&self) {
        self.shared.stop.cancel();
        for thread in self.threads.lock().unwrap().drain(..) {
            let _ = thread.join();
        }
    }
    pub(in crate::client::desktop) fn start(
        http: http::Transport,
        cache: Arc<Mutex<ImageCache>>,
        global_stop: Arc<CancelToken>,
        failures: Failures,
        authorize: impl Fn(bool, &CancelToken) -> std::result::Result<Option<String>, Failure>
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
                    .name(format!("sen-asset-{index}"))
                    .spawn(move || {
                        set_request_cancel(shared.stop.clone());
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
                                        let blacklisted = shared.failures.blocked(&key);
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
                                fetch(&http, &cache, &job, |renew| authorize(renew, &shared.stop))
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
                                        shared.failures.clear(&key);
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
                                        key: job.asset.key(),
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
        Self {
            shared,
            threads: Mutex::new(threads),
        }
    }

    pub fn retry_original(&self, asset: Asset) -> bool {
        self.shared.failures.clear(&asset.key());
        self.enqueue_job(Job {
            asset,
            purpose: Purpose::Original,
            bypass: true,
        })
    }

    pub fn failure(&self, key: &str) -> Option<Failure> {
        self.shared
            .failures
            .entries
            .lock()
            .unwrap()
            .get(key)
            .map(|f| f.class)
    }

    pub fn retry(&self, key: &str) -> bool {
        self.shared.failures.retry_job(key).is_some_and(|job| {
            self.enqueue_job(Job {
                bypass: true,
                ..job
            })
        })
    }

    pub fn enqueue(&self, asset: Asset, purpose: Purpose) -> bool {
        let bypass = !matches!(purpose, Purpose::Thumbnail)
            && self.shared.failures.retry_job(&queue_key(&asset, &purpose)).is_some();
        let key = queue_key(&asset, &purpose);
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
        let key = queue_key(&job.asset, &job.purpose);
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
        self.stop_and_join();
    }
}

fn fetch(
    http: &http::Transport,
    cache: &Mutex<ImageCache>,
    job: &Job,
    authorize: impl Fn(bool) -> std::result::Result<Option<String>, Failure>,
) -> Result<Payload> {
    if let Some(failure) = job.asset.failure {
        return Err(failure.into());
    }
    let download = || {
        let token = if job.asset.private {
            authorize(false)?
        } else {
            None
        };
        match job.asset.fetch(http, token.as_deref()) {
            Err(error)
                if job.asset.private
                    && error.downcast_ref::<Failure>() == Some(&Failure::Http(401)) =>
            {
                job.asset.fetch(http, authorize(true)?.as_deref())
            }
            result => result,
        }
    };
    match job.purpose {
        Purpose::Thumbnail => {
            let cached = if job.bypass {
                None
            } else {
                cache.lock().unwrap().read(&job.asset)?
            };
            let path = match cached {
                Some(path) => path,
                None => {
                    let bytes = download()?;
                    cache.lock().unwrap().store(&job.asset, &bytes)?
                }
            };
            Ok(Payload::Image(path))
        }
        Purpose::Original => {
            if !job.bypass
                && let Some((path, source)) = cache.lock().unwrap().read_original(&job.asset)?
            {
                return Ok(Payload::Original { path, source });
            }
            let bytes = download()?;
            let (path, source) = cache.lock().unwrap().original(&job.asset, &bytes)?;
            Ok(Payload::Original { path, source })
        }
        Purpose::File { .. } => Ok(Payload::File(Arc::new(download()?))),
        Purpose::Media { ref name } => {
            let directory = cache.lock().unwrap().media_directory(&job.asset.key())?;
            if !job.asset.video {
                // Photos reuse the cached original and take its real extension.
                let cached = cache.lock().unwrap().read_original(&job.asset)?;
                let source = match cached {
                    Some((_, source)) => source,
                    None => {
                        let bytes = download()?;
                        cache.lock().unwrap().original(&job.asset, &bytes)?.1
                    }
                };
                let extension = source.extension().and_then(|e| e.to_str()).unwrap_or("");
                let path = directory.join(media_name(name, extension));
                if !path.is_file() {
                    publish_file(&path, |file| {
                        std::io::copy(&mut fs::File::open(&source)?, file)?;
                        Ok(())
                    })?;
                }
                return Ok(Payload::Media(path));
            }
            let path = directory.join(media_name(name, "mp4"));
            if job.bypass || !path.is_file() {
                publish_file(&path, |file| {
                    let mut out = std::io::BufWriter::new(file);
                    let token = if job.asset.private { authorize(false)? } else { None };
                    match job.asset.stream(http, token.as_deref(), &mut out) {
                        Err(error)
                            if job.asset.private
                                && error.downcast_ref::<Failure>() == Some(&Failure::Http(401)) =>
                        {
                            out.get_mut().set_len(0)?;
                            std::io::Seek::rewind(out.get_mut())?;
                            job.asset.stream(http, authorize(true)?.as_deref(), &mut out)
                        }
                        result => result,
                    }?;
                    out.into_inner().map_err(|e| e.into_error())?;
                    Ok(())
                })?;
            }
            Ok(Payload::Media(path))
        }
    }
}

/// Listeners share one fetch per key, so a media copy must not join a pending
/// thumbnail or original fetch of the same asset (it would get their payload).
fn queue_key(asset: &Asset, purpose: &Purpose) -> String {
    match purpose {
        Purpose::Media { .. } => format!("media:{}", asset.key()),
        _ => asset.key(),
    }
}

/// Keeps the sender's name when it already has an equivalent extension.
pub fn media_name(name: &str, extension: &str) -> String {
    let name = Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.starts_with('.') && !n.contains(['\r', '\n']))
        .unwrap_or("sen_media");
    let path = Path::new(name);
    let current = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let jpeg = |e: &str| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg");
    if extension.is_empty()
        || current.eq_ignore_ascii_case(extension)
        || (jpeg(current) && jpeg(extension))
    {
        return name.to_owned();
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    format!("{stem}.{extension}")
}

/// Writes privately beside `path` and renames only after `write` succeeds.
fn publish_file(path: &Path, write: impl FnOnce(&mut fs::File) -> Result<()>) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let name = path.file_name().context("media filename")?.to_string_lossy();
    let part = path.with_file_name(format!(".{name}.{}.part", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&part)?;
    let result = write(&mut file).and_then(|()| Ok(file.sync_all()?));
    match result.and_then(|()| Ok(fs::rename(&part, path)?)) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&part);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests;
