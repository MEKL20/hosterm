# AGENTS.md

Agent instructions for working in this repo. The operational backbone — architecture, invariants,
commands, pitfalls, and the canonical test baselines — lives in [docs/STATE.md](docs/STATE.md).
**Read STATE.md before touching code.** When this file and a live run disagree, the live run wins
and STATE.md is the number of record.

## 0. Karpathy principles (mandatory, every task)

1. **Think before coding.** Read the relevant code and docs first; trace a symbol to its definition
   and usages. State the plan before editing. No speculative edits.
2. **Simplicity first.** Local-first, auditable, fewest moving parts. No new dependency, abstraction,
   or layer unless the task cannot be done with the stdlib / an existing dep. "New dependency = the
   task states why existing code can't do it."
3. **Surgical changes.** Touch only what the task needs. No drive-by refactors, renames, or
   reformatting. A bug fix does not clean up surrounding code; a small feature does not grow config.
4. **Goal-driven execution.** The deliverable is a working, verified artifact backed by real tool
   output — not a plan or a stub. Finish the verify loop (§4) before claiming done.

## 1. Source of truth & file-scope map

Product truth flows PRD → SDD → TSD → tasks. For process/baselines, STATE.md wins; for behavior/
architecture, SDD wins; for product scope, PRD wins.

- `docs/PRD.md` — product truth (what/why, EARS acceptance criteria).
- `docs/SDD.md` — architecture truth (components, IPC contract, ADRs, invariants).
- `docs/TSD.md` — execution truth (locked stack, rollout, release procedure, dev rules).
- `docs/STATE.md` — **canonical repo state and test baselines**, re-measured each dispatch.
- `DESIGN.md` — the Dusk visual contract (tokens, WCAG-measured pairs, component specs).

Where code lives (change only the file the task owns; never two cards on the same hotspot file):

| File | Scope |
|---|---|
| `src/main.ts` | ALL frontend: tabs, terminals (`sessions`/`splitSessions`), SFTP browser, remote editor (CodeMirror), host form / raw editor / dialogs, split divider (manual mouse), status bar, Dusk Settings/Appearance, Snippets drawer, Google chip (visual-only), AI approval state machine. All rendering is DOM. |
| `src/styles.css` | Active **Dusk** theme `:root` + all component CSS. No Abyss-Emerald literals, no `var(--glow)` (INV-21). |
| `index.html` | Static markup + static templates the renderer clones (AI/approval/Google surfaces clone templates, never build via dynamic `innerHTML`). |
| `src-tauri/src/lib.rs` | Every backend `#[tauri::command]` except fleet; config parse/serialize (sacred: preserves comments/order/unknown directives); atomic write; secrets sidecar. `sftp_safe` lives here (`pub(crate)`). |
| `src-tauri/src/fleet.rs` | ALL Fleet Run code: pure fns + DTOs. No fleet logic elsewhere. |
| `qa-reports/headless-smoke.mjs` | Playwright-chromium smoke over `dist/` with the prod CSP header + mock invoke map. Extend here for every new behavior. |

IPC inventory is a gate: `grep -rc "tauri::command" src-tauri/src/*.rs` must match the count STATE.md
records (currently `lib.rs` + `fleet.rs`). Never invent commands, fields, or files — go read first.

## 2. Hard invariants (never break)

