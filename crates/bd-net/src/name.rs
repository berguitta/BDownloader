use url::Url;

/// Characters that Windows refuses in file names.
const FORBIDDEN: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// Device names reserved by Windows, whatever the extension.
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Makes a server-provided name safe to use as a local file name.
pub fn sanitize_file_name(name: &str) -> String {
    let cleaned: String =
        name.chars().map(|c| if FORBIDDEN.contains(&c) || c.is_control() { '_' } else { c }).collect();
    let trimmed = cleaned.trim().trim_end_matches(['.', ' ']).to_owned();
    if trimmed.is_empty() {
        return "download".to_owned();
    }
    let stem = trimmed.split('.').next().unwrap_or_default().to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        return format!("_{trimmed}");
    }
    // Keep names well below the classic 260 character path limit.
    trimmed.chars().take(200).collect()
}

/// Last non-empty path segment of the URL, percent-decoded.
pub fn file_name_from_url(url: &Url) -> Option<String> {
    let segment = url.path_segments()?.rfind(|s| !s.is_empty())?;
    let decoded = percent_decode(segment);
    (!decoded.is_empty()).then(|| sanitize_file_name(&decoded))
}

/// Extracts the file name from a `Content-Disposition` header value.
/// Supports both `filename="a.zip"` and RFC 5987 `filename*=UTF-8''a%20b.zip`.
pub(crate) fn file_name_from_disposition(value: &str) -> Option<String> {
    let mut plain = None;
    for part in value.split(';').map(str::trim) {
        let Some((key, val)) = part.split_once('=') else { continue };
        let key = key.trim().to_ascii_lowercase();
        let val = val.trim();
        if key == "filename*" {
            let encoded = val.splitn(3, '\'').nth(2).unwrap_or(val);
            let decoded = percent_decode(encoded.trim_matches('"'));
            if !decoded.is_empty() {
                return Some(sanitize_file_name(&decoded));
            }
        } else if key == "filename" {
            plain = Some(val.trim_matches('"').to_owned());
        }
    }
    plain.filter(|s| !s.is_empty()).map(|s| sanitize_file_name(&s))
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_forbidden_and_reserved_names() {
        assert_eq!(sanitize_file_name("a:b?.zip"), "a_b_.zip");
        assert_eq!(sanitize_file_name("CON.txt"), "_CON.txt");
        assert_eq!(sanitize_file_name("  "), "download");
        assert_eq!(sanitize_file_name("name. "), "name");
    }

    #[test]
    fn reads_name_from_url() {
        let url = Url::parse("https://example.com/files/My%20File.part1.rar?x=1").unwrap();
        assert_eq!(file_name_from_url(&url).as_deref(), Some("My File.part1.rar"));
        let root = Url::parse("https://example.com/").unwrap();
        assert_eq!(file_name_from_url(&root), None);
    }

    #[test]
    fn reads_name_from_content_disposition() {
        assert_eq!(file_name_from_disposition(r#"attachment; filename="a b.zip""#).as_deref(), Some("a b.zip"));
        assert_eq!(
            file_name_from_disposition("attachment; filename=x.bin; filename*=UTF-8''%C3%A9t%C3%A9.7z").as_deref(),
            Some("été.7z")
        );
        assert_eq!(file_name_from_disposition("inline"), None);
    }
}
