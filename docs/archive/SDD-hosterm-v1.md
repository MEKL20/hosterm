# hosterm — Software Design Document (SDD) v1

| Field | Value |
|---|---|
| Author | crew-coding-architect |
| Input | docs/archive/PRD-hosterm-v1.md (requirements authoritative; not reopened here) |
| Status | Draft — for MEKL review; downstream input to lead's TSD |
| Downstream | TSD (crew-coding-lead) derives technical task specs from this |
| Scope | Whole product, v1. Wave-1 security work is summarized where it shapes architecture; its own spec lives in `docs/EPIC-TSD-security-hardening-wave-1.md`. |
| Version constraint | No commits by architect; this document only |

---

## 1. Context

hosterm is a lightweight desktop SSH/SFTP client for Windows 10/11 (WebView2), developed on Linux. Two product-defining decisions constrain everything below:

1. **`~/.ssh/config` is the single source of truth.** No vault, no proprietary store, no database. hosterm is a *faithful editor and spawner for OpenSSH config*, not a superset of it (PRD §6.1).
2. **The OS's real `ssh`/`sftp` binaries are spawned.** No ssh2/russh-style libraries. Every config directive a user declares (ProxyJump, IdentityFile, forwarding, Match) behaves exactly as on the command line.

Everything a hostile or merely misconfigured remote can send — SFTP filenames, file contents, terminal output — is rendered into a web DOM. The architecture must treat that as attacker-reachable input while keeping user key material and stored passwords out of reach.

Current implementation reality this document consolidates: architecture lives organically in `docs/STATE.md` + `AGENTS.md`; the codebase is deliberately two big modules (`src/main.ts` ~1.4k lines, `src-tauri/src/lib.rs` ~2.3k lines) plus QA harness and CI workflow.

### 1.1 Traceability: PRD areas → components

| PRD area | Stories | Primary components |
|---|---|---|
| A — Host connection | US-A1..A5 | Sidebar UI, host form, raw editor (FE) · config model + identity-file store (BE) |
| B — Terminal | US-B1..B3 | Tab shell + xterm.js (FE) · PTY engine (BE) |
| C — SFTP | US-C1..C3 | SFTP browser dual panes (FE) · SFTP layer: batch + persistent session (BE) |
| D — Password hosts | US-D1..D2 | Secrets sidecar + prompt-watcher injection (BE) · persistent sftp session (BE) |
| E — Remote editor | US-E1..E2 | Editor tab + CodeMirror (FE) · sftp read/write-file (BE) |
| F — Release | US-F1..F2 | GitHub Actions `build-windows.yml`, NSIS/WiX, tags |

---

## 2. Architecture overview

### 2.1 Component diagram

```
┌───────────────────────────── Windows process (Tauri v2) ─────────────────────────────┐
│                                                                                      │
│  WebView2 (http://tauri.localhost)                 Rust core (src-tauri/src/lib.rs)  │
│  ┌───────────────────────────────┐   IPC (invoke /  ┌─────────────────────────────┐  │
│  │ Frontend  src/main.ts         │   emit events)    │ Config model                │  │
│  │  sidebar · host form          │ ────────────────► │  parse/serialize (round-    │  │
│  │  tab shell (4 session maps)   │   20 commands     │  trip) · atomic write       │  │
│  │  xterm.js terminals           │ ◄──────────────── │ Secrets sidecar (0600)      │  │
│  │  CodeMirror editor            │   pty://output/N  │ Identity-file store (0600)  │  │
│  │  SFTP browser (dual panes)    │   pty://exit/N    │ PTY engine (portable-pty)   │  │
│  │  dialogs · manual DnD         │                   │ SFTP layer (batch + persist)│  │
│  │  pasteNormalize               │                   │ Clipboard (arboard)         │  │
│  └───────────────────────────────┘                   └──────────────┬──────────────┘  │
│         CSP: script-src 'self'; style-src + 'unsafe-inline'          │ spawn            │
│              (tauri.conf.json, SEC-4 landed)                         ▼                  │
└──────────────────────────────────────────── real OS binaries ────────────────────────┘
                                                ssh ── PTY (one per terminal/split)
                                                sftp -b -  (batch, key/agent auth)
                                                sftp (interactive, one persistent
                                                      PTY per password-auth host)
                                                      │
        ~/.ssh/config ◄── reads/writes ──┐            ▼ honors the config
        ~/.ssh/hosterm.secrets ◄─ rw ────┤   OpenSSH client (ssh + sftp)
        ~/.ssh/<name> keys ◄─ writes ────┘        │
                                                  ▼
                                           remote hosts
```

