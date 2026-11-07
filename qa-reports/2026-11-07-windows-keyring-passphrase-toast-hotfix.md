# QA Gate — Windows Google keyring, passphrase reveal, toast lifetime

Date: 2026-11-07
Scope: hotfix after v0.14.2
Kanban: `t_06ae82f6`
Verdict: **PASS**

## Root-cause verification

`keyring` 3.6.3 has no default platform credential-store features. The prior manifest enabled Linux Secret Service but omitted `windows-native`, so Windows selected keyring's mock store. OAuth wrote the refresh token to one mock `Entry`; Drive created a new `Entry`, received `NoEntry`, and returned `not signed in to Google — sign in first`.

The manifest now enables `windows-native`, and the Windows-target feature tree reports:

- `keyring feature "windows-native"`
- `keyring feature "sync-secret-service"`
- `keyring feature "crypto-rust"`

A Rust manifest regression test fails if `windows-native` is removed.

## Executed gates

- `npm run build`: PASS, 0 TypeScript errors; Vite production bundle created.
- `node qa-reports/headless-smoke.mjs`: **445/445 PASS**.
- `cargo test --lib`: **176/176 PASS**.
- `git diff --check`: PASS.
- `cargo tree --target x86_64-pc-windows-gnu -e features -i keyring`: PASS; `windows-native` present.

A direct `cargo check --target x86_64-pc-windows-msvc --lib` cannot complete on this Linux host because the MSVC tool `lib.exe` is unavailable. The Windows GitHub Actions build is therefore the executable-artifact gate.

## UI acceptance evidence

- `Show passphrase` button exists, is keyboard focusable, and exposes `aria-pressed`.
- Toggle changes both backup fields from `password` to `text`, then masks both again.
- Every prompt starts masked; cancel/close wipes both values and restores focus without moving the shell.
- Backup success toast survives before the 5-second boundary and auto-dismisses after it.
- Backup error toasts remain visible initially and all auto-dismiss after 5 seconds without a click.
- Restore success/failure uses the same bounded outcome behavior.
- Close button remains available for early dismissal.
- Narrow 360px viewport, overlay geometry, CSP, hostile-string rendering, and signed-in account regression checks pass.

## Visual decision reason

The reveal control is a quiet, right-aligned Dusk text button so it stays visibly secondary to the Back up/Restore CTA, avoids shell-layout movement, and one control keeps the two confirmation fields synchronized.
