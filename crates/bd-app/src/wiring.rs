//! UI callbacks.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use bd_core::accounts::{Account, SERVICES};
use bd_core::links::extract_links;
use bd_core::task::ControlSlot;
use bd_core::tools::{self, Tool};
use bd_core::{JobSnapshot, JobState, ThemeMode};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app::{App, UiMsg};
use crate::i18n::t;
use crate::{
    platform, settings_view, AccountsState, AppWindow, Columns, DownloadsState, ExtractionState, GrabberState,
    SettingsState,
};

type Shared = Rc<RefCell<App>>;
type Action<'a> = &'a dyn Fn(&mut App, &AppWindow);

/// Runs `f` with the app state and the window, if the window still exists.
fn with(ui: &AppWindow, app: &Shared) -> impl Fn(Action) {
    let (weak, app) = (ui.as_weak(), app.clone());
    move |f| {
        if let Some(ui) = weak.upgrade() {
            f(&mut app.borrow_mut(), &ui);
        }
    }
}

pub fn init_ui(ui: &AppWindow, app: &Shared) {
    let app = app.borrow();
    let s = ui.global::<SettingsState>();
    s.set_theme_mode(match app.settings.theme {
        ThemeMode::System => 0,
        ThemeMode::Light => 1,
        ThemeMode::Dark => 2,
    });
    ui.set_compact(app.settings.text("density", "comfortable") == "compact");
    ui.global::<DownloadsState>().set_columns(columns_from(app.settings.text("columns", DEFAULT_COLUMNS)));
    let e = ui.global::<ExtractionState>();
    e.set_passwords(app.settings.archive_passwords.join("\n").into());
    e.set_auto(app.modules.is_enabled("extraction"));
    e.set_delete_after(app.settings.delete_archives);
    e.set_try_passwords(app.settings.flag("try-passwords", true));
    ui.global::<DownloadsState>().set_parallel(app.settings.max_parallel as i32);
    ui.global::<GrabberState>().set_auto_start(app.settings.auto_start_grabbed);

    let a = ui.global::<AccountsState>();
    a.set_services(ModelRc::new(VecModel::from(
        SERVICES.iter().map(|s| SharedString::from(s.name)).collect::<Vec<_>>(),
    )));
    a.set_service_needs_username(ModelRc::new(VecModel::from(
        SERVICES.iter().map(|s| s.needs_username).collect::<Vec<_>>(),
    )));
    a.set_service_secret_labels(ModelRc::new(VecModel::from(
        SERVICES.iter().map(|s| SharedString::from(s.secret_label)).collect::<Vec<_>>(),
    )));
    a.set_service_help(ModelRc::new(VecModel::from(
        SERVICES.iter().map(|s| SharedString::from(s.help)).collect::<Vec<_>>(),
    )));
    app.refresh_accounts(ui);
    app.refresh_modules(ui);
}

pub fn wire_all(ui: &AppWindow, app: &Shared) {
    wire_downloads(ui, app);
    wire_add_dialog(ui, app);
    wire_grabber(ui, app);
    wire_accounts(ui, app);
    wire_settings(ui, app);
    wire_extraction(ui, app);
    wire_window(ui, app);
}

pub const DEFAULT_COLUMNS: &str = "hoster,size,progress,speed,eta,status";

fn columns_from(list: &str) -> Columns {
    let has = |name: &str| list.split(',').any(|c| c.trim() == name);
    Columns {
        hoster: has("hoster"),
        size: has("size"),
        progress: has("progress"),
        speed: has("speed"),
        eta: has("eta"),
        status: has("status"),
        package: has("package"),
        added: has("added"),
        downloaded: has("downloaded"),
        connections: has("connections"),
    }
}

fn toggle(app: &App, job: &JobSnapshot) {
    match job.state {
        JobState::Downloading | JobState::Queued => app.manager.pause(job.id),
        JobState::Paused | JobState::Failed(_) => app.manager.resume(job.id),
        JobState::Finished => {}
    }
}

