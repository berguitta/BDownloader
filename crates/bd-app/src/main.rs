//! BDownloader desktop application.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod app;
#[cfg(windows)]
mod browser;
mod captcha;
mod favicon;
mod format;
mod frame;
mod i18n;
mod mini;
mod platform;
mod scheduler;
mod schema;
mod settings_view;
#[cfg(feature = "snapshot")]
mod snapshot;
mod tray;
mod wiring;

use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::Result;
use bd_core::accounts::{AccountStore, SystemKeyring};
use bd_core::grabber::Grabber;
use bd_core::hosters::HosterResolver;
use bd_core::manager::load_records;
use bd_core::{DownloadManager, ModuleRegistry, Settings};
use bd_net::Engine;
use slint::{CloseRequestResponse, ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

use crate::app::{App, HISTORY_SECONDS};
use crate::tray::{Tray, TrayAction};

slint::include_modules!();

const TICK: Duration = Duration::from_millis(250);

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("BD_LOG").unwrap_or_else(|_| "info".into()))
        .init();

    let settings_path = Settings::default_path();
    let settings = settings_path.as_deref().map(Settings::load_from).unwrap_or_default();
    let modules = ModuleRegistry::builtin().with_states(&settings.modules);
    let accounts = AccountStore::load(AccountStore::default_path(), Box::new(SystemKeyring));

    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().thread_name("bd-worker").build()?;
    let engine = Engine::new()?;
    let manager =
        DownloadManager::new(engine.clone(), App::manager_config(&settings, &modules), runtime.handle().clone());
    let resolver = HosterResolver::new(engine.client().clone());
    manager.set_resolver(resolver.clone());
    let grabber = Grabber::new(engine, runtime.handle().clone());

    #[cfg(feature = "snapshot")]
    let snapshot = snapshot::Snapshot::from_env();
    #[cfg(feature = "snapshot")]
    if let Some(snapshot) = &snapshot {
        snapshot.install()?;
    }

    let ui = AppWindow::new()?;
    i18n::select(i18n::index_of(&settings.language));
    let rows = Rc::new(VecModel::<JobRow>::default());
    ui.global::<DownloadsState>().set_rows(ModelRc::from(rows.clone()));
    let grab_rows = Rc::new(VecModel::<GrabRow>::default());
    ui.global::<GrabberState>().set_rows(ModelRc::from(grab_rows.clone()));

    let (sender, messages) = std::sync::mpsc::channel();
    let queue_path = Settings::queue_path();
    let tray = if modules.is_enabled("tray") {
        match Tray::new(modules.is_enabled("clipboard")) {
            Ok(tray) => Some(tray),
            Err(e) => {
                tracing::warn!(error = %e, "cannot create the tray icon");
                None
            }
        }
    } else {
        None
    };
    let start_on_launch = settings.start_on_launch;

    let app = Rc::new(RefCell::new(App {
        runtime: runtime.handle().clone(),
        events: manager.take_events(),
        manager: manager.clone(),
        grabber,
        resolver,
        accounts,
        account_status: Vec::new(),
        settings,
        settings_path,
        queue_path: queue_path.clone(),
        modules,
        rows,
        row_ids: Vec::new(),
        jobs: Vec::new(),
        selected: None,
        history: VecDeque::from(vec![0.0; HISTORY_SECONDS]),
        last_history: Instant::now(),
        toast: None,
        grab_rows,
        grab_revision: u64::MAX,
        closed_packages: HashSet::new(),
        saved_revision: 0,
        last_save: Instant::now(),
        messages,
        sender,
        tray,
        clipboard_seq: platform::clipboard_sequence(),
        ignore_clipboard: None,
        clicknload: None,
        clicknload_status: String::new(),
        tool_slot: None,
        quitting: false,
        favicons: Default::default(),
        extract_jobs: Vec::new(),
        extract_revision: 1,
        input_action: None,
        was_busy: false,
        schedule: Default::default(),
        torrent: None,
        last_slow_tick: Instant::now(),
        last_account_check: Instant::now(),
        awake: false,
        mini: None,
        captchas: Default::default(),
        #[cfg(windows)]
        browser: Default::default(),
    }));
    {
        let mut a = app.borrow_mut();
        a.apply_config();
        a.sync_clicknload();
        if let Some(path) = &queue_path {
            manager.restore(load_records(path), start_on_launch);
        }
        a.saved_revision = manager.revision();
    }

    wiring::init_ui(&ui, &app);
    wiring::wire_all(&ui, &app);

    // Captchas in free mode: code and image captchas are typed in the app;
    // "I am not a robot" widgets can only be done in the browser.
    {
        let a = app.borrow();
        a.resolver.set_captcha_ask(captcha::ask_fn(a.sender.clone()));
        #[cfg(windows)]
        a.resolver.set_browser(browser::browser_fn(a.sender.clone()));
        a.resolver.set_captcha_handler(std::sync::Arc::new(move |page, hoster, mode| {
            if mode == bd_core::hosters::CaptchaMode::Browser {
                platform::open_url(page.as_str());
            }
            platform::notify(&format!("{hoster}: captcha"), page.as_str());
        }));
        if a.settings.flag("update-yt-dlp", true) {
            if let Some(path) = bd_core::tools::find(bd_core::tools::Tool::YtDlp) {
                platform::run_hidden(&path, &["-U"]);
            }
        }
    }
    // Development aid: BD_BROWSER_TEST=<page> opens the browser window on it.
    #[cfg(windows)]
    if let Ok(page) = std::env::var("BD_BROWSER_TEST") {
        if let Ok(page) = url::Url::parse(&page) {
            let (weak, a) = (ui.as_weak(), app.clone());
            Timer::single_shot(Duration::from_millis(1500), move || {
                let Some(ui) = weak.upgrade() else { return };
                let (reply, answer) = tokio::sync::oneshot::channel();
                let request = bd_core::free::BrowserRequest { hoster: "Test".into(), page: page.clone() };
                let mut app = a.borrow_mut();
                app.runtime.spawn(async move {
                    let result = answer.await;
                    tracing::info!(result = ?result, "browser test finished");
                });
                browser::push(&mut app, &ui, request, reply);
            });
        }
    }
    let start_hidden = std::env::args().any(|a| a == "--minimized") && app.borrow().tray.is_some();

    #[cfg(feature = "snapshot")]
    if let Some(snapshot) = &snapshot {
        apply_snapshot_options(&ui, &app, snapshot);
    }

    // Apply the saved theme now that the settings global is filled.
    ui.invoke_apply_theme(ui.global::<SettingsState>().get_theme_mode());

    // Closing hides to the tray when the user wants it, otherwise quits.
    {
        let app = app.clone();
        ui.window().on_close_requested(move || {
            let a = app.borrow();
            if a.tray.is_some() && a.settings.close_to_tray && !a.quitting {
                CloseRequestResponse::HideWindow
            } else {
                let _ = slint::quit_event_loop();
                CloseRequestResponse::HideWindow
            }
        });
    }

    let timer = Timer::default();
    {
        let weak = ui.as_weak();
        let app = app.clone();
        timer.start(TimerMode::Repeated, TICK, move || {
            let Some(ui) = weak.upgrade() else { return };
            let mut a = app.borrow_mut();
            a.check_clipboard();
            let actions = a.tray.as_ref().map(|t| t.poll()).unwrap_or_default();
            for action in actions {
                handle_tray(&mut a, &ui, action);
            }
            ui.set_is_max(ui.window().is_maximized());
            if a.settings.minimize_to_tray && a.tray.is_some() && ui.window().is_minimized() {
                ui.window().set_minimized(false);
                let _ = ui.hide();
            }
            a.refresh(&ui);
        });
    }
    app.borrow_mut().refresh(&ui);

    if !start_hidden {
        ui.show()?;
    }
    slint::run_event_loop_until_quit()?;

    // Save progress of running downloads so they resume next time.
    runtime.block_on(manager.shutdown());
    let mut a = app.borrow_mut();
    a.save_queue();
    a.save_settings();
    Ok(())
}

