# FEATURE-SPEC: Fleet Run v1

> Milik epic produk — sitasi `PRD.md §7` (row ditambahkan pm, satu baris).
> EARS untuk semua behavior. Initiative tunggal → dokumen ini FEATURE-SPEC, bukan PRD.
> Boss intent (MEKL via nano): "fitur ansible — bisa jalanin script/command ke banyak host".
> Keputusan final nano (1–7, card t_04e6f2a2) di bawah diperlakukan sebagai constraint,
> bukan bahan debat; bukti teknis yang melawan dituang di Clarifying Questions / OPEN QUESTIONS.

| Field | Value |
|---|---|
| Author | crew-coding-pm |
| Status | Speced — awaiting MEKL approval. No code, no commits. |
| Baseline | v0.10.0 (bb93631) + post-tag W2-A (3a3b5a0) + Arus U1–U4; verify loop STATE.md: cargo 52 / smoke 144 (re-measure at dispatch) |
| Non-negotiables | spawn OS **real `ssh`** (SDD ADR-1) · `~/.ssh/config` = satu-satunya sumber host (PRD §1.1) · **NOL dependency baru, NOL file persisten baru** · tanpa embed Ansible/python |
| Traceability | PRD v2 UC1/UC5/UC7 · FR-A1/A7 · FR-B2 · FR-E2/E3 · NFR-1/2/3/4/5 · INV-1/2/3/10/11 · SDD ADR-1/3/6/7/8, §6 |

---

## Clarifying Questions (front-loaded — Quick Spec principle)

> Jawab DULU sebelum dokumen dipakai; tiap pertanyaan WAJIB disertai rekomendasi.

1. **"Group" tidak ada di OpenSSH config.** Yang ada di app = label turunan prefix alias sebelum `-`
   (`prod-web` → `PROD`), main.ts:346. Rekomendasi: **seleksi per-group pakai turunan prefix itu** —
   nol perubahan config. Rekomendasi keras: **jangan pernah nulis direktif `Group`** ke `~/.ssh/config`;
   ssh asli gagal keras (`Bad configuration option`) di SETIAP connect user, bukan cuma warning.
2. Multi-line command vs script: satu jalur transport atau dua? Rekomendasi: **`ssh <alias> <cmd>` sebagai
   satu elemen argv untuk input satu baris; stdin `bash -s` untuk script file ATAU input multi-baris**
   (multi-baris = script, bukan command). Tidak ada quoting/escaping yang harus benar.
3. Password-host di fleet bentrok secara teknis: feed password butuh PTY (stdin = terminal), script-via-stdin
   butuh pipe (stdin = file). Rekomendasi: **v1 = key/agent-host saja; password-host → status `skipped` dengan
   reason eksplisit** (jalur escape yang sudah nano sediakan di keputusan 5); fleet password jadi v2 setelah
   architect membuktikannya di SDD addendum (Task 0).
4. Host key belum dikenal + `BatchMode=yes` → `Host key verification failed` (exit 255). Rekomendasi:
   **fail-closed, tanpa auto-accept** (fitur blast-radius tidak boleh diam-diam menerima key baru); pesan
   error menyebut jalan pemulihan (buka satu terminal tab ke host itu sekali).
5. Timeout per host tidak disebut nano. Rekomendasi: **default 60 s, bisa diubah 5–600 s**.
6. Cap output per host (run `yes` / `find /` jadi runaway). Rekomendasi: **retensi 64 KiB per host**,
   byte tertua dibuang, penanda `truncated` tampak di baris host.
7. Cap concurrency yang bisa diubah: nano fix default 5. Rekomendasi: **range 1–20** (di atas 20 antrean
   tetap jalan, tidak menambah apa pun yang bisa diuji).
8. Direktif user `ForwardAgent yes` membuat agent ssh user bisa dipakai remote root saat fleet jalan.
   Rekomendasi: **v1 tidak meng-override config user** (janji UC1: config berperilaku persis seperti CLI),
   tapi dialog konfirmasi memberi baris peringatan per host yang terdeteksi `ForwardAgent`.
9. Cap ukuran script. Rekomendasi: **≤ 2 MiB** — angka yang sama dengan editor (FR-D2), nol ambang baru
   yang harus dijelaskan ke user.
10. Tutup panel Fleet saat run berjalan. Rekomendasi: **tutup panel = Stop** (konfirmasi in-app), bukan
    run hantu di background; state run jadi sederhana dan bisa di-smoke.

---

## 1. Requirements (EARS)

**Non-goals v1 (eksplisit):** Ansible/playbook/inventory file; templating/variabel per-host; scp/upload
tahap depan; retry-failed-only; history/laporan run ke file; diff antar-host; rolling/batch-serial dengan
failure budget; target host non-POSIX (Windows SSH server); sudo otomatis; dukungan host password;
perintah penambah direktif config apa pun (fleet tidak menulis config).

