//! Free mode for hosters: follow the same steps as a visitor of the site
//! (pages, forms, wait timers) to get the file, all in the background.
//! Captchas are never solved here: a code or image captcha is shown to the
//! user inside the app ([CaptchaAsk]); an "I am not a robot" widget can only
//! be done in the user's browser.

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

/// Asks the internet box for a new IP; true when it worked.
pub type ReconnectFn = Arc<dyn Fn() -> futures_util::future::BoxFuture<'static, bool> + Send + Sync>;

/// Reconnect instead of waiting when a hoster asks to wait at least this
/// many minutes.
#[derive(Clone)]
pub struct Reconnect {
    pub run: ReconnectFn,
    pub after_minutes: u64,
}

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
    pub id: Option<String>,
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

/// End (exclusive) of the tag starting at `start`; a `>` inside a quoted
/// attribute value (`value="Free Download >>"`) does not close it.
fn tag_end(html: &str, start: usize) -> usize {
    let mut quote = None;
    for (i, c) in html[start..].char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => return start + i + 1,
            _ => {}
        }
    }
    html.len()
}

fn tags<'a>(html: &'a str, name: &str) -> Vec<&'a str> {
    let lower = html.to_ascii_lowercase();
    let open = format!("<{name}");
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(found) = lower[pos..].find(&open) {
        let start = pos + found;
        let end = tag_end(html, start);
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
        let head_end = tag_end(block, 0);
        let head = &block[..head_end];
        let mut form = Form {
            id: attr(head, "id"),
            action: attr(head, "action"),
            method: attr(head, "method").unwrap_or_else(|| "get".into()).to_ascii_lowercase(),
            fields: Vec::new(),
            html: block.to_owned(),
        };
        // A named submit button sends its value, like the browser does when
        // the user clicks it ("method_free" on XFileSharing sites).
        for button in tags(block, "button") {
            let kind = attr(button, "type").unwrap_or_else(|| "submit".into()).to_ascii_lowercase();
            if let (true, Some(name)) = (kind == "submit", attr(button, "name")) {
                if form.field(&name).is_none() {
                    form.fields.push((name, attr(button, "value").unwrap_or_default()));
                }
            }
        }
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

/// Whether an element of the HTML has this CSS class. Scripts that only
/// mention the class (`querySelector('.cf-turnstile')`) do not count.
fn has_class(html: &str, class: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(pos) = lower[from..].find("class=") {
        let start = from + pos + 6;
        from = start;
        let rest = &lower[start..];
        let value = match rest.chars().next() {
            Some(q @ ('"' | '\'')) => rest[1..].split(q).next().unwrap_or_default(),
            _ => rest.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or_default(),
        };
        if value.split_whitespace().any(|c| c == class) {
            return true;
        }
    }
    false
}

/// Kind of captcha present in a piece of HTML (best given the download form
/// only, not the whole page), if any. "code" is a code or image to copy,
/// which the user can answer inside the app; the others are widgets that
/// only work in a browser.
pub fn captcha_on(html: &str) -> Option<&'static str> {
    if has_class(html, "g-recaptcha") {
        Some("reCAPTCHA")
    } else if has_class(html, "h-captcha") {
        Some("hCaptcha")
    } else if has_class(html, "cf-turnstile") {
        Some("Turnstile")
    } else if forms(html).iter().any(|f| f.field("code").is_some()) && code_challenge(html).is_some() {
        Some("code")
    } else if html.to_ascii_lowercase().contains("api.solvemedia.com") {
        Some("SolveMedia")
    } else {
        None
    }
}

// ------------------------------------------------------------------ captchas

/// What the user has to read to answer a code captcha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Challenge {
    /// Image bytes (PNG, JPEG or GIF).
    Image(Vec<u8>),
    /// Characters placed at a horizontal offset in pixels, as the site
    /// draws them (XFileSharing "code" captcha). Shown as is, never read
    /// for the user.
    Placed(Vec<(u32, char)>),
}

