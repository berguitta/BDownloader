//! Small OS integrations: Explorer, clipboard, notifications.

use std::path::Path;
use std::process::Command;

/// Opens Explorer with the file selected, or the folder itself. When the
/// folder does not exist yet, the closest existing parent opens.
pub fn reveal(path: &Path) {
    let mut cmd = Command::new("explorer.exe");
    if path.is_file() {
        // Explorer needs `/select,"path"` as one raw argument: the default
        // quoting of the whole argument makes it ignore the path.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.raw_arg(format!("/select,\"{}\"", path.display()));
        }
        #[cfg(not(windows))]
        cmd.arg(path);
    } else {
        let mut dir = path.to_path_buf();
        while !dir.is_dir() {
            match dir.parent() {
                Some(parent) => dir = parent.to_path_buf(),
                None => break,
            }
        }
        cmd.arg(&dir);
    }
    if let Err(e) = cmd.spawn() {
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

/// Adds or removes BDownloader from the programs started with Windows
/// (current user only). It starts hidden in the tray.
pub fn set_launch_with_windows(on: bool) {
    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    let result = if on {
        let Ok(exe) = std::env::current_exe() else { return };
        Command::new("reg")
            .args([
                "add",
                KEY,
                "/v",
                "BDownloader",
                "/t",
                "REG_SZ",
                "/f",
                "/d",
                &format!("\"{}\" --minimized", exe.display()),
            ])
            .output()
    } else {
        Command::new("reg").args(["delete", KEY, "/v", "BDownloader", "/f"]).output()
    };
    if let Err(e) = result {
        tracing::warn!(error = %e, "cannot change the startup entry");
    }
}

/// Opens a web page in the default browser.
pub fn open_url(url: &str) {
    if let Err(e) = Command::new("explorer.exe").arg(url).spawn() {
        tracing::warn!(error = %e, "cannot open the browser");
    }
}

/// Puts the computer to sleep (used when the queue is finished).
pub fn sleep_computer() {
    let _ = Command::new("rundll32.exe").args(["powrprof.dll,SetSuspendState", "0,1,0"]).spawn();
}

/// Shuts the computer down in one minute, so it can still be cancelled with
/// `shutdown /a`.
pub fn shutdown_computer() {
    let _ = Command::new("shutdown").args(["/s", "/t", "60", "/c", "BDownloader: all downloads are finished"]).spawn();
}

/// Runs a helper program without a console window, in the background.
pub fn run_hidden(program: &Path, args: &[&str]) {
    let mut cmd = Command::new(program);
    cmd.args(args).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    if let Err(e) = cmd.spawn() {
        tracing::debug!(error = %e, program = %program.display(), "cannot run helper");
    }
}

/// Hibernates the computer (used when the queue is finished).
pub fn hibernate_computer() {
    let _ = Command::new("shutdown").args(["/h"]).spawn();
}

/// Anti-standby: keeps the computer (and optionally the screen) awake while
/// downloads or extractions run. Call with `false` to let it sleep again.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn keep_awake(on: bool, screen: bool) {
    use windows_sys::Win32::System::Power::{
        SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED,
    };
    let flags = if on {
        ES_CONTINUOUS | ES_SYSTEM_REQUIRED | if screen { ES_DISPLAY_REQUIRED } else { 0 }
    } else {
        ES_CONTINUOUS
    };
    // SAFETY: plain flags, no pointers; always valid to call.
    unsafe {
        SetThreadExecutionState(flags);
    }
}

#[cfg(not(windows))]
pub fn keep_awake(_on: bool, _screen: bool) {}
