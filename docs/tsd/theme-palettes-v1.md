# TSD — Theme-aware terminal + editor palettes (wave theme v1)

Status: ACTIVE · Breakdown via nano salvage (kartu t_b7971fa4; lead narration-exit 3x)
Repo: /home/mekl/src/hosterm · Base: main @ 26b1862 · Direct-push main (bukan PR)
Refs: docs/PRD.md §4 FR-THM1/THM2 · docs/SDD.md §698 (readiness caveat) · SDD §729-731 (AA palettes) · docs/FEATURE-SPEC-redesign-dusk-v1.md · docs/EPIC-TSD-redesign-dusk-v1.md (U-D4 Settings)

## 1. Problem (cited @ 26b1862)

Chrome aplikasi (sidebar/surface/text/accent) sudah mengikuti 4 preset via `:root[data-theme]`
(src/styles.css:45 slate, :65 ink, :85 light). Dua area TIDAK ikut:

1. **Terminal**: `TERMINAL_THEME` (src/main.ts:3019-3045) adalah palet Dusk JS-literal
   (bg #14100d, fg #f2e9dc, cursor #e2a35c, 16 ANSI) dan dipakai DUA konstruktor xterm:
   - main: `openTerminal` main.ts:3189-3194 (`theme:` @3193)
   - split pane: `splitTerminal` main.ts:3137-3142 (`theme:` @3141)
   xterm 5.5 `ITheme` tidak bisa membaca CSS var → ganti preset tidak me-repaint terminal.
2. **Editor**: `cmTheme` (main.ts:91-105) structural SUDAH CSS-var (theme-aware). HANYA
   token highlight `cmHighlight` (main.ts:107-118) = 10 hex literal Dusk
   (keyword/function #e2a35c, string #9dc489, number #e0a96d, comment #93836f,
   variableName #f2e9dc, typeName/propertyName #8fc4b8, operator/punctuation #b9aa95),
   di-wire statis di main.ts:2826 → warna token tidak pernah berubah.

Dokumentasi limitasi: PRD FR-THM2 ("known limitation"), SDD §698 (caveat Dusk-only).

Live registries: `sessions` Map + `splitSessions` Map (keduanya menyimpan `.term`);
editor session menyimpan `sess.view` (dipakai main.ts:2812).

## 2. Design (LOCKED — ikuti, jangan karang ulang)

### 2.1 Single source of truth: `src/theme-palettes.json`
```json
{ "dusk":   { "terminal": { background, foreground, cursor, cursorAccent,
                            selectionBackground, selectionForeground,
                            selectionInactiveBackground, black..brightWhite (16 ANSI) },
              "cm": { keyword, string, number, comment, variableName, function,
                      typeName, propertyName, operator, punctuation } },
  "slate": …, "ink": …, "light": … }
```
- main.ts membangun `THEME_TERMINAL: Record<ThemePreset, ITheme>` + HighlightStyle per
  preset dari JSON ini (aktifkan `resolveJsonModule` di tsconfig.json bila belum).
- `qa-reports/theme-contrast.mjs` memaca FILE YANG SAMA → gate dan produk tak bisa drift.
- **Dusk byte-identical** dengan `TERMINAL_THEME` + `cmHighlight` hari ini (regresi nol;
  smoke Dusk tetap hijau).

### 2.2 Mekanisme runtime (`applyTheme` main.ts:717)
a. Setelah swap `data-theme`: loop SEMUA terminal hidup (sessions + splitSessions) →
   `term.options.theme = THEME_TERMINAL[preset]`. xterm repaint saat option di-set;
   JANGAN recreate Terminal/PTY.
b. Editor: bungkus `syntaxHighlighting(...)` (main.ts:2826) dalam sebuah `Compartment`;
   saat applyTheme, untuk tiap editor view terbuka:
   `view.dispatch({ effects: cmp.reconfigure(syntaxHighlighting(highlightFor(preset))) })`.
   JANGAN recreate EditorView — doc, cursor, dirty flag WAJIB selamat.
c. Konstruktor membaca preset AKTIF saat open (terminal/editor baru setelah swap langsung
   dapat palet baru).
d. applyTheme jalan di startup (main.ts:809), klik preset (:947), reset (:957) → loop
   repaint harus no-op-safe saat nol terminal/editor terbuka.

### 2.3 Smoke visibility
Expose hook debug kecil yang di-update `applyTheme`, mis.
`(window as any).__hostermTheme = { preset, termBg, editorReconfigured }` → smoke assert
`termBg` benar-benar berubah saat swap + editor doc selamat. Tidak ada IPC baru → tidak
ada case mock invoke baru di harness.

## 3. Units

| Unit | Role | Deliverable |
|---|---|---|
| THEME-ARCH | architect | `src/theme-palettes.json` (4 palet) + `qa-reports/theme-contrast.mjs` (skrip rasio WCAG 2.1, exit 0 = lolos) + Appendix A tabel rasio di TSD ini. Dusk = salin byte-identik. |
| THEME-FE | frontend | Impl §2 + smoke ≥2 check baru + docs (PRD FR-THM2 → shipped, SDD §698 caveat diupdate) SAME COMMIT. Tanpa perubahan Rust. |
| THEME-QA | qa | Gate diff nyata: build 0 TS, contrast.mjs exit 0, smoke hijau incl. check baru, cargo --lib pass, re-verify AC §4. |
| THEME-SEC | security | Scan ringan diff: sink, sumber warna, CSP, INV-21/22/25. |

Palet baru (LOCKED): **Light** = terminal terang sungguhan (bg terang, fg gelap, ANSI
kontras di bg terang) — desain baru dari nol, bukan invert. **Slate/Ink** = palet gelap
diturunkan dari anchor preset (styles.css:45/65/85): slate --bg #171d24 / --surface
#222a33 / --text #e4ebf2 / --accent #6cc4dc; ink #141414 / #1e1e1e / #ececec / #e0563f.
AA = WCAG 2.1 rasio ≥ 4.5 untuk pasangan teks (fg vs bg, cursorAccent vs bg, tiap ANSI
sebagai teks vs bg, tiap token CM vs bg editor preset) — dihitung skrip, bukan mata;
pengecualian disengaja (mis. brightBlack sebagai comment faint) wajib tercatat dengan
rasio terukur + justifikasi.

## 4. Wave acceptance criteria
1. Slate/Ink/Light → background/teks terminal + 16 ANSI + token editor berganti per
   preset; Dusk tak berubah.
2. Ganti tema saat terminal (termasuk split pane) + editor TERBUKA → langsung repaint
   tanpa buka ulang; editor doc/cursor/dirty selamat.
3. Light terminal terang + teks gelap, AA pass.
4. Keempat palet terdokumentasi rasionya; `node qa-reports/theme-contrast.mjs` exit 0.
5. Gate hijau: `npm run build` 0 TS · headless smoke hijau incl. ≥2 check baru ·
   `cargo test --manifest-path src-tauri/Cargo.toml --lib` pass (bukti Rust untouched) ·
   INV-21 anti-Abyss grep pass.
6. Docs same-commit (PRD FR-THM2, SDD §698). Headless TIDAK bisa konfirmasi warna
   render nyata → MEKL buka .exe (final visual gate).

## 5. Invariants & guardrails
- **INV-21** anti-Abyss: nilai Abyss tidak boleh muncul lagi; Dusk byte-identical.
- **INV-22**: schema settings/sidecar TIDAK berubah (persistence via write_settings
  sudah ada; theme-palettes.json = bundled asset, bukan config user).
- **INV-25** approval visual-weight: untouched.
- xterm 5.5: warna scrollbar slider tidak bisa lewat ITheme (komentar main.ts:3027) —
  out of scope.

## 6. Logistik
- Commit identity: `git -c user.name="Edwin Khoirudin" -c user.email="69399477+MEKL20@users.noreply.github.com"`
- Push: `git push github-mekl20 main` (direct-push, repo publik MIT — tanpa secret/path personal).
- completion_contract = local-only di semua kartu. Terminal kanban call WAJIB di akhir.
- Smoke: port 4199 sekali jalan; playwright --no-save sering hilang setelah checkout baru
  (`npm install --no-save playwright@1.63.0`).

## Appendix A — Palet & bukti WCAG AA (THEME-ARCH, kartu t_3b1a8c1b)

SSOT: `src/theme-palettes.json` · Gate: `qa-reports/theme-contrast.mjs` (exit 0 = AA pass).
Tabel rasio di A.4 digenerate `node qa-reports/theme-contrast.mjs --markdown` — jangan
edit angkanya manual; regenerate dari skrip.

### A.1 Metode & design rules

- Rasio WCAG 2.1: relative luminance (threshold linearisasi 0.03928), rasio
  `(L1+0.05)/(L2+0.05)`, ambang AA teks ≥ 4.5:1.
- Pasangan yang digerbangi (120 total): `foreground × background`, `cursor × background`
  (bukan pasangan teks WCAG murni, dijaga ≥ 4.5 agar kursor selalu kelihatan),
  `cursorAccent × cursor` (warna glyph di bawah kursor blok), 16 ANSI × background,
  dan 10 token cm × bg editor preset. `selection*` = overlay rgba, bukan pasangan teks →
  tidak digerbangi (LOCKED §2.1). Scrollbar slider tidak settable lewat ITheme xterm 5.5
  (komentar main.ts:3027) — out of scope.
- Design rule yang di-assert skrip (drift-proof): per preset,
  `terminal.background == --term`, `terminal.foreground == --text`,
  `terminal.cursor == --accent` (pola yang Dusk sudah pakai hari ini), dan bg editor cm
  `== --bg` (yang `cmTheme` paint via `&`). Anchor dibaca LANGSUNG dari `src/styles.css`
  oleh skrip — ubah anchor tanpa sinkron palet = gate merah.
- Kalibrasi: skrip mereproduksi persis angka anchor SDD §13.3 — slate text 14.11 /
  accent 8.54 · ink text 15.59 / accent 4.89 · light text 14.29 / accent 5.07.
- Pengecualian disengaja tercatat di `ALLOWED_EXCEPTIONS` (dalam skrip) dengan rasio
  terukur runtime + sanity cap — rasio yang menyimpang jauh dari nilai terdokumentasi
  tetap FAIL (A.3).

### A.2 Catatan desain per preset

| Preset | terminal bg (= --term) | editor bg (= --bg) | fg (= --text) | cursor (= --accent) | Bahasa warna |
|---|---|---|---|---|---|
| dusk | `#14100d` | `#191512` | `#f2e9dc` | `#e2a35c` | byte-identical `TERMINAL_THEME` + `cmHighlight` (regresi nol) |
| slate | `#11161c` | `#171d24` | `#e4ebf2` | `#6cc4dc` | keluarga cool blue-grey; red=--danger `#e0806a`, green=--live `#8fd0a8`, cyan=--accent |
| ink | `#0e0e0e` | `#141414` | `#ececec` | `#e0563f` | charcoal hampir-akromatik; red=--danger=--accent `#e0563f`, green=--live `#86c98a`, slot lain grey-toned |
| light | `#faf7f0` | `#f5f1e8` | `#26201a` | `#96571a` | kertas hangat; ANSI gelap-kontras di bg terang, red/green = --danger/--live preset light |

- **dusk** — disalin 1:1 dari `TERMINAL_THEME` (main.ts:3019-3045) + `cmHighlight`
  (main.ts:107-118); tidak ada hex yang disentuh. Diverifikasi terprogram: 23/23 field
  terminal + 10/10 token cm identik dengan literal main.ts.
- **slate** — ANSI diturunkan dari anchor slate: slot semantik memakai token chrome yang
  sudah AA-proven (red=--danger, green=--live, cyan=--accent), sisanya cool blue-grey
  mid-tone; `white` tetap slot terang (`#c9d4de`, 12.08:1). Semua pasangan ≥ 4.5
  (terkecil: cm comment `#7e8b9c` = 4.90).
- **ink** — palet hampir-akromatik mengikuti anchor charcoal: white/brightBlack abu
  netral (`#c9c9c9`/`#808080`), warna hanya di slot semantik (red=accent=danger,
  green=live) + teal redup. Keyword/function cm = accent `#e0563f` → 4.89:1 (pasangan
  termisket di palet ini, tetap di atas 4.5; tokennya bisa digelapkan nanti bila butuh
  headroom — jangan naikkan di atas accent tanpa keputusan SDD).
- **light** — terminal terang sungguhan, desain baru dari nol (BUKAN invert): ANSI
  digelapkan agar kontras di bg terang (black 17.06:1 … green 4.63:1; semua ≥ 4.5
  kecuali slot putih). Slot `white` memetakan ke abu hangat mid-tone `#57503f` (7.48:1)
  supaya output white-fg tetap terbaca (konvensi light theme), sedangkan `brightWhite`
  tetap `#fffdf8` demi fidelitas slot → satu-satunya exception ANSI light (A.3). Token
  cm light memakai versi lebih dalam dari ANSI (mis. string `#356b40` vs green ANSI
  `#3f7d4a`) karena bg editor `--bg #f5f1e8` lebih gelap dari `--term #faf7f0` — kontras
  token sedikit lebih rendah di editor, perlu satu langkah kedalaman ekstra agar ≥ 4.5.

### A.3 Pengecualian terdaftar (ALLOWED_EXCEPTIONS)

| Preset | Pair | Rasio terukur | Cap | Justifikasi |
|---|---|---|---|---|
| dusk/slate/ink/light | `cursorAccent × background` | 1.00:1 | ≤1.10 | cursorAccent = warna glyph DI BAWAH kursor blok, bukan teks di atas bg; == background by design (konvensi xterm). Pasangan yang benar-benar terbaca, `cursorAccent × cursor`, digerbangi terpisah dan PASS (dusk 8.68 / slate 9.15 / ink 5.12 / light 5.34). |
| dusk | `black × background` | 1.22:1 | ≤2.00 | slot ANSI black harus tetap near-bg by convention; bukan body text di terminal gelap (dipakai sebagai SGR-40 background / overlay di atas bg berwarna). Menaikkan ke 4.5 mengubahnya jadi slot abu gelap yang salah semantik. |
| slate | `black × background` | 1.29:1 | ≤2.00 | idem dusk. |
| ink | `black × background` | 1.17:1 | ≤2.00 | idem dusk. |
| light | `brightWhite × background` | 1.05:1 | ≤1.10 | fidelitas slot putih pada terminal terang; tidak ada tone putih yang bisa mencapai 4.5 di bg terang — slot `white` yang perlu terbaca sudah dipetakan ke mid-grey `#57503f` (7.48:1). |

### A.4 Tabel rasio lengkap (generate: `node qa-reports/theme-contrast.mjs --markdown`)

### dusk — terminal bg `#14100d` (= --term), editor bg `#191512` (= --bg)

| Pair | Foreground | Background | Ratio | Verdict |
|---|---|---|---|---|
| `foreground × background` | `#f2e9dc` | `#14100d` | 15.74:1 | PASS |
| `cursor × background` | `#e2a35c` | `#14100d` | 8.68:1 | PASS |
| `cursorAccent × background` | `#14100d` | `#14100d` | 1.00:1 | EXCEPTION |
| `cursorAccent × cursor` | `#14100d` | `#e2a35c` | 8.68:1 | PASS |
| `black × background` | `#2a231c` | `#14100d` | 1.22:1 | EXCEPTION |
| `red × background` | `#e07856` | `#14100d` | 6.30:1 | PASS |
| `green × background` | `#9dc489` | `#14100d` | 9.63:1 | PASS |
| `yellow × background` | `#e2a35c` | `#14100d` | 8.68:1 | PASS |
| `blue × background` | `#8fb3c9` | `#14100d` | 8.52:1 | PASS |
| `magenta × background` | `#c99bb3` | `#14100d` | 7.95:1 | PASS |
| `cyan × background` | `#8fc4b8` | `#14100d` | 9.69:1 | PASS |
| `white × background` | `#b9aa95` | `#14100d` | 8.33:1 | PASS |
| `brightBlack × background` | `#93836f` | `#14100d` | 5.15:1 | PASS |
| `brightRed × background` | `#e07856` | `#14100d` | 6.30:1 | PASS |
| `brightGreen × background` | `#9dc489` | `#14100d` | 9.63:1 | PASS |
| `brightYellow × background` | `#e2a35c` | `#14100d` | 8.68:1 | PASS |
| `brightBlue × background` | `#8fb3c9` | `#14100d` | 8.52:1 | PASS |
| `brightMagenta × background` | `#c99bb3` | `#14100d` | 7.95:1 | PASS |
| `brightCyan × background` | `#8fc4b8` | `#14100d` | 9.69:1 | PASS |
| `brightWhite × background` | `#f2e9dc` | `#14100d` | 15.74:1 | PASS |
| `keyword × cm-bg(--bg)` | `#e2a35c` | `#191512` | 8.32:1 | PASS |
| `string × cm-bg(--bg)` | `#9dc489` | `#191512` | 9.24:1 | PASS |
| `number × cm-bg(--bg)` | `#e0a96d` | `#191512` | 8.70:1 | PASS |
| `comment × cm-bg(--bg)` | `#93836f` | `#191512` | 4.94:1 | PASS |
| `variableName × cm-bg(--bg)` | `#f2e9dc` | `#191512` | 15.09:1 | PASS |
| `function × cm-bg(--bg)` | `#e2a35c` | `#191512` | 8.32:1 | PASS |
| `typeName × cm-bg(--bg)` | `#8fc4b8` | `#191512` | 9.29:1 | PASS |
| `propertyName × cm-bg(--bg)` | `#8fc4b8` | `#191512` | 9.29:1 | PASS |
| `operator × cm-bg(--bg)` | `#b9aa95` | `#191512` | 7.99:1 | PASS |
| `punctuation × cm-bg(--bg)` | `#b9aa95` | `#191512` | 7.99:1 | PASS |

### slate — terminal bg `#11161c` (= --term), editor bg `#171d24` (= --bg)

| Pair | Foreground | Background | Ratio | Verdict |
|---|---|---|---|---|
| `foreground × background` | `#e4ebf2` | `#11161c` | 15.12:1 | PASS |
| `cursor × background` | `#6cc4dc` | `#11161c` | 9.15:1 | PASS |
| `cursorAccent × background` | `#11161c` | `#11161c` | 1.00:1 | EXCEPTION |
| `cursorAccent × cursor` | `#11161c` | `#6cc4dc` | 9.15:1 | PASS |
| `black × background` | `#242c35` | `#11161c` | 1.29:1 | EXCEPTION |
| `red × background` | `#e0806a` | `#11161c` | 6.46:1 | PASS |
| `green × background` | `#8fd0a8` | `#11161c` | 10.17:1 | PASS |
| `yellow × background` | `#d4b06a` | `#11161c` | 8.84:1 | PASS |
| `blue × background` | `#7ea8d8` | `#11161c` | 7.34:1 | PASS |
| `magenta × background` | `#c793c4` | `#11161c` | 7.27:1 | PASS |
| `cyan × background` | `#6cc4dc` | `#11161c` | 9.15:1 | PASS |
| `white × background` | `#c9d4de` | `#11161c` | 12.08:1 | PASS |
| `brightBlack × background` | `#7e8b9c` | `#11161c` | 5.25:1 | PASS |
| `brightRed × background` | `#e0806a` | `#11161c` | 6.46:1 | PASS |
| `brightGreen × background` | `#8fd0a8` | `#11161c` | 10.17:1 | PASS |
| `brightYellow × background` | `#d4b06a` | `#11161c` | 8.84:1 | PASS |
| `brightBlue × background` | `#7ea8d8` | `#11161c` | 7.34:1 | PASS |
| `brightMagenta × background` | `#c793c4` | `#11161c` | 7.27:1 | PASS |
| `brightCyan × background` | `#6cc4dc` | `#11161c` | 9.15:1 | PASS |
| `brightWhite × background` | `#e4ebf2` | `#11161c` | 15.12:1 | PASS |
| `keyword × cm-bg(--bg)` | `#6cc4dc` | `#171d24` | 8.54:1 | PASS |
| `string × cm-bg(--bg)` | `#8fd0a8` | `#171d24` | 9.50:1 | PASS |
| `number × cm-bg(--bg)` | `#d4b06a` | `#171d24` | 8.26:1 | PASS |
| `comment × cm-bg(--bg)` | `#7e8b9c` | `#171d24` | 4.90:1 | PASS |
| `variableName × cm-bg(--bg)` | `#e4ebf2` | `#171d24` | 14.11:1 | PASS |
| `function × cm-bg(--bg)` | `#6cc4dc` | `#171d24` | 8.54:1 | PASS |
| `typeName × cm-bg(--bg)` | `#7ea8d8` | `#171d24` | 6.85:1 | PASS |
| `propertyName × cm-bg(--bg)` | `#7ea8d8` | `#171d24` | 6.85:1 | PASS |
| `operator × cm-bg(--bg)` | `#a6b4c2` | `#171d24` | 8.02:1 | PASS |
| `punctuation × cm-bg(--bg)` | `#a6b4c2` | `#171d24` | 8.02:1 | PASS |

### ink — terminal bg `#0e0e0e` (= --term), editor bg `#141414` (= --bg)

| Pair | Foreground | Background | Ratio | Verdict |
|---|---|---|---|---|
| `foreground × background` | `#ececec` | `#0e0e0e` | 16.34:1 | PASS |
| `cursor × background` | `#e0563f` | `#0e0e0e` | 5.12:1 | PASS |
| `cursorAccent × background` | `#0e0e0e` | `#0e0e0e` | 1.00:1 | EXCEPTION |
| `cursorAccent × cursor` | `#0e0e0e` | `#e0563f` | 5.12:1 | PASS |
| `black × background` | `#1f1f1f` | `#0e0e0e` | 1.17:1 | EXCEPTION |
| `red × background` | `#e0563f` | `#0e0e0e` | 5.12:1 | PASS |
| `green × background` | `#86c98a` | `#0e0e0e` | 9.87:1 | PASS |
| `yellow × background` | `#d2a45c` | `#0e0e0e` | 8.46:1 | PASS |
| `blue × background` | `#9aabc4` | `#0e0e0e` | 8.27:1 | PASS |
| `magenta × background` | `#c291a8` | `#0e0e0e` | 7.27:1 | PASS |
| `cyan × background` | `#86c4bd` | `#0e0e0e` | 9.79:1 | PASS |
| `white × background` | `#c9c9c9` | `#0e0e0e` | 11.66:1 | PASS |
| `brightBlack × background` | `#808080` | `#0e0e0e` | 4.89:1 | PASS |
| `brightRed × background` | `#e0563f` | `#0e0e0e` | 5.12:1 | PASS |
| `brightGreen × background` | `#86c98a` | `#0e0e0e` | 9.87:1 | PASS |
| `brightYellow × background` | `#d2a45c` | `#0e0e0e` | 8.46:1 | PASS |
| `brightBlue × background` | `#9aabc4` | `#0e0e0e` | 8.27:1 | PASS |
| `brightMagenta × background` | `#c291a8` | `#0e0e0e` | 7.27:1 | PASS |
| `brightCyan × background` | `#86c4bd` | `#0e0e0e` | 9.79:1 | PASS |
| `brightWhite × background` | `#ececec` | `#0e0e0e` | 16.34:1 | PASS |
| `keyword × cm-bg(--bg)` | `#e0563f` | `#141414` | 4.89:1 | PASS |
| `string × cm-bg(--bg)` | `#86c98a` | `#141414` | 9.42:1 | PASS |
| `number × cm-bg(--bg)` | `#d2a45c` | `#141414` | 8.08:1 | PASS |
| `comment × cm-bg(--bg)` | `#808080` | `#141414` | 4.66:1 | PASS |
| `variableName × cm-bg(--bg)` | `#ececec` | `#141414` | 15.59:1 | PASS |
| `function × cm-bg(--bg)` | `#e0563f` | `#141414` | 4.89:1 | PASS |
| `typeName × cm-bg(--bg)` | `#86c4bd` | `#141414` | 9.34:1 | PASS |
| `propertyName × cm-bg(--bg)` | `#86c4bd` | `#141414` | 9.34:1 | PASS |
| `operator × cm-bg(--bg)` | `#aaaaaa` | `#141414` | 7.93:1 | PASS |
| `punctuation × cm-bg(--bg)` | `#aaaaaa` | `#141414` | 7.93:1 | PASS |

### light — terminal bg `#faf7f0` (= --term), editor bg `#f5f1e8` (= --bg)

| Pair | Foreground | Background | Ratio | Verdict |
|---|---|---|---|---|
| `foreground × background` | `#26201a` | `#faf7f0` | 15.05:1 | PASS |
| `cursor × background` | `#96571a` | `#faf7f0` | 5.34:1 | PASS |
| `cursorAccent × background` | `#faf7f0` | `#faf7f0` | 1.00:1 | EXCEPTION |
| `cursorAccent × cursor` | `#faf7f0` | `#96571a` | 5.34:1 | PASS |
| `black × background` | `#1a140e` | `#faf7f0` | 17.06:1 | PASS |
| `red × background` | `#b3442a` | `#faf7f0` | 5.20:1 | PASS |
| `green × background` | `#3f7d4a` | `#faf7f0` | 4.63:1 | PASS |
| `yellow × background` | `#8f6400` | `#faf7f0` | 4.92:1 | PASS |
| `blue × background` | `#2d5f8a` | `#faf7f0` | 6.30:1 | PASS |
| `magenta × background` | `#a0307a` | `#faf7f0` | 6.14:1 | PASS |
| `cyan × background` | `#0e7a72` | `#faf7f0` | 4.85:1 | PASS |
| `white × background` | `#57503f` | `#faf7f0` | 7.48:1 | PASS |
| `brightBlack × background` | `#6e6455` | `#faf7f0` | 5.43:1 | PASS |
| `brightRed × background` | `#b3442a` | `#faf7f0` | 5.20:1 | PASS |
| `brightGreen × background` | `#3f7d4a` | `#faf7f0` | 4.63:1 | PASS |
| `brightYellow × background` | `#8f6400` | `#faf7f0` | 4.92:1 | PASS |
| `brightBlue × background` | `#2d5f8a` | `#faf7f0` | 6.30:1 | PASS |
| `brightMagenta × background` | `#a0307a` | `#faf7f0` | 6.14:1 | PASS |
| `brightCyan × background` | `#0e7a72` | `#faf7f0` | 4.85:1 | PASS |
| `brightWhite × background` | `#fffdf8` | `#faf7f0` | 1.05:1 | EXCEPTION |
| `keyword × cm-bg(--bg)` | `#96571a` | `#f5f1e8` | 5.07:1 | PASS |
| `string × cm-bg(--bg)` | `#356b40` | `#f5f1e8` | 5.60:1 | PASS |
| `number × cm-bg(--bg)` | `#7d5600` | `#f5f1e8` | 5.82:1 | PASS |
| `comment × cm-bg(--bg)` | `#6e6455` | `#f5f1e8` | 5.15:1 | PASS |
| `variableName × cm-bg(--bg)` | `#26201a` | `#f5f1e8` | 14.29:1 | PASS |
| `function × cm-bg(--bg)` | `#96571a` | `#f5f1e8` | 5.07:1 | PASS |
| `typeName × cm-bg(--bg)` | `#2d5f8a` | `#f5f1e8` | 5.98:1 | PASS |
| `propertyName × cm-bg(--bg)` | `#2d5f8a` | `#f5f1e8` | 5.98:1 | PASS |
| `operator × cm-bg(--bg)` | `#5f564a` | `#f5f1e8` | 6.39:1 | PASS |
| `punctuation × cm-bg(--bg)` | `#5f564a` | `#f5f1e8` | 6.39:1 | PASS |

### A.5 Ringkasan gate

`pairs: 120 · PASS 112 · EXCEPTION 8 · FAIL 0` → `node qa-reports/theme-contrast.mjs` exit 0.
Skrip juga menolak (exit 1) bila: shape JSON menyimpang dari TSD §2.1, anchor
styles.css/palet tidak sinkron (assert --term/--text/--accent/--bg), atau rasio
exception melewati cap-nya.
