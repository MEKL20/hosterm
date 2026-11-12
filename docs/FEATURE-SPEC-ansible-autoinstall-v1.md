# FEATURE-SPEC: Ansible auto-install (v1)

> Milik epic produk — sitasi `PRD.md §7` (parking-lot row, ditambahkan pm).
> EARS untuk semua behavior. Initiative tunggal → dokumen ini FEATURE-SPEC, bukan PRD.
> Boss intent (MEKL via Tier-0 nano, card t_994dddd2): "(1) one-click install untuk
> Linux/macOS, (2) path Windows opsional yang benar-benar jalan". Constraint nano diperlakukan
> sebagai LOCK, bukan bahan debat. Fakta teknis yang sudah terverifikasi (di bawah) dikutip,
> tidak dipertanyakan ulang.

| Field | Value |
|---|---|
| Author | crew-coding-pm |
| Status | Speced — awaiting nano re-verify → architect SDD. No code, no commits (PM = doc only). |
| Baseline | main `7621b72` (post-v0.15.4); verify loop STATE.md: re-measure at dispatch (cargo/build/smoke) |
| Source | `docs/FEATURE-SPEC-ansible-v1.md` §10 item 1 (wave-2 parking lot) · `docs/PRD.md` §7 parking-lot row |
| Non-negotiables | spawn OS **real installer** (pipx/brew/apt-in-WSL) via PTY lokal, pola IDENTIK `ansible_run`/UC11 · argv langsung, TIDAK `sh -c`/shell string (INV-14) · secrets tak pernah lewat argv/env/logs/UI (INV-28/29/31, ADR-A2) · `host_key_checking = True` tetap (ADR-A1) · install HANYA atas klik eksplisit user · nol dependency baru tanpa justifikasi |
| Traceability | FR-ANS1/ANS2 (binary check + guide) · FR-ANS12 (env posture) · FR-ANS16 (tutup panel = Stop) · NFR-ANS1/ANS2 (nol embed, check read-only) · INV-14 (no shell interp) · INV-22 (store app-data) · INV-28/29/31 (argv/secrets) · ADR-A1/A2 · `HOSTERM_SFTP_BIN` hook (lib.rs:1203/1420/2508) |
| Feature name (LOCKED) | **Ansible auto-install (v1)** |

---

## 0. Scope lock (dari nano brief — bukan bahan debat)

**v1 = dua unit:**

- **Unit A — one-click install (Linux/macOS).** Tombol Install di guide block yang sudah ada;
  installer no-root (pipx / brew); output stream ke console PTY; auto re-check setelah selesai.
- **Unit B — path Windows opsional (WSL).** Deteksi WSL, install ansible di dalam distro,
  invoki ansible yang hidup di dalam WSL dengan **semua path ditranslasi** (`wslpath`).

**Excluded (named, JANGAN desain — item §10 lain):** playbook dari Git repo private
(surface OAuth/token); password-host Ansible run; ad-hoc module run; vault editing dari UI;
rolling/serial failure-budget. Semua tetap backlog.

**Fakta teknis yang sudah terverifikasi (kutip, jangan re-litigasi):**

1. Ansible **tidak punya control node native Windows**. Docs Ansible: *"Windows without WSL is
   not natively supported as a control node"* dan *"Ansible cannot run on Windows as the control
   node due to API limitations on the platform."*
2. `pip install ansible` di Windows **terpasang**, tapi CLI mati saat startup:
   `check_blocking_io()` → `os.get_blocking(fd)` → `OSError: [WinError 1] Incorrect function`.
   Struktural: worker model ansible-core pakai `fork()` + `fcntl`/`grp` (POSIX-only).
   Bundling Python+Ansible ke exe Windows **tidak memperbaiki** ini.
3. **Binary WSL tidak ada di Windows PATH.** WSL menambahkan Windows PATH ke dalam Linux,
   tidak pernah sebaliknya. `wsl -- sudo apt install ansible` menginstall di dalam distro;
   hosterm di Windows tetap melaporkan `missing`.
