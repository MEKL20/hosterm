# U-D7 Security Scan — AI assistant + approval VISUAL (SECURITY-SENSITIVE)

- **Target:** working tree, uncommitted on HEAD=`7cccf6c` (`fix(ui): remove Google chip innerHTML sink`); `M index.html`, `M qa-reports/headless-smoke.mjs`, `M src/main.ts`, `M src/styles.css` (+772/−11)
- **Mode:** WHITEBOX, read-only; no runtime probing, no code changes applied
- **Scope:** `src/main.ts`, `index.html`, `src/styles.css`, `qa-reports/headless-smoke.mjs`
- **Contract basis:** SDD §13.6 (approval contract FROZEN) + §13.4/§13.7 (allowlist persist/data model), EPIC §U-D7 (TSD lines 233–261), INV-25/26, INV-11 family, INV-22, FR-AI1..13, ADR-1/2/6/21/22
- **Verdict:** **FINDINGS — 1 MEDIUM, 4 LOW, 0 Critical/High** (no exploitable injection; no new network/execution path; approval contract honored)
- **Evidence basis:** `git diff` of the dirty working tree vs `7cccf6c`. Line numbers refer to the working-tree blobs.

## Coverage ledger

| # | Checklist item | Result | Severity | Evidence |
|---|---|---|---|---|
| 1 | NOL backend AI / NOL network baru / NOL jalur eksekusi baru | **CLEAN** | — | Thread = static stub: `AI_PROPOSALS` is a compile-time constant array (`src/main.ts:150-168`); `sendAiStub()` (`src/main.ts:831-843`) does pure DOM push + honest copy "The AI backend is not connected. This mock did not send or execute your request." Added-line scan: `fetch`/XHR/WebSocket/EventSource/sendBeacon = **0**. `package.json`/lockfiles/`src-tauri/**` = 0-diff (git status lists exactly the 4 scoped files; no npm/cargo dep added). Execution funnels through the single existing path: `executeAiCommand` (`src/main.ts:771-779`) → existing `liveTerminalForHost`/`openTerminal`/`startPty` (ADR-1/2, only signature changed: returns `Session \| null`, `null` when `binaryBlocked` or `ptyId` unset) → existing `pty_write`. No second execution engine, no new IPC command (`write_settings`/`read_settings` already registered since U-D3). |
| 2 | Injection surface: all thread/output/host/command via `esc()`/`escAttr()`; hostile fixture passes; **no NEW innerHTML/insertAdjacentHTML sink** (U-D6 FIX `7cccf6c` marker) | **FINDING UD7-SEC-01** (sinks) + escaping itself CLEAN | MEDIUM | Every interpolation in both new sinks is escaped — `renderApprovals` (`src/main.ts:532-537`): `esc(entry.host)`, `esc(entry.command)`, `data-index` numeric; `renderAiThread`/`aiCardMarkup` (`src/main.ts:669-737`): `escAttr(proposal.id)`, `esc(host/command/description/result/prompt/response)`, static `stateLabel`. `esc()` = textContent→innerHTML round-trip (`src/main.ts:187-191`), `escAttr` adds `&quot;`; all attribute positions use `escAttr`. Smoke hostile fixture `HOSTILE = "<img src=x onerror=window.__pwn=1>"` passes on both submit paths: `childElementCount 0`, `__pwn === "undefined"`, `csp === 0` (smoke "FR-AI11 … stays raw inert text" + "…after reload"). **However the diff introduces 2 new innerHTML sink assignment sites** — the exact regression marker the U-D6 fix removed. See finding below. |
| 3 | INV-26: `sftp_safe` host-alias guard (leading `-`/CR/LF refuse) BEFORE execute on ALL paths | **CLEAN** | — | `sftpSafe()` (`src/main.ts:198-200`) = frontend mirror of backend argv guard: `!startsWith("-") && !includes("\n") && !includes("\r")`. Guarded at every entry: `runAiOnce` checks before execute (`src/main.ts:781-785`), `alwaysAllowAi` checks before persist AND execute (`src/main.ts:799-803`), plus defense-in-depth re-check inside the choke point `executeAiCommand` (`src/main.ts:772`). The only remaining state, exact allowlist re-match (FR-AI7), renders `actions = ""` — **no execution at all** on that path, so nothing bypasses the guard. Smoke covers `–prod-web` / `prod\rweb` / `prod\nweb` on both `.approval-always` and `.approval-run`, asserting zero IPC delta + "Nothing was executed or saved" (3 INV-26 checks). |
| 4 | INV-25: permission-widening action never primary | **CLEAN** | — | Markup order+classes per frozen contract: `btn-primary approval-run` / `btn-secondary approval-always` / `btn-ghost approval-reject` (`src/main.ts:680-684`). CSS weights: `.btn-primary` = accent fill (`--accent` bg + `--accent-ink` ink, weight 650), `.btn-secondary` = muted `--raised` (`src/styles.css` diff), `.btn-ghost` = transparent. No CSS rule re-targets `.approval-always` toward primary weight; no code path swaps classes. Smoke asserts class-level INV-25 including explicit `!classes[1].includes("btn-primary")`. |
| 5 | FR-AI9: zero implicit auto-run | **CLEAN** | — | Execution exists in exactly 2 places, both direct click handlers (`runAiOnce`, `alwaysAllowAi`). No timer/interval/load handler executes. Allowlist re-match renders ALWAYS ALLOWED **without actions** (conservative vs SDD "MAY execute"). Smoke: opening the panel produces zero delta in `pty_spawn`/`pty_write`/`write_settings`/`read_settings` ("FR-AI9: opening the mock never runs or persists a command implicitly"). |
| 6 | Allowlist persist: exact `(host,command)` byte-match, `write_settings` only, INV-22 untouched, revoke = read+remove+write | **CLEAN** (2 Low sub-findings) | LOW | Match is strict `===` on both tuple members, no trim/normalize/glob (`aiAllowlisted`, `src/main.ts:668-670`); smoke proves trailing-space command ≠ byte-match (ADR-21 check). Persist only via `invoke("write_settings", …)` (`persistAllowlist`, `src/main.ts:520-528`); payload preserves `version/theme/accent`. INV-22: `src-tauri/**` 0-diff — no `~/.ssh` path touched anywhere in the diff; allowlist lives in app-data `settings.json` only. Settings›Approvals = read (`read_settings`) + splice-by-index + `write_settings` (`src/main.ts:548-584`), zero pattern inputs (smoke asserts `patternInputs === 0`). Sub-findings UD7-SEC-02 (empty `added_at`) and UD7-SEC-03 (duplicate append) below. |
| 7 | FR-AI12: no misleading "on-device" claim | **CLEAN** | — | Head-tag renders `mock` with `title="UI-only mock; no AI backend is connected"` (`index.html` AI drawer head). Zero occurrences of "on-device" in the diff; smoke asserts `tag === "mock"`, `tagTitle.includes("no AI backend")`, and `!JSON.stringify(aiPanel).includes("on-device")`. Stub reply explicitly states the backend is not connected. |
| 8 | styles.css: no external font/asset remote | **CLEAN** | — | Added-line scan: `url(`, `@import`, `@font-face` = **0**; fonts resolve via existing `--font-mono` system stack; all colors via existing tokens (`--accent`, `--raised`, `--danger`, `--live`). CSP 0 violations asserted in smoke on hostile fixtures (both pre- and post-reload). |

