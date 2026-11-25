//! External helper programs fetched on demand: yt-dlp for video sites and
//! ffmpeg for merging video/audio and converting to MP3. They live in
//! `%LOCALAPPDATA%\BDownloader\data\tools`, never in system folders.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bd_net::{DownloadRequest, Engine};
use directories::ProjectDirs;
use url::Url;

use crate::task::ControlSlot;

const YT_DLP_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";
const FFMPEG_URL: &str =
    "https://github.com/yt-dlp/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-win64-gpl.zip";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    YtDlp,
    Ffmpeg,
}

impl Tool {
    pub fn exe_name(self) -> &'static str {
        match self {
            Tool::YtDlp => "yt-dlp.exe",
            Tool::Ffmpeg => "ffmpeg.exe",
        }
    }
}

pub fn tools_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("BD_TOOLS").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    ProjectDirs::from("", "", "BDownloader").map(|d| d.data_local_dir().join("tools"))
}

/// Looks in the tools folder first, then on the PATH.
pub fn find(tool: Tool) -> Option<PathBuf> {
    let name = tool.exe_name();
    if let Some(path) = tools_dir().map(|d| d.join(name)).filter(|p| p.is_file()) {
        return Some(path);
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).map(|p| p.join(name)).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .find(|p| p.is_file())
}

/// Downloads a tool into the tools folder. Progress goes to `slot`.
pub async fn install(tool: Tool, engine: &Engine, slot: Arc<ControlSlot>) -> Result<PathBuf, String> {
    let dir = tools_dir().ok_or("no folder for tools")?;
    tokio::fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;
    let (url, file) = match tool {
        Tool::YtDlp => (YT_DLP_URL, "yt-dlp.exe.download"),
        Tool::Ffmpeg => (FFMPEG_URL, "ffmpeg.zip"),
    };
    let request = DownloadRequest::new(Url::parse(url).map_err(|e| e.to_string())?, &dir).file_name(file).resume(false);
    let task = engine.start(request);
    slot.set(Arc::new(task.control()));
    let downloaded = task.wait().await.map_err(|e| e.to_string())?;

    let target = dir.join(tool.exe_name());
    match tool {
        Tool::YtDlp => {
            tokio::fs::rename(&downloaded, &target).await.map_err(|e| e.to_string())?;
        }
        Tool::Ffmpeg => {
            let dir2 = dir.clone();
            let zip_path = downloaded.clone();
            tokio::task::spawn_blocking(move || unpack_ffmpeg(&zip_path, &dir2)).await.map_err(|e| e.to_string())??;
            let _ = tokio::fs::remove_file(&downloaded).await;
        }
    }
    Ok(target)
}

/// Copies `bin/ffmpeg.exe` and `bin/ffprobe.exe` out of the release zip.
fn unpack_ffmpeg(zip_path: &Path, dir: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut found = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().map(|n| n.into_owned()).unwrap_or_default();
        let Some(exe) = ["ffmpeg.exe", "ffprobe.exe"].into_iter().find(|exe| name.ends_with(&format!("bin/{exe}")))
        else {
            continue;
        };
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(exe), bytes).map_err(|e| e.to_string())?;
        found += 1;
    }
    if found == 0 {
        return Err("ffmpeg.exe not found in the download".to_owned());
    }
    Ok(())
}
