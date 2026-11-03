# FEATURE-SPEC: Redesign Dusk v1

> Milik epic produk — sitasi `PRD.md §7` (row ditambahkan pm). EARS untuk semua behavior.
> Visual = APPROVED prototypes (MEKL), sumber kebenaran — **cite, jangan reka ulang**:
> `docs/UX/prototypes/proto-a-dusk.html` (shell) · `docs/UX/prototypes/proto-a-dusk-settings.html`
> (Settings/Appearance) · `docs/UX/prototypes/README.md` + `verification-report.json` (3 arah, A dipilih).
> Boss intent (MEKL via nano): arah visual BARU "Dusk" menggantikan "Abyss Emerald / Arus" (rilis v0.11.0);
> + 3 fitur fungsional baru (Google login, Snippet panel, AI + approval) + Settings/theme.
> Keputusan MEKL yang membatasi scope (preset + 1 accent custom; TANPA token editor / import-export /
> light-dark toggle di luar preset) diperlakukan sebagai constraint, bukan bahan debat.

| Field | Value |
|---|---|
| Author | crew-coding-pm |
| Status | Speced — awaiting MEKL approval. No code, no commits (PM = doc only). |
| Baseline | v0.11.0 (tag 4be09c5, Arus U1–U9 shipped, unsigned) + Fleet FL-1 (801af16) + `forget_secret` (U8 D2). Verify loop STATE.md: cargo 77 / smoke 196 (re-measure at dispatch) |
| Supersedes (visual only) | Tema **Abyss Emerald / Arus** (redesign-hosterm-v1 §6.5) sebagai identitas visual aktif. Dokumen Arus historis TIDAK diubah — Dusk berdiri sendiri; PRD §7 merujuk epic ini |
| Non-negotiables | spawn OS **real `ssh`/`sftp`** (SDD ADR-1) · `~/.ssh/config` + sidecar invariants (STATE.md §Hard invariants 1–6) utuh · drag = manual mouse (BUKAN HTML5 DnD) · paste PTY normalize CRLF→LF · komponen shell (sidebar/tab/split/status bar/SFTP) = perilaku v0.11.0 dipertahankan, hanya token/geometri/motion yang berubah |
| Traceability | PRD v2 UC8 (supersede), US-8 (supersede Arus) · FR-G1..G7 (supersede) · NFR-2/7 (a11y/usability) · STATE.md invariants 1–6 · redesign §6.5 (apa yang dulu, untuk honest-diff) |

---

## Clarifying Questions (front-loaded — jawab DULU, tiap pertanyaan + rekomendasi pm)

> Keputusan yang mengubah implementasi materially → OPEN QUESTIONS (OQ-n) untuk MEKL; TIDAK dijawab sendiri.
> Yang di bawah = pertanyaan desain yang pm rekomendasikan jawabannya langsung (default aman, bisa dibantah).

1. **Dusk = preset "Dusk" YANG AKTIF, atau tema tetap bernama internal?** Prototype Settings menampilkan 4
   preset (Dusk/Slate/Ink/Light) tapi hanya Dusk yang terverifikasi AA penuh di shell; Slate/Ink/Light hanya
   diverifikasi untuk swatch mini + teks/accent (lihat verification-report pairs). Rekomendasi: **v1 =
   Dusk satu-satunya tema yang BENAR-BENAR mengganti seluruh shell; Slate/Ink/Light = preset yang bisa
   dipilih dan mengubah token shell**, tapi kontras AA penuh ketiganya di SEMUA pasangan shell adalah
   prasyarat lead/architect sebelum di-expose (lihat OQ-2). Bila MEKL mau v1 kirim Dusk-only + 3 preset
   placeholder "coming", itu keputusan MEKL (OQ-2).
2. **Accent custom mengubah apa, persis?** Prototype `set-note`: "Recolors the accent only. Surfaces,
   text and status colors stay with the chosen theme." Rekomendasi: **accent custom HANYA me-remap
   `--accent` (+ turunannya `--accent-soft`, `--accent-ink` bila perlu kontras ink); surface/text/status
   (sage/terracotta) TIDAK berubah** — sesuai note prototype + anti-regresi kontras.
3. **Accent ink (`--accent-ink #241404`) ikut berubah saat accent custom di luar quick-pick?** Quick-pick
   5 warna sudah diverifikasi ink `#241404` ≥ 4.5:1 di atas tiap chip. Color-input bebas bisa memilih accent
   gelap → teks ink gelap di atasnya gagal. Rekomendasi: **auto-pilih ink (gelap `#241404` atau terang)
   berdasar luminance accent terpilih, ATAU batasi color-input ke rentang yang lolos** — keputusan teknis
   architect/lead; FR di bawah menyatakan kontrak AA, bukan algoritma.