### 2.2 Components and responsibilities

| Component | Location | Responsibility | Talks to |
|---|---|---|---|
| **UI shell & tab system** | `src/main.ts` | Sidebar, tabs (terminal / SFTP ⇅ / editor ✎), split terminals, dialogs/overlays, tab drag (manual mouse events), context menu | Session maps below; all rendering via DOM |
| **Terminal sessions** | `src/main.ts` (`sessions`, `splitSessions` maps) | One xterm.js `Terminal` per tab/split; fit addon; key/copy/paste handling; `pasteNormalize` (CRLF→LF) | `pty_spawn/write/resize/kill`, `pty://output/{id}`, `pty://exit/{id}` |
| **SFTP browser** | `src/main.ts` (`sftpSessions`, `localPane`, `sftpHostCache` maps) | Dual-pane remote + local listing, upload/download, manual-mouse drag/copy (never HTML5 DnD — dead in WebView2) | `sftp_list`, `local_list`, `home_dir`, `sftp_upload/download` |
| **Remote editor** | `src/main.ts` (`editorSessions`) | CodeMirror 6 with basicSetup, line numbers, Ctrl/Cmd+S write-back; 2 MB text cap, binary refusal | `sftp_read_file`, `sftp_write_file` |
| **Host management UI** | `src/main.ts` | Host form (key vs password auth mode), key-paste modal, raw config editor, dirty-state guards | `read_ssh_config`, `save_host`, `delete_host`, `read/write_config_raw`, `list_identity_files`, `create_identity_file` |
| **Config model** | `src-tauri/src/lib.rs` | Round-trip parse/serialize of `~/.ssh/config` (preserves comments, order, unknown directives); atomic write (sibling temp file → chmod 0600 → rename); `HOSTERM_CONFIG` override | Filesystem `~/.ssh/config` |
| **Secrets sidecar** | `src-tauri/src/lib.rs` | `~/.ssh/hosterm.secrets` (`alias=password` lines, 0600, removed when empty); lookup result **never crosses IPC** | Filesystem sidecar |
| **Identity-file store** | `src-tauri/src/lib.rs` | Paste-private-key → `~/.ssh/<name>` at 0600; dropdown listing | Filesystem `~/.ssh/` |
| **PTY engine** | `src-tauri/src/lib.rs` (`PtyState`, `pump_pty`) | portable-pty openpty per terminal; spawns real `ssh <host>`; reader thread pumps output → `emit("pty://output/{id}")`; one-shot password injection on `'s password:` prompt; reaper thread | portable-pty, `ssh` binary, secrets |
| **SFTP layer** | `src-tauri/src/lib.rs` | Two modes, same operation API `run_sftp_op`: **batch** `sftp -b -` per operation (real exit codes; key/agent auth) and **persistent** interactive `sftp` in a PTY, one per host alias, shared across tabs (password auth; completion = next `sftp> ` prompt; dead session → one respawn-retry). `sftp_safe` input validation (no leading `-`, no newlines) on every host/path argument | `sftp` binary, secrets |
| **Editor transfer** | `src-tauri/src/lib.rs` | `sftp_read_file` / `sftp_write_file` via run_sftp_op; write verified by re-read before success | SFTP layer |
| **Clipboard & local FS** | `src-tauri/src/lib.rs` | arboard clipboard read/write; `local_list` for the local pane | OS |
| **QA harness** | `qa-reports/headless-smoke.mjs` | Playwright chromium, serves `dist/` on :4199 with the prod CSP header, mock invoke map (hosts: prod-web, db-core); 43+ checks incl. CSP-violation counter == 0 | `dist/` build |