4. hosterm menyerahkan path **Windows** ke ansible: playbook (file picker), inventory/cfg
   ephemeral di `data_dir()`, `IdentityFile` dari `~/.ssh/config`, dan env `ANSIBLE_CONFIG`.
   Proses Linux butuh bentuk `/mnt/c/...`.
5. GUI Linux tak punya root: `sudo apt install` tidak bisa prompt dari Tauri webview.
   `pipx install --include-deps ansible` user-space, tanpa root.

---

## 1. Decisions (LOCKED — concrete, bukan daftar opsi)

| ID | Decision |
|---|---|
| D-AI1 | **Linux installer = `pipx`.** No-root, distro-independent, isolate deps in venv. Resolusi: `pipx` di PATH → `python3 -m pipx` → jika keduanya tak ada, **tidak ada one-click** (lihat D-AI4). |
| D-AI2 | **macOS installer = `brew`.** No-root (brew menolak root), de-facto standard, taruh binary di PATH dir. Resolusi: `brew` di PATH → `/opt/homebrew/bin/brew` → `/usr/local/bin/brew`. |
| D-AI3 | **apt/dnf/pacman = fallback MANUAL saja** (copyable command), tidak pernah di-run hosterm. Alasan: butuh root; hosterm tak punya jalur eskalasi privilege dari Tauri webview; console read-only tidak bisa prompt password (dan password di UI = secret masuk UI, langgar posture). |
| D-AI4 | **Tanpa root, fallback lanjut manual di terminal user.** One-click pipx tetap jalan tanpa root; jika pipx tak ada, user menjalankan `sudo apt install -y ansible` (dnf/pacman) sendiri. Guide menampilkan perintah + tombol copy. |
| D-AI5 | **Trigger = tombol Install di `.ans-guide-actions`** (sebelah Re-check) + console read-only `#ans-install-log` di guide block. Output stream via family event PTY yang **sudah ada** (`pty://output|exit/{id}`), render `textContent` (ADR-8). |
| D-AI6 | **Console read-only, TANPA input field.** Installer butuh interaksi hanya untuk password (sudo) — dan kita tidak pernah memakai sudo di one-click. Tanpa field input, mustahil secret masuk UI. |
| D-AI7 | **Post-install = auto re-run `check_ansible`** (pola FR-ANS2) saat `pty://exit/{id}`; unlock runner iff binary sekarang resolvable. Gagal → status line + manual commands tetap. |
| D-AI8 | **Ekstensi resolver (WAJIB):** `ansible_missing` memprobe PATH **+** `$HOME/.local/bin` (Unix) / `%USERPROFILE%\.local\bin` (Windows) / `/opt/homebrew/bin` (macOS). Read-only, 0 spawn, tetap <50 ms. Tanpa ini, install pipx/brew yang sukses tetap terbaca `missing` (GUI app dapat PATH minimal) dan user menganggap install gagal. |
| D-AI9 | **Deteksi WSL = `check_wsl()`**: spawn `wsl.exe --list --quiet` (argv array, no shell). rc=0 + stdout non-kosong → `{installed:true, distro:<nama>}`; rc=0 + kosong → `{installed:true, distro:null}`; rc≠0 / spawn gagal → `{installed:false}` (fail-closed). Dipanggil hanya di Windows saat `missing`. |
| D-AI10 | **WSL absent / tanpa distro → hosterm TIDAK menjalankan `wsl --install`.** Guide menampilkan perintah (copyable) + tiga fakta: butuh admin (UAC), butuh reboot, dan reboot **menginterupsi hosterm** (user harus relaunch lalu Re-check). |
| D-AI11 | **Install di dalam WSL = apt sebagai root via `wsl --user root`.** Dua spawn terpisah: `apt-get update`, lalu `apt-get install -y ansible`. Password-free di image default (Microsoft docs sendiri memakai `wsl -u root` untuk password recovery). Root di WSL tidak bisa melampaui permission user Windows. |
| D-AI12 | **Distro non-apt (Fedora/Arch/dll): manual-only** (`wsl -- sudo dnf install -y ansible` / `pacman`). One-click v1 = apt (Ubuntu/Debian-family, distro default `wsl --install`). |
| D-AI13 | **Invokasi (CRUX) = backend-native `wsl.exe --exec ansible-playbook`, BUKAN shim file.** Backend yang membangun argv (sudah ada di `build_ansible_argv`), jadi translasi path bisa eksak + unit-testable. `HOSTERM_ANSIBLE_BIN` dipertahankan sebagai hook override/testability, meniru `HOSTERM_SFTP_BIN`. |
| D-AI14 | **Translasi path via `wslpath -a -u <winpath>`** (satu subprocess per path, argv array), dengan fallback pure-Rust `win_to_wsl_path()`. Diterapkan ke **playbook, inventory ephemeral, cfg ephemeral, semua `IdentityFile` (setelah ekspansi `~`), dan env `ANSIBLE_CONFIG`**. |
| D-AI15 | **Run-unlock di Windows = `missing && !(wsl_installed && distro && wsl_ansible_present)`.** `check_ansible` (PATH-only, 0 spawn) tidak pernah memprobe WSL; probe WSL = command terpisah yang user-triggered. |
| D-AI16 | **Known limitation (documented, bukan auto-fix):** `IdentityFile` yang menunjuk drive Windows (`/mnt/c/...`) bisa ditolak ssh-in-WSL karena semantik permission DrvFs. v1 = gagal dengan pesan jelas; **tidak ada copy key ke WSL** (OQ-AI1). |
| D-AI17 | **Tutup panel saat install → konfirmasi cancel + `pty_kill`** (pola FR-ANS16). Nol installer yatim. |

