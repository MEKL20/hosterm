#!/usr/bin/env node
// qa-reports/theme-contrast.mjs — WCAG 2.1 contrast gate for src/theme-palettes.json.
//
// Reads the SAME file the runtime consumes (TSD §2.1: main.ts builds THEME_TERMINAL +
// HighlightStyle from it), plus the per-preset editor/chrome anchors straight out of
// src/styles.css, so the gate and the product cannot drift apart.
//
// Gated text pairs (AA = ratio >= 4.5, WCAG 2.1):
//   terminal : foreground x background, cursor x background (non-text UI, kept >= 4.5),
//              cursorAccent x cursor (glyph color under the block cursor),
//              every ANSI slot x background
//   editor   : every cm token x editor bg (= the preset's --bg, what cmTheme paints)
// Deliberate exceptions live in ALLOWED_EXCEPTIONS below: each carries a measured
// ratio at runtime plus a justification and a sanity cap so the exception cannot mask
// a regression (a slot drifting far past its documented value fails the gate).
// selection* are rgba overlays, not text pairs — excluded by design (TSD §2.1 LOCKED).
//
// Anchor asserts: terminal.background == --term, foreground == --text, cursor ==
// --accent per preset (the mapping Dusk already exhibits today), editor bg == --bg.
// Usage: node qa-reports/theme-contrast.mjs [--markdown]
//   default  : human table, exit 0 only when every gated pair passes or matches an
//              ALLOWED_EXCEPTIONS entry within its cap
//   --markdown: emit Appendix A tables for docs/tsd/theme-palettes-v1.md

import { readFileSync } from "node:fs";

const PRESETS = ["dusk", "slate", "ink", "light"];
const PALETTES = JSON.parse(
  readFileSync(new URL("../src/theme-palettes.json", import.meta.url), "utf8"),
);
const STYLES = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

const ANSI = [
  "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
  "brightBlack", "brightRed", "brightGreen", "brightYellow", "brightBlue",
  "brightMagenta", "brightCyan", "brightWhite",
];
const CM = [
  "keyword", "string", "number", "comment", "variableName", "function",
  "typeName", "propertyName", "operator", "punctuation",
];

// ---- WCAG 2.1 relative luminance + contrast ratio -------------------------------
// 0.03928 threshold per WCAG 2.1 (2.2 erratum switched to 0.04045; we stay on 2.1).
function luminance(hex) {
  const m = /^#[0-9a-fA-F]{6}$/.exec(hex);
  if (!m) throw new Error(`not a 6-digit hex color: ${hex}`);
  const [r, g, b] = [1, 3, 5]
    .map((i) => parseInt(hex.slice(i, i + 2), 16) / 255)
    .map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}
function ratio(a, b) {
  const [l1, l2] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (l1 + 0.05) / (l2 + 0.05);
}

// ---- chrome anchors from styles.css (single source, no hardcoded mirror) ---------
function tokenFromStyles(theme, token) {
  const blockRe =
    theme === "dusk"
      ? /:root\s*\{([\s\S]*?)\n\}/ // plain :root = Dusk default (styles.css:1)
      : new RegExp(`:root\\[data-theme="${theme}"\\]\\s*\\{([\\s\\S]*?)\\n\\}`);
  const m = STYLES.match(blockRe);
  if (!m) throw new Error(`styles.css: no :root block for theme "${theme}"`);
  const tok = m[1].match(new RegExp(`--${token}:\\s*(#[0-9a-fA-F]{6})`));
  if (!tok) throw new Error(`styles.css: --${token} not found in "${theme}" block`);
  return tok[1].toLowerCase();
}
const ANCHORS = Object.fromEntries(
  PRESETS.map((p) => [p, {
    bg: tokenFromStyles(p, "bg"),
    term: tokenFromStyles(p, "term"),
    text: tokenFromStyles(p, "text"),
    accent: tokenFromStyles(p, "accent"),
  }]),
);

