//! Free mode for hosters: follow the same steps as a visitor of the site
//! (pages, forms, wait timers) to get the file. Captchas are never solved
//! here: when a site asks for one, the page is handed to the user's browser.

use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{COOKIE, REFERER};
use serde_json::Value;
use url::Url;

use crate::task::{ControlSlot, Resolved};

const BROWSER_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

/// Why a free download could not be resolved automatically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FreeError {
    /// The site wants a captcha: the user has to open the page.
    Captcha(Url),
    Offline(String),
    Other(String),
}

impl std::fmt::Display for FreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FreeError::Captcha(_) => write!(f, "this hoster asks for a captcha in free mode"),
            FreeError::Offline(why) => write!(f, "file offline: {why}"),
            FreeError::Other(why) => f.write_str(why),
        }
    }
}

type FreeResult<T> = Result<T, FreeError>;

fn other(e: impl std::fmt::Display) -> FreeError {
    FreeError::Other(e.to_string())
}

/// Browser-like client with its own cookie jar for one resolution.
pub fn session_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(BROWSER_UA)
        .cookie_store(true)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60))
        .build()
        .expect("static client configuration")
}

// ------------------------------------------------------------------ HTML helpers

/// A `<form>` with its hidden and visible inputs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Form {
    pub action: Option<String>,
    pub method: String,
    pub fields: Vec<(String, String)>,
    pub html: String,
}

impl Form {
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn set(&mut self, name: &str, value: &str) {
        match self.fields.iter_mut().find(|(k, _)| k == name) {
            Some(field) => field.1 = value.to_owned(),
            None => self.fields.push((name.to_owned(), value.to_owned())),
        }
    }
}

/// Value of an attribute inside a tag, e.g. `attr(r#"<input name="x">"#, "name")`.
pub fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(pos) = lower[from..].find(&format!("{name}=")) {
        let start = from + pos;
        // Must be a whole attribute name.
        let before = lower[..start].chars().last().unwrap_or(' ');
        from = start + name.len() + 1;
        if !(before.is_whitespace() || before == '<') {
            continue;
        }
        let rest = &tag[from..];
        let value = match rest.chars().next() {
            Some(q @ ('"' | '\'')) => rest[1..].split(q).next().unwrap_or_default(),
            _ => rest.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or_default(),
        };
        return Some(decode_html(value));
    }
    None
}

pub fn decode_html(s: &str) -> String {
    s.replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">")
}

fn tags<'a>(html: &'a str, name: &str) -> Vec<&'a str> {
    let lower = html.to_ascii_lowercase();
    let open = format!("<{name}");
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(found) = lower[pos..].find(&open) {
        let start = pos + found;
        let end = lower[start..].find('>').map(|e| start + e + 1).unwrap_or(html.len());
        out.push(&html[start..end]);
        pos = end;
    }
    out
}

pub fn forms(html: &str) -> Vec<Form> {
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(found) = lower[pos..].find("<form") {
        let start = pos + found;
        let end = lower[start..].find("</form>").map(|e| start + e).unwrap_or(html.len());
        let block = &html[start..end];
        let head_end = block.find('>').unwrap_or(block.len());
        let head = &block[..head_end];
        let mut form = Form {
            action: attr(head, "action"),
            method: attr(head, "method").unwrap_or_else(|| "get".into()).to_ascii_lowercase(),
            fields: Vec::new(),
            html: block.to_owned(),
        };
        for input in tags(block, "input") {
            let kind = attr(input, "type").unwrap_or_default().to_ascii_lowercase();
            if kind == "checkbox" && !input.to_ascii_lowercase().contains("checked") {
                continue;
            }
            if let Some(name) = attr(input, "name") {
                form.fields.push((name, attr(input, "value").unwrap_or_default()));
            }
        }
        out.push(form);
        pos = end.max(start + 5);
    }
    out
}

/// Kind of captcha present on a page, if any.
pub fn captcha_on(html: &str) -> Option<&'static str> {
    let lower = html.to_ascii_lowercase();
    if lower.contains("g-recaptcha") || lower.contains("recaptcha/api") {
        Some("reCAPTCHA")
    } else if lower.contains("h-captcha") || lower.contains("hcaptcha.com") {
        Some("hCaptcha")
    } else if lower.contains("cf-turnstile") || lower.contains("challenges.cloudflare.com/turnstile") {
        Some("Turnstile")
    } else if lower.contains("name=\"code\"") && lower.contains("captcha") {
        Some("code")
    } else if lower.contains("solvemedia") {
        Some("SolveMedia")
    } else {
        None
    }
}