fn wire_downloads(ui: &AppWindow, app: &Shared) {
    let state = ui.global::<DownloadsState>();

    let run = with(ui, app);
    state.on_filter_changed(move || run(&|app, ui| app.refresh(ui)));

    let run = with(ui, app);
    state.on_parallel_changed(move |value| {
        run(&|app, ui| {
            app.settings.max_parallel = value.clamp(1, 50) as usize;
            app.apply_config();
            app.save_settings();
            crate::settings_view::refresh(app, ui);
        })
    });

    let weak = ui.as_weak();
    state.on_add_links(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_add_dialog_open(true);
        }
    });

    let run = with(ui, app);
    state.on_start_all(move || {
        run(&|app, _| {
            app.manager.resume_all();
            app.toast(t("Queue started"));
        })
    });

    let run = with(ui, app);
    state.on_pause_all(move || {
        run(&|app, _| {
            app.manager.pause_all();
            app.toast(t("All downloads paused"));
        })
    });

    let run = with(ui, app);
    state.on_select(move |row| {
        run(&|app, ui| {
            app.selected = app.job_at(row).map(|j| j.id);
            app.refresh(ui);
        })
    });

    let run = with(ui, app);
    state.on_activate(move |row| {
        run(&|app, _| {
            if let Some(job) = app.job_at(row).cloned() {
                match (&job.state, &job.path) {
                    (JobState::Finished, Some(path)) => platform::open(path),
                    _ => toggle(app, &job),
                }
            }
        })
    });

    let run = with(ui, app);
    state.on_pause_resume(move |row| {
        run(&|app, _| {
            if let Some(job) = app.job_at(row).cloned() {
                toggle(app, &job);
            }
        })
    });

    let run = with(ui, app);
    state.on_row_action(move |row, action| {
        run(&|app, ui| {
            let Some(job) = app.job_at(row).cloned() else { return };
            match action.as_str() {
                "open-file" => {
                    if let Some(path) = &job.path {
                        platform::open(path);
                    }
                }
                "open-folder" => {
                    // The finished file, else the partial file, else the folder.
                    let part = job.dest_dir.join(format!("{}.part", job.name));
                    let target = job
                        .path
                        .clone()
                        .filter(|p| p.exists())
                        .or_else(|| part.exists().then_some(part))
                        .unwrap_or_else(|| job.dest_dir.clone());
                    platform::reveal(&target);
                }
                "copy-link" => {
                    app.ignore_clipboard = Some(job.url.to_string());
                    if platform::copy_text(job.url.as_str()) {
                        app.toast(t("Link copied"));
                    }
                }
                "copy-name" => {
                    app.ignore_clipboard = Some(job.name.clone());
                    platform::copy_text(&job.name);
                }
                "copy-path" => {
                    let path = job.path.clone().unwrap_or_else(|| job.dest_dir.join(&job.name)).display().to_string();
                    app.ignore_clipboard = Some(path.clone());
                    platform::copy_text(&path);
                }
                "remove" | "delete" => {
                    app.manager.remove(job.id, action == "delete");
                    if app.selected == Some(job.id) {
                        app.selected = None;
                    }
                    app.toast(format!("− {}", job.name));
                }
                "clear-finished" => app.manager.clear_finished(),
                "start" => app.manager.resume(job.id),
                "force-start" => app.manager.force_start(job.id),
                "pause" => app.manager.pause(job.id),
                "reset" => app.manager.reset(job.id, false),
                "restart" => app.manager.reset(job.id, true),
                "extract" => {
                    if let Some(path) = job.path.clone() {
                        let package = job.spec.package.clone().unwrap_or_else(|| job.name.clone());
                        let files: Vec<PathBuf> = app
                            .jobs
                            .iter()
                            .filter(|j| j.spec.package.is_some() && j.spec.package == job.spec.package)
                            .filter_map(|j| j.path.clone())
                            .chain(std::iter::once(path))
                            .collect();
                        app.start_extraction(package, job.dest_dir.clone(), files, job.spec.password.clone());
                        ui.set_page(3);
                    }
                }
                "rename" => {
                    app.input_action = Some((job.id, "rename".to_owned()));
                    ui.set_input_title(t("Rename").into());
                    ui.set_input_hint(job.name.as_str().into());
                    ui.set_input_value(job.name.as_str().into());
                    ui.set_input_open(true);
                }
                "file-limit" => {
                    app.input_action = Some((job.id, "file-limit".to_owned()));
                    ui.set_input_title(t("Speed limit for this file").into());
                    ui.set_input_hint(t("In MB/s, any value. 0 removes the limit.").into());
                    let current = job.spec.rate_limit as f64 / 1_048_576.0;
                    ui.set_input_value(if current > 0.0 { format!("{current:.2}") } else { String::new() }.into());
                    ui.set_input_open(true);
                }
                "file-limit:0" => app.manager.set_job_limit(job.id, 0),
                "change-folder" => {
                    if let Some(dir) = rfd::FileDialog::new().set_directory(&job.dest_dir).pick_folder() {
                        if let Err(e) = app.manager.change_folder(job.id, &dir) {
                            app.toast(e);
                        }
                    }
                }
                "properties" => {
                    app.selected = Some(job.id);
                }
                other => {
                    if let Some(p) = other.strip_prefix("priority:") {
                        app.manager.set_priority(job.id, p.parse().unwrap_or(0));
                    } else if let Some(n) = other.strip_prefix("connections:") {
                        app.manager.set_job_connections(job.id, n.parse().unwrap_or(8));
                        app.toast(t("Applies at the next start"));
                    } else if let Some(to) = other.strip_prefix("move:") {
                        let to = match to {
                            "top" => bd_core::manager::Move::Top,
                            "up" => bd_core::manager::Move::Up,
                            "down" => bd_core::manager::Move::Down,
                            _ => bd_core::manager::Move::Bottom,
                        };
                        app.manager.move_job(job.id, to);
                    } else {
                        tracing::debug!(action = other, "unknown row action");
                    }
                }
            }
            app.refresh(ui);
        })
    });

    let run = with(ui, app);
    state.on_clear_finished(move || {
        run(&|app, ui| {
            app.manager.clear_finished();
            app.refresh(ui);
        })
    });

    let run = with(ui, app);
    state.on_toggle_column(move |name| {
        run(&|app, ui| {
            let mut list: Vec<String> = app
                .settings
                .text("columns", DEFAULT_COLUMNS)
                .split(',')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect();
            if name == "reset" {
                list = DEFAULT_COLUMNS.split(',').map(str::to_owned).collect();
            } else if let Some(pos) = list.iter().position(|c| *c == *name) {
                list.remove(pos);
            } else {
                list.push(name.to_string());
            }
            let joined = list.join(",");
            ui.global::<DownloadsState>().set_columns(columns_from(&joined));
            app.settings.set_option("columns", joined);
            app.save_settings();
        })
    });
}