---

## 2. Arsitektur

### 2.1 Unit A — install pipeline (Linux/macOS)

```
[Guide block]  #ans-install  (primary)   #ans-recheck
     │ click
     ▼
ansible_install()  (backend, lib.rs)  → resolves platform + installer ladder
     │ spawns installer as PTY child (portable-pty, SAME PtyState/NEXT_ID map)
     ▼
pty://output/{id}  ──▶  #ans-install-log  (read-only <pre>, textContent)
pty://exit/{id}    ──▶  auto ansRefreshCheck()  (FR-ANS2)
```

**Ladder per platform (LOCKED):**

| Platform | One-click sequence (argv array, tiap baris = satu spawn) | Fallback (manual, copyable) |
|---|---|---|
| Linux | `pipx install --include-deps ansible` (atau `python3 -m pipx …`) | `sudo apt install -y ansible` / `sudo dnf install -y ansible` / `sudo pacman -S --noconfirm ansible` |
| macOS | `brew install ansible` | Homebrew installer one-liner + `brew install ansible` |
| Windows | lihat §2.2 | — |

- Semua spawn = **argv array langsung ke `CommandBuilder`**, nol string shell (INV-14).
- Nol root, nol password, nol secret di argv/env.
- `ansible_install()` mengembalikan `Result<u32, String>` (pty id); error = token opak
  `ansible-install:<code>` (nol host/path data).

**Concreteness flags:** `pipx`/`brew` resolution dilakukan backend (frontend tidak menebak UA
untuk aksi — UA sniffing yang ada di `ansRenderGuide()` diganti oleh field `os` dari
`check_ansible`, lih. FR-AI1).

**Post-install (LOCKED):**

1. `pty://exit/{id}` → frontend memanggil `ansRefreshCheck()` (FR-ANS2).
2. `check_ansible` sekarang memprobe PATH **+** `$HOME/.local/bin` **+** `/opt/homebrew/bin`
   (D-AI8) → pipx/brew install yang sukses langsung terdeteksi tanpa restart shell.
