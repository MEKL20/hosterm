# EPIC-TSD: AI backend v1 — playbook implementasi

| Field | Value |
|---|---|
| Author | crew-coding-lead (card `t_ede1d566`) |
| Sumber | `docs/SDD.md` §16 (addendum AI backend v1, baris 1874–2223; kontrak IPC §16.3 **BEKU** — implement verbatim) · `docs/FEATURE-SPEC-ai-backend-v1.md` §8 (test plan) + §9 (OQ residual) · `docs/STATE.md` verify loop |
| Status | Spec — belum ada kode AI. Unit **AI-1..AI-6** siap dispatch. |
| Gate | AI-1..AI-4 = kode lane → **triple gate** (lead review + QA PASS + SEC clean). AI-5 = smoke lane (lead review + QA verify). AI-6 = docs lane (nano re-verify). Developer tidak pernah push; lead collect & push. |
| Aturan kontrak | Nama payload/command/error = §16.3 verbatim. Perubahan = addendum SDD (ADR baru), bukan drift implementasi. Angka verify = "baseline saat dispatch + N" (re-measure; STATE.md kanonik). |
| Baseline terukur (14 Nov 2026, working tree) | IPC **45** (`grep -rc "#[tauri::command]" src-tauri/src/*.rs` → `lib.rs:43  fleet.rs:2`; registered `generate_handler!` = 45) · `cargo test --lib` **225** · smoke **500/500** (di-run ulang 14 Nov 2026) · `git diff src-tauri/Cargo.toml` **kosong** (`reqwest =0.12.9` + `keyring =3.6.3` sudah ada) |

---

## 0. Analyze-Requirements Pass (LQ-1..LQ-8 + temuan terukur)

> Reasoning LINTAS SDD §16 × FEATURE-SPEC §8/§9 × STATE.md × kondisi repo. Setiap LQ di §16.12
> diikat jadi keputusan final; tidak ada yang dropped.