4. **Google login v1 = visual saja atau fungsional?** Task menyatakan scope visual + alur auth tingkat
   tinggi; backend OAuth boleh fase berikut. Rekomendasi: **v1 = state visual (chip signed-in + popover
   "Sign in with Google") fungsional sebagai UI; alur OAuth nyata + sync = FR ditandai "fase berikut",
   bergantung OQ-3** (provider, redirect, apa yang di-sync). Tanpa backend, chip = mock state yang
   bisa di-toggle; TIDAK ada klaim "signed in" yang menyesatkan tanpa sesi nyata.
5. **AI backend mana + di mana command dieksekusi?** Prototype head-tag "on-device". Rekomendasi: **FR v1
   mengunci KONTRAK UX approval (human-in-the-loop) terlepas backend**; backend AI nyata (lokal/remote,
   model apa) = OQ-4 untuk MEKL. Eksekusi command = lewat terminal tab host yang sudah ada (real ssh),
   bukan jalur eksekusi baru — konsisten invariant spawn-real-ssh.
6. **Snippet disimpan di mana?** `~/.ssh/config` HARAM untuk data non-ssh (invariant 1). Rekomendasi:
   **OQ-5 untuk MEKL** — kandidat: file app-data terpisah (JSON) / sidecar-style file / bundled default +
   user file. v1 FR bisa mengunci perilaku UI (kategori/filter/copy) dengan snippet dari sumber read-only
   default dulu; persistensi user = fase yang bergantung OQ-5.
7. **"Always allow" scope — command persis atau pattern?** Prototype `cmd-foot`: "Allowlist stores this
   exact command only. Review patterns in Settings > Approvals." Rekomendasi: **v1 = exact-command-only
   allowlist (string persis + scope host)**; pattern/glob = fase berikut (OQ-6), karena pattern memperluas
   blast-radius dan butuh desain keamanan tersendiri. "Review in Settings > Approvals" = section Settings
   yang discoped v1 sebagai read/revoke list (lihat FR-SET), BUKAN editor pattern.
