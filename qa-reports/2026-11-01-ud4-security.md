# U-D4 Security Scan — Settings + Appearance

- **Target:** `876d32d..ca3e86b` (`feat(ui): U-D4 settings panel and appearance`)
- **Mode:** WHITEBOX, read-only; no runtime probing
- **Scope:** `src/main.ts`, `index.html`, `src/styles.css`, `qa-reports/headless-smoke.mjs`
- **Verdict:** **FINDINGS — 1 MAJOR, 0 BLOCKER/MINOR/INFO**
- **Evidence basis:** commit objects, not the dirty working tree. Line numbers below refer to blobs at `ca3e86b`.

## Coverage ledger

| # | Result | Severity | Evidence |
|---|---|---|---|
| 1 | **CLEAN** — no network primitive and no IPC outside the frozen inventory | — | Added-line scan: `fetch`/XHR/WebSocket/EventSource/sendBeacon/remote CSS = **0**. The only production invokes added are `read_settings` at `src/main.ts:334` and `write_settings` at `src/main.ts:362`; both already exist and are registered in parent `876d32d` (`src-tauri/src/lib.rs:778,794,3196-3198`). Parent and target both contain 27 `#[tauri::command]` entries (`lib.rs` 25 + `fleet.rs` 2). |
| 2 | **CLEAN** — INV-22 path separation holds | — | Diff changes only the four scoped frontend/harness files; backend/config/sidecar files have zero diff. `src/main.ts:362-368` calls only `write_settings` with the frozen settings payload. Its existing resolver is `dirs::config_dir()` or `HOSTERM_DATA_DIR`, then `hosterm/settings.json` (`src-tauri/src/lib.rs:726-742`); `write_settings` writes only that path (`src-tauri/src/lib.rs:793-825`). Existing isolation test proves config and sidecar bytes+mtime unchanged and store lands outside SSH data (`src-tauri/src/lib.rs:2865-2941`). No `known_hosts` path is referenced by the U-D4 additions. |
| 3 | **CLEAN** — no dynamic evaluation/HTML sink; accent CSS value is syntactically gated | — | Added-line scan: `eval`, `new Function`, `Function`, and `innerHTML` = **0**. `validAccent()` requires exactly `^#[0-9a-fA-F]{6}$` (`src/main.ts:264-266`); `applyAccent()` returns before all four dynamic `style.setProperty` calls unless that check passes (`src/main.ts:296-307`). The displayed value uses `textContent` (`src/main.ts:304`). Thus the color input cannot inject CSS tokens, declarations, or markup. |
| 4 | **CLEAN** — INV-25 approval visual weight is untouched | — | Added-line scan for `approval-run`, `approval-always`, `approval-reject`, `cmd-card`, `btn-primary`, `btn-secondary`, and `btn-ghost` = **0**. The only AI addition is a non-functional Settings navigation label (`index.html:78`); no U-D7 approval card/action exists in this diff. The global primary button remains accent-filled; only its foreground changes to `var(--accent-ink)` (`src/styles.css:77`). |
| 5 | **CLEAN** — INV-21 anti-Abyss gate remains zero | — | U-D4 added lines contain **0** occurrences of `#08080a`, `#050507`, `#00d992`, `#818cf8`, `#0e2a20`, or `var(--glow)`. The complete target production sources (`src/main.ts`, `src/styles.css`, `index.html`) also contain 0 of each. |
| 6 | **FINDING UD4-SEC-01** — INV-23 contrast guard has a valid-color gap | **MAJOR** | See finding below. |
| 7 | **CLEAN** — no credential field or sensitive logging added | — | Added-line scan: credential/secret/password/token terms = **0** and `console.*` logging = **0**. The added schema is exactly `version`, `theme`, `accent`, `allowlist`; allowlist entries are `host`, `command`, `added_at` (`src/main.ts:21-33`). Persist sends only those fields (`src/main.ts:362-368`). The frozen allowlist command text is round-tripped, but no password/token/credential field or log sink is introduced. |
| 8 | **CLEAN** — no dependency change | — | `git diff --name-only 876d32d..ca3e86b` lists only the four scoped files. `package.json`, JS lockfiles, `src-tauri/Cargo.toml`, and `src-tauri/Cargo.lock` have zero diff. |

