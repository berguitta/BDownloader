# Architecture

BDownloader is a Cargo workspace of four crates plus a development helper.
Everything runs in one process: the Slint interface talks to the core
directly, there is no local web server and no IPC.

```
            ┌──────────────┐     ┌──────────────┐
            │   bd-app     │     │   bd-cli     │
            │ (Slint GUI)  │     │ (`bdl` tool) │
            └──────┬───────┘     └──────┬───────┘
                   │                    │
            ┌──────▼────────────────────▼──────┐
            │             bd-core              │
            │ modules · settings · queue ·     │
            │ grabber · hosters · free mode ·  │
            │ accounts · extraction · video ·  │
            │ torrent · Click'n'Load · …       │
            └──────────────┬───────────────────┘
                           │
            ┌──────────────▼───────────────────┐
            │              bd-net              │
            │ segmented HTTP engine · resume · │
            │ retries · rate limit · DoH DNS   │
            └──────────────────────────────────┘
```

Dependencies only point down: `bd-net` knows nothing about hosters,
`bd-core` knows nothing about Slint.

## Crates

### `bd-net`: the download engine

| File | Role |
|---|---|
| `engine.rs` | `Engine`, `DownloadRequest`, `DownloadControl`, `Progress`: splits a file into byte ranges, preallocates it, writes each segment at its offset, retries with backoff, adapts connections per host on 429/503 |
| `state.rs` | The `name.part.json` file that makes exact resume possible |
| `probe.rs` | `RemoteFile`: what the server says (size, ranges, name) |
| `rate.rs` | `RateLimiter` for global and per-download speed limits |
| `name.rs` | File names from URLs and headers, sanitised for Windows |
| `dns.rs` | Optional DNS over HTTPS used by every HTTP client of the app |
| `error.rs` | `Error` / `Result` |

### `bd-core`: everything that is not UI

| File | Role |
|---|---|
| `modules.rs` | `ModuleRegistry` and `BUILTIN_MODULES`: every feature is a module with dependencies |
| `settings.rs` | `Settings` persisted as TOML; extra options live in a string map so new options never break old files |
| `manager.rs` | `DownloadManager`: the queue (HTTP, video, torrent jobs), priorities, pause/resume, saved between sessions |
| `task.rs` | `LinkResolver`, `Resolved`, `ControlSlot` (progress, status text, wait countdown, cancel) |
| `grabber.rs` | Link grabber: online check, packages, mirrors, selection |
| `links.rs` | Finding links in any text (URL lists, forum posts, HTML), including `hxxp://` obfuscations |
| `packages.rs` | Package names for multi-part archives, archive detection, file links in web pages |
| `hosters.rs` | `HOSTERS` table, `HosterResolver` (debrid → premium → free), link checks, debrid and premium API clients |
| `free.rs` | Free mode flows (forms, waits, code captchas handed to the user), HTML helpers |
| `accounts.rs` | `SERVICES` table, account list (`accounts.toml`) and secrets in the Windows Credential Manager |
| `decrypt.rs` | Folder / list links expanded into files |
| `clicknload.rs` | Local Click'n'Load listener |
| `video.rs`, `tools.rs` | yt-dlp and ffmpeg, installed on demand |
| `torrent.rs` | BitTorrent and magnets through librqbit |
| `extract.rs` | ZIP, 7z and RAR extraction with password lists |
| `checksum.rs` | CRC / MD5 / SHA verification |
| `reconnect.rs` | Asking the internet box for a new IP (UPnP or a custom command), optional |

### `bd-app`: the desktop application

| Path | Role |
|---|---|
| `ui/*.slint` | Screens (`downloads`, `grabber`, `accounts`, `extraction`, `settings`), `theme.slint` design tokens, shared `widgets.slint` |
| `src/main.rs` | Start-up: settings, engine, manager, resolver, window, timers |
| `src/app.rs` | `App` state shared by callbacks and the refresh timer; `UiMsg` messages from background tasks |
| `src/wiring.rs` | All UI callbacks |
| `src/schema.rs` | Every option shown in Settings (English and French text, defaults, ranges). The Settings UI is generated from this table |
| `src/captcha.rs` | In-app dialog for code and image captchas |
| `src/i18n.rs` | Language selection and the few texts built in Rust |
| `src/tray.rs`, `mini.rs`, `platform.rs`, `scheduler.rs` | Tray icon, mini window, Windows specifics, scheduler |
| `src/snapshot.rs` | Headless screenshots (`snapshot` feature) |
| `translations/` | gettext `.po` files bundled at build time |

### `bd-cli`

`bdl get` and `bdl probe`: the engine without the UI. Useful to reproduce
transfer bugs.

## How a link becomes a file

1. **Capture.** A link arrives from the clipboard watcher, a paste,
   Click'n'Load or drag and drop (`App::add_links`).
2. **Grabber.** Unless "start automatically" is on, it goes to the
   `Grabber`: `decrypt::expand` turns folders into files,
   `hosters::check_link` gets name / size / online status, and files are
   grouped into packages with their mirrors.
3. **Queue.** The user starts a selection: `Grabber::take_selected` produces
   `JobSpec`s that `DownloadManager` schedules (`max_parallel`, priorities).
4. **Resolve.** For an HTTP job the manager calls
   `LinkResolver::resolve(url, slot)`. In the app this is `HosterResolver`:
   - links of unknown sites are sniffed: a file is downloaded as is, an
     unknown XFileSharing page goes through free mode;
   - known hosters try, in order (configurable): **debrid accounts**,
     **hoster premium API**, then **free mode** if enabled;
   - while it works, the resolver writes into the `ControlSlot`: a status
     text ("Real-Debrid…"), or a wait countdown that the UI draws inside the
     progress bar.
5. **Download.** The resulting `Resolved { url, headers, file_name, via }`
   becomes a `DownloadRequest` for `bd-net::Engine`.
6. **Post-processing.** When a package finishes: checksum, extraction, event
   scripts and notifications, depending on enabled modules.

## Design rules

- **One process, no server.** Do not add a listening socket except opt-in
  modules like Click'n'Load.
- **Everything optional is a module** (`modules.rs`), off means it does not
  run.
- **Secrets never touch a plain file**: they go through `AccountStore` and
  the system keyring.
- **No `unsafe`** (`unsafe_code = "deny"` workspace-wide).
- **Free mode is a visitor, not an attacker**: it follows the site's own
  steps and waits, and hands any captcha to the user. See
  [free-mode-and-captchas.md](free-mode-and-captchas.md).
- **Settings are forward compatible**: new options go into
  `Settings::options` and `bd-app/src/schema.rs`, stored as text.
