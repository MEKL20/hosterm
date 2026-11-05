# FEATURE-SPEC: Ansible (v1)

> Milik epic produk — sitasi `PRD.md §7` (row ditambahkan pm, satu baris).
> EARS untuk semua behavior. Initiative tunggal → dokumen ini FEATURE-SPEC, bukan PRD.
> Boss intent (MEKL via Tier-0 nano clarify, 2026-11-05): "fitur Ansible di hosterm — jalankan
> playbook + inventory + browse output + history run; dry-run WAJIB".
> Keputusan final nano (card t_e7e32c25, brief di bawah) diperlakukan sebagai constraint, bukan
> bahan debat. Bukti teknis yang melawan dituang di Clarifying Questions / OPEN QUESTIONS.

| Field | Value |
|---|---|
| Author | crew-coding-pm |
| Status | Speced — awaiting nano re-verify → architect SDD. No code, no commits (PM = doc only). |
| Baseline | post-v0.12.0 feature wave di main (e9a5f9c); verify loop STATE.md: re-measure at dispatch (cargo/build/smoke) |
| Non-negotiables | spawn OS **real `ansible-playbook`** via PTY lokal (pola IDENTIK `local_pty_spawn`/UC11, SDD ADR-1 spirit) · `~/.ssh/config` = sumber host yang sudah ada (PRD §1.1) · **NOL embed Ansible/python** (opsional, user install dulu) · argv langsung, TIDAK `sh -c`/shell string (INV-14) · secrets tak pernah lewat CLI/argv/IPC (posture zero-knowledge hosterm) |
| Traceability | PRD UC11/US-12 FR-LT1..3 (pola local PTY) · FR-E2/E3 (pola binary-missing W2-A) · FR-A1/A7 (host dari config) · INV-14 (no shell interp) · INV-22 (store app-data isolated) · INV-27 (direct PTY child) · SDD §13.3, §13.4 data store, ADR-8 safe DOM |
| Feature name (LOCKED) | **Ansible (v1)** |

---

## 0. Scope lock (dari nano brief — bukan bahan debat)

**v1 = LENGKAP tapi lokal + folder:**
1. Run playbook (dengan dry-run `--check` WAJIB sebelum real run).
2. Inventory builder dari host hosterm yang sudah ada.
3. Browse output per host.
4. History run (lokal app-data).
5. Binary-check + guide install per-OS (pola W2-A ssh-missing-check).

