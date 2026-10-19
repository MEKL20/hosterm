# Feature Spec (DRAFT section) — Startup ssh/sftp missing-check + OpenSSH install guide

> **Status:** Draft text only — NOT committed. Merge target: wave-2 feature-spec doc (task t_cf0e7adf).
> **Priority:** Q2 — small, may ship early in wave 2 (after wave-1 hardening).
> **Naming note for merger:** this section uses locally-prefixed IDs (`US-SSH-*`, `FR-SSH-*`, `NFR-SSH-*`) to avoid collisions with sibling sections (signing/auto-update, config mtime reload). Renumber into the unified scheme at merge; keep this section's cross-refs.
> **Origin:** PRD §6.2 Q2 (resolved by MEKL, 19 Okt): add a startup check + one-screen install pointer for the OpenSSH client. **No bundling** — that drags in update/security surface (see SDD ADR-1).

---

## 1. Problem statement

hosterm's founding principle is spawning the OS's real `ssh`/`sftp` binaries (PRD §1.1; SDD ADR-1). On Windows 10/11 the OpenSSH **Client** is an *optional* feature: a machine can lack `ssh.exe`/`sftp.exe` entirely (PRD §5 Dependencies; SDD §5 R-3). On such a machine the app launches, shows hosts, and every connect/SFTP attempt dies inside `pty_spawn`/`run_sftp_op` with an opaque spawn error (lib.rs:1287 `map_err(|e| e.to_string())`) — first-launch failure with no path to recovery. UC7 (new user, download-and-run) breaks at step one.

**Non-problem (explicit):** config editing (host form, raw editor, key paste) touches only files under `~/.ssh/` and needs no binaries — it must keep working when they are missing.

## 2. Goals / Non-goals

**Goals**
- G1: Missing binaries are detected at app launch, before the user wonders why connect fails.
- G2: A single guide screen turns "broken app" into a 2-minute OS fix (Windows Settings path + copyable PowerShell command).
- G3: Declining the guide leaves the app usable for everything that doesn't need binaries, with a clear per-attempt recovery path.
- G4: Installing mid-session is picked up without an app restart.

**Non-goals**
- No bundling of ssh/sftp binaries (SDD ADR-1; PRD Q2) — ever, in this feature.
- No auto-install or elevation from within the app (security surface; OS owns the install).
- No version/feature probing of the binaries in v1 — presence only.
- No per-connection re-checks of on-disk presence after launch (result cached for the launch lifetime; re-check is explicit or error-triggered).
- No Linux/macOS special-casing (binaries are near-universal there; the check still runs harmlessly).

## 3. User stories

- **US-SSH-1:** As a Windows user on a fresh machine, I want hosterm to tell me at launch that the OpenSSH client is missing and show me exactly how to install it, so that first run ends in a working app instead of a cryptic error. *(PRD UC7, US-B2/US-C1; SDD ADR-1, R-3)*
- **US-SSH-2:** As the same user, I want the install steps to be copy-pasteable (one PowerShell command) with a "Re-check" button, so that I can fix the OS and confirm the fix without restarting or googling. *(PRD Q2)*
- **US-SSH-3:** As an operator who doesn't want to install OpenSSH right now, I want to dismiss the screen and still edit my `~/.ssh/config` hosts in the app, so that preparation work isn't blocked. *(PRD §1.1 config-as-source-of-truth; §6.2 Q2)*
- **US-SSH-4:** As that operator, when I do try to connect, I want the error to name the real cause and offer the install guide, so that the app never leaves me debugging a spawn error string. *(PRD US-B2; SDD ADR-1)*

## 4. Functional requirements

### FR-SSH-1 — Launch-time detection of `ssh` + `sftp`

