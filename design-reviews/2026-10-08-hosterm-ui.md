# hosterm UI Audit: 2026-10-08

## Scope and boundary

Reviewed (code only): `src/main.ts` (595 lines), `src/styles.css` (158 lines), `index.html` (102 lines), `README.md` (90 lines). Method: static read plus 32 computed contrast pairs via `python3 ~/.hermes/skills/creative/antislop-human/contrast-check.py`. Every contrast claim below quotes the measured ratio.

Not reviewed:

- `docs/screenshot-*.png`: stale v0.1.0 renders, and this host has no vision model. Screenshots were not used as evidence.
- Rust backend (`src-tauri/`): out of scope; no UI surface.
- Runtime behavior: no build was run or clicked through. R-35 click-through evidence is a gate for the implementing agent, not this audit.
- No alternate themes exist to test (single dark theme, no toggle).

Positive note, for balance: the sidebar empty state already does the right thing (`main.ts:55`): it says why it is empty and names the next action ("No hosts in ~/.ssh/config yet. Click + to add one.").

Severity per antislop tiering: Hard Gate rule = high, Purpose-Gate/UX = med, consistency = low.

## Findings

### High severity

**H-1. Secondary text fails WCAG AA on every surface** (R-25)
`--muted: #565f89` (`styles.css:6`) measures **2.76:1** on `--bg`, **2.91:1** on `--bg2`, **2.51:1** on `--panel`. All fail the 4.5:1 normal-text minimum. It is the color of host subtitles (`styles.css:47`), inactive tab labels (`62`), input/field labels (`105`), notes (`153`), editor path (`135`), both status lines (`125`, `143`), and SFTP size column (`120`). Most of the app's informational text is below AA.
Fix: DESIGN.md S-2 replaces `--muted` with `#8b98c9` (5.51:1 to 6.37:1 on all three surfaces).

