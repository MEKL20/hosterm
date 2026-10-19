# PRD: hosterm v2

> Standar: crew-coding-ops.md (doc chain). AC/behavior = EARS. Living doc — epic
> dirujuk ke sini, bukan sebaliknya. Kata vague dilarang tanpa kuantifikasi;
> validasi grep wajib 0 hit kata sifat vague di luar kutipan.
>
> Supersedes `docs/PRD-hosterm-v1.md` (kept as history, never edited). Regenerated
> to reflect shipped reality v0.10.0+ (19 Okt 2026) and in-flight epics.

| Field | Value |
|---|---|
| Author | crew-coding-pm |
| Status | Living doc — current |
| Product | hosterm — desktop SSH/SFTP client (Windows-first) |
| Shipped baseline | v0.10.0 (tag bb93631) + post-tag commits 3a3b5a0 (W2-A), 33a1833 (U1) |
| Supersedes | docs/PRD-hosterm-v1.md (t_c1bc9a7a) — archive only |
| Version constraint | No git commits/push by PM; this document only |

---

## 1. Overview & Use Cases

### 1.1 Product overview

hosterm is a lightweight desktop SSH/SFTP client (Termius-style) for Windows, built on Tauri v2 (Rust backend + web UI). Its defining product decision: **`~/.ssh/config` is the single source of truth** — no vault, no proprietary store, no database. Switch devices by copying one file.

Founding technical principle (product-visible): hosterm spawns the OS's **real `ssh`/`sftp` binaries**. Everything a user's ssh config declares — ProxyJump, IdentityFile, forwarding, Match blocks — behaves exactly as on the command line. No ssh2/russh-style libraries.

| Aspect | Position |
|---|---|
| Platform | Windows 10/11 first (WebView2 runtime); developed on Linux/macOS |
| Core surfaces | Host sidebar · embedded terminal · SFTP browser · remote editor |
| Storage | `~/.ssh/config` only (+ secrets sidecar `~/.ssh/hosterm.secrets`, 0600, for optional opt-in-per-host stored passwords) |
| Telemetry | None. No network calls except user-initiated SSH/SFTP (plus, from wave 2 M3, the user-initiated update check — see NFR-3 amendment pending MEKL, OQ-1) |
| Visual identity | Theme Abyss Emerald, direction A "Arus" — FINAL (MEKL, redesign §6.5; not reopenable) |
| License | MIT, public GitHub Releases |
| Shipped version | v0.10.0 (security hardening wave 1 + password-SFTP persistent + sidecar copy fix); W2-A startup ssh/sftp check and U1 theme landed post-tag, ride the next release |

### 1.2 Use cases

| # | Use case | Surfaces | Status |
|---|---|---|---|
| UC1 | Operator opens terminals to several servers side by side in one window; ssh config (ProxyJump chains, keys, aliases) behaves exactly as in a real terminal | Sidebar, terminal tabs | Shipped |
| UC2 | Operator browses a remote directory, uploads a build artifact, downloads a log | SFTP browser | Shipped |
| UC3 | Operator fixes a typo in a remote nginx config: clicks the file, edits with line numbers, Ctrl+S writes it back — no nano/vim over ssh | SFTP browser → editor | Shipped |
| UC4 | Operator adds a password-only legacy host, types the password once at the prompt (or stores it opt-in per host in the 0600 sidecar), opens a persistent SFTP session shared by every tab of that host | Host form, terminal, SFTP | Shipped (8b36f2b) |
| UC5 | Operator switches laptops: copies `~/.ssh/config` (+ keys, + sidecar if used); hosterm on the new machine shows the same hosts | Whole product | Shipped |
| UC6 | Operator hand-edits the raw config inside the app for directives the form doesn't cover | Raw config editor | Shipped |
| UC7 | New user runs `hosterm.exe` on a machine missing the optional OpenSSH client: the app detects missing `ssh`/`sftp` at launch and shows a one-screen OpenSSH install guide instead of opaque spawn errors | Startup check (W2-A) | Shipped (3a3b5a0) |
| UC8 | Operator sees the app in the Abyss Emerald theme: single-line grouped sidebar, kind-colored tabs, terminal well + status bar, SFTP transfer strip | Full UI (redesign Arus) | U1 shipped (33a1833); U2–U9 in flight |
| UC9 | Operator clicks "Check for updates"; the app verifies the downloaded installer against a baked minisign key before installing | Update flow (wave 2 M3) | Speced (FEATURE-SPEC-wave-2-draft §4/B) |

