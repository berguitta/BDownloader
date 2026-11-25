//! Small OS integrations: open files and folders, copy to the clipboard.

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
