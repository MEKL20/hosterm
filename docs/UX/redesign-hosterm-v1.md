# hosterm — Redesign UX v1 (arah + mockup + spec)

| Field | Value |
|---|---|
| Author | crew-coding-uiux |
| Status | For MEKL review — design deliverable only, no code |
| Input | `docs/PRD-hosterm-v1.md`, `docs/STATE.md`, `DESIGN.md`, `src/styles.css`, `src/main.ts`, root `index.html`, `docs/screenshot-main.png` + `docs/screenshot-edit.png` (2240×1440) |
| Scope | docs/UX/ only. Tidak menyentuh `src/`, tidak mengubah PRD/STATE/DESIGN.md |
| Mockup | `docs/UX/mockups/main.html`, `editor.html`, `dialog-password-host.html` (statis, self-contained, buka di browser) |

Metode note (jujur): worker ini tidak punya vision — dua screenshot tidak bisa dibaca sebagai piksel. Analisis "tampilan sekarang" di bawah diturunkan dari kode aktual: `index.html` (markup), `styles.css` (semua surface), `main.ts` (perilaku, states, copy). Struktur visual lengkap, posisi, dan copy persis seperti yang ship.

---

## 1. Analisis UX tampilan sekarang

Basis: Tokyo Night, chrome sans + mono untuk konten fungsional, ENERGY 1 / RHYTHM 2 / MOTION 1 (DESIGN.md). Fondasi sudah sehat — fokus analisis pada masalah konkret, bukan re-skin.

### 1.1 Sidebar host
| # | Masalah | Bukti | Dampak |
|---|---|---|---|
| S1 | Baris host 2 lini (~52px) + 3 tombol aksi (SFTP/Edit/Delete) selalu terlihat per baris | `.host-item`, `.h-acts` | Density rendah untuk 20+ host; ikon trash permanen = risiko klik salah; noise visual 3×N ikon |
| S2 | Tidak ada indikasi host mana yang punya sesi/tab terbuka | `renderHostList()` tidak sadar tab | Power user tidak bisa lihat topology sesi dari sidebar; buka dua terminal ke host sama = tidak terlihat |
| S3 | Tidak ada grouping/pinning saat host banyak | flat list | UC "handful-to-dozens" — di 30 host, scroll datar tidak bisa discan cepat |
| S4 | Aksi ikon 22×22px, tanpa jarak aman dari klik-baris | `.h-acts button` 22px | Kesalahan target mudah (klik dekat edge = buka terminal) |
| S5 | Search bagus, tapi tidak pernah dapat fokus dari keyboard | tidak ada shortcut | Keyboard-first user harus mouse dulu |

### 1.2 Tab bar
| # | Masalah | Bukti | Dampak |
|---|---|---|---|
| T1 | Affordance split terminal (drag tab ke tab) sepenuhnya tersembunyi | `tabDrag` manual mousedown→split; tidak ada hint visual/statik | Fitur UC1 inti (side-by-side) nyaris tak ter-discover; hanya yang baca kode tahu |
| T2 | Tab SFTP dan editor tidak punya pembeda warna kind; ikon sama-sama muted | `.t-ico` | Scan 10 tab lambat; dirty dot hanya ada di editor |
| T3 | Tidak ada indikator sesi terputus di tab (disconnect baru terasa saat user mengetik) | tidak ada state | Terminal mati terlihat identik dengan terminal sibuk |
| T4 | Overflow hanya scroll horizontal | `#tabs` overflow-x | Dengan 15+ tab, tab aktif bisa keluar viewport tanpa auto-scroll ke view |
| T5 | Tidak ada close-others / close-host dari konteks | `#tab-menu` (Split/Duplicate/Close saja) | Rutinitas rapikan workspace butuh klik per tab |

