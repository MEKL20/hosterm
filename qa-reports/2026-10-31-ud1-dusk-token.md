# QA — U-D1 Dusk token swap — re-verify + INV-21 grep

- **Unit:** U-D1 (Dusk token swap) — EPIC-TSD-redesign-dusk-v1.md §U-D1
- **Date:** 2026-10-31
- **QA:** crew-coding-qa (black-box, READ-ONLY — no fix, no commit)
- **Tree:** working tree, uncommitted (git policy; U-D1 + U-D3 co-resident)
- **Verdict: PASS** (0 blocker / 0 major / 2 minor / 1 note)

## Overall verdict

U-D1 PASS. All 7 AC verified against real output. INV-21 anti-Abyss grep = **0 hit** (acceptance kritis, confirmed). Verify loop re-run independently: cargo **87** / build **0 TS** / smoke **230/230** (2 consecutive runs). Scope clean — only the 3 in-scope files carry U-D1 changes; geometry/layout untouched (that is U-D2).

**Baseline drift note (not a defect):** task body expected cargo **78**. Live canary = **87** because U-D3 store backend (`lib.rs` + `snippets.default.json`, `store_tests::` 8 tests) is co-resident uncommitted in the same working tree. cargo 87 = correct live number; 78 is stale. Smoke 230 matches task expectation (225 base + 5 U-D1 group checks region reporting 7 asserts).

---

## Per-AC evidence

### AC1 — :root exposes all 16 Dusk tokens byte-exact §13.3 — **PASS**
Read `src/styles.css:1-36`. Every token matches SDD §13.3 table byte-exact:
`--bg #191512`, `--sidebar #1d1814`, `--surface #221c17`, `--raised #2a231c`, `--term #14100d`, `--border #332a21`, `--hairline rgba(226,163,92,.14)`, `--text #f2e9dc`, `--dim #b9aa95`, `--faint #93836f`, `--accent #e2a35c`, `--accent-ink #241404`, `--accent-soft rgba(226,163,92,.12)`, `--live #9dc489`, `--danger #e07856`, `--r-lg/md/sm 10/8/6px`, `--shadow-drawer 0 10px 30px rgba(0,0,0,.42)`, `--font-ui`/`--font-mono` stacks. `color-scheme: dark` preserved (line 2).

Smoke live getComputedStyle:
```
PASS U-D1: --bg == #191512 (Dusk) — #191512
PASS U-D1: --accent == #e2a35c (Dusk) — #e2a35c
PASS U-D1: all Dusk :root tokens byte-exact §13.3
```

### AC2 — INV-21 anti-Abyss grep = 0 hit (ACCEPTANCE KRITIS) — **PASS**
Independent grep re-run:
```
$ grep -rnE '#08080a|#050507|#00d992|#818cf8|#0e2a20|var\(--glow\)' src/
exit=1  (zero hits = PASS)
```
Broader `grep -rn -- '--glow' src/` → single hit is a **comment** in styles.css:40 ("Abyss --glow box-shadow removed") — descriptive text, not a live literal; does not match the smoke's `var(--glow)` token. Smoke gate confirms:
```
PASS U-D1 INV-21: zero Abyss literal in src/ (grep gate)
```

### AC3 — accent-fill penuh only on 3 active selectors — **PASS (1 nuance)**
Read CSS. Full accent treatment on the 3 active states as specified:
- `button.primary` (line 77) — full `background: var(--accent)` fill
- `.tab.active` (line 219) — `border-bottom-color: var(--accent)` underline (2px motif)
- `.term-side.focused` (line 252) — `border-top: 2px solid var(--accent)`

No hover/idle accent fill. Other `background: var(--accent)` occurrences are small indicator affordances, not component surface fills: `.t-dot` 8px dirty dot (228), `.xfer-bar.on::before` 2px progress bar (329), `#sb-resize:hover/.dragging` resize handle (119), `.term-divider:hover/.dragging` (260) — all transient/indicator, consistent with prototype semantics and the design comment (styles.css:38-41). **Nuance, not a defect.**

