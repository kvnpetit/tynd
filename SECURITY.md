# Security Policy

Thanks for helping keep Tynd and its users safe. This document describes how to report a vulnerability and what you can expect from us in return.

## Supported Versions

| Version | Supported |
|---------|-----------|
| 1.x     | Yes       |

We provide security fixes for the current `1.x` line.

## Reporting a Vulnerability

**Please do not open a public GitHub issue for security reports.**

Open a private GitHub Security Advisory at
<https://github.com/kvnpetit/tynd/security/advisories/new>.
Settings -> Security -> Report a vulnerability.

Please include:

- A description of the issue and the impact you believe it has.
- Steps to reproduce, ideally with a minimal proof-of-concept.
- Affected versions / commit SHA.
- Your platform (OS, architecture) and the plugins listed in your `tynd.config.ts`.
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
- **MCP server (`@tynd/mcp`)**: JSON-RPC framing or injection over stdio;
  path traversal in tool arguments; crafted tool responses (file contents,
  logs) that steer an agent beyond the user's intent; a non-Tynd local
  process impersonating a running app during discovery; secrets or tokens
  leaked into agent-visible output.
- **Plugin SDK (`@tynd/plugin-sdk`, `tynd-plugin-sdk`)**: `unsafe` boundaries
  that allow memory corruption from plugin input; injection across the
  host <-> plugin event channel; SDK APIs that let a plugin break host
  isolation beyond the documented capability model.
- **Official plugins**: path traversal in plugin file APIs, SQL injection in
  `@tynd/plugin-sql` helpers, privilege escalation from malicious plugin input.
- **Docs site**: XSS in rendered MDX content, workflow misconfigurations that
  expose secrets, open redirects on documentation links.

When reporting against the plugin SDK or a plugin, name the package
(npm package or Rust crate) and its version.

**Out-of-scope** — please do not file these as security reports:

- DoS via OS-imposed quotas (file handles, memory, disk).
- Risks inherent to running untrusted plugin code on the user's machine —
  plugins execute by design with user-level trust. Document the risk in
  the plugin's README instead. The plugin SDK does not sandbox third-party
  code.
- Risks inherent to letting an AI agent operate through `@tynd/mcp` on a
  developer machine: the agent runs with user-level trust by design.
- Missing security headers on third-party assets the docs site does not control.
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
