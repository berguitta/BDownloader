//! Application state shared by the UI callbacks and the refresh timer.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bd_core::accounts::{service, AccountStore, ServiceKind};
use bd_core::clicknload::Received;
use bd_core::extract::{ExtractOptions, ExtractReport};
use bd_core::grabber::{Grabber, LinkStatus};
use bd_core::hosters::{AccountCheck, CaptchaMode, HosterResolver, ResolverOptions};
use bd_core::manager::{save_records, ManagerConfig, ManagerEvent};
use bd_core::task::ControlSlot;
use bd_core::tools::{self, Tool};
use bd_core::video::{is_video_url, VideoBackend, VideoFormat, VideoOptions};
use bd_core::{DownloadManager, JobId, JobKind, JobSnapshot, JobSpec, JobState, ModuleRegistry, Settings};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::i18n::t;
use crate::tray::Tray;
use crate::{
    format, platform, AccountRow, AppWindow, Connection, DownloadsState, GrabRow, GrabberState, JobRow, KeyValue,
};
use crate::{AccountsState, ExtractRow, ExtractionState, SettingsState};

const TOAST_DURATION: Duration = Duration::from_millis(2600);
pub const HISTORY_SECONDS: usize = 60;

/// Results of background work, handled on the UI thread.
pub enum UiMsg {
    AccountChecked {
        service: String,
        username: String,
        result: Result<AccountCheck, String>,
        save_secret: Option<String>,
    },
    ToolInstalled {
        tool: Tool,
        result: Result<PathBuf, String>,
    },
    ClickNLoad(Received),
    ClickNLoadStopped(String),
    Extracted {
        package: String,
        report: ExtractReport,
    },
    Favicon {
        host: String,
        image: Option<(Vec<u8>, u32, u32)>,
    },
}

/// One archive on the Extraction screen.
pub struct ExtractJob {
    pub archive: PathBuf,
    pub package: String,
    pub dir: PathBuf,
    /// None while running.
    pub outcome: Option<Result<usize, String>>,
}

pub struct App {
    pub runtime: Handle,
    pub manager: DownloadManager,
    pub grabber: Grabber,
    pub resolver: Arc<HosterResolver>,
    pub accounts: AccountStore,
    pub account_status: Vec<(String, String, Result<AccountCheck, String>)>,
    pub settings: Settings,
    pub settings_path: Option<PathBuf>,
    pub queue_path: Option<PathBuf>,
    pub modules: ModuleRegistry,
    pub rows: std::rc::Rc<VecModel<JobRow>>,
    pub row_ids: Vec<JobId>,
    pub jobs: Vec<JobSnapshot>,
    pub selected: Option<JobId>,
    pub history: VecDeque<f64>,
    pub last_history: Instant,
    pub toast: Option<(String, Instant)>,
    pub grab_rows: std::rc::Rc<VecModel<GrabRow>>,
    pub grab_revision: u64,
    pub closed_packages: HashSet<String>,
    pub saved_revision: u64,
    pub last_save: Instant,
    pub events: Option<Receiver<ManagerEvent>>,
    pub messages: Receiver<UiMsg>,
    pub sender: Sender<UiMsg>,
    pub tray: Option<Tray>,
    pub clipboard_seq: u32,
    pub ignore_clipboard: Option<String>,
    pub clicknload: Option<CancellationToken>,
    pub clicknload_status: String,
    pub tool_slot: Option<(Tool, Arc<ControlSlot>)>,
    pub quitting: bool,
    pub favicons: HashMap<String, Option<slint::Image>>,
    pub extract_jobs: Vec<ExtractJob>,
    pub extract_revision: u64,
    /// Job and action waiting for the input dialog (rename, file limit).
    pub input_action: Option<(JobId, String)>,
    pub was_busy: bool,
}

impl App {
    pub fn save_settings(&mut self) {
        self.settings.modules = self.modules.states();
        if let Some(path) = &self.settings_path {
            if let Err(e) = self.settings.save_to(path) {
                tracing::warn!(error = %e, "cannot save settings");
            }
        }
    }

