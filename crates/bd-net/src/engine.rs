use std::collections::HashMap;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::RANGE;
use reqwest::StatusCode;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Semaphore;
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::name::sanitize_file_name;
use crate::probe::{probe, RemoteFile};
use crate::rate::RateLimiter;
use crate::state::{self, ResumeState, Segment, STATE_VERSION};
use crate::{Error, Result};

const DEFAULT_MIN_SEGMENT: u64 = 1024 * 1024;
const DEFAULT_RETRIES: u32 = 5;
const DEFAULT_MAX_PER_HOST: usize = 16;
const SAVE_INTERVAL: Duration = Duration::from_secs(1);

/// What to download and where.
#[derive(Clone, Debug)]
pub struct DownloadRequest {
    pub url: Url,
    pub dest_dir: PathBuf,
    /// Overrides the name suggested by the server.
    pub file_name: Option<String>,
    /// Maximum parallel connections for this file.
    pub connections: usize,
    /// Continue from saved progress when possible (default `true`).
    pub resume: bool,
    /// Extra headers sent with every request (cookies, authorization).
    pub headers: Vec<(String, String)>,
    /// Speed limit for this file in bytes per second, 0 for none.
    pub rate_limit: u64,
    /// What to do when the destination file already exists.
    pub if_exists: IfExists,
    /// Reserve the whole file on disk before writing.
    pub preallocate: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IfExists {
    /// Save as `name (1).ext`.
    #[default]
    Rename,
    /// Consider the existing file as the result.
    Skip,
    Overwrite,
}

impl DownloadRequest {
    pub fn new(url: Url, dest_dir: impl Into<PathBuf>) -> Self {
        DownloadRequest {
            url,
            dest_dir: dest_dir.into(),
            file_name: None,
            connections: 8,
            resume: true,
            headers: Vec::new(),
            rate_limit: 0,
            if_exists: IfExists::Rename,
            preallocate: true,
        }
    }

    pub fn resume(mut self, resume: bool) -> Self {
        self.resume = resume;
        self
    }

    pub fn connections(mut self, connections: usize) -> Self {
        self.connections = connections.max(1);
        self
    }

    pub fn file_name(mut self, name: impl Into<String>) -> Self {
        self.file_name = Some(name.into());
        self
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

/// Progress of one connection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SegmentProgress {
    pub done: u64,
    pub len: Option<u64>,
}

/// Snapshot of a running download.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub file_name: Option<String>,
    pub total: Option<u64>,
    pub downloaded: u64,
    pub segments: Vec<SegmentProgress>,
    pub active_connections: usize,
    /// True when this run continued a previous partial download.
    pub resumed: bool,
}

#[derive(Default)]
struct Inner {
    file_name: Option<String>,
    total: Option<u64>,
    segments: Vec<Segment>,
    active: usize,
    resumed: bool,
}

#[derive(Default)]
struct Shared {
    inner: Mutex<Inner>,
    /// Per-download speed limit, changeable while running.
    rate: RateLimiter,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic in a worker must not take the whole progress view down.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn segment(&self, idx: usize) -> Segment {
        self.lock().segments[idx]
    }

    fn snapshot(&self) -> Progress {
        let inner = self.lock();
        Progress {
            file_name: inner.file_name.clone(),
            total: inner.total,
            downloaded: inner.segments.iter().map(|s| s.done).sum(),
            segments: inner.segments.iter().map(|s| SegmentProgress { done: s.done, len: s.len() }).collect(),
            active_connections: inner.active,
            resumed: inner.resumed,
        }
    }
}

/// Decrements the active connection counter when a request ends.
struct ActiveGuard(Arc<Shared>);

impl ActiveGuard {
    fn new(shared: &Arc<Shared>) -> Self {
        shared.lock().active += 1;
        ActiveGuard(shared.clone())
    }
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.lock().active -= 1;
    }
}

