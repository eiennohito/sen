//! Event-driven worker driver shared by unit and integration tests.
use super::*;
use crate::controller::AppState;

pub const WAIT: Duration = Duration::from_secs(10);
pub(super) type EngineWaiter = Box<dyn Fn(&Engine) -> bool + Send>;

pub struct App<T = ()> {
    pub worker: Worker,
    pub state: AppState,
    pub notifications: usize,
    pub fake: T,
}
impl App {
    pub fn from_worker(worker: Worker) -> Self {
        Self::with_fake(worker, ())
    }
    pub fn new(backend: impl Backend + 'static, location: StoreLocation) -> Self {
        let mut backend = Some(backend);
        Self::from_worker(Worker::start(
            location,
            move || Ok(backend.take().expect("backend already opened")),
            WorkerOptions {
                timing: test_timing(),
            },
        ))
    }
}
pub fn test_timing() -> Timing {
    let now = Instant::now();
    Timing {
        retry_initial: Duration::from_millis(5),
        retry_max: Duration::from_millis(50),
        health_now: Arc::new(move || now),
        ..Default::default()
    }
}
impl<T> App<T> {
    pub fn with_fake(worker: Worker, fake: T) -> Self {
        Self {
            worker,
            state: AppState::default(),
            notifications: 0,
            fake,
        }
    }
    pub fn send(&self, intent: Intent) {
        self.worker.sender.send(intent);
    }
    pub fn select(&mut self, chat: &str) {
        let intent = self.state.select(ChatId(chat.into()));
        self.send(intent);
    }
    fn apply(&mut self, delta: Delta) {
        if matches!(delta, Delta::Notify(_)) {
            self.notifications += 1;
        }
        assert!(
            !matches!(delta, Delta::Fatal | Delta::StorageFailed(_)),
            "engine failure"
        );
        self.state.apply(delta);
    }
    pub fn until(&mut self, ready: impl Fn(&AppState) -> bool) {
        let deadline = Instant::now() + WAIT;
        while !ready(&self.state) {
            self.until_delta(|_| true, deadline.saturating_duration_since(Instant::now()));
        }
    }
    pub fn until_delta(&mut self, ready: impl Fn(&Delta) -> bool, timeout: Duration) -> Delta {
        let deadline = Instant::now() + timeout;
        loop {
            let delta = self
                .worker
                .deltas
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("engine progress");
            let done = ready(&delta);
            self.apply(delta.clone());
            if done {
                return delta;
            }
        }
    }
    pub(super) fn on_engine(&self, run: impl FnOnce(&mut Engine) + Send + 'static) {
        let (done, wait) = mpsc::channel();
        upload_completed(
            &self.worker.sender.0,
            Box::new(move |engine| {
                run(engine);
                done.send(()).unwrap();
                Ok(())
            }),
        );
        wait.recv_timeout(WAIT).expect("engine callback");
    }
    pub(super) fn wait_engine(&self, ready: impl Fn(&Engine) -> bool + Send + 'static) {
        let (done, wait) = mpsc::channel();
        self.on_engine(move |engine| {
            engine.waiters.push(Box::new(move |engine| {
                if ready(engine) {
                    done.send(()).unwrap();
                    true
                } else {
                    false
                }
            }))
        });
        wait.recv_timeout(WAIT).expect("engine condition");
    }
    /// Process preceding user and live events without writing preferences.
    pub fn fence(&mut self) {
        self.on_engine(|_| {});
        self.drain();
    }
    fn drain(&mut self) {
        while let Ok(delta) = self.worker.deltas.try_recv() {
            self.apply(delta);
        }
    }
    pub fn settle(&mut self) {
        self.wait_engine(|engine| {
            let q = engine.inbox.lanes.lock().unwrap();
            engine.net_pending == 0
                && q.users.is_empty()
                && q.live.is_empty()
                && q.completed.is_empty()
                && q.refresh.is_empty()
        });
        self.drain();
    }
    pub fn online(&mut self) {
        self.until(|s| s.connection == Connection::Online && s.chats.len() == 2);
    }
    pub fn body_key(&mut self, body: &str) -> Key {
        self.until(|s| s.messages.values().any(|m| m.body.as_text() == body));
        self.state
            .messages
            .values()
            .find(|m| m.body.as_text() == body)
            .unwrap()
            .key
    }
}
