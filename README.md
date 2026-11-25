# BDownloader

A fast, native download manager written in Rust. No Java, no Python, no web
server, no browser extension required: one small executable.

> **Status: early preview (v0.1).** Direct HTTP/HTTPS links work end to end.
> Hoster modules (1fichier, Rapidgator…), debrid services, the link grabber,
> torrents, videos and extraction are on the roadmap.

## Features

- **Multi-connection downloads**: files are split into segments downloaded in
  parallel when the server allows it.
- **Exact resume**: progress is saved next to the partial file
  (`name.part` + `name.part.json`), so a download continues at the right byte
  after a pause, a crash or a reboot.
- **Polite with servers**: connections per host adapt automatically when a
  server answers `429 Too Many Requests` or `503`.
- **Modules everywhere**: every feature is a module you can turn on or off in
  Settings; dependencies are handled for you.
- **Light and dark themes**, or follow the Windows setting.
- **Paste anything**: the Add links dialog finds every link in a block of text.

## Workspace

| Crate | Role |
|---|---|
| `crates/bd-net` | Download engine: probing, segmented transfers, resume, retries, per-host limits |
| `crates/bd-core` | Module registry, settings (TOML), download queue, link extraction |
| `crates/bd-cli` | `bdl` command line tool |
| `crates/bd-app` | Desktop application built with [Slint](https://slint.dev) |

## Build

Requirements on Windows: Rust (stable, MSVC toolchain) and Visual Studio Build
Tools with the "Desktop development with C++" workload.

```powershell
cargo build --release
```

Run the desktop app:

```powershell
cargo run --release -p bd-app
```

Use the command line tool:

```powershell
cargo run --release -p bd-cli -- get https://example.com/file.zip -o D:\Downloads -c 8
cargo run --release -p bd-cli -- probe https://example.com/file.zip
```

Press Ctrl+C to stop a command line download; run the same command again to
resume it.

Run the tests:

```powershell
cargo test --workspace
```

### Settings

Settings live in `%APPDATA%\BDownloader\config\settings.toml`. Set the
`BD_SETTINGS` environment variable to use another file (portable installs,
development). Set `BD_LOG=debug` for detailed logs.

### Headless screenshots

The `snapshot` feature renders the real UI without a window and saves a PNG,
which is handy to check the UI from scripts:

```powershell
$env:BD_SNAPSHOT = "out.png"; cargo run --release -p bd-app --features snapshot
```

See `crates/bd-app/src/snapshot.rs` for the other `BD_SNAPSHOT_*` variables.

## Responsible use

BDownloader respects the rules of the services it talks to: wait times are
honoured and captchas are shown to you, never bypassed. You are responsible
for what you download.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. The desktop UI uses Slint under its royalty-free license.
