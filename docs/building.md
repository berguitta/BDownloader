# Building, running and testing

## Requirements

- **Windows 10 or 11.** Other platforms are not supported yet (the account
  store uses the Windows Credential Manager, and some features call Windows
  APIs). Porting is on the [roadmap](roadmap.md).
- **Rust stable, MSVC toolchain**, at least the version in
  `Cargo.toml` (`rust-version`). Install with [rustup](https://rustup.rs).
- **Visual Studio Build Tools** with the *Desktop development with C++*
  workload (needed by the bundled UnRAR library and the Windows resources).

## Build and run

```powershell
cargo build --release                 # whole workspace
cargo run --release -p bd-app         # desktop app (binary: bdownloader.exe)
cargo run --release -p bd-cli -- get <url> -o D:\Downloads -c 8   # CLI (binary: bdl.exe)
cargo run --release -p bd-cli -- probe <url>
```

Release binaries land in `target\release\`. Debug builds show a console with
logs; release builds of the app use the Windows GUI subsystem.

## The checks CI runs

Run these before every pull request. CI (`.github/workflows/ci.yml`, on
`windows-latest`) fails on any of them:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Clippy warnings are errors in CI. `rustfmt.toml` sets `max_width = 120`.

## Tests

- **Unit tests** live next to the code (`#[cfg(test)] mod tests`). Hoster
  parsing helpers in `free.rs` are tested with small HTML snippets; add one
  for every site quirk you handle.
- **Integration tests**: `crates/bd-net/tests/engine.rs` (local HTTP server,
  ranges, resume) and `crates/bd-core/tests/manager.rs` (queue).
- **Live free-mode test** (ignored by default, hits the real site):

  ```powershell
  $env:BD_FREE_URL = "https://hoster.example/abc123/file.zip"
  cargo test -p bd-core --test free_live -- --ignored --nocapture
  ```

  Add `$env:BD_SECURE_DNS = "cloudflare"` (or `quad9`, `google`, or an
  `https://` DoH URL) if your provider blocks the hoster. A code captcha is
  printed and left unanswered. Never commit real links to tests.

## Useful environment variables

| Variable | Effect |
|---|---|
| `BD_LOG` | Log filter, e.g. `debug` or `bd_core=debug,info` |
| `BD_SETTINGS` | Use another settings file (development, portable tests) |
| `BD_FREE_DUMP` | Folder where free mode saves each HTML page of a flow (`step0.html`, `step1.html`…). The fastest way to debug a hoster |
| `BD_FREE_URL` | Link used by the live free-mode test |
| `BD_SECURE_DNS` | DoH provider for the live free-mode test |
| `BD_SNAPSHOT*` | Headless screenshots, see below |

Settings live in `%APPDATA%\BDownloader\config\settings.toml`. A
`portable.flag` file next to the executable keeps all data in a `data`
folder beside it.

## Headless screenshots

The `snapshot` feature renders the real UI without a window and saves a PNG.
It is handy for pull requests that change the UI and for agents that cannot
see a screen:

```powershell
$env:BD_SNAPSHOT = "out.png"
$env:BD_SNAPSHOT_PAGE = "3"      # 0 downloads, 3 settings
$env:BD_SNAPSHOT_THEME = "2"     # 1 light, 2 dark
cargo run --release -p bd-app --features snapshot
```

Other variables: `BD_SNAPSHOT_SECONDS`, `BD_SNAPSHOT_LINKS`,
`BD_SNAPSHOT_SELECT`, `BD_SNAPSHOT_SIZE` (`1440x900`). See
`crates/bd-app/src/snapshot.rs`. Use a temporary settings file
(`BD_SETTINGS`) so screenshots do not show your own folders.

## Icons

`assets/logo.svg` is the source of every icon. After changing it:

```powershell
cargo run -p xtask -- icons
```

The generated PNG and ICO files are committed, so normal builds do not need
this step.
