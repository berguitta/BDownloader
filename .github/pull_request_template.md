## What

<!-- One or two sentences: what does this change do? -->

## Why

<!-- The problem it solves. Link the issue if there is one (Fixes #123). -->

## How it was tested

<!-- Unit tests, live free-mode test, manual steps, screenshots for UI changes.
     Never paste private links, API keys, tokens or cookies. -->

## Checklist

- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass
- [ ] New parsing logic has unit tests with anonymised fixtures
- [ ] User-visible texts are in English (French added where the surrounding code has it)
- [ ] Docs updated if behaviour changed
- [ ] No captcha solving or anti-bot circumvention (see `docs/free-mode-and-captchas.md`)