fn wire_add_dialog(ui: &AppWindow, app: &Shared) {
    let weak = ui.as_weak();
    ui.on_add_dialog_edited(move |text| {
        if let Some(ui) = weak.upgrade() {
            ui.set_add_dialog_detected(extract_links(&text).len() as i32);
        }
    });

    let run = with(ui, app);
    ui.on_add_dialog_confirm(move |text| {
        run(&|app, ui| {
            let links = extract_links(&text);
            if links.is_empty() {
                app.toast(t("No link found in the text"));
                return;
            }
            let count = app.add_links(links, "Paste", false);
            app.toast(format!("{} {count}", t("Links added:")));
            ui.set_add_dialog_open(false);
            ui.set_add_dialog_text(SharedString::new());
            ui.set_add_dialog_detected(0);
            ui.set_page(if app.settings.auto_start_grabbed { 0 } else { 1 });
            app.refresh_grabber(ui, true);
        })
    });
}

fn wire_grabber(ui: &AppWindow, app: &Shared) {
    let g = ui.global::<GrabberState>();

    let weak = ui.as_weak();
    g.on_add_links(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_add_dialog_open(true);
        }
    });

    let run = with(ui, app);
    g.on_start_selected(move || {
        run(&|app, ui| {
            let specs =
                app.grabber.take_selected(&app.settings.download_dir, app.settings.flag("folder-per-package", true));
            let count = specs.len();
            for spec in specs {
                app.manager.add_spec(spec);
            }
            if count > 0 {
                app.toast(format!("▶ {count}"));
                ui.set_page(0);
            }
            app.refresh_grabber(ui, true);
        })
    });

    let run = with(ui, app);
    g.on_select_all(move |on| {
        run(&|app, ui| {
            app.grabber.select_all(on);
            app.refresh_grabber(ui, true);
        })
    });
    let run = with(ui, app);
    g.on_remove_offline(move || {
        run(&|app, ui| {
            app.grabber.remove_offline();
            app.refresh_grabber(ui, true);
        })
    });
    let run = with(ui, app);
    g.on_clear(move || {
        run(&|app, ui| {
            app.grabber.clear();
            app.refresh_grabber(ui, true);
        })
    });
    let run = with(ui, app);
    g.on_toggle_package(move |package| {
        run(&|app, ui| {
            let package = package.to_string();
            if !app.closed_packages.remove(&package) {
                app.closed_packages.insert(package);
            }
            app.refresh_grabber(ui, true);
        })
    });
    let run = with(ui, app);
    g.on_check_package(move |package, on| {
        run(&|app, ui| {
            app.grabber.set_package_selected(&package, on);
            app.refresh_grabber(ui, true);
        })
    });
    let run = with(ui, app);
    g.on_check_link(move |id, on| {
        run(&|app, ui| {
            app.grabber.set_selected(id as u64, on);
            app.refresh_grabber(ui, true);
        })
    });
    let run = with(ui, app);
    g.on_choose_format(move |id, index| {
        run(&|app, ui| {
            app.grabber.set_video_choice(id as u64, index.max(0) as usize);
            app.refresh_grabber(ui, true);
        })
    });
    let run = with(ui, app);
    g.on_package_password(move |package, password| {
        run(&|app, _| app.grabber.set_package_password(&package, &password))
    });
    let run = with(ui, app);
    g.on_remove_link(move |id| {
        run(&|app, ui| {
            app.grabber.remove(&[id as u64]);
            app.refresh_grabber(ui, true);
        })
    });
    let run = with(ui, app);
    g.on_remove_package(move |package| {
        run(&|app, ui| {
            app.grabber.remove_package(&package);
            app.refresh_grabber(ui, true);
        })
    });
    let run = with(ui, app);
    g.on_use_mirror(move |id| {
        run(&|app, ui| {
            app.grabber.use_mirror(id as u64, 0);
            app.refresh_grabber(ui, true);
        })
    });
    let run = with(ui, app);
    g.on_clipboard_toggled(move |on| run(&|app, ui| set_module(app, ui, "clipboard", on)));
    let run = with(ui, app);
    g.on_clicknload_toggled(move |on| run(&|app, ui| set_module(app, ui, "click-n-load", on)));
    let run = with(ui, app);
    g.on_auto_start_toggled(move |on| {
        run(&|app, _| {
            app.settings.auto_start_grabbed = on;
            app.save_settings();
        })
    });
}