/// Seconds of the wait timer shown on a page (XFileSharing style).
pub fn countdown(html: &str) -> Option<u64> {
    let lower = html.to_ascii_lowercase();
    for marker in ["id=\"countdown_str\"", "class=\"seconds\"", "id=\"countdown\"", "id=\"seconds\"", "var countdown ="]
    {
        if let Some(pos) = lower.find(marker) {
            let digits: String = lower[pos + marker.len()..]
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(n) = digits.parse::<u64>() {
                if n > 0 && n < 3600 {
                    return Some(n);
                }
            }
        }
    }
    None
}

const OFFLINE_MARKERS: &[&str] = &[
    "file not found",
    "file was removed",
    "file has been removed",
    "no such file",
    "the file you were looking for could not be found",
    "file is no longer available",
    "this file is no longer available",
    "file deleted",
    "file does not exist",
    "le fichier demandé n'existe pas",
];

pub fn looks_offline(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    OFFLINE_MARKERS.iter().any(|m| lower.contains(m))
}

/// Size written on a page, like "(1.4 GB)" or "Size: 250 MB".
pub fn size_on_page(html: &str) -> Option<u64> {
    let text = html.replace("&nbsp;", " ");
    let lower = text.to_ascii_lowercase();
    let units: &[(&str, f64)] =
        &[("tb", 1e12), ("gb", 1e9), ("mb", 1e6), ("kb", 1e3), ("to", 1e12), ("go", 1e9), ("mo", 1e6), ("ko", 1e3)];
    // Prefer a number near the word "size".
    let start = lower.find("size").or_else(|| lower.find("taille")).unwrap_or(0);
    let scan = &lower[start..];
    let bytes = scan.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let num_end = scan[i..]
                .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','))
                .map(|e| i + e)
                .unwrap_or(scan.len());
            let number: f64 = scan[i..num_end].replace(',', ".").parse().unwrap_or(0.0);
            let rest = scan[num_end..].trim_start();
            for (unit, factor) in units {
                if rest.starts_with(unit) && !rest[unit.len()..].starts_with(|c: char| c.is_ascii_alphabetic()) {
                    return Some((number * factor) as u64);
                }
            }
            i = num_end;
        } else {
            i += 1;
        }
        if i > 4000 {
            break;
        }
    }
    None
}

/// File name written on a page.
pub fn name_on_page(html: &str) -> Option<String> {
    for form in forms(html) {
        if let Some(name) = form.field("fname").filter(|n| !n.is_empty()) {
            return Some(name.to_owned());
        }
    }
    for meta in tags(html, "meta") {
        if attr(meta, "property").as_deref() == Some("og:title") {
            if let Some(content) = attr(meta, "content").filter(|c| c.contains('.')) {
                return Some(content);
            }
        }
    }
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title>")? + 7;
    let end = lower[start..].find("</title>")? + start;
    let title = decode_html(html[start..end].trim());
    let title = title.trim_start_matches("Download ").split(" - ").next().unwrap_or(&title).trim().to_owned();
    (title.contains('.') && title.len() < 250).then_some(title)
}

// ------------------------------------------------------------------ waiting

