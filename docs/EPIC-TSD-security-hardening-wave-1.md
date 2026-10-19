# TSD — hosterm Security Hardening, Wave 1

| Field | Value |
|---|---|
| Author | crew-coding-lead |
| Input | docs/PRD-security-hardening-wave-1.md (requirements + AC are authoritative; this doc does not reopen them) |
| Status | Approved for backlog dispatch |
| Repo | /home/mekl/src/hosterm (worktree per task; no commits by task agents without lead review) |
| Decision lock | Q1–Q7 from PRD §8 are FINAL (see §2). Do not reopen. |

---

## 1. Context and architecture fit

hosterm is a Tauri v2 desktop SSH/SFTP client (Windows/WebView2 first, developed on Linux). It renders attacker-reachable bytes — SFTP filenames, remote file contents, terminal output — into the DOM via 21 `innerHTML` sites with only 13 escape calls, and it holds user SSH credentials in the secrets sidecar. Wave 1 adds three defenses without touching the frontend data-flow or the Rust backend logic:

1. **R1 — CSP** (containment: injected script cannot run, so `innerHTML` XSS cannot reach the IPC bridge).
2. **R2 — immutable action pins** (supply chain: a hijacked action tag cannot run in CI).
3. **R3 — dependency CVE gates** (known-bad versions cannot ship silently).
4. **R4 — CI token scope** (blast radius of 2/3: the write token exists only where release attachment needs it).

Non-goals are the PRD's (§2): no `lib.rs` changes, no `innerHTML` refactor (CSP is the wave-1 compensating control), no SBOM/signing/auto-update, no upgrade campaign, no migration off real `ssh`/`sftp` binaries.

### Verified repo facts (measured, not assumed)