### 2.3 IPC surface (the contract between the two modules)

Commands (invoke): `read_ssh_config`, `save_host`, `delete_host`, `read_config_raw`, `write_config_raw`, `list_identity_files`, `create_identity_file`, `sftp_list`, `sftp_upload`, `sftp_download`, `sftp_read_file`, `sftp_write_file`, `local_list`, `home_dir`, `clipboard_read`, `clipboard_write`, `pty_spawn`, `pty_write`, `pty_resize`, `pty_kill`.

Events (emit, backend → frontend): `pty://output/{id}` (terminal bytes), `pty://exit/{id}` (terminal closed).

Rules:
- Request/response for everything except streaming terminal output — that alone uses events.
- **Secrets never appear in IPC payloads.** `lookup_stored_password` runs inside the Rust process; the stored password is injected directly into the PTY writer (see §3.3 flow 4).
- All host/path strings crossing IPC pass `sftp_safe` (or equivalent) before reaching a spawned binary.

### 2.4 Delivery & packaging

Vite bundles the frontend to `dist/` (one external JS, one external CSS — zero inline scripts/styles in HTML); Tauri embeds it and serves over the custom protocol with the CSP response header. `v*` tag → `build-windows.yml` on `windows-latest` → NSIS `.exe` + WiX `.msi` attached to the GitHub release. Dev mode: Vite on `:1420` via `devUrl` (CSP in dev is delivered by Vite `server.headers`, see ADR-6).

---

## 3. Data model

There is **no database**. Three files under `~/.ssh/` plus environment overrides.

### 3.1 `~/.ssh/config` — source of truth

Parsed model (mirrors `lib.rs`):

```
SshConfig
├── preamble: string[]                    # lines before first Host block, verbatim
└── hosts: HostBlock[]
    ├── patterns: string[]                # every token after "Host" (aliases, wildcards)
    └── lines: CfgLine[]                  # in file order
        ├── Raw(string)                   # comments + blank lines, verbatim
        └── Opt { key, value }            # directive, key case preserved on read
```

Constraints and behaviors:

| Rule | Enforcement |
|---|---|
| File stays parseable by real ssh at all times | Atomic write (below); unreadable existing file **refuses** edit rather than overwrite (`read_config_text` errors out) |
| Round-trip fidelity: comments, order, unknown directives preserved | `parse_config`/`serialize_config` keep `Raw` lines and unknown `Opt` keys; cargo round-trip tests (48 total) |
| No secrets in the config — ever | Form writes only standard directives: key auth → `IdentityFile`; password auth → `PreferredAuthentications password` + `PubkeyAuthentication no`. Grep-tested in verify loop |
| Wildcard blocks (`Host *`) read-only | Sidebar renders them disabled; form never rewrites them |
| Atomic writes | Sibling temp file `.config.hosterm-tmp-<pid>` → chmod 0600 (Unix) → rename over target; failed rename cleans up temp |
| Testability | `HOSTERM_CONFIG` env overrides the path (smoke suite runs against a temp config) |

### 3.2 `~/.ssh/hosterm.secrets` — sidecar (the only secret store)

```
<alias>=<password>\n     # BTreeMap on disk; written 0600 (Unix); file deleted when empty
```

- Opt-in **per host** (form checkbox); deleting the host or unchecking purges the entry (`purge_secret`).
- Read only inside the Rust process. Never returned over IPC, never logged, never rendered.
- Legacy `Password` directives found in the config are migrated out into the sidecar on read (`migrate_legacy_password_directives`) — the config must not carry secrets.
- Portability trade-off (accepted): device switch = copy `~/.ssh/config` + keys **+ optional sidecar**. Without the sidecar, password hosts simply prompt again. Documented in PRD NFR-8 and Q1 (README drift fix pending).