async fn wait_seconds(slot: &Arc<ControlSlot>, seconds: u64, what: &str) -> FreeResult<()> {
    for left in (1..=seconds).rev() {
        if slot.is_cancelled() {
            return Err(FreeError::Other("cancelled".into()));
        }
        slot.set_status(Some(&format!("{what}: {left} s")));
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    slot.set_status(None);
    Ok(())
}

async fn get_text(client: &reqwest::Client, url: &Url) -> FreeResult<(Url, String)> {
    let response = client.get(url.clone()).send().await.map_err(other)?;
    let status = response.status();
    let final_url = response.url().clone();
    if status.as_u16() == 404 || status.as_u16() == 410 {
        return Err(FreeError::Offline(format!("HTTP {}", status.as_u16())));
    }
    let text = response.text().await.map_err(other)?;
    Ok((final_url, text))
}

fn direct(url: Url, via: &str) -> Resolved {
    Resolved { via: via.to_owned(), ..Resolved::direct(url) }
}

/// Finds a direct link to the file in a page.
fn file_link(page: &Url, html: &str, host_hint: &str) -> Option<Url> {
    for a in tags(html, "a") {
        let Some(href) = attr(a, "href") else { continue };
        let lower = a.to_ascii_lowercase();
        let Ok(url) = page.join(&href) else { continue };
        let looks_file = url
            .path()
            .rsplit('/')
            .next()
            .is_some_and(|n| n.contains('.') && !n.ends_with(".html") && !n.ends_with(".php"));
        let marked = lower.contains("download") || lower.contains("btn-general") || lower.contains("direct");
        let other_host = url.host_str() != page.host_str() || url.host_str().is_some_and(|h| h.contains(host_hint));
        if looks_file && marked && other_host && url.scheme().starts_with("http") {
            return Some(url);
        }
    }
    None
}

// ------------------------------------------------------------------ hosters

/// 1fichier free mode: the page may ask to wait; then a form gives the link.
pub async fn onefichier(url: &Url, slot: &Arc<ControlSlot>) -> FreeResult<Resolved> {
    let client = session_client();
    let (page, html) = get_text(&client, url).await?;
    if looks_offline(&html) {
        return Err(FreeError::Offline("removed".into()));
    }
    let lower = html.to_ascii_lowercase();
    if let Some(pos) = lower.find("you must wait").or_else(|| lower.find("vous devez attendre")) {
        let minutes: String =
            lower[pos..].chars().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit()).collect();
        let minutes = minutes.parse::<u64>().unwrap_or(1).max(1);
        wait_seconds(slot, minutes * 60 + 5, "Free mode, 1fichier asks to wait").await?;
        return Box::pin(onefichier(url, slot)).await;
    }
    let mut form =
        forms(&html).into_iter().find(|f| f.field("adz").is_some()).ok_or_else(|| other("1fichier page changed"))?;
    form.set("dl_no_ssl", "on");
    form.set("dlinline", "on");
    let response =
        client.post(page.clone()).header(REFERER, page.as_str()).form(&form.fields).send().await.map_err(other)?;
    let html = response.text().await.map_err(other)?;
    let link = file_link(&page, &html, "1fichier.com").ok_or_else(|| {
        if captcha_on(&html).is_some() {
            FreeError::Captcha(url.clone())
        } else {
            other("no download link on the 1fichier page")
        }
    })?;
    Ok(direct(link, "1fichier (free)"))
}

/// Sites built on XFileSharing (uploady, dailyuploads, ddownload, katfile…):
/// "Free download" form, wait timer, second form, then the file.
pub async fn xfilesharing(url: &Url, slot: &Arc<ControlSlot>, via: &str) -> FreeResult<Resolved> {
    let client = session_client();
    let (page, mut html) = get_text(&client, url).await?;
    if looks_offline(&html) {
        return Err(FreeError::Offline("removed".into()));
    }
    for step in 0..3 {
        // Already a direct link on the page?
        if let Some(link) = file_link(&page, &html, page.host_str().unwrap_or_default()) {
            return Ok(direct(link, via));
        }
        let form = forms(&html)
            .into_iter()
            .find(|f| matches!(f.field("op"), Some("download1" | "download2" | "download3")))
            .ok_or_else(|| {
                if captcha_on(&html).is_some() {
                    FreeError::Captcha(url.clone())
                } else {
                    other(format!("{via}: page not recognised"))
                }
            })?;
        let mut fields = form.fields.clone();
        if form.field("op") == Some("download1") {
            // Choose the free download.
            fields.retain(|(k, _)| k != "method_premium");
            if !fields.iter().any(|(k, _)| k == "method_free") {
                fields.push(("method_free".into(), "Free Download".into()));
            }
        } else {
            if let Some(kind) = captcha_on(&form.html) {
                tracing::info!(hoster = via, captcha = kind, "free mode needs a captcha");
                return Err(FreeError::Captcha(url.clone()));
            }
            if let Some(seconds) = countdown(&html) {
                wait_seconds(slot, seconds + 1, &format!("Free mode, {via} wait")).await?;
            }
        }
        let action = form
            .action
            .as_deref()
            .filter(|a| !a.is_empty())
            .and_then(|a| page.join(a).ok())
            .unwrap_or_else(|| page.clone());
        let response = client.post(action).header(REFERER, page.as_str()).form(&fields).send().await.map_err(other)?;
        // The last step often answers with a redirect to the file.
        let final_url = response.url().clone();
        let is_file = response
            .headers()
            .get(reqwest::header::CONTENT_DISPOSITION)
            .is_some_and(|v| v.to_str().unwrap_or_default().contains("attachment"))
            || response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .is_some_and(|v| !v.to_str().unwrap_or_default().starts_with("text/"));
        if is_file {
            return Ok(direct(final_url, via));
        }
        html = response.text().await.map_err(other)?;
        tracing::debug!(step, hoster = via, "free mode step done");
    }
    Err(other(format!("{via}: no download link after the wait")))
}