| # | Finding | Tipe | Keputusan final |
|---|---|---|---|
| 1 | **LQ-1 module split** (`ai.rs` vs `lib.rs`) | Placement | **DECIDED: `src-tauri/src/ai.rs` BARU** (mirror `fleet.rs`, §12.2). Alasan: (a) §16.10 grep gate #7 mensyaratkan `src-tauri/src/ai.rs` sebagai jalur AI kanonik; (b) body (DTO + 8 konstanta + 6 validasi + store + keyring + HTTP wiring) > 150 LOC, jauh di atas ambang "kecil" §16.4; (c) `lib.rs` diff tetap minimal = `mod ai;` + 3 entri `generate_handler!` + `pub(crate)` visibility yang dipakai ai.rs. Fallback: tidak dipakai. Command names/payloads §16.3 identik either way. |
| 2 | **LQ-2 system prompt wording** | Copy | Kontrak perilaku beku (§16.6). Wording final (draft, AI-2 yang mengimplementasi): lihat §3 AI-2 "System prompt". Poin wajib: never execute, never claim terminal access, never ask/send secrets, `proposal` hanya command single-line + host alias + deskripsi, `proposal: null` bila tidak aman. |
| 3 | **LQ-3 reply truncation marker** | Copy | Cap beku `AI_REPLY_MAX_BYTES = 16384`. Marker final: `"\n\n[truncated: exceeded 16384 bytes]"` (truncate ke 16384 byte UTF-8, lalu append marker). |
| 4 | **LQ-4 `ai_chat` request JSON shape** | Serde detail | `#[serde(rename_all = "snake_case")]` pada req/resp; `AiChatReq { prompt: String, host: Option<String>, history: Vec<AiMessage>, timeout_secs: u32 }`; `AiMessage { role: String, content: String }` dengan `role` divalidasi `"user"|"assistant"`; `AiChatResp { reply: String, proposal: Option<AiProposal>, backend: String, warning: Option<String> }`. Nama field = §16.3 verbatim. |
| 5 | **LQ-5 keyring error mapping** | Error mapping | `keyring::Error::NoEntry` → `has_key = false` (bukan error). `PlatformUnavailable`/error lain saat key dibutuhkan → **fail-closed**: `write_ai_settings` key non-kosong ⇒ `Err("ai-settings:keyring-unavailable")` + 0 write; `ai_chat` remote + key dibutuhkan ⇒ `Err("ai:keyring-unavailable")`. Loopback v1 tidak butuh key ⇒ tidak membaca keyring di jalur `ai_chat` loopback. Nol fallback file 0600 (OQ-AI3). |
| 6 | **LQ-6 `has_key` probe cost** | Perf | **Tanpa cache.** `read_ai_settings` memprobe keyring setiap panggilan (`Entry::get_password()`; `NoEntry` → false). Probe murah; menghindari state basi saat key dihapus di luar app. |
| 7 | **LQ-7 `aiErrText()` copy table** | Copy | Tabel lihat §3 AI-3. Prefix set beku §16.8; raw token **tidak pernah** dirender — selalu lewat mapper + `textContent`/`esc()`. |
| 8 | **LQ-8 smoke loopback mock** | Harness | **Mock di batas invoke** (harness `qa-reports/headless-smoke.mjs` sudah memmock `window.__TAURI_INTERNALS__.invoke`): fixture §2.7 untuk `read_ai_settings`, fixture reply+proposal untuk `ai_chat`, reject token untuk kasus error. **Nol socket nyata** di smoke (deterministik, nol flake). Jalur HTTP reqwest yang nyata = matrix manual loopback (§5.6) + code review; bukan gate otomatis. |
| 9 | **Baseline IPC** | Ukur ulang | `grep -rc "#[tauri::command]" src-tauri/src/*.rs` → `lib.rs:43  fleet.rs:2` = **45** (14 Nov 2026). `generate_handler!` (lib.rs:6770) = 45 entri (43 `lib.rs` + `fleet::fleet_start` + `fleet::fleet_stop`). Gate == registered. AI-2 menambah 3 ⇒ **48**. |
| 10 | **Hotspot `lib.rs`** | Repo rule | `lib.rs` = satu kartu aktif pada satu waktu (standing rule). AI-1 dan AI-2 sama-sama menyentuh `lib.rs` (`mod ai;` / 3 `generate_handler!`) ⇒ **serial AI-1 → AI-2**. `ai.rs` juga dibagi AI-1/AI-2 ⇒ serial alami. |
| 11 | **Hotspot frontend** | Repo rule | `src/main.ts` + `index.html` = satu kartu aktif. AI-3 dan AI-4 sama-sama menyentuh keduanya ⇒ **serial AI-3 → AI-4**. `qa-reports/headless-smoke.mjs` dibagi AI-3/AI-4/AI-5 ⇒ AI-5 paling akhir. |
| 12 | **Stub `AI_PROPOSALS` + `sendAiStub`** | Inventori | `AI_PROPOSALS` (`src/main.ts:188`, 2 proposal hardcoded) **dihapus** di AI-4 (D-AI12/INV-38); `sendAiStub` (`src/main.ts:1587`) → `sendAiMessage`. Head-tag `.head-tag` (`index.html:139`, kini `"mock"`) diganti label jujur dari `AiChatResp.backend` (§4 spec). |
| 13 | **`on-device` baseline** | Ukur ulang | `grep -rn "on-device" src/` = **0** (14 Nov 2026). Gate AI-5: tetap 0. |
| 14 | **`sh -c\|bash -c\|cmd /C` baseline** | Ukur ulang | `grep -rnE "sh -c\|bash -c\|cmd /C" src-tauri/src/` = **7** (pre-existing, jalur non-AI). Gate AI-5: **0 di `src-tauri/src/ai.rs`** (jalur AI). |
| 15 | **Helper reuse** | Implementasi | Atomic write = `write_store_atomic` (`lib.rs:1048`, sibling temp → chmod 0600 → rename) — **jangan bikin helper kedua**. Path = `data_dir().join("ai-settings.json")` (`data_dir()` `lib.rs:881`, `HOSTERM_DATA_DIR` override). `sftp_safe` = `pub(crate)` (`lib.rs:1133`). Config snapshot = `parse_config` (`lib.rs:67`) / `fleet::snapshot_hosts` (`fleet.rs:244`). Keyring pattern = `backup.rs:856` (`keyring::Entry::new(service, account)`). |

---

## 1. Technical Context (locked stack — nol dependency baru)

- **Backend:** Rust, `src-tauri/src/ai.rs` (BARU). HTTP = `reqwest 0.12.9` blocking + rustls (sudah ada; dipakai OAuth/Drive). Secret = `keyring 3.6.3` (sudah ada; service `hosterm`, account `ai-api-key`). JSON = `serde`/`serde_json` (sudah ada). Path = `dirs` (sudah ada). **Nol crate baru** — `git diff src-tauri/Cargo.toml` = kosong (gate).
- **Frontend:** vanilla TS di `src/main.ts`; markup statis di `index.html` (section `#settings-ai` baru / perluas `#settings-approvals`); CSS `src/styles.css` (Dusk tokens only). **Nol JS dep baru.** Semua render remote-origin lewat `esc()`/`escAttr()` (ADR-8).
- **Store:** `dirs::config_dir()/hosterm/ai-settings.json` (atau `HOSTERM_DATA_DIR` override), pola `settings.json` §13.4. Nol DB, nol migration.
- **IPC beku:** inventori **45 → 48** (`read_ai_settings`, `write_ai_settings`, `ai_chat`); request/response only, no events; args camelCase di boundary, payload snake_case; rejection strings persis §16.8.
- **Execution:** `ai_chat` **proposal-only, nol execute** (INV-36). Eksekusi hanya lewat state machine §13.6 yang existing (Run once / exact `(host,command)` allowlist).

