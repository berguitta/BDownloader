//! BitTorrent and magnet links through librqbit, next to the DDL queue.

use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bd_net::{Progress, SegmentProgress};
use futures_util::future::BoxFuture;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ListenerOptions, ManagedTorrent, Session, SessionOptions,
};
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::task::{ControlSlot, RunError, RunResult, TaskControl};

/// Whether a link is a magnet or a .torrent file.
pub fn is_torrent_url(url: &Url) -> bool {
    url.scheme() == "magnet" || url.path().to_ascii_lowercase().ends_with(".torrent")
}

/// Name given in a magnet link (`dn=`), if any.
pub fn magnet_name(url: &Url) -> Option<String> {
    (url.scheme() == "magnet")
        .then(|| url.query_pairs().find(|(k, _)| k == "dn").map(|(_, v)| v.into_owned()))
        .flatten()
}

#[derive(Clone, Debug, PartialEq)]
pub struct TorrentOptions {
    pub port: u16,
    /// Upload limit in KiB/s, 0 for none.
    pub upload_kib: u32,
    /// Keep seeding until uploaded / size reaches this ratio (0 = stop at once).
    pub ratio: f64,
}

impl Default for TorrentOptions {
    fn default() -> Self {
        TorrentOptions { port: 51413, upload_kib: 2048, ratio: 1.0 }
    }
}

/// One librqbit session shared by every torrent job, created on first use.
pub struct TorrentBackend {
    options: TorrentOptions,
    session: OnceCell<Arc<Session>>,
    base_dir: PathBuf,
}

impl TorrentBackend {
    pub fn new(options: TorrentOptions, base_dir: PathBuf) -> Arc<Self> {
        Arc::new(TorrentBackend { options, session: OnceCell::new(), base_dir })
    }

    pub fn options(&self) -> &TorrentOptions {
        &self.options
    }

    async fn session(&self) -> Result<Arc<Session>, String> {
        self.session
            .get_or_try_init(|| async {
                let options = SessionOptions {
                    listen: Some(ListenerOptions {
                        listen_addr: SocketAddr::from(([0, 0, 0, 0], self.options.port)),
                        ..Default::default()
                    }),
                    ratelimits: librqbit::limits::LimitsConfig {
                        upload_bps: NonZeroU32::new(self.options.upload_kib.saturating_mul(1024)),
                        download_bps: None,
                    },
                    client_name_and_version: Some(concat!("BDownloader ", env!("CARGO_PKG_VERSION")).to_owned()),
                    ..Default::default()
                };
                Session::new_with_opts(self.base_dir.clone(), options).await.map_err(|e| e.to_string())
            })
            .await
            .cloned()
    }

    pub fn run(self: &Arc<Self>, url: Url, dest_dir: PathBuf, slot: Arc<ControlSlot>) -> BoxFuture<'static, RunResult> {
        let backend = self.clone();
        Box::pin(async move {
            slot.set_status(Some("Connecting to peers…"));
            let session = backend.session().await.map_err(RunError::Failed)?;
            tokio::fs::create_dir_all(&dest_dir).await.map_err(|e| RunError::Failed(e.to_string()))?;
            let added = session
                .add_torrent(
                    AddTorrent::from_url(url.as_str()),
                    Some(AddTorrentOptions {
                        output_folder: Some(dest_dir.to_string_lossy().into_owned()),
                        overwrite: true,
                        ..Default::default()
                    }),
                )
                .await
                .map_err(|e| RunError::Failed(e.to_string()))?;
            let handle = match added {
                AddTorrentResponse::Added(_, handle) | AddTorrentResponse::AlreadyManaged(_, handle) => handle,
                AddTorrentResponse::ListOnly(_) => return Err(RunError::Failed("torrent could not be started".into())),
            };
            slot.set_status(None);
            let control = Arc::new(TorrentControl {
                handle: handle.clone(),
                cancel: CancellationToken::new(),
                last: Mutex::default(),
            });
            slot.set(control.clone());

            tokio::select! {
                _ = control.cancel.cancelled() => {
                    let _ = session.pause(&handle).await;
                    return Err(RunError::Cancelled);
                }
                done = handle.wait_until_completed() => done.map_err(|e| RunError::Failed(e.to_string()))?,
            }

            // Seed until the wanted ratio, then leave the swarm (files stay).
            let ratio = backend.options.ratio;
            loop {
                let stats = handle.stats();
                if ratio <= 0.0
                    || stats.total_bytes == 0
                    || stats.uploaded_bytes as f64 >= ratio * stats.total_bytes as f64
                {
                    break;
                }
                slot.set_status(Some("Seeding"));
                tokio::select! {
                    _ = control.cancel.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                }
            }
            let name = handle.name().unwrap_or_else(|| "torrent".to_owned());
            let _ = session.delete(handle.id().into(), false).await;
            Ok(dest_dir.join(name))
        })
    }
}

struct TorrentControl {
    handle: Arc<ManagedTorrent>,
    cancel: CancellationToken,
    last: Mutex<Progress>,
}

impl TaskControl for TorrentControl {
    fn progress(&self) -> Progress {
        let stats = self.handle.stats();
        let mut p = self.last.lock().unwrap_or_else(|e| e.into_inner());
        p.file_name = self.handle.name().or(p.file_name.take());
        p.total = (stats.total_bytes > 0).then_some(stats.total_bytes);
        p.downloaded = stats.progress_bytes;
        p.segments = vec![SegmentProgress { done: stats.progress_bytes, len: p.total }];
        p.active_connections = stats.live.as_ref().map(|l| l.snapshot.peer_stats.live as usize).unwrap_or(0);
        p.clone()
    }

    fn cancel(&self) {
        self.cancel.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_torrent_links() {
        let magnet = Url::parse("magnet:?xt=urn:btih:abc&dn=debian-13.iso").unwrap();
        assert!(is_torrent_url(&magnet));
        assert_eq!(magnet_name(&magnet).as_deref(), Some("debian-13.iso"));
        assert!(is_torrent_url(&Url::parse("https://example.com/x.torrent").unwrap()));
        assert!(!is_torrent_url(&Url::parse("https://example.com/x.zip").unwrap()));
    }
}