### 1.3 Terminal
| # | Masalah | Bukti | Dampak |
|---|---|---|---|
| B1 | Split dua terminal pakai `float: left` 50% — tanpa divider, tanpa resize, tanpa fokus visual pemisah | `.panel.split .term-host` | Dua layar menempel jadi satu massa; tidak bisa atur 70/30 |
| B2 | Tidak ada status bar sesi (host, user@host, auth mode, uptime sesi) | — | Konteks hilang setelah banner login terlewat scroll |
| B3 | Font size terminal tidak bisa diatur (fixed 13px) | xterm option | Terminal = pusat layar; power user punya preferensi density sendiri |
| B4 | pasteNormalize bekerja senyap (benar) tapi tidak ada feedback sekali-jalan | `pasteNormalize` | User tidak pernah tahu CRLF dibersihkan — fitur keamanan tak terlihat (minor) |

### 1.4 SFTP browser
| # | Masalah | Bukti | Dampak |
|---|---|---|---|
| F1 | Dua pane selalu 50/50 walau tugas umum "browse remote saja" hanya butuh remote | `.sftp-panes` flex:1 keduanya | Terminal/file list kehilangan separuh lebar permanen |
| F2 | Pola transfer selection-based sudah ada, tapi affordance lemah: status "Selected X for upload" hanya di status line bawah, sekilas hilang | `__selLocal/__selRemote`, `.sftp-status` | User tidak paham kenapa tombol Upload/Download tidak jalan (error text menuntun, tapi ini reactive, bukan preventif) |
| F3 | Tombol Upload/Download selection di footer bawah, jauh dari kedua pane | `.sftp-foot` | Jarak gerak mouse besar; label "selection" abstrak |
| F4 | Manual drag (mousedown→mouseup antar pane) bekerja tapi tanpa affordance statik: tidak ada hint "drag di sini" / drop hint hanya saat hover | `wirePane` | Constraint STATE.md #4 dipenuhi di kode, tapi user tidak tahu drag itu ada; click-click VSCode-style juga tidak dijelaskan |
| F5 | Transfer hanya indeterminate bar; tanpa queue, tanpa byte/ETA, satu status line untuk semua | `.xfer-bar` | Multi-file = antrean tak terlihat; gagal/berhasil saling menimpa pesan |
| F6 | Tanpa kolom permission/mtime; ukuran saja | `rowFor()` | Untuk audit config server, mtime sering dibutuhkan (nice-to-have, bukan blocker) |
| F7 | Loading mengganti seluruh isi list dengan satu baris spinner → layout jump | `list.innerHTML = spinner` | Kesan "kedip" tiap navigasi direktori |

### 1.5 Remote editor
| # | Masalah | Bukti | Dampak |
|---|---|---|---|
| E1 | Ctrl+S hanya terpasang pada keydown di dalam `.cm-editor` | `view.dom.addEventListener("keydown")` | Fokus di tombol Save/status → Ctrl+S mati (inkonsisten) |
| E2 | Dirty state hanya titik kecil di tab; ed-bar tidak menunjukkan dirty/unsaved | `.t-dot` | Detak kedua terlewat; user tekan Reload dan buang editan |
| E3 | Tidak ada go-to-line, encoding/EOL info di ed-status | — | Editing nginx/sshd config sering butuh nomor baris presisi |

### 1.6 Dialog password host
| # | Masalah | Bukti | Dampak |
|---|---|---|---|
| D1 | **Copy salah & kontradiktif dengan STATE.md**: `applyAuthMode` menulis "The password is stored in **~/.ssh/config** (file perms 0600)" — yang benar sidecar `~/.ssh/hosterm.secrets` (0600); STATE.md invariant #1: tidak ada secret di config | `main.ts:325` | Copy menakut-nakuti user sekaligus salah faktual; menyiratkan pelanggaran invariant padahal tidak |
| D2 | Placeholder "stored in ~/.ssh/config" untuk state tersimpan tidak dibedakan visual dari field kosong | `pw.placeholder` | User ragu: apakah password tersimpan? |
| D3 | Konsekuensi "Stored password auto-typed at connect" tidak dijelaskan di dialog | note hanya soal storage | Opt-in per host (PRD Q3) tapi trade-off-nya tidak tampil |