3. Sukses → `#ans-guide` hide, `#ans-body` show. Gagal → `#ans-install-status` =
   "Installation finished but `ansible-playbook` is still not available. Run the command in
   your terminal, then press Re-check." + manual commands tetap.

**Busy/concurrency:** `ansState.installing` — satu install pada satu waktu; tombol Install
disabled + label "Installing…" saat jalan; console di-cap (ring buffer 64 KB terakhir) agar
memori tak terbatas.

### 2.2 Unit B — Windows (WSL)

#### 2.2.1 Deteksi & state machine (LOCKED)

```
panel open (Windows, missing)
      │
      ▼
check_wsl()  ── wsl.exe --list --quiet
      │
      ├─ installed:false ─────────────────────────────▶ [WSL-required block]
      │                                                  cmd: wsl --install (copy)
      │                                                  + admin + reboot + interupsi app
      ├─ installed:true, distro:null ─────────────────▶ [WSL-required block]
      │                                                  cmd: wsl --install -d Ubuntu
      └─ installed:true, distro:"Ubuntu"
              │ Re-check
              ▼
        ansible_wsl_probe()  ── wsl.exe --distribution Ubuntu --exec ansible-playbook --version
              │
              ├─ present:false ──▶ [Install Ansible inside WSL] + manual cmd
              └─ present:true  ──▶ runner body (UNLOCKED)
```

- `check_wsl()`: 1 query `wsl.exe`, **tanpa shell distro**, <500 ms. Tidak memprobe ansible.
- `ansible_wsl_probe()`: 1 spawn `wsl.exe --exec ansible-playbook --version`, ~1–3 s,
  **hanya user-triggered** (Re-check / auto re-check pasca-install). Bukan bagian panel-open
  check (NFR-ANS2 tetap 0-spawn untuk `check_ansible`).
- Parse stdout `wsl --list --quiet`: trim `\r\n`/whitespace; baris pertama non-kosong = distro.

#### 2.2.2 Install di dalam WSL (LOCKED)

Dua spawn terpisah (tanpa `&&`, tanpa shell — INV-14), sebagai **root** di distro:

```
wsl.exe --distribution <distro> --user root --exec apt-get update
wsl.exe --distribution <distro> --user root --exec apt-get install -y ansible
```

- Output stream ke console yang sama (`pty://output/{id}`), read-only.
- `apt-get` (bukan `apt`) = non-interactive-friendly, stabil di script.
- Gagal (root terkunci / distro non-apt / jaringan) → console menampilkan error native;
  guide menampilkan manual commands (apt/dnf/pacman) + Re-check. Run tetap diblok.
- `wsl --user root` password-free di image default; jika distro mengunci root, command gagal
  non-interaktif (tanpa prompt) → fallback manual. hosterm **tidak pernah** prompt password
  di dalam WSL.

#### 2.2.3 Invokasi & translasi path (CRUX — LOCKED)

`ansible_run` me-resolve launcher:

| Kondisi | program | argv |
|---|---|---|
| `HOSTERM_ANSIBLE_BIN` diset | nilai env (verbatim) | argv ansible polos (hook testability, meniru `HOSTERM_SFTP_BIN`) |
| Windows + WSL active | `wsl.exe` | `--distribution <distro> --exec ansible-playbook <translated args…>` |
| selain itu (Unix) | `ansible-playbook` | argv ansible polos (perilaku hari ini) |

**Translasi (D-AI14) — tabel eksplisit:**

| Elemen | Sumber | Translasi |
|---|---|---|
| playbook | picker (`req.playbook`, Windows path) | `wslpath -a -u` → `/mnt/c/...` |
| inventory ephemeral | `data_dir()/ansible-inv-<suffix>.ini` | `wslpath -a -u` → `/mnt/c/...` |
| cfg ephemeral | `data_dir()/ansible-cfg-<suffix>.ini` | `wslpath -a -u` → `/mnt/c/...` |
| `IdentityFile` (per host) | `~/.ssh/config` | ekspansi `~` ke path Windows nyata (`C:\Users\<me>\...`), lalu `wslpath -a -u` → `/mnt/c/Users/<me>/.ssh/...` |
| `ANSIBLE_CONFIG` (env) | cfg ephemeral | nilai yang **sudah ditranslasi** |

