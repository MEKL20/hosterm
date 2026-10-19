# EPIC-TSD: Redesign UX "Arus" + tema Abyss Emerald — playbook implementasi

| Field | Value |
|---|---|
| Author | crew-coding-lead |
| Source | `docs/UX/redesign-hosterm-v1.md` (§2 Arah A, §3 layout, §4 komponen, §5 keyboard, §6 + §6.5 token, §7 mockup, §8 fix list), `docs/PRD-hosterm-v1.md`, `docs/STATE.md`, `DESIGN.md` |
| Keputusan MEKL | FINAL: Arah A "Arus" + tema **Abyss Emerald** (redesign §6.5, 19 Okt 2026). Bukan untuk didiskusikan ulang. |
| Mockup visual | `docs/UX/mockups/main.html`, `editor.html`, `dialog-password-host.html` (link `theme-abyss-emerald.css`) |
| Gate | Semua unit kode = **triple gate penuh**: lead review + QA PASS + security clean. Developer tidak pernah push; lead yang collect & push. |
| Status | Spec — belum ada kode |

## 0. Sequencing (WAJIB dibaca sebelum dispatch)

Blocker awal §0 ini **SUDAH MENDARAT**. "Kerja signing/updater di working tree" ternyata = W2-A (startup ssh/sftp check) yang dikerjakan VIA KANBAN, bukan di luar: backend t_60b0c27b + frontend t_f9164f39, review chain QA t_367c902c PASS + SEC t_e16c372c CLEAN, di-push bersama dokumen ini (commit `feat: W2-A …` diikuti commit docs ini). Tidak ada working-tree blocker lagi; unit-unit di bawah boleh dispatch setelah push ini.

**Hotspot:** `src/styles.css` + `src/main.ts` disentuh SEMUA unit → dispatch unit harus **seri** (urutan §2), tidak paralel. Jangan pernah dua card hosterm aktif di file yang sama.

Baseline verify loop (STATE.md; terukur 19 Okt 2026 setelah W2-A): `cargo test --lib` → 52 pass; `node qa-reports/headless-smoke.mjs` → 70 pass; `npm run build` → 0 TS error. **Ukur ulang di working tree saat dispatch** — unit sebelumnya bisa menggeser baseline; angka terbaru jadi baseline unit berikutnya.

## 1. Yang sudah selesai (JANGAN dikerjakan ulang)

- Fix list §8 **#1** (copy `applyAuthMode` sidecar): **SUDAH SHIP** (t_ffc96bd4, commit 6ce0381). `main.ts` applyAuthMode sudah menulis "stored in ~/.ssh/hosterm.secrets (file perms 0600)…". Sisa dari §1.6 dialog: badge chip D2 + note opt-in D3 → Unit 8.
- Fondasi yang wajib dipertahankan apa adanya (redesign §1.7): copy empty/error/loading naratif, overlay focus-trap + Escape + restore focus, `:focus-visible` global, xfer-bar indeterminate + `prefers-reduced-motion`, tab keyboard ←/→.

## 2. Unit breakdown (urutan dispatch = urutan tabel)

| Unit | Judul | Deps | Fix list §8 | File utama |
|---|---|---|---|---|
| U1 | Tema Abyss Emerald (token swap) | — | — | styles.css, main.ts (theme literal), smoke |
| U2 | Sidebar Arus (single-line + grouping + context menu + Ctrl+K) | U1 | #4, #5(sebagian) | index.html, main.ts, styles.css, smoke |
| U3 | Tab bar Arus (kind color, scroll, disconnect, close-others) | U2 | #3 | main.ts, styles.css, index.html, smoke |
| U4 | Terminal: well + divider split + status bar global | U1 | #6, #7 | index.html, main.ts, styles.css, smoke |
| U5 | SFTP: transfer strip + pane-head actions + selection/drop | U1 | #8 | main.ts, styles.css, smoke |
| U6 | Skeleton rows (local/remote list) + empty CTA | U2 | #5(sisa) | main.ts, styles.css, smoke |
| U7 | Editor: Ctrl+S panel-level + dirty di ed-bar | U1 | #2 | main.ts, styles.css, smoke |
| U8 | Dialog password host (badge stored + note opt-in) | U1 | §4.6 (D2/D3) | index.html, main.ts, styles.css, smoke |
| U9 | Smoke suite: kanonisisasi cek UX baru + audit regressi | U1–U8 | #9 | qa-reports/headless-smoke.mjs |

