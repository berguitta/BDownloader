//! Download queue: runs up to `max_parallel` jobs (HTTP or video), lets the
//! UI pause, resume and remove them, reports finished packages, and can be
//! saved and restored between sessions.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use bd_net::{DownloadRequest, Engine, SegmentProgress};
use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use url::Url;

use crate::task::{ControlSlot, LinkResolver, NoResolver, RunError, RunResult};
use crate::video::{VideoBackend, VideoFormat};

pub type JobId = u64;

/// How a job is downloaded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JobKind {
    /// Plain or hoster link through the HTTP engine.
    Http,
    /// Video site through yt-dlp.
    Video { format: VideoFormat },
}

/// Everything needed to (re)start a job.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobSpec {
    pub url: Url,
    /// Name to save under; the server's name is used otherwise.
    #[serde(default)]
    pub name: Option<String>,
    pub dest_dir: PathBuf,
    /// Package the file belongs to (extraction runs per package).
    #[serde(default)]
    pub package: Option<String>,
    /// Archive password for this package.
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default = "default_kind")]
    pub kind: JobKind,
    /// Connections for this file instead of the global setting.
    #[serde(default)]
    pub connections: Option<usize>,
    /// Speed limit for this file in bytes per second, 0 for none.
    #[serde(default)]
    pub rate_limit: u64,
    /// When it was added, in seconds since 1970.
    #[serde(default)]
    pub added: u64,
    /// Higher starts first: -2 lowest … 2 highest.
    #[serde(default)]
    pub priority: i32,
}

fn default_kind() -> JobKind {
    JobKind::Http
}

impl JobSpec {
    pub fn http(url: Url, dest_dir: impl Into<PathBuf>) -> Self {
        JobSpec {
            url,
            name: None,
            dest_dir: dest_dir.into(),
            package: None,
            password: None,
            kind: JobKind::Http,
            connections: None,
            rate_limit: 0,
            added: now_secs(),
            priority: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "error", rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Downloading,
    Paused,
    Finished,
    Failed(String),
}

/// Read-only view of a job for the UI.
#[derive(Clone, Debug)]
pub struct JobSnapshot {
    pub id: JobId,
    pub spec: JobSpec,
    pub url: Url,
    pub name: String,
    pub state: JobState,
    pub total: Option<u64>,
    pub downloaded: u64,
    /// Bytes per second, smoothed.
    pub speed: f64,
    pub segments: Vec<SegmentProgress>,
    pub connections: usize,
    pub dest_dir: PathBuf,
    pub path: Option<PathBuf>,
    pub resumed: bool,
    /// Status text while there is no progress yet ("Resolving link…").
    pub status: Option<String>,
}

impl JobSnapshot {
    /// Seconds left at the current speed.
    pub fn eta(&self) -> Option<f64> {
        let total = self.total?;
        (self.state == JobState::Downloading && self.speed > 1.0)
            .then(|| total.saturating_sub(self.downloaded) as f64 / self.speed)
    }
}

/// What gets saved between sessions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRecord {
    pub spec: JobSpec,
    #[serde(flatten)]
    pub state: JobState,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub total: Option<u64>,
    #[serde(default)]
    pub downloaded: u64,
    #[serde(default)]
    pub path: Option<PathBuf>,
}

/// Something the application may want to react to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManagerEvent {
    JobFinished {
        id: JobId,
        name: String,
        path: PathBuf,
    },
    JobFailed {
        id: JobId,
        name: String,
        error: String,
    },
    /// Every file of a package is finished.
    PackageFinished {
        package: String,
        dir: PathBuf,
        files: Vec<PathBuf>,
        password: Option<String>,
    },
}

/// Queue settings, usually derived from [`crate::Settings`] and modules.
#[derive(Clone, Debug)]
pub struct ManagerConfig {
    pub dest_dir: PathBuf,
    pub max_parallel: usize,
    pub connections: usize,
    /// `multi-connection` module.
    pub multi_connection: bool,
    /// `resume` module.
    pub resume: bool,
    pub if_exists: bd_net::IfExists,
    pub preallocate: bool,
}

