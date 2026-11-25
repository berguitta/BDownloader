//! Click'n'Load: a tiny listener on 127.0.0.1:9666 that link sites use to
//! send links to a download manager. It only accepts local connections and
//! links always go to the link grabber, never straight to the queue.

use std::net::SocketAddr;
use std::sync::Arc;

use aes::cipher::{BlockModeDecrypt, KeyIvInit};
use base64::Engine as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::links::extract_links;

pub const DEFAULT_PORT: u16 = 9666;
const MAX_REQUEST: usize = 1024 * 1024;

/// Links received from a site, with the package name it suggested.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Received {
    pub links: Vec<Url>,
    pub package: Option<String>,
    pub source: Option<String>,
    pub passwords: Option<String>,
}

pub type OnLinks = Arc<dyn Fn(Received) + Send + Sync>;

/// Runs until `cancel` fires. Fails if the port is taken (for example by
/// JDownloader running at the same time).
pub async fn serve(port: u16, on_links: OnLinks, cancel: CancellationToken) -> std::io::Result<()> {
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).await?;
    tracing::info!(port, "Click'n'Load listening");
    loop {
        let (socket, peer) = tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            accepted = listener.accept() => accepted?,
        };
        if !peer.ip().is_loopback() {
            continue;
        }
        let on_links = on_links.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(socket, on_links).await {
                tracing::debug!(error = %e, "Click'n'Load request failed");
            }
        });
    }
}

async fn handle(mut socket: TcpStream, on_links: OnLinks) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let (head_end, content_length) = loop {
        let n = socket.read(&mut tmp).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..pos]).to_ascii_lowercase();
            let length = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            break (pos + 4, length.min(MAX_REQUEST));
        }
        if buf.len() > MAX_REQUEST {
            return Ok(());
        }
    };
    while buf.len() < head_end + content_length {
        let n = socket.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let body = &buf[head_end..buf.len().min(head_end + content_length)];
    let mut first = head.lines().next().unwrap_or_default().split_whitespace();
    let (method, path) = (first.next().unwrap_or_default(), first.next().unwrap_or("/"));
    let path = path.split('?').next().unwrap_or("/").trim_end_matches('/');

    let (content_type, response) = match (method, path) {
        ("GET", "/jdcheck.js") => ("text/javascript", "jdownloader=true;\nvar version='18507';\n".to_owned()),
        ("GET", "/crossdomain.xml") => (
            "text/xml",
            "<?xml version=\"1.0\"?>\n<cross-domain-policy><allow-access-from domain=\"*\" /></cross-domain-policy>\n".to_owned(),
        ),
        ("GET", "" | "/flash") => ("text/plain", "JDownloader\r\n".to_owned()),
        ("POST", "/flash/add" | "/flash/addcnl") => {
            let form = parse_form(body);
            let links = extract_links(&field(&form, "urls"));
            deliver(&on_links, links, &form);
            ("text/plain", "success\r\n".to_owned())
        }
        ("POST", "/flash/addcrypted2") => {
            let form = parse_form(body);
            match decrypt(&field(&form, "crypted"), &field(&form, "jk")) {
                Some(text) => {
                    deliver(&on_links, extract_links(&text), &form);
                    ("text/plain", "success\r\n".to_owned())
                }
                None => ("text/plain", "failed\r\n".to_owned()),
            }
        }
        _ => ("text/plain", "not found\r\n".to_owned()),
    };
    let reply = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{response}",
        response.len()
    );
    socket.write_all(reply.as_bytes()).await?;
    socket.shutdown().await
}

fn deliver(on_links: &OnLinks, links: Vec<Url>, form: &[(String, String)]) {
    if links.is_empty() {
        return;
    }
    let opt = |k: &str| Some(field(form, k)).filter(|v| !v.is_empty());
    on_links(Received { links, package: opt("package"), source: opt("source"), passwords: opt("passwords") });
}

fn parse_form(body: &[u8]) -> Vec<(String, String)> {
    url::form_urlencoded::parse(body).map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
}

fn field(form: &[(String, String)], key: &str) -> String {
    form.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()).unwrap_or_default()
}

/// `jk` is a small JavaScript function returning the key as 32 hex digits,
/// e.g. `function f(){ return '31323334...';}`. The key is also the IV.
fn decrypt(crypted: &str, jk: &str) -> Option<String> {
    let key_hex = jk
        .split(['\'', '"'])
        .find(|s| s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit()))?;
    let key: Vec<u8> = (0..16).map(|i| u8::from_str_radix(&key_hex[i * 2..i * 2 + 2], 16)).collect::<Result<_, _>>().ok()?;
    let mut data = base64::engine::general_purpose::STANDARD.decode(crypted.trim().replace(' ', "+")).ok()?;
    let data_len = data.len() - data.len() % 16;
    data.truncate(data_len);
    let cipher = cbc::Decryptor::<aes::Aes128>::new_from_slices(&key, &key).ok()?;
    let plain = cipher.decrypt_padded::<aes::cipher::block_padding::NoPadding>(&mut data).ok()?;
    Some(String::from_utf8_lossy(plain).trim_end_matches('\0').to_owned())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use aes::cipher::BlockModeEncrypt;

    use super::*;

    fn encrypt(plain: &str, key: &[u8; 16]) -> String {
        let mut data = plain.as_bytes().to_vec();
        data.resize(data.len().div_ceil(16) * 16, 0);
        let len = data.len();
        let cipher = cbc::Encryptor::<aes::Aes128>::new_from_slices(key, key).unwrap();
        let out = cipher.encrypt_padded::<aes::cipher::block_padding::NoPadding>(&mut data, len).unwrap().to_vec();
        base64::engine::general_purpose::STANDARD.encode(out)
    }

    #[test]
    fn decrypts_addcrypted2_payloads() {
        let key = *b"1234567890abcdef";
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        let crypted = encrypt("https://a.example/1.rar\r\nhttps://a.example/2.rar", &key);
        let jk = format!("function f(){{ return '{hex}';}}");
        let text = decrypt(&crypted, &jk).unwrap();
        assert_eq!(extract_links(&text).len(), 2);
        assert!(decrypt(&crypted, "function f(){ return 'nope'; }").is_none());
    }

    #[tokio::test]
    async fn receives_links_over_http() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = received.clone();
        let on_links: OnLinks = Arc::new(move |r| sink.lock().unwrap().push(r));
        let cancel = CancellationToken::new();
        let port = 19666;
        tokio::spawn(serve(port, on_links, cancel.clone()));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let client = reqwest::Client::new();
        let check = client.get(format!("http://127.0.0.1:{port}/jdcheck.js")).send().await.unwrap().text().await.unwrap();
        assert!(check.contains("jdownloader=true"));
        let reply = client
            .post(format!("http://127.0.0.1:{port}/flash/add"))
            .form(&[("urls", "https://a.example/x.zip\nhttps://b.example/y.zip"), ("package", "Pack")])
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(reply.trim(), "success");
        cancel.cancel();
        let got = received.lock().unwrap().clone();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].links.len(), 2);
        assert_eq!(got[0].package.as_deref(), Some("Pack"));
    }
}
