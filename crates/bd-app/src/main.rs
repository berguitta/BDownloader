//! BDownloader desktop application.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod format;
mod platform;
#[cfg(feature = "snapshot")]
mod snapshot;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::Result;
use bd_core::links::extract_links;
use bd_core::manager::ManagerConfig;
use bd_core::modules::ModuleStatus;
use bd_core::{DownloadManager, JobId, JobSnapshot, JobState, ModuleRegistry, Settings, ThemeMode};
use bd_net::Engine;
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};

slint::include_modules!();

const TICK: Duration = Duration::from_millis(250);
const TOAST_DURATION: Duration = Duration::from_millis(2600);
const HISTORY_SECONDS: usize = 60;

struct App {
    manager: DownloadManager,
    settings: Settings,
    settings_path: Option<PathBuf>,
    modules: ModuleRegistry,
    rows: Rc<VecModel<JobRow>>,
    row_ids: Vec<JobId>,
    jobs: Vec<JobSnapshot>,
    selected: Option<JobId>,
    history: VecDeque<f64>,
    last_history: Instant,
    toast: Option<(String, Instant)>,
}

impl App {
    fn save_settings(&mut self) {
        self.settings.modules = self.modules.states();
        if let Some(path) = &self.settings_path {
            if let Err(e) = self.settings.save_to(path) {
                tracing::warn!(error = %e, "cannot save settings");
            }
        }
    }

    fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), Instant::now()));
    }

    fn manager_config(settings: &Settings, modules: &ModuleRegistry) -> ManagerConfig {
        ManagerConfig {
            dest_dir: settings.download_dir.clone(),
            max_parallel: settings.max_parallel,
            connections: settings.connections_per_download,
            multi_connection: modules.is_enabled("multi-connection"),
            resume: modules.is_enabled("resume"),
        }
    }

    fn apply_config(&self) {
        let config = Self::manager_config(&self.settings, &self.modules);
        self.manager.update_config(|c| *c = config);
    }

    fn job_at(&self, row: i32) -> Option<&JobSnapshot> {
        let id = *self.row_ids.get(usize::try_from(row).ok()?)?;
        self.jobs.iter().find(|j| j.id == id)
    }

    /// Pulls fresh state from the download manager and pushes it to the UI.
    fn refresh(&mut self, ui: &AppWindow) {
        self.jobs = self.manager.snapshot();
        let state = ui.global::<DownloadsState>();

        // Counters and global speed.
        let count = |f: &dyn Fn(&JobState) -> bool| self.jobs.iter().filter(|j| f(&j.state)).count();
        let active = count(&|s| *s == JobState::Downloading);
        let queued = count(&|s| *s == JobState::Queued);
        // + 0.0 turns the -0.0 of an empty sum into 0.0.
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

        if self.last_history.elapsed() >= Duration::from_secs(1) {
            self.last_history = Instant::now();
            self.history.pop_front();
            self.history.push_back(speed);
        }
        let (line, area) = sparkline(&self.history);
        state.set_spark_line(line.into());
        state.set_spark_area(area.into());

        // Visible rows.
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
            })
            .collect();
        let ids: Vec<JobId> = visible.iter().map(|j| j.id).collect();
        let rows: Vec<JobRow> = visible.iter().map(|j| job_row(j, self.selected == Some(j.id))).collect();
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

        match &self.toast {
            Some((text, at)) if at.elapsed() < TOAST_DURATION => {
                ui.set_toast_text(text.as_str().into());
                ui.set_toast_visible(true);
            }
            _ => ui.set_toast_visible(false),
        }
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
                "{} of {} · {:.1}%",
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
        if let JobState::Failed(error) = &job.state {
            info.push(kv("Error", error.clone()));
        }
        if job.resumed {
            info.push(kv("Note", "Resumed from a previous session".to_owned()));
        }
        state.set_detail_info(ModelRc::new(VecModel::from(info)));
        state.set_detail_right_title(
            match job.segments.len() {
                0 => "Waiting for the server".to_owned(),
                1 => "1 connection".to_owned(),
                n => format!("{n} connections ({} active)", job.connections),
            }
            .into(),
        );
        let connections: Vec<Connection> = job
            .segments
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let progress = s.len.map(|len| if len == 0 { 1.0 } else { s.done as f64 / len as f64 }).unwrap_or(0.0);
                Connection {
                    label: format!("Conn {}", i + 1).into(),
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

    fn module_rows(&self) -> Vec<ModuleRow> {
        let mut last_category = None;
        self.modules
            .list()
            .into_iter()
            .map(|(info, enabled)| {
                let first = last_category != Some(info.category);
                last_category = Some(info.category);
                ModuleRow {
                    id: info.id.into(),
                    name: info.name.into(),
                    description: info.description.into(),
                    category: info.category.label().into(),
                    first_in_category: first,
                    enabled,
                    available: info.status == ModuleStatus::Available,
                }
            })
            .collect()
    }
}

