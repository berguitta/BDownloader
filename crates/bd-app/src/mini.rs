//! Mini mode: a small always-on-top window in a corner of the desktop that
//! shows the speed and the current download.

use slint::winit_030::winit::dpi::PhysicalPosition;
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, Weak};

use crate::app::App;
use crate::{format, AppWindow, MiniWindow};

pub fn open(app: &mut App, ui: &AppWindow) {
    if app.mini.is_none() {
        let Ok(mini) = MiniWindow::new() else { return };
        wire(&mini, ui.as_weak());
        app.mini = Some(mini);
    }
    let Some(mini) = &app.mini else { return };
    refresh(app, mini);
    let _ = mini.show();
    place(mini, app.settings.text("mini-corner", "top-right"));
    let _ = ui.hide();
}

fn wire(mini: &MiniWindow, ui: Weak<AppWindow>) {
    let (main, me) = (ui.clone(), mini.as_weak());
    mini.on_restore(move || {
        if let (Some(ui), Some(me)) = (main.upgrade(), me.upgrade()) {
            let _ = me.hide();
            let _ = ui.show();
        }
    });
    let main = ui.clone();
    mini.on_pause_all(move || {
        if let Some(ui) = main.upgrade() {
            ui.invoke_pause_all();
        }
    });
    let main = ui.clone();
    mini.on_resume_all(move || {
        if let Some(ui) = main.upgrade() {
            ui.invoke_resume_all();
        }
    });
    let (main, me) = (ui, mini.as_weak());
    mini.on_close_mini(move || {
        if let (Some(ui), Some(me)) = (main.upgrade(), me.upgrade()) {
            let _ = me.hide();
            let _ = ui.show();
        }
    });
    let me = mini.as_weak();
    mini.on_drag(move || {
        if let Some(me) = me.upgrade() {
            me.window().with_winit_window(|w| {
                let _ = w.drag_window();
            });
        }
    });
}

/// Puts the mini window in a corner of the screen it is on.
fn place(mini: &MiniWindow, corner: &str) {
    mini.window().with_winit_window(|w| {
        let Some(monitor) = w.current_monitor().or_else(|| w.primary_monitor()) else { return };
        let (screen, origin, size) = (monitor.size(), monitor.position(), w.outer_size());
        let margin = (16.0 * monitor.scale_factor()) as i32;
        let taskbar = (56.0 * monitor.scale_factor()) as i32;
        let right = origin.x + screen.width as i32 - size.width as i32 - margin;
        let bottom = origin.y + screen.height as i32 - size.height as i32 - taskbar;
        let (x, y) = match corner {
            "top-left" => (origin.x + margin, origin.y + margin),
            "bottom-left" => (origin.x + margin, bottom),
            "bottom-right" => (right, bottom),
            _ => (right, origin.y + margin),
        };
        w.set_outer_position(PhysicalPosition::new(x, y));
    });
}

pub fn refresh(app: &App, mini: &MiniWindow) {
    let active: Vec<_> = app.jobs.iter().filter(|j| j.state == bd_core::JobState::Downloading).collect();
    let queued = app.jobs.iter().filter(|j| j.state == bd_core::JobState::Queued).count();
    let speed: f64 = active.iter().map(|j| j.speed).sum::<f64>() + 0.0;
    mini.set_speed(if speed >= 1.0 { format::speed(speed) } else { "0 B/s".to_owned() }.into());
    mini.set_summary(format!("{} ↓ · {} …", active.len(), queued).into());
    match active.first() {
        Some(job) => {
            let fraction =
                job.total.map(|t| if t == 0 { 1.0 } else { job.downloaded as f64 / t as f64 }).unwrap_or(0.0);
            mini.set_current(format!("{} · {:.0}%", job.name, fraction * 100.0).into());
            mini.set_progress(fraction as f32);
        }
        None => {
            mini.set_current(crate::i18n::t("Nothing is downloading").into());
            mini.set_progress(0.0);
        }
    }
    mini.set_paused(active.is_empty() && app.jobs.iter().any(|j| j.state == bd_core::JobState::Paused));
}