8. **Density/geometri berubah dari Arus — berapa?** Prototype Dusk: host-row 34px (Arus 28px), radius
   10/8/6px (`--r-lg/md/sm`), tab 32px, status bar 24px (SAMA Arus), sidebar 248px. Rekomendasi: **ambil
   angka persis dari prototype** (tabel §3.1); ini ARAH LEBIH LAPANG dari Arus, disengaja ("generous
   spacing, candlelit workshop"). Lead/architect yang mengunci tiap px di TSD.
9. **Motion/animasi.** Prototype: shimmer skeleton 1.4s, cursor blink 1.1s, transisi row-actions .12s,
   `prefers-reduced-motion` mematikan semua. Rekomendasi: **motion identik prototype; `prefers-reduced-motion`
   WAJIB (NFR a11y) — tidak ada keyframe baru di luar yang sudah ada di prototype.**
10. **Font.** Prototype deklarasi `--font-ui` Inter-stack, `--font-mono` JetBrains-Mono-stack, keduanya
    dengan fallback sistem. Invariant Arus: ZERO CDN/webfont (NFR-7). Rekomendasi: **font-stack persis
    prototype TAPI local-only; jika Inter/JetBrains Mono tidak di-bundle, fallback sistem yang dipakai —
    NOL network request** (lead putuskan bundle vs system-only; OQ bila MEKL mau bundle font).

---

## 1. Ringkasan & posisi vs v0.11.0 (Arus / Abyss Emerald)

**Jujur:** Dusk **mengganti identitas visual** hosterm. v0.11.0 ship tema **Abyss Emerald** (cool
green-on-black, arah "Arus", radius tajam, density ketat). Dusk = **warm paper-dark minimalist** —
brown-black surface, satu accent honey-amber, spacing lebih lapang, radius 10px lembut. Ini perubahan
hue family (cool→warm) dan density (ketat→lapang), BUKAN penyesuaian kecil.

### 1.1 Dipertahankan (TIDAK berubah)
- Arsitektur Tauri v2 + Rust backend + web UI; spawn **real `ssh`/`sftp`** (SDD ADR-1).
- Invariants config/sidecar (STATE.md §Hard invariants 1, 2, 6): `~/.ssh/config` parseable, round-trip
  comments/order/unknown, no secrets in config, sidecar 0600, authorized_keys dev tak disentuh.
- Invariant interaksi (3, 4): paste PTY CRLF→LF; drag/copy = manual mouse, BUKAN HTML5 DnD.
- Invariant staging (5): grep `target/|node_modules|.exe$|dist/` sebelum commit.
- **Struktur & perilaku komponen shell**: sidebar host-list (group label, dot live/idle, auth icon,
  skeleton, error row, retry), tab bar (kind icon, active underline, dead tab + reconnect pill, split
  drop-zone, unsaved dot), split terminal (divider drag manual + dblclick 50/50, focused-side accent),
  status bar global (`host · user@ip:port · connected · N live · UTF-8 · LF · shell · clock`), SFTP
  transfer engine. Perilaku = v0.11.0; hanya skin/geometri/motion yang berubah.
- Semua FR fungsional existing (US-1..US-7, US-9..US-10 PRD) tetap berlaku.

### 1.2 Berubah (Dusk)
- **Token tema**: palet (warm brown-black + honey-amber), lihat §3.1. 16+ token baru menggantikan
  Abyss Emerald.
- **Geometri/density**: radius 10/8/6px, host-row 34px, spacing lebih lapang (lihat §3.1).
- **Motion**: shimmer/blink/transition persis prototype; `prefers-reduced-motion` mematikan.
- **Elevation**: drawer + modal = satu-satunya elemen yang "float" (`--shadow-drawer`).

### 1.3 Baru (fitur fungsional — tidak ada di v0.11.0)
- **Settings panel** + section **Appearance** (preset picker + accent custom + live preview).
- **Google login** (chip signed-in + popover sign-in) — scope §4.3.
- **Snippet / cheatsheet panel** (kategori, filter, copy) — §4.4.
- **AI assistant + eksekusi command dengan APPROVAL** (Run once / Always allow / Reject) — §4.5,
  security-sensitive.

---

## 2. Scope IN / OUT

### 2.1 IN (v1)
- Tema Dusk: token, radius, spacing, motion (§3).
- Settings panel: modal + nav; section **Appearance** fungsional penuh (preset picker Dusk/Slate/Ink/Light,
  accent custom color-input + 5 quick-pick chips, live preview, Done + Reset). Section nav lain
  (General / Terminal / SSH & Keys / AI Assistant / Snippets / Account) = **placeholder** (nav ada,
  konten "coming" / kosong — bukan scope v1 kecuali di-list di bawah).
- Google login **visual** (chip signed-in + popover signed-out) + alur auth tingkat tinggi (OQ-3).
- Snippet panel: render kategori, filter substring, copy per-item (persistensi user = OQ-5).
- AI panel + approval UX: thread, input, command card dengan Run once / Always allow / Reject;
  allowlist exact-command + scope host; "Always allow" visual sekunder.
- Settings > Approvals sebagai **read/revoke list** allowlist (konsekuensi "Review patterns in Settings").

### 2.2 OUT (eksplisit — keputusan MEKL)
- **Editor token tema penuh** (scope tema = preset + 1 accent custom SAJA).
- **Import/export tema**.
- **Light/dark toggle** di luar yang preset tawarkan (Light = salah satu preset, bukan toggle terpisah).
- Pattern/glob di "Always allow" (v1 = exact command only, OQ-6).
- Backend OAuth nyata + cross-device sync (visual v1; fungsional = fase berikut, OQ-3).
- Backend AI nyata / pemilihan model (kontrak UX dikunci v1; backend = OQ-4).
- Persistensi snippet yang bisa diedit user (UI read + copy v1; store = OQ-5).
- Section Settings non-Appearance berisi kontrol fungsional (placeholder v1).

---

## 3. Tema Dusk — token, geometri, motion (FR-DT)

### 3.1 Token (ambil PERSIS dari prototype `proto-a-dusk.html` / `-settings.html` `:root`)

| Token | Nilai | Peran |
|---|---|---|
| `--bg` | `#191512` | app background (warm brown-black) |
| `--sidebar` | `#1d1814` | sidebar / tab bar / status bar / drawer |
| `--surface` | `#221c17` | row hover, kartu, input |
| `--raised` | `#2a231c` | elemen terangkat (chip, kbd, avatar, btn-secondary) |
| `--term` | `#14100d` | terminal well + cmd-line/output bg |
| `--border` | `#332a21` | garis 1px |
| `--hairline` | `rgba(226,163,92,.14)` | garis accent samar |
| `--text` | `#f2e9dc` | teks utama |
| `--dim` | `#b9aa95` | teks sekunder |
| `--faint` | `#93836f` | teks tersier / placeholder |
| `--accent` | `#e2a35c` | honey amber — focus, tab aktif, primary action |
| `--accent-ink` | `#241404` | teks di atas accent (kontras 8.18:1) |
| `--accent-soft` | `rgba(226,163,92,.12)` | fill accent samar |
| `--live` | `#9dc489` | sage — status live (semantik) |
| `--danger` | `#e07856` | terracotta — error/dead/reject (semantik) |
| `--r-lg` / `--r-md` / `--r-sm` | `10px` / `8px` / `6px` | radius |
| `--shadow-drawer` | `0 10px 30px rgba(0,0,0,.42)` | elevation (drawer/modal only) |
| `--font-ui` | Inter-stack (fallback sistem) | UI |
| `--font-mono` | JetBrains-Mono-stack (fallback sistem) | mono/terminal |

Preset non-Dusk (swatch terverifikasi, lihat verification-report `NEW PAIRS`): Slate `#171d24`/`#222a33`/`#6cc4dc`/`#e4ebf2`;
Ink `#141414`/`#1e1e1e`/`#e0563f`/`#ececec`; Light `#f5f1e8`/`#ffffff`/`#96571a`/`#26201a`.
Quick-pick accent chips: amber `#e2a35c`, teal `#5fbfae`, rose `#e0869b`, lime `#aec267`, violet `#b99be3`.

### 3.2 Geometri (persis prototype)

| Elemen | Nilai |
|---|---|
| Sidebar width | `248px` |
| Host row height | `34px` |
| Tab height / tab bar | `32px` / `42px` |
| Drawer (AI/Snippets) width | `340px` |
| Settings modal | `900px` × `620px` (max 94vw × 92vh) |
| Status bar height | `24px` |
| Term divider | `4px`, `col-resize` |
| Base font-size | `13px`, line-height `1.5` |

### 3.3 Functional requirements — tema

| ID | Requirement (EARS) | Prioritas |
|---|---|---|
| FR-DT1 | WHEN aplikasi me-render THE SYSTEM SHALL memakai token Dusk §3.1 PERSIS (nilai hex/rgba identik prototype) dengan NOL literal Abyss Emerald tersisa di `src/` | must |
| FR-DT2 | WHEN accent, tab aktif, sisi terminal fokus, atau primary button di-render THE SYSTEM SHALL memakai `--accent`; WHEN hover/idle THE SYSTEM SHALL TIDAK memakai glow/accent-fill penuh (elevation lewat `--shadow-drawer` hanya di drawer + settings modal) | must |
| FR-DT3 | WHEN daftar memuat (sidebar, SFTP) THE SYSTEM SHALL menampilkan skeleton shimmer (`1.4s linear infinite`) tanpa layout jump (delta tinggi = 0px) | must |
| FR-DT4 | WHEN `prefers-reduced-motion: reduce` aktif THE SYSTEM SHALL mematikan SEMUA animasi + transisi (shimmer, blink, row-actions fade) | must |
| FR-DT5 | WHEN font dimuat THE SYSTEM SHALL memakai `--font-ui`/`--font-mono` stack prototype dengan NOL network request (CDN/webfont) — local/bundle/system-only | must |
| FR-DT6 | WHEN radius diaplikasikan THE SYSTEM SHALL memakai `--r-lg 10px` / `--r-md 8px` / `--r-sm 6px`; tombol Google = radius 4px sendiri (brand spec), bukan token app | must |
| FR-DT7 | WHEN status live/error di-render THE SYSTEM SHALL memakai `--live` (sage) / `--danger` (terracotta) sebagai warna SEMANTIK saja, tidak sebagai dekorasi | must |

---

## 4. Functional requirements per area (EARS)

### 4.1 Shell (dipertahankan dari v0.11.0, re-skin Dusk)

| ID | Requirement (EARS) | Prioritas |
|---|---|---|
| FR-SH1 | WHEN shell di-render THE SYSTEM SHALL menyusun sidebar (248px) + main column (tab bar 42px → workbench → status bar 24px), persis struktur prototype | must |
| FR-SH2 | WHEN host-list di-render THE SYSTEM SHALL menampilkan group label (prefix-derived), host row 34px dengan dot live/idle + auth icon + row-actions (Open SFTP, Host menu) yang muncul on hover/focus-within | must |
| FR-SH3 | WHEN sebuah host punya sesi live THE SYSTEM SHALL menampilkan `dot-live` (sage + ring); WHEN idle THE SYSTEM SHALL menampilkan `dot-idle` (outline faint) | must |
| FR-SH4 | WHEN host gagal auth THE SYSTEM SHALL menampilkan error row (`<name> auth failed`) + tombol Retry, tanpa menghapus row lain | must |
| FR-SH5 | WHEN tab bar di-render THE SYSTEM SHALL menampilkan tab kind-icon, tab aktif dengan underline `--accent` 2px, unsaved-dot untuk editor dirty, dead tab `--danger` + reconnect pill, dan split drop-zone di akhir | must |
| FR-SH6 | WHEN sisi split di-fokus THE SYSTEM SHALL menampilkan accent underline 2px di atas sisi itu; WHEN divider di-drag THE SYSTEM SHALL resize via manual mouse event; WHEN dblclick THE SYSTEM SHALL reset 50/50 | must |
| FR-SH7 | WHEN mode-switch ditekan (Terminal/Snippets/AI) THE SYSTEM SHALL menoggle drawer terkait dan menandai `aria-pressed=true` tepat satu tombol | must |
| FR-SH8 | WHEN status bar di-render THE SYSTEM SHALL menampilkan host · `user@ip:port · connected` (kiri) dan `N live · UTF-8 · LF · shell · clock` (kanan), mono 11px | must |

### 4.2 Settings panel + Appearance

| ID | Requirement (EARS) | Prioritas |
|---|---|---|
| FR-SET1 | WHEN tombol gear (sidebar footer) ditekan THE SYSTEM SHALL membuka Settings sebagai modal in-app (scrim `rgba(12,9,6,.55)`, `role=dialog aria-modal=true`), BUKAN window/native dialog | must |
| FR-SET2 | WHEN Settings terbuka THE SYSTEM SHALL menampilkan nav kiri (General, Appearance, Terminal, SSH & Keys, AI Assistant, Snippets, Account) dengan Appearance `aria-current=page` sebagai default | must |
| FR-SET3 | WHEN section nav selain Appearance (dan Approvals, FR-AI8) dipilih THE SYSTEM SHALL menampilkan placeholder (bukan kontrol fungsional) — di luar scope v1 | must |
| FR-SET4 | WHEN Appearance di-render THE SYSTEM SHALL menampilkan preset grid 4 kolom (Dusk/Slate/Ink/Light) dengan swatch mini + nama; preset terpilih `aria-pressed=true` + check mark | must |
| FR-SET5 | WHEN operator memilih preset THE SYSTEM SHALL menandai preset itu terpilih dan menerapkan token-set preset ke shell (keputusan cakupan preset non-Dusk = OQ-2) | must |
| FR-SET6 | WHEN Appearance di-render THE SYSTEM SHALL menampilkan satu color-input accent + label hex + 5 quick-pick chip (amber/teal/rose/lime/violet) | must |
| FR-SET7 | WHEN operator memilih quick-pick chip ATAU mengubah color-input ke hex `#RRGGBB` valid THE SYSTEM SHALL meng-update `--accent` + live preview, dan menandai chip yang cocok `aria-pressed=true` | must |
| FR-SET8 | WHEN accent berubah THE SYSTEM SHALL me-recolor accent SAJA (surface/text/status tetap tema terpilih — prototype `set-note`) | must |
| FR-SET9 | WHEN accent custom diterapkan THE SYSTEM SHALL menjaga kontras teks-ink di atas accent ≥ 4.5:1 (auto ink / batas range — OQ pm #3) | must |
| FR-SET10 | WHEN live preview di-render THE SYSTEM SHALL menampilkan tab + primary button + prompt line yang memakai accent terpilih, update real-time saat accent berubah | must |
| FR-SET11 | WHEN "Reset to Dusk default" ditekan THE SYSTEM SHALL mengembalikan accent ke `#e2a35c` dan preset ke Dusk | must |
| FR-SET12 | WHEN "Done" ditekan, scrim diklik di luar panel, atau Escape ditekan THE SYSTEM SHALL menutup Settings dan mempertahankan pilihan tema/accent | must |
| FR-SET13 | WHEN pilihan tema/accent diubah THE SYSTEM SHALL mem-persist pilihan antar sesi (lokasi store = konsekuensi OQ-5/arsitektur; TIDAK ke `~/.ssh/config`) | must |

### 4.3 Google login (visual + alur tingkat tinggi)

| ID | Requirement (EARS) | Prioritas |
|---|---|---|
| FR-GL1 | WHEN signed-out THE SYSTEM SHALL menampilkan user-chip yang membuka popover "Use hosterm with Google" dengan tombol "Sign in with Google" (surface putih `#ffffff`, teks `#1f1f1f`, radius 4px — Google brand spec) | must |
| FR-GL2 | WHEN signed-in THE SYSTEM SHALL menampilkan user-chip (avatar inisial + nama + G mark + caret) di sidebar footer | must |
| FR-GL3 | WHEN tombol "Sign in with Google" ditekan THE SYSTEM SHALL memulai alur auth Google (provider/redirect/scope = OQ-3); sampai backend OAuth ada, state chip adalah UI-only dan TIDAK mengklaim sesi tersaut tanpa auth nyata | must (visual) / fase berikut (fungsional) |
| FR-GL4 | WHEN signed-in dan user memilih sign-out THE SYSTEM SHALL kembali ke state signed-out (chip → popover) | must (visual) |
| FR-GL5 | THE SYSTEM SHALL TIDAK menulis kredensial/token Google ke `~/.ssh/config`; penyimpanan token (bila backend ada) mengikuti invariant secret (sidecar-class, bukan config) — detail = OQ-3 | must |

### 4.4 Snippet / cheatsheet panel

| ID | Requirement (EARS) | Prioritas |
|---|---|---|
| FR-SN1 | WHEN panel Snippets dibuka THE SYSTEM SHALL menampilkan snippet dikelompokkan per kategori (mis. Docker/Git/systemd) dengan nama + command mono per item | must |
| FR-SN2 | WHEN operator mengetik di filter THE SYSTEM SHALL menyempitkan item via substring (nama + command); IF nol item cocok THEN THE SYSTEM SHALL menampilkan empty-state (`No snippets match "<q>"`) | must |
| FR-SN3 | WHEN tombol copy per-item ditekan THE SYSTEM SHALL menyalin command ke clipboard dan menampilkan konfirmasi visual sementara (`.done`, ~900ms) | must |
| FR-SN4 | WHEN head-count di-render THE SYSTEM SHALL menampilkan jumlah snippet terlihat | should |
| FR-SN5 | WHEN command snippet di-render ke DOM THE SYSTEM SHALL meng-escape teks (textContent / esc) — konsisten INV output hostile Arus | must |
| FR-SN6 | WHEN snippet sumber-persistensi diputuskan (OQ-5) THE SYSTEM SHALL membaca dari sumber itu; v1 minimum = set default read-only | must (v1 read) / user-store fase berikut |

### 4.5 AI assistant + eksekusi command dengan APPROVAL (SECURITY-SENSITIVE)

> Human-in-the-loop WAJIB eksplisit. Eksekusi = lewat terminal real-ssh host yang ada, bukan jalur baru.

| ID | Requirement (EARS) | Prioritas |
|---|---|---|
| FR-AI1 | WHEN panel AI dibuka THE SYSTEM SHALL menampilkan thread (pesan user + AI), command card untuk perintah yang diusulkan, dan input "Ask hosterm" | must |
| FR-AI2 | WHEN AI mengusulkan sebuah command THE SYSTEM SHALL merender command card dengan: scope host (alias persis), command line mono, deskripsi dampak, dan state badge | must |
| FR-AI3 | WHEN sebuah command card menunggu keputusan THE SYSTEM SHALL menampilkan state `AWAITING APPROVAL` (warn/accent) dan tiga aksi: **Run once** (primary), **Always allow** (secondary), **Reject** (ghost) | must |
| FR-AI4 | THE SYSTEM SHALL merender "Always allow" secara VISUAL SEKUNDER (btn-secondary, bukan primary) sehingga "Run once" adalah default mata/klik — anti salah-klik pada aksi yang memperluas izin | must |
| FR-AI5 | WHEN "Run once" ditekan THE SYSTEM SHALL mengeksekusi command itu SEKALI pada host scope (via terminal real-ssh), tanpa menambah entri allowlist | must |
| FR-AI6 | WHEN "Always allow" ditekan THE SYSTEM SHALL menyimpan entri allowlist berisi **command string PERSIS + scope host** (BUKAN pattern/glob di v1) dan mengeksekusi; entri berlaku hanya untuk pasangan command-exact + host itu | must |
| FR-AI7 | WHEN sebuah command sudah ada di allowlist (exact + host) THE SYSTEM SHALL menampilkan state `ALWAYS ALLOWED` (ok/sage) dan boleh mengeksekusi tanpa menunggu approval ulang | must |
| FR-AI8 | WHEN "Reject" ditekan THE SYSTEM SHALL membatalkan command tanpa eksekusi dan tanpa menambah allowlist | must |
| FR-AI9 | THE SYSTEM SHALL TIDAK PERNAH mengeksekusi command yang diusulkan AI tanpa salah satu dari: (a) klik Run once, (b) match exact pada allowlist yang pernah di-Always-allow user; tidak ada auto-run implisit | must |
| FR-AI10 | WHEN Settings > AI Assistant / Approvals dibuka THE SYSTEM SHALL menampilkan daftar allowlist (command + host) dengan aksi revoke per entri (read/revoke list; BUKAN editor pattern di v1) | must |
| FR-AI11 | WHEN output remote command di-render ke DOM THE SYSTEM SHALL meng-escape byte (output remote = input hostile, INV Arus) | must |
| FR-AI12 | THE SYSTEM SHALL menampilkan head-tag sumber AI (mis. "on-device"); backend nyata = OQ-4, kontrak approval di atas berlaku terlepas backend | must |
| FR-AI13 | IF command yang diusulkan AI gagal safety-check argv (alias berawalan `-`, newline/CR) THEN THE SYSTEM SHALL menolaknya sebelum eksekusi (paritas FR-FL20 Fleet) | must |

---

## 5. Non-functional requirements (TERUKUR)

| ID | Requirement | Verifikasi |
|---|---|---|
| NFR-DK1 (a11y) | SEMUA pasangan teks normal tema Dusk ≥ WCAG AA 4.5:1; prototype terverifikasi (contrast-check.py): teks `#f2e9dc/#191512` **15.09:1**, dim `#b9aa95/#191512` **7.99:1**, faint `#93836f/#191512` **4.94:1**, accent `#e2a35c/#191512` **8.32:1**, ink `#241404/#e2a35c` **8.18:1**, live `#9dc489/#191512` **9.24:1**, danger `#e07856/#191512` **6.04:1**, tombol Google `#1f1f1f/#ffffff` **16.48:1** | cite verification-report + re-check saat token mendarat |
| NFR-DK2 (a11y preset) | Preset non-Dusk pasangan teks/accent ≥ AA: Slate teks **14.11**/accent **8.54**; Ink teks **15.59**/accent **4.89**; Light teks **14.29**/accent **5.07**. Quick-pick chip vs bg + ink di atas chip semua ≥ 4.5 (lihat verification-report). Kontras AA PENUH preset non-Dusk di SELURUH pasangan shell = prasyarat sebelum di-expose (OQ-2) | verification-report pairs; lead/architect gate |
| NFR-DK3 (a11y accent custom) | Accent custom yang dipilih WAJIB menjaga teks-ink ≥ 4.5:1 (FR-SET9); color-input bebas tanpa guard = regresi — ditangani OQ pm #3 | smoke kontras ink/accent |
| NFR-DK4 (WebView2/Windows) | Semua interaksi drag (term divider, SFTP, snippet reorder bila ada) = manual mouse event, BUKAN HTML5 DnD (mati di WebView2 — pitfall diketahui, invariant 4). `draggable` grep di `src/` = 0 | grep + smoke |
| NFR-DK5 (motion) | `prefers-reduced-motion: reduce` mematikan semua animasi/transisi; nol keyframe baru di luar prototype (shimmer, blink) | smoke media-query |
| NFR-DK6 (network) | Dusk + Settings + Snippet TIDAK menambah network request (font local-only). Google sign-in = SATU-SATUNYA jalur network baru, user-initiated, bergantung OQ-3; NFR-3 PRD (zero background) tetap berlaku | smoke network log |
| NFR-DK7 (invariants) | NOL pelanggaran STATE.md invariants 1–6 (config/sidecar/paste/DnD/staging/authorized_keys); snippet/allowlist/token Google TIDAK masuk `~/.ssh/config` | grep gate + cargo round-trip |
| NFR-DK8 (regression) | Verify loop STATE.md (cargo + build + smoke) hijau; tiap unit menambah smoke check untuk perilaku barunya (baseline re-measure saat dispatch, bukan di-hardcode di sini) | task log per unit |
| NFR-DK9 (CSP) | `script-src 'self'` tetap; 0 CSP violation di smoke; output AI/snippet/terminal di-escape (hostile input) | smoke CSP counter == 0 |

---

## 6. Open Questions (OQ-n — untuk MEKL, JANGAN dijawab sendiri)

Setiap OQ materially mengubah implementasi; pm beri rekomendasi, keputusan = MEKL.

1. **OQ-1 — Supersede Arus di PRD/STATE:** Dusk mengganti identitas visual aktif v0.11.0. Tandai Arus
   "superseded by Dusk" di PRD §1.1/US-8/§7 (dilakukan di delta PRD epic ini) — tapi apakah MEKL mau
   STATE.md §layout juga di-update oleh architect/lead saat token Dusk mendarat (Arus token notes jadi
   historis)? Rekomendasi pm: **ya, saat unit tema mendarat, bukan sekarang** (PM tak tulis STATE).
2. **OQ-2 — Cakupan preset non-Dusk v1:** Slate/Ink/Light hanya terverifikasi untuk swatch + teks/accent,
   BUKAN seluruh pasangan shell. v1 = (a) Dusk-only aktif + 3 preset placeholder "coming", atau (b) 4 preset
   fungsional dengan prasyarat lead/architect verifikasi AA penuh ketiganya dulu? Rekomendasi pm: **(b) hanya
   jika AA penuh terbukti; kalau tidak, (a)** — jangan ship preset yang gagal kontras di sebagian shell.
3. **OQ-3 — Google OAuth nyata:** provider/flow (Google Identity? device-code? loopback redirect?), scope,
   APA yang di-sync (hosts? snippets? settings? — prototype popover menjanjikan ketiganya), dan di mana token
   disimpan (sidecar-class). v1 visual-only tanpa ini. Rekomendasi pm: **putuskan sebelum fase fungsional;
   v1 ship chip visual + popover, backend = epic sendiri** (sync lintas-device melanggar "zero cloud" PRD
   out-of-scope v1 — perlu keputusan eksplisit MEKL bahwa Dusk membuka pintu itu).
4. **OQ-4 — Backend AI:** on-device (model apa, ukuran, bundle?) vs remote API (endpoint, key, privacy —
   melanggar NFR-3 zero-network?). Kontrak approval dikunci terlepas backend, tapi backend menentukan
   privacy/jalur network. Rekomendasi pm: **MEKL tentukan arah; v1 bisa ship UX approval + input stub bila
   backend belum siap** — jangan klaim "on-device" bila nyatanya remote.
5. **OQ-5 — Store snippet + settings:** di mana (file JSON app-data? sidecar-style?)? `~/.ssh/config` HARAM.
   Mempengaruhi FR-SN6 + FR-SET13. Rekomendasi pm: **satu file app-data terpisah (mis.
   `~/.config/hosterm/` atau setara OS), bundled default snippets read-only + user overlay** — keputusan
   arsitektur, tapi MEKL konfirmasi lokasi/portabilitas (invariant portabilitas NFR-8: copy config pindah
   mesin — apakah snippet/settings ikut?).
6. **OQ-6 — "Always allow" pattern di fase berikut:** v1 = exact command only. Apakah MEKL mau roadmap
   pattern/glob allowlist (prototype menjanjikan "Review patterns in Settings > Approvals")? Rekomendasi pm:
   **v1 exact-only; pattern = epic keamanan terpisah** — pattern memperluas blast-radius, butuh desain
   (anchoring, escaping, preview match) tersendiri.

---

## 7. Milestone / unit hint (kasar — breakdown FINAL = architect/lead)

> Urutan AMAN (ketergantungan token → komponen → fitur). PM TIDAK mengunci granularity unit; ini hint.

1. **Tema token Dusk dulu** (swap `:root`, remap xterm/CodeMirror, glow/elevation di selector yang benar) —
   fondasi semua komponen. Analog U1 Arus.
2. **Re-skin komponen shell** ke token Dusk (sidebar, tab bar, split, status bar, SFTP) — perilaku existing
   dipertahankan, hanya skin/geometri/motion. Serial karena hotspot `styles.css`/`main.ts`.
3. **Settings panel + Appearance** (modal, nav, preset picker, accent custom, preview, persist) — butuh
   token + store (OQ-5).
4. **Snippet panel** (kategori/filter/copy; store OQ-5).
5. **Google login visual** (chip + popover; backend OQ-3 fase berikut).
6. **AI assistant + approval** (thread, command card, allowlist exact + revoke list; backend OQ-4) — paling
   security-sensitive, paling akhir.
7. **Smoke canonization + docs** (seperti U9 Arus).

Hotspot diketahui (postur Arus §0): `src/styles.css` + `src/main.ts` di-sentuh hampir tiap unit → serial,
satu card aktif per file. Flag ke nano bila dua unit butuh file sama bersamaan.

---

*Sources: docs/UX/prototypes/proto-a-dusk.html · proto-a-dusk-settings.html · README.md · verification-report.json ·
docs/STATE.md (invariants, verify loop, Arus/Fleet state) · docs/PRD.md (UC8/US-8/FR-G*/NFR) ·
docs/UX/redesign-hosterm-v1.md §6.5 (Abyss Emerald, untuk honest-diff).
Read 2026-10-31.*
