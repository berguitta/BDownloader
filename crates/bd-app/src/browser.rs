//! Browser window inside BDownloader for checks that only a browser can do
//! ("I am not a robot" boxes of reCAPTCHA, hCaptcha, Cloudflare Turnstile).
//!
//! Free mode does everything it can in the background. When a site insists
//! on such a check, its page opens here (WebView2, the engine of Edge) in
//! the same window. The user ticks the box and clicks the site's download
//! button; the file download is caught, handed back with the page's cookies,
//! and BDownloader's engine downloads it like any other file. BDownloader
//! never ticks the box itself.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use bd_core::free::{BrowserFn, BrowserRequest, BrowserResult, BROWSER_UA};
use slint::winit_030::WinitWindowAccessor;
use slint::ComponentHandle;
use tokio::sync::oneshot;
use url::Url;
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{NewWindowResponse, Rect, WebView, WebViewBuilder};

use crate::app::{App, UiMsg};
use crate::{platform, AppWindow};

type Reply = oneshot::Sender<Option<BrowserResult>>;

/// Geometry of the browser area, in logical pixels: it must match the
/// `BrowserPanel` card in app.slint.
const CARD_X: f32 = 32.0;
const CARD_TOP: f32 = 56.0;
const CARD_BOTTOM: f32 = 32.0;
const CARD_HEADER: f32 = 64.0;

struct Open {
    webview: WebView,
    request: BrowserRequest,
    reply: Reply,
    /// File link the page started to download.
    caught: Rc<RefCell<Option<String>>>,
    /// Same-site link the page tried to open in a new window.
    navigate: Rc<RefCell<Option<String>>>,
    bounds: (f32, f32),
}

#[derive(Default)]
pub struct Browser {
    open: Option<Open>,
    pending: VecDeque<(BrowserRequest, Reply)>,
}

/// The function free mode calls, from any thread.
pub fn browser_fn(sender: Sender<UiMsg>) -> BrowserFn {
    let sender = Arc::new(Mutex::new(sender));
    Arc::new(move |request| {
        let (reply, answer) = oneshot::channel();
        let sent = sender.lock().unwrap_or_else(|e| e.into_inner()).send(UiMsg::Browser(request, reply)).is_ok();
        Box::pin(async move {
            if !sent {
                return None;
            }
            answer.await.ok().flatten()
        })
    })
}

pub fn push(app: &mut App, ui: &AppWindow, request: BrowserRequest, reply: Reply) {
    let hoster = request.hoster.clone();
    app.browser.pending.push_back((request, reply));
    app.notify(false, &format!("{hoster}: browser check"), &crate::i18n::t("Finish the check in BDownloader"));
    if app.settings.flag("captcha-front", true) {
        let _ = ui.show();
        ui.window().set_minimized(false);
        ui.window().with_winit_window(|w| w.focus_window());
    }
    refresh(app, ui);
}

/// Opens the next page, hands caught downloads back, follows the window size.
pub fn refresh(app: &mut App, ui: &AppWindow) {
    let browser = &mut app.browser;
    if let Some(open) = &browser.open {
        if open.reply.is_closed() {
            // The download was cancelled meanwhile.
            close(browser, ui, None);
        }
    }
    let caught = browser.open.as_ref().and_then(|open| open.caught.borrow_mut().take());
    if let Some(link) = caught {
        let result = browser.open.as_ref().and_then(|open| handoff(open, &link));
        close(browser, ui, result);
    }
    if let Some(open) = &mut browser.open {
        let navigate = open.navigate.borrow_mut().take();
        if let Some(link) = navigate {
            let _ = open.webview.load_url(&link);
        } else {
            let size = logical_size(ui);
            if size != open.bounds {
                open.bounds = size;
                let _ = open.webview.set_bounds(bounds(size));
            }
        }
    }
    while browser.open.is_none() {
        let Some((request, reply)) = browser.pending.pop_front() else { break };
        if reply.is_closed() {
            continue;
        }
        match create(ui, &request) {
            Some((webview, caught, navigate)) => {
                ui.set_browser_hoster(request.hoster.as_str().into());
                ui.set_browser_open(true);
                browser.open = Some(Open { webview, request, reply, caught, navigate, bounds: logical_size(ui) });
            }
            None => {
                // No WebView2 (or headless): fall back to the user's browser.
                platform::open_url(request.page.as_str());
                let _ = reply.send(None);
            }
        }
    }
}

