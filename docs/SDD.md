# SDD: hosterm v2

> Deliverable architect. Requirements-first lane: derived dari `docs/PRD.md` (§3/§4/§5, pm's living doc) + wave-2 feature draft §4/§5 + Arus epic. SDD v1 + its signing/updater extension are folded here; this document supersedes SDD v1 as the single architecture source.

| Field | Value |
|---|---|
| Author | crew-coding-architect |
| Inputs | `docs/PRD.md` (requirements authoritative), `docs/archive/SDD-hosterm-v1.md` (basis), `docs/SDD-signing-updater.md` (folded into §2.2/§7 ADR-9..12), `docs/EPIC-TSD-redesign-arus-v1.md`, `docs/archive/FEATURE-SPEC-wave-2-draft.md` §4/§5, `docs/FEATURE-SPEC-fleet-run-v1.md` (drives the §12 Fleet Run v1 addendum + ADR-13..16 + INV-14..19), `docs/STATE.md`, `docs/SPIKE-csp-findings.md`, `src-tauri/src/lib.rs`, `src/main.ts`, `src-tauri/tauri.conf.json`, `docs/FEATURE-SPEC-ansible-autoinstall-v1.md` (drives the §15 Ansible auto-install addendum + ADR-AI1..AI7 + INV-32..35) |
| Status | Draft — for MEKL review; downstream input to lead's TSD. Supersedes SDD v1 (kept as history, never edited). |
| Scope | Whole product as shipped today (v0.10.0 + W2-A + U1) plus committed in-flight work: wave-2 signing/updater (M2/M3, designed, not implemented) and UI redesign "Arus" (U1 landed, U2–U9 pending). ADRs are harvested, not invented. |
| Version constraint | No commits by architect; this document only |

---

## 1. Context

hosterm is a lightweight desktop SSH/SFTP client for Windows 10/11 (WebView2), developed on Linux. Two product-defining decisions constrain everything:

1. **`~/.ssh/config` is the single source of truth.** No vault, no database. hosterm is a faithful editor and spawner for OpenSSH config, not a superset of it (PRD §6.1).
2. **The OS's real `ssh`/`sftp` binaries are spawned.** No ssh2/russh libraries. Every declared directive (ProxyJump, IdentityFile, forwarding, Match) behaves exactly as on the command line.

Everything a hostile or misconfigured remote sends — SFTP filenames, file contents, terminal output — renders into a web DOM. The architecture treats that as attacker-reachable input while keeping user key material and stored passwords out of reach.

Shipped reality this document consolidates (STATE.md, 19 Okt 2026): wave-1 security hardening landed (strict CSP, SHA-pinned CI, audit gates, minimized tokens); W2-A startup `check_binaries` landed (missing ssh/sftp → blocking install guide with "Continue anyway" + re-check); Arus redesign U1 (Abyss Emerald theme) landed, U2–U9 in flight (hotspot: `src/styles.css` + `src/main.ts`, dispatched strictly serial); wave-2 signing/updater (§B) and config auto-reload (§C, FR-H3) are designed but have **no code** — `plugins.updater` absent from `tauri.conf.json` until M3. Codebase remains deliberately two big modules (`src/main.ts` ~1.65k lines, `src-tauri/src/lib.rs` ~2.5k lines) plus QA harness and CI workflow.

---

## 2. Architecture Overview

### 2.1 Component diagram

```
┌───────────────────────────── Windows process (Tauri v2) ─────────────────────────────┐
│                                                                                      │
│  WebView2 (http://tauri.localhost)                 Rust core (src-tauri/src/lib.rs)  │
│  ┌───────────────────────────────┐   IPC (invoke /  ┌─────────────────────────────┐  │
│  │ Frontend  src/main.ts         │   emit events)    │ Config model                │  │
│  │  sidebar · host form          │ ────────────────► │  parse/serialize (round-    │  │
│  │  tab shell (4 session maps)   │   24 commands     │  trip) · atomic write       │  │
│  │  xterm.js terminals           │ ◄──────────────── │ Secrets sidecar (0600)      │  │
│  │  CodeMirror editor            │   pty://output/N  │ Identity-file store (0600)  │  │
│  │  SFTP browser (dual panes)    │   pty://exit/N    │ PTY engine (portable-pty)   │  │
│  │  dialogs · manual DnD         │                   │ SFTP layer (batch + persist)│  │
│  │  pasteNormalize               │                   │ Clipboard (arboard)         │  │
│  │  [W2-A] startup bin-check UI  │                   │ [W2-A] check_binaries       │  │
│  │  [Arus U2–U9] new UI layer    │                   └──────────────┬──────────────┘  │
│  │  [M3 planned] update UI       │                                  │ spawn           │
│  └───────────────────────────────┘                                  ▼                 │
│         CSP: script-src 'self'; style-src + 'unsafe-inline'   real OS binaries        │
│              (tauri.conf.json, SEC-4 landed)               ssh ── PTY (per terminal)  │
│                                                            sftp -b -  (batch)         │
│                                                            sftp (interactive, one     │
│        ~/.ssh/config ◄── reads/writes ──┐                  persistent PTY per        │
│        ~/.ssh/hosterm.secrets ◄─ rw ────┤                  password-auth host)       │
│        ~/.ssh/<name> keys ◄─ writes ────┘                        │                    │
│                                                                  ▼                    │
│                                                    OpenSSH client (honors config)     │
│                                                           │                           │
│  [M2/M3 planned, outside app runtime]                     ▼                          │
│  CI build-windows.yml: bundler → SignPath round-trip →   remote hosts               │
│  sig regen → verify gates → .exe/.msi/.sig/latest.json → GitHub                    │
│  Releases (only feed); [M3] tauri-plugin-updater ← user-initiated check only        │
└──────────────────────────────────────────────────────────────────────────────────────┘
```

### 2.2 Delivery & packaging (folded from the signing/updater extension)

Vite bundles the frontend to `dist/` (one external JS, one external CSS — zero inline scripts/styles in HTML); Tauri embeds it and serves over the custom protocol with the CSP response header. Dev mode: Vite on `:1420` via `devUrl` with CSP delivered by Vite `server.headers` (ADR-6).

Wave-2 stages, fixed order (HR-W2-1: sign first, updater only after a fully signed release exists):

- **M2 (Stage A):** every released artifact Authenticode-signed through **SignPath Foundation** (free OSS code-signing program; OV certificate issued atas nama SignPath Foundation; private key generated on and held by SignPath's HSM, never exportable) and carries updater artifacts (`.sig` siblings via `createUpdaterArtifacts: true` + `latest.json`); fail-closed gates (`signtool verify /pa`, `tauri signer verify`, `fail_on_unmatched_files: true`) inside the existing build/release jobs. No runtime behavior change; zero network calls preserved (NFR-3).
- **M3 (Stage B):** official `tauri-plugin-updater` enabled — user-initiated check only, single GitHub Releases endpoint, minisign pubkey baked inline, verification non-disableable (platform guarantee), `installMode: "passive"`. Updater traffic is native reqwest — never enters the WebView, `connect-src` untouched. Version compare strict forward-only; refused/failed update = atomic refusal, zero filesystem side effects.

**M2 signing mechanism (SignPath, ADR-9 Rev. 2).** Signing is a service round-trip **inside the same build job** — no separate sign job, no second runner: upload unsigned artifacts → `signpath/github-action-submit-signing-request` (`SIGNPATH_API_TOKEN` + `SIGNPATH_ORGANIZATION_ID` + project-slug + signing-policy-slug; SHA-pinned in TSD) → download signed artifacts over the unsigned ones → **regenerate `*.sig` over the SIGNED binaries** (Authenticode writes the cert table into the PE, so the bundler's provisional `.sig` becomes stale) → gates (`signtool verify /pa`, `tauri signer verify`) → upload. Denied or unapproved submission = red job before any upload. `tauri.conf.json` stays vendor-neutral (no `bundle.windows.signCommand`; Authenticode is workflow-only).

**Release checklist gains one standing step:** grant the **one manual SignPath approval** in the SignPath dashboard when the submission appears. No approval → no signed artifacts → job stays red (fail-closed).

**CI secret surface (SignPath):** `SIGNPATH_API_TOKEN` (GH secret, submitter scope only, revocable in the SignPath dashboard), `SIGNPATH_ORGANIZATION_ID` (GH variable, public identifier); project-slug + signing-policy-slug are plain workflow config. Accepted debt: `SIGNPATH_API_TOKEN` is **long-lived** (no per-run OIDC token exists under this vendor), compensated by submitter-only scope + SignPath origin verification (submissions accepted only from GitHub workflow runs of this repo on GitHub-hosted agents) + the per-release manual approval — a leaked token alone cannot produce a signed release silently.

Key custody: Authenticode key lives and dies inside SignPath's HSM (non-exportable; custody = SignPath org hygiene + the release approval); minisign keypair generated offline at the M2 ceremony, stored as GH secret + encrypted offline backup; public key recorded in `docs/KEY-CEREMONY.md` (ADR-12).

---

## 3. Components

| Komponen | Tanggung jawab | Interface (contract) |
|---|---|---|
| UI shell & tab system (`src/main.ts`) | Sidebar, tabs (terminal / SFTP ⇅ / editor ✎), split terminals, dialogs/overlays, tab drag (manual mouse events), context menu; Arus U2–U4 reshapes sidebar/tab bar/terminal well + global status bar | Session maps (`sessions`, `splitSessions`, `sftpSessions`, `editorSessions`); all rendering via DOM |
| Terminal sessions (`src/main.ts`) | One xterm.js `Terminal` per tab/split; fit addon; key/copy/paste; `pasteNormalize` (CRLF→LF); a local-shell tab (`local_pty_spawn`) runs the OS default shell in a PTY with no host/password (INV-27) | `pty_spawn`, `local_pty_spawn`, `pty_write/resize/kill`; events `pty://output/{id}`, `pty://exit/{id}` |
| SFTP browser (`src/main.ts`) | Dual-pane remote + local listing, upload/download, manual-mouse drag/copy (never HTML5 DnD — dead in WebView2); Arus U5/U6: transfer strip, pane-head actions, selection, skeleton rows | `sftp_list`, `local_list`, `home_dir`, `sftp_upload/download` |
| Remote editor (`src/main.ts`) | CodeMirror 6, line numbers, Ctrl/Cmd+S write-back (Arus U7: Ctrl+S panel-level + dirty marker); 2 MB text cap, binary refusal | `sftp_read_file`, `sftp_write_file` |
| Host management UI (`src/main.ts`) | Host form (key vs password auth), key-paste modal, raw config editor, dirty-state guards; Arus U8: stored-password chip + delete-with-confirm; U8 D2: "hapus password tersimpan" → `forget_secret` (sidecar-only, host block tetap) | `read_ssh_config`, `save_host`, `delete_host`, `read/write_config_raw`, `list_identity_files`, `create_identity_file`, `forget_secret` |
| Startup binary check (W2-A, shipped) | Read-only presence check of ssh/sftp; blocking "install OpenSSH" guide screen in frontend, no spawn/side effects | `check_binaries` → `BinCheck {ssh_missing, sftp_missing}` |
| Fleet runner (`src-tauri/src/fleet.rs`, planned v1 — §12) | Run one command/script across selected hosts: thread-per-host behind an N-permit token channel, per-host deadline kill, Stop = kill-all + skipped, 64 KiB/host retention counters; zero new deps, zero writes, zero secret reads | `fleet_start`, `fleet_stop`; events `fleet://output/{run_id}`, `fleet://host/{run_id}`, `fleet://summary/{run_id}` |
| Config model (`src-tauri/src/lib.rs`) | Round-trip parse/serialize of `~/.ssh/config` (preserves comments, order, unknown directives); atomic write (sibling temp → chmod 0600 → rename); `HOSTERM_CONFIG` override | Filesystem `~/.ssh/config` |
| Secrets sidecar (`lib.rs`) | `~/.ssh/hosterm.secrets` (`alias=password` lines, 0600, deleted when empty); opt-in per host, purged on delete/uncheck; lookup result **never crosses IPC** | Filesystem sidecar |
| Secret forget (U8 D2, shipped) | Drop **one** stored sidecar row for one alias; the `~/.ssh/config` host block stays untouched; idempotent (unknown alias = no-op, empty alias = no-op); sidecar stays 0600 and is deleted when it becomes empty; the secret value never enters `~/.ssh/config` and never crosses IPC | `forget_secret(alias: String) -> ()` |
| Identity-file store (`lib.rs`) | Paste-private-key → `~/.ssh/<name>` at 0600; dropdown listing | Filesystem `~/.ssh/` |
| PTY engine (`lib.rs`: `PtyState`, `pump_pty`) | portable-pty openpty per terminal; spawns real `ssh <host>`; reader thread → `pty://output/{id}`; one-shot password injection on `'s password:` prompt; reaper thread | portable-pty, `ssh` binary, secrets |
| SFTP layer (`lib.rs`: `run_sftp_op`, `sftp_spawn`, `parse_sftp_ls`) | Dual mode, one operation API: stored secret → **persistent** interactive `sftp` in a PTY per host alias, shared across tabs (completion = next `sftp> ` prompt, 120 s watchdog, one respawn-retry); no secret → one-shot `sftp -b -` batch (real exit codes). `sftp_safe` validation (no leading `-`, no newlines) on every host/path argument | `sftp` binary, secrets |
| Editor transfer (`lib.rs`) | `sftp_read_file` / `sftp_write_file` via run_sftp_op; write verified by re-read before success | SFTP layer |
| Clipboard & local FS (`lib.rs`) | arboard clipboard; `local_list` for the local pane | OS |
| QA harness (`qa-reports/headless-smoke.mjs`) | Playwright chromium, serves `dist/` on :4199 with the prod CSP header, mock invoke map (hosts: prod-web, db-core); 75 checks incl. CSP-violation counter == 0 | `dist/` build |
| Authenticode signing hook (M2, planned) | Build-job step: upload unsigned → `signpath/github-action-submit-signing-request` → download signed (one manual SignPath approval per release); key material never on runner; `.sig` regenerated over the signed binaries | SignPath.io service (SignPath Foundation cert) |
| Signature gates + `latest.json` assembler (M2, planned) | Post-build bash gates before upload; ~30-line in-repo python3 stdlib script assembles the feed (no tauri-action) | Build outputs, GH release job |
| Updater runtime (M3, planned) | `tauri-plugin-updater`: check → forward-only version gate → consent → download → non-disableable minisign verify → passive install → relaunch | Releases endpoint, baked pubkey |
| Key ceremony record (M2, planned) | `docs/KEY-CEREMONY.md`: date/machine/operator/fingerprints/expiry/backup class | This doc |

---

## 4. Data Flow

1. **Startup gate (W2-A):** app launch → `check_binaries` → `BinCheck` (Windows only); missing flag → blocking install-guide overlay with "Continue anyway" (Escape) + re-check; config surfaces stay functional; declined connects fail per-attempt with a friendly error naming the recovery path. No spawn, no network.
2. **Open a terminal session:** sidebar click → tab in `sessions` → `invoke("pty_spawn", {host, cols, rows})` → backend opens portable-pty pair, spawns `ssh <host>`, optional `lookup_stored_password`; reader thread emits `pty://output/{id}` → xterm writes bytes; keyboard → `pty_write`; resize → `pty_resize`; tab close → `pty_kill`. No stored secret → user types at the prompt (same path as any keystroke).
3. **Browse SFTP:** ⇅ tab → `invoke("home_dir")` + `invoke("sftp_list", {host, path})` → `run_sftp_op` dispatch: stored secret for host → persistent interactive `sftp` session (spawn once, password typed at login, keyed by alias in a global map, shared by every tab), else one-shot `sftp -b -` batch. Listing text parsed by pure `parse_sftp_ls`. Upload/download → `sftp_upload/download` (same dispatch; src==dst aliasing refused).
4. **Edit a remote file:** click file in SFTP pane → `sftp_read_file` → CodeMirror tab → Ctrl/Cmd+S → `sftp_write_file` with full document text → backend writes temp + verified re-read → editor stays open.
5. **Fill a host password:** stored-secret path — `pump_pty` watches PTY output for `'s password:`, writes secret + CR **once** into the PTY writer; secret never enters JS. Unstored path — user types. SFTP variant: `sftp_spawn` types the password during login and waits for `sftp>` (wrong password → clear error).
6. **Release & update (planned M2/M3):** `v*` tag → CI build (bundler emits unsigned artifacts + provisional `.sig` → SignPath round-trip signs them → `.sig` regenerated over the signed binaries → gates verify → upload) → release job (assembles `latest.json`, `fail_on_unmatched_files: true`) → GitHub Releases = only feed. Update path: user clicks "Check for updates" → plugin GET `releases/latest/download/latest.json` (one request) → forward-only compare → consent → download → minisign verify vs baked pubkey → passive install → app exits (Windows limitation) → relaunch.
7. **Forget a stored password (U8 D2, shipped):** host dialog → user clicks "hapus password tersimpan" → in-app confirm (never native `confirm`) → `invoke("forget_secret", { alias })` → backend removes exactly that one sidecar row (0600 kept, file deleted when empty) and returns unit → frontend clears its own `has_password` chip. `~/.ssh/config` is never read or written on this path, and the secret value never crosses the IPC boundary in either direction (ADR-17, INV-20).

---

## 5. Data Model

No database. Three files under `~/.ssh/` plus environment overrides.

### 5.1 `~/.ssh/config` — source of truth

```
SshConfig
├── preamble: string[]                    # lines before first Host block, verbatim
└── hosts: HostBlock[]
    ├── patterns: string[]                # every token after "Host" (aliases, wildcards)
    └── lines: CfgLine[]                  # in file order
        ├── Raw(string)                   # comments + blank lines, verbatim
        └── Opt { key, value }            # directive, key case preserved on read
```

| Rule | Enforcement |
|---|---|
| File stays parseable by real ssh at all times | Atomic write; unreadable existing file **refuses** edit rather than overwrite |
| Round-trip fidelity: comments, order, unknown directives preserved | `parse_config`/`serialize_config` keep `Raw` lines and unknown `Opt` keys; cargo round-trip tests (52 total) |
| No secrets in the config — ever | Form writes only standard directives (key → `IdentityFile`; password → `PreferredAuthentications password` + `PubkeyAuthentication no`); grep-tested; legacy `Password` directives migrated out into the sidecar on read |
| Wildcard blocks (`Host *`) read-only | Sidebar renders them disabled; form never rewrites them |
| Atomic writes | Sibling temp `.config.hosterm-tmp-<pid>` → chmod 0600 (Unix) → rename; failed rename cleans up temp |
| Testability | `HOSTERM_CONFIG` overrides the path (smoke runs against a temp config) |

### 5.2 `~/.ssh/hosterm.secrets` — sidecar (only secret store)

`<alias>=<password>\n` (BTreeMap on disk; 0600 Unix; file deleted when empty). Opt-in per host (Q3 decision); deleting the host or unchecking purges the entry. Read only inside the Rust process — never over IPC, never logged, never rendered. Portability trade-off accepted (NFR-8): device switch = copy config + keys + optional sidecar.

### 5.3 Supporting files, env, and planned feed

| Store | Shape | Notes |
|---|---|---|
| `~/.ssh/<name>` | Pasted private keys | 0600 (Unix); key dropdown |
| `HOSTERM_CONFIG` | env | Config path override (tests) |
| `HOSTERM_SFTP_BIN` | env | sftp binary override (tests; replaces sftp lookup entirely, incl. `check_binaries`) |
| `HOSTERM_SSH_BIN` | env | ssh binary override for the **fleet path only** (tests; replaces argv[0] of fleet spawns; terminal PTY, sftp and `check_binaries` do not read it — §12.5, ADR-16) |
| `latest.json` (M2+, planned) | Static JSON feed: `version`, `notes`, `pub_date` (RFC 3339 UTC), `platforms["windows-x86_64"]{signature, url}` | `signature` = verbatim `.sig` content; `version` written verbatim from tag (no normalization); only `windows-x86_64` key emitted |

Evolution strategy: no schema migration exists (flat files, re-read whole). Wave-2 §C (FR-H3, speced) adds config auto-reload: mtime+size stat on window focus, refresh without restart, unreadable → last-good state + named-path error — additive, no model change. `latest.json` assembler gains keys only if a second platform ships.

---

## 6. Error-Handling Approach

Failure modes and the system's observable response:

- IF `ssh`/`sftp` is missing at startup (check_binaries reports a missing flag on Windows) THEN THE SYSTEM SHALL show the blocking OpenSSH install-guide overlay once per launch — Escape = "Continue anyway", a re-check closes it once both resolve — keep config surfaces fully functional, and on every declined connect/SFTP attempt show a friendly error naming the missing binaries and recovery path with raw spawn strings suppressed (W2-A, shipped).
- IF an ssh connection dies (PTY exit event) THEN THE SYSTEM SHALL mark the terminal tab session-ended and leave reconnect to an explicit user action (no auto-respawn).
- IF a batch `sftp -b -` operation fails (non-zero exit) THEN THE SYSTEM SHALL surface the real exit code + stderr text as the command's error result and render the existing error copy with actionable Go/Up controls — never a silent empty listing.
- IF a persistent sftp session dies or its 120 s prompt watchdog times out THEN THE SYSTEM SHALL respawn the session once and retry the same operation; a second failure returns the error to the caller.
- IF a remote editor save fails (write or verification re-read) THEN THE SYSTEM SHALL keep the editor open with content intact, mark the tab dirty, and show the error — the remote file is left unmodified on failed verification.
- IF a remote read is binary or exceeds the 2 MB cap THEN THE SYSTEM SHALL refuse to open the editor with a visible message before any content renders.
- IF the secrets sidecar is unreadable or a secret write fails THEN THE SYSTEM SHALL return an error on the triggering command (save/uncheck/delete), leave the config untouched, and never fall back to writing the password into `~/.ssh/config`.
- IF the existing `~/.ssh/config` is unreadable/unparseable THEN THE SYSTEM SHALL refuse the edit (error to the UI) and never overwrite the file with generated content.
- IF a downloaded updater artifact fails minisign verification or is truncated (M3, planned) THEN THE SYSTEM SHALL refuse atomically — no install, no filesystem side effects, current app keeps running — with the explicit refusal message (FR-W2-10).

---

## 7. Tech Decisions (ADR)

### ADR-1: Spawn real `ssh`/`sftp` binaries; no SSH protocol libraries
- Context: hosterm must honor every OpenSSH directive (ProxyJump, Match, forwarding) exactly as the command line does, without re-implementing protocol semantics.
- Decision: all connectivity through the OS's OpenSSH client — `CommandBuilder::new("ssh")` per terminal, `sftp -b -` batch and interactive `sftp` in a PTY for file ops. No ssh2/russh/thrussh dependency.
- Consequences: config semantics are OpenSSH's problem (zero re-implementation, CVE coverage via OS updates); costs: OpenSSH must be installed (W2-A startup check now covers detection), per-operation connection setup for batch sftp (mitigated by ADR-7 for password hosts), textual output parsing (`parse_sftp_ls`) is a watched drift risk, password auth needs PTY prompt-watching.
- Rejected alternatives: ssh2/russh in-process — would re-implement and re-expose config semantics, permanent divergence risk, bigger crypto surface; bundling an ssh binary — drags update/license surface.

### ADR-2: portable-pty + xterm.js for terminals
- Context: password prompts and TUI apps need a real TTY; the UI stack is web-based.
- Decision: Rust `portable-pty` hosts the spawned `ssh`; frontend renders xterm.js 5.5 (+fit) per tab; bridge = reader thread → `pty://output/{id}` events, input → `pty_write`.
- Consequences: full TUI fidelity free; explicit process lifecycle (spawn ids, reaper, kill on tab close); pasted text normalized CRLF→LF in frontend (`pasteNormalize`); on Windows the sftp child spawns with `CREATE_NO_WINDOW`.
- Rejected alternatives: library-driven terminal emulation without a PTY — incompatible with ADR-1; ConPTY/winpty direct bindings — portable-pty already abstracts both platforms; native terminal widget — abandons the web UI stack.

### ADR-3: Secrets in a 0600 sidecar file; no vault, no OS keychain
- Context: optional stored passwords must not break the copy-one-directory portability story (UC5) nor the founding invariant.
- Decision: `~/.ssh/hosterm.secrets` (0600 Unix, plain `alias=password`), opt-in per host, purged on host delete, deleted when empty; lookup stays inside Rust; injection straight into the PTY writer.
- Consequences: plaintext at rest behind file permissions — accepted, consistent with how `~/.ssh/` keys already live; empty-map case deletes the file so no stale entries linger; README must match reality (Q1 fixed).
- Rejected alternatives: OS keychain — breaks portability, platform glue for a solo maintainer; secrets inside `~/.ssh/config` — violates the founding invariant; full vault — out of scope (PRD §6.1).

### ADR-4: `~/.ssh/config` edited by lossless round-trip, not regeneration
- Context: users hand-tune configs; every save must preserve what the form doesn't own (US-A2).
- Decision: parser models preamble + Host blocks + typed lines (`Raw` | `Opt`); edits only the `Opt` entries the form owns; serialization replays the rest verbatim; writes atomic; unreadable file refuses edit.
- Consequences: byte-faithful survival of hand-tuned configs, guarded by cargo round-trip tests; `set_opt` manages only known directives — anything exotic goes through the raw editor, which round-trips through the same parser.
- Rejected alternatives: regenerate-the-whole-file — destroys comments/order/unknown directives on every save; ssh2-config-style parser crates — extra dependency, most don't preserve byte layout; the grammar is ~150 lines of stdlib.

### ADR-5: Tauri v2 + WebView2, single-window web UI
- Context: lightweight (~10 MB-class) binary, web ecosystem (xterm, CodeMirror), Linux dev path, single-window workflow (NFR-7).
- Decision: Tauri 2.x, one window, one TypeScript module, Vite bundling, `withGlobalTauri: false`. MIT, GitHub Releases distribution (signing stage added by ADR-9/10).
- Consequences: WebView2 preinstalled on current Windows; known platform debt handled: HTML5 drag-and-drop dead in WebView2 → all drag/copy uses manual mouse events (invariant INV-6); CSP posture per ADR-6.
- Rejected alternatives: Electron — ~100 MB runtime + bundled Chromium to patch; native WinUI/Qt — abandons web ecosystem and Linux dev; multi-window — contradicts NFR-7.

### ADR-6: Strict CSP with a single, proven relaxation
- Context: 21 `innerHTML` sites vs 13 escapes make CSP the structural backstop against XSS into the IPC bridge (wave-1 compensating control); CodeMirror's `style-mod` injects runtime `<style>` elements.
- Decision: `app.security.csp` in `tauri.conf.json`: `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self'; font-src 'self'; connect-src 'self' ipc: http://ipc.localhost; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; form-action 'none'`. Evidence in `docs/SPIKE-csp-findings.md`: Tauri does not auto-append IPC origins (they stay in the string); Tauri appends its own script nonces at runtime; `'unsafe-inline'` scoped to `style-src` solely for style-mod and works only while built HTML contains no `<style>` element; dev delivery via Vite `server.headers`.
- Consequences: injected scripts can't execute or reach IPC (`connect-src` locked to IPC origins); never relax `script-src`; keep zero inline scripts; re-run spike analysis if Vite CSS delivery changes; residual: one-time manual WebView2 pass (SEC-5 waived low-risk 19 Okt).
- Rejected alternatives: per-injection hashes for style-mod — runtime CSS/class names break silently on dep bumps; nonce plumbing via `EditorView.cspNonce` — disproportionate when script-src/connect-src carry the blast radius; `csp: null` + escaping only — insufficient with 21 innerHTML sites.

### ADR-7: Persistent interactive `sftp` per password-auth host; batch mode otherwise
- Context: password hosts must not re-prompt per transfer (UC4/D2); key/agent hosts want real exit codes.
- Decision: `run_sftp_op` dispatches by auth reality: stored secret → one interactive `sftp` in a PTY per host alias (global map, shared by every tab), password typed once at login, completion = next `sftp> ` prompt (120 s watchdog), `put` verified by re-listing, one respawn-retry on dead/timeout; no secret → one-shot `sftp -b -` batch per operation.
- Consequences: one password entry per host per app run; interactive sftp has no per-command exit codes → completion/success inferred (prompt + re-list verification); long-lived sessions die with the app process (process exit reaps PTY children; explicit teardown hook exists but unused — `master` kept for future logout-kill).
- Rejected alternatives: batch mode for everything — re-prompt per transfer, unacceptable for D2; second password prompt per tab — map is keyed by alias precisely so tabs share; library SFTP — violates ADR-1.

### ADR-8: Trust boundary — remote bytes are hostile; CSP is containment, escaping is hygiene
- Context: SFTP filenames, file contents, terminal output all render into the DOM and are attacker-reachable.
- Decision: all remote-origin bytes render through the escape helpers where reachable and are treated as attacker-controlled regardless; CSP (ADR-6) is the structural backstop keeping a missed escape from executing script or exfiltrating via `connect-src`.
- Consequences: debt is explicit (21 innerHTML sites, 13 escape calls) with a funded wave-2 DOM-hardening plan shape; new rendering code routes through `esc()`/`escAttr()` — review checklist item.
- Rejected alternatives: ship-of-Theseus innerHTML refactor in wave 1 — rejected by the EPIC's non-goals; CSP first, refactor later, in that order.

### ADR-9: Authenticode via SignPath Foundation (free OSS program); OV is sufficient (Rev. 2, planned M2)
- Context: released installers need a user-side trust anchor; signing must stay automatable on GitHub-hosted runners; Microsoft removed EV's SmartScreen shortcut (2024), so EV's premium buys nothing; the vendor must be affordable for a solo maintainer.
- Decision (Rev. 2 — supersedes Rev. 1): sign installers through the **SignPath Foundation** free OSS code-signing program. OV certificate issued **atas nama SignPath Foundation** (not MEKL personally); private key generated on and held by SignPath's HSM (non-exportable); signing executed by the SignPath.io service. Mechanism: the build job uploads unsigned artifacts and submits via the official `signpath/github-action-submit-signing-request` action (`SIGNPATH_API_TOKEN` + organization-id + project-slug + signing-policy-slug; origin-verified against this repo's GitHub workflows on GitHub-hosted agents). **One manual approval per release** in the SignPath dashboard — a policy gate, added to the release checklist. `tauri.conf.json` stays vendor-neutral; Authenticode is a workflow-only change.
- Why the change: cost **$0** (Foundation program is free for maintained OSI-licensed OSS — hosterm is MIT, public repo) vs ≈ $120/tahun for the paid cloud alternative plus identity validation; the Foundation vouches for the repo↔binary link, which is exactly the trust claim an OSS installer needs. Trade-offs accepted: (1) publisher name in binary properties = "SignPath Foundation" — cosmetic; (2) one manual approval per release — deliberate friction doubling as a release checkpoint; (3) SmartScreen reputation accrues per-cert, OV-class, gradually — parity with the paid alternative, no EV shortcut exists at all.
- Rev. 1 (superseded, kept for the record): **Azure Trusted Signing** (renamed "Azure Artifact Signing" in current docs), OIDC-federated Entra workload identity, invoked via Tauri's `bundle.windows.signCommand` hook — now the **paid fallback** only: if the Foundation program ever becomes unavailable, revert to it; the swap stays localized to workflow steps + one config key.
- Consequences: signing stays tag-push-automatable modulo one human approval; custody reduces to "protect one SignPath org + token"; the integration mechanism moves from a local CLI hook to an in-job service round-trip, which **amends ADR-10's "no artifact round-trip" clause** and adds the `.sig`-regeneration step; releases get slower by minutes and gain a human-dependent step; fail-closed is preserved (denied/unapproved submission = red job before any upload). Feature-spec amendment retained: FR-W2-5(a) "EV preferred" → "OV cloud/HSM sufficient" — now satisfied by the Foundation cert.
- Rejected alternatives: self-managed OV/EV USB token — incompatible with hosted runners, single physical point of loss; P12/PFX as GH secret — key material on every runner, the exact surface FR-W2-5 exists to avoid; SSL.com eSigner and per-signature cloud-HSM vendors — valid but paid, for properties SignPath gives free; self-signed/unsigned — no SmartScreen path, no trust anchor; keeping the paid cloud vendor despite cost — ≈$120/tahun buys no property the Foundation cert lacks for this product.

### ADR-10: Fail-closed signing inside the existing build job; `latest.json` assembled in-repo (planned M2)
- Context: an unsigned or unverified artifact must never reach a release, with minimal added CI surface.
- Decision: signing inside the existing `build` job — under SignPath the bundler emits unsigned artifacts + provisional `.sig`, the job round-trips them through `signpath/github-action-submit-signing-request` (upload unsigned → submit → download signed) and regenerates `.sig` over the **signed** binaries; post-build bash gates (`signtool verify /pa`, `tauri signer verify` per `.sig`) run before `upload-artifact`; `release` keeps `needs: [build, audit]`; `fail_on_unmatched_files: true` extended to `.sig` + `latest.json`. Feed assembled by a ~30-line in-repo python3 stdlib script.
- Rev. 2 note (SignPath vendor): the Rev. 1 clause "no artifact round-trip" is amended — the round-trip to SignPath happens **inside the build job**, so "no separate sign job / no second runner" still stands; the step sequence gains the `.sig` regeneration (the `tauri signer verify` gate is what proves the ordering: a stale `.sig` over unsigned bytes fails it). Rejected alternative (a) — a separate sign job — stays rejected.
- Consequences: fail-closed partly free (`createUpdaterArtifacts: true` makes a missing minisign secret fail the bundler before artifacts exist); small reviewable diff; assembler is ours to maintain (fixture test in-workflow); one manual SignPath approval per release becomes a human-dependent step in the pipeline.
- Rejected alternatives: separate sign job — artifact round-trip + second Windows runner for zero added enforcement; `tauri-apps/tauri-action` — large third-party action that rebuilds the app outside our audited steps and bypasses the gates; R2 posture is minimal SHA-pinned action surface; driving the round-trip through `bundle.windows.signCommand` + a wrapper CLI — would serialize a service wait (incl. the manual approval) inside the bundler and require custom wrapper code.

### ADR-11: Updater = official `tauri-plugin-updater`, single GitHub Releases feed, user-initiated only (planned M3)
- Context: stale builds are a CVE risk, but background polling/silent installs violate the product's zero-background posture (NFR-3, FR-W2-8, D3); signature verification must be structurally non-disableable.
- Decision: official plugin (Rust-side HTTP + verify + install) configured with exactly one endpoint (`releases/latest/download/latest.json`), baked inline pubkey, `installMode: "passive"`, strict forward-only compare, user-initiated check only; frontend adds one action + result/refusal rendering routed through `esc()`/`escAttr()`.
- Consequences: zero CSP impact (traffic never enters the WebView); zero background work; trust chain = baked pubkey → plugin-enforced verify; accepted limits: app exits on install (Windows limitation — safe, config writes are atomic and nothing is cached at consent time), no delta updates (~10 MB-class installers), `releases/latest` serves non-prerelease only.
- Rejected alternatives: custom updater — re-implements verification the plugin got right; hand-rolled updater verify bugs are a classic RCE class; third-party update server — violates NFR-W2-6, hosted dependency for a weeks-cadence product; background polling/silent install — product constraint, non-negotiable.

### ADR-12: Key custody — two keys, two lifetimes, offline-backup rotation (planned M2/M3)
- Context: signing needs exactly two private keys (Authenticode, minisign) with different custody realities; leaks must be recoverable via rotation.
- Decision: Authenticode lives and dies inside SignPath's HSM (non-exportable; custody = SignPath org hygiene + the per-release manual approval — ADR-9 Rev. 2); minisign generated offline at the M2 ceremony (`tauri signer generate` on the maintainer machine), password-encrypted, stored as GH secret + encrypted offline backup; public key recorded in `docs/KEY-CEREMONY.md` at M2, baked into `tauri.conf.json` only at the M3 flip; rotation per runbook (new pair offline → release signed with old key carrying new baked pubkey → one adoption cycle → retire). Authenticode certificate lifecycle (issuance/renewal/reissue) is owned by SignPath Foundation — there is no profile of ours to rotate; a Foundation-side reissue just changes the chain in future signed releases.
- Consequences: secret surface in GitHub = three secrets (`SIGNPATH_API_TOKEN`, `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`) + one public variable (`SIGNPATH_ORGANIZATION_ID`), none of them Authenticode key material; rotation rehearsed once (tabletop); pubkey change always signed by the previous trusted key — never both in one flip.
- Rejected alternatives: generating the minisign key in CI — private key transits runner memory/disk caches for a one-time 5-minute offline step; hardware token for minisign — reintroduces hosted-runner incompatibility for a key already bounded by rotation; storing the pubkey nowhere until M3 — loses the ceremony record the M3 flip is verified against.

### ADR-13: Fleet transport = real `ssh`, two argv shapes, no PTY, zero new deps (Fleet Run v1, §12)
- Context: boss intent is "run a command on many hosts" (ansible-lite) under the founding principle (ADR-1) and FR-FL12/NFR-FL1 (no local shell, no new dependency). `run_sftp_batch` (lib.rs:717) already proves the exact spawn shape fleet needs.
- Decision: every fleet child is `std::process::Command::new(HOSTERM_SSH_BIN ∨ "ssh")` with fixed argv: `-o BatchMode=yes <alias>` plus exactly one of two transports — command mode: the single-line text as one further argv element (remote login shell interprets it there, never locally); script mode: `bash -s` argv with the normalized script bytes piped to stdin. Stdin = `Stdio::null()` for command mode, piped for script mode. stdout/stderr piped, real exit code captured. Windows parity: `CREATE_NO_WINDOW (0x0800_0000)` creation flag, identical to the sftp path.
- Consequences: FR-FL12/NFR-FL4 hold structurally (no shell exists in the path at all); quoting/escaping is a non-issue because the text is one argv element or raw stdin bytes; BatchMode makes auth/host-key prompts fail fast and cleanly (exit 255, `fail` not `timeout`); kill-on-timeout ends the local ssh only — the remote keeps running, exactly like Ctrl-C on the CLI (documented, spec edge case); per-chunk `String::from_utf8_lossy` may show U+FFFD when a multi-byte sequence straddles an 8 KiB read boundary — accepted, the fleet pane is a plain text view with no terminal emulation.
- Rejected alternatives: `sh -c "ssh …"` wrappers — the exact surface FR-FL12 removes; ssh library (russh/ssh2) — ADR-1; PTY per fleet host — stdin would be a terminal, breaking script-via-stdin (`bash -s`) and adding portable-pty cost for zero v1 need (password hosts are `skipped`, §12.8); scp-then-execute — non-goal (staged upload excluded in spec §1).

### ADR-14: Runner = thread-per-host + N-token std channel as semaphore + deadline poll (Fleet Run v1, §12)
- Context: ≤ N (1–20, default 5) live children, per-host timeout (5–600 s, default 60) covering connect+exec (FR-FL23), Stop kills everything (FR-FL30), first spawn ≤ 200 ms after confirmation (NFR-FL5), UI never blocks (FR-FL24) — without adding a dependency.
- Decision: `fleet_start` validates + plans synchronously and returns immediately after launching a coordinator thread. The coordinator walks the eligible queue, acquires a permit from a bounded `mpsc` channel seeded with N tokens (acquire = `recv_timeout(50 ms)` loop so Stop is honored while queued), releases on send-back, and spawns one host-task thread per started host. Host task owns its `Child` exclusively and polls every 50 ms: `try_wait` (natural exit wins) → stop flag (kill, `skipped`/"stopped by user") → deadline (kill, `timeout`). Helpers per started host: two reader threads (stdout, stderr → chunk events) plus, in script mode only, one stdin-writer thread (a backpressured 2 MiB write can never stall the deadline poll). ≤ ~4 threads and ~3×8 KiB buffers per live host; ≤ 20 live hosts worst case.
- Consequences: std-only, matches the proven pump_pty reader-thread posture; stop/timeout latency is bounded by the 50 ms poll; exit-vs-kill races have one deterministic resolver (the poll order above), so a fast auth failure is never swallowed as a timeout; app exit reaps children (same posture as ADR-7).
- Rejected alternatives: tokio/async-std task pool — fleet code would sit on tauri's internal runtime and `wait_with_timeout` on `std::process` still needs threads anyway; a `Semaphore` dep — the token channel is ~15 std lines and already Stop-aware; dedicated watchdog thread per host — same thread count with kill/deadline/exit logic split across two places instead of one poll loop; `child.wait()` + separate timer thread — reintroduces that split.

### ADR-15: IPC surface = 2 commands + 3 run-scoped event families, plan-in-response, error-code prefixes (Fleet Run v1, §12.3)
- Context: Unit 3–4 (frontend) must be buildable and smoke-mockable before the backend exists (OQ-9 closed: contract frozen early); Tauri events are fire-and-forget with no replay, so anything emitted before the frontend attaches a listener is lost.
- Decision: exactly two commands, inventory 21 → 23: `fleet_start(req) -> FleetStartResp` and `fleet_stop(run_id) -> ()`; three event families keyed by run: `fleet://output/{run_id}` (chunk: alias, seq, stream, bytes, total_bytes, truncated), `fleet://host/{run_id}` (terminal: status ok|fail|timeout|skipped, exit_code, reason, stdout_bytes, stderr_bytes, truncated), `fleet://summary/{run_id}` (counts, elapsed_ms, min/max non-zero exit). `run_id` is a **client-generated opaque string** supplied in the request: the frontend subscribes to the exact event names *before* invoking `fleet_start`, which closes the fire-and-forget race structurally; the backend echoes it and never invents one. The response additionally carries the authoritative per-host plan (queued|skipped+reason) and, on the all-skipped fast path, the complete summary with no events at all. Rejection errors are stable machine-readable strings (`fleet:already-running`, `fleet:text-too-large`, …) that frontend maps to copy. Payload fields are snake_case, matching every existing DTO as consumed by `src/main.ts`.
- Consequences: the full contract lives in §12.3 with JSON fixtures the smoke harness can replay via `__emitTauri` without a backend; initial skipped states are response-derived while every post-start transition is event-carried, and pre-subscription (client-run-id) means no transition can be emitted into a window with no listener; contract change = addendum revision, reviewable; 23 commands keeps the §3/§11 inventory self-check meaningful (current total 24 after ADR-17 — this ADR's own count is 21 → 23 and stays historical).
- Rejected alternatives: `tauri::ipc::Channel` streams — breaks the `pty://`-style event convention the smoke mock plugin already replays; per-host event names `fleet://output/{run_id}/{alias}` — puts still-hostile alias bytes into listener strings for no gain; a `fleet_status` poll command — latency cost and a second state source; backend-assigned run_id — frontend cannot subscribe before invoke, so fast finishes on CI (fake ssh in milliseconds) could emit into a listener-less window.

### ADR-16: `HOSTERM_SSH_BIN` — fleet-only ssh binary override (Fleet Run v1, §12.5)
- Context: feature-spec requires the fleet runner to be testable against a fake ssh on Linux CI with no server; precedent is `HOSTERM_SFTP_BIN` (lib.rs:721), which replaces the sftp lookup entirely.
- Decision: env var `HOSTERM_SSH_BIN`, read per fleet spawn: if set, its value is argv[0] verbatim (literal path, no PATH magic, no shell); if unset, `ssh`. Scope is the fleet spawn path only — `pty_spawn` (portable-pty `CommandBuilder::new("ssh")`), the sftp layer, and `check_binaries`/`binaries_missing` never read it.
- Consequences: Unit 1 cargo tests drive the real runner (concurrency ceiling, timeout kill, exit classification, byte-ordering) against shell-script fakes; `check_binaries` stays honest on test machines; override on a production machine is an accepted, documented foot-gun identical in risk profile to the sftp hook.
- Rejected alternatives: global ssh override across all paths — drags portable-pty spawn changes and terminal smoke churn into a fleet testability need; `#[cfg(test)]`-only injection — tests must exercise the shipping spawn code path, not a parallel one; a config-file directive — fleet must touch no persisted config (NFR-FL2).

### ADR-17: `forget_secret` = one sidecar row, host block untouched, idempotent, no IPC payload (U8 D2, inventory 23 → 24)
- Context: U8 D2 asks the host dialog for a "Hapus password tersimpan" action (EPIC-TSD §U8). The stored secret lives only in the 0600 sidecar; the `~/.ssh/config` host block is the SoT and holds no secret (INV-3). The only existing way to drop a stored password was deleting the whole host, which is a different user intent (and destroys the host block). The save path never re-sends the secret (empty password = keep stored), so "forget" needs its own command.
- Decision: one new Tauri command, `forget_secret(alias: String) -> ()` — unit return, no DTO, no error string, no event. It removes exactly the sidecar row keyed by that alias (the alias is also matched against its trimmed spelling, so a padded name still resolves to its own row — no other alias' row is ever touched); the `~/.ssh/config` host block is not read, parsed, or written. Unknown alias = no-op (no file rewrite — the sidecar is only rewritten when a row was actually removed); empty/whitespace alias = no-op. Sidecar keeps 0600 and is deleted when it becomes empty (INV-8 unchanged). The secret value never crosses the IPC boundary in either direction (INV-4 unchanged). Frontend contract: `invoke("forget_secret", { alias })` — the `alias` argument name is camelCase at the Tauri boundary, matching every existing command.
- Consequences: the IPC surface grows 23 → 24 (`grep -rc "tauri::command" src-tauri/src/*.rs` → 24: lib.rs 22 + fleet.rs 2); §3 and the §11 self-check carry the new number. The command is sidecar-only, so the Fleet Run frozen contract (§12.3) is untouched — `forget_secret` is not a fleet payload and the §12.3 table stays frozen. The frontend owns the confirm dialog (in-app, never native `confirm`) and refreshes its own `has_password` flag; the backend returns nothing, so the frontend must not expect a re-read of the sidecar.
- Rejected alternatives: `forget_secret(alias) -> bool` (changed?) — the frontend already knows the row existed (chip state) and a false return would be indistinguishable from a race; a `delete_host`-style cascade — destroys the host block, wrong intent; writing a "no password" marker into `~/.ssh/config` — violates INV-3/NFR-1 (no secret material, and no marker directives in a real ssh config); re-reading the sidecar and returning it to the frontend — violates INV-4 (secret crossing IPC); batching several aliases in one call — no caller needs it, YAGNI.

---

## 8. Invariants (properties — mengalir ke STATE.md + check qa)

| ID | Properti | Verifikasi |
|---|---|---|
| INV-1 | Untuk setiap operasi tulis config yang valid, THE SYSTEM SHALL menuntut `~/.ssh/config` tetap parseable oleh ssh asli — tulisan atomic (temp → 0600 → rename) dan file unreadable tidak pernah ditimpa | `cargo test --lib` (atomic write + unreadable-file refusal tests); real-ssh parse spot-check |
| INV-2 | Untuk setiap config valid, THE SYSTEM SHALL mempertahankan komentar, urutan, dan directive tak dikenal byte-setia pada round-trip edit | Property-style round-trip tests di `cargo test --lib` (52 pass baseline) |
| INV-3 | Untuk setiap operasi simpan host yang valid, THE SYSTEM SHALL menolak menulis secret ke `~/.ssh/config` — secret hanya di sidecar 0600 | Grep test di verify loop + legacy `Password` migration tests (cargo) |
| INV-4 | Untuk setiap state aplikasi valid, THE SYSTEM SHALL menjaga stored password tidak pernah melintasi IPC, log, atau render — lookup hanya di dalam proses Rust, injeksi langsung ke PTY writer | Code audit (review checklist) + smoke mock invoke map tanpa payload secret |
| INV-5 | Untuk setiap paste multi-baris ke PTY yang valid, THE SYSTEM SHALL menormalisasi CRLF→LF sebelum byte mencapai remote shell (`pasteNormalize`) | Smoke check `pasteNormalize` + cargo pump tests |
| INV-6 | Untuk setiap interaksi drag yang valid, THE SYSTEM SHALL memakai manual mouse events dan tidak pernah memakai HTML5 DnD (dead di WebView2) | Smoke: `grep draggable` di `src/` = 0 (U9 audit check, proposed canon) |
| INV-7 | Untuk setiap render halaman valid di bawah CSP produksi, THE SYSTEM SHALL melarang eksekusi script inline/eksternal non-bundled dan membatasi `connect-src` ke origin IPC saja | Smoke CSP-violation counter == 0 (existing, 75 checks) |
| INV-8 | Untuk setiap state sidecar valid, THE SYSTEM SHALL menulis file 0600 (Unix), menghapus file saat kosong, dan purge entri saat host dihapus/uncheck | `cargo test --lib` secrets handling tests |
| INV-9 | Untuk setiap Host wildcard (`Host *`) yang valid, THE SYSTEM SHALL menampilkannya read-only dan tidak pernah menulis-ulang blok itu | `cargo test --lib` + smoke (wildcard tidak dirender sebagai row) |
| INV-10 | Untuk setiap host/path argument yang valid menuju binary sftp, THE SYSTEM SHALL menolak argumen berawalan `-` atau mengandung newline (`sftp_safe`) | `cargo test --lib` sftp_safe tests |
| INV-11 | Untuk setiap konten remote yang valid dirender ke DOM, THE SYSTEM SHALL melewatkan escape helper (`esc()`/`escAttr()`) pada titik render baru — remote bytes adalah input hostile | Proposed: smoke/audit grep situs `innerHTML` baru tanpa esc (wave-2 DOM hardening) |
| INV-12 | Untuk setiap rilis bertanda (M2+, planned), THE SYSTEM SHALL menolak publish artifact tanpa signature valid — gate verify sebelum upload, `fail_on_unmatched_files: true` | AC-B3.1/.2: dry-run tanpa secret gagal pre-publish + workflow audit (extension §E.8) |
| INV-13 | Untuk setiap update yang valid diunduh (M3+, planned), THE SYSTEM SHALL memverifikasi minisign terhadap pubkey baked dan menolak downlevel/tampered artifact tanpa efek samping filesystem | AC-B5.x/B6.x tamper fixtures (extension §E.8.5) |
| INV-14 | Untuk setiap input command/script fleet yang valid, THE SYSTEM SHALL tidak pernah meneruskannya ke shell lokal — byte mencapai proses hanya sebagai elemen argv `ssh` atau byte stdin child; tidak ada `sh -c`/`bash -c`/`cmd /C` di jalur fleet | grep gate verify loop (NFR-FL4) + cargo fixture marker-file (Unit 1) |
| INV-15 | Untuk setiap pesan IPC fleet yang valid (request, response, event), THE SYSTEM SHALL menjaga 0 secret byte — tipe payload tidak punya field secret; jalur fleet tidak pernah memanggil `lookup_stored_password`/`read_secrets` | code audit + smoke DOM assert sidecar password tak pernah muncul (NFR-FL7) |
| INV-16 | Untuk setiap run fleet yang valid, THE SYSTEM SHALL tidak pernah menulis `~/.ssh/config`, sidecar, atau file persisten baru — fleet memakai read-only config snapshot tanpa self-heal write | cargo test hash+mtime config/sidecar identik sebelum/sesudah run (NFR-FL2/FL8); FS diff |
| INV-17 | Untuk setiap byte output remote fleet yang valid dirender ke DOM, THE SYSTEM SHALL melewatkan `esc()`/`escAttr()` — output fleet adalah input hostile (kelas INV-11); pane fleet bukan terminal emulator | smoke fixture `<script>` di stdout chunk → 0 CSP violation, 0 script element di DOM (Unit 4) |
| INV-18 | Untuk setiap spawn fleet yang valid, argv SHALL memuat `-o BatchMode=yes` dan tidak pernah memuat flag auto-accept host key (`StrictHostKeyChecking=no`, `-o UserKnownHostsFile=/dev/null`), forwarding tambahan (`-A`/`-X`), PTY (`-t`), atau eskalasi (`sudo`, `-A` ssh-agent forwarding ke perintah) | cargo test pure argv-builder + grep gate (FR-FL18/FL23/FL35, OQ-2/OQ-3 closed) |
| INV-19 | Untuk setiap alias terpilih yang valid menuju spawn fleet, THE SYSTEM SHALL menerapkan guard yang sama dengan `sftp_safe` (leading `-`, CR atau LF → `skipped` reason `unsafe alias`, 0 spawn) | cargo test guard alias; reuse fungsi yang sama dengan INV-10 (FR-FL20) |
| INV-20 | Untuk setiap pemanggilan `forget_secret` yang valid, THE SYSTEM SHALL menghapus hanya entri sidecar untuk alias itu, tidak pernah mengubah blok host di `~/.ssh/config`, tidak pernah menulis secret ke config, menjaga sidecar 0600, dan bersifat idempoten (alias tak dikenal / kosong = no-op tanpa penulisan file) | `cargo test --lib` `forget_secret_tests::` (4 test: hapus hanya row bernama, idempotency, sidecar 0600 + config untouched, flag drop) |

---

## 9. Risks

| # | Risiko | Impact | Mitigasi / status |
|---|---|---|---|
| R-1 | `innerHTML` + escaping gaps → XSS into IPC bridge | High (key material, sidecar) | ADR-6 CSP (0 violations smoke + manual matrix; SEC-5 waiver logged); wave-2 DOM hardening parked behind feature spec |
| R-2 | Textual parsing of `sftp` output drifts (locale/format) | Medium | `parse_sftp_ls` pure + unit-tested; batch `ls -la` stable upstream; watch on OpenSSH upgrades |
| R-3 | OpenSSH client missing on a Windows machine | Medium | **Closed by W2-A**: `check_binaries` + blocking install guide (shipped) |
| R-4 | Vite CSS delivery plants `<style>` in HTML → style nonce → `'unsafe-inline'` neutralized → editor unstyled | Medium | Prohibited-by-review (spike §5); smoke CSP-violation counter fails the build; re-run spike if it changes |
| R-5 | WebView2 runtime drift vs Chromium behavior | Low–medium | Chromium-engine harness + smoke cover semantics; manual Windows matrix per wave; self-hosted runner proposed |
| R-6 | Config corrupted by crashed/interrupted write | High (user data) | Atomic temp+rename, unreadable-file refusal, round-trip suite |
| R-7 | Sidecar plaintext-at-rest copied | Medium | 0600, opt-in per host, purged on delete; consistent with `~/.ssh` threat model (ADR-3) |
| R-8 | Persistent sftp sessions leak (zombies) across tab churn | Low | Liveness watcher + one respawn-retry; process exit reaps; explicit teardown future work |
| R-9 | Supply chain: mutable CI actions, CVE-laden deps | High | Wave 1: SHA-pinned actions + minimized token scope, dual audit gates; M2 adds signing trust anchor (ADR-9/10) |
| R-10 | Performance targets (≤200 ms feedback, ≤100 ms/1k hosts) unmeasured | Low | Q5: bind on first complaint; no obvious hotspot (linear parse, incremental listings) |
| R-11 | SignPath org / API-token compromise → signed malware (T1) | High | Non-exportable HSM key, submitter-scoped token, origin verification (this repo, GitHub-hosted agents only), manual approval per release, fork-PR isolation, publicly diffable releases; recovery = token revocation + pending-request rejection + cert-chain reissue (owned by SignPath Foundation) |
| R-12 | minisign secret leak → malicious updates until rotation (T2) | High | Encrypted secret + offline backup, log masking; baked pubkey makes leaks visible (refusals); tabletop rotation drill |
| R-13 | `latest.json` assembler bug points users at wrong asset | Medium | Plugin refuses on failed fetch/verify (no install, no side effects); fixture test; `fail_on_unmatched_files: true`; `windows-x86_64` key only |
| R-14 | Arus units collide on `styles.css`/`main.ts` hotspot | Medium (churn/rework) | Epic §0 mandates serial dispatch, one card at a time on those files, per-unit commits for per-unit revert |
| R-15 | `SIGNPATH_API_TOKEN` is long-lived (no per-run OIDC token under this vendor) — leak = reusable signing authority until revoked | High | Accepted debt (ADR-9 Rev. 2): submitter-only scope (cannot export key material — HSM-held), origin verification (workflow-of-this-repo, GitHub-hosted agents, artifact uploaded pre-submission), one manual approval per release — a leaked token alone cannot produce a signed release silently; recovery = revoke in SignPath dashboard + reject pending requests |

---

## 10. Testing Strategy

Pyramid mapped to the real, measured baseline (STATE.md, 19 Okt 2026, post-W2-A/post-U1):

1. **Rust unit tests:** `cd src-tauri && cargo test --lib` → **52 pass** (config round-trip incl. unknown directives, atomic write + unreadable-file case, secrets handling/purge, `parse_sftp_ls`, `sftp_safe`, password-injection pump, `binaries_missing` pure core). Guards INV-1/2/3/8/9/10.
2. **TypeScript build:** `npm run build` → ~500 ms, **0 TS errors**.
3. **Headless smoke:** `npm install --no-save playwright@1.63.0 && node qa-reports/headless-smoke.mjs` → **75 checks** (Playwright chromium, serves `dist/` on :4199 with prod CSP header, mock invoke map hosts prod-web/db-core). Guards INV-5/6/7 + UC1/UC2/UC3 equivalents + Arus U1 token checks; extended ≥ N new checks per Arus unit (U9 canonicalizes). Cleanup after: `rm -rf node_modules/playwright node_modules/playwright-core node_modules/.bin/playwright*`.
4. **Invariant greps (pre-commit):** staging grep `target/|node_modules|.exe$|dist/` clean; secrets-in-config grep clean (NFR-1); wave-2 adds private-key-marker grep (AC-B1.2).
5. **Manual Windows matrix (per release wave):** UC1–UC7 on the real build; zero CSP violations; one-time WebView2 CSP injection proof (spike §4); Arus U4 divider + U5 manual drag before next release (mouse interactions Playwright can't fully cover).
6. **Planned (M2/M3, extension §E.8):** fail-closed dry-run without secrets; workflow audit; updater-absence assert pre-M3; tamper fixtures (byte-flip, wrong key, truncated, downlevel); network-surface audit (zero outbound at idle; exactly one HTTPS request per manual update check post-M3).

**Explicitly NOT tested:**
- Performance targets (NFR-6) — unmeasured by decision Q5; bind on first complaint.
- Real network/SSH to live hosts — smoke uses the mock invoke map; real connectivity is manual-matrix territory.
- WebView2 runtime on Windows in CI — SEC-5 waived low-risk; Chromium-based smoke is the proxy; self-hosted Windows runner is the future fix.
- Sidecar create-then-chmod micro-window — logged INFO/ditunda (lead, t_6608fa42); fix (`OpenOptions::mode(0o600)`) queued for a later wave.
- Updater runtime behavior — no code exists until M3; everything in §E.8 is proposed, not running.
- Linux/macOS installers, multi-instance runs, editor beyond 2 MB, drag physics beyond Playwright's event-dispatch fidelity.

---

## 11. Verification Steps

1. `cd src-tauri && cargo test --lib` → 77 pass (73 FL-1 + 4 `forget_secret_tests::`; re-measure at dispatch — Arus units shift smoke only; backend untouched by Arus units).
2. `npm run build` → 0 TS errors.
3. `npm install --no-save playwright@1.63.0 && node qa-reports/headless-smoke.mjs` → 196 pass (186 U7 + 10 U8 dialog checks), CSP-violation counter == 0; then remove playwright per repo hygiene.
4. SDD self-checks (executed at authoring): `grep -c 'INV-' docs/SDD.md` → ≥ 8; `grep -c 'IF .* THEN THE SYSTEM SHALL' docs/SDD.md` → ≥ 6; section headers byte-identical to `templates/sdd.md`; every ADR carries Context/Decision/Consequences/Rejected alternatives; testing commands above are the live verify loop.
5. Code-claim traceability: command list §3 matches `#[tauri::command]` inventory (**24 today** = 21 base + `fleet_start`/`fleet_stop` (Fleet Run v1, §12.3) + `forget_secret` (U8 D2, ADR-17)); canonical count gate = `grep -rc "tauri::command" src-tauri/src/*.rs` → 24 (lib.rs 22 + fleet.rs 2); CSP string §7 ADR-6 matches `tauri.conf.json`; `plugins.updater` absent from `tauri.conf.json` until M3 (AC-B3.3).
6. Invariant flow: INV-1..20 carry named existing-or-proposed checks; wave-2 rows (INV-12/13) activate with M2/M3 code per extension §E.8; fleet rows (INV-14..19) activate with Fleet Run v1 code per §12; INV-20 (`forget_secret`, U8 D2) is active now — the command and its 4 cargo tests shipped.

---

## 12. Fleet Run v1 (addendum Unit 0 — runner architecture + frozen IPC contract)

> Architect addendum (card t_a367888f, 19 Okt 2026), derived FROM `docs/FEATURE-SPEC-fleet-run-v1.md`. Design-first lane: the spec's OQ decisions 1–9 are CLOSED constraints, not inputs to re-debate here; this section supplies exactly what Unit 0 owed them — the password-host PTY-vs-pipe verdict (§12.8), command/event names, env-override name, kill/timeout form. Units 1–2 (backend) and 3–4 (frontend) code against §12.3 verbatim; **the frontend mocks this contract without any backend**. A payload or name change requires an edit to this section (new ADR), never an implementation-side drift.

### 12.1 Constraints honored (trace)

| Constraint | Mechanism (this section) |
|---|---|
| FR-FL12 / NFR-FL4 — no local shell ever | ADR-13: text reaches a process only as one `ssh` argv element or stdin bytes; INV-14 |
| FR-FL18 / FL23 — `BatchMode=yes`, N parallel, timeout covers connect+exec | ADR-13 argv + ADR-14 deadline measured from spawn; INV-18 |
| FR-FL20 / FL21 — skip unsafe alias / password host, 0 spawn for them | pure `host_eligibility` (§12.4) + `sftp_safe` reuse; INV-19 |
| FR-FL26 / INV-11 — hostile output bytes | INV-17: `esc()`/`escAttr()` at every fleet render; no terminal emulation |
| FR-FL34 / NFR-FL2 / NFR-FL8 — fleet writes nothing | read-only config snapshot (§12.2), no self-heal write on this path; INV-16 |
| FR-FL35 — no sudo/escalation/forwarding flags inserted | fixed argv builder only (§12.4 #6); INV-18 |
| FR-FL36 / NFR-FL7 — 0 secret bytes over IPC | request/event types have no secret field; fleet never calls `lookup_stored_password`/`read_secrets`; INV-15 |
| NFR-FL1 — 0 new dependencies | std threads + mpsc token channel + `std::process` (ADR-13/14) |
| NFR-FL3 — output ≤ 2 MiB + 64 KiB × hosts | backend streams pass-through (8 KiB read buffer), counts only; retention ring lives in frontend (§12.6) |
| NFR-FL5 — first spawn ≤ 200 ms after confirm | `fleet_start` = validate + snapshot + spawn coordinator, returns immediately; coordinator's first action is token-acquire + spawn of host 1 |
| NFR-FL9 — never touch `authorized_keys`, no elevation | spawn is `ssh` with config-derived identity only; nothing writes under `~/.ssh` beyond the ssh binary's own known_hosts behavior (unchanged from CLI) |

### 12.2 Runner architecture

Module placement (keeps the `lib.rs` diff ~3 lines — `mod fleet;` + `.manage(FleetState::default())` + 2 `generate_handler!` entries; also shrinks the Unit 1–2 hotspot collision surface):

```
src-tauri/src/fleet.rs   ← ALL fleet code: pure fns (§12.4), DTO structs (Serialize/Deserialize),
                           FleetState, coordinator + host-task threads, #[tauri::command] wrappers
                           fleet_start / fleet_stop, and #[cfg(test)] mod tests (fake-ssh fixtures)
src-tauri/src/lib.rs     ← sftp_safe stays there (pub(crate)); fleet reuses it. No other fleet logic.
src/main.ts              ← panel + dialog + per-host output ring buffer (Units 3–4, after Arus U9)
```

Process topology per run (ADR-13/14):

```
frontend invoke fleet_start(req{run_id,…})                 ← run_id = client-generated string (UUIDv4)
  │ validate (pure): ranges, text cap, aliases non-empty   → on failure: Err("fleet:<code>"), 0 spawn
  │ ssh presence probe (binaries_missing core, no spawn)   → missing: Err("fleet:ssh-missing")
  │ config_snapshot(): read+parse ONLY (no migrate/self-heal write — INV-16)
  │ plan: host_eligibility per alias → queued / skipped+reason
  │ spawn coordinator thread; return FleetStartResp{run_id, hosts, finished, summary} immediately
  ▼
coordinator thread ── token channel (N permits) ──┬─ host-task thread × ≤N
  │  for each queued alias: acquire (recv_timeout, stop-aware) → spawn host task
  │  on stop observed with unstarted hosts left: emit fleet://host(skipped,"stopped by user") for each
  │  after all host tasks done: aggregate counts + emit fleet://summary → Drop guard clears FleetState.active
  ▼
host-task thread (owns its Child exclusively)
  │  build argv (pure §12.4 #6) → std Command spawn (CREATE_NO_WINDOW on Windows, sftp-path parity)
  │  script mode: stdin-writer helper thread (write_all normalized bytes → drop = EOF)
  │  reader helper threads ×2: stdout, stderr → 8 KiB reads → String::from_utf8_lossy → emit chunk event
  │  poll loop, 50 ms: child.try_wait()? → finished(exit)  else stop flag? → kill+finished(stop)
  │                    else deadline? → kill+finished(timeout)   (precedence: natural exit > stop > timeout)
  │  join readers (kill/exit closes pipes) → classify (§12.4 #4) → emit fleet://host → return token
```

Per-host state machine (frozen; FR-FL27 exactly one terminal state per host):

```
plan:  queued ──────────────► running ──► ok | fail | timeout | skipped("stopped by user")
       skipped(reason) [terminal at start — carried in the response, NEVER re-emitted as event]
queued hosts killed by Stop before spawn: coordinator emits skipped("stopped by user") — that is their only event.
```

Panic-safety: the coordinator clears `FleetState.active` through a guard struct's `Drop`, so a panicking run can never wedge the single-run lock (FR-FL31).

### 12.3 Frozen IPC contract (Unit 2 implements verbatim; Unit 3–4 mock verbatim)

Conventions: command args are camelCase at the Tauri boundary (v2 auto-maps to snake_case Rust fields, same as every existing command); **event payload fields are snake_case JSON**, matching how `src/main.ts` already consumes `HostDto` (`h.host_name`, `h.auth_method`). `run_id` is an opaque client-generated string, max 64 chars, treated only as an event-name suffix + echo — backend never parses it.

Commands (inventory 21 → 23 saat Fleet Run v1; **total IPC inventory sekarang 24** — `forget_secret` ADR-17; **tabel payload di bawah BEKU, tidak disentuh oleh 24**):

> Addendum (ADR-17, U8 D2): `forget_secret(alias: String) -> ()` menambah command ke-24. Addendum ini **bukan** perubahan §12.3 — `forget_secret` bukan payload fleet, tidak menambah event family, dan tidak mengubah struct/field mana pun di bawah. Tabel §12.3 tetap beku untuk Unit 2–4 (implement verbatim / mock verbatim). Yang berubah hanya angka inventory (23 → 24) yang dirujuk di §3 dan §11.5.

```ts
invoke("fleet_start", { req: FleetStartReq }): Promise<FleetStartResp>
interface FleetStartReq {
  run_id: string;            // UUIDv4 generated by frontend BEFORE the call (pre-subscribed events, ADR-15)
  mode: "command" | "script";
  text: string;              // command (1 line) or script body as UTF-8 text; ≤ 2 MiB (UTF-8 bytes)
  aliases: string[];         // picker selection, dedup'd by picker; backend replans authoritatively
  timeout_secs: number;      // 5..600; frontend default 60 — backend REJECTS out of range, never clamps
  concurrency: number;       // 1..20; frontend default 5 — same reject rule
}
interface FleetStartResp {
  run_id: string;            // echo
  hosts: FleetHostPlan[];    // picker order; authoritative initial state
  finished: boolean;         // true ⇒ zero hosts eligible to run: no event will EVER come for this run
  summary: FleetSummary | null; // non-null only when finished == true (computed from the plan)
}
interface FleetHostPlan {
  alias: string;
  eligible: boolean;
  status: "queued" | "skipped"; // skipped = TERMINAL, carried here, never as an event
  reason: string | null;        // frozen reason codes below; null when eligible
}

invoke("fleet_stop", { runId: string }): Promise<null>
// Idempotent: unknown/finished run_id → Ok no-op (panel-close Stop can always fire, FR-FL37).
```

Frozen reason codes (exact strings; `skipped` carries only the first five; `fail` carries only `spawn failed` / `killed by signal`; `ok` and `timeout` always carry null):

| reason | rule |
|---|---|
| `unsafe alias` | `sftp_safe`-guard fails: leading `-` or contains CR/LF (FR-FL20, INV-19) |
| `unknown alias` | alias absent from the config snapshot (picker race / stale selection; trust boundary) |
| `wildcard pattern` | alias contains `*`/`?` — never spawnable (spec §: pickers exclude, backend re-guards) |
| `password host (not supported in v1)` | `auth_method == "password"` OR sidecar entry exists (FR-FL21; §12.8) |
| `stopped by user` | queued host never started after Stop, or running host killed by Stop (FR-FL30) |
| `spawn failed` | `Command::spawn` error → status `fail`; message never contains raw spawn strings (FR-E3 posture) |

Eligibility precedence is fixed in that row order (unsafe alias first — spawn-safety dominates product-skip reasons).

Events (backend `app.emit`; frontend `listen("fleet://output/" + run_id, …)` **before** invoking `fleet_start`):

```ts
// fleet://output/{run_id} — one per read chunk, per host, ordered per host by seq
interface FleetOutputChunk {
  alias: string;
  seq: number;               // u32, starts 1, monotonic per host ACROSS both streams (shared counter)
  stream: "stdout" | "stderr";
  bytes: string;             // from_utf8_lossy of ≤ 8192 raw bytes (ADR-13 boundary note)
  total_bytes: number;       // u64, running stdout+stderr observed AFTER this chunk (pre-cap truth)
  truncated: boolean;        // total_bytes > 65536 at this chunk (row marker; FR-FL25)
}
// fleet://host/{run_id} — terminal, exactly once per STARTED host (never for plan-skipped hosts)
interface FleetHostDone {
  alias: string;
  status: "ok" | "fail" | "timeout" | "skipped";
  exit_code: number | null;  // null when killed by our timeout/Stop (post-kill code meaningless)
                             // or signal-death; frontend shows code only when non-null
  reason: string | null;     // frozen codes above; null for ok + timeout
  stdout_bytes: number; stderr_bytes: number;  // u64 totals observed (uncapped)
  truncated: boolean;        // final truth for the row (stdout+stderr > 65536)
}
// fleet://summary/{run_id} — exactly once, ALWAYS last (after every host event of started hosts)
interface FleetSummary {
  run_id: string;
  ok: number; fail: number; timeout: number; skipped: number; // skipped INCLUDES plan-skips; ok+fail+timeout+skipped == plan hosts
  elapsed_ms: number;        // coordinator: after validation → last terminal state
  min_exit: number | null;   // min non-zero exit code observed among fail hosts (FR-FL29); null if none
  max_exit: number | null;
}
```

Frozen ordering rules:
1. Plan (queued/skipped initial states) = response only. Events = post-start transitions only.
2. Pre-subscription makes event loss structurally impossible; if an `emit` still fails (webview gone), the backend keeps counting and proceeds — delivery failure never kills a child or wedges state (FR-FL24 non-blocking).
3. `fleet://summary` is the last event of every non-finished run; frontend uses it to unlisten and re-enable Run (FR-FL31).
4. App quit / panel close during run → Stop path first (FR-FL37), then unlisten.

Rejection errors (exact strings — frontend maps prefix → copy; 0 spawn, 0 events):

| `Err(...)` | condition |
|---|---|
| `fleet:already-running` | a run is active (FR-FL31) or req.run_id == active run's id |
| `fleet:empty-selection` | `aliases` empty |
| `fleet:empty-input` | `text` empty/whitespace-only (backend re-gates FR-FL11) |
| `fleet:text-too-large` | `text` > 2 MiB UTF-8 bytes (FR-FL10 backend half of the gate) |
| `fleet:bad-timeout` | outside 5..600 |
| `fleet:bad-concurrency` | outside 1..20 |
| `fleet:ssh-missing` | presence probe says no ssh binary (FR-FL32; recovery-path copy is frontend) |
| `fleet:config-unreadable` | snapshot read failed (same posture as config editor refusing) |

Constants (single source of truth; frontend mirrors them with a comment naming this section):

| Name | Value | Meaning |
|---|---|---|
| `RETENTION_BYTES` | 65536 (64 KiB) | per-host display ring + `truncated` threshold |
| chunk read size | 8192 | max raw bytes per chunk event |
| poll interval | 50 ms | deadline/stop/exit granularity |
| `TEXT_MAX_BYTES` | 2097152 | request text cap (FR-FL10) |
| concurrency | default 5, range 1–20 | FR-FL18 / OQ-6 closed |
| per-host timeout | default 60 s, range 5–600 s | Clarifying Q5 closed |

Mock fixture (copy-paste for Unit 3–4; `run_id = "r-7"`): `fleet_start` resolves

```json
{ "run_id": "r-7", "finished": false, "summary": null,
  "hosts": [
    { "alias": "prod-web", "eligible": true,  "status": "queued",  "reason": null },
    { "alias": "db-core",  "eligible": true,  "status": "queued",  "reason": null },
    { "alias": "web-03",   "eligible": true,  "status": "queued",  "reason": null },
    { "alias": "pw-host",  "eligible": false, "status": "skipped", "reason": "password host (not supported in v1)" } ] }
```

then replay, in this order, via `window.__emitTauri`:

```json
{ "name": "fleet://output/r-7", "payload": { "alias": "prod-web", "seq": 1, "stream": "stdout", "bytes": "up 3d\n", "total_bytes": 7, "truncated": false } }
{ "name": "fleet://output/r-7", "payload": { "alias": "db-core", "seq": 1, "stream": "stderr", "bytes": "Host key verification failed.\r\n", "total_bytes": 31, "truncated": false } }
{ "name": "fleet://host/r-7",   "payload": { "alias": "prod-web", "status": "ok", "exit_code": 0, "reason": null, "stdout_bytes": 7, "stderr_bytes": 0, "truncated": false } }
{ "name": "fleet://host/r-7",   "payload": { "alias": "db-core", "status": "fail", "exit_code": 255, "reason": null, "stdout_bytes": 0, "stderr_bytes": 31, "truncated": false } }
{ "name": "fleet://host/r-7",   "payload": { "alias": "web-03", "status": "timeout", "exit_code": null, "reason": null, "stdout_bytes": 12, "stderr_bytes": 0, "truncated": false } }
{ "name": "fleet://summary/r-7", "payload": { "run_id": "r-7", "ok": 1, "fail": 1, "timeout": 1, "skipped": 1, "elapsed_ms": 60042, "min_exit": 255, "max_exit": 255 } }
```

All-skipped fast path (FR edge case "0 host layak"): `finished: true`, `summary` non-null (`ok/fail/timeout` = 0, `skipped` = |plan|, `elapsed_ms` small, min/max null), NO events, unlisten immediately. `fleet_stop` on it is a no-op.

Hostile-render fixture for Unit 4 CSP check: one chunk with `bytes: "<script>alert(1)</script>"` → DOM must contain the escaped text only, CSP violation counter == 0 (INV-17).

Frontend-owned behavior (not backend, fixed here so nobody re-decides): dialog copy derives counts from `read_ssh_config` DTOs before calling (FR-FL13/14/15 — `sudo` regex and `ForwardAgent yes` scan over `HostDto.options` happen in main.ts); `bash`-absent targets (exit 127) and `Host key verification failed` (exit 255 + stderr substring) get their recovery-hint extra line rendered frontend-side from event payloads — backend classification stays exit-code-honest (§12.7).

### 12.4 The five pure functions (+ argv builder) — §12.4 in `fleet.rs`, all `#[cfg(test)]`-covered, no network, no real ssh

| # | Function | Rule (frozen) | Unit-1 test |
|---|---|---|---|
| 1 | `fn host_eligibility(alias: &str, snap: Option<&HostSnapshot>) -> Eligibility` | precedence: `crate::sftp_safe(alias,"Alias").is_err()` → Skip(`unsafe alias`); `snap.is_none()` → Skip(`unknown alias`); wildcard chars → Skip(`wildcard pattern`); `auth_method=="password" \|\| has_password` → Skip(`password host (not supported in v1)`); else Run | skip-reason matrix incl. precedence |
| 2 | `fn pick_transport(mode: Mode, text: &str) -> Transport` | `Script` unless mode==Command AND text has no `\n` (multi-line input is always a script — FR-FL7) | 1-line/2-line/`\r\n`-only inputs |
| 3 | `fn normalize_script(text: &str) -> String` | `\r\n`→`\n`, then lone `\r`→`\n`, then ensure final byte is `\n` (append if missing; never strips existing newlines/blank lines — they are content) | CRLF file, lone CR, no-final-newline, ends-with-newline |
| 4 | `fn classify_status(ev: &FinishEvidence) -> (Status, Option<&'static str>)` | precedence: `killed_by_timeout` → `timeout`; `killed_by_stop` → (`skipped`, `stopped by user`); `exit==Some(0)` → `ok`; `exit==Some(n)` → `fail`; `exit==None` → (`fail`, `killed by signal`) | matrix incl. 255 and None |
| 5 | alias guard = **reuse `crate::sftp_safe`** (no new fn; leading `-`/CR/LF) | INV-19 == INV-10 family; one implementation, two call sites | existing tests carry it; fleet call-site test |
| 6 | `fn ssh_argv(bin: &str, alias: &str, t: &Transport) -> Vec<String>` (bonus pure, is the spawn's only argv source) | `[bin, "-o", "BatchMode=yes", alias]` + Command: `[text]` / Script: `["bash","-s"]`; NOTHING else is ever appended | INV-18 assert: contains BatchMode, never `StrictHostKeyChecking`/`-A`/`-X`/`-t`/`sudo` |

`HostSnapshot` is `{ alias, auth_method, has_password, is_wildcard }` built by `fn snapshot_hosts(text: &str) -> Vec<HostSnapshot>` — pure wrapper over the existing `parse_config` with the migration/self-heal write branch REMOVED from the fleet call path (INV-16; the normal `read_ssh_config` command keeps its self-heal).

### 12.5 `HOSTERM_SSH_BIN` (ADR-16) — semantics

- Read fresh per spawn (no caching): set → argv[0] = value verbatim (absolute path recommended); unset → `"ssh"`.
- Fleet spawn path ONLY: `pty_spawn` (terminal), sftp layer, `check_binaries`/`binaries_missing` never consult it.
- Fake-ssh test pattern (Unit 1): a shell script honoring the real interface — reads stdin when given `bash -s`, prints to stdout/stderr, exits with a scripted code. The concurrency ceiling is asserted by child-lifetime overlap: the fake logs its start and exit timestamps as output lines, and the test asserts no more than N overlapping intervals among the recorded pairs. Same fixtures drive timeout kill (fake sleeps 30 s vs test timeout 5 s), exit classification (codes 0/2/255/None), and the config-bytes invariant (snapshot hash + mtime before/after a multi-host run).

### 12.6 Output retention (NFR-FL3 mechanism)

Backend never buffers host output: reader threads copy one 8 KiB read into one event and drop it — per-host backend memory is O(8 KiB) regardless of run size. Two counters per host (stdout, stderr) ride in the events. The 64 KiB RETENTION ring lives in the frontend per host: append `bytes` to a string buffer; when length > 65536, cut from the FRONT (oldest bytes die), set the `truncated` row marker (also carried authoritatively on chunk + terminal events from the backend counters, so the marker survives even if display trimming is reworked). NFR-FL3 bound follows: ≤ 64 KiB × hosts (frontend) + 8 KiB × live hosts (backend) + ≤ 2 MiB input copy, plus one `fleet://summary` event.

### 12.7 Error-handling approach (fleet delta, EARS)

- IF `fleet_start` validation fails (any §12.3 error prefix) THEN THE SYSTEM SHALL reject the invoke with that exact prefix, spawn 0 processes, and emit 0 events for the run.
- IF a child fails to spawn (ssh vanished after probe, EACCES) THEN THE SYSTEM SHALL mark that host `fail` with reason `spawn failed`, a friendly message with no raw spawn strings, and release its permit to the queue.
- IF a host's ssh exits non-zero (including 255 for connect/auth/host-key failures) THEN THE SYSTEM SHALL classify `fail` with the raw exit code and stream the stderr tail, and never phrase it as "remote command failed" for 255 (ambiguity per spec edge case).
- IF a host reaches its deadline (counted from spawn, covering connect+exec — FR-FL23) THEN THE SYSTEM SHALL kill the local child, classify `timeout` with `exit_code: null`, and keep other hosts running.
- IF Stop fires (`fleet_stop` or panel-close) THEN THE SYSTEM SHALL set the run stop flag, let each host task kill its own child (≤ 50 ms), mark started-but-unfinished hosts `skipped`/`stopped by user`, mark unstarted queued hosts the same via the coordinator, and still emit exactly one summary.
- IF natural exit and kill race inside one poll tick THEN THE SYSTEM SHALL take the `try_wait` result (exit wins) — the fixed precedence that keeps fast auth failures `fail`, never `timeout`.
- IF an event emit fails THEN THE SYSTEM SHALL log nothing to disk, keep counters correct, and complete the run (delivery ≠ execution).
- IF the app quits mid-run THEN THE SYSTEM SHALL let OS process teardown reap children (ADR-7 posture); fleet writes no files, so there is nothing to clean (NFR-FL2).

### 12.8 Password hosts in fleet — v2 note (NOT v1 design)

Structural verdict for the spec's Clarifying Q3 / R-FL-1: a spawn has exactly one stdin — either the pipe carrying the script (`bash -s`, FR-FL7/8) or the PTY slave carrying a fed password. The two cannot coexist in one child, and feeding a password to `ssh <alias> <cmd>` through a PTY has never been proven in this codebase (what is proven: interactive persistent sessions — terminal PTY and persistent `sftp` — where a prompt-watching pump types the secret once). v1 therefore skips password hosts with the frozen reason (`FR-FL21`), per CLOSED OQ-1. v2 (fleet passwords) requires, BEFORE any spec: a spike proving prompt-injection into `ssh` under a piped-stdin alternative (expect-style prompt watch on a PTY with script delivered via `ssh -o … 'bash -s' < heredoc` re-think, or `SSH_ASKPASS` with a helper that never holds the secret across IPC — neither is designed here). This note is the owed input to the v2 spec author; it is explicitly out of v1 scope and blocks nothing.

### 12.9 Verification mapping (Unit 0 → Units 1–5)

- Unit 1 (runner core): `cd src-tauri && cargo test --lib` → baseline 52 + ≥ 8 new (rows §12.4 tests 1–4 + argv-builder INV-18 + eligibility precedence + concurrency-ceiling vs fake ssh + timeout kill + `snapshot_hosts` byte-identity: config file hash+mtime unchanged across a multi-host run — INV-16/NFR-FL8).
- Unit 2 (IPC): `fleet_start`/`fleet_stop` registered (inventory 23 → 24 setelah `forget_secret` ADR-17 — §11.5 self-check), payloads byte-for-byte as §12.3 structs; error prefixes asserted by string; `finished:true` fast path emits zero events.
- Unit 3–4 (frontend): mock replay of the §12.3 fixtures; Cancel path = 0 `fleet_start` invocations (NFR-FL6 smoke counter); hostile-bytes fixture + CSP counter 0 (INV-17); `truncated` marker from a >64 KiB mock (Unit 4).
- Unit 5: grep gate `sh -c|bash -c|cmd /C` = 0 hits in `src-tauri/src/fleet.rs` + fleet section of `main.ts` (INV-14); STATE.md verify numbers re-measured.

---

## 13. Redesign Dusk v1 (addendum — token-system architecture + app-data store + AI approval contract)

> Architect addendum (card t_60df4386, 31 Okt 2026), derived FROM `docs/FEATURE-SPEC-redesign-dusk-v1.md` (pm, FR/NFR/OQ) with the **visual truth** taken verbatim from the MEKL-approved prototypes `docs/UX/prototypes/proto-a-dusk.html` + `proto-a-dusk-settings.html` (+ `verification-report.json`). This extends — does not replace — §§1–12. New decisions: **ADR-18..22**; new invariants: **INV-21..27** (INV-27 = local-terminal direct PTY spawn). IPC inventory **24 → 27** at spec time (`read_settings`, `write_settings`, `read_snippets`), since grown to **31** by shipped follow-ups: `local_pty_spawn` (local terminal), `read_user_snippets` + `write_user_snippets` (snippet CRUD), `local_shell_info` (Settings → General). The §12 Fleet IPC table stays frozen; nothing here touches it.
>
> **INV-23 correction (card t_4575ff90, 1 Nov 2026; ADR-20 Rev. 2).** U-D4 QA at `ca3e86b` proved the original free-accent endpoints `#241404` / `#f2e9dc` leave a luminance gap where neither reaches `4.5:1` (`#777777` → `3.9848605473:1`; `#6f6f6f` → `4.1788873680:1`). §13.2.1 below supersedes only that endpoint rule. No IPC, store schema, or other ADR changes.
>
> **MEKL-pending assumptions (reversible defaults, NOT final facts).** nano locked these on 31 Okt as working defaults because MEKL has not overridden; every one is tagged `[ASSUME-OQ-n]` where it bites and re-surfaced in §13.9. They are design inputs here, not closed constraints like Fleet's OQ-1..9.

### 13.1 Frontend-only vs new-IPC split (the first architectural cut)

The redesign is **mostly a frontend re-skin**; only persistence and the allowlist store cross into Rust. Separated hard so the lead can parallel-plan the backend unit against the serial frontend units:

| Area | Surface | New IPC? | FR trace |
|---|---|---|---|
| Dusk token swap (`:root`, xterm/CodeMirror remap) | `src/styles.css`, `src/main.ts` theme literals | **No** — pure frontend | FR-DT1..7 |
| Shell re-skin (sidebar/tab/split/status bar/SFTP geometry+motion) | `src/styles.css`, `src/main.ts` | **No** | FR-SH1..8 |
| Settings modal + Appearance (preset picker, accent, preview) UI | `src/main.ts`, `index.html`, `src/styles.css` | **No** for the UI itself | FR-SET1..12 |
| **Theme/accent persistence** across sessions | app-data JSON | **Yes** — `read_settings`/`write_settings` (ADR-18/19) | FR-SET13 |
| Snippet panel UI (render/filter/copy) | `src/main.ts`, `index.html`, `src/styles.css` | **No** for UI | FR-SN1..5 |
| **Snippet source** (bundled default read-only v1) | bundled JSON asset + optional app-data overlay | **Yes** — `read_snippets` (ADR-19) | FR-SN6 |
| Google login chip + popover | `src/main.ts`, `index.html`, `src/styles.css` | **No** — visual-only v1 `[ASSUME-OQ-3]` | FR-GL1..5 |
| AI panel + command card + approval UX | `src/main.ts`, `index.html`, `src/styles.css` | **No** backend v1 `[ASSUME-OQ-4]` | FR-AI1..13 |
| **AI allowlist** (exact-command + host, revoke) persistence | app-data JSON (same store as settings) | **Yes** — folded into `read_settings`/`write_settings` (ADR-21) | FR-AI6/7/10 |

**Net new IPC = 3 commands** (`read_settings`, `write_settings`, `read_snippets`). Everything else is CSS variables + DOM behavior the frontend owns, and is mockable in smoke with zero backend (same posture Fleet Units 3–4 used). The AI *backend* (model, execution wiring) is explicitly **not designed here** `[ASSUME-OQ-4]` — §13.6 defines only the UX contract + the integration point, mirroring how §12.8 owed the v2 spec author a note.

### 13.2 Theme token as a SYSTEM (ADR-20 Rev. 2)

**Decision — presets are a data-driven `:root` variable set swapped by a single root attribute; custom accent is one overridable variable.** This is what lets Slate/Ink/Light be "filled in later" with zero rework (`[ASSUME-OQ-2]`).

Mechanism:
1. **Dusk is the default `:root`** — all tokens §13.3 live directly on `:root` (so the app renders Dusk with no attribute set, and a token-less boot is always a valid theme).
2. **Preset swap = `document.documentElement.setAttribute("data-theme", preset)`.** Each non-default preset is a CSS block `:root[data-theme="slate"] { … }` overriding only the surface/text/semantic tokens. Dusk = absence of the attribute (or `data-theme="dusk"`, treated identically). No per-component re-theming code — components only ever read `var(--token)`.
3. **Accent is one variable + derived.** `--accent` is the single source; `--accent-soft` and `--accent-ink` are its only derivatives. Custom accent overrides **`--accent` alone** by writing an inline style on `:root` (`document.documentElement.style.setProperty("--accent", hex)`), leaving every preset block's surface/text/status untouched — this is the prototype `set-note` guarantee ("Recolors the accent only…") expressed structurally (FR-SET8).
4. **Accent-ink contrast guard (FR-SET9/NFR-DK3; ADR-20 Rev. 2).** `--accent-ink` must stay ≥ 4.5:1 over every applied `--accent`. The five curated quick-pick values keep the approved warm ink `#241404`; every other valid accent uses the mathematically complete safe pair `#000000` / `#ffffff` under the exact rule in §13.2.1. Classification is by normalized accent **value**, never by whether the event came from a chip, color input, boot, or reload. The accent itself is never clamped, remapped, or rejected.
5. **xterm/CodeMirror remap (the literal that CSS vars can't reach).** Same pitfall as Arus U1 (STATE.md): the xterm `theme` object (23 keys) and CodeMirror `cmTheme`/`cmHighlight` are JS literals, not CSS vars. v1 scope locks them to **Dusk values only**; when a preset is active the terminal/editor palette does **not** repaint (documented limitation — terminal well stays Dusk-dark across presets). Making xterm/CM preset-aware is explicitly deferred with the preset-expose gate (`[ASSUME-OQ-2]`), because it needs its own AA pass per preset × ANSI-16.

#### 13.2.1 INV-23 free-accent rule — FROZEN (ADR-20 Rev. 2)

1. Normalize a valid accent to lowercase `#rrggbb`. If it is exactly one of the five curated values in §13.3, set `--accent-ink: #241404`; those five pairs are independently pre-verified ≥4.5:1.
2. Otherwise compute WCAG sRGB relative luminance. For each 8-bit channel, `s = channel / 255` and `linear(s) = s / 12.92` when `s <= 0.04045`, otherwise `((s + 0.055) / 1.055) ^ 2.4`; then `L = 0.2126R + 0.7152G + 0.0722B`.
3. For a non-curated value, choose `#000000` when its contrast is greater than or equal to white's; otherwise choose `#ffffff`. The equivalent exact switch is:

   ```text
   Lswitch = sqrt(0.05 × 1.05) − 0.05
           = 0.179128784747792

   L >= Lswitch  → #000000   (tie goes dark)
   L <  Lswitch  → #ffffff
   ```

   Comparing the two unrounded contrast ratios is preferred to comparing a rounded threshold. For every `L ∈ [0,1]`, the selected ratio is at least `sqrt(21) = 4.58257569495584:1`, so a 4.5 gap is impossible. The former free-color pair `#241404` / `#f2e9dc` is forbidden: its uncovered interval is `0.144049732010495 < L < 0.214786549673552`.
4. Selection must be a pure function of the normalized hex value. A quick-pick value entered through the free color input, persisted, then reloaded must still resolve to `#241404`; a non-curated value must resolve to the same black/white endpoint across those paths.
5. **No constrain/remap/reject UX.** FR-SET7 promises application of any valid `#RRGGBB`; remapping would persist a color the operator did not choose, while rejection would introduce an unspecified error/recovery flow. Changing only the derived ink preserves the chosen accent and the prototype's surface/text/status isolation (FR-SET8).

Frozen regression vectors (ratios use the formula above; expected ink is byte-exact lowercase):

| Class | Accent | Expected ink | Expected contrast |
|---|---:|---:|---:|
| curated | `#e2a35c` | `#241404` | `8.18060039728731:1` |
| curated | `#5fbfae` | `#241404` | `8.115110618605044:1` |
| curated | `#e0869b` | `#241404` | `6.842802495508517:1` |
| curated | `#aec267` | `#241404` | `9.102683799795606:1` |
| curated | `#b99be3` | `#241404` | `7.529220380096464:1` |
| free extreme | `#000000` | `#ffffff` | `21:1` |
| free existing dark sentinel | `#101010` | `#ffffff` | `19.028110547666497:1` |
| free QA failure | `#6f6f6f` | `#ffffff` | `5.024865064757633:1` |
| free grayscale below switch | `#757575` | `#ffffff` | `4.607518093747377:1` |
| free closest 24-bit value below switch | `#766cb5` | `#ffffff` | `4.58257700240974:1` |
| free closest 24-bit value above switch | `#cf0dcc` | `#000000` | `4.5825758149421025:1` |
| free grayscale above switch | `#767676` | `#000000` | `4.6232848849972035:1` |
| free QA failure | `#777777` | `#000000` | `4.68949989000882:1` |
| free extreme | `#ffffff` | `#000000` | `21:1` |

Preset readiness gate (NFR-DK2): a preset may be **exposed as functional** only after full-shell AA is proven for *every* text pair in that preset. **Slate/Ink/Light are now SHIPPED as functional** (MEKL, 2026-11-04; resolves `[ASSUME-OQ-2]` toward "ship all four"): each is a `:root[data-theme="…"]` CSS block (styles.css) overriding the surface/line/text/accent/status tokens with the §729-731 AA-proven palette; the `aria-disabled` + "coming" badge are removed, so clicking swaps `data-theme` and persists via `write_settings`. **Theme-aware terminal + editor palettes SHIPPED (theme palettes v1, FR-THM2):** the former "xterm/CodeMirror JS-literal palettes are Dusk-only" caveat is resolved — `THEME_TERMINAL` and the per-preset CodeMirror `HighlightStyle` are now built from `src/theme-palettes.json` (SSOT, WCAG-gated by `qa-reports/theme-contrast.mjs`; Dusk byte-identical to the old literals); `applyTheme` repaints every live terminal via `term.options.theme` and re-highlights every open editor through a `Compartment` (no Terminal/PTY/EditorView recreation; doc, cursor and dirty flag survive). Spec: docs/tsd/theme-palettes-v1.md §2.

### 13.3 Dusk tokens (verbatim from prototype `:root`) — FR-DT1

Taken byte-exact from `proto-a-dusk.html` / `-settings.html`. Zero Abyss-Emerald literal may remain in `src/` (FR-DT1, grep gate INV-21).

| Token | Value | Role |
|---|---|---|
| `--bg` | `#191512` | app background |
| `--sidebar` | `#1d1814` | sidebar / tab bar / status bar / drawer |
| `--surface` | `#221c17` | row hover, card, input |
| `--raised` | `#2a231c` | chip, kbd, avatar, btn-secondary |
| `--term` | `#14100d` | terminal well + cmd-line/output bg |
| `--border` | `#332a21` | 1px line |
| `--hairline` | `rgba(226,163,92,.14)` | faint accent line |
| `--text` | `#f2e9dc` | primary text |
| `--dim` | `#b9aa95` | secondary text |
| `--faint` | `#93836f` | tertiary / placeholder |
| `--accent` | `#e2a35c` | honey amber — focus, active tab, primary action |
| `--accent-ink` | `#241404` | static fallback + curated-accent ink (8.18:1 over default); non-curated free accents override with §13.2.1 black/white guard |
| `--accent-soft` | `rgba(226,163,92,.12)` | faint accent fill |
| `--live` | `#9dc489` | sage — live status (semantic only) |
| `--danger` | `#e07856` | terracotta — error/dead/reject (semantic only) |
| `--r-lg`/`--r-md`/`--r-sm` | `10px`/`8px`/`6px` | radius |
| `--shadow-drawer` | `0 10px 30px rgba(0,0,0,.42)` | elevation — **drawer + settings modal ONLY** |
| `--font-ui` | Inter-stack, system fallback | UI |
| `--font-mono` | JetBrains-Mono-stack, system fallback | mono/terminal |

Geometry (FR-SH1/2, prototype §3.2): sidebar `248px`, host-row `34px`, tab `32px` / tab-bar `42px`, drawer `340px`, Settings modal `900×620` (max `94vw×92vh`), status bar `24px`, term divider `4px` `col-resize`, base font `13px`/lh `1.5`, Settings scrim `rgba(12,9,6,.55)`, Google button radius `4px` (brand, not `--r-*`).

Non-Dusk preset token sets (for the preset CSS blocks when expose-gated open) — surface/text/accent only, verified pairs from the report:
- **Slate** `data-theme="slate"`: `--bg #171d24` · `--surface #222a33` · `--accent #6cc4dc` · `--text #e4ebf2` (text 14.11:1, accent 8.54:1).
- **Ink** `data-theme="ink"`: `--bg #141414` · `--surface #1e1e1e` · `--accent #e0563f` · `--text #ececec` (text 15.59:1, accent 4.89:1).
- **Light** `data-theme="light"`: `--bg #f5f1e8` · `--surface #ffffff` · `--accent #96571a` · `--text #26201a` (text 14.29:1, accent 5.07:1).

Quick-pick accent chips (FR-SET6): amber `#e2a35c`, teal `#5fbfae`, rose `#e0869b`, lime `#aec267`, violet `#b99be3`. This exact normalized set is the value-based curated allow-set for `#241404` ink (§13.2.1); interaction source is not part of the mapping.

### 13.4 Store: settings + snippet + allowlist (ADR-18/19/21)

**One app-data JSON file for settings+allowlist; one read-only bundled JSON for default snippets** (`[ASSUME-OQ-5]`). This is additive — zero impact on `~/.ssh/config` or the sidecar (INV-22).

**Location (ADR-18).** Follow the existing `config_path()`/`secrets_path()` posture: resolve via the **`dirs` crate already in the dependency tree** (NOT a new `tauri-plugin-store`/`tauri-plugin-fs` — zero new deps, same rule as NFR-FL1). Path = `dirs::config_dir()/hosterm/settings.json`:
- Linux: `~/.config/hosterm/settings.json`
- Windows: `%APPDATA%\hosterm\settings.json`
- macOS (dev only): `~/Library/Application Support/hosterm/settings.json`

`HOSTERM_DATA_DIR` env override (test hook, parity with `HOSTERM_CONFIG`) points the parent dir elsewhere so smoke runs against a temp file.

**Schema — `settings.json` (ADR-18, data model §13.7):**

```jsonc
{
  "version": 1,                       // schema version; unknown → read as default, never crash
  "theme": "dusk",                    // "dusk" | "slate" | "ink" | "light"; unknown → "dusk"
  "accent": "#e2a35c",                // #RRGGBB; invalid → ignored, falls back to preset accent
  "allowlist": [                      // AI exact-command approvals (ADR-21)
    { "host": "prod-web", "command": "docker system prune -af", "added_at": "2026-10-31T06:00:00Z" }
  ]
}
```

**Snippets — `read_snippets` (ADR-19).** The bundled read-only default is shipped as a JSON asset (`src/assets/snippets.default.json`, embedded in the bundle, zero network, FR-DT5/NFR-DK6 clean). **As shipped (commit `e9a5f9c`, PRD FR-SN9) the bundled set is EMPTY (`[]`)** — the read path + read-only contract (ADR-19) are retained for forward-compat, but no default snippets ship; all snippets are user-created via the CRUD overlay below. The bundled shape, when populated, is:

```jsonc
[ { "category": "Docker", "items": [ { "name": "Prune everything", "command": "docker system prune -af" } ] },
  { "category": "Git",    "items": [ … ] },
  { "category": "systemd","items": [ … ] } ]
```

`read_snippets` returns the bundled set (still read-only, ADR-19 intact). **The user-editable overlay is now SHIPPED** (resolves `[ASSUME-OQ-5]` toward "bundled stays read-only, user rows are CRUD"): `snippets.user.json` in the same app-data dir holds user snippets, read via `read_user_snippets` and persisted via `write_user_snippets` (atomic whole-file, same discipline as `write_settings`). Each `UserSnippet` is `{ id, name, command, category }`; `id` is backend-assigned (stable key for edit/delete), name+command are validated non-empty (trimmed), empty category defaults to `"Custom"`. The frontend renders bundled groups first (no edit/delete), then user groups with per-row copy/edit/delete. Merge is concatenation, not dedup — bundled and user rows are distinct namespaces.

**Write posture (INV-22/24).** `write_settings` is **atomic + whole-file** (sibling temp → rename), identical to the config/sidecar write discipline (§5.1). No partial writes, no self-heal of `~/.ssh/*`. An unreadable/corrupt `settings.json` → the backend returns defaults (theme=dusk, accent=#e2a35c, allowlist=[]) and does **not** overwrite until the next legitimate `write_settings`, so a hand-corrupted file is never silently clobbered on read (mirrors "unreadable config refuses edit", ADR-4).

**Portability (NFR-8 / NFR-DK7).** The store lives **outside `~/.ssh/`**, so the existing "copy `~/.ssh` to move machines" story is unchanged. Settings/snippets/allowlist do **not** travel with the ssh config by default — documented trade-off: a device switch carries hosts+keys (ssh dir) but resets theme/allowlist unless the user also copies `~/.config/hosterm/`. This is the deliberate separation that keeps the ssh config pure (INV-3 class) and is the honest answer MEKL must confirm in `[ASSUME-OQ-5]` (does the snippet/settings store ride along, or stay machine-local?).

### 13.5 New IPC contract (frozen like §12.3 — lead's TSD implements verbatim; frontend mocks verbatim)

Conventions identical to §12.3: command args camelCase at the boundary, JSON payload fields snake_case. Inventory **24 → 27** (`grep -rc "tauri::command" src-tauri/src/*.rs` must read 27 after these land). This block is the freeze — a shape change here is an addendum revision, never an implementation drift.

```ts
invoke("read_settings"): Promise<Settings>
// Never throws for a missing/corrupt file: returns DEFAULT_SETTINGS instead (INV-24).
interface Settings {
  version: number;                 // 1
  theme: "dusk" | "slate" | "ink" | "light";   // unknown value coerced to "dusk" by backend
  accent: string;                  // "#RRGGBB"; backend validates 6-hex, else returns preset default
  allowlist: AllowEntry[];
}
interface AllowEntry {
  host: string;                    // ssh config alias, EXACT (no pattern/glob — v1, OQ-6 default)
  command: string;                 // exact command string the user approved
  added_at: string;                // RFC 3339 UTC; backend-stamped on write, display/audit only
}

invoke("write_settings", { settings: Settings }): Promise<null>
// Atomic whole-file write (temp → rename), 0600-class perms where the OS supports it.
// Backend re-validates: theme enum, accent 6-hex, allowlist entries non-empty host+command.
// Rejects with "settings:invalid-accent" | "settings:invalid-theme" | "settings:write-failed".
// NEVER reads/writes ~/.ssh/config or the sidecar on this path (INV-22).

invoke("read_snippets"): Promise<SnippetGroup[]>
// Bundled read-only default set; never throws (empty array on read failure).
interface SnippetGroup { category: string; items: Snippet[] }
interface Snippet { name: string; command: string }

invoke("read_user_snippets"): Promise<UserSnippet[]>
// User CRUD snippet store (snippets.user.json); never throws (empty array on missing/corrupt).
invoke("write_user_snippets", { snippets: UserSnippet[] }): Promise<null>
// Atomic whole-file write; validates name+command non-empty (trimmed), assigns id to new rows,
// defaults empty category to "Custom". Rejects "snippets:empty-name" | "snippets:empty-command" | "snippets:write-failed".
interface UserSnippet { id: string; name: string; command: string; category: string }

invoke("local_shell_info"): Promise<{ shell: string; overridden: boolean }>
// Read-only info for Settings → General: the shell a Local terminal launches and whether
// HOSTERM_LOCAL_SHELL is overriding the OS default. No persistence, no ssh-config/sidecar access.
```

Rejection strings (frontend maps prefix → copy; 0 file write on any): `settings:invalid-accent`, `settings:invalid-theme`, `settings:write-failed`.

**No events.** Unlike Fleet, this surface is request/response only — there is no streaming. The AI approval flow (§13.6) is **frontend-state-driven in v1** (no backend AI), so it emits no IPC until the AI backend epic defines one `[ASSUME-OQ-4]`.

Mock fixture (copy-paste for the Settings/AI frontend units):

```json
{ "version": 1, "theme": "dusk", "accent": "#e2a35c",
  "allowlist": [ { "host": "prod-web", "command": "sudo systemctl restart netdata", "added_at": "2026-10-30T12:00:00Z" } ] }
```

### 13.6 AI command-approval contract (FROZEN like §12 Fleet — security-sensitive)

Human-in-the-loop is mandatory and explicit. In v1 there is **no AI backend and no new execution path**: a "Run once" executes through the **existing real-ssh terminal** for that host (ADR-1/2), exactly as if the user typed the command — no second execution engine is introduced `[ASSUME-OQ-4]`.

**Command-card state machine (frozen, FR-AI2/3/7):**

```
proposed ─► AWAITING APPROVAL ─┬─ Run once      ─► (execute once via terminal) ─► done, allowlist UNCHANGED
  (cmd-card.awaiting,          ├─ Always allow   ─► (persist AllowEntry + execute) ─► ALWAYS ALLOWED
   cmd-state.warn)             └─ Reject         ─► rejected (no execute, no allowlist write)

Any newly-proposed command whose (host, command) EXACTLY matches an AllowEntry renders directly as
ALWAYS ALLOWED (cmd-state.ok, sage) and MAY execute without re-prompting (FR-AI7).
```

Prototype markup is the visual truth (`proto-a-dusk.html` lines 580–608):
- Awaiting: `.cmd-card.awaiting` + `.cmd-state.warn` "AWAITING APPROVAL"; actions `button.btn-primary.approval-run` **Run once** / `button.btn-secondary.approval-always` **Always allow** / `button.btn-ghost.approval-reject` **Reject**; foot copy "Allowlist stores this exact command only. Review patterns in Settings > Approvals."
- Allowed: `.cmd-state.ok` "ALWAYS ALLOWED".

**Visual-weight invariant (FR-AI4, INV-25).** "Run once" is `btn-primary` (accent fill, the eye/click default); "Always allow" is `btn-secondary` (muted, `--raised`). This is a *security* property — the permission-widening action must never be the primary target — and is asserted in smoke (INV-25), not left to CSS drift.

**Allowlist data = exact-command-only (ADR-21, `[ASSUME-OQ-6]` default).** An `AllowEntry` is the tuple `(host, command)` compared **byte-exact**; no pattern, glob, prefix, or normalization in v1. Stored in `settings.json.allowlist` (§13.4). "Always allowed" fires only for an exact `(host, command)` re-match. **Revoke** (FR-AI10) = Settings › AI Assistant/Approvals is a **read + revoke list**: render each entry, revoke = remove that entry and `write_settings`. It is NOT a pattern editor in v1.

**Execution safety (FR-AI13, reuses Fleet posture).** Before any execute, a proposed command's **host alias** passes the same `sftp_safe` guard Fleet uses (leading `-`, CR/LF → refuse, INV-19 family → INV-26). The command string itself reaches the host only through the existing terminal-PTY write path (never `sh -c` locally — INV-14 class), so no new local-shell surface is created.

**Hostile-output escape (FR-AI11, INV-11 family).** Both AI thread text and any rendered command output route through `esc()`/`escAttr()` — AI/remote output is hostile input, same rule as Fleet §12.3 and SFTP. CSP (ADR-6) remains the structural backstop.

**Threat note (what the contract prevents):**
- *Execution without consent* — FR-AI9 forbids any implicit/auto-run; the ONLY execute paths are an explicit Run-once click or an exact allowlist re-match the user previously created by clicking Always-allow. A compromised/hallucinating AI cannot run anything the user never approved.
- *Mis-click widening permissions* — INV-25 keeps "Always allow" visually secondary so the reflexive click is the one-shot, not the standing grant.
- *Allowlist as a blast-radius* — exact-command-only (ADR-21) means an approved `docker system prune -af` can never silently authorize `docker system prune -af --volumes`; the tuple must match byte-for-byte. Pattern/glob is explicitly out (`[ASSUME-OQ-6]`) precisely because it widens blast radius and needs its own security design (anchoring/escaping/preview-match), exactly as §12.8 deferred fleet passwords.
- *Secret leakage* — the allowlist stores commands + host aliases only; it lives outside `~/.ssh/` and never holds secrets (INV-22). Stored passwords never cross into the AI path (INV-4 class).

### 13.7 Data model (new entities)

No DB; one new app-data file + one bundled asset (§13.4). Entities:

```
Settings (settings.json, app-data)
├── version: u32
├── theme: enum { dusk, slate, ink, light }        # default dusk
├── accent: string "#RRGGBB"                        # default #e2a35c
└── allowlist: AllowEntry[]
    ├── host: string        # ssh alias, exact
    ├── command: string     # exact approved command
    └── added_at: string    # RFC 3339 UTC, audit only

SnippetGroup[] (snippets.default.json, bundled read-only)
└── { category: string, items: { name, command }[] }
```

Evolution: `version` gates future migration; unknown `version` or unparseable file → defaults on read, no destructive rewrite (ADR-4 posture). A user snippet overlay adds a sibling file, never mutates the bundled default — additive, no model break.

### 13.8 Impact on the verify loop + invariants

**New ADRs:**
- **ADR-18** — settings/allowlist store = one app-data JSON resolved via existing `dirs` crate (NOT `~/.ssh`, NOT a new plugin dep); atomic whole-file write; `HOSTERM_DATA_DIR` test override.
- **ADR-19** — snippet source v1 = bundled read-only JSON asset (`read_snippets`); user overlay deferred.
- **ADR-20 Rev. 2** — theme = data-driven `:root` token system swapped by one `data-theme` attribute; accent = single `--accent` variable overridden inline; presets are "fill-in-later" CSS blocks; Dusk = default (no attribute). INV-23 correction: curated accents retain `#241404`; all other valid accents derive ink from `#000000` / `#ffffff` at `Lswitch = sqrt(0.0525) − 0.05`, without changing or rejecting the accent (§13.2.1).
- **ADR-21** — AI allowlist = exact `(host, command)` tuple, byte-match, no pattern in v1; revoke-list UI, not a pattern editor.
- **ADR-22** — Google login + AI are **visual-only v1**; no OAuth backend, no AI backend; the approval UX contract (§13.6) is frozen independent of any future backend, and execution reuses the existing real-ssh terminal (no new path).

**New invariants (carry named checks; flow to STATE.md when units land):**

| INV | Property | Check |
|---|---|---|
| INV-21 | Zero Abyss-Emerald literal survives in `src/`; all rendered tokens are the Dusk values §13.3 | grep gate (`#08080a`/`#00d992`/`--glow` etc. = 0 hits) + smoke `:root` token asserts |
| INV-22 | `read_settings`/`write_settings`/`read_snippets` never read or write `~/.ssh/config` or the sidecar; store lives under app-data only | cargo test: config+sidecar hash/mtime unchanged across a settings write; FS diff |
| INV-23 | For every applied accent, `--accent-ink` contrast over `--accent` stays ≥ 4.5:1: curated set → `#241404`; every other value → §13.2.1 black/white guard; mapping is stable across input/boot/reload | smoke the complete frozen vector table, including both former failures + both switch sides; independently recompute ratios (not implementation helper reuse) |
| INV-24 | A missing/corrupt `settings.json` yields `DEFAULT_SETTINGS` on read and is never overwritten until a legitimate write | cargo test: corrupt-file read returns defaults, file bytes unchanged |
| INV-25 | "Run once" renders `btn-primary`, "Always allow" renders `btn-secondary` (permission-widening action never primary) | smoke: class assertion on the approval card |
| INV-26 | A proposed command's host alias passes the `sftp_safe` guard before any execute (leading `-`/CR/LF refused) | reuses INV-19/INV-10 impl; frontend call-site smoke |
| INV-27 | The local terminal (`local_pty_spawn`) spawns the OS default shell DIRECTLY as a PTY child (`CommandBuilder::new(shell)`), never by interpolating a user string into `sh -c`/`bash -c`/`cmd /C`. The shell program is the OS default (`$SHELL`/`%COMSPEC%`, fallback `/bin/bash`/`cmd.exe`) or the exact `HOSTERM_LOCAL_SHELL` override — a bare program path, not a command line. This is the INV-14 posture applied to the interactive local shell: user keystrokes flow as PTY stdin bytes, identical to how `pty_spawn` feeds `ssh`; no new local command-injection surface is created. | `cargo test --lib` `auth_key_tests::default_local_shell_honors_override_then_os_default` (override wins, OS default otherwise, resolved value never an `-c` command string) + grep gate: `sh -c`/`bash -c`/`cmd /C` = 0 in `local_pty_spawn` path |

Plus the carried-forward Dusk NFR checks (pm §5): INV-DnD (`grep draggable` = 0, NFR-DK4/INV-6), reduced-motion kill-switch (NFR-DK5), zero-network font (NFR-DK6/FR-DT5), CSP counter == 0 with hostile AI/snippet fixtures (NFR-DK9/INV-11 family).

**Verify-loop deltas (baseline re-measured in working tree 31 Okt, HEAD 31bef63):**
- `cargo test --lib` → **78 pass** today (lib 57 + fleet 21 by static count; STATE.md canon = 77). The store commands add cargo tests (write/read round-trip, corrupt-file defaults, config/sidecar byte-identity INV-22/24) → baseline + ≥ 4.
- `node qa-reports/headless-smoke.mjs` → **225 pass** today. Each Dusk unit adds smoke for its behavior (token asserts, preset swap, accent+contrast, approval-card classes, hostile-output CSP); final number re-measured per unit, not hard-coded here.
- IPC inventory gate: `grep -rc "tauri::command" src-tauri/src/*.rs` must move **24 → 27** when the store commands land (lib.rs 22 → 25; fleet.rs 2 unchanged). §11.5 self-check updates accordingly.
- `npm run build` → 0 TS errors (unchanged gate).

**INV-23 Rev. 2 fresh-QA gate.** The frontend correction is not accepted on the prior U-D4 `255/255` result. QA must test the new implementation SHA, rerun the full canonical loop, execute the frozen vectors through the rendered Settings color-input path, verify input → persist → reload mapping stability, and reproduce the former `#6f6f6f` / `#777777` probes twice with ratios ≥4.5. Re-measure the smoke total; do not hard-code the old count.

**Invariants that MUST stay green (STATE.md §Hard invariants 1–6):** config parseable + round-trip (1/2), no secret in config (3 — the store is explicitly outside this boundary), paste CRLF→LF (3/INV-5), **drag = manual mouse, never HTML5 DnD** (4/INV-6 — any new drag in snippet reorder etc. follows this), staging grep (5), authorized_keys untouched (6). None of this addendum's surfaces alter them; the store write is the one new FS mutation and it is confined to app-data.

### 13.9 Hotspot + recommended unit sequence (RECOMMENDATION — final granularity = lead's TSD)

**Hotspot (binding, from Arus §0 + STATE.md):** `src/styles.css` and `src/main.ts` are touched by nearly every Dusk unit. Units that touch either file **must be serial — one active card per file, never parallel.** The backend store unit (new `settings`/`snippets` commands in `lib.rs`) is **file-disjoint** from the frontend hotspot and MAY dispatch in parallel with an early frontend unit (same way Fleet FL-1 ran parallel to Arus U5–U9), as long as no frontend unit is mid-flight in `lib.rs`.

> `hotspot: src/styles.css + src/main.ts — every Dusk frontend unit touches them; serialize, one card per file (Arus §0 posture).`

Recommended order (dependency-driven; lead locks the actual unit boundaries):
1. **Dusk token swap** — `:root` tokens §13.3 + xterm/CM literal remap + glow→elevation moved to drawer/modal only. Foundation for everything. (frontend hotspot; analog of Arus U1)
2. **Shell re-skin** — sidebar/tab/split/status bar/SFTP to Dusk geometry+motion, behavior preserved. (serial, hotspot)
3. **Store backend** (`read_settings`/`write_settings`/`read_snippets`, app-data JSON, bundled snippets asset). *File-disjoint — can run parallel to #1/#2 as long as no frontend card is in `lib.rs`.*
4. **Settings modal + Appearance** — preset picker (Dusk functional; others gated "coming" per `[ASSUME-OQ-2]`), accent control + contrast guard, live preview, persist via #3. Needs tokens (#1) + store (#3).
5. **Snippet panel** — render/filter/copy from `read_snippets`. Needs #3.
6. **Google login visual** — chip + popover, no backend (`[ASSUME-OQ-3]`).
7. **AI assistant + approval** — thread, command card, allowlist exact + revoke list (§13.6). **Last** — most security-sensitive; depends on #3 (allowlist persistence) + #1 (tokens).
8. **Smoke canonization + docs** — fold all new checks, re-measure baselines, update STATE.md (analog of Arus U9).

### 13.10 Open items still gated on MEKL (next gate for nano)

Every `[ASSUME-*]` below is a reversible default nano locked 31 Okt; none is a closed constraint. Listed so nano knows what to confirm before / during the relevant unit:

- **OQ-1** (Dusk supersedes Arus as active identity; Arus = archived history): assumed **yes**. STATE.md §layout update (Arus token notes → historical) recommended **when the token unit lands**, not now — architect/lead does it then, not pm.
- **OQ-2** (non-Dusk preset scope): **RESOLVED — Slate/Ink/Light SHIPPED as functional** (MEKL, 2026-11-04, commit `e9a5f9c`; was: assumed Dusk-only, others disabled "coming" placeholders). Each is a `:root[data-theme="…"]` CSS block overriding surface/line/text/accent/status with the §729-731 AA-proven palette; the `aria-disabled` + "coming" badge are removed. **Follow-up also shipped (theme palettes v1):** xterm/CodeMirror JS-literal palettes now repaint per preset from `src/theme-palettes.json` — `applyTheme` swaps `term.options.theme` on live terminals and re-highlights open editors via a `Compartment` (§698, PRD FR-THM2 shipped).
- **OQ-3** (Google OAuth backend): assumed **visual-only v1** — chip + popover, no real session, no cloud sync. Backend (provider/redirect/scope/what-syncs/token-storage=sidecar-class) is a later epic; MEKL must confirm that Dusk opening a cloud-sync door is acceptable vs PRD "zero cloud".
- **OQ-4** (AI backend): assumed **deferred** — approval UX frozen (§13.6), no model/execution backend designed. §13.6 is the owed input to the future AI-backend spec author (mirrors §12.8). MEKL picks the direction (on-device vs remote; privacy/NFR-3 impact) in that epic.
- **OQ-5** (store location + portability): **PARTIALLY RESOLVED — user-snippet CRUD overlay SHIPPED** (MEKL, 2026-11-04, commits `9b80c94`/`e9a5f9c`; §13.4, PRD FR-SN7..9). Store = `dirs::config_dir()/hosterm/` (`settings.json` + `snippets.user.json`), outside `~/.ssh/`, atomic whole-file writes. Bundled default snippets now ship EMPTY (`snippets.default.json` → `[]`, FR-SN9); all snippets are user-created. **Still open:** the portability trade-off — whether settings/snippets ride along on a device move or stay machine-local (NFR-8, §13.4) — remains for MEKL to confirm.
- **OQ-6** ("Always allow" scope): assumed **exact-command-only v1**; pattern/glob = separate security epic. MEKL confirms the roadmap exists (prototype foot copy promises "Review patterns in Settings › Approvals") but is out of v1.

---

*Sources: docs/PRD.md, docs/archive/SDD-hosterm-v1.md, docs/SDD-signing-updater.md, docs/EPIC-TSD-redesign-arus-v1.md, docs/archive/FEATURE-SPEC-wave-2-draft.md, docs/FEATURE-SPEC-fleet-run-v1.md, docs/STATE.md, docs/SPIKE-csp-findings.md, src-tauri/src/lib.rs, src/main.ts, qa-reports/headless-smoke.mjs, src-tauri/tauri.conf.json (all read 2026-10-19). Note: PRD-hosterm-v2.md landed mid-authoring from pm; header/flows reconciled against it. §12 addendum + ADR-13..16 + INV-14..19 authored for card t_a367888f (Fleet Run Unit 0). §13 Dusk addendum + ADR-18..22 + INV-21..26 authored for card t_60df4386 (Redesign Dusk v1 SDD), derived from docs/FEATURE-SPEC-redesign-dusk-v1.md + docs/UX/prototypes/proto-a-dusk{,-settings}.html + verification-report.json (all read 2026-10-31). §14 Ansible v1 addendum + ADR-A1..A3 + INV-28..31 authored for card t_ed2de8f6 (Ansible v1 SDD), derived FROM docs/FEATURE-SPEC-ansible-v1.md (commit 9dae0b1, nano-verified + MEKL-confirmed 2026-11-05) + live tree src-tauri/src/lib.rs (local_pty_spawn:1768, check_binaries:1901) + src/main.ts (fleet group-derive) (all read 2026-11-05). §15 Ansible auto-install v1 addendum + ADR-AI1..AI7 + INV-32..35 authored for card t_04709068 (Ansible auto-install SDD), derived FROM docs/FEATURE-SPEC-ansible-autoinstall-v1.md (spec commit `4b87a97` + OQ-AI2 lock `b22fdc5`, MEKL-approved 2026-11-12) + live tree src-tauri/src/lib.rs (PtyState/PtySession/NEXT_ID/pump_pty/pty_kill; binaries_missing/check_binaries/ansible_missing/check_ansible/ansible_run/build_ansible_argv) + src/main.ts (ansState/ansRefreshCheck/`pty://exit` handler) (all read 2026-11-12).*

---

## 14. Ansible v1 (addendum — playbook-runner architecture + frozen IPC contract)

> Architect addendum (card t_ed2de8f6, 5 Nov 2026), derived FROM `docs/FEATURE-SPEC-ansible-v1.md`
> (commit `9dae0b1`, nano-verified + MEKL-confirmed 2026-11-05). **Design-first lane:** the spec's
> scope lock (§0), its LOCKED Clarifying-Question answers (§1), and its ADR-A1/A2/A3 (§7) are CLOSED
> constraints, not inputs to re-debate here. This section supplies exactly what the lead's TSD owes
> for unit decomposition: the component map, the frozen IPC contract (new commands + payloads), the
> ephemeral-store schemas, four new named invariants (INV-28..31), and the formal ADRs. Units code
> against §14.4 verbatim; the frontend mocks this contract without any backend. A payload or name
> change requires an edit to this section (new ADR revision), never an implementation-side drift.
>
> **Transport is a reuse, not an invention.** The whole feature spawns ONE new kind of local PTY
> child — real `ansible-playbook` — over the exact `local_pty_spawn` mechanism (lib.rs:1768). This
> is the Fleet-Run posture (ADR-1 "spawn the OS real binaries") applied to a second binary. Nothing
> about ssh/sftp/config changes.

### 14.1 Constraints honored (trace)

| Constraint (FEATURE-SPEC) | Mechanism (this section) |
|---|---|
| FR-ANS1/ANS2 — detect `ansible-playbook`, guide+block when missing | `check_ansible` command (§14.4), read-only resolve, pattern of `check_binaries` (lib.rs:1901); ADR-A3 |
| FR-ANS6 / NFR-ANS6 — ephemeral inventory 0600 from hosterm hosts, deleted after run, never writes `~/.ssh/config` | backend generates INI inventory + ephemeral cfg in `data_dir()/hosterm/`, unlinked on run finish/stop; INV-29, INV-30 |
| FR-ANS7 / FR-ANS11 / NFR-ANS5 — argv built as array, never `sh -c`/shell string | `ansible_run` builds a fixed `Vec<String>` argv → `CommandBuilder` (same discipline as INV-14 fleet); INV-28 |
| FR-ANS4 / FR-ANS13 — password hosts skipped, secrets never in argv/env/`ps`/IPC | `host_eligibility_ansible` (pure, mirrors fleet) emits `skipped`+reason; only `ansible_ssh_private_key_file` **path** goes to inventory; ADR-A2, INV-31 |
| FR-ANS9 / FR-ANS10 — dry-run WAJIB gate + explicit confirm before real run, never auto-run | frontend state machine (§14.5): Run-real DISABLED until a `--check` run for the current playbook+target succeeds; mirrors INV-25 approval posture |
| FR-ANS12 / FR-ANS17 — host key checking ON, fail-closed | backend pins `ANSIBLE_HOST_KEY_CHECKING=True` + `ANSIBLE_CONFIG=<ephemeral cfg>` on spawn; ADR-A1 |
| FR-ANS14 / FR-ANS15 — history app-data JSON, atomic 0600, corrupt→empty, 200 FIFO, no full output/secret | `ansible-history.json` via `read_ansible_history`/`write_ansible_history`, pattern of `settings.json` (INV-22/INV-24); INV-30 |
| FR-ANS8 — hostile `PLAY RECAP`/task output | `esc()`/`escAttr()` at every Ansible render (ADR-8); no new `innerHTML` sink |
| NFR-ANS1 / NFR-ANS3 — zero new Rust/JS deps, zero bundled python/ansible, zero idle network | all new backend logic is stdlib + `portable_pty` (already a dep); binary is user-installed; INV-28 path adds no runtime |
| NFR-ANS4 — a run never mutates `~/.ssh/config` or sidecar | the Ansible path never calls config/sidecar writers; cargo byte-identity test; INV-29 |

### 14.2 Component map

```
src-tauri/src/lib.rs          ← 3 NEW #[tauri::command] wrappers (thin) + pure helpers:
                                  • check_ansible() -> AnsibleCheck         (resolve ansible-playbook, read-only)
                                  • ansible_run(req: AnsibleRunReq) -> u32  (generate inventory+cfg, spawn PTY, return pty id)
                                  • ansible_history(op: AnsibleHistoryOp)   (read / append — atomic, app-data)
                                  pure cores (unit-testable, no I/O): build_ansible_argv(),
                                  build_inventory_ini(), host_eligibility_ansible(),
                                  ansible_missing() (mirrors binaries_missing:1883),
                                  history_append_fifo() (200-cap).
                                  REUSES: default pump_pty / NEXT_ID / PtyState / pty_kill / pty_write /
                                  pty_resize — ansible_run returns a pty id into the SAME PtySession map,
                                  so stop/resize/write are the EXISTING commands (0 new pty plumbing).
src/main.ts                   ← Ansible panel: binary-missing guide (reuse W2-A openSshGuide pattern),
                                  playbook file-picker + validate, host/group picker (reuse fleet picker),
                                  dry-run→recap summary, Run-real gate + confirm dialog, PTY terminal tab
                                  (reuse openTerminal local path), History sub-panel. All remote-origin
                                  render via esc()/escAttr(); markup cloned from static index.html templates.
index.html                    ← static Ansible panel markup + templates (guide modal, host-row, recap row,
                                  confirm dialog, history row) — renderer clones, never dynamic innerHTML.
qa-reports/headless-smoke.mjs ← extend: panel render, mock ansible_missing:true guide, dry-run-wajib gate,
                                  confirm-before-real, hostile PLAY RECAP → 0 CSP/0 script, sh -c grep = 0.
```

**Why `ansible_run` returns a PTY id and not a new event family.** Fleet Run needed `fleet://` events
because it fans out N parallel ssh children with per-host aggregation. Ansible v1 spawns exactly ONE
`ansible-playbook` child whose own stdout already interleaves all hosts (`PLAY RECAP`). So the child
rides the **existing** `pty://output/{id}` + `pty://exit/{id}` events (same as `local_pty_spawn`), and
Stop/resize/write are the existing `pty_kill`/`pty_resize`/`pty_write` — **no new event family, no new
pty command.** Recap parsing is frontend-side off the output stream. This keeps the backend diff to the
three genuinely-new commands.

Data flow (one run):

```
frontend: openAnsiblePanel()
  │ invoke check_ansible() ──► AnsibleCheck{ missing }   (read-only PATH resolve, 0 spawn, FR-ANS1)
  │   missing=true → render per-OS guide, block run, re-check button (FR-ANS2, W2-A pattern)
  ▼
frontend: pick playbook (file picker, *.yml/*.yaml, UTF-8, ≤2 MiB, FR-ANS5) + pick hosts/groups
  │   host rows from existing sidebar query; password-auth rows marked skipped (FR-ANS4)
  ▼
frontend: Dry-run → invoke ansible_run(req{ mode:"check", playbook, targets, aliases })
  │ backend (ansible_run):
  │   host_eligibility_ansible(aliases) → eligible (key/agent) | skipped(password/unsafe) — pure
  │   build_inventory_ini(eligible)  → write data_dir()/hosterm/ansible-inv-<rand>.ini   (0600)
  │   write ephemeral cfg            → data_dir()/hosterm/ansible-cfg-<rand>.ini          (0600)
  │   build_ansible_argv(check=true) → ["-i",<inv>,"--check","--diff","--limit",<targets>,<playbook>]
  │   CommandBuilder::new("ansible-playbook").args(argv)  +env HOST_KEY_CHECKING=True, ANSIBLE_CONFIG=<cfg>
  │   spawn PTY child → NEXT_ID → PtySession map (same as local_pty_spawn) → return pty id
  │   on pty exit (frontend observes pty://exit): backend unlinks inv+cfg (ephemeral cleanup, INV-29)
  ▼
frontend: parse PLAY RECAP off pty://output → "N hosts will change, M tasks changed" (FR-ANS8)
  │   on check success for this playbook+target → ENABLE Run-real (FR-ANS9 gate)
  ▼
frontend: Run-real → confirm dialog (FR-ANS10) → invoke ansible_run(req{ mode:"real", … })
  │   backend: identical path, build_ansible_argv(check=false) = argv WITHOUT --check/--diff
  ▼
frontend: on pty://exit → invoke ansible_history({op:"append", entry:{…}})  (FR-ANS14)
```

### 14.3 Inventory + cfg generation (pure core)

- Source is READ-ONLY: the hosterm host list already rendered in the sidebar (same query Fleet Run
  uses; wildcard `*`/`?` hosts excluded). hosterm **never** writes `~/.ssh/config`.
- **Group derivation** = alias prefix before the first `-` (`prod-web` → group `prod`), the exact
  derivation Fleet Run FR-FL2 already uses. Never emits an ssh `Group` directive (real ssh fails hard
  on it) — groups exist only inside the generated inventory file.
- **Format = INI** (OQ-ANS3 resolved to INI; architect decision — INI is the terser alias→vars map and
  Ansible's native default; YAML would add indentation fragility with no benefit for a flat host map).
- Per eligible host, one line:
  `<alias> ansible_host=<HostName> ansible_user=<User> ansible_port=<Port> ansible_ssh_private_key_file=<IdentityFile>`
  Fields absent in config are omitted (Ansible applies its own defaults). **Password-auth hosts are
  skipped** — never written to the inventory (ADR-A2/INV-31).
- Both inventory and cfg are written to `dirs::config_dir()/hosterm/` with a random suffix, `0600`,
  and **unlinked when the run's PTY exits or is stopped** (INV-29). Zero new persistent files under
  `~/.ssh`. App-data store is isolated from config + sidecar (INV-22 family).

### 14.4 Frozen IPC contract (lead's TSD implements verbatim; frontend mocks verbatim)

Conventions identical to §12.3/§13.5: command args camelCase at the Tauri boundary, JSON payload
fields snake_case. **IPC inventory 40 → 43** (see §14.8 for the count-of-record reconciliation). This
block is the freeze — a shape change here is an addendum revision, never implementation drift.

```ts
// 1. Binary presence — read-only, 0 spawn, 0 network (FR-ANS1/ANS2; ADR-A3)
invoke("check_ansible"): Promise<AnsibleCheck>
interface AnsibleCheck {
  missing: boolean;          // true ⇒ ansible-playbook not resolvable in PATH; frontend shows guide + blocks run
}

// 2. Spawn a run (check or real) — returns a PTY id into the EXISTING pty session map (FR-ANS6..12)
invoke("ansible_run", { req: AnsibleRunReq }): Promise<number>   // pty id; errors as Err("ansible:<code>")
interface AnsibleRunReq {
  mode: "check" | "real";    // check = --check --diff (dry-run); real = no --check
  playbook: string;          // absolute path to the picked *.yml/*.yaml (validated UTF-8 ≤2 MiB by FRONTEND before call)
  aliases: string[];         // picker selection; backend REPLANS authoritatively via host_eligibility_ansible
  limit: string;             // the --limit targets expression (one argv element; alias/group names, comma-joined)
  cols: number;              // PTY geometry, like local_pty_spawn
  rows: number;
}
// Backend behavior (frozen):
//   • host_eligibility_ansible(aliases): key/agent-auth → eligible; password-auth or unsafe alias → skipped (NOT spawned)
//   • if 0 eligible hosts → Err("ansible:no-eligible-hosts"), 0 spawn, 0 files written
//   • generates ephemeral inventory + cfg (0600, data_dir()), builds argv as a Vec<String> (INV-28), spawns PTY
//   • env on spawn: ANSIBLE_HOST_KEY_CHECKING=True, ANSIBLE_CONFIG=<ephemeral cfg>  (ADR-A1)
//   • NO password/secret ever enters argv, env, or this payload (ADR-A2 / INV-31)
//   • ephemeral files are unlinked when this pty's pty://exit fires or pty_kill is called (INV-29)
//   • output/exit ride the EXISTING pty://output/{id} + pty://exit/{id}; stop = existing pty_kill(id)

// 3. History — app-data JSON, atomic 0600, corrupt→empty, 200 FIFO (FR-ANS14/ANS15)
invoke("ansible_history", { op: AnsibleHistoryOp }): Promise<AnsibleHistoryEntry[]>
type AnsibleHistoryOp =
  | { kind: "read" }                               // → full array (newest-last), [] if missing/corrupt (INV-24 family)
  | { kind: "append"; entry: AnsibleHistoryEntry } // → array after append + FIFO-cap at 200; atomic whole-file write
interface AnsibleHistoryEntry {
  playbook: string;          // path (basename shown in UI; full path stored)
  targets: string;           // the --limit expression as run
  mode: "check" | "real";
  started_at: string;        // ISO-8601 UTC
  duration_ms: number;
  rc: number;                // ansible-playbook exit code
  recap: string;             // the trimmed PLAY RECAP block text ONLY — NEVER full task output, NEVER any secret
}
```

**No new events, no streaming command.** The run's bytes ride the existing `pty://` family (§14.2).
`ansible_run` is the only new command that spawns; `check_ansible` and `ansible_history` are
request/response. Error strings are opaque `ansible:<code>` tokens the frontend maps to copy (same
posture as Fleet's `fleet:<code>`); the backend never embeds host/path data an attacker controls into
an error string.

### 14.5 Dry-run-WAJIB gate (frontend state machine — FR-ANS9/ANS10)

```
state: { playbookPath, targets, checkedKey: string|null }
  checkedKey = `${playbookPath}\u0000${targets}`  // identity of the last SUCCESSFUL dry-run
Run-real button ENABLED  ⇔  checkedKey === current(`${playbookPath}\u0000${targets}`)
  • change playbook OR targets → checkedKey := null → Run-real DISABLED (gate reset)
  • dry-run (mode:"check") pty exits rc==0 → checkedKey := current key → Run-real ENABLED
  • Run-real always requires an explicit confirm dialog first (FR-ANS10); never auto-runs (INV-25 sibling)
```

This is the frontend mirror of INV-25's approval posture: the permission-widening action (a real,
mutating run) is gated behind a cheaper, safe action (dry-run) plus an explicit human confirm, and is
never the default/primary auto-path. It is a security layer, not just UX (spec §4).

### 14.6 Data model (new entities)

No DB. One new app-data file + two transient files per run:

| Entity | Location | Lifetime | Shape |
|---|---|---|---|
| `ansible-history.json` | `dirs::config_dir()/hosterm/` | persistent, app-data (INV-22) | `AnsibleHistoryEntry[]`, 200-cap FIFO, atomic 0600 write, corrupt→`[]` (INV-24) |
| ephemeral inventory | `dirs::config_dir()/hosterm/ansible-inv-<rand>.ini` | per-run, 0600, unlinked on pty exit/stop | INI: one `<alias> ansible_host=… ansible_user=… ansible_port=… ansible_ssh_private_key_file=…` line per eligible host; `[group]` headers from alias-prefix derivation |
| ephemeral cfg | `dirs::config_dir()/hosterm/ansible-cfg-<rand>.ini` | per-run, 0600, unlinked on pty exit/stop | pinned-defaults `ansible.cfg` (host_key_checking via env, nocolor optional) — pins a safe cfg so a stray `ansible.cfg` in cwd is never honored |

All three live in app-data, **isolated from `~/.ssh/config` and the secrets sidecar** (INV-29). The
history store never reads or writes config/sidecar (INV-30, a sibling of INV-22). Secrets are never
serialized into any of them (INV-31).

### 14.7 ADRs (frozen in spec §7; stated formally here)

**ADR-A1 — Host key checking stays ON (fail-closed).**
*Decision:* every Ansible spawn pins `ANSIBLE_HOST_KEY_CHECKING=True` and `ANSIBLE_CONFIG=<ephemeral
cfg>`. An unknown host key makes that host's task fail with a directive message (open one terminal tab
to the host once for TOFU), never an auto-accept.
*Rationale:* hosterm spawns the user's real `ssh` against the user's own `known_hosts`; silently
accepting new keys would weaken the posture for a blast-radius-wide tool. Consistent with Fleet Run's
`BatchMode=yes` fail-closed stance. Pinning `ANSIBLE_CONFIG` also stops a hostile `ansible.cfg` in the
user's cwd from being honored.
*Alternatives rejected:* `False` (auto-accept) — trades a one-time TOFU friction for a persistent MITM
exposure; rejected as a security regression.

**ADR-A2 — Credentials: key/agent hosts only in v1; secrets never leave the sidecar.**
*Decision:* for key/agent-auth hosts the ephemeral inventory carries
`ansible_ssh_private_key_file=<IdentityFile path>` — the **path already in the user's ssh config**, not
the key bytes. Password-auth hosts are `skipped` with an explicit reason and are never added to the
inventory. No password/secret ever becomes an argv element, `--extra-vars`, an env var visible in `ps`,
or an IPC payload field.
*Rationale:* argv is world-readable via `ps`; `--extra-vars` and env leak the same way. hosterm's
zero-knowledge posture keeps passwords only in the 0600 sidecar, never crossing IPC — the same decision
Fleet Run FR-FL20/21 made. v1 shipping key/agent-only matches Fleet Run v1 exactly, so it is not a
surprise regression.
*Deferred to wave-2 (architect spike owed):* password-host Ansible runs need a proven no-expose feed
(`sshpass` via fd, Ansible Vault, or prompt-via-PTY) before they can ship.

**ADR-A3 — Binary-check lifecycle: a separate `check_ansible` command.**
*Decision:* detection is a new command `check_ansible` called when the Ansible panel opens, NOT an
extension of launch-time `check_binaries`.
*Rationale:* `ansible-playbook` is OPTIONAL (the user installs it themselves), so it has a different
lifecycle from the mandatory ssh/sftp binaries. Folding it into `check_binaries` would couple an
optional tool's absence to app launch and muddy the W2-A guide flow. A separate command keeps the
"check when you open the panel" semantics clean and read-only (0 spawn, mirrors `binaries_missing`).

### 14.8 Impact on the verify loop + invariants

**New named invariants (gate in STATE.md + smoke/cargo):**

| ID | Property | Gate |
|---|---|---|
| INV-28 | The Ansible run path builds its `ansible-playbook` argv as a direct array (`Vec<String>` → `CommandBuilder`); zero `sh -c`/`bash -c`/`cmd /C`. Playbook path and `--limit` targets are each a single argv element, never interpolated into a shell string (sibling of INV-14/INV-27). | cargo argv-builder test (array shape, marker-file fixture) + grep `sh -c\|bash -c\|cmd /C` = 0 on the Ansible path (NFR-ANS5) |
| INV-29 | An Ansible run writes ONLY to app-data (`dirs::config_dir()/hosterm/`); it never reads or writes `~/.ssh/config` or the secrets sidecar, and its ephemeral inventory+cfg are deleted when the run's PTY exits or is stopped. Zero new persistent files under `~/.ssh`. | cargo byte-identity test (config+sidecar hash+mtime identical before/after, NFR-ANS4) + ephemeral-cleanup test |
| INV-30 | `ansible-history.json` is app-data-only, atomic whole-file 0600 write, 200-entry FIFO; a missing/corrupt file returns `[]` and the corrupt bytes stay untouched until the next legitimate write (INV-22/INV-24 family). | cargo: atomic write, FIFO cap, corrupt→`[]`-no-clobber |
| INV-31 | No secret byte ever enters the Ansible path's argv, env, inventory file, history entry, or IPC payload. Password-auth hosts are `skipped` (never spawned, never in inventory); only `IdentityFile` **paths** are referenced. | cargo: inventory-generate test (password host skipped, no key bytes) + payload-shape review (no secret field) |

**ADR map:** ADR-A1 (host key checking ON), ADR-A2 (credentials key/agent-only v1), ADR-A3
(`check_ansible` lifecycle) — all §14.7.

**IPC inventory gate (count of record).** The canonical gate is
`grep -rc "tauri::command" src-tauri/src/*.rs`. At HEAD it sums to **40** (37 lib.rs + 2 fleet.rs + 1
false-positive: a comment in `backup.rs:8` that contains the literal `#[tauri::command]` inside prose —
the grep counts it; the actual `generate_handler!` list registers **39** real commands). The three new
Ansible commands land in `lib.rs`, so the gate moves **40 → 43** (lib.rs 37 → 40; fleet.rs 2 unchanged;
backup.rs comment unchanged) and the registered-command count moves **39 → 42**. §11 self-check updates
to 43. *Lead action owed:* the stale `backup.rs:8` comment inflates the gate by 1 — the lead MAY
neutralize it (reword the comment so it no longer matches) in the same wave to make gate == registered,
but that is a hygiene nicety, not a blocker; document whichever number the gate actually emits.

**Carried-forward checks that must stay green:** ADR-8 hostile-render (PLAY RECAP + task output through
`esc()`/`escAttr()`, CSP counter == 0 with hostile fixtures), no-background-network (NFR-ANS3 — the only
new outbound traffic is the ssh that `ansible-playbook` itself triggers, user-initiated), `draggable`
grep stays 0 (picker reuses fleet's manual-mouse pattern, not HTML5 DnD).

### 14.9 Hotspot + recommended unit sequence (RECOMMENDATION — final granularity = lead's TSD)

**Hotspot (binding):** `src/main.ts` and `index.html` are the Ansible frontend hotspot (panel + picker +
dialogs + history). The backend unit (three new commands + pure cores in `lib.rs`) is a HOTSPOT with
every other `lib.rs` card — **`lib.rs` takes one active card at a time, never parallel** (standing repo
rule). The backend unit is file-disjoint from the frontend hotspot and MAY run parallel to an early
frontend unit as long as no other `lib.rs` card is mid-flight.

> `hotspot: src-tauri/src/lib.rs — Ansible backend (3 commands + pure cores) collides with every other lib.rs card; serialize, one active lib.rs card.`
> `hotspot: src/main.ts + index.html — Ansible frontend panel/picker/dialogs; serialize per repo rule.`

Recommended sequence (lead finalizes):
1. **Backend: `check_ansible` + `ansible_run` + `ansible_history`** — pure cores (`build_ansible_argv`,
   `build_inventory_ini`, `host_eligibility_ansible`, `ansible_missing`, `history_append_fifo`) with
   full cargo coverage (INV-28..31). Frozen to §14.4. File-disjoint; can start first.
2. **Frontend panel shell + binary-missing guide** — W2-A pattern, mocks `check_ansible`. (main.ts/index.html)
3. **Playbook picker + host/group picker** — reuse fleet picker; password rows skipped. (main.ts)
4. **Dry-run → recap + the WAJIB gate state machine** (§14.5) — depends on #1 (ansible_run) + #2/#3.
5. **Run-real confirm dialog + PTY terminal tab** (reuse openTerminal local) — depends on #4.
6. **History sub-panel** — depends on #1 (`ansible_history`). **Last** (read + render, lowest risk).

### 14.10 Open items for the lead (unit boundaries only — NO architectural decision left open)

Every architectural decision is LOCKED above. These are purely implementation-granularity choices the
lead settles in the TSD — none change the contract (§14.4), the invariants (§14.8), or the ADRs (§14.7):

- **LQ-1 (unit split of the backend command):** whether `ansible_run`'s inventory/cfg generation lives
  in `lib.rs` directly or a small `mod ansible` (mirroring how Fleet got its own `fleet.rs`). Architect
  note: if the backend body grows past ~150 LOC, a `mod ansible` keeps `lib.rs` diff thin and shrinks
  the hotspot collision surface (same reasoning as §12.2). **Decision is the lead's** — the command
  names and payloads (§14.4) are identical either way.
- **LQ-2 (ephemeral cleanup trigger wiring):** where the inventory/cfg unlink hooks onto the PTY exit —
  backend-side in the pty exit path, or a frontend `invoke` on `pty://exit`. Architect recommendation:
  backend-side (register the temp paths against the pty id, unlink in the same place `pty_kill`/exit
  clears the session) so cleanup is not contingent on the frontend being alive. INV-29 holds either way;
  lead picks the wiring.
- **LQ-3 (`ANSIBLE_NOCOLOR`):** whether to also pin `ANSIBLE_NOCOLOR=1` for cleaner `PLAY RECAP`
  parsing. Recommendation: yes (frontend parse is simpler, no ANSI strip). Trivial; lead's call.

### 14.11 Wave-2 parking lot (NOT designed — carried from spec §10)

1. Auto-install Ansible from inside the app (pipx/apt/brew on Unix, WSL on Windows) — **DESIGNED, see §15**.
2. Playbook from a private Git repo (adds an OAuth/token surface).
3. Password-host Ansible runs (the no-expose secret feed — architect spike owed, ADR-A2).
4. Ad-hoc module runs, Vault editing from the UI, rolling/serial with a failure budget.

## 15. Ansible auto-install v1 (addendum — installer pipeline, WSL invocation, frozen IPC contract)

> Architect addendum (card t_04709068, 12 Nov 2026), derived FROM
> `docs/FEATURE-SPEC-ansible-autoinstall-v1.md` (spec commit `4b87a97`; OQ-AI2 lock commit
> `b22fdc5`, MEKL-approved 2026-11-12). **Design-first lane:** the spec's scope lock (§0), its
> LOCKED Clarifying-Question answers (§1), its ADR-AI1..AI4 (§7) and its NFR-AI1..AI8 (§9) are
> CLOSED — this section designs the mechanics only and re-opens no product decision. **OQ-AI2 is
> CLOSED** (root-in-WSL for the in-distro install, approved 12 Nov 2026) and is carried as a fact.
> This is an addendum to §14 (Ansible v1), not a replacement: §14's IPC inventory, invariants and
> ADRs stay in force; §15 extends them. Downstream consumer: lead's TSD.
>
> **Numbering note.** The brief asked for a subsection under §14. This document's own convention
> for a feature addendum is a top-level `##` section (§12 Fleet Run, §13 Dusk); auto-install ships
> its own ADR range (ADR-AI1..AI7), invariant range (INV-32..35) and NFR set, so it lands as the
> sibling section **§15**, with a cross-reference from §14.11. Nothing else in §14 is edited.
>
> **Relationship to `docs/tsd/ansible-v1.md`.** That TSD builds §14 (the runner). §15.6 changes
> `ansible_run`'s *internals* (launcher resolution, cwd, path translation) without changing its IPC
> payload — the lead carries those deltas into the runner TSD's §4 contract reproduction, and writes
> the auto-install TSD from §15.

### 15.1 Constraints honored (trace)

| Constraint | Source | Honored at |
|---|---|---|
| No new Rust/JS dependency | NFR-AI1 | §15.4/§15.5 use std + the existing `portable-pty`/`tauri` stack; no `Cargo.toml`/`package.json` change |
| No bundled Python/Ansible | NFR-AI1 | install spawns the OS package manager only (`pipx`/`brew`/`apt`); nothing is shipped |
| pipx/brew are the only one-click installers | ADR-AI1 | `resolve_install_plan()` (§15.4) |
| Install only on explicit user click | FR-AI9 | §15.7 — `ansible_install` is invoked from a click handler only; never on panel open or launch |
| No root escalation from the app | §0 / ADR-AI3 | root only inside the WSL distro via `wsl --user root`; the host process stays the user |
| No shell interpreter | INV-14 | every spawn is an argv array; `wsl --exec` bypasses the distro shell |
| No secret in any payload | INV-28/29/31 | install/probe requests carry no host or secret; the install env adds nothing |
| `HOSTERM_ANSIBLE_BIN` mirrors `HOSTERM_SFTP_BIN` | §2.2.3 | §15.6 launcher resolution |
| Panel-open checks stay read-only, 0 spawn | NFR-AI2 | §15.3 — `check_ansible` keeps its 0-spawn guarantee; probe dirs are static |
| Console renders via `textContent` | ADR-8 | §15.7 — installer output is untrusted bytes |

### 15.2 Architecture

Two units, one shared mechanism.

- **Unit A — one-click installer pipeline (FR-AI1..AI9).** A pure plan resolver picks the installer
  (`pipx` → `python3 -m pipx` on Linux, `brew` on macOS, `apt` inside WSL on Windows). A PTY session
  runs the plan's command ladder, streams output over the existing `pty://output|exit/{id}` family,
  and is killed by the existing `pty_kill`.
- **Unit B — Windows WSL invocation + path translation (FR-AI10..AI14).** A pure translator
  (`win_to_wsl_path`) plus the platform translator (`wslpath -a -u`) convert every path element the
  Linux child sees. `ansible_run` resolves the launcher and sets the child cwd.
- **Shared mechanism — the existing PTY engine.** Both units reuse `PtyState`/`NEXT_ID`/`pump_pty`/
  `pty://output|exit/{id}`/`pty_kill`. No new IPC channel, no new state map, no new dependency.

```
          FRONTEND (src/main.ts)                     BACKEND (src-tauri/src/lib.rs)
 ───────────────────────────────────────   ──────────────────────────────────────────────────────
 #ansible-modal / #ansible-guide          PURE CORES (no I/O · no spawn · cargo --lib)
   ansState guide machine (§15.7)           ansible_probe_dirs()    resolve_install_plan()
   #ans-install-log (pre, read-only)        win_to_wsl_path()       build_wsl_argv()
   Install · Re-check · Run               RESOLVER (spawns only in the WSL branch)
                                              resolve_ansible_launcher() (§15.6)
 ───────────────────────────────────────   ──────────────────────────────────────────────────────
        │             │            │               │                │                 │
        ▼             ▼            ▼               ▼                ▼                 ▼
  check_ansible   check_wsl   ansible_wsl_      (internal)      ansible_install    ansible_run
  (0 spawn)       (1 spawn)   probe          resolve_ansible_   (1 session id)     (§15.6 delta)
        │             │       (1 spawn,      launcher()             │                  │
        │             │        user-only)      │                    ▼                  ▼
        └─────────────┴────────┘                │            PtyState.sessions[id]  PtyState.sessions[id]
                                              │            pump_pty / pty://…     pump_pty / pty://…
                                              │            pty_kill(id)           pty_kill(id)
                                              ▼
                              WSL mode → translate every path element (§15.5)
                              native / HOSTERM_ANSIBLE_BIN → no translation
```

**What is new vs reused:**

| Piece | Status | Note |
|---|---|---|
| `resolve_install_plan()` | NEW pure core | no I/O, unit-testable on Linux CI |
| `win_to_wsl_path()` | NEW pure core | no I/O, unit-testable on Linux CI |
| `translate_path_to_wsl()` | NEW thin wrapper | one `wslpath` subprocess + the pure fallback |
| `resolve_ansible_launcher()` | NEW pure-ish core | reads env + OS; calls `check_wsl`/probe helpers only in the WSL branch |
| `ansible_install` command | NEW | one session id, ladder in an orchestration thread |
| `check_wsl` / `ansible_wsl_probe` | NEW | piped one-shot spawns, fail-closed |
| `check_ansible` | EXTENDED | additive `os` + `probe_dirs`; `missing` semantics unchanged |
| `PtyState` / `NEXT_ID` / `pump_pty` / `pty_kill` | REUSED | untouched; `pump_pty` carries the `7621b72` UTF-8 fix |
| `PtyState.ephemeral` | REUSED, unchanged | stays exclusive to `ansible_run` (§15.4) |
| `pty://output\|exit/{id}` | REUSED | no new event family |

### 15.3 Frozen IPC contract

Style follows §14.4 exactly: camelCase at the Tauri boundary, opaque error tokens, no secret in any
payload. All four shapes below are **additive** — nothing in §14.4 changes.

| Command | Request | Response | Errors | Spawn budget |
|---|---|---|---|---|
| `ansible_install` | `{ distro?: string }` | `number` (bare pty id) | `ansible-install:<code>` | 1 PTY session; N spawns = ladder length |
| `check_wsl` | `{}` | `{ installed: boolean, distro: string \| null }` | none (classification) | exactly 1; 0 on non-Windows |
| `ansible_wsl_probe` | `{ distro: string }` | `{ present: boolean }` | `ansible-wsl:<code>` | exactly 1 |
| `check_ansible` | `{}` | `{ missing: boolean, os: "linux"\|"macos"\|"windows", probe_dirs: string[] }` | none | 0 |

#### 15.3.1 `ansible_install`

```ts
interface AnsibleInstallReq { distro?: string }   // required on the Windows branch only
type AnsibleInstallRes = number;                  // bare pty id — same shape as ansible_run
```

`distro` is omitted on Linux/macOS — the request stays platform-agnostic and the backend derives
the platform itself (this is what removes the UA sniffing the spec's D-AI11 dropped). The spec locks
the return type (`Result<u32, String>`, §2.1) and the error prefix; the request shape is the
architect's minimal additive choice — one optional field, no UA, no platform enum. The id is
allocated **before** any spawn so the console can render immediately; the caller never polls.

Error vocabulary — opaque `ansible-install:<code>` tokens, **never** host/path/stdout data:

| Token | Meaning | Side effects |
|---|---|---|
| `ansible-install:no-installer` | Linux: `pipx` and `python3` both unresolvable. macOS: `brew` unresolvable in PATH+probe dirs and at neither fixed path. | 0 spawn, no PTY allocated |
| `ansible-install:wsl-unavailable` | Windows branch with no usable distro (absent/empty/unsafe) | 0 spawn, no PTY allocated |
| `ansible-install:bad-distro` | `distro` fails the argv guard (INV-33) | 0 spawn, no PTY allocated |
| `ansible-install:pty-failed` | `openpty` / reader / writer failure | PTY freed, no leak |
| `ansible-install:spawn-failed` | installer child spawn failure | PTY freed, no leak |

A non-zero installer exit is **not** an error token — it is streamed to the console and surfaced by
the post-install re-check (UC-AI3 manual fallback).

#### 15.3.2 `check_wsl`

```ts
type WslCheck = { installed: boolean; distro: string | null };
```

Behavior matrix (fail-closed):

| Condition | Result |
|---|---|
| non-Windows host | `{ installed: false, distro: null }` — **no spawn**, no Err |
| `wsl.exe --list --quiet` spawn failure, rc≠0, or deadline exceeded | `{ installed: false, distro: null }` |
| rc=0, stdout non-empty | `{ installed: true, distro: <first non-empty line, trimmed> }` |
| rc=0, stdout empty/whitespace | `{ installed: true, distro: null }` |

This command **never returns `Err`** — every failure is a classification, so the frontend's `catch`
path cannot be triggered by a WSL problem. Deadline: 500 ms (NFR-AI3), enforced by polling
`Child::try_wait` every 10 ms (the ADR-14 pattern) and killing the child on expiry.

#### 15.3.3 `ansible_wsl_probe`

```ts
interface AnsibleWslProbeReq { distro: string }
interface AnsibleWslProbeRes { present: boolean }
```

Spawn `wsl.exe --distribution <distro> --exec ansible-playbook --version` (argv array, no shell,
stdout/stderr piped). rc=0 → `present:true`; rc≠0, spawn failure, or deadline → `present:false`.
Deadline: 5 s (NFR-AI3). **User-triggered only** — never called on panel open (§15.7).

Error vocabulary: `Err("ansible-wsl:bad-distro")` when `distro` fails the argv guard (INV-33). That
token is reachable only from a frontend bug (the distro value comes from `check_wsl`), so the
frontend maps it to a generic "could not probe the WSL distro" line and never surfaces the raw token.

#### 15.3.4 `check_ansible` (extended, backward-compatible)

```ts
type AnsibleOs = "linux" | "macos" | "windows";
interface AnsibleCheck {
  missing: boolean;      // unchanged: not resolvable in PATH + probe dirs
  os: AnsibleOs;         // NEW — backend-derived, replaces UA sniffing (D-AI11)
  probe_dirs: string[];  // NEW — additional dirs the resolver consulted
}
```

- Still **read-only, 0 spawn, no network, no side effects** (NFR-AI2). The existing frontend code
  that reads only `.missing` keeps working.
- `os` = `std::env::consts::OS` mapped: `"windows"` → `windows`, `"macos"` → `macos`, anything else
  → `linux` (the Unix default; the frontend treats it as the pipx branch).
- `probe_dirs` = `ansible_probe_dirs()` deduped against PATH entries and filtered to directories
  that exist. Empty when `HOSTERM_ANSIBLE_BIN` is set (the override replaces the lookup, §15.6).
- `missing` keeps its exact v1 meaning — the resolver now also consults `probe_dirs` (D-AI8), so a
  binary in `~/.local/bin` is found on a GUI app whose PATH lacks it.

**Error-token summary (frozen):**

| Command | Prefix | Tokens |
|---|---|---|
| `ansible_install` | `ansible-install:` | `no-installer`, `wsl-unavailable`, `bad-distro`, `pty-failed`, `spawn-failed` |
| `check_wsl` | — | none (classification only) |
| `ansible_wsl_probe` | `ansible-wsl:` | `bad-distro` |
| `check_ansible` | — | none |
| existing `ansible:*` (unchanged) | `ansible:` | `no-eligible-hosts`, `config-unreadable`, `inv-write-failed`, `cfg-write-failed`, `pty-failed`, `spawn-failed`, **`path-untranslatable`** (§15.5) |

The frontend's `ansErrText()` gains a second lookup for `ansible-install:`; `ansible-wsl:` maps to a
generic line. Raw tokens are never rendered.

**IPC inventory:** 45 registered commands at the time of writing (lib.rs 43 + fleet.rs 2; measured
`grep -rc "tauri::command" src-tauri/src/*.rs` == `generate_handler!` entries) → **48** after this
addendum. §14.8's "43 gate / 42 registered" and `docs/STATE.md`'s "42" are the BE-ANS1..3 baseline;
the managed-key edit wave (`read_identity_file`, `rename_identity_file`, `write_identity_file`) added
3 commands after that baseline. The lead re-measures at dispatch.

### 15.4 Install-plan resolution + install session orchestration

#### 15.4.1 Pure core

```rust
enum InstallPlan {
    Pipx       { bin: String },   // argv[0] = resolved pipx path
    PipxModule { bin: String },   // python3 -m pipx
    Brew       { bin: String },
    WslApt     { distro: String },
}
enum InstallError { NoInstaller, WslUnavailable }

fn resolve_install_plan(
    os: AnsibleOs,
    distro: Option<&str>,
    path_dirs: &[PathBuf],
    probe_dirs: &[PathBuf],
) -> Result<InstallPlan, InstallError>
```

Exact resolution order:

- **linux** — `find_in_dirs("pipx", PATH+probe)` → `Pipx`; else `find_in_dirs("python3", PATH+probe)`
  → `PipxModule`; else `NoInstaller`.
- **macos** — `find_in_dirs("brew", PATH+probe)` → `Brew`; else `/opt/homebrew/bin/brew` is a file →
  `Brew`; else `/usr/local/bin/brew` is a file → `Brew`; else `NoInstaller`.
- **windows** — `distro` is `Some` **and** passes `sftp_safe(distro, "Distro")` → `WslApt`; else
  `WslUnavailable`.

`probe_dirs` MUST be in the search path: pipx installs itself into `~/.local/bin`, which a GUI
app's PATH frequently lacks — this is exactly why D-AI8 added probe dirs. The two fixed brew paths
are **not** redundant with that list: D-AI8's macOS probe dirs cover `/opt/homebrew/bin` (Apple
Silicon) and `~/.local/bin`, but Intel Macs keep brew in `/usr/local/bin`, which the probe list
omits — the explicit check closes that gap. `find_in_dirs` is the existing helper (`name` + `.exe`
on Windows).

#### 15.4.2 Command ladder (frozen argv)

| Plan | argv |
|---|---|
| `Pipx` | `[bin, "install", "--include-deps", "ansible"]` |
| `PipxModule` | `[bin, "-m", "pipx", "install", "--include-deps", "ansible"]` |
| `Brew` | `[bin, "install", "ansible"]` |
| `WslApt` | 1. `["wsl.exe","--distribution",distro,"--user","root","--exec","apt-get","update"]` <br> 2. `["wsl.exe","--distribution",distro,"--user","root","--exec","apt-get","install","-y","ansible"]` |

- Rung 2 runs only when rung 1 exits 0. A non-zero rung 1 stops the ladder; the console shows the
  native output and the post-install re-check decides the next state (UC-AI3 manual fallback).
- `apt-get` (not `apt`) is deliberate: script-friendly, never opens a debconf TUI. The lead MAY also
  set `DEBIAN_FRONTEND=noninteractive` in the child env (an env detail, **not** contract) so a package
  hook cannot block on a prompt.
- Install-spawn env = the parent env, unchanged, plus nothing. No secret, host, or path from
  `~/.ssh` enters the environment (INV-31 extension).

#### 15.4.3 Session orchestration (ADR-AI6)

One `ansible_install` call = one PTY session id = one console. The ladder runs in an orchestration
thread so INV-14 is honored without `sh -c "a && b"`.

1. `id = NEXT_ID.fetch_add(1, SeqCst)`; allocated **before** any spawn.
2. Spawn the orchestration thread; return the bare `id` immediately (the console shows
   "Installing…" from that moment).
3. Per rung: `openpty` → spawn with the frozen argv → `try_clone_reader` / `take_writer` →
   **replace** `sessions[id]` with the new `PtySession` → spawn the pump thread → wait for pump
   completion, polling `sessions.contains_key(id)` every 50 ms.
4. **Cancellation:** if the entry vanished, `child.kill()` + `child.wait()`, stop the ladder.
   `pty_kill` is the only other remover.
5. **Ladder stop:** a non-zero rung rc ends the ladder (last rc wins).
6. **End:** `sessions.remove(id)` (idempotent with `pty_kill`), `cleanup_ephemeral(ephemeral, id)`
   (no-op — install registers nothing), emit `pty://exit/{id}` **exactly once**.

**Why one id for two rungs (ADR-AI6):** INV-14 forbids `sh -c "a && b"`, so the two apt spawns must
be separate processes; a single session id keeps one console, one exit event, one kill handle and
one slot in the ring buffer. The frontend never sees a rung boundary.

**UTF-8 carry:** the pump thread calls the existing `pump_pty`, which carries incomplete trailing
bytes via `decode_utf8_stream` (commit `7621b72`). Installer output therefore gets the same carry as
Fleet Run and TUIs — no new decoder, and no second implementation to drift.

**Ephemeral registration:** install registers **zero** entries in `PtyState.ephemeral`; the map stays
exclusive to `ansible_run`. `cleanup_ephemeral` is still called on the install exit path so any future
install artifact is cleaned by the same idempotent helper.

**Residual risk (documented, not solved here):** a child that ignores SIGHUP keeps its PTY open; the
50 ms poll bounds how fast the kill is noticed and `child.kill()` then force-terminates it. This is
strictly better than the existing `pty_kill` posture for `ssh` (§12.7) and adds no machinery.

### 15.5 Path translation mechanics

Two layers: a pure Rust core and a platform-translator wrapper.

#### 15.5.1 Pure core — `win_to_wsl_path()`

```rust
/// Translate a Windows-form path to the WSL `/mnt/<drive>/...` form.
/// `None` = not translatable (UNC, drive-relative, relative, empty, NUL) —
/// the caller fails the run closed (INV-34).
fn win_to_wsl_path(path: &str) -> Option<String>
```

Exact rules, in order:

1. Already-Unix (`starts_with('/')`) → return verbatim (passthrough, NFR-AI8).
2. `\` → `/`; collapse repeated separators.
3. Drive-letter absolute (`C:\...`, `C:/...`) → `/mnt/<lowercased letter>/<rest>`; a trailing slash is
   trimmed except at the drive root (`C:\` → `/mnt/c`).
4. Everything else → `None`: UNC (`\\server\share`), drive-relative (`C:foo`), bare relative
   (`foo\bar`), empty, or containing NUL.

#### 15.5.2 Platform translator — `wslpath -a -u`

```rust
/// The only outward failure: the path is not translatable. A `wslpath`
/// absence/failure is handled INSIDE (pure-Rust fallback), never returned.
enum TranslateError { Untranslatable }
fn translate_path_to_wsl(path: &str) -> Result<String, TranslateError>
```

1. Spawn `wslpath.exe` with argv `["wslpath.exe", "-a", "-u", path]` — one subprocess per path, argv
   array, no shell, stdout piped, 2 s deadline (poll `try_wait` every 10 ms; `kill()` on expiry).
2. rc=0 **and** trimmed stdout is non-empty **and** starts with `/` → that value.
3. Otherwise (absent / rc≠0 / deadline / empty output) → `win_to_wsl_path(path)`; `Some` → value,
   `None` → `Err(Untranslatable)`.
4. **Never** fall back to the raw Windows path (INV-34).

`wslpath -a -u` is the platform's own translator and stays correct across WSL versions (trailing
slashes, `//`, UNC-adjacent edge cases); the pure-Rust layer keeps the logic unit-testable on Linux CI
and covers a missing or failing `wslpath` without weakening the fail-closed rule.

**When `wslpath` is unavailable:** the pure-Rust fallback runs. A translatable path (drive-letter
absolute) proceeds; an untranslatable one (UNC, drive-relative, relative) refuses the run with
`ansible:path-untranslatable` and **no spawn** occurs. The run is never attempted with an untranslated
path.

#### 15.5.3 Every translated element + ordering (spec §2.2.3)

| # | Element | Written at | Child receives | Notes |
|---|---|---|---|---|
| 1 | every `IdentityFile` (after `~` expansion to a real Windows path) | inside the ephemeral inventory body | translated `/mnt/...` value | **must precede the inventory write** — the Linux child reads it at parse time |
| 2 | ephemeral inventory path | `data_dir()/ansible-inv-<id>.ini` (Windows path, 0600) | translated `/mnt/...` value as `-i` | the body already holds translated IdentityFile values |
| 3 | ephemeral cfg path | `data_dir()/ansible-cfg-<id>.ini` (Windows path, 0600) | translated `/mnt/...` value as `ANSIBLE_CONFIG` | |
| 4 | playbook path | — (user-supplied) | translated `/mnt/...` value as the positional arg | |
| 5 | `ANSIBLE_CONFIG` env value | — | the translated cfg path | same value as #3 — no extra spawn |

The translation pass runs on the eligible-host list first, then the inventory body is built from the
translated values, then the files are written, then argv/env are assembled from the translated paths.
A translation failure aborts **before** any file write or spawn — fail-closed, no partial artifacts.

**`wslpath` spawn count:** `1 (inventory) + 1 (cfg) + 1 (playbook) + N (IdentityFile)` = **N + 3**
subprocesses per WSL run, bounded by the picker size. Do not "optimise" this away without re-reading
ADR-AI5 — one subprocess per path is what keeps each translation independently verifiable.

### 15.6 Launcher resolution + cwd (`ansible_run` delta)

```rust
enum AnsibleLauncher {
    Native(String),          // argv[0] verbatim, NO translation
    Wsl { distro: String },  // wsl.exe --distribution <distro> --exec ansible-playbook <argv…>, ALL paths translated
}

fn resolve_ansible_launcher() -> AnsibleLauncher
```

Exact order:

1. `HOSTERM_ANSIBLE_BIN` set and non-empty → `Native(value)` verbatim. No PATH lookup, **no
   translation** (test hook; ADR-AI7).
2. `cfg!(windows)` **and** `wsl_ansible_ready()` → `Wsl { distro }`.
3. Otherwise → `Native("ansible-playbook")`.

`wsl_ansible_ready()` = `check_wsl()` returns `installed && distro.is_some()` **and**
`ansible_wsl_probe(distro)` returns `present`. Both are the exact helper paths the commands expose —
no hidden third route.

**Cost (accepted):** on Windows this adds at most 2 spawns to a run — `check_wsl` (<500 ms) and, only
when a distro exists, `ansible_wsl_probe` (<5 s). When WSL is absent (the common Windows case) only
the <500 ms query is paid. **Rejected alternative:** a frontend-declared `launcher` field — the
backend must own argv shape, and the launcher choice decides whether paths are translated, so it must
not come from the webview.

**cwd (new).** The child is spawned with `cwd = playbook parent directory` for **every** launcher
mode. `wsl.exe` inherits the Windows cwd and maps it to `/mnt/<drive>/...` inside the distro, so
playbook-relative lookups (`roles/`, `files/`, `collections/`) resolve; the plain `ansible-playbook`
path gets the same cwd so all three modes behave identically. The playbook argument stays absolute
(translated in WSL mode), so cwd only affects relative resolution. This is a behavior change from v1
(which inherited the app's cwd) and is required by FR-AI14. If the playbook path has no parent
(bare filename), fall back to the v1 behavior (no cwd).

**`wsl.exe` argv shape (frozen):**

```
wsl.exe --distribution <distro> --exec ansible-playbook <argv…>
```

`--exec` runs the command directly with no shell (INV-14). `<distro>` passes the `sftp_safe` guard
(INV-33). `<argv…>` is exactly `build_ansible_argv(...)` with translated values.

**Delta summary vs v1 `ansible_run`:**

| Concern | v1 (§14.6) | auto-install addendum (§15.6) |
|---|---|---|
| argv[0] | `"ansible-playbook"` | `resolve_ansible_launcher()` |
| path translation | none | all elements, WSL mode only |
| cwd | inherited from the app | playbook parent (all modes) |
| inventory / cfg | same | same (written at Windows paths, handed over translated) |
| env | `ANSIBLE_HOST_KEY_CHECKING=True`, `ANSIBLE_CONFIG`, `ANSIBLE_NOCOLOR=1` | unchanged; `ANSIBLE_CONFIG` carries the translated path |
| error tokens | `ansible:*` | unchanged, plus `ansible:path-untranslatable` |
| IPC payload | `AnsibleRunReq` | **unchanged** |

### 15.7 Frontend guide state machine + install console + panel close

#### 15.7.1 Guide state machine (explicit)

The spec §2.2.1 flow, spelled out as the machine the frontend implements (its "5 state" is a sketch;
this is the explicit version):

| # | State | Entry condition | Render |
|---|---|---|---|
| 1 | `checking` | panel opened, `check_ansible` in flight | spinner |
| 2 | `native-ready` | `missing:false` | runner body |
| 3 | `native-missing` | `missing:true`, `os != "windows"` | Install (pipx/brew) + manual fallback |
| 4 | `wsl-absent` | `missing:true`, Windows, `check_wsl.installed:false` | WSL-required block, `wsl --install` |
| 5 | `wsl-no-distro` | `installed:true`, `distro:null` | WSL-required block, `wsl --install -d Ubuntu` |
| 6 | `wsl-unknown` | `installed:true`, `distro:Some` — panel open stops here | "Install Ansible inside WSL" + Re-check |
| 7 | `wsl-ansible-missing` | probe `present:false` | Install inside WSL + manual fallback |
| 8 | `wsl-ready` | probe `present:true` | runner body |

#### 15.7.2 Call sequences (frozen)

**Panel open** (`openAnsible` → `ansRefreshCheck`):

1. `check_ansible()` → `missing:false` → state 2. Stop.
2. `missing:true`, `os != "windows"` → state 3. Stop.
3. `missing:true`, `os == "windows"` → `check_wsl()`: `installed:false` → 4; `distro:null` → 5;
   else → 6.

No `ansible_wsl_probe` on panel open (NFR-AI3 — the probe is user-triggered). Panel-open spawns:
**0** (states 2–3) or **1** (states 4–6). Never on app launch.

**Re-check (user click):**

- state 3 → `check_ansible()` again.
- state 6 → `ansible_wsl_probe(distro)` → `present:true` → 8; `present:false` → 7.
- states 4/5 → `check_wsl()` again (the user may have installed WSL since).

**Install click:**

- state 3 → `ansible_install({})` → `ptyId` → console + busy state.
- states 6/7 → `ansible_install({distro})` → same.
- Guard: `ansState.installing` true → ignore the click (NFR-AI7).

#### 15.7.3 Install console + busy state

- `#ans-install-log` is a read-only `<pre>`, auto-scrolling, with **no input field ever** (D-AI6).
- Content is set via `textContent` only (ADR-8) — installer output is untrusted bytes.
- Frontend ring buffer capped at **64 KB** (NFR-AI7), the same retention pattern as the fleet console.
- `#ans-install-status` status line: "Installing…" → "Done" / "Finished with errors".
- Busy state: Install disabled + label "Installing…". **Decision (flagged, not in the spec):** Re-check
  is also disabled during an install — a concurrent read-only check would race the console and could
  show "still missing" while the installer is mid-flight. Low-risk and reversible.

#### 15.7.4 Exit handling (existing handler, extended trigger)

The existing `pty://exit/{id}` listener runs `ansRefreshCheck()` when the id is the install session:

- non-Windows → `check_ansible()` → `missing:false` → state 2.
- Windows + WSL → `check_ansible()` **and** `ansible_wsl_probe(distro)` → `present:true` → state 8;
  otherwise stay in state 7 with the manual fallback visible.

This is D-AI7. The re-check is read-only and safe to run after a user cancel too (the state may have
changed — e.g. rung 1 of 2 completed).

#### 15.7.5 Panel close during install (FR-AI8 / D-AI17)

`#ansible-close` → if `ansState.installing`:

1. in-app confirm ("Cancel the installation? The installer will be stopped.");
2. confirm → `invoke("pty_kill", { id: ansState.installPtyId })`, clear the id, `installing = false`;
3. dismiss → abort the close; the panel stays open and the install continues.

The backend orchestration thread observes the removed session and aborts the ladder (§15.4.3) — no
orphan process, no leaked temp file, no orphaned PTY.

**Out of scope (flagged):** v1's FR-ANS16 (`should`: closing the panel during a *run* → confirm +
Stop) is **not implemented today** — `#ansible-close` calls `closeOverlay` only, and `pty_kill` is
reachable solely from terminal-tab close. This addendum implements the install case (FR-AI8) only.
Lead/nano: decide whether to close the FR-ANS16 gap in the same TSD.

### 15.8 Security invariants

Restated from §14.8 and extended. The mapping is normative; the spec's invariants keep their
original wording.

| Invariant | Source | How §15 honors it |
|---|---|---|
| **INV-14** — no shell interpreter | §14.8 | every install/probe/run spawn is an argv array; `wsl --exec` bypasses the distro shell. The install ladder is two spawns, never `sh -c "a && b"` |
| **INV-28** — argv only, no string interpolation | §14.8 | `distro` and every translated path is its own argv element; `build_wsl_argv` never joins a string |
| **INV-29** — no secret in any payload | §14.8 | install/probe requests carry no host or secret; install env adds nothing; `pty://output` carries installer stdout, which is not a secret |
| **INV-31** — secrets never in env/argv/payload | §14.8 | unchanged; the install path reads no secret store at all |
| **ADR-A1** — host key checking stays `True` | §14.7 | untouched; `ANSIBLE_HOST_KEY_CHECKING=True` + the pinned cfg remain |
| **ADR-A2** — password hosts stay skipped | §14.7 | untouched; eligibility is computed before any translation or spawn |
| **ADR-8** — console renders via `textContent` | §13 | `#ans-install-log` sets text only; installer bytes are never parsed as HTML |

**New invariants (the spec missed these):**

- **INV-32 — install/probe byte-identity.** `ansible_install` and `ansible_wsl_probe` never read or
  write `~/.ssh/config` or the secrets sidecar; the config file and sidecar are hash- and
  mtime-identical before and after. Install registers zero persistent artifacts. Gate: cargo test +
  source audit (§15.10).
- **INV-33 — WSL distro argv guard.** The distro name is a single argv element and must pass
  `sftp_safe` (refuse a leading `-`, CR, LF) before it reaches `wsl.exe --distribution`, mirroring
  the existing guard on host/path values. Gate: cargo unit test + the NFR-AI4 grep.
- **INV-34 — path translation is total and fail-closed.** In WSL mode every path element (playbook,
  inventory, cfg, every `IdentityFile`, `ANSIBLE_CONFIG`) is either translated to a `/mnt/...` form or
  the run is refused with `ansible:path-untranslatable`; **no Windows-form path reaches the Linux
  child**. `wslpath` absence falls back to the pure-Rust translator, never to an untranslated path.
  Gate: cargo unit tests + Windows CI integration (§15.10).
- **INV-35 — install is explicit-click-only.** No installer process is spawned except as a direct
  result of an `ansible_install` invoke. Panel open, app launch, Re-check and the post-install
  re-check never spawn an installer. Gate: smoke invoke counter (panel open → 0 installer spawns)
  + source audit.

### 15.9 ADRs

**Confirmed from the spec (unchanged):**

- **ADR-AI1 — installer choice.** pipx on Linux (module fallback), brew on macOS, `apt-get` inside the
  WSL distro on Windows. No bundled runtime, no new dependency. *Confirmed* — it is the only set that
  satisfies NFR-AI1 and "no root escalation from the app".
- **ADR-AI2 — WSL invocation.** Backend-native `wsl.exe --distribution <distro> --exec …` with
  pre-translated argv, not a shim script. *Confirmed* — a shim would be a second file to secure and
  would re-introduce a shell boundary.
- **ADR-AI3 — root in WSL.** The in-distro install runs as `wsl --user root`. *Confirmed* — OQ-AI2 is
  closed (MEKL, 12 Nov 2026); root is scoped to the distro, never the host.
- **ADR-AI4 — no key copy.** The install and probe paths never copy or read SSH keys. *Confirmed*.

**New ADRs:**

- **ADR-AI5 — hybrid path translation (`wslpath` preferred, pure-Rust fallback).** *Status:
  accepted.* `wslpath -a -u` is authoritative when present; `win_to_wsl_path()` covers its absence and
  keeps the logic unit-testable on Linux CI. *Consequences:* N+3 subprocesses per WSL run (bounded);
  a missing `wslpath` is not fatal for drive-letter paths; UNC/drive-relative/relative paths still
  fail closed. *Alternatives rejected:* pure-Rust only (drifts from platform edge cases); always
  `wslpath` (breaks when WSL is installed but the service is not running, and is untestable on Linux
  CI).
- **ADR-AI6 — one install session id, sequential PTY children.** *Status: accepted.* INV-14 forbids
  `sh -c "a && b"`, so the two apt spawns are separate processes; one session id keeps one console,
  one exit event, one kill handle. *Consequences:* the orchestration thread must poll for
  cancellation (§15.4.3); a rung boundary is invisible to the frontend. *Alternative rejected:* two
  session ids (two consoles, two exit events, ambiguous kill semantics).
- **ADR-AI7 — `HOSTERM_ANSIBLE_BIN` override semantics.** *Status: accepted.* Read fresh per
  resolution; when set, it replaces the launcher and **disables path translation** (argv[0] verbatim,
  no PATH lookup, no shell). *Consequences:* on a Windows+WSL host a set override makes the run
  interpret Windows paths inside the distro and fail — it is a **test hook only**, never set by the
  app, the same risk class as `HOSTERM_SSH_BIN` (ADR-16) and `HOSTERM_SFTP_BIN`. Documented in the
  Settings → Advanced help text the same way. *Alternative rejected:* translating even under the
  override (the override exists precisely to point at a native/fake binary with plain argv).

### 15.10 Test strategy

Each NFR-AI maps to a concrete gate. Baselines are never lowered (STATE.md carries the live numbers).

| NFR | Gate | Type |
|---|---|---|
| **AI1** no new dep, no bundled runtime | `cargo build --locked` (Cargo.lock unchanged) + `npm run build` (package-lock unchanged); argv-builder test asserts the installer resolves only `pipx`/`python3`/`brew`/`wsl.exe` | cargo + build |
| **AI2** `check_ansible` read-only, 0 spawn, <50 ms, static deduped probe dirs | cargo test on the pure resolver with a temp PATH (binary only in a probe dir → found; probe dir already in PATH → listed once); smoke asserts panel open issues exactly one `check_ansible` invoke and **no other backend call** | cargo + smoke |
| **AI3** `check_wsl` 1 query <500 ms; probe user-initiated <5 s | cargo test with a fake `wsl.exe` on PATH asserting the classification matrix (absent / rc≠0 / empty / named) and that `check_wsl` spawns exactly once; smoke asserts `ansible_wsl_probe` is **not** invoked on panel open and is invoked on Re-check | cargo + smoke |
| **AI4** no shell string in the install/WSL path | grep gate: `sh -c`, `bash -c`, `cmd /C`, `powershell` = **0** matches in the Ansible install/WSL regions of `src-tauri/src/lib.rs` and `src/main.ts` | grep |
| **AI5** install/probe byte-identity (`~/.ssh/config` + sidecar hash **and** mtime) | cargo test `ansible_install_path_never_touches_ssh_state`: snapshot config+sidecar hash/mtime → run `resolve_install_plan` + the WSL probe helper with a fake `wsl.exe` → assert identical. Wired path covered by smoke + a source audit that the install body calls no `config_path`/`read_secrets` | cargo + smoke + audit |
| **AI6** console renders via `textContent`, 0 CSP violation, 0 script element | smoke: mock `pty://output` emitting `<script>alert(1)</script>` + a hostile-bytes fixture → assert 0 CSP violations and 0 `script` elements in `#ans-install-log` | smoke |
| **AI7** one install at a time; button disabled; 64 KB ring | smoke: click Install twice → exactly 1 `ansible_install` invoke; assert Install disabled + label "Installing…"; emit >64 KB → assert the buffer is capped | smoke |
| **AI8** `win_to_wsl_path` unit tests + `wslpath` integration | cargo unit tests: drive letter, backslash, `~`-expanded path, UNC rejection, already-Unix passthrough; Windows CI integration test running the real `wslpath -a -u` (skipped when WSL is absent) | cargo + Windows CI |

Plus the spec §8 plan: cargo (argv-builder WSL shape, install-plan resolution, `check_ansible`
probe-dir dedup, byte-identity, `check_wsl` classification), build 0 TS errors, smoke (platform-specific
Install button, busy state + console streaming mock, auto re-check on exit, Windows WSL state machine,
`HOSTERM_ANSIBLE_BIN` override, hostile bytes), Windows CI `wslpath` integration.

### 15.11 Hotspot + recommended unit sequence

**Hotspot:** `src-tauri/src/lib.rs` is already a 6.5k-line hotspot and carries an **uncommitted
sibling edit** at the time of writing (an SFTP UTF-8 carry fix). The auto-install work adds ~200 LOC
to the same file. Architect recommendation (unchanged from §14.10 LQ-1, now stronger): if the body
grows past ~150 LOC, extract the install code into a `mod ansible_install` (or keep the pure cores
in a `mod ansible`) so the `lib.rs` diff stays thin — the command names and payloads are identical
either way. The pure cores
(`resolve_install_plan`, `win_to_wsl_path`, `build_wsl_argv`, `ansible_probe_dirs`) should land as a
separate unit from the command wiring regardless.

**Recommended unit order (dependency-driven):**

1. Pure cores + cargo tests (no wiring): `ansible_probe_dirs`, `resolve_install_plan`,
   `win_to_wsl_path`, `build_wsl_argv`.
2. `check_wsl` + `ansible_wsl_probe` + the `check_ansible` extension (additive IPC).
3. `ansible_install` + orchestration + PTY wiring.
4. `ansible_run` delta (launcher resolution, cwd, translation).
5. Frontend guide state machine + install console + panel-close cancel.
6. Windows CI `wslpath` integration.

### 15.12 Open items for the lead (granularity only — no architectural decision left open)

- **LQ-4 (install console retention primitive):** whether the 64 KB ring buffer is a shared helper
  with the fleet console or a local one. Recommendation: shared helper (one retention rule, one test).
- **LQ-5 (`DEBIAN_FRONTEND=noninteractive`):** whether to set it for the apt rungs. Recommendation:
  yes (env only, no argv change) so a package hook cannot block the PTY.
- **LQ-6 (`wslpath` deadline):** the 2 s cap is an implementation choice; the fallback makes it
  non-fatal. Lead may tune, but the fallback order (wslpath → pure Rust → refuse) is contract.
- **LQ-7 (`HOSTERM_ANSIBLE_BIN` + `check_ansible` interaction):** when the override is set,
  `check_ansible` resolves it as a file (mirroring `binaries_missing`' `HOSTERM_SFTP_BIN` branch) and
  `probe_dirs` is empty. Lead confirms the smoke expectation.
- **LQ-8 (module split):** see §15.11 — the pure cores should be a separate unit from the wiring.