Smoke coverage: **17 new U-D7 checks** (≥8 required), including class-level INV-25 assertion, zero-IPC INV-26 loop, byte-match negative test, reject no-write check, and 2 hostile-escape fixtures with CSP counter.

## Findings

### UD7-SEC-01 — Two new `innerHTML` sinks reintroduce the U-D6 regression marker

- **Severity:** MEDIUM
- **Status:** CONFIRMED (all interpolations escaped; exploitability not demonstrated — this is the policy/regression marker the unit brief and `7cccf6c` fix establish)
- **Invariant:** INV-11 family / U-D6 FIX `7cccf6c` precedent / SDD ADR-6 wave-2 DOM-hardening debt
- **Locations:**
  - `src/main.ts:532` — `list.innerHTML = appSettings.allowlist.map(...)` (Settings›Approvals list)
  - `src/main.ts:731` — `thread.innerHTML = proposals + extras` (AI thread re-render)

**Analysis.** `7cccf6c` removed the last mock-surface innerHTML sink even though its content was `esc()`-wrapped, replacing it with `<template>` + `replaceChildren` + `textContent`. The U-D7 diff adds two brand-new sink sites. Today they are XSS-safe: every interpolation is `esc()`/`escAttr()` (verified interpolation-by-interpolation above), inputs are static constants or settings round-trips, CSP is the structural backstop, and the hostile fixture passes with `childElementCount 0` / `__pwn undefined` / CSP 0. But the sink count grows the wave-2 debt inventory (SDD ADR-6 tracks 21 sites) and re-arms exactly the marker the previous fix deleted; a future edit that adds one unescaped interpolation to these templates silently becomes executable markup inside the approval surface.

