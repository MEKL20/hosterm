# hosterm STATE (brief backbone for agents/children)

Read this before touching code. It is the shared context: architecture, invariants, commands, pitfalls.

## What hosterm is
Desktop SSH/SFTP client, Tauri v2 + TypeScript, Windows-first (WebView2) but dev'd on Linux. Founding principle: spawn the OS's REAL `ssh`/`sftp` binaries (portable-pty + xterm.js for terminals; batch `sftp -b -` for transfers) — no ssh2/russh libs.

## Layout
- `src/main.ts` — all frontend: tabs, terminals (sessions/splitSessions maps), SFTP browser (openSftp/renderSftp/renderLocalPane), remote editor (editorSessions/CodeMirror), dialogs.
- `src/styles.css` — theme (Tokyo Night-ish); layout chains: `#panels .panel` (absolute) → `.sftp-wrap` height:100% → `.sftp-panes` flex → `.sftp-list` flex:1 overflow-y:auto.
- `src-tauri/src/lib.rs` — every backend command; config parse/serialize is sacred (preserves comments/order/unknown lines); secrets sidecar `~/.ssh/hosterm.secrets` (0600).
- `qa-reports/headless-smoke.mjs` — 43+ checks, Playwright chromium, mock invoke map (hosts: prod-web, db-core).

## Hard invariants (never break)
1. `~/.ssh/config` must stay parseable by REAL ssh at all times. No secrets in it — ever. Sidecar only.
2. Round-trip edits preserve comments, order, unknown directives.
3. Paste into PTY normalizes CRLF→LF (`pasteNormalize`).
4. Drag interactions use manual mouse events, NOT HTML5 DnD (dead in WebView2).
5. Staging grep before commit: `target/|node_modules|.exe$|dist/`.
6. NEVER touch the dev machine's real `~/.ssh/authorized_keys`.

## Verify loop (every change)
1. `cd src-tauri && cargo test --lib` → 48.
2. `npm run build` → ~500ms, no TS errors.
3. `npm install --no-save playwright@1.63.0 && node qa-reports/headless-smoke.mjs` → 43+; then `rm -rf node_modules/playwright node_modules/playwright-core node_modules/.bin/playwright*`.
4. Add/extend smoke checks for new behavior.

## Release (orchestrator does this, not children)
Commit → bump version (package.json, tauri.conf.json, Cargo.toml + `cargo update -p hosterm --offline`) → commit → tag → push tag → poll Actions API → report release URL.
