# Fresh QA regression — INV-23 / ADR-20 Rev. 2

- **Date:** 2026-11-01 WIB
- **Tester:** `crew-coding-qa` (`t_8afcb484`)
- **Target SHA:** `f56e2ed04a2d744267fbd025afda999a34b46b78`
- **Prior failing baseline:** `ca3e86b25ed64a226fda262ada9c00e2e6f1d5f8`
- **Acceptance source:** `docs/SDD.md` §13.2.1 and INV-23 in §13.8
- **Verdict:** **PASS**
- **Defects:** Critical 0 · High 0 · Medium 0 · Low 0

## Summary

Fresh execution against `f56e2ed` passed all canonical gates and the independent rendered-UI regression:

- Rust: **87/87 passed**.
- TypeScript/Vite build: **PASS**, zero TypeScript errors.
- Canonical headless smoke: **271/271 passed** (freshly re-measured; not the old 255 total).
- Independent focused rendered probe: **25/25 passed**.
- Frozen ADR-20 Rev. 2 vectors: **14/14 passed** using DOM-observed colors and a separate Python WCAG oracle that does not import or execute the implementation helper.
- Former failures `#6f6f6f` and `#777777`: each reproduced **twice**, exact required endpoint both times, ratio ≥4.5.
- Curated and non-curated value-based input/chip/persist/reload determinism: **PASS**.
- Accent remap/reject: **none observed**; entered valid values remained byte-exact after application and persistence.
- Backend diff: **0 lines** committed and **0 lines** in the working tree.
- INV-21 forbidden Abyss-Emerald literals: **0 hits**.
- Focused browser console/page errors: **0**; CSP violations: **0**.

## Canonical gates

| Gate | Result | Evidence |
|---|---:|---|
| `(cd src-tauri && cargo test --lib)` | PASS — 87 passed, 0 failed | `2026-11-01-inv23-adr20-rev2-fresh-evidence/cargo-test-lib.log` |
| `npm run build` | PASS — 26 modules, no TS errors | `2026-11-01-inv23-adr20-rev2-fresh-evidence/npm-run-build.log` |
| `npm install --no-save playwright@1.63.0` | PASS — Playwright 1.63.0 | `2026-11-01-inv23-adr20-rev2-fresh-evidence/playwright-install.log` |
| `node qa-reports/headless-smoke.mjs` | PASS — **271/271** | `2026-11-01-inv23-adr20-rev2-fresh-evidence/headless-smoke.log` |
| Playwright cleanup | PASS — package dirs and `.bin` links absent; package hashes unchanged | `2026-11-01-inv23-adr20-rev2-fresh-evidence/playwright-cleanup.log` |

Rust emitted two pre-existing `unused_mut` warnings at `src/lib.rs:1160` and `:1208`; no failure.

## Independent rendered Settings vectors

Method:

1. Built production `dist/` at the target SHA.
2. Loaded it in Playwright with only Tauri IPC mocked.
3. Opened rendered Settings and drove the real `#accent-input` (`input[type=color]`) by assigning a valid color and dispatching its rendered `input` event.
4. Captured input value, root inline/computed `--accent` and `--accent-ink`, preview variables, and pressed-chip state.
5. Recomputed WCAG ratios in a standalone Python oracle using cutoff `0.04045`; it imports no product or smoke helper.

| Class | Applied accent | DOM ink | Independent ratio | Result |
|---|---:|---:|---:|---:|
| curated | `#e2a35c` | `#241404` | `8.18060039728731:1` | PASS |
| curated | `#5fbfae` | `#241404` | `8.115110618605044:1` | PASS |
| curated | `#e0869b` | `#241404` | `6.842802495508517:1` | PASS |
| curated | `#aec267` | `#241404` | `9.102683799795606:1` | PASS |
| curated | `#b99be3` | `#241404` | `7.529220380096464:1` | PASS |
| free extreme | `#000000` | `#ffffff` | `21:1` | PASS |
| free dark sentinel | `#101010` | `#ffffff` | `19.028110547666497:1` | PASS |
| former failure | `#6f6f6f` | `#ffffff` | `5.024865064757633:1` | PASS |
| grayscale below switch | `#757575` | `#ffffff` | `4.607518093747377:1` | PASS |
| closest 24-bit below switch | `#766cb5` | `#ffffff` | `4.58257700240974:1` | PASS |
| closest 24-bit above switch | `#cf0dcc` | `#000000` | `4.5825758149421025:1` | PASS |
| grayscale above switch | `#767676` | `#000000` | `4.6232848849972035:1` | PASS |
| former failure | `#777777` | `#000000` | `4.68949989000882:1` | PASS |
| free extreme | `#ffffff` | `#000000` | `21:1` | PASS |

