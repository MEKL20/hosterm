# QA: hosterm UI redesign — 2026-10-08

Verifier attribution: QA subagent ran build re-verification and contrast
re-measurement, then died at its final message (gateway 502). The parent
completed the remaining layers directly; all claims below were executed by
whoever states them, never taken from the coder's summary.

Producer claim baseline re-verified by parent before QA: `npm run build`
PASS (exit 0, vite 519ms), `cargo test --lib` 41 passed (parent's own run,
backend untouched — diff touches only index.html, src/main.ts, src/styles.css).

## Layer A: spec compliance (static) — PASS with 2 deviations (both fixed)

- Tokens §2: all new values present in styles.css (`--fg #c8d3f5`,
  `--muted #8b98c9`, `--accent2`, `--ok/--warn`, `--border #6270a2`,
  `--border-soft`, `--hover`, `--font-sans/--font-mono`). Verified in
  built CSS bundle too (grep dist/assets/*.css: c8d3f5, 8b98c9, 6270a2 present).
- S-5 rows: `role="button" tabindex="0" aria-label`, `.h-acts` real `<button>`
  elements with SVG icons (16px, currentColor, stroke 1.5), aria-labels set
  dynamically (`Open SFTP for {name}` / `Edit {name}`) — verified in headless
  DOM dump, not just source.
- S-6 tabs: `role="tablist"`/#tabs, tabpanel role on panels, dirty dot as
  `.t-dot` element, close is a real button, overflow-x with thin scrollbar.
- S-7: xterm theme 23 keys (spec 26 minus scrollbarSlider* 3 keys — coder
  deviation #1, declared: not in xterm 5.5 ITheme typing; values kept as
  comment). CodeMirror dark via `EditorView.theme` + `HighlightStyle.define`
  (`@codemirror/language`, no new deps).
- S-10: overlay manager (Escape, focus trap, inert, restore), `dialog()`
  primitive, delete flows focus Cancel, dirty close = Save and close /
  Discard / Cancel. `confirm(`/`alert(` count in src/: **0**.
- S-11: `outline: none` count in styles.css: **0**; `:focus-visible` rules
  present (verified in-browser via cssRules scan).
- Copy: em dash count in UI files: **0**; errors follow "Failed to X + next
  action" pattern.
- Grep sweep (parent, python unicode ranges): emoji/arrow/box glyphs in
  src/main.ts + styles.css + index.html: **0** (dist bundle contains vendor
  charset/completion glyphs from xterm/CodeMirror libraries only).

Deviation #2 (found by Layer B, fixed during QA): the CSS rewrite dropped the
`#main` flex-container rule, so `#panels { flex: 1 }` collapsed to zero height
— every panel (SFTP/editor/terminal) rendered crushed. Headless smoke caught
it (`.sftp-wrap` computed height 24px). Fixed by parent with one rule
(`#main { flex: 1; display: flex; flex-direction: column; min-width: 0; }`),
rebuilt, re-smoked.

## Layer B: functional regression (headless Chromium) — 13/13 PASS

Script: `qa-reports/headless-smoke.mjs` (playwright chromium-1243 via
playwright@1.63.0 --no-save; removed after run). Mocks
`window.__TAURI_INTERNALS__.invoke`. Results (real output):

```
PASS host list renders — count=1
PASS sidebar sftp action is a button
PASS action buttons not hover-hidden — opacity=1
PASS sftp rows render from mock — rows=2
PASS dir size cell empty (no em dash) — ""
PASS editor opens with mocked file — hello editorline two
PASS dirty dot appears on edit — dots=1
PASS dirty close uses in-app dialog (no native) — native=null
PASS dialog has >=3 action buttons — buttons=11
PASS raw modal opens
PASS Escape closes modal
PASS :focus-visible rule present
PASS no console/page errors
13/13 passed
```

Incidental verification of S-12/S-13: an early mock gap made the host list
fall into its error branch, which rendered
`.list-state.err` "Could not read ~/.ssh/config: … Check the file, then press
Reload." — the error state and copy rules work as specced.

## Layer C: residual check of the 7 HIGH findings — all resolved

- H-1 (muted 2.76:1): tokens now `#8b98c9`; contrast-check re-run by QA child:
  11.47 / 6.37 / 6.79 etc. all match DESIGN.md §2. One doc-claim deviation:
  `selectionInactiveBackground` foreground ratio is actually 8.78:1 vs 5.68:1
  documented — conservative direction, spec note only.
- H-2 (outline:none): 0 occurrences; ring via :focus-visible (smoke-verified).
- H-3 (Escape/focus on modals): smoke-verified (Escape closes; overlay
  manager in code).
- H-4 (emoji icons): 0 in app sources; SVG icon set in place (smoke-verified
  buttons render SVGs).
- H-5 (em dash): 0 in UI strings; dir size cell empty (smoke-verified).
- H-6 (undesigned states): designed + incidentally verified (above).
- H-7 (no DESIGN.md): committed at a066737.

## Boundary: tested vs not tested

Tested: built bundle (vite output), DOM behavior headless (real chromium),
source greps, contrast re-measurement, cargo test (parent).
Not tested: real PTY terminal rendering (needs Tauri backend + real ssh —
headless mock cannot exercise xterm stream), WebKit-specific quirks (tested
in chromium only), real keyboard focus order beyond scripted Tab/Escape,
screen-reader announcement quality (aria labels present but not a11y-tree
audited), native Windows WebView2 runtime.
