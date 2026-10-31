# EPIC-TSD: Redesign Dusk v1 — playbook implementasi (HOW)

| Field | Value |
|---|---|
| Author | crew-coding-lead |
| Status | Written — awaiting nano verify (TSD + breakdown); NO push. nano creates per-unit tasks from §2. |
| Source (WHAT → HOW) | `docs/SDD.md §13` (ADR-18..22, INV-21..26, IPC 24→27 §13.5, approval contract §13.6, token system §13.2, store §13.4 — **arsitektur FINAL, jangan tulis ulang**) · `docs/FEATURE-SPEC-redesign-dusk-v1.md` (FR-DT/SH/SET/GL/SN/AI, NFR-DK) · `docs/UX/prototypes/proto-a-dusk.html` + `proto-a-dusk-settings.html` + `verification-report.json` (visual truth, APPROVED MEKL) · `docs/EPIC-TSD-redesign-arus-v1.md` (pola 9-unit serial terbukti) · `docs/STATE.md` (invariants + verify loop) |
| Keputusan MEKL (31 Okt) | OQ-3/OQ-4 = **VISUAL DULU** (Google login + AI = UI + kontrak approval lengkap; backend OAuth & AI = fase berikut). Default lain = `[ASSUME-OQ-n]` SDD §13: Dusk supersede Arus · preset non-Dusk placeholder "coming" aria-disabled · store JSON app-data (`dirs::config_dir()/hosterm/`) · Always-allow exact-command. |
| Gate | Docs (file ini) = lead boleh commit docs-scope (orchestrator-verify → push langsung). **Untuk task ini: tulis, JANGAN push** — nano verifikasi dulu. Tiap unit KODE = pipeline: unit done → lead CHECKPOINT commit lokal → QA + SEC review paralel atas SHA → lead push pasca PASS + CLEAN. |

---

## 0. Sequencing (WAJIB dibaca sebelum dispatch)

**Hotspot (binding, SDD §13.9 + Arus §0 + STATE.md):** `src/styles.css` + `src/main.ts` disentuh HAMPIR SETIAP unit frontend Dusk → dispatch **SERIAL**, satu card aktif per file, **tidak pernah paralel**. Satu-satunya pengecualian file-disjoint = unit **store backend** (`lib.rs` + asset JSON), yang BOLEH jalan paralel dengan unit frontend AWAL (pola Fleet FL-1 ∥ Arus U5–U9) — **syarat: tidak ada card frontend yang sedang menyentuh `lib.rs`** (unit Dusk frontend memang tidak menyentuh `lib.rs`, jadi aman).

> `hotspot: src/styles.css + src/main.ts — every Dusk frontend unit touches them; serialize, one card per file (Arus §0 posture).`

**Baseline verify loop — RE-MEASURED di working tree 31 Okt 2026, HEAD `31bef63`** (angka STATE.md kanonik; ukur ULANG saat dispatch tiap unit karena unit sebelumnya menggeser baseline):

| Gate | Nilai @ 31bef63 | Perintah |
|---|---|---|
| `cargo test --lib` | **78 pass** (lib 57 + fleet 21 by static test count; STATE.md canon lama tertulis 77 — angka LIVE = 78) | `cd src-tauri && cargo test --lib` |
| `npm run build` | **0 TS error** (~500ms) | `npm run build` |
| `node qa-reports/headless-smoke.mjs` | **225 pass** | `npm install --no-save playwright@1.63.0 && node qa-reports/headless-smoke.mjs` lalu bersihkan playwright (lihat §1) |
| IPC inventory `grep -rc "tauri::command" src-tauri/src/*.rs` | **24** (lib.rs 22 + fleet.rs 2) → wajib **27** setelah store unit (lib.rs 25) | grep |

Angka ini = baseline unit PERTAMA. Tiap unit menaikkan smoke (+N cek baru) dan jadi baseline unit berikutnya. **Dilarang hard-code angka spec/PRD lama** (FEATURE-SPEC menyebut 196 — itu basi; STATE.md menyebut 77/196 — juga tergeser; LIVE = 78/225/24).

---

## 1. Dev playbook

### 1.1 Stack final

- **Frontend:** TypeScript (`src/main.ts`), `src/styles.css`, `index.html` — tema = sistem token data-driven (ADR-20): Dusk = `:root` default, preset = blok `:root[data-theme="…"]`, accent custom = inline `--accent` di `:root`. NOL framework UI baru, NOL dep frontend baru. Font local/system-only (NFR-DK6, FR-DT5) — dilarang `@import url(http…)`/`<link>` webfont.
- **Backend:** Rust (`src-tauri/src/lib.rs`) — **3 IPC command BARU** (SDD §13.5, frozen verbatim): `read_settings`, `write_settings`, `read_snippets`. Resolusi path via **`dirs` crate yang SUDAH di tree** (ADR-18; NOL dep baru — bukan `tauri-plugin-store`/`-fs`). `HOSTERM_DATA_DIR` env override = test hook (paritas `HOSTERM_CONFIG`). Write = atomic temp→rename (INV-22/24), 0600-class. Bundled asset `src/assets/snippets.default.json` (ADR-19, embedded, zero network).
- **Store file:** `dirs::config_dir()/hosterm/settings.json` (Linux `~/.config/hosterm/`). DI LUAR `~/.ssh/` (INV-22). Schema `settings.json` = `{version, theme, accent, allowlist[]}` persis SDD §13.4; corrupt/missing → `DEFAULT_SETTINGS`, tak pernah di-clobber (INV-24).
- **AI/Google:** VISUAL-ONLY v1 (ADR-22). NOL backend OAuth, NOL backend AI, NOL IPC baru untuk mereka. AI "Run once" = eksekusi lewat **terminal real-ssh existing** (ADR-1/2), bukan jalur eksekusi baru. Allowlist persist via `read_settings`/`write_settings` (folded, ADR-21).

