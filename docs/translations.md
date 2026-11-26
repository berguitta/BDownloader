# Translations

BDownloader ships in **English** (source language) and **French**. Texts come
from three places:

| Where | Format | Example |
|---|---|---|
| Slint UI (`crates/bd-app/ui/*.slint`) | `@tr("…")` strings, translated with gettext `.po` files in `crates/bd-app/translations/<lang>/LC_MESSAGES/bd-app.po`, bundled into the binary at build time | Buttons, labels, menus |
| Settings (`crates/bd-app/src/schema.rs`) | English and French text side by side in each option entry | Option titles and help |
| Rust texts (`crates/bd-app/src/i18n.rs`) | `t("…")` with a lookup table | Statuses, notifications, tray menu |

## Improving the French translation

Edit `crates/bd-app/translations/fr/LC_MESSAGES/bd-app.po` (any `.po` editor
such as Poedit works), or the French strings in `schema.rs` and `i18n.rs`.
Rebuild and switch the language in **Settings → Language**.

## Adding a new language

Adding a language currently needs a small refactor, which is a great first
contribution:

1. Create `crates/bd-app/translations/<code>/LC_MESSAGES/bd-app.po` from the
   French file, keep the `msgid`s and translate the `msgstr`s. Set the
   `Language` and `Plural-Forms` headers.
2. Add `("<code>", "<native name>")` to `LANGUAGES` in `i18n.rs`.
3. Rust texts and Settings texts are English/French only today
   (`i18n::french` and the two-language entries in `schema.rs`). Generalise
   them, for example by moving these strings into the same `.po` catalogues,
   then translate them.

Open an issue first so two people do not do the same refactor.

## Rules

- English is the source text, in plain words and sentence case.
- Keep placeholders exactly (`{}`, `{n}`).
- Keep translations short enough for the UI; check with a
  [headless screenshot](building.md#headless-screenshots).
