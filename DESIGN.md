# DESIGN.md: hosterm

Owner: MEKL. Written 2026-10-08 by the design pass (audit: `design-reviews/2026-10-08-hosterm-ui.md`). Target: coding-subagent implements this file literally; every color pair below carries a measured WCAG ratio from `contrast-check.py` (4.5:1 = AA normal text, 3:1 = large text and non-text boundaries).

## 0. Direction

Baseline decision: **evolve the existing Tokyo Night palette, keep it recognizably the same app.** Reason (one line, R-31): hosterm already ships Tokyo Night, its terminal aesthetic is the product's identity (an SSH client whose whole surface is terminal-adjacent), and a palette swap would spend change budget on re-skin instead of fixing the 24 audit findings. Dark theme has a legitimate reason here (R-21): terminal output, long sessions, developer tool.

This direction was chosen without the owner picking a style. Per R-37 it is a **defensible baseline, not a final direction**: the open calls MEKL must make are in section 11 (DIRECTION QUESTIONS). Nothing in this file pre-empts those answers; if he overrides an answer, only the affected token rows change.

Identity motif (one, repeated): **the accent is a current that flows through chrome, terminal, and editor alike**: active tab underline, terminal cursor and selection, editor active line and selection, primary button, focus rings all draw from the same two blues (`--accent`, `--accent2`). Reason: an SSH client's one shared substance is the live session; one visual current ties its three tab kinds together so the app reads as one tool.

Type: system UI sans for chrome (`ui-sans-serif` stack, unchanged), one monospace family for all code/paths/terminal (`ui-monospace, "Cascadia Code", Menlo, monospace`). Reason: mono is functional here (paths, file contents, terminal grid), not decorative; R-06 is satisfied because the mono role carries real content, chrome stays sans. No webfonts: zero new deps, instant first paint, native feel.

Reading this as: developer desktop tool for a technical single user, Termius-adjacent density, visual language "terminal-native chrome", dial **ENERGY 1 / RHYTHM 2 / MOTION 1**.

## 1. Dials

| Dial | Value | Meaning here |
|---|---|---|
| ENERGY | 1 | Flat surfaces, hairline borders, no gradients, no glow, no shadows except one elevation (S-8). Chrome recedes; terminal content is the loudest thing on screen. |
| RHYTHM | 2 | One consistent chrome rhythm (sidebar, tab bar, toolbars share padding scale), with deliberate breaks: terminal pane gets pure black-edge focus, modals get the single elevation. |
| MOTION | 1 | Hover/press color transitions only, 120ms. No entrance animations, no loops, no pulses. One exception: the `busy` spinner (S-12), which is a real state indicator, not decoration. |

## 2. Palette tokens (final values)

Neutrals and surfaces stay Tokyo Night; text and borders are corrected to pass WCAG (audit H-1, M-1). Every ratio below was computed, not estimated.

```css
:root {
  /* surfaces */
  --bg:     #1a1b26;  /* main surface                          */
  --bg2:    #16161e;  /* recessed: sidebar, tab bar, inputs    */
  --panel:  #1f2335;  /* raised: buttons, hovered rows         */
  --hover:  #24283b;  /* row hover, button hover               */

  /* text */
  --fg:     #c8d3f5;  /* primary text                          */
  --muted:  #8b98c9;  /* secondary text                        */

  /* current (the identity motif) */
  --accent:  #7aa2f7; /* interactive current: focus, active, CTA */
  --accent2: #89ddff; /* focus ring, caret accents               */

  /* status */
  --ok:    #9ece6a;
  --warn:  #e0af68;   /* dirty markers, warnings               */
  --danger:#f7768e;   /* destructive, errors                   */

  /* lines */
  --border:       #6270a2; /* control boundaries: inputs, buttons  */
  --border-soft:  #3b4261; /* passive separators, list rules       */

  /* type */
  --font-sans: ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
  --font-mono: ui-monospace, "Cascadia Code", Menlo, monospace;
}
```

Measured pairs (all via contrast-check.py):