/// The user gave up, or prefers the external browser.
pub fn cancel(app: &mut App, ui: &AppWindow, external: bool) {
    if external {
        if let Some(open) = &app.browser.open {
            platform::open_url(open.request.page.as_str());
        }
    }
    close(&mut app.browser, ui, None);
    refresh(app, ui);
}

fn close(browser: &mut Browser, ui: &AppWindow, result: Option<BrowserResult>) {
    if let Some(open) = browser.open.take() {
        let _ = open.webview.set_visible(false);
        let _ = open.reply.send(result);
    }
    ui.set_browser_open(false);
}

/// The caught link with the cookies and user agent of the window.
fn handoff(open: &Open, link: &str) -> Option<BrowserResult> {
    let url = Url::parse(link).ok()?;
    let cookies: Vec<String> = open
        .webview
        .cookies_for_url(link)
        .unwrap_or_default()
        .iter()
        .map(|c| format!("{}={}", c.name(), c.value()))
        .collect();
    let mut headers = vec![("User-Agent".to_owned(), BROWSER_UA.to_owned())];
    if !cookies.is_empty() {
        headers.push(("Cookie".to_owned(), cookies.join("; ")));
    }
    if let Ok(page) = open.webview.url() {
        headers.push(("Referer".to_owned(), page));
    }
    tracing::info!(hoster = open.request.hoster, "browser window handed a download back");
    Some(BrowserResult { url, headers })
}

type Created = (WebView, Rc<RefCell<Option<String>>>, Rc<RefCell<Option<String>>>);

fn create(ui: &AppWindow, request: &BrowserRequest) -> Option<Created> {
    let caught: Rc<RefCell<Option<String>>> = Rc::default();
    let navigate: Rc<RefCell<Option<String>>> = Rc::default();
    let site = site_of(&request.page);
    let size = logical_size(ui);
    let (on_download, on_window) = (caught.clone(), navigate.clone());
    let built = ui.window().with_winit_window(|window| {
        WebViewBuilder::new()
            .with_url(request.page.as_str())
            .with_user_agent(BROWSER_UA)
            .with_document_title_changed_handler(|title| tracing::debug!(title, "browser window page"))
            .with_bounds(bounds(size))
            .with_download_started_handler(move |url, _path| {
                *on_download.borrow_mut() = Some(url);
                false // BDownloader downloads it, not the window.
            })
            .with_new_window_req_handler(move |url, _features| {
                // Ads open other sites in new windows: ignored. A link of the
                // hoster itself (FuckingFast's download) opens in place.
                if Url::parse(&url).ok().as_ref().map(site_of) == Some(site.clone()) {
                    *on_window.borrow_mut() = Some(url);
                }
                NewWindowResponse::Deny
            })
            .build_as_child(window)
    })?;
    match built {
        Ok(webview) => Some((webview, caught, navigate)),
        Err(e) => {
            tracing::warn!(error = %e, "cannot create the browser window");
            None
        }
    }
}

/// Last two labels of the host ("dl3.datanodes.to" → "datanodes.to").
fn site_of(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    let labels: Vec<&str> = host.rsplit('.').take(2).collect();
    labels.into_iter().rev().collect::<Vec<_>>().join(".")
}

fn logical_size(ui: &AppWindow) -> (f32, f32) {
    let window = ui.window();
    let size = window.size().to_logical(window.scale_factor());
    (size.width, size.height)
}

fn bounds((width, height): (f32, f32)) -> Rect {
    Rect {
        position: LogicalPosition::new(f64::from(CARD_X + 1.0), f64::from(CARD_TOP + CARD_HEADER)).into(),
        size: LogicalSize::new(
            f64::from((width - 2.0 * CARD_X - 2.0).max(100.0)),
            f64::from((height - CARD_TOP - CARD_HEADER - CARD_BOTTOM - 1.0).max(100.0)),
        )
        .into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_sites() {
        assert_eq!(site_of(&Url::parse("https://dl3.datanodes.to/x").unwrap()), "datanodes.to");
        assert_eq!(site_of(&Url::parse("https://fuckingfast.co/dl/a").unwrap()), "fuckingfast.co");
    }
}