**Ordering yang WAJIB:** translasi `IdentityFile` dilakukan **sebelum** inventory body ditulis
(linux ansible membaca file itu), lalu file ditulis di path Windows (proses Linux membacanya
lewat `/mnt/c/...`). `-i` menerima path yang **sudah ditranslasi**.

**cwd:** child di-spawn dengan cwd = direktori parent playbook (bentuk Windows); `wsl.exe`
menerjemahkan cwd inherited → playbook-relative lookup (`roles/`, `files/`) resolve.
(TSD spike memverifikasi cwd semantics `wsl.exe`; constraint yang dibawa: playbook-relative
lookup `roles/`/`files/` harus resolve.)

**Env posture tidak berubah:** `ANSIBLE_HOST_KEY_CHECKING=True`, `ANSIBLE_NOCOLOR=1` (ADR-A1).
`--limit` dan `--check`/`--diff` polos (bukan path).

**Run-unlock (D-AI15):**

```
block run  ⟺  ansible_missing(PATH+probe dirs)  &&  !(wsl_installed && wsl_distro && wsl_ansible_present)
```

### 2.3 Posture invariant (cross-cutting)

- Install path **tidak pernah** membaca `~/.ssh/config` atau sidecar (INV-29/31); tidak menulis
  apa pun ke `~/.ssh` (INV-22/29).
- Console render via `textContent`/`esc()` — output installer = input hostile (ADR-8).
- Nol secret di argv/env/logs/UI (INV-28/31, ADR-A2). Password host tetap `skipped`.
- `host_key_checking = True` tetap (ADR-A1).
- Install hanya pada klik eksplisit; nol auto-install on launch/panel-open; nol background install.

---

## 3. Use Cases v1 (5 minimal)

| # | Use case | Surface |
|---|---|---|
| UC-AI1 | Operator Linux/macOS tanpa ansible membuka panel → guide menampilkan Install → satu klik → console stream pipx/brew → auto re-check → runner unlock | Panel Ansible + console guide |
| UC-AI2 | Operator Windows tanpa WSL → guide menampilkan WSL-required block (`wsl --install`, admin + reboot + interupsi app) → user install WSL, reboot, relaunch, Re-check → WSL block | Panel Ansible |
| UC-AI3 | Operator Windows dengan WSL, ansible tak ada di dalam distro → "Install inside WSL" → apt as root → probe → unlock; run di-invoke via `wsl.exe --exec ansible-playbook` dengan semua path ditranslasi | Panel Ansible + tab terminal PTY |
| UC-AI4 | Install gagal (pipx/brew tak ada; apt error; root terkunci) → console menampilkan error native; manual commands tetap; run tetap diblok | Panel Ansible + console guide |
| UC-AI5 | Operator menutup panel saat install jalan → konfirmasi cancel → `pty_kill`; nol installer yatim | Panel Ansible |

---

## 4. Requirements (EARS)