/// MediaFire shows the direct link on the page.
pub async fn mediafire(url: &Url) -> FreeResult<Resolved> {
    let client = session_client();
    let (page, html) = get_text(&client, url).await?;
    if looks_offline(&html) || html.contains("error.php") {
        return Err(FreeError::Offline("removed".into()));
    }
    for a in tags(&html, "a") {
        if attr(a, "id").as_deref() == Some("downloadButton") {
            if let Some(href) = attr(a, "href").or_else(|| attr(a, "data-scrambled-url")) {
                let link = page.join(&href).map_err(other)?;
                return Ok(direct(link, "MediaFire (free)"));
            }
        }
    }
    Err(other("MediaFire page changed"))
}

/// Gofile: a guest token, then the content API lists the file links.
pub async fn gofile(url: &Url) -> FreeResult<Resolved> {
    let id = url
        .path_segments()
        .and_then(|mut s| {
            s.next();
            s.next()
        })
        .ok_or_else(|| other("not a Gofile link"))?
        .to_owned();
    let client = session_client();
    let account: Value =
        client.post("https://api.gofile.io/accounts").send().await.map_err(other)?.json().await.map_err(other)?;
    let token = account["data"]["token"].as_str().ok_or_else(|| other("Gofile refused a guest session"))?.to_owned();
    let content: Value = client
        .get(format!("https://api.gofile.io/contents/{id}?wt=4fd6sg89d7s6"))
        .bearer_auth(&token)
        .send()
        .await
        .map_err(other)?
        .json()
        .await
        .map_err(other)?;
    if content["status"] != "ok" {
        return Err(FreeError::Offline(content["status"].as_str().unwrap_or("unknown").to_owned()));
    }
    let children = content["data"]["children"].as_object().ok_or_else(|| other("Gofile folder is empty"))?;
    let file = children.values().find(|c| c["type"] == "file").ok_or_else(|| other("no file in this Gofile folder"))?;
    let link = Url::parse(file["link"].as_str().ok_or_else(|| other("no link"))?).map_err(other)?;
    Ok(Resolved {
        url: link,
        headers: vec![(COOKIE.as_str().to_owned(), format!("accountToken={token}"))],
        file_name: file["name"].as_str().map(str::to_owned),
        via: "Gofile (free)".to_owned(),
    })
}

/// Google Drive: direct export link, with the "large file" confirmation.
pub async fn google_drive(url: &Url) -> FreeResult<Resolved> {
    let id = url
        .query_pairs()
        .find(|(k, _)| k == "id")
        .map(|(_, v)| v.into_owned())
        .or_else(|| {
            let segments: Vec<&str> = url.path_segments()?.collect();
            segments.iter().position(|s| *s == "d").and_then(|i| segments.get(i + 1)).map(|s| s.to_string())
        })
        .ok_or_else(|| other("not a Google Drive file link"))?;
    let client = session_client();
    let first =
        Url::parse(&format!("https://drive.usercontent.google.com/download?id={id}&export=download")).map_err(other)?;
    let response = client.get(first.clone()).send().await.map_err(other)?;
    let is_page = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .is_some_and(|v| v.to_str().unwrap_or_default().starts_with("text/html"));
    if !is_page {
        return Ok(direct(response.url().clone(), "Google Drive"));
    }
    let html = response.text().await.map_err(other)?;
    let form = forms(&html)
        .into_iter()
        .find(|f| f.field("confirm").is_some())
        .ok_or(FreeError::Offline("not shared publicly".into()))?;
    let mut link = first;
    link.query_pairs_mut().clear().extend_pairs(form.fields.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    Ok(direct(link, "Google Drive"))
}

/// Krakenfiles: the page form posts a token and answers with the link.
pub async fn krakenfiles(url: &Url) -> FreeResult<Resolved> {
    let client = session_client();
    let (page, html) = get_text(&client, url).await?;
    if looks_offline(&html) {
        return Err(FreeError::Offline("removed".into()));
    }
    let form = forms(&html)
        .into_iter()
        .find(|f| f.field("token").is_some())
        .ok_or_else(|| other("Krakenfiles page changed"))?;
    let action = page.join(form.action.as_deref().unwrap_or_default()).map_err(other)?;
    let answer: Value =
        client.post(action).form(&form.fields).send().await.map_err(other)?.json().await.map_err(other)?;
    let link = answer["url"].as_str().ok_or_else(|| other("Krakenfiles gave no link"))?;
    Ok(direct(page.join(link).map_err(other)?, "Krakenfiles (free)"))
}

/// Name, size and availability read from a hoster page, for the link
/// grabber. `None` fields mean "not found on the page".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageInfo {
    pub name: Option<String>,
    pub size: Option<u64>,
    pub offline: bool,
}

