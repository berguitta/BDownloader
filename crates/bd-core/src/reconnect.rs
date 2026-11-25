//! Reconnect: get a new IP address from the internet box, to skip the wait
//! timers that free hosters apply per IP. Two methods: UPnP (most boxes)
//! or a custom command chosen by the user.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::UdpSocket;
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Method {
    Off,
    Upnp,
    /// Command line run through `cmd /C`.
    Script(String),
}

/// Asks for a new IP and waits until the connection is back.
pub async fn reconnect(method: &Method, client: &reqwest::Client) -> Result<String, String> {
    match method {
        Method::Off => Err("reconnect is turned off".into()),
        Method::Script(command) => {
            let status =
                tokio::process::Command::new("cmd").args(["/C", command]).status().await.map_err(|e| e.to_string())?;
            wait_online(client).await;
            if status.success() {
                Ok("script finished".into())
            } else {
                Err(format!("script failed: {status}"))
            }
        }
        Method::Upnp => {
            let (control, service) = discover().await?;
            soap(client, &control, &service, "ForceTermination").await?;
            tokio::time::sleep(Duration::from_secs(3)).await;
            // Most boxes reconnect by themselves; ask anyway.
            let _ = soap(client, &control, &service, "RequestConnection").await;
            wait_online(client).await;
            let ip = soap(client, &control, &service, "GetExternalIPAddress").await.ok().and_then(|xml| {
                let start = xml.find("<NewExternalIPAddress>")? + 22;
                let end = xml[start..].find('<')? + start;
                Some(xml[start..end].to_owned())
            });
            Ok(ip.map(|ip| format!("new IP {ip}")).unwrap_or_else(|| "reconnected".into()))
        }
    }
}

/// Waits (up to two minutes) until the internet answers again.
async fn wait_online(client: &reqwest::Client) {
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if client.head("https://www.google.com/generate_204").timeout(Duration::from_secs(3)).send().await.is_ok() {
            return;
        }
    }
}

const SERVICES: &[&str] =
    &["urn:schemas-upnp-org:service:WANIPConnection:1", "urn:schemas-upnp-org:service:WANPPPConnection:1"];

/// Finds the box's control URL with an SSDP search on the local network.
async fn discover() -> Result<(Url, String), String> {
    let socket = UdpSocket::bind("0.0.0.0:0").await.map_err(|e| e.to_string())?;
    let target: SocketAddr = "239.255.255.250:1900".parse().expect("static address");
    for service in SERVICES {
        let search = format!(
            "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {service}\r\n\r\n"
        );
        socket.send_to(search.as_bytes(), target).await.map_err(|e| e.to_string())?;
    }
    let mut buf = [0u8; 2048];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while let Ok(Ok((n, _))) = tokio::time::timeout_at(deadline, socket.recv_from(&mut buf)).await {
        let answer = String::from_utf8_lossy(&buf[..n]);
        let Some(location) = answer.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case("location").then(|| v.trim().to_owned())
        }) else {
            continue;
        };
        if let Ok(found) = control_url(&location).await {
            return Ok(found);
        }
    }
    Err("no UPnP internet box answered (is UPnP enabled on the box?)".into())
}

async fn control_url(location: &str) -> Result<(Url, String), String> {
    let base = Url::parse(location).map_err(|e| e.to_string())?;
    let xml = reqwest::get(base.clone()).await.map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())?;
    for service in SERVICES {
        let Some(pos) = xml.find(service) else { continue };
        let rest = &xml[pos..];
        let start = rest.find("<controlURL>").ok_or("no control URL")? + 12;
        let end = rest[start..].find('<').ok_or("bad description")? + start;
        let url = base.join(rest[start..end].trim()).map_err(|e| e.to_string())?;
        return Ok((url, (*service).to_owned()));
    }
    Err("this device is not an internet box".into())
}

async fn soap(client: &reqwest::Client, control: &Url, service: &str, action: &str) -> Result<String, String> {
    let body = format!(
        "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{service}\"/></s:Body></s:Envelope>"
    );
    let response = client
        .post(control.clone())
        .header("Content-Type", "text/xml; charset=\"utf-8\"")
        .header("SOAPAction", format!("\"{service}#{action}\""))
        .body(body)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let ok = response.status().is_success();
    let text = response.text().await.unwrap_or_default();
    if ok {
        Ok(text)
    } else {
        Err(format!("{action} refused by the box"))
    }
}
