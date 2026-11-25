//! Finds download links in any text: a single URL, a list, a forum post or
//! an HTML page.

use url::Url;

/// Returns the unique http(s) URLs found in `text`, in order of appearance.
/// Handles common obfuscations such as `hxxp://` and trailing punctuation.
pub fn extract_links(text: &str) -> Vec<Url> {
    let mut found: Vec<Url> = Vec::new();
    for raw in text.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '(' | ')' | '[' | ']')) {
        if let Some(pos) = raw.find("magnet:?") {
            if let Ok(url) = Url::parse(raw[pos..].trim_end_matches(['.', ',', ';'])) {
                if !found.contains(&url) {
                    found.push(url);
                }
            }
            continue;
        }
        let Some(start) = find_scheme(raw) else { continue };
        let candidate = raw[start..].trim_end_matches(['.', ',', ';', ':', '!', '?']);
        let normalized = if let Some(rest) = candidate.strip_prefix("hxxps://") {
            format!("https://{rest}")
        } else if let Some(rest) = candidate.strip_prefix("hxxp://") {
            format!("http://{rest}")
        } else {
            candidate.to_owned()
        };
        if let Ok(url) = Url::parse(&normalized) {
            if matches!(url.scheme(), "http" | "https") && url.host().is_some() && !found.contains(&url) {
                found.push(url);
            }
        }
    }
    found
}

fn find_scheme(s: &str) -> Option<usize> {
    let lower = s.to_ascii_lowercase();
    ["https://", "http://", "hxxps://", "hxxp://"].iter().filter_map(|p| lower.find(p)).min()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_links_in_free_text() {
        let text = "Parts: https://a.example/f1.rar, https://a.example/f2.rar.\n\
                    mirror (hxxp://b.example/x.zip) and <a href=\"https://c.example/y\">y</a>\n\
                    again https://a.example/f1.rar and ftp://nope.example/z";
        let links: Vec<String> = extract_links(text).iter().map(|u| u.to_string()).collect();
        assert_eq!(
            links,
            vec![
                "https://a.example/f1.rar",
                "https://a.example/f2.rar",
                "http://b.example/x.zip",
                "https://c.example/y"
            ]
        );
    }

    #[test]
    fn ignores_text_without_links() {
        assert!(extract_links("nothing to see here").is_empty());
        assert!(extract_links("https://").is_empty());
    }
}
