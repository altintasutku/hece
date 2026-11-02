# Security policy

## Supported versions

Hece is in early development. Only the latest release gets security fixes; please update before
reporting. Fixes land on `main` first and ship in the next release.

| Version | Supported |
| --- | --- |
| Latest release | Yes |
| Older releases | No |
| Unreleased `main` | Best effort |

## Reporting a vulnerability

Please report vulnerabilities privately, not in a public issue, pull request or discussion.

Use GitHub's private vulnerability reporting: open the repository's
[Security tab](https://github.com/altintasutku/hece/security) and choose **Report a vulnerability**.
Only the maintainer sees the report.

Please include:

- The Hece version (`hece --version` or the release zip name) and your Windows version.
- The affected component (see the scope below) and the impact you think it has.
- Steps to reproduce, or a proof of concept. Synthetic input only: do not send real recordings,
  transcripts or API keys. If a key was exposed, rotate it with its provider first.
- Any suggested fix or mitigation, if you have one.

## What to expect

Hece is maintained by one person in their spare time, so responses are best effort and no response
time is promised. As a rough guide, you should hear back within a couple of weeks. Once the report is
confirmed, the fix is developed in a private security advisory, released, and the advisory is
published with credit to you unless you prefer to stay anonymous. Please give a reasonable time for
a fix before disclosing publicly.

## Scope

In scope:

- **Installer and uninstaller scripts** (`install.ps1`, `uninstall.ps1`, the release zip): anything
  that writes outside the per-user install folder, gains privileges, or can be hijacked.
- **Model and runtime downloads**: anything that lets a download skip or defeat the pinned revision
  and sha256 checks, or write outside `%LOCALAPPDATA%\hece`.
- **Local API** (the OpenAI-compatible server on `127.0.0.1`, off by default): access without the
  token, reachability from other machines or from web pages, request handling that crashes the
  daemon or reads files.
- **Local IPC**: the daemon control pipe and the engine worker pipe being usable by another user.
- **API key storage**: keys leaking out of the Windows Credential Manager into logs, config,
  history, crash output or the network beyond the configured provider.
- **Paste injection**: transcripts being inserted somewhere other than the focused window, leaking
  into clipboard history or the cloud clipboard, or Hece being driven to type or paste
  attacker-controlled input.
- **Logs and history**: audio, transcripts or keys ending up in `hece.log`.

Out of scope:

- Anything that needs an attacker already running code as the same Windows user, or as
  administrator.
- Cloud speech or LLM providers themselves (report those to the provider), and what they do with
  audio you chose to send them.
- Speech recognition quality, hallucinations or wrong transcripts, unless they are an injection
  vector.
- Pasting into elevated windows being blocked by Windows (UIPI); this is a documented limit.
- Missing hardening without a concrete attack (for example, unsigned binaries before code signing
  lands), denial of service by the local user against their own daemon.
- Vulnerabilities in dependencies without a way to reach them through Hece; Renovate keeps
  dependencies up to date.
- The project website.