pub fn set_module(app: &mut App, ui: &AppWindow, id: &str, on: bool) {
    match app.modules.set_enabled(id, on) {
        Ok(changed) if !changed.is_empty() => {
            let names: Vec<&str> = changed.iter().filter_map(|c| app.modules.get(c)).map(|m| m.name).collect();
            app.toast(format!("{} {}", if on { "+" } else { "−" }, names.join(", ")));
        }
        Ok(_) => {}
        Err(e) => app.toast(e.to_string()),
    }
    app.apply_config();
    app.sync_clicknload();
    app.save_settings();
    app.refresh_modules(ui);
}

fn check_account(app: &App, service: String, username: String, secret: String, save: bool) {
    let resolver = app.resolver.clone();
    let sender = app.sender.clone();
    app.runtime.spawn(async move {
        let result = resolver.check_account(&service, &username, &secret).await;
        let _ = sender.send(UiMsg::AccountChecked { service, username, result, save_secret: save.then_some(secret) });
    });
}

fn wire_accounts(ui: &AppWindow, app: &Shared) {
    let a = ui.global::<AccountsState>();

    let run = with(ui, app);
    a.on_add_account(move || {
        run(&|app, ui| {
            let state = ui.global::<AccountsState>();
            let Some(service) = SERVICES.get(state.get_form_service().max(0) as usize) else { return };
            let secret = state.get_form_secret().trim().to_owned();
            let username = if service.needs_username {
                state.get_form_username().trim().to_owned()
            } else {
                service.name.to_owned()
            };
            if secret.is_empty() || username.is_empty() {
                state.set_form_error(true);
                state.set_form_message(format!("{} ?", service.secret_label).into());
                return;
            }
            state.set_form_busy(true);
            state.set_form_message(SharedString::new());
            check_account(app, service.id.to_owned(), username, secret, true);
        })
    });

    let run = with(ui, app);
    a.on_check(move |service, username| {
        run(&|app, _| {
            let account =
                app.accounts.list().iter().find(|a| a.service == *service && a.username == *username).cloned();
            if let Some(secret) = account.as_ref().and_then(|a| app.accounts.secret(a)) {
                check_account(app, service.to_string(), username.to_string(), secret, false);
            }
        })
    });

    let run = with(ui, app);
    a.on_remove(move |service, username| {
        run(&|app, ui| {
            if let Err(e) = app.accounts.remove(&service, &username) {
                app.toast(e);
            }
            app.apply_config();
            app.refresh_accounts(ui);
        })
    });

    let run = with(ui, app);
    a.on_set_enabled(move |service, username, on| {
        run(&|app, ui| {
            let _ = app.accounts.set_enabled(&service, &username, on);
            app.apply_config();
            app.refresh_accounts(ui);
        })
    });
    let _ = Account { service: String::new(), username: String::new(), enabled: true };
}

