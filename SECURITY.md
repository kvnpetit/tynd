# Security Policy

Thanks for helping keep Tynd and its users safe. This document describes how to report a vulnerability and what you can expect from us in return.

## Supported Versions

| Version | Supported |
|---------|-----------|
| 1.x     | Yes       |
| 0.x     | No        |

We provide security fixes only for the current `1.x` line. Pre-1.0 releases were development previews and receive no security patches — please upgrade.

## Reporting a Vulnerability

**Please do not open a public GitHub issue for security reports.**

Open a private GitHub Security Advisory at
<https://github.com/tynd-apps/tynd/security/advisories/new>.
Settings -> Security -> Report a vulnerability.

Please include:

- A description of the issue and the impact you believe it has.
- Steps to reproduce, ideally with a minimal proof-of-concept.
- Affected versions / commit SHA.
- Your platform (OS, architecture, runtime: full / lite / custom).
- Whether you'd like public credit (and under what name / handle).

## Response Timeline

- **Acknowledgement** within **72 hours** of receipt.
- **Triage and severity assessment** within **7 days**.
- **Fix target** for high-severity issues: **14 days** from triage. Lower-severity issues are batched into the next minor release.
- **Coordinated disclosure** — we'll agree a disclosure date with you before publishing the advisory.

If you do not hear back within 72 hours, please re-send to make sure the message wasn't lost.

## CVE / GHSA Numbering

Tynd uses **GitHub-issued GHSA identifiers** as the canonical record for each vulnerability. GitHub forwards qualifying advisories to MITRE for CVE assignment, so a CVE may also be issued — both will be referenced in the published advisory and the changelog.

## Scope

**In-scope** — please report:

- Path traversal in the host runtime, IPC handlers, or build-time bundling
  (escaping `app.tyndpkg`, project root, cache directories).
- IPC injection or command-injection across the host -> backend channel.
- Code-signing bypass or signature verification flaws in the bundler / host.
- Updater MITM, downgrade attacks, or signature-verification bypass on
  update channels.
- Arbitrary code execution triggered by malicious project config
  (`tynd.config.ts`, plugin manifests, fixture files) or by a malicious
  `app.tyndpkg`.
- Privilege escalation via the host's elevated APIs.
- WebView sandbox escapes or content-isolation bypass.

**Out-of-scope** — please do not file these as security reports:

- DoS via OS-imposed quotas (file handles, memory, disk).
- Risks inherent to running untrusted plugin code on the user's machine —
  plugins execute by design with user-level trust. Document the risk in
  the plugin's README instead.
- Self-XSS or attacks requiring an already-compromised host machine.
- Findings from automated scanners without a working exploit.
- Denial of service via deliberately malformed but locally-controlled
  inputs (e.g., feeding the runtime a 100 GB project file).

## Recognition

Reporters who follow coordinated disclosure are credited in:

- The published GHSA acknowledgments section.
- The release `CHANGELOG.md` entry for the fix.
- An optional thank-you in the release announcement, if you'd like one.

We don't run a paid bounty program at this time.

## Maintainer

Security contact: **Kevin Petit** ([@kvnpetit](https://github.com/kvnpetit)).