// ---- structural shape gate (TSD §2.1: exact key sets, no extras) -----------------
function shapeCheck() {
  const errs = [];
  for (const p of PRESETS) {
    const pal = PALETTES[p];
    if (!pal) { errs.push(`preset "${p}" missing`); continue; }
    const wantT = ["background", "foreground", "cursor", "cursorAccent",
      "selectionBackground", "selectionForeground", "selectionInactiveBackground",
      ...ANSI];
    const wantC = CM;
    const gotT = Object.keys(pal.terminal ?? {});
    const gotC = Object.keys(pal.cm ?? {});
    if (gotT.length !== wantT.length || !wantT.every((k) => gotT.includes(k)))
      errs.push(`${p}.terminal keys != TSD §2.1 set (got ${gotT.join(",")})`);
    if (gotC.length !== wantC.length || !wantC.every((k) => gotC.includes(k)))
      errs.push(`${p}.cm keys != TSD §2.1 set (got ${gotC.join(",")})`);
    for (const [k, v] of Object.entries(pal.terminal ?? {}))
      if (!k.startsWith("selection") && !/^#[0-9a-fA-F]{6}$/.test(v))
        errs.push(`${p}.terminal.${k}: expected #rrggbb, got "${v}"`);
  }
  return errs;
}

// ---- deliberate exceptions (measured at runtime; cap = sanity bound) -------------
const BLACK_DARK =
  "ANSI black slot stays near-background by convention; it is never body text on a " +
  "dark terminal — apps emit SGR-40 (background) or paint it over colored backgrounds. " +
  "Raising it to 4.5 would turn it into a visible dark-grey slot and break black output.";
const CURSOR_ACCENT =
  "cursorAccent is the glyph color painted UNDER the block cursor, not text on the " +
  "background; it equals background by design (xterm convention). The readable pair " +
  "is cursorAccent x cursor, which the gate checks separately and passes >= 4.5.";
const BRIGHT_WHITE_LIGHT =
  "brightWhite keeps near-white slot fidelity; as foreground on a light terminal no " +
  "white tone can ever reach 4.5 — white fg output on light terminals is conventionally " +
  "delegated to colored backgrounds. The readable 'white' slot maps to mid-grey instead.";
const ALLOWED_EXCEPTIONS = [
  { preset: "dusk", pair: "cursorAccent × background", maxRatio: 1.10, reason: CURSOR_ACCENT },
  { preset: "slate", pair: "cursorAccent × background", maxRatio: 1.10, reason: CURSOR_ACCENT },
  { preset: "ink", pair: "cursorAccent × background", maxRatio: 1.10, reason: CURSOR_ACCENT },
  { preset: "light", pair: "cursorAccent × background", maxRatio: 1.10, reason: CURSOR_ACCENT },
  { preset: "dusk", pair: "black × background", maxRatio: 2.00, reason: BLACK_DARK },
  { preset: "slate", pair: "black × background", maxRatio: 2.00, reason: BLACK_DARK },
  { preset: "ink", pair: "black × background", maxRatio: 2.00, reason: BLACK_DARK },
  { preset: "light", pair: "brightWhite × background", maxRatio: 1.10, reason: BRIGHT_WHITE_LIGHT },
];

// ---- build all gated rows ---------------------------------------------------------
function buildRows() {
  const errs = shapeCheck();
  if (errs.length) throw new Error(`theme-palettes.json shape != TSD §2.1:\n  ${errs.join("\n  ")}`);
  const rows = [];
  for (const p of PRESETS) {
    const t = PALETTES[p].terminal;
    const bg = t.background;
    const a = ANCHORS[p];
    // design-rule asserts: palettes tied to chrome anchors (Dusk's own mapping today)
    if (bg !== a.term) errs.push(`${p}.terminal.background ${bg} != styles.css --term ${a.term}`);
    if (t.foreground !== a.text) errs.push(`${p}.terminal.foreground ${t.foreground} != --text ${a.text}`);
    if (t.cursor !== a.accent) errs.push(`${p}.terminal.cursor ${t.cursor} != --accent ${a.accent}`);
    rows.push({ preset: p, group: "terminal", pair: "foreground × background", fg: t.foreground, bg });
    rows.push({ preset: p, group: "terminal", pair: "cursor × background", fg: t.cursor, bg });
    rows.push({ preset: p, group: "terminal", pair: "cursorAccent × background", fg: t.cursorAccent, bg });
    rows.push({ preset: p, group: "terminal", pair: "cursorAccent × cursor", fg: t.cursorAccent, bg: t.cursor });
    for (const n of ANSI)
      rows.push({ preset: p, group: "terminal", pair: `${n} × background`, fg: t[n], bg });
    for (const c of CM)
      rows.push({ preset: p, group: "editor", pair: `${c} × cm-bg(--bg)`, fg: PALETTES[p].cm[c], bg: a.bg });
  }
  if (errs.length) throw new Error(`anchor asserts failed (styles.css vs palettes):\n  ${errs.join("\n  ")}`);
  return rows;
}

// ---- run the gate ------------------------------------------------------------------
const markdown = process.argv.includes("--markdown");
const rows = buildRows();
let pass = 0, exc = 0, fail = 0;
const failLines = [];
const excLog = [];
const mdByPreset = Object.fromEntries(PRESETS.map((p) => [p, []]));

for (const r of rows) {
  const v = ratio(r.fg, r.bg);
  const verdict = v >= 4.5 ? "PASS" : null;
  if (verdict) {
    pass++;
  } else {
    const e = ALLOWED_EXCEPTIONS.find((x) => x.pair === r.pair && x.preset === r.preset);
    if (e && v <= e.maxRatio) {
      exc++;
      const meaningful = r.pair === "cursorAccent × background"
        ? ` (readable pair cursorAccent × cursor = ${ratio(PALETTES[r.preset].terminal.cursorAccent, PALETTES[r.preset].terminal.cursor).toFixed(2)}:1 — PASS)`
        : "";
      excLog.push(`EXCEPTION ${r.preset} · ${r.pair} = ${v.toFixed(2)}:1 (cap ${e.maxRatio.toFixed(2)})${meaningful}\n  ↳ ${e.reason}`);
    } else {
      fail++;
      failLines.push(`FAIL ${r.preset} · ${r.pair} = ${v.toFixed(2)}:1 (${r.fg} on ${r.bg})${e ? " — ratio exceeded exception cap" : " — no ALLOWED_EXCEPTIONS entry"}`);
    }
  }
  mdByPreset[r.preset].push({ pair: r.pair, fg: r.fg, bg: r.bg, v, verdict: v >= 4.5 ? "PASS" : (failLines[failLines.length - 1]?.startsWith(`FAIL ${r.preset} · ${r.pair} `) ? "FAIL" : "EXCEPTION") });
}

if (markdown) {
  for (const p of PRESETS) {
    const a = ANCHORS[p];
    console.log(`### ${p} — terminal bg \`${PALETTES[p].terminal.background}\` (= --term), editor bg \`${a.bg}\` (= --bg)\n`);
    console.log("| Pair | Foreground | Background | Ratio | Verdict |");
    console.log("|---|---|---|---|---|");
    for (const r of mdByPreset[p])
      console.log(`| \`${r.pair}\` | \`${r.fg}\` | \`${r.bg}\` | ${r.v.toFixed(2)}:1 | ${r.verdict} |`);
    console.log("");
  }
  process.exit(fail === 0 ? 0 : 1);
}

console.log(`theme-contrast: WCAG 2.1 AA gate — src/theme-palettes.json`);
console.log(
  `chrome anchors (styles.css): ${PRESETS.map((p) => `${p} bg=${ANCHORS[p].bg} term=${ANCHORS[p].term} text=${ANCHORS[p].text} accent=${ANCHORS[p].accent}`).join(" · ")}`,
);
for (const p of PRESETS) {
  const t = PALETTES[p].terminal;
  console.log(`\n== ${p} (terminal bg ${t.background}, editor bg ${ANCHORS[p].bg})`);
  for (const r of mdByPreset[p])
    console.log(
      `  ${r.pair.padEnd(26)} ${r.fg} / ${r.bg}  ${r.v.toFixed(2).padStart(6)}:1  ${r.verdict}`,
    );
}
console.log(`\n== summary`);
console.log(`  pairs: ${rows.length} · PASS ${pass} · EXCEPTION ${exc} · FAIL ${fail}`);
for (const line of excLog) console.log(`  ${line.replace(/\n/g, "\n  ")}`);
for (const line of failLines) console.log(`  ${line}`);
console.log(
  `  selection* excluded (rgba overlay, not a text pair — TSD §2.1). scrollbar sliders not settable in xterm 5.5 ITheme.`,
);
console.log(fail === 0 ? `  GATE: PASS (exit 0)` : `  GATE: FAIL (exit 1)`);
process.exit(fail === 0 ? 0 : 1);