**Di mana Ansible jalan (kontradiksi #2 DIKUNCI nano):** spawn `ansible-playbook` via PTY lokal, pola
**IDENTIK** `local_pty_spawn` (lib.rs:1768) + `openTerminal(host,{local:true})` (main.ts). Binary check
saat buka panel: tak ada → guide install per-OS + blok run (pola W2-A). **Auto-install dari dalam app =
wave berikutnya** — jangan desain di sini.

**Wave-2 parking lot (JANGAN desain, catat saja):**
- Auto-install Ansible dari dalam app (pipx/apt/brew Unix, WSL Windows — layak desain sendiri).
- Playbook dari Git repo private (tambah surface OAuth/token).

---

## 1. Clarifying Questions (front-loaded — Quick Spec; tiap jawaban + rekomendasi)

> Jawab DULU sebelum dokumen dipakai. Yang LOCKED = sudah diputus nano/MEKL; sisanya rekomendasi PM
> yang perlu nano/MEKL konfirmasi (lihat OPEN QUESTIONS §9).

1. **Inventory ↔ `~/.ssh/config` — relasi mana?** (nano suruh putuskan, JANGAN tumpang tindih). 
   Rekomendasi **(LOCK ke ini)**: hosterm TIDAK menulis file inventory persisten ke `~/.ssh/config` dan
   TIDAK pernah menyentuh config. Host hosterm = sumber READ-ONLY; saat run, hosterm meng-**generate
   inventory ephemeral** (temp file 0600 di app-data, dihapus setelah run) dari daftar host yang
   dirender sidebar. Alias hosterm = `inventory_hostname`; `HostName`/`User`/`Port`/`IdentityFile`
   dipetakan ke `ansible_host`/`ansible_user`/`ansible_port`/`ansible_ssh_private_key_file`. **Grup** =
   turunan prefix alias sebelum `-` (`prod-web` → grup `prod`), persis derivasi Fleet Run FR-FL2
   (main.ts:346). TIDAK pernah menulis direktif `Group` ke config (ssh asli gagal keras).
   **Alasan:** satu sumber kebenaran host (PRD §1.1) tetap utuh; nol file persisten baru di `~/.ssh`;
   inventory = proyeksi, bukan store kedua yang bisa drift.

2. **Ansible pakai `ansible.cfg`/`ANSIBLE_CONFIG`?** Rekomendasi: hosterm set `ANSIBLE_CONFIG` ke file
   ephemeral yang IA generate (app-data, 0600), TIDAK memakai `ansible.cfg` liar di cwd user (hindari
   eksekusi config tak terduga). Default aman di-pin di env (§2.3).

3. **Host key checking ON/OFF?** (nano minta ADR kecil). Rekomendasi **(ADR-A1)**: ikut perilaku ssh
   hosterm = **host key checking tetap ON** (`ANSIBLE_HOST_KEY_CHECKING=True`). hosterm spawn real ssh
   dengan known_hosts user yang sama; auto-accept key baru = melemahkan postur (sama seperti Fleet Run
   FR-FL* fail-closed). Host yang key-nya belum dikenal → task gagal dengan pesan arahan (buka 1 terminal
   tab ke host itu sekali untuk TOFU), BUKAN auto-accept.

4. **Kredensial — mekanisme paling aman (argv terlihat di `ps`)?** Rekomendasi **(LOCK, ADR-A2)**:
   - **Key/agent host:** inventory ephemeral mencantumkan `ansible_ssh_private_key_file` = path
     `IdentityFile` yang SUDAH ada di config user (path, bukan isi key). Tak ada secret di argv.
   - **Password host (sidecar):** JANGAN pernah taruh password di argv/`--extra-vars`/env yang kelihatan
     di `ps`/IPC. **v1 = key/agent host SAJA**; password host → status `skipped` dengan reason eksplisit
     (pola IDENTIK Fleet Run FR-FL20/21 keputusan 3). Password-host Ansible = wave-2 (butuh architect
     buktikan mekanisme tanpa expose: `sshpass`-via-fd / `--vault` / prompt-via-PTY). Ini menjaga
     janji zero-knowledge client-side hosterm (password cuma di sidecar 0600, tak pernah cross IPC).

5. **Playbook source v1.** LOCKED nano: **folder workspace lokal** (folder/file picker). Rekomendasi:
   panel "Ansible" punya tombol "Pilih playbook" → file picker `*.yml`/`*.yaml`; hosterm baca path-nya
   (tidak menyalin), validasi ekstensi + baca sebagai teks UTF-8 (tolak binary / >2 MiB, sama ambang
   FR-D2/Fleet). Git private repo = wave-2.

6. **Dry-run `--check` — apa persisnya "ringkasan host & task yang akan berubah"?** Rekomendasi: jalankan
   `ansible-playbook --check --diff` dulu; parse ringkasan `PLAY RECAP` (ok/changed/unreachable/failed
   per host) + tampilkan baris "N host akan berubah, M task changed". Tombol **Run** (real, tanpa
   `--check`) DISABLED sampai dry-run sukses sekali untuk playbook+target itu. Dry-run WAJIB = lapisan
   keamanan, bukan cuma UX (§4 security).

7. **Streaming output.** Rekomendasi: run (check & real) spawn via PTY lokal, output stream ke tab
   terminal (pola `openTerminal` local) + panel ringkasan status per host dari `PLAY RECAP`. Output
   remote-origin (nama host, task output) = input hostile → render via `esc()`/`escAttr()` (ADR-8).

8. **History run — simpan apa + di mana?** Rekomendasi **(LOCK)**: app-data JSON
   (`data_dir()/hosterm/ansible-history.json`, pola `settings.json` INV-22, 0600, atomic write). Per
   entry: playbook path, target host/group, mode (check/real), waktu mulai, durasi, rc, ringkasan
   `PLAY RECAP`. TIDAK simpan output penuh (hindari bloat + leak); TIDAK pernah simpan secret.
   Retensi: 200 entry terakhir (FIFO).

9. **Binary tidak ada.** LOCKED nano: deteksi saat buka panel → guide install per-OS + blok run (pola
   W2-A `check_binaries`/`openSshGuide`/`recheckBinaries`). Perluas `BinCheck` dengan
   `ansible_missing: bool` ATAU command baru `check_ansible` (architect putuskan; rekomendasi: command
   terpisah `check_ansible` karena beda lifecycle — dicek saat buka panel, bukan saat launch).

10. **Tutup panel saat run jalan.** Rekomendasi: pola Fleet — tutup panel = Stop (konfirmasi in-app),
    tidak ada run hantu background. `pty_kill` mengirim SIGHUP ke `ansible-playbook` lokal (child-nya =
    ssh tiap host dapat SIGHUP juga; task remote yang sudah jalan lanjut di host — sama seperti Ctrl-C
    CLI, didokumentasikan sebagai edge case).

---

## 2. Arsitektur

### 2.1 Pola transport (LOCK)

`ansible-playbook` di-spawn sebagai PTY child lokal — **persis** `local_pty_spawn` (lib.rs:1768) dengan
dua beda: (a) program = `ansible-playbook` (bukan default shell), (b) argv = array yang dibangun backend
dari input tervalidasi. **Argv = array langsung ke `CommandBuilder`, TIDAK string shell** (INV-14/INV-27):

```
argv (check):  ansible-playbook  -i <inv_ephemeral>  --check  --diff  --limit <targets>  <playbook_path>
argv (real):   ansible-playbook  -i <inv_ephemeral>              --limit <targets>  <playbook_path>
```

- `<playbook_path>`, `<targets>` = satu elemen argv masing-masing, BUKAN diinterpolasi ke string shell.
- `<inv_ephemeral>` = path temp file inventory yang digenerate backend (0600, app-data, dihapus setelah run).
- Password/secret TIDAK PERNAH jadi elemen argv (ADR-A2).

### 2.2 Inventory builder

- Sumber READ-ONLY = daftar host hosterm yang dirender sidebar (sama query seperti Fleet Run; wildcard
  `*`/`?` dikecualikan).
- Format generate = INI standar Ansible (atau YAML — architect pilih; INI lebih ringkas untuk map
  alias→vars). Grup dari prefix alias sebelum `-`.
- Per host entry: `<alias> ansible_host=<HostName> ansible_user=<User> ansible_port=<Port>
  ansible_ssh_private_key_file=<IdentityFile>`. Password host DILEWATI (skipped, §ADR-A2).
- File ephemeral 0600 di `data_dir()`, dihapus setelah run selesai/stop (INV: nol file persisten baru di
  `~/.ssh`; store app-data isolated dari config/sidecar — INV-22).

### 2.3 Env posture (ADR-A1)

Backend set env eksplisit saat spawn (tidak mewarisi `ansible.cfg` liar):
- `ANSIBLE_HOST_KEY_CHECKING=True` (host key checking ON, ikut postur ssh hosterm).
- `ANSIBLE_CONFIG=<ephemeral cfg app-data 0600>` (default aman di-pin; tak pakai cfg di cwd user).
- `ANSIBLE_NOCOLOR` opsional untuk parse `PLAY RECAP` yang bersih (architect putuskan).

### 2.4 History store (app-data, INV-22)

`data_dir()/hosterm/ansible-history.json`, atomic write (temp→chmod 0600→rename), pola `settings.json`.
Isolated dari `~/.ssh/config` + sidecar. Korup → default (array kosong), byte korup tak disentuh sampai
write sah (pola INV-24).

---

## 3. Use Cases v1 (5 minimal)

| # | Use case | Surface |
|---|---|---|
| UC-ANS1 | Operator buka panel Ansible; binary `ansible-playbook` tak ada → guide install per-OS, run diblok, tombol re-check (pola W2-A) | Panel Ansible + modal guide |
| UC-ANS2 | Operator pilih playbook YAML + target host/group; hosterm generate inventory ephemeral dari host hosterm (key/agent host; password host skipped) | Panel Ansible |
| UC-ANS3 | Operator jalankan **dry-run `--check` WAJIB** → ringkasan host & task yang akan berubah (`PLAY RECAP` + diff); tombol Run real DISABLED sampai dry-run sukses | Panel Ansible + tab terminal PTY |
| UC-ANS4 | Operator konfirmasi eksplisit → real run; output stream ke tab terminal, status per host; secret tak pernah lewat argv/IPC | Tab terminal PTY + panel status |
| UC-ANS5 | Run selesai → dicatat di history (playbook, target, mode, waktu, durasi, rc, recap); operator browse history lokal | Panel history (app-data) |

---

## 4. Requirements (EARS)

**Non-goals v1 (eksplisit):** auto-install Ansible dari app; playbook dari Git private repo; password-host
Ansible run; templating/vault editing dari UI; ad-hoc `ansible` module run (hanya playbook); menulis file
inventory/`Group`/apa pun ke `~/.ssh/config`; menyimpan secret atau output run penuh; rolling/serial dengan
failure-budget; target non-POSIX (Windows SSH server).

| ID | Requirement (EARS) | Prioritas |
|---|---|---|
| FR-ANS1 | WHEN panel Ansible dibuka THE SYSTEM SHALL mendeteksi apakah `ansible-playbook` resolvable di PATH (read-only, nol spawn) | must |
| FR-ANS2 | IF `ansible-playbook` tidak ada THEN THE SYSTEM SHALL menampilkan guide install per-OS (apt/dnf/pacman/pipx di Unix; brew di macOS; WSL note di Windows), memblok semua run, dan menyediakan tombol re-check yang mengulang deteksi di tempat (pola W2-A FR-E2) | must |
| FR-ANS3 | WHEN panel Ansible menampilkan picker host THE SYSTEM SHALL merender satu baris pilih-per-host untuk tiap host sidebar (wildcard `*`/`?` dikecualikan), dengan alias, `user@HostName`, dan metode auth — grup = turunan prefix alias sebelum `-` | must |
| FR-ANS4 | WHEN operator memilih host password-auth sebagai target THE SYSTEM SHALL menandainya `skipped` dengan reason eksplisit dan TIDAK menyertakannya di inventory ephemeral (v1 key/agent-host only, ADR-A2) | must |
| FR-ANS5 | WHEN operator memilih playbook THE SYSTEM SHALL menerima path file `*.yml`/`*.yaml` via picker, membacanya sebagai teks UTF-8, dan menolak binary/invalid-UTF-8/>2 MiB dengan pesan yang menyebut sebabnya sebelum run | must |
| FR-ANS6 | WHEN run disiapkan THE SYSTEM SHALL meng-generate file inventory ephemeral (app-data, 0600) dari host hosterm terpilih — alias=`inventory_hostname`, `HostName`/`User`/`Port`/`IdentityFile` → `ansible_host`/`ansible_user`/`ansible_port`/`ansible_ssh_private_key_file` — dan menghapusnya setelah run selesai/stop; TIDAK pernah menulis `~/.ssh/config` | must |
| FR-ANS7 | WHEN operator menjalankan dry-run THE SYSTEM SHALL meng-spawn `ansible-playbook` sebagai PTY child lokal dengan argv `-i <inv> --check --diff --limit <targets> <playbook>` dibangun sebagai array langsung (TIDAK `sh -c`/string shell, INV-14/INV-27) | must |
| FR-ANS8 | WHEN dry-run selesai THE SYSTEM SHALL menampilkan ringkasan `PLAY RECAP` per host (ok/changed/unreachable/failed) + indikasi "N host akan berubah, M task changed"; output remote-origin dirender via `esc()`/`escAttr()` (ADR-8) | must |
| FR-ANS9 | **Dry-run WAJIB (lapisan keamanan, bukan hanya UX):** THE SYSTEM SHALL menjaga tombol Run-real DISABLED sampai dry-run `--check` sukses sekali untuk kombinasi playbook+target saat ini; mengubah playbook/target me-reset gate | must |
| FR-ANS10 | WHEN operator menekan Run-real THE SYSTEM SHALL meminta konfirmasi eksplisit dengan ringkasan host & task yang akan berubah SEBELUM spawn; Run-real tidak pernah auto-jalan (sejajar INV-25 approval posture) | must |
| FR-ANS11 | WHEN operator mengonfirmasi Run-real THE SYSTEM SHALL meng-spawn `ansible-playbook` PTY lokal dengan argv tanpa `--check`, stream output ke tab terminal, dan menampilkan status per host | must |
| FR-ANS12 | WHEN run (check atau real) di-spawn THE SYSTEM SHALL set env `ANSIBLE_HOST_KEY_CHECKING=True` dan `ANSIBLE_CONFIG=<ephemeral cfg app-data>` — host key checking ON (ikut postur ssh hosterm, ADR-A1), tidak mewarisi `ansible.cfg` di cwd user | must |
| FR-ANS13 | THE SYSTEM SHALL tidak pernah menaruh password/secret ke argv, `--extra-vars`, env yang terlihat di `ps`, atau payload IPC; secret tetap hanya di sidecar 0600 (zero-knowledge posture) | must |
| FR-ANS14 | WHEN sebuah run selesai THE SYSTEM SHALL mencatat satu entry history di `data_dir()/ansible-history.json` (playbook, target, mode, waktu mulai, durasi, rc, ringkasan recap) via atomic write 0600; store ini TIDAK pernah membaca/menulis `~/.ssh/config` atau sidecar (INV-22); retensi 200 entry FIFO | must |
| FR-ANS15 | IF `ansible-history.json` hilang/korup THEN THE SYSTEM SHALL mengembalikan history kosong dan membiarkan byte korup tak tersentuh sampai write sah berikutnya (pola INV-24) | must |
| FR-ANS16 | WHEN operator menutup panel Ansible saat run berjalan THE SYSTEM SHALL meminta konfirmasi dan memperlakukan tutup = Stop (`pty_kill` → SIGHUP ke `ansible-playbook` lokal); tidak ada run hantu background | should |
| FR-ANS17 | IF host target key-nya belum dikenal (host key verification failed) THEN THE SYSTEM SHALL menampilkan task gagal untuk host itu dengan pesan arahan (buka 1 terminal tab ke host sekali untuk TOFU), TANPA auto-accept key (ADR-A1 fail-closed) | must |

---

## 5. Non-Functional Requirements (TERUKUR)

| ID | Requirement |
|---|---|
| NFR-ANS1 | Nol dependency baru Rust/JS; nol Ansible/python dibundel. `ansible-playbook` = binary OS yang user install sendiri (opsional). |
| NFR-ANS2 | Deteksi binary = read-only, nol spawn, nol network; selesai < 50 ms untuk PATH tipikal. |
| NFR-ANS3 | Nol network call di idle (PRD NFR-3); satu-satunya traffic = ssh yang di-trigger ansible-playbook saat run (user-initiated). |
| NFR-ANS4 | Byte-identity: sebuah run Ansible TIDAK mengubah `~/.ssh/config` atau sidecar (hash+mtime identik sebelum/sesudah) — cargo test gate. |
| NFR-ANS5 | Grep gate: `sh -c`/`bash -c`/`cmd /C` = 0 hit di jalur Ansible (backend + frontend) — argv langsung (INV-14). |
| NFR-ANS6 | Inventory ephemeral + cfg ephemeral = 0600, dihapus setelah run; nol file persisten baru di `~/.ssh`. |

---

## 6. Design sketch (UI — pola yang ada, architect/lead finalize)

- Tab baru di sidebar "Ansible" (sejajar tombol "Local" / Fleet entry; pola UI yang sudah ada).
- Panel: (a) picker playbook (file picker), (b) picker host/group (reuse komponen Fleet picker),
  (c) tombol Dry-run → ringkasan check, (d) tombol Run-real (disabled sampai dry-run sukses) +
  dialog konfirmasi, (e) tab terminal PTY untuk streaming, (f) sub-panel History.
- Semua render konten remote-origin via `esc()`/`escAttr()`, markup statis dari `index.html` (ADR-8).

---

## 7. ADR kecil (di-lock di spec, bukan open)

- **ADR-A1 (host key checking):** `ANSIBLE_HOST_KEY_CHECKING=True` — ON, ikut postur ssh hosterm;
  fail-closed untuk host key tak dikenal (tanpa auto-accept). Alasan: blast-radius tool tak boleh
  diam-diam menerima key baru; konsisten Fleet Run fail-closed.
- **ADR-A2 (kredensial):** key/agent host → `ansible_ssh_private_key_file` = path IdentityFile existing
  (path, bukan isi); password host = `skipped` di v1 (butuh architect buktikan mekanisme tanpa expose
  di wave-2). Secret TIDAK PERNAH di argv/env/IPC. Alasan: argv terlihat di `ps`; jaga zero-knowledge
  client-side hosterm.
- **ADR-A3 (binary check lifecycle):** `check_ansible` command terpisah (bukan diperluas ke
  `check_binaries` launch-time), dicek saat buka panel. Alasan: Ansible opsional → beda lifecycle dari
  ssh/sftp yang wajib.

---

## 8. Test plan (verify loop — extend STATE.md baseline, jangan turunkan)

- **cargo (lib.rs/ansible module):** argv-builder test (array, bukan string; INV-14 marker-file fixture);
  inventory-generate test (alias→vars map, password host skipped); byte-identity config/sidecar tak
  berubah lintas run (NFR-ANS4); history atomic write + korup→default (FR-ANS15); check_ansible resolve.
- **build:** 0 TS error.
- **smoke (headless-smoke.mjs):** panel Ansible render; binary-missing guide (mock `ansible_missing:true`);
  dry-run-wajib gate (Run-real disabled sampai check sukses); konfirmasi dialog muncul sebelum real run;
  hostile-bytes di output `PLAY RECAP` → 0 CSP violation, 0 script element (ADR-8); grep `sh -c|bash -c`
  = 0 di jalur Ansible (NFR-ANS5).

---

## 9. OPEN QUESTIONS (butuh nano/MEKL konfirmasi sebelum PRD dikunci penuh)

> Semua keputusan nano LOCKED sudah dipatuhi. Yang di bawah = rekomendasi PM yang belum MEKL eksplisit
> putuskan; nano yang teruskan.

- **OQ-ANS1:** Inventory relasi (CQ#1) — konfirmasi "inventory = proyeksi ephemeral read-only dari host
  hosterm, nol file persisten" sebagai keputusan final (nano suruh pm putuskan; PM sudah pilih, minta
  stempel supaya architect tak re-litigasi).
- **OQ-ANS2:** Password-host Ansible = `skipped` di v1 (CQ#4/ADR-A2) — sama seperti Fleet Run v1.
  Konfirmasi ini bukan regresi yang mengejutkan MEKL (MEKL minta "kredensial yang sudah ada di hosterm";
  password sidecar TIDAK dipakai di v1, hanya key/agent).
- **OQ-ANS3:** Format inventory INI vs YAML (CQ§2.2) — PM rekomendasi INI; final = architect, tapi kalau
  MEKL punya preferensi catat di sini.
- **OQ-ANS4:** History retensi 200 entry FIFO + tidak simpan output penuh (CQ#8) — konfirmasi cukup.

---

## 10. Wave-2 parking lot (JANGAN desain — catat di PRD §7)

1. Auto-install Ansible dari dalam app (pipx/apt/brew Unix, WSL Windows).
2. Playbook dari Git repo private (surface OAuth/token).
3. Password-host Ansible run (mekanisme feed secret tanpa expose — architect spike).
4. Ad-hoc module run, vault editing dari UI, rolling/serial failure-budget.