| ID | Requirement (EARS) | Prioritas |
|---|---|---|
| FR-AI1 | WHEN panel Ansible dibuka dan `ansible-playbook` tak resolvable THE SYSTEM SHALL menampilkan guide dengan aksi Install yang spesifik-platform (pipx Linux, brew macOS) + manual fallback commands, dan memblok semua run | must |
| FR-AI2 | WHEN user menekan Install di Linux THE SYSTEM SHALL menjalankan `pipx install --include-deps ansible` (resolusi `pipx` → `python3 -m pipx`) sebagai argv-array spawn langsung, tanpa root dan tanpa prompt password | must |
| FR-AI3 | WHEN user menekan Install di macOS THE SYSTEM SHALL menjalankan `brew install ansible` (resolusi `brew` → `/opt/homebrew/bin/brew` → `/usr/local/bin/brew`) sebagai argv-array spawn langsung, tanpa root | must |
| FR-AI4 | IF pipx (Linux) atau brew (macOS) tak resolvable THEN THE SYSTEM SHALL TIDAK menjalankan installer apa pun dan SHALL menampilkan manual fallback commands (apt/dnf/pacman; Homebrew installer + `brew install ansible`) dengan affordance copy | must |
| FR-AI5 | WHEN install berjalan THE SYSTEM SHALL stream output ke console read-only di guide block via family event PTY yang sudah ada (`pty://output|exit/{id}`), merender via `textContent` (ADR-8), men-disable tombol Install, dan SHALL TIDAK menyediakan input field apa pun | must |
| FR-AI6 | WHEN proses install keluar THE SYSTEM SHALL otomatis menjalankan kembali `check_ansible` (pola FR-ANS2) dan SHALL unlock runner iff binary sekarang resolvable; jika tidak, SHALL menampilkan status line + manual commands | must |
| FR-AI7 | THE SYSTEM SHALL me-resolve `ansible-playbook` di atas PATH **ditambah** `$HOME/.local/bin` (Unix) / `%USERPROFILE%\.local\bin` (Windows) / `/opt/homebrew/bin` (macOS), read-only dan spawn-free | must |
| FR-AI8 | WHEN user menutup panel Ansible saat install berjalan THE SYSTEM SHALL meminta konfirmasi dan SHALL memperlakukan "konfirmasi" sebagai cancel (`pty_kill`); nol installer yatim | should |
| FR-AI9 | THE SYSTEM SHALL memulai install HANYA pada klik eksplisit user; TIDAK PERNAH auto-install saat panel-open, launch, atau background | must |
| FR-AI10 | WHEN panel dibuka di Windows dan `ansible-playbook` tak resolvable THE SYSTEM SHALL mengquery presence WSL via `wsl.exe --list --quiet` (argv array, no shell) dan mengklasifikasi installed+distro / installed+no-distro / absent, fail-closed ke absent pada error | must |
| FR-AI11 | IF WSL absent (atau tanpa distro) THEN THE SYSTEM SHALL menampilkan WSL-required block dengan perintah `wsl --install`, menyatakan butuh administrator + reboot, menyatakan reboot menginterupsi hosterm, dan SHALL TIDAK menjalankan perintah itu | must |
| FR-AI12 | IF WSL terinstall dengan distro dan ansible tak ada di dalamnya THEN THE SYSTEM SHALL menawarkan one-click "Install Ansible inside WSL" yang menjalankan `wsl.exe --distribution <distro> --user root --exec apt-get update` lalu `… --exec apt-get install -y ansible` sebagai spawn argv-array terpisah, di-stream ke console yang sama | must |
| FR-AI13 | WHEN operator memicu Re-check di Windows THE SYSTEM SHALL mem-probe ulang ansible di dalam WSL via `wsl.exe --distribution <distro> --exec ansible-playbook --version` dan SHALL unlock runner iff probe sukses | must |
| FR-AI14 | **CRUX:** WHEN run Ansible disiapkan di Windows dengan path WSL active THE SYSTEM SHALL spawn `wsl.exe --distribution <distro> --exec ansible-playbook` (atau override `HOSTERM_ANSIBLE_BIN`, meniru `HOSTERM_SFTP_BIN`) dan SHALL mentranslasi **setiap** path ke bentuk WSL via `wslpath -a -u` sebelum spawn — playbook, inventory ephemeral, cfg ephemeral, semua `IdentityFile` (setelah ekspansi `~`), dan env `ANSIBLE_CONFIG` — serta SHALL set cwd child ke direktori parent playbook | must |
| FR-AI15 | THE SYSTEM SHALL TIDAK PERNAH menaruh secret/kredensial di argv/env/logs/UI install; install path TIDAK PERNAH membaca `~/.ssh/config` atau sidecar; `ANSIBLE_HOST_KEY_CHECKING=True` dan skip password-host (ADR-A2) SHALL tetap berlaku | must |
| FR-AI16 | IF `IdentityFile` yang ditranslasi menunjuk drive Windows (`/mnt/c/...`) dan ssh-in-WSL menolaknya karena permission THEN THE SYSTEM SHALL menggagalkan task host itu dengan pesan jelas dan SHALL TIDAK menyalin key ke WSL | must |