fn wire_settings(ui: &AppWindow, app: &Shared) {
    let s = ui.global::<SettingsState>();

    let run = with(ui, app);
    s.on_category_selected(move |_| run(&|app, ui| settings_view::refresh(app, ui)));
    let run = with(ui, app);
    s.on_search_changed(move || run(&|app, ui| settings_view::refresh(app, ui)));
    let run = with(ui, app);
    s.on_show_advanced_changed(move |_| run(&|app, ui| settings_view::refresh(app, ui)));
    let run = with(ui, app);
    s.on_item_changed(move |id, value| run(&|app, ui| settings_view::changed(app, ui, &id, &value)));
    let run = with(ui, app);
    s.on_browse(move |id| run(&|app, ui| settings_view::browse(app, ui, &id)));
    let run = with(ui, app);
    s.on_reset_page(move || run(&|app, ui| settings_view::reset_page(app, ui)));
    let run = with(ui, app);
    s.on_copy_json(move || run(&|app, _| settings_view::copy_json(app)));

    let run = with(ui, app);
    s.on_theme_changed(move |mode| {
        run(&|app, ui| {
            app.settings.theme = match mode {
                1 => ThemeMode::Light,
                2 => ThemeMode::Dark,
                _ => ThemeMode::System,
            };
            app.save_settings();
            ui.invoke_apply_theme(mode);
            settings_view::refresh(app, ui);
        })
    });

    let run = with(ui, app);
    s.on_install_tool(move |name| {
        run(&|app, ui| {
            if app.tool_slot.is_some() {
                return;
            }
            let tool = if name == "ffmpeg" { Tool::Ffmpeg } else { Tool::YtDlp };
            let slot = Arc::new(ControlSlot::default());
            app.tool_slot = Some((tool, slot.clone()));
            let engine = app.manager.engine().clone();
            let sender = app.sender.clone();
            app.runtime.spawn(async move {
                let result = tools::install(tool, &engine, slot).await;
                let _ = sender.send(UiMsg::ToolInstalled { tool, result });
            });
            if !app.modules.is_enabled("video") {
                set_module(app, ui, "video", true);
            }
            app.refresh_tool_status(ui);
        })
    });
}

fn wire_extraction(ui: &AppWindow, app: &Shared) {
    let e = ui.global::<ExtractionState>();

    let run = with(ui, app);
    e.on_passwords_changed(move |text| {
        run(&|app, _| {
            app.settings.archive_passwords =
                text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect();
            app.save_settings();
        })
    });

    let run = with(ui, app);
    e.on_flag_changed(move |flag, on| {
        run(&|app, ui| {
            match flag.as_str() {
                "auto" => set_module(app, ui, "extraction", on),
                "delete" => app.settings.delete_archives = on,
                "passwords" => app.settings.set_option("try-passwords", on),
                _ => {}
            }
            app.save_settings();
        })
    });

    let run = with(ui, app);
    e.on_clear_finished(move || {
        run(&|app, ui| {
            app.extract_jobs.retain(|j| j.outcome.is_none());
            app.refresh_extraction(ui, true);
        })
    });

    let run = with(ui, app);
    e.on_open_folder(move |row| {
        run(&|app, _| {
            // Rows are shown newest first.
            let index = app.extract_jobs.len().checked_sub(1 + row.max(0) as usize);
            if let Some(job) = index.and_then(|i| app.extract_jobs.get(i)) {
                platform::reveal(&job.dir);
            }
        })
    });
}

