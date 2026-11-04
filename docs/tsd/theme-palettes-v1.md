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
