//! Segmented HTTP download engine.
//!
//! Features:
//! - splits a file into several byte ranges downloaded in parallel when the
//!   server supports `Range` requests, and falls back to a single stream
//!   otherwise;
//! - preallocates the file and writes each segment at its own offset;
//! - persists progress next to the partial file so an interrupted download
//!   (cancel, crash, reboot) resumes at the exact byte where it stopped;
//! - retries failed segments with exponential backoff.

mod engine;
mod error;
mod name;
mod probe;
mod rate;
mod state;

pub use engine::{DownloadControl, DownloadRequest, DownloadTask, Engine, Progress, SegmentProgress};
pub use error::{Error, Result};
pub use name::{file_name_from_url, sanitize_file_name};
pub use probe::RemoteFile;
pub use rate::RateLimiter;
