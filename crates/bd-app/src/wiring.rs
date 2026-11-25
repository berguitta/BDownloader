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
use crate::i18n::{self, t};
use crate::{platform, AccountsState, AppWindow, DownloadsState, GrabberState, SettingsState};

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
    let languages: Vec<SharedString> = i18n::LANGUAGES.iter().map(|(_, name)| SharedString::from(*name)).collect();
    s.set_languages(ModelRc::new(VecModel::from(languages)));
    s.set_language_index(i18n::index_of(&app.settings.language) as i32);
    s.set_download_dir(app.settings.download_dir.display().to_string().into());
    s.set_parallel(app.settings.max_parallel as i32);
    s.set_connections(app.settings.connections_per_download as i32);
    s.set_speed_limit_kib(app.settings.speed_limit_kib.min(i32::MAX as u64) as i32);
    s.set_start_on_launch(app.settings.start_on_launch);
    s.set_close_to_tray(app.settings.close_to_tray);
    s.set_minimize_to_tray(app.settings.minimize_to_tray);
    s.set_archive_passwords(app.settings.archive_passwords.join("\n").into());
    s.set_delete_archives(app.settings.delete_archives);
    s.set_config_path(app.settings_path.as_ref().map(|p| p.display().to_string()).unwrap_or_default().into());
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
            ui.global::<SettingsState>().set_parallel(value);
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
                "open-folder" => platform::reveal(job.path.as_deref().unwrap_or(&job.dest_dir)),
                "copy-link" => {
                    app.ignore_clipboard = Some(job.url.to_string());
                    if platform::copy_text(job.url.as_str()) {
                        app.toast(t("Link copied"));
                    }
                }
                "remove" | "delete" => {
                    app.manager.remove(job.id, action == "delete");
                    if app.selected == Some(job.id) {
                        app.selected = None;
                    }
                    app.toast(format!("− {}", job.name));
                }
                other => tracing::debug!(action = other, "unknown row action"),
            }
            app.refresh(ui);
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
            app.toast(format!("+{count}"));
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
            let specs = app.grabber.take_selected(&app.settings.download_dir);
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
            // Enable the matching module so the account is used right away.
            if !app.modules.is_enabled(service.module) {
                set_module(app, ui, service.module, true);
            }
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
    s.on_theme_changed(move |mode| {
        run(&|app, ui| {
            app.settings.theme = match mode {
                1 => ThemeMode::Light,
                2 => ThemeMode::Dark,
                _ => ThemeMode::System,
            };
            app.save_settings();
            ui.invoke_apply_theme(mode);
        })
    });

    let run = with(ui, app);
    s.on_language_changed(move |index| {
        run(&|app, ui| {
            app.settings.language = i18n::select(index.max(0) as usize).to_owned();
            app.save_settings();
            app.refresh_accounts(ui);
            app.refresh_grabber(ui, true);
        })
    });

    let run = with(ui, app);
    s.on_download_dir_changed(move |dir| {
        run(&|app, _| {
            let dir = dir.trim();
            if dir.is_empty() {
                return;
            }
            app.settings.download_dir = PathBuf::from(dir);
            app.apply_config();
            app.save_settings();
        })
    });

    let run = with(ui, app);
    s.on_parallel_changed(move |value| {
        run(&|app, ui| {
            app.settings.max_parallel = value.clamp(1, 50) as usize;
            app.apply_config();
            app.save_settings();
            ui.global::<DownloadsState>().set_parallel(value);
        })
    });

    let run = with(ui, app);
    s.on_connections_changed(move |value| {
        run(&|app, _| {
            app.settings.connections_per_download = value.clamp(1, 32) as usize;
            app.apply_config();
            app.save_settings();
        })
    });

    let run = with(ui, app);
    s.on_speed_limit_changed(move |value| {
        run(&|app, _| {
            app.settings.speed_limit_kib = value.max(0) as u64;
            if let Some(tray) = &app.tray {
                tray.set_speed(app.settings.speed_limit_kib);
            }
            app.apply_config();
            app.save_settings();
        })
    });

    let run = with(ui, app);
    s.on_flag_changed(move |flag, on| {
        run(&|app, _| {
            match flag.as_str() {
                "start-on-launch" => app.settings.start_on_launch = on,
                "close-to-tray" => app.settings.close_to_tray = on,
                "minimize-to-tray" => app.settings.minimize_to_tray = on,
                "delete-archives" => app.settings.delete_archives = on,
                _ => {}
            }
            app.save_settings();
        })
    });

    let run = with(ui, app);
    s.on_passwords_changed(move |text| {
        run(&|app, _| {
            app.settings.archive_passwords =
                text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect();
            app.save_settings();
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

    let run = with(ui, app);
    s.on_module_toggled(move |id, on| run(&|app, ui| set_module(app, ui, &id, on)));
}