### 1.2 Testing strategy

| Jenis | Kapan | Isi |
|---|---|---|
| `cargo test --lib` | **HANYA unit store** (U-D3) butuh test baru | round-trip write→read; corrupt-file → defaults (INV-24); config+sidecar hash/mtime unchanged across settings write (INV-22); accent/theme/allowlist validasi (reject `settings:invalid-accent`/`-theme`/`write-failed`). Baseline **+ ≥4**. Semua unit lain: cargo WAJIB tetap ≥ baseline sebagai kanari (0 diff backend). |
| `npm run build` | tiap unit | 0 TS error. Gate tak berubah. |
| `headless-smoke.mjs` | **tiap unit** | pola mock existing: `window.__TAURI_INTERNALS__.invoke` → dispatch `cmd` di `__invokeImpl`; `calls[]` merekam `{cmd,args}`; `page.addInitScript(mockTauri)`. **3 IPC baru di-mock di `__invokeImpl` pola Fleet** (tambah arm `read_settings`/`write_settings`/`read_snippets` → return fixture SDD §13.5; rekam ke `calls` untuk assert persist). Visual unit = smoke-only (0 cargo baru). |

Mock fixture settings (copy dari SDD §13.5): `{ "version":1, "theme":"dusk", "accent":"#e2a35c", "allowlist":[{ "host":"prod-web", "command":"sudo systemctl restart netdata", "added_at":"2026-10-30T12:00:00Z" }] }`.

**Cleanup playbook (headless guard memblokir `rm -rf`):** setelah smoke, `mkdir -p "$TMPDIR/pw-trash" && mv node_modules/playwright node_modules/playwright-core "$TMPDIR/pw-trash/"` lalu `unlink` symlink `node_modules/.bin/playwright*` — JANGAN `rm -rf` (diblokir single-query mode). Grep staging `target/|node_modules|.exe$|dist/` sebelum commit (INV-5).

### 1.3 Rollout — pipeline mode (per keputusan MEKL)

Per unit KODE: **unit done → lead CHECKPOINT commit LOKAL (NO push, NO tag, NO version bump) → QA + SEC review PARALEL atas SHA checkpoint → lead push ke main pasca QA PASS + SEC CLEAN.** Tiap unit = 1 commit terpisah (revert per-unit tanpa menyentuh lain; frontend-only kecuali store = tak ada migrasi data). Risiko terbesar = U-D1 token swap (xterm/CM literal) — regresi kontras → fallback `git revert` 1 commit. Manual Windows/WebView2 = waiver (lihat §5).

---

## 2. BREAKDOWN UNIT FINAL (granularity call lead)

**7 unit + 1 audit = 8 unit.** Urutan SERIAL dari SDD §13.9, dependency-driven. Store backend (U-D3) file-disjoint → BOLEH paralel U-D1/U-D2. Semua unit lain serial karena hotspot `styles.css`/`main.ts`.

Prefix unit = **U-D** (Dusk) untuk membedakan dari U1–U9 Arus yang sudah landed.

| Unit | Judul | Size | Hotspot | Deps | cargo baru? | FR/INV dicover |
|---|---|---|---|---|---|---|
| U-D1 | Dusk token swap | M | ✅ styles.css + main.ts | — | tidak (smoke-only) | FR-DT1..7, INV-21/23(partial) |
| U-D2 | Shell re-skin (sidebar/tab/split/status/SFTP) | L | ✅ styles.css + main.ts | U-D1 | tidak (smoke-only) | FR-SH1..8, INV-21 |
| U-D3 | Store backend (3 IPC + schema + asset) | M | ❌ lib.rs + asset (file-disjoint) | — (∥ U-D1/U-D2) | **YA (+≥4)** | FR-SET13, FR-SN6, FR-AI6/7/10 (persist), INV-22/24, ADR-18/19/21 |
| U-D4 | Settings panel + Appearance | L | ✅ styles.css + main.ts + index.html | U-D1, U-D3 | tidak (smoke-only) | FR-SET1..12, INV-23, ADR-20, `[ASSUME-OQ-2]` |
| U-D5 | Snippet panel | M | ✅ styles.css + main.ts + index.html | U-D3 | tidak (smoke-only) | FR-SN1..6, INV-11-family |
| U-D6 | Google login VISUAL | S | ✅ styles.css + main.ts + index.html | U-D1 | tidak (smoke-only) | FR-GL1..5, ADR-22, `[ASSUME-OQ-3]` |
| U-D7 | AI assistant + approval VISUAL | L | ✅ styles.css + main.ts + index.html | U-D1, U-D3 | tidak (smoke-only) | FR-AI1..13, INV-25/26, ADR-21/22, §13.6 |
| U-D8 | Smoke canonization + docs (STATE.md) | S | ❌ smoke + docs | U-D1..U-D7 | tidak | NFR-DK8, kanonisisasi semua cek |

**Urutan dispatch serial (hotspot):** U-D1 → U-D2 → U-D4 → U-D5 → U-D6 → U-D7 → U-D8. **U-D3 (store, file-disjoint) dispatch paralel dengan U-D1/U-D2**, WAJIB landed sebelum U-D4 (settings persist) + U-D7 (allowlist persist) — gate `git log origin/main` untuk U-D3 SHA sebelum dispatch U-D4.

---

### U-D1 — Dusk token swap

**Sumber:** SDD §13.2/§13.3 (token verbatim + ADR-20 mekanisme), FR-DT1..7, INV-21/23.

