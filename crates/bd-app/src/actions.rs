//! Actions started from buttons and modules: reconnect, event scripts.

use std::sync::Arc;

use bd_core::reconnect::Method;

use crate::app::{App, UiMsg};
use crate::AppWindow;

/// "Button" options in Settings.
pub fn button(app: &mut App, _ui: &AppWindow, id: &str) {
    match id {
        "reconnect-now" => reconnect(app),
        "event-test" => send_event(app, "test", "BDownloader", "", "", true),
        _ => {}
    }
}

pub fn reconnect_method(app: &App) -> Method {
    match app.settings.text("reconnect-method", "upnp") {
        "script" => Method::Script(app.settings.text("reconnect-script", "").to_owned()),
        _ => Method::Upnp,
    }
}

/// Asks for a new IP in the background and reports the result.
pub fn reconnect(app: &mut App) {
    let method = reconnect_method(app);
    let client = app.manager.engine().client().clone();
    let sender = app.sender.clone();
    app.toast("Reconnecting…");
    app.runtime.spawn(async move {
        let text = match bd_core::reconnect::reconnect(&method, &client).await {
            Ok(done) => format!("Reconnect: {done}"),
            Err(e) => format!("Reconnect failed: {e}"),
        };
        let _ = sender.send(UiMsg::Toast(text));
    });
}

/// Hook given to the resolver: reconnect before a long free-mode wait.
pub fn reconnect_hook(app: &App) -> Option<bd_core::free::Reconnect> {
    if !app.modules.is_enabled("reconnect") {
        return None;
    }
    let method = reconnect_method(app);
    let client = app.manager.engine().client().clone();
    let run: bd_core::free::ReconnectFn = Arc::new(move || {
        let (method, client) = (method.clone(), client.clone());
        Box::pin(async move { bd_core::reconnect::reconnect(&method, &client).await.is_ok() })
    });
    Some(bd_core::free::Reconnect { run, after_minutes: app.settings.number("reconnect-after", 5).max(1) as u64 })
}

/// Event scripts module: Discord, Telegram and a custom command.
pub fn send_event(app: &App, event: &str, name: &str, path: &str, package: &str, force: bool) {
    if !force {
        if !app.modules.is_enabled("event-scripts") {
            return;
        }
        let toggle = match event {
            "finished" => "event-finished",
            "package" => "event-package",
            "failed" => "event-failed",
            "extracted" => "event-extracted",
            "queue-done" => "event-queue-done",
            _ => return,
        };
        let default = event != "finished";
        if !app.settings.flag(toggle, default) {
            return;
        }
    }
    let message = match event {
        "finished" => format!("✅ Downloaded: {name}"),
        "package" => format!("📦 Package finished: {name}"),
        "failed" => format!("❌ Failed: {name}"),
        "extracted" => format!("🗜️ Extracted: {name}"),
        "queue-done" => "🏁 All downloads are finished".to_owned(),
        _ => "👋 Test message from BDownloader".to_owned(),
    };
    let discord = app.settings.text("discord-webhook", "").trim().to_owned();
    let telegram = (
        app.settings.text("telegram-token", "").trim().to_owned(),
        app.settings.text("telegram-chat", "").trim().to_owned(),
    );
    let command = app.settings.text("script-command", "").trim().to_owned();
    let client = app.manager.engine().client().clone();
    let sender = app.sender.clone();
    let (event, name, path, package) = (event.to_owned(), name.to_owned(), path.to_owned(), package.to_owned());
    app.runtime.spawn(async move {
        let mut errors = Vec::new();
        if discord.starts_with("https://") {
            let sent = client.post(&discord).json(&serde_json::json!({ "content": message })).send().await;
            if !sent.map(|r| r.status().is_success()).unwrap_or(false) {
                errors.push("Discord");
            }
        }
        if !telegram.0.is_empty() && !telegram.1.is_empty() {
            let url = format!("https://api.telegram.org/bot{}/sendMessage", telegram.0);
            let sent =
                client.post(url).form(&[("chat_id", telegram.1.as_str()), ("text", message.as_str())]).send().await;
            if !sent.map(|r| r.status().is_success()).unwrap_or(false) {
                errors.push("Telegram");
            }
        }
        if !command.is_empty() {
            let line = command
                .replace("{event}", &event)
                .replace("{name}", &name)
                .replace("{path}", &path)
                .replace("{package}", &package);
            let mut cmd = tokio::process::Command::new("cmd");
            cmd.args(["/C", &line]);
            #[cfg(windows)]
            cmd.creation_flags(0x0800_0000);
            if cmd.status().await.map(|s| !s.success()).unwrap_or(true) {
                errors.push("command");
            }
        }
        if event == "test" {
            let text =
                if errors.is_empty() { "Test sent".to_owned() } else { format!("Test failed: {}", errors.join(", ")) };
            let _ = sender.send(UiMsg::Toast(text));
        }
    });
}
