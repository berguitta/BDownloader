# Free mode, captchas and the road to full automation

This page explains what BDownloader can automate on free file hosters, where
it has to stop, and why. It is the most common question from users and the
most important rule for contributors.

## The goal

Copy a link, come back later, find the file on disk. With a **debrid or
premium account** BDownloader already does exactly that. Without an account,
it goes as far as each site allows.

## What free mode automates today

Free mode (`crates/bd-core/src/free.rs`) behaves like a patient visitor of the
site, in the background, for many files at once:

| Step on the site | What BDownloader does |
|---|---|
| "Free download" button, multi-step forms | Fills and submits them for you |
| Wait timer ("please wait 60 seconds") | Waits, with the countdown shown inside the progress bar; the rest of the queue keeps going |
| Per-IP wait between files (e.g. 1fichier "you must wait 15 minutes") | Waits, or asks your internet box for a new IP if you turned on the Reconnect module |
| Code or image captcha ("type the characters") | Shows it in a small BDownloader dialog, you type the answer, the download goes on. BDownloader never reads it for you |
| File removed | Marks the link offline in the link grabber |
| Hoster blocked by your provider's DNS | Optional **Secure DNS** in **Settings → Connection** (DNS over HTTPS: Cloudflare, Quad9, Google or a custom server). Only BDownloader's own downloads use it; Windows, other programs, yt-dlp and torrents keep the system DNS |

## Where it stops: interactive captchas

Some hosters put an **interactive "I am not a robot" widget** in front of the
free download: Cloudflare **Turnstile**, Google **reCAPTCHA**, **hCaptcha**.
Today this is the case for:

- **Rapidgator** free mode (always);
- **DataNodes**, whose last step inserts a Turnstile widget with a script;
- **Uploady** and some other XFileSharing sites, depending on the day and on
  your IP (the site keeps refusing the first step and wants a real browser);
- many other hosters marked `Free::Browser` in `hosters.rs` (Turbobit,
  Nitroflare, Mega, Keep2Share…).

These widgets exist precisely to make sure a human, in a real browser, is
present. **BDownloader does not solve them, does not send them to solving
services, and does not try to get around them.** This is a project rule, not
a missing feature: it keeps the project on the right side of the hosters'
terms, keeps users' accounts and IPs safe, and keeps the code maintainable.

When free mode meets such a widget, the resolver returns
`FreeError::Captcha(page)` and follows the user's setting
(**Settings → Captcha**, option `captcha-mode`):

| Setting | Behaviour |
|---|---|
| Open the page in my browser (default) | The hoster page opens in the user's default browser; they finish there |
| Notify me only | A notification says which file needs a browser |
| Skip the file | The file is skipped, nothing interrupts the user |

### Why there is no embedded browser

An embedded WebView was tried to show these pages inside BDownloader. It was
**removed by maintainer decision**, and bringing it back is not planned:
it adds a large platform dependency to a project whose promise is one small
native executable, and it would not change the rule above (the user still has
to answer the widget). Please do not open pull requests that reintroduce it.

## How to get full automation

| Need | Answer |
|---|---|
| Zero interaction, any hoster | A **debrid account** (Real-Debrid, AllDebrid, Premiumize, Debrid-Link, TorBox, Offcloud, LinkSnappy, Mega-Debrid) |
| Zero interaction, one hoster | That hoster's **premium account** (1fichier, Rapidgator APIs today) |
| Free, no account | Hosters without interactive captchas are already automatic; for the others, one click in the browser per file |

## Ideas we are exploring (help welcome)

All of these keep the human answer to the captcha where it belongs: with the
user.

- **Batch hand-off**: group every file that needs a browser check into one
  list, so the user clears them in one sitting instead of being interrupted
  for each file.
- **Smarter routing**: when a file has mirrors, prefer the mirror that does
  not need a browser, and only fall back to a browser mirror when nothing else
  is left.
- **Clear status in the queue**: "needs your browser" as its own state, with a
  button to open the page again later, and automatic retry of the free flow
  once the per-IP limit has passed.
- **A companion browser extension (to discuss)**: after the user finishes the
  check in their own browser, the extension sends the final file link back to
  BDownloader, the way Click'n'Load already sends links.
- **Keeping flows alive**: sites change their pages often. Fixtures and live
  tests per hoster make breakages visible fast; see
  [adding-a-hoster.md](adding-a-hoster.md).

## What we will not accept

- Captcha solving: automated recognition, machine learning models, or paid
  solving services.
- Code that hides or fakes what BDownloader is to defeat a site's bot
  protection, or that tries to pass browser challenges without a human.
- Ways around waits or download limits other than the user's own optional
  Reconnect setting.
- An embedded WebView (see above).

If you are not sure whether an idea fits, open a discussion before writing
code.
