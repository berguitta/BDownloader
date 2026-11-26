# Contributing to BDownloader

Thanks for helping! File hosters change their pages all the time, so
community fixes are what keep BDownloader working. This guide covers the
workflow; the [docs](docs/README.md) cover the code.

## Ways to help

| You want to… | Start here |
|---|---|
| Report a bug | Open a **Bug report** issue |
| Report a hoster that stopped working | Open a **Hoster broken** issue (no private links needed) |
| Fix or add a hoster | [docs/adding-a-hoster.md](docs/adding-a-hoster.md) |
| Add a debrid service | [docs/adding-a-debrid-service.md](docs/adding-a-debrid-service.md) |
| Translate | [docs/translations.md](docs/translations.md) |
| Work on a bigger feature | [docs/roadmap.md](docs/roadmap.md), then open an issue to discuss it first |

## Ground rules

1. **Respect the hosters' protections.** Free mode honours waits and hands
   every captcha to the user. Pull requests that solve or bypass captchas,
   use solving services, or defeat anti-bot checks are closed. Read
   [docs/free-mode-and-captchas.md](docs/free-mode-and-captchas.md).
2. **No secrets in the repository**: no API keys, tokens, cookies, account
   names, or private download links in code, tests, fixtures, logs or
   screenshots.
3. **English everywhere in the repository**: code, comments, docs, commit
   messages, issue and pull request titles.
4. **Be kind.** See the [Code of Conduct](CODE_OF_CONDUCT.md).

## Development setup

See [docs/building.md](docs/building.md). In short, on Windows with Rust
(MSVC) and the Visual Studio Build Tools:

```powershell
cargo run -p bd-app
```

## Before you open a pull request

Run the same checks as CI:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Then check that:

- [ ] the change is focused (one hoster, one feature, one fix);
- [ ] new parsing logic has unit tests with small, anonymised HTML or JSON;
- [ ] user-visible texts are in English, and French is added where the
      surrounding code has it (`schema.rs`, `i18n.rs`, `.po` file);
- [ ] new settings are added to `crates/bd-app/src/schema.rs`, with a
      sensible default, and stored in `Settings::options`;
- [ ] docs are updated when behaviour changes (README feature list, docs
      pages);
- [ ] UI changes come with a screenshot (see
      [headless screenshots](docs/building.md#headless-screenshots)).

## Code style

- `rustfmt` with the repository's `rustfmt.toml` (`max_width = 120`).
- Clippy clean with `-D warnings`. `unsafe` is denied workspace-wide.
- Every source file starts with a `//!` comment saying what it is for.
  Public items get a `///` comment when the name is not enough.
- Prefer small, pure functions for parsing so they can be tested without the
  network.
- Errors shown to users are short sentences in plain English ("file offline:
  removed", "Examplehost page changed"), not debug dumps.
- Locks: follow the existing pattern `lock().unwrap_or_else(|e| e.into_inner())`
  so a panic in one task does not poison the whole app.
- Async code runs on tokio; never block the UI thread. Background tasks talk
  to the UI through `UiMsg` (`crates/bd-app/src/app.rs`).
- New optional features are modules (`crates/bd-core/src/modules.rs`).
- Dependencies: prefer pure Rust crates with `rustls`; explain any new
  dependency in the pull request.

## Commits and pull requests

- Write commit messages in the imperative mood with a short summary line
  ("Add Examplehost free mode", "Fix DataNodes countdown parsing"), and a body
  explaining *why* when it is not obvious.
- One logical change per commit when practical.
- Fill in the pull request template: what, why, how you tested.
- AI-assisted contributions are welcome. You are responsible for the code you
  submit: review it, run the checks, and make sure it follows these rules.
  Agents should read [AGENTS.md](AGENTS.md).

## License

By contributing, you agree that your contributions are dual licensed under
the MIT and Apache-2.0 licenses, like the rest of the project.
