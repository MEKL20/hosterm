# hosterm — TSD v1 (Product Dev Playbook)

| Field | Value |
|---|---|
| Author | crew-coding-lead |
| Date | 2026-10-19 |
| Status | Active playbook for all hosterm dev tasks |
| Inputs | `docs/PRD-hosterm-v1.md` (product truth) · `docs/SDD-hosterm-v1.md` (design truth, untracked — lands in the wave-2 docs commit) · `docs/FEATURE-SPEC-wave-2-draft.md` (wave-2 scope) · `docs/STATE.md` + `AGENTS.md` (repo working rules) · `docs/EPIC-TSD-security-hardening-wave-1.md` (wave-1 gates, shipped) |
| Role of this doc | HOW we build and ship: locked stack, test baselines, rollout, dev rules. NOT a design doc — behavior/architecture lives in SDD; product scope lives in PRD. |
| Commit policy | This file ships in the single deferred docs commit (wave-2 draft §10); no commits by the authoring task. |

---

## 1. Document chain & precedence

```
PRD v1  ──what/why──►  SDD v1  ──how it fits together──►  TSD v1 (this)  ──how we execute──►  kanban tasks
```

- **Conflict rule:** product scope → PRD wins; design/behavior → SDD wins; process, test baselines, gates, release procedure → this TSD wins.
- Every kanban task body must cite `PRD §x` and/or `SDD §y` (+ feature-spec ID for wave-2 work). A task that can't cite its source is scope creep — reject at triage.
- Feature work enters "Next" only with a feature spec (PRD §5 roadmap rule); enters "Now" only with verify-loop coverage (§3 here).

---

## 2. Locked stack

All versions below are **read from the lockfiles** (2026-10-19), not from manifest ranges. `package-lock.json` / `src-tauri/Cargo.lock` are the binding record; the table is a convenience mirror.

| Layer | Component | Locked | Manifest | Bump policy |
|---|---|---|---|---|
| Shell | Tauri (Rust) | 2.12.1 | `^2` | Minor/patch via Dependabot + audit gates; major → spike + SDD ADR update first |
| Shell | `@tauri-apps/api` / `cli` | 2.12.1 | `^2` | With Tauri crate, same PR |
| PTY | portable-pty | 0.8.1 | `0.8` | Pin-major; watch reaper/kill behavior on bump |
| Clipboard | arboard | 3.6.1 | `3` | Routine |
| Paths | dirs | 5.0.1 | `5` | Routine |
| Serde | serde | 1.0.229 | `1` | Routine (lockfile-only churn) |
| Terminal UI | @xterm/xterm + addon-fit | 5.5.0 / 0.10.0 | `^5.5.0` / `^0.10.0` | Bump only with full verify + manual Windows pass (PTY/CSP-adjacent) |
| Editor | codemirror | 6.0.2 | `^6.0.2` | Bump re-runs the CSP spike check if CSS delivery changes (SDD ADR-6 / R-4) |
| Build | vite | 8.3.3 | `^8.0.16` | Minor/patch routine; anything touching HTML/CSS emission → CSP spike re-run |
| Types | typescript | 6.0.3 | `~6.0.3` | Tilde-locked; bump = its own task |
| QA | playwright | **1.63.0 exact** | — (`--no-save`) | Never float; smoke determinism depends on one engine version |

**Frozen strings (change = security review, not a normal PR):**
- CSP in `tauri.conf.json` (`app.security.csp`) — any edit re-runs `docs/SPIKE-csp-findings.md` analysis + smoke CSP counter. `'unsafe-inline'` stays scoped to `style-src` only.
- `withGlobalTauri: false` — never re-enable (SEC-6).
- CI action pins in `.github/workflows/build-windows.yml` — SHA-pinned (R2); Dependabot may propose SHA updates, never tag-only downgrades.

**Never add:** ssh2/russh-style libraries (ADR-1), fs-watch crates (wave-2 HR-W2-2), any telemetry/network client (PRD NFR-3). New dependency = task states why stdlib/existing dep can't do it.

---

## 3. Testing strategy

### 3.1 Baselines (measured 2026-10-19, this working tree)

