# hosterm — WAVE 2 Feature Spec (DRAFT — merged, not committed)

| Field | Value |
|---|---|
| Status | **DRAFT — PM-reviewed (root card t_803099fc, 19 Okt). Text only; commit deferred (§10); MEKL approval pending (§8).** |
| Doc type | Feature spec covering three wave-2 features (one document, per merge task t_cf0e7adf) — NOT the product PRD |
| Merges | Signing→auto-update draft (t_3aadacdf) · Startup ssh/sftp check draft (t_f879edb2, `docs/FEATURE-SPEC-startup-ssh-check.md`) · Config mtime-reload draft (t_4d06aa0e) |
| Decisions of record | PRD §6.2 (Q2, Q4, Q6 — resolved by MEKL 19 Okt) |
| Numbering | Unified scheme: `US-W2-n`, `FR-W2-n`, `NFR-W2-n` (sequential, zero collisions); ACs per section `AC-A/B/C<n>.<k>`; hard constraints `HR-W2-1/2`. Old→new map: §9 |
| Counts | 11 user stories · 14 FR · 11 NFR · 54 AC |
| Feeds | Root card t_803099fc (WAVE 2 feature spec, PARKED) |

---

## 1. Scope & how to read this document

Three independent features, one wave, one document. Sections are ordered by release flexibility:

- **Section A — Startup ssh/sftp missing-check + OpenSSH guide (Q2).** Smallest, ships early in wave 2.
- **Section B — Code signing gates auto-update (Q4).** Two stages with a **hard release-order constraint (HR-W2-1)**: signing ships first; the updater is enabled only after a fully signed release exists.
- **Section C — Config auto-reload via mtime-check-on-focus (Q6).** Independent of A and B; no CI/release surface. Carries hard constraint **HR-W2-2** (no watcher/daemon/polling).

Format everywhere: user stories → functional requirements with acceptance criteria → NFRs with measure/target → sequencing. Full per-requirement PRD §x + SDD §y traceability in §6. Open questions (each with a recommendation) in §8.

Out of scope for the whole document: implementation choices (SDD/TSD land), Linux/macOS signing, background/silent auto-updates, SBOM generation (parked, PRD §5), bundling of ssh/sftp binaries (SDD ADR-1 — never).

---

## 2. Cross-feature decisions & contradiction check

Conflicts between the three draft sections were cross-checked at merge. Resolutions below are binding for all three sections.

| # | Topic | Decision (binding) | What it resolves |
|---|---|---|---|
| D1 | Release order vs early ship | **HR-W2-1 gates only updater enablement, not releases.** Section A (Q2) may ship inside *unsigned* installers (current FR-E1 state) before Section B lands; A has no updater dependency. No contradiction between A and B. | Prevents misreading "sign first" as blocking the Q2 small feature. |
| D2 | Network surface | PRD NFR-3 ("no network calls except user-initiated SSH/SFTP") gains exactly one exception from this wave: the **user-initiated** update check (FR-W2-8). PRD NFR-3 wording must be amended — tracked as OQ-B1 with recommendation. | NFR-B/SIG-1 (now NFR-W2-6) vs PRD NFR-3, flagged by the signing draft. |
| D3 | Background work | Zero-background posture holds across the wave: B = user-initiated check only, no polling (FR-W2-8); C = stat-on-focus only, no watcher/daemon/timer (NFR-W2-10 / HR-W2-2). Consistent with PRD §1.1 + NFR-3. | Aligns the two "no background activity" constraints; they reinforce, not conflict. |
| D4 | Caching semantics differ by design | Binaries: detected **once per launch**, cached for launch lifetime (FR-W2-1); re-check is explicit (FR-W2-4). Config: mtime re-checked **on every focus transition** (FR-W2-11). Different objects, different staleness tolerance — not a contradiction; do not "harmonize" them. | SSH draft's "no per-connection re-checks" vs reload draft's "check per focus". |
| D5 | No-restart recovery is the wave's shared UX rule | Installing OpenSSH mid-session needs no app restart (FR-W2-4); external config edits need no restart (FR-W2-11). Both sections state it; keep both. | Consistency check, no action. |
| D6 | Raw errors never reach the user | A: friendly per-attempt error, raw spawn strings suppressed (FR-W2-3). C: last-good state + named-path error on unreadable config (FR-W2-14). Both follow SDD ADR-8 hygiene. | Consistency check, no action. |

---

## 3. Section A — Startup ssh/sftp missing-check + OpenSSH install guide (Q2)

### 3.1 Problem statement

hosterm's founding principle is spawning the OS's real `ssh`/`sftp` binaries (PRD §1.1; SDD ADR-1). On Windows 10/11 the OpenSSH **Client** is an *optional* feature: a machine can lack `ssh.exe`/`sftp.exe` entirely (PRD §5 Dependencies; SDD §5 R-3). On such a machine the app launches, shows hosts, and every connect/SFTP attempt dies inside `pty_spawn`/`run_sftp_op` with an opaque spawn error (lib.rs:1287 `map_err(|e| e.to_string())`) — first-launch failure with no path to recovery. UC7 (new user, download-and-run) breaks at step one.

**Non-problem (explicit):** config editing (host form, raw editor, key paste) touches only files under `~/.ssh/` and needs no binaries — it must keep working when they are missing.

### 3.2 Goals / Non-goals

**Goals**
- G-A1: Missing binaries are detected at app launch, before the user wonders why connect fails.
- G-A2: A single guide screen turns "broken app" into a 2-minute OS fix (Windows Settings path + copyable PowerShell command).
- G-A3: Declining the guide leaves the app usable for everything that doesn't need binaries, with a clear per-attempt recovery path.
- G-A4: Installing mid-session is picked up without an app restart.