**H-2. Focus outline removed, nothing replaces it** (R-32)
`styles.css:91` and `:149`: `input:focus, textarea:focus` and `.modal-box select:focus` set `outline: none`, leaving only a border-color change to `--accent` (#7aa2f7 on #16161e = 7.14:1, passes 3:1, but only on two element types). Buttons have no focus style at all. Keyboard users lose position in every modal.
Fix: S-11 global `:focus-visible` ring spec.

**H-3. Modals cannot be closed with Escape; no focus management** (R-32, R-26)
`main.ts:580-595` wires every button but contains no `keydown` listener anywhere. None of the three modals (`#modal`, `#raw-modal`, `#key-modal`) closes on Escape, none traps focus, background stays interactive while a modal is open.
Fix: S-10 modal behavior spec (Escape closes, focus moves in on open and returns on close).

**H-4. Emoji doing icon work, unstyled and font-dependent** (R-04, R-25)
`📁 / 📄 / ↪` file-type glyphs (`main.ts:325`), `⬇` download (`327`), `⬆ Upload` (`258`), `↑` parent dir (`250`), `⇅` SFTP tab (`237`), `✎` editor tab (`375`), `⟳` reload (`index.html:20`, `main.ts:253`). Emoji render as color glyphs on some WebViews and monochrome text on others, ignore `currentColor`, and their built-in colors have no guaranteed 3:1 against the background. Tab kind is distinguishable only by these glyphs.
Fix: S-9 inline SVG icon set, 16px, `currentColor`, with named glyph per function.

**H-5. Em dash rendered in UI text** (R-02)
`main.ts:326`: directory rows show `—` in the size column. Also `→` in status copy: `uploaded → ${remote}` (`main.ts:294`), `saved → ${local}` (`335`).
Fix: copy brief (S-13): no em dash; directories show blank size cell; status copy uses words ("to" or plain path).

**H-6. Error, empty and loading states are un designed** (R-27)
Every failure path is raw `String(e)` dropped into a 12px status line: modal save (`main.ts:149`), key save (`173`), raw save (`205`), SFTP list (`349`), editor save (`411`). Error and success share one element, differ only by color (`.err` red vs `.ok` green), and carry no structure, icon, or next action. SFTP "loading …" is a text swap (`312`); terminal pane is blank until the first PTY bytes arrive (`459-523`); transfers show no progress at all.
Fix: S-12 state matrix and S-13 copy rules for all three states.

**H-7. No design direction existed for v0.x** (R-37)
No DESIGN.md in repo; palette was inherited ad hoc. Resolved by this delivery: DESIGN.md (same date) now holds direction, tokens, and dials; remaining owner decisions are isolated in its DIRECTION QUESTIONS section.

### Medium severity

**M-1. All component boundaries fail non-text contrast** (WCAG 1.4.11 via R-25)
`--border: #2a2e42` (`styles.css:9`) on `--bg` = **1.28:1**, on `--bg2` = **1.34:1**, on `--panel` = **1.74:1**. Every input, button, list row, tab separator, and modal edge is below the 3:1 boundary minimum.
Fix: S-2 `--border` to `#6270a2` (3.23:1 to 3.73:1).

**M-2. Loading is a text swap; transfers have no progress** (R-27)
SFTP list: `loading …` (`main.ts:312`). Upload/download: status line only, no progress bar, no cancel (`287-299`, `328-343`). Terminal: empty black pane while ssh spawns, no "connecting" signal.
Fix: S-12 (spinner pattern, indeterminate transfer bar, connecting state).

**M-3. Inactive tab labels below AA** (R-25)
`.tab` uses `--muted` at 13px (`styles.css:62`): **2.76:1** on `--bg2`. The dominant state of the tab bar fails.
Fix: S-6 tab spec (inactive text `#8b98c9` = 6.37:1 on `--bg2`).

**M-4. Hover-revealed actions are spans, invisible to keyboard** (R-32, R-26)
`.h-edit` and `.h-sftp` (`main.ts:66-68`, `styles.css:48-51`) are `opacity: 0` until row hover, and are `<span>`s inside the row's click handler, not focusable elements. Keyboard users cannot see or reach edit/SFTP at all; mouse users can trigger them by accident inside a row whose click opens a terminal.
Fix: S-5 row spec (always-styled focusable buttons, visible on `:focus-within`).

**M-5. No `:focus-visible` on buttons** (R-32)
No button focus style exists in `styles.css`; only inputs/selects were touched (and broken, H-2).
Fix: S-11.

**M-6. xterm theme sets 3 of 20+ values** (R-25)
`main.ts:465`: only `background`, `foreground`, `cursor`. The 16-color ANSI palette falls back to xterm defaults; default `brightBlack #555555` on `#1a1b26` measures **2.29:1**, so `ls` directory listings and any prompt using bright black are near-invisible. No selection color, so selection uses xterm's default blue-grey.
Fix: S-7 full 20-value theme, every pair measured.

**M-7. CodeMirror runs a light theme on a dark background** (R-25)
`basicSetup` (`main.ts:433`) ships no dark theme; `styles.css:139-142` overrides only backgrounds. Default token colors are tuned for white: keyword `#770088` = **1.74:1**, string `#aa1111` = **2.28:1** on `#1a1b26`. Code is barely legible.
Fix: S-7: `@uiw/codemirror-themes` is a new dependency, so instead spec explicit `EditorView.theme` + `HighlightStyle` overrides in `main.ts` using the Tokyo Night token values, zero new deps.

**M-8. Native `confirm()`/`alert()` for destructive and unsaved flows** (R-26, R-31)
Host delete uses `confirm()` (`main.ts:179`), dirty-editor close `confirm()` (`552`), raw-config load failure `alert()` (`197`). Native dialogs break the app's visual language and the dirty-close dialog offers only OK/Cancel: no "Save and close" path, so the safest flow costs the user their work or an extra round trip.
Fix: S-10 in-app confirm pattern with Save / Discard / Cancel on dirty close.

**M-9. Status meaning carried by color alone** (antislop-human: color-only feedback)
One status line per panel (`styles.css:125`, `143`); success/error distinguished only by `.ok` green vs default red. No icon or prefix. Fails for color-blind users.
Fix: S-13: status messages carry a text prefix (`OK`/`ERR` semantics via wording), status dot only as secondary signal.

**M-10. Tab bar has no overflow strategy** (C-4)
`#tabs` (`styles.css:59-67`) is a plain flex row, `white-space: nowrap`, no `overflow-x`, no min-width on `.tab`. Past ~6 tabs, labels clip with no scroll or affordance.
Fix: S-6: `overflow-x: auto`, thin scrollbar, `min-width` per tab, new-tab-drop behavior unchanged.

### Low severity

**L-1. Border radius is ad hoc** (R-11)
Radii in use: 4, 6, 7, 12px (`styles.css:38`, `44`, `100`, `121`, `127`), plus 4px on `.note code`. No scale; 7px on host rows vs 6px on buttons is arbitrary.
Fix: S-3 radius token set (4/6/10).

**L-2. Spacing is ad hoc** (R-31)
Paddings observed: 6/10, 8/10, 8/12, 8/14, 10/12, 12/14, 20 (`styles.css` throughout). No scale; rhythm comes from one-off values.
Fix: S-3 spacing scale (4/8/12/16/24).

**L-3. Hard-coded literals bypass the token system** (R-31)
`#1a1b26` as button.primary text (`styles.css:84`) and `.term-host` background (`71`); `#9ece6a` green literal twice (`126`, `144`) with a dead `var(--green, ...)` fallback pattern proving the token was meant to exist.
Fix: S-2: every value tokenized, including `--ok` and `--warn`.

**L-4. Mono font stack duplicated four times** (R-31)
`styles.css:109`, `135`, `158`, and `main.ts:463` repeat `ui-monospace, "Cascadia Code", Menlo, monospace`.
Fix: S-4 single `--font-mono` token consumed by CSS; xterm value spec'd to the same family.

**L-5. Zero motion, undeclared** (R-19)
No `transition` anywhere in `styles.css`. Hover, modal open (display toggle), tab switch: all instant snaps. For a terminal tool, MOTION 1 is defensible, but it must be declared and then held.
Fix: S-3 motion dials (MOTION 1) plus a 120ms transition on hover/press colors only.

**L-6. Icon-only buttons rely on native `title` tooltips** (R-04, R-31)
`#btn-reload`, `#btn-add`, `.sftp-up`, `.sftp-refresh` expose function only via `title` (`index.html:20/28`, `main.ts:250/253`). No `aria-label`, no visible label, discoverability depends on hover patience.
Fix: S-9: `aria-label` mandatory on icon-only buttons; tooltips via `title` kept as secondary.

**L-7. Dirty-state marker is an easy-to-miss `•`** (R-31)
`editorLabel` (`main.ts:358`) appends `" •"` to the tab text. No color, no title, same color as the label text; users can close a dirty tab and only then meet the native confirm (M-8).
Fix: S-6: dirty dot as a colored element (`--warn`, 8.55:1) with `title="Unsaved changes"`.

**L-8. Em dash also used as "no size" for directories** (R-02)
Counted under H-5 for the fix; listed here so the SFTP row spec is explicit: the size cell for directories must be empty, not `—` (`main.ts:326`).

## Summary counts

24 findings: 7 high, 10 medium, 7 low. Every high-severity item violates a Hard Gate rule (R-02, R-04, R-25, R-26, R-27, R-32, R-37). DESIGN.md sections S-1 to S-14 resolve each one; the mapping is named per finding.