**Recommendation (not applied — scan only).** Port both renderers to the `7cccf6c` pattern: static `<template>` skeleton + `replaceChildren(template.content.cloneNode(true))` + per-node `textContent` for dynamic values; keep listeners bound after clone. At minimum, add both sites to the wave-2 sink inventory with an "all-esc" annotation.

### UD7-SEC-02 — `added_at` persisted as empty string, breaking the frozen data model

- **Severity:** LOW
- **Invariant:** SDD §13.7 (`added_at: string # RFC 3339 UTC, audit only`)
- **Locations:** `src/main.ts:805` — `const next = [...appSettings.allowlist, { host, command, added_at: "" }]`; smoke codifies it (`savedPrune?.added_at === ""`)

**Analysis.** The frozen data model defines `added_at` as an RFC 3339 UTC audit timestamp. U-D7 always writes `""`. No security function depends on it (byte-match uses host+command only), but the approval audit trail loses "when was this standing grant created" — the exact field that matters when reviewing blast radius after an incident.

**Recommendation.** Write `new Date().toISOString()` at append time and update the smoke assertion to format-validate instead of asserting emptiness.

### UD7-SEC-03 — `alwaysAllowAi` can append a duplicate exact tuple (no re-check before persist)

- **Severity:** LOW
- **Invariant:** ADR-21 hygiene; FR-AI10 revoke UX
- **Locations:** `src/main.ts:799-811` — appends without `aiAllowlisted(host, command)` re-check

**Analysis.** Reachable path: `read_settings` fails at startup → `appSettings.allowlist` falls back to `[]` (`src/main.ts:465-478` `catch { loaded = DEFAULT_SETTINGS; }`) while the disk still holds entries → cards render as AWAITING with actions → clicking **Always allow** appends a tuple that already exists on disk. Impact is hygiene, not privilege: byte-match semantics unchanged, but the Approvals list shows the same grant N times and revoke needs N clicks, and each duplicate is one more `write_settings` round-trip. (Normal UI flow is protected: after a successful persist the card re-renders actionless.)

**Recommendation.** In `alwaysAllowAi`, skip the append (and jump straight to execute) when `aiAllowlisted(host, command)` is already true; optionally re-read settings before rendering actions.

### UD7-SEC-04 — Execution tuple is sourced from rendered DOM, not the data model

- **Severity:** LOW (hardening)
- **Locations:** `src/main.ts:742-746` (`visibleAiCommand` reads `.cmd-scope`/`.cmd-line` `textContent`), consumed at `src/main.ts:781,799`

**Analysis.** Today this round-trips exactly: `esc()` is textContent→innerHTML and `textContent` decodes entities back, so DOM value === model value, and the smoke exploits this deliberately (mutating `.cmd-scope` to hostile aliases) — with the guard correctly refusing. But it makes the rendered DOM the trust anchor for what gets executed and persisted, while allowlist *matching* uses the model (`aiCardMarkup` → `aiAllowlisted(proposal.host, …)`). Two divergent sources: any future change to the render path (e.g., syntax highlighting inside `.cmd-line`) would change executed bytes without touching the approval logic.

