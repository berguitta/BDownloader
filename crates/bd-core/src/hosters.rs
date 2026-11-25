//! Hoster support: recognise hoster links and turn them into direct links
//! through a debrid service or the hoster's official premium API.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

use futures_util::future::BoxFuture;
use serde_json::Value;
use url::Url;

use crate::accounts::{service, Account};
use crate::task::{LinkResolver, Resolved};

const AGENT: &str = "BDownloader";

/// Known file hosters: (canonical name, domains).
pub const HOSTERS: &[(&str, &[&str])] = &[
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
    ),
    ("Rapidgator", &["rapidgator.net", "rg.to"]),
    ("Turbobit", &["turbobit.net", "turbobit.cc"]),
    ("Nitroflare", &["nitroflare.com", "nitro.download"]),
    ("Uploaded", &["uploaded.net", "ul.to"]),
    ("DDownload", &["ddownload.com", "ddl.to"]),
    ("Katfile", &["katfile.com"]),
    ("Uptobox", &["uptobox.com", "uptobox.eu"]),
    ("Mega", &["mega.nz", "mega.co.nz"]),
    ("MediaFire", &["mediafire.com"]),
    ("FileFactory", &["filefactory.com"]),
    ("Hitfile", &["hitfile.net"]),
    ("Gofile", &["gofile.io"]),
    ("Pixeldrain", &["pixeldrain.com"]),
    ("Dropbox", &["dropbox.com"]),
    ("Google Drive", &["drive.google.com"]),
];

/// Hoster name for a link, if it is a known file hoster.
pub fn hoster_of(url: &Url) -> Option<&'static str> {
    let host = url.host_str()?.to_ascii_lowercase();
    HOSTERS
        .iter()
        .find(|(_, domains)| domains.iter().any(|d| host == *d || host.ends_with(&format!(".{d}"))))
        .map(|(name, _)| *name)
}

/// Hosters whose free links can be turned into direct links without account.
fn free_direct(url: &Url) -> Option<Url> {
    match hoster_of(url)? {
        // https://pixeldrain.com/u/ID → https://pixeldrain.com/api/file/ID
        "Pixeldrain" => {
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
        "Dropbox" => {
            let mut direct = url.clone();
            let pairs: Vec<(String, String)> =
                url.query_pairs().filter(|(k, _)| k != "dl").map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
            direct.query_pairs_mut().clear().extend_pairs(pairs).append_pair("dl", "1");
            Some(direct)
        }
        _ => None,
    }
}

/// Account details shown in the Accounts screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountCheck {
    pub premium: bool,
    pub summary: String,
}

/// Resolves links with the user's accounts and enabled modules.
pub struct HosterResolver {
    client: reqwest::Client,
    credentials: RwLock<Vec<(Account, String)>>,
    modules: RwLock<HashSet<String>>,
    rapidgator_tokens: Mutex<HashMap<String, String>>,
}

impl HosterResolver {
    pub fn new(client: reqwest::Client) -> Arc<Self> {
        Arc::new(HosterResolver {
            client,
            credentials: RwLock::default(),
            modules: RwLock::default(),
            rapidgator_tokens: Mutex::default(),
        })
    }

    pub fn set_credentials(&self, credentials: Vec<(Account, String)>) {
        *self.credentials.write().unwrap_or_else(|e| e.into_inner()) = credentials;
    }

    /// Ids of enabled modules (hoster and debrid modules are checked).
    pub fn set_enabled_modules(&self, modules: impl IntoIterator<Item = String>) {
        *self.modules.write().unwrap_or_else(|e| e.into_inner()) = modules.into_iter().collect();
    }