### AC4 — elevation only drawer+modal; no glow — **PASS (1 minor)**
- Old Abyss `--glow` box-shadows **removed** (diff shows 3 deleted lines: `.tab.active`, `.term-side.focused`, `button.primary` box-shadow → gone).
- `--shadow-drawer` used only on `.modal-box` (line 450). Smoke:
```
PASS U-D1: Abyss glow box-shadow fully removed
PASS U-D1: elevation (--shadow-drawer) only on modal/drawer box — .modal-box
```
- **MINOR-1:** `#tab-menu, #host-menu` (line 298) carry a hardcoded `box-shadow: 0 8px 24px rgb(0 0 0 / 0.5)` instead of the `--shadow-drawer` token. It is a popup-menu elevation (arguably drawer/modal-class) so not a spec violation, but it is a 5th box-shadow selector not using the canonical token — inconsistent with the "elevation via --shadow-drawer" intent. Cosmetic; smoke does not flag it (glow-check keys on the old Abyss rgba only). Suggest U-D2 fold it onto `--shadow-drawer`.

### AC5 — xterm/CM literals remapped to Dusk — **PASS**
Read `src/main.ts`:
- `TERMINAL_THEME` (1665-1691): `background #14100d` (--term), `foreground #f2e9dc`, `cursor #e2a35c`, `cursorAccent #14100d`, ANSI `green #9dc489` (live), `red #e07856` (danger), `yellow/brightYellow #e2a35c` (accent). No emerald hex.
- `cmHighlight` (63-72): keyword/function `#e2a35c`, string `#9dc489`, comment `#93836f`, text `#f2e9dc`, type/property `#8fc4b8` — Dusk values.
- `FONT_MONO` (22) = JetBrains Mono stack.
```
PASS U-D1: xterm/CM literal remapped to Dusk (no Abyss hex)
```

### AC6 — Verify loop re-run (REAL numbers) — **PASS**
```
$ cd src-tauri && cargo test --lib
test result: ok. 87 passed; 0 failed; 0 ignored; finished in 7.49s

$ npm run build
✓ built in 648ms   (0 TS errors)

$ npm install --no-save playwright@1.63.0 && node qa-reports/headless-smoke.mjs
230/230 passed   (run 1)
230/230 passed   (run 2)
```
Playwright cleaned via guard-safe `mv` to $TMPDIR/pw-trash + `unlink` .bin symlinks (NOT rm -rf). Confirmed 0 playwright remnants in node_modules; `git status` shows no test artifacts staged (only the 4 source files + snippets asset).

cargo 87 (not 78 — see baseline drift note). smoke 230 (225 base + U-D1 checks). build 0.

### AC7 — geometry/layout unchanged (U-D2 territory) — **PASS**
`git diff src/styles.css`: 131+/125- all token/color var remaps. Every width/height/px value in the diff is unchanged on both sides (`.h-dot` 6px→6px, `.skel` 28px→28px, radius tokens are the Dusk definitions). Removed lines = the 3 Abyss `--glow` box-shadows only. Old tokens `--accent2`/`--border-soft`/`--muted`/`--warn`/`--hover`/`--shadow-modal` correctly remapped to Dusk tokens. No size/position/flex/grid/padding/margin value changed.

---

## Issues

| # | Sev | Title | Repro | Expected | Actual | Evidence |
|---|---|---|---|---|---|---|
| MINOR-1 | minor | Context-menu box-shadow not on `--shadow-drawer` token | read styles.css:298 | elevation via `--shadow-drawer` | hardcoded `0 8px 24px rgb(0 0 0 / 0.5)` on `#tab-menu, #host-menu` | styles.css:298 |
| MINOR-2 | minor | `button.primary` text color uses `--bg` not `--accent-ink` | read styles.css:77 | §13.3 `--accent-ink #241404` is "text on accent" | `color: var(--bg)` (#191512) | styles.css:77 — contrast still high (#191512/#e2a35c ≈ 7.8:1 AA); cosmetic deviation from token role, not INV-23 (that's U-D4) |
| NOTE-1 | note | cargo baseline drift 78→87 | `cargo test --lib` | task body says 78 | 87 (U-D3 store co-resident uncommitted) | report §AC6; 78 is stale, 87 is correct live canary |

Neither minor blocks U-D1. Both are candidates to fold into U-D2 (shell re-skin, same hotspot files) — routed to nano, not fixed here.

## Scope confirmation
U-D1 touched exactly: `src/styles.css`, `src/main.ts`, `qa-reports/headless-smoke.mjs`. `lib.rs` + `snippets.default.json` changes belong to U-D3 (co-resident, not U-D1 scope). No geometry change (U-D2). No `lib.rs` touch by U-D1.