### 1.7 Global
| # | Masalah | Bukti | Dampak |
|---|---|---|---|
| G1 | Tidak ada status bar global — konteks (host aktif, jumlah sesi, aktivitas transfer) tersebar | — | Idle screen = sidebar + tab kosong; info density rendah |
| G2 | Loading pattern "replace innerHTML" di semua list (host, sftp, local) → layout jump seragam | `loadHosts`, `renderSftp`, `renderLocalPane` | Terasa murah; NFR-6 (≤200ms feedback) terasa dilanggar walau cepat |
| G3 | Aksesibilitas sudah baik (aria-label, focus ring, overlay trap) — pertahankan; titik lemah: `.list-state` bukan fokus target (oke, informatif), icon 22px di bawah 24px minimum nyaman desktop | — | Minor |

Yang sudah benar dan wajib dipertahankan: copy empty/error/loading bernarasi (house style DESIGN.md S-13), overlay focus-trap + Escape + restore focus, `:focus-visible` global, indeterminate xfer-bar dengan `prefers-reduced-motion`, tab keyboard ←/→.

---

## 2. Tiga arah desain

### Arah A — "Arus" (The Current)
**Prinsip.** DESIGN.md sudah menyatakan motif identitas: *accent adalah arus yang mengalir lewat chrome, terminal, editor*. Arah ini menuntaskan motif itu sampai interaksi, bukan hanya warna: (1) terminal = pusat, chrome = tiga surface datar; (2) density naik satu tingkat (single-line host rows, grouping, status bar); (3) setiap state penting (sesi hidup, dirty, transfer) punya sinyal permanen di chrome — tidak ada state yang hanya hidup di status line sekilas.

