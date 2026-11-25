//! Download queue: runs up to `max_parallel` downloads, lets the UI pause,
//! resume and remove them, and exposes snapshots with live speed.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use bd_net::{DownloadControl, DownloadRequest, Engine, Error as NetError, SegmentProgress};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use url::Url;

pub type JobId = u64;

#[derive(Clone, Debug, PartialEq, Eq)]
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
}

impl JobSnapshot {
    /// Seconds left at the current speed.
    pub fn eta(&self) -> Option<f64> {
        let total = self.total?;
        (self.state == JobState::Downloading && self.speed > 1.0)
            .then(|| total.saturating_sub(self.downloaded) as f64 / self.speed)
    }
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
}

struct Job {
    id: JobId,
    url: Url,
    name: Option<String>,
    dest_dir: PathBuf,
    state: JobState,
    control: Option<DownloadControl>,
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
}

impl Job {
    fn display_name(&self) -> String {
        self.name.clone().or_else(|| bd_net::file_name_from_url(&self.url)).unwrap_or_else(|| self.url.to_string())
    }
}

struct Inner {
    jobs: Vec<Job>,
    next_id: JobId,
    config: ManagerConfig,
}

#[derive(Clone)]
pub struct DownloadManager {
    inner: Arc<Mutex<Inner>>,
    engine: Engine,
    runtime: Handle,
}

impl DownloadManager {
    pub fn new(engine: Engine, config: ManagerConfig, runtime: Handle) -> Self {
        DownloadManager { inner: Arc::new(Mutex::new(Inner { jobs: Vec::new(), next_id: 1, config })), engine, runtime }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn config(&self) -> ManagerConfig {
        self.lock().config.clone()
    }

    pub fn update_config(&self, change: impl FnOnce(&mut ManagerConfig)) {
        change(&mut self.lock().config);
        self.schedule();
    }

    /// Queues a link and returns its id.
    pub fn add(&self, url: Url) -> JobId {
        let id = {
            let mut inner = self.lock();
            let id = inner.next_id;
            inner.next_id += 1;
            let dest_dir = inner.config.dest_dir.clone();
            inner.jobs.push(Job {
                id,
                url,
                name: None,
                dest_dir,
                state: JobState::Queued,
                control: None,
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
            });
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
                if let Some(control) = &job.control {
                    control.cancel();
                }
            }
        }
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
        }
        self.schedule();
    }

    /// Removes a job. With `delete_files`, also deletes the partial or
    /// finished file from disk.
    pub fn remove(&self, id: JobId, delete_files: bool) {
        let removed = {
            let mut inner = self.lock();
            let Some(pos) = inner.jobs.iter().position(|j| j.id == id) else { return };
            inner.jobs.remove(pos)
        };
        if let Some(control) = &removed.control {
            control.cancel();
        }
        if delete_files {
            let name = removed.name.clone();
            let dir = removed.dest_dir.clone();
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
            if let Some(control) = &job.control {
                let p = control.progress();
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
        }
        inner
            .jobs
            .iter()
            .map(|j| JobSnapshot {
                id: j.id,
                url: j.url.clone(),
                name: j.display_name(),
                state: j.state.clone(),
                total: j.total,
                downloaded: j.downloaded,
                speed: if j.state == JobState::Downloading { j.speed } else { 0.0 },
                segments: j.segments.clone(),
                connections: j.connections,
                dest_dir: j.dest_dir.clone(),
                path: j.path.clone(),
                resumed: j.resumed,
            })
            .collect()
    }

    /// Stops every running download, saving progress, and waits for them.
    pub async fn shutdown(&self) {
        let waiters: Vec<JoinHandle<()>> = {
            let mut inner = self.lock();
            inner
                .jobs
                .iter_mut()
                .filter_map(|job| {
                    if let Some(control) = &job.control {
                        control.cancel();
                    }
                    job.waiter.take()
                })
                .collect()
        };
        for waiter in waiters {
            let _ = tokio::time::timeout(Duration::from_secs(5), waiter).await;
        }
    }

    /// Starts queued jobs while there are free slots.
    fn schedule(&self) {
        let mut inner = self.lock();
        let running = inner.jobs.iter().filter(|j| j.state == JobState::Downloading).count();
        let mut free = inner.config.max_parallel.saturating_sub(running);
        let config = inner.config.clone();
        for job in inner.jobs.iter_mut() {
            if free == 0 {
                break;
            }
            if job.state != JobState::Queued {
                continue;
            }
            free -= 1;
            let mut request = DownloadRequest::new(job.url.clone(), job.dest_dir.clone())
                .connections(if config.multi_connection { config.connections } else { 1 })
                .resume(config.resume);
            if let Some(name) = &job.name {
                request = request.file_name(name.clone());
            }
            let _guard = self.runtime.enter();
            let task = self.engine.start(request);
            job.generation += 1;
            job.state = JobState::Downloading;
            job.control = Some(task.control());
            job.last_sample = None;
            job.speed = 0.0;
            let (id, generation, manager) = (job.id, job.generation, self.clone());
            job.waiter = Some(self.runtime.spawn(async move {
                let result = task.wait().await;
                manager.finish(id, generation, result);
            }));
        }
    }

    fn finish(&self, id: JobId, generation: u64, result: bd_net::Result<PathBuf>) {
        {
            let mut inner = self.lock();
            let Some(job) = inner.jobs.iter_mut().find(|j| j.id == id && j.generation == generation) else { return };
            if let Some(control) = job.control.take() {
                let p = control.progress();
                job.name = p.file_name.or(job.name.take());
                job.total = p.total;
                job.downloaded = p.downloaded;
                job.segments = p.segments;
            }
            job.connections = 0;
            job.speed = 0.0;
            job.waiter = None;
            match result {
                Ok(path) => {
                    job.name = path.file_name().map(|n| n.to_string_lossy().into_owned()).or(job.name.take());
                    job.path = Some(path);
                    job.state = JobState::Finished;
                }
                // Paused by the user: keep the state. Cancelled otherwise
                // (shutdown): queue it again for next time.
                Err(NetError::Cancelled) => {
                    if job.state == JobState::Downloading {
                        job.state = JobState::Queued;
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, url = %job.url, "download failed");
                    job.state = JobState::Failed(e.to_string());
                }
            }
        }
        self.schedule();
    }
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