**WHAT:** At every app launch, after config load, hosterm resolves both binaries using the **same lookup rules the spawn path uses**: `PATH` search, then `%SystemRoot%\System32\OpenSSH\`; for `sftp`, a set `HOSTERM_SFTP_BIN` override replaces the lookup (mirrors lib.rs:721/856 so tests stay honest). Result is one of: `both present`, `missing: ssh`, `missing: sftp`, `missing: both` — held for the launch lifetime. No processes are spawned for detection, no network. Non-Windows: same logic, System32 step skipped.

**Traceability:** PRD §5 (OpenSSH dependency), §6.2 Q2 · SDD ADR-1 (spawn dependency), §5 R-3, §3.3 (env overrides).

**AC:**
1. Detection logic is a pure function over (PATH dirs, system dir, env override) → unit-testable on Linux CI; tests cover all four result states incl. the `HOSTERM_SFTP_BIN` override.
2. Machine with both binaries: guide screen never appears, and no guide-related UI flash occurs at launch.
3. Machine missing one binary only: result names the missing one specifically (not a generic "missing").
4. Detection adds ≤ 200 ms to launch and runs without blocking config parse/sidebar render (PRD NFR-6 budget).
5. Verify loop stays green with smoke suite extended (≥ 2 new checks: missing-binary state simulated, present-binary no-op).

### FR-SSH-2 — Guide screen (content + flow)

**WHAT:** When detection reports anything missing, the app shows a single blocking-but-dismissible screen instead of the main workflow:

1. **What's wrong:** plain statement naming the missing binary/binaries ("OpenSSH client (`ssh.exe`) not found on this machine").
2. **Why it matters:** one line — hosterm runs the OS's real ssh/sftp; without them, connect and file transfer cannot work.
3. **Install path A (GUI):** Windows Settings → Apps → Optional features → "OpenSSH Client" → Install (Windows 10 1809+ and 11).
4. **Install path B (PowerShell, copyable):** `Add-WindowsCapability -Online -Name OpenSSH.Client~~~~0.0.1.0` — one-click copy button, with a note it needs an elevated PowerShell.
5. **"Re-check" button** (FR-SSH-4).
6. **"Continue anyway"** dismissal (FR-SSH-3).
7. Link to troubleshooting docs (non-load-bearing; steps above must suffice offline).

**Traceability:** PRD §6.2 Q2 (one-screen pointer, no bundling) · SDD ADR-1 (rejected alternative: bundled binary).

**AC:**
1. All 7 content elements present; PowerShell command renders as exact, copyable text (copy button writes the byte-exact command to the clipboard).
2. Screen is shown at most once per launch unless Re-check re-evaluates it; no stacking/duplication.
3. Manual Windows matrix (once per release): steps A and B verified against current Windows 11 Settings UI.
4. Screen renders correctly in the existing CSP posture (no new `connect-src`/`script-src` needs; any new markup routes through `esc()`/`escAttr()` — SDD ADR-8 rule).

### FR-SSH-3 — Fallback when the user declines the guide

**WHAT:** "Continue anyway" dismisses the screen to the normal app. Config-only features (sidebar, host form, raw editor, key paste) work unchanged. Any binary-dependent action (terminal connect, SFTP list/transfer/editor open) returns a friendly per-attempt error: cause + what to do + an **"Open install guide"** button that re-opens the FR-SSH-2 screen. The launch-time detection result is not silently refreshed by these attempts.

**Traceability:** PRD §1.1 (config = source of truth, binary-free), §6.2 Q2 · SDD §2.2 (host-management UI needs no binaries), ADR-1, R-3.

**AC:**
1. After dismissal: host add/edit/delete, raw config edit, key paste all succeed with binaries still missing.
2. A connect attempt shows the friendly error (not the raw spawn error string) with a working "Open install guide" button.
3. Raw spawn error strings (e.g. `No such file or directory`) never surface to the user in this state.
4. No data loss path: dismissal never writes to `~/.ssh/config`.

### FR-SSH-4 — Re-check behavior

**WHAT:** "Re-check" on the guide screen re-runs the FR-SSH-1 detection immediately, in place: if both binaries now resolve, the screen closes and the app proceeds normally — **no app restart required** (user installs via Settings while hosterm stays open). If still missing, the screen stays as-is (no reflow/flicker, no duplicate screen). Re-check is also available from the per-attempt error state (FR-SSH-3). Detection result updates to the fresh outcome.

**Traceability:** PRD §6.2 Q2 · SDD §5 R-3, §3.4 flow 1 (spawn path unchanged).

**AC:**
1. Install via PowerShell → click Re-check → guide closes and a terminal tab can be opened without restart (manual Windows matrix).
2. Re-check with binaries still missing: screen content unchanged, exactly one guide screen in the DOM.
3. Re-check completes ≤ 200 ms (local resolution only); button shows busy state while running.
4. Re-check never spawns `ssh`/`sftp` (presence only, per Non-goals).

## 5. Non-functional requirements

| ID | Requirement | Measure / target | Traceability |
|---|---|---|---|
| NFR-SSH-1 | Detection makes no network calls and spawns no processes; feature adds no CSP relaxations | Code audit: no outbound calls; smoke CSP-violation counter stays 0 | PRD NFR-3, NFR-2 · SDD ADR-6, ADR-8 |
| NFR-SSH-2 | Detection is fast and non-blocking | ≤ 200 ms, off the config-parse path (PRD NFR-6 budget unchanged) | PRD NFR-6 |
| NFR-SSH-3 | No bundling, no elevation, no OS modification from within the app | Code audit: only read/lookup operations; guide content is instructions, not actions | PRD §6.1, §6.2 Q2 · SDD ADR-1 |
| NFR-SSH-4 | Detection testable without Windows or real binaries | Pure-function unit tests on Linux CI using fake dirs + `HOSTERM_SFTP_BIN`-style override; smoke suite extended ≥ 2 checks | PRD NFR-5, FR-A7 pattern · SDD §3.3, §6 |
| NFR-SSH-5 | Feature carries zero telemetry about detection outcomes | Code audit (PRD NFR-3 invariant) | PRD NFR-3 |

## 6. Traceability matrix

| ID | PRD ref | SDD ref |
|---|---|---|
| US-SSH-1 | §1.3 UC7, §2 US-B2/US-C1 | §1.1 areas B/C, ADR-1, §5 R-3 |
| US-SSH-2 | §6.2 Q2 | ADR-1 |
| US-SSH-3 | §1.1, §6.2 Q2 | §2.2 (host management UI) |
| US-SSH-4 | §2 US-B2 | ADR-1, §5 R-3 |
| FR-SSH-1 | §5, §6.2 Q2 | ADR-1, §5 R-3, §3.3 |
| FR-SSH-2 | §6.2 Q2 | ADR-1 |
| FR-SSH-3 | §1.1, §6.2 Q2 | §2.2, ADR-1, R-3 |
| FR-SSH-4 | §6.2 Q2 | §5 R-3, §3.4 |
| NFR-SSH-1 | NFR-3, NFR-2 | ADR-6, ADR-8 |
| NFR-SSH-2 | NFR-6 | — (bounded by ADR-1 spawn path) |
| NFR-SSH-3 | §6.1, §6.2 Q2 | ADR-1 |
| NFR-SSH-4 | NFR-5, FR-A7 | §3.3, §6 |
| NFR-SSH-5 | NFR-3 | ADR-1 |

## 7. Dependencies & sequencing

- **Wave:** 2 (early — Q2 priority), gated on wave-1 hardening shipping (PRD §5 roadmap rule; AC-3).
- **Hard dependency:** none outside existing app structure. Reuses existing IPC command pattern (SDD §2.3) — one new read-only command is an implementation detail for architect/TSD.
- **Release-order constraint (merger):** independent of signing/auto-update and mtime-reload sections; no cross-constraints except shared doc numbering.

## 8. OPEN QUESTIONS

| # | Question | Recommendation |
|---|---|---|
| OQ-1 | Should the guide screen block the whole app, or sit as an interstitial banner over a usable app? | **Blocking screen with "Continue anyway".** Missing binaries make 4 of 5 core surfaces dead; a banner invites clicking connect into the same dead end. Blocking + escape hatch is one screen, zero extra state. |
| OQ-2 | Version probing (`ssh -V`) to detect ancient/removed OpenSSH builds? | **No, v1 presence-only.** `ssh -V` output parsing is locale-fragile and Windows 10 1809+ builds are fine for hosterm's use. Revisit only with a real user report. |
| OQ-3 | Per-attempt error (FR-SSH-3): auto-trigger Re-check before showing the error, or show error immediately? | **Show error immediately + manual re-check inside the guide.** Auto-recheck per failed connect hides the real error for seconds on genuinely broken machines; manual is predictable and testable. |
| OQ-4 | External link target for troubleshooting (Microsoft docs URL)? | **Static in-app steps primary; optional external link secondary.** Steps must work offline (machines missing OpenSSH may also have restricted networks — that's often *why* they lack it). |
| OQ-5 | Should Linux/macOS builds skip the check entirely? | **Run it, skip the guide.** Same code path, and on dev machines a missing `sftp` is a real (if rare) condition; the guide text is Windows-specific but the detection is not. |