| Lane | Command | Baseline | Evidence |
|---|---|---|---|
| Rust unit | `cd src-tauri && cargo test --lib` | **49 passed, 0 failed** | Run today |
| TS build | `npm run build` | 0 TS errors | Repo gate; not re-run today (see 3.4 note) |
| Headless smoke | `npm install --no-save playwright@1.63.0 && node qa-reports/headless-smoke.mjs` | **51/51 passed**, CSP-violation counter == 0, served CSP == `tauri.conf.json` policy | Run today |

These counts are **floors, not targets**:

- New behavior → extend `qa-reports/headless-smoke.mjs` (AGENTS step 4). Smoke check count may only grow.
- New pure backend logic (parser, detection, compare) → cargo unit test first (TDD per repo skill).
- A PR that lowers any baseline count fails review regardless of PASS/FAIL wording.
- Counts drift: `STATE.md`/`AGENTS.md` still say 48 / 43+. Refresh those numbers in the deferred wave-2 docs commit (orchestrator's edit, not per-task).

### 3.2 Lane responsibilities

| Lane | Covers | Does NOT cover |
|---|---|---|
| cargo unit | Config parse/serialize round-trip, atomic write + unreadable refusal, secrets sidecar, `parse_sftp_ls`, PTY pump injection — and all new pure logic (wave-2 binary detection: 4 result states + `HOSTERM_SFTP_BIN` override per AC-A1.1) | IPC, DOM, real processes on Windows |
| smoke (Playwright chromium) | UC1/UC2/UC3 equivalents vs mock invoke map (`prod-web`, `db-core`), tabs/splits, drag-resize, paste CRLF, SFTP layout, editor save path, **CSP-violation counter == 0**, served-CSP == config. Wave-2 extensions: missing-binary state + present-binary no-op (AC-A1.5), focus-reload vs temp config (AC-C1.x), refusal fixtures (AC-B5/B6 fixtures where headless-able) | Real `ssh`/`sftp` I/O, WebView2 quirks, real Windows |
| CI (`build-windows.yml`) | cargo-audit + npm-audit prod gates (high/critical fail; dated waivers in `.github/cargo-audit.toml` / `npm-audit-waivers.txt`), Windows build, tag release attachment | Unit/smoke (runs locally pre-push) |
| Manual Windows matrix | Once per release tag: UC1–UC7, WebView2 CSP injection proof (spike §4 one-liner), wave-2 manual ACs (install steps AC-A2.3, re-check AC-A4.1, updater install AC-B5.1) | Anything automatable above |

HOSTERM_CONFIG / HOSTERM_SFTP_BIN env overrides exist precisely so lanes 1–2 never touch the real `~/.ssh/` (SDD §3.3). Tests that would touch the dev machine's real `authorized_keys` must not exist.

### 3.3 Verify loop (canonical — every change, every PR)

1. `cd src-tauri && cargo test --lib` → ≥ 49 pass.
2. `npm run build` → no TS errors.
3. `npm install --no-save playwright@1.63.0 && node qa-reports/headless-smoke.mjs` → 51/51, CSP counter 0; then remove playwright (`node_modules/playwright*`, `node_modules/.bin/playwright*`) before staging.
4. Extend smoke for new behavior (in the same PR).
5. Pre-stage grep: `target/|node_modules|.exe$|dist/` → 0 tracked matches; secrets-in-config grep → 0.

A worker reporting done without quoting loop output gets RETURNed, not reviewed.

### 3.4 Note from the 19 Okt baseline run

Smoke + cargo were executed on the live tree; `npm run build` was **skipped** because the tree carried the in-flight review diff (`index.html`, `src/main.ts`, chain t_ffc96bd4) and rebuilding would overwrite `dist/` under review. The build lane was green at its last full run (SEC-4 chain). Rule this note encodes: **on a shared tree, never rebuild artifacts another review is inspecting** — run the lane, or state plainly why it was skipped.

---

## 4. Rollout strategy

### 4.1 Where we are

- v0.9.0 shipped with assets (tag `7a8bd8c`, asset-attach repair in release job: `fail_on_unmatched_files: true`).
- Landed post-tag, riding the **next release**: SEC-6 `withGlobalTauri: false` (5339962), SEC-4 CSP + `form-action 'none'`, R2 SHA-pinned actions, R3 audit gates, R4 least-privilege tokens. Wave-1 hardening is in-tree; its *release* happens with the next tag.
- SEC-5 (manual WebView2 pass) waived 19 Okt; re-run recommended via self-hosted Windows runner in a later wave (STATE.md waiver table).

### 4.2 Milestones (adopts wave-2 draft §7; binding)

| Milestone | Content | Gate | Constraint |
|---|---|---|---|
| M0 | Wave-1 hardening reaches a tagged release | Next `v*` tag after this TSD | All wave-2 work waits for M0 (PRD NFR-2) |
| M1 | Wave-2 Section A — startup ssh/sftp check + guide (Q2) | M0 | May ship in **unsigned** installers (D1; HR-W2-1 does not apply) |
| M2 | Wave-2 Section B Stage A — signing pipeline (FR-W2-5..7) | M0 + v0.9.0-with-assets (met) | Produces first fully signed release; requires SDD §2.4 extension (signing, updater config, key custody) authored **before** implementation (draft §4.1 flagged gap) |
| M3 | Wave-2 Section B Stage B — updater (FR-W2-8..10) | M2: one signed tagged release live | **HR-W2-1**: updater enabled only after signed release exists; CI-enforced (AC-B3.1/.2); PRD NFR-3 amendment (OQ-B1) lands with/before M3 |
| M4 | Wave-2 Section C — config mtime-reload-on-focus (Q6) | M0 only | Independent of M1–M3; slot after M1 to keep PRs small |

### 4.3 Release procedure (orchestrator only — never per-task agents)

Per `STATE.md`: commit work → bump **four** files (`package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, then `cd src-tauri && cargo update -p hosterm --offline`) → commit → tag `v*` → push tag → poll Actions → verify **assets actually attached** to the release page (v0.9.0 shipped zero assets once; the CI gate now fails loudly — still verify by eye).

### 4.4 Update channel

| Phase | Channel | Integrity |
|---|---|---|
| Now → M2 | Manual: GitHub Releases page only. App makes **zero** network calls (PRD NFR-3) | — |
| M3+ | User-initiated "Check for updates" → HTTPS GET `latest.json` from GitHub Releases → minisign verify vs baked-in pubkey → strict forward-only version → consented download/install (FR-W2-8..10). No polling, no silent install | Authenticode on installers (M2) + minisign on updater artifacts; verification failure = atomic refusal, no side effects (AC-B6.1) |

### 4.5 Rollback

- No server side; "rollback" = previous tag's installers remain on Releases. Users reinstall older artifact manually.
- Updater (post-M3) refuses downgrades by design (AC-B5.2iii) — deliberate: a rollback release must ship as a **new forward version**, never as a downgrade feed entry.
- On-disk formats are the compatibility surface: `~/.ssh/config` (OpenSSH grammar — never ours to change), sidecar `alias=password` lines, keys. Any format change needs a migration note + round-trip tests both directions; none is planned.
- Bad release: cut `vN+1` fix-forward; do not delete tags (users pin URLs).

---

## 5. Dev rules

### 5.1 Git policy

- **Developers never commit, never push.** Work lands in their kanban workspace (scratch/worktree for code, `dir` for docs); the lead collects, re-verifies, and pushes.
- Staging hygiene before any commit (whoever commits): grep staged files for `target/|node_modules|.exe$|dist/`; secrets-in-config grep; playwright removal (3.3 step 3).
- Version bumps only via the 4-file release procedure (4.3); releases/tags are cut by the orchestrator only.

### 5.2 Gate chain (what must pass before lead push)

| Artifact | Required chain |
|---|---|
| App code + configs that ship (frontend/backend) | Lead code review + **crew-coding-qa verify PASS** + **crew-coding-security clean** → lead pushes |
| Docs (PRD/TSD/feature specs/spikes) and CI/workflow files | Nano verification + lead; no full chain |
| In doubt which side an artifact is on | Treat as code; full gate applies |

Every gate citation travels with the push record: task ids + commit hash (see t_981b4ece pattern).

### 5.3 Repo invariants (full list: `docs/STATE.md`; these are the ones tasks violate most)

1. `~/.ssh/config` stays parseable by REAL ssh at all times; no secrets in it — ever. Sidecar only.
2. Round-trip edits preserve comments, order, unknown directives (ADR-4).
3. PTY paste normalizes CRLF→LF (`pasteNormalize`).
4. Drag/copy = manual mouse events, **never** HTML5 DnD (dead in WebView2).
5. Never touch the dev machine's real `~/.ssh/authorized_keys`.
6. New rendering code routes through `esc()`/`escAttr()` (ADR-8); CSP never relaxed for convenience.

### 5.4 IPC contract rule (new commands)

New backend commands follow SDD §2.3 patterns: request/response via invoke (events only for PTY streaming), host/path strings validated (`sftp_safe` or equivalent) before reaching any spawned binary, secrets never cross IPC. Wave-2 additions conform — lead's contract sketches in §6.2; final signatures land in the SDD §2.4/§2.3 extension with the wave-2 implementation spec.

---

## 6. Wave-2 technical decisions (delegated to lead/TSD — binding for Sections A–C)

### 6.1 OQ-C3 — config change detection: **mtime + size**

Decided: compare `(mtime, size)` from a **single `stat` call**. 

- Rationale: same `stat` returns both — zero extra cost vs mtime alone (keeps NFR-W2-10's zero-work posture). Size catches same-timestamp rewrites some sync tools produce (draft §8 OQ-C3's stated risk). FAT/exFAT 2-second mtime granularity is compensated by the size component.
- Not a content hash: hashing requires a read of the whole file per focus — violates the "at most one stat on unchanged file" budget (NFR-W2-11). Revisit only if a real missed-detection report exists.
- Contract: values held in frontend/backend memory only (last-known pair), initialized at startup check, updated after each reload. Persistence across restarts is pointless — startup re-reads anyway.

### 6.2 OQ-C5 — Windows `stat` while another process holds the file open

Decided behavior (to be restated in SDD §3.1):

1. **Metadata `stat` on a Windows file open by another process succeeds.** Windows has no mandatory read locks for metadata queries; editors/IDEs keep write handles (share-read) that don't block `GetFileAttributesEx`-class calls. hosterm therefore never needs special handling to stat a config that an editor has open.
2. **hosterm itself never holds the config open across calls**: reads open/close within one command; writes are temp-file + atomic rename (FR-A5). Consequence: hosterm's own writes can never produce a torn file that a focus `stat` observes — a stat either sees the old file or the renamed new one.
3. **The dangerous case is read-while-external-write** (sync tool mid-write when the reload read happens): partial/invalid content → parse fails → **FR-W2-14 path**, not a crash or a blank sidebar: keep last-good model, surface named-path error, retry next focus. No special pre-locking, no retry loop within a focus event (would drift toward polling).
4. Prohibition restated: no opening the file with exclusive/share-none flags to "test" readability — that creates the lock contention the feature must avoid, and can break the user's own editor save. `stat` only; errors handled at read time.

### 6.3 Contract sketches (lead-owned; SDD formalizes with implementation)

- Section A: one new read-only command `check_binaries() -> { ssh_missing: bool, sftp_missing: bool }` — pure lookup (PATH then `%SystemRoot%\System32\OpenSSH\`; `HOSTERM_SFTP_BIN` override honored for sftp), no spawn, no network, unit-testable per AC-A1.1.
- Section C: one new read-only command `config_stat() -> { mtime_ms: u128, size: u64 }` (+ error variant) — metadata only per §6.1/6.2; the reload itself reuses the existing `read_ssh_config`.
- Section B: no IPC sketch here — updater config/artifacts are CI + plugin surface, defined in the SDD §2.4 extension required before M2 implementation.

### 6.4 OQ endorsements (not lead-delegated, noted for coherence)

- OQ-B1 (amend PRD NFR-3): agree — one-line PRD edit at MEKL approval, must land by M3.
- OQ-B2 (≤ 5 s update-check bound: one-off measurement at M3, no CI timing gate): agree — consistent with PRD Q5 / NFR-6 posture.

---

## 7. Open items carried (flagged to orchestrator)

1. **Count refresh:** `STATE.md`/`AGENTS.md` verify-loop numbers (48 / 43+) → 49 / 51 — ride the single deferred docs commit.
2. **SDD §2.4 extension** (signing + updater + key custody) must exist before M2 implementation tasks are created (wave-2 draft §4.1 gap; architect's deliverable).
3. **PRD NFR-3 amendment** (OQ-B1) by M3 at the latest.
4. Playwright residue from today's baseline run sits in untracked `node_modules/` (cleanup command blocked by session policy; staging grep covers it — zero commit risk).

---

*Baselines in §3 are re-measurable at any time; if a number here disagrees with a live run, the live run wins and this doc gets patched in the next docs commit.*