**Non-goals**
- No bundling of ssh/sftp binaries (SDD ADR-1; PRD §6.2 Q2) — ever, in this feature.
- No auto-install or elevation from within the app (security surface; OS owns the install).
- No version/feature probing of the binaries in v1 — presence only.
- No per-connection re-checks of on-disk presence after launch (result cached for the launch lifetime; re-check is explicit or error-triggered — see D4).
- No Linux/macOS special-casing (binaries are near-universal there; the check still runs harmlessly).

### 3.3 User stories

| ID | Story | Traceability |
|---|---|---|
| US-W2-1 | As a Windows user on a fresh machine, I want hosterm to tell me at launch that the OpenSSH client is missing and show me exactly how to install it, so that first run ends in a working app instead of a cryptic error. | PRD §1.3 UC7, §2 US-B2/US-C1 · SDD ADR-1, §5 R-3 |
| US-W2-2 | As the same user, I want the install steps to be copy-pasteable (one PowerShell command) with a "Re-check" button, so that I can fix the OS and confirm the fix without restarting or googling. | PRD §6.2 Q2 · SDD ADR-1 |
| US-W2-3 | As an operator who doesn't want to install OpenSSH right now, I want to dismiss the screen and still edit my `~/.ssh/config` hosts in the app, so that preparation work isn't blocked. | PRD §1.1, §6.2 Q2 · SDD §2.2 |
| US-W2-4 | As that operator, when I do try to connect, I want the error to name the real cause and offer the install guide, so that the app never leaves me debugging a spawn error string. | PRD §2 US-B2 · SDD ADR-1, §5 R-3 |

### 3.4 Functional requirements + acceptance criteria

