//! End-to-end tests against a tiny local HTTP server.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bd_net::{DownloadRequest, Engine, Error};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use url::Url;

#[derive(Clone, Default)]
struct ServerOptions {
    /// Honour `Range` requests.
    ranges: bool,
    /// Pause between 16 KiB chunks, to keep a download running for a while.
    chunk_delay: Option<Duration>,
    /// Number of body responses that get cut after `cut_after` bytes.
    cuts: Arc<AtomicUsize>,
    cut_after: usize,
    /// Answer 429 when more than this many body transfers run at once (0 = no limit).
    max_active: usize,
    active: Arc<AtomicUsize>,
}

struct TestServer {
    addr: SocketAddr,
    requests: Arc<AtomicUsize>,
}

impl TestServer {
    fn url(&self, name: &str) -> Url {
        Url::parse(&format!("http://{}/files/{name}", self.addr)).unwrap()
    }
}

async fn start_server(data: Arc<Vec<u8>>, options: ServerOptions) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = requests.clone();
    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else { break };
            counter.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(handle(socket, data.clone(), options.clone()));
        }
    });
    TestServer { addr, requests }
}

async fn handle(mut socket: TcpStream, data: Arc<Vec<u8>>, options: ServerOptions) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        match socket.read(&mut tmp).await {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }
    let head = String::from_utf8_lossy(&buf).to_string();
    let range = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("range: bytes=").map(str::to_owned));

    let total = data.len();
    let (status, start, end) = match (range, options.ranges) {
        (Some(r), true) => {
            let (a, b) = r.trim().split_once('-').unwrap();
            let start: usize = a.parse().unwrap();
            let end: usize = if b.is_empty() { total - 1 } else { b.parse::<usize>().unwrap().min(total - 1) };
            ("206 Partial Content", start, end)
        }
        _ => ("200 OK", 0, total - 1),
    };
    let mut header =
        format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nETag: \"v1\"\r\nConnection: close\r\n", end - start + 1);
    if status.starts_with("206") {
        header.push_str(&format!("Content-Range: bytes {start}-{end}/{total}\r\nAccept-Ranges: bytes\r\n"));
    }
    header.push_str("\r\n");
    let body = &data[start..=end];
    let _active = ActiveCount::enter(&options.active);
    if options.max_active > 0 && body.len() > 1 && options.active.load(Ordering::SeqCst) > options.max_active {
        let _ =
            socket.write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        return;
    }
    // Probes (one byte) are never cut, real transfers may be.
    let cut =
        body.len() > 1 && options.cuts.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1)).is_ok();
    if socket.write_all(header.as_bytes()).await.is_err() {
        return;
    }
    let limit = if cut { options.cut_after.min(body.len()) } else { body.len() };
    for chunk in body[..limit].chunks(16 * 1024) {
        if socket.write_all(chunk).await.is_err() {
            return;
        }
        if let Some(delay) = options.chunk_delay {
            tokio::time::sleep(delay).await;
        }
    }
    let _ = socket.shutdown().await;
}

fn sample_data(len: usize) -> Arc<Vec<u8>> {
    let mut x: u32 = 0x1234_5678;
    Arc::new(
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect(),
    )
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bd-net-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn engine() -> Engine {
    Engine::new().unwrap().min_segment(64 * 1024).retries(3)
}

#[tokio::test]
async fn downloads_with_several_connections() {
    let data = sample_data(3 * 1024 * 1024 + 123);
    let server = start_server(data.clone(), ServerOptions { ranges: true, ..Default::default() }).await;
    let dir = temp_dir("multi");

    let task = engine().start(DownloadRequest::new(server.url("big.bin"), &dir).connections(6));
    let path = task.wait().await.unwrap();

    assert_eq!(path, dir.join("big.bin"));
    assert_eq!(std::fs::read(&path).unwrap(), *data);
    assert!(!dir.join("big.bin.part").exists());
    assert!(!dir.join("big.bin.part.json").exists());
    // One probe plus six segments.
    assert_eq!(server.requests.load(Ordering::SeqCst), 7);
}

#[tokio::test]
async fn falls_back_to_one_stream_without_ranges() {
    let data = sample_data(700_000);
    let server = start_server(data.clone(), ServerOptions::default()).await;
    let dir = temp_dir("single");

    let path = engine().start(DownloadRequest::new(server.url("plain.bin"), &dir)).wait().await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), *data);
}

#[tokio::test]
async fn resumes_after_cancel() {
    let data = sample_data(2 * 1024 * 1024);
    let options = ServerOptions { ranges: true, chunk_delay: Some(Duration::from_millis(5)), ..Default::default() };
    let server = start_server(data.clone(), options).await;
    let dir = temp_dir("resume");
    let request = DownloadRequest::new(server.url("resume.bin"), &dir).connections(4);

    let task = engine().start(request.clone());
    tokio::time::sleep(Duration::from_millis(300)).await;
    let before = task.progress();
    task.cancel();
    assert!(matches!(task.wait().await, Err(Error::Cancelled)));
    assert!(before.downloaded > 0 && before.downloaded < data.len() as u64, "{before:?}");
    assert!(dir.join("resume.bin.part.json").exists());

    let task = engine().start(request);
    let path = loop {
        if task.is_finished() {
            break task.wait().await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(std::fs::read(path).unwrap(), *data);
}

#[tokio::test]
async fn retries_cut_connections() {
    let data = sample_data(1024 * 1024);
    let options =
        ServerOptions { ranges: true, cuts: Arc::new(AtomicUsize::new(3)), cut_after: 100_000, ..Default::default() };
    let server = start_server(data.clone(), options).await;
    let dir = temp_dir("retry");

    let path = engine().start(DownloadRequest::new(server.url("flaky.bin"), &dir).connections(4)).wait().await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), *data);
}

#[tokio::test]
async fn reports_http_errors() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut tmp = [0u8; 1024];
            let _ = socket.read(&mut tmp).await;
            let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        }
    });
    let url = Url::parse(&format!("http://{addr}/missing.bin")).unwrap();
    let result = engine().start(DownloadRequest::new(url, temp_dir("404"))).wait().await;
    assert!(matches!(result, Err(Error::Status(404))), "{result:?}");
}

struct ActiveCount(Arc<AtomicUsize>);

impl ActiveCount {
    fn enter(counter: &Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        ActiveCount(counter.clone())
    }
}

impl Drop for ActiveCount {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn lowers_connections_when_throttled() {
    let data = sample_data(2 * 1024 * 1024);
    let options = ServerOptions {
        ranges: true,
        max_active: 2,
        chunk_delay: Some(Duration::from_millis(1)),
        ..Default::default()
    };
    let server = start_server(data.clone(), options).await;
    let dir = temp_dir("throttle");

    let path =
        engine().start(DownloadRequest::new(server.url("limited.bin"), &dir).connections(8)).wait().await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), *data);
}