---

## 5. Non-Functional Requirements (TERUKUR)

| ID | Requirement |
|---|---|
| NFR-AI1 | Nol dependency Rust/JS baru; nol Python/Ansible dibundel (NFR-ANS1 tetap). Satu-satunya tool eksternal = installer OS + `wsl.exe`/`wslpath.exe` (ship dengan Windows/WSL). |
| NFR-AI2 | `check_ansible` tetap read-only, 0 spawn, <50 ms (NFR-ANS2); probe dirs tambahan = static list, di-dedup terhadap PATH. |
| NFR-AI3 | `check_wsl` = 1 query `wsl.exe --list --quiet`, tanpa shell distro, <500 ms; `ansible_wsl_probe` user-initiated, <5 s. |
| NFR-AI4 | Grep gate: `sh -c`/`bash -c`/`cmd /C`/`powershell` = 0 hit di jalur install + WSL (backend + frontend) — argv langsung (INV-14). |
| NFR-AI5 | Byte-identity: install atau WSL probe TIDAK mengubah `~/.ssh/config` atau sidecar (hash+mtime identik sebelum/sesudah) — cargo test gate. |
| NFR-AI6 | Console install render byte OS-origin via `textContent`; 0 CSP violation, 0 script element (ADR-8). |
| NFR-AI7 | Satu install pada satu waktu; tombol disabled saat jalan; console ring buffer 64 KB. |
| NFR-AI8 | Translasi path: `win_to_wsl_path()` pure-Rust ter-unit-test (drive letter, backslash, `~`, UNC rejection, already-Unix passthrough) + integrasi `wslpath` di Windows CI (skip jika WSL absent). |

---

## 6. Design sketch (UI — pola yang ada, architect/lead finalize)

- Guide block (`index.html:571-578`) dapat: `#ans-install` (primary, di `.ans-guide-actions`),
  `#ans-install-log` (`<pre>` read-only, auto-scroll), `#ans-install-status` (status line).
- State machine guide = §2.2.1 (5 state). Guide rendering memakai field `os` dari `check_ansible`
  (UA sniffing di `ansRenderGuide()` dihapus — backend truth).
- Console pakai xterm? **Tidak** — `<pre>` + `textContent` sudah cukup (output installer = teks;
  progress bar brew/pipx render sebagai teks di PTY). Konsisten dengan ADR-8 (nol innerHTML).
- WSL-required block: perintah `<code>` + tombol copy (pola copy yang sudah ada), tiga baris
  fakta (admin / reboot / interupsi app).

---

## 7. ADR kecil (di-lock di spec, bukan open)

- **ADR-AI1 (installer choice):** pipx (Linux) / brew (macOS) / apt-in-WSL (Windows). Alasan:
  no-root, no-password, distro-independent, cocok dengan guide statis yang sudah ada;
  apt/dnf/pacman manual-only karena hosterm tak bisa eskalasi privilege dari Tauri webview dan
  console read-only tak bisa prompt password (secret di UI = langgar posture).
- **ADR-AI2 (WSL invocation):** backend-native `wsl.exe --exec ansible-playbook` + argv
  pre-translated; **BUKAN shim file** di Windows PATH. Alasan: shim tak bisa membedakan arg path
  vs flag dan butuh shell quoting (INV-14); backend sudah membangun argv, jadi translasi eksak +
  unit-testable. `HOSTERM_ANSIBLE_BIN` dipertahankan sebagai hook override/testability
  (meniru `HOSTERM_SFTP_BIN`).
