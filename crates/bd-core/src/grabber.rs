//! Link grabber: links land here first (paste, clipboard, Click'n'Load), get
//! checked online, grouped into packages with their mirrors, and only start
//! once the user picked which files to download.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use bd_net::{sanitize_file_name, Engine};
use futures_util::StreamExt;
use tokio::runtime::Handle;
use url::Url;

use crate::hosters::hoster_of;
use crate::manager::{JobKind, JobSpec};
use crate::packages::{links_in_page, package_name};
use crate::video::{is_video_url, VideoBackend, VideoInfo};

pub type GrabId = u64;

const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkStatus {
    Checking,
    Online,
    /// Hoster link that cannot be checked without an account.
    Unchecked,
    Offline(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrabbedLink {
    pub id: GrabId,
    pub url: Url,
    /// Other links to the same file (same name and size, other hosts).
    pub mirrors: Vec<Url>,
    pub name: String,
    pub size: Option<u64>,
    pub status: LinkStatus,
    pub selected: bool,
    pub package: String,
    /// Where it came from: "Paste", "Clipboard", "Click'n'Load", a page title…
    pub source: String,
    /// Format picker for video sites.
    pub video: Option<VideoInfo>,
    pub video_choice: usize,
}

impl GrabbedLink {
    pub fn host(&self) -> String {
        hoster_of(&self.url).map(str::to_owned).unwrap_or_else(|| self.url.host_str().unwrap_or_default().to_owned())
    }

    pub fn is_video(&self) -> bool {
        is_video_url(&self.url)
    }
}

#[derive(Default)]
struct Inner {
    links: Vec<GrabbedLink>,
    next_id: GrabId,
    revision: u64,
    passwords: Vec<(String, String)>,
}

#[derive(Clone)]
pub struct Grabber {
    inner: Arc<Mutex<Inner>>,
    engine: Engine,
    runtime: Handle,
    video: Arc<Mutex<Option<VideoBackend>>>,
    page_scan: Arc<std::sync::atomic::AtomicBool>,
}

impl Grabber {
    pub fn new(engine: Engine, runtime: Handle) -> Self {
        Grabber {
            inner: Arc::new(Mutex::new(Inner { next_id: 1, ..Inner::default() })),
            engine,
            runtime,
            video: Arc::default(),
            page_scan: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_video_backend(&self, backend: Option<VideoBackend>) {
        *self.video.lock().unwrap_or_else(|e| e.into_inner()) = backend;
    }

    /// `page-scan` module: web page links become pools of file links.
    pub fn set_page_scan(&self, on: bool) {
        self.page_scan.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn revision(&self) -> u64 {
        self.lock().revision
    }

    pub fn snapshot(&self) -> Vec<GrabbedLink> {
        self.lock().links.clone()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().links.is_empty()
    }

    /// Adds links (duplicates are ignored) and checks them in the background.
    /// Returns how many were new.
    pub fn add(&self, urls: Vec<Url>, source: &str) -> usize {
        let ids: Vec<GrabId> = {
            let mut inner = self.lock();
            let mut ids = Vec::new();
            for url in urls {
                if inner.links.iter().any(|l| l.url == url || l.mirrors.contains(&url)) {
                    continue;
                }
                let id = inner.next_id;
                inner.next_id += 1;
                let name =
                    bd_net::file_name_from_url(&url).unwrap_or_else(|| url.host_str().unwrap_or("link").to_owned());
                inner.links.push(GrabbedLink {
                    id,
                    package: package_name(&name),
                    name,
                    url,
                    mirrors: Vec::new(),
                    size: None,
                    status: LinkStatus::Checking,
                    selected: true,
                    source: source.to_owned(),
                    video: None,
                    video_choice: 0,
                });
                ids.push(id);
            }
            inner.revision += 1;
            ids
        };
        for id in &ids {
            let grabber = self.clone();
            let id = *id;
            self.runtime.spawn(async move { grabber.check(id).await });
        }
        ids.len()
    }

    async fn check(&self, id: GrabId) {
        let Some(link) = self.lock().links.iter().find(|l| l.id == id).cloned() else { return };
        let url = link.url.clone();

        if crate::torrent::is_torrent_url(&url) {
            let name = crate::torrent::magnet_name(&url);
            self.apply(id, Update::Online { name, size: None });
            return;
        }

        if crate::decrypt::is_container(&url) {
            match crate::decrypt::expand(&url).await {
                Ok(expanded) if !expanded.links.is_empty() => {
                    self.replace_with_pool(id, expanded.links, &expanded.name)
                }
                Ok(_) => self.apply(id, Update::Offline("empty folder".into())),
                Err(e) => self.apply(id, Update::Offline(e)),
            }
            return;
        }

        if is_video_url(&url) {
            let backend = self.video.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let update = match backend {
                Some(backend) => match backend.info(&url).await {
                    Ok(info) => Update::Video(info),
                    Err(e) => Update::Offline(e),
                },
                None => Update::Offline("video link: turn on the video module and install yt-dlp in Settings".into()),
            };
            self.apply(id, update);
            return;
        }

        if hoster_of(&url).is_some() {
            // Hoster pages answer with HTML: read name and size from the
            // hoster's check API or its page.
            let update = match crate::hosters::check_link(&url).await {
                crate::hosters::LinkCheck::Online { name, size } => Update::Online { name, size },
                crate::hosters::LinkCheck::Offline(why) => Update::Offline(why),
                crate::hosters::LinkCheck::Unknown => Update::Unchecked,
            };
            self.apply(id, update);
            return;
        }

        match self.engine.probe(&url).await {
            Ok(remote) => {
                let is_page = remote.content_type.as_deref().is_some_and(|t| t.starts_with("text/html"));
                if is_page && self.page_scan.load(std::sync::atomic::Ordering::Relaxed) {
                    self.scan_page(id, &remote.url).await;
                } else if is_page {
                    self.apply(id, Update::Offline("web page, not a file (turn on Web page scan)".into()));
                } else {
                    self.apply(id, Update::Online { name: remote.file_name, size: remote.size });
                }
            }
            Err(e) => {
                // Some unknown hosters refuse plain clients: look at the page
                // like a browser before calling the link offline.
                let update = match crate::free::page_info(&url).await {
                    Ok(info) if !info.offline && (info.name.is_some() || info.size.is_some()) => {
                        Update::Online { name: info.name, size: info.size }
                    }
                    _ => Update::Offline(short_error(&e)),
                };
                self.apply(id, update);
            }
        }
    }

    /// A web page: replace it by the file links it contains, as one package.
    async fn scan_page(&self, id: GrabId, page: &Url) {
        let html = match fetch_text(&self.engine, page).await {
            Ok(html) => html,
            Err(e) => return self.apply(id, Update::Offline(e)),
        };
        if crate::free::is_xfs_page(&html) {
            // File page of an unknown hoster built on XFileSharing.
            let update = if crate::free::looks_offline(&html) {
                Update::Offline("file removed".into())
            } else {
                Update::Online { name: crate::free::name_on_page(&html), size: crate::free::size_on_page(&html) }
            };
            return self.apply(id, update);
        }
        let (title, links) = links_in_page(page, &html);
        if links.is_empty() {
            return self.apply(id, Update::Offline("web page without file links".into()));
        }
        let package = title.clone().unwrap_or_else(|| page.host_str().unwrap_or("Page").to_owned());
        self.replace_with_pool(id, links, &package);
    }

    /// Replaces a link (page, folder) by the links it stands for, grouped in
    /// one package.
    fn replace_with_pool(&self, id: GrabId, links: Vec<Url>, package: &str) {
        let package = package.to_owned();
        {
            let mut inner = self.lock();
            inner.links.retain(|l| l.id != id);
            inner.revision += 1;
        }
        let before = self.lock().next_id;
        self.add(links, &package);
        // Keep the page's files together under the page title.
        let mut inner = self.lock();
        for link in inner.links.iter_mut().filter(|l| l.id >= before) {
            link.package = package.clone();
        }
        inner.revision += 1;
    }

    fn apply(&self, id: GrabId, update: Update) {
        let mut inner = self.lock();
        let Some(link) = inner.links.iter_mut().find(|l| l.id == id) else { return };
        let keep_package = link.package != package_name(&link.name);
        match update {
            Update::Online { name, size } => {
                if let Some(name) = name {
                    link.name = name;
                }
                link.size = size;
                link.status = LinkStatus::Online;
            }
            Update::Unchecked => link.status = LinkStatus::Unchecked,
            Update::Video(info) => {
                link.name = info.title.clone();
                link.size = info.choices.first().and_then(|c| c.size);
                link.status = LinkStatus::Online;
                link.video = Some(info);
            }
            Update::Offline(reason) => {
                link.status = LinkStatus::Offline(reason);
                link.selected = false;
            }
        }
        if !keep_package {
            link.package = if link.is_video() { "Videos".to_owned() } else { package_name(&link.name) };
        }
        merge_mirrors(&mut inner.links);
        inner.revision += 1;
    }

    pub fn set_selected(&self, id: GrabId, selected: bool) {
        let mut inner = self.lock();
        if let Some(link) = inner.links.iter_mut().find(|l| l.id == id) {
            link.selected = selected && !matches!(link.status, LinkStatus::Offline(_));
        }
        inner.revision += 1;
    }

    pub fn set_package_selected(&self, package: &str, selected: bool) {
        let mut inner = self.lock();
        for link in inner.links.iter_mut().filter(|l| l.package == package) {
            link.selected = selected && !matches!(link.status, LinkStatus::Offline(_));
        }
        inner.revision += 1;
    }

    pub fn select_all(&self, selected: bool) {
        let mut inner = self.lock();
        for link in inner.links.iter_mut() {
            link.selected = selected && !matches!(link.status, LinkStatus::Offline(_));
        }
        inner.revision += 1;
    }

    pub fn set_video_choice(&self, id: GrabId, choice: usize) {
        let mut inner = self.lock();
        if let Some(link) = inner.links.iter_mut().find(|l| l.id == id) {
            let count = link.video.as_ref().map(|v| v.choices.len()).unwrap_or(0);
            if choice < count {
                link.video_choice = choice;
                link.size = link.video.as_ref().and_then(|v| v.choices[choice].size);
            }
        }
        inner.revision += 1;
    }

    /// Makes another mirror the main link of an entry.
    pub fn use_mirror(&self, id: GrabId, mirror: usize) {
        let mut inner = self.lock();
        if let Some(link) = inner.links.iter_mut().find(|l| l.id == id) {
            if mirror < link.mirrors.len() {
                std::mem::swap(&mut link.url, &mut link.mirrors[mirror]);
            }
        }
        inner.revision += 1;
    }

    pub fn rename_package(&self, old: &str, new: &str) {
        let new = new.trim();
        if new.is_empty() {
            return;
        }
        let mut inner = self.lock();
        for link in inner.links.iter_mut().filter(|l| l.package == old) {
            link.package = new.to_owned();
        }
        inner.revision += 1;
    }

    pub fn set_package_password(&self, package: &str, password: &str) {
        let mut inner = self.lock();
        inner.passwords.retain(|(p, _)| p != package);
        if !password.is_empty() {
            inner.passwords.push((package.to_owned(), password.to_owned()));
        }
        inner.revision += 1;
    }

    pub fn package_password(&self, package: &str) -> Option<String> {
        self.lock().passwords.iter().find(|(p, _)| p == package).map(|(_, pw)| pw.clone())
    }

    pub fn remove(&self, ids: &[GrabId]) {
        let mut inner = self.lock();
        inner.links.retain(|l| !ids.contains(&l.id));
        inner.revision += 1;
    }

    pub fn remove_package(&self, package: &str) {
        let mut inner = self.lock();
        inner.links.retain(|l| l.package != package);
        inner.revision += 1;
    }

    pub fn remove_offline(&self) {
        let mut inner = self.lock();
        inner.links.retain(|l| !matches!(l.status, LinkStatus::Offline(_)));
        inner.revision += 1;
    }

    pub fn clear(&self) {
        let mut inner = self.lock();
        inner.links.clear();
        inner.revision += 1;
    }

    /// Takes the selected links out of the grabber as jobs. Packages with
    /// several files get their own sub-folder of `root`.
    pub fn take_selected(&self, root: &Path, folder_per_package: bool) -> Vec<JobSpec> {
        let mut inner = self.lock();
        let counts = |package: &str, links: &[GrabbedLink]| links.iter().filter(|l| l.package == package).count();
        let mut specs = Vec::new();
        let mut taken = Vec::new();
        for link in inner.links.iter() {
            if !link.selected || matches!(link.status, LinkStatus::Offline(_) | LinkStatus::Checking) {
                continue;
            }
            let dir: PathBuf = if folder_per_package && counts(&link.package, &inner.links) > 1 {
                root.join(sanitize_file_name(&link.package))
            } else {
                root.to_path_buf()
            };
            let kind = match &link.video {
                Some(info) => JobKind::Video { format: info.choices[link.video_choice].format.clone() },
                None if crate::torrent::is_torrent_url(&link.url) => JobKind::Torrent,
                None => JobKind::Http,
            };
            specs.push(JobSpec {
                package: Some(link.package.clone()),
                password: inner.passwords.iter().find(|(p, _)| p == &link.package).map(|(_, pw)| pw.clone()),
                kind,
                ..JobSpec::http(link.url.clone(), dir)
            });
            taken.push(link.id);
        }
        inner.links.retain(|l| !taken.contains(&l.id));
        inner.revision += 1;
        specs
    }
}

enum Update {
    Online { name: Option<String>, size: Option<u64> },
    Unchecked,
    Video(VideoInfo),
    Offline(String),
}

/// Same file on several hosts: keep one entry, list the others as mirrors.
fn merge_mirrors(links: &mut Vec<GrabbedLink>) {
    let mut i = 0;
    while i < links.len() {
        let (name, size, online) = (links[i].name.clone(), links[i].size, links[i].status == LinkStatus::Online);
        if !online || size.is_none() {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < links.len() {
            let same = links[j].status == LinkStatus::Online
                && links[j].name == name
                && links[j].size == size
                && links[j].url.host_str() != links[i].url.host_str();
            if same {
                let other = links.remove(j);
                links[i].mirrors.push(other.url);
                links[i].mirrors.extend(other.mirrors);
            } else {
                j += 1;
            }
        }
        i += 1;
    }
}

async fn fetch_text(engine: &Engine, url: &Url) -> Result<String, String> {
    let response = engine.client().get(url.clone()).send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        body.extend_from_slice(&chunk);
        if body.len() > MAX_PAGE_BYTES {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

fn short_error(e: &bd_net::Error) -> String {
    match e {
        bd_net::Error::Status(404) => "File not found (404)".to_owned(),
        bd_net::Error::Status(403) => "Access denied (403)".to_owned(),
        bd_net::Error::Status(410) => "File removed (410)".to_owned(),
        bd_net::Error::Status(code) => format!("HTTP {code}"),
        bd_net::Error::Http(_) => "Server unreachable".to_owned(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(id: GrabId, url: &str, name: &str, size: Option<u64>) -> GrabbedLink {
        GrabbedLink {
            id,
            url: Url::parse(url).unwrap(),
            mirrors: Vec::new(),
            name: name.into(),
            size,
            status: LinkStatus::Online,
            selected: true,
            package: package_name(name),
            source: "Paste".into(),
            video: None,
            video_choice: 0,
        }
    }

    #[test]
    fn merges_same_file_from_other_hosts() {
        let mut links = vec![
            link(1, "https://a.example/x.rar", "x.rar", Some(10)),
            link(2, "https://b.example/x.rar", "x.rar", Some(10)),
            link(3, "https://a.example/y.rar", "y.rar", Some(10)),
            link(4, "https://c.example/x.rar", "x.rar", Some(11)),
        ];
        merge_mirrors(&mut links);
        assert_eq!(links.len(), 3);
        assert_eq!(links[0].mirrors, vec![Url::parse("https://b.example/x.rar").unwrap()]);
    }

    #[tokio::test]
    async fn takes_selected_links_into_package_folders() {
        let grabber = Grabber::new(Engine::new().unwrap(), Handle::current());
        {
            let mut inner = grabber.lock();
            inner.links = vec![
                link(1, "https://a.example/Movie.part1.rar", "Movie.part1.rar", Some(1)),
                link(2, "https://a.example/Movie.part2.rar", "Movie.part2.rar", Some(1)),
                link(3, "https://a.example/single.iso", "single.iso", Some(1)),
                link(4, "https://a.example/other.zip", "other.zip", Some(1)),
            ];
            inner.links[3].selected = false;
        }
        grabber.set_package_password("Movie", "secret");
        let specs = grabber.take_selected(Path::new("D:/dl"), true);
        assert_eq!(specs.len(), 3);
        assert_eq!(specs[0].dest_dir, Path::new("D:/dl").join("Movie"));
        assert_eq!(specs[0].password.as_deref(), Some("secret"));
        assert_eq!(specs[2].dest_dir, Path::new("D:/dl"));
        assert_eq!(grabber.snapshot().len(), 1, "unselected link stays in the grabber");
    }
}