---

## 2. Personas

| Persona | Stake |
|---|---|
| **SSH operator (end user)** | Manages a handful-to-dozens of servers from a Windows desktop. Already lives in `~/.ssh/config`; rejects tools that hide or rewrite it. Wants terminal + file transfer + remote edits in one window. |
| **Multi-device operator** | Works from desktop and laptop; needs identical host setup everywhere without re-entering secrets into a vendor cloud. |
| **MEKL (solo maintainer)** | Ships with a verify loop (52 cargo tests, 75 smoke checks measured 19 Okt post-U1) and minimal Windows test time. Every release is publicly diffable. |
| **Adversary (hostile server / supply chain)** | Handles user key material and the release pipeline; must not get an XSS path to the IPC bridge (CSP wave 1) nor a silent-code-distribution channel (signing wave 2). |

---

## 3. User Stories & Acceptance Criteria (EARS)

### US-1: Host connection from `~/.ssh/config` (SSH operator)
As an operator I want every `Host` entry listed in a sidebar and editable through a form that writes back the file faithfully, so that connecting is one click and my hand-tuned config survives app edits.
- WHEN hosterm starts THE SYSTEM SHALL list one sidebar entry per `Host` block from `~/.ssh/config` within the NFR-6 budget for the config size present.
- WHEN the host form saves a host THE SYSTEM SHALL preserve comments, order, and unknown directives byte-for-byte outside the edited lines.
- WHEN a `Host *` (wildcard) block exists THE SYSTEM SHALL render it read-only and never rewrite it from the form.
- IF the existing config file is unreadable THEN THE SYSTEM SHALL refuse the write and leave the original file intact.

### US-2: Per-host auth as standard OpenSSH directives (SSH operator)
As an operator I want key or password auth expressed as plain OpenSSH directives, so that nothing proprietary enters my config.
- WHEN auth mode "key" is saved THE SYSTEM SHALL write `IdentityFile` and no auth-mode directives beyond it.
- WHEN auth mode "password" is saved THE SYSTEM SHALL write `PreferredAuthentications password` and `PubkeyAuthentication no`.
- WHEN a private key is pasted THE SYSTEM SHALL save it as `~/.ssh/<name>` with permissions 0600 (Unix) and list it in the key dropdown.
- IF a stored password exists for a host THEN THE SYSTEM SHALL keep it only in `~/.ssh/hosterm.secrets` (0600) and never write it into `~/.ssh/config`.

### US-3: Embedded real-binary terminal (SSH operator)
- WHEN a host is clicked THE SYSTEM SHALL open a terminal tab running the OS's real `ssh` binary in a PTY; no ssh-library substitutes.
- WHEN multi-line text is pasted into a PTY THE SYSTEM SHALL normalize CRLF→LF before the bytes reach the remote shell.
- WHEN 3 or more terminal tabs are opened THE SYSTEM SHALL keep every session interactive concurrently.

### US-4: SFTP browse and transfer (SSH operator)
- WHEN a host's SFTP tab is opened THE SYSTEM SHALL list the remote directory via the real `sftp` binary, honoring ssh config directives.
- WHEN a file is transferred on a key/agent-auth host THE SYSTEM SHALL use batch `sftp -b -` for the operation.
- WHEN a file is dragged between panes THE SYSTEM SHALL use manual mouse events (mousedown/mousemove/mouseup) and never HTML5 drag-and-drop.
- WHEN one transfer fails THE SYSTEM SHALL show its failure state with a retry affordance without clearing other transfers' rows.

### US-5: Password-auth hosts with persistent SFTP (legacy-host operator)
- WHEN a password host connects in the terminal THE SYSTEM SHALL prompt for the password in-terminal and write no secret into `~/.ssh/config`.
- WHEN a stored password exists (opt-in per host) THE SYSTEM SHALL auto-type it once at the prompt and share one persistent interactive `sftp` process per host across all tabs of that host.
- WHEN a second tab of the same password host opens SFTP THE SYSTEM SHALL reuse the existing session and never prompt for the password again.

### US-6: Remote editor (SSH operator)
- WHEN a file in the SFTP browser is clicked THE SYSTEM SHALL open it in a CodeMirror editor tab with line numbers and save back in place on Ctrl/Cmd+S.
- WHEN a text file ≤ 2 MB is opened THE SYSTEM SHALL load it fully for editing.
- IF a file is binary or > 2 MB THEN THE SYSTEM SHALL refuse to open it with a visible message before any load.