struct Job {
    id: JobId,
    spec: JobSpec,
    name: Option<String>,
    state: JobState,
    slot: Option<Arc<ControlSlot>>,
    waiter: Option<JoinHandle<()>>,
    /// Incremented on every start so late completions of an old run are ignored.
    generation: u64,
    total: Option<u64>,
    downloaded: u64,
    segments: Vec<SegmentProgress>,
    connections: usize,
    speed: f64,
    last_sample: Option<(Instant, u64)>,
    path: Option<PathBuf>,
    resumed: bool,
    /// Start even when all parallel slots are taken.
    force: bool,
}

impl Job {
    fn new(id: JobId, spec: JobSpec) -> Self {
        Job {
            id,
            name: spec.name.clone(),
            spec,
            state: JobState::Queued,
            slot: None,
            waiter: None,
            generation: 0,
            total: None,
            downloaded: 0,
            segments: Vec::new(),
            connections: 0,
            speed: 0.0,
            last_sample: None,
            path: None,
            resumed: false,
            force: false,
        }
    }

    fn display_name(&self) -> String {
        self.name
            .clone()
            .or_else(|| bd_net::file_name_from_url(&self.spec.url))
            .unwrap_or_else(|| self.spec.url.to_string())
    }
}

struct Inner {
    jobs: Vec<Job>,
    next_id: JobId,
    config: ManagerConfig,
    video: Option<VideoBackend>,
    resolver: Arc<dyn LinkResolver>,
    /// Packages already reported as finished.
    reported: HashSet<String>,
    events: Sender<ManagerEvent>,
    /// Bumped on every change worth saving.
    revision: u64,
}

#[derive(Clone)]
pub struct DownloadManager {
    inner: Arc<Mutex<Inner>>,
    events: Arc<Mutex<Option<Receiver<ManagerEvent>>>>,
    engine: Engine,
    runtime: Handle,
}