fn handle_tray(app: &mut App, ui: &AppWindow, action: TrayAction) {
    match action {
        TrayAction::Show => {
            let _ = ui.show();
            ui.window().set_minimized(false);
        }
        TrayAction::AddFromClipboard => {
            if let Some(text) = platform::clipboard_text() {
                let links = bd_core::links::extract_links(&text);
                let added = app.add_links(links, "Clipboard", false);
                app.toast(format!("+{added}"));
            }
            let _ = ui.show();
            ui.set_page(1);
        }
        TrayAction::ToggleClipboard(on) => wiring::set_module(app, ui, "clipboard", on),
        TrayAction::PauseAll => app.manager.pause_all(),
        TrayAction::ResumeAll => app.manager.resume_all(),
        TrayAction::SpeedLimit => {
            let _ = ui.show();
            ui.window().set_minimized(false);
            ui.invoke_show_speed_popup();
        }
        TrayAction::Quit => {
            app.quitting = true;
            let _ = slint::quit_event_loop();
        }
    }
}

#[cfg(feature = "snapshot")]
fn apply_snapshot_options(ui: &AppWindow, app: &Rc<RefCell<App>>, snapshot: &snapshot::Snapshot) {
    ui.set_custom_frame(false);
    if let Ok(cat) = std::env::var("BD_SNAPSHOT_CATEGORY") {
        ui.global::<SettingsState>().set_category(cat.parse().unwrap_or(0));
        settings_view::refresh(&app.borrow(), ui);
    }
    if let Ok(search) = std::env::var("BD_SNAPSHOT_SEARCH") {
        ui.global::<SettingsState>().set_search(search.into());
        settings_view::refresh(&app.borrow(), ui);
    }
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
        let (weak, a) = (ui.as_weak(), app.clone());
        Timer::single_shot(Duration::from_millis(1500), move || {
            if let Some(ui) = weak.upgrade() {
                let mut app = a.borrow_mut();
                app.selected = app.job_at(row).map(|j| j.id);
                app.refresh(&ui);
            }
        });
    }
    if std::env::var("BD_SNAPSHOT_CAPTCHA").is_ok() {
        // Demo XFileSharing code captcha, to check the dialog.
        let (reply, _answer) = tokio::sync::oneshot::channel();
        std::mem::forget(_answer);
        let request = bd_core::free::CaptchaRequest {
            hoster: "Filekeeper".into(),
            page: url::Url::parse("https://filekeeper.net/abc").expect("static url"),
            challenge: bd_core::free::Challenge::Placed(vec![(6, '4'), (22, '8'), (38, '1'), (54, '6')]),
            timeout: Duration::from_secs(120),
        };
        captcha::push(&mut app.borrow_mut(), ui, request, reply);
    }
    if let Ok(lang) = std::env::var("BD_SNAPSHOT_LANG") {
        let index = i18n::index_of(&lang);
        i18n::select(index);
        app.borrow_mut().settings.language = lang.clone();
        settings_view::refresh(&app.borrow(), ui);
    }
    if std::env::var("BD_SNAPSHOT_START").is_ok() {
        let (weak, a) = (ui.as_weak(), app.clone());
        Timer::single_shot(Duration::from_millis(4000), move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<GrabberState>().invoke_start_selected();
                a.borrow_mut().refresh(&ui);
            }
        });
    }
}