### US-7: Release and install (Windows user)
- WHEN a `v*` tag is pushed THE SYSTEM SHALL build NSIS `.exe` + WiX `.msi` installers on `windows-latest` and attach them to the release page.
- WHEN the app launches on a machine where `ssh.exe` or `sftp.exe` is absent THE SYSTEM SHALL detect the absence once per launch and show a blocking OpenSSH install guide with a "Continue anyway" escape.
- WHEN config surfaces (host form, raw editor, key paste) are used while binaries are missing THE SYSTEM SHALL keep them fully functional.
- IF a connect/SFTP attempt fails after the user declined the guide THEN THE SYSTEM SHALL show a per-attempt error naming the recovery path, with the raw spawn strings suppressed.

### US-8: Arus UI (redesign, Abyss Emerald) (SSH operator)
- WHEN the app renders THE SYSTEM SHALL use the 16+3 Abyss Emerald tokens (§6.5 redesign) with zero Tokyo Night literals in `src/`.
- WHEN a sidebar host has a live terminal tab THE SYSTEM SHALL show its status dot in `--ok`, and revert it to `--muted` when the last tab of that host closes.
- WHEN a tab is active, terminal pane focused, or a primary button rendered THE SYSTEM SHALL apply `var(--glow)` — and never on hover/idle states.
- WHEN a font is loaded THE SYSTEM SHALL use only local system fonts (zero CDN/webfont network requests).
- WHEN lists load (sidebar, SFTP local/remote) THE SYSTEM SHALL render static skeleton rows without layout jump, keeping the existing narrative empty/error copy byte-identical.
- WHEN an SFTP selection is made THE SYSTEM SHALL mark rows with `--sel` background plus a 1px `--accent` outline (bg alone fails non-text contrast).
- WHEN the editor buffer is dirty THE SYSTEM SHALL show "● unsaved" in the ed-bar and keep Ctrl+S working at panel level (focus anywhere inside the panel).
- IF the stored-password host dialog opens THEN THE SYSTEM SHALL show the `● stored (sidecar, 0600)` chip, the opt-in-per-host note, and a delete action confirmed via in-app dialog (never native confirm).

### US-9: Signed releases and user-initiated updates (wave 2, Windows user)
- WHEN a tagged build runs with signing secrets present THE SYSTEM SHALL Authenticode-sign both installers and emit `.sig` updater artifacts inside the build job.
- IF signing secrets are absent THEN THE SYSTEM SHALL fail the build before any artifact upload (fail-closed) and leave the previous release untouched.
- WHEN a user clicks "Check for updates" THE SYSTEM SHALL make exactly one HTTPS request to the single GitHub Releases feed endpoint, verify the download against the baked minisign key, and require explicit consent before install.
- IF the downloaded update fails signature verification or carries a version ≤ current THEN THE SYSTEM SHALL refuse atomically with zero filesystem side effects and the running app keeps working.
- WHEN the config file changes externally while the app is open (wave 2 §C) THE SYSTEM SHALL re-check mtime+size on every window-focus transition and refresh host lists without restart.
- IF the config is unreadable at a focus re-check THEN THE SYSTEM SHALL keep the last-good rendered state and show an error naming the config path.

### US-10: Maintainability for a solo maintainer (MEKL)
- WHEN any change is made THE SYSTEM SHALL pass the full verify loop (NFR-5 baselines) before staging.
- WHEN a new behavior ships THE SYSTEM SHALL have an explicit smoke check covering it.
- WHEN a working tree is staged THE SYSTEM SHALL contain no `target/|node_modules|.exe$|dist/` paths.

---

## 4. Functional Requirements

