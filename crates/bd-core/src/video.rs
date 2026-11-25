//! Video sites through yt-dlp: list the available formats, then download the
//! one the user picked (video with audio, a maximum height, or audio only).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use bd_net::{Progress, SegmentProgress};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::task::{ControlSlot, RunError, RunResult, TaskControl};

/// Format chosen for a video link.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VideoFormat {
    /// Best video and best audio, merged (needs ffmpeg, else best single file).
    Best,
    /// Best quality up to this height (1080, 720…).
    MaxHeight { height: u32 },
    /// Audio only, in its original format (m4a or webm/opus), no conversion.
    AudioBest,
    /// Audio only converted to MP3 (needs ffmpeg).
    AudioMp3,
}

impl VideoFormat {
    pub fn label(&self) -> String {
        match self {
            VideoFormat::Best => "Best quality (video + audio)".to_owned(),
            VideoFormat::MaxHeight { height } => format!("{}p video + audio", height),
            VideoFormat::AudioBest => "Audio only (original, M4A/Opus)".to_owned(),
            VideoFormat::AudioMp3 => "Audio only (MP3)".to_owned(),
        }
    }

    pub fn needs_ffmpeg(&self) -> bool {
        matches!(self, VideoFormat::AudioMp3)
    }

    fn args(&self, ffmpeg: bool) -> Vec<String> {
        let s = |v: &str| v.to_owned();
        match self {
            VideoFormat::Best if ffmpeg => vec![s("-f"), s("bv*+ba/b")],
            VideoFormat::Best => vec![s("-f"), s("b")],
            VideoFormat::MaxHeight { height } if ffmpeg => {
                vec![s("-f"), format!("bv*[height<={height}]+ba/b[height<={height}]/b")]
            }
            VideoFormat::MaxHeight { height } => vec![s("-f"), format!("b[height<={height}]/b")],
            VideoFormat::AudioBest => vec![s("-f"), s("ba[ext=m4a]/ba")],
            VideoFormat::AudioMp3 => {
                vec![s("-f"), s("ba"), s("-x"), s("--audio-format"), s("mp3"), s("--audio-quality"), s("0")]
            }
        }
    }
}

/// One entry of the format picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoChoice {
    pub format: VideoFormat,
    pub label: String,
    /// Estimated size in bytes when yt-dlp knows it.
    pub size: Option<u64>,
    /// False when the choice needs ffmpeg and it is not installed.
    pub available: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoInfo {
    pub title: String,
    pub uploader: Option<String>,
    pub duration: Option<u64>,
    pub choices: Vec<VideoChoice>,
}

const VIDEO_HOSTS: &[&str] = &[
    "youtube.com",
    "youtu.be",
    "youtube-nocookie.com",
    "vimeo.com",
    "dailymotion.com",
    "twitch.tv",
    "tiktok.com",
    "soundcloud.com",
    "bandcamp.com",
    "x.com",
    "twitter.com",
    "instagram.com",
    "facebook.com",
    "reddit.com",
    "bilibili.com",
    "rumble.com",
    "odysee.com",
];

/// Whether the link points to a site handled by yt-dlp rather than plain HTTP.
pub fn is_video_url(url: &Url) -> bool {
    let Some(host) = url.host_str() else { return false };
    let host = host.to_ascii_lowercase();
    VIDEO_HOSTS.iter().any(|h| host == *h || host.ends_with(&format!(".{h}")))
}