### 3.3 Supporting files and env

| Store | Shape | Notes |
|---|---|---|
| `~/.ssh/<name>` | Pasted private keys | 0600 (Unix); offered in key dropdown; standard OpenSSH usage only |
| `HOSTERM_CONFIG` | env | Config path override (tests) |
| `HOSTERM_SFTP_BIN` | env | sftp binary override (tests) |

### 3.4 Key dynamic flows (component interactions)

1. **Open a terminal session** (UC1): sidebar click → `openTerminal` → tab created in `sessions` → `invoke("pty_spawn", {host, cols, rows})` → backend opens portable-pty pair, spawns `ssh <host>`, `lookup_stored_password` (optional), reader thread emits `pty://output/{id}` → xterm writes bytes; keyboard → `pty_write`; resize → `pty_resize`; tab close → `pty_kill`. Password prompt (no stored secret) is answered by the user typing into xterm — same path as any keystroke.
2. **Browse SFTP** (UC2): ⇅ tab → `openSftp` → `invoke("home_dir")` + `invoke("sftp_list", {host, path})` → backend `run_sftp_op`: if a stored secret exists for the host → persistent interactive `sftp` session (spawn once, type password at prompt, reuse; keyed by alias in a global map), else → one-shot `sftp -b -` batch. Listing text parsed by pure `parse_sftp_ls`. Upload/download → `sftp_upload/download` (same dual-mode dispatch; same-machine aliasing guard refuses src==dst).
3. **Edit a remote file** (UC3): click file in SFTP pane → `sftp_read_file` → editor tab (CodeMirror) → Ctrl/Cmd+S → `sftp_write_file` with full document text → backend writes via temp + verified re-read → editor stays open.
4. **Fill a host password** (UC4): stored secret path — `pty_spawn`'s `pump_pty` watches PTY output for `'s password:`, then writes the secret + CR **once** into the PTY writer; the secret never enters JS. Unstored path — user types at the prompt (backend does nothing). SFTP variant: `sftp_spawn` types the password during login and waits for `sftp>` (30 s timeout, wrong password → clear error).

---

## 4. Technical decisions (ADR)

Format: decision, alternatives rejected, consequence. Each validated against the current code (file:line evidence noted where load-bearing).

### ADR-1 — Spawn real `ssh`/`sftp` binaries; no SSH protocol libraries

- **Decision.** All connectivity goes through the OS's OpenSSH client: `CommandBuilder::new("ssh")` per terminal (lib.rs `pty_spawn`), `sftp -b -` batch and interactive `sftp` in a PTY for file operations. No ssh2/russh/thrussh dependency (Cargo.toml carries none).
- **Alternatives rejected.** ssh2/russh library in-process: would force hosterm to re-implement and re-expose config semantics (ProxyJump chains, Match, agent, forwarding) — permanent divergence risk and a bigger crypto/supply-chain surface. Bundling an ssh binary: drags update/license surface (PRD Q2 recommendation).
- **Consequence.** Config semantics are OpenSSH's problem, not ours (zero re-implementation, automatic CVE coverage via the user's OS updates). Costs: dependency on OpenSSH being installed (Windows optional feature — PRD OQ2 wants a startup check); per-operation connection setup for batch sftp (mitigated by ADR-7 for password hosts); output parsing is textual (`parse_sftp_ls`), so sftp output format drift is a watched risk; password auth needs PTY prompt-watching (ADR-3/§3.4.4).

### ADR-2 — portable-pty + xterm.js for terminals

- **Decision.** Rust `portable-pty 0.8` opens pseudo-terminals and hosts the spawned `ssh`; frontend renders xterm.js 5.5 (+fit addon) per tab. Bridge = backend reader thread → `pty://output/{id}` events; frontend input → `pty_write`.
- **Alternatives rejected.** Library-driven terminal emulation without a real PTY: incompatible with ADR-1 (password prompts, TUI apps need a TTY). ConPTY/winpty direct bindings: portable-pty already abstracts Unix/Windows; xterm.js alternatives (terminal in native widget) would abandon the web UI stack.
- **Consequence.** Full TUI fidelity (vim, htop) for free; process lifecycle must be managed explicitly (spawn ids, reaper threads, kill on tab close — done). Platform quirk accommodated: pasted text normalized CRLF→LF in the frontend (`pasteNormalize`) so Windows clipboard CR never reaches remote shells; on Windows the sftp child is spawned with `CREATE_NO_WINDOW` to avoid console flash.