    fn account_for(&self, service_id: &str) -> Option<(Account, String)> {
        let module = service(service_id)?.module;
        if !self.modules.read().unwrap_or_else(|e| e.into_inner()).contains(module) {
            return None;
        }
        self.credentials
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(a, _)| a.service == service_id)
            .cloned()
    }

    async fn resolve_inner(&self, url: &Url) -> Result<Resolved, String> {
        let Some(hoster) = hoster_of(url) else { return Ok(Resolved::direct(url.clone())) };

        if let Some(direct) = free_direct(url) {
            return Ok(Resolved { via: format!("{hoster} (free)"), ..Resolved::direct(direct) });
        }
        if hoster == "1fichier" {
            if let Some((_, key)) = self.account_for("1fichier") {
                return self.onefichier(url, &key).await;
            }
        }
        if hoster == "Rapidgator" {
            if let Some((account, password)) = self.account_for("rapidgator") {
                return self.rapidgator(url, &account.username, &password).await;
            }
        }
        let mut last_error = None;
        for debrid in ["real-debrid", "alldebrid", "premiumize"] {
            let Some((_, key)) = self.account_for(debrid) else { continue };
            let result = match debrid {
                "real-debrid" => self.real_debrid(url, &key).await,
                "alldebrid" => self.alldebrid(url, &key).await,
                _ => self.premiumize(url, &key).await,
            };
            match result {
                Ok(resolved) => return Ok(resolved),
                Err(e) => {
                    tracing::info!(service = debrid, error = %e, "debrid could not unlock the link, trying the next one");
                    last_error = Some(e);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            format!("{hoster} needs a premium or debrid account in this version: add one in Accounts")
        }))
    }

    async fn json(&self, request: reqwest::RequestBuilder) -> Result<Value, String> {
        let response = request.send().await.map_err(|e| e.to_string())?;
        let status = response.status();
        let body: Value = response.json().await.map_err(|e| format!("invalid answer ({status}): {e}"))?;
        Ok(body)
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
        Ok(Resolved {
            url: Url::parse(link).map_err(|e| e.to_string())?,
            headers: Vec::new(),
            file_name: body["filename"].as_str().map(str::to_owned),
            via: "Real-Debrid".to_owned(),
        })
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
        Ok(Resolved {
            url: Url::parse(link).map_err(|e| e.to_string())?,
            headers: Vec::new(),
            file_name: body["data"]["filename"].as_str().map(str::to_owned),
            via: "AllDebrid".to_owned(),
        })
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
        Ok(Resolved {
            url: Url::parse(link).map_err(|e| e.to_string())?,
            headers: Vec::new(),
            file_name: first["path"].as_str().and_then(|p| p.rsplit('/').next()).map(str::to_owned),
            via: "Premiumize".to_owned(),
        })
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
        Ok(Resolved {
            url: Url::parse(link).map_err(|e| e.to_string())?,
            headers: Vec::new(),
            file_name: None,
            via: "1fichier premium".to_owned(),
        })
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
        let cached = self.rapidgator_tokens.lock().unwrap_or_else(|e| e.into_inner()).get(login).cloned();
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
                self.rapidgator_tokens
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(login.to_owned(), token.clone());
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
            self.rapidgator_tokens.lock().unwrap_or_else(|e| e.into_inner()).remove(login);
            return Err(api_error(&body, "details"));
        };
        Ok(Resolved {
            url: Url::parse(link).map_err(|e| e.to_string())?,
            headers: Vec::new(),
            file_name: None,
            via: "Rapidgator premium".to_owned(),
        })
    }

    /// Validates an account and returns a short summary.
    pub async fn check_account(&self, service_id: &str, username: &str, secret: &str) -> Result<AccountCheck, String> {
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
            "1fichier" => {
                // No cheap "who am I" call: the key is checked on first use.
                Ok(AccountCheck { premium: true, summary: "Saved, checked on first download".to_owned() })
            }
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
    fn resolve<'a>(&'a self, url: &'a Url) -> BoxFuture<'a, Result<Resolved, String>> {
        Box::pin(self.resolve_inner(url))
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
        assert_eq!(hoster_of(&url("https://example.com/a.zip")), None);
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
    async fn plain_links_stay_direct_and_hosters_explain_what_is_missing() {
        let resolver = HosterResolver::new(reqwest::Client::new());
        let plain = resolver.resolve(&url("https://example.com/a.zip")).await.unwrap();
        assert_eq!(plain.via, "Direct");
        let err = resolver.resolve(&url("https://1fichier.com/?abc")).await.unwrap_err();
        assert!(err.contains("account"), "{err}");
    }
}