/// Builds the format picker from `yt-dlp -J` output.
pub fn parse_info(json: &Value, ffmpeg: bool) -> VideoInfo {
    let formats = json["formats"].as_array().cloned().unwrap_or_default();
    let size_of = |f: &Value| f["filesize"].as_u64().or_else(|| f["filesize_approx"].as_u64());
    let is_video = |f: &Value| f["vcodec"].as_str().is_some_and(|c| c != "none");
    let is_audio = |f: &Value| f["acodec"].as_str().is_some_and(|c| c != "none");

    let best_audio = formats
        .iter()
        .filter(|f| is_audio(f) && !is_video(f))
        .max_by_key(|f| (f["abr"].as_f64().unwrap_or(0.0) * 1000.0) as u64);
    let audio_size = best_audio.and_then(size_of);

    let mut heights: Vec<u32> =
        formats.iter().filter(|f| is_video(f)).filter_map(|f| f["height"].as_u64()).map(|h| h as u32).collect();
    heights.sort_unstable_by(|a, b| b.cmp(a));
    heights.dedup();

    let video_size_up_to = |max: u32| {
        formats
            .iter()
            .filter(|f| is_video(f) && f["height"].as_u64().is_some_and(|h| h as u32 <= max))
            .max_by_key(|f| (f["height"].as_u64().unwrap_or(0), (f["tbr"].as_f64().unwrap_or(0.0) * 100.0) as u64))
            .and_then(size_of)
            .map(|v| v + audio_size.unwrap_or(0))
    };

    let mut choices = vec![VideoChoice {
        format: VideoFormat::Best,
        label: VideoFormat::Best.label(),
        size: heights.first().and_then(|h| video_size_up_to(*h)),
        available: true,
    }];
    for height in heights.iter().copied().filter(|h| [2160, 1440, 1080, 720, 480, 360].contains(h)) {
        let format = VideoFormat::MaxHeight { height };
        choices.push(VideoChoice { label: format.label(), format, size: video_size_up_to(height), available: true });
    }
    if best_audio.is_some() || formats.is_empty() {
        choices.push(VideoChoice {
            format: VideoFormat::AudioBest,
            label: VideoFormat::AudioBest.label(),
            size: audio_size,
            available: true,
        });
        choices.push(VideoChoice {
            format: VideoFormat::AudioMp3,
            label: VideoFormat::AudioMp3.label(),
            size: audio_size,
            available: ffmpeg,
        });
    }

    VideoInfo {
        title: json["title"].as_str().unwrap_or("Video").to_owned(),
        uploader: json["uploader"].as_str().map(str::to_owned),
        duration: json["duration"].as_f64().map(|d| d as u64),
        choices,
    }
}

/// yt-dlp (and optionally ffmpeg) found on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoBackend {
    pub yt_dlp: PathBuf,
    pub ffmpeg: Option<PathBuf>,
    pub options: VideoOptions,
}

/// User preferences for video downloads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoOptions {
    /// mp4, mkv or webm, when streams are merged.
    pub container: String,
    /// Subtitle languages ("en,fr"), None for no subtitles.
    pub subtitles: Option<String>,
    /// Thumbnail and metadata embedded in the file (needs ffmpeg).
    pub embed_metadata: bool,
}

impl Default for VideoOptions {
    fn default() -> Self {
        VideoOptions { container: "mp4".to_owned(), subtitles: None, embed_metadata: true }
    }
}

fn command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW: no console flashing for every download.
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