| ID | Requirement (EARS) | Prioritas |
|---|---|---|
| FR-A1 | WHEN hosterm starts THE SYSTEM SHALL list every `Host` block in the sidebar; wildcard blocks render read-only and are never rewritten | must |
| FR-A2 | WHEN the host form saves THE SYSTEM SHALL round-trip comments, order, and unknown directives verbatim (parser round-trip test) | must |
| FR-A3 | WHEN auth mode is saved THE SYSTEM SHALL write exactly the standard directives for key (`IdentityFile`) or password (`PreferredAuthentications password` + `PubkeyAuthentication no`) | must |
| FR-A4 | WHEN a key is pasted THE SYSTEM SHALL save it `~/.ssh/<name>` 0600 and refresh the key dropdown | must |
| FR-A5 | WHEN a config write runs THE SYSTEM SHALL write atomically (temp + rename) and refuse overwrite on unreadable original | must |
| FR-A6 | WHEN the raw editor saves THE SYSTEM SHALL round-trip through the same parser as FR-A2 | must |
| FR-A7 | WHEN `HOSTERM_CONFIG` is set THE SYSTEM SHALL use that path (smoke suite runs against a temp config) | must |
| FR-B1 | WHEN a host is clicked THE SYSTEM SHALL open an embedded xterm.js tab; ≥ 3 concurrent tabs supported in smoke | must |
| FR-B2 | WHEN a terminal spawns THE SYSTEM SHALL exec the OS's real `ssh` binary in a PTY (portable-pty); config directives honored | must |
| FR-B3 | WHEN text is pasted into a PTY THE SYSTEM SHALL normalize CRLF→LF (`pasteNormalize`) | must |
| FR-C1 | WHEN a SFTP tab opens THE SYSTEM SHALL render directory listings via the real `sftp` binary | must |
| FR-C2 | WHEN a transfer runs on key/agent auth THE SYSTEM SHALL use batch `sftp -b -` | must |
| FR-C3 | WHEN drag/copy interactions occur THE SYSTEM SHALL use manual mouse events only (`draggable` grep = 0 in `src/`) | must |
| FR-C4 | WHEN a password-host SFTP opens THE SYSTEM SHALL reuse one persistent interactive `sftp` per host across tabs; batch mode unchanged for key/agent auth | must |
| FR-D1 | WHEN a file is clicked in the SFTP browser THE SYSTEM SHALL open a CodeMirror tab (line numbers) and save back on Ctrl/Cmd+S | must |
| FR-D2 | IF a file is binary or > 2 MB THEN THE SYSTEM SHALL refuse to open it with a visible message | must |
| FR-E1 | WHEN a `v*` tag is pushed THE SYSTEM SHALL produce and attach `.exe` + `.msi` installers to the release (SHA-pinned actions, audit gates green) | must |
| FR-E2 | WHEN the app starts without `ssh`/`sftp` on PATH THE SYSTEM SHALL detect it once per launch and show the blocking OpenSSH guide with "Continue anyway" + re-check | must (shipped W2-A) |
| FR-E3 | WHEN a connect/SFTP attempt fails on a declined-guide machine THE SYSTEM SHALL show a per-attempt friendly error with raw spawn strings suppressed | must (shipped W2-A) |
| FR-G1 | WHEN the UI renders THE SYSTEM SHALL apply the Abyss Emerald token set; WHEN glow appears THE SYSTEM SHALL restrict it to `.tab.active`, `.term-side.focused`, `button.primary` | must (U1 shipped) |
| FR-G2 | WHEN the Arus sidebar ships THE SYSTEM SHALL render 28px single-line rows with group headers, context menu (Open SFTP · Edit · Delete), and Ctrl+K search focus | must (U2, in flight) |
| FR-G3 | WHEN the Arus tab bar ships THE SYSTEM SHALL color tabs by kind, scroll-activate via `scrollIntoView`, offer reconnect-on-disconnect, Close others/Close host, Ctrl+W / Ctrl+Shift+W | must (U3) |
| FR-G4 | WHEN the Arus terminal ships THE SYSTEM SHALL render the `--well` background, a 4px draggable split divider (manual mouse, dblclick = 50/50), and a global 24px status bar (`user@host · auth · N sesi`) | must (U4) |
| FR-G5 | WHEN the Arus SFTP ships THE SYSTEM SHALL show pane-head source/target chips, 28px rows, selection = `--sel` + 1px `--accent` outline, three transfer paths (manual drag, click-click, pane-head buttons), and a per-file transfer strip (max 3 rows + scroll) | must (U5) |
| FR-G6 | WHEN lists load THE SYSTEM SHALL show static skeleton rows with zero layout jump; WHEN editor is dirty THE SYSTEM SHALL show "● unsaved" panel-wide | must (U6/U7) |
| FR-G7 | WHEN the password dialog opens for a stored-password host THE SYSTEM SHALL show the stored chip, opt-in note, and sidecar-delete with in-app confirmation | must (U8) |
| FR-H1 | WHEN a tagged build runs with secrets present THE SYSTEM SHALL sign installers (Authenticode, Trusted Signing) and emit `.sig` + `latest.json`; IF secrets are absent THEN THE SYSTEM SHALL fail the build before upload | must (wave 2 M2) |
| FR-H2 | WHEN a user clicks "Check for updates" THE SYSTEM SHALL check exactly one endpoint, verify minisign against the baked pubkey, require consent, and refuse downgrades/invalid signatures atomically | must (wave 2 M3, after HR-W2-1) |
| FR-H3 | WHEN the window regains focus THE SYSTEM SHALL stat config mtime+size and refresh changed state without restart; IF unreadable THEN THE SYSTEM SHALL keep last-good state + named-path error | must (wave 2 §C) |
| FR-H4 | WHEN wave 2 M3 flips on the updater THE SYSTEM SHALL have a live release whose artifacts all carry `.sig` (HR-W2-1: sign first, updater second) | must |

