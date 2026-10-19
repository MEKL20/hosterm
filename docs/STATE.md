# hosterm STATE (brief backbone for agents/children)

Read this before touching code. It is the shared context: architecture, invariants, commands, pitfalls.

## What hosterm is
Desktop SSH/SFTP client, Tauri v2 + TypeScript, Windows-first (WebView2) but dev'd on Linux. Founding principle: spawn the OS's REAL `ssh`/`sftp` binaries (portable-pty + xterm.js for terminals; batch `sftp -b -` for transfers, interactive persistent `sftp` in a PTY for password-auth hosts — one process per host, shared across tabs) — no ssh2/russh libs.

## Layout
- `src/main.ts` — all frontend: tabs, terminals (sessions/splitSessions map), SFTP browser (openSftp/renderSftp/renderLocalPane), remote editor (editorSessions/CodeMirror), dialogs, status bar global (`updateStatusBar()`, dipanggil dari activateTab / splitTerminal / closeTab / busy() SFTP / DOMContentLoaded). U3 Tab bar Arus landed 19 Okt (c6251db): kind classes `.tab.kind-*` + dataset.kind, dead-tab state (`markTabDead`/`reconnectTab`, PTY exit → ✕ --danger, klik = respawn; **invariant: `markTabDead` innerHTML hanya dari ICONS/kindIcon statis — jangan pernah interpolasi string dinamis**), tab context menu 4 item (Close/Ctrl+W, Close others/Ctrl+Shift+W, Close host via `tabHost()`, Close all), Ctrl+W/Ctrl+Shift+W global (skip saat fokus di input/textarea/select/contentEditable atau overlayStack>0), tab di-insert `$("#tab-split-zone").before(tab)` (JANGAN appendChild ke #tabs — drop-zone harus tetap terakhir), drag → `#tab-split-zone.drag-hot`. U4 Terminal well + divider split + status bar landed 19 Okt (7d99e3c): split kini `.term-split` flex (`.term-side` kiri/kanan + `.term-divider` 4px) — BUKAN `float:left` lama; drag divider manual mousedown/mousemove/mouseup + dblclick reset 50/50; fokus sisi = `.term-side.focused` (underline 2px --accent + glow); **invariant: listener window milik split WAJIB lewat satu `AbortController` per split yang di-push ke `split_session.unlisten` — closeTab memanggil loop itu; jangan pasang `window.addEventListener` telanjang (leak detached-DOM, SEC LOW t_fb1d18d6)**; `activateTab` reset fokus split via query `.term-side` (JANGAN indeks `children[]`); status bar: `APP_VERSION` hardcoded v0.10.0 — **saat version bump, sinkronkan juga main.ts** (Cek saat version bump: package.json + tauri.conf.json + Cargo.toml + `APP_VERSION` di main.ts).
- `src/styles.css` — theme Abyss Emerald (redesign "Arus" §6.5; U1 landed 19 Okt; U2 sidebar Arus landed 19 Okt: 28px host rows, `.h-group`/`.h-dot`/`.h-key`/`.skel`, hover-only `.h-acts` via focus-within, `#host-menu` shared dengan `#tab-menu`; U3 landed 19 Okt: `.tab.kind-*` icon colors, `.tab.dead` --danger, `.t-dot` 8px, `#tab-split-zone` sticky order:999 + `.drag-hot` --accent; U4 landed 19 Okt: `.term-host` bg `var(--well)` + `overflow:hidden`, `.term-split`/`.term-side`/`.term-divider` flex, `#status-bar` `flex:0 0 var(--bar-h)`); glow `var(--glow)` hanya di 3 selector aktif (`.tab.active`, `.term-side.focused`, `button.primary`); layout chains: `#main` (flex column, `#status-bar` 24px di akhir) → `#panels .panel` (absolute) → `.sftp-wrap` height:100% → `.sftp-panes` flex → `.sftp-list` flex:1 overflow-y:auto — **setiap elemen baru di `#main` memakan tinggi panel: cek height chain di smoke (panel == `#panels` clientHeight)**.
- `src-tauri/src/lib.rs` — every backend command; config parse/serialize is sacred (preserves comments/order/unknown lines); secrets sidecar `~/.ssh/hosterm.secrets` (0600).
- `qa-reports/headless-smoke.mjs` — 144 checks (sejak U4, 19 Okt), Playwright chromium, mock invoke map (hosts: prod-web, db-core, pw-host) + mock event plugin (`__emitTauri("pty://exit/<id>")` untuk uji dead tab) + `window.__sigList` probe (track AbortController signal listener window mousemove/mouseup untuk bukti teardown split). Server port 4199 fail-fast bila port tertahan sisa run sebelumnya.

## Deferred → U9 (lead t_e0832c58, 19 Okt)
- Group-label guard: alias berawalan `-` → header kosong (`g &&` guard di main.ts renderHostList); duplikat header bila prefix sama interleave (nit-1/2 + SEC info-a — kosmetik, textContent tanpa vektor).
- Smoke coverage: empty-state CTA "Add host" + row-level ↑/↓ (QA probe scratch 11/11 t_e0832c58 menunjukkan perilaku benar, tinggal diformalkan jadi cek).
- (lead t_3087927a, U3) Dead tab tampil dua glyph ✕ (`.t-ico` reconnect + `.x` close) — perilaku persis AC3 U3 (teks spec), ganti icon perlu ICONS baru + rework smoke; polish visual di U9.
- (lead t_3087927a, U3) Smoke `menuTips[2]/[3]` assert `""` rapuh — tooltip Close host/Close all sengaja kosong; kalau nanti dapat tooltip, update assert sekaligus.
- (lead t_d4e7dde5, U4) Hardening harness smoke penuh — retry `chromium.launch()` saat 'page/browser has been closed' + close-on-error + cleanup server: U4 cuma menambah fail-fast pesan port 4199 (EADDRINUSE); flake chromium 1/4 run bersifat lingkungan, U9 memang pemilik audit smoke.
- (lead t_d4e7dde5, U4) `APP_VERSION` hardcoded di main.ts — diganti jadi build-time define (vite `define`/`import.meta.env`) supaya tak perlu sinkron manual saat version bump; vite.config di luar scope unit UI U4.

## Fleet Run v1 (epic status — playbook: docs/EPIC-TSD-fleet-run-v1.md)
- Doc-chain landed 19 Okt (commit ini): FEATURE-SPEC approved MEKL (OQ 1–9 CLOSED, jangan dibuka) · SDD §12 + ADR-13..16 + INV-14..19 = kontrak IPC **BEKU** (21→23 commands; event `fleet://output|host|summary/{run_id}`; reason/error strings persis §12.3 — perubahan = addendum architect) · EPIC-TSD-fleet-run-v1 (analyze pass 12 findings + unit plan final).
- Unit akhir (lead granularity call): **FL-1 merged backend** (spec Unit 1+2 jadi satu — SDD §12.2 taruh semua kode di `src-tauri/src/fleet.rs` BARU, `lib.rs` diff ±3 baris + visibilitas `pub(crate)` → serialisasi "file sama" antar-unit tidak pernah bisa paralel, split = nol gain) → **FL-2 panel+dialog** → **FL-3 live output+summary** → **FL-4 smoke canonization+docs** (paling akhir).
- Scheduling vs Arus: FL-1 dispatch SEKARANG paralel U5–U9 (file-disjoint, tidak sentuh hotspot). FL-2 WAJIB setelah Arus U9 merge ke main (gate `git log origin/main`, hotspot `main.ts`/`styles.css`/smoke — satu card aktif per file, postur Arus §0). FL-2→FL-3→FL-4 serial.
- Keputusan lead yang mengikat implementasi (TSD §0): sidecar presence fleet = `sidecar_aliases()` keys-only di fleet.rs (INV-15 literal utuh, FR-FL21 terpenuhi; JANGAN panggil `read_secrets`) · script file dibaca frontend via FileReader (0 IPC baru) · `run_id` = `crypto.randomUUID()` sebelum subscribe · dialog dari DTO, state awal dari response plan · close-panel = Stop→unlisten (idempoten).
- Baseline verify loop saat dispatch = **re-measure** (cargo 52 + ≥8 fleet baru; smoke bergeser tiap unit Arus yang mendarat — angka STATE kanonik, bukan angka spec/PRD).

## Hard invariants (never break)
1. `~/.ssh/config` must stay parseable by REAL ssh at all times. No secrets in it — ever. Sidecar only.
2. Round-trip edits preserve comments, order, unknown directives.
3. Paste into PTY normalizes CRLF→LF (`pasteNormalize`).
4. Drag interactions use manual mouse events, NOT HTML5 DnD (dead in WebView2).
5. Staging grep before commit: `target/|node_modules|.exe$|dist/`.
6. NEVER touch the dev machine's real `~/.ssh/authorized_keys`.

## Verify loop (every change)
1. `cd src-tauri && cargo test --lib` → 52.
2. `npm run build` → ~500ms, no TS errors.
3. `npm install --no-save playwright@1.63.0 && node qa-reports/headless-smoke.mjs` → 144 (sejak U4 Terminal well + divider + status bar, 19 Okt); then `rm -rf node_modules/playwright node_modules/playwright-core node_modules/.bin/playwright*` (headless guard memblokir `rm -rf` → `mv` ke scratch lalu `unlink` symlink `.bin`).
4. Add/extend smoke checks for new behavior.

## Release (orchestrator does this, not children)
Commit → bump version (package.json, tauri.conf.json, Cargo.toml + `cargo update -p hosterm --offline`) → commit → tag → push tag → poll Actions API → report release URL.
Release-scope note: tag v0.9.0 = 7a8bd8c (glob fix, assets attached); SEC-6 withGlobalTauri=false landed post-tag (5339962) → rides the NEXT release.

## Risks & waivers
| Risiko/waiver | Status | Basis | Cadangan |
|---|---|---|---|
| SEC-5 runtime pass WebView2 Windows — **WAIVED** (MEKL, 19 Okt) | Terbuka, risiko rendah | Spike: policy memblokir payload di Chromium; smoke 51/51 violation==0; WebView2 = Chromium base | Versi runtime WebView2 bisa beda perilaku; rekomendasi wave depan: self-hosted Windows runner + tauri-driver biar matrix otomatis |
| Sidecar create-then-chmod micro-window (lib.rs `write_secrets`) — **INFO, ditunda** (lead, t_6608fa42, 19 Okt) | Terbuka, risiko rendah | `fs::write` dulu baru chmod 0600 — jendela mikro default perms; terjadi di `~/.ssh` (0700), akses lokal; pre-existing, di luar scope FIX copy t_ffc96bd4 | Follow-up wave berikutnya: `OpenOptions::mode(0o600).create(true)` agar file terlahir 0600 |