**Yang berubah.**
- Sidebar: single-line host row (28px) + subtitle jadi tooltip, group header (Produksi/Lab dsb. dari prefix alias atau folder konfiguratif — v1: grouping manual by prefix `-`), dot status sesi per host, 3 aksi row jadi 1 (klik = terminal; SFTP via klik-kanan context menu + tombol ⇅ muncul saat hover; edit/delete hanya di context menu). Search dapat Ctrl+K.
- Tab bar: kode warna kind (terminal = default, SFTP = `--accent2`, editor dirty = `--warn`), auto-scroll tab aktif ke view, menu + "Close others", hint permanen split: garis drop-zone tipis di tengah tab bar yang menyala saat drag (affordance untuk T1).
- Terminal: well lebih dalam (`--well` #101017), divider split nyata 4px draggable + double-click reset 50/50, status bar bawah global (baru): `user@host · auth:key · 3 sesi · transfer ●`.
- SFTP: pane head jadi zona transfer eksplisit — row terpilih menempel chip "▲ sumber upload" di pane asal + pane target dapat outline drop; tombol Upload/Download pindah ke pane head target (dekat titik keputusan); klik-klik (VSCode style) dan drag manual (mousedown/mouseup, bukan HTML5 DnD) keduanya didukung + hint statik satu baris di foot; strip transfer per-file menggantikan satu status line.
- Editor: dirty muncul di ed-bar (teks "unsaved" warna `--warn`), Ctrl+S global di panel editor (bukan hanya cm-editor), go-to-line `Ctrl+G` input kecil di ed-status.
- Empty/loading/error: skeleton baris (3 baris `--hover` pulse-free, statis) menggantikan spinner-replace → tidak ada layout jump; copy naratif tetap.

**Trade-off.** Evolusi, bukan revolusi: aplikasi tetap dikenali (konsisten dengan baseline DESIGN.md), semua perubahanCSS/struktur DOM ringan di kode yang sudah ada. Kekurangan: tidak ada "wow" baru; identitas visual tidak berubah.

### Arah B — "Konsol" (Tiling)
**Prinsip.** Aplikasi adalah window manager mini: semua chrome mono, grid hairline, navigasi primer via command palette (Ctrl+P "fuzzy: host/file/action"), tab opsional (buffer list seperti Vim).
**Yang berubah.** Semua chrome monospace; sidebar jadi buffer list; layout tiling (v/split, resize keyboard); palette sebagai gerbang semua aksi.
**Trade-off.** Daya tarik kuat untuk power user ekstrem; tapi: host form & dialog perlu gaya kedua, mouse-first workflow jadi warga kelas dua, biaya implementasi besar (palette = komponen baru dengan state sendiri), dan melanggar semangat DESIGN.md ("chrome stays sans", ENERGY 1). Risiko tinggi untuk solo maintainer.

### Arah C — "Kokpit"
**Prinsip.** Sidebar menjadi navigator workspace: host card dengan metrik (uptime, last connect, SFTP shortcut, quick actions), SFTP dengan preview kolom, dashboard per sesi.
**Yang berubah.** Host rows jadi kartu multi-lini kaya info; panel ringkasan sesi; SFTP dapat kolom perm/mtime + preview.
**Trade-off.** Info banyak tapi dekorasi kosong mengintai (kartu menghabiskan ruang tanpa menambah aksi); terminal — pusat layar — justru tergerus; mendorong fitur monitoring yang PRD taruh di luar scope v1. Bertentangan langsung dengan constraint "tanpa dekorasi kosong".

### Rekomendasi: **Arah A — "Arus"**
Alasan:
1. **Konsisten baseline DESIGN.md** (keputusan R-31: evolusi Tokyo Night, tetap aplikasi yang sama) — A adalah penuntasan motif, bukan fork.
2. **Terminal tetap pusat** — A menambah ruang terminal (well dalam, divider, status bar global), B/C menggerus.
3. **Density untuk power user** tanpa dekorasi: single-line rows + grouping + status bar = informasi naik, piksel hias nol.
4. **Biaya implementasi realistis** untuk solo maintainer: mayoritas perubahan CSS + sedikit struktur DOM; tidak ada komponen arsitektural baru (palette, dashboard).
5. B gagal pada "chrome stays sans" + biaya; C gagal pada "tanpa dekorasi kosong" + terminal-first.

Sisa arah B yang layak dicuri ke A: command palette bisa jadi wave-2 (tertatap di §8), bukan bagian v1.

---

## 3. Layout target (Arah A) — wireframe

```
┌──────────┬──────────────────────────────────────────────────────────────┐
│ hosterm ⓘ│ [⌘ prod-web×] [⌘ db-core×] [⇅ prod-web] [✎ nginx.conf ●]     │ tab bar 36px
├──────────┼──────────────────────────────────────────────────────────────┤
│⌕ Ctrl+K  │┌────────────────────────┬────────────────────────┐            │
│ PRODUKSI ││ term 1 (well)          │ term 2 (well)          │            │
│ ● prod-web││ $ tail -f access.log  │ $ htop                 │            │
│ ● db-core││                        │                        │            │
│ LAB      ││                        ├────divider 4px─────────┤            │
│ ● jumphost││                        │        (drag resize)   │            │
│ + Add    │└────────────────────────┴────────────────────────┘            │
│──────────├──────────────────────────────────────────────────────────────┤
│ Raw cfg  │ mekl@prod-web · auth: key · 3 sesi · ⇅ idle          v0.8.1  │ status bar 24px
└──────────┴──────────────────────────────────────────────────────────────┘
```

SFTP view (tab ⇅ aktif) — pola "sumber → target" eksplisit:

```
┌ local (This PC) ──────────────┬─ remote (prod-web) ──────────┐
│ …/deploy                      │ /var/www                     │
│ ▸ build.tar.gz  [▲ sumber]    │ ▸ releases/                  │
│   notes.md                    │ ▸ current -> releases/7      │
│   (baris terpilih: --sel +    │   [▼ target upload — drop    │
│    outline --accent)          │    di sini / klik-klik]      │
├───────────────────────────────┴──────────────────────────────┤
│ ⇅ build.tar.gz → /var/www · 12.4 MB · berjalan               │ strip transfer
│ Space = pilih · Enter transfer · drag manual mouse = oke      │ hint statik
└──────────────────────────────────────────────────────────────┘
```

---

## 4. Inventory komponen (spec per komponen)

Semua ukuran/warna mengikuti §6 tokens; nilai contrast yang dikutip diukur (§6.2), bukan estimasi. Nilai tanpa catatan = diwarisi DESIGN.md (sudah terukur di sana).

### 4.1 Sidebar
| Bagian | Spec |
|---|---|
| Head | brand + add (26×26 icon) + collapse. Tetap. |
| Search | input 12px, **shortcut Ctrl+K memfokuskan**; placeholder "Search hosts (Ctrl+K)…" |
| Group header | 11px uppercase `--muted`, padding 8px, garis `--border-soft` di atas; label = prefix alias sebelum `-` (mis. `prod-web` → PROD). `Host *` wildcard tetap tidak dirender sebagai host (FR-A1) |
| Host row | **satu lini, 28px**: dot status 6px (hijau `--ok` = ada tab terminal hidup; abu `--muted` = idle) + nama ellipsis + (hover) tombol ⇅ SFTP 24×24. Subtitle `user@host:port` jadi `title` tooltip. Auth password = ikon kunci kecil `--warn` di ujung (has_password). Klik = terminal (US-A1). Enter/Space sama. Konteks (klik kanan / tombol ⋯): Open SFTP · Edit · Duplicate? · Delete |
| Destructive | Edit/Delete **keluar dari row** (context menu saja) — permukaan klik salah hilang (S1/S4) |
| Foot | Raw config + Reload tetap |
| Empty | copy tetap ("No hosts in ~/.ssh/config yet. Click + to add one.") + CTA button "Add host" inline |
| Error | copy tetap, warna `--danger` (6.46:1 on bg — PASS) |
| Loading | skeleton 3 baris statis `--hover` radius 4 (G2) |

### 4.2 Tab bar
| Bagian | Spec |
|---|---|
| Tab | 36px, icon 16 + label ellipsis + close. Aktif: bg `--bg`, underline 2px `--accent` (tetap) |
| Kind color | terminal icon `--muted`→`--fg` aktif; SFTP icon `--accent2` (11.86:1 on bg2 — PASS); editor icon `--muted`, **dirty dot `--warn` 8px** tetap |
| Active auto-scroll | saat activate, `scrollIntoView({inline:'nearest'})` (T4) |
| Split affordance | **drop-zone tetap terlihat**: area kanan tab bar (setelah tab terakhir) berisi ikon ⫿ split samar `--border-soft`; saat tab didrag manual, seluruh tab jadi drop-target dashed `--accent` seperti sekarang + drop-zone menyala `--accent`. Drag tetap manual mousedown>6px (STATE.md #4) |
| Context menu | + Close others · Close host (semua tab host itu). Item existing (Split/Duplicate/Close) tetap |
| Disconnect | tab terminal sesi mati: icon berubah ✕ `--danger` + title "Session ended — click to reconnect"; klik = spawn ulang PTY |

### 4.3 Terminal
| Bagian | Spec |
|---|---|
| Well | bg `--well` #101017 (fg 12.71:1 — PASS), padding 8px, radius 0 |
| Split | divider **4px draggable** bg `--border-soft`, hover/drag `--accent`; double-click reset 50/50; masing-masing sisi punya xterm sendiri (existing splitSessions) |
| Focus | sisi split yang fokus: underline 2px `--accent` di atas divider sisi itu (arus motif) |
| Status bar (global, baru) | 24px, bg `--bg2`, text 12px: kiri `user@host · auth:key|pw · N sesi`; kanan indikator transfer (spinner mini saat aktif) + versi app. Selalu tampil (G1) |
| Paste | pasteNormalize tetap; tanpa UI tambahan v1 (B4 diterima sebagai senyap-by-design; dokumentasi README) |
| Font size | **v1.1 (opsional)**: Ctrl+= / Ctrl+- persist localStorage; tidak di v1 agar diff kecil |

### 4.4 SFTP panel
| Bagian | Spec |
|---|---|
| Pane head | 28px bg `--bg2`: judul ("This PC — local" / `host — remote`) + **slot chip sumber/target** |
| Toolbar | up · path (mono, Enter=Go) · Go · refresh — tetap (DESIGN.md S-14) |
| Row | 28px (dari 32px), icon 16 (dir `--accent` 7.14:1, file `--muted`), nama ellipsis, size kanan mono 12px, direktori = sel kosong (L-8 tetap) |
| Selection (sumber) | klik file = pilih: bg `--sel` #2a3050 **+ outline 1px `--accent`** (outline wajib: bg saja 1.40:1 vs bg2 — FAIL non-text) + chip di pane head asal: `▲ sumber: build.tar.gz` (icon `--warn`) |
| Drop-target | pane lawan saat drag manual (atau setelah klik-klik tahap 1): outline dashed 2px `--accent` + chip di pane head target: `▼ target: drop di sini / klik file lalu Upload` |
| Tiga jalur transfer | (1) drag manual mouse antar pane (existing wirePane, bukan HTML5 DnD); (2) klik-klik VSCode-style: klik sumber → klik file/dir target → transfer; (3) tombol `Upload ▸` / `◂ Download` di **pane head target** (pindah dari foot — F3). Keyboard: Space pilih · Tab ke pane lawan · Ctrl+Enter transfer |
| Transfer strip | menggantikan `.sftp-foot`+`.sftp-status`: daftar transfer aktif/per-file 1 baris (`⇅ name → path · size · state`), state: berjalan (spinner 12px) / ok `--ok` / gagal `--danger` + tombol retry. Max 3 baris + scroll (F5) |
| Hint statik | 11px `--muted` satu baris di bawah strip: "Pilih file → klik pane tujuan / drag manual mouse — HTML5 DnD tidak dipakai" (F4) |
| Loading | skeleton rows statis (F7); error copy naratif tetap |

### 4.5 Editor
| Bagian | Spec |
|---|---|
| Ed-bar | path mono 12px + **dirty text "● unsaved" `--warn`** (E2) + Reload + Save(primary; disabled sampai load — tetap) |
| Ctrl+S | dipasang di level panel editor (keydown panel), bukan hanya cm-editor (E1) |
| Ed-status | kiri: `N lines · host:path`; kanan (v1.1): Ln x, Col y · Ctrl+G go-to-line |
| Gutter/gaya | CodeMirror theme existing tetap (sudah on-token) |

### 4.6 Dialog password host (mockup: `dialog-password-host.html`)
| Bagian | Spec |
|---|---|
| Struktur | modal 380px radius 10 bg `--bg` shadow elevation (tetap); field: Alias, HostName, User, Port, Auth (select), kondisional key/password |
| Auth=password, kosong | note: "Password disimpan di sidecar `~/.ssh/hosterm.secrets` (perm 0600) — **tidak pernah** di `~/.ssh/config`. Diisi otomatis sekali saat connect." **Perbaikan copy D1** |
| Auth=password, tersimpan | input placeholder "kept — type to replace" (tetap) + badge chip `● stored (sidecar, 0600)` warna `--ok` di samping label (D2) + link-ish tombol kecil "Hapus password tersimpan" (`--danger`, konfirmasi dialog) |
| Konsekuensi | satu baris note tambahan: "Auto-typing hanya aktif untuk host ini (opt-in per host)." (D3) |
| Tombol | Save (primary) · Cancel · Delete (danger outline, hanya edit-mode). Urutan & verb-first tetap (DESIGN.md S-13) |
| Error | `.err` di atas actions, copy naratif tetap |

### 4.7 Empty / loading / error / success (matriks)
| State | Sidebar | SFTP pane | Editor | Terminal |
|---|---|---|---|---|
| Empty | copy + CTA "Add host" | "Empty directory." | — (tidak dibuka kosong) | — |
| Loading | skeleton 3 baris statis | skeleton rows statis | "Opening X…" status line (tetap) | banner connect (tetap) |
| Error | copy naratif `--danger` + tombol Reload inline | copy naratif + Go/Up actionable | status `--danger` + tombol Reload tetap aktif | tab icon ✕ `--danger` + reconnect |
| Success | — | strip transfer ok `--ok` + auto-fade 4s | "Saved HH:MM:SS" `--ok` (tetap) | — |

---

## 5. Keyboard map (proposal v1 — keyboard-first)
| Shortcut | Aksi |
|---|---|
| Ctrl+K | fokus search host |
| ↑/↓ + Enter | navigasi host list → buka terminal (existing row tabindex, tinggal dirapikan) |
| Ctrl+B | collapse/expand sidebar |
| Ctrl+W / Ctrl+Shift+W | close tab / close others (tab aktif) |
| ←/→ (pada tab) | pindah tab (existing — pertahankan) |
| Space / Ctrl+Enter | pilih file / transfer (SFTP, sesuai §4.4) |
| Ctrl+S | save editor (level panel, E1) |
| Esc | tutup overlay/menu (existing) |

Semua shortcut baru harus muncul di title/tooltip agar discoverable. Wave-2: Ctrl+P command palette (curian dari Arah B).

## 6. Design tokens (dark-first — memperluas DESIGN.md, tidak mem-fork)

### 6.1 Tambahan token
```css
:root {
  /* wells: permukaan konten primer, lebih dalam dari chrome */
  --well:      #101017; /* terminal + editor well; --fg di atasnya 12.71:1 */

  /* selection (SFTP/source) */
  --sel:       #2a3050; /* SELALU berpasangan dgn outline 1px --accent (bg sendiri 1.40:1 vs bg2) */

  /* status bar */
  --bar-h:     24px;
}
```
Palet inti, type scale, spacing (4/8/12/16/24), radius (4/6/10), motion 120ms: **identik DESIGN.md §2–3**. Tidak ada warna baru di luar daftar itu + `--well`/`--sel`.

### 6.2 Pengukuran kontras baru (diukur skrip WCAG rel-luminance, bukan estimasi)
| Pasangan | Rasio | Batas | Hasil |
|---|---|---|---|
| `--fg` #c8d3f5 on `--well` #101017 | **12.71:1** | 4.5 | PASS |
| `--muted` #8b98c9 on `--well` | **6.58:1** | 4.5 | PASS |
| `--accent` #7aa2f7 on `--well` | **7.39:1** | 4.5 | PASS |
| `--fg` on `--sel` #2a3050 | **8.61:1** | 4.5 | PASS |
| `--sel` vs `--bg2` (non-text) | **1.40:1** | 3.0 | **FAIL → wajib outline `--accent`** (accent on bg2 7.14:1 PASS) |
| `--ok` #9ece6a on `--bg2` (dot sesi) | **9.84:1** | 3.0 | PASS |
| `--warn` #e0af68 on `--bg2` (dirty/🔑) | **8.99:1** | 3.0 | PASS |
| `--danger` #f7768e on `--bg2` (disconnect ✕) | **6.80:1** | 3.0 | PASS |
| `--accent2` #89ddff on `--bg2` (SFTP tab icon) | **11.86:1** | 3.0 | PASS |
| `--warn` on `--hover` (chip sumber) | **7.28:1** | 4.5 | PASS |

`--well` vs `--bg` (1.11:1) **disengaja**: well adalah permukaan konten, bukan boundary komponen interaktif — WCAG 1.4.11 tidak menuntut pemisahan antar permukaan pasif; dipertahankan agar terminal terlihat "lebih hitam" dari chrome (motif current).

### 6.3 Z-index scale (dokumentasi nilai existing)
panel 0 · xfer/status sticky auto · overlay/modal 50 · tab context menu 1000.

### 6.4 Aturan tegas tokens
1. `--well` hanya untuk terminal & editor content area. Chrome dilarang pakai well.
2. `--sel` tidak pernah tanpa outline `--accent` (aturan FAIL di 6.2).
3. Dot status 6px memakai token status apa adanya; tidak ada warna dot baru.
4. Split divider: bg `--border-soft`, interaksi `--accent`. Bukan warna lain.

## 7. Mockup map
| File | Isi |
|---|---|
| `mockups/main.html` | Frame 1: main view — sidebar grouping + single-line rows, tab bar (2 terminal split aktif + tab SFTP + editor dirty), terminal split dengan divider, status bar. Frame 2: SFTP view aktif — dua pane, chip sumber/target, drop-target, transfer strip, hint manual-drag |
| `mockups/editor.html` | Editor view: ed-bar + dirty unsaved, gutter+syntax ala CodeMirror, ed-status, status bar |
| `mockups/styles.css` | tokens + komponen bersama (dark-first, semua §6) |
| `mockups/dialog-password-host.html` | Dua varian dialog host: password kosong & password tersimpan (badge sidecar) + state key |
| `mockups/b-konsol.html` | *(addendum, dibuat atas permintaan MEKL untuk perbandingan)* Arah B "Konsol": chrome mono, buffer list, tiling hairline, command palette — mockup pembanding, bukan arah rekomendasi |
| `mockups/c-kokpit.html` | *(addendum, dibuat atas permintaan MEKL untuk perbandingan)* Arah C "Kokpit": host card bermetrik, dashboard sesi, SFTP kolom perm/mtime + preview — mockup pembanding, bukan arah rekomendasi |

Affordance manual drag (STATE.md #4) dieksplisitkan di mockup SFTP: chip sumber/target, outline dashed drop-target, hint statik "drag manual mouse — HTML5 DnD tidak dipakai", tombol transfer di pane head, dan label shortcut Space/Ctrl+Enter. Tidak ada `draggable=` di mana pun.

## 8. Fix list untuk implementasi (frontend, terpisah — bukan task ini)
| # | Fix | Mengapa | Di mana |
|---|---|---|---|
| 1 | Ganti copy `applyAuthMode`: "stored in ~/.ssh/config" → "stored in `~/.ssh/hosterm.secrets` (0600)" | Salah faktual, menyiratkan pelanggaran invariant STATE #1 | `main.ts` note password |
| 2 | Ctrl+S di level panel editor | E1 | `main.ts` openRemoteEditor |
| 3 | Tab aktif `scrollIntoView` | T4 | `main.ts` activateTab |
| 4 | `.h-acts` → context menu + tombol ⇅ hover-only | S1/S4 | index.html + main.ts |
| 5 | Skeleton rows menggantikan spinner-replace | G2/F7 | loadHosts/renderSftp/renderLocalPane |
| 6 | Divider split terminal draggable | B1 | CSS + sedikit JS mouse |
| 7 | Status bar global | G1 | index.html + main.ts |
| 8 | Transfer strip per-file + tombol di pane head + hint statik | F2/F3/F4/F5 | openSftp panel template |
| 9 | Smoke suite: tambahkan cek untuk context menu host, divider, status bar, transfer strip | Verify loop AGENTS.md #4 | `qa-reports/headless-smoke.mjs` |

Dilarang dalam implementasi nanti: HTML5 DnD (STATE.md #4), merusak pasteNormalize, merender ulang copy empty/error yang sudah naratif.