/// A captcha the user is asked to answer in the app.
#[derive(Debug, Clone)]
pub struct CaptchaRequest {
    pub hoster: String,
    pub page: Url,
    pub challenge: Challenge,
    pub timeout: Duration,
}

pub type CaptchaFn =
    Arc<dyn Fn(CaptchaRequest) -> futures_util::future::BoxFuture<'static, Option<String>> + Send + Sync>;

/// How to ask the user for a captcha answer, and how long to wait for it.
#[derive(Clone)]
pub struct CaptchaAsk {
    pub ask: CaptchaFn,
    pub timeout: Duration,
}

/// Where the code captcha of a form comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CodeSource {
    Image(String),
    Placed(Vec<(u32, char)>),
}

fn code_challenge(html: &str) -> Option<CodeSource> {
    for img in tags(html, "img") {
        if let Some(src) = attr(img, "src").filter(|s| s.to_ascii_lowercase().contains("captcha")) {
            return Some(CodeSource::Image(src));
        }
    }
    // Characters in absolutely placed spans: <span style="position:absolute;padding-left:24px">&#51;</span>
    let lower = html.to_ascii_lowercase();
    let mut placed = Vec::new();
    let mut pos = 0;
    while let Some(found) = lower[pos..].find("<span") {
        let start = pos + found;
        let head_end = lower[start..].find('>').map(|e| start + e + 1).unwrap_or(lower.len());
        let head = &lower[start..head_end];
        pos = head_end;
        let Some(pad) = head.find("padding-left:") else { continue };
        let digits: String = head[pad + 13..].trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
        let Ok(x) = digits.parse::<u32>() else { continue };
        let Some(close) = lower[head_end..].find("</span>") else { continue };
        let inner = decode_entities(html[head_end..head_end + close].trim());
        let mut chars = inner.chars();
        if let (Some(ch), None) = (chars.next(), chars.next()) {
            placed.push((x, ch));
        }
    }
    (!placed.is_empty()).then_some(CodeSource::Placed(placed))
}

/// Decodes `&#NN;` / `&#xNN;` character references and the usual entities.
fn decode_entities(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(pos) = rest.find("&#") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 2..];
        let Some(end) = after.find(';') else {
            out.push_str(&rest[pos..]);
            return decode_html(&out);
        };
        let code = &after[..end];
        let value = match code.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => code.parse().ok(),
        };
        match value.and_then(char::from_u32) {
            Some(ch) => out.push(ch),
            None => out.push_str(&rest[pos..pos + 3 + end]),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    decode_html(&out)
}