impl DownloadManager {
    pub fn new(engine: Engine, config: ManagerConfig, runtime: Handle) -> Self {
        let (tx, rx) = channel();
        DownloadManager {
            inner: Arc::new(Mutex::new(Inner {
                jobs: Vec::new(),
                next_id: 1,
                config,
                video: None,
                resolver: Arc::new(NoResolver),
                reported: HashSet::new(),
                events: tx,
                revision: 0,
            })),
            events: Arc::new(Mutex::new(Some(rx))),
            engine,
            runtime,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Takes the event receiver (once).
    pub fn take_events(&self) -> Option<Receiver<ManagerEvent>> {
        self.events.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn config(&self) -> ManagerConfig {
        self.lock().config.clone()
    }

    pub fn update_config(&self, change: impl FnOnce(&mut ManagerConfig)) {
        change(&mut self.lock().config);
        self.schedule();
    }

    pub fn set_video_backend(&self, backend: Option<VideoBackend>) {
        self.lock().video = backend;
    }

    pub fn set_resolver(&self, resolver: Arc<dyn LinkResolver>) {
        self.lock().resolver = resolver;
    }

    /// Global speed limit in bytes per second, 0 for none.
    pub fn set_speed_limit(&self, bytes_per_sec: u64) {
        self.engine.set_speed_limit(bytes_per_sec);
    }

    /// Changes every time something worth saving changes.
    pub fn revision(&self) -> u64 {
        self.lock().revision
    }

    /// Queues a plain link in the default folder and returns its id.
    pub fn add(&self, url: Url) -> JobId {
        let dir = self.lock().config.dest_dir.clone();
        self.add_spec(JobSpec::http(url, dir))
    }

    pub fn add_spec(&self, spec: JobSpec) -> JobId {
        let id = {
            let mut inner = self.lock();
            let id = inner.next_id;
            inner.next_id += 1;
            inner.jobs.push(Job::new(id, spec));
            inner.revision += 1;
            id
        };
        self.schedule();
        id
    }

    pub fn pause(&self, id: JobId) {
        let mut inner = self.lock();
        if let Some(job) = inner.jobs.iter_mut().find(|j| j.id == id) {
            if matches!(job.state, JobState::Downloading | JobState::Queued) {
                job.state = JobState::Paused;
                job.speed = 0.0;
                if let Some(slot) = &job.slot {
                    slot.cancel();
                }
            }
        }
        inner.revision += 1;
    }

    /// Resumes a paused job or retries a failed one.
    pub fn resume(&self, id: JobId) {
        {
            let mut inner = self.lock();
            if let Some(job) = inner.jobs.iter_mut().find(|j| j.id == id) {
                if matches!(job.state, JobState::Paused | JobState::Failed(_)) {
                    job.state = JobState::Queued;
                }
            }
            inner.revision += 1;
        }
        self.schedule();
    }

    pub fn pause_all(&self) {
        let ids: Vec<JobId> = self.lock().jobs.iter().map(|j| j.id).collect();
        ids.into_iter().for_each(|id| self.pause(id));
    }

    pub fn resume_all(&self) {
        {
            let mut inner = self.lock();
            for job in inner.jobs.iter_mut().filter(|j| j.state == JobState::Paused) {
                job.state = JobState::Queued;
            }
            inner.revision += 1;
        }
        self.schedule();
    }

    /// Removes every finished job from the list (files stay on disk).
    pub fn clear_finished(&self) {
        let mut inner = self.lock();
        inner.jobs.retain(|j| j.state != JobState::Finished);
        inner.revision += 1;
    }

    /// Removes a job. With `delete_files`, also deletes the partial or
    /// finished file from disk.
    pub fn remove(&self, id: JobId, delete_files: bool) {
        let removed = {
            let mut inner = self.lock();
            let Some(pos) = inner.jobs.iter().position(|j| j.id == id) else { return };
            inner.revision += 1;
            inner.jobs.remove(pos)
        };
        if let Some(slot) = &removed.slot {
            slot.cancel();
        }
        if delete_files {
            let name = removed.name.clone();
            let dir = removed.spec.dest_dir.clone();
            let path = removed.path.clone();
            let waiter = removed.waiter;
            self.runtime.spawn(async move {
                // Let the cancelled task release its file handles first.
                if let Some(waiter) = waiter {
                    let _ = tokio::time::timeout(Duration::from_secs(5), waiter).await;
                }
                delete_job_files(&dir, name.as_deref(), path.as_deref()).await;
            });
        }
        self.schedule();
    }

    /// Refreshes progress from running downloads and returns all jobs.
    pub fn snapshot(&self) -> Vec<JobSnapshot> {
        let mut inner = self.lock();
        let now = Instant::now();
        for job in inner.jobs.iter_mut() {
            let Some(p) = job.slot.as_ref().and_then(|s| s.progress()) else { continue };
            job.name = p.file_name.clone().or(job.name.take());
            job.total = p.total;
            job.downloaded = p.downloaded;
            job.segments = p.segments;
            job.connections = p.active_connections;
            job.resumed = p.resumed;
            if job.state == JobState::Downloading {
                match job.last_sample {
                    Some((at, bytes)) => {
                        let dt = now.duration_since(at).as_secs_f64();
                        if dt >= 0.2 {
                            let instant = p.downloaded.saturating_sub(bytes) as f64 / dt;
                            job.speed = if job.speed == 0.0 { instant } else { job.speed * 0.7 + instant * 0.3 };
                            job.last_sample = Some((now, p.downloaded));
                        }
                    }
                    // The first sample may include bytes from a previous run.
                    None => job.last_sample = Some((now, p.downloaded)),
                }
            }
        }
        inner
            .jobs
            .iter()
            .map(|j| JobSnapshot {
                id: j.id,
                spec: j.spec.clone(),
                url: j.spec.url.clone(),
                name: j.display_name(),
                state: j.state.clone(),
                total: j.total,
                downloaded: j.downloaded,
                speed: if j.state == JobState::Downloading { j.speed } else { 0.0 },
                segments: j.segments.clone(),
                connections: j.connections,
                dest_dir: j.spec.dest_dir.clone(),
                path: j.path.clone(),
                resumed: j.resumed,
                status: j.slot.as_ref().and_then(|s| s.status()),
            })
            .collect()
    }

    /// Jobs to save so the queue survives a restart.
    pub fn records(&self) -> Vec<JobRecord> {
        self.lock()
            .jobs
            .iter()
            .map(|j| JobRecord {
                spec: j.spec.clone(),
                // A running job continues next time.
                state: if j.state == JobState::Downloading { JobState::Queued } else { j.state.clone() },
                name: j.name.clone(),
                total: j.total,
                downloaded: j.downloaded,
                path: j.path.clone(),
            })
            .collect()
    }

    /// Restores saved jobs. Queued ones start when `start` is true, otherwise
    /// they come back paused.
    pub fn restore(&self, records: Vec<JobRecord>, start: bool) {
        {
            let mut inner = self.lock();
            for record in records {
                let id = inner.next_id;
                inner.next_id += 1;
                let mut job = Job::new(id, record.spec);
                job.state = match record.state {
                    JobState::Queued | JobState::Downloading if !start => JobState::Paused,
                    JobState::Downloading => JobState::Queued,
                    other => other,
                };
                job.name = record.name.or(job.name);
                job.total = record.total;
                job.downloaded = record.downloaded;
                job.path = record.path;
                if let Some(package) = &job.spec.package {
                    if job.state == JobState::Finished {
                        // Do not report old packages again.
                        inner.reported.insert(package.clone());
                    }
                }
                inner.jobs.push(job);
            }
        }
        self.schedule();
    }

    /// Stops every running download, saving progress, and waits for them.
    pub async fn shutdown(&self) {
        let waiters: Vec<JoinHandle<()>> = {
            let mut inner = self.lock();
            inner
                .jobs
                .iter_mut()
                .filter_map(|job| {
                    if let Some(slot) = &job.slot {
                        slot.cancel();
                    }
                    job.waiter.take()
                })
                .collect()
        };
        for waiter in waiters {
            let _ = tokio::time::timeout(Duration::from_secs(5), waiter).await;
        }
    }

    fn with_job<T>(&self, id: JobId, f: impl FnOnce(&mut Job) -> T) -> Option<T> {
        let mut inner = self.lock();
        inner.revision += 1;
        inner.jobs.iter_mut().find(|j| j.id == id).map(f)
    }

    pub fn set_priority(&self, id: JobId, priority: i32) {
        self.with_job(id, |j| j.spec.priority = priority.clamp(-2, 2));
        self.schedule();
    }

    /// Moves a job in the list (the list order is the start order for jobs
    /// of the same priority).
    pub fn move_job(&self, id: JobId, to: Move) {
        let mut inner = self.lock();
        let Some(pos) = inner.jobs.iter().position(|j| j.id == id) else { return };
        let last = inner.jobs.len() - 1;
        let target = match to {
            Move::Top => 0,
            Move::Up => pos.saturating_sub(1),
            Move::Down => (pos + 1).min(last),
            Move::Bottom => last,
        };
        let job = inner.jobs.remove(pos);
        inner.jobs.insert(target, job);
        inner.revision += 1;
    }

    /// Starts now, even when every parallel slot is taken.
    pub fn force_start(&self, id: JobId) {
        self.with_job(id, |j| {
            if matches!(j.state, JobState::Queued | JobState::Paused | JobState::Failed(_)) {
                j.state = JobState::Queued;
                j.force = true;
            }
        });
        self.schedule();
    }

    /// Speed limit for one file in bytes per second (0 removes it). Applies
    /// immediately to a running download.
    pub fn set_job_limit(&self, id: JobId, bytes_per_sec: u64) {
        self.with_job(id, |j| {
            j.spec.rate_limit = bytes_per_sec;
            if let Some(slot) = &j.slot {
                slot.set_speed_limit(bytes_per_sec);
            }
        });
    }

    /// Connections for one file, used from its next start.
    pub fn set_job_connections(&self, id: JobId, connections: usize) {
        self.with_job(id, |j| j.spec.connections = Some(connections.clamp(1, 32)));
    }

    /// Renames a job. Before it starts the new name is used for the file;
    /// once finished the file itself is renamed.
    pub fn rename(&self, id: JobId, new_name: &str) -> Result<(), String> {
        let name = bd_net::sanitize_file_name(new_name.trim());
        self.with_job(id, |j| match j.state {
            JobState::Downloading => Err("pause the download before renaming it".to_owned()),
            JobState::Finished => {
                let Some(old) = j.path.clone() else { return Err("no file".to_owned()) };
                let new = old.with_file_name(&name);
                std::fs::rename(&old, &new).map_err(|e| e.to_string())?;
                j.path = Some(new);
                j.name = Some(name.clone());
                Ok(())
            }
            _ => {
                if j.downloaded > 0 {
                    // Move the partial files so the download keeps its progress.
                    if let Some(old) = j.name.clone() {
                        for suffix in [".part", ".part.json"] {
                            let _ = std::fs::rename(
                                j.spec.dest_dir.join(format!("{old}{suffix}")),
                                j.spec.dest_dir.join(format!("{name}{suffix}")),
                            );
                        }
                    }
                }
                j.spec.name = Some(name.clone());
                j.name = Some(name.clone());
                Ok(())
            }
        })
        .unwrap_or_else(|| Err("unknown download".to_owned()))
    }

    /// Changes the folder of a job, moving its files when it has some.
    pub fn change_folder(&self, id: JobId, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        self.with_job(id, |j| match j.state {
            JobState::Downloading => Err("pause the download before moving it".to_owned()),
            JobState::Finished => {
                let Some(old) = j.path.clone() else { return Err("no file".to_owned()) };
                let new = dir.join(old.file_name().unwrap_or_default());
                move_file(&old, &new)?;
                j.path = Some(new);
                j.spec.dest_dir = dir.to_path_buf();
                Ok(())
            }
            _ => {
                if let Some(name) = j.name.clone() {
                    for suffix in [".part", ".part.json"] {
                        let from = j.spec.dest_dir.join(format!("{name}{suffix}"));
                        if from.exists() {
                            move_file(&from, &dir.join(format!("{name}{suffix}")))?;
                        }
                    }
                }
                j.spec.dest_dir = dir.to_path_buf();
                Ok(())
            }
        })
        .unwrap_or_else(|| Err("unknown download".to_owned()))
    }

    /// Stops a job and throws its partial data away. With `start`, it starts
    /// again from zero; otherwise it stays paused.
    pub fn reset(&self, id: JobId, start: bool) {
        let cleanup = self.with_job(id, |j| {
            if let Some(slot) = j.slot.take() {
                slot.cancel();
            }
            j.generation += 1;
            let files = (j.spec.dest_dir.clone(), j.name.clone(), j.waiter.take());
            j.state = if start { JobState::Queued } else { JobState::Paused };
            j.downloaded = 0;
            j.total = None;
            j.segments.clear();
            j.speed = 0.0;
            j.path = None;
            files
        });
        if let Some((dir, name, waiter)) = cleanup {
            let manager = self.clone();
            self.runtime.spawn(async move {
                if let Some(waiter) = waiter {
                    let _ = tokio::time::timeout(Duration::from_secs(5), waiter).await;
                }
                delete_job_files(&dir, name.as_deref(), None).await;
                if start {
                    manager.schedule();
                }
            });
        }
    }

    /// Starts queued jobs while there are free slots.
    fn schedule(&self) {
        let mut inner = self.lock();
        let running = inner.jobs.iter().filter(|j| j.state == JobState::Downloading).count();
        let mut free = inner.config.max_parallel.saturating_sub(running);
        let config = inner.config.clone();
        let video = inner.video.clone();
        let resolver = inner.resolver.clone();
        // Forced jobs first, then by priority, then in list order.
        let mut order: Vec<usize> =
            (0..inner.jobs.len()).filter(|&i| inner.jobs[i].state == JobState::Queued).collect();
        order.sort_by_key(|&i| (!inner.jobs[i].force, -inner.jobs[i].spec.priority, i));
        for index in order {
            let job = &mut inner.jobs[index];
            if free == 0 && !job.force {
                continue;
            }
            free = free.saturating_sub(1);
            job.force = false;
            let slot = Arc::new(ControlSlot::default());
            let future = match &job.spec.kind {
                JobKind::Http => {
                    run_http(self.engine.clone(), resolver.clone(), job.spec.clone(), config.clone(), slot.clone())
                }
                JobKind::Video { format } => match &video {
                    Some(backend) => backend.run(
                        job.spec.url.clone(),
                        format.clone(),
                        job.spec.dest_dir.clone(),
                        self.engine.speed_limit(),
                        slot.clone(),
                    ),
                    None => Box::pin(async {
                        Err(RunError::Failed("turn on the video module and install yt-dlp in Settings".to_owned()))
                    }),
                },
            };
            job.generation += 1;
            job.state = JobState::Downloading;
            job.slot = Some(slot);
            job.last_sample = None;
            job.speed = 0.0;
            let (id, generation, manager) = (job.id, job.generation, self.clone());
            job.waiter = Some(self.runtime.spawn(async move {
                let result = future.await;
                manager.finish(id, generation, result);
            }));
        }
    }

    fn finish(&self, id: JobId, generation: u64, result: RunResult) {
        {
            let mut inner = self.lock();
            let inner = &mut *inner;
            let Some(job) = inner.jobs.iter_mut().find(|j| j.id == id && j.generation == generation) else { return };
            if let Some(p) = job.slot.take().and_then(|s| s.progress()) {
                job.name = p.file_name.or(job.name.take());
                job.total = p.total;
                job.downloaded = p.downloaded;
                job.segments = p.segments;
            }
            job.connections = 0;
            job.speed = 0.0;
            job.waiter = None;
            inner.revision += 1;
            match result {
                Ok(path) => {
                    job.name = path.file_name().map(|n| n.to_string_lossy().into_owned()).or(job.name.take());
                    job.path = Some(path.clone());
                    job.state = JobState::Finished;
                    let _ = inner.events.send(ManagerEvent::JobFinished { id, name: job.display_name(), path });
                    if let Some(package) = job.spec.package.clone() {
                        report_package(inner, &package);
                    }
                }
                // Paused by the user: keep the state. Cancelled otherwise
                // (shutdown): queue it again for next time.
                Err(RunError::Cancelled) => {
                    if job.state == JobState::Downloading {
                        job.state = JobState::Queued;
                    }
                }
                Err(RunError::Failed(error)) => {
                    tracing::warn!(error = %error, url = %job.spec.url, "download failed");
                    let _ = inner.events.send(ManagerEvent::JobFailed {
                        id,
                        name: job.display_name(),
                        error: error.clone(),
                    });
                    job.state = JobState::Failed(error);
                }
            }
        }
        self.schedule();
    }
}

fn report_package(inner: &mut Inner, package: &str) {
    if inner.reported.contains(package) {
        return;
    }
    let members: Vec<&Job> = inner.jobs.iter().filter(|j| j.spec.package.as_deref() == Some(package)).collect();
    if members.is_empty() || !members.iter().all(|j| j.state == JobState::Finished) {
        return;
    }
    let dir = members[0].spec.dest_dir.clone();
    let password = members.iter().find_map(|j| j.spec.password.clone());
    let files = members.iter().filter_map(|j| j.path.clone()).collect();
    inner.reported.insert(package.to_owned());
    let _ = inner.events.send(ManagerEvent::PackageFinished { package: package.to_owned(), dir, files, password });
}

/// Resolves the link if needed, then downloads it with the HTTP engine.
fn run_http(
    engine: Engine,
    resolver: Arc<dyn LinkResolver>,
    spec: JobSpec,
    config: ManagerConfig,
    slot: Arc<ControlSlot>,
) -> futures_util::future::BoxFuture<'static, RunResult> {
    Box::pin(async move {
        slot.set_status(Some("Resolving link…"));
        let resolved = resolver.resolve(&spec.url, slot.clone()).await.map_err(RunError::Failed)?;
        slot.set_status(None);
        if slot.is_cancelled() {
            return Err(RunError::Cancelled);
        }
        let connections = spec.connections.unwrap_or(config.connections);
        let mut request = DownloadRequest::new(resolved.url, spec.dest_dir.clone())
            .connections(if config.multi_connection { connections } else { 1 })
            .resume(config.resume);
        request.headers = resolved.headers;
        request.rate_limit = spec.rate_limit;
        request.if_exists = config.if_exists;
        request.preallocate = config.preallocate;
        if let Some(name) = spec.name.clone().or(resolved.file_name) {
            request = request.file_name(name);
        }
        let task = engine.start(request);
        slot.set(Arc::new(task.control()));
        task.wait().await.map_err(RunError::from)
    })
}

async fn delete_job_files(dir: &Path, name: Option<&str>, path: Option<&Path>) {
    if let Some(path) = path {
        let _ = tokio::fs::remove_file(path).await;
    }
    if let Some(name) = name {
        for suffix in [".part", ".part.json"] {
            let _ = tokio::fs::remove_file(dir.join(format!("{name}{suffix}"))).await;
        }
    }
}

/// Where to move a job in the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Top,
    Up,
    Down,
    Bottom,
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Renames, or copies then deletes when the target is on another drive.
fn move_file(from: &Path, to: &Path) -> Result<(), String> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to).map_err(|e| e.to_string())?;
    std::fs::remove_file(from).map_err(|e| e.to_string())
}