fn job_row(job: &JobSnapshot, selected: bool) -> JobRow {
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
        JobState::Queued => ("Queued".to_owned(), 0),
        JobState::Downloading if job.total.is_none() && job.downloaded == 0 => ("Connecting…".to_owned(), 1),
        // The host limits connections: segments wait for a free slot.
        JobState::Downloading if job.connections == 0 => ("Waiting for a connection".to_owned(), 1),
        JobState::Downloading => (
            if job.connections > 1 {
                format!("Downloading · {} conn", job.connections)
            } else {
                "Downloading".to_owned()
            },
            1,
        ),
        JobState::Paused => ("Paused".to_owned(), 6),
        JobState::Finished => ("Finished".to_owned(), 4),
        JobState::Failed(e) => (short_error(e), 5),
    };
    JobRow {
        id: job.id as i32,
        name: job.name.as_str().into(),
        host: job.url.host_str().unwrap_or_default().into(),
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
    }
}

fn short_error(error: &str) -> String {
    if error.contains("status 404") {
        "File not found (404)".to_owned()
    } else if error.contains("status 403") {
        "Access denied (403)".to_owned()
    } else if error.contains("status 429") {
        "Too many requests".to_owned()
    } else if error.starts_with("network error") {
        "Network error".to_owned()
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

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("BD_LOG").unwrap_or_else(|_| "info".into()))
        .init();

    let settings_path = Settings::default_path();
    let settings = settings_path.as_deref().map(Settings::load_from).unwrap_or_default();
    let modules = ModuleRegistry::builtin().with_states(&settings.modules);

    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().thread_name("bd-worker").build()?;
    let manager =
        DownloadManager::new(Engine::new()?, App::manager_config(&settings, &modules), runtime.handle().clone());

    #[cfg(feature = "snapshot")]
    let snapshot = snapshot::Snapshot::from_env();
    #[cfg(feature = "snapshot")]
    if let Some(snapshot) = &snapshot {
        snapshot.install()?;
    }

    let ui = AppWindow::new()?;
    let rows = Rc::new(VecModel::<JobRow>::default());
    ui.global::<DownloadsState>().set_rows(ModelRc::from(rows.clone()));

    let app = Rc::new(RefCell::new(App {
        manager: manager.clone(),
        settings,
        settings_path,
        modules,
        rows,
        row_ids: Vec::new(),
        jobs: Vec::new(),
        selected: None,
        history: VecDeque::from(vec![0.0; HISTORY_SECONDS]),
        last_history: Instant::now(),
        toast: None,
    }));

    init_settings_ui(&ui, &app);
    wire_downloads(&ui, &app);
    wire_add_dialog(&ui, &app);
    wire_settings(&ui, &app);
    #[cfg(feature = "snapshot")]
    if let Some(snapshot) = &snapshot {
        if let Some(theme) = snapshot.theme {
            ui.global::<SettingsState>().set_theme_mode(theme);
        }
        if let Some(links) = &snapshot.links {
            ui.invoke_add_dialog_confirm(links.as_str().into());
        }
        if let Some(page) = snapshot.page {
            ui.set_page(page);
        }
        if let Some(row) = snapshot.select {
            // Select once the first jobs have appeared.
            let (weak, a) = (ui.as_weak(), app.clone());
            Timer::single_shot(Duration::from_millis(1500), move || {
                if let Some(ui) = weak.upgrade() {
                    let mut app = a.borrow_mut();
                    app.selected = app.job_at(row).map(|j| j.id);
                    app.refresh(&ui);
                }
            });
        }
    }

    // Apply the saved theme now that the settings global is filled.
    ui.invoke_apply_theme(ui.global::<SettingsState>().get_theme_mode());

    let timer = Timer::default();
    {
        let weak = ui.as_weak();
        let app = app.clone();
        timer.start(TimerMode::Repeated, TICK, move || {
            if let Some(ui) = weak.upgrade() {
                app.borrow_mut().refresh(&ui);
            }
        });
    }
    app.borrow_mut().refresh(&ui);

    ui.run()?;

    // Save progress of running downloads so they resume next time.
    runtime.block_on(manager.shutdown());
    app.borrow_mut().save_settings();
    Ok(())
}