**Cakupan file (persis):**
- `src/styles.css` — ganti SELURUH blok `:root` token Abyss Emerald → **16 token Dusk byte-exact** dari SDD §13.3 (`--bg #191512`, `--sidebar #1d1814`, `--surface #221c17`, `--raised #2a231c`, `--term #14100d`, `--border #332a21`, `--hairline rgba(226,163,92,.14)`, `--text #f2e9dc`, `--dim #b9aa95`, `--faint #93836f`, `--accent #e2a35c`, `--accent-ink #241404`, `--accent-soft rgba(226,163,92,.12)`, `--live #9dc489`, `--danger #e07856`, `--r-lg/md/sm 10/8/6px`, `--shadow-drawer 0 10px 30px rgba(0,0,0,.42)`, `--font-ui`/`--font-mono` stack). Pertahankan `color-scheme: dark`.
- `src/styles.css` — glow/elevation: `--shadow-drawer` HANYA di drawer + settings modal (FR-DT2). Hapus `--glow` lama (Abyss Emerald); accent-fill penuh hanya di state aktif (`.tab.active`/`.term-side.focused`/`button.primary` → `--accent`), TIDAK di hover/idle.
- `src/main.ts` — literal yang CSS var tak jangkau (SDD §13.2 pt 5): objek `theme` xterm (23 key) + `cmTheme`/`cmHighlight` CodeMirror → **remap ke nilai Dusk** (bg=`--term` #14100d, fg=#f2e9dc, cursor=#e2a35c, ANSI dari palet Dusk live/danger/accent). `FONT_MONO` const → JetBrains-Mono-stack. **Lock ke Dusk saja** — preset lain TIDAK merepaint terminal/editor (documented limitation §13.2, bukan bug).
- `qa-reports/headless-smoke.mjs` — cek baru: (a) `getPropertyValue('--bg')` == `#191512`; (b) `--accent` == `#e2a35c`; (c) grep anti-Abyss: NOL `#08080a`/`#00d992`/`#818cf8`/`var(--glow)` di stylesheet + main.ts bundle (INV-21); (d) box-shadow/elevation hanya pada drawer/modal + glow hanya 3 selector aktif.

**AC (EARS, VERIFIABLE):**
1. WHEN app me-render THE SYSTEM SHALL mengekspos SEMUA 16 token Dusk §13.3 di `:root` dengan nilai hex/rgba IDENTIK prototype (smoke `getComputedStyle` assert tiap token kritis).
2. WHEN `src/` di-grep THE SYSTEM SHALL mengandung NOL literal Abyss Emerald (`#08080a`, `#050507`, `#00d992`, `#818cf8`, `#0e2a20`, `var(--glow)`) — grep gate 0 hit (INV-21, **acceptance kritis**).
3. WHEN terminal/editor di-render THE SYSTEM SHALL memakai palet Dusk (xterm bg #14100d, cursor #e2a35c) — bukan palet emerald.
4. WHEN hover/idle THE SYSTEM SHALL TIDAK menampilkan glow; elevation (`--shadow-drawer`) hanya di drawer + settings modal (smoke box-shadow count).
5. WHEN font dimuat THE SYSTEM SHALL NOL network request (grep `url(http`/`@import` = 0).
6. Verify loop hijau: cargo == 78 (kanari, 0 diff backend), build 0, smoke 225 + ≥4 cek baru.

**Dilarang:** ubah geometri/layout (U-D2), sentuh `lib.rs`, tambah keyframe baru.

---

### U-D2 — Shell re-skin (sidebar / tab / split / status bar / SFTP)

**Sumber:** FR-SH1..8, SDD §13.3 geometri, prototype `proto-a-dusk.html` struktur. **Perilaku v0.11.0 DIPERTAHANKAN** — hanya skin/geometri/motion berubah.

**Cakupan file:** `src/styles.css` (geometri + motion), `src/main.ts` (bila markup geometri butuh adjust; perilaku existing UTUH), `index.html` (bila struktur shell perlu slot baru), smoke. **Tidak sentuh** `lib.rs`.

**Detail geometri (persis prototype §13.3):** sidebar 248px · host-row **34px** (Arus 28px → lebih lapang) · tab 32px / tab-bar 42px · status bar 24px (SAMA Arus) · term divider 4px `col-resize` · radius 10/8/6px · base font 13px lh 1.5. Motion: shimmer skeleton 1.4s, cursor blink 1.1s, row-actions fade .12s; `prefers-reduced-motion: reduce` mematikan SEMUA (FR-DT4/NFR-DK5). Drag divider/SFTP = **manual mouse** (INV-6, dilarang HTML5 DnD). Status bar format (FR-SH8): kiri `host · user@ip:port · connected`, kanan `N live · UTF-8 · LF · shell · clock` mono 11px.

**AC (EARS):**
1. WHEN host-list di-render THE SYSTEM SHALL menampilkan `.host-item` tinggi **34px** terukur (smoke), group label prefix-derived, dot live(`--live`)/idle, auth icon, row-actions on hover/focus-within (FR-SH2/3).
2. WHEN host gagal auth THE SYSTEM SHALL menampilkan error row + Retry tanpa menghapus row lain (FR-SH4) — perilaku v0.11.0 utuh.
3. WHEN tab aktif THE SYSTEM SHALL underline `--accent` 2px; dead tab `--danger` + reconnect; split drop-zone di akhir (FR-SH5) — grep `draggable` = 0.
4. WHEN divider di-drag (smoke mousedown/mousemove/mouseup) THE SYSTEM SHALL resize manual mouse; dblclick → 50/50 (FR-SH6).
5. WHEN status bar di-render THE SYSTEM SHALL format FR-SH8 persis, mono 11px.
6. WHEN `prefers-reduced-motion` aktif THE SYSTEM SHALL mematikan shimmer/blink/fade (smoke media-query).
7. Height chain utuh: `.sftp-wrap`/`.ed-wrap` height:100% setelah status bar 24px (smoke offsetHeight panel == panels container).
8. NOL literal Abyss tersisa (INV-21 re-assert); verify loop hijau + smoke ≥6 cek baru; cargo == baseline.

**Pitfall:** `pasteNormalize` off-limits (0 diff); hover-reveal tetap keyboard-reachable; copy empty/error naratif pindah rumah boleh, ubah kata TIDAK.

---

### U-D3 — Store backend (3 IPC command + schema + asset)  *[file-disjoint, ∥ U-D1/U-D2]*

**Sumber:** SDD §13.4/§13.5/§13.7 (frozen verbatim), ADR-18/19/21, INV-22/24, FR-SET13/FR-SN6/FR-AI6/7/10.

**Cakupan file:** `src-tauri/src/lib.rs` (3 command baru + struct `Settings`/`AllowEntry`/`SnippetGroup`/`Snippet` + resolver path + cargo tests), `src/assets/snippets.default.json` (bundled, embedded via `include_str!` atau asset bundler), `src-tauri/tauri.conf.json` bila perlu daftarkan asset. **Tidak sentuh** `styles.css`/`main.ts` (frontend konsumsi di U-D4/U-D5/U-D7).

**Detail (SDD §13.5 verbatim — a change here = addendum revision, bukan drift):**
- `read_settings() -> Settings` — NEVER throws; missing/corrupt → `DEFAULT_SETTINGS` (theme=dusk, accent=#e2a35c, allowlist=[]) (INV-24). Unknown `version`/`theme` → coerce dusk; invalid accent → preset default.
- `write_settings({settings}) -> null` — atomic temp→rename, 0600-class. Re-validasi: theme enum, accent 6-hex, allowlist entry non-empty host+command. Reject `settings:invalid-accent` | `settings:invalid-theme` | `settings:write-failed`. NEVER baca/tulis `~/.ssh/config` atau sidecar (INV-22). `added_at` backend-stamped RFC3339 UTC.
- `read_snippets() -> SnippetGroup[]` — v1 bundled read-only; never throws (empty array on fail).
- Path = `dirs::config_dir()/hosterm/settings.json`; `HOSTERM_DATA_DIR` override test hook.

**Bundled snippets** `src/assets/snippets.default.json` = min 3 kategori (Docker/Git/systemd) pola SDD §13.4 shape `[{category, items:[{name,command}]}]`.

**AC (EARS):**
1. WHEN `read_settings` dipanggil tanpa file THE SYSTEM SHALL mengembalikan `DEFAULT_SETTINGS` tanpa panic (cargo test).
2. WHEN `settings.json` corrupt THE SYSTEM SHALL mengembalikan defaults DAN TIDAK menimpa file (INV-24 — cargo test assert byte unchanged).
3. WHEN `write_settings` dengan accent non-6-hex THE SYSTEM SHALL reject `settings:invalid-accent` tanpa menulis file (cargo test).
4. WHEN `write_settings` sukses THE SYSTEM SHALL menulis atomic DAN `~/.ssh/config` + sidecar hash/mtime UNCHANGED (INV-22 — cargo test FS diff, **acceptance kritis**).
5. WHEN `read_snippets` dipanggil THE SYSTEM SHALL mengembalikan ≥3 kategori dari bundled asset, 0 network.
6. IPC inventory `grep -rc "tauri::command" src-tauri/src/*.rs` == **27** (lib.rs 25, fleet.rs 2).
7. Verify loop: **cargo 78 + ≥4 test baru**, build 0, smoke == baseline (0 diff frontend).

**Pitfall:** config parser `~/.ssh/config` SACRED — store tak boleh menyentuhnya; NOL dep crate baru (pakai `dirs` existing, ADR-18).

---

### U-D4 — Settings panel + Appearance

**Sumber:** FR-SET1..12, SDD §13.2/§13.2.1 (ADR-20 Rev. 2 preset + INV-23 accent-ink mechanism) + §13.4 (persist), prototype `proto-a-dusk-settings.html` (preset-grid + accent-row + live-preview markup = visual truth), INV-23, `[ASSUME-OQ-2]`.

> **Kanonisitas:** SDD §13.2.1 / ADR-20 Rev. 2 adalah sumber kanonik untuk aturan, angka, dan vector INV-23. Jika wording TSD ini konflik dengan SDD, **SDD yang berlaku**.

**Cakupan file:** `src/main.ts` (Settings modal open/close, nav, preset picker, accent control + contrast guard, live preview, persist via `write_settings`/boot via `read_settings`), `index.html` (markup modal), `src/styles.css` (`.preset`/`.accent-chip`/`.set-nav`/live-preview + scrim). Deps: U-D1 (token) + U-D3 (store).

**Detail (prototype markup verbatim):**
- Modal: gear sidebar footer → modal in-app `role=dialog aria-modal=true`, scrim `rgba(12,9,6,.55)`, 900×620 (max 94vw×92vh). BUKAN native/window (FR-SET1).
- Nav kiri: General/Appearance/Terminal/SSH&Keys/AI Assistant/Snippets/Account; Appearance `aria-current=page` default; non-Appearance (dan Approvals, kecuali revoke list U-D7) = placeholder "coming" (FR-SET2/3).
- Preset grid 4 kolom `.preset` (markup: `aria-pressed`, `.preset-swatch` style `--pb/--ps/--pa/--pt`, `.preset-check`): **Dusk FUNGSIONAL** (klik → aktif); **Slate/Ink/Light = placeholder "coming" `aria-disabled="true"`** — klik TIDAK swap `data-theme` (`[ASSUME-OQ-2]`, SDD §13.2 expose gate). Mekanisme swap = `setAttribute("data-theme", preset)` (ADR-20).
- Accent: `input[type=color]#accent-input` + `.color-hex` + 5 `.accent-chip` (data-accent amber/teal/rose/lime/violet). Pilih chip/hex valid → `document.documentElement.style.setProperty("--accent", hex)` + live preview + `aria-pressed` match (FR-SET6/7). **Recolor accent SAJA** (surface/text/status tetap, FR-SET8).
- **Contrast guard (INV-23, SDD §13.2.1; ADR-20 Rev. 2):** normalisasi setiap accent valid ke lowercase `#rrggbb`. Exact curated set `{#e2a35c,#5fbfae,#e0869b,#aec267,#b99be3}` selalu memakai `--accent-ink:#241404`. Untuk semua nilai non-curated, hitung WCAG sRGB relative luminance: per channel `s = channel / 255`, `linear(s) = s / 12.92` bila `s <= 0.04045`, selain itu `((s + 0.055) / 1.055) ^ 2.4`; lalu `L = 0.2126R + 0.7152G + 0.0722B`. Pilih `#000000` bila `L >= Lswitch`, selain itu `#ffffff`, dengan `Lswitch = sqrt(0.0525) - 0.05 = 0.179128784747792` (tie → dark; membandingkan ratio unrounded lebih disukai). Mapping = pure function dari normalized value, bukan interaction source (chip/free-input/boot/reload). Accent valid WAJIB diterapkan + dipersist apa adanya: NOL clamp/remap/reject. Pair free-color lama `#241404/#f2e9dc` dilarang; safe endpoints non-curated hanya `#000000/#ffffff` dan menjamin minimum `sqrt(21) = 4.58257569495584:1`.
- **Frozen regression vectors (byte-exact; mirror SDD §13.2.1):**

  | Class | Accent | Expected ink | Expected contrast |
  |---|---:|---:|---:|
  | curated | `#e2a35c` | `#241404` | `8.18060039728731:1` |
  | curated | `#5fbfae` | `#241404` | `8.115110618605044:1` |
  | curated | `#e0869b` | `#241404` | `6.842802495508517:1` |
  | curated | `#aec267` | `#241404` | `9.102683799795606:1` |
  | curated | `#b99be3` | `#241404` | `7.529220380096464:1` |
  | free extreme | `#000000` | `#ffffff` | `21:1` |
  | free existing dark sentinel | `#101010` | `#ffffff` | `19.028110547666497:1` |
  | free QA failure | `#6f6f6f` | `#ffffff` | `5.024865064757633:1` |
  | free grayscale below switch | `#757575` | `#ffffff` | `4.607518093747377:1` |
  | free closest 24-bit value below switch | `#766cb5` | `#ffffff` | `4.58257700240974:1` |
  | free closest 24-bit value above switch | `#cf0dcc` | `#000000` | `4.5825758149421025:1` |
  | free grayscale above switch | `#767676` | `#000000` | `4.6232848849972035:1` |
  | free QA failure | `#777777` | `#000000` | `4.68949989000882:1` |
  | free extreme | `#ffffff` | `#000000` | `21:1` |
- Live preview: tab + primary button + prompt line pakai accent terpilih, real-time (FR-SET10).
- "Reset to Dusk default" → accent `#e2a35c` + preset Dusk (FR-SET11). Done/scrim-click/Escape tutup + pertahankan pilihan (FR-SET12). Persist via `write_settings` (FR-SET13).

**AC (EARS):**
1. WHEN gear ditekan THE SYSTEM SHALL membuka modal in-app `role=dialog aria-modal=true` + scrim, bukan native (smoke).
2. WHEN Settings terbuka THE SYSTEM SHALL menampilkan 7 nav item, Appearance `aria-current=page` default (FR-SET2).
3. WHEN preset Dusk dipilih THE SYSTEM SHALL `aria-pressed=true` + check mark; WHEN Slate/Ink/Light THE SYSTEM SHALL `aria-disabled=true` DAN klik TIDAK meng-set `data-theme` (smoke assert, `[ASSUME-OQ-2]`).
4. WHEN chip teal dipilih THE SYSTEM SHALL set inline `--accent #5fbfae` + chip `aria-pressed=true` + preview update; surface/text TIDAK berubah (FR-SET8, smoke).
5. WHEN seluruh frozen regression vector di atas dijalankan lewat Settings color-input path THE SYSTEM SHALL menghasilkan expected ink byte-exact DAN independent test calculation SHALL membuktikan ratio ≥ 4.5:1. Coverage WAJIB mencakup kedua former failures (`#6f6f6f`, `#777777`), kedua closest switch sides (`#766cb5`, `#cf0dcc`), grayscale sides (`#757575`, `#767676`), extremes (`#000000`, `#ffffff`), existing sentinel `#101010`, dan kelima curated values (**acceptance kritis**).
6. WHEN normalized curated value yang sama masuk lewat chip ATAU free color-input THE SYSTEM SHALL selalu memilih `#241404`; WHEN normalized non-curated value masuk lewat free-input/boot/reload THE SYSTEM SHALL selalu memilih endpoint black/white yang sama — interaction source TIDAK boleh memengaruhi mapping.
7. WHEN accent valid diubah lalu Done THE SYSTEM SHALL memanggil `write_settings` dengan normalized chosen accent TANPA clamp/remap/reject dan mempertahankan pilihan (smoke `calls` assert).
8. WHEN saved accent dimuat ulang THE SYSTEM SHALL `read_settings`, apply accent yang sama, lalu derive ink deterministically dari value yang sama; smoke WAJIB membuktikan input → persist → reload untuk sedikitnya satu curated value dan satu non-curated value.
9. Verify loop hijau + smoke bertambah sesuai seluruh vector + determinism coverage; cargo == baseline. Re-measure total, jangan hard-code count lama.

---

### U-D5 — Snippet panel

**Sumber:** FR-SN1..6, SDD §13.4 (`read_snippets`), prototype `proto-a-dusk.html` drawer-snippets markup (`.snip-filter`/`.snip-group-label`/`.snippet-item`/`.snip-name`/`.copy-btn`/`.snip-cmd`/`.snip-empty`). Deps: U-D3.

**Cakupan file:** `src/main.ts` (buka drawer 340px, render dari `read_snippets`, filter substring, copy per-item + `.done` 900ms, head-count), `index.html` (drawer markup), `src/styles.css` (snippet styles), smoke.

**Detail (prototype verbatim):** drawer 340px `aria-label="Command snippets"`, mode-btn `aria-pressed`. Grup per kategori (`.snip-group-label` uppercase) + item (`.snip-name` + `.snip-cmd` code + `.copy-btn`). Filter substring (nama + command); 0 match → `.snip-empty` (`No snippets match "<q>"`). Copy → clipboard + `.copy-btn.done` (`--live`) ~900ms. Head-count = jumlah terlihat (FR-SN4 should). **Escape semua teks via `esc()`** (FR-SN5, INV-11-family — snippet command = render ke DOM).

**AC (EARS):**
1. WHEN panel Snippets dibuka THE SYSTEM SHALL `read_snippets` lalu render grup kategori + item mono (FR-SN1, smoke `calls`).
2. WHEN filter diketik THE SYSTEM SHALL substring nama+command; 0 match → empty-state `No snippets match "<q>"` (FR-SN2).
3. WHEN copy ditekan THE SYSTEM SHALL `clipboard_write` + `.copy-btn.done` ~900ms (FR-SN3, smoke).
4. WHEN command di-render THE SYSTEM SHALL escape (fixture hostile `<img src=x onerror=…>` → `.snip-cmd` childCount 0, `__pwn` undefined, CSP 0) (FR-SN5, **acceptance kritis**).
5. Verify loop hijau + smoke ≥4 cek baru; cargo == baseline.

---

### U-D6 — Google login VISUAL

**Sumber:** FR-GL1..5, ADR-22, `[ASSUME-OQ-3]`, prototype `proto-a-dusk.html` (`.user-chip`/`.avatar`/`.gmark`/`.signin-popover`/`.signin-btn`). **VISUAL ONLY — NOL backend OAuth.**

**Cakupan file:** `src/main.ts` (toggle chip↔popover state UI-only), `index.html` (user-chip + popover markup), `src/styles.css` (chip/popover/signin-btn — Google brand: surface `#ffffff`, teks `#1f1f1f`, radius 4px), smoke.

**Detail (prototype verbatim):** signed-out → `.user-chip` `aria-haspopup=dialog` membuka `.signin-popover role=dialog "Use hosterm with Google"` + `.signin-btn "Sign in with Google"` (putih #ffffff, radius 4px brand spec, BUKAN `--r-*`). signed-in → chip (avatar inisial + nama + `.gmark` Google SVG + caret). Toggle = mock state UI-only; **TIDAK mengklaim sesi tersaut tanpa auth nyata** (FR-GL3). NOL token/kredensial ke `~/.ssh/config` (FR-GL5, INV-22-class).

**AC (EARS):**
1. WHEN signed-out THE SYSTEM SHALL menampilkan user-chip yang membuka popover + signin-btn (surface #ffffff, radius 4px) (FR-GL1).
2. WHEN signed-in (mock) THE SYSTEM SHALL chip avatar+nama+gmark+caret (FR-GL2).
3. WHEN signin-btn ditekan THE SYSTEM SHALL toggle state UI-only; TIDAK ada invoke OAuth/network (smoke: 0 network, no claim sesi) (FR-GL3, ADR-22).
4. WHEN sign-out THE SYSTEM SHALL kembali ke signed-out (FR-GL4).
5. NOL token Google di `~/.ssh/config` path (grep); verify loop hijau + smoke ≥3 cek baru; cargo == baseline.

---

### U-D7 — AI assistant + approval VISUAL (SECURITY-SENSITIVE)

**Sumber:** FR-AI1..13, SDD §13.6 (kontrak approval FROZEN) + §13.4 (allowlist persist), INV-25/26, ADR-21/22, prototype `proto-a-dusk.html` (`.cmd-card`/`.cmd-card.awaiting`/`.cmd-state.warn/.ok`/`.btn-primary.approval-run`/`.btn-secondary.approval-always`/`.btn-ghost.approval-reject`). Deps: U-D1 + U-D3. **PALING AKHIR fungsional, paling security-sensitive.**

**Cakupan file:** `src/main.ts` (AI drawer, thread, command card state machine, allowlist via `read_settings`/`write_settings`, Run-once lewat terminal real-ssh existing, revoke list), `index.html` (AI drawer + Settings>Approvals markup), `src/styles.css` (cmd-card/btn-*), smoke. **NOL backend AI — thread = stub/mock; eksekusi = terminal real-ssh existing (ADR-1/2, bukan jalur baru).**

**Detail (SDD §13.6 frozen + prototype verbatim):**
- Panel: thread (user+AI), command card, input "Ask hosterm". AI head-tag "on-device" (FR-AI12 — jangan klaim on-device bila nyatanya tak ada backend; v1 = stub/mock thread).
- Command card (FR-AI2): scope host (alias persis), command line mono, deskripsi dampak, state badge.
- State machine (SDD §13.6 frozen): `proposed → AWAITING APPROVAL (.cmd-card.awaiting + .cmd-state.warn)` → **Run once** (`btn-primary.approval-run`, execute via terminal, allowlist UNCHANGED) / **Always allow** (`btn-secondary.approval-always`, persist AllowEntry + execute → `.cmd-state.ok ALWAYS ALLOWED`) / **Reject** (`btn-ghost.approval-reject`, no execute, no write). Exact `(host,command)` match AllowEntry → render langsung ALWAYS ALLOWED (FR-AI7).
- **INV-25 (acceptance KRITIS):** "Run once" = `btn-primary` (accent, default mata/klik), "Always allow" = `btn-secondary` (`--raised` muted). Permission-widening action TAK PERNAH primary — smoke class assertion, bukan CSS drift.
- FR-AI9: NOL auto-run implisit; eksekusi HANYA via (a) klik Run once (b) exact allowlist re-match.
- **INV-26:** host alias lewat `sftp_safe` guard (leading `-`/CR/LF refuse) sebelum execute (FR-AI13, reuse Fleet INV-19).
- **FR-AI11/INV-11-family:** thread text + output remote via `esc()`/`escAttr()` (hostile input).
- Allowlist = exact `(host,command)` byte-match, persist `settings.json.allowlist` via `write_settings` (ADR-21). Settings>AI/Approvals = read+revoke list (FR-AI10 — render entry + revoke=remove+write_settings, BUKAN pattern editor).

**AC (EARS):**
1. WHEN panel AI dibuka THE SYSTEM SHALL thread + command card + input (FR-AI1).
2. WHEN command card awaiting THE SYSTEM SHALL `.cmd-card.awaiting` + `.cmd-state.warn "AWAITING APPROVAL"` + 3 aksi (FR-AI3).
3. WHEN approval card di-render THE SYSTEM SHALL Run-once=`btn-primary`, Always-allow=`btn-secondary`, Reject=`btn-ghost` (INV-25 — smoke class assert, **acceptance KRITIS**).
4. WHEN Run once ditekan THE SYSTEM SHALL eksekusi SEKALI via terminal real-ssh (smoke: `pty_write` path tercatat) tanpa menulis allowlist (FR-AI5).
5. WHEN Always allow ditekan THE SYSTEM SHALL `write_settings` dengan AllowEntry `(host,command)` persis + execute → `ALWAYS ALLOWED` (FR-AI6, smoke `calls`).
6. WHEN `(host,command)` exact match allowlist THE SYSTEM SHALL render `.cmd-state.ok ALWAYS ALLOWED` tanpa approval ulang (FR-AI7).
7. WHEN Reject THE SYSTEM SHALL no execute + no allowlist write (FR-AI8).
8. WHEN host alias leading `-`/CR/LF THE SYSTEM SHALL refuse sebelum execute (INV-26, smoke).
9. WHEN output/thread di-render THE SYSTEM SHALL escape (hostile fixture → childCount 0, `__pwn` undefined, CSP 0) (FR-AI11).
10. WHEN Settings>Approvals dibuka THE SYSTEM SHALL read+revoke list; revoke → `write_settings` tanpa entri (FR-AI10).
11. Verify loop hijau + smoke ≥8 cek baru; cargo == baseline.

---

### U-D8 — Smoke canonization + docs (STATE.md)

**Sumber:** NFR-DK8, AGENTS.md verify loop #4, Arus U9 pola, SDD §13.8/§13.9 pt 8 + OQ-1 (STATE.md update saat token unit landed → di sini saat semua landed).

**Cakupan file:** `qa-reports/headless-smoke.mjs` (kanonisisasi + audit kelengkapan + dedup) + `docs/STATE.md` (update: §layout Dusk token notes, Arus → historis per OQ-1; verify loop baseline baru; invariant INV-21..26 flow in).

**Detail:** pastikan tiap fitur U-D1..U-D7 punya cek eksplisit. Audit lintas-fitur: (a) grep `draggable` = 0 (NFR-DK4/INV-6); (b) copy empty/error byte-identical baseline; (c) `prefers-reduced-motion` kill-switch (NFR-DK5); (d) 0 CSP violation + hostile fixture AI/snippet (NFR-DK9/INV-11); (e) INV-21 anti-Abyss grep; (f) INV-23 kontras ink; (g) INV-25 visual-weight. STATE.md: tulis baseline cargo/smoke/IPC-count FINAL terukur.

**AC (EARS):**
1. Semua cek U-D1..U-D7 tercatat PASS; total smoke ≥ baseline + 30.
2. Grep `draggable` di `src/` = 0 (smoke assert).
3. STATE.md §layout + verify loop + invariants di-update ke angka terukur; Arus token notes ditandai historis (OQ-1).
4. Verify loop hijau penuh; hasil dilampirkan ke task.

---

## 3. Per-unit: cargo baru vs smoke-only

| | cargo test BARU | smoke-only |
|---|---|---|
| **cargo + smoke** | **U-D3** (store: round-trip, corrupt→defaults INV-24, config/sidecar byte-identity INV-22, validasi reject) — **+≥4** | — |
| **smoke-only** (cargo kanari == baseline) | — | U-D1, U-D2, U-D4, U-D5, U-D6, U-D7, U-D8 |

Hanya **U-D3** menambah `cargo test --lib`. Semua unit frontend = smoke-only, cargo WAJIB tetap ≥ baseline (78) sebagai kanari (0 diff backend). **Baseline verify loop RE-MEASURE saat dispatch tiap unit** (angka STATE.md kanonik; live @31bef63 = cargo 78 / build 0 / smoke 225 / IPC 24).

---

## 4. Invariant yang WAJIB tetap hijau tiap unit

**STATE.md Hard invariants 1–6 (never break, tiap unit):** (1) `~/.ssh/config` parseable + (2) round-trip comments/order/unknown + no secret in config · (3) paste PTY CRLF→LF · (4) drag = manual mouse, NOT HTML5 DnD · (5) staging grep `target/|node_modules|.exe$|dist/` · (6) authorized_keys dev tak disentuh.

**SDD INV-21..26 (Dusk, acceptance per unit):**

| INV | Unit utama | Cek | Kritis? |
|---|---|---|---|
| INV-21 anti-Abyss | U-D1 (set), U-D2/U-D8 (re-assert) | grep 0 literal Abyss (`#08080a`/`#00d992`/`var(--glow)`) + smoke `:root` token | **YA** |
| INV-22 store isolation | U-D3 | cargo: config+sidecar hash/mtime unchanged across write | **YA** |
| INV-23 accent-ink kontras | U-D4 | smoke seluruh frozen vector SDD §13.2.1 (curated, former failures, kedua switch sides, extremes), independent WCAG ratio calculation, + input/persist/reload determinism | **YA** |
| INV-24 corrupt → defaults | U-D3 | cargo: corrupt file → defaults, byte unchanged | ya |
| INV-25 visual-weight approval | U-D7 | smoke class: Run-once=btn-primary, Always-allow=btn-secondary | **YA** |
| INV-26 host alias safe-guard | U-D7 | smoke: leading `-`/CR/LF refuse sebelum execute | ya |

**Carried Dusk NFR (SDD §13.8):** INV-DnD (grep `draggable`=0), reduced-motion kill-switch (NFR-DK5), zero-network font (NFR-DK6), CSP 0 + hostile fixture (NFR-DK9). **Grep gate anti-Abyss (INV-21) + visual-weight (INV-25) = acceptance kritis** — unit tak boleh PASS tanpa keduanya hijau.

---

## 5. Risks & waivers

| Risiko/waiver | Status | Basis | Cadangan |
|---|---|---|---|
| Manual Windows/WebView2 (DnD manual U-D2 divider/SFTP; snippet reorder bila ada) — **WAIVER pola Arus** | Terbuka, risiko rendah | Tak ada mesin Windows tersedia; drag = manual mouse-event (bukan HTML5 DnD mati di WebView2) + smoke headless menutup logika; sisa risiko = perilaku spesifik-WebView2 | Uji manual Windows asli pasca-rilis → patch bila defek. Row baru STATE.md §Risks pola waiver U4/U5 Arus |
| xterm/CM TIDAK repaint antar-preset (SDD §13.2 pt 5) — **DOCUMENTED LIMITATION, bukan bug** | Diterima v1 | Literal JS (bukan CSS var); terminal well stay Dusk-dark lintas preset; preset-aware xterm/CM butuh AA pass per preset×ANSI-16 = deferred dengan expose-gate `[ASSUME-OQ-2]` | Fase berikut saat preset non-Dusk di-expose fungsional (butuh AA penuh dulu) |
| Preset non-Dusk = placeholder "coming" (`[ASSUME-OQ-2]`) — belum AA-verified full-shell | Gated | NFR-DK2: hanya swatch+accent terverifikasi, bukan seluruh pasangan shell; expose fungsional = prasyarat AA penuh lead/architect | ADR-20 bikin drop-in; MEKL konfirmasi ship placeholder vs hide |
| Google OAuth + AI backend = fase berikut (`[ASSUME-OQ-3/4]`, KEPUTUSAN MEKL 31 Okt visual-dulu) | By design v1 | ADR-22: UI + kontrak approval §13.6 frozen; backend = epic sendiri. Run-once lewat terminal real-ssh existing (bukan jalur baru) | Epic backend terpisah; kontrak approval sudah frozen siap backend |
| Store tak ikut `~/.ssh` portability (NFR-8, `[ASSUME-OQ-5]`) | By design | Store di `~/.config/hosterm/` di luar ssh dir (INV-22 class) — device switch bawa hosts+keys, reset theme/allowlist kecuali user copy app-data | MEKL konfirmasi ride-along vs machine-local (§13.4) |

---

## 6. Out of scope (v1 — keputusan MEKL)

Token editor penuh · import/export tema · light-dark toggle di luar preset · pattern/glob di Always-allow (`[ASSUME-OQ-6]` exact-only) · backend OAuth nyata + cross-device sync · backend AI nyata / pilih model · persistensi snippet user-editable (v1 read-only bundled) · section Settings non-Appearance berisi kontrol fungsional (placeholder) · `~/.ssh/config` untuk data non-ssh (HARAM, INV-1).

---

*Sources: docs/SDD.md §13 (ADR-18..22, INV-21..26, §13.2–13.9) · docs/FEATURE-SPEC-redesign-dusk-v1.md (FR/NFR/OQ) · docs/UX/prototypes/proto-a-dusk.html + proto-a-dusk-settings.html + verification-report.json (visual truth, APPROVED MEKL) · docs/EPIC-TSD-redesign-arus-v1.md (pola 9-unit serial) · docs/STATE.md (invariants + verify loop). Baseline verify loop RE-MEASURED live di working tree HEAD 31bef63 pada 2026-10-31: cargo 78 / build 0 TS / smoke 225 / IPC-count 24. Angka STATE.md kanonik, bukan angka spec/PRD lama. Keputusan MEKL 31 Okt (OQ-3/OQ-4 visual-dulu) dikonfirmasi via nano.*