impl VideoBackend {
    pub async fn info(&self, url: &Url) -> Result<VideoInfo, String> {
        let output = command(&self.yt_dlp)
            .args(["-J", "--no-playlist", "--no-warnings", url.as_str()])
            .output()
            .await
            .map_err(|e| format!("cannot run yt-dlp: {e}"))?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            let line = err.lines().rev().find(|l| l.contains("ERROR")).unwrap_or("yt-dlp failed");
            return Err(line.trim_start_matches("ERROR: ").to_owned());
        }
        let json: Value = serde_json::from_slice(&output.stdout).map_err(|e| format!("invalid yt-dlp output: {e}"))?;
        Ok(parse_info(&json, self.ffmpeg.is_some()))
    }

    /// Downloads `url` in `format` into `dest_dir`. Progress goes to `slot`.
    pub fn run(
        &self,
        url: Url,
        format: VideoFormat,
        dest_dir: PathBuf,
        rate_limit: u64,
        slot: Arc<ControlSlot>,
    ) -> BoxFuture<'static, RunResult> {
        let backend = self.clone();
        Box::pin(async move {
            if format.needs_ffmpeg() && backend.ffmpeg.is_none() {
                return Err(RunError::Failed("this format needs ffmpeg: install it in Settings".to_owned()));
            }
            tokio::fs::create_dir_all(&dest_dir).await.map_err(|e| RunError::Failed(e.to_string()))?;
            let mut cmd = command(&backend.yt_dlp);
            // --print implies --quiet and may imply --simulate: force both back.
            cmd.env("PYTHONIOENCODING", "utf-8")
                .args(["--encoding", "utf-8", "--no-simulate", "--progress"])
                .args(["--no-playlist", "--newline", "--no-warnings", "--no-colors", "--continue"])
                .args(["--progress-template", "download:BDPROG %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s %(info.title)s"])
                .args(["--print", "before_dl:BDID %(id)s"])
                .args(["--print", "after_move:BDPATH %(filepath)s"])
                .args(["-o", &dest_dir.join("%(title)s [%(id)s].%(ext)s").to_string_lossy()])
                .args(format.args(backend.ffmpeg.is_some()));
            if let Some(ffmpeg) = &backend.ffmpeg {
                cmd.arg("--ffmpeg-location").arg(ffmpeg);
                let audio_only = matches!(format, VideoFormat::AudioBest | VideoFormat::AudioMp3);
                if !audio_only {
                    cmd.args(["--merge-output-format", &backend.options.container]);
                }
                if backend.options.embed_metadata {
                    cmd.args(["--embed-metadata", "--embed-thumbnail"]);
                }
            }
            if let Some(langs) = backend.options.subtitles.as_deref().filter(|l| !l.trim().is_empty()) {
                cmd.args(["--write-subs", "--sub-langs", &langs.replace(' ', "")]);
                if backend.ffmpeg.is_some() {
                    cmd.arg("--embed-subs");
                }
            }
            if rate_limit > 0 {
                cmd.args(["--limit-rate", &rate_limit.to_string()]);
            }
            cmd.arg(url.as_str());

            let mut child = cmd.spawn().map_err(|e| RunError::Failed(format!("cannot run yt-dlp: {e}")))?;
            let control = Arc::new(VideoControl::default());
            slot.set(control.clone());
            let stdout = child.stdout.take().expect("stdout is piped");
            let stderr = child.stderr.take().expect("stderr is piped");
            let errors = tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                let mut last_error = None;
                while let Ok(Some(line)) = lines.next_line().await {
                    if line.contains("ERROR") {
                        last_error = Some(line.trim_start_matches("ERROR: ").to_owned());
                    }
                }
                last_error
            });

            // Read raw bytes: a title in another encoding must not stop the
            // reading (that lost the final path before).
            let mut reader = BufReader::new(stdout);
            let mut buf = Vec::new();
            let mut final_path = None;
            let mut video_id = None;
            loop {
                buf.clear();
                tokio::select! {
                    _ = control.cancel.cancelled() => {
                        let _ = child.kill().await;
                        return Err(RunError::Cancelled);
                    }
                    read = reader.read_until(b'\n', &mut buf) => match read {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let line = String::from_utf8_lossy(&buf);
                            let line = line.trim_end_matches(['\r', '\n']);
                            if let Some(rest) = line.strip_prefix("BDPROG ") {
                                control.update(rest);
                            } else if let Some(path) = line.strip_prefix("BDPATH ") {
                                final_path = Some(PathBuf::from(path.trim()));
                            } else if let Some(id) = line.strip_prefix("BDID ") {
                                video_id = Some(id.trim().to_owned());
                            }
                        }
                    }
                }
            }
            // Fallback: the newest file whose name carries the video id.
            if final_path.as_ref().is_none_or(|p| !p.exists()) {
                if let Some(id) = &video_id {
                    final_path = newest_file_with(&dest_dir, &format!("[{id}]")).or(final_path);
                }
            }
            let status = child.wait().await.map_err(|e| RunError::Failed(e.to_string()))?;
            if control.cancel.is_cancelled() {
                return Err(RunError::Cancelled);
            }
            let error = errors.await.ok().flatten();
            match (status.success(), final_path) {
                (true, Some(path)) => Ok(path),
                (true, None) => Err(RunError::Failed("yt-dlp did not report the file".to_owned())),
                (false, _) => Err(RunError::Failed(error.unwrap_or_else(|| format!("yt-dlp exited with {status}")))),
            }
        })
    }
}

fn newest_file_with(dir: &Path, marker: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            // Skip partial and per-format intermediate files ("x [id].f137.mp4").
            let intermediate = name
                .split('.')
                .any(|p| p.len() > 1 && p.starts_with('f') && p[1..].chars().all(|c| c.is_ascii_digit()));
            name.contains(marker) && !name.ends_with(".part") && !name.ends_with(".ytdl") && !intermediate
        })
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
}

