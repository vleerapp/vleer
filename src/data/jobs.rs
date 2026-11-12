use crate::data::db::Database;
use crate::data::models::Cuid;
use crate::ui::components::context_menu::{BackgroundUiEvent, BackgroundUiNotifier};
use futures::StreamExt;
use gpui::BackgroundExecutor;
use parking_lot::Mutex;
use priority::{Progress, Tier};
use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tracing::{debug, info};

pub mod priority {
    use parking_lot::Mutex;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    use tokio::sync::Notify;

    use crate::data::models::Cuid;

    const MAX_VISIBLE: usize = 64;
    const MAX_NEAR: usize = 128;
    const PROGRESS_INTERVAL: Duration = Duration::from_millis(750);

    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
    pub enum Tier {
        Playing,
        Visible,
        Near,
        Bulk,
    }

    #[derive(Clone, Default)]
    pub struct Snapshot {
        pub playing: Option<Cuid>,
        pub songs: Vec<Cuid>,
        pub albums: Vec<Cuid>,
        pub near_songs: Vec<Cuid>,
        pub near_albums: Vec<Cuid>,
    }

    pub struct Signal {
        epoch: AtomicU64,
        notify: Notify,
    }

    impl Signal {
        pub(crate) const fn new() -> Self {
            Self {
                epoch: AtomicU64::new(0),
                notify: Notify::const_new(),
            }
        }

        pub fn fire(&self) {
            self.epoch.fetch_add(1, Ordering::AcqRel);
            self.notify.notify_waiters();
        }

        pub fn seen(&self) -> u64 {
            self.epoch.load(Ordering::Acquire)
        }

        pub async fn changed(&self, seen: u64) {
            loop {
                let notified = self.notify.notified();
                let mut notified = std::pin::pin!(notified);
                notified.as_mut().enable();
                if self.seen() != seen {
                    return;
                }
                notified.await;
                if self.seen() != seen {
                    return;
                }
            }
        }
    }

    pub static WAKE: Signal = Signal::new();
    pub static FILE_PROGRESS: Signal = Signal::new();
    static FILE_BUSY: AtomicBool = AtomicBool::new(false);

    static STATE: Mutex<Snapshot> = Mutex::new(Snapshot {
        playing: None,
        songs: Vec::new(),
        albums: Vec::new(),
        near_songs: Vec::new(),
        near_albums: Vec::new(),
    });
    static LAST_START: AtomicUsize = AtomicUsize::new(0);

    pub fn wake() {
        WAKE.fire();
    }

    pub fn set_file_busy(busy: bool) {
        FILE_BUSY.store(busy, Ordering::Release);
        FILE_PROGRESS.fire();
    }

    pub fn file_busy() -> bool {
        FILE_BUSY.load(Ordering::Acquire)
    }

    pub fn snapshot() -> Snapshot {
        STATE.lock().clone()
    }

    pub fn set_playing(id: Option<Cuid>) {
        let mut state = STATE.lock();
        if state.playing != id {
            state.playing = id;
            drop(state);
            wake();
        }
    }

    pub fn scroll_forward(start: usize) -> bool {
        LAST_START.swap(start, Ordering::AcqRel) <= start
    }

    pub fn outward(before: Vec<Cuid>, after: Vec<Cuid>, forward: bool) -> Vec<Cuid> {
        let (lead, trail) = if forward {
            (after, before)
        } else {
            (before, after)
        };
        let mut lead = lead.into_iter();
        let mut trail = trail.into_iter();
        let mut out = Vec::with_capacity(MAX_NEAR);
        while out.len() < MAX_NEAR {
            let before_len = out.len();
            out.extend(lead.next());
            out.extend(lead.next());
            out.extend(trail.next());
            if out.len() == before_len {
                break;
            }
        }
        out.truncate(MAX_NEAR);
        out
    }

    pub fn set_visible_songs(mut ids: Vec<Cuid>, near: Vec<Cuid>) {
        ids.truncate(MAX_VISIBLE);
        let mut state = STATE.lock();
        let changed = !state.albums.is_empty() || state.songs != ids || state.near_songs != near;
        state.albums.clear();
        state.near_albums.clear();
        if changed {
            state.songs = ids;
            state.near_songs = near;
            drop(state);
            wake();
        }
    }

    pub fn set_visible_albums(mut ids: Vec<Cuid>, near: Vec<Cuid>) {
        ids.truncate(MAX_VISIBLE);
        let mut state = STATE.lock();
        let changed = !state.songs.is_empty() || state.albums != ids || state.near_albums != near;
        state.songs.clear();
        state.near_songs.clear();
        if changed {
            state.albums = ids;
            state.near_albums = near;
            drop(state);
            wake();
        }
    }

    pub fn clear_visible() {
        let mut state = STATE.lock();
        state.songs.clear();
        state.albums.clear();
        state.near_songs.clear();
        state.near_albums.clear();
    }

    pub struct Progress {
        notify: Arc<dyn Fn() + Send + Sync>,
        last: Instant,
        dirty: bool,
    }