---

## 5. Non-Functional Requirements

| ID | Requirement (TERUKUR: angka, batas, satuan) |
|---|---|
| NFR-1 | Secrets: 0 secret bytes in `~/.ssh/config` (grep gate in verify loop); sidecar perms 0600; pasted keys 0600. Verified every PR. |
| NFR-2 | CSP: `script-src 'self'` enforced in shipped builds; 0 CSP violations across the full smoke suite (count == 0 asserted) and recorded once on real Windows (SEC-5 waiver active — see §8). |
| NFR-3 | Privacy/zero background: 0 network requests at idle and during normal use except user-initiated SSH/SFTP; amendment pending MEKL (OQ-1): + exactly 1 user-initiated HTTPS GET + 1 download to the Releases endpoint from wave 2 M3. Measured by smoke network log (SDD §E.8.6). |
| NFR-4 | Config integrity: `~/.ssh/config` parseable by REAL ssh at all times; round-trip fidelity asserted; failed writes never destroy the file. 52 cargo tests green every PR. |
| NFR-5 | Regression gate (baselines measured 19 Okt 2026, post-U1; re-measure at each dispatch): `cargo test --lib` → 52 pass; `npm run build` → 0 TS errors; smoke suite → 75 pass (≥ baseline + N per epic unit); playwright cleanup after runs. |
| NFR-6 | Performance targets: UI feedback (tab open, listing render, editor open) ≤ 200 ms; config parse of 1,000 `Host` blocks ≤ 100 ms; connect latency dominated by ssh/network, not app. Binding when first measured (resolution Q5: measure on first complaint; wave-2 NFR-W2-9 check ≤ 5 s measured once at M3). |
| NFR-7 | Usability: single-window workflow (sidebar + tabs cover terminal/files/editing); all drag via manual mouse; skeleton loading without layout jump (list height delta = 0 px); glow restricted to 3 active-state selectors; `prefers-reduced-motion` respected (no new keyframes). |
| NFR-8 | Portability: full setup movable by copying `~/.ssh/config` (+ keys + optional sidecar); 0 registry/proprietary-store writes. Fresh-machine checklist passes with 1 file + keys. |
| NFR-9 | Supply chain (wave 1, shipped): 100% of workflow `uses:` refs pinned to full 40-hex SHAs (grep = 0 mutable refs); both audit gates fail on high+critical in prod deps; `contents: write` scoped to the release job only. |
| NFR-10 | Release latency: tag→published release including audit+sign steps ≤ baseline + 5 min (wave-1 budget; re-verify at M2 when signing joins the build job). |

---

## 6. Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Verify-loop green rate | 100% of merged changes: cargo 52+, build 0 errors, smoke 75+ | CI/task logs per task; STATE.md baseline re-measured each dispatch |
| CSP violations | 0 across every smoke run | Smoke console listener counter == 0 |
| Secret leakage | 0 hits: secrets in `~/.ssh/config`; private-key markers in repo | grep gates in verify loop + pre-stage check |
| CI supply chain | 100% pinned action refs; 0 mutable | `grep -nE "uses:\s*\S+@(v[0-9]+…)"` = 0 matches |
| Release completeness | Every tag ≥ 2 installers attached (≥ 5 files incl. `.sig`×2 + `latest.json` from M2) | Release page inventory per tag |
| Regression escapes | 0 shipped bugs breaking the 5 hard invariants (STATE.md) | Invariant smoke checks + manual Windows pass before each tag |
| UX unit acceptance | Each Arus unit U1–U9 adds its contracted new smoke checks (U1 +4, U2 ≥5, U3 ≥4, U4 ≥4, U5 ≥6, U6 ≥2, U7 ≥2, U8 ≥3; total ≥ baseline + 25 by U9) | Smoke suite count per unit task |
| Update path trust (post-M3) | 100% of updater installs signature-verified; 0 installs from failed verification (structural: plugin verification non-disableable) | Workflow gates + tamper fixtures (SDD §E.8.5) |