pub async fn page_info(url: &Url) -> FreeResult<PageInfo> {
    let client = session_client();
    let (_, html) = get_text(&client, url).await?;
    Ok(PageInfo { name: name_on_page(&html), size: size_on_page(&html), offline: looks_offline(&html) })
}

/// 1fichier offers a free API to check links: `url;name;size` per line.
pub async fn onefichier_check(url: &Url) -> FreeResult<PageInfo> {
    let client = session_client();
    let text = client
        .post("https://1fichier.com/check_links.pl")
        .form(&[("links[]", url.as_str())])
        .send()
        .await
        .map_err(other)?
        .text()
        .await
        .map_err(other)?;
    let line = text.lines().next().unwrap_or_default();
    let parts: Vec<&str> = line.split(';').collect();
    if parts.len() >= 3 && !parts[1].is_empty() {
        Ok(PageInfo { name: Some(parts[1].to_owned()), size: parts[2].trim().parse().ok(), offline: false })
    } else {
        Ok(PageInfo { offline: line.to_ascii_uppercase().contains("NOT FOUND"), ..PageInfo::default() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XFS_PAGE: &str = r#"<html><head><title>Download My.Movie.mkv - uploady</title></head><body>
        <h2>My.Movie.mkv</h2><span>Size: 1.4 GB</span>
        <form method="POST" action=''>
          <input type="hidden" name="op" value="download1">
          <input type="hidden" name="usr_login" value="">
          <input type="hidden" name="id" value="abc123">
          <input type="hidden" name="fname" value="My.Movie.mkv">
          <input type="submit" name="method_free" value="Free Download">
          <input type="submit" name="method_premium" value="Premium">
        </form></body></html>"#;

    #[test]
    fn parses_forms_and_attributes() {
        let forms = forms(XFS_PAGE);
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].method, "post");
        assert_eq!(forms[0].field("op"), Some("download1"));
        assert_eq!(forms[0].field("id"), Some("abc123"));
        assert_eq!(attr(r#"<a class="x" href='/f/1'>"#, "href").as_deref(), Some("/f/1"));
        assert_eq!(attr(r#"<a data-href="no" href=yes>"#, "href").as_deref(), Some("yes"));
    }

    #[test]
    fn reads_name_size_and_state() {
        assert_eq!(name_on_page(XFS_PAGE).as_deref(), Some("My.Movie.mkv"));
        assert_eq!(size_on_page(XFS_PAGE), Some(1_400_000_000));
        assert!(!looks_offline(XFS_PAGE));
        assert!(looks_offline("<h1>File Not Found</h1>"));
        assert_eq!(size_on_page("Taille : 250,5 Mo"), Some(250_500_000));
    }

    #[test]
    fn detects_captchas_and_timers() {
        assert_eq!(captcha_on(r#"<div class="g-recaptcha" data-sitekey="x">"#), Some("reCAPTCHA"));
        assert_eq!(captcha_on(r#"<div class="h-captcha">"#), Some("hCaptcha"));
        assert_eq!(captcha_on("<p>hello</p>"), None);
        assert_eq!(countdown(r#"<span id="countdown_str">Wait <span id="cx">45</span> seconds</span>"#), Some(45));
        assert_eq!(countdown("no timer"), None);
    }

    #[test]
    fn finds_direct_links() {
        let page = Url::parse("https://uploady.io/abc123").unwrap();
        let html = r#"<a href="https://s3.uploady.io/d/xyz/My.Movie.mkv" class="btn btn-download">Download</a>"#;
        assert_eq!(file_link(&page, html, "uploady.io").unwrap().as_str(), "https://s3.uploady.io/d/xyz/My.Movie.mkv");
        assert!(file_link(&page, r#"<a href="/premium.html">Download faster</a>"#, "uploady.io").is_none());
    }
}