| ID | Requirement |
|---|---|
| FR-FL1 | WHEN panel Fleet dibuka THE SYSTEM SHALL menampilkan satu baris pilih-per-host untuk setiap host yang juga dirender sidebar (pola `*`/`?` dikecualikan), lengkap dengan alias, `user@HostName`, dan metode auth |
| FR-FL2 | WHEN operator memilih header group THE SYSTEM SHALL menoggle seleksi seluruh host di bawah label prefix group itu |
| FR-FL3 | WHEN operator menekan Select all THE SYSTEM SHALL memilih seluruh host yang terdaftar di picker |
| FR-FL4 | WHEN operator mengetik di filter picker THE SYSTEM SHALL menyempitkan baris dengan aturan substring alias yang sama dengan search sidebar, tanpa mengubah status seleksi host yang tersembunyi |
| FR-FL5 | IF nol host terpilih THEN THE SYSTEM SHALL menjaga tombol Run dalam keadaan disabled dan tidak membuat dialog konfirmasi bisa dijangkau |
| FR-FL6 | WHEN operator memilih mode command THE SYSTEM SHALL menerima teks satu baris dan mengirimnya ke `ssh <alias> <cmd>` sebagai satu elemen argv setelah alias |
| FR-FL7 | WHEN teks input memuat lebih dari satu baris THE SYSTEM SHALL memperlakukannya sebagai script dan menyalurkannya via stdin (`ssh <alias> bash -s`) |
| FR-FL8 | WHEN operator memilih mode script dan satu file lokal THE SYSTEM SHALL membaca file itu sebagai teks UTF-8 dan menyalurkan byte-nya via stdin ke `bash -s`, tanpa scp/upload/perantara file di host |
| FR-FL9 | WHEN file script dibaca untuk run THE SYSTEM SHALL menormalisasi CRLF→LF dan menjamin satu newline di akhir byte yang dikirim |
| FR-FL10 | IF file script tidak terbaca, binary/invalid UTF-8, atau > 2 MiB THEN THE SYSTEM SHALL menolak dengan pesan yang menyebut sebabnya sebelum satu host pun disentuh |
| FR-FL11 | IF input kosong atau hanya whitespace THEN THE SYSTEM SHALL menjaga Run disabled |
| FR-FL12 | THE SYSTEM SHALL tidak pernah meneruskan teks command/script ke shell lokal — tanpa `sh -c`, `bash -c`, `cmd /C`; input hanya boleh sampai ke `ssh` sebagai argv atau sebagai byte stdin child |
| FR-FL13 | WHEN operator menekan Run THE SYSTEM SHALL menampilkan dialog konfirmasi yang menyebut: jumlah host, seluruh alias terpilih, teks command persis ATAU nama file + ukuran script, timeout per host, concurrency cap, dan jumlah host yang akan di-skip beserta reason |
| FR-FL14 | IF command atau script memuat invocation `sudo` THE SYSTEM SHALL menambahkan baris peringatan sudo ke dialog konfirmasi |
| FR-FL15 | IF salah satu host terpilih punya `ForwardAgent` aktif di config THE SYSTEM SHALL menampilkan peringatan agent-forwarding di dialog konfirmasi |
| FR-FL16 | WHEN dialog konfirmasi ditutup atau dibatalkan (Escape/Cancel/close) THE SYSTEM SHALL men-spawn nol proses |
| FR-FL17 | THE SYSTEM SHALL memakai dialog in-app untuk konfirmasi — tidak pernah `confirm()`/`prompt()`/`alert()` native |
| FR-FL18 | WHEN run dimulai THE SYSTEM SHALL menjalankan `ssh -o BatchMode=yes <alias> …` per host terpilih yang layak, maksimal N paralel (N default 5, adjustable 1–20), sisanya berstatus `queued` |
| FR-FL19 | WHEN proses ssh satu host selesai THE SYSTEM SHALL mencatat status + exit code host itu dan melepas slot concurrency ke host `queued` berikutnya |
| FR-FL20 | IF sebuah host alias GAGAL safety-check argv (berawalan `-`, atau memuat newline/carriage return) THEN THE SYSTEM SHALL menandainya `skipped` dengan reason `unsafe alias` dan tidak pernah men-spawn untuknya |
| FR-FL21 | IF sebuah host terpilih memakai auth password (auth method password ATAU punya entri sidecar) THEN THE SYSTEM SHALL menandainya `skipped` dengan reason `password host (not supported in v1)` dan tidak pernah men-spawn untuknya |
| FR-FL22 | WHEN sebuah host mencapai timeout per host THE SYSTEM SHALL mengakhiri proses host itu, menandainya `timeout`, dan membiarkan host lain tetap berjalan |
| FR-FL23 | THE SYSTEM SHALL memakai timeout untuk seluruh percobaan per host (connect + eksekusi), bukan hanya fase eksekusi |
| FR-FL24 | WHEN ssh menerbitkan stdout/stderr THE SYSTEM SHALL mengalirkannya ke pane output host itu dalam urutan kedatangan, tanpa memblokir host lain dan tanpa memblokir UI |
| FR-FL25 | WHEN output tersimpan untuk satu host melewati 64 KiB THE SYSTEM SHALL membuang byte tertua dan menampilkan penanda `truncated` di baris host itu |
| FR-FL26 | WHEN byte output remote dirender ke DOM THE SYSTEM SHALL melewatkannya lewat `esc()`/`escAttr()` — output remote adalah input hostile (INV-11) |
| FR-FL27 | THE SYSTEM SHALL memberi setiap host tepat satu status akhir dari `ok` \| `fail` \| `timeout` \| `skipped`; `queued`/`running` hanya state sementara |
| FR-FL28 | WHEN proses ssh keluar 0 THE SYSTEM SHALL menandai host `ok`; WHEN keluar non-zero THE SYSTEM SHALL menandainya `fail` dengan exit code dan stderr tail yang terlihat |
| FR-FL29 | WHEN semua host mencapai status akhir THE SYSTEM SHALL menampilkan ringkasan run `ok X · fail Y · timeout Z · skipped W` + total elapsed + exit code non-zero terkecil/besar yang muncul |
| FR-FL30 | WHEN operator menekan Stop THE SYSTEM SHALL mengakhiri proses yang sedang jalan, menandai host `running` dan `queued` sebagai `skipped` dengan reason `stopped by user`, dan menampilkan ringkasan |
| FR-FL31 | IF satu run sedang aktif THEN THE SYSTEM SHALL menolak memulai run kedua (Run disabled, satu run aktif per app) |
| FR-FL32 | IF startup check W2-A melaporkan `ssh` tidak ada THEN THE SYSTEM SHALL menampilkan error per-attempt yang menyebut recovery path dan men-spawn nol proses, tanpa string spawn mentah (FR-E3) |
| FR-FL33 | WHEN sebuah host mencapai status akhir THEN THE SYSTEM SHALL memperbarui status dot baris host itu di picker tanpa me-reload host list dari disk |
| FR-FL34 | THE SYSTEM SHALL tidak pernah menulis `~/.ssh/config` atau sidecar sebagai efek dari fleet run |
| FR-FL35 | THE SYSTEM SHALL tidak pernah menyisipkan `sudo` atau flag escalating ke argv, dan tidak pernah menambah `-A`/`-X` forwarding ke perintah yang di-spawn |
| FR-FL36 | THE SYSTEM SHALL tidak pernah mengirim, me-log, atau me-render isi sidecar password ke output fleet, event IPC, atau DOM |
| FR-FL37 | WHEN panel Fleet ditutup selama run aktif THE SYSTEM SHALL menghentikan run lewat jalur Stop FR-FL30 dengan konfirmasi in-app |

