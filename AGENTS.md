# AGENTS.md

Agent instructions for working in this repo. Full context lives in [docs/STATE.md](docs/STATE.md) — read it before touching code.

## Hard rules (never break)

1. `~/.ssh/config` (the user's real file) must stay parseable by REAL ssh at all times. No secrets in the config — ever. Secrets live in the sidecar `~/.ssh/hosterm.secrets` (0600). The config parse/serialize round-trip must preserve comments, order, and unknown directives.
2. Never touch the dev machine's real `~/.ssh/authorized_keys`.
3. Founding principle: spawn the OS's REAL `ssh`/`sftp` binaries. Do not introduce ssh2/russh-style libraries.
4. Drag/copy interactions use manual mouse events, NOT HTML5 drag-and-drop (unreliable in WebView2 — proven).
5. PTY paste normalizes CRLF→LF (`pasteNormalize` in `src/main.ts`).

## Verify loop (every change)

1. `cd src-tauri && cargo test --lib` → 49 pass.
2. `npm run build` → no TS errors.
3. `npm install --no-save playwright@1.63.0 && node qa-reports/headless-smoke.mjs` → 51 pass, then `rm -rf node_modules/playwright node_modules/playwright-core node_modules/.bin/playwright*` before staging.
4. Extend `qa-reports/headless-smoke.mjs` for new behavior.

## Commit hygiene

- Grep staged files for `target/|node_modules|.exe$|dist/` before every commit.
- Version bumps touch: `package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, then `(cd src-tauri && cargo update -p hosterm --offline)`.
- Releases (tags) are cut by the orchestrator, not by per-task agents.
