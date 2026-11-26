//! Live check of a free hoster flow against the real site. Ignored by
//! default; run with a link of your own:
//! `BD_FREE_URL=https://… cargo test -p bd-core --test free_live -- --ignored --nocapture`
//! A code captcha is printed and left unanswered.

use std::sync::Arc;
use std::time::Duration;

use bd_core::free::{self, CaptchaAsk, Challenge};
use bd_core::task::ControlSlot;

#[tokio::test]
#[ignore]
async fn resolves_a_free_link() {
    let Ok(link) = std::env::var("BD_FREE_URL") else { return };
    let url = url::Url::parse(&link).unwrap();
    if let Ok(provider) = std::env::var("BD_SECURE_DNS") {
        bd_net::dns::set_secure_dns(bd_net::dns::endpoint_of(&provider));
    }
    let slot = Arc::new(ControlSlot::default());
    let ask = CaptchaAsk {
        ask: Arc::new(|request| {
            match &request.challenge {
                Challenge::Image(bytes) => println!("captcha: image of {} bytes", bytes.len()),
                Challenge::Placed(chars) => println!("captcha: {} placed characters", chars.len()),
            }
            Box::pin(async { None })
        }),
        timeout: Duration::from_secs(1),
    };
    let result = free::xfilesharing(&url, &slot, "live (free)", Some(&ask)).await;
    println!("{result:?}");
    let resolved = result.expect("free mode");
    // The file server must answer too (first bytes only).
    let engine = bd_net::Engine::new().unwrap();
    let response = engine.client().get(resolved.url.clone()).header("Range", "bytes=0-99").send().await.unwrap();
    println!("file server: {} {:?}", response.status(), response.headers().get("content-length"));
    assert!(response.status().is_success());
}