/// Progress of a yt-dlp run. Each stream (video, then audio) is a segment.
#[derive(Default)]
struct VideoControl {
    progress: Mutex<Progress>,
    cancel: CancellationToken,
}

impl VideoControl {
    /// Parses `downloaded total estimate title`.
    fn update(&self, line: &str) {
        let mut parts = line.splitn(4, ' ');
        let num = |s: Option<&str>| s.and_then(|v| v.parse::<f64>().ok()).map(|v| v as u64);
        let downloaded = num(parts.next()).unwrap_or(0);
        let (exact, estimate) = (num(parts.next()), num(parts.next()));
        let total = exact.or(estimate);
        let title = parts.next().map(str::to_owned);
        let mut p = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        let new_stream = match p.segments.last() {
            None => true,
            Some(last) => downloaded < last.done && last.len.is_some_and(|l| last.done >= l),
        };
        if new_stream {
            p.segments.push(SegmentProgress::default());
        }
        if let Some(last) = p.segments.last_mut() {
            last.done = downloaded;
            last.len = total;
        }
        p.downloaded = p.segments.iter().map(|s| s.done).sum();
        p.total = p.segments.iter().map(|s| s.len).sum::<Option<u64>>();
        p.active_connections = 1;
        if p.file_name.is_none() {
            p.file_name = title.filter(|t| !t.is_empty() && t != "NA");
        }
    }
}

impl TaskControl for VideoControl {
    fn progress(&self) -> Progress {
        self.progress.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn cancel(&self) {
        self.cancel.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_video_sites() {
        assert!(is_video_url(&Url::parse("https://www.youtube.com/watch?v=abc").unwrap()));
        assert!(is_video_url(&Url::parse("https://youtu.be/abc").unwrap()));
        assert!(!is_video_url(&Url::parse("https://1fichier.com/?abc").unwrap()));
        assert!(!is_video_url(&Url::parse("https://notyoutube.com/x").unwrap()));
    }

    #[test]
    fn builds_choices_from_formats() {
        let json = serde_json::json!({
            "title": "Demo",
            "duration": 61.5,
            "formats": [
                {"format_id": "140", "vcodec": "none", "acodec": "mp4a", "abr": 128.0, "filesize": 1_000},
                {"format_id": "251", "vcodec": "none", "acodec": "opus", "abr": 160.0, "filesize": 1_200},
                {"format_id": "137", "vcodec": "avc1", "acodec": "none", "height": 1080, "tbr": 4000.0, "filesize": 50_000},
                {"format_id": "136", "vcodec": "avc1", "acodec": "none", "height": 720, "tbr": 2000.0, "filesize": 20_000},
                {"format_id": "18", "vcodec": "avc1", "acodec": "mp4a", "height": 360, "filesize_approx": 9_000}
            ]
        });
        let info = parse_info(&json, false);
        assert_eq!(info.title, "Demo");
        assert_eq!(info.duration, Some(61));
        let formats: Vec<_> = info.choices.iter().map(|c| c.format.clone()).collect();
        assert_eq!(
            formats,
            vec![
                VideoFormat::Best,
                VideoFormat::MaxHeight { height: 1080 },
                VideoFormat::MaxHeight { height: 720 },
                VideoFormat::MaxHeight { height: 360 },
                VideoFormat::AudioBest,
                VideoFormat::AudioMp3,
            ]
        );
        assert_eq!(info.choices[1].size, Some(51_200));
        assert_eq!(info.choices[4].size, Some(1_200));
        assert!(!info.choices[5].available, "MP3 needs ffmpeg");
    }

    #[test]
    fn tracks_video_then_audio_streams() {
        let c = VideoControl::default();
        c.update("500 1000 NA Demo");
        c.update("1000 1000 NA Demo");
        c.update("100 400 NA Demo");
        let p = c.progress();
        assert_eq!(p.segments.len(), 2);
        assert_eq!(p.downloaded, 1100);
        assert_eq!(p.total, Some(1400));
        assert_eq!(p.file_name.as_deref(), Some("Demo"));
    }
}
