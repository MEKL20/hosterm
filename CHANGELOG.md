# Changelog

All notable changes to hosterm are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Windows releases are currently **unsigned** (Authenticode signing postponed —
see [docs/STATE.md](docs/STATE.md)), so SmartScreen may warn on first run.

## [0.11.0] - 2026-10-22

UX redesign "Arus" (Abyss Emerald theme) reaches the dialog + audit units.

### Added
- Password-host dialog: a "stored" badge and an opt-in note for passwords kept in the `~/.ssh/hosterm.secrets` sidecar (redesign unit U8).
- `forget_secret` IPC command to drop a stored password for one host.
- Smoke suite expanded from 196 to 225 checks, covering the full U1–U8 UI audit plus hostile-byte fixtures (redesign unit U9).

### Fixed
- `forget_secret` now propagates sidecar write/remove IO errors to the UI instead of silently reporting success when the secret is still on disk.

### Changed
- U5 transfer copy and the U3 dead-tab icon adjusted for the Arus redesign.

## [0.10.0] - 2026-10-19

Security hardening wave 1 and the first slices of the Arus redesign.

### Added
- Content-Security-Policy enforced in both the app and the dev server, with a smoke check asserting zero CSP violations.
- Persistent password-SFTP session: one `sftp` process per host, shared across tabs.
- UX redesign groundwork (theme tokens, sidebar, tab bar, terminal/divider/status bar, SFTP, skeleton rows, editor — units U1–U7).
- Fleet Run backend (parallel multi-host runner, frozen IPC contract — unit FL-1).

### Fixed
- Password copy writes to the `~/.ssh/hosterm.secrets` sidecar, never to `~/.ssh/config`.
- `withGlobalTauri` disabled (removes an unused global attack surface).

### Security
- CI actions pinned to full commit SHAs, least-privilege workflow token, dependency-audit gates.

## [0.9.0] - 2026-10-19

### Added
- Content-Security-Policy baseline (enforced policy + dev-server CSP header).
- Product documentation: PRD, STATE.md shared brief, AGENTS.md for third-party agents.

### Fixed
- Release workflow correctly attaches `.msi`/`.exe` assets (upload-artifact v4 nests under a per-pattern directory; globs now match any depth).

## [0.8.0] - 2026-10-16

### Fixed
- SFTP list scrolls correctly; transfer buttons stay visible; drag-and-drop works between panes.

## [0.7.3] - 2026-10-16

### Fixed
- Pasting multi-line text into the terminal no longer inserts blank lines (CRLF normalized to LF).

## [0.7.2] - 2026-10-09

### Fixed
- Tab drag-to-split works in WebView2 (manual mouse drag instead of HTML5 drag-and-drop, which is unreliable in WebView2).

## [0.7.1] - 2026-10-09

### Fixed
- Right-clicking a tab opens a real tab menu instead of reloading the app.

## [0.7.0 and earlier]

Early releases: core SSH client — host sidebar from `~/.ssh/config`, embedded
xterm.js terminal tabs, SFTP file browser, remote text editor, add/edit/delete
hosts with round-trip-safe config writes, key and password auth, paste-to-create
identity keys, raw config editor. See the git tag history for details.

[0.11.0]: https://github.com/MEKL20/hosterm/releases/tag/v0.11.0
[0.10.0]: https://github.com/MEKL20/hosterm/releases/tag/v0.10.0
[0.9.0]: https://github.com/MEKL20/hosterm/releases/tag/v0.9.0
[0.8.0]: https://github.com/MEKL20/hosterm/releases/tag/v0.8.0
[0.7.3]: https://github.com/MEKL20/hosterm/releases/tag/v0.7.3
[0.7.2]: https://github.com/MEKL20/hosterm/releases/tag/v0.7.2
[0.7.1]: https://github.com/MEKL20/hosterm/releases/tag/v0.7.1