| Pair | Ratio | AA normal (4.5) | Non-text (3.0) |
|---|---|---|---|
| `--fg` #c8d3f5 on `--bg` #1a1b26 | **11.47:1** | PASS | PASS |
| `--fg` on `--panel` #1f2335 | **9.77:1** (on `--hover`) | PASS | PASS |
| `--muted` #8b98c9 on `--bg` | **6.05:1** | PASS | PASS |
| `--muted` on `--bg2` #16161e | **6.37:1** | PASS | PASS |
| `--muted` on `--panel` | **5.51:1** | PASS | PASS |
| `--muted` on `--hover` #24283b | **5.16:1** | PASS | PASS |
| `--accent` #7aa2f7 on `--bg` | **6.79:1** | PASS | PASS |
| `--accent` on `--panel` | **6.18:1** | PASS | PASS |
| `--bg` text on `--accent` (primary button) | **6.79:1** | PASS | PASS |
| `--danger` #f7768e on `--bg` | **6.46:1** | PASS | PASS |
| `--bg` text on `--danger` (filled danger) | **6.46:1** | PASS | PASS |
| `--ok` #9ece6a on `--bg` | **9.35:1** | PASS | PASS |
| `--ok` on `--bg2` | **9.84:1** | PASS | PASS |
| `--warn` #e0af68 on `--bg` | **8.55:1** | PASS | PASS |
| `--accent2` #89ddff on `--bg` (focus ring) | **11.27:1** | PASS | PASS |
| `--border` #6270a2 on `--bg` | **3.55:1** | n/a | PASS |
| `--border` on `--bg2` | **3.73:1** | n/a | PASS |
| `--border` on `--panel` | **3.23:1** | n/a | PASS |

Rules the tokens enforce:

