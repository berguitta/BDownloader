# Roadmap

BDownloader is an **early preview (v0.1)**. The engine, the link grabber,
debrid and premium accounts, free mode for many hosters, video sites,
torrents and extraction work today. This page lists what comes next and where
contributions help most. Items are not promises or dates; open an issue to
discuss or claim one.

## Now: make the project easy to contribute to and to use

- [ ] **Prebuilt Windows releases** from CI (zip and installer), so users do
      not need Rust.
- [ ] **Update check** in the app (opt-in), pointing to new releases.
- [ ] **Per-hoster fixtures**: a small saved HTML page for each free flow,
      tested offline in CI, so a site change shows up as a failing test.
- [ ] **Hoster status table** in the docs, generated from `HOSTERS`
      (free mode, premium, folders, last verified).
- [ ] **More hosters** without interactive captchas, and fixes as sites change.
      See [adding-a-hoster.md](adding-a-hoster.md).
- [ ] **More debrid services**. See
      [adding-a-debrid-service.md](adding-a-debrid-service.md).

## Next: smoother free downloads, within the rules

See [free-mode-and-captchas.md](free-mode-and-captchas.md) for the rules and
the reasoning.

- [ ] **"Needs your browser" state** in the queue, with "open page again" and
      automatic retry later.
- [ ] **Batch browser hand-off**: clear all pending browser checks in one
      sitting.
- [ ] **Mirror routing**: prefer the mirror that can be downloaded without a
      browser check; use accounts first when the user has them.
- [ ] **Batch link checks** through hoster APIs when they allow many links per
      request.
- [ ] **Companion browser extension (to discuss)**: send links and, after the
      user's own check, final file links back to BDownloader.

## Later: hosters that update without a new release

Hosters change faster than an app can be released. The long-term plan is to
move hoster definitions out of the binary:

- [ ] **Declarative hoster rules** (TOML) for the common cases: domains,
      XFileSharing variant, form ids, link patterns. Loaded at start-up,
      updatable on their own.
- [ ] **Signed rule updates** downloaded by the app, so users get hoster fixes
      within hours.
- [ ] **Sandboxed plugins** (for example WebAssembly) for flows that rules
      cannot describe.

## Platform and interface

- [ ] **Linux and macOS ports**: the UI (Slint) and the engine are portable;
      the account store, tray, clipboard watcher and a few Windows calls need
      platform backends.
- [ ] **More languages**, after generalising the Rust and Settings texts. See
      [translations.md](translations.md).
- [ ] Accessibility review of the UI (keyboard navigation, contrast, screen
      readers).

## Not planned

- Captcha solving or anti-bot circumvention of any kind.
- An embedded WebView.
- A mandatory local web server or remote web UI.