**Recommendation.** Resolve the proposal by `data-ai-id` from `AI_PROPOSALS` and execute/persist from the model; keep the DOM only for display.

### UD7-SEC-05 — Reused live PTY means approved bytes can concatenate with user-typed bytes (forward-looking)

- **Severity:** LOW (latent; contract-accepted posture, mock-gated in v1)
- **Locations:** `src/main.ts:765-779` — `liveTerminalForHost` prefers an existing open session for the host; `pty_write` sends `` `${command}\r` ``

**Analysis.** SDD §13.6 freezes "exactly as if the user typed the command" into the existing terminal, so reuse is by design. Residual edge: writing into a live PTY concatenates with whatever is already in the input buffer or lands inside a non-shell context (half-typed line, heredoc, `vim`, nested `ssh`, a sudo password prompt awaiting input) — so the executed byte stream can diverge from the approved command even though the *approval* was honored. Also note the frozen guard scopes `sftp_safe` to the **host alias only**; a command containing `\n` would be typed as multiple lines, each executing after one Run-once click. In v1 both are inert: proposals are compile-time constants (no CR/LF) and every execution requires an explicit click. This becomes live the day the AI-backend epic connects real generated commands.

**Recommendation (input to the future backend epic, per SDD §13.6 "owed input").** Consider executing in a fresh PTY (or verifying an idle prompt) before `pty_write`, and extending the guard to refuse/strip CR/LF inside the command string; record both as §13.6 integration requirements.

## Counterevidence considered

- **Could `esc()` round-trip diverge from the executed value?** No — `esc` is `textContent → innerHTML` (entity encode), and reading back via `textContent` decodes to the identical string for all inputs, escaped or not; verified against the guard's smoke vectors.
- **Does allowlist re-match auto-execute (FR-AI9 violation)?** No — the allowed branch renders `actions = ""`; there is no execute call on that path (stricter than the SDD's optional "MAY execute").
- **Does any CSS rule swap INV-25 weights?** No — `.approval-*` classes carry no style of their own; only the frozen `btn-primary/secondary/ghost` classes style the actions.
- **New IPC or backend surface?** None — `src-tauri/**` 0-diff; only pre-existing `read_settings`/`write_settings`/`pty_write`/`pty_spawn` invoked.
- **Theme persistence regression (allowlist clobber)?** `persistSettings` (pre-existing U-D3 path) round-trips `appSettings.allowlist`, and `persistAllowlist` writes the full snapshot — the two writers stay consistent on the same in-memory state.

## Commands and raw scan summary

```text
$ git log --oneline -1            # base being diffed against
7cccf6c fix(ui): remove Google chip innerHTML sink
$ git status --short
 M index.html  M qa-reports/headless-smoke.mjs  M src/main.ts  M src/styles.css   (+772/-11)

Added-line scan (U-D7 diff):
fetch/XHR/WebSocket/sendBeacon = 0      npm/cargo dep changes = 0 (package.json, Cargo.* 0-diff)
new innerHTML sink sites      = 2 (main.ts:532, main.ts:731 — both fully esc()'d)
insertAdjacentHTML/eval/Function = 0    ~/.ssh path references = 0 (src-tauri 0-diff)
U-D7 smoke checks             = 17 (>= 8 required)
sftp_safe guard call sites    = runAiOnce + alwaysAllowAi + executeAiCommand choke point
approval classes              = run:btn-primary / always:btn-secondary / reject:btn-ghost (frozen contract intact)
```

**Verdict: FINDINGS — 1 MEDIUM, 4 LOW, 0 Critical/High.** Approval contract §13.6 honored end-to-end (state machine, INV-25 class weights, INV-26 guard on every path, zero implicit execution, exact-byte allowlist, revoke list); no new network, dependency, or execution path. UD7-SEC-01 should be re-worked to the `7cccf6c` template pattern before canonization; UD7-SEC-02..05 are cheap hardening items. No code was modified, committed, or pushed in this scan.