---

## 2. Physical Schema (no DB — lead owns physical realization)

Logical model §16.2 diwujudkan konkret:

| Entity | Location | Mode | Lifetime | Writer | Corrupt/missing |
|---|---|---|---|---|---|
| `ai-settings.json` | `data_dir()/ai-settings.json` (= `dirs::config_dir()/hosterm/`; `HOSTERM_DATA_DIR` override) | 0600-class | persistent | `write_ai_settings` (atomic temp → rename, `write_store_atomic`) | → `DEFAULT_AI_SETTINGS`; byte corrupt **tidak** disentuh sampai write sah (INV-24) |
| AI API key | OS keyring, service `hosterm`, account `ai-api-key` | secret | persistent | `write_ai_settings` (`key: non-empty`); `ai_chat` (read-only) | `NoEntry` → `has_key=false`; platform unavailable → fail-closed |

**JSON schema `AiSettings` (verbatim §16.2):**

```json
{ "version": 1, "endpoint": "", "model": "", "remote_allowed": false, "has_key": false }
```

- `endpoint` / `model` **nol default** — kosong = not configured (MEKL 13 Nov 2026).
- `remote_allowed` = defense-in-depth only; UI toggle dihapus (D-AI5).
- `has_key` = boolean only; key byte **tidak pernah** masuk file ini, IPC, log, argv, atau error string (INV-37).
- **Stateless:** nol prompt/response/history persistence (OQ-AI5); nol sync (NFR-8).
- **Isolation:** nol baca/tulis `~/.ssh/config` atau sidecar `~/.ssh/hosterm.secrets` (INV-22 class).

**Physical constants (single source of truth §16.4):** `AI_PROMPT_MAX_BYTES=8192` · `AI_HISTORY_MAX_TURNS=20` · `AI_HISTORY_MAX_BYTES=16384` · `AI_REPLY_MAX_BYTES=16384` · `AI_COMMAND_MAX_BYTES=2048` · `AI_DESC_MAX_BYTES=1024` · `AI_ENDPOINT_MAX_BYTES=2048` · `AI_MODEL_MAX_BYTES=256`. `timeout_secs`: default 30, range 5–120 (reject, never clamp).

---

## 3. Unit Breakdown (final — granularity = keputusan lead)

Pemetaan task body → 6 unit. **AI-1..AI-2** backend (serial `ai.rs`/`lib.rs`); **AI-3..AI-4** frontend (serial `main.ts`/`index.html`); **AI-5** smoke; **AI-6** docs.

### AI-1 — Backend pure cores (`ai.rs` baru; nol network)