Setiap unit = 1 task kanban frontend, dibuat nano dari breakdown ini. Sitasi wajib di tiap task body: `EPIC-TSD §unit` + `redesign §x` + `PRD §y`.

---

### U1 — Tema Abyss Emerald (token swap)

**Sumber:** redesign §6 + §6.5 (token + kontras terukur + aturan tegas), §6.4 (aturan), PRD §3.2 NFR-5, AC-1.

**Cakupan file (persis):**
- `src/styles.css` — ganti seluruh blok `:root` token dengan nilai Abyss Emerald (§6.5 / `mockups/theme-abyss-emerald.css`): `--bg #08080a`, `--bg2 #050507`, `--panel #101010`, `--hover #1a1a19`, `--well #020203`, `--fg #f2f2f2`, `--muted #b8b3b0`, `--accent #00d992`, `--accent2 #818cf8`, `--ok #80d280`, `--warn #ffdd80`, `--danger #fd9c9f`, `--border #3d3a39`, `--border-soft #262423`, plus token baru `--sel #0e2a20`, `--glow rgba(0,217,146,.22)`, `--bar-h 24px` (redesign §6.1). Tambah `color-scheme: dark` di `:root`.
- `src/styles.css` — font stack TANPA CDN (app offline): `--font-sans`/`--font-mono` persis `theme-abyss-emerald.css` baris 36–37 (JetBrains Mono/Geist/Cascadia/Inter prioritas lokal, fallback sistem). **Dilarang** `@import url(...)`, `@font-face` dengan `src: url(http…)`, atau `<link>` font eksternal — verify grep.
- `src/styles.css` — aturan glow **hanya state aktif** (§6.5 + superhuman.md): `.tab.active`, `.term-side.focused`, `button.primary` box-shadow `var(--glow)`. Dilarang glow pada hover/idle.
- `src/main.ts` — sinkronkan literal yang CSS var tidak bisa jangkau: `FONT_MONO` const, objek `theme` xterm (26 key, remap ke palet emerald: bg=`--well` #020203, fg=#f2f2f2, cursor=#00d992, ANSI map dari #80d280/#ffdd80/#fd9c9f dst.), `cmTheme`/`cmHighlight` (activeLine/selection dihitung ulang di atas #020203/#0e2a20; syntax: kw #818cf8, str #80d280, num #ffb86c, com #7d7873, prop #2fd6a1 — §6.5).
- `qa-reports/headless-smoke.mjs` — cek baru: (a) `getComputedStyle(document.documentElement).getPropertyValue('--bg')` = `#08080a`; (b) `color-scheme` dark; (c) tidak ada `@import`/`url(http` di stylesheet; (d) box-shadow glow hanya pada `.tab.active`/`.term-side.focused`/`button.primary` (query selector count).

**AC terukur:**
1. Semua 16+3 token di `styles.css` identik nilai §6.5; tidak ada sisa literal Tokyo Night (`#1a1b26`, `#7aa2f7`, `#c8d3f5`, dst.) di `src/` — grep 0 hit (kecuali komentar historis dilarang; bersihkan).
2. `color-scheme: dark` ada di `:root`; smoke PASS cek (a)–(d).
3. Nol permintaan jaringan font: grep `url(http\|@import` di `styles.css` + `index.html` = 0.
4. Glow muncul hanya di 3 selector aktif; `.tab:not(.active)` dan hover tanpa box-shadow (smoke assertion).
5. Verify loop hijau: cargo ≥ baseline, build 0 error, smoke baseline + 4 cek baru.

**Dilarang:** menyentuh geometri/layout (unit lain), mengubah `--shadow-modal`, mengganti struktur selector.

---

### U2 — Sidebar Arus

**Sumber:** redesign §1.1 (S1–S5), §4.1, §3 wireframe, §5 keyboard (Ctrl+K, ↑/↓+Enter); PRD US-A1, FR-A1 (wildcard tetap tidak dirender), US-C3; §8 fix #4 + #5 bagian sidebar.

**Cakupan file:** `src/main.ts` (`renderHostList`, context-menu helper baru, handler Ctrl+K), `src/styles.css` (host row/group/ctx-menu), `index.html` (placeholder search → "Search hosts (Ctrl+K)…"), smoke.

**Detail implementasi (§4.1 literal):** row satu lini 28px (dot status 6px `--ok` bila ada tab terminal hidup host itu / `--muted` idle; nama ellipsis; subtitle `user@host:port` → `title` tooltip; ikon kunci `--warn` bila has_password; tombol ⇅ 24×24 hover-only `stopPropagation`). Group header 11px uppercase `--muted`, label = prefix alias sebelum `-`. Edit/Delete **keluar dari row** → context menu klik-kanan/tombol ⋯: Open SFTP · Edit · Delete (danger). Search `Ctrl+K` fokus + `↑/↓+Enter` navigasi. Loading sidebar = skeleton 3 baris statis `--hover` (bukan spinner-replace); empty copy tetap + CTA "Add host" inline; error copy tetap `--danger`.

**AC terukur:**
1. `.host-item` tinggi terukur 28px, satu lini; subtitle tidak dirender sebagai elemen kedua (jadi `title`).
2. Host `prod-web` (mock) tampil di bawah group header "PROD"; `Host *` wildcard tidak muncul sebagai row (FR-A1 tetap).
3. Edit/Delete tidak ada di DOM row; context menu berisi 3 item; klik Delete → dialog konfirmasi existing (jangan native confirm).
4. Ctrl+K memfokuskan `#host-search` (smoke: `document.activeElement.id`).
5. Dot `--ok` menyala persis untuk host dengan tab terminal terbuka; tutup tab → dot kembali `--muted` (smoke).
6. Skeleton: `loadHosts()` tidak lagi `innerHTML = spinner`; 3 elemen `.skel` statis (smoke + tanpa layout jump: tinggi container stabil).
7. Verify loop hijau + smoke bertambah ≥ 5 cek baru.

**Pitfall:** tombol ⇅ hover-only tetap `<button>` asli (bukan span/opacity-0 yang tak focusable — R-32 DESIGN.md); focus-within harus tetap menampilkannya.

---

### U3 — Tab bar Arus

**Sumber:** redesign §1.2 (T1–T5), §4.2, §5 (Ctrl+W/Ctrl+Shift+W, ←/→ existing); PRD US-B1, UC1; fix §8 #3.

**Cakupan file:** `src/main.ts` (`makeTab`, `activateTab`, `showTabMenu`, `closeTab`, handler keyboard), `src/styles.css` (kind color, drop-zone), `index.html` (slot drop-zone di `#tabs`), smoke.

**Detail (§4.2 literal):** kind color — terminal icon `--muted`→`--fg` aktif, SFTP icon `--accent2`, editor dirty dot `--warn` 8px (tetap). `activateTab` → `scrollIntoView({inline:'nearest'})`. Tab disconnect: icon ✕ `--danger` + title "Session ended — click to reconnect", klik = spawn ulang PTY. Drop-zone split statik: area kanan tab bar berisi glyph ⫿ `--border-soft`; saat tab didrag (manual mousedown>6px — STATE.md #4) drop-zone menyala `--accent`. Context menu + "Close others" + "Close host". Ctrl+W close tab, Ctrl+Shift+W close others (title/tooltip semua shortcut baru).

**AC terukur:**
1. Tab SFTP icon computed color = `#818cf8` (`--accent2`); terminal aktif `#f2f2f2`.
2. Buka 12 tab (mock), activate tab pertama → `scrollLeft` > 0 setelah `scrollIntoView` (smoke).
3. Bunuh sesi (mock PTY exit) → tab icon ✕ `--danger` + title persis; klik membuka sesi baru (smoke dengan mock invoke).
4. Context menu memuat Close others; aksi menutup semua tab kecuali aktif; Close host menutup semua tab host itu.
5. Ctrl+W / Ctrl+Shift+W bekerja dan tidak mengintervensi input (hanya saat fokus di luar field teks).
6. Split-drag tetap manual mouse (`tabFromPoint` path); grep `draggable` = 0.
7. Verify loop hijau + smoke ≥ 4 cek baru.

---

### U4 — Terminal: well, divider split, status bar global

**Sumber:** redesign §1.3 (B1–B3), §4.3, §3 wireframe; PRD US-B1/UC1, NFR-6; fix §8 #6, #7.

**Cakupan file:** `index.html` (elemen `#status-bar` di `#main` setelah `#panels`), `src/main.ts` (`splitTerminal` divider drag + dblclick reset, updater status bar dari `sessions`/transfer state), `src/styles.css` (`.term-host` bg `--well`, `.split-divider`, `#status-bar`), smoke.

**Detail (§4.3 literal):** well bg `--well` #020203 padding 8 radius 0. Divider split **4px draggable**, bg `--border-soft`, hover/drag `--accent`, double-click reset 50/50 — drag pakai **manual mousedown/mousemove/mouseup** (STATE.md #4; dilarang HTML5 DnD). Sisi fokus: underline 2px `--accent` + glow `.term-side.focused`. Status bar 24px `--bar-h`, bg `--bg2`: kiri `user@host · auth:key|pw · N sesi`, kanan indikator transfer (spinner mini saat aktif) + versi app. Font size terminal: **tidak di v1** (§4.3 menandai v1.1 opsional — skip).

**AC terukur:**
1. `.term-host` computed background = `#020203`.
2. Split dua terminal: elemen divider 4px ada di antara; drag mouse (smoke: dispatch mousedown/mousemove/mouseup) mengubah rasio >40/<60; dblclick balik ke 50/50.
3. `#status-bar` selalu tampil; teks kiri format `user@host · auth:key · N sesi` untuk mock host; bertambah saat tab dibuka (smoke).
4. `.panel.split .term-host { float:left }` lama hilang — layout split kini flex + divider (grep `float: left` = 0).
5. Height chain utuh: `.sftp-wrap` dan `.ed-wrap` masih `height:100%` penuh setelah status bar memakan 24px `#main` (smoke: offsetHeight panel == panels container height).
6. `pasteNormalize` tidak tersentuh — diff berisi 0 baris di fungsi itu (review manual + cargo/smoke hijau).
7. Verify loop hijau + smoke ≥ 4 cek baru.

---

### U5 — SFTP: transfer strip, pane-head actions, selection/drop

**Sumber:** redesign §1.4 (F1–F5), §4.4, §3 wireframe SFTP; PRD US-C1/C2/C3, FR-C3 (manual mouse), NFR-7; fix §8 #8.

**Cakupan file:** `src/main.ts` (`openSftp` template, `renderSftp`, `renderLocalPane`, `rowFor`, `wirePane`, selection state), `src/styles.css` (chips, strip, row 28px), smoke. **Tidak menyentuh** backend `lib.rs`.

**Detail (§4.4 literal):** pane head 28px `--bg2` + slot chip: sumber `▲ sumber: {name}` (`--warn`), target `▼ target: …`. Row 28px (dari 32). Selection: bg `--sel` #0e2a20 **+ outline 1px `--accent` wajib** (§6.4.2 — bg sendiri FAIL non-text). Drop-target pane lawan: outline dashed 2px `--accent` saat drag manual. Tiga jalur transfer: (1) drag manual mouse existing `wirePane` — **dilarang HTML5 DnD**; (2) klik-klik VSCode-style; (3) tombol `Upload ▸`/`◂ Download` pindah ke **pane head target**. Keyboard: Space pilih, Tab pindah pane, Ctrl+Enter transfer. Transfer strip per-file menggantikan `.sftp-foot`+`.sftp-status` tunggal: `⇅ name → path · size · state` (berjalan spinner 12px / ok `--ok` + auto-fade 4s / gagal `--danger` + retry), max 3 baris + scroll. Hint statik 11px `--muted`: "Pilih file → klik pane tujuan / drag manual mouse — HTML5 DnD tidak dipakai". Loading = skeleton rows (kalau U6 belum jadi, pakai pola skeleton U2).

**AC terukur:**
1. Klik file → `.sftp-row.sel` punya `background: var(--sel)` **dan** `outline: 1px solid var(--accent)` (keduanya di computed style — smoke).
2. Tombol Upload/Download berada di `.pane-head` (bukan foot); foot lama tidak ada (grep `.sftp-foot` di main.ts/styles.css = 0 atau hanya strip).
3. Drag manual antar pane (smoke dispatch mousedown/mouseup + koordinat) → transfer terpanggil dengan path benar; grep `draggable=` = 0.
4. Ctrl+Enter mentransfer pilihan; Space memilih tanpa membuka editor.
5. Strip menampilkan baris per transfer; dua transfer berturut = 2 baris (tidak saling menimpa); gagal menampilkan retry (smoke mock fail).
6. Row `.sftp-row` tinggi 28px terukur.
7. Verify loop hijau + smoke ≥ 6 cek baru.

**Pitfall:** klik-klik tahap 1 (pilih sumber) tidak boleh memicu open-editor pada file remote — batasi ke jalur selection; copy empty/error naratif yang sudah ada TIDAK dirender ulang (hanya pindah rumah ke strip).

---

### U6 — Skeleton rows list + empty CTA (sisa #5)

**Sumber:** redesign §1.7 G2, §4.7 matriks; fix §8 #5 (renderSftp/renderLocalPane — bagian sidebar sudah di U2).

**Cakupan file:** `src/main.ts` (`renderSftp`, `renderLocalPane`), `src/styles.css` (`.skel-row` reuse dari U2), smoke.

**AC terukur:**
1. `renderSftp`/`renderLocalPane` loading = N elemen `.skel-row` statis (tanpa `animation` — pulse-free, §2 Arah A); tinggi list stabil selama loading (tanpa jump — smoke ukur offsetHeight sebelum/saat).
2. Empty SFTP tetap copy "Empty directory."; error tetap copy naratif + Go/Up actionable — byte-identical dengan sebelumnya (diff review: tidak ada perubahan string copy).
3. Verify loop hijau + smoke ≥ 2 cek baru.

---

### U7 — Editor: Ctrl+S panel-level + dirty

**Sumber:** redesign §1.5 (E1–E2), §4.5; PRD US-E1, FR-D1; fix §8 #2.

**Cakupan file:** `src/main.ts` (`openRemoteEditor` — keydown di panel element, dirty flag ke tab label + ed-bar), `src/styles.css` (`.ed-dirty`), smoke.

**Detail:** Ctrl+S terpasang di level panel editor (keydown pada elemen panel, bukan hanya `view.dom`). Dirty: teks "● unsaved" `--warn` di ed-bar (E2) + dirty dot tab existing tetap. Ed-status kanan (Ln/Col + Ctrl+G): **v1.1, skip** — hanya jika trivial (`codemirror` updateListener sudah ada); putuskan di review, bukan scope AC.

**AC terukur:**
1. Fokus di tombol Save/status lalu Ctrl+S → save terpanggil (smoke: mock invoke `write` path tercatat).
2. Edit isi → ed-bar menampilkan "unsaved" berwarna `--warn`; simpan → hilang; Reload saat dirty → dialog dirty existing tetap muncul.
3. Verify loop hijau + smoke ≥ 2 cek baru.

---

### U8 — Dialog password host (D2/D3)

**Sumber:** redesign §4.6 + mockup `dialog-password-host.html`; PRD US-D1/D2, NFR-1; §1.6 D2/D3 (#1 sudah ship).

**Cakupan file:** `index.html` (`#modal` field password area), `src/main.ts` (`applyAuthMode`, `openEditor`, tombol hapus password → reuse path forget sidecar existing), `src/styles.css` (badge chip), smoke.

**Detail (§4.6 literal):** auth=password tersimpan → chip `● stored (sidecar, 0600)` warna `--ok` di samping label (D2; placeholder "kept — type to replace" tetap); tombol kecil "Hapus password tersimpan" (`--danger`, konfirmasi via dialog in-app — jangan native confirm). Note tambahan satu baris: "Auto-typing hanya aktif untuk host ini (opt-in per host)." (D3, PRD Q3). Note existing sidecar 0600 tetap (sudah benar).

**AC terukur:**
1. Host dengan has_password=true → chip `● stored (sidecar, 0600)` terlihat di `#modal` (smoke); has_password=false → chip absen.
2. "Hapus password tersimpan" → dialog konfirmasi → sidecar entry hilang (mock invoke terpanggil), chip hilang, note kembali versi kosong.
3. Note D3 tampil persis saat auth=password.
4. Tidak ada secret baru di `~/.ssh/config` path (NFR-1 — copy hanya menyebut sidecar; grep `hosterm.secrets` di copy = benar).
5. Verify loop hijau + smoke ≥ 3 cek baru.

---

### U9 — Smoke suite kanonisisasi + audit regressi (fix §8 #9)

**Sumber:** §8 #9; AGENTS.md verify loop #4; PRD NFR-5/AC-2.

**Cakupan file:** `qa-reports/headless-smoke.mjs` saja.

**Detail:** pastikan setiap fitur baru punya cek eksplisit (checklist §8 #9: context menu host, divider split, status bar, transfer strip — sudah tersebar U2/U4/U5; unit ini mengaudit kelengkapan + rapikan duplikasi). Tambah cek lintas-fitur: (a) seluruh permukaan tanpa `draggable` attr; (b) copy naratif empty/error sidebar & SFTP byte-identical baseline; (c) `pasteNormalize` unit-check tetap; (d) glow hanya 3 selector (bila U1 smoke belum cover).

**AC terukur:**
1. Semua cek baru dari U1–U8 tercatat PASS; total checks ≥ baseline + 25.
2. Grep `draggable` di `src/` = 0 hit (smoke assert).
3. Verify loop hijau penuh; hasil smoke dilampirkan ke task.

---

## 3. Risiko & pitfall (wajib di tiap task body)

1. **DILARANG HTML5 DnD** (STATE.md #4, PRD FR-C3): semua drag — tab split, SFTP antar-pane, divider — pakai manual mousedown/mousemove/mouseup. Grep `draggable` harus 0.
2. **pasteNormalize jangan rusak** (STATE.md #3, PRD FR-B3): fungsi + wire-nya off-limits kecuali U4 memastikan 0 diff di fungsi.
3. **Height chain**: `.panel` absolute inset 0; turunannya `.sftp-wrap`/`.ed-wrap` butuh `height:100%` eksplisit (STATE.md layout chains). Penambahan status bar (U4) mengubah tinggi `#panels` — pastikan chain tidak pecah; smoke ukur offsetHeight.
4. **Copy empty/error naratif tidak dirender ulang** (DESIGN.md S-12/S-13 house style): pindah rumah boleh, ubah kata tidak.
5. **Focus & keyboard**: hover-reveal tetap harus reachable keyboard (R-32); shortcut baru semua ada di `title`.
6. **Offline font**: tanpa CDN/webfont runtime (§6.5 aturan) — NFR-3 no network calls.
7. **Prefers-reduced-motion**: spinner/xfer-bar exception existing; glow = box-shadow statis (bukan animation) jadi aman, jangan tambah keyframes baru.
8. **Jangan sentuh** working-tree signing (§0), `src-tauri/src/lib.rs`, config parser, native dialog path.
9. **Baseline bergeser**: ukur ulang cargo/smoke saat dispatch unit pertama; tulis angka aktual di task body.

## 4. Rollout & rollback

- Urutan merge: U1 dulu (fondasi visual; semua unit lain menumpuk token-nya). Tiap unit = 1 commit terpisah oleh lead (triple gate dulu), sehingga revert per-unit bisa tanpa menyentuh lainnya.
- Rollback per unit: `git revert <commit>` — tidak ada migrasi data/schema (frontend-only). Risiko terbesar U1 (tema menyentuh xterm/CM literal) — kalau regresi kontras, fallback = revert ke palet lama 1 commit.
- Tidak ada perubahan perilaku backend; `cargo test --lib` wajib tetap baseline di setiap unit sebagai kanari.

## 5. Test plan (ringkas)

Per unit: verify loop AGENTS.md penuh (cargo ≥ baseline / build 0 err / smoke baseline+N cek baru) + cek spesifik di AC unit. U9 = audit menyeluruh. Manual Windows (WebView2) hanya untuk U4 divider + U5 drag sebelum rilis berikutnya (interaksi mouse halus tidak ter-cover Playwright sepenuhnya).

## 6. Out of scope (v1)

Command palette (Ctrl+P, wave-2 §2), font size terminal (v1.1 §4.3), go-to-line editor (v1.1 §4.5), kolom permission/mtime SFTP (F6), grouping selain prefix `-`, `lib.rs` apa pun.