    impl Progress {
        pub fn new(notify: Arc<dyn Fn() + Send + Sync>) -> Self {
            Self {
                notify,
                last: Instant::now(),
                dirty: false,
            }
        }

        pub fn changed(&mut self, tier: Tier) {
            self.dirty = true;
            if tier == Tier::Playing || self.last.elapsed() >= PROGRESS_INTERVAL {
                self.flush();
            }
        }

        pub fn flush(&mut self) {
            if self.dirty {
                self.dirty = false;
                self.last = Instant::now();
                (self.notify)();
            }
        }
    }
}

pub fn lower_thread_priority() {
    #[cfg(target_os = "linux")]
    unsafe {
        let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, 19);
    }
    #[cfg(target_os = "macos")]
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_BACKGROUND, 0);
    }
}

pub enum Bulk<W> {
    Works(Vec<W>),
    WaitFile,
    Done,
}

#[derive(Clone)]
pub struct Stop {
    cancel: Arc<AtomicBool>,
    urgent: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    checked: Arc<AtomicU64>,
    latched: Arc<AtomicBool>,
}

impl Stop {
    fn new(cancel: Arc<AtomicBool>, urgent: Option<Arc<dyn Fn() -> bool + Send + Sync>>) -> Self {
        Self {
            cancel,
            urgent,
            checked: Arc::new(AtomicU64::new(priority::WAKE.seen())),
            latched: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn requested(&self) -> bool {
        if self.cancel.load(Ordering::Relaxed) || self.latched.load(Ordering::Relaxed) {
            return true;
        }
        let Some(urgent) = &self.urgent else {
            return false;
        };
        let now = priority::WAKE.seen();
        if self.checked.swap(now, Ordering::AcqRel) == now {
            return false;
        }
        let stop = urgent();
        if stop {
            self.latched.store(true, Ordering::Relaxed);
        }
        stop
    }
}

#[derive(Default)]
pub struct Claims {
    state: Mutex<ClaimState>,
}

#[derive(Default)]
struct ClaimState {
    claimed: HashSet<Cuid>,
    failed: HashSet<Cuid>,
}

pub struct Claim<'a> {
    claims: &'a Claims,
    id: Cuid,
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        self.claims.state.lock().claimed.remove(&self.id);
    }
}

impl Claims {
    pub fn claim(&self, id: &Cuid) -> Option<Claim<'_>> {
        let mut state = self.state.lock();
        if state.failed.contains(id) || !state.claimed.insert(id.clone()) {
            return None;
        }
        Some(Claim {
            claims: self,
            id: id.clone(),
        })
    }

    pub fn fail(&self, id: Cuid) {
        self.state.lock().failed.insert(id);
    }

    pub fn blocked(&self, id: &Cuid) -> bool {
        let state = self.state.lock();
        state.failed.contains(id) || state.claimed.contains(id)
    }

    pub fn failed(&self) -> usize {
        self.state.lock().failed.len()
    }
}

pub trait Job: Send + Sync + 'static {
    type Work: Send + 'static;

    fn name(&self) -> &'static str;

    fn bulk_concurrency(&self) -> usize {
        1
    }

    fn exclusive(&self) -> Option<&'static tokio::sync::Mutex<()>> {
        None
    }

    fn prepare(&self) {}

    fn priority(&self) -> Option<(Tier, Self::Work)>;

    fn bulk(&self) -> impl Future<Output = Bulk<Self::Work>> + Send;

    fn run(&self, tier: Tier, work: Self::Work, stop: &Stop) -> impl Future<Output = ()> + Send;

    fn recover(&self) -> impl Future<Output = bool> + Send {
        async { true }
    }

    fn retry_in(&self) -> Option<Duration> {
        None
    }

    fn report(&self) {}

    fn resume(&self) {}
}

pub async fn sleep(duration: Duration, cancel: Arc<AtomicBool>) {
    let (tx, rx) = futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("vleer-job-sleep".into())
        .spawn(move || {
            let deadline = std::time::Instant::now() + duration;
            while std::time::Instant::now() < deadline && !cancel.load(Ordering::Acquire) {
                std::thread::sleep(
                    Duration::from_millis(100)
                        .min(deadline.saturating_duration_since(std::time::Instant::now())),
                );
            }
            let _ = tx.send(());
        });
    if spawned.is_ok() {
        let _ = rx.await;
    }
}

async fn wait_for_change<J: Job>(job: &J, cancel: &Arc<AtomicBool>, seen: u64) {
    match job.retry_in() {
        Some(wait) => {
            futures::future::select(
                Box::pin(priority::WAKE.changed(seen)),
                Box::pin(sleep(wait, cancel.clone())),
            )
            .await;
        }
        None => priority::WAKE.changed(seen).await,
    }
}

async fn priority_loop<J: Job>(job: &J, cancel: &Arc<AtomicBool>) {
    let stop = Stop::new(cancel.clone(), None);
    loop {
        if cancel.load(Ordering::Acquire) {
            return;
        }
        let seen = priority::WAKE.seen();
        match job.priority() {
            Some((tier, work)) => job.run(tier, work, &stop).await,
            None => wait_for_change(job, cancel, seen).await,
        }
    }
}