    pub fn save_queue(&mut self) {
        let Some(path) = &self.queue_path else { return };
        if let Err(e) = save_records(path, &self.manager.records()) {
            tracing::warn!(error = %e, "cannot save the queue");
        }
        self.saved_revision = self.manager.revision();
        self.last_save = Instant::now();
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), Instant::now()));
    }

    pub fn manager_config(settings: &Settings, modules: &ModuleRegistry) -> ManagerConfig {
        ManagerConfig {
            dest_dir: settings.download_dir.clone(),
            max_parallel: settings.max_parallel,
            connections: settings.connections_per_download,
            multi_connection: modules.is_enabled("multi-connection"),
            resume: modules.is_enabled("resume"),
            if_exists: match settings.text("if-exists", "rename") {
                "skip" => bd_net::IfExists::Skip,
                "overwrite" => bd_net::IfExists::Overwrite,
                _ => bd_net::IfExists::Rename,
            },
            preallocate: settings.flag("preallocate", true),
        }
    }

    /// Pushes settings and module states to the engine, queue and resolver.
    pub fn apply_config(&self) {
        let config = Self::manager_config(&self.settings, &self.modules);
        self.manager.update_config(|c| *c = config);
        let limit = if self.modules.is_enabled("speed-limit") { self.settings.speed_limit_kib * 1024 } else { 0 };
        self.manager.set_speed_limit(limit);
        let enabled: Vec<String> =
            self.modules.list().into_iter().filter(|(_, on)| *on).map(|(m, _)| m.id.to_owned()).collect();
        self.resolver.set_enabled_modules(enabled);
        self.resolver.set_credentials(self.accounts.credentials());
        self.resolver.set_options(ResolverOptions {
            prefer_debrid: self.settings.flag("prefer-debrid", false),
            free_fallback: self.settings.flag("free-fallback", true),
            captcha: match self.settings.text("captcha-mode", "browser") {
                "notify" => CaptchaMode::Notify,
                "skip" => CaptchaMode::Skip,
                _ => CaptchaMode::Browser,
            },
        });
        self.grabber.set_page_scan(self.modules.is_enabled("page-scan"));
        let backend = self.video_backend();
        self.manager.set_video_backend(backend.clone());
        self.grabber.set_video_backend(backend);
    }

    pub fn video_backend(&self) -> Option<VideoBackend> {
        if !self.modules.is_enabled("video") {
            return None;
        }
        let options = VideoOptions {
            container: self.settings.text("video-container", "mp4").to_owned(),
            subtitles: self
                .settings
                .flag("subtitles", false)
                .then(|| self.settings.text("subtitle-langs", "en, fr").to_owned()),
            embed_metadata: self.settings.flag("embed-metadata", true),
        };
        Some(VideoBackend { yt_dlp: tools::find(Tool::YtDlp)?, ffmpeg: tools::find(Tool::Ffmpeg), options })
    }

    /// Entry point for every new link: to the grabber, or straight to the
    /// queue when the user chose so (never for Click'n'Load).
    pub fn add_links(&mut self, links: Vec<Url>, source: &str, force_grabber: bool) -> usize {
        if links.is_empty() {
            return 0;
        }
        if self.settings.auto_start_grabbed && !force_grabber {
            let count = links.len();
            for url in links {
                let kind = if is_video_url(&url) {
                    JobKind::Video { format: self.default_video_format() }
                } else {
                    JobKind::Http
                };
                self.manager.add_spec(JobSpec { kind, ..JobSpec::http(url, self.settings.download_dir.clone()) });
            }
            count
        } else {
            self.grabber.add(links, source)
        }
    }

    fn default_video_format(&self) -> VideoFormat {
        match self.settings.text("video-quality", "best") {
            "2160" => VideoFormat::MaxHeight { height: 2160 },
            "1080" => VideoFormat::MaxHeight { height: 1080 },
            "720" => VideoFormat::MaxHeight { height: 720 },
            "audio" => VideoFormat::AudioBest,
            "mp3" => VideoFormat::AudioMp3,
            _ => VideoFormat::Best,
        }
    }

    pub fn notify(&self, level_error: bool, title: &str, body: &str) {
        if !self.modules.is_enabled("notifications") {
            return;
        }
        let wanted = match self.settings.text("notifications", "finished-errors") {
            "none" => false,
            "errors" => level_error,
            _ => true,
        };
        if wanted {
            platform::notify(title, body);
        }
    }

    /// Favicon of a host, fetched once in the background.
    pub fn favicon(&mut self, host: &str) -> Option<slint::Image> {
        if let Some(icon) = self.favicons.get(host) {
            return icon.clone();
        }
        self.favicons.insert(host.to_owned(), None);
        let (host, sender, client) = (host.to_owned(), self.sender.clone(), self.manager.engine().client().clone());
        self.runtime.spawn(async move {
            let image = crate::favicon::fetch(&client, &host).await;
            let _ = sender.send(UiMsg::Favicon { host, image });
        });
        None
    }

    pub fn rebuild_tray(&mut self) {
        if !self.modules.is_enabled("tray") {
            self.tray = None;
            return;
        }
        self.tray = None;
        match Tray::new(self.modules.is_enabled("clipboard")) {
            Ok(tray) => self.tray = Some(tray),
            Err(e) => tracing::warn!(error = %e, "cannot create the tray icon"),
        }
    }

    pub fn restart_clicknload(&mut self) {
        if let Some(cancel) = self.clicknload.take() {
            cancel.cancel();
        }
        self.sync_clicknload();
    }

    /// Extracts the archives of a finished package in the background.
    pub fn start_extraction(&mut self, package: String, dir: PathBuf, files: Vec<PathBuf>, password: Option<String>) {
        let firsts: Vec<PathBuf> = files
            .iter()
            .filter(|f| f.file_name().and_then(|n| n.to_str()).is_some_and(bd_core::packages::is_first_volume))
            .cloned()
            .collect();
        if firsts.is_empty() {
            return;
        }
        for archive in &firsts {
            self.extract_jobs.retain(|j| &j.archive != archive);
            self.extract_jobs.push(ExtractJob {
                archive: archive.clone(),
                package: package.clone(),
                dir: dir.clone(),
                outcome: None,
            });
        }
        self.extract_revision += 1;
        let options = ExtractOptions {
            passwords: if self.settings.flag("try-passwords", true) {
                self.settings.archive_passwords.clone()
            } else {
                Vec::new()
            },
            delete_archives: self.settings.delete_archives,
        };
        let target = if self.settings.text("extract-into", "package") == "subfolder" {
            let first =
                firsts[0].file_name().and_then(|n| n.to_str()).map(bd_core::packages::package_name).unwrap_or_default();
            dir.join(first)
        } else {
            dir
        };
        let sender = self.sender.clone();
        self.runtime.spawn_blocking(move || {
            let report = bd_core::extract::extract_package(&files, &target, password.as_deref(), &options);
            let _ = sender.send(UiMsg::Extracted { package, report });
        });
    }

    pub fn refresh_extraction(&mut self, ui: &AppWindow, force: bool) {
        if !force && self.extract_revision == 0 {
            return;
        }
        let rows: Vec<ExtractRow> = self
            .extract_jobs
            .iter()
            .rev()
            .map(|j| {
                let name = j.archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let format = j.archive.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default();
                let (status, kind) = match &j.outcome {
                    None => ("Extracting…".to_owned(), 1),
                    Some(Ok(n)) => (format!("Done · {n} files"), 2),
                    Some(Err(e)) => (e.clone(), 3),
                };
                ExtractRow {
                    archive: name.into(),
                    package: j.package.as_str().into(),
                    format: format.into(),
                    status: status.into(),
                    kind,
                }
            })
            .collect();
        let running = self.extract_jobs.iter().filter(|j| j.outcome.is_none()).count();
        ui.set_extract_badge(if running > 0 { running.to_string() } else { String::new() }.into());
        ui.global::<ExtractionState>().set_jobs(ModelRc::new(VecModel::from(rows)));
        self.extract_revision = 0;
    }

    /// "When the queue is finished" action, once the last download ends.
    fn check_queue_done(&mut self, _ui: &AppWindow) {
        let busy = self.jobs.iter().any(|j| matches!(j.state, JobState::Downloading | JobState::Queued));
        let finished_something = self.jobs.iter().any(|j| j.state == JobState::Finished);
        if self.was_busy && !busy && finished_something {
            match self.settings.text("when-done", "nothing") {
                "close" => {
                    self.quitting = true;
                    let _ = slint::quit_event_loop();
                }
                "sleep" => platform::sleep_computer(),
                "shutdown" => platform::shutdown_computer(),
                _ => {}
            }
        }
        self.was_busy = busy;
    }

    pub fn job_at(&self, row: i32) -> Option<&JobSnapshot> {
        let id = *self.row_ids.get(usize::try_from(row).ok()?)?;
        self.jobs.iter().find(|j| j.id == id)
    }

    /// Called by the timer: pulls fresh state and pushes it to the UI.
    pub fn refresh(&mut self, ui: &AppWindow) {
        self.handle_messages(ui);
        self.handle_events();
        self.refresh_downloads(ui);
        self.refresh_grabber(ui, false);
        self.refresh_extraction(ui, false);
        self.refresh_tool_status(ui);
        self.check_queue_done(ui);

        if self.manager.revision() != self.saved_revision && self.last_save.elapsed() > Duration::from_secs(2) {
            self.save_queue();
        }
        match &self.toast {
            Some((text, at)) if at.elapsed() < TOAST_DURATION => {
                ui.set_toast_text(text.as_str().into());
                ui.set_toast_visible(true);
            }
            _ => ui.set_toast_visible(false),
        }
    }

    fn refresh_downloads(&mut self, ui: &AppWindow) {
        self.jobs = self.manager.snapshot();
        let state = ui.global::<DownloadsState>();
        let count = |f: &dyn Fn(&JobState) -> bool| self.jobs.iter().filter(|j| f(&j.state)).count();
        let active = count(&|s| *s == JobState::Downloading);
        let queued = count(&|s| *s == JobState::Queued);
        // `+ 0.0` turns the -0.0 of an empty sum into 0.0.
        let speed: f64 = self.jobs.iter().map(|j| j.speed).sum::<f64>() + 0.0;
        state.set_total_jobs(self.jobs.len() as i32);
        state.set_count_all(format::thousands(self.jobs.len()).into());
        state.set_count_active(format::thousands(active).into());
        state.set_count_queued(format::thousands(queued).into());
        state.set_count_paused(format::thousands(count(&|s| *s == JobState::Paused)).into());
        state.set_count_finished(format::thousands(count(&|s| *s == JobState::Finished)).into());
        state.set_count_failed(format::thousands(count(&|s| matches!(s, JobState::Failed(_)))).into());
        state.set_speed_now(format!("{:.1} MB/s", speed / 1_048_576.0).into());
        ui.set_nav_badge(if active > 0 { active.to_string() } else { String::new() }.into());
        ui.set_status_down(if speed >= 1.0 { format::speed(speed) } else { "0 B/s".to_owned() }.into());
        ui.set_status_active(active.to_string().into());
        ui.set_status_queued(queued.to_string().into());
        ui.set_status_folder(self.settings.download_dir.display().to_string().into());
        let limit = self.settings.speed_limit_kib;
        let limited = limit > 0 && self.modules.is_enabled("speed-limit");
        ui.set_speed_limited(limited);
        ui.set_speed_limit_label(if limited { format::speed(limit as f64 * 1024.0) } else { t("Unlimited") }.into());
        ui.set_window_title(if self.settings.flag("speed-in-title", false) && speed >= 1.0 {
            format!("{} · BDownloader", format::speed(speed)).into()
        } else {
            "BDownloader".into()
        });
        if let Some(tray) = &self.tray {
            let tip = if active > 0 {
                format!("BDownloader · {active} ↓ · {}", format::speed(speed))
            } else {
                "BDownloader".to_owned()
            };
            tray.set_tooltip(&tip);
        }

        if self.last_history.elapsed() >= Duration::from_secs(1) {
            self.last_history = Instant::now();
            self.history.pop_front();
            self.history.push_back(speed);
        }
        let (line, area) = sparkline(&self.history);
        state.set_spark_line(line.into());
        state.set_spark_area(area.into());

        let filter = state.get_filter();
        let query = state.get_query().to_lowercase();
        let query = query.trim();
        let visible: Vec<&JobSnapshot> = self
            .jobs
            .iter()
            .filter(|j| match filter {
                1 => j.state == JobState::Downloading,
                2 => j.state == JobState::Queued,
                3 => j.state == JobState::Paused,
                4 => j.state == JobState::Finished,
                5 => matches!(j.state, JobState::Failed(_)),
                _ => true,
            })
            .filter(|j| {
                query.is_empty()
                    || j.name.to_lowercase().contains(query)
                    || j.url.host_str().is_some_and(|h| h.to_lowercase().contains(query))
                    || j.spec.package.as_deref().is_some_and(|p| p.to_lowercase().contains(query))
            })
            .collect();
        let ids: Vec<JobId> = visible.iter().map(|j| j.id).collect();
        let visible: Vec<JobSnapshot> = visible.into_iter().cloned().collect();
        let rows: Vec<JobRow> = visible.iter().map(|j| self.job_row(j)).collect();
        if ids == self.row_ids {
            for (i, row) in rows.into_iter().enumerate() {
                if self.rows.row_data(i).as_ref() != Some(&row) {
                    self.rows.set_row_data(i, row);
                }
            }
        } else {
            self.row_ids = ids;
            self.rows.set_vec(rows);
        }
        self.refresh_detail(ui);
    }

    fn refresh_detail(&self, ui: &AppWindow) {
        let state = ui.global::<DownloadsState>();
        let Some(job) = self.selected.and_then(|id| self.jobs.iter().find(|j| j.id == id)) else {
            state.set_has_selection(false);
            return;
        };
        state.set_has_selection(true);
        state.set_detail_title(job.name.as_str().into());
        let kv = |k: &str, v: String| KeyValue { key: k.into(), value: v.into() };
        let progress = match job.total {
            Some(total) if total > 0 => format!(
                "{} / {} · {:.1}%",
                format::bytes(job.downloaded),
                format::bytes(total),
                job.downloaded as f64 * 100.0 / total as f64
            ),
            _ => format::bytes(job.downloaded),
        };
        let saved = job.path.clone().unwrap_or_else(|| job.dest_dir.join(&job.name));
        let mut info = vec![
            kv("Link", job.url.to_string()),
            kv("Progress", progress),
            kv("Saved as", saved.display().to_string()),
        ];
        if let Some(package) = &job.spec.package {
            info.push(kv("Package", package.clone()));
        }
        if let JobKind::Video { format } = &job.spec.kind {
            info.push(kv("Format", format.label()));
        }
        if let JobState::Failed(error) = &job.state {
            info.push(kv("Error", error.clone()));
        }
        state.set_detail_info(ModelRc::new(VecModel::from(info)));
        state.set_detail_right_title(
            match (job.segments.len(), &job.spec.kind) {
                (_, JobKind::Video { .. }) => "Streams".to_owned(),
                (0, _) => "Waiting for the server".to_owned(),
                (1, _) => "1 connection".to_owned(),
                (n, _) => format!("{n} connections ({} active)", job.connections),
            }
            .into(),
        );
        let video = matches!(job.spec.kind, JobKind::Video { .. });
        let connections: Vec<Connection> = job
            .segments
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let progress = s.len.map(|len| if len == 0 { 1.0 } else { s.done as f64 / len as f64 }).unwrap_or(0.0);
                let label = if video { format!("Stream {}", i + 1) } else { format!("Conn {}", i + 1) };
                Connection {
                    label: label.into(),
                    progress: progress as f32,
                    percent: if s.len.is_some() {
                        format!("{}%", (progress * 100.0).floor())
                    } else {
                        format::bytes(s.done)
                    }
                    .into(),
                }
            })
            .collect();
        state.set_detail_connections(ModelRc::new(VecModel::from(connections)));
    }

    pub fn refresh_grabber(&mut self, ui: &AppWindow, force: bool) {
        let revision = self.grabber.revision();
        if revision == self.grab_revision && !force {
            return;
        }
        self.grab_revision = revision;
        let links = self.grabber.snapshot();
        for link in &links {
            self.favicon(&favicon_host(&link.url));
        }
        let state = ui.global::<GrabberState>();
        let mut packages: Vec<String> = Vec::new();
        for link in &links {
            if !packages.contains(&link.package) {
                packages.push(link.package.clone());
            }
        }
        let mut rows = Vec::new();
        let mut selected = 0;
        let mut selected_size = 0u64;
        for package in &packages {
            let members: Vec<_> = links.iter().filter(|l| &l.package == package).collect();
            let online =
                members.iter().filter(|l| matches!(l.status, LinkStatus::Online | LinkStatus::Unchecked)).count();
            let size: u64 = members.iter().filter_map(|l| l.size).sum();
            let open = !self.closed_packages.contains(package);
            let all_checked = members.iter().all(|l| l.selected || matches!(l.status, LinkStatus::Offline(_)));
            let name = if package == "Videos" { t("Videos") } else { package.clone() };
            rows.push(GrabRow {
                is_package: true,
                id: -1,
                package: package.as_str().into(),
                name: name.into(),
                host: SharedString::new(),
                has_icon: false,
                icon: Default::default(),
                size: if size > 0 { format::bytes(size) } else { String::new() }.into(),
                status: format!("{} · {} ✓", members.len(), online).into(),
                status_kind: 1,
                checked: all_checked && members.iter().any(|l| l.selected),
                mirrors: SharedString::new(),
                is_video: false,
                formats: ModelRc::default(),
                format_index: 0,
                open,
                password: self.grabber.package_password(package).unwrap_or_default().into(),
                source: members.first().map(|l| l.source.clone()).unwrap_or_default().into(),
                archive: members.iter().any(|l| bd_core::packages::is_archive_part(&l.name)),
                files: members.len() as i32,
                online: online as i32,
            });
            for link in &members {
                if link.selected && !matches!(link.status, LinkStatus::Offline(_) | LinkStatus::Checking) {
                    selected += 1;
                    selected_size += link.size.unwrap_or(0);
                }
                if !open {
                    continue;
                }
                let (status, kind) = match &link.status {
                    LinkStatus::Checking => (t("Checking…"), 0),
                    LinkStatus::Online => (t("Online"), 1),
                    LinkStatus::Unchecked => (t("Checked when downloading"), 2),
                    LinkStatus::Offline(reason) => (reason.clone(), 3),
                };
                let formats: Vec<SharedString> = link
                    .video
                    .as_ref()
                    .map(|v| {
                        v.choices
                            .iter()
                            .map(|c| {
                                let size = c.size.map(|s| format!(" · {}", format::bytes(s))).unwrap_or_default();
                                let missing = if c.available { "" } else { " (ffmpeg)" };
                                SharedString::from(format!("{}{size}{missing}", c.label))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                rows.push(GrabRow {
                    is_package: false,
                    id: link.id as i32,
                    package: package.as_str().into(),
                    name: link.name.as_str().into(),
                    host: link.host().into(),
                    has_icon: self.favicons.get(&favicon_host(&link.url)).is_some_and(|i| i.is_some()),
                    icon: self.favicons.get(&favicon_host(&link.url)).cloned().flatten().unwrap_or_default(),
                    size: link.size.map(format::bytes).unwrap_or_default().into(),
                    status: status.into(),
                    status_kind: kind,
                    checked: link.selected,
                    mirrors: if link.mirrors.is_empty() { String::new() } else { format!("+{}", link.mirrors.len()) }
                        .into(),
                    is_video: link.video.is_some(),
                    formats: ModelRc::new(VecModel::from(formats)),
                    format_index: link.video_choice as i32,
                    open: false,
                    password: SharedString::new(),
                    source: link.source.as_str().into(),
                    archive: false,
                    files: 0,
                    online: 0,
                });
            }
        }
        state.set_total(links.len() as i32);
        state.set_selected_count(selected);
        state.set_selected_size(format::bytes(selected_size).into());
        ui.set_grab_badge(if links.is_empty() { String::new() } else { links.len().to_string() }.into());
        self.grab_rows.set_vec(rows);
    }

    pub fn refresh_accounts(&self, ui: &AppWindow) {
        let state = ui.global::<AccountsState>();
        let rows: Vec<AccountRow> = self
            .accounts
            .list()
            .iter()
            .map(|a| {
                let svc = service(&a.service);
                let (status, kind) =
                    match self.account_status.iter().find(|(s, u, _)| s == &a.service && u == &a.username) {
                        Some((_, _, Ok(check))) => (check.summary.clone(), if check.premium { 1 } else { 2 }),
                        Some((_, _, Err(e))) => (e.clone(), 3),
                        None => (t("Saved"), 0),
                    };
                AccountRow {
                    service: a.service.as_str().into(),
                    service_name: svc.map(|s| s.name).unwrap_or(&a.service).into(),
                    username: a.username.as_str().into(),
                    kind: match svc.map(|s| s.kind) {
                        Some(ServiceKind::Debrid) => t("Debrid"),
                        _ => t("Premium"),
                    }
                    .into(),
                    status: status.into(),
                    status_kind: kind,
                    enabled: a.enabled,
                }
            })
            .collect();
        state.set_accounts(ModelRc::new(VecModel::from(rows)));
    }

    pub fn refresh_modules(&self, ui: &AppWindow) {
        crate::settings_view::refresh(self, ui);
        let g = ui.global::<GrabberState>();
        g.set_clipboard_on(self.modules.is_enabled("clipboard"));
        g.set_clicknload_on(self.modules.is_enabled("click-n-load"));
        g.set_clicknload_status(self.clicknload_status.as_str().into());
        if let Some(tray) = &self.tray {
            tray.set_watch_clipboard(self.modules.is_enabled("clipboard"));
        }
    }

    pub fn refresh_tool_status(&self, ui: &AppWindow) {
        let s = ui.global::<SettingsState>();
        let describe = |tool: Tool| match tools::find(tool) {
            Some(path) => (format!("{} · {}", t("Installed"), path.display()), true),
            None => (t("Not installed"), false),
        };
        let (mut yt, yt_ok) = describe(Tool::YtDlp);
        let (mut ff, ff_ok) = describe(Tool::Ffmpeg);
        if let Some((tool, slot)) = &self.tool_slot {
            let text = match slot.progress() {
                Some(p) => match p.total {
                    Some(total) if total > 0 => {
                        format!("{:.0}% · {}", p.downloaded as f64 * 100.0 / total as f64, format::bytes(total))
                    }
                    _ => format::bytes(p.downloaded),
                },
                None => "…".to_owned(),
            };
            match tool {
                Tool::YtDlp => yt = text,
                Tool::Ffmpeg => ff = text,
            }
        }
        s.set_yt_dlp_status(yt.into());
        s.set_yt_dlp_installed(yt_ok);
        s.set_ffmpeg_status(ff.into());
        s.set_ffmpeg_installed(ff_ok);
        s.set_tool_busy(self.tool_slot.is_some());
    }

    /// Finished packages: notification and extraction.
    fn handle_events(&mut self) {
        let Some(events) = &self.events else { return };
        let drained: Vec<ManagerEvent> = events.try_iter().collect();
        for event in drained {
            match event {
                ManagerEvent::JobFailed { name, error, .. } => {
                    self.notify(true, &t("Download failed"), &format!("{name}\n{error}"));
                }
                ManagerEvent::JobFinished { name, .. } => {
                    if self.settings.text("notifications", "finished-errors") == "all" {
                        self.notify(false, &t("Download finished"), &name);
                    }
                }
                ManagerEvent::PackageFinished { package, dir, files, password } => {
                    let has_archives = files.iter().any(|f| {
                        f.file_name().and_then(|n| n.to_str()).is_some_and(bd_core::packages::is_first_volume)
                    });
                    if has_archives && self.modules.is_enabled("extraction") {
                        self.start_extraction(package, dir, files, password);
                    } else {
                        self.notify(false, &t("Package finished"), &package);
                    }
                }
            }
        }
    }

    fn handle_messages(&mut self, ui: &AppWindow) {
        while let Ok(msg) = self.messages.try_recv() {
            match msg {
                UiMsg::AccountChecked { service, username, result, save_secret } => {
                    let state = ui.global::<AccountsState>();
                    state.set_form_busy(false);
                    match (&result, save_secret) {
                        (Ok(check), Some(secret)) => {
                            let account = bd_core::accounts::Account {
                                service: service.clone(),
                                username: username.clone(),
                                enabled: true,
                            };
                            match self.accounts.upsert(account, &secret) {
                                Ok(()) => {
                                    state.set_form_error(false);
                                    state.set_form_message(check.summary.as_str().into());
                                    state.set_form_secret(SharedString::new());
                                    state.set_form_username(SharedString::new());
                                }
                                Err(e) => {
                                    state.set_form_error(true);
                                    state.set_form_message(e.into());
                                }
                            }
                        }
                        (Err(e), Some(_)) => {
                            state.set_form_error(true);
                            state.set_form_message(e.as_str().into());
                        }
                        _ => {}
                    }
                    self.account_status.retain(|(s, u, _)| !(s == &service && u == &username));
                    self.account_status.push((service, username, result));
                    self.resolver.set_credentials(self.accounts.credentials());
                    self.refresh_accounts(ui);
                }
                UiMsg::ToolInstalled { tool, result } => {
                    self.tool_slot = None;
                    match result {
                        Ok(path) => self.toast(format!("{} · {}", t("Installed"), path.display())),
                        Err(e) => self.toast(format!("{}: {e}", tool.exe_name())),
                    }
                    self.apply_config();
                    self.refresh_tool_status(ui);
                }
                UiMsg::ClickNLoad(received) => {
                    let count = received.links.len();
                    let source = received
                        .package
                        .clone()
                        .or(received.source.clone())
                        .unwrap_or_else(|| "Click'n'Load".to_owned());
                    let before: HashSet<u64> = self.grabber.snapshot().iter().map(|l| l.id).collect();
                    self.add_links(received.links, &source, true);
                    if let Some(package) = &received.package {
                        for link in self.grabber.snapshot().iter().filter(|l| !before.contains(&l.id)) {
                            self.grabber.rename_package(&link.package, package);
                        }
                    }
                    if let Some(pw) = &received.passwords {
                        if let Some(package) = &received.package {
                            self.grabber.set_package_password(package, pw);
                        }
                    }
                    self.toast(format!("Click'n'Load: {count}"));
                    self.notify(false, "Click'n'Load", &format!("{count} · {source}"));
                }
                UiMsg::ClickNLoadStopped(error) => {
                    self.clicknload = None;
                    self.clicknload_status = error;
                    self.refresh_modules(ui);
                }
                UiMsg::Favicon { host, image } => {
                    let icon = image.map(|(rgba, w, h)| {
                        slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(&rgba, w, h))
                    });
                    self.favicons.insert(host, icon);
                    self.grab_revision = u64::MAX;
                }
                UiMsg::Extracted { package, report } => {
                    for result in &report.results {
                        if let Some(job) = self.extract_jobs.iter_mut().find(|j| j.archive == result.archive) {
                            job.outcome = Some(result.outcome.clone());
                        }
                    }
                    self.extract_revision += 1;
                    let files: usize = report.results.iter().filter_map(|r| r.outcome.as_ref().ok()).sum();
                    let errors: Vec<String> = report
                        .results
                        .iter()
                        .filter_map(|r| r.outcome.as_ref().err().map(|e| format!("{}: {e}", r.archive.display())))
                        .collect();
                    let (title, body) = if errors.is_empty() {
                        (t("Extraction finished"), format!("{package} · {files}"))
                    } else {
                        (t("Extraction failed"), format!("{package}\n{}", errors.join("\n")))
                    };
                    self.toast(format!("{title}: {package}"));
                    self.notify(!errors.is_empty(), &title, &body);
                }
            }
        }
    }

    /// Starts or stops the Click'n'Load listener to match the module state.
    pub fn sync_clicknload(&mut self) {
        let wanted = self.modules.is_enabled("click-n-load");
        match (wanted, &self.clicknload) {
            (true, None) => {
                let cancel = CancellationToken::new();
                let sender = self.sender.clone();
                let on_links: bd_core::clicknload::OnLinks = {
                    let sender = sender.clone();
                    Arc::new(move |r| {
                        let _ = sender.send(UiMsg::ClickNLoad(r));
                    })
                };
                let port = self.settings.clicknload_port;
                let token = cancel.clone();
                self.runtime.spawn(async move {
                    if let Err(e) = bd_core::clicknload::serve(port, on_links, token).await {
                        let _ = sender.send(UiMsg::ClickNLoadStopped(format!("port {port}: {e}")));
                    }
                });
                self.clicknload = Some(cancel);
                self.clicknload_status = format!("127.0.0.1:{port}");
            }
            (false, Some(cancel)) => {
                cancel.cancel();
                self.clicknload = None;
                self.clicknload_status = String::new();
            }
            _ => {}
        }
    }

    /// Reacts to clipboard changes (sequence number) and grabs new links.
    pub fn check_clipboard(&mut self) {
        let seq = platform::clipboard_sequence();
        if seq == self.clipboard_seq {
            return;
        }
        self.clipboard_seq = seq;
        if !self.modules.is_enabled("clipboard") {
            return;
        }
        let Some(text) = platform::clipboard_text() else { return };
        if self.ignore_clipboard.as_deref() == Some(text.as_str()) {
            return;
        }
        let links = bd_core::links::extract_links(&text);
        let added = self.add_links(links, "Clipboard", false);
        if added > 0 {
            self.toast(format!("+{added}"));
        }
    }
}

impl App {
    fn job_row(&mut self, job: &JobSnapshot) -> JobRow {
        let selected = self.selected == Some(job.id);
        let icon_host = favicon_host(&job.url);
        let icon = self.favicon(&icon_host);
        let fraction = match job.total {
            Some(0) => 1.0,
            Some(total) => job.downloaded as f64 / total as f64,
            None if job.state == JobState::Finished => 1.0,
            None => 0.0,
        };
        let segs: Vec<f32> = if job.segments.is_empty() {
            vec![fraction as f32]
        } else {
            job.segments
                .iter()
                .map(|s| match s.len {
                    Some(0) => 1.0,
                    Some(len) => (s.done as f64 / len as f64) as f32,
                    None => {
                        if job.state == JobState::Finished {
                            1.0
                        } else {
                            0.0
                        }
                    }
                })
                .collect()
        };
        let (status, kind) = match &job.state {
            JobState::Queued => (t("Queued"), 0),
            JobState::Downloading if job.status.is_some() => (t(job.status.as_deref().unwrap_or_default()), 1),
            JobState::Downloading if job.total.is_none() && job.downloaded == 0 => (t("Connecting…"), 1),
            // The host limits connections: segments wait for a free slot.
            JobState::Downloading if job.connections == 0 => (t("Waiting for a connection"), 1),
            JobState::Downloading => (
                if job.connections > 1 {
                    format!("{} · {} conn", t("Downloading"), job.connections)
                } else {
                    t("Downloading")
                },
                1,
            ),
            JobState::Paused => (t("Paused"), 6),
            JobState::Finished => (t("Finished"), 4),
            JobState::Failed(e) => (short_error(e), 5),
        };
        let tag = match job.spec.kind {
            JobKind::Video { ref format } if matches!(format, VideoFormat::AudioBest | VideoFormat::AudioMp3) => {
                "AUDIO"
            }
            JobKind::Video { .. } => "VIDEO",
            JobKind::Http => match bd_core::hosters::hoster_of(&job.url) {
                Some(_) => "DDL",
                None => "HTTP",
            },
        };
        JobRow {
            id: job.id as i32,
            name: job.name.as_str().into(),
            host: bd_core::hosters::hoster_of(&job.url)
                .map(str::to_owned)
                .unwrap_or_else(|| job.url.host_str().unwrap_or_default().to_owned())
                .into(),
            size: job.total.map(format::bytes).unwrap_or_default().into(),
            segs: ModelRc::new(VecModel::from(segs)),
            bar_kind: match job.state {
                JobState::Finished => 1,
                JobState::Downloading => 0,
                _ => 2,
            },
            percent: if job.total.is_some() || job.state == JobState::Finished {
                format!("{}%", (fraction * 100.0).floor())
            } else {
                String::new()
            }
            .into(),
            speed: format::speed(job.speed).into(),
            eta: job.eta().map(format::duration).unwrap_or_default().into(),
            status: status.into(),
            status_kind: kind,
            selected,
            tag: tag.into(),
            has_icon: icon.is_some(),
            icon: icon.unwrap_or_default(),
            package: job.spec.package.clone().unwrap_or_default().into(),
            added: format::date(job.spec.added).into(),
            downloaded: if job.downloaded > 0 { format::bytes(job.downloaded) } else { String::new() }.into(),
            conns: if job.connections > 0 { job.connections.to_string() } else { String::new() }.into(),
            priority: job.spec.priority,
            is_archive: bd_core::packages::is_first_volume(&job.name),
            has_limit: job.spec.rate_limit > 0,
        }
    }
}

/// Domain whose favicon represents the link (the hoster's main domain).
pub fn favicon_host(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    bd_core::hosters::HOSTERS
        .iter()
        .find(|(_, domains, _)| domains.iter().any(|d| host == *d || host.ends_with(&format!(".{d}"))))
        .map(|(_, domains, _)| domains[0].to_owned())
        .unwrap_or_else(|| host.trim_start_matches("www.").to_owned())
}

fn short_error(error: &str) -> String {
    if error.contains("status 404") {
        t("File not found (404)")
    } else if error.contains("status 403") {
        t("Access denied (403)")
    } else if error.contains("status 429") {
        t("Too many requests")
    } else if error.starts_with("network error") {
        t("Network error")
    } else {
        error.to_owned()
    }
}

/// Sparkline paths in a 150x34 viewbox.
fn sparkline(history: &VecDeque<f64>) -> (String, String) {
    let max = history.iter().copied().fold(1_048_576.0, f64::max);
    let n = history.len().max(2) - 1;
    let line: String = history
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = i as f64 / n as f64 * 150.0;
            let y = 32.0 - v / max * 30.0;
            format!("{}{x:.1} {y:.1}", if i == 0 { "M" } else { "L" })
        })
        .collect();
    let area = format!("{line}L150 34L0 34Z");
    (line, area)
}
