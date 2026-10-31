# SEC review — U-D1 Dusk token swap (token/literal)

- **Date:** 2026-10-31
- **Reviewer:** crew-coding-security (task t_a9e3ce82)
- **Mode:** WHITEBOX, read-only. Zero traffic, no edits to reviewed files.
- **Baseline:** working tree vs `HEAD` = `d36a137` (docs: redesign Dusk v1 chain).
- **Scope (tested ONLY these):** `src/styles.css`, `src/main.ts` (U-D1 token swap
  hunks), `qa-reports/headless-smoke.mjs`.
- **Out of scope (observed, NOT probed):** `src-tauri/src/lib.rs` (+515, untracked
  feature work) and `src/assets/snippets.default.json` (new, untracked) are present
  in the working tree but belong to a different unit. Not reviewed here.

## VERDICT: CLEAN

0 Critical / 0 High / 0 Medium. 2 INFO (non-security hygiene). The unit is a pure
CSS-var rename + static hex/rgba literal recolor (Abyss Emerald → Dusk) plus
test-assertion updates. No IPC, no new user input, no new sink, no network.

---

## Coverage ledger (every surface reviewed, incl. clean)

| Surface | What was checked | Status |
|---|---|---|
| `src/main.ts` cmTheme / cmHighlight | CSS-var rename + static hex recolor only; no dynamic value, no interpolation | CLEAN |
| `src/main.ts` TERMINAL_THEME (xterm) | static literals (hex + rgba); locked Dusk values; no CSS-var read, no input-derived value | CLEAN |
| `src/main.ts` — new innerHTML/sink | diff adds **0** innerHTML/outerHTML/eval/Function/insertAdjacentHTML lines | RULED_OUT |
| `src/main.ts` markTabDead invariant | `markTabDead`/`ico.innerHTML` (L1601) uses static `ICONS.reload`/`kindIcon`; untouched, outside U-D1 diff | RULED_OUT |
| `src/styles.css` token block (:root) | 16 Dusk tokens = static hex/rgba literals; no secret, no url() | CLEAN |
| `src/styles.css` network (webfont) | **0** `@import`, **0** `url(http…)`, **0** `url()` of any kind; font stacks local/system-only | RULED_OUT (NFR-DK6 zero-network holds) |
| `index.html` network (adjacent) | only `<link href="/src/styles.css">` (local); one pre-existing `<a href="https://learn.microsoft.com…">` doc link, NOT introduced by U-D1 | CLEAN |
| Hardcoded secrets in tokens/literals | all added values are color hex / rgba / font-family names — no credential-shaped literal | RULED_OUT |
| `qa-reports/headless-smoke.mjs` | assertion RGB swaps + new read-only gates (INV-21 grep, font-CDN gate, glow-removal, literal-remap); all `page.evaluate`/`readFileSync`; no child_process/exec/network/injection | CLEAN |
| INV-21 (stale Abyss literal in src/) | grep for Abyss hex/`var(--glow)` in main.ts+styles.css = **0 in live code** (see INFO-1) | CLEAN |
| Build integrity | `npm run build` (tsc + vite) → exit 0, no TS error → no dangling token ref | CLEAN |

---

## Findings

### INFO-1 — stale Abyss token NAMES survive in comments (dead-comment drift)
- **Severity:** INFO (not security; INV-21 hygiene).
- **Location:**
  - `src/styles.css:220` `/* …SFTP --accent2, dead --danger */`
  - `src/styles.css:247` `/* terminal well (§4.3): bg --well… */`
  - `src/styles.css:262` `/* …bg --bg2 */`
  - `src/styles.css:400` `/* …chip sumber (▲, --warn) / target (▼, --accent2)… */`
  - `src/main.ts:298` `// …3 static skeleton rows (--hover, pulse-free)…`
  - `src/main.ts:406` `// live-session dot: --ok while the host…`
- **Description:** old token names (`--accent2`, `--well`, `--bg2`, `--warn`,
  `--hover`, `--ok`) linger in **comments only**. No live `var(--…)` or hex
  literal references them (verified by grep + clean build). The smoke INV-21 gate
  checks Abyss *hex* literals + `var(--glow)`, which are all gone — comments are
  not caught by it and are not a security concern; flagged for doc-hygiene so the
  comments don't mislead a future reader into thinking the old tokens still exist.
- **Closure:** CONFIRMED (misleading dead comments), impact = documentation only.
- **Counterevidence:** none needed — comments have no runtime effect; not a vuln.
- **Recommended fix (optional, not SEC-blocking):** refresh the six comment lines
  to Dusk names in a later cosmetic pass.

### INFO-2 — selection/accent-soft literals moved to rgba() in JS theme
- **Severity:** INFO.
- **Location:** `src/main.ts:103-105` (`selectionBackground: "rgba(226,163,92,.18)"`,
  `selectionInactiveBackground: "rgba(226,163,92,.12)"`).
- **Description:** xterm ITheme selection values changed from opaque hex to rgba
  literals. Still fully static string literals, not computed and not input-derived.
  Noted only because it's a shape change (hex→rgba) rather than a straight recolor.
- **Closure:** RULED_OUT as a vuln — static literal, no injection path.

---

## Boundary — what was NOT tested and why
- **Dynamic / runtime:** no app run, no PoC traffic (static review was sufficient;
  the change has no reachable sink). Visual/contrast correctness of the Dusk
  palette is a QA/UX concern, not SEC.
- **Full headless-smoke execution:** not run here (needs playwright install/teardown
  per AGENTS.md; that is the QA gate). Build (`tsc`+`vite`) WAS run and passed.
- **`src-tauri/src/lib.rs` (+515) and `snippets.default.json`:** out of scope;
  present in the working tree as a separate unit — flagged to nano, not reviewed.

## Confidence
High for this unit: it is mechanically a rename+recolor with a diff that provably
introduces no sink, no network, and no secret (static-only review is adequate when
there is no reachable dynamic surface, which is the case here).