### ADR-3 — Secrets in a 0600 sidecar file; no vault, no OS keychain

- **Decision.** Optional stored host passwords live in `~/.ssh/hosterm.secrets` (0600 Unix, plain `alias=password`), opt-in per host, purged on host delete, deleted when empty. Lookup stays inside Rust; injection goes straight into the PTY writer.
- **Alternatives rejected.** OS keychain (Windows Credential Manager / libsecret): breaks the "copy one directory to switch devices" portability story (UC5) and adds platform glue for a solo maintainer. Secrets inside `~/.ssh/config`: violates the founding invariant (file must stay portable and shareable). Full vault product: explicitly out of scope (PRD §6.1).
- **Consequence.** Secrets are plaintext at rest behind file permissions — accepted trade-off, consistent with how `~/.ssh/` keys already live; README wording must match reality (PRD Q1 doc fix). Empty-map case deletes the file, so the sidecar never lingers with stale entries.

### ADR-4 — `~/.ssh/config` edited by lossless round-trip, not regeneration

- **Decision.** The parser models the file as preamble + Host blocks + typed lines (`Raw` | `Opt`) and edits only the specific `Opt` entries the form owns; serialization replays everything else verbatim. Writes are atomic (temp sibling → 0600 → rename) and refuse to proceed if the existing file is unreadable.
- **Alternatives rejected.** Regenerate-the-whole-file from an internal model: would destroy comments/order/unknown directives on every save — product-killing for the target persona (US-A2). DOM/parser crates (e.g. ssh2-config): another dependency and most don't preserve byte layout; the grammar is tiny enough that ~150 lines of stdlib code cover it, with round-trip tests.
- **Consequence.** Users' hand-tuned configs survive app edits byte-faithfully; the 48-test cargo suite guards it. Cost: `set_opt` can only manage directives it knows; anything exotic goes through the raw editor (US-A5), which round-trips through the same parser.

### ADR-5 — Tauri v2 + WebView2, single-window web UI