/// Cloneable view on a running download: read progress, cancel it.
#[derive(Clone)]
pub struct DownloadControl {
    shared: Arc<Shared>,
    cancel: CancellationToken,
}

impl DownloadControl {
    /// Changes the speed limit of this download while it runs (0 = none).
    pub fn set_speed_limit(&self, bytes_per_sec: u64) {
        self.shared.rate.set_limit(bytes_per_sec);
    }

    pub fn progress(&self) -> Progress {
        self.shared.snapshot()
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

/// A download running in the background.
pub struct DownloadTask {
    shared: Arc<Shared>,
    cancel: CancellationToken,
    handle: JoinHandle<Result<PathBuf>>,
}

impl DownloadTask {
    /// Handle that stays usable after `wait` consumed the task.
    pub fn control(&self) -> DownloadControl {
        DownloadControl { shared: self.shared.clone(), cancel: self.cancel.clone() }
    }

    pub fn progress(&self) -> Progress {
        self.shared.snapshot()
    }

    /// Stops the download. Progress is saved so it can resume later.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }

    /// Waits for the end and returns the path of the finished file.
    pub async fn wait(self) -> Result<PathBuf> {
        match self.handle.await {
            Ok(result) => result,
            Err(e) => Err(Error::Io(std::io::Error::other(e))),
        }
    }
}

/// Download engine. Cheap to clone: clones share the HTTP connection pool.
#[derive(Clone)]
pub struct Engine {
    client: reqwest::Client,
    min_segment: u64,
    retries: u32,
    max_per_host: usize,
    /// Connection limits shared by every download to the same host, so
    /// several downloads together respect what the server accepts.
    hosts: Arc<Mutex<HashMap<String, Arc<ConnectionLimiter>>>>,
    /// Global bandwidth limit.
    rate: Arc<RateLimiter>,
}

impl Engine {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("BDownloader/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self::with_client(client))
    }

    pub fn with_client(client: reqwest::Client) -> Self {
        Engine {
            client,
            min_segment: DEFAULT_MIN_SEGMENT,
            retries: DEFAULT_RETRIES,
            max_per_host: DEFAULT_MAX_PER_HOST,
            hosts: Arc::default(),
            rate: Arc::default(),
        }
    }

    /// Smallest segment worth its own connection.
    pub fn min_segment(mut self, bytes: u64) -> Self {
        self.min_segment = bytes.max(1);
        self
    }

    pub fn retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }

    /// Starting limit of simultaneous connections to one host. It is lowered
    /// automatically when the host answers 429 or 503.
    pub fn max_connections_per_host(mut self, limit: usize) -> Self {
        self.max_per_host = limit.max(1);
        self
    }

    fn host_limiter(&self, url: &Url) -> Arc<ConnectionLimiter> {
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        let mut hosts = self.hosts.lock().unwrap_or_else(|e| e.into_inner());
        hosts.entry(host).or_insert_with(|| Arc::new(ConnectionLimiter::new(self.max_per_host))).clone()
    }

    /// Global speed limit in bytes per second for every download of this
    /// engine and its clones. 0 removes the limit. Takes effect immediately.
    pub fn set_speed_limit(&self, bytes_per_sec: u64) {
        self.rate.set_limit(bytes_per_sec);
    }

    pub fn speed_limit(&self) -> u64 {
        self.rate.limit()
    }

    /// The shared HTTP client, for callers that talk to APIs.
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    pub async fn probe(&self, url: &Url) -> Result<RemoteFile> {
        probe(&self.client, url, &[]).await
    }

    pub async fn probe_with_headers(&self, url: &Url, headers: &[(String, String)]) -> Result<RemoteFile> {
        probe(&self.client, url, headers).await
    }

    /// Starts a download on the current Tokio runtime.
    pub fn start(&self, request: DownloadRequest) -> DownloadTask {
        let shared = Arc::new(Shared::default());
        let cancel = CancellationToken::new();
        let handle = tokio::spawn(run(self.clone(), request, shared.clone(), cancel.clone()));
        DownloadTask { shared, cancel, handle }
    }
}

async fn run(
    engine: Engine,
    request: DownloadRequest,
    shared: Arc<Shared>,
    cancel: CancellationToken,
) -> Result<PathBuf> {
    let remote = tokio::select! {
        _ = cancel.cancelled() => return Err(Error::Cancelled),
        remote = engine.probe_with_headers(&request.url, &request.headers) => remote?,
    };
    let name = request
        .file_name
        .as_deref()
        .map(sanitize_file_name)
        .or_else(|| remote.file_name.clone())
        .unwrap_or_else(|| "download".to_owned());
    tokio::fs::create_dir_all(&request.dest_dir).await?;
    let dest = request.dest_dir.join(&name);
    let part = state::part_path(&dest);
    let state_file = state::state_path(&dest);
    shared.rate.set_limit(request.rate_limit);

    if request.if_exists == IfExists::Skip && tokio::fs::metadata(&dest).await.is_ok_and(|m| m.is_file()) {
        let mut inner = shared.lock();
        inner.file_name = Some(name.clone());
        inner.total = remote.size;
        return Ok(dest);
    }

    let (segments, resumed) = prepare(&engine, &remote, &request, &part, &state_file).await?;
    {
        let mut inner = shared.lock();
        inner.file_name = Some(name.clone());
        inner.total = remote.size;
        inner.segments = segments;
        inner.resumed = resumed;
    }
    let resumable = remote.ranges && remote.size.is_some();
    let save = |shared: &Arc<Shared>| {
        let state = ResumeState {
            version: STATE_VERSION,
            size: remote.size,
            etag: remote.etag.clone(),
            last_modified: remote.last_modified.clone(),
            segments: shared.lock().segments.clone(),
        };
        let path = state_file.clone();
        async move { state::save(&path, &state).await }
    };
    if resumable {
        save(&shared).await?;
    }

    let mut workers = JoinSet::new();
    let count = shared.lock().segments.len();
    let limiter = Arc::new(ConnectionLimiter::new(request.connections));
    let headers = Arc::new(request.headers.clone());
    for idx in 0..count {
        if !shared.segment(idx).is_complete() {
            workers.spawn(worker(
                engine.clone(),
                remote.url.clone(),
                part.clone(),
                shared.clone(),
                idx,
                remote.ranges,
                limiter.clone(),
                headers.clone(),
                cancel.clone(),
            ));
        }
    }

    let mut failure: Option<Error> = None;
    let mut ticker = tokio::time::interval(SAVE_INTERVAL);
    ticker.tick().await;
    loop {
        tokio::select! {
            joined = workers.join_next() => match joined {
                None => break,
                Some(Ok(Ok(()))) => {}
                Some(Ok(Err(e))) => {
                    if failure.is_none() && !matches!(e, Error::Cancelled) {
                        tracing::warn!(error = %e, file = %name, "segment failed, stopping download");
                        failure = Some(e);
                        cancel.cancel();
                    }
                }
                Some(Err(join_error)) => {
                    failure.get_or_insert(Error::Io(std::io::Error::other(join_error)));
                    cancel.cancel();
                }
            },
            _ = ticker.tick(), if resumable => save(&shared).await?,
        }
    }

    if failure.is_some() || cancel.is_cancelled() {
        if resumable {
            save(&shared).await?;
        }
        return Err(failure.unwrap_or(Error::Cancelled));
    }

    // Unknown size: whatever the stream gave is the file.
    let final_path = match request.if_exists {
        IfExists::Overwrite => dest.clone(),
        _ => unique_path(&dest).await,
    };
    tokio::fs::rename(&part, &final_path).await?;
    let _ = tokio::fs::remove_file(&state_file).await;
    tracing::info!(path = %final_path.display(), "download finished");
    Ok(final_path)
}

/// Reuses valid resume data, otherwise plans a fresh download.
async fn prepare(
    engine: &Engine,
    remote: &RemoteFile,
    request: &DownloadRequest,
    part: &Path,
    state_file: &Path,
) -> Result<(Vec<Segment>, bool)> {
    if let (Some(size), true, true) = (remote.size, remote.ranges, request.resume) {
        if let Some(saved) = state::load(state_file).await {
            let same_file = saved.size == Some(size) && validators_match(&saved, remote);
            let part_ok = tokio::fs::metadata(part).await.map(|m| m.len() <= size).unwrap_or(false);
            if same_file && part_ok && !saved.segments.is_empty() {
                tracing::info!(file = %part.display(), "resuming previous download");
                return Ok((saved.segments, true));
            }
        }
    }

    let file = tokio::fs::File::create(part).await?;
    let segments = match (remote.size, remote.ranges) {
        (Some(size), true) => {
            if request.preallocate {
                file.set_len(size).await?;
            }
            state::plan(size, request.connections, engine.min_segment)
        }
        (Some(0), false) => Vec::new(),
        (size, _) => vec![Segment { start: 0, end: size.map(|s| s - 1), done: 0 }],
    };
    let _ = tokio::fs::remove_file(state_file).await;
    Ok((segments, false))
}

fn validators_match(saved: &ResumeState, remote: &RemoteFile) -> bool {
    match (&saved.etag, &remote.etag) {
        (Some(a), Some(b)) => return a == b,
        (Some(_), None) | (None, Some(_)) => return false,
        (None, None) => {}
    }
    match (&saved.last_modified, &remote.last_modified) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    }
}