fn init_settings_ui(ui: &AppWindow, app: &Rc<RefCell<App>>) {
    let app = app.borrow();
    let s = ui.global::<SettingsState>();
    s.set_theme_mode(match app.settings.theme {
        ThemeMode::System => 0,
        ThemeMode::Light => 1,
        ThemeMode::Dark => 2,
    });
    s.set_download_dir(app.settings.download_dir.display().to_string().into());
    s.set_parallel(app.settings.max_parallel as i32);
    s.set_connections(app.settings.connections_per_download as i32);
    s.set_modules(ModelRc::new(VecModel::from(app.module_rows())));
    s.set_config_path(app.settings_path.as_ref().map(|p| p.display().to_string()).unwrap_or_default().into());
    ui.global::<DownloadsState>().set_parallel(app.settings.max_parallel as i32);
}

fn wire_downloads(ui: &AppWindow, app: &Rc<RefCell<App>>) {
    let state = ui.global::<DownloadsState>();

    let (weak, a) = (ui.as_weak(), app.clone());
    state.on_filter_changed(move || {
        if let Some(ui) = weak.upgrade() {
            a.borrow_mut().refresh(&ui);
        }
    });

    let (weak, a) = (ui.as_weak(), app.clone());
    state.on_parallel_changed(move |value| {
        let mut app = a.borrow_mut();
        app.settings.max_parallel = value.clamp(1, 50) as usize;
        app.apply_config();
        app.save_settings();
        if let Some(ui) = weak.upgrade() {
            ui.global::<SettingsState>().set_parallel(value);
        }
    });

    let weak = ui.as_weak();
    state.on_add_links(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_add_dialog_open(true);
        }
    });

    let a = app.clone();
    state.on_start_all(move || {
        let mut app = a.borrow_mut();
        app.manager.resume_all();
        app.toast("Queue started");
    });

    let a = app.clone();
    state.on_pause_all(move || {
        let mut app = a.borrow_mut();
        app.manager.pause_all();
        app.toast("All downloads paused");
    });

    let (weak, a) = (ui.as_weak(), app.clone());
    state.on_select(move |row| {
        let mut app = a.borrow_mut();
        app.selected = app.job_at(row).map(|j| j.id);
        if let Some(ui) = weak.upgrade() {
            app.refresh(&ui);
        }
    });

    let a = app.clone();
    state.on_activate(move |row| {
        let app = a.borrow();
        if let Some(job) = app.job_at(row) {
            match (&job.state, &job.path) {
                (JobState::Finished, Some(path)) => platform::open(path),
                _ => toggle(&app.manager, job),
            }
        }
    });

    let a = app.clone();
    state.on_pause_resume(move |row| {
        let app = a.borrow();
        if let Some(job) = app.job_at(row) {
            toggle(&app.manager, job);
        }
    });

    let (weak, a) = (ui.as_weak(), app.clone());
    state.on_row_action(move |row, action| {
        let mut app = a.borrow_mut();
        let Some(job) = app.job_at(row).cloned() else { return };
        match action.as_str() {
            "open-file" => {
                if let Some(path) = &job.path {
                    platform::open(path);
                }
            }
            "open-folder" => platform::reveal(job.path.as_deref().unwrap_or(&job.dest_dir)),
            "copy-link" => {
                if platform::copy_text(job.url.as_str()) {
                    app.toast("Link copied");
                }
            }
            "remove" | "delete" => {
                let delete = action == "delete";
                app.manager.remove(job.id, delete);
                if app.selected == Some(job.id) {
                    app.selected = None;
                }
                app.toast(if delete { format!("Deleted {}", job.name) } else { format!("Removed {}", job.name) });
            }
            other => tracing::debug!(action = other, "unknown row action"),
        }
        if let Some(ui) = weak.upgrade() {
            app.refresh(&ui);
        }
    });
}