- **Decision.** Tauri 2.x (Rust core + WebView2 on Windows), one window, one TypeScript module for the whole frontend, Vite bundling, `withGlobalTauri: false` (no global API surface). MIT, GitHub Releases distribution.
- **Alternatives rejected.** Electron: ~100 MB runtime and a bundled Chromium to keep patched, against the lightweight goal. Native WinUI/Qt UI: abandons the web ecosystem (xterm, CodeMirror) and Linux dev path. Multi-window: single-window workflow is an explicit NFR-7 goal.
- **Consequence.** ~10 MB-class binary, WebView2 preinstalled on current Windows (UC7), dev works on Linux. Platform debts are known and handled: HTML5 drag-and-drop is dead in WebView2 → all drag/copy uses manual mouse events (invariant #4); drag interactions and PTY paste are smoke-covered.

### ADR-6 — Strict CSP with a single, proven relaxation

- **Decision.** `app.security.csp` in `tauri.conf.json` (SEC-4 landed): `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self'; font-src 'self'; connect-src 'self' ipc: http://ipc.localhost; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; form-action 'none'`. Evidence and engineering in `docs/SPIKE-csp-findings.md`. Key facts now settled: Tauri v2 does **not** auto-append IPC origins (must stay in the string); Tauri appends its own script nonces at runtime; `'unsafe-inline'` is scoped to `style-src` solely for CodeMirror's `style-mod` runtime `<style>` injection — and works only while the built HTML contains no `<style>` element (a nonce landing in `style-src` would neutralize `'unsafe-inline'`; smoke's CSP-violation counter backstops this). Dev delivery is Vite `server.headers` (`devUrl` setups get no Tauri CSP injection), policy + `ws://localhost:1420`.
- **Alternatives rejected.** Per-injection hashes for style-mod: runtime-generated CSS/class names, silently broken on dep bumps. Nonce plumbing via `EditorView.cspNonce` + dummy token: disproportionate to protect `style-src` when `script-src`/`connect-src` already contain the XSS blast radius (escape hatch documented in the spike). Leaving `csp: null` with only the innerHTML-escape discipline: insufficient — 21 `innerHTML` sites vs 13 escapes make CSP the wave-1 compensating control (EPIC §1).
- **Consequence.** Injected scripts can't execute and can't reach the IPC bridge (`connect-src` locked to IPC origins only). Never relax `script-src`; keep zero inline scripts; re-run the spike analysis if Vite CSS delivery changes. Residual: one-time manual WebView2 pass on real Windows (SEC-5, waived as low-risk on 19 Okt with a re-run recommended via Windows runner in a later wave).

### ADR-7 — Persistent interactive `sftp` per password-auth host; batch mode otherwise

- **Decision.** `run_sftp_op` dispatches by auth reality: host has a stored secret → spawn **one** interactive `sftp` in a PTY per host alias (global map keyed by alias, shared by every tab), type the password once at login, run commands with completion = next `sftp> ` prompt (120 s watchdog), verify `put` by re-listing, respawn-retry once on dead/timeout. Host without secret → one-shot `sftp -b -` batch per operation (real exit codes).
- **Alternatives rejected.** Batch mode for everything: password hosts would re-prompt (and re-type) per transfer — UC4/D2 unacceptable. A second password prompt per tab: the map is keyed by host alias precisely so every SFTP tab shares the session. Library SFTP (ssh2): violates ADR-1.
- **Consequence.** Password hosts get full SFTP with one password entry per host per app run. Costs: interactive sftp has no per-command exit codes → completion and success are inferred (prompt + `put` verification); sessions are long-lived state that must die with the app (process exit takes the PTY children; explicit teardown hook exists but is currently unused — `master` kept for future logout-kill).

### ADR-8 — Trust boundary: remote bytes are hostile; CSP is containment, escaping is hygiene

- **Decision.** All remote-origin bytes (SFTP filenames, file contents, terminal output) are rendered through the existing escape helpers where reachable and treated as attacker-controlled regardless; CSP (ADR-6) is the structural backstop that keeps a missed escape from executing script or exfiltrating via `connect-src`.
- **Alternatives rejected.** Ship-of-Theseus innerHTML refactor in wave 1: rejected by the EPIC's non-goals (refactor is parked for wave 2 with a feature spec); CSP first, refactor later, in that order.
- **Consequence.** Known debt is explicit (21 innerHTML sites, 13 escape calls) with a funded plan shape (wave 2 DOM hardening). New rendering code must route through `esc()`/`escAttr()`; review checklist item.

---

## 5. Risks

