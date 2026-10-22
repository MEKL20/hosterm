# SDD: hosterm v2

> Deliverable architect. Requirements-first lane: derived dari `docs/PRD.md` (§3/§4/§5, pm's living doc) + wave-2 feature draft §4/§5 + Arus epic. SDD v1 + its signing/updater extension are folded here; this document supersedes SDD v1 as the single architecture source.

| Field | Value |
|---|---|
| Author | crew-coding-architect |
| Inputs | `docs/PRD.md` (requirements authoritative), `docs/archive/SDD-hosterm-v1.md` (basis), `docs/SDD-signing-updater.md` (folded into §2.2/§7 ADR-9..12), `docs/EPIC-TSD-redesign-arus-v1.md`, `docs/archive/FEATURE-SPEC-wave-2-draft.md` §4/§5, `docs/FEATURE-SPEC-fleet-run-v1.md` (drives the §12 Fleet Run v1 addendum + ADR-13..16 + INV-14..19), `docs/STATE.md`, `docs/SPIKE-csp-findings.md`, `src-tauri/src/lib.rs`, `src/main.ts`, `src-tauri/tauri.conf.json` |
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
| Terminal sessions (`src/main.ts`) | One xterm.js `Terminal` per tab/split; fit addon; key/copy/paste; `pasteNormalize` (CRLF→LF) | `pty_spawn/write/resize/kill`; events `pty://output/{id}`, `pty://exit/{id}` |
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

*Sources: docs/PRD.md, docs/archive/SDD-hosterm-v1.md, docs/SDD-signing-updater.md, docs/EPIC-TSD-redesign-arus-v1.md, docs/archive/FEATURE-SPEC-wave-2-draft.md, docs/FEATURE-SPEC-fleet-run-v1.md, docs/STATE.md, docs/SPIKE-csp-findings.md, src-tauri/src/lib.rs, src/main.ts, qa-reports/headless-smoke.mjs, src-tauri/tauri.conf.json (all read 2026-10-19). Note: PRD-hosterm-v2.md landed mid-authoring from pm; header/flows reconciled against it. §12 addendum + ADR-13..16 + INV-14..19 authored for card t_a367888f (Fleet Run Unit 0).*
