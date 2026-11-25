//! Small OS integrations: Explorer, clipboard, notifications.

use std::path::Path;
use std::process::Command;

/// Opens Explorer with the file selected, or the folder itself.
pub fn reveal(path: &Path) {
    let result = if path.is_file() {
        Command::new("explorer.exe").arg(format!("/select,{}", path.display())).spawn()
    } else {
        Command::new("explorer.exe").arg(path).spawn()
    };
    if let Err(e) = result {
        tracing::warn!(error = %e, path = %path.display(), "cannot open Explorer");
    }
}

/// Opens a file with its default application.
pub fn open(path: &Path) {
    if let Err(e) = Command::new("explorer.exe").arg(path).spawn() {
        tracing::warn!(error = %e, path = %path.display(), "cannot open file");
    }
}

pub fn copy_text(text: &str) -> bool {
    match arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_owned())) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(error = %e, "cannot write to the clipboard");
            false
        }
    }
}

pub fn clipboard_text() -> Option<String> {
    arboard::Clipboard::new().and_then(|mut c| c.get_text()).ok()
}

/// Number Windows increments on every clipboard change. Comparing it is
/// enough to react to changes without reading the content every time.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn clipboard_sequence() -> u32 {
    // SAFETY: no arguments, no pointers; documented as always safe to call.
    unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() }
}

#[cfg(not(windows))]
pub fn clipboard_sequence() -> u32 {
    0
}

/// Shows a Windows notification. Never blocks the UI.
pub fn notify(title: &str, body: &str) {
    let (title, body) = (title.to_owned(), body.to_owned());
    std::thread::spawn(move || {
        if let Err(e) = notify_rust::Notification::new().appname("BDownloader").summary(&title).body(&body).show() {
            tracing::debug!(error = %e, "notification failed");
        }
    });
}