- `src-tauri/tauri.conf.json:24` → `"csp": null`. Single source of the policy; Tauri injects it into served HTML and additionally rewrites it per-asset (nonce/hash for Tauri's own injected scripts) unless disabled — spike must confirm for our Tauri v2 minor.
- `.github/workflows/build-windows.yml` is the only workflow. Six `uses:` refs, all mutable; `softprops/action-gh-release@v2` (third-party) holds `contents: write` granted **workflow-wide** (line 13–14) and runs only on tags (line 50).
- Both lockfiles committed (`package-lock.json`, `src-tauri/Cargo.lock`) → audit tooling is meaningful.
- Frontend deps: `@xterm/xterm@^5.5`, `@xterm/addon-fit`, `codemirror@^6`, `@tauri-apps/api`, `@tauri-apps/plugin-opener`. xterm CSS is bundle-imported (`src/main.ts:8`) — no runtime style injection from xterm. CodeMirror 6 **does** inject runtime `<style>` via `style-mod` (`doc.createElement("style")`) — the one hard CSP compatibility risk.
- All assets bundle locally via Vite; no external origins, no `@font-face`, zero inline `style=` attributes in `src/main.ts`.

---

## 2. Decision record (locked — PM recommendations adopted)

| # | Decision |
|---|---|
| Q1 | `'unsafe-inline'` permitted **only** in `style-src`, and only if the spike proves per-injection hashes impractical for `style-mod`. `script-src` stays `'self'` + Tauri-injected IPC needs, never relaxed further. |
| Q2 | Whether Tauri v2 auto-appends IPC origins (`ipc:` / `http://ipc.localhost`) to `connect-src` is a **spike deliverable** (R1.7), not an assumption. |
| Q3 | Audit gate: fail on **high+critical in production dependencies**. Dev-dependency findings advisory (logged, non-blocking). |
| Q4 | Waiver = ignore-entry **in the audit config** + inline reason + review date ≤ 90 days out. No separate waiver doc. |
| Q5 | Dependabot config prepared (github-actions ecosystem); merge remains MEKL-only. |
| Q6 | R4 is in wave 1, same PR as R2. |
| Q7 | One-time manual WebView2 compatibility pass on real Windows before the wave-1 release tag (MEKL), then automated Chromium CSP check guards regression. |

---

## 3. Design

### 3.1 R1 — CSP (highest value, highest risk, ships last)

**Policy home:** the `csp` string in `src-tauri/tauri.conf.json` (single place; Tauri serves and injects it). No meta tags, no header plumbing.

**Draft policy shape (starting point for the spike, not final):**

```
default-src 'self';
script-src 'self';
style-src 'self' [spike: hash/nonce  OR  'unsafe-inline' if Q1 condition proven];
img-src 'self' data:;
connect-src 'self' [spike: Tauri IPC origins or confirmation they are auto-appended];
font-src 'self';
object-src 'none';
base-uri 'self';
frame-ancestors 'none'
```

Deny-by-default: no `default-src *`, no wildcard origins. `data:` in `img-src` only if xterm/addon-fit or CodeMirror demonstrably need it in the spike build; drop otherwise.

**Spike (architect, runs first, parallel to everything):** a throwaway branch, not shipped code. Deliverable = spike notes answering, with evidence from a real `npm run tauri build` + dev run:
1. Exact Tauri v2 CSP injection behavior in dev (`localhost:1420` + Vite HMR websocket — is `ws://localhost:1420` needed in dev `connect-src`, and how to scope it to dev) vs prod.
2. Whether Tauri auto-appends `ipc:` / `http://ipc.localhost` to `connect-src` when `csp` is set, or the config must carry them (PRD Q2).
3. Minimal style accommodation for CodeMirror `style-mod`: test per-injection hash feasibility first; only if impractical, `'unsafe-inline'` scoped to `style-src` (Q1 condition), with the proof recorded.
4. One-time enforcement proof (R1.5): injected inline `<script>` in the built app is blocked; recorded in the spike notes.
5. Whether xterm (DOM/canvas renderer paths) or `addon-fit` triggers any CSP report at all.

**Implementation (frontend, after spike):** set the final policy, extend `qa-reports/headless-smoke.mjs` with a console listener that collects CSP-violation events across the full run and asserts count == 0 (R1.4). No changes to rendering code, no `innerHTML` rewrites.

**Compatibility contract (equal weight to the security goal):** terminal (`ls --color`, resize), SFTP browser (unicode/special-char filenames), CodeMirror editor (syntax highlight, edit, save) — zero CSP console violations on the real Windows build (manual matrix, MEKL, once before tag per Q7).

### 3.2 R2 — Pin actions + R4 — token scope (one devops task, one file)

- Every `uses:` in `.github/workflows/build-windows.yml` → full 40-hex commit SHA, release tag preserved as trailing comment (`uses: actions/checkout@<sha> # v4`). All six refs, including third-party `softprops/action-gh-release@v2` — that pin is the point of the exercise since it holds the release token.
- `permissions:` at workflow top → `contents: read`. `contents: write` moves to the narrowest scope that still attaches artifacts on tag builds — the release-attach step/job only (`if: startsWith(github.ref, 'refs/tags/')`). Manual dispatch builds must complete with the read-only default (R4.2).
- Refresh path: `.github/dependabot.yml` with `package-ecosystem: github-actions`, weekly; PRs land as proposals, MEKL merges (Q5). Pinning without an update path rots.

### 3.3 R3 — Dependency audit gates (second devops task)

Two audit lanes, both on PR + push-to-default + tag builds, both with human-readable findings in the job log (R3.4):

- **Rust:** `cargo audit` against committed `src-tauri/Cargo.lock`. cargo-audit has no prod/dev split — lockfile findings gate on high/critical severity, advisory otherwise. Waivers = `ignore` entries in the audit config with `# reason + review-by: YYYY-MM-DD` inline (Q4). Windows-runner install cost (compile from source) must respect the ≤ +5 min release-latency budget (success metric); if it busts it, use a maintained audit action instead — tool choice is the worker's, the AC and budget are not.
- **Frontend:** `npm audit --omit=dev --audit-level=high` as the **gate** (prod deps, high+critical fail the build — Q3); a second full `npm audit` run marked advisory (`continue-on-error`) surfaces dev-chain findings in the log without blocking. Waivers use npm's override/ignore mechanism or an inline-suppressed advisory entry — same rule: reason + review date next to the config (Q4).

**Negative test (R3.2, one-time):** temporarily declare a dep with a known high/critical CVE → CI red; revert → green. The temporary commit is removed in the same PR; evidence (job URLs) recorded in the task.

### 3.4 Data / API changes

None. No Rust source changes, no schema, no config-file format changes, no IPC contract changes. Wave 1 touches: `tauri.conf.json` (one key), `.github/workflows/build-windows.yml`, new `.github/dependabot.yml`, new audit config file(s), `qa-reports/headless-smoke.mjs` (one added check), and the spike branch (thrown away).

---

## 4. Edge cases and risks

| Risk | Mitigation |
|---|---|
| CodeMirror breaks under `style-src` without accommodation | Spike proves the accommodation **before** the frontend task starts; R1 ships only after the Windows matrix is green. This is the #1 breakage risk of the wave (PRD-verified). |
| Tauri dev-mode HMR blocked by CSP in dev | Spike scopes any dev-only relaxation (e.g. HMR websocket) to dev; prod policy never carries it. |
| Pinned SHA wrong / stale action behavior | SHAs resolve to the currently-used tag (AC R2.2); first post-pin run is a manual `workflow_dispatch` (R2.3), then verified end-to-end on the next real tag (R2.4). |
| Audit gate fails main on day one | AC R3.3: main is green **or** every finding carries a waiver with a review date ≤ 90 days. CVE triage session with MEKL in the same week (rollout §6). |
| Audit steps blow the release latency budget (≤ +5 min) | Budget is a design constraint; caching or action-based tooling allowed, AC never negotiable. |
| Smoke CSP check passes in Chromium but WebView2 drifts | Accepted per Q7: one manual Windows pass before the wave tag, then automated regression coverage. |
| `npm audit --omit=dev` semantics drift across npm majors | Pin/verify npm major on the runner in the task; negative test re-proves the gate bites at whatever version ships. |

---

## 5. Rollout and rollback

**Order:** (1) R2+R4 PR — immediate, zero UI risk, verified by dispatch run + next tag. (2) R3 PR — same week, with CVE triage. (3) R1 — spike parallel from day one, implementation after spike, manual Windows matrix, ship with next regular release. Wave 1 is done when R1 ships.

**Rollback:** each requirement is one independent revert. R1: revert `csp` to `null` (or revert the single PR) — no other surface depends on it. R2/R4: revert workflow file (pins and permissions are colocated; a revert restores today's behavior exactly). R3: revert the audit workflow block. No data migrations exist to unwind.

---

## 6. Test plan (per existing verify loop)

1. `cd src-tauri && cargo test --lib` → 48 pass.
2. `npm run build` → no TS errors.
3. `npm install --no-save playwright@1.63.0 && node qa-reports/headless-smoke.mjs` → 43+ pass **and** the new CSP-violation counter == 0; then clean playwright per repo hygiene.
4. `grep -nE "uses:\s*\S+@(v[0-9]+(\.[0-9]+)*|stable|master|main)" .github/workflows/*.yml` → 0 matches (AC R2.1).
5. R3: negative-test evidence (one red run, one green run) attached to the task.
6. R1 ship gate: manual Windows matrix (terminal / SFTP / editor) with zero CSP violations, once, by MEKL before tag (Q7).

---

## 7. Backlog

| ID | Title | Owner | Depends on | Key AC (PRD refs) |
|---|---|---|---|---|
| SEC-1 | Pin CI actions to immutable SHAs + minimize token permissions | crew-coding-devops | none | R2.1–R2.4, R4.1–R4.3, Dependabot (Q5) |
| SEC-2 | Add dependency vulnerability audit gates to CI | crew-coding-devops | none (independent of SEC-1; same-lane ordering preferred) | R3.1–R3.4, thresholds per Q3, waivers per Q4 |
| SEC-3 | CSP spike: Tauri v2 injection behavior + CodeMirror style accommodation | crew-coding-architect | none — starts immediately, parallel | R1.7, Q1, Q2, R1.5 proof |
| SEC-4 | Implement CSP policy + automated smoke CSP-violation check | crew-coding-frontend | SEC-3 | R1.1–R1.4, R1.6 |
| SEC-5 | Manual WebView2 compatibility matrix on real Windows (MEKL, one-time before wave tag) | MEKL (manual, not dispatched) | SEC-4 | R1.3, Q7 |

Verification commands per task live in the task bodies (§6 commands map onto them). No task may commit/push without lead review + QA PASS + security scan clean (push gate).

---

## 8. Success metrics (from PRD §6)

100% pinned refs (grep = 0) · both audit gates green on main, negative test proven · CSP enforced in shipped builds, 0 violations across smoke + Windows matrix · 48 cargo tests / 43+ smoke checks / all three surfaces verified · release latency +≤ 5 min.