/// Reads a saved queue. A missing or broken file gives an empty queue.
pub fn load_records(path: &Path) -> Vec<JobRecord> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            tracing::warn!(error = %e, path = %path.display(), "invalid queue file, starting empty");
            Vec::new()
        }),
        Err(_) => Vec::new(),
    }
}

pub fn save_records(path: &Path, records: &[JobRecord]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(records).map_err(std::io::Error::other)?)?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_round_trip_as_json() {
        let records = vec![
            JobRecord {
                spec: JobSpec {
                    package: Some("Pack".into()),
                    password: Some("pw".into()),
                    ..JobSpec::http(Url::parse("https://a.example/f.rar").unwrap(), "C:/dl")
                },
                state: JobState::Failed("boom".into()),
                name: Some("f.rar".into()),
                total: Some(10),
                downloaded: 4,
                path: None,
            },
            JobRecord {
                spec: JobSpec {
                    kind: JobKind::Video { format: VideoFormat::MaxHeight { height: 720 } },
                    ..JobSpec::http(Url::parse("https://youtu.be/x").unwrap(), "C:/dl")
                },
                state: JobState::Queued,
                name: None,
                total: None,
                downloaded: 0,
                path: None,
            },
        ];
        let json = serde_json::to_string(&records).unwrap();
        let back: Vec<JobRecord> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, records);
    }
}