| # | Risk | Impact | Mitigation / status |
|---|---|---|---|
| R-1 | `innerHTML` + escaping gaps → XSS into IPC bridge | High (key material, sidecar) | ADR-6 CSP enforcement (0 violations across smoke + manual matrix; SEC-5 waiver logged); wave-2 DOM-hardening refactor parked behind a feature spec (PRD §5) |
| R-2 | Textual parsing of `sftp` output drifts (locale/format changes) | Medium (browser breaks) | `parse_sftp_ls` is pure + unit-tested; batch `ls -la` is stable upstream; watch on OpenSSH upgrades |
| R-3 | OpenSSH client missing on a Windows machine | Medium (first-launch failure) | PRD Q2 recommendation: startup detection + guidance; not bundled deliberately (ADR-1) |
| R-4 | Vite CSS delivery change plants `<style>` in HTML → style nonce → `'unsafe-inline'` neutralized → editor unstyled | Medium (editor breaks) | Prohibited-by-review note in spike §5; smoke CSP-violation counter fails the build; re-run spike if it changes |
| R-5 | WebView2 runtime drift vs Chromium behavior (CSP edge cases, `ipc.localhost`) | Low–medium | Chromium-engine harness + smoke cover semantics; one-time manual Windows matrix per release wave (Q7); self-hosted Windows runner proposed for automation (STATE.md waiver) |
| R-6 | Config corrupted by crashed/interrupted write | High (user data) | Atomic temp+rename write, unreadable-file refusal, round-trip test suite (NFR-4) |
| R-7 | Sidecar plaintext-at-rest discovered/copied | Medium (credential leak) | 0600, opt-in per host, purged on delete; consistent with `~/.ssh` threat model; portability trade-off accepted (ADR-3) |
| R-8 | Persistent sftp sessions leak (zombie processes) across tab churn | Low | Liveness watcher + respawn-retry; process exit reaps; explicit teardown is future work (ADR-7) |
| R-9 | Supply chain: mutable CI actions, CVE-laden deps | High (release integrity) | Wave 1: SHA-pinned actions + minimized token scope (R2/R4), dual audit gates high+critical prod (R3); Dependabot updates |
| R-10 | Performance targets (≤200 ms feedback, ≤100 ms/1k hosts) unmeasured | Low | PRD Q5: bind on first complaint; architecture has no obvious hotspot (parse is linear, listings incremental) |

---

## 6. Verification steps

Product-level verification maps onto the existing verify loop (AGENTS.md / STATE.md) plus wave-1 additions:

1. **Rust unit tests:** `cd src-tauri && cargo test --lib` → 48 pass (config round-trip, atomic write incl. unreadable-file case, secrets handling, `parse_sftp_ls`, password-injection pump).
2. **TypeScript build:** `npm run build` → no TS errors.
3. **Headless smoke:** `npm install --no-save playwright@1.63.0 && node qa-reports/headless-smoke.mjs` → 43+ checks green **including** CSP-violation counter == 0 (serves `dist/` with the prod CSP header); covers UC1/UC2/UC3 equivalents against mock hosts; then remove playwright per repo hygiene.
4. **Invariant grep (pre-commit):** staging grep `target/|node_modules|.exe$|dist/` clean; secrets-in-config grep stays clean (NFR-1).
5. **Security wave gates:** action-pin grep `grep -nE "uses:\s*\S+@(v[0-9]+...)"` → 0 matches; both audit lanes green on main or every finding carries a dated waiver ≤ 90 days; negative audit test evidence recorded.
6. **Manual Windows matrix (per wave tag):** UC1–UC7 executable on the real build; terminal/SFTP/editor zero CSP violations; one-time WebView2 CSP injection proof (devtools one-liner from spike §4).
7. **SDD-specific checks (this document):** ADR claims traceable to code (verified during authoring: command list in §2.3 matches `#[tauri::command]` inventory; flows in §3.4 match `pump_pty`/`sftp_spawn`/`run_sftp_op` implementations); CSP string quoted here matches `tauri.conf.json`.

---

*Sources: docs/archive/PRD-hosterm-v1.md, docs/STATE.md, AGENTS.md, docs/SPIKE-csp-findings.md, docs/EPIC-TSD-security-hardening-wave-1.md, src-tauri/src/lib.rs, src/main.ts, src-tauri/tauri.conf.json, src-tauri/Cargo.toml, package.json (all read 2026-10-19).*
