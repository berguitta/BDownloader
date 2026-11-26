# AGENTS.md

Guide for AI coding agents (and humans in a hurry) working on BDownloader.
Read this file fully before changing code. Deeper explanations are in
[docs/](docs/README.md).

## Project in one paragraph

BDownloader is a native Windows download manager in Rust. A Cargo workspace:
`bd-net` (segmented HTTP engine with exact resume), `bd-core` (modules,
settings, queue, link grabber, hosters, debrid and premium APIs, free mode,
extraction, video, torrent), `bd-app` (Slint desktop UI, binary
`bdownloader`), `bd-cli` (binary `bdl`), `xtask` (icon rendering). Everything
runs in one process; there is no web server.

## Hard rules

1. **Never implement captcha solving or anti-bot circumvention.** No OCR or ML
   on captchas, no solving services, no code that fakes or hides what the
   client is in order to pass a site's bot checks, no automated passing of
   Turnstile / reCAPTCHA / hCaptcha. When a human is needed, return
   `FreeError::Captcha(page_url)`. Code and image captchas go to the user via
   `CaptchaAsk`. See [docs/free-mode-and-captchas.md](docs/free-mode-and-captchas.md).
2. **Do not reintroduce an embedded WebView.** It was removed on purpose.
3. **Honour hoster wait timers** with `wait_seconds` (in `free.rs`); never shorten them.
4. **No secrets in files.** Account secrets go through `AccountStore` and the
   Windows Credential Manager. Never write keys, tokens, cookies or private
   links into code, tests, fixtures or logs.
5. **English only** in code, comments, docs and commit messages.
6. **No `unsafe`.** The workspace denies it.
7. **Keep the CI green**: `cargo fmt --all --check`,
   `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace`.

## Where things are

| Task | Files |
|---|---|
| Add / fix a hoster | `crates/bd-core/src/hosters.rs` (`HOSTERS`, `HosterResolver`), `crates/bd-core/src/free.rs` |
| Add a debrid or premium service | `crates/bd-core/src/accounts.rs` (`SERVICES`), `crates/bd-core/src/hosters.rs` (`try_debrid`, `try_premium`, `check_account`) |
| Folder / list links | `crates/bd-core/src/decrypt.rs` |
| Download engine, resume, retries | `crates/bd-net/src/engine.rs`, `state.rs` |
| Secure DNS | `crates/bd-net/src/dns.rs` |
| Queue | `crates/bd-core/src/manager.rs`, `task.rs` |
| Link grabber | `crates/bd-core/src/grabber.rs`, `links.rs`, `packages.rs` |
| Modules (feature switches) | `crates/bd-core/src/modules.rs` |
| A new setting | `crates/bd-app/src/schema.rs` (UI is generated from it), value read from `Settings::options` |
| UI layout | `crates/bd-app/ui/*.slint`, tokens in `theme.slint` |
| UI callbacks / state | `crates/bd-app/src/wiring.rs`, `app.rs` |
| Translations | `crates/bd-app/translations/fr/LC_MESSAGES/bd-app.po`, `src/i18n.rs`, French strings in `schema.rs` |

## Commands

```powershell
cargo build --workspace
cargo run -p bd-app                                   # desktop app
cargo run -p bd-cli -- probe <url>                    # engine only
cargo test --workspace                                # all tests
cargo test -p bd-core hosters                         # tests matching a name
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings

# Headless screenshot of the real UI (no window needed)
$env:BD_SNAPSHOT="out.png"; cargo run -p bd-app --features snapshot

# Live free-mode check against a real site (ignored by default)
$env:BD_FREE_URL="https://…"; cargo test -p bd-core --test free_live -- --ignored --nocapture
```

Windows with the MSVC toolchain is required to build. If you run on another
OS, you can still read and edit code, but say in your summary that you could
not build or test.

## Patterns to follow

- **Hoster table entry**: `("Name", &["domain.com", "mirror.to"], Free::Mode)`.
  A domain appears once in the whole table (a test enforces it). Use
  `Free::XFileSharing` for XFileSharing sites, `Free::Browser` when free mode
  always needs an interactive captcha.
- **Free flow functions** return `FreeResult<Resolved>`, use
  `session_client()`, wait with `wait_seconds(slot, secs, label)`, and return
  `Offline` / `Captcha` / `Other` errors as documented in
  [docs/adding-a-hoster.md](docs/adding-a-hoster.md).
- **API clients** are `HosterResolver` methods using `self.json(...)`,
  `Self::resolved(link, name, via)` and `api_error(...)`. Link the official
  API docs in a `///` comment.
- **Parsing** goes into small pure functions with unit tests built from tiny,
  anonymised HTML snippets.
- **Status for the UI** while resolving: `slot.set_status(Some("…"))`, then
  `slot.set_status(None)`. Countdown: handled by `wait_seconds`.
- **Locks**: `lock().unwrap_or_else(|e| e.into_inner())`.
- **User-facing strings**: short plain English. Add the French text where the
  surrounding code has one.
- **Module headers**: every file starts with a `//!` comment explaining its
  purpose; keep it accurate when you change the file.

## Before you finish

- Run fmt, clippy and tests (or say clearly that you could not).
- Update docs when behaviour changes: `README.md` feature list, the relevant
  page in `docs/`.
- Keep the change focused; do not reformat or refactor unrelated code.
- Do not commit generated files from `target/`, local planning notes
  (`docs/PLAN.md`) or `prototypes/` (all git-ignored).
