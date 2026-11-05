# Changelog

All notable changes to hosterm are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Windows releases are currently **unsigned** (Authenticode signing postponed —
see [docs/STATE.md](docs/STATE.md)), so SmartScreen may warn on first run.

## [Unreleased]

## [0.13.3] - 2026-11-05

### Added
- Sidebar section navigation: a vertical Hosts / Keys / Ansible menu at the top
  of the sidebar swaps the sidebar body between the host list, an inline SSH keys
  manager, and an Ansible launcher. The footer (Raw config, Local, Reload,
  Settings) and the Google account chip are unchanged.

### Fixed
- Google sign-in now surfaces the token endpoint's actual error on an HTTP 400
  (for example `invalid_request (client_secret is missing.)`) instead of a bare
  status code, so a failed exchange is diagnosable. No token material is logged.

## [0.13.2] - 2026-11-05

### Fixed
- The Settings → General and terminal status-bar version now report `v0.13.2`;
  v0.13.1 had shipped the OAuth fix correctly but still displayed `v0.13.0`.

## [0.13.1] - 2026-11-05

### Fixed
- Google sign-in on Windows: the OAuth URL was opened via `cmd /C start`, whose
  `&` handling truncated the URL and made Google return
  `Error 400: invalid_request` (`Required parameter is missing: response_type`).
  Windows now opens the browser with `rundll32 url.dll,FileProtocolHandler`,
  passing the full URL as a single argument with no shell involved.
- Google sign-in no longer freezes the window: the `google_sign_in` command ran
  the blocking loopback flow (up to a 5-minute wait) on the UI thread, so a
  failed/stalled sign-in showed *Not Responding*. It now runs off-thread.

## [0.13.0] - 2026-11-05

This release adds local workflows, live theme palettes, and the first safe Ansible
runner while keeping `~/.ssh/config` as hosterm's source of truth.

### Added
- Local terminal tabs that spawn the platform shell directly, without SSH or command-string interpolation.
- User snippet create/edit/delete storage plus a Settings → General view for shell, platform, and version information.
- Slate, Ink, and Light live palettes for both xterm and CodeMirror, alongside Dusk.
- Ansible v1 panel and PTY runner: playbook picker, key/SSH-agent hosts, streamed output, and bounded history.

### Changed
- Bundled command snippets now ship empty; user snippets remain isolated in app data.
- Terminal and editor colors now come from one contrast-gated theme-palette source of truth.
- Test-only Argon2 parameters reduce the Rust test suite from roughly 494 seconds to about 11 seconds; production Argon2 parameters remain unchanged at the OWASP floor.

### Fixed
- The Windows CI Rust test no longer attempts to create a filename containing Windows-illegal shell metacharacters.

### Security
- A successful Ansible `--check --diff` dry-run is mandatory before a real run is enabled.
- Ansible uses fixed argv arrays rather than `sh -c`, `bash -c`, or `cmd /C`; host-key checking remains enabled.
- Generated inventory never carries passwords or private-key bytes, and hostile recap text is rendered inert.

### Known limitations
- Ansible v1 supports key/SSH-agent hosts only; password-host support is deferred to wave 2.
- Windows installers remain unsigned, so Microsoft SmartScreen may warn on first launch.

## [0.12.0] - 2026-11-01

Dusk replaces Arus as hosterm's active visual identity and adds the first
settings, snippets, and exact-command AI approval surfaces.

### Added
- Persistent app-data settings and bundled command snippets, isolated from SSH config and the secrets sidecar.
- In-app Settings/Appearance panel with a custom accent and contrast-safe derived ink.
- Searchable command snippets drawer with clipboard feedback.
- Visual Google account mock with no OAuth, session, or network behavior.
- Visual AI assistant with Run once, exact `(host, command)` Always allow, and Settings revoke flow; approved commands still run through the existing real-ssh PTY.

### Changed
- Replaced the Abyss Emerald/Arus skin with the Dusk token system and shell geometry.
- Reorganized product documentation around canonical PRD, SDD, TSD, STATE, and Dusk design artifacts.
- Expanded the canonical headless smoke suite to 343 checks.

### Security
- Removed a dynamic `innerHTML` sink from the Google account chip.
- Added host-alias rejection for leading `-`, carriage return, and line feed before approval persistence or PTY execution.
- Added frozen contrast, store-isolation, corrupt-settings, visual-weight, and anti-Abyss invariants.

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

[0.13.3]: https://github.com/MEKL20/hosterm/releases/tag/v0.13.3
[0.13.2]: https://github.com/MEKL20/hosterm/releases/tag/v0.13.2
[0.13.1]: https://github.com/MEKL20/hosterm/releases/tag/v0.13.1
[0.13.0]: https://github.com/MEKL20/hosterm/releases/tag/v0.13.0
[0.12.0]: https://github.com/MEKL20/hosterm/releases/tag/v0.12.0
[0.11.0]: https://github.com/MEKL20/hosterm/releases/tag/v0.11.0
[0.10.0]: https://github.com/MEKL20/hosterm/releases/tag/v0.10.0
[0.9.0]: https://github.com/MEKL20/hosterm/releases/tag/v0.9.0
[0.8.0]: https://github.com/MEKL20/hosterm/releases/tag/v0.8.0
[0.7.3]: https://github.com/MEKL20/hosterm/releases/tag/v0.7.3
[0.7.2]: https://github.com/MEKL20/hosterm/releases/tag/v0.7.2
[0.7.1]: https://github.com/MEKL20/hosterm/releases/tag/v0.7.1
