# QA gate — Google backup session + account UX fixes

- Date: 2026-11-07
- Target: hosterm working tree on `main` (uncommitted diff), HEAD `9e148b3`
- Tester: crew-coding-qa (kanban t_44bed95a)
- Spec: `design-reviews/2026-11-07-google-account-backup-ux.md`; STATE.md baselines
- Root card: t_3c9ed4f3 · impl cards t_7f5686c9 (BE oauth), t_cee5db41 (FE)
- Mode: read-only verifier. No production source edited. Report-only artifact.

## Verdict: PASS — 0 findings

All required gates green against the real diff. No blocker, major, or minor defect.

## Diff under review
```
 index.html                    |  41 +++-
 qa-reports/headless-smoke.mjs | 490 +++++++++-
 src-tauri/src/oauth.rs        |  96 +++-
 src/main.ts                   | 184 +++++-
 src/styles.css                |  94 +++-
 5 files changed, 878 insertions(+), 27 deletions(-)
```
Untracked (not code): `design-reviews/…ux.md`, `security-reports/…ui.md`.

## Verify loop (real output)

1. **cargo test --lib** → `175 passed; 0 failed` (10.75s). Matches STATE.md §Verify loop baseline (175). Durable sign-in gate subset `durable_refresh` → `4 passed; 0 failed`.
2. **npm run build** → `tsc && vite build` → built in 730ms, **0 TS errors**.
3. **node qa-reports/headless-smoke.mjs** (canonical, no artificial timeout) → **440/440 passed**, exit 0, wall-clock **2:05.78**. STATE.md baseline is 410/410 (nav-rail); this diff adds 30 FE-FIX checks → 440 (baseline only grows, per AGENTS.md §4). No external wrapper kill; single canonical run.
   - `CSP violations == 0 across suite` PASS; `served CSP == tauri.conf.json policy` PASS; `winPage CSP violations == 0` PASS.
4. Playwright removed after run (move to `$TMPDIR/pw-trash`, unlink bins). `node_modules/playwright` absent.
5. Pre-stage grep: tracked `target/|node_modules|.exe$|dist/` = **0**; untracked artifact dirs = none; secrets-in-`~/.ssh/config` = 0.

## Required-area inspection (code + regression assertions)

All items from the task checklist confirmed present in the shipped diff AND locked by a smoke assertion:

- **Passphrase fixed overlay / geometry unchanged** — `#pass-dialog` added to the fixed-overlay selector (`styles.css:1287`). Smoke `FE-FIX F-1`: `#pass-dialog` computed `position:fixed`, viewport-sized, centered, opacity 1; shell app/rail/panel rect identical closed vs open.
- **inert / focus trap / Escape / focus restoration / passphrase wipe** — `#app` gets `inert` at stack depth 1 (`main.ts` openOverlay); `cycleFocusWithin` traps; Escape → cancel. Smoke `FE-FIX F-1/§2.5`: Escape hides dialog, clears `ov-enter`, releases shell, restores focus to the Backup button, and wipes the passphrase inputs.
- **Normal + reduced motion** — scoped `#pass-dialog` fade/lift 120ms via `.ov-enter`; close path uses `transitionend` + 140ms fallback timer (reduced-motion never fires `transitionend`). Smoke `FE-FIX F-2`: reduced-motion overlay reaches the visible final state (opacity 1, fixed).
- **Backup success/failure, restore success/failure, no-backups toast role/copy/persistence** — `showToast` routes all four terminal outcomes; `#drive-status` cleared so the message does not double. Smoke `FE-FIX §2.3`: backup success = 1 `role=status .toast--ok` "Backup complete"; backup failure = persistent `role=alert .toast--error` "Backup failed"; restore success/failure analogous; `nano-lock #5` no-backups = neutral `role=status .toast--info`, locked copy "No backups on Drive yet. Back up first to enable restore."
- **Hostile dynamic strings inert + CSP 0** — all toast text set via `textContent` on the cloned `#toast-template` node (no `innerHTML`). Smoke `FE-FIX ADR-8`: hostile error byte renders as `msgKids === 0`, `window.__pwn` undefined, CSP 0.
- **Hover/focus timer pause, keyboard close, focus ring** — `armToastTimer` pauses on mouseenter/focusin, resumes on mouseleave/focusout. Smoke (fake-clock page): success alive at 4000ms, auto-dismisses after 5000ms; pointer hover and keyboard focus each survive 8000ms held and resume on leave/blur; close button is keyboard-focusable with a 44px hit target; Enter dismisses; `:focus-visible` = 2px `--accent` ring.
- **Max-stack: no UI-thread hang, synchronous oldest non-error eviction, all-error preservation** — eviction loop uses synchronous `dropToastNow` (not the animated remover, which would leave the victim in-DOM and spin the loop). Smoke `FE-FIX §2.3`: caps at 3 visible, newest-first, never evicts the error; four simultaneous errors all preserved and never auto-dismiss.
- **Signed-in account hierarchy, no remote avatar request, narrow/200% geometry, Dusk themes** — popover identity row: 32px initials `#pop-avatar` (initials only, never the remote picture), name 13px/650 `--text` focal point, email `--dim`, Drive group label, footer groups Settings + Sign out under a hairline. Smoke `FE-FIX §2.2`: identity row shape; hostile email inert text in `--dim`; `FE-FIX NFR-3`: opening the signed-in popover fires no remote image request. `FE-FIX §2.5`: passphrase box + toast stack stay inside a 360px viewport (no horizontal overflow). Token-driven styling resolves per active theme; theme preset swaps (dusk/ink/light) covered by existing THEME v1 smoke.

## Backend sign-in gate (t_7f5686c9)
`durable_refresh_or_error` rejects a sign-in that returns no durable refresh_token BEFORE presenting an account (root cause of the v0.14.1 false signed-in state). Tests: rejects None, rejects empty string, accepts stored token, passes through rotated token — `4 passed`. Error carries no token-shaped material (asserted). This is the already-accepted BE fix; re-verified clean in the aggregate.

## Invariants re-checked
- INV-21 anti-Abyss source scan (`#08080a|#050507|#00d992|#818cf8|#0e2a20|var(--glow)` under `src/`) → 0 matches.
- `draggable` grep in `src/` → 0.
- No `innerHTML` assignment in the new toast/account code (only a comment mentions it); new surfaces clone `#toast-template` / set `textContent` (ADR-8).

## Not tested (boundary)
- Live rendered app at real DPI / WebView2-specific layout (no Windows host; headless Chromium only — same waiver class as STATE.md §Risks).
- Actual focus-ring pixel rendering (asserted by CSS rule + computed style, not by eye; host has no vision model).
- Real Google Drive round-trip (mocked `drive_backup`/`drive_list_backups`/`drive_restore` in smoke; live OAuth is post-release manual acceptance per prior cards).

## Severity counts
Critical 0 · High 0 · Medium 0 · Low 0.
