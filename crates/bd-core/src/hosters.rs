//! Hoster support: recognise hoster links, check them, and turn them into
//! direct links through a debrid service, the hoster's official premium API,
//! or the free mode of the site (wait timers, captchas handed to the user).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::Value;
use url::Url;

use crate::accounts::{service, Account, ServiceKind, SERVICES};
use crate::free::{self, FreeError, PageInfo};
use crate::task::{ControlSlot, LinkResolver, Resolved};

const AGENT: &str = "BDownloader";

/// How the free mode of a hoster works.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Free {
    /// Implemented flow without captcha in the usual case.
    OneFichier,
    XFileSharing,
    MediaFire,
    Gofile,
    GoogleDrive,
    Krakenfiles,
    FuckingFast,
    Pixeldrain,
    Dropbox,
    /// The site always asks for a captcha or uses encryption: the page is
    /// opened in the browser (or a debrid/premium account is used).
    Browser,
}

/// Known file hosters: (name, domains, free mode).
pub const HOSTERS: &[(&str, &[&str], Free)] = &[
    (
        "1fichier",
        &[
            "1fichier.com",
            "alterupload.com",
            "cjoint.net",
            "desfichiers.com",
            "dfichiers.com",
            "megadl.fr",
            "mesfichiers.org",
            "piecejointe.net",
            "pjointe.com",
            "tenvoi.com",
            "dl4free.com",
        ],
        Free::OneFichier,
    ),
    ("Rapidgator", &["rapidgator.net", "rapidgator.asia", "rg.to"], Free::Browser),
    ("Turbobit", &["turbobit.net", "turbobit.cc", "turbo.to", "turb.to", "turbobit.pw", "trbbt.net"], Free::Browser),
    ("Nitroflare", &["nitroflare.com", "nitro.download"], Free::Browser),
    ("Uploaded", &["uploaded.net", "uploaded.to", "ul.to"], Free::Browser),
    ("Hitfile", &["hitfile.net", "hil.to"], Free::Browser),
    ("Mega", &["mega.nz", "mega.co.nz", "mega.io"], Free::Browser),
    ("Uptobox", &["uptobox.com", "uptobox.eu", "uptostream.com"], Free::Browser),
    ("Fileboom", &["fileboom.me", "fboom.me"], Free::Browser),
    ("Keep2Share", &["k2s.cc", "keep2share.cc", "keep2s.cc"], Free::Browser),
    ("Tezfiles", &["tezfiles.com"], Free::Browser),
    ("Filefactory", &["filefactory.com"], Free::Browser),
    ("Wupfile", &["wupfile.com"], Free::XFileSharing),
    ("DataNodes", &["datanodes.to"], Free::XFileSharing),
    ("Filekeeper", &["filekeeper.net"], Free::XFileSharing),
    ("Fastupload", &["fastupload.io"], Free::XFileSharing),
    ("Nelion", &["nelion.me"], Free::XFileSharing),
    ("DDownload", &["ddownload.com", "ddl.to"], Free::XFileSharing),
    ("Katfile", &["katfile.com", "katfile.cloud"], Free::XFileSharing),
    ("Uploady", &["uploady.io", "uploady.to"], Free::XFileSharing),
    ("Dailyuploads", &["dailyuploads.net", "dailyupload.io", "dailyuploads.io"], Free::XFileSharing),
    ("Hexupload", &["hexupload.net", "hexload.com"], Free::XFileSharing),
    ("Uploadrar", &["uploadrar.com", "uploadrar.net"], Free::XFileSharing),
    ("Usersdrive", &["usersdrive.com"], Free::XFileSharing),
    ("Mexashare", &["mexa.sh", "mexashare.com"], Free::XFileSharing),
    ("Drop.download", &["drop.download"], Free::XFileSharing),
    (
        "Clicknupload",
        &["clicknupload.click", "clicknupload.to", "clicknupload.org", "clickndownload.click"],
        Free::XFileSharing,
    ),
    ("Filerio", &["filerio.in"], Free::XFileSharing),
    ("Send.cm", &["send.cm", "send.now"], Free::XFileSharing),
    ("File-upload", &["file-upload.org", "file-upload.com"], Free::XFileSharing),
    ("Upload-4ever", &["upload-4ever.com"], Free::XFileSharing),
    ("Worldbytez", &["worldbytez.com"], Free::XFileSharing),
    ("Frdl", &["frdl.io", "frdl.to"], Free::XFileSharing),
    ("Uploadev", &["uploadev.org"], Free::XFileSharing),
    ("Filespace", &["filespace.com"], Free::XFileSharing),
    ("Rapidcloud", &["rapidcloud.cc"], Free::XFileSharing),
    ("Up-load", &["up-load.io"], Free::XFileSharing),
    ("Userload", &["userload.co"], Free::XFileSharing),
    ("Vidoza", &["vidoza.net"], Free::XFileSharing),
    ("Filefox", &["filefox.cc"], Free::XFileSharing),
    ("Isra.cloud", &["isra.cloud"], Free::XFileSharing),
    ("Fikper", &["fikper.com"], Free::Browser),
    ("Mediafire", &["mediafire.com"], Free::MediaFire),
    ("Gofile", &["gofile.io"], Free::Gofile),
    ("Google Drive", &["drive.google.com", "drive.usercontent.google.com"], Free::GoogleDrive),
    ("Krakenfiles", &["krakenfiles.com"], Free::Krakenfiles),
    ("FuckingFast", &["fuckingfast.co"], Free::FuckingFast),
    ("Pixeldrain", &["pixeldrain.com", "pixeldra.in"], Free::Pixeldrain),
    ("Dropbox", &["dropbox.com"], Free::Dropbox),
    ("OneDrive", &["onedrive.live.com", "1drv.ms"], Free::Browser),
    ("Box", &["box.com", "app.box.com"], Free::Browser),
    ("pCloud", &["pcloud.com", "u.pcloud.link"], Free::Browser),
    ("Icedrive", &["icedrive.net"], Free::Browser),
    ("Terabox", &["terabox.com", "1024terabox.com", "teraboxapp.com"], Free::Browser),
    ("Sendspace", &["sendspace.com"], Free::Browser),
    ("Zippyshare", &["zippyshare.com"], Free::Browser),
    ("Depositfiles", &["depositfiles.com", "dfiles.eu"], Free::Browser),
    ("Easybytez", &["easybytez.com"], Free::Browser),
    ("Mixdrop", &["mixdrop.co", "mixdrop.ag"], Free::Browser),
    ("Streamtape", &["streamtape.com"], Free::Browser),
    ("Doodstream", &["dood.la", "doodstream.com", "dood.wf"], Free::Browser),
    ("Filestore", &["filestore.to"], Free::Browser),
    ("Uploadgig", &["uploadgig.com"], Free::Browser),
    ("Alfafile", &["alfafile.net"], Free::Browser),
    ("Ex-load", &["ex-load.com"], Free::Browser),
    ("Prefiles", &["prefiles.com"], Free::Browser),
    ("Darkibox", &["darkibox.com"], Free::Browser),
];

