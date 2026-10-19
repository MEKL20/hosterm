# hosterm — Product PRD v1

| Field | Value |
|---|---|
| Author | crew-coding-pm |
| Status | Draft — for MEKL review |
| Product | hosterm — desktop SSH/SFTP client (Windows-first) |
| Current version | v0.8.1 (public releases) |
| Document type | **Product PRD** (whole product). The security-hardening work is an EPIC referenced from here, not a substitute for this document: see `docs/FEATURE-SPEC-security-hardening-wave-1.md` + `docs/EPIC-TSD-security-hardening-wave-1.md`. |
| Version constraint | No git commits by PM; this document only |

---

## 1. Overview & Use Cases

### 1.1 Product overview

hosterm is a lightweight desktop SSH/SFTP client (Termius-style) for Windows, built on Tauri v2 (Rust backend + web UI). Its defining product decision: **`~/.ssh/config` is the single source of truth** — no vault, no proprietary store, no database. Switch devices by copying one file.

Founding technical principle (product-visible): hosterm spawns the OS's **real `ssh`/`sftp` binaries**. Everything a user's ssh config declares — ProxyJump, IdentityFile, forwarding, Match blocks — behaves exactly as on the command line. No ssh2/russh-style libraries.

| Aspect | Position |
|---|---|
| Platform | Windows 10/11 first (WebView2 runtime); developed on Linux/macOS |
| Core surfaces | Host sidebar · embedded terminal · SFTP browser · remote editor |
| Storage | `~/.ssh/config` only (+ secrets sidecar `~/.ssh/hosterm.secrets`, 0600, for optional stored passwords) |
| Telemetry | None. No network calls except SSH/SFTP connections the user starts |
| License | MIT, public GitHub releases |

### 1.2 Personas

| Persona | Stake |
|---|---|
| **SSH operator (end user)** | Manages a handful-to-dozens of servers from a Windows desktop. Already lives in `~/.ssh/config`; rejects tools that hide or rewrite it. Wants terminal + file transfer + quick edits in one window. |
| **Multi-device operator** | Works from desktop and laptop; needs identical host setup everywhere without re-entering secrets into a vendor cloud. |
| **MEKL (solo maintainer)** | Ships fast with a verify loop (48 Rust tests, 43+ smoke checks) and minimal Windows test time. |
| **Adversary (hostile server / supply chain)** | See `docs/FEATURE-SPEC-security-hardening-wave-1.md` §3. Handles user key material; must not become an XSS/supply-chain path to it. |

### 1.3 Use cases

| # | Use case | Surfaces |
|---|---|---|
| UC1 | Operator opens terminals to several servers side by side in one window; ssh config (ProxyJump chains, keys, aliases) behaves exactly as in a real terminal | Sidebar, terminal tabs |
| UC2 | Operator browses a remote directory, uploads a build artifact, downloads a log | SFTP browser |
| UC3 | Operator fixes a typo in a remote nginx config: clicks the file, edits with line numbers, Ctrl+S writes it back — no nano/vim over ssh | SFTP browser → editor |
| UC4 | Operator adds a password-only legacy host, types the password once at the prompt (or stores it locally), opens a persistent SFTP session to it | Host form, terminal, SFTP |
| UC5 | Operator switches laptops: copies `~/.ssh/config` (+ keys, + sidecar if used); hosterm on the new machine shows the same hosts | Whole product |
| UC6 | Operator hand-edits the raw config inside the app for directives the form doesn't cover | Raw config editor |
| UC7 | New user downloads `hosterm.exe` from GitHub Releases and runs it; WebView2 is preinstalled on current Windows | Release |

---

## 2. User Stories (per product area)

### A — Host connection (`~/.ssh/config` as single source)
- **US-A1:** As an SSH operator, I want every `Host` entry from my `~/.ssh/config` listed in a sidebar, so that connecting is one click instead of remembering commands. *(shipped)*
- **US-A2:** As an SSH operator, I want to add/edit/delete hosts through a form that writes back to `~/.ssh/config` preserving comments, ordering, and unknown directives, so that my hand-tuned config survives app edits byte-faithfully where it should. *(shipped)*
- **US-A3:** As an SSH operator, I want per-host auth choice — key (pick an `IdentityFile`) or password (`PreferredAuthentications password` + `PubkeyAuthentication no`, prompted in-terminal) — so that auth lives in standard OpenSSH directives, not a proprietary vault. *(shipped)*
- **US-A4:** As an SSH operator, I want to paste a private key's text and have hosterm save it as `~/.ssh/<name>` at 0600 and offer it in the key dropdown, so that I don't hand-edit key files. *(shipped)*
- **US-A5:** As an SSH operator, I want a raw config editor inside the app, so that directives the form doesn't model stay one pane away. *(shipped)*