1. `~/.ssh/config` (the user's real file) must stay parseable by REAL `ssh` at all times. **No secrets
   in the config — ever.** Secrets live only in the sidecar `~/.ssh/hosterm.secrets` (0600). Secrets
   never cross the IPC boundary.
2. Config parse/serialize round-trip preserves comments, order, and unknown directives byte-for-byte
   outside the edited lines. Writes are atomic (sibling temp → chmod 0600 → rename) and refuse to
   overwrite an unreadable original.
3. Never touch the dev machine's real `~/.ssh/authorized_keys`.
4. Founding principle: spawn the OS's REAL `ssh`/`sftp` binaries. Do NOT introduce ssh2/russh-style
   libraries, and never bundle ssh/sftp binaries (ADR-1).
5. Drag/copy interactions use manual mouse events (mousedown/mousemove/mouseup), **NOT** HTML5
   drag-and-drop (dead in WebView2 — proven). `draggable` grep in `src/` stays 0.
6. PTY paste normalizes CRLF→LF (`pasteNormalize` in `src/main.ts`).

### Dusk invariants (INV-21..26 — named gates)

- **INV-21 anti-Abyss:** zero `#08080a`/`#050507`/`#00d992`/`#818cf8`/`#0e2a20`/`var(--glow)` under
  `src/`; rendered `:root` stays byte-exact Dusk. Gate: source scan + smoke.
- **INV-22 store isolation:** settings/snippets/allowlist never read or write `~/.ssh/config` or the
  secrets sidecar — app-data only (`dirs::config_dir()/hosterm/`). Gate: cargo byte-identity test.
- **INV-23 accent ink:** the five curated accents map to `#241404`; every other valid accent maps to
  the safer `#000000`/`#ffffff` endpoint per frozen SDD §13.2.1 (independent WCAG ratio ≥ 4.5).
- **INV-24 corrupt defaults:** missing/corrupt `settings.json` returns defaults and leaves the corrupt
  bytes untouched until a legitimate write.
- **INV-25 approval visual weight:** Run once = `btn-primary`; the permission-widening "Always allow"
  = `btn-secondary`, never primary, and the AI assistant NEVER auto-runs.
- **INV-26 host alias guard:** leading `-`, CR, or LF is refused before any approval persist or PTY
  execution.

### Safe DOM rendering (ADR-8)

Remote bytes (SFTP names/paths, fleet output, file contents) are hostile input. Every render of
remote-origin content routes through the escape helpers `esc()` / `escAttr()`; dynamic strings are
never interpolated into `innerHTML`. New renderer surfaces clone static templates from `index.html`
rather than building markup from attacker-influenced strings. CSP (ADR-6) is the structural backstop,
escaping is the hygiene layer — both apply, neither substitutes for the other. New `innerHTML` without
`esc()`/`escAttr()` is a review-blocker.

### No background network / telemetry (PRD NFR-3)

The app makes zero network calls at idle and during normal use — the only outbound traffic is
user-initiated SSH/SFTP (and, once wave-2 M3 ships, the single user-initiated update check against one
GitHub Releases endpoint). No telemetry, no analytics, no webfonts/CDN (local system fonts only), no
polling, no silent/background updates. `withGlobalTauri: false` stays off. The smoke network log
asserts this.

## 3. CSP (frozen — edits are a security review, not a normal PR)

`app.security.csp` in `src-tauri/tauri.conf.json` keeps `script-src 'self'` and `connect-src` scoped to
the IPC origin; `'unsafe-inline'` stays confined to `style-src` only (CodeMirror `style-mod`). Any edit
re-runs the `docs/SPIKE-csp-findings.md` analysis and must keep the smoke CSP-violation counter at 0.

## 4. Verify loop (canonical — every change, every PR)

Baselines are **not hardcoded here.** The authoritative counts live in `docs/STATE.md` §Verify loop and
are re-read and re-measured at each dispatch. A PR that lowers any baseline fails review regardless of
PASS/FAIL wording; smoke counts may only grow.

1. `cd src-tauri && cargo test --lib` → all green at the STATE.md cargo baseline.
2. `npm run build` → 0 TS errors.
3. `npm install --no-save playwright@1.63.0 && node qa-reports/headless-smoke.mjs` → all green at the
   STATE.md smoke baseline, CSP-violation counter == 0, served CSP == `tauri.conf.json` policy.
   Then remove Playwright before staging (do NOT `rm -rf` in headless single-query mode):

   ```sh
   mkdir -p "$TMPDIR/pw-trash"
   mv node_modules/playwright node_modules/playwright-core "$TMPDIR/pw-trash/" 2>/dev/null || true
   unlink node_modules/.bin/playwright 2>/dev/null || true
   unlink node_modules/.bin/playwright-core 2>/dev/null || true
   ```

4. Extend `qa-reports/headless-smoke.mjs` for new behavior in the same PR; keep the named-invariant
   gates (INV-21..26, draggable/copy/reduced-motion/CSP-hostile) green.
5. Pre-stage grep: `target/|node_modules|.exe$|dist/` → 0 tracked matches; secrets-in-config grep → 0.

A worker reporting done without quoting real loop output gets RETURNed, not reviewed. If a step can't
be run (missing dep, no Windows host), say so plainly — never fabricate output.

## 5. Independent verification

Self-reports are not evidence. Every gate claim travels with reproducible proof: the exact command and
its real output, plus task ids + commit hash on the push record. The orchestrator re-verifies a child's
"PASS" against the live tree before gating a release; "it built" / "tests pass" without quoted output is
treated as unverified.

## 6. Roles, commits & push boundaries

- **Developers never commit and never push.** Code lands in the task's workspace (scratch/worktree);
  docs land in their `dir`. The lead collects, re-verifies, and pushes.
- Gate chain before a lead push of shipping code/config: lead code review + crew-coding-qa verify PASS
  + crew-coding-security clean. Docs / CI-workflow files: nano verification + lead, no full chain. When
  unsure which side an artifact is on, treat it as code and apply the full gate.
- **Releases (tags) are cut by the orchestrator only**, never by per-task agents.
- Push to a new branch, never directly to `main`, unless explicitly told. Only commit when explicitly
  asked.

## 7. Commit hygiene & version/release

- Before every commit, grep staged files for `target/|node_modules|.exe$|dist/` and for secrets in
  config → both 0. Confirm Playwright residue is gone (§4 step 3).
- A version bump touches **all four** in one step: `package.json`, `src-tauri/tauri.conf.json`,
  `src-tauri/Cargo.toml`, then `(cd src-tauri && cargo update -p hosterm --offline)` to refresh the
  lockfile. Keep the `APP_VERSION` string in `src/main.ts` in sync with the bump (status bar reads it).
- Full release procedure (orchestrator): commit work → bump the four files → commit → tag `v*` →
  push tag → poll the Actions API → **verify installers are actually attached** to the release page by
  eye (a tag once shipped zero assets; the CI gate now fails loudly, but still confirm).
- Signing is currently DEFERRED — releases ship UNSIGNED (SmartScreen "unknown publisher"); the
  SignPath workflow is inert and fail-open (STATE.md §Wave-2 B signing). Do not add claims of signed
  releases until signing is actually enabled.
- Preserve hooks; use non-destructive git by default (no force push / reset --hard / branch -D without
  explicit permission). Leave git config and secret files alone.