fn entry_of(url: &Url) -> Option<&'static (&'static str, &'static [&'static str], Free)> {
    let host = url.host_str()?.to_ascii_lowercase();
    HOSTERS.iter().find(|(_, domains, _)| domains.iter().any(|d| host == *d || host.ends_with(&format!(".{d}"))))
}

/// Hoster name for a link, if it is a known file hoster.
pub fn hoster_of(url: &Url) -> Option<&'static str> {
    entry_of(url).map(|(name, _, _)| *name)
}

pub fn free_mode_of(url: &Url) -> Option<Free> {
    entry_of(url).map(|(_, _, free)| *free)
}

/// Hosters whose links become direct without any page or account.
fn free_direct(url: &Url) -> Option<Url> {
    match free_mode_of(url)? {
        // https://pixeldrain.com/u/ID → https://pixeldrain.com/api/file/ID
        Free::Pixeldrain => {
            let mut segments = url.path_segments()?;
            match (segments.next(), segments.next()) {
                (Some("u"), Some(id)) if !id.is_empty() => {
                    Url::parse(&format!("https://pixeldrain.com/api/file/{id}?download")).ok()
                }
                (Some("api"), _) => Some(url.clone()),
                _ => None,
            }
        }
        // Dropbox shares become direct downloads with dl=1.
        Free::Dropbox => {
            let mut direct = url.clone();
            let pairs: Vec<(String, String)> =
                url.query_pairs().filter(|(k, _)| k != "dl").map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
            direct.query_pairs_mut().clear().extend_pairs(pairs).append_pair("dl", "1");
            Some(direct)
        }
        _ => None,
    }
}

