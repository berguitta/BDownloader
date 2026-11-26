# Adding or fixing a file hoster

Hoster support lives in `crates/bd-core`. Most changes touch one or two files:

| File | What goes there |
|---|---|
| `src/hosters.rs` | The `HOSTERS` table (name, domains, free mode), the resolver order, premium API clients, link checks |
| `src/free.rs` | Free-mode flows and the HTML helpers they use (`forms`, `attr`, `countdown`, `captcha_on`, `file_link`…) |
| `src/accounts.rs` | The `SERVICES` table, when a hoster has a premium account |
| `src/decrypt.rs` | Folder or list links that stand for several files |

Before you start, read [free-mode-and-captchas.md](free-mode-and-captchas.md):
it explains what free mode may and may not do. **Pull requests that solve or
bypass captchas, or defeat a site's anti-bot protection, are not accepted.**

## Pick the right case

1. **The site is built on XFileSharing** (URLs like `site.com/abc123xyz/file.zip`,
   a "Free Download" button posting `op=download1`, then a wait and
   `op=download2`): add one line to `HOSTERS` with `Free::XFileSharing`. This is
   the most common case and usually all it takes.
2. **The site gives direct links without a page** (like Pixeldrain or Dropbox):
   add a `Free` variant and a rewrite rule in `free_direct`.
3. **The site has its own flow** (an API, a JSON step, a special form): add a
   `Free` variant and a `free::yourhoster` function.
4. **The site has an official premium API**: add a service and a
   `try_premium` arm.
5. **The site always shows an interactive captcha in free mode** (Turnstile,
   reCAPTCHA, hCaptcha): add it with `Free::Browser`. Users with a debrid or
   premium account still get full automation, and free users are handed the
   page in their browser.

## 1. Register the hoster

Add an entry to `HOSTERS` in `crates/bd-core/src/hosters.rs`:

```rust
("Examplehost", &["examplehost.com", "exmpl.to"], Free::XFileSharing),
```

- The **name** is shown to users and used in messages.
- List **every domain and mirror domain**. Subdomains (`www.`, `dl.`) match
  automatically. A domain may appear only once in the whole table; the
  `hoster_domains_are_unique` test checks it.
- Add a case to the `recognises_hosters_and_aliases` test.

That is enough for the link grabber to recognise the hoster, check links
(`free::page_info` reads name, size and offline state from the page), and for
debrid services to be tried on it.

## 2. A custom free flow

Add a variant to `enum Free`, then a function in `free.rs`:

```rust
/// Examplehost: JSON API gives a token, then a direct link.
pub async fn examplehost(url: &Url, slot: &Arc<ControlSlot>) -> FreeResult<Resolved> {
    let client = session_client();             // own cookie jar, secure DNS aware
    let (page, html) = get_text(&client, url).await?;
    if looks_offline(&html) {
        return Err(FreeError::Offline("removed".into()));
    }
    if let Some(seconds) = countdown(&html) {
        // Shows the countdown in the progress bar and honours cancel.
        wait_seconds(slot, seconds + 1, "Free mode, Examplehost wait").await?;
    }
    if captcha_on(&html).is_some_and(|kind| kind != "code") {
        return Err(FreeError::Captcha(url.clone()));   // hand the page to the user
    }
    let link = file_link(&page, &html, "examplehost.com")
        .ok_or_else(|| other("Examplehost page changed"))?;
    Ok(direct(link, "Examplehost (free)"))
}
```

Wire it in `hosters.rs`:

- in `HosterResolver::resolve_inner`, the `match free_mode` block:
  `Free::Examplehost => free::examplehost(url, &slot).await,`
- in `check_link`, if the hoster has a cheaper way to check a link than its
  page (an API), use it there.

### Rules for free flows

- **Use `session_client()`** for every request of one resolution, so cookies
  and secure DNS behave like the rest of the app.
- **Honour every wait** with `wait_seconds`. It updates the countdown shown in
  the progress bar and returns early when the user cancels.
- **Return the right error**:
  - `FreeError::Offline(reason)` when the file is gone (404, "file not
    found"). The grabber shows it as offline.
  - `FreeError::Captcha(page_url)` when a human is needed in a browser. The
    resolver then follows the user's captcha setting (open the page, notify,
    or skip).
  - `FreeError::Other(text)` for everything else, with a message a user can
    understand ("Examplehost page changed", not a parser dump).
- **Code and image captchas** (a code to copy) are never read by
  BDownloader. Pass a `CaptchaAsk` and send a `CaptchaRequest`, as
  `xfilesharing` does; the user types the answer in the in-app dialog.
- **Keep parsing in small pure functions** (`fn examplehost_link(page: &Url,
  html: &str) -> Option<Url>`) so they can be unit-tested with HTML snippets.
- **Log with `tracing`** at `info` for decisions ("handing the page to the
  browser") and `debug` for steps. Never log secrets or full cookies.

## 3. Premium API

1. Add a `Service` to `SERVICES` in `accounts.rs` with `kind:
   ServiceKind::Premium`, the secret label ("API key", "Password") and a
   `help` hint telling users where to find it.
2. Add an arm to `HosterResolver::try_premium` keyed by the hoster name from
   `HOSTERS`.
3. Implement the client as a method of `HosterResolver`, using the official
   API only. Use `self.json(...)` for requests, `Self::resolved(...)` for the
   result, and `api_error(...)` for readable errors. Cache session tokens in
   `self.tokens` as `rapidgator` does, and drop them when the API says the
   session expired.
4. Add an arm to `check_account` so the Accounts screen can show "Premium
   until …". If the API cannot check cheaply, return the "checked on first
   download" summary.

Secrets are handed to you as `&str`; never store them anywhere else.

## 4. Folders and lists

If the hoster has folder links, add a recogniser to `decrypt::is_container`
and a branch to `decrypt::expand` that returns the file links. They become a
pool in the link grabber.

## Testing your change

1. **Unit tests** for every parser helper, with the smallest HTML snippet that
   shows the quirk. Strip real file names and IDs from fixtures.
2. **Capture the real pages** while developing:

   ```powershell
   $env:BD_FREE_DUMP = "D:\tmp\dump"
   $env:BD_LOG = "bd_core=debug,info"
   cargo run -p bd-app
   ```

   Each page of an XFileSharing flow is saved as `stepN.html`.
3. **Live check** against the site (not run in CI):

   ```powershell
   $env:BD_FREE_URL = "https://examplehost.com/abc123/test.bin"
   cargo test -p bd-core --test free_live -- --ignored --nocapture
   ```

4. Run the [CI checks](building.md#the-checks-ci-runs).

In the pull request, say which hoster, what changed on the site, and how you
tested it (live test output is welcome, without private links).