- **ADR-AI3 (root di dalam WSL):** install in-WSL jalan sebagai root via `wsl --user root`.
  Alasan: console read-only → `sudo` tak bisa di-prompt; root-in-WSL adalah satu-satunya jalur
  password-free, dan root WSL tak bisa melampaui permission user Windows. Jika root terkunci,
  command gagal non-interaktif → fallback manual.
- **ADR-AI4 (no key copy):** hosterm TIDAK menyalin private key ke WSL. Alasan: copy = artefak
  secret baru, langgar posture zero-knowledge/no-persistent-secret; edge permission DrvFs
  disurfasse sebagai kegagalan dengan pesan jelas (OQ-AI1).

---

## 8. Test plan (verify loop — extend STATE.md baseline, jangan turunkan)

- **cargo (lib.rs):** `win_to_wsl_path` unit tests; argv-builder test launcher WSL (assert
  `--distribution/--exec` + path ter-translasi, no shell string); install-plan resolution tests
  (pipx/brew/wsl branches, `HOSTERM_ANSIBLE_BIN` override); `check_ansible` probe-dir test;
  byte-identity config/sidecar lintas install + WSL probe (NFR-AI5); `check_wsl` classification
  (installed+distro / installed+null / absent, fail-closed).
- **build:** 0 TS error.
- **smoke (headless-smoke.mjs):** guide menampilkan Install button yang sesuai platform; busy
  state + console streaming (mock); auto re-check on exit; Windows WSL state machine (absent /
  no-distro / installed+missing / installed+present); `HOSTERM_ANSIBLE_BIN` override; grep
  `sh -c|bash -c|cmd /C|powershell` = 0 (NFR-AI4); hostile bytes di console → 0 CSP violation,
  0 script element (ADR-8).
- **Windows CI:** `wslpath` integrasi test (skip jika WSL absent) — runner Windows sudah ada.

---

## 9. OPEN QUESTIONS (butuh nano/MEKL konfirmasi sebelum PRD dikunci penuh)

- **OQ-AI1 (DrvFs key permission):** terima kegagalan terdokumentasi (ADR-AI4), atau tambah
  `StrictModes no` untuk run WSL, atau copy key on-demand (0600 + cleanup) di dalam WSL?
  Rekomendasi: **v1 = kegagalan terdokumentasi**; putuskan v2 bersama MEKL.
- **OQ-AI2 (`wsl --user root`):** konfirmasi MEKL nyaman dengan root-in-WSL untuk install
  (password-free di image default). Alternatif jika tidak: `sudo -n` (hanya jalan di distro
  ber-passwordless-sudo) atau manual-only.
- **OQ-AI3 (pipx hilang di Linux):** terima fallback manual-only, atau bootstrap pipx venv
  terisolasi di app-data? Rekomendasi: **manual-only v1** — bootstrap rapuh di distro PEP 668
  yang tak punya `python3-venv`.
- **OQ-AI4 (distro non-apt):** one-click v1 apt-only + manual command untuk Fedora/Arch?
  Rekomendasi: **apt-only one-click**, sisanya manual.
- **OQ-AI5 (verifikasi macOS):** macOS CI sedang disabled (PRD §7 CI note) → path brew tak
  bisa CI-verified. Rekomendasi: **terima verifikasi manual di Mac MEKL** (konsisten dengan
  skip CI yang ada).

---

## 10. Out of scope (eksplisit — item §10 lain, tetap backlog)

1. Playbook dari Git repo private (surface OAuth/token).
2. Password-host Ansible run (mekanisme feed secret tanpa expose — architect spike).
3. Ad-hoc module run; vault editing dari UI.
4. Rolling/serial failure-budget.

Nol dependency baru; nol embed Python/Ansible; nol perubahan arsitektur PTY (install + WSL run
memakai `PtyState`/`pump_pty`/`pty://output|exit/{id}` yang sudah ada).
