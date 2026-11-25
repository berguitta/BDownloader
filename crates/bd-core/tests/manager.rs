//! Queue behaviour against a local HTTP server.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bd_core::manager::ManagerConfig;
use bd_core::{DownloadManager, JobState};
use bd_net::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

/// Serves `data` at any path, with range support and a small delay per chunk.
async fn server(data: Arc<Vec<u8>>, delay: Duration) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let data = data.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 1024];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&buf).to_ascii_lowercase();
                let total = data.len();
                let (start, end) = head
                    .lines()
                    .find_map(|l| l.strip_prefix("range: bytes="))
                    .map(|r| {
                        let (a, b) = r.trim().split_once('-').unwrap();
                        (
                            a.parse::<usize>().unwrap(),
                            if b.is_empty() { total - 1 } else { b.parse::<usize>().unwrap() },
                        )
                    })
                    .unwrap_or((0, total - 1));
                let header = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{total}\r\nConnection: close\r\n\r\n",
                    end - start + 1
                );
                let _ = socket.write_all(header.as_bytes()).await;
                for chunk in data[start..=end].chunks(16 * 1024) {
                    if socket.write_all(chunk).await.is_err() {
                        return;
                    }
                    tokio::time::sleep(delay).await;
                }
            });
        }
    });
    format!("http://{addr}")
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bd-core-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

async fn wait_until(manager: &DownloadManager, timeout: Duration, done: impl Fn(&[bd_core::JobSnapshot]) -> bool) {
    let start = Instant::now();
    loop {
        let jobs = manager.snapshot();
        if done(&jobs) {
            return;
        }
        assert!(start.elapsed() < timeout, "timed out: {jobs:#?}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn runs_the_queue_with_a_parallel_limit_and_pause() {
    let data = Arc::new((0..600_000u32).map(|i| (i * 7 % 251) as u8).collect::<Vec<u8>>());
    let base = server(data.clone(), Duration::from_millis(2)).await;
    let dir = temp_dir("queue");
    let config = ManagerConfig {
        dest_dir: dir.clone(),
        max_parallel: 2,
        connections: 4,
        multi_connection: true,
        resume: true,
        if_exists: bd_net::IfExists::Rename,
        preallocate: true,
    };
    let manager =
        DownloadManager::new(Engine::new().unwrap().min_segment(64 * 1024), config, tokio::runtime::Handle::current());

    let ids: Vec<_> = (1..=3).map(|i| manager.add(Url::parse(&format!("{base}/file{i}.bin")).unwrap())).collect();
    let states: Vec<JobState> = manager.snapshot().into_iter().map(|j| j.state).collect();
    assert_eq!(states, vec![JobState::Downloading, JobState::Downloading, JobState::Queued]);

    manager.pause(ids[0]);
    wait_until(&manager, Duration::from_secs(5), |jobs| jobs[2].state == JobState::Downloading).await;
    assert_eq!(manager.snapshot()[0].state, JobState::Paused);

    manager.resume(ids[0]);
    wait_until(&manager, Duration::from_secs(30), |jobs| jobs.iter().all(|j| j.state == JobState::Finished)).await;

    for i in 1..=3 {
        assert_eq!(std::fs::read(dir.join(format!("file{i}.bin"))).unwrap(), *data);
    }
    manager.remove(ids[1], true);
    assert_eq!(manager.snapshot().len(), 2);
}
