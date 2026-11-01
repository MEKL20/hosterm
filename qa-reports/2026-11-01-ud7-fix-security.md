# U-D7 FIX Security Re-scan

- **Target:** uncommitted working tree on `HEAD=7cccf6c5ea4a1d680f5b9886b0009b2bbde204dd`
- **Reviewed diff fingerprint:** `git diff` SHA-256 `497e9613947b687d78f1f342519ca1882285b15a8fe6bcea71579c4ee91a0d5d`
- **Contract:** EPIC §U-D7; SDD §13.4 and frozen §13.6; FR-AI9; INV-21/22/25/26
- **Verdict:** **CLEAN — 0 Critical / 0 High / 0 Medium / 0 Low**

## Closure of prior findings

### UD7-SEC-01 — FIXED

Both policy-forbidden renderer sinks are gone.

- Static template skeletons are created only with DOM APIs at `src/main.ts:203-272`.
- `renderApprovals` clones `APPROVAL_ITEM_TEMPLATE` and writes host, command, and index only through `textContent`/`setAttribute` (`src/main.ts:600-608`). It installs the fragment with `replaceChildren` first (`:611`), then queries the live DOM and binds listeners (`:613-617`).
- `renderAiThread` clones `AI_PROPOSAL_TEMPLATE` (`src/main.ts:779-784`). Prompt, response, host, state label, command, description, output, and extra thread text use `textContent`; classes and `data-ai-id` use `setAttribute` (`:785-810`). It installs the fragment at `:813`, then binds action listeners at `:815-820`.
- The frozen action classes remain exact: `btn-primary approval-run`, `btn-secondary approval-always`, `btn-ghost approval-reject` (`src/main.ts:248-253`). State labels/classes remain the finite §13.6 values (`:745-776`).
- Source scan of both renderer bodies found **0** `.innerHTML` assignments and confirmed clone + replace + post-install listener ordering. Full added-production-line scan found **0** `innerHTML`/`outerHTML` assignments, `insertAdjacentHTML`, or `document.write`.
- Runtime smoke passed the source sink gate, exact inert approval-row checks, hostile re-render after Run once, and hostile re-render after Always allow. In each hostile case: `childElementCount=0`, `window.__pwn` remained `undefined`, CSP violations `0`.

### UD7-SEC-03 — FIXED

- Matching remains byte-exact `(host, command)` via strict `===` at `src/main.ts:741-743`.
- `alwaysAllowAi` checks the in-memory tuple, then re-reads `read_settings` before any append when needed (`src/main.ts:887-894`). The append/write branch is reachable only while the exact tuple is still absent (`:895-905`).
- A duplicate skips `write_settings` and reaches exactly one explicit `executeAiCommand` call (`src/main.ts:907-914`). The only UI entry is the post-render `.approval-always` click listener (`:819`); no timer, load, or render path invokes execution.
- Runtime startup-fallback fixture proved: one `read_settings`, zero `write_settings`, one `pty_write`, unchanged allowlist, and exactly one retained tuple. Opening the AI drawer also produced zero spawn/write/settings deltas, so FR-AI9 has no new auto-run path.

### Other prior items

- **UD7-SEC-02: N/A / NO ACTION.** Frontend sentinel `added_at: ""` remains intentional. Backend stamps fresh entries with RFC3339 at `src-tauri/src/lib.rs:811-821`; `write_then_read_roundtrips_and_stamps_added_at` passed.
- **UD7-SEC-04: DEFERRED.** DOM/model trust-anchor hardening was not changed.
- **UD7-SEC-05: DEFERRED.** Existing live-PTY execution semantics were not changed.

## No new sink or path

Added production lines across the cumulative U-D7 diff were scanned independently:

| Gate | Result |
|---|---:|
| HTML sink assignments / `insertAdjacentHTML` / `document.write` | 0 |
| `eval` / `Function` | 0 |
| `fetch` / XHR / WebSocket / EventSource / `sendBeacon` | 0 |
| Hardcoded credential assignment pattern | 0 |
| Inline `on*=` handlers | 0 |
| Added inline `<script>`, `<style>`, or `style=` | 0 |
| Abyss tokens (`#08080a`, `#050507`, `#00d992`, `#818cf8`, `#0e2a20`, `var(--glow)`) | 0 |

Added frontend invokes are only existing commands: `read_settings`, `write_settings`, and `pty_write`. No backend command, dependency, network surface, or second execution engine was added.

## Scope integrity

The repository intentionally stacks U-D7 and its FIX uncommitted, so cumulative `git diff --name-only` against `7cccf6c` returns the four parent files. It is not a fix-only comparison. Fix-local scope is nevertheless verified:

- First-scan baseline: `index.html +34/−0`, smoke `+234/−0`, `src/main.ts +344/−11`, styles `+160/−0`.
- Current: `index.html +34/−0`, smoke `+319/−1`, `src/main.ts +436/−11`, styles `+160/−0`.
- Therefore only `src/main.ts` and `qa-reports/headless-smoke.mjs` changed in the tracked FIX delta.
- Current protected-file hashes match the FIX handoff exactly:
  - `index.html`: `64d8d90c5146187f55a369b4c788acd9069e806ec5ec4e6e317ab3f4c2034dd6`
  - `src/styles.css`: `e2fd655bd4a04e38c65af75bd8b799920dad4031931ac9f3ae69357d43afe7c8`
- `src-tauri/**`, package manifests, and lockfiles: **zero diff**.
- `git diff --check`: **PASS**.

## Independent verification

| Command | Result |
|---|---|
| `npm run build` | PASS; TypeScript clean; Vite built in 999 ms |
| `(cd src-tauri && cargo test --lib)` | PASS — **87 passed, 0 failed**; two pre-existing `unused_mut` warnings |
| `node qa-reports/headless-smoke.mjs` | PASS — **313/313**; no console/page errors; CSP violations **0** |
| Static security/invariant scan | PASS — all counts above are zero; renderer ordering confirmed |

No code, commit, push, or stash was produced by this scan. This file is the only report created by the security task.
