//! File name helpers: package names for multi-part archives, archive
//! detection, and links found in web pages.

use url::Url;

const ARCHIVE_EXTENSIONS: &[&str] = &["rar", "zip", "7z", "tar", "gz", "tgz", "bz2", "xz"];

const FILE_EXTENSIONS: &[&str] = &[
    "rar", "zip", "7z", "tar", "gz", "tgz", "bz2", "xz", "iso", "img", "dmg", "exe", "msi", "apk", "pdf", "epub", "mp4",
    "mkv", "avi", "mov", "webm", "mp3", "flac", "wav", "ogg", "m4a", "jpg", "png", "bin", "dat", "deb", "rpm",
];

/// Lower-cased extension chain, e.g. `["part01", "rar"]` for `x.part01.rar`.
fn suffixes(name: &str) -> Vec<String> {
    name.rsplit('.').take(3).map(str::to_ascii_lowercase).collect::<Vec<_>>().into_iter().rev().collect()
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// Name shared by all parts of a split archive: `Movie.part01.rar`,
/// `Movie.r00`, `Movie.7z.001` → `Movie`. Other files lose their extension.
pub fn package_name(file_name: &str) -> String {
    let mut base = file_name.to_owned();
    // Strip up to two extension levels when they look like archive parts.
    for round in 0..2 {
        let Some((stem, ext)) = base.rsplit_once('.') else { break };
        let ext_l = ext.to_ascii_lowercase();
        let strip = ARCHIVE_EXTENSIONS.contains(&ext_l.as_str())
            // Split volumes (.001) only as the last extension.
            || (round == 0 && is_digits(&ext_l))
            || (ext_l.starts_with("part") && is_digits(&ext_l[4..]))
            || (ext_l.len() == 3 && (ext_l.starts_with('r') || ext_l.starts_with('z')) && is_digits(&ext_l[1..]))
            || FILE_EXTENSIONS.contains(&ext_l.as_str());
        if !strip || stem.is_empty() {
            break;
        }
        base = stem.to_owned();
    }
    let trimmed = base.trim_matches(|c: char| c == '.' || c == '_' || c == '-' || c.is_whitespace());
    if trimmed.is_empty() { file_name.to_owned() } else { trimmed.to_owned() }
}

/// Whether the file is the volume an extractor should open (the first part
/// of a split archive, or a single archive).
pub fn is_first_volume(file_name: &str) -> bool {
    let s = suffixes(file_name);
    let last = s.last().map(String::as_str).unwrap_or_default();
    let prev = if s.len() >= 2 { s[s.len() - 2].as_str() } else { "" };
    match last {
        "rar" => {
            if let Some(n) = prev.strip_prefix("part") {
                return n.parse::<u32>().is_ok_and(|n| n == 1);
            }
            true
        }
        "zip" | "7z" => true,
        // Split 7z/zip: only the .001 volume.
        n if is_digits(n) => (prev == "7z" || prev == "zip") && n.parse::<u32>().is_ok_and(|n| n == 1),
        _ => false,
    }
}

/// Whether a file belongs to an archive set (to delete after extraction).
pub fn is_archive_part(file_name: &str) -> bool {
    let s = suffixes(file_name);
    let last = s.last().map(String::as_str).unwrap_or_default();
    let prev = if s.len() >= 2 { s[s.len() - 2].as_str() } else { "" };
    ARCHIVE_EXTENSIONS.contains(&last)
        || (last.len() == 3 && last.starts_with('r') && is_digits(&last[1..]))
        || (is_digits(last) && (prev == "7z" || prev == "zip"))
}

/// Links worth downloading found in an HTML page: hoster links and links to
/// files, resolved against the page URL. Also returns the page title.
pub fn links_in_page(base: &Url, html: &str) -> (Option<String>, Vec<Url>) {
    let title = find_between(html, "<title", "</title>")
        .and_then(|t| t.split_once('>').map(|(_, rest)| rest))
        .map(|t| decode_entities(t.trim()))
        .filter(|t| !t.is_empty());
    let mut links: Vec<Url> = Vec::new();
    let mut push = |url: Url| {
        if !links.contains(&url) && &url != base {
            links.push(url);
        }
    };
    let lower = html.to_ascii_lowercase();
    let mut pos = 0;
    while let Some(found) = lower[pos..].find("href=") {
        let start = pos + found + 5;
        pos = start;
        let rest = &html[start..];
        let (quote, rest) = match rest.chars().next() {
            Some(q @ ('"' | '\'')) => (Some(q), &rest[1..]),
            _ => (None, rest),
        };
        let end = match quote {
            Some(q) => rest.find(q),
            None => rest.find(|c: char| c.is_whitespace() || c == '>'),
        }
        .unwrap_or(rest.len());
        let href = decode_entities(&rest[..end]);
        if let Ok(url) = base.join(href.trim()) {
            if is_wanted(&url) {
                push(url);
            }
        }
    }
    for url in crate::links::extract_links(html) {
        if is_wanted(&url) {
            push(url);
        }
    }
    (title, links)
}

fn is_wanted(url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    if crate::hosters::hoster_of(url).is_some() {
        return true;
    }
    let path = url.path().to_ascii_lowercase();
    let name = path.rsplit('/').next().unwrap_or_default();
    match name.rsplit_once('.') {
        Some((_, ext)) => FILE_EXTENSIONS.contains(&ext) || is_digits(ext) && name.contains(".7z.") || name.contains(".zip."),
        None => false,
    }
}

fn find_between<'a>(haystack: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let lower = haystack.to_ascii_lowercase();
    let a = lower.find(start)?;
    let b = lower[a..].find(end)? + a;
    Some(&haystack[a + start.len()..b])
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names() {
        assert_eq!(package_name("Holiday_Photos_2026.part01.rar"), "Holiday_Photos_2026");
        assert_eq!(package_name("Holiday_Photos_2026.part12.rar"), "Holiday_Photos_2026");
        assert_eq!(package_name("Movie.r00"), "Movie");
        assert_eq!(package_name("Textures.7z.003"), "Textures");
        assert_eq!(package_name("backup.zip"), "backup");
        assert_eq!(package_name("debian-13.iso"), "debian-13");
        assert_eq!(package_name("noext"), "noext");
        assert_eq!(package_name("report.2026.zip"), "report.2026");
    }

    #[test]
    fn first_volumes() {
        assert!(is_first_volume("a.part1.rar"));
        assert!(is_first_volume("a.part01.rar"));
        assert!(!is_first_volume("a.part02.rar"));
        assert!(is_first_volume("a.rar"));
        assert!(!is_first_volume("a.r00"));
        assert!(is_first_volume("a.zip"));
        assert!(is_first_volume("a.7z.001"));
        assert!(!is_first_volume("a.7z.002"));
        assert!(!is_first_volume("a.mkv"));
        assert!(is_archive_part("a.r05") && is_archive_part("a.part3.rar") && !is_archive_part("a.mp4"));
    }

    #[test]
    fn finds_links_in_pages() {
        let base = Url::parse("https://blog.example/post/42").unwrap();
        let html = r#"<html><head><title>My &amp; files</title></head><body>
            <a href="/files/a.part1.rar">part 1</a> <a href='https://1fichier.com/?xyz'>mirror</a>
            <a href=b.zip>b</a> <a href="/about">about</a> <a href="mailto:x@y">mail</a>
            Also https://rapidgator.net/file/123/a.part2.rar.html in text.</body></html>"#;
        let (title, links) = links_in_page(&base, html);
        assert_eq!(title.as_deref(), Some("My & files"));
        let links: Vec<String> = links.iter().map(|u| u.to_string()).collect();
        assert_eq!(
            links,
            vec![
                "https://blog.example/files/a.part1.rar",
                "https://1fichier.com/?xyz",
                "https://blog.example/post/b.zip",
                "https://rapidgator.net/file/123/a.part2.rar.html",
            ]
        );
    }
}