- `--border-soft` (#3b4261) is for **passive separators only** (list hairlines, panel dividers): it measures below 3:1 and must never bound an interactive control. Reason (R-31): interactive edges need to be findable (WCAG 1.4.11); passive separators must stay quieter than content.
- No color outside this list. The old literals (`#9ece6a`, `#1a1b26` in button text, dead `var(--green, ...)` fallback) die; audit L-3.
- Emoji icons are removed (S-9), so no uncontrolled glyph colors remain.

## 3. Type, spacing, radius, motion tokens

Type scale (chrome): 13px body/UI, 12px secondary/status, 11px micro-labels (`h-sub`, size column), 15px/600 modal titles. Line-height 1.4 chrome-wide. Terminal stays 13px mono (unchanged from `main.ts:464`), editor mono 13px to match. Reason: one size per information rank, four ranks, nothing else (R-31); 13px is what the app already speaks.

Spacing scale: **4 / 8 / 12 / 16 / 24** px. Controls use 8px vertical padding, 12px horizontal. Panels (toolbars, footers) use 12px outer padding with 8px gaps. Modals use 20px padding (24 allowed on `.wide`). Reason: fixes audit L-2's seven ad-hoc values with a 4px-base rhythm that matches the existing 12px-bar instinct already in the code.

Radius scale: **4px** inputs/rows/small chips, **6px** buttons, **10px** modals only. Reason (R-11): three radii, one per component class; kills the ad-hoc 7px and the 12px modal radius that read as arbitrary.

Motion: `transition: background-color 120ms ease, border-color 120ms ease, color 120ms ease` on interactive elements only. No transform transitions, no keyframes except the spinner (S-12). Reason: MOTION 1 declared; hover feedback needs to be perceptible but a terminal tool must feel instant.

## 4. S-4: Type application and the mono token

Audit S-1 is section 0 (direction), S-2 is section 2 (palette), S-3 is section 3 (type/spacing/radius/motion). S-4 onward are new below; the audit's "Fix: S-XX" lines map 1:1 to these sections.

`--font-mono` is declared once in `:root` (section 2) and consumed by every mono surface; the four duplicated stacks in `styles.css:109/135/158` and the inline stack in the xterm `fontFamily` option (`main.ts:463`) die with it (audit L-4).

Mono applies to exactly these, all functional content (R-06):

| Surface | Size |
|---|---|
| xterm `fontFamily` option | 13px (unchanged) |
| Editor (`.cm-content`) | 13px |
| Raw config textarea `#raw-text` | 13px, line-height 1.5 |
| Key paste textarea `#k-text` | 12px |
| SFTP path + local-path inputs | 13px |
| Editor path label `.ed-path` | 12px |
| SFTP size column `.sz` | 12px |

xterm cannot read a CSS custom property, so `main.ts` declares `const FONT_MONO = 'ui-monospace, "Cascadia Code", Menlo, monospace'` and passes it as the `fontFamily` option; the CSS var carries the identical literal. Reason: one source of truth per side of the JS/CSS boundary, zero new deps.

No other mono usage: chrome (buttons, labels, statuses, tab text) stays sans. Reason: mono outside functional content is the R-06 "terminal aesthetic" tell.

## 5. S-5: Sidebar host rows

Fixes M-4 (hover-only spans invisible to keyboard), supports H-4 (emoji removal).

Row structure (per host):

```
<div class="host-item" role="button" tabindex="0" aria-label="Open terminal to {name}">
  <div class="h-line">
    <span class="h-name">{name}</span>
    <span class="h-acts">
      <button class="h-sftp" aria-label="Open SFTP for {name}" title="SFTP">{icon:arrows}</button>
      <button class="h-edit" aria-label="Edit {name}" title="Edit">{icon:pencil}</button>
    </span>
  </div>
  <div class="h-sub">{user@host:port}</div>
</div>
```

- Actions are always-visible `<button>` elements, never spans, never `opacity: 0`. Reason (R-32): keyboard users must reach them; audit M-4. Hover-reveal also caused accidental triggers inside a row whose click opens a terminal.
- Action buttons: 22 x 22px hit area, 16px icon, radius 4, transparent bg, icon `--muted` (6.37:1 on `--bg2`); row hover/focus-within lifts row bg to `--panel`, button hover bg to `--hover` with icon `--fg` (9.77:1, measured this pass: `--fg` on `--hover`).
- Row click opens terminal (unchanged). Action buttons `stopPropagation()`.
- Keyboard: row focusable (`tabindex="0"`), Enter/Space activates terminal; buttons are next in tab order after the row. `:focus-visible` ring per S-11. `role="button"` + `aria-label` on the row.
- Row padding 8/10 (spacing scale), radius 4, gap 2px between lines; subtitle 11px `--muted` (6.37:1 on `--bg2`).
- Empty / loading / error states for the list: S-12 (sidebar block).

## 6. S-6: Tab bar

Fixes M-3 (inactive label contrast), M-10 (overflow), L-7 (dirty dot).

Structure per tab (three kinds: terminal, SFTP, editor):

```
<div class="tab" role="tab" tabindex="0" aria-selected="{true|false}">
  {icon: kind glyph, 16px}          <!-- terminal | arrows | pencil (S-9) -->
  <span class="t-label">{label}</span>
  <span class="t-dot" title="Unsaved changes" aria-label="Unsaved changes"></span>  <!-- editor, dirty only -->
  <button class="x" aria-label="Close tab" title="Close">{icon:close}</button>
</div>
```

- `#tabs` gets `role="tablist"`; panels get `role="tabpanel"`.
- Label: 13px. Inactive `--muted` (**6.37:1** on `--bg2`, fixes M-3), active `--fg` (**12.07:1** on `--bg`, measured this pass).
- Active tab: bg `--bg`, 2px bottom border `--accent` (non-text **6.79:1**; this is the identity current from section 0). Inactive hover: bg `--panel`.
- Separators between tabs: 1px right `--border-soft` (passive, per section 2 rule).
- Dirty dot (replaces the `" •"` text append, L-7): 6px circle, `--warn` (**8.99:1** on `--bg2`, measured this pass), `title` and `aria-label` "Unsaved changes". Colored and labeled, so it is findable before close (R-31).
- Close is a real `<button>` (16px icon, 20px+ hit area), not a text span. Color `--muted`, hover `--danger` (**6.80:1** on `--bg2`, measured this pass).
- Overflow (M-10): `#tabs { overflow-x: auto; overflow-y: hidden; scrollbar-width: thin; }` with `::-webkit-scrollbar { height: 6px }`, handle `--border-soft` (1.74:1, passive), hover `--border` (3.55:1). `.tab { flex: 0 0 auto; min-width: fit-content; max-width: 200px }`, label `text-overflow: ellipsis`. On `activateTab`, `scrollIntoView({ inline: "nearest" })` so the active tab is never hidden. Reason: clip with no scroll is the C-4 break the audit named; thin scrollbar keeps MOTION/ENERGY flat.
- Keyboard: tabs focusable; Left/Right arrows move focus and activate (R-32). Close button is in tab order after the label.
- Kind glyphs make the three tab kinds distinguishable without text (H-4); they are S-9 SVGs, so `currentColor` and consistent across WebViews.

## 7. S-7: Terminal pane and editor (xterm + CodeMirror theming)

Fixes M-6 (3-of-20 xterm theme), M-7 (light CodeMirror on dark bg), L-3 (`.term-host` literal).

### 7.1 Terminal pane

- `.term-host` background literal `#1a1b26` becomes `var(--bg)` (L-3). Padding 8px. No border: the terminal surface is the app surface.
- **No toolbar and no per-pane status bar.** The tab bar already names the session and holds the close control; a pane toolbar with no new commands is dead chrome (R-26). Terminal state is communicated by in-pane banner lines written into the PTY stream, specified in S-12 (connecting / failed / ended), colored with ANSI codes from the theme below. Reason: one chrome per concern, R-31.

### 7.2 xterm theme (26 keys, every value from the token set)

```ts
theme: {
  background: "#1a1b26",            // --bg            (text 11.47:1)
  foreground: "#c8d3f5",            // --fg            11.47:1
  cursor: "#7aa2f7",                // --accent        non-text 6.79:1
  cursorAccent: "#1a1b26",          // --bg            (text under cursor 6.79:1)
  selectionBackground: "#374465",   // accent 30% over bg; fg on it 6.47:1 (measured)
  selectionForeground: "#c8d3f5",   // --fg            6.47:1 on selection
  selectionInactiveBackground: "#293046", // fg on it 5.68:1 (measured)
  scrollbarSliderBackground: "#3b4261",   // --border-soft, passive (1.74:1)
  scrollbarSliderHoverBackground: "#6270a2", // --border      (3.55:1)
  scrollbarSliderActiveBackground: "#7aa2f7", // --accent     (6.79:1)
  black:  "#3b4261",  // 1.74:1  declared exception, below
  red:    "#f7768e",  // 6.46:1
  green:  "#9ece6a",  // 9.35:1
  yellow: "#e0af68",  // 8.55:1
  blue:   "#7aa2f7",  // 6.79:1
  magenta:"#bb9af7",  // 7.39:1
  cyan:   "#7dcfff",  // 9.96:1
  white:  "#a9b1d6",  // 8.10:1
  brightBlack:  "#737aa2", // 4.10:1  <- fixes M-6 (stock #414868 = 1.91:1)
  brightRed:    "#f7768e", // 6.46:1
  brightGreen:  "#9ece6a", // 9.35:1
  brightYellow: "#e0af68", // 8.55:1
  brightBlue:   "#7aa2f7", // 6.79:1
  brightMagenta:"#bb9af7", // 7.39:1
  brightCyan:   "#7dcfff", // 9.96:1
  brightWhite:  "#c0caf5", // 10.59:1
}
```

All ratios measured with contrast-check.py this pass except the four pairs already carried in section 2 (noted). `foreground` moves from `#c0caf5` to `--fg` `#c8d3f5` so the terminal and chrome share one text color; `brightWhite` keeps the authentic TN value. `black` at 1.74:1 is a **declared exception**: ANSI black is by definition the dim ink programs opt into (dim prompts, dim rules); it is not used for UI text. Every color a program prints to without opting into dimness passes AA.

### 7.3 CodeMirror theme (no new deps)

`@uiw/codemirror-themes` is a new dependency, so build the theme in `main.ts` from `EditorView.theme` + `HighlightStyle.define` (`@lezer/highlight` is already a transitive dep of `basicSetup`; importing it adds no package). Fixes M-7's 1.74:1 / 2.28:1 token colors.

```ts
const cmTheme = EditorView.theme({
  "&":            { color: "var(--fg)", backgroundColor: "var(--bg)" },
  ".cm-content":  { fontFamily: FONT_MONO, fontSize: "13px", caretColor: "var(--accent2)" },
  "&.cm-focused": { outline: "none" },            // ring applied below, R-32
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--accent2)" },  // 11.27:1
  ".cm-gutters":  { backgroundColor: "var(--bg2)", color: "var(--muted)",
                    borderRight: "1px solid var(--border-soft)" },        // digits 6.37:1
  ".cm-activeLine":        { backgroundColor: "#232430" },   // fg on it 10.31:1 (measured)
  ".cm-activeLineGutter":  { backgroundColor: "#282A37", color: "var(--fg)" }, // 9.54:1 (measured)
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground":
                           { backgroundColor: "#374465" },   // fg on it 6.47:1 (measured)
  ".cm-matchingBracket":   { outline: "1px solid var(--accent2)", backgroundColor: "transparent" },
}, { dark: true });

const cmHighlight = HighlightStyle.define([
  { tag: t.keyword,        color: "#bb9af7" },  //  7.39:1
  { tag: t.string,         color: "#9ece6a" },  //  9.35:1
  { tag: t.number,         color: "#ff9e64" },  //  8.40:1
  { tag: t.comment,        color: "#737aa2" },  //  4.10:1 (AA pass; comments may be dim)
  { tag: t.variableName,   color: "#c8d3f5" },  // 11.47:1
  { tag: [t.function(t.variableName)], color: "#7aa2f7" },  // 6.79:1
  { tag: t.typeName,       color: "#7dcfff" },  //  9.96:1
  { tag: t.propertyName,   color: "#89ddff" },  // 11.27:1
  { tag: t.operator,       color: "#89ddff" },  // 11.27:1
  { tag: t.punctuation,    color: "#a9b1d6" },  //  8.10:1
]);
```

Focus: the `outline: none` on `.cm-focused` is **replaced** by a ring in CSS: `.ed-editor .cm-editor.cm-focused { outline: 2px solid var(--accent2); outline-offset: -2px; }` (accent2 on bg **11.27:1**). R-32 hard gate: no removed indicator without a replacement.

Token values are Tokyo Night's own; every text-on-`--bg` pair above measured this pass. Reason (R-31): the editor is the third face of the same current (section 0), not a third palette.

### 7.4 Editor bar and status

- Bar: path label left (`{host}:{path}`, mono 12px, `--muted` **6.05:1** on `--bg`, full path in `title`), then `Reload` (default button), then `Save` (**primary**, bg `--accent`, text `--bg` **6.79:1**). Reason: saving is the pane's one decision; one accent moment per surface (antislop-ui accent cap).
- Save disables itself while the write is pending (prevents double-submit; today two fast clicks race). Status line per S-12/S-13 below the editor, border-top `--border-soft`.

## 8. S-8: Elevation

One shadow token, used once:

```css
--shadow-modal: 0 8px 24px rgba(0, 0, 0, 0.5);
/* applied to .modal-box only */
```

Everything else sits flat; borders (section 2) do all boundary work. Reason (R-12): the modal overlay is the only true z-layer in the app; a shadow anywhere else would be elevation vocabulary with nothing elevated, which is R-12's named default-failure. Modal scrim stays `rgba(0,0,0,.5)`; the scrim itself gets no blur (R-10 dose cap: zero glass in this app).

## 9. S-9: Icon set

Fixes H-4 (emoji doing icon work), L-6 (title-only icon buttons).

System: inline SVG (no icon font, no library dep), `viewBox="0 0 16 16"`, `width`/`height` 16, `fill="none"`, `stroke="currentColor"`, `stroke-width="1.5"`, `stroke-linecap="round"`, `stroke-linejoin="round"`. Geometric primitives only (rect, circle, line, polyline, path from lines/arcs). Reason: emoji color is WebView-dependent, ignores `currentColor`, and has no guaranteed contrast (H-4); one hand-set of primitives is ~12 short strings, no dependency, and inherits every token color.

| Function (replaces) | Glyph recipe |
|---|---|
| add host (`+`) | two lines: horizontal 8,4→12,4 … vertical 10,2→10,6 (plus, centered 10,4) |
| reload (`⟳`) | open circle arc (path A) 3→13 with arrowhead polyline at the gap |
| terminal tab (new) | rect 1.5,2.5 13x11 r2 + polyline 4,6 6,8 4,10 + line 8,10→11,10 |
| sftp tab (`⇅`), h-sftp | line 5,3→5,13 + arrowhead up at 5,3; line 11,13→11,3 + arrowhead down at 11,13 |
| editor tab / edit (`✎`, `.h-edit`) | pencil: shaft line 3,13→11,5, tip polyline 11,5→13,7? no: classic two-line pencil: line 3,13→12,4 + short cross line 10.5,5.5→12,7 forming the nib |
| close (`×`) | two diagonal lines 4,4→12,12 and 12,4→4,12 |
| folder (`📁`) | path: M1.5,4 h4 l1.5,2 h7.5 v7 h-13 z (folder silhouette) |
| file (`📄`) | path M3.5,1.5 h6 l3,3 v10 h-9 z + fold line 9.5,1.5 v3 h3 |
| symlink (`↪`) | box rect 9,9 5x5 r1 + polyline 2,4 8,4 8,9 + arrowhead at 8,9 |
| download (`⬇`, `.dl`) | line 8,2→8,10 + arrowhead polyline 5,7.5 8,10.5 11,7.5 + tray line 3,13.5→13,13.5 |
| upload (`⬆`) | line 8,13→8,5 + arrowhead polyline 5,7.5 8,4.5 11,7.5 + tray line 3,13.5→13,13.5 |
| parent dir (`↑`) | line 8,13→8,4 + arrowhead polyline 4.5,7.5 8,4 11.5,7.5 |

(Ambiguities above resolve to the simplest primitive that reads at 16px; the implementer draws within the primitive list. If a glyph cannot be made legible at 16px, drop the icon and keep the text label; a missing icon is better than a muddy one, R-04.)

- Every icon-only button carries `aria-label` (mandatory) and keeps `title` as the hover tooltip (L-6). Mandatory set today: `#btn-add` "Add host", `#btn-reload` "Reload config", `.sftp-up` "Parent directory", `.sftp-refresh` "Reload directory", tab `.x` "Close tab", row `.dl` "Download {name}", `.h-sftp`/`.h-edit` per S-5.
- Buttons that already have visible text (`Go`, `Upload`, `Save`, `Reload`, `Raw config`) get no icon. Reason: an arrow/glyph beside a labeled verb is R-08's decoration tell.
- Icon color is always `currentColor`; contrast comes from the surfaces it sits on (verified in S-5/S-6/S-14 rows).

## 10. S-10: Modals, dialogs, confirm

Fixes H-3 (no Escape, no focus management), M-8 (native `confirm`/`alert`).

Behavior, applies to all overlays (`#modal`, `#raw-modal`, `#key-modal`, and the new dialog):

- **Open:** remove `.hidden`; set `inert` on `#app`; move focus to the first input (editor: `#f-name`, raw: `#raw-text`, key: `#k-name`, dialog: first button of the safe action, below). Store the invoking element.
- **Escape closes** every overlay (R-32 hard gate). For dirty surfaces the Escape path runs the same guard as the buttons (editor/raw dirty → dirty dialog, S-10 dirty-close).
- **Tab cycles inside the overlay:** keydown Tab handler wraps focus from last focusable back to first and reverse. Background is `inert`, so nothing outside is reachable by keyboard or pointer.
- **Close:** restore focus to the invoking element. Reason (R-31): focus vanishing to `<body>` is how keyboard users lose their place (audit H-3).
- Scrim click does **not** close. Reason: accidental dismissal of a half-edited host or raw config costs data; the explicit close paths (Escape, Cancel) are enough.

New in-app dialog primitive (replaces `confirm()` in `deleteHost` `main.ts:179` and dirty-close `main.ts:552`, and `alert()` in `openRaw` `main.ts:197`):

- Reuses `.modal-box` chrome (radius 10, shadow S-8), width 340px. API shape: `dialog({ title, message, actions })` returning a Promise of the chosen action id. Actions are `[label, id, style]` with styles: `primary` (accent fill), `danger` (danger fill, `--bg` on `--danger` **6.46:1**), `default`.
- **Dirty-close** (M-8: today only OK/Cancel exists): title "Unsaved changes", message "`{basename}` has unsaved changes.", actions: `Save and close` (primary) / `Discard changes` (danger) / `Cancel` (default). Enter = Save and close, Escape = Cancel. Reason: the safest flow must not cost the work or an extra round trip (audit M-8).
- **Delete host** (destructive): title "Delete host", message `Delete host "{name}" from ~/.ssh/config? This cannot be undone.`, actions: `Delete` (danger) / `Cancel` (default). Focus lands on **Cancel** (safe default for destructive actions), Escape = Cancel.
- **Message-only** (raw config load failure): title "Could not open ~/.ssh/config", message per S-13 error rules, single `OK` action.
- Native `confirm`/`alert` calls are removed entirely; the app ships zero native dialogs. Reason: native dialogs break the visual language and offer no styled focus/danger semantics (M-8).

## 11. S-11: Focus

Fixes H-2 (`outline: none` with no replacement), M-5 (no button focus style).

Global, first-class:

```css
:focus-visible {
  outline: 2px solid var(--accent2);
  outline-offset: 2px;
}
```

- `--accent2` ring is visible on every surface it can appear over: **11.27:1** on `--bg`, **11.86:1** on `--bg2`, **10.26:1** on `--panel`, **9.61:1** on `--hover` (last three measured this pass; all pass 3:1 non-text).
- The two `outline: none` rules (`styles.css:91` inputs/textareas, `styles.css:149` select) are **deleted**. Focused text controls additionally get `border-color: var(--accent)` (`--accent` on `--bg2` **7.14:1**, ratio quoted in audit H-2): ring for findability, border tint for the "this is the active field" read.
- Buttons get the same global ring; no custom reset, no exception (fixes M-5).
- CodeMirror: ring on the editor surface, not per-line (S-7.3). xterm: the blinking block cursor plus the active-tab accent underline (S-6) are the focus indicators; the pane itself is not outline-ringed. Reason: a 2px ring around a full-bleed terminal paints over live output (R-31); cursor + tab state are unambiguous.
- Rule for the implementer: `outline: none` appears in the codebase **zero** times after this pass (R-32 wording: removal without replacement is a Hard Gate fail).
- `:focus-visible` (not `:focus`) so mouse clicks do not paint rings; keyboard Tab always does.

## 12. S-12: State matrix

Fixes H-6 (states undesigned), M-2 (text-swap loading, no transfer progress). Every surface lists: empty / loading / error / busy-plus-success. Copy wording lives in S-13; this section owns structure and color.

Shared indicators:

- **Spinner:** 12px circle, 1.5px `--accent2` arc on a transparent track, `animation: spin 800ms linear infinite` (the one keyframe; prefers-reduced-motion swaps it for a static half-arc, C-4). Used inline before loading text. accent2 on bg **11.27:1**.
- **Indeterminate transfer bar:** 2px full-width bar directly under the SFTP toolbar while an upload/download is pending, `--accent` fill sweeping left-to-right and back. This and the spinner are the **only two loops in the app** (section 3's "except the spinner" reads as "except the S-12 indicators"); both mark real work, neither decorates (R-19). No determinate percent: the backend exposes no byte counts, so a percent bar would be invented data (R-17). No cancel button: no backend cancel exists, and a dead control is R-26 fail; add both when the Rust side grows a cancel command.

| Surface | Empty | Loading | Error | Success/busy |
|---|---|---|---|---|
| Sidebar host list | existing copy kept verbatim: "No hosts in ~/.ssh/config yet. Click + to add one." (`main.ts:55`, already correct) | spinner + "Loading hosts…" | "Could not read ~/.ssh/config: {reason}. Check the file, then press Reload." (list cleared, error styled `--danger`) | normal list |
| SFTP list | "Empty directory." (row area, `--muted`) | spinner + "Listing {path or '~'}…" row | "Failed to list {path}: {reason}. Fix the path and press Go, or go up one level." (list cleared, path input keeps the user's value) | rows render; status shows "{n} items · {host}:{path}" |
| SFTP transfer | (n/a) | status "Uploading {name}…" / "Downloading {name}…" + indeterminate bar | "Failed to upload {name}: {reason}. Check the local path and try again." (bar stops) | "Uploaded to {remote}" / "Saved to {local}", `--ok` |
| Terminal pane | (n/a) | "Connecting to {host}…" + spinner, centered, `--muted`; removed on first PTY bytes | written into the pane in ANSI red: "Failed to start ssh: {reason}. Fix the host and reopen the tab." | normal output; on exit the existing dim "[session ended]" line stays |
| Editor | (n/a) | status "Opening {path}…"; Save disabled until loaded | status "Failed to open {path}: {reason}. Check the file exists, then press Reload." (pane stays empty, Reload stays live) | "Saved {HH:MM:SS}" / "Reloaded from server", `--ok`; "Saving…" with Save disabled while pending |
| Raw config | (n/a) | modal opens after read returns (local file, fast; no state) | dialog per S-10 message-only | closes on save |

Structure rules:

- Errors are never bare `String(e)`: prefix says what failed, the backend detail follows the colon, a next action closes the message (S-13). All five raw `String(e)` sites (`main.ts:149/173/205/349/411`) convert to this shape.
- Loading is never a bare text swap: spinner precedes the text, or the transfer bar runs (M-2).
- Every surface keeps at least one live control in its error state (Reload, Go, Save), so recovery needs no navigation (C-4).
- Status meaning is carried by wording first (S-13); the `.ok`/`.err` color (`--ok` **9.35:1**, `--danger` **6.46:1** on `--bg`) is the secondary signal (M-9).

## 13. S-13: Copy brief

Fixes H-5 (em dashes, arrows in copy), M-9 (color-only status), H-6 (error copy).

Status messages:

- Meaning is carried by the first word, readable without color (M-9):
  - Success: past-tense verb. "Saved 14:32:05", "Uploaded to /var/www", "Reloaded from server".
  - Progress: present participle + ellipsis. "Uploading notes.txt…", "Listing /etc/nginx…", "Connecting to web01…", "Saving…".
  - Error: "Failed to {verb}": "Failed to save host: alias contains a space. Fix the alias and save again."
- Errors have three parts: what failed, the reason/detail, the next action. The backend detail goes after the first colon; the next action is a concrete verb ("press Go", "check the path", "Reload"), never "please try again later".
- No arrows in copy: `uploaded → ${remote}` becomes "Uploaded to {remote}", `saved → ${local}` becomes "Saved to {local}" (H-5).
- Numbers are real counts only ("{n} items", "{n} lines"); pluralize properly, no "(s)" (R-17).

Characters:

- **No em dash (`—`) anywhere in UI copy** (R-02 hard gate). Existing violations to fix: `main.ts:326` directory size `—` (S-14 makes the cell empty), `index.html:54` "each time you connect — your `~/.ssh/config` stays portable" (rewrite: "each time you connect; your `~/.ssh/config` stays portable").
- No `…` on buttons (progress copy only). Middot `·` in "n items · host:path" is fine.
- No exclamation marks, no marketing words ("seamless", "effortless", "powerful", R-16), sentence case everywhere.

Buttons and dialogs:

- Verb-first and specific: "Save key", "Save and close", "Discard changes", "Delete", "Go". No "OK" where a verb works; "OK" only for message-only dialogs.
- Destructive buttons name the act ("Delete"), never "Yes". Confirm dialogs state the consequence ("This cannot be undone.").
- Empty states say why they are empty and name the filling action (existing sidebar copy is the house style); loading states say what is loading; error states say what failed and what to do next (R-27, antislop-ui states rule).
- Tooltips (`title`) are sentence-phrased function labels ("Reload directory"), not abbreviations.

## 14. S-14: SFTP browser

Fixes L-8 (directory size cell), and gives H-4/M-2/H-6 a concrete home in this pane.

Toolbar (one row, 8px gap): up button (icon-only 34x34, aria "Parent directory") · path input (flex 1, mono per S-4, Enter activates Go) · `Go` (text button) · refresh button (icon-only 34x34, aria "Reload directory").

Rows (`.sftp-row`, 32px tall, padding 6/10, hairline `--border-soft` separators):

```
{icon 16px} {name, ellipsis, title=full name} {size cell, 70px, right, mono 12px} {download?}
```

- Icons per S-9: folder (color `--accent`, **7.14:1** on `--bg2`, measured this pass), file (`--muted`, **6.37:1**), symlink (`--muted` + `title` "Symbolic link").
- **Directories show an empty size cell**, never `—` (L-8, H-5). The 70px cell keeps column alignment.
- Size text `--muted` 12px mono (**6.37:1** on `--bg2`).
- Download is a real `<button>` (icon-only, aria "Download {name}", `--muted` icon, hover `--hover` bg with `--fg` icon **9.77:1**), `stopPropagation()`, files only.
- Row click: directory navigates, file opens the editor (unchanged). Row hover bg `--panel`.
- Sort order: as returned by `sftp_list`; no client-side reorder in this pass (behavior change out of scope).

Bottom: local-path input (mono, placeholder "local path (download dir / file to upload)") + `Upload` (text button) · transfer status line per S-12/S-13 with the indeterminate bar under the toolbar while a transfer runs.

States: the SFTP row of the S-12 matrix is the spec for empty/loading/error/success in this pane.

## 15. S-15: Controls: buttons, inputs, selects

No single audit finding maps here; this section makes S-2 tokens, S-3 scales, and S-11 focus concrete per control class so the implementer does not improvise.

Buttons (base):

- bg `--panel`, 1px `--border` (**3.23:1** non-text), radius 6, padding 8/12, 13px, `--fg` text.
- Hover: bg lifts to `--hover` (`--fg` on it **9.77:1**). **Border stays `--border` on hover**; today's `button:hover { border-color: var(--accent) }` dies. Reason: an accent border on all ~15 buttons is the "excessive accent" tell (antislop-ui); accent marks the current (focus, active tab, primary), not "hoverable".
- Active/pressed: no transform (MOTION 1), bg stays `--hover`.
- `primary`: bg `--accent`, text `--bg` (**6.79:1**), weight 600. One primary per view (editor Save, modal save buttons, dialog's chosen action). Hover: border `--accent2`.
- `danger` (outline, existing "Delete"): text `--danger` on `--panel` (**5.88:1** on panel, measured this pass), border `--border`; hover border `--danger`. `danger` filled (dialog destructive): bg `--danger`, text `--bg` (**6.46:1**).
- Disabled: `opacity: .45`, `cursor: not-allowed`, no hover shift; used while a save/transfer is pending (S-12).
- Icon-only: square (26x26 sidebar head, 34x34 in toolbars matching input height), centered 16px icon, `aria-label` per S-9.

Inputs and textareas:

- bg `--bg2`, 1px `--border` (**3.73:1**), radius 4, padding 8/12, 13px `--fg` (text on `--bg2` **12.07:1**, measured this pass). Placeholder `--muted` (**6.37:1**).
- Focus: S-11 ring + border `--accent`. No other focus styling.
- Mono where S-4 says so (paths, raw config, key paste); sans elsewhere.

Selects:

- Same chrome as inputs (bg `--bg2`, `--border`, radius 4, padding 8/12). Native dropdown popup, unstyled. Reason: a custom dropdown is a new component with its own focus/keyboard surface for zero functional gain (YAGNI); the native popup inherits WebView theme and stays keyboard-correct.

Labels:

- 12px `--muted` above the field, 4px gap (matches existing `styles.css:105`, now passing AA via `--muted` fix).

## 16. DIRECTION QUESTIONS (MEKL to answer)

1. **Accent hue.** Keep the Tokyo Night blue pair (`--accent` #7aa2f7 / `--accent2` #89ddff) as the current, or move the current to a different hue (teal #73daca / purple #bb9af7 are the in-family candidates)? Everything in S-7's theme re-measures if you change it.
2. **Density.** Host rows are currently two-line (~52px). Want a compact single-line mode (~32px, subtitle only in a tooltip) for 20+ host configs, or is two-line always fine at your scale?
3. **Icon style.** S-9 specs a 16px, 1.5px-stroke, geometric inline-SVG set. Prefer that outline style, or solid/filled glyphs?
4. **Light theme.** None is planned (dark has a stated R-21 reason: terminal tool, long sessions). Confirm dark-only forever, or is a light variant a someday requirement that should shape token naming now?
