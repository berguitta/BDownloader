//! User settings, stored as TOML in the user's configuration folder
//! (`%APPDATA%\BDownloader\config\settings.toml` on Windows).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use directories::{ProjectDirs, UserDirs};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    /// Follow the Windows light/dark setting.
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeMode,
    pub language: String,
    pub download_dir: PathBuf,
    /// Downloads running at the same time.
    pub max_parallel: usize,
    /// Connections per download when the server allows several.
    pub connections_per_download: usize,
    /// Module id → enabled, only for modules the user changed.
    pub modules: BTreeMap<String, bool>,
    /// Global speed limit in KiB/s, 0 for none (speed-limit module).
    pub speed_limit_kib: u64,
    /// Start queued downloads when the app opens.
    pub start_on_launch: bool,
    /// Send grabbed links straight to the queue instead of the link grabber
    /// (never for Click'n'Load).
    pub auto_start_grabbed: bool,
    /// Closing the window keeps BDownloader running in the tray (tray module).
    pub close_to_tray: bool,
    /// Minimizing the window hides it to the tray (tray module).
    pub minimize_to_tray: bool,
    /// Passwords tried on archives, in order (extraction module).
    pub archive_passwords: Vec<String>,
    /// Delete archives after a successful extraction.
    pub delete_archives: bool,
    pub clicknload_port: u16,
    /// Other options, by id (see the settings schema of the app). Values are
    /// stored as text so new options never break old settings files.
    pub options: BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeMode::System,
            language: "en".to_owned(),
            download_dir: default_download_dir(),
            max_parallel: 4,
            connections_per_download: 8,
            modules: BTreeMap::new(),
            speed_limit_kib: 0,
            start_on_launch: true,
            auto_start_grabbed: false,
            close_to_tray: true,
            minimize_to_tray: false,
            archive_passwords: Vec::new(),
            delete_archives: true,
            clicknload_port: crate::clicknload::DEFAULT_PORT,
            options: BTreeMap::new(),
        }
    }
}

fn default_download_dir() -> PathBuf {
    UserDirs::new()
        .and_then(|d| d.download_dir().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("BDownloader")
}

impl Settings {
    pub fn option(&self, id: &str) -> Option<&str> {
        self.options.get(id).map(String::as_str)
    }

    pub fn flag(&self, id: &str, default: bool) -> bool {
        self.option(id).map(|v| v == "true").unwrap_or(default)
    }

    pub fn number(&self, id: &str, default: i64) -> i64 {
        self.option(id).and_then(|v| v.parse().ok()).unwrap_or(default)
    }

    pub fn text<'a>(&'a self, id: &str, default: &'a str) -> &'a str {
        self.option(id).unwrap_or(default)
    }

    pub fn set_option(&mut self, id: &str, value: impl ToString) {
        self.options.insert(id.to_owned(), value.to_string());
    }

    /// Next to the settings: the saved download queue.
    pub fn queue_path() -> Option<PathBuf> {
        Self::default_path().map(|p| p.with_file_name("queue.json"))
    }

    /// `BD_SETTINGS` overrides the location (handy for development and
    /// portable installs).
    pub fn default_path() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("BD_SETTINGS").filter(|p| !p.is_empty()) {
            return Some(PathBuf::from(path));
        }
        ProjectDirs::from("", "", "BDownloader").map(|d| d.config_dir().join("settings.toml"))
    }

    /// Loads settings, falling back to defaults when the file is missing or
    /// invalid. Values out of range are clamped.
    pub fn load_from(path: &Path) -> Self {
        let mut settings = match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!(error = %e, path = %path.display(), "invalid settings file, using defaults");
                Settings::default()
            }),
            Err(_) => Settings::default(),
        };
        settings.max_parallel = settings.max_parallel.clamp(1, 50);
        settings.connections_per_download = settings.connections_per_download.clamp(1, 32);
        settings
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let dir = std::env::temp_dir().join(format!("bd-settings-{}", std::process::id()));
        let path = dir.join("settings.toml");
        let mut settings = Settings { theme: ThemeMode::Dark, max_parallel: 7, ..Settings::default() };
        settings.modules.insert("resume".into(), false);
        settings.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), settings);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_or_partial_files_use_defaults() {
        let missing = Settings::load_from(Path::new("Z:/does/not/exist.toml"));
        assert_eq!(missing.max_parallel, Settings::default().max_parallel);

        let partial: Settings = toml::from_str("theme = \"light\"\nmax_parallel = 999").unwrap();
        assert_eq!(partial.theme, ThemeMode::Light);
        assert_eq!(partial.connections_per_download, 8);
    }
}