/// Result of checking a hoster link for the link grabber.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkCheck {
    Online {
        name: Option<String>,
        size: Option<u64>,
    },
    Offline(String),
    /// Could not tell (the page gives nothing reliable).
    Unknown,
}

/// Checks a hoster link without downloading it: free APIs when the hoster
/// has one, otherwise the file page.
pub async fn check_link(url: &Url) -> LinkCheck {
    let result = match free_mode_of(url) {
        Some(Free::OneFichier) => free::onefichier_check(url).await,
        Some(Free::Pixeldrain | Free::Dropbox | Free::GoogleDrive | Free::Gofile) => return LinkCheck::Unknown,
        Some(_) => free::page_info(url).await,
        None => return LinkCheck::Unknown,
    };
    match result {
        Ok(PageInfo { offline: true, .. }) => LinkCheck::Offline("file removed".to_owned()),
        Ok(PageInfo { name: None, size: None, .. }) => LinkCheck::Unknown,
        Ok(PageInfo { name, size, .. }) => LinkCheck::Online { name, size },
        Err(FreeError::Offline(why)) => LinkCheck::Offline(why),
        Err(_) => LinkCheck::Unknown,
    }
}

/// Account details shown in the Accounts screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountCheck {
    pub premium: bool,
    pub summary: String,
}

/// What to do when a free download needs a captcha.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaptchaMode {
    #[default]
    Browser,
    Notify,
    Skip,
}

/// Called with the page, the hoster name and the mode when a captcha can only
/// be done in a browser.
pub type CaptchaHandler = Arc<dyn Fn(Url, String, CaptchaMode) + Send + Sync>;

#[derive(Clone)]
pub struct ResolverOptions {
    pub prefer_debrid: bool,
    pub free_fallback: bool,
    pub captcha: CaptchaMode,
    pub reconnect: Option<free::Reconnect>,
    /// How long the in-app captcha dialog waits for the user.
    pub captcha_timeout: std::time::Duration,
}

impl Default for ResolverOptions {
    fn default() -> Self {
        ResolverOptions {
            prefer_debrid: false,
            free_fallback: true,
            captcha: CaptchaMode::Browser,
            reconnect: None,
            captcha_timeout: std::time::Duration::from_secs(120),
        }
    }
}

/// Resolves links with the user's accounts and options.
pub struct HosterResolver {
    client: reqwest::Client,
    credentials: RwLock<Vec<(Account, String)>>,
    options: RwLock<ResolverOptions>,
    on_captcha: RwLock<Option<CaptchaHandler>>,
    ask_captcha: RwLock<Option<free::CaptchaFn>>,
    tokens: Mutex<HashMap<String, String>>,
}

impl HosterResolver {
    pub fn new(client: reqwest::Client) -> Arc<Self> {
        Arc::new(HosterResolver {
            client,
            credentials: RwLock::default(),
            options: RwLock::default(),
            on_captcha: RwLock::default(),
            ask_captcha: RwLock::default(),
            tokens: Mutex::default(),
        })
    }

    pub fn set_credentials(&self, credentials: Vec<(Account, String)>) {
        *self.credentials.write().unwrap_or_else(|e| e.into_inner()) = credentials;
    }

    pub fn set_options(&self, options: ResolverOptions) {
        *self.options.write().unwrap_or_else(|e| e.into_inner()) = options;
    }

    /// Called with the page URL and the hoster name when a captcha is needed.
    pub fn set_captcha_handler(&self, handler: CaptchaHandler) {
        *self.on_captcha.write().unwrap_or_else(|e| e.into_inner()) = Some(handler);
    }

    /// Shows code and image captchas to the user inside the app.
    pub fn set_captcha_ask(&self, ask: free::CaptchaFn) {
        *self.ask_captcha.write().unwrap_or_else(|e| e.into_inner()) = Some(ask);
    }

    fn captcha_ask(&self, options: &ResolverOptions) -> Option<free::CaptchaAsk> {
        if options.captcha == CaptchaMode::Skip {
            return None;
        }
        let ask = self.ask_captcha.read().unwrap_or_else(|e| e.into_inner()).clone()?;
        Some(free::CaptchaAsk { ask, timeout: options.captcha_timeout })
    }