async fn bulk_loop<J: Job>(job: &Arc<J>, cancel: &Arc<AtomicBool>) {
    let mut gave_up = false;

    loop {
        if cancel.load(Ordering::Acquire) {
            return;
        }
        let seen = priority::WAKE.seen();
        if gave_up {
            wait_for_change(&**job, cancel, seen).await;
            continue;
        }

        match job.bulk().await {
            Bulk::Works(works) => {
                let urgent_job = job.clone();
                let stop = Stop::new(
                    cancel.clone(),
                    Some(Arc::new(move || urgent_job.priority().is_some())),
                );
                futures::stream::iter(works)
                    .for_each_concurrent(job.bulk_concurrency(), |work| {
                        job.run(Tier::Bulk, work, &stop)
                    })
                    .await;
                if !job.recover().await {
                    gave_up = !cancel.load(Ordering::Acquire);
                }
            }
            Bulk::WaitFile => {
                let file = priority::FILE_PROGRESS.seen();
                futures::future::select(
                    Box::pin(priority::FILE_PROGRESS.changed(file)),
                    Box::pin(priority::WAKE.changed(seen)),
                )
                .await;
            }
            Bulk::Done => {
                job.report();
                wait_for_change(&**job, cancel, seen).await;
                job.resume();
            }
        }
    }
}

pub async fn run_job<J: Job>(job: Arc<J>, cancel: Arc<AtomicBool>) {
    let _exclusive = match job.exclusive() {
        Some(lock) => Some(lock.lock().await),
        None => None,
    };
    if cancel.load(Ordering::Acquire) {
        return;
    }
    job.prepare();
    futures::future::join(priority_loop(&*job, &cancel), bulk_loop(&job, &cancel)).await;
    job.report();
}

#[derive(Clone)]
pub struct JobContext {
    pub db: Database,
    pub cancel: Arc<AtomicBool>,
    pub ui: Option<BackgroundUiNotifier>,
    executor: BackgroundExecutor,
}

impl JobContext {
    pub fn notify(&self, event: BackgroundUiEvent) {
        if let Some(ui) = &self.ui {
            ui.notify(event);
        }
    }

    pub fn progress(&self) -> Progress {
        let ui = self.ui.clone();
        Progress::new(Arc::new(move || {
            if let Some(ui) = &ui {
                ui.notify(BackgroundUiEvent::CoversChanged);
            }
        }))
    }

    pub fn spawn<F, Fut>(&self, name: &'static str, job: F)
    where
        F: FnOnce(JobContext) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let ctx = self.clone();
        self.executor
            .spawn(async move {
                debug!("{name} job started");
                job(ctx).await;
            })
            .detach();
    }

    pub fn start<J: Job>(&self, job: J) {
        let name = job.name();
        let cancel = self.cancel.clone();
        let job = Arc::new(job);
        self.spawn(name, move |_| run_job(job, cancel));
    }

    pub fn library_changed(&self) {
        self.notify(BackgroundUiEvent::LibraryDataChanged);
        self.notify(BackgroundUiEvent::HomeDataChanged);
    }
}

#[derive(Clone)]
pub struct Jobs {
    cancel: Arc<std::sync::Mutex<Arc<AtomicBool>>>,
    executor: BackgroundExecutor,
    ui: Option<BackgroundUiNotifier>,
}

impl Jobs {
    pub fn new(executor: BackgroundExecutor, ui: Option<BackgroundUiNotifier>) -> Self {
        Self {
            cancel: Arc::new(std::sync::Mutex::new(Arc::new(AtomicBool::new(false)))),
            executor,
            ui,
        }
    }

    fn flag(&self) -> std::sync::MutexGuard<'_, Arc<AtomicBool>> {
        self.cancel
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn cancel(&self) {
        self.flag().store(true, Ordering::Release);
        priority::wake();
    }

    pub fn start(&self, db: &Database, reset_metadata: bool) {
        if reset_metadata {
            self.cancel();
        }
        let flag = Arc::new(AtomicBool::new(false));
        *self.flag() = flag.clone();

        let ctx = JobContext {
            db: db.clone(),
            cancel: flag,
            ui: self.ui.clone(),
            executor: self.executor.clone(),
        };

        ctx.start(crate::data::images::ImageJob::new(&ctx));
        ctx.start(crate::data::omm::Pass::new(&ctx, reset_metadata));
        ctx.start(crate::data::loudness::LoudnessJob::new(&ctx));
        if !reset_metadata {
            spawn_genre_covers(&ctx);
        }
    }
}

pub fn spawn_genre_covers(ctx: &JobContext) {
    ctx.spawn("genre cover", |ctx| async move {
        let built =
            crate::data::images::warm_genre_covers(ctx.db.clone(), ctx.cancel.clone()).await;
        if built > 0 {
            info!("Composed {built} genre cover(s)");
            ctx.notify(BackgroundUiEvent::CoversChanged);
        }
    });
}
