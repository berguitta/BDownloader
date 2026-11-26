# Security policy

## Supported versions

BDownloader is in early preview. Only the latest commit on `main` receives
security fixes.

## Reporting a vulnerability

Please **do not open a public issue** for security problems. Use GitHub's
private vulnerability reporting instead: on the repository page, open the
**Security** tab and choose **Report a vulnerability**.

Useful details:

- what an attacker can do, and under which conditions;
- steps to reproduce, or a proof of concept;
- the commit or version you tested.

You will get an answer as soon as possible. Once a fix is ready, the report
can be published with credit to you if you wish.

## Scope

Examples of what we want to hear about:

- leaks of account secrets (the Windows Credential Manager entries, logs,
  settings files);
- the Click'n'Load listener or any other local socket accepting input it
  should not;
- path traversal when naming, moving or extracting files;
- code execution through downloaded files, event scripts or settings.

Hoster sites refusing BDownloader, or captchas, are not security issues;
please open a regular issue.