    fn account_for(&self, service_id: &str) -> Option<(Account, String)> {
        // Hosters and debrid services are part of the app: an enabled account
        // is all it takes.
        service(service_id)?;
        self.credentials
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(a, _)| a.service == service_id)
            .cloned()
    }

    async fn resolve_inner(&self, url: &Url, slot: Arc<ControlSlot>) -> Result<Resolved, String> {
        let Some((hoster, _, free_mode)) = entry_of(url).copied() else {
            // Unknown site: a file, or an unknown XFileSharing hoster page?
            return match free::sniff(url).await {
                free::Sniff::Xfs => {
                    let name = url.host_str().unwrap_or("hoster").to_owned();
                    let options = self.options.read().unwrap_or_else(|e| e.into_inner()).clone();
                    let ask = self.captcha_ask(&options);
                    free::xfilesharing(url, &slot, &format!("{name} (free)"), ask.as_ref())
                        .await
                        .map_err(|e| format!("{name}: {e}"))
                }
                free::Sniff::Page => Err("this link is a web page, not a file".to_owned()),
                _ => Ok(Resolved::direct(url.clone())),
            };
        };
        let options = self.options.read().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(direct) = free_direct(url) {
            return Ok(Resolved { via: format!("{hoster} (free)"), ..Resolved::direct(direct) });
        }

        let mut errors: Vec<String> = Vec::new();
        if options.prefer_debrid {
            match self.try_debrid(url, &slot).await {
                Ok(Some(resolved)) => return Ok(resolved),
                Ok(None) => {}
                Err(e) => errors.push(e),
            }
        }
        match self.try_premium(hoster, url).await {
            Ok(Some(resolved)) => return Ok(resolved),
            Ok(None) => {}
            Err(e) => errors.push(e),
        }
        if !options.prefer_debrid {
            match self.try_debrid(url, &slot).await {
                Ok(Some(resolved)) => return Ok(resolved),
                Ok(None) => {}
                Err(e) => errors.push(e),
            }
        }

        if options.free_fallback {
            slot.set_status(Some("Free mode…"));
            let result = match free_mode {
                Free::OneFichier => free::onefichier(url, &slot, options.reconnect.as_ref()).await,
                Free::XFileSharing => {
                    let ask = self.captcha_ask(&options);
                    free::xfilesharing(url, &slot, &format!("{hoster} (free)"), ask.as_ref()).await
                }
                Free::MediaFire => free::mediafire(url).await,
                Free::Gofile => free::gofile(url).await,
                Free::GoogleDrive => free::google_drive(url).await,
                Free::Krakenfiles => free::krakenfiles(url).await,
                Free::FuckingFast => free::fuckingfast(url).await,
                Free::Pixeldrain | Free::Dropbox => Err(FreeError::Other("this link is not a file link".into())),
                Free::Browser => Err(FreeError::Captcha(url.clone())),
            };
            slot.set_status(None);
            match result {
                Ok(resolved) => return Ok(resolved),
                Err(FreeError::Captcha(page)) => {
                    let message = match options.captcha {
                        CaptchaMode::Browser => {
                            format!("{hoster} free mode needs a check in your browser: the page was opened there")
                        }
                        CaptchaMode::Notify => {
                            format!("{hoster} free mode needs a check in your browser: open the link there")
                        }
                        CaptchaMode::Skip => format!("{hoster} free mode needs a check in a browser: skipped"),
                    };
                    if options.captcha != CaptchaMode::Skip {
                        if let Some(handler) = self.on_captcha.read().unwrap_or_else(|e| e.into_inner()).clone() {
                            handler(page, hoster.to_owned(), options.captcha);
                        }
                    }
                    errors.push(message);
                }
                Err(e) => errors.push(format!("{hoster}: {e}")),
            }
        }

        Err(errors.pop().unwrap_or_else(|| format!("{hoster}: add a premium or debrid account, or turn on free mode")))
    }

    async fn try_premium(&self, hoster: &str, url: &Url) -> Result<Option<Resolved>, String> {
        match hoster {
            "1fichier" => match self.account_for("1fichier") {
                Some((_, key)) => self.onefichier(url, &key).await.map(Some),
                None => Ok(None),
            },
            "Rapidgator" => match self.account_for("rapidgator") {
                Some((account, password)) => self.rapidgator(url, &account.username, &password).await.map(Some),
                None => Ok(None),
            },
            _ => Ok(None),
        }
    }

    async fn try_debrid(&self, url: &Url, slot: &Arc<ControlSlot>) -> Result<Option<Resolved>, String> {
        let mut last = None;
        for svc in SERVICES.iter().filter(|s| s.kind == ServiceKind::Debrid) {
            let Some((account, secret)) = self.account_for(svc.id) else { continue };
            slot.set_status(Some(&format!("{}…", svc.name)));
            let result = match svc.id {
                "real-debrid" => self.real_debrid(url, &secret).await,
                "alldebrid" => self.alldebrid(url, &secret).await,
                "premiumize" => self.premiumize(url, &secret).await,
                "debrid-link" => self.debrid_link(url, &secret).await,
                "torbox" => self.torbox(url, &secret, slot).await,
                "offcloud" => self.offcloud(url, &secret).await,
                "linksnappy" => self.linksnappy(url, &account.username, &secret).await,
                "mega-debrid" => self.mega_debrid(url, &account.username, &secret).await,
                _ => continue,
            };
            slot.set_status(None);
            match result {
                Ok(resolved) => return Ok(Some(resolved)),
                Err(e) => {
                    tracing::info!(service = svc.id, error = %e, "debrid could not unlock the link");
                    last = Some(format!("{}: {e}", svc.name));
                }
            }
        }
        last.map_or(Ok(None), Err)
    }

    async fn json(&self, request: reqwest::RequestBuilder) -> Result<Value, String> {
        let response = request.send().await.map_err(|e| e.to_string())?;
        let status = response.status();
        response.json().await.map_err(|e| format!("invalid answer ({status}): {e}"))
    }

    fn resolved(link: &str, name: Option<&str>, via: &str) -> Result<Resolved, String> {
        Ok(Resolved {
            url: Url::parse(link).map_err(|e| e.to_string())?,
            headers: Vec::new(),
            file_name: name.map(str::to_owned),
            via: via.to_owned(),
        })
    }

    async fn real_debrid(&self, url: &Url, token: &str) -> Result<Resolved, String> {
        let body = self
            .json(
                self.client
                    .post("https://api.real-debrid.com/rest/1.0/unrestrict/link")
                    .bearer_auth(token)
                    .form(&[("link", url.as_str())]),
            )
            .await?;
        let link = body["download"].as_str().ok_or_else(|| api_error(&body, "error"))?;
        Self::resolved(link, body["filename"].as_str(), "Real-Debrid")
    }

    async fn alldebrid(&self, url: &Url, key: &str) -> Result<Resolved, String> {
        let body = self
            .json(self.client.get("https://api.alldebrid.com/v4/link/unlock").query(&[
                ("agent", AGENT),
                ("apikey", key),
                ("link", url.as_str()),
            ]))
            .await?;
        if body["status"] != "success" {
            return Err(api_error(&body["error"], "message"));
        }
        let link =
            body["data"]["link"].as_str().filter(|l| !l.is_empty()).ok_or("AllDebrid is still preparing this link")?;
        Self::resolved(link, body["data"]["filename"].as_str(), "AllDebrid")
    }

    async fn premiumize(&self, url: &Url, key: &str) -> Result<Resolved, String> {
        let body = self
            .json(
                self.client
                    .post("https://www.premiumize.me/api/transfer/directdl")
                    .query(&[("apikey", key)])
                    .form(&[("src", url.as_str())]),
            )
            .await?;
        if body["status"] != "success" {
            return Err(api_error(&body, "message"));
        }
        let first = &body["content"][0];
        let link = first["link"].as_str().ok_or("Premiumize returned no file")?;
        Self::resolved(link, first["path"].as_str().and_then(|p| p.rsplit('/').next()), "Premiumize")
    }

    /// https://debrid-link.com/api_doc/v2
    async fn debrid_link(&self, url: &Url, token: &str) -> Result<Resolved, String> {
        let body = self
            .json(
                self.client
                    .post("https://debrid-link.com/api/v2/downloader/add")
                    .bearer_auth(token)
                    .json(&serde_json::json!({ "url": url.as_str() })),
            )
            .await?;
        if body["success"] != true {
            return Err(api_error(&body, "error"));
        }
        let link = body["value"]["downloadUrl"].as_str().ok_or("Debrid-Link returned no link")?;
        Self::resolved(link, body["value"]["name"].as_str(), "Debrid-Link")
    }

    /// https://api-docs.torbox.app: the file is first fetched by TorBox.
    async fn torbox(&self, url: &Url, token: &str, slot: &Arc<ControlSlot>) -> Result<Resolved, String> {
        let created = self
            .json(
                self.client
                    .post("https://api.torbox.app/v1/api/webdl/createwebdownload")
                    .bearer_auth(token)
                    .form(&[("link", url.as_str())]),
            )
            .await?;
        if created["success"] != true {
            return Err(api_error(&created, "detail"));
        }
        let id = created["data"]["webdownload_id"].as_u64().ok_or("TorBox returned no id")?;
        for _ in 0..120 {
            if slot.is_cancelled() {
                return Err("cancelled".into());
            }
            let link = self
                .json(self.client.get("https://api.torbox.app/v1/api/webdl/requestdl").query(&[
                    ("token", token.to_owned()),
                    ("web_id", id.to_string()),
                    ("zip_link", "false".to_owned()),
                ]))
                .await?;
            if let Some(direct) = link["data"].as_str().filter(|l| l.starts_with("http")) {
                return Self::resolved(direct, None, "TorBox");
            }
            slot.set_status(Some("TorBox is preparing the file…"));
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        Err("TorBox did not finish preparing the file".into())
    }

    /// https://offcloud.com/api
    async fn offcloud(&self, url: &Url, key: &str) -> Result<Resolved, String> {
        let body = self
            .json(
                self.client
                    .post("https://offcloud.com/api/instant/download")
                    .query(&[("key", key)])
                    .form(&[("url", url.as_str())]),
            )
            .await?;
        let link = body["url"].as_str().ok_or_else(|| api_error(&body, "error"))?;
        Self::resolved(link, body["fileName"].as_str(), "Offcloud")
    }

    /// https://linksnappy.com/api
    async fn linksnappy(&self, url: &Url, user: &str, password: &str) -> Result<Resolved, String> {
        let client = free::session_client();
        let login: Value = client
            .get("https://linksnappy.com/api/AUTHENTICATE")
            .query(&[("username", user), ("password", password)])
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        if login["status"] != "OK" {
            return Err(api_error(&login, "error"));
        }
        let request = serde_json::json!({ "link": url.as_str() }).to_string();
        let body: Value = client
            .get("https://linksnappy.com/api/linkgen")
            .query(&[("genLinks", request.as_str())])
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let item = &body["links"][0];
        if item["status"] != "OK" {
            return Err(api_error(item, "error"));
        }
        let link = item["generated"].as_str().ok_or("LinkSnappy returned no link")?;
        Self::resolved(link, item["filename"].as_str(), "LinkSnappy")
    }

    /// https://www.mega-debrid.eu/index.php?page=api
    async fn mega_debrid(&self, url: &Url, user: &str, password: &str) -> Result<Resolved, String> {
        let token = self.mega_debrid_token(user, password).await?;
        let body = self
            .json(
                self.client
                    .post("https://www.mega-debrid.eu/api.php")
                    .query(&[("action", "getLink"), ("token", token.as_str())])
                    .form(&[("link", url.as_str())]),
            )
            .await?;
        if body["response_code"] != "ok" {
            self.tokens.lock().unwrap_or_else(|e| e.into_inner()).remove(&format!("mega-debrid:{user}"));
            return Err(api_error(&body, "response_text"));
        }
        let link = body["debridLink"].as_str().ok_or("Mega-Debrid returned no link")?.trim_matches('"');
        Self::resolved(link, None, "Mega-Debrid")
    }

    async fn mega_debrid_token(&self, user: &str, password: &str) -> Result<String, String> {
        let key = format!("mega-debrid:{user}");
        if let Some(token) = self.tokens.lock().unwrap_or_else(|e| e.into_inner()).get(&key).cloned() {
            return Ok(token);
        }
        let body = self
            .json(self.client.get("https://www.mega-debrid.eu/api.php").query(&[
                ("action", "connectUser"),
                ("login", user),
                ("password", password),
            ]))
            .await?;
        let token = body["token"].as_str().ok_or_else(|| api_error(&body, "response_text"))?.to_owned();
        self.tokens.lock().unwrap_or_else(|e| e.into_inner()).insert(key, token.clone());
        Ok(token)
    }

    /// Official API: https://api.1fichier.com/
    async fn onefichier(&self, url: &Url, key: &str) -> Result<Resolved, String> {
        let body = self
            .json(
                self.client
                    .post("https://api.1fichier.com/v1/download/get_token.cgi")
                    .bearer_auth(key)
                    .json(&serde_json::json!({ "url": url.as_str() })),
            )
            .await?;
        if body["status"] != "OK" {
            return Err(api_error(&body, "message"));
        }
        let link = body["url"].as_str().ok_or("1fichier returned no link")?;
        Self::resolved(link, None, "1fichier premium")
    }

    /// Official API v2: https://rapidgator.net/article/api/
    async fn rapidgator(&self, url: &Url, login: &str, password: &str) -> Result<Resolved, String> {
        let file_id = url
            .path_segments()
            .and_then(|mut s| {
                let first = s.next()?;
                if first == "file" {
                    s.next()
                } else {
                    Some(first)
                }
            })
            .filter(|id| !id.is_empty())
            .ok_or("this is not a Rapidgator file link")?
            .to_owned();
        let key = format!("rapidgator:{login}");
        let cached = self.tokens.lock().unwrap_or_else(|e| e.into_inner()).get(&key).cloned();
        let token = match cached {
            Some(token) => token,
            None => {
                let body = self
                    .json(
                        self.client
                            .get("https://rapidgator.net/api/v2/user/login")
                            .query(&[("login", login), ("password", password)]),
                    )
                    .await?;
                let token = body["response"]["token"].as_str().ok_or_else(|| api_error(&body, "details"))?.to_owned();
                self.tokens.lock().unwrap_or_else(|e| e.into_inner()).insert(key.clone(), token.clone());
                token
            }
        };
        let body = self
            .json(
                self.client
                    .get("https://rapidgator.net/api/v2/file/download")
                    .query(&[("file_id", file_id.as_str()), ("token", token.as_str())]),
            )
            .await?;
        let Some(link) = body["response"]["download_url"].as_str() else {
            // The session may have expired: log in again next time.
            self.tokens.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
            return Err(api_error(&body, "details"));
        };
        Self::resolved(link, None, "Rapidgator premium")
    }

    /// Validates an account and returns a short summary.
    pub async fn check_account(&self, service_id: &str, username: &str, secret: &str) -> Result<AccountCheck, String> {
        let checked_later =
            || Ok(AccountCheck { premium: true, summary: "Saved, checked on first download".to_owned() });
        match service_id {
            "real-debrid" => {
                let body =
                    self.json(self.client.get("https://api.real-debrid.com/rest/1.0/user").bearer_auth(secret)).await?;
                let kind = body["type"].as_str().ok_or_else(|| api_error(&body, "error"))?;
                let premium = kind == "premium";
                let until = body["expiration"].as_str().map(|d| d.get(..10).unwrap_or(d).to_owned());
                Ok(AccountCheck { premium, summary: summary(body["username"].as_str(), premium, until) })
            }
            "alldebrid" => {
                let body = self
                    .json(
                        self.client
                            .get("https://api.alldebrid.com/v4/user")
                            .query(&[("agent", AGENT), ("apikey", secret)]),
                    )
                    .await?;
                if body["status"] != "success" {
                    return Err(api_error(&body["error"], "message"));
                }
                let user = &body["data"]["user"];
                let premium = user["isPremium"].as_bool().unwrap_or(false);
                Ok(AccountCheck { premium, summary: summary(user["username"].as_str(), premium, None) })
            }
            "premiumize" => {
                let body = self
                    .json(self.client.get("https://www.premiumize.me/api/account/info").query(&[("apikey", secret)]))
                    .await?;
                if body["status"] != "success" {
                    return Err(api_error(&body, "message"));
                }
                let premium = body["premium_until"].as_u64().is_some_and(|t| t > 0);
                Ok(AccountCheck { premium, summary: summary(None, premium, None) })
            }
            "debrid-link" => {
                let body = self
                    .json(self.client.get("https://debrid-link.com/api/v2/account/infos").bearer_auth(secret))
                    .await?;
                if body["success"] != true {
                    return Err(api_error(&body, "error"));
                }
                let premium = body["value"]["accountType"].as_i64().is_some_and(|t| t > 0);
                Ok(AccountCheck { premium, summary: summary(body["value"]["pseudo"].as_str(), premium, None) })
            }
            "torbox" => {
                let body =
                    self.json(self.client.get("https://api.torbox.app/v1/api/user/me").bearer_auth(secret)).await?;
                if body["success"] != true {
                    return Err(api_error(&body, "detail"));
                }
                let premium = body["data"]["plan"].as_i64().is_some_and(|p| p > 0);
                Ok(AccountCheck { premium, summary: summary(body["data"]["email"].as_str(), premium, None) })
            }
            "mega-debrid" => {
                self.tokens.lock().unwrap_or_else(|e| e.into_inner()).remove(&format!("mega-debrid:{username}"));
                self.mega_debrid_token(username, secret).await?;
                Ok(AccountCheck { premium: true, summary: summary(Some(username), true, None) })
            }
            "linksnappy" => {
                let body = self
                    .json(
                        self.client
                            .get("https://linksnappy.com/api/AUTHENTICATE")
                            .query(&[("username", username), ("password", secret)]),
                    )
                    .await?;
                if body["status"] != "OK" {
                    return Err(api_error(&body, "error"));
                }
                Ok(AccountCheck { premium: true, summary: summary(Some(username), true, None) })
            }
            "rapidgator" => {
                let body = self
                    .json(
                        self.client
                            .get("https://rapidgator.net/api/v2/user/login")
                            .query(&[("login", username), ("password", secret)]),
                    )
                    .await?;
                let user = &body["response"]["user"];
                if user.is_null() {
                    return Err(api_error(&body, "details"));
                }
                let premium = user["is_premium"].as_bool().unwrap_or(false);
                Ok(AccountCheck { premium, summary: summary(Some(username), premium, None) })
            }
            "1fichier" | "offcloud" => checked_later(),
            other => Err(format!("unknown service `{other}`")),
        }
    }
}

