# QA Report: U-D4 Settings panel + Appearance — 2026-11-01

| Field | Value |
|---|---|
| **Target** | `hosterm` U-D4, commit `ca3e86b25ed64a226fda262ada9c00e2e6f1d5f8` |
| **Baseline** | `876d32d9b0dd41f671dbf191f3ce7a5c37d55e9d` (`origin/main` U-D2 baseline) |
| **Date** | 2026-11-01 01:57 WIB |
| **Tester** | `crew-coding-qa`, task `t_6dbee3a1` (black-box/evidence-first) |
| **Working tree at start** | Clean; `git status --short` printed no entries |
| **Report status** | **OPEN — FAIL** |

## Verdict

**FAIL — 0 blocker / 1 major (High) / 0 medium / 0 low.**

The canonical loop is green (`cargo 87/87`, build exit 0, smoke `255/255`), and the exact five quick-pick colors plus the required dark free-color sample pass. U-D4 still fails acceptance-critical **INV-23**: valid mid-tone free colors can leave both fixed ink choices below `4.5:1`. The failure reproduced twice in the rendered app.

## Issue summary

| Severity | Count |
|---|---:|
| Critical / blocker | 0 |
| High / major | 1 |
| Medium | 0 |
| Low | 0 |
| **Total** | **1** |

## Verify loop — independent rerun

| Gate | Actual result | Evidence |
|---|---|---|
| `cd src-tauri && cargo test --lib` | **PASS**, exit `0`; `87 passed; 0 failed; 0 ignored`; baseline exactly matches `docs/STATE.md`. Two pre-existing `unused_mut` warnings. | `2026-11-01-ud4-settings-appearance-evidence/01-cargo-test.log` |
| `npm run build` | **PASS**, exit `0`; `tsc && vite build`; built in `594ms`; no TypeScript errors. One non-fatal Vite chunk-size warning. | `2026-11-01-ud4-settings-appearance-evidence/02-npm-build.log` |
| `node qa-reports/headless-smoke.mjs` | **PASS**, exit `0`; **255/255**. Canonical baseline `241`; delta **14** U-D4 checks (requirement `>=8`). | `2026-11-01-ud4-settings-appearance-evidence/04-headless-smoke.log`, `13-scope-and-smoke-delta.log` |
| Playwright cleanup | **PASS**; `node_modules/playwright` and `playwright-core` moved to `$TMPDIR`; both `.bin/playwright*` links unlinked; `0` links remain. | `2026-11-01-ud4-settings-appearance-evidence/12-playwright-final-cleanup.log` |
| Commit hygiene | **PASS**; `git diff --check` empty. Target diff contains only `index.html`, `src/main.ts`, `src/styles.css`, `qa-reports/headless-smoke.mjs`. | `2026-11-01-ud4-settings-appearance-evidence/06-git-scope.log` |

## Per-acceptance evidence

| # | Verdict | Evidence from real execution |
|---:|---|---|
| 1 | **PASS** | Gear opened an in-app `role="dialog"`, `aria-modal="true"`; scrim computed `rgba(12, 9, 6, 0.55)`; panel measured `900x620`; background app became inert. CSS source fixes `max-width:94vw`, `max-height:92vh`. No native dialog/window call occurred. Smoke log lines under `U-D4 FR-SET1`. |
| 2 | **PASS** | Rendered nav labels were exactly `General`, `Appearance`, `Terminal`, `SSH & Keys`, `AI Assistant`, `Snippets`, `Account`; current section was `appearance`. General rendered `Coming soon.` with Appearance content hidden; the section handler is shared by all non-Appearance items. Smoke `U-D4 FR-SET2/3`. |
| 3 | **PASS** | Four presets rendered; Dusk `aria-pressed=true` with visible check; three entries had `aria-disabled=true`. Clicking disabled Slate kept `data-theme` `dusk -> dusk`; the same generic `aria-disabled` guard handles Slate/Ink/Light. Smoke `FR-SET4` + `OQ-2`; markup in target diff. |
| 4 | **PASS** | Teal produced inline `--accent:#5fbfae`, `--accent-ink:#241404`, teal `aria-pressed=true`, preview `--pa:#5fbfae`, preview button `rgb(95,191,174)`. Stable values remained `--surface #221c17`, `--text #f2e9dc`, `--live #9dc489`, `--danger #e07856`. Smoke `FR-SET7/8/10`. |
| 5 | **FAIL** | Required nominal samples pass, but INV-23 does not hold for every valid custom accent. Full measured ratios below; BUG-1 reproduced twice. |
| 6 | **PASS** | Teal + Done added exactly one `write_settings` call and closed the modal. Payload was `{version:1,theme:"dusk",accent:"#5fbfae",allowlist:[...]}`. Reload retained teal. Scrim click and Escape both closed; Escape restored gear focus. Smoke `FR-SET12/13`. |
| 7 | **PASS** | Boot called `read_settings`; default boot applied Dusk/`#e2a35c`. After persisted teal and reload, a new read occurred and applied `theme=dusk`, `accent=#5fbfae`, teal pressed. Smoke `U-D4 boot` and `FR-SET13 boot`. |
| 8 | **PASS** | Independent `search_files` regex scan of `src/` for `#08080a|#050507|#00d992|#818cf8|#0e2a20|var(--glow)` returned **`total_count: 0`**. Canonical smoke also passed `U-D1 INV-21: zero Abyss literal in src/`. |
| 9 | **PASS** | `git diff --quiet 876d32d..ca3e86b -- src-tauri` returned `0`; target diff has no backend file. Added-line scan for `~/.ssh/config`, `hosterm.secrets`, `HOSTERM_CONFIG`, `config_path`, and `secrets_path` returned **0 hits**. Cargo passed `store_tests::write_settings_leaves_ssh_config_and_sidecar_untouched`; live backend resolves `dirs::config_dir()/hosterm/settings.json`. |