## Finding UD4-SEC-01 — valid middle-luminance accents produce unreadable primary text

- **Severity:** MAJOR
- **Status:** CONFIRMED (deterministic white-box arithmetic)
- **Confidence:** Medium (static-only review; exact reachable formula independently recomputed)
- **Invariant:** INV-23 / FR-SET9 / NFR-DK3
- **Locations:**
  - `src/main.ts:109-110` — endpoints are `#241404` and `#f2e9dc`.
  - `src/main.ts:268-284` — computes WCAG luminance/contrast, then selects whichever of those two endpoints scores higher.
  - `src/main.ts:296-307` — applies the selected ink to root and preview.
  - `src/main.ts:415-417` + `index.html:126` — every valid color-input value reaches that path.
  - `src/styles.css:77,632` — primary action and preview consume the derived ink over the chosen accent.
  - `qa-reports/headless-smoke.mjs:1718-1736` — free-color coverage tests only the easy dark sentinel `#101010`, not middle-luminance failure vectors.

### Proof

Both available ink endpoints fail WCAG AA for reachable valid `#RRGGBB` values. The code picks the larger failing ratio:

```text
accent #6f6f6f:
  contrast vs #241404 = 3.551251976104001
  contrast vs #f2e9dc = 4.178887368021940  <- selected, still < 4.5

accent #777777:
  contrast vs #241404 = 3.984860547308140  <- selected, still < 4.5
  contrast vs #f2e9dc = 3.724165964510075
```

Independent calculation used the SDD WCAG sRGB formula (`0.04045` cutoff). The implementation's `0.03928` cutoff does not change any branch for 8-bit `#RRGGBB` channels (0 differing channel values out of 256), so it does not invalidate the proof.

### Reachability and impact

`#6f6f6f` and `#777777` both pass `validAccent()`. Entering either through `input[type=color]` applies it without clamp/rejection, and `button.primary` immediately renders with contrast below 4.5:1. Operators can therefore create hard-to-read primary-action text, violating the no-unreadable-text guarantee and the unit's critical INV-23 acceptance criterion.

### Counterevidence considered

All five curated chips remain safe with `#241404` (independently recomputed ratios: 6.84–9.10), and the smoke sentinel `#101010` reaches 15.82 with `#f2e9dc`. Those endpoints do not cover arbitrary valid colors; the two valid middle-gray counterexamples close the candidate as confirmed. The implementation also branches on interaction source (`source === "chip"`, `src/main.ts:299`) instead of deriving solely from normalized value; current static chip values happen to be curated, but remediation should remove that source dependency.

### Required remediation (not applied in this read-only task)

Keep `#241404` only for the five normalized curated values. For every other valid accent, select `#000000` or `#ffffff` by unrounded contrast (equivalent switch `L >= 0.179128784747792` → black, otherwise white), then add full input/persist/reload regression vectors including `#6f6f6f`, `#777777`, both switch sides, and extremes.

## Commands and raw scan summary

```text
$ git diff --name-only 876d32d..ca3e86b
index.html
qa-reports/headless-smoke.mjs
src/main.ts
src/styles.css

$ git grep -c '#\[tauri::command\]' 876d32d -- 'src-tauri/src/*.rs'
fleet.rs:2  lib.rs:25
$ git grep -c '#\[tauri::command\]' ca3e86b -- 'src-tauri/src/*.rs'
fleet.rs:2  lib.rs:25

Added-line scan:
NETWORK=0  DYNAMIC_EVAL=0  INNERHTML=0  LOGGING=0
SECRET_TERMS=0  APPROVAL_CLASSES=0  ABYSS=0
INVOKES=[src/main.ts:334 read_settings, src/main.ts:362 write_settings]
```

## Boundary

No browser/app execution, filesystem mutation test, external traffic, dependency audit, or post-`ca3e86b` remediation review was performed. This review covers only the specified commit diff plus read-only verification of the pre-existing frozen IPC/store resolver it calls.