Independent oracle values:

- `Lswitch = 0.17912878474779204`
- theoretical selected-endpoint floor `sqrt(21) = 4.58257569495584:1`
- closest-below `#766cb5`: `L = 0.1791287193751157`, selected white
- closest-above `#cf0dcc`: `L = 0.17912879074710514`, selected black

Raw DOM observations: `2026-11-01-inv23-adr20-rev2-fresh-evidence/rendered-settings.json`  
Independent calculations: `2026-11-01-inv23-adr20-rev2-fresh-evidence/wcag-ratios.json`

## Former-failure repetition

| Probe | Run 1 | Run 2 | Exact endpoint | Ratio |
|---|---:|---:|---:|---:|
| `#6f6f6f` | PASS | PASS | `#ffffff` both runs | `5.024865064757633:1` both runs |
| `#777777` | PASS | PASS | `#000000` both runs | `4.68949989000882:1` both runs |

For regression context, the independent oracle also re-measured the forbidden old pairs:

- `#6f6f6f` + `#f2e9dc` = `4.1788873680219405:1` — fails.
- `#777777` + `#241404` = `3.9848605473081395:1` — fails.

## Value-based determinism and no remap/reject

### Curated `#5fbfae`

All paths resolved identically to `#5fbfae` + `#241404` (`8.115110618605044:1`):

- free color input;
- quick-pick chip;
- free input again before save;
- persisted settings payload (`accent: "#5fbfae"`);
- boot after page reload.

### Non-curated `#777777`

All paths resolved identically to `#777777` + `#000000` (`4.68949989000882:1`):

- free color input;
- persisted settings payload (`accent: "#777777"`);
- boot after page reload.

Additional normalization probe: uppercase valid input `#E0869B` normalized to `#e0869b` and selected curated ink `#241404`.

Every frozen valid value remained exact in input, root inline/computed accent, preview accent, and persistence where exercised. No clamp, remap, or rejection observed.

## Scope and regression checks

- `git rev-parse HEAD`: exact target `f56e2ed04a2d744267fbd025afda999a34b46b78`.
- Target commit files only:
  - `src/main.ts`
  - `qa-reports/headless-smoke.mjs`
- `git diff --numstat ca3e86b..f56e2ed -- src-tauri`: empty.
- `git diff --numstat -- src-tauri`: empty.
- INV-21 scan across `src/main.ts`, `src/styles.css`, and `index.html`: zero forbidden Abyss-Emerald / old terminal-editor literal hits.
- Canonical smoke independently confirmed `U-D1 INV-21`, all frozen INV-23 vectors, no console errors, and zero CSP violations.

Evidence: `2026-11-01-inv23-adr20-rev2-fresh-evidence/scope-inv21.json`.

## Boundaries

Tested on Linux headless Chromium against the production build with mocked Tauri IPC persistence. The focused probe did not exercise a real Windows WebView2 color-picker popup or real settings filesystem I/O; Rust store/config behavior remained covered by the full 87-test library suite. No production source or backend file was changed by QA.

## Evidence index

Directory: `qa-reports/2026-11-01-inv23-adr20-rev2-fresh-evidence/`

- `cargo-test-lib.log`
- `npm-run-build.log`
- `playwright-install.log`
- `headless-smoke.log`
- `rendered-probe.log`
- `rendered-settings.json`
- `wcag-recompute.log`
- `wcag-ratios.json`
- `scope-inv21.log`
- `scope-inv21.json`
- `playwright-cleanup.log`
- `settings-rendered.png`
- `inv23-rendered-probe.mjs`
- `recompute_wcag.py`
- `check_scope_inv21.py`
- `node-check.log`
- `sha256sums.txt`