---

## 7. Epics & Milestones

Living-doc rule: FEATURE-SPEC-*/EPIC-TSD-* implement this PRD; this section tracks their status. Nothing enters "Next" without a feature spec; nothing enters "Now" without verify-loop coverage.

| Epic / Milestone | Scope | Status | Reference |
|---|---|---|---|
| Security hardening wave 1 | CSP enforcement + dev-server CSP header (R1); CI action SHA pinning + token scope (R2/R4); dependency audit gates (R3) | **Shipped — v0.10.0** (bb93631; SEC-4 CSP 6f5ce3f + ae60f28; SEC-1/2 bb6e1fc; withGlobalTauri=false 5339962) | docs/PRD-security-hardening-wave-1.md · docs/EPIC-TSD-security-hardening-wave-1.md · docs/SPIKE-csp-findings.md |
| Password-SFTP persistent sessions | One interactive `sftp` per host shared across tabs; sidecar-stored password auto-type; sidecar copy fix | **Shipped — v0.10.0** (8b36f2b, 6ce0381) | docs/STATE.md §layout; PRD v1 US-D2 |
| W2-A startup ssh/sftp missing-check + OpenSSH guide | Launch-time binary detection, blocking guide + re-check, per-attempt refusal errors | **Shipped — post-tag, rides next release** (3a3b5a0) | docs/FEATURE-SPEC-startup-ssh-check.md · FEATURE-SPEC-wave-2-draft §3 |
| Redesign Arus U1 — Abyss Emerald theme | 16+3 token swap, xterm/CodeMirror remap, glow on 3 active selectors, offline fonts, +4 smoke checks | **Shipped — post-tag** (33a1833) | EPIC-TSD-redesign-arus-v1.md §U1 · redesign §6.5 |
| Redesign Arus U2–U9 | Sidebar (U2, dispatch in progress), tab bar (U3), terminal well/divider/status bar (U4), SFTP strip/selection (U5), skeletons (U6), editor dirty/Ctrl+S (U7), password dialog chips (U8), smoke canonization (U9) | **In flight** (U2 running on working tree; U1 done; units dispatched serially — hotspot `src/styles.css`/`src/main.ts`) | docs/EPIC-TSD-redesign-arus-v1.md §2 · docs/UX/redesign-hosterm-v1.md §2/§6.5 |
| Wave 2 B — signing (M2) then updater (M3) | Authenticode via Trusted Signing + minisign artifacts, fail-closed gates, `latest.json` feed; then user-initiated verified updater | **Speced — awaiting MEKL approval** (draft uncommitted, commit deferred §10) | docs/FEATURE-SPEC-wave-2-draft.md §4 · docs/SDD-hosterm-v1-extension-signing-updater.md |
| Wave 2 C — config auto-reload on focus | mtime+size stat on window focus, no watcher daemon | **Speced** (same draft; HR-W2-2) | docs/FEATURE-SPEC-wave-2-draft.md §5 |
| Parking lot | SBOM per release; 21-site `innerHTML` refactor (CSP is the compensating control until then); first-class Linux/macOS installers; terminal font size, go-to-line (v1.1 options) | **Backlog — no spec yet** | PRD v1 §5 · redesign §6 out-of-scope |

Milestone order (binding): next release carries W2-A + U1 + U2–U9 as they land; M2 (signing) precedes M3 (updater) — HR-W2-1, not reopenable.

---

## 8. Dependencies & Risks

**Dependencies:** Microsoft Edge WebView2 runtime (end user); OpenSSH client binaries present on the OS — optional on Windows 10/11, mitigated by the shipped W2-A startup check (UC7); GitHub Actions `windows-latest` runner; from M2: Azure Trusted Signing account with completed identity validation (lead-time risk — start account setup before M2 dispatch); from M3: none new (single GitHub Releases endpoint).

