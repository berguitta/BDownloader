//! Small site icons shown before hoster names. Fetched from the site itself
//! (no third-party favicon service) and cached on disk.

use std::path::PathBuf;
use std::time::Duration;

/// Returns RGBA pixels and size, or `None` when the site has no usable icon.
pub async fn fetch(client: &reqwest::Client, host: &str) -> Option<(Vec<u8>, u32, u32)> {
    if host.is_empty() {
        return None;
    }
    let cache = cache_path(host);
    if let Some(path) = &cache {
        if let Ok(bytes) = tokio::fs::read(path).await {
            return decode(&bytes);
        }
    }
    for url in [format!("https://{host}/favicon.ico"), format!("https://www.{host}/favicon.ico")] {
        let Ok(response) = client.get(&url).timeout(Duration::from_secs(8)).send().await else { continue };
        if !response.status().is_success() {
            continue;
        }
        let Ok(bytes) = response.bytes().await else { continue };
        if let Some(icon) = decode(&bytes) {
            if let Some(path) = &cache {
                if let Some(parent) = path.parent() {
                    let _ = tokio::fs::create_dir_all(parent).await;
                }
                let _ = tokio::fs::write(path, &bytes).await;
            }
            return Some(icon);
        }
    }
    None
}

fn cache_path(host: &str) -> Option<PathBuf> {
    let safe: String =
        host.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    directories::ProjectDirs::from("", "", "BDownloader")
        .map(|d| d.cache_dir().join("favicons").join(format!("{safe}.ico")))
}

/// Decodes ICO or PNG data into 16×16 RGBA (or the closest size).
fn decode(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let image = image::load_from_memory(bytes).ok()?;
    let image = if image.width() > 32 { image.resize(32, 32, image::imageops::FilterType::Triangle) } else { image };
    let rgba = image.to_rgba8();
    let (w, h) = rgba.dimensions();
    (w > 0 && h > 0).then(|| (rgba.into_raw(), w, h))
}