## INV-23 measured contrast

Canonical quick picks use ink `#241404`:

| Accent | Ink | Ratio | Result |
|---|---|---:|---|
| `#e2a35c` | `#241404` | `8.18060039728731` | PASS |
| `#5fbfae` | `#241404` | `8.115110618605044` | PASS |
| `#e0869b` | `#241404` | `6.842802495508517` | PASS |
| `#aec267` | `#241404` | `9.102683799795606` | PASS |
| `#b99be3` | `#241404` | `7.529220380096464` | PASS |
| free `#101010` | auto `#f2e9dc` | `15.824570367603279` | PASS |
| free `#777777` | auto `#241404` | **`3.9848605473081395`** | **FAIL** |
| free `#6f6f6f` | auto `#f2e9dc` | **`4.1788873680219405`** | **FAIL** |

Sources: canonical output `04-headless-smoke.log`; independent calculation `07-contrast-boundary-calc.log`; live UI repros `09-contrast-ui-repro-1.log` and `10-contrast-ui-repro-2.log`.

## BUG-1: Contrast guard does not guarantee 4.5:1 for valid mid-tone custom accents

- **Severity:** **High / major** — Accessibility + functional invariant; blocks U-D4 acceptance.
- **Status:** **VERIFIED**, reproduced twice.
- **Spec:** `docs/SDD.md §13.2 item 4`, `FR-SET9`, `INV-23`; custom accent ink must remain `>=4.5:1`.

**Repro:**

1. Build production UI: `npm run build`.
2. Open Settings → Appearance.
3. Set `#accent-input` to valid free color `#777777` and dispatch its normal `input` event.
4. Read computed inline `--accent` / `--accent-ink`; calculate WCAG relative-luminance contrast.
5. Repeat with `#6f6f6f`.

Automated repro:

```bash
npm install --no-save playwright@1.63.0
node qa-reports/.ud4-contrast-probe.mjs
```

The executed probe is archived as `2026-11-01-ud4-settings-appearance-evidence/11-contrast-probe.mjs`.

**Expected:** Every valid custom `#RRGGBB` accent gets an ink color with contrast `>=4.5:1`, or the accent is constrained/rejected so the invariant remains true.

**Actual:** `guardedAccentInk()` chooses whichever of `#241404` or `#f2e9dc` has the higher ratio, but does not handle the range where **both** are below `4.5:1`:

```text
#777777 + #241404 = 3.9848605473081395
#6f6f6f + #f2e9dc = 4.1788873680219405
255/256 passed; probe exit 1
```

**Evidence:**

- `2026-11-01-ud4-settings-appearance-evidence/09-contrast-ui-repro-1.log` — first rendered-app repro, exit `1`.
- `2026-11-01-ud4-settings-appearance-evidence/10-contrast-ui-repro-2.log` — identical second repro, exit `1`.
- `2026-11-01-ud4-settings-appearance-evidence/07-contrast-boundary-calc.log` — independent ratio calculation.
- `src/main.ts:281-284` — selection chooses the larger of two ratios but does not enforce the threshold.

## Scope and boundaries

**Tested:** U-D4 modal/geometry/scrim; seven-item nav and placeholder; presets and disabled guard; accent chips/free input/live preview; reset; Done/scrim/Escape; `read_settings`/`write_settings` mock IPC; persistence across reload; INV-21; INV-22; canonical cargo/build/smoke.

**Not tested:** native Windows WebView2 rendering or OS color-picker chrome; full Tauri frontend-to-real-filesystem integration (frontend smoke uses IPC mocks, while Rust store behavior is covered by cargo tests). No real `~/.ssh/config`, sidecar, or `authorized_keys` was touched.

**QA changes:** no production/test source modified. Only this report and its evidence directory were added; no commit or push.