fn toggle(manager: &DownloadManager, job: &JobSnapshot) {
    match job.state {
        JobState::Downloading | JobState::Queued => manager.pause(job.id),
        JobState::Paused | JobState::Failed(_) => manager.resume(job.id),
        JobState::Finished => {}
    }
}

fn wire_add_dialog(ui: &AppWindow, app: &Rc<RefCell<App>>) {
    let weak = ui.as_weak();
    ui.on_add_dialog_edited(move |text| {
        if let Some(ui) = weak.upgrade() {
            ui.set_add_dialog_detected(extract_links(&text).len() as i32);
        }
    });

    let (weak, a) = (ui.as_weak(), app.clone());
    ui.on_add_dialog_confirm(move |text| {
        let Some(ui) = weak.upgrade() else { return };
        let mut app = a.borrow_mut();
        let links = extract_links(&text);
        if links.is_empty() {
            app.toast("No link found in the text");
            return;
        }
        if !app.modules.is_enabled("http") {
            app.toast("Turn on the Direct HTTP/HTTPS module in Settings to download links");
            return;
        }
        let count = links.len();
        for link in links {
            app.manager.add(link);
        }
        app.toast(if count == 1 { "1 link added".to_owned() } else { format!("{count} links added") });
        ui.set_add_dialog_open(false);
        ui.set_add_dialog_text(SharedString::new());
        ui.set_add_dialog_detected(0);
        ui.set_page(0);
        app.refresh(&ui);
    });
}

fn wire_settings(ui: &AppWindow, app: &Rc<RefCell<App>>) {
    let s = ui.global::<SettingsState>();

    let (weak, a) = (ui.as_weak(), app.clone());
    s.on_theme_changed(move |mode| {
        let mut app = a.borrow_mut();
        app.settings.theme = match mode {
            1 => ThemeMode::Light,
            2 => ThemeMode::Dark,
            _ => ThemeMode::System,
        };
        app.save_settings();
        if let Some(ui) = weak.upgrade() {
            ui.invoke_apply_theme(mode);
        }
    });

    let a = app.clone();
    s.on_download_dir_changed(move |dir| {
        let dir = dir.trim();
        if dir.is_empty() {
            return;
        }
        let mut app = a.borrow_mut();
        app.settings.download_dir = PathBuf::from(dir);
        app.apply_config();
        app.save_settings();
    });

    let (weak, a) = (ui.as_weak(), app.clone());
    s.on_parallel_changed(move |value| {
        let mut app = a.borrow_mut();
        app.settings.max_parallel = value.clamp(1, 50) as usize;
        app.apply_config();
        app.save_settings();
        if let Some(ui) = weak.upgrade() {
            ui.global::<DownloadsState>().set_parallel(value);
        }
    });

    let a = app.clone();
    s.on_connections_changed(move |value| {
        let mut app = a.borrow_mut();
        app.settings.connections_per_download = value.clamp(1, 32) as usize;
        app.apply_config();
        app.save_settings();
    });

    let (weak, a) = (ui.as_weak(), app.clone());
    s.on_module_toggled(move |id, enabled| {
        let Some(ui) = weak.upgrade() else { return };
        let mut app = a.borrow_mut();
        match app.modules.set_enabled(&id, enabled) {
            Ok(changed) if !changed.is_empty() => {
                let names: Vec<&str> = changed.iter().filter_map(|c| app.modules.get(c)).map(|m| m.name).collect();
                let verb = if enabled { "Also turned on" } else { "Also turned off" };
                app.toast(format!("{verb}: {}", names.join(", ")));
            }
            Ok(_) => {}
            Err(e) => app.toast(e.to_string()),
        }
        app.apply_config();
        app.save_settings();
        ui.global::<SettingsState>().set_modules(ModelRc::new(VecModel::from(app.module_rows())));
    });
}