/// Frameless window: drag, resize, minimize, maximize, close, speed box.
fn wire_window(ui: &AppWindow, app: &Shared) {
    let run = with(ui, app);
    ui.on_open_help(move || run(&|app, _| app.toast(t("Documentation coming soon"))));
    let run = with(ui, app);
    ui.on_open_mini(move || run(&|app, ui| crate::mini::open(app, ui)));

    let s = ui.global::<SettingsState>();
    let run = with(ui, app);
    s.on_rule_add(move || {
        run(&|app, ui| {
            crate::scheduler::add(&mut app.settings);
            app.save_settings();
            settings_view::refresh(app, ui);
        })
    });
    let run = with(ui, app);
    s.on_rule_remove(move |index| {
        run(&|app, ui| {
            crate::scheduler::remove(&mut app.settings, index.max(0) as usize);
            app.save_settings();
            settings_view::refresh(app, ui);
        })
    });
    let run = with(ui, app);
    s.on_rule_changed(move |index, field, value| {
        run(&|app, ui| {
            crate::scheduler::change(&mut app.settings, index.max(0) as usize, &field, &value);
            app.save_settings();
            // Text fields would lose focus on every key: only rebuild for choices.
            if field != "from" && field != "to" && field != "value" {
                settings_view::refresh(app, ui);
            }
        })
    });

    let weak = ui.as_weak();
    ui.on_window_drag(move || {
        if let Some(ui) = weak.upgrade() {
            crate::frame::drag(&ui);
        }
    });
    let weak = ui.as_weak();
    ui.on_window_resize(move |direction| {
        if let Some(ui) = weak.upgrade() {
            crate::frame::resize(&ui, direction);
        }
    });
    let weak = ui.as_weak();
    ui.on_window_minimize(move || {
        if let Some(ui) = weak.upgrade() {
            ui.window().set_minimized(true);
        }
    });
    let weak = ui.as_weak();
    ui.on_window_toggle_maximize(move || {
        if let Some(ui) = weak.upgrade() {
            let maximized = !ui.window().is_maximized();
            ui.window().set_maximized(maximized);
            ui.set_is_max(maximized);
        }
    });
    let run = with(ui, app);
    ui.on_window_close(move || {
        run(&|app, ui| {
            if app.tray.is_some() && app.settings.close_to_tray && !app.quitting {
                let _ = ui.hide();
            } else {
                let _ = slint::quit_event_loop();
            }
        })
    });

    let run = with(ui, app);
    ui.on_apply_speed_limit(move |text, unit| {
        run(&|app, ui| {
            let value: f64 = text.trim().replace(',', ".").parse().unwrap_or(0.0);
            let kib = if unit == 1 { value * 1024.0 } else { value };
            app.settings.speed_limit_kib = kib.max(0.0).round() as u64;
            if app.settings.speed_limit_kib > 0 && !app.modules.is_enabled("speed-limit") {
                let _ = app.modules.set_enabled("speed-limit", true);
            }
            app.apply_config();
            app.save_settings();
            settings_view::refresh(app, ui);
        })
    });
    let run = with(ui, app);
    ui.on_clear_speed_limit(move || {
        run(&|app, ui| {
            app.settings.speed_limit_kib = 0;
            ui.set_speed_input(SharedString::new());
            app.apply_config();
            app.save_settings();
            settings_view::refresh(app, ui);
        })
    });
    let run = with(ui, app);
    ui.on_pause_all(move || run(&|app, _| app.manager.pause_all()));
    let run = with(ui, app);
    ui.on_resume_all(move || run(&|app, _| app.manager.resume_all()));

    let run = with(ui, app);
    ui.on_captcha_answer(move |value| {
        run(&|app, ui| crate::captcha::answer(app, ui, Some(value.to_string())));
    });
    let run = with(ui, app);
    ui.on_captcha_skip(move || run(&|app, ui| crate::captcha::answer(app, ui, None)));
    let run = with(ui, app);
    ui.on_captcha_open_page(move || run(&|app, _| crate::captcha::open_page(app)));
    #[cfg(windows)]
    {
        let run = with(ui, app);
        ui.on_browser_cancel(move || run(&|app, ui| crate::browser::cancel(app, ui, false)));
        let run = with(ui, app);
        ui.on_browser_external(move || run(&|app, ui| crate::browser::cancel(app, ui, true)));
    }

    let run = with(ui, app);
    ui.on_input_confirm(move |value| {
        run(&|app, ui| {
            let Some((id, action)) = app.input_action.take() else { return };
            match action.as_str() {
                "rename" => {
                    if let Err(e) = app.manager.rename(id, &value) {
                        app.toast(e);
                    }
                }
                "file-limit" => {
                    let mb: f64 = value.trim().replace(',', ".").parse().unwrap_or(0.0);
                    app.manager.set_job_limit(id, (mb * 1_048_576.0).max(0.0) as u64);
                }
                _ => {}
            }
            app.refresh(ui);
        })
    });
}
