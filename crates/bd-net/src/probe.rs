use reqwest::header::{HeaderMap, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, ETAG, LAST_MODIFIED, RANGE};
use reqwest::StatusCode;
use url::Url;

use crate::name::{file_name_from_disposition, file_name_from_url};
use crate::{Error, Result};

/// What the server tells us about a file before downloading it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFile {
    /// Final URL after redirects.
    pub url: Url,
    /// Size in bytes, when announced.
    pub size: Option<u64>,
    /// Whether the server honours `Range` requests.
    pub ranges: bool,
    /// Name suggested by the server or derived from the URL.
    pub file_name: Option<String>,
    /// Validators used to detect that the remote file changed between runs.
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

/// Asks for the first byte only. Unlike `HEAD`, which many hosters answer
/// badly, this tells us reliably whether ranges work and what the size is.
pub(crate) async fn probe(client: &reqwest::Client, url: &Url) -> Result<RemoteFile> {
    let response = client.get(url.clone()).header(RANGE, "bytes=0-0").send().await?;
    let status = response.status();
    if !status.is_success() {
        return Err(Error::Status(status.as_u16()));
    }
    let headers = response.headers().clone();
    let final_url = response.url().clone();
    let ranges = status == StatusCode::PARTIAL_CONTENT;
    let size = if ranges { total_from_content_range(&headers) } else { header_u64(&headers, CONTENT_LENGTH) };
    // Do not download the body of a 200 answer: drop the connection.
    drop(response);

    let file_name = headers
        .get(CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .and_then(file_name_from_disposition)
        .or_else(|| file_name_from_url(&final_url))
        .or_else(|| file_name_from_url(url));

    Ok(RemoteFile {
        url: final_url,
        size,
        ranges,
        file_name,
        etag: header_string(&headers, ETAG),
        last_modified: header_string(&headers, LAST_MODIFIED),
    })
}

fn header_string(headers: &HeaderMap, name: reqwest::header::HeaderName) -> Option<String> {
    headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned)
}

fn header_u64(headers: &HeaderMap, name: reqwest::header::HeaderName) -> Option<u64> {
    headers.get(name)?.to_str().ok()?.trim().parse().ok()
}

/// Parses the total size from `Content-Range: bytes 0-0/12345`.
fn total_from_content_range(headers: &HeaderMap) -> Option<u64> {
    let value = headers.get(CONTENT_RANGE)?.to_str().ok()?;
    let total = value.rsplit('/').next()?.trim();
    total.parse().ok()
}