/// Seconds of the wait timer shown on a page (XFileSharing style).
pub fn countdown(html: &str) -> Option<u64> {
    let lower = html.to_ascii_lowercase();
    for marker in [
        "countdown=\"",
        "id=\"countdown_str\"",
        "class=\"seconds\"",
        "id=\"countdown\"",
        "id=\"seconds\"",
        "var countdown =",
    ] {
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
pub async fn onefichier(url: &Url, slot: &Arc<ControlSlot>, reconnect: Option<&Reconnect>) -> FreeResult<Resolved> {
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
        let reconnected = match reconnect {
            Some(r) if minutes >= r.after_minutes => {
                slot.set_status(Some("Getting a new IP…"));
                (r.run)().await
            }
            _ => false,
        };
        if !reconnected {
            wait_seconds(slot, minutes * 60 + 5, "Free mode, 1fichier asks to wait").await?;
        }
        return Box::pin(onefichier(url, slot, None)).await;
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
pub async fn xfilesharing(
    url: &Url,
    slot: &Arc<ControlSlot>,
    via: &str,
    captcha: Option<&CaptchaAsk>,
) -> FreeResult<Resolved> {
    let client = session_client();
    let mut get = client.get(url.clone());
    if url.host_str().is_some_and(|h| h.ends_with("datanodes.to")) {
        // Simple referer protection (same trick as JDownloader).
        get = get.header(REFERER, "https://datanodes.to/users");
    }
    let response = get.send().await.map_err(other)?;
    if matches!(response.status().as_u16(), 404 | 410) {
        return Err(FreeError::Offline(format!("HTTP {}", response.status().as_u16())));
    }
    let page = response.url().clone();
    let mut html = response.text().await.map_err(other)?;
    if looks_offline(&html) {
        return Err(FreeError::Offline("removed".into()));
    }
    let mut file_id = page.path_segments().and_then(|mut s| s.next()).unwrap_or_default().to_owned();
    let mut first_pages = 0;
    for step in 0..5 {
        // Already a direct link on the page?
        if let Some(link) = file_link(&page, &html, page.host_str().unwrap_or_default()) {
            return Ok(direct(link, via));
        }
        let found = forms(&html).into_iter().find(|f| {
            matches!(f.field("op"), Some("download1" | "download2" | "download3"))
                || f.id.as_deref() == Some("downloadForm")
        });
        let form = match found.or_else(|| script_download2(&html, &page, &file_id)) {
            Some(form) => form,
            // Only a real captcha widget on the page sends the user to the
            // browser; scripts merely mentioning one do not.
            None if captcha_on(&html).is_some() => return Err(FreeError::Captcha(url.clone())),
            None => return Err(other(format!("{via}: page not recognised"))),
        };
        if let Some(id) = form.field("id").filter(|id| !id.is_empty()) {
            file_id = id.to_owned();
        }
        let mut fields = form.fields.clone();
        if form.field("op") == Some("download1") {
            // The site keeps showing the first page: it wants checks only a
            // real browser passes, so the user finishes it there.
            first_pages += 1;
            if first_pages > 2 {
                tracing::info!(hoster = via, "free step refused, handing the page to the browser");
                return Err(FreeError::Captcha(url.clone()));
            }
            // Choose the free download.
            fields.retain(|(k, _)| k != "method_premium");
            if !fields.iter().any(|(k, _)| k == "method_free") {
                fields.push(("method_free".into(), "Free Download".into()));
            }
            // The free button only becomes clickable after a few seconds
            // (datanodes): wait like a visitor would.
            if free_button_disabled(&form.html) {
                wait_seconds(slot, 10, &format!("Free mode, {via} wait")).await?;
            }
        } else {
            let wait = countdown(&html);
            match captcha_on(&form.html) {
                None => {
                    if let Some(seconds) = wait {
                        wait_seconds(slot, seconds + 1, &format!("Free mode, {via} wait")).await?;
                    }
                }
                Some("code") => {
                    let Some(ask) = captcha else { return Err(FreeError::Captcha(url.clone())) };
                    let challenge = match code_challenge(&form.html) {
                        Some(CodeSource::Placed(chars)) => Challenge::Placed(chars),
                        Some(CodeSource::Image(src)) => {
                            let src = page.join(&src).map_err(other)?;
                            let bytes = client
                                .get(src)
                                .header(REFERER, page.as_str())
                                .send()
                                .await
                                .map_err(other)?
                                .bytes()
                                .await
                                .map_err(other)?;
                            Challenge::Image(bytes.to_vec())
                        }
                        None => return Err(FreeError::Captcha(url.clone())),
                    };
                    let request = CaptchaRequest {
                        hoster: via.trim_end_matches(" (free)").to_owned(),
                        page: page.clone(),
                        challenge,
                        timeout: ask.timeout,
                    };
                    // The user types the code while the site's timer runs.
                    let answer = tokio::spawn(tokio::time::timeout(ask.timeout, (ask.ask)(request)));
                    if let Some(seconds) = wait {
                        wait_seconds(slot, seconds + 1, &format!("Free mode, {via} wait")).await?;
                    }
                    slot.set_status(Some("Captcha: waiting for your answer"));
                    let code = await_answer(answer, slot).await?;
                    slot.set_status(None);
                    fields.retain(|(k, _)| k != "code");
                    fields.push(("code".into(), code));
                }
                Some(kind) => {
                    tracing::info!(hoster = via, captcha = kind, "free mode needs a browser captcha");
                    return Err(FreeError::Captcha(url.clone()));
                }
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
            || response.headers().get(reqwest::header::CONTENT_TYPE).is_some_and(|v| {
                let kind = v.to_str().unwrap_or_default();
                !kind.starts_with("text/") && !kind.contains("json")
            });
        if is_file {
            return Ok(direct(final_url, via));
        }
        html = response.text().await.map_err(other)?;
        // Some sites (datanodes) answer the last step with {"url": "..."}.
        if let Some(link) = json_link(&html, &page) {
            return Ok(direct(link, via));
        }
        if html.to_ascii_lowercase().contains("wrong captcha") {
            tracing::info!(hoster = via, "wrong captcha, asking again");
        }
        tracing::debug!(step, hoster = via, "free mode step done");
    }
    Err(other(format!("{via}: no download link after the wait")))
}

fn free_button_disabled(form_html: &str) -> bool {
    tags(form_html, "button").iter().chain(tags(form_html, "input").iter()).any(|tag| {
        attr(tag, "name").as_deref() == Some("method_free") && {
            let lower = tag.to_ascii_lowercase();
            lower.contains(" disabled") && !lower.contains("disabled=\"false\"")
        }
    })
}

/// Second step built by JavaScript (datanodes): the page only carries the
/// timer (`countdown="30"`) and a `rand` value; the form is the usual one.
fn script_download2(html: &str, page: &Url, file_id: &str) -> Option<Form> {
    if !html.contains("countdown=\"") || file_id.is_empty() {
        return None;
    }
    let rand = html.find("rand=\"").map(|p| html[p + 6..].split('"').next().unwrap_or_default().to_owned());
    Some(Form {
        method: "post".into(),
        fields: vec![
            ("op".into(), "download2".into()),
            ("id".into(), file_id.to_owned()),
            ("rand".into(), rand.unwrap_or_default()),
            ("referer".into(), page.as_str().to_owned()),
            ("method_free".into(), "Free Download >>".into()),
            ("method_premium".into(), String::new()),
        ],
        ..Form::default()
    })
}

/// `{"url": "https%3A%2F%2F..."}` answers.
fn json_link(text: &str, page: &Url) -> Option<Url> {
    let value: Value = serde_json::from_str(text.trim()).ok()?;
    let raw = value.get("url").and_then(Value::as_str)?;
    let decoded = if raw.contains("%3A") || raw.contains("%2F") {
        url::form_urlencoded::parse(format!("u={raw}").as_bytes()).next().map(|(_, v)| v.into_owned())?
    } else {
        raw.to_owned()
    };
    page.join(&decoded).ok().filter(|u| u.scheme().starts_with("http"))
}

/// Waits for the user's captcha answer, giving up when the download is
/// cancelled.
async fn await_answer(
    mut answer: tokio::task::JoinHandle<Result<Option<String>, tokio::time::error::Elapsed>>,
    slot: &Arc<ControlSlot>,
) -> FreeResult<String> {
    loop {
        tokio::select! {
            done = &mut answer => {
                return match done {
                    Ok(Ok(Some(code))) if !code.trim().is_empty() => Ok(code.trim().to_owned()),
                    Ok(Err(_)) => Err(other("captcha not answered in time")),
                    _ => Err(other("captcha skipped")),
                };
            }
            _ = tokio::time::sleep(Duration::from_millis(500)) => {
                if slot.is_cancelled() {
                    answer.abort();
                    return Err(other("cancelled"));
                }
            }
        }
    }
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
        let dn = r#"<form method="POST" action='' id="downloadForm"><input type="hidden" name="op" value="download1">
            <button class="btn" type="submit" id="method_free" name="method_free" disabled value="Free Download >>">Continue</button></form>"#;
        let form = &forms(dn)[0];
        assert_eq!(form.id.as_deref(), Some("downloadForm"));
        assert_eq!(form.field("method_free"), Some("Free Download >>"));
        assert!(free_button_disabled(&form.html));
        assert!(!free_button_disabled(XFS_PAGE));
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
        // A script that only looks for a widget is not a captcha (datanodes).
        assert_eq!(captcha_on("<script>if (f.querySelector('.cf-turnstile')) {}</script>"), None);
        assert_eq!(captcha_on(r#"<button class="btn g-recaptcha" data-sitekey="k">"#), Some("reCAPTCHA"));
        let code_form = r#"<form method="POST"><input name="op" value="download2"><input name="code" value="">
            <div><span style='position:absolute;padding-left:46px;padding-top:6px;'>&#55;</span>
            <span style='position:absolute;padding-left:6px;padding-top:4px;'>&#49;</span></div></form>"#;
        assert_eq!(captcha_on(code_form), Some("code"));
        assert_eq!(code_challenge(code_form), Some(CodeSource::Placed(vec![(46, '7'), (6, '1')])));
        let image_form = r#"<form><input name="code"><img src="/captchas/a1b2.jpg"></form>"#;
        assert_eq!(code_challenge(image_form), Some(CodeSource::Image("/captchas/a1b2.jpg".into())));
        assert_eq!(countdown(r#"<span class="timer" countdown="30" rand="xyz">"#), Some(30));
        assert_eq!(countdown(r#"<span id="countdown_str">Wait <span id="cx">45</span> seconds</span>"#), Some(45));
        assert_eq!(countdown("no timer"), None);
    }

    #[test]
    fn builds_the_script_step_and_reads_json_links() {
        let page = Url::parse("https://datanodes.to/abc123def456/file.rar").unwrap();
        let form = script_download2(r#"<div countdown="15" rand="r4nd">"#, &page, "abc123def456").unwrap();
        assert_eq!(form.field("op"), Some("download2"));
        assert_eq!(form.field("id"), Some("abc123def456"));
        assert_eq!(form.field("rand"), Some("r4nd"));
        assert!(script_download2("<p>nothing</p>", &page, "abc").is_none());
        assert_eq!(
            json_link(r#"{"url":"https%3A%2F%2Fcdn.example.net%2Fd%2Ffile.rar"}"#, &page).unwrap().as_str(),
            "https://cdn.example.net/d/file.rar"
        );
        assert!(json_link("<html>", &page).is_none());
    }

    #[test]
    fn finds_direct_links() {
        let page = Url::parse("https://uploady.io/abc123").unwrap();
        let html = r#"<a href="https://s3.uploady.io/d/xyz/My.Movie.mkv" class="btn btn-download">Download</a>"#;
        assert_eq!(file_link(&page, html, "uploady.io").unwrap().as_str(), "https://s3.uploady.io/d/xyz/My.Movie.mkv");
        assert!(file_link(&page, r#"<a href="/premium.html">Download faster</a>"#, "uploady.io").is_none());
    }
}

/// What an unknown link points to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sniff {
    File,
    /// A file page of a site built on XFileSharing.
    Xfs,
    /// Some other web page.
    Page,
    /// Could not tell (network error): try it as a file.
    Unknown,
}

/// Looks at an unknown link like a browser would.
pub async fn sniff(url: &Url) -> Sniff {
    let client = session_client();
    let Ok(response) = client.get(url.clone()).send().await else { return Sniff::Unknown };
    let is_html = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .is_some_and(|v| v.to_str().unwrap_or_default().starts_with("text/html"));
    if !is_html {
        return Sniff::File;
    }
    let Ok(html) = response.text().await else { return Sniff::Unknown };
    if is_xfs_page(&html) {
        Sniff::Xfs
    } else {
        Sniff::Page
    }
}

/// A file page with the XFileSharing download form.
pub fn is_xfs_page(html: &str) -> bool {
    forms(html).iter().any(|f| matches!(f.field("op"), Some("download1" | "download2" | "download3")))
}
