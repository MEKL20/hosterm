# U-D7 QA Verify — AI assistant + approval VISUAL (working tree, uncommitted)

- **Target:** working tree di atas `HEAD=7cccf6c` (deliverable frontend `t_6a661349`), **uncommitted**: `M index.html`, `M qa-reports/headless-smoke.mjs`, `M src/main.ts`, `M src/styles.css` (+772/−11). Diff penuh 914 baris dibaca, bukan self-report.
- **Mode:** WHITEBOX + verify loop penuh dijalankan (bukan read-only); tidak ada commit/push; tidak ada stash. Backend/config/sidecar tidak disentuh.
- **Verdict: PASS** — 0 CRITICAL, 0 MAJOR, 0 MINOR blocking; 4 INFO non-blocking.
- **Spec:** docs/EPIC-TSD-redesign-dusk-v1.md §U-D7 (L233–261) + SDD §13.6 frozen + §13.4.

## Verify loop (fresh, semua dijalankan untuk laporan ini)

| Perintah | Hasil | Bukti |
|---|---|---|
| `npm run build` (tsc && vite build) | PASS — `dist/assets/index-C5xXSrHx.js` built 677ms | exit 0 |
| `cd src-tauri && cargo test` | **87 passed; 0 failed; 0 ignored** — `test result: ok … finished in 7.60s` | **== baseline kanonik** (docs/STATE.md; idem gate U-D2 & U-D4 `cargo test --lib` 87/87). `src-tauri/` zero-diff vs HEAD, jadi input test identik baseline. |
| `cd src-tauri && cargo build` | PASS — `Finished dev profile in 4.10s`; 2 warning `unused_mut` pre-existing (sama dengan laporan U-D4) | exit 0 |
| `node qa-reports/headless-smoke.mjs` | **308/308 passed**, exit 0; `PASS no console/page errors`; `CSP violations == 0 across suite`; `served CSP == tauri.conf.json policy` | log penuh dijalankan fresh; 9 hit `grep -i FAIL` semuanya substring dalam baris PASS ("Failed to …", "fail fixture") |