/// Limits parallel connections and lowers the limit when the server says
/// "too many requests", instead of failing the download.
struct ConnectionLimiter {
    permits: Semaphore,
    limit: AtomicUsize,
    max: usize,
}

impl ConnectionLimiter {
    fn new(limit: usize) -> Self {
        let limit = limit.max(1);
        ConnectionLimiter { permits: Semaphore::new(limit), limit: AtomicUsize::new(limit), max: limit }
    }

    /// Gives one connection back after a success, up to the starting limit,
    /// so a temporary throttle does not slow the host down forever.
    fn grow(&self) {
        if self.limit.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| (n < self.max).then(|| n + 1)).is_ok() {
            self.permits.add_permits(1);
        }
    }

    /// Lowers the limit by one if above one. Returns whether it did.
    fn reduce(&self) -> bool {
        self.limit.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| (n > 1).then(|| n - 1)).is_ok()
    }
}

fn is_throttled(error: &Error) -> bool {
    matches!(error, Error::Status(429 | 503))
}

#[allow(clippy::too_many_arguments)]
async fn worker(
    engine: Engine,
    url: Url,
    part: PathBuf,
    shared: Arc<Shared>,
    idx: usize,
    ranges: bool,
    limiter: Arc<ConnectionLimiter>,
    headers: Arc<Vec<(String, String)>>,
    cancel: CancellationToken,
) -> Result<()> {
    let host = engine.host_limiter(&url);
    let mut attempt = 0;
    loop {
        let permit = tokio::select! {
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            permit = limiter.permits.acquire() => permit.map_err(|_| Error::Cancelled)?,
        };
        let host_permit = tokio::select! {
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            permit = host.permits.acquire() => permit.map_err(|_| Error::Cancelled)?,
        };
        let result = fetch_segment(&engine, &headers, &url, &part, &shared, idx, ranges, &cancel).await;
        if matches!(&result, Err(e) if is_throttled(e)) && host.reduce() {
            // Give this host slot up for good and wait for another connection
            // to the host to finish before trying this segment again.
            tracing::debug!(segment = idx, host = url.host_str(), "server throttles connections, lowering the limit");
            host_permit.forget();
            drop(permit);
            continue;
        }
        drop(host_permit);
        drop(permit);
        match result {
            Ok(()) => {
                host.grow();
                return Ok(());
            }
            // Throttling deserves more patience than a broken connection.
            Err(e) if e.is_transient() && attempt < engine.retries * if is_throttled(&e) { 3 } else { 1 } => {
                attempt += 1;
                let delay = Duration::from_millis((500 * (1u64 << attempt.min(6))).min(30_000));
                tracing::debug!(segment = idx, attempt, error = %e, "retrying segment");
                tokio::select! {
                    _ = cancel.cancelled() => return Err(Error::Cancelled),
                    _ = tokio::time::sleep(delay) => {}
                }
            }
            Err(e) => return Err(e),
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn fetch_segment(
    engine: &Engine,
    headers: &[(String, String)],
    url: &Url,
    part: &Path,
    shared: &Arc<Shared>,
    idx: usize,
    ranges: bool,
    cancel: &CancellationToken,
) -> Result<()> {
    let mut seg = shared.segment(idx);
    if seg.is_complete() {
        return Ok(());
    }
    let mut request = engine.client.get(url.clone());
    for (name, value) in headers {
        request = request.header(name.as_str(), value.as_str());
    }
    if ranges {
        let offset = seg.start + seg.done;
        let range = match seg.end {
            Some(end) => format!("bytes={offset}-{end}"),
            None => format!("bytes={offset}-"),
        };
        request = request.header(RANGE, range);
    } else if seg.done > 0 {
        // Without range support a retry has to start over.
        shared.lock().segments[idx].done = 0;
        seg.done = 0;
    }

    let response = tokio::select! {
        _ = cancel.cancelled() => return Err(Error::Cancelled),
        response = request.send() => response?,
    };
    let status = response.status();
    if ranges && status != StatusCode::PARTIAL_CONTENT {
        return Err(if status.is_success() { Error::RangeNotHonored } else { Error::Status(status.as_u16()) });
    }
    if !status.is_success() {
        return Err(Error::Status(status.as_u16()));
    }

    let mut file = tokio::fs::OpenOptions::new().write(true).open(part).await?;
    file.seek(SeekFrom::Start(seg.start + seg.done)).await?;
    let _active = ActiveGuard::new(shared);
    let mut stream = response.bytes_stream();

    loop {
        let next = tokio::select! {
            _ = cancel.cancelled() => {
                file.flush().await?;
                return Err(Error::Cancelled);
            }
            next = stream.next() => next,
        };
        let Some(chunk) = next else { break };
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(e) => {
                file.flush().await?;
                return Err(e.into());
            }
        };
        let remaining = shared.segment(idx).len().map(|len| len - shared.segment(idx).done);
        let data = match remaining {
            Some(remaining) if chunk.len() as u64 > remaining => &chunk[..remaining as usize],
            _ => &chunk[..],
        };
        file.write_all(data).await?;
        tokio::select! {
            _ = cancel.cancelled() => {
                file.flush().await?;
                shared.lock().segments[idx].done += data.len() as u64;
                return Err(Error::Cancelled);
            }
            _ = async { engine.rate.acquire(data.len()).await; shared.rate.acquire(data.len()).await } => {}
        }
        let complete = {
            let mut inner = shared.lock();
            inner.segments[idx].done += data.len() as u64;
            inner.segments[idx].is_complete()
        };
        if complete {
            break;
        }
    }
    file.flush().await?;
    file.sync_data().await?;

    let seg = shared.segment(idx);
    if seg.end.is_some() && !seg.is_complete() {
        return Err(Error::Incomplete);
    }
    Ok(())
}

/// `file.zip` → `file (1).zip` when the name is already taken.
async fn unique_path(dest: &Path) -> PathBuf {
    if tokio::fs::metadata(dest).await.is_err() {
        return dest.to_path_buf();
    }
    let stem = dest.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = dest.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for n in 1.. {
        let candidate = dest.with_file_name(format!("{stem} ({n}){ext}"));
        if tokio::fs::metadata(&candidate).await.is_err() {
            return candidate;
        }
    }
    unreachable!("an unused file name always exists")
}
