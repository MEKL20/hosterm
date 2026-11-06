# Security review — OAuth durable-refresh gate, Drive feedback toasts, account UI

Date: 2026-11-07
Reviewer: crew-coding-security (read-only gate)
Scope: uncommitted working-tree diff of root card t_3c9ed4f3 (post backend+frontend impl)
Mode: WHITEBOX (static read-only) + verify loop (cargo test, build). No dynamic/network work.

Target files (git diff vs HEAD, all unstaged):
- src-tauri/src/oauth.rs  (+96)
- src/main.ts             (+184)
- index.html              (+41)
- src/styles.css          (+94)
- qa-reports/headless-smoke.mjs (+490)

## Result: NO FINDINGS (0 Critical/High/Medium/Low)

## Invariants verified (each preserved)

- **PKCE S256 / state CSRF / loopback single-shot / drive.file scope / keyring refresh_token / no
  client_secret**: diff does not touch any of these paths. `interactive_sign_in` change is confined to
  gating the exchange outcome; existing PKCE/state/loopback/scope logic unchanged. cargo test
  `loopback_accepts_single_request_and_parses` still green.
- **No token/credential in logs/errors/IPC/source**: diff-scoped grep for
  `println!|eprintln!|console.(log|error|warn)|client_secret|GOCSPX|refresh_token =|access_token =`
  → 0 matches. New `durable_refresh_or_error` error string carries no token material; tests assert the
  error never contains `refresh_token`/`access_token` substrings.
- **False signed-in state fixed (root cause of v0.14.1)**: `durable_refresh_or_error` rejects both
  `None` and empty-string refresh tokens *before* userinfo / before presenting an account, so a
  sign-in that yields no durable keyring token now fails loudly instead of showing a usable-looking
  account that cannot do Drive work. Pure fn, tested 4 ways (missing/empty reject; stored/rotated
  pass-through). Error copy is actionable and non-sensitive.

## ADR-8 (safe DOM rendering) — remote-origin strings

All dynamic account/backup/restore/error strings reach the DOM via `textContent`, never `innerHTML`:
- popover title/sub/avatar initials → `textContent` (main.ts ~346–357).
- toast `title`/`msg` → `textContent` on cloned `#toast-template` nodes (main.ts showToast).
- chip name → `textContent`; chip `aria-label` via setAttribute.
- `googleAccount.picture` is stored only as `data-picture` via `escAttr` and **never fetched**: no
  `<img>` element created, and `grep url(|attr(|data-picture` in styles.css → 0 — the attribute is dead
  storage. `img-src 'self'` is the structural backstop.
- Smoke asserts hostile bytes render inert: error toast with `<img src=x onerror=alert(1)>`
  → `msgKids === 0`, exact textContent, no `__pwn`; identity email with injected `<img ... onerror>`
  → `subKids === 0`.

## Toast lifecycle

- Non-modal, never joins overlayStack, never traps focus, not dismissed by Escape (Escape stays owned
  by overlays). Cannot hang the UI: overflow eviction is synchronous (`dropToastNow`) — the animated
  remover is explicitly not used in the eviction loop, avoiding the infinite-reselect hang.
- Max-stack guarantee intact: caps at 3 visible; errors are never silently evicted (evicts oldest
  non-error; if all visible are errors, keeps them). Smoke proves this (1 error + 3 success → 3 visible,
  error retained, newest-first).
- Error text comes only from `msg(e)` (backend error message), success copy from
  `res.name/bytes/restored.length/pre_restore_dir`. The passphrase plaintext never reaches a toast or
  status line.
- aria-live=polite container; error toasts carry role=alert (assertive); success/info role=status.

## Passphrase dialog

- `cleanup()` wipes `input.value` and `confirmInput.value` on every finish path (ok + cancel) before
  the overlay goes hidden. Plaintext passes straight to `invoke()`; it is never stored in a module-level
  variable (`passDialogResolve` holds only the onCancel closure) and never logged.
- Inert/trap semantics preserved: `#app` gets `inert` while overlay open; focus restored only after the
  dialog is actually hidden (deterministic 140ms fallback for reduced-motion where transitionend never
  fires). `passDialogExitTimer` is cleared on reopen, so a stale exit timer cannot re-hide a freshly
  reopened dialog — confirmed by reading openOverlay/closeOverlay.

## CSP / network

- `tauri.conf.json` CSP unchanged and strict: `script-src 'self'`, `img-src 'self'`,
  `connect-src 'self' ipc: http://ipc.localhost`, `style-src 'self' 'unsafe-inline'` (CodeMirror only),
  `object-src 'none'`, `frame-ancestors 'none'`, `form-action 'none'`. The conf stat change in the
  commit history is a version bump, not a CSP edit (working-tree `git diff tauri.conf.json` empty).
- No new external/network asset. No remote avatar fetch (see ADR-8). NFR-3 (no idle/background network)
  preserved.

## Smoke mocks — false-positive check

The new smoke assertions are red-capable security coverage, not label-only: hostile-byte injection with
child-node counting, CSP-violation counter asserted 0 on the toast surface, deterministic eviction and
timer-pause tests on an isolated fake-clock page. No weakening of existing gates.

## Verify loop (real output)

- `npm run build` → built OK, 0 TS errors.
- `cd src-tauri && cargo test --lib` → **175 passed; 0 failed** (baseline 164, counts grew only).
- `cargo test --lib durable_refresh` → 4 passed (the new gate tests).
- Playwright smoke NOT re-run here: nano already verified canonical 440/440, CSP 0 on t_cee5db41;
  this review relies on static read + cargo + build as the security evidence.

## Residual risks (not blockers)

1. `data-picture` holds the remote avatar URL as an attribute. Dead today (no fetch), but a future CSS
   `url(attr(data-picture))` or an `<img>` added later would turn it live and defeat NFR-3. Low — flag
   for anyone touching the chip avatar.
2. Durable-refresh error directs users to revoke access at myaccount.google.com. Correct UX, but if
   Google stops omitting refresh_token on re-consent this path becomes unreachable (benign).
3. Static-only review: no live OAuth exchange exercised (no owner dynamic-approval line, and not needed
   for a code gate). Confidence: medium-high on the code paths read; the runtime browser/keyring I/O in
   `interactive_sign_in` remains untested by design (module header).

Read-only: no files edited, no commit, no push.