Smoke delta vs HEAD: `check()` 275 → 292 statis, **0 cek lama dihapus** (diff nama cek via script), +17 baru + 1 check dalam loop INV-26 (×3 runtime) = **20 baris `PASS U-D7`** ≥ 8 (AC#11 ✓).

## Coverage ledger — AC §U-D7 (EARS 1–11)

| # | AC | Result | Evidence (runtime smoke, kecuali disebut lain) |
|---|---|---|---|
| 1 | FR-AI1 panel AI: thread + card + input | **PASS** | `U-D7 FR-AI1/12` — drawer 340px, `aria-label="AI assistant"`, 2 `.msg-user`, 2 `.cmd-card`, placeholder `"Ask hosterm"`, fokus `ai-input`. Markup: `index.html` `#ai-drawer` + `#mode-ai`. |
| 2 | FR-AI3 awaiting state | **PASS** | `U-D7 FR-AI2/3` — `.cmd-card.awaiting`, `.cmd-state.warn` = `"AWAITING APPROVAL"`, scope `prod-web`, command mono `docker system prune -af`, deskripsi dampak ada, labels persis `["Run once","Always allow","Reject"]`. |
| 3 | **INV-25 (KRITIS)** class assertion | **PASS** | `U-D7 INV-25` — `classes[0] ⊇ {btn-primary, approval-run}`, `classes[1] ⊇ {btn-secondary, approval-always}` **dan ∌ btn-primary**, `classes[2] ⊇ {btn-ghost, approval-reject}`. Markup `aiCardMarkup()` (src/main.ts) verbatim. CSS: `.btn-primary` accent+`--accent-ink`, `.btn-secondary` `--raised` muted, `.btn-ghost` transparan. Permission-widening tidak pernah primary. |
| 4 | FR-AI5 Run once, allowlist UNCHANGED | **PASS** | `U-D7 FR-AI5 ×2` — `pty_spawn +1`, `pty_write +1` tepat 1× dengan `data === "docker system prune -af\r"`; `write_settings` delta 0 dan `storedSettings.allowlist` byte-for-byte identik sebelum/sesudah. Output card: `"Command sent once to the real-ssh terminal."` Jalur = `executeAiCommand` → `openTerminal`/`pty_write` existing (ADR-1/2), bukan jalur baru. |
| 5 | FR-AI6 Always allow → write_settings + execute | **PASS** | `U-D7 FR-AI6/ADR-21` — `write_settings` +1 dengan AllowEntry `{host:"prod-web", command:"docker system prune -af"}` persis, **index write_settings < index pty_write** (persist dulu, eksekusi kemudian), lalu `pty_write` 1×; `U-D7 FR-AI6/7` — state `.ok` = `"ALWAYS ALLOWED"`, 0 action. `added_at:""` dikirim frontend dan **backend-stamp** RFC3339 (`src-tauri/src/lib.rs:810-817`; test `write_then_read_roundtrips_and_stamps_added_at` ok) — sesuai SDD §13.4 "backend-stamped", bukan bug. |
| 6 | FR-AI7 exact re-match → ALWAYS ALLOWED | **PASS** | `U-D7 FR-AI2/7` — card `restart-netdata` render langsung `.ok` `"ALWAYS ALLOWED"` tanpa approval ulang, 0 action. Negatif `U-D7 ADR-21`: command + trailing space → tetap `AWAITING APPROVAL` + 3 action (byte-match ketat, `aiAllowlisted()` pakai `===` di kedua field). |
| 7 | FR-AI8 Reject | **PASS** | `U-D7 FR-AI8` — `aiCounts()` before==after (0 invoke: no spawn/write/settings), state `.danger` `"REJECTED"`, output `"Rejected. Nothing was executed or saved."` |
| 8 | **INV-26** host alias guard | **PASS** | 3 vektor `U-D7 INV-26` (leading dash `-prod-web` via Always-allow; `prod\rweb`; `prod\nweb` via Run-once): state `"REFUSED"`, output `"...Nothing was executed or saved"`, **delta invoke 0** → refuse SEBELUM write/execute. Guard `sftpSafe()` frontend mirror (main.ts, leading `-`/CR/LF) dipanggil di `runAiOnce`/`alwaysAllowAi` DAN sekali lagi di dalam `executeAiCommand` sebelum `openTerminal`/`pty_write`. Backend `sftp_safe` existing tak berubah (lib.rs:923–931). |
| 9 | FR-AI11/INV-11 escape thread+output | **PASS** | 2 fixture hostile (klik send + Enter setelah reload): `.msg-user` `childElementCount 0`, teks raw utuh, `__pwn` undefined, **CSP 0**. Semua sink render lewat `esc()`/`escAttr()`: `renderAiThread`, `aiCardMarkup` (host/command/desc/id), `.cmd-output` (result termasuk `msg(e)`), `renderApprovals` (host/command). Scan diff: tidak ada interpolasi tak-escape ke innerHTML. |
| 10 | FR-AI10 Settings>Approvals read+revoke | **PASS** | `U-D7 FR-AI10 ×2` — nav `data-section="ai"`: `read_settings` +1, list 2 item (host+command exact), **0 input/contenteditable** (bukan pattern editor); Revoke → `write_settings` +1 dengan allowlist len 1 tanpa tuple terhapus, status `"Approval revoked."` |
| 11 | Verify loop hijau + smoke ≥8 baru + cargo == baseline | **PASS** | Lihat tabel verify loop: build PASS, cargo **87/87 == baseline**, smoke **308/308** dengan **20** cek U-D7 baru, 0 cek lama dihapus. |

## Invariant & constraint gates

| Gate | Result | Evidence |
|---|---|---|
| INV-21 anti-Abyss | **PASS** | grep token Abyss (`#08080a #050507 #00d992 #818cf8 #0e2a20 var(--glow)`) pada added lines = **0**. Semua warna pakai token Dusk (`--accent --raised --term --live --danger --faint --hairline --accent-ink`). |
| INV-22 config/sidecar untouched | **PASS** | `git diff --name-only` = hanya 4 file scoped. Allowlist HANYA via `invoke("write_settings")` (`persistAllowlist`); 0 fs write frontend, 0 jalur baru. Backend `write_settings` (atomic temp→rename) tak berubah; test isolasi `write_settings_leaves_ssh_config_and_sidecar_untouched` ok. |
| FR-AI9 no implicit auto-run | **PASS** | `U-D7 FR-AI9` — buka drawer: delta invoke 0. Stub "Ask hosterm" (`U-D7 ADR-22`): 0 invoke + 0 network request; balasan jujur `"The AI backend is not connected…"`. Eksekusi hanya dari klik Run-once/Always-allow (masing-masing handler eksplisit). |
| FR-AI12 no "on-device" palsu | **PASS** | Head-tag `mock` + `title="UI-only mock; no AI backend is connected"`; grep `on-device` di 4 file = hanya assertion negatif smoke; smoke assert `!JSON.stringify(aiPanel).includes("on-device")` PASS. Thread = stub (`AI_PROPOSALS` konstan + `aiExtraMessages`). |
| ADR-21/§13.6 frozen contract | **PASS** | State machine persis: `proposed → AWAITING APPROVAL (warn) → RAN ONCE (ok)/REJECTED (danger)/ALWAYS ALLOWED (ok)`; `REFUSED (danger)` untuk guard. Exact `(host,command)` tuple; persist→execute ordering di-assert smoke. |

## Temuan (semua non-blocking)

- **UD7-QA-1 — INFO** — `executeAiCommand()` memanggil `activateTab(session.id)`: menjalankan perintah AI memindahkan tab terminal yang tampak. Side-effect UX, tidak dispesifikasi, tidak melanggar AC manapun.
- **UD7-QA-2 — INFO** — Card berstate ALWAYS ALLOWED dirender **tanpa tombol aksi** sama sekali; FR-AI9(b) menyebut "eksekusi via exact allowlist re-match" sebagai jalur eksekusi, tapi implementasi tidak menyediakan affordance Run untuk card yang sudah allowed (eksekusi re-match hanya relevan dari terminal itu sendiri). Arah aman (lebih ketat dari spec); perlu keputusan lead bila affordance Run untuk allowed card diinginkan di unit berikutnya.
- **UD7-QA-3 — INFO** — Label transien `"RUNNING ONCE"`/`"SAVING APPROVAL"` (`setAiCardBusy`, button disabled saat in-flight → tidak mungkin double-submit) adalah tambahan di luar machine §13.6; semua terminal state & kontrak class tetap persis.
- **UD7-QA-4 — INFO** — Buka Settings>AI merender dua kali (dari list in-memory lalu re-read `read_settings`); flicker sesaat saja, state akhir selalu dari disk.

## State yang diuji

```text
HEAD       = 7cccf6c (fix(ui): remove Google chip innerHTML sink)
working    = M index.html, M qa-reports/headless-smoke.mjs, M src/main.ts, M src/styles.css (+772/−11)
dist       = REBUILT dari working tree ini (tsc + vite, index-C5xXSrHx.js) sebelum smoke
src-tauri  = zero-diff vs HEAD; cargo test 87/0 == baseline; cargo build ok
smoke      = 308/308 PASS (20 U-D7 baru; 0 cek lama dihapus); playwright di-install --no-save (node_modules saja; package.json/lockfile tetap bersih — git status terverifikasi)
```

Tidak ada commit, tidak ada push. Laporan ini = self-report QA, terbuka untuk re-verify nano.