fn summary(user: Option<&str>, premium: bool, until: Option<String>) -> String {
    let mut s = if premium { "Premium".to_owned() } else { "Free account".to_owned() };
    if let Some(until) = until {
        s.push_str(&format!(" until {until}"));
    }
    if let Some(user) = user {
        s.push_str(&format!(" · {user}"));
    }
    s
}

fn api_error(value: &Value, field: &str) -> String {
    value[field]
        .as_str()
        .or_else(|| value["error"].as_str())
        .or_else(|| value["message"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("unexpected answer: {value}"))
}

impl LinkResolver for HosterResolver {
    fn resolve<'a>(&'a self, url: &'a Url, slot: Arc<ControlSlot>) -> BoxFuture<'a, Result<Resolved, String>> {
        Box::pin(self.resolve_inner(url, slot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn recognises_hosters_and_aliases() {
        assert_eq!(hoster_of(&url("https://1fichier.com/?abc")), Some("1fichier"));
        assert_eq!(hoster_of(&url("https://www.dl4free.com/?abc")), Some("1fichier"));
        assert_eq!(hoster_of(&url("https://rg.to/file/123")), Some("Rapidgator"));
        assert_eq!(hoster_of(&url("https://uploady.to/abc/file.zip")), Some("Uploady"));
        assert_eq!(hoster_of(&url("https://dailyuploads.net/abc")), Some("Dailyuploads"));
        assert_eq!(free_mode_of(&url("https://uploady.io/abc")), Some(Free::XFileSharing));
        assert_eq!(hoster_of(&url("https://example.com/a.zip")), None);
    }

    #[test]
    fn hoster_domains_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for (_, domains, _) in HOSTERS {
            for d in *domains {
                assert!(seen.insert(*d), "{d} listed twice");
            }
        }
    }

    #[test]
    fn free_direct_links() {
        assert_eq!(
            free_direct(&url("https://pixeldrain.com/u/AbC123")).unwrap().as_str(),
            "https://pixeldrain.com/api/file/AbC123?download"
        );
        assert_eq!(
            free_direct(&url("https://www.dropbox.com/s/x/file.zip?dl=0")).unwrap().as_str(),
            "https://www.dropbox.com/s/x/file.zip?dl=1"
        );
        assert_eq!(free_direct(&url("https://1fichier.com/?abc")), None);
    }

    #[tokio::test]
    async fn captcha_hosters_hand_the_page_to_the_user() {
        let resolver = HosterResolver::new(reqwest::Client::new());
        let opened = Arc::new(Mutex::new(Vec::new()));
        let sink = opened.clone();
        resolver.set_captcha_handler(Arc::new(move |page, hoster, _| sink.lock().unwrap().push((page, hoster))));
        let slot = Arc::new(ControlSlot::default());
        let err = resolver.resolve(&url("https://rapidgator.net/file/abc/f.rar.html"), slot).await.unwrap_err();
        assert!(err.contains("browser"), "{err}");
        assert_eq!(opened.lock().unwrap()[0].1, "Rapidgator");
    }
}
