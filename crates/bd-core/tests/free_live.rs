//! Live check of a free hoster flow against the real site. Ignored by
//! default; run with a link of your own:
//! `BD_FREE_URL=https://… cargo test -p bd-core --test free_live -- --ignored --nocapture`

use std::sync::Arc;

use bd_core::free;
use bd_core::task::ControlSlot;

#[tokio::test]
#[ignore]
async fn resolves_a_free_link() {
    let Ok(link) = std::env::var("BD_FREE_URL") else { return };
    let url = url::Url::parse(&link).unwrap();
    let slot = Arc::new(ControlSlot::default());
    let result = free::xfilesharing(&url, &slot, "live (free)", None).await;
    println!("{result:?}");
    assert!(result.is_ok(), "{result:?}");
}
