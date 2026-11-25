//! What the queue needs from a running job, whatever downloads it (the HTTP
//! engine, yt-dlp, …): progress, cancellation and a final result.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bd_net::{DownloadControl, Progress};
use futures_util::future::BoxFuture;
use url::Url;

/// A running download seen from the queue.
pub trait TaskControl: Send + Sync {
    fn progress(&self) -> Progress;
    fn cancel(&self);
}

impl TaskControl for DownloadControl {
    fn progress(&self) -> Progress {
        DownloadControl::progress(self)
    }

    fn cancel(&self) {
        DownloadControl::cancel(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    Cancelled,
    Failed(String),
}

impl From<bd_net::Error> for RunError {
    fn from(e: bd_net::Error) -> Self {
        match e {
            bd_net::Error::Cancelled => RunError::Cancelled,
            other => RunError::Failed(other.to_string()),
        }
    }
}

pub type RunResult = Result<PathBuf, RunError>;

/// Shared between a job and its runner. The runner may start the real
/// transfer later (after resolving a hoster link), so the control arrives
/// asynchronously; a cancel issued before that is remembered.
#[derive(Default)]
pub struct ControlSlot {
    control: Mutex<Option<Arc<dyn TaskControl>>>,
    status: Mutex<Option<String>>,
    cancelled: AtomicBool,
}

impl ControlSlot {
    pub fn set(&self, control: Arc<dyn TaskControl>) {
        if self.is_cancelled() {
            control.cancel();
        }
        *self.control.lock().unwrap_or_else(|e| e.into_inner()) = Some(control);
    }

    pub fn progress(&self) -> Option<Progress> {
        self.control.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|c| c.progress())
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(control) = self.control.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            control.cancel();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Short text shown while there is no progress yet ("Resolving link…").
    pub fn set_status(&self, status: Option<&str>) {
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = status.map(str::to_owned);
    }

    pub fn status(&self) -> Option<String> {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// Direct link obtained from a hoster or debrid service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub url: Url,
    pub headers: Vec<(String, String)>,
    pub file_name: Option<String>,
    /// Human-readable route, e.g. "Real-Debrid" or "Premium".
    pub via: String,
}

impl Resolved {
    pub fn direct(url: Url) -> Self {
        Resolved { url, headers: Vec::new(), file_name: None, via: "Direct".to_owned() }
    }
}

/// Turns a hoster link into a direct one (debrid, premium API…).
pub trait LinkResolver: Send + Sync {
    fn resolve<'a>(&'a self, url: &'a Url) -> BoxFuture<'a, Result<Resolved, String>>;
}

/// Resolver that keeps every link as it is.
pub struct NoResolver;

impl LinkResolver for NoResolver {
    fn resolve<'a>(&'a self, url: &'a Url) -> BoxFuture<'a, Result<Resolved, String>> {
        Box::pin(async move { Ok(Resolved::direct(url.clone())) })
    }
}
