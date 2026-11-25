//! BDownloader core.
//!
//! - [`modules`]: every feature is a module the user can turn on or off.
//! - [`settings`]: user preferences persisted as TOML.
//! - [`manager`]: the download queue (HTTP and video jobs), saved between sessions.
//! - [`grabber`]: the link grabber (checking, packages, mirrors, selection).
//! - [`hosters`] and [`accounts`]: hoster links through debrid or premium APIs.
//! - [`video`] and [`tools`]: video sites through yt-dlp and ffmpeg.
//! - [`extract`]: automatic extraction of finished packages.
//! - [`clicknload`]: the local Click'n'Load listener.

pub mod accounts;
pub mod checksum;
pub mod clicknload;
pub mod decrypt;
pub mod extract;
pub mod free;
pub mod grabber;
pub mod hosters;
pub mod links;
pub mod manager;
pub mod modules;
pub mod packages;
pub mod reconnect;
pub mod settings;
pub mod task;
pub mod tools;
pub mod torrent;
pub mod video;

pub use manager::{DownloadManager, JobId, JobKind, JobSnapshot, JobSpec, JobState};
pub use modules::{Category, ModuleInfo, ModuleRegistry, ModuleStatus};
pub use settings::{Settings, ThemeMode};