**FR-W2-1 — Launch-time detection of `ssh` + `sftp`**
At every app launch, after config load, hosterm resolves both binaries using the **same lookup rules the spawn path uses**: `PATH` search, then `%SystemRoot%\System32\OpenSSH\`; for `sftp`, a set `HOSTERM_SFTP_BIN` override replaces the lookup (mirrors lib.rs:721/856 so tests stay honest). Result is one of: `both present`, `missing: ssh`, `missing: sftp`, `missing: both` — held for the launch lifetime. No processes are spawned for detection, no network. Non-Windows: same logic, System32 step skipped.
*Refs: PRD §5, §6.2 Q2 · SDD ADR-1, §5 R-3, §3.3*

- AC-A1.1: Detection logic is a pure function over (PATH dirs, system dir, env override) → unit-testable on Linux CI; tests cover all four result states incl. the `HOSTERM_SFTP_BIN` override.
- AC-A1.2: Machine with both binaries: guide screen never appears, and no guide-related UI flash occurs at launch.
- AC-A1.3: Machine missing one binary only: result names the missing one specifically (not a generic "missing").
- AC-A1.4: Detection adds ≤ 200 ms to launch and runs without blocking config parse/sidebar render (PRD NFR-6 budget).
- AC-A1.5: Verify loop stays green with smoke suite extended (≥ 2 new checks: missing-binary state simulated, present-binary no-op).

**FR-W2-2 — Guide screen (content + flow)**
When detection reports anything missing, the app shows a single blocking-but-dismissible screen instead of the main workflow:
1. **What's wrong:** plain statement naming the missing binary/binaries ("OpenSSH client (`ssh.exe`) not found on this machine").
2. **Why it matters:** one line — hosterm runs the OS's real ssh/sftp; without them, connect and file transfer cannot work.
3. **Install path A (GUI):** Windows Settings → Apps → Optional features → "OpenSSH Client" → Install (Windows 10 1809+ and 11).
4. **Install path B (PowerShell, copyable):** `Add-WindowsCapability -Online -Name OpenSSH.Client~~~~0.0.1.0` — one-click copy button, with a note it needs an elevated PowerShell.
5. **"Re-check" button** (FR-W2-4).
6. **"Continue anyway"** dismissal (FR-W2-3).
7. Link to troubleshooting docs (non-load-bearing; steps above must suffice offline).
*Refs: PRD §6.2 Q2 · SDD ADR-1*

- AC-A2.1: All 7 content elements present; PowerShell command renders as exact, copyable text (copy button writes the byte-exact command to the clipboard).
- AC-A2.2: Screen is shown at most once per launch unless Re-check re-evaluates it; no stacking/duplication.
- AC-A2.3: Manual Windows matrix (once per release): steps A and B verified against current Windows 11 Settings UI.
- AC-A2.4: Screen renders correctly in the existing CSP posture (no new `connect-src`/`script-src` needs; any new markup routes through `esc()`/`escAttr()` — SDD ADR-8 rule).

**FR-W2-3 — Fallback when the user declines the guide**
"Continue anyway" dismisses the screen to the normal app. Config-only features (sidebar, host form, raw editor, key paste) work unchanged. Any binary-dependent action (terminal connect, SFTP list/transfer/editor open) returns a friendly per-attempt error: cause + what to do + an **"Open install guide"** button that re-opens the FR-W2-2 screen. The launch-time detection result is not silently refreshed by these attempts.
*Refs: PRD §1.1, §6.2 Q2 · SDD §2.2, ADR-1, §5 R-3*

- AC-A3.1: After dismissal: host add/edit/delete, raw config edit, key paste all succeed with binaries still missing.
- AC-A3.2: A connect attempt shows the friendly error (not the raw spawn error string) with a working "Open install guide" button.
- AC-A3.3: Raw spawn error strings (e.g. `No such file or directory`) never surface to the user in this state.
- AC-A3.4: No data loss path: dismissal never writes to `~/.ssh/config`.

**FR-W2-4 — Re-check behavior**
"Re-check" on the guide screen re-runs the FR-W2-1 detection immediately, in place: if both binaries now resolve, the screen closes and the app proceeds normally — **no app restart required** (user installs via Settings while hosterm stays open). If still missing, the screen stays as-is (no reflow/flicker, no duplicate screen). Re-check is also available from the per-attempt error state (FR-W2-3). Detection result updates to the fresh outcome.
*Refs: PRD §6.2 Q2 · SDD §5 R-3, §3.4 flow 1*

- AC-A4.1: Install via PowerShell → click Re-check → guide closes and a terminal tab can be opened without restart (manual Windows matrix).
- AC-A4.2: Re-check with binaries still missing: screen content unchanged, exactly one guide screen in the DOM.
- AC-A4.3: Re-check completes ≤ 200 ms (local resolution only); button shows busy state while running.
- AC-A4.4: Re-check never spawns `ssh`/`sftp` (presence only, per Non-goals).

### 3.5 Non-functional requirements (Section A)

| ID | Requirement | Measure / verification | Traceability |
|---|---|---|---|
| NFR-W2-1 | Detection makes no network calls and spawns no processes; feature adds no CSP relaxations | Code audit: no outbound calls; smoke CSP-violation counter stays 0 | PRD NFR-3, NFR-2 · SDD ADR-6, ADR-8 |
| NFR-W2-2 | Detection is fast and non-blocking | ≤ 200 ms, off the config-parse path (PRD NFR-6 budget unchanged) | PRD NFR-6 |
| NFR-W2-3 | No bundling, no elevation, no OS modification from within the app | Code audit: only read/lookup operations; guide content is instructions, not actions | PRD §6.1, §6.2 Q2 · SDD ADR-1 |
| NFR-W2-4 | Detection testable without Windows or real binaries | Pure-function unit tests on Linux CI using fake dirs + `HOSTERM_SFTP_BIN`-style override; smoke suite extended ≥ 2 checks | PRD NFR-5, FR-A7 · SDD §3.3, §6 |
| NFR-W2-5 | Feature carries zero telemetry about detection outcomes | Code audit (PRD NFR-3 invariant) | PRD NFR-3 |

### 3.6 Sequencing (Section A)

- Wave 2, **early** (Q2 priority); gated only on wave-1 hardening shipping (PRD §5 roadmap rule; NFR-2).
- Hard dependency: none outside existing app structure. Reuses the existing IPC command pattern (SDD §2.3) — one new read-only command is an implementation detail for architect/TSD.
- Cross-feature: independent of B and C except shared doc numbering and D1 (may ship in unsigned installers before signing lands).

---

## 4. Section B — Code signing gates auto-update (Q4)

### 4.1 Hard constraint + rationale

**HR-W2-1 (hard requirement, no exceptions):** code signing ships FIRST; the auto-update path is enabled ONLY AFTER a fully signed release has been produced and verified in production. Rationale (PRD §6.2 Q4, resolved): *auto-update without signed artifacts is a supply-chain downgrade* — an unsigned updater converts the release channel into a silent remote-code-execution distribution channel: any compromise of the release pipeline (GitHub token, runner, action dependency) would push executable code to every user with no user-side trust anchor. Signing first establishes the trust anchor (Authenticode + minisign public key baked into the app); the updater then inherits that anchor and can never distribute code the key did not sign. This closes the same risk class tracked as SDD §5 R-9 (release integrity) and extends wave-1 CI hardening (SHA-pinned actions, audit gates — PRD NFR-2).

**Enforcement is structural, not procedural:** the ordering is enforced in CI and config, not by convention. The updater plugin stays disabled in `tauri.conf.json` until the signing pipeline has produced a signed tagged release; the release job fails if any artifact is unsigned (FR-W2-6/FR-W2-7 ACs). A reviewer cannot "accidentally" merge the updater before signing — the pipeline refuses.

**SDD gap (PM-flagged for lead/architect):** SDD v1 §2.4 (Delivery & packaging) contains no signing/updater content — Section B's SDD refs (`§2.4`, `§5 R-9`, ADR-5) are landing zones, not existing contracts. Shipping Section B requires an SDD §2.4 extension (signing step in `build-windows.yml`, updater config + `latest.json` contract, key custody) before or with the wave-2 TSD (§10 prerequisite 2).

Per D1: HR-W2-1 gates the **updater**, not ordinary releases — Section A (Q2) is not blocked by this constraint.

### 4.2 User stories

| ID | Story | Traceability |
|---|---|---|
| US-W2-5 | As a Windows user, I want installers that are Authenticode-signed, so that SmartScreen/AV warnings stop training me to click through security dialogs on the hosterm download. | PRD §2 US-F2 (dependency note), §5 |
| US-W2-6 | As a Windows user, I want the app to tell me a new release exists and update itself only from artifacts whose signature verifies, so that I never run stale builds with known CVEs and never install tampered code. | PRD §2 US-F2; §6.2 Q4 |
| US-W2-7 | As the maintainer, I want CI to refuse publishing any release with unsigned artifacts, so that the supply-chain order (sign → update) cannot be violated by accident or by a compromised partial pipeline. | SDD §2.4, §5 R-9; PRD §3.1 FR-E1 |

### 4.3 Functional requirements + acceptance criteria — Stage A: signing pipeline (ships first)

**FR-W2-5 — Key material exists and is protected**
(a) Authenticode code-signing certificate (OV minimum; EV preferred for SmartScreen reputation) with hardware token or CI-hosted HSM; (b) Tauri updater minisign keypair generated with `tauri signer generate`. Private halves live ONLY as GitHub Actions secrets / HSM; the minisign **public** key is embedded in `tauri.conf.json` (`plugins.updater.pubkey`) and committed.
*Refs: SDD §2.4, §3.2 (secrets never in tracked files), ADR-5 · PRD §6.2 Q4*

- AC-B1.1: Key ceremony recorded in docs (who, where, expiry).
- AC-B1.2: Repo grep finds no private key material (`BEGIN PRIVATE KEY`, sig private key strings) in verify loop.
- AC-B1.3: CI logs for a tagged build contain no key material (secrets masked).

**FR-W2-6 — `build-windows.yml` signs every shipped artifact on `v*` tag**
NSIS `.exe` + WiX `.msi` via `signtool`/HSM signing step, then generates Tauri updater artifacts (`.sig` + `latest.json`) with `createUpdaterArtifacts: true`. Release publish depends on the sign job.
*Refs: PRD §3.1 FR-E1; §5 AC-5 extension · SDD §2.4*

- AC-B2.1: A tagged build attaches `.exe`, `.msi`, `.sig`, `latest.json` to the release, all signatures present.
- AC-B2.2: `signtool verify /pa` passes on both installers on a clean Windows VM.
- AC-B2.3: Tamper test — flipping one byte in a downloaded `.exe`/`.sig` pair makes updater verification fail (covered by QA tamper fixture).

**FR-W2-7 — Unsigned-release gate**
The release/publish job hard-fails when signing inputs (cert secrets, minisign secret key) are absent, and `createUpdaterArtifacts` is enabled only when signing config is present. No path exists to attach unsigned artifacts to a public release.
*Refs: SDD §5 R-9 · PRD §6.2 Q4 (structural enforcement of HR-W2-1)*

- AC-B3.1: Dry-run CI without cert secrets → job fails before publish, release page unchanged.
- AC-B3.2: Audit of workflow shows publish step strictly `needs: [sign]`.
- AC-B3.3: Smoke/docs assert `plugins.updater` absent or disabled in `tauri.conf.json` until Stage B feature flag flips.

### 4.4 Functional requirements + acceptance criteria — Stage B: update channel verification (enabled only after Stage A is live)

**FR-W2-8 — User-initiated update check only**
"Check for updates" menu action (v1 of this feature): HTTPS GET of `latest.json` from GitHub Releases, version compare, then user-consented download. No background polling, no silent installs in this scope.
*Refs: PRD §3.2 NFR-3 (amended per D2/OQ-B1); §2 US-F2*

- AC-B4.1: Smoke suite network log shows zero outbound requests from the app at idle and during normal terminal/SFTP/editor use.
- AC-B4.2: Check triggers exactly one HTTPS request to the Releases endpoint; UI shows result (up-to-date / new version) within 5 s.

**FR-W2-9 — Signature verification gates install**
Downloaded artifact + `latest.json` must verify against the baked-in minisign public key before any install step runs; version comparison must be a strict forward update. Verification failure = no download install, no config change.
*Refs: PRD §6.2 Q4; §2 US-F2 · SDD §5 R-9*

- AC-B5.1: Valid signed pair → update installs, app restarts on new version, config/`~/.ssh` state intact.
- AC-B5.2: Fixture tests: (i) signature from a different key → refuse; (ii) truncated artifact → refuse; (iii) downlevel version in signed `latest.json` → refuse.
- AC-B5.3: Public key is compiled into the binary/config and matches FR-W2-5 record.

**FR-W2-10 — Failure behavior: atomic refusal**
When signature invalid or network fails: update refused atomically — current version keeps running, user sees explicit error ("signature verification failed; update not installed"), error logged locally, no partial writes anywhere (no temp installers left, no modified config). App remains fully functional; retry allowed.
*Refs: PRD §6.2 Q4; §3.2 NFR-4 spirit (never destroy working state) · SDD §2.4*

- AC-B6.1: Tampered `latest.json` (CI fixture) → refusal message shown, app functional afterward, zero filesystem side effects (assert via smoke pre/post snapshot of install dir + `~/.ssh`).
- AC-B6.2: Offline / 404 / TLS-failure → same refusal path, no crash, retry works when network returns.
- AC-B6.3: Updater never auto-launches an unverified binary (code-review + fixture).

### 4.5 Non-functional requirements (Section B)

| ID | Requirement | Measure / verification | Traceability |
|---|---|---|---|
| NFR-W2-6 | Network surface stays minimal: only user-initiated HTTPS to `github.com` Releases endpoint; no telemetry, no third-party update servers | Code audit + smoke network log: no outbound traffic outside user-initiated check | PRD §3.2 NFR-3 (as amended per D2) |
| NFR-W2-7 | Key-compromise runbook exists: revoke cert, rotate minisign keypair, rotate = shipped pubkey change = new minor release with signed migration note | Runbook doc reviewed in this wave; one tabletop rotation drill executed | SDD §5 R-9; PRD §5 AC-5 |
| NFR-W2-8 | Verify loop unchanged and green: `cargo test --lib` → 48+, `npm run build` clean, smoke suite extended for update-refusal fixtures | Existing gates per AGENTS.md pass on every PR in this feature | PRD §3.2 NFR-5; §4 AC-2 |
| NFR-W2-9 | Update check is non-blocking and bounded: UI never freezes; check/verify completes or times out ≤ 5 s with explicit error | Smoke timing assert; manual pass on Windows | PRD §3.2 NFR-6 (targets bind per OQ-B2 / PRD §6.2 Q5) |

### 4.6 Sequencing (Section B)

1. Stage A (FR-W2-5 → 6 → 7) lands and one tagged release ships fully signed. **Only then** Stage B work begins.
2. Stage B (FR-W2-8 → 9 → 10) flips the updater on in a later release; that release itself must be signed (chicken-and-egg resolved: the trust anchor from Stage A signs the release that introduces the updater).
3. Depends on: wave-1 CI hardening complete (SHA-pinned actions R2, audit gates R3 — PRD NFR-2); v0.9.0 shipped (wave-2 gate, PRD §6.2 Q4).
4. **HR-W2-1 restated as release gate:** no release may enable the updater unless the previous release (or the same release) carries verified Authenticode + minisign signatures. CI enforces; AC-B3.1/AC-B3.2 are the test.

---

## 5. Section C — Config auto-reload via mtime-check-on-focus (Q6)

### 5.1 Problem statement

`~/.ssh/config` is the single source of truth (PRD §1.1) and users hand-edit it with real editors, dotfile managers, or another machine's sync (UC5). Today hosterm reads the config once at startup and on its own editor actions; an external edit made while the app is open is invisible until restart or sidebar re-open (PRD §6.2 Q6, current workaround). The user sees stale hosts, connects to a host that no longer exists, or misses a new one.

The fix must not cost a resident file-watcher: the product runs from a single `.exe` with zero background footprint (PRD §1.1, NFR-3 posture). Hence: check file mtime **when the window regains focus** — a moment the user is about to look at the sidebar anyway.

### 5.2 Goals / Non-goals

**Goals**
- G-C1: Host list (sidebar) reflects external config edits without app restart.
- G-C2: Detection costs nothing while the app is unfocused or idle — zero CPU between focus events.
- G-C3: User's in-flight edits (dirty host form, open raw editor) are never silently discarded by a reload.
- G-C4: Unreadable/corrupt config never destroys the last-good in-app state (extends FR-A5 / NFR-4).

**Non-goals**
- No file-watcher daemon, no inotify/FSEvents/ReadDirectoryChangesW subscription, no polling timer (NFR-W2-10 / HR-W2-2 — hard product constraint).
- No reload of *running* sessions: live PTY terminals and SFTP sessions keep the config they were spawned with (SDD §3.4 flows 1–2); reloading re-reads the file, it does not reconfigure ssh processes.
- No merge of external + in-app concurrent edits (conflict = user decides, FR-W2-13).
- No reload of the secrets sidecar — it is already read live at use time, never cached across focus (SDD §3.2).
- No watching of identity files (`~/.ssh/<name>`) — only the config file.

### 5.3 User stories

| ID | Story | Traceability |
|---|---|---|
| US-W2-8 | As an SSH operator, when I edit `~/.ssh/config` in my editor and switch back to hosterm, I want the sidebar to already show the change, so that I never connect to a stale host list. | PRD §6.2 Q6, §1.3 UC5 · SDD §3.1 |
| US-W2-9 | As a multi-device operator (UC5), when my config-sync tool rewrites the file while hosterm is open, I want the new host set on next focus, so that switching machines mid-day needs no restart. | PRD §6.2 Q6, §1.3 UC5 · SDD §3.1 |
| US-W2-10 | As an SSH operator, when I have an unsaved host-form edit or an open raw-editor buffer and an external change lands, I want hosterm to warn me and let me choose, so that my typing is never silently overwritten. | PRD §2 US-A2, §3.1 FR-A2/A6 · SDD ADR-4 |
| US-W2-11 | As an SSH operator, when my config is temporarily unreadable or broken (sync mid-write, bad syntax), I want hosterm to keep showing my last-known-good hosts and tell me, so that a half-written file never blanks my sidebar. | PRD §3.1 FR-A5; §3.2 NFR-4 · SDD §3.1 |

### 5.4 Functional requirements + acceptance criteria

**FR-W2-11 — mtime check on focus**
Check the config file's mtime (and size) every time the app window transitions unfocused→focused, and once at startup. One metadata `stat` per focus transition; no timers.
*Refs: PRD §6.2 Q6 · FR-A7 (`HOSTERM_CONFIG` honored by the check) · SDD §3.1, §3.3*

- AC-C1.1: With a temp config via `HOSTERM_CONFIG` (SDD §3.3), external modification of that file followed by a window-focus event updates the sidebar host list without app restart — asserted in the smoke suite by dispatching a focus event and re-reading the sidebar DOM.
- AC-C1.2: Between focus events the app performs **zero** config-file reads — verifiable by code inspection (no `setInterval`, no watcher registration in the diff) plus a smoke-suite counter of config reads during an idle 5 s window == baseline.
- AC-C1.3: Focus events that do not follow an unfocused state (e.g. refocus within the same window, OS notifications stealing focus) still fire at most one check per transition, not per event burst — smoke asserts check count == 1 across 3 rapid focus/unfocus cycles.
- AC-C1.4: Startup check present: app launch reads the config exactly as today (no behavioral change) — covered by existing suite.

**FR-W2-12 — what reloads on detected change**
When mtime/size differs from the last-known values: re-parse the config and re-render — (a) sidebar host list (FR-A1 semantics: wildcard blocks stay read-only), (b) identity-file key dropdown options, (c) raw config editor **content** if open and clean (per OQ-C2). Running terminal/SFTP sessions are untouched.
*Refs: PRD §3.1 FR-A1, FR-A5; §6.2 Q6 · SDD §2.2, §3.1, §3.4*

- AC-C2.1: Adding a `Host` block externally → new sidebar entry appears on next focus; deleting one → entry gone; renaming one → renamed. Smoke asserts all three against the temp config.
- AC-C2.2: A new `IdentityFile` target added externally appears in the host-form key dropdown after reload (smoke: open form, assert option present).
- AC-C2.3: With a live terminal tab open, an external config change + focus does **not** spawn, kill, or alter any PTY session (smoke asserts session map length unchanged, terminal content untouched).
- AC-C2.4: Round-trip fidelity after reload holds: subsequently editing a reloaded host through the form preserves comments/order/unknown directives (existing FR-A2 cargo round-trip tests stay green, extended with a reload-then-edit case).

**FR-W2-13 — conflict handling (external edit vs in-flight edit)**
If the user has unsaved state at reload time: dirty host form for an unchanged host → form untouched, reload proceeds; dirty host form for a host whose block changed externally → reload proceeds for the sidebar, form keeps user's values, non-blocking banner names the conflict and offers "reload form from file" / "keep mine"; open raw editor with unsaved changes → editor content is NOT auto-replaced, banner offers "reload from disk" / "keep editing"; save afterwards overwrites the external change (last-writer-wins, user-informed). No silent loss in any path.
*Refs: PRD §2 US-A2; §3.1 FR-A2, FR-A6 · SDD §3.1 (read refuses clobber), ADR-4*

- AC-C3.1: Smoke: dirty the host form, externally change a *different* host, focus → form values intact, no banner, other host updated.
- AC-C3.2: Smoke: dirty the host form for host X, externally change host X's block, focus → form still shows user's values, conflict banner rendered, both banner buttons work (keep-mine → save path writes user's version; reload-form → form shows file version).
- AC-C3.3: Smoke: type in raw editor, externally modify file, focus → editor buffer unchanged, banner shown; "reload from disk" swaps content to the external version and clears dirty state; "keep editing" + save results in the user's version on disk (assert file bytes).
- AC-C3.4: No code path in the reload can write to the config file — the feature is read-only against the file (grep the diff: no write/rename calls reachable from reload; existing FR-A5 atomic-write tests unchanged).

**FR-W2-14 — unreadable / corrupt config on check**
If the stat fails (file missing, EACCES) or the re-parse fails (invalid content): keep the last-good in-memory model and UI, surface a non-blocking error naming the path and reason, and retry the check on the next focus. Never blank the sidebar, never write a replacement file, never disable editing silently.
*Refs: PRD §3.1 FR-A5; §3.2 NFR-4 · SDD §3.1 (unreadable file refuses rather than overwrite), §2.2*

- AC-C4.1: Smoke: point `HOSTERM_CONFIG` at the temp file, delete it, focus → sidebar shows previous hosts, error message visible containing the path, app stays interactive (can still open terminals with old host data).
- AC-C4.2: Smoke: externally write an unparseable config (e.g. truncate mid-directive in a way the model rejects), focus → same last-good behavior; restore a valid file, focus again → sidebar recovers to the new content without restart (recovery path asserted).
- AC-C4.3: Error state clears on the first successful reload (no stale error persisted after recovery) — smoke asserts error element absent post-recovery.
- AC-C4.4: Unreadable-file behavior composes with the existing read-refusal guard: an unreadable file can never be overwritten by hosterm (FR-A5 unit test for the unreadable case still passes).

### 5.5 Non-functional requirements (Section C)

**NFR-W2-10 — no watcher, no daemon, no polling (hard constraint; = HR-W2-2)**
Auto-reload MUST be implemented as an on-demand mtime check executed inside the existing renderer/backend request path on focus transitions. MUST NOT: spawn any background process or thread that outlives a check; register OS filesystem-watch APIs (inotify, FSEvents, ReadDirectoryChangesW, `watchdog`/`notify`-style crates); run any timer/poll loop. Idle app cost = zero scheduled work. This is a product invariant (single-exe, zero-background-footprint posture, PRD §1.1/NFR-3) and outranks detection latency.
*Refs: PRD §1.1, §3.2 NFR-3, §6.2 Q6 · SDD §2.2*

- AC-C5.1: Code review gate: no fs-watch API import, no `std::thread`/`tokio::spawn` dedicated to watching, no interval timer in the diff — reviewer checklist item added to the PR template for this feature.
- AC-C5.2: Observable: with the app running and unfocused, no CPU time is consumed by config detection — verified once manually via Task Manager / `top` (documented in PR), since no automated probe exists for "nothing runs".
- AC-C5.3: Staleness bound is documented in the UI help/README: config changes are picked up at the next focus, not instantly; the known staleness window = time the window stays unfocused. Replaces the current "restart or re-open sidebar" wording from PRD §6.2 Q6.

**NFR-W2-11 — cost per focus check**
One focus transition performs at most one `stat` and, on change, one config read+parse. Parse of a 1,000-`Host` config stays within the existing NFR-6 budget (≤ 100 ms); a focus check on an unchanged file adds no parse at all.
*Refs: PRD §3.2 NFR-6 · SDD §3.1*

- AC-C6.1: Smoke (temp config, 1,000 hosts): unchanged-file focus transition adds no parse (read-counter delta == 0); changed-file focus completes sidebar re-render within NFR-6's 100 ms budget — timing asserted in the smoke run (first binding measurement; per PRD §6.2 Q5 this converts NFR-6's target into a measured one for this path only).

### 5.6 Sequencing (Section C)

- Wave 2 item per PRD §6.2 Q6; sequencing relative to B (Q4) is independent — this feature touches only the config read path, no CI/release surface.
- Requires no new dependencies (fs-watch crates explicitly excluded by NFR-W2-10).
- README/help text update is part of delivery (AC-C5.3), replacing the interim "restart" workaround wording.
- PRD §5 "Later" row ("config auto-reload on external change") is promoted to "Next (spec exists)" once MEKL approves this merged draft — PM follow-up, one-line PRD edit.

---

## 6. Unified traceability matrix (PRD §x + SDD §y)

Every requirement in this document. PRD = `docs/PRD-hosterm-v1.md`; SDD = `docs/SDD-hosterm-v1.md`.

| ID | PRD ref | SDD ref |
|---|---|---|
| US-W2-1 | §1.3 UC7, §2 US-B2/US-C1 | §1.1 areas B/C, ADR-1, §5 R-3 |
| US-W2-2 | §6.2 Q2 | ADR-1 |
| US-W2-3 | §1.1, §6.2 Q2 | §2.2 (host management UI) |
| US-W2-4 | §2 US-B2 | ADR-1, §5 R-3 |
| US-W2-5 | §2 US-F2, §5 | §2.4 |
| US-W2-6 | §2 US-F2, §6.2 Q4 | §2.4, §5 R-9 |
| US-W2-7 | §3.1 FR-E1, §6.2 Q4 | §2.4, §5 R-9 |
| US-W2-8 | §6.2 Q6, §1.3 UC5 | §3.1 |
| US-W2-9 | §6.2 Q6, §1.3 UC5 | §3.1 |
| US-W2-10 | §2 US-A2, §3.1 FR-A2/A6 | ADR-4 |
| US-W2-11 | §3.1 FR-A5, §3.2 NFR-4 | §3.1 |
| FR-W2-1 | §5, §6.2 Q2 | ADR-1, §5 R-3, §3.3 |
| FR-W2-2 | §6.2 Q2 | ADR-1 |
| FR-W2-3 | §1.1, §6.2 Q2 | §2.2, ADR-1, §5 R-3 |
| FR-W2-4 | §6.2 Q2 | §5 R-3, §3.4 |
| FR-W2-5 | §6.2 Q4 | §2.4, §3.2, ADR-5 |
| FR-W2-6 | §3.1 FR-E1, §5 | §2.4 |
| FR-W2-7 | §6.2 Q4 | §5 R-9 |
| FR-W2-8 | §3.2 NFR-3 (as amended, D2), §2 US-F2 | §2.4 |
| FR-W2-9 | §2 US-F2, §6.2 Q4 | §5 R-9 |
| FR-W2-10 | §6.2 Q4, §3.2 NFR-4 (spirit) | §2.4 |
| FR-W2-11 | §6.2 Q6, §3.1 FR-A7 | §3.1, §3.3 |
| FR-W2-12 | §3.1 FR-A1/A5, §6.2 Q6 | §2.2, §3.1, §3.4 |
| FR-W2-13 | §2 US-A2, §3.1 FR-A2/A6 | §3.1, ADR-4 |
| FR-W2-14 | §3.1 FR-A5, §3.2 NFR-4 | §3.1, §2.2 |
| NFR-W2-1 | §3.2 NFR-3, NFR-2 | ADR-6, ADR-8 |
| NFR-W2-2 | §3.2 NFR-6 | — (bounded by ADR-1 spawn path) |
| NFR-W2-3 | §6.1, §6.2 Q2 | ADR-1 |
| NFR-W2-4 | §3.2 NFR-5, §3.1 FR-A7 | §3.3, §6 |
| NFR-W2-5 | §3.2 NFR-3 | ADR-1 |
| NFR-W2-6 | §3.2 NFR-3 (as amended, D2/OQ-B1) | §2.4 |
| NFR-W2-7 | §5 AC-5 | §5 R-9 |
| NFR-W2-8 | §3.2 NFR-5, §4 AC-2 | §6 |
| NFR-W2-9 | §3.2 NFR-6, §6.2 Q5 | §2.4 |
| NFR-W2-10 | §1.1, §3.2 NFR-3, §6.2 Q6 | §2.2 |
| NFR-W2-11 | §3.2 NFR-6, §6.2 Q5 | §3.1 |

---

## 7. Wave-2 release-order & dependency plan

| Milestone | Content | Gate | Constraint |
|---|---|---|---|
| M0 | Wave-1 hardening shipped (PRD NFR-2: CSP R1, SHA-pinning R2, audit gates R3, token scope R4) | Wave-1 feature spec acceptance | Prerequisite for all wave-2 items |
| M1 | Section A (Q2 ssh/sftp check) | M0 only | May ship in **unsigned** installers (D1) — HR-W2-1 does not apply |
| M2 | Section B Stage A (FR-W2-5..7) | M0 + **v0.9.0 shipped with assets** (PRD §6.2 Q4) | Produces first fully signed release |
| M3 | Section B Stage B (FR-W2-8..10) | M2: one signed tagged release live | **HR-W2-1 release gate**: updater enabled only after signed release exists; CI-enforced (AC-B3.1/.2) |
| M4 | Section C (Q6 mtime reload) | M0 only | Independent of M1–M3; any slot in wave 2 (recommended after M1 to keep PRs small) |

Shared across all: verify loop green on every PR (`cargo test --lib` → 48+, `npm run build`, smoke suite extended per section); PRD NFR-3 amendment (OQ-B1) must land with/before M3.

---

## 8. OPEN QUESTIONS (each with recommendation)

### Section A
| # | Question | Recommendation |
|---|---|---|
| OQ-A1 | Should the guide screen block the whole app, or sit as an interstitial banner over a usable app? | **Blocking screen with "Continue anyway".** Missing binaries make 4 of 5 core surfaces dead; a banner invites clicking connect into the same dead end. Blocking + escape hatch is one screen, zero extra state. |
| OQ-A2 | Version probing (`ssh -V`) to detect ancient/removed OpenSSH builds? | **No, v1 presence-only.** `ssh -V` output parsing is locale-fragile and Windows 10 1809+ builds are fine for hosterm's use. Revisit only with a real user report. |
| OQ-A3 | Per-attempt error (FR-W2-3): auto-trigger Re-check before showing the error, or show error immediately? | **Show error immediately + manual re-check inside the guide.** Auto-recheck per failed connect hides the real error for seconds on genuinely broken machines; manual is predictable and testable. |
| OQ-A4 | External link target for troubleshooting (Microsoft docs URL)? | **Static in-app steps primary; optional external link secondary.** Steps must work offline (machines missing OpenSSH may also have restricted networks — that's often *why* they lack it). |
| OQ-A5 | Should Linux/macOS builds skip the check entirely? | **Run it, skip the guide.** Same code path, and on dev machines a missing `sftp` is a real (if rare) condition; the guide text is Windows-specific but the detection is not. |

### Section B
| # | Question | Recommendation |
|---|---|---|
| OQ-B1 | PRD NFR-3 wording ("no network calls except user-initiated SSH/SFTP") conflicts with the user-initiated update check. Amend PRD NFR-3, or footnote in this spec only? | **Amend PRD NFR-3** (one-line PRD edit at MEKL approval: add "and the user-initiated update check (wave 2)"). Footnote-only leaves a permanent spec-vs-PRD contradiction that every future audit re-trips over. |
| OQ-B2 | Do NFR-W2-9's ≤ 5 s update-check bounds become CI-measured now, or per PRD §6.2 Q5 ("on first complaint")? | **Manual/one-off measurement at M3, no CI timing gate** — same resolution as PRD Q5 for NFR-6. Smoke asserts the refusal path functionally; the timing assert runs once and is recorded in the PR. |

### Section C
| # | Question | Recommendation |
|---|---|---|
| OQ-C1 | Focus granularity: window focus only, or also tab-switch/visibility (user reads sidebar while "in" the app)? | Window focus only for v1 of the feature. Tab switches don't change what's on disk; checking there doubles the trigger surface for zero expected gain. Revisit on first complaint. |
| OQ-C2 | Does a detected change also refresh the raw editor when it is open but *clean*? | Yes (FR-W2-12c as written) — clean buffer has no user state to lose, and a stale raw view that then gets saved would clobber the external edit. This is the only case auto-replace is safe. |
| OQ-C3 | Compare mtime alone, or mtime+size? | mtime+size — two integers, same `stat` call, catches same-timestamp rewrites some sync tools produce. Exact comparison fields are the lead's call in SDD; the requirement is "detect realistic external modifications". |
| OQ-C4 | Manual refresh button as an escape hatch? | No, not in this feature. Focus check covers the real workflow (you just came back from your editor). A button is UI surface to test; add later only if focus detection proves unreliable on some platform. |
| OQ-C5 | Windows file-locking edge: editor holds the config open while hosterm stats it? | Out of PM scope to resolve — flag to lead/architect: `stat` on a file open by another Windows process generally succeeds (no mandatory lock), but the SDD should state the behavior when read-while-open occurs. Filed here so it isn't discovered mid-implementation. |

---

## 9. ID mapping (draft-local → unified)

Old draft IDs are preserved here so reviewer comments against the three source drafts stay resolvable.

| Draft ID | This doc | Draft ID | This doc |
|---|---|---|---|
| US-SSH-1..4 | US-W2-1..4 | AC-SIG-1a/1b/1c | AC-B1.1/.2/.3 |
| US-SIG-1..3 | US-W2-5..7 | AC-SIG-2a/2b/2c | AC-B2.1/.2/.3 |
| US-AR1..4 | US-W2-8..11 | AC-SIG-3a/3b/3c | AC-B3.1/.2/.3 |
| FR-SSH-1..4 | FR-W2-1..4 | AC-SIG-4a/4b | AC-B4.1/.2 |
| FR-SIG-1..6 | FR-W2-5..10 | AC-SIG-5a/5b/5c | AC-B5.1/.2/.3 |
| FR-AR1..4 | FR-W2-11..14 | AC-SIG-6a/6b/6c | AC-B6.1/.2/.3 |
| NFR-SSH-1..5 | NFR-W2-1..5 | AC-AR1.1..4 | AC-C1.1..4 |
| NFR-SIG-1..4 | NFR-W2-6..9 | AC-AR2.1..4 | AC-C2.1..4 |
| NFR-AR1 | NFR-W2-10 (+ HR-W2-2) | AC-AR3.1..4 | AC-C3.1..4 |
| NFR-AR2 | NFR-W2-11 | AC-AR4.1..4 | AC-C4.1..4 |
| HR-SIG-1 | HR-W2-1 | AC-AR5.1..3 | AC-C5.1..3 |
| — | HR-W2-2 (= NFR-W2-10) | AC-AR6.1 | AC-C6.1 |

AC numbering rule: `AC-<section><FR-index-within-section>.<k>`; section NFRs with ACs continue the section's index sequence (AC-C5.x, AC-C6.x). Source drafts' letter-suffixed AC IDs (AC-SIG-2a) are renumbered numeric only.

---

## 10. Commit deferral (explicit)

**This document is DRAFT TEXT ONLY. No `git add`, no `git commit`, no repo files modified by this merge task.**

Commit is deferred until ALL of the root card's PARKED prerequisites hold (t_803099fc; PRD §6.2 Q4):

1. SDD produk v1 done (t_bfe6e9f5).
2. Release lead has produced the product TSD.
3. v0.9.0 shipped with assets.

Only after all three: the lead/orchestrator commits, in one docs commit — never by PM (PM git policy: no commit, no push, ever). Contents: this merged spec + `docs/FEATURE-SPEC-startup-ssh-check.md` (in-repo source draft). The signing draft (t_3aadacdf) and mtime-reload draft (t_4d06aa0e) live only in worker scratch / board attachments (scratch is pruned after 24 h idle) — include them only if still retrievable; this merged doc supersedes all three either way (old→new ID map: §9).
