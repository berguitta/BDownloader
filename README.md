<div align="center">

<img src="assets/logo-256.png" alt="BDownloader logo" width="112">

# BDownloader

**A fast, native download manager for Windows, written in Rust.**
One small executable. No Java, no Python, no web server, no browser extension.

[![CI](https://github.com/berguitta/BDownloader/actions/workflows/ci.yml/badge.svg)](https://github.com/berguitta/BDownloader/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
![Platform: Windows](https://img.shields.io/badge/platform-Windows-0078D6.svg)
![Status: early preview](https://img.shields.io/badge/status-early%20preview-orange.svg)

[Features](#features) ·
[Getting started](#getting-started) ·
[Full-auto downloads](#full-auto-downloads-what-works-today) ·
[Roadmap](docs/roadmap.md) ·
[Contributing](CONTRIBUTING.md) ·
[Documentation](docs/README.md)

</div>

---

## Why BDownloader?

Download managers for file hosters have historically been heavy (a Java VM,
a Python runtime, a local web server you have to secure). BDownloader keeps
the ideas that made them great and drops the weight:

- **Native and light.** A single Rust binary that starts in under a second and
  stays small in memory. Nothing else to install.
- **Built for file hosters.** Copy a link and it shows up in the link grabber,
  already checked online, named, sized and grouped with its mirrors.
- **As automatic as the sites allow.** Wait timers, multi-step forms and
  "Free download" buttons are handled in the background, with a countdown in
  the progress bar. With a debrid or premium account, downloads are fully
  automatic.
- **Respectful.** Wait times are honoured and captchas are always answered by
  you. BDownloader never tries to defeat a site's protections.
- **Open and modular.** Every feature is a module you can switch off, and
  adding a hoster is usually a few lines of Rust. See
  [how to add a hoster](docs/adding-a-hoster.md).

## Features

**Downloads**
- **Multi-connection transfers**: files are split into segments downloaded in
  parallel when the server allows it.
- **Exact resume**: progress is saved next to the partial file
  (`name.part` + `name.part.json`), so a download continues at the right byte
  after a pause, a crash or a reboot.
- **Polite with servers**: connections per host adapt automatically when a
  server answers `429 Too Many Requests` or `503`.
- **Saved queue**, global and per-download speed limits, scheduler, power
  actions when the queue is done, anti-standby.

**Links**
- **Link grabber**: links from the clipboard, a paste or Click'n'Load are
  checked online, grouped into packages with their mirrors, and start only
  when you pick them. A web page link becomes a pool of the files it lists.
- **Folder decrypters** for 1fichier, Gofile, Mediafire and Pixeldrain lists.
- **Video sites** through yt-dlp (installed on demand): pick the quality, or
  audio only (original or MP3 with ffmpeg).
- **BitTorrent and magnet links** next to the DDL queue.

**Hosters and accounts**
- **8 debrid services**: Real-Debrid, AllDebrid, Premiumize, Debrid-Link,
  TorBox, Offcloud, LinkSnappy, Mega-Debrid.
- **Premium APIs**: 1fichier and Rapidgator.
- **Free mode** for 1fichier, Mediafire, Gofile, Google Drive, Krakenfiles,
  Pixeldrain, Dropbox and 30+ XFileSharing-based hosters (DDownload, Katfile,
  Uploady, DataNodes, Clicknupload…). Unknown XFileSharing sites are detected
  automatically.
- **Secrets stay safe**: passwords and API keys live in the Windows Credential
  Manager, never in a plain file.
- **Secure DNS** (DNS over HTTPS) for BDownloader's own connections, when your
  provider's DNS blocks a hoster.

**After the download**
- **Automatic extraction** of ZIP, 7z and RAR (multi-part) packages with the
  package password and your password list.
- **Checksum verification**, event scripts, Discord and Telegram notifications.

**Interface**
- Light and dark themes, English and French, tray icon with a quick menu,
  Windows notifications, mini window, portable mode.

## Getting started

> **Status: early preview (v0.1).** There are no prebuilt releases yet; build
> from source. Windows 10/11 only for now.

1. Install [Rust](https://rustup.rs) (stable, MSVC toolchain) and the
   Visual Studio Build Tools with the **Desktop development with C++**
   workload.
2. Build and run:

   ```powershell
   git clone https://github.com/berguitta/BDownloader
   cd BDownloader
   cargo run --release -p bd-app
   ```

The command line tool is also available:

```powershell
cargo run --release -p bd-cli -- get https://example.com/file.zip -o D:\Downloads -c 8
cargo run --release -p bd-cli -- probe https://example.com/file.zip
```

Full build, test and debugging instructions: [docs/building.md](docs/building.md).

## Full-auto downloads: what works today

BDownloader resolves a hoster link in this order (configurable in
**Settings → Hosters & accounts**):

| Route | Automation | What you need |
|---|---|---|
| Debrid service | Fully automatic | A debrid account (API key) |
| Hoster premium API | Fully automatic | A premium account (1fichier, Rapidgator) |
| Free mode, no captcha | Fully automatic, waits shown as a countdown | Nothing |
| Free mode, code or image captcha | You type the code in a small BDownloader dialog | A few seconds of your time |
| Free mode, "I am not a robot" widget (Turnstile, reCAPTCHA, hCaptcha) | Not automatable: the page opens in your browser | Your browser, or a debrid account |

The last row is the main limit today. Some free hosters (for example
Rapidgator, and DataNodes or Uploady depending on the day) put an interactive
captcha in front of the file. Those widgets are designed to need a human in a
real browser, and BDownloader deliberately does not solve or bypass them. The
full explanation, and the ideas we are exploring to make this smoother, are in
[docs/free-mode-and-captchas.md](docs/free-mode-and-captchas.md).

## Contributing

Hoster sites change all the time, so BDownloader lives on community fixes.
Good places to start:

- **A hoster broke?** Open a "Hoster broken" issue, or fix it: most fixes are
  in one file. See [docs/adding-a-hoster.md](docs/adding-a-hoster.md).
- **Add a debrid service**: [docs/adding-a-debrid-service.md](docs/adding-a-debrid-service.md).
- **Translate the interface**: [docs/translations.md](docs/translations.md).
- **Pick an item from the [roadmap](docs/roadmap.md).**

Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request. AI
coding agents should start with [AGENTS.md](AGENTS.md).

## Workspace at a glance

| Crate | Role |
|---|---|
| [`crates/bd-net`](crates/bd-net) | Download engine: probing, segmented transfers, resume, retries, per-host limits, secure DNS |
| [`crates/bd-core`](crates/bd-core) | Modules, settings, queue, link grabber, hosters, debrid, free mode, extraction, video, torrent |
| [`crates/bd-cli`](crates/bd-cli) | `bdl` command line tool |
| [`crates/bd-app`](crates/bd-app) | Desktop application built with [Slint](https://slint.dev) |
| [`xtask`](xtask) | Development tasks (icon rendering) |

Details in [docs/architecture.md](docs/architecture.md).

## Responsible use

BDownloader follows the rules of the services it talks to: wait times are
honoured, captchas are shown to you and never bypassed, and accounts are used
through official APIs. You are responsible for what you download and for
respecting each service's terms.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. The desktop UI uses Slint under its royalty-free license.

Unless you explicitly state otherwise, any contribution you submit for
inclusion in this project, as defined in the Apache-2.0 license, is dual
licensed as above, without any additional terms or conditions.