### B — Terminal
- **US-B1:** As an SSH operator, I want an embedded terminal tab per host, several at once, so that I stop juggling separate terminal windows. *(shipped)*
- **US-B2:** As an SSH operator, I want the terminal to run the real `ssh` binary in a PTY, so that every config directive (ProxyJump, forwarding, Match) behaves exactly as on the command line. *(shipped; founding principle)*
- **US-B3:** As an SSH operator, I want pasted multi-line text normalized CRLF→LF, so that Windows clipboard paste doesn't inject stray carriage returns into remote commands. *(shipped)*

### C — SFTP browse/transfer
- **US-C1:** As an SSH operator, I want to browse remote directories in a per-host SFTP tab, so that I can inspect server files without a second app. *(shipped)*
- **US-C2:** As an SSH operator, I want to upload and download files, so that deployment and log-grabbing happen in place. *(shipped; key/agent auth via batch `sftp -b -`)*
- **US-C3:** As an SSH operator, I want a local file pane alongside the remote one with manual-mouse drag/copy, so that transfers are direct; HTML5 drag-and-drop unreliability in WebView2 must never bite me. *(shipped)*

### D — Password-auth hosts
- **US-D1:** As an operator of password-only legacy hosts, I want to connect from the terminal and be prompted for the password there, with no secret written into `~/.ssh/config`, so that the config stays portable and safe to copy. *(shipped)*
- **US-D2:** As the same operator, I want an optionally stored password (secrets sidecar, 0600) auto-typed once at the prompt, and a persistent interactive `sftp` session per host shared by every tab, so that password-auth hosts get full SFTP too, without re-typing per transfer. *(shipped — persistent-session work, 2026-10-18)*

### E — Remote editor
- **US-E1:** As an SSH operator, I want to click any file in the SFTP browser and edit it in place (CodeMirror, line numbers, Ctrl/Cmd+S saves back), so that remote text edits stop requiring vim over ssh. *(shipped)*
- **US-E2:** As the same operator, I want text files up to 2 MB supported and binary files refused with a clear message, so that I never mangle a binary by accidentally opening it. *(shipped)*

### F — Release & update
- **US-F1:** As a Windows user, I want a single `hosterm.exe` (plus NSIS/WiX installers) on the GitHub Releases page, so that install is download-and-run with no wizard. *(shipped; `v*` tag → `build-windows.yml` on `windows-latest`)*
- **US-F2 (proposed):** As a Windows user, I want the app to notify me of a new release (and later auto-update), so that I don't run stale builds with known CVEs. *Depends on the signing/trust-chain work parked in the feature spec §9.*

---

## 3. Requirements

### 3.1 Functional requirements (measurable)

**Area A — Host connection**

| ID | Requirement | Measure |
|---|---|---|
| FR-A1 | Sidebar lists every `Host` entry from `~/.ssh/config`; pattern/wildcard blocks (`Host *`) shown read-only and never rewritten by the form | 1 sidebar entry per `Host` block; wildcard blocks render disabled/read-only |
| FR-A2 | Host form add/edit/delete round-trips the file: comments, order, and unknown directives preserved verbatim | Parser round-trip test in `cargo test --lib`; manual: edit host → diff config shows only intended lines |
| FR-A3 | Per-host auth writes standard directives: key → `IdentityFile`; password → `PreferredAuthentications password` + `PubkeyAuthentication no` | Saved config contains exactly these directives; real ssh accepts the file |
| FR-A4 | Pasted private key saved as `~/.ssh/<name>` with 0600 (Unix) and appears in the key dropdown | File perms 0600; dropdown refresh includes it |
| FR-A5 | Config writes atomic (temp + rename); never overwrite with empty if existing file unreadable; 0600 on Unix | Unit test for unreadable-file case; interrupted write leaves original intact |
| FR-A6 | Raw config editor reads/writes the file directly | Save round-trips through the same parser as FR-A2 |
| FR-A7 | Config path overridable via `HOSTERM_CONFIG` for testing | Smoke suite runs against a temp config |

**Area B — Terminal**

| ID | Requirement | Measure |
|---|---|---|
| FR-B1 | Click a host → embedded terminal tab (xterm.js); multiple tabs concurrently | ≥ 3 simultaneous tabs in smoke suite |
| FR-B2 | Terminal runs the OS's real `ssh` binary in a PTY; no ssh-library substitutes | `portable-pty` spawn; config directives (ProxyJump etc.) honored |
| FR-B3 | PTY paste normalizes CRLF→LF (`pasteNormalize`) | Unit/smoke check on paste path |

