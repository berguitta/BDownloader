//! Captchas typed by the user inside the app.
//!
//! Free mode runs in the background; when a site asks for a code or an
//! image captcha, the request comes here through [`UiMsg::Captcha`]. The
//! dialog shows what the site shows, the user types the answer, and the
//! download goes on by itself. BDownloader never reads the captcha.

use std::collections::VecDeque;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bd_core::free::{CaptchaFn, CaptchaRequest, Challenge};
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};
use tokio::sync::oneshot;

use crate::app::{App, UiMsg};
use crate::i18n::t;
use crate::{platform, AppWindow, CaptchaChar};

/// A captcha waiting for the user's answer.
pub struct Pending {
    pub request: CaptchaRequest,
    pub reply: oneshot::Sender<Option<String>>,
    pub since: Instant,
}

#[derive(Default)]
pub struct Queue {
    pub pending: VecDeque<Pending>,
    /// Whether the first pending captcha is the one shown in the dialog.
    shown: bool,
}

/// The function free mode calls to ask the user, from any thread.
pub fn ask_fn(sender: Sender<UiMsg>) -> CaptchaFn {
    let sender = Arc::new(Mutex::new(sender));
    Arc::new(move |request| {
        let (reply, answer) = oneshot::channel();
        let sent = sender.lock().unwrap_or_else(|e| e.into_inner()).send(UiMsg::Captcha(request, reply)).is_ok();
        Box::pin(async move {
            if !sent {
                return None;
            }
            answer.await.ok().flatten()
        })
    })
}

pub fn push(app: &mut App, ui: &AppWindow, request: CaptchaRequest, reply: oneshot::Sender<Option<String>>) {
    let hoster = request.hoster.clone();
    app.captchas.pending.push_back(Pending { request, reply, since: Instant::now() });
    app.notify(false, &format!("{hoster}: captcha"), &t("Type the code to continue the download"));
    if app.settings.flag("captcha-front", true) {
        let _ = ui.show();
        ui.window().set_minimized(false);
        ui.window().with_winit_window(|w| w.focus_window());
    }
    refresh(app, ui);
}

/// Drops answered, cancelled or expired captchas and shows the next one.
pub fn refresh(app: &mut App, ui: &AppWindow) {
    let queue = &mut app.captchas;
    let before = queue.pending.len();
    queue.pending.retain(|p| !p.reply.is_closed() && p.since.elapsed() < p.request.timeout);
    if queue.pending.len() != before {
        queue.shown = false;
    }
    let Some(front) = queue.pending.front() else {
        if ui.get_captcha_open() {
            ui.set_captcha_open(false);
        }
        return;
    };
    let left = front.request.timeout.saturating_sub(front.since.elapsed()).as_secs() as i32;
    ui.set_captcha_seconds(left);
    ui.set_captcha_waiting(queue.pending.len() as i32);
    if queue.shown {
        return;
    }
    queue.shown = true;
    ui.set_captcha_hoster(front.request.hoster.as_str().into());
    ui.set_captcha_value(SharedString::new());
    match &front.request.challenge {
        Challenge::Image(bytes) => {
            let picture = decode(bytes);
            ui.set_captcha_has_picture(picture.is_some());
            ui.set_captcha_picture(picture.unwrap_or_default());
            ui.set_captcha_chars(ModelRc::default());
        }
        Challenge::Placed(chars) => {
            let model: Vec<CaptchaChar> =
                chars.iter().map(|(x, ch)| CaptchaChar { x: *x as f32, ch: ch.to_string().into() }).collect();
            ui.set_captcha_has_picture(false);
            ui.set_captcha_chars(ModelRc::new(VecModel::from(model)));
        }
    }
    ui.set_captcha_open(true);
}

/// The user answered (Some) or skipped (None) the captcha on screen.
pub fn answer(app: &mut App, ui: &AppWindow, code: Option<String>) {
    if let Some(front) = app.captchas.pending.pop_front() {
        let _ = front.reply.send(code.filter(|c| !c.trim().is_empty()));
    }
    app.captchas.shown = false;
    refresh(app, ui);
}

pub fn open_page(app: &App) {
    if let Some(front) = app.captchas.pending.front() {
        platform::open_url(front.request.page.as_str());
    }
}

fn decode(bytes: &[u8]) -> Option<Image> {
    let rgba = image::load_from_memory(bytes).ok()?.to_rgba8();
    let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height());
    Some(Image::from_rgba8(buffer))
}