| Risk | Impact | Mitigation / status |
|---|---|---|
| SEC-5: CSP behavior on runtime WebView2 unverified (Chromium-based spike + smoke only) | Medium | **WAIVED** by MEKL 19 Okt — risk accepted; follow-up: self-hosted Windows runner + tauri-driver for automated matrix (OQ-3) |
| Sidecar create-then-chmod micro-window (`fs::write` before chmod 0600) | Low | **INFO, deferred** (lead, t_6608fa42): file born in `~/.ssh` (0700); fix `OpenOptions::mode(0o600).create(true)` in a future security touch (OQ-2) |
| Arus hotspot: `src/styles.css` + `src/main.ts` touched by every U1–U9 unit | Medium | Units dispatch strictly serially; never two cards on the same files (EPIC §0) |
| Manual-mouse interactions (U4 divider, U5 drag) not fully covered by Playwright | Medium | One manual Windows (WebView2) pass before next release tag, batched with signed-release checklist (OQ-4) |
| Single-maintainer bus factor; no 4-eyes on releases | Accepted | Every release publicly diffable; keys-not-host trust anchor from M2 (SDD §E.5.2 T1) |
| CodeMirror `style-mod` vs CSP | Closed | Resolved in wave-1 spike; `'unsafe-inline'` scoped to `style-src` only (TSD Q1) |

**Out of scope (unchanged from v1):** any vault beyond the 0600 sidecar; ssh2/russh-style libraries; telemetry/accounts/cloud sync; background or silent auto-updates (user-initiated check only, FR-H2); mobile/web clients; rewriting OpenSSH config semantics; bundling ssh/sftp binaries (SDD ADR-1 — never).

---

## OPEN QUESTIONS (wajib, masing-masing + rekomendasi pm)

1. **NFR-3 amendment (wave-2 D2/OQ-B1):** the update check breaks "no network calls except user-initiated SSH/SFTP". Amend NFR-3 now or keep footnote in the wave-2 spec? — rekomendasi: **amend in this PRD at MEKL approval of the wave-2 draft** (NFR-3 already carries the pending wording above); footnote-only leaves a permanent spec-vs-PRD contradiction every future audit re-trips over.
2. **Sidecar 0600-at-birth:** fix the create-then-chmod micro-window now or with the next security touch? — rekomendasi: **next security touch, not a standalone release**; one-line `OpenOptions` change riding a wave that already pays verify-loop cost. Not urgent: file lives in a 0700 directory, local-access risk only.
3. **SEC-5 waiver retirement:** when do we add a self-hosted Windows runner + tauri-driver CSP matrix? — rekomendasi: **after wave 2 M2** — the signing pipeline makes Windows CI value higher (signed-release verification can reuse the runner), so one infra investment serves both; before that the manual pre-tag pass suffices.
4. **Manual Windows pass batching (U4/U5 mouse + signed-release checklist + CSP matrix):** separate sessions or one? — rekomendasi: **one batched manual session immediately before the next release tag**; solo-maintainer time is the scarcest resource and all three gates block the same tag.
5. **Wave-2 draft approval timing:** review the merged draft (t_803099fc) now or after U2–U9 land? — rekomendasi: **after U9** — one moving tree at a time; the draft is text-only and its §10 commit deferral already prevents tree interference, but MEKL review attention is also serial.
6. **Perf NFR-6 measurement:** bind the ≤ 200 ms / ≤ 100 ms targets now that smoke is 75 checks? — rekomendasi: **keep the Q5 resolution (bind on first complaint)**; functional regression coverage exists, and premature timing gates rot CI for a solo maintainer. Wave-2 NFR-W2-9's one-off ≤ 5 s check stays the only scheduled timing measurement.

---

*Sources: docs/STATE.md · docs/PRD-hosterm-v1.md (superseded) · docs/PRD-security-hardening-wave-1.md · docs/EPIC-TSD-security-hardening-wave-1.md · docs/FEATURE-SPEC-security-hardening-wave-1.md · docs/FEATURE-SPEC-startup-ssh-check.md · docs/FEATURE-SPEC-wave-2-draft.md · docs/SDD-hosterm-v1-extension-signing-updater.md · docs/EPIC-TSD-redesign-arus-v1.md · docs/UX/redesign-hosterm-v1.md §2/§6.5 · git log (33a1833, 3a3b5a0, bb93631, 8b36f2b, 6ce0381, 5339962, 6f5ce3f, ae60f28, bb6e1fc). Read 2026-10-19.*