**Area C — SFTP**

| ID | Requirement | Measure |
|---|---|---|
| FR-C1 | Per-host SFTP browser tab (⇅); directory listing navigation via real `sftp` binary | Listing renders for mock hosts in smoke suite; honors ssh config |
| FR-C2 | Upload/download via batch `sftp -b -` for key/agent-auth hosts | Round-trip transfer check in QA |
| FR-C3 | Local pane beside remote pane; drag/copy via manual mouse events (never HTML5 DnD) | Drag works in WebView2 (manual mouse events; invariant #4 in `docs/STATE.md`) |
| FR-C4 | Password-auth hosts: one persistent interactive `sftp` per host in a PTY, shared by every tab for that host; stored password auto-typed once at the prompt | Second tab reuses the session (no second password prompt); batch mode remains for key/age auth |

**Area D — Remote editor**

| ID | Requirement | Measure |
|---|---|---|
| FR-D1 | Click file in SFTP browser → editor tab (✎): CodeMirror, line numbers, Ctrl/Cmd+S writes back in place | Smoke suite opens + saves a file |
| FR-D2 | Text files ≤ 2 MB editable; binary files refused before open | > 2 MB and binary inputs rejected with visible message |

**Area E — Release/update**

| ID | Requirement | Measure |
|---|---|---|
| FR-E1 | `v*` tag → GitHub Actions builds NSIS `.exe` installer + WiX `.msi` on `windows-latest`, attaches to the release | Tag build green end-to-end (also acceptance of feature-spec R2.4/R4.3) |
| FR-E2 | README documents the WebView2 requirement and download path | New user installs from Releases page instructions alone |

### 3.2 Non-functional requirements

| ID | Category | Requirement | Measure / target |
|---|---|---|---|
| NFR-1 | Security — secrets | No secrets ever in `~/.ssh/config`; secrets only in `~/.ssh/hosterm.secrets` (0600); pasted keys 0600 | Grep test in verify loop; perms asserted |
| NFR-2 | Security — hardening wave | Ship wave 1 per `docs/FEATURE-SPEC-security-hardening-wave-1.md`: CSP enforcement (R1), CI action SHA pinning incl. third-party `softprops/action-gh-release` (R2), dependency audit gates (R3), CI token scope minimized (R4) | Feature-spec acceptance criteria, incl. 0 CSP violations across smoke suite + manual Windows matrix |
| NFR-3 | Security — privacy | No telemetry; no network calls except user-initiated SSH/SFTP | Code audit: no outbound HTTP outside spawn of ssh/sftp |
| NFR-4 | Reliability — config integrity | `~/.ssh/config` stays parseable by REAL ssh at all times; failed/partial writes never destroy the existing file | Atomic-write tests; round-trip tests (48 cargo tests must stay green) |
| NFR-5 | Reliability — regression gate | Every change passes the verify loop: `cargo test --lib` → 48; `npm run build` → no TS errors; `qa-reports/headless-smoke.mjs` → 43+ checks, extended for new behavior | Verify loop green on every PR |
| NFR-6 | Performance | UI actions respond with visible feedback ≤ 200 ms (tab open, directory listing render, editor open); host startup parses a 1,000-`Host` config ≤ 100 ms; connect-to-prompt dominated by ssh itself (network), not app overhead | Targets — add timing checks to smoke suite when first perf bug appears (see OQ5) |
| NFR-7 | Usability | Single-window workflow: sidebar + tabs cover terminal, files, editing without app switching; drag interactions never rely on HTML5 DnD; pasted text never carries CR into remote shells | Smoke checks FR-C3/FR-B3; manual pass on Windows |
| NFR-8 | Portability | Whole user setup movable by copying `~/.ssh/config` (+ keys + optional sidecar); no registry/proprietary store | Fresh-machine checklist: copy 1 file (+keys) → hosts appear, connect works |

---

## 4. Acceptance Criteria

Per-requirement ACs live in the tables above (Measure column). Release-level acceptance:

1. **AC-1 (invariants):** All five hard invariants in `docs/STATE.md` hold after every change (config parseable by real ssh; round-trip fidelity; CRLF paste; no HTML5 DnD; staging grep).
2. **AC-2 (verify loop):** 48 cargo tests + TS build + 43+ smoke checks green; smoke suite extended for any new behavior.
3. **AC-3 (security wave):** Wave-1 feature-spec ACs R1–R4 all pass; zero CSP violations in smoke + manual Windows matrix; next tag build produces installers end-to-end.
4. **AC-4 (product smoke):** UC1–UC7 each executable on a real Windows build with no manual config-file hand-editing beyond what the app's own editors offer.
5. **AC-5 (release):** Every release tag yields installers attached to the release page, built from SHA-pinned CI with audit gates green (once wave 1 ships).

---

## 5. Dependencies & Milestones

**Dependencies:** Microsoft Edge WebView2 runtime (end user); OpenSSH client binaries present on the OS (Windows 10/11 ship them optionally — see OQ2); GitHub Actions windows runner for releases. Internal: wave-1 hardening gates releases (AC-5).

| Horizon | Scope | Status / reference |
|---|---|---|
| **Now (v0.8.x, shipped)** | Areas A–E as listed: sidebar, terminal, SFTP browse/transfer incl. persistent password-SFTP, remote editor, host form, key paste, raw editor, releases | Live |
| **Now (in flight)** | Security hardening wave 1 (CSP, action pinning, audit gates, token scope) — spike findings in `docs/SPIKE-csp-findings.md`; ship gate for next release | EPIC: `docs/EPIC-TSD-security-hardening-wave-1.md` |
| **Next (proposed)** | Release trust chain: installer signing + auto-update notification/update path (US-F2), SBOM per release, DOM-hardening refactor of the 21 `innerHTML` sites | Parking lot, feature-spec §9 — needs a wave-2 feature spec before any code |
| **Later (proposed)** | Broader platform story (first-class Linux/macOS installers), config auto-reload on external change, editor limit beyond 2 MB | Ideas only; not committed |

Roadmap rule: nothing enters "Next" without a feature spec like wave 1's; nothing enters "Now" without verify-loop coverage.

---

## 6. Out of Scope, Open Questions, Recommendations

### 6.1 Out of scope (v1)

- Any vault/proprietary credential store beyond the 0600 sidecar — contradicts the founding product decision.
- ssh2/russh-style libraries — founding principle is real binaries.
- Telemetry, accounts, cloud sync of any kind.
- Auto-update in v1 (proposed for next horizon, after signing).
- Mobile/web clients.
- Rewriting config semantics — hosterm is a faithful editor/spawner for OpenSSH config, not a superset of it.

### 6.2 OPEN QUESTIONS (for MEKL — each with PM recommendation; old habit: recommendation always attached)

| # | Question | Recommendation |
|---|---|---|
| Q1 | README says password auth "no secret is ever stored", but the shipped persistent-session work stores passwords in the sidecar (0600) and auto-types them. Doc drift — which is canonical? | STATE.md + shipped behavior are canonical (sidecar storage exists, opt-in). Fix README in a docs-only PR — no code change, no version bump. |
| Q2 | Windows 10/19 ship an *optional* OpenSSH client; a machine without `ssh.exe`/`sftp.exe` on PATH breaks the founding principle at first launch. Detect and guide? | Add a startup check (proposed, small): detect missing binaries, show a one-screen "install OpenSSH client" pointer. No bundling — that drags in update/security surface. |
| Q3 | Stored password: opt-in per host (current) or global setting? | Keep opt-in per host. Global toggles invite accidental storage; per-host matches the "password hosts are the exception" reality. |
| Q4 | When does wave 2 (signing → auto-update) get its feature spec? | Right after wave 1 ships — auto-update without signed artifacts is a supply-chain downgrade, so order is fixed: sign first, then update. |
| Q5 | NFR-6 perf targets (≤ 200 ms feedback, ≤ 100 ms/1k hosts) have no measurement in CI yet. Add now or on first complaint? | On first complaint. Smoke suite already catches functional regressions; premature timing gates add CI rot for a solo maintainer. Targets become binding when measured. |
| Q6 | Config auto-reload while the app is open and the file changes externally (another terminal, another machine's sync)? | Propose mtime-check-on-focus in wave 2 — cheap, no file-watcher daemon. Until then, document "restart or re-open sidebar to pick up external edits". |
| Q7 | Editor 2 MB limit — user-facing enough to raise? | Keep 2 MB. Files beyond it are logs/binaries in practice; streaming edits are scope creep for v1. Revisit only with a real request. |

---

*Sources: `README.md`, `docs/STATE.md`, `AGENTS.md`, `docs/FEATURE-SPEC-security-hardening-wave-1.md`, `docs/EPIC-TSD-security-hardening-wave-1.md`, `docs/SPIKE-csp-findings.md`, completed tasks t_a93ae98f (persistent password-SFTP) and t_9af83e50 (wave-1 spec).*
