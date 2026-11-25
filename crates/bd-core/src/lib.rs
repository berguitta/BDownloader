//! BDownloader core.
//!
//! - [`modules`]: every feature is a module the user can turn on or off.
//! - [`settings`]: user preferences persisted as TOML.
//! - [`manager`]: the download queue built on top of `bd-net`.
//! - [`links`]: extraction of URLs from arbitrary pasted text.

pub mod links;
pub mod manager;
pub mod modules;
pub mod settings;

pub use manager::{DownloadManager, JobId, JobSnapshot, JobState};
pub use modules::{Category, ModuleInfo, ModuleRegistry, ModuleStatus};
pub use settings::{Settings, ThemeMode};
