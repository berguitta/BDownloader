//! Decrypters: links that stand for several files (hoster folders, lists)
//! are expanded into their file links, which then form one pool.

use serde_json::Value;
use url::Url;

use crate::free::session_client;

/// A folder or list expanded into files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expanded {
    pub name: String,
    pub links: Vec<Url>,
}

/// Whether the link is a folder or list this module can expand.
pub fn is_container(url: &Url) -> bool {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let path = url.path();
    (host.ends_with("1fichier.com") && path.starts_with("/dir/"))
        || (host.ends_with("mediafire.com") && path.starts_with("/folder/"))
        || (host.ends_with("pixeldrain.com") && path.starts_with("/l/"))
}

pub async fn expand(url: &Url) -> Result<Expanded, String> {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let segment = |i: usize| url.path_segments().and_then(|mut s| s.nth(i)).unwrap_or_default().to_owned();
    if host.ends_with("1fichier.com") {
        onefichier_folder(url).await
    } else if host.ends_with("gofile.io") {
        gofile_folder(&segment(1)).await
    } else if host.ends_with("mediafire.com") {
        mediafire_folder(&segment(1)).await
    } else if host.ends_with("pixeldrain.com") {
        pixeldrain_list(&segment(1)).await
    } else {
        Err("not a folder link".into())
    }
}

async fn get_json(client: &reqwest::Client, url: &str) -> Result<Value, String> {
    client.get(url).send().await.map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())
}

/// 1fichier folders answer in JSON with `?json=1`.
async fn onefichier_folder(url: &Url) -> Result<Expanded, String> {
    let client = session_client();
    let mut api = url.clone();
    api.query_pairs_mut().append_pair("json", "1");
    let list = get_json(&client, api.as_str()).await?;
    let links = list
        .as_array()
        .ok_or("1fichier folder is private or empty")?
        .iter()
        .filter_map(|f| Url::parse(f["link"].as_str()?).ok())
        .collect::<Vec<_>>();
    Ok(Expanded { name: "1fichier folder".into(), links })
}

/// Gofile: guest token, then every file of the folder.
async fn gofile_folder(id: &str) -> Result<Expanded, String> {
    let client = session_client();
    let account: Value = client
        .post("https://api.gofile.io/accounts")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let token = account["data"]["token"].as_str().ok_or("Gofile refused a guest session")?.to_owned();
    let content: Value = client
        .get(format!("https://api.gofile.io/contents/{id}?wt=4fd6sg89d7s6"))
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let data = &content["data"];
    let children = data["children"].as_object().ok_or("empty Gofile folder")?;
    // Each file gets a link to its own folder page, resolved one by one.
    let links = children
        .values()
        .filter(|c| c["type"] == "file")
        .filter_map(|c| Url::parse(c["link"].as_str()?).ok())
        .collect();
    Ok(Expanded { name: data["name"].as_str().unwrap_or("Gofile folder").to_owned(), links })
}

/// MediaFire public folders through its public API.
async fn mediafire_folder(key: &str) -> Result<Expanded, String> {
    let client = session_client();
    let info = get_json(
        &client,
        &format!("https://www.mediafire.com/api/1.5/folder/get_content.php?folder_key={key}&content_type=files&response_format=json"),
    )
    .await?;
    let files = info["response"]["folder_content"]["files"].as_array().ok_or("MediaFire folder is private or empty")?;
    let links = files.iter().filter_map(|f| Url::parse(f["links"]["normal_download"].as_str()?).ok()).collect();
    Ok(Expanded { name: "MediaFire folder".into(), links })
}

/// Pixeldrain lists: `/l/ID` → files `/u/ID`.
async fn pixeldrain_list(id: &str) -> Result<Expanded, String> {
    let client = session_client();
    let list = get_json(&client, &format!("https://pixeldrain.com/api/list/{id}")).await?;
    let links = list["files"]
        .as_array()
        .ok_or("Pixeldrain list not found")?
        .iter()
        .filter_map(|f| Url::parse(&format!("https://pixeldrain.com/u/{}", f["id"].as_str()?)).ok())
        .collect();
    Ok(Expanded { name: list["title"].as_str().unwrap_or("Pixeldrain list").to_owned(), links })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_containers() {
        for link in [
            "https://1fichier.com/dir/AbCdEf",
            "https://www.mediafire.com/folder/abc/name",
            "https://pixeldrain.com/l/abc",
        ] {
            assert!(is_container(&Url::parse(link).unwrap()), "{link}");
        }
        assert!(!is_container(&Url::parse("https://1fichier.com/?abc").unwrap()));
    }
}