- **Tujuan:** semua validasi + truncasi sebagai pure fn yang `#[cfg(test)]`-covered, nol network/spawn/FS.
- **Cakupan file (persis):** `src-tauri/src/ai.rs` (BARU: 8 konstanta, DTO `AiSettings`/`WriteAiSettingsReq`/`AiChatReq`/`AiMessage`/`AiChatResp`/`AiProposal`, pure cores, `#[cfg(test)]`); `src-tauri/src/lib.rs` = **hanya** `mod ai;`.
- **Sumber:** SDD §16.4 (aturan beku #1–6) + §16.2 (konstanta) + §16.3 (DTO).
- **AC (EARS):** WHEN prompt kosong/whitespace THE SYSTEM SHALL menolak `ai:empty-prompt`; WHEN prompt > 8192 byte THE SYSTEM SHALL menolak `ai:prompt-too-large`; WHEN history > 20 turn ATAU total > 16384 byte THE SYSTEM SHALL menolak `ai:history-too-large`; WHEN `timeout_secs` di luar 5..120 THE SYSTEM SHALL menolak `ai:bad-timeout` (never clamp); WHEN URL parse gagal / scheme ∉ {http,https} / userinfo ada / > 2048 byte THE SYSTEM SHALL menolak `ai:bad-endpoint`; WHEN non-loopback + `remote_allowed=false` THE SYSTEM SHALL menolak `ai:remote-endpoint-disabled`; WHEN non-loopback + `remote_allowed=true` THE SYSTEM SHALL menolak selain `https`; WHEN proposal host gagal `sftp_safe` ATAU tidak ada di config snapshot THE SYSTEM SHALL `warning` + `proposal:null`; WHEN command > 2048 ATAU description > 1024 THE SYSTEM SHALL `warning` + `proposal:null`; WHEN reply > 16384 byte THE SYSTEM SHALL truncate + marker.
- **Verify (terukur):** `cargo test --lib` → **≥ 239** (225 + ≥14 baru); `grep -c '#\[tauri::command\]' src-tauri/src/ai.rs` = **0**; `grep -nE 'sh -c|bash -c|cmd /C' src-tauri/src/ai.rs` = **0**; `grep -c '#\[test\]' src-tauri/src/ai.rs` ≥ **14**; `git diff src-tauri/Cargo.toml` kosong.
- **Demo:** `cargo test --lib` hijau ≥239; test name `ai_prompt_rejects_empty_and_oversize`, `ai_endpoint_rejects_userinfo_and_remote`, `ai_proposal_requires_known_safe_host`, `ai_reply_truncates_with_marker` ada di output.
- **Deps:** — (Wave 1). **Paralel-aman dengan AI-3** (file-disjoint). Owner: `crew-coding-backend`. Gate: triple code lane.

### AI-2 — 3 command IPC + `ai-settings.json` + keyring

- **Tujuan:** `read_ai_settings`, `write_ai_settings`, `ai_chat` + store atomic + keyring integration; inventory 45 → 48.
- **Cakupan file (persis):** `src-tauri/src/ai.rs` (commands, store, keyring, HTTP wiring, tests); `src-tauri/src/lib.rs` = **3 entri `generate_handler!`** (`read_ai_settings`, `write_ai_settings`, `ai_chat`) + `mod ai;` (dari AI-1).
- **Sumber:** SDD §16.3 (kontrak beku) + §16.5 (flow 9 langkah) + §16.7 (security) + §16.8 (error strings) + §2.2 spec.
- **AC (EARS):** WHEN `read_ai_settings` dipanggil THE SYSTEM SHALL mengembalikan `AiSettings` dan **tidak pernah throw** (missing/corrupt → defaults; `has_key` dari keyring); WHEN `write_ai_settings` dipanggil THE SYSTEM SHALL menulis atomic (temp → rename 0600) dan mengaplikasikan key semantics (`undefined/null` = biarkan; `""` = hapus; non-empty = simpan keyring) dengan reject beku `ai-settings:invalid-endpoint|remote-disabled|invalid-model|keyring-unavailable|write-failed`; WHEN `ai_chat` dipanggil THE SYSTEM SHALL menjalankan 9 langkah §16.5 (load → validate → endpoint → build → header → send → parse → validate proposal → return) dengan `Authorization: Bearer` **hanya** bila `has_key` + remote, redirect policy **none**, timeout `timeout_secs`, dan **nol execute**; WHEN endpoint/model kosong THE SYSTEM SHALL menolak `ai:not-configured` (0 network); WHEN keyring unavailable + key dibutuhkan THE SYSTEM SHALL fail-closed `ai:keyring-unavailable`/`ai-settings:keyring-unavailable`; semua error string persis §16.8 (12 Err + 4 warning + 5 write).
- **System prompt (LQ-2, implement verbatim):**
  ```
  You are hosterm's command assistant. The user describes an operational task.
  Reply with ONLY one JSON object, no prose outside it:
  { "reply": "<short user-facing explanation>",
    "proposal": { "host": "<exact host alias from the user's wording>",
                  "command": "<single-line shell command>",
                  "description": "<one sentence on the impact>" } }
  Rules: never execute anything; you have no terminal or host access; never ask for
  or include secrets/keys/credentials; the command must be a single line; if you
  cannot propose a safe command, set "proposal": null.
  ```
- **Verify (terukur):** `cargo test --lib` → **≥ 251** (239 + ≥12 baru); `grep -rc '#\[tauri::command\]' src-tauri/src/*.rs` = **48**; `grep -n 'read_ai_settings\|write_ai_settings\|ai_chat' src-tauri/src/lib.rs` ≥ **3**; `grep -c '#\[test\]' src-tauri/src/ai.rs` ≥ **26**; `grep -n 'ai-api-key' src-tauri/src/ai.rs` ≥ **1**; `git diff src-tauri/Cargo.toml` kosong; key-custody grep (`ai-api-key` not in `settings.json`/`~/.ssh/config`) bersih.
- **Demo:** `cargo test --lib` hijau ≥251; test name `ai_settings_roundtrip_atomic_corrupt_defaults`, `ai_chat_rejects_not_configured_zero_network`, `ai_chat_proposal_unsafe_host_warns`, `ai_keyring_fail_closed` ada di output.
- **Deps:** **AI-1** (serial `ai.rs`/`lib.rs` + cores dipakai commands). **Paralel-aman dengan AI-4**. Owner: `crew-coding-backend`. Gate: triple code lane.

### AI-3 — Settings → AI Assistant (frontend)

- **Tujuan:** permukaan Settings > AI Assistant (§3 spec) + copy mapper `aiErrText()`.
- **Cakupan file (persis):** `index.html` (section `#settings-ai` baru di atas `#settings-approvals`, atau perluas section yang ada — nav `data-section="ai"` sudah ada di `index.html:221`); `src/main.ts` (wiring + `aiErrText()` + `ai-backend-label` preview); `src/styles.css`.
- **Sumber:** FEATURE-SPEC §3 (field, empty state, penyimpanan key) + §4 (label) + §2.2 (strings) + SDD §16.3/§16.8.
- **AC (EARS):** WHEN Settings > AI Assistant dibuka THE SYSTEM SHALL menampilkan field `#ai-endpoint` (text, nol default, placeholder `http://127.0.0.1:11434/v1`), `#ai-model` (text, nol default, placeholder `llama3.2`), `#ai-key` (password, hint "Stored in the OS keyring. Leave blank to keep the saved key."), indikator `#ai-key-state` = `Saved`/`Not saved`/`Keyring unavailable`, tombol `#ai-test` + `#ai-status`, dan `#ai-backend-label`; WHEN field kosong THE SYSTEM SHALL menampilkan status *"Not configured — AI requests are disabled until a model is set."* + test disabled; WHEN "Send test request" diklik THE SYSTEM SHALL memanggil `ai_chat` dengan prompt tetap (bukan user prompt) dan **tidak** menyimpan apa pun; THE SYSTEM SHALL **tidak** menampilkan toggle "Allow remote endpoints" (D-AI5); semua error string dilewatkan `aiErrText()` (raw token tidak pernah dirender) + `textContent`/`esc()`.
- **`aiErrText()` copy table (LQ-7, prefix beku §16.8):**
  ```
  ai:not-configured          → "AI is not set up. Add an endpoint and model in Settings → AI Assistant."
  ai:empty-prompt           → "Type a message first."
  ai:prompt-too-large        → "That message is too long (max 8192 bytes)."
  ai:history-too-large       → "The conversation is too long (max 20 turns / 16384 bytes)."
  ai:bad-timeout             → "Timeout must be between 5 and 120 seconds."
  ai:bad-endpoint            → "That endpoint URL is not valid (http/https only, no credentials)."
  ai:remote-endpoint-disabled→ "Remote endpoints are disabled in v1; use a loopback endpoint."
  ai:endpoint-unreachable    → "Could not reach the endpoint."
  ai:timeout                 → "The endpoint took too long to respond."
  ai:unauthorized            → "The endpoint rejected the API key."
  ai:model-error             → "The endpoint returned an error or an unreadable response."
  ai:keyring-unavailable     → "The OS keyring is unavailable; the API key could not be read."
  ai-settings:invalid-endpoint / remote-disabled / invalid-model / keyring-unavailable / write-failed → copy analog.
  default                    → code (escaped)
  ```
- **Verify (terukur):** `npm run build` → **0 TS**; smoke **≥ 504** (500 + ≥4 baru: §8.1 #1, #2); `grep -c 'ai-endpoint\|ai-model\|ai-key' index.html` ≥ **3**; `grep -n 'aiErrText' src/main.ts` ≥ **1**; `grep -in 'allow remote' index.html src/` = **0**.
- **Demo:** smoke: Settings > AI Assistant render field kosong + status not configured + toggle tidak ada; `ai_chat` invoke tercatat dengan prompt tetap.
- **Deps:** **AI-2** (kontrak tersedia; smoke mem-mock invoke). **Paralel-aman dengan AI-1**. Owner: `crew-coding-frontend`. Gate: triple code lane.

### AI-4 — AI panel: `sendAiMessage` + render + head-tag jujur

- **Tujuan:** ganti stub dengan backend nyata; render reply + proposal card; label jujur.
- **Cakupan file (persis):** `src/main.ts` (`sendAiStub` → `sendAiMessage`, hapus `AI_PROPOSALS`, render thread dari state nyata, head-tag dari `AiChatResp.backend`, in-flight disable); `index.html` (head-tag `.head-tag` di `index.html:139`); `src/styles.css`.
- **Sumber:** FEATURE-SPEC §2.6 + §4 + §3.3 + §5.1 + SDD §16.3 (label honesty) + §16.7 (FR-AI11).
- **AC (EARS):** WHEN user mengirim pesan THE SYSTEM SHALL trim, abort bila kosong, append user message, panggil `ai_chat`, render reply (escaped) + proposal card (client-generated `id`) via template existing; WHEN `ai:not-configured` THE SYSTEM SHALL menampilkan empty state + tombol "Set up AI" (input disabled); WHEN error lain THE SYSTEM SHALL menampilkan pesan escaped via `aiErrText()` + **nol execute**; WHEN response tiba THE SYSTEM SHALL set head-tag dari `AiChatResp.backend` (`not configured`/`local endpoint`/`remote endpoint`) — **`on-device` dilarang**, `mock` dihapus; WHEN request in-flight THE SYSTEM SHALL disable input + tombol send; THE SYSTEM SHALL menampilkan proposal card hanya dari response nyata (nol hardcoded, INV-38); semua render remote-origin lewat `textContent`/`esc()`/`escAttr()` (nol `innerHTML` dengan data model).
- **Verify (terukur):** `npm run build` → **0 TS**; smoke **≥ 513** (504 + ≥9 baru: §8.1 #3–#7, #9–#11); `grep -n 'sendAiStub\|AI_PROPOSALS' src/main.ts` = **0**; `grep -n 'sendAiMessage' src/main.ts` ≥ **1**; `grep -rn 'on-device' src/` = **0**.
- **Demo:** smoke: kirim pesan → reply escaped + proposal card render; fixture hostile → 0 child node, `__pwn` undefined, CSP 0; head-tag = `local endpoint` saat endpoint loopback.
- **Deps:** **AI-3** (serial `main.ts`/`index.html`) + **AI-2** (kontrak `ai_chat`). **Paralel-aman dengan AI-2**. Owner: `crew-coding-frontend`. Gate: triple code lane.

### AI-5 — Smoke assertion baru (§8.1) + grep gates (§8.3)

- **Tujuan:** kanonisasi assertion AI + grep gates; harness loopback mock (LQ-8).
- **Cakupan file (persis):** `qa-reports/headless-smoke.mjs` (mock state AI + assertion §8.1 sisa + grep gates §8.3 + ledger AI).
- **Sumber:** FEATURE-SPEC §8.1 (11 item) + §8.3 (grep gates) + §8.4 (verify loop).
- **AC (EARS):** WHEN suite dijalankan THE SYSTEM SHALL memuat assertion §8.1 #8 (key custody: `has_key=true`, key tidak di `ai-settings.json`/log) + loopback mock reply+proposal + semua grep gate §8.3; THE SYSTEM SHALL melaporkan total ≥ **516** (500 + ≥16 AI) dengan CSP counter **0**; grep gate: `on-device` di `src/` = 0; `sh -c|bash -c|cmd /C` di `src-tauri/src/ai.rs` = 0; `ai-api-key` tidak di `settings.json`/`~/.ssh/config`; nol log key/prompt.
- **Verify (terukur):** `node qa-reports/headless-smoke.mjs` → **≥ 516/≥516**, CSP 0; keempat grep gate di atas dieksekusi sebagai command, output ditempel di task log.
- **Demo:** smoke hijau ≥516; `grep -rn "on-device" src/ && echo FAIL || echo GATE-OK` → `GATE-OK`.
- **Deps:** **AI-3 + AI-4** (UI untuk assertion) + **AI-2** (jalur `ai.rs` untuk grep). Owner: `crew-coding-frontend` (smoke harness owner). Gate: lead review + QA verify.

### AI-6 — Verify loop: §11.5 self-check + STATE.md

- **Tujuan:** angka terukur mendarat di docs; verify loop kanonik.
- **Cakupan file (persis):** `docs/STATE.md` (verify loop: IPC 45→48, cargo 225→≥251, smoke 500/500→≥516 + assertion AI); `docs/SDD.md` **§11.5 only** (self-check line 5: command list §3 matches inventory 48) — **§16 tidak disentuh**.
- **Sumber:** SDD §11.5 + STATE.md verify loop + §16.11.
- **AC:** STATE.md mencantumkan IPC **48** (`lib.rs` 43+3 AI, `fleet.rs` 2), `cargo test --lib` **≥251**, smoke **≥516** + AI checks, semua **terukur** (bukan proyeksi); §11.5 self-check diperbarui ke inventori 48.
- **Verify (terukur):** `grep -n '48' docs/STATE.md` ≥1; `grep -n '251' docs/STATE.md` ≥1; `grep -n '516' docs/STATE.md` ≥1; `grep -n '48' docs/SDD.md` (§11.5) ≥1.
- **Deps:** **AI-5**. Owner: `crew-coding-lead` (docs lane). Gate: nano re-verify.

---

## 4. Hotspot + Urutan Rekomendasi + Dependency Graph

**Hotspot (satu kartu aktif per berkas pada satu waktu):**

| Berkas | Kartu yang menyentuh | Urutan |
|---|---|---|
| `src-tauri/src/lib.rs` | AI-1, AI-2 | **serial** AI-1 → AI-2 |
| `src-tauri/src/ai.rs` (BARU) | AI-1, AI-2 | **serial** AI-1 → AI-2 |
| `src/main.ts` + `index.html` | AI-3, AI-4 | **serial** AI-3 → AI-4 |
| `qa-reports/headless-smoke.mjs` | AI-3, AI-4, AI-5 | **serial**; AI-5 paling akhir |
| `docs/SDD.md` | kartu architect (sedang jalan) + AI-6 (§11.5 only) | AI-6 koordinasi; §16 beku |

**Dependency graph:**

```
AI-1 (ai.rs/lib.rs) ──> AI-2 (ai.rs/lib.rs) ──┐
                                              ├──> AI-5 (smoke) ──> AI-6 (docs)
AI-3 (main.ts/index.html) ──> AI-4 (main.ts/index.html) ──┘
```

**Paralel-aman:** AI-1 ∥ AI-3 (file-disjoint: `ai.rs`/`lib.rs` vs `main.ts`/`index.html`/`styles.css`); AI-2 ∥ AI-4 (file-disjoint). Frontend unit mem-mock kontrak beku §16.3 di smoke — tidak hard-blocked pada backend, tapi **integrasi nyata** (app terpasang) hanya lengkap setelah AI-2 + AI-3 + AI-4 mendarat.

**Waves rekomendasi:**

| Wave | Unit | Mulai | Paralel dengan |
|---|---|---|---|
| 1 | AI-1 (backend cores) **atau** AI-3 (frontend settings) | sekarang | saling paralel (file-disjoint) |
| 2 | AI-2 (after AI-1) **atau** AI-4 (after AI-3) | gate wave 1 | saling paralel |
| 3 | AI-5 (smoke + grep) | after AI-2 + AI-4 | — |
| 4 | AI-6 (docs) | after AI-5 | — |

**Caveat main-hijau:** bila lead memilih `main` hijau runtime di setiap commit (pola ansible FE-ANS), serialkan **AI-2 → AI-3** (frontend menunggu command nyata) dengan biaya satu handoff; rekomendasi default = paralel di atas dengan integrasi diverifikasi di AI-5/AI-6.

---

## 5. Risiko + Mitigasi

| # | Risiko | Mitigasi |
|---|---|---|
| 1 | **reqwest blocking di main thread** membekukan UI | `ai_chat` = `async fn` + `tauri::async_runtime::spawn_blocking` (atau std thread) untuk seluruh HTTP; timeout mencakup connect+read; UI disable selama in-flight (AI-4). |
| 2 | **keyring gagal di platform tanpa backend** (Linux headless, CI) | Fail-closed (LQ-5): `write_ai_settings` key non-kosong ⇒ reject `ai-settings:keyring-unavailable` + 0 write; `ai_chat` remote + key dibutuhkan ⇒ `ai:keyring-unavailable`. Cargo test keyring di-skip bila keyring tidak tersedia (pola `backup.rs`). Nol fallback file 0600 (OQ-AI3). |
| 3 | **SSRF / credential leak via redirect** | `reqwest::Client::builder().redirect(reqwest::redirect::Policy::none())`; userinfo dilarang (`ai:bad-endpoint`); loopback-only v1 (D-AI5). |
| 4 | **Eksekusi tak disengaja** | `ai_chat` nol spawn/execute (INV-36); grep `sh -c\|bash -c\|cmd /C` = 0 di `ai.rs`; eksekusi hanya state machine §13.6. |
| 5 | **Key/prompt bocor** | Keyring only (`hosterm`/`ai-api-key`); `has_key` boolean; nol key di `ai-settings.json`/IPC/log/argv/error (INV-37); nol log prompt/response/key (§16.7). |
| 6 | **Jalur HTTP reqwest tak teruji otomatis** | Core murni teruji cargo (AI-1/AI-2); smoke mem-mock invoke (AI-3..AI-5); **matrix manual loopback** sebelum rilis (endpoint Ollama-compatible + model kecil) + code review AI-2. Dicatat sebagai gap sadar, bukan gate palsu. |
| 7 | **Hostile output model** | Semua render lewat `esc()`/`escAttr()`/`textContent`; nol `innerHTML` dengan data model (FR-AI11/ADR-8); CSP 0 (smoke). |
| 8 | **Label bohong** | Head-tag diturunkan dari `AiChatResp.backend`; `on-device` dilarang (grep 0); `mock` dihapus (AI-4). |
| 9 | **Kolisi hotspot** | Serial AI-1→AI-2, AI-3→AI-4, AI-3→AI-4→AI-5; `docs/SDD.md` §11.5 koordinasi dengan kartu architect. |
| 10 | **Drift baseline** | Re-measure tiap dispatch (NFR-5); angka di TSD = baseline + N, bukan angka spec. |
| 11 | **Stub tersisa** | `AI_PROPOSALS` + `sendAiStub` dihapus (AI-4); empty state nol fake card (INV-38). |

---

## 6. Testing Strategy

1. **Cargo (`cargo test --lib`):** AI-1 ≥14 test baru (pure cores §16.4 #1–6, konstanta, caps); AI-2 ≥12 test baru (store round-trip + corrupt no-clobber + atomic, keyring save/delete/has_key skip-if-unavailable, error-string table §16.8 persis, endpoint/proposal validation, `ai:not-configured` 0 network, fail-closed keyring). Semua nol network nyata & nol ssh asli — `HOSTERM_DATA_DIR` fixture (jangan sentuh home nyata, hard rule #6). Target: 225 → **≥251**.
2. **Build:** `npm run build` → 0 TS tiap unit frontend (AI-3, AI-4).
3. **Smoke (`node qa-reports/headless-smoke.mjs`):** mock invoke map + fixture §2.7; AI-3 +≥4, AI-4 +≥9, AI-5 +≥3 + kanonisasi §8.1/§8.3; target **≥516**, CSP 0.
4. **Grep gates (AI-5):** `on-device` src/ = 0 · `sh -c|bash -c|cmd /C` ai.rs = 0 · `ai-api-key` not in `settings.json`/`~/.ssh/config` · nol log key/prompt · `git diff src-tauri/Cargo.toml` kosong.
5. **Manual matrix (release gate, bukan CI):** loopback endpoint nyata (Ollama-compatible) — reply + proposal + Run once lewat terminal real-ssh; keyring ada/tidak; hostile output; semua label jujur.

---

## 7. Rollout & Rollback

- **Urutan merge:** AI-1 → AI-2 → [AI-3 → AI-4] → AI-5 → AI-6. Tiap unit = 1 commit feat + docs STATE (konvensi per-unit revert).
- **Tanpa feature flag:** AI-1/AI-2 inert (UI belum ada); AI-3/AI-4 membuat UI reachable; integrasi penuh AI-2+AI-3+AI-4.
- **Rollback:** revert commit unit; nol state persisten perlu dimigrasi (ai-settings.json bisa dibiarkan; keyring entry terpisah). Revert AI-4 meninggalkan AI-3 valid; revert AI-2 sebelum AI-3 mendarat = aman.
- **Rilis:** keputusan tag = orchestrator (nano), BUKAN anak kartu.

---

## 8. Kriteria "Selesai" per Unit (nano-verifiable, tanpa baca kode)

| Unit | Command | Ekspektasi |
|---|---|---|
| AI-1 | `cargo test --lib` | exit 0, **≥239** pass |
| AI-1 | `grep -c '#\[tauri::command\]' src-tauri/src/ai.rs` | **0** |
| AI-1 | `grep -nE 'sh -c\|bash -c\|cmd /C' src-tauri/src/ai.rs` | **0** |
| AI-2 | `grep -rc '#\[tauri::command\]' src-tauri/src/*.rs` | **48** |
| AI-2 | `git diff src-tauri/Cargo.toml` | kosong |
| AI-3 | `npm run build` | exit 0, 0 TS |
| AI-3 | `node qa-reports/headless-smoke.mjs` | **≥504/≥504**, CSP 0 |
| AI-3 | `grep -c 'ai-endpoint\|ai-model\|ai-key' index.html` | **≥3** |
| AI-4 | `grep -n 'sendAiStub\|AI_PROPOSALS' src/main.ts` | **0** |
| AI-4 | `node qa-reports/headless-smoke.mjs` | **≥513/≥513**, CSP 0 |
| AI-5 | `node qa-reports/headless-smoke.mjs` | **≥516/≥516**, CSP 0 |
| AI-5 | `grep -rn 'on-device' src/` | **0** |
| AI-5 | `grep -rnE 'sh -c\|bash -c\|cmd /C' src-tauri/src/ai.rs` | **0** |
| AI-6 | `grep -n '48\|251\|516' docs/STATE.md` | ≥1 masing-masing |

---

*Sources: `docs/SDD.md` §16 (read 2026-11-14, working-tree version; §16.3 beku) · `docs/FEATURE-SPEC-ai-backend-v1.md` §8/§9 · `docs/STATE.md` verify loop · `src-tauri/src/lib.rs` (`generate_handler!`:6770, `data_dir`:881, `write_store_atomic`:1048, `sftp_safe`:1133, `parse_config`:67) · `src-tauri/src/fleet.rs` (`snapshot_hosts`:244) · `src-tauri/src/backup.rs` (keyring pattern:856) · `src/main.ts` (`AI_PROPOSALS`:188, `renderAiThread`:1443, `executeAiCommand`:1517, `sendAiStub`:1587, `ansErrText`:2306) · `index.html` (`.head-tag`:139, nav `data-section="ai"`:221, `#settings-approvals`:295) · `src-tauri/Cargo.toml` (`reqwest =0.12.9`, `keyring =3.6.3`) · live grep IPC 45 + smoke 500/500 (14 Nov 2026).*
