# Changelog

All notable changes to hosterm are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Windows releases are currently **unsigned** (Authenticode signing postponed —
see [docs/STATE.md](docs/STATE.md)), so SmartScreen may warn on first run.

## [0.15.2] - 2026-11-11

### Changed
- The Edit SSH key dialog is now larger so a prefilled private key is
  readable: the box is wider (up to 780px, clamped to the viewport and to 88%
  of the window height) and the key editor is a tall monospace field
  (320px minimum, resizable) that scrolls long key lines horizontally instead
  of soft-wrapping them. Only the Edit-key dialog is resized; other dialogs
  keep their previous size.

## [0.15.1] - 2026-11-09

### Changed
- Editing an SSH key now loads the current private key into the editor, so you
  can tweak it in place instead of re-pasting the whole key. The editor still
  clears the contents when closed, and only keys the app manages can be read
  or edited.

## [0.15.0] - 2026-11-09

### Added
- SSH keys you created in the app can now be edited: an Edit button on each
  managed key opens an editor to rename it and replace its contents. Legacy
  keys outside the app's keys folder stay read-only.

### Changed
- The host list no longer groups hosts by the prefix before a dash; it is now
  a single flat, alphabetically sorted list.
- The SSH keys list shows just the key name (the full path is available on
  hover) and fills the panel without being clipped.
- Each sidebar section no longer shows a "hosterm" brand header; the host
  section is titled "Host", matching "SSH keys" and "Ansible".

### Fixed
- The snippets panel no longer shows the editor form on open. It starts as the
  snippet list; the form appears only after clicking "+ New", and Cancel now
  closes it.

## [0.14.9] - 2026-11-07

### Fixed
- Restoring from Drive could get stuck after picking a backup: the chooser's
  dimming overlay lingered for a moment and swallowed the first click on the
  passphrase prompt. The chooser now closes immediately, so the passphrase
  prompt is clickable right away.

## [0.14.8] - 2026-11-07

### Fixed
- The restore backup chooser could open fully transparent (invisible) because
  its new fade-in styling was applied but the entry animation was only ever
  triggered for the passphrase dialog. The chooser now fades in and is visible.

## [0.14.7] - 2026-11-07

### Changed
- The restore backup chooser is now a centered modal (matching the backup
  passphrase dialog) instead of an inline panel, so it no longer disturbs the
  surrounding layout.
- Removed the sidebar collapse/expand arrow buttons. The section panel is now
  toggled solely by clicking the active rail item (Hosts/Keys/Ansible).

## [0.14.6] - 2026-11-07

### Added
- The backup passphrase is now optional. Leaving it empty still encrypts the
  backup, using a random key stored in this device's OS keyring (a dedicated
  entry, separate from the Google sign-in token). Such a backup restores
  automatically on the same device; restoring it on another machine needs that
  device's key and fails with a clear message otherwise.
- Restore now lets you pick which backup to restore from a list (newest first,
  showing host and date/time), since there can be several.

### Changed
- Clicking the active sidebar rail item (Hosts/Keys/Ansible) now collapses or
  expands the section panel; clicking another item switches section and expands.

### Security
- Empty-passphrase backups are never stored in plaintext. The device key lives
  only in the OS keyring, never in the `.htbk` file, its Drive metadata, or logs.
  Passphrase and device-key containers cannot be confused or downgraded, and
  OAuth remains limited to `drive.file`.

## [0.14.5] - 2026-11-07

### Fixed
- The backup/restore passphrase fields no longer show two eye icons on Windows.
  WebView2's native password reveal control (`::-ms-reveal`/`::-ms-clear`) is now
  suppressed so only hosterm's own embedded eye icon appears.

## [0.14.4] - 2026-11-07

### Added
- New encrypted backup snapshots are stored inside an app-created
  `HostermBackups` Google Drive folder. Existing app-created backups at Drive
  root remain discoverable for restore compatibility.
- Each backup passphrase field now has its own accessible eye icon embedded in
  the field, with independent reveal state and masked reset on close or reopen.

### Fixed
- Drive media downloads now remain byte-exact instead of passing encrypted
  `.htbk` data through UTF-8 text decoding, fixing restore failures reported as
  `malformed .htbk container: ct_len mismatch`.
- Windows backup names now use `%COMPUTERNAME%` instead of falling back to
  `unknown-host`; filename components are sanitized before upload.

### Changed
- Repeated backups remain timestamped snapshots: each Backup action creates a
  new file rather than overwriting an earlier snapshot.

### Security
- OAuth remains limited to `drive.file`; folder metadata contains only the
  fixed folder name and MIME type. Ciphertext is never decoded as text, and
  passphrases remain ephemeral, wiped, and re-masked on prompt close.

## [0.14.3] - 2026-11-07

### Added
- Backup and restore passphrase prompts now include one keyboard-accessible
  Show/Hide control. It reveals both backup fields together and always resets to
  masked when the prompt closes or reopens.

### Fixed
- Windows Google sign-in now stores its refresh token in Windows Credential
  Manager. Previous Windows builds omitted keyring's `windows-native` feature,
  silently used a per-entry mock store, and therefore lost the token between
  sign-in and the first Drive backup.
- Backup and restore success or failure notifications now auto-close after five
  seconds, while retaining the Close button for early dismissal.

### Security
- OAuth remains PKCE S256 with state validation, a loopback-only single-shot
  listener, `drive.file` scope, CI-injected client credentials, OS-keyring
  refresh-token storage, and no token logging. Passphrases remain ephemeral and
  are wiped and re-masked when the prompt closes.

## [0.14.2] - 2026-11-07

### Added
- Backup, restore, and empty-Drive outcomes now appear as accessible shell-level
  toasts that remain visible after the Google account menu closes.
- The signed-in account menu now has a clearer identity row and grouped Google
  Drive actions while preserving the Dusk visual system.

### Fixed
- Google sign-in no longer reports success when the authorization exchange omits
  a durable refresh token. It now fails before presenting an account and gives
  actionable re-authorization guidance instead of letting the first Drive action
  fail as “not signed in.”
- The backup passphrase prompt is now a fixed, smoothly animated overlay, so it
  no longer displaces the rail, section panel, or workspace.
- Toast overflow eviction is synchronous, preventing a fourth notification from
  hanging the UI while retaining persistent error evidence.

### Security
- OAuth remains PKCE S256 with state validation, a loopback-only single-shot
  listener, `drive.file` scope, OS-keyring refresh-token storage, and no token
  logging. Dynamic account and Drive messages continue to render as inert text.

## [0.14.1] - 2026-11-07

### Fixed
- Google sign-in now uses the new Desktop OAuth client with credentials injected
  from GitHub Actions secrets during official builds. Token exchange and refresh
  supply the required installed-app values while PKCE
  S256, loopback state validation, Drive scope, keyring storage, and token
  logging protections remain unchanged.

## [0.14.0] - 2026-11-06

### Changed
- Reworked the shell into a persistent 56px left navigation rail, a contextual
  section panel, and the main workspace. The Google account control now stays at
  the bottom of the rail, with Settings inside its account menu.
- Add host, Raw config, Local, and Reload config now appear only in Hosts. Keys
  and Ansible get focused contextual views without hidden Host actions remaining
  in the keyboard order.
- Panel collapse and narrow-window behavior now keep the rail available and refit
  both normal and split terminals after layout changes.

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