### Non-functional requirements (EARS + measured)

| ID | Requirement (TERUKUR) | Measure |
|---|---|---|
| NFR-FL1 | THE SYSTEM SHALL menambah 0 dependency runtime (no ansible, no python, no ssh library) | `Cargo.toml` + `package.json` dependency diff = 0 entries |
| NFR-FL2 | THE SYSTEM SHALL menulis 0 file persisten baru dari jalur fleet (no history, no cache, no report file) | file-system diff selama cargo test run = 0 file baru di `~/.ssh` dan di app dir |
| NFR-FL3 | THE SYSTEM SHALL menahan total bytes output per run di ≤ 2 MiB + 64 KiB × jumlah host (cap retensi FR-FL25) | unit test feed 10 MiB per host → memory-bound check + `truncated` ter-set |
| NFR-FL4 | IF command/script user mengandung `;`/`&&`/`$()`/backtick THEN THE SYSTEM SHALL tidak pernah mengeksekusinya secara lokal (0 proses shell lokal ter-spawn) | grep gate `sh -c`/`bash -c`/`cmd /C` = 0 di jalur fleet + cargo fixture: marker file lokal tidak pernah terbentuk |
| NFR-FL5 | THE SYSTEM SHALL menyelesaikan spawn pertama ≤ 200 ms setelah konfirmasi Run (bukan latency connect — itu ssh/network, sesuai NFR-6 PRD) | cargo test stopwatch spawn, fake-ssh |
| NFR-FL6 | THE SYSTEM SHALL men-spawn 0 proses bila konfirmasi tidak diberikan (Escape/Cancel/close/disabled-Run) | smoke mock spawn-invoke counter == 0 |
| NFR-FL7 | THE SYSTEM SHALL menjaga 0 secret byte di output fleet, event IPC, DOM, dan log (NFR-1/INV-4 PRD) | grep gate verify loop + smoke DOM assert (mock sidecar password tidak pernah muncul) |
| NFR-FL8 | THE SYSTEM SHALL tidak pernah menulis/mengubah `~/.ssh/config` atau sidecar karena fleet (INV-1/2/3) | cargo test: hash + mtime config dan sidecar identik sebelum/sesudah run multi-host |
| NFR-FL9 | THE SYSTEM SHALL tidak pernah menyentuh `~/.ssh/authorized_keys` (hard invariant STATE.md #6) dan tidak menjalankan apa pun dengan elevated/default privilege | code audit argv + verify loop grep |
| NFR-FL10 | THE SYSTEM SHALL menambah ≥ 8 cargo test dan ≥ 10 smoke checks; seluruh verify loop hijau sebelum staging (NFR-5 PRD) | angka di task log tiap unit; 0 CSP violation |

### Prioritas (MVP cut)

| ID | Prioritas | Catatan cut |
|---|---|---|
| FR-FL1, 5, 6, 7, 8, 11, 12, 13, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 27, 28, 29, 32, 34, 35, 36, 37 | **must** | inti runner + blast radius gate; tanpa ini fitur tidak bisa dirilis |
| FR-FL2, 3, 9, 10, 26, 30, 31, 33 | **must** | keamanan (26) + hygiene seleksi/stop; murah, satu unit yang sama |
| FR-FL4, 14, 15 | **should** | filter picker + dua peringatan dialog; dipotong bila lead butuh memangkas unit frontend |
| NFR-FL1–FL10 | **must** | gate keamanan/integrity/regresi tidak bisa di-cut; NFR-FL5 (≤ 200 ms spawn) ikut konvensi PRD OQ-6 "bind on first complaint" — diukur sekali saat unit 1 jalan, tidak jadi CI timing gate |
| — | **v2 (bukan v1)** | host password, retry-failed-only, diff antar-host, persistensi run, history, env/args per run, target Windows SSH |

### Edge cases & error handling

- **exit 255 ambiguum.** `ssh` memakai 255 untuk kegagalannya sendiri (connect/auth/host key) — tidak
  bisa dibedakan dari perintah remote yang exit 255. Perilaku: status `fail`, exit code apa adanya,
  stderr tail ditampilkan; pesan tidak boleh mengklaim "perintah remote gagal".
- **Host key belum dikenal** → `Host key verification failed` (BatchMode, tidak pernah auto-accept): `fail`
  + pesan yang menyebut "buka satu terminal tab ke host ini untuk menerima host key"-nya.
- **Key ber-passphrase tanpa agent** → `Permission denied (publickey)`: `fail`; pesan menyebut penyebab.
- **Kedua bentuk kegagalan auth di atas harus tampil sebagai `fail`, bukan `timeout`** — jangan menelan
  kegagalan cepat sebagai kegagalan lambat.
- **ProxyJump/shared bastion mati** → semua host lewat baston itu `fail` hampir bersamaan; tidak ada
  dedup/cluster-analysis di v1 (ringkasan count sudah cukup untuk melihat pola).
- **`bash` tidak ada di target** → exit 127 + stderr; target v1 = host POSIX. Known limitation, tercatat §4.
- **Script tanpa newline di akhir / CRLF (file Windows-authored)** → dinormalisasi FR-FL9; tanpa ini
  `bash -s` gagal baris pertama (`$'line\r'`), dan bug itu terasa seperti bug app.
- **Alias dengan spasi/metachar (`;`, `&&`, `$()`)** → aman terhadap shell lokal (FR-FL12) karena satu elemen
  argv; tetap lolos `sftp_safe`-analog kecuali `-`/newline (FR-FL20).
- **Alias berawalan `-`** → `skipped` `unsafe alias` (INV-10), jangan pernah jadi argv[1] option.
- **Duplikat alias / satu blok `Host a b`** → picker satu baris per blok (alias pertama), sama seperti sidebar;
  tidak pernah menjalankan dua kali untuk satu host.
- **0 host layak** (semua terpilih = password host) → dialog menunjukkan `0 to run · W skipped`; Run confirm
  menolak spawn, ringkasan muncul penuh `skipped`.
- **Run besar (200 host)** → picker harus tetap bisa di-scroll; output cap per host menahan memori (NFR-FL3).
- **App ditutup saat run aktif** → anak-anak ssh ikut mati bersama proses (postur sama ADR-7); tidak ada
  cleanup file karena v1 tidak menulis file apa pun.
- **`ssh` child di Windows** → spawn tanpa jendela konsol (`CREATE_NO_WINDOW`, paritas jalur sftp lib.rs);
  tanpa ini fleet menghasilkan kedipan jendela per host.
- **Timeout kill ≠ remote kill.** Sisi remote terus jalan (persis seperti Ctrl-C di ssh CLI). Didokumentasikan
  di hasil run, bukan bug yang dijanjikan selesai.
- **Output mengandung ANSI escape** → dirender apa adanya sebagai teks ter-escape di pane text (bukan xterm;
  tidak ada terminal emulation di fleet pane).

---

## 2. Design Sketch

> PM-level, bukan arsitektur final. Versi penuh = SDD addendum (Task 0, architect lane).

- **Komponen tersentuh:** `src-tauri/src/lib.rs` (runner baru + 2–3 command IPC + event stream),
  `src/main.ts` + `src/styles.css` (panel Fleet — antre setelah Arus U9, hotspot constraint),
  `qa-reports/headless-smoke.mjs` (mock + checks baru), `src-tauri` tests.
- **Tidak ada model baru.** Host picker membaca ulang `read_ssh_config()` yang sudah ada (HostDto:
  `name`, `user`, `host_name`, `auth_method`, `has_password`) — sudah cukup untuk FR-FL1/21; tidak ada
  store/inventori kedua (PRD §1.1).
- **Transport = pola yang sudah terbukti.** `run_sftp_batch` (lib.rs:717) sudah melakukan persis bentuk yang
  dibutuhkan fleet: `Command` + `-o BatchMode=yes` + stdin piped + stdout/stderr piped + exit code asli,
  tanpa shell lokal. Fleet reuse bentuk itu dengan biner `ssh` dan arg `bash -s` / command argv.
- **Runner = satu thread/async task per host dalam batas semaphore N**; reader thread men-emit chunk per host
  (`fleet://output/...`, penamaan final milik architect); satu event selesai per host dengan
  (status, exit code, byte counts). Timeout = kill handle per child; Stop = kill semua child aktif.
- **Fungsi murni yang wajib ada supaya ter-unit-test:** (a) kelayakan host → `ok|skipped(reason)` dari DTO,
  (b) pemilihan transport → argv-command vs stdin-script dari bentuk input, (c) normalisasi script
  (CRLF→LF + trailing newline), (d) klasifikasi status akhir dari (exit code, killed-by-timeout, skipped),
  (e) guard alias analog `sftp_safe`. Lima fungsi itu = lima grup cargo test, tanpa network dan tanpa ssh asli.
- **Testability hook:** butuh override biner ssh untuk test (mirip `HOSTERM_SFTP_BIN`, lib.rs:721) supaya
  cargo test bisa menjalankan runner vs fake-ssh yang mengembalikan exit code/stderr ditentukan; nama final
  milik architect, requirement-nya: fleet harus bisa diuji tanpa server nyata di CI Linux.
- **Auth:** key/agent saja (FR-FL21 skip password). Password fleet = v2 dan butuh bukti PTY-vs-pipe di
  SDD addendum — alasan teknisnya ada di Clarifying Q3.
- **Keamanan struktural:** command/script tidak menyentuh shell lokal (FR-FL12); output remote = bytes hostile
  → `esc()`/`escAttr()` (INV-11/ADR-8, CSP ADR-6 tidak berubah); sidecar tidak pernah ke IPC (INV-4);
  dialog konfirmasi = satu-satunya gerbang sebelum spawn.
- **Testing strategy:** cargo (5 fungsi murni di atas + "config bytes unchanged across a run" + concurrency
  ceiling vs fake ssh) ≥ 8 test baru; smoke ≥ 10 checks baru (picker render, select-all, group toggle,
  Run-disabled state, dialog blast-radius copy, cancel = 0 invoke spawn, status/summary dari mock, skip
  password host, CSP counter tetap 0); build TS 0 error; manual Windows matrix 1 sesi (SEC-5 waiver berlaku —
  spawn-window/CREATE_NO_WINDOW cuma terbukti di sana).

---

## 3. Tasks Overview (input backlog lead)

| Unit | Cakupan file | Deps | Demo: (bukti observable selesai) |
|---|---|---|---|
| 0 — SDD addendum Fleet Run | `docs/SDD.md` (architect lane, bukan deliverable pm) | spec ini | Architect memutuskan: kelayakan password-host PTY vs pipe, nama command/event IPC, nama env override ssh-test, bentuk kill/timeout. Ditulis sebagai ADR + INV baru sebelum kode apa pun |
| 1 — Fleet runner core (backend) | `src-tauri/src/lib.rs`, `src-tauri/src/*` tests | Unit 0 | `cd src-tauri && cargo test --lib` → ≥ 52 + 8 baru pass: kelayakan/skip reason, pemilihan transport, normalisasi CRLF+trailing newline, guard alias, klasifikasi status, concurrency ceiling ≤ N hidup serentak vs fake-ssh, timeout kill, dan assert byte-config `~/.ssh/config` tidak berubah |
| 2 — IPC surface + event contract (backend) | `src-tauri/src/lib.rs` | Unit 1; **satu file dengan Unit 1 → serial di dalam epic ini** | Daftar `#[tauri::command]` bertambah (inventori 21 → baru) dengan payload start/run/stop + event output/selesai; `cargo test --lib` hijau; kontrak didokumentasikan buat frontend (request/response shape per host: alias, status, exit code, chunk seq) |
| 3 — Panel Fleet + dialog konfirmasi (frontend) | `src/main.ts`, `src/styles.css` | **Arus U9 selesai (hotspot queue — nano keputusan 7)** + Unit 2 kontrak IPC | `npm run build` 0 TS error; smoke: panel render N baris per host, select-all/group toggle, Run disabled saat 0 terpilih, dialog menampilkan `N hosts · command · timeout · cap · skipped`, Cancel → 0 invoke spawn tercatat di mock |
| 4 — Live output + status + ringkasan (frontend) | `src/main.ts`, `src/styles.css` | Unit 3 | smoke: mock run menghasilkan baris `ok/fail/timeout/skipped` + exit code, ringkasan `ok X · fail Y · timeout Z · skipped W` tampil, penanda `truncated` muncul saat mock mengirim > 64 KiB, output ter-escape (fixture `<script>` di stdout → 0 CSP violation, 0 elemen script di DOM) |
| 5 — Smoke canonization + docs sync | `qa-reports/headless-smoke.mjs`, `docs/STATE.md` (lead lane) | Unit 1–4 | `node qa-reports/headless-smoke.mjs` ≥ baseline + 10 pass, CSP counter 0; STATE verify-loop di-update; README/guide punya satu seksi "Run on hosts" (owner: lead) |

Rollout constraint (nano keputusan 7, dicatat di sini): Unit 1–2 (backend, `lib.rs`) boleh jalan paralel dengan
redesign Arus — bukan file hotspot Arus. Unit 3–4 (main.ts/styles.css) WAJIB antre setelah U9; Unit 1–2 tetap
serial terhadap satu sama lain (file sama). Tidak ada unit frontend yang bisa di-merge ke `main.ts` sebelum U9
selesai; lead yang menjadwalkan.

---

## 4. Risks & Pitfalls

| # | Risiko | Impact | Mitigasi / status |
|---|---|---|---|
| R-FL-1 | Path stdin-script (pipe) vs password-feed (PTY) tidak bisa disatukan: stdin PTY adalah terminal, `bash -s` membaca script dari terminal = echo + line discipline | Medium (scope v1) | Clarifying Q3 → v1 key-only + `skipped` eksplisit (FR-FL21); bukti teknis diminta di Unit 0, bukan diperdebatkan di kode |
| R-FL-2 | Blast radius: satu klik bisa menjalankan perintah merusak di puluhan host sekaligus | **High** | FR-FL13–17 (satu gerbang konfirmasi dengan jumlah host + teks persis + skip count), tanpa sudo default (FR-FL35), Escape = 0 spawn (NFR-FL6) |
| R-FL-3 | Output remote dirender ke DOM = jalur XSS ke IPC bridge (INV-11; 21 situs `innerHTML` existing) | High | FR-FL26 wajib `esc()`/`escAttr()`; CSP ADR-6 tidak disentuh; smoke fixture bytes hostile + CSP counter == 0 (SDD R-1) |
| R-FL-4 | Perintah user bocor ke shell lokal (injection lokal, `; rm -rf ~`) | High | FR-FL12 + NFR-FL4 grep gate (`sh -c`/`bash -c`/`cmd /C` = 0 di jalur fleet) + cargo fixture filesystem-tak-tersentuh |
| R-FL-5 | Secret bocor ke output/log (BatchMode path tidak mengirim password, tapi host yang prompt-nya muncul di stderr bisa membingungkan) | High | FR-FL36/INV-4: sidecar tidak pernah menyeberang IPC; v1 bahkan tidak membacanya untuk fleet; grep gate verify loop (NFR-1) |
| R-FL-6 | `BatchMode=yes` gagal untuk host key belum dikenal → lonjakan kegagalan first-run yang terasa seperti bug app | Medium | Clarifying Q4 + edge case: fail-closed dengan pesan actionable "buka terminal tab sekali untuk host ini"; sadar memilih TIDAK auto-accept (MITM surface) |
| R-FL-7 | `ForwardAgent yes` di config user exposes ssh agent user ke remote root saat fleet jalan | Medium | Tidak di-override (janji UC1/ADR-1); peringatan eksplisit di dialog (FR-FL15) + risk register; override = keputusan MEKL di OQ-3 |
| R-FL-8 | Target non-POSIX (Windows SSH server) tidak punya `bash` | Low | Non-goal v1; exit 127 + stderr apa adanya (edge case tercatat) |
| R-FL-9 | Hotspot `src/main.ts` + `src/styles.css` — tabrakan dengan Arus U2–U9 (SDD R-14, EPIC §0) | Medium | Unit 3–4 di-queue setelah U9; tidak ada unit frontend boleh menyentuh file itu sebelumnya; backend (Unit 1–2) aman paralel tapi serial internal |
| R-FL-10 | Tidak ada bukti runtime Windows (SEC-5 WAIVED MEKL 19 Okt) — `CREATE_NO_WINDOW`/spawn-window flash cuma kelihatan di sana | Medium | Satu manual Windows matrix sebelum tag berikutnya (dibatch dengan gate U4/U5 + checklist rilis — PRD OQ-4): fleet 3 host key-auth, termasuk 1 unknown-host-key |
| R-FL-11 | Run tanpa batas output/memori besar (mis. `find /`), 20 host × tak terbatas | Medium | Cap retensi 64 KiB/host + kill on timeout (FR-FL22/25); v1 tidak menyimpan output ke file (NFR-FL2) |
| R-FL-12 | Fitur ini gampang melebar jadi Ansible-lite (inventory file, variables, playbook, diff) | Medium (product) | Non-goals §1 eksplisit; setiap tambahan lewat spec baru, bukan lewat unit |
| R-FL-13 | Angka verify loop di PRD §5 NFR-5 (smoke 75) sudah tertinggal STATE.md (144 pasca-U4) | Low (doc drift) | Spec ini mengutip STATE.md + konvensi "re-measure at dispatch"; PRD row §7 hanya satu baris (deliverable pm dibatasi), fix NFR-5 = task doc-sync terpisah |

### Success metrics (feature-level, terukur)

| Metric | Target | Cara ukur |
|---|---|---|
| Gate blast radius | tepat 1 konfirmasi antara Run → spawn pertama; Cancel/Escape → 0 spawn | smoke mock invoke counter == 0 setelah cancel |
| Penyelesaian run | 100% host terpilih punya satu status akhir; 0 host tertinggal `running`/`queued` setelah Stop | smoke atas mock run campur ok/fail/timeout/skipped |
| Concurrency ceiling | tidak pernah > N proses hidup serentak (N default 5) | cargo test vs fake-ssh yang menghitung child hidup |
| Integrity config | 0 byte perubahan pada `~/.ssh/config` + sidecar selama run | cargo test (mtime + hash) |
| Secret hygiene | 0 hit secret di output fleet, event IPC, DOM, log | grep gate verify loop + smoke DOM assert |
| Regresi minimal | verify loop hijau + ≥ 8 cargo test baru + ≥ 10 smoke checks baru; 0 CSP violation | angka di task log tiap unit |
| Adopsi (MEKL) | 1 loop ad-hoc `for h in …; ssh` diganti 1 run in-app dalam minggu pertama | laporan manual maintainer; bukan gate rilis |

---

## 5. OPEN QUESTIONS (for MEKL — each with PM recommendation)

> **STATUS: CLOSED (19 Okt 2026) — jangan buka lagi.** Keputusan final:
> OQ-1 = **v2**; v1 key/agent-only, password host `skipped` + reason eksplisit (**MEKL**, clarify round nano).
> OQ-2 = **tidak auto-accept**; fail-closed + pesan actionable "buka satu terminal tab ke host itu sekali" (**MEKL**).
> OQ-3 = v1 tidak override `ForwardAgent`, hanya peringatan di dialog (**nano**, adopsi rekomendasi).
> OQ-4 = grouping turunan prefix alias saja; TIDAK pernah menulis direktif `Group` ke config (**nano**).
> OQ-5 = retry-failed-only **bukan v1**; host gagal dipilih manual dari picker (**MEKL**).
> OQ-6 = cap concurrency 1–20 (**nano**). OQ-7 = tanpa args/env per run di v1 (**nano**). OQ-8 = tanpa mode dry-run; perintah `true` sudah cukup (**nano**).
> OQ-9 = Unit 1–2 (backend) dispatch sekarang paralel Arus; Unit 3–5 (frontend) setelah Arus U9 (**nano**, = keputusan 7).

1. **Password-host: v1 atau v2?** Bukti teknis (Clarifying Q3): stdin-script butuh pipe, password-feed butuh
   PTY; menggabungkan keduanya = dua jalur transport + dua surface uji, dan feed password di bawah concurrency
   cap belum pernah dibuktikan untuk `ssh <alias> <cmd>` (yang terbukti: PTY terminal + persistent `sftp`).
   — rekomendasi: **v2.** v1 key/agent-only dengan `skipped` reason eksplisit (FR-FL21); keputusan akhir tetap
   di architect (Unit 0) sesuai escape hatch nano keputusan 5.
2. **Auto-accept host key baru di fleet?** Kemudahan vs MITM surface, dan fitur ini punya blast radius besar.
   — rekomendasi: **tidak** — fail + pesan "buka satu terminal tab ke host ini untuk menerima key"-nya.
   Sekali per host, sadar, dan tidak menambah flag yang diam-diam mengubah known_hosts.
3. **Override `-o ForwardAgent=no` di fleet?** Melanggar janji "config berperilaku persis seperti CLI" (UC1)
   tapi melindungi agent user dari remote root. — rekomendasi: **v1 tidak override, hanya peringatan di dialog
   (FR-FL15).** Kalau MEKL ingin hardening, jadikan satu keputusan global untuk SEMUA spawn (terminal, sftp,
   fleet), bukan khusus fleet — inkonsistensi lebih buruk daripada keduanya.
4. **Grouping: turunan prefix alias, atau support `Group` pertama-kelas?** — rekomendasi: **prefix turunan
   saja.** Direktif `Group` membuat ssh asli gagal (`Bad configuration option`) di setiap connect CLI user —
   harga kerusakannya nyata, manfaatnya bisa dicapai tanpa sentuh config. Group berbasis komentar (`# group: x`)
   boleh jadi v2 read-only.
5. **Retry failed-only?** after-the-fact conveniences paling banyak diminta. — rekomendasi: **bukan v1.**
   Operator bisa membuka panel lagi dan memilih host gagal secara manual; fitur retry butuh state run
   tersimpan dan menabrak non-goal "0 persistensi baru".
6. **Cap concurrency 1–20 atau bebas?** — rekomendasi: **1–20.** Nilai di atas 20 tidak mengubah perilaku
   yang bisa diuji (queue selalu benar), dan menambah surface failure (fd/thread exhaustion) untuk nol manfaat
   produk yang terukur.
7. **Script args / environment per run** (mis. `bash -s -- <args>`, atau var yang dikirim)? — rekomendasi:
   **v1 tidak.** Tanpa args, spec tetap satu dimensi (teks atau file); args menambah quoting surface persis di
   tempat yang paling rawan salah (FR-FL12 ada untuk menghapus quoting, bukan menambah varian baru).
8. **Perlu fleet punya "dry run" (connect-only, tidak menjalankan perintah)?** — rekomendasi: **v1 tidak,
   di-parkir ke v2.** `ssh -o BatchMode=yes <alias> true` adalah one-liner yang bisa dijalankan operator
   lewat fitur ini sendiri dengan perintah `true` — tidak perlu mode kedua untuk sesuatu yang bisa ditulis user.
9. **Kapan unit frontend boleh mulai?** Gate keras: setelah Arus U9 merge. — rekomendasi: **Unit 1–2 (backend)
   dispatch sekarang**, paralel dengan Arus; Unit 3–5 dijadwalkan setelah U9. Alasannya: backend bukan file
   hotspot, kontrak IPC bisa dibekukan lebih awal, dan frontend lalu punya target yang sudah stabil.

---

*Sources: docs/STATE.md (verify loop 52 cargo / 144 smoke post-U4) · docs/PRD.md §1.1/§3/§4/§5/§7/§8 ·
docs/SDD.md ADR-1/3/6/7/8, §3, §5, §6, §8 INV-1..11, §9 R-1/R-3/R-5/R-14, §10 ·
src-tauri/src/lib.rs (read_ssh_config 369–417, sftp_safe 655–666, run_sftp_batch 716+, pty_spawn 1266–1324,
inventori 21 `#[tauri::command]`) · src/main.ts:317–366 (host list + group prefix derivation + wildcard filter) ·
qa-reports/headless-smoke.mjs:38–40 (mock hosts prod-web/db-core/pw-host) ·
docs/FEATURE-SPEC-startup-ssh-check.md (gaya spec + check_binaries contract) ·
templates/feature-spec.md. Read 2026-10-19.*
