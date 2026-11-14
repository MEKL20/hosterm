# FEATURE-SPEC: AI backend (v1) — resolusi OQ-4

> Milik epic produk — sitasi `PRD.md §7` (US-11 / OQ-4) + `docs/FEATURE-SPEC-redesign-dusk-v1.md` §4.5.
> EARS untuk semua behavior. Initiative tunggal → dokumen ini FEATURE-SPEC, bukan PRD.
> Boss intent (MEKL via Tier-0 nano, card `t_f47c14fa`; ditutup MEKL 13 Nov 2026, card `t_5009863d`):
> lanjutkan fitur AI. **OQ-4 RESOLVED** — lihat §1. Doc only — **NOL commit/push** (PRD §"Version constraint").

| Field | Value |
|---|---|
| Author | crew-coding-pm |
| Status | **Speced — OQ-4 LOCKED oleh MEKL 13 Nov 2026.** Arah C (BYO endpoint), **nol default endpoint/model**, v1 **loopback-only**, **nol amendemen NFR-3** untuk jalur AI. No code, no commits (PM = doc only). |
| Baseline | main `6c8c7cd` (v0.16.0). Live verify loop at HEAD (re-measured 13 Nov 2026): cargo **225/225**, build **0 TS**, smoke **500/500**, IPC **45** — **re-measure at dispatch** (NFR-5). STATE.md now records the same (corrected 13 Nov 2026, card `t_ec21322b`): newest note v0.16.0, IPC line **45** (`lib.rs` 43 + `fleet.rs` 2). The canonical gate is the live grep — re-measure, don't trust verbatim. |
| Source | `docs/FEATURE-SPEC-redesign-dusk-v1.md` §4.5 + OQ-4 · `docs/SDD.md` §13.4/§13.5/§13.6 (kontrak approval FROZEN) · `docs/PRD.md` NFR-3 + US-11 · `docs/EPIC-TSD-redesign-dusk-v1.md` §U-D7 + §"Settings nav" · ADR-21/22 · FR-AI1..13 · INV-25/26 · `docs/STATE.md` (verify loop + invariants) |
| Non-negotiables | Kontrak approval §13.6 **FROZEN** (AI hanya mengusulkan; eksekusi = terminal real-ssh existing, ADR-1/2) · FR-AI9 nol auto-run implisit · allowlist exact `(host,command)` (ADR-21) · key **tidak** masuk `~/.ssh/config`, **tidak** masuk log, **tidak** masuk argv · NFR-3 default = nol jaringan eksternal · PM = dokumen saja |
| Traceability | OQ-4 (FEATURE-SPEC-redesign §6) · FR-AI1..13 · INV-25/26 · ADR-21/22 · NFR-3 · US-11 · U-D7 · STATE.md verify loop |
| Feature name (LOCKED) | **AI backend v1 — BYO OpenAI-compatible endpoint, nol default (loopback-only)** |

---

## 0. Scope lock (dari nano brief — bukan bahan debat)

**v1 = backend AI nyata lewat endpoint OpenAI-compatible yang dikonfigurasi user:**

- **Nol default endpoint maupun model** — kedua field mulai **kosong**; hosterm tidak punya sponsor AI / model sendiri, jadi tidak ada nilai bawaan yang bisa dipertanggungjawabkan.
- **v1 = loopback-only**: endpoint non-loopback ditolak (`ai-settings:remote-disabled` / `ai:remote-endpoint-disabled`); toggle "Allow remote endpoints" **dihapus** dari UI. Contoh loopback `http://127.0.0.1:11434/v1` (Ollama-compatible) → **nol jaringan eksternal**.
- **Nol amendemen NFR-3** untuk jalur AI (§7).
- **Settings > AI Assistant** (nav sudah ada) = tempat endpoint/model/key diatur.
- **IPC baru**: `read_ai_settings`, `write_ai_settings`, `ai_chat` (inventory 45 → 48).
- **Kontrak approval §13.6 FROZEN** — AI hanya mengusulkan command; eksekusi tetap lewat terminal real-ssh existing.

**Excluded (named, JANGAN desain):** on-device model bundling (GGUF + sidecar); pattern/glob allowlist (OQ-6); sync lintas-device; streaming; auto-exec; manajemen model (download/quantize); injeksi konteks host otomatis ke prompt.

**Fakta live yang sudah terverifikasi (kutip, jangan re-litigasi):**

1. `AI_PROPOSALS` di `src/main.ts:188` = **2 proposal hardcoded**; `sendAiStub` (`src/main.ts:1587`) hanya mendorong pesan mock *"The AI backend is not connected…"*. Tidak ada model yang menjawab.
2. Head-tag `.head-tag` di `index.html:138` saat ini berisi **"mock"** (title: *"UI-only mock; no AI backend is connected"*) — jujur untuk stub, tapi **tidak boleh** dipakai saat backend nyata tersambung.
3. State machine approval + `executeAiCommand` + `sftpSafe` + `persistAllowlist` sudah ada dan **tetap dipakai verbatim** (§13.6). Yang berubah hanya **asal proposal** (dari hardcoded → model) dan **head-tag**.
4. `keyring` crate **sudah ada** di dependency tree (`=3.6.3`, fitur `sync-secret-service`/`crypto-rust`/`windows-native`) dan sudah dipakai untuk OAuth refresh token + backup device key. Entri key AI = entri terpisah (service `hosterm`, account `ai-api-key`).
5. `reqwest` blocking + rustls **sudah ada** (dipakai OAuth/Drive). Nol dependency baru untuk HTTP.
6. NFR-3 **sudah usang** di PRD: OAuth token exchange + Drive API (shipped v0.14.x) sudah menambah jalur network user-initiated yang **tidak terdaftar** di NFR-3. Normalisasi itu = utang dokumen pra-OQ-4 → **kartu terpisah** (§7); jalur AI v1 loopback-only → nol amendemen.
7. **IPC inventory live = 45** (`grep -rc "tauri::command" src-tauri/src/*.rs` → `lib.rs` 43 + `fleet.rs` 2). STATE.md's IPC line was stale (42) until corrected 13 Nov 2026 (card `t_ec21322b`); the canonical gate is the live grep. AI menambah 3 command → **48** (§2.7).

---

## 1. Resolusi OQ-4 (KEPUTUSAN MEKL — 13 Nov 2026)

### 1.1 Tiga arah, trade-off NYATA

| Arah | Kualitas model | Privacy / jaringan | Installer & kompleksitas | Secret | Label jujur | NFR-3 |
|---|---|---|---|---|---|---|
| **A. On-device** (GGUF di-bundle + sidecar llama.cpp) | Lemah–sedang (model kecil agar installer muat; halusinasi command lebih sering) | **Aman**: nol network | **Buruk**: installer membengkak (model Q4 ≈ 0.5–4 GB per platform), sidecar binary per-OS, lifecycle proses, manajemen model/version | Nol key | `on-device` (akurat) | **Tidak perlu amendemen** |
| **B. Remote API** (key user, endpoint vendor) | **Terbaik** | **Buruk**: prompt + konteks host leave device; butuh key; vendor = pihak ketiga | Sedang: keyring + HTTP client (sudah ada) | API key (keyring) | `remote endpoint` | **Melanggar NFR-3** → butuh amendemen |
| **C. BYO endpoint** ✅ **DIPILIH MEKL** (nol default, v1 loopback-only) | **Tergantung setup user** (lokal bisa kuat: Ollama/llama.cpp/LM Studio/vLLM) | **Baik**: loopback = nol jaringan eksternal; remote ditolak di v1 | **Ringan**: nol bundle, nol sidecar; hosterm hanya HTTP client | API key opsional (keyring) | `not configured` / `local endpoint` | **Tidak perlu** amendemen (v1 loopback-only) |

### 1.2 Keputusan MEKL (LOCKED — 13 Nov 2026)

> Verbatim MEKL (13 Nov 2026):
> *"model nya ga ada default, karena hosterm ini ga punya sponsor AI atau model milik
> sendiri, jadi dibuat model settings custom"*

**Arah: C — BYO endpoint.** hosterm **tidak menyediakan model bawaan apa pun**; semua
konfigurasi AI = setting custom yang diisi user. Konsekuensi yang dikunci:

1. **Nol default endpoint, nol default model.** Kedua field mulai **kosong**; kosong = not configured. hosterm tidak punya sponsor AI / model sendiri, jadi tidak ada nilai bawaan yang bisa dipertanggungjawabkan.
2. **v1 = loopback-only.** Endpoint non-loopback ditolak `write_ai_settings` (`ai-settings:remote-disabled`) dan `ai_chat` (`ai:remote-endpoint-disabled`). Toggle "Allow remote endpoints" **dihapus** dari UI. Pertahanan berlapis (validasi loopback, HTTPS-only, redirect policy none) **tetap ada di kode** — UI tidak menawarkannya.
3. **Nol amendemen NFR-3** untuk jalur AI (§7).

Alasan (trade-off yang dipertimbangkan):

1. **NFR-3 murni.** Loopback (`127.0.0.1`/`localhost`/`[::1]`) = lokal, bukan jaringan eksternal. hosterm tetap "zero background / zero cloud" seperti founding principle.
2. **Model nyata, kualitas tidak diragukan.** User memakai model lokal yang kuat (tanpa batasan ukuran bundle) — jauh lebih baik dari model on-device yang dipaksa muat di installer.
3. **Nol installer bloat, nol sidecar, nol dependency baru.** `reqwest` + `keyring` sudah ada; nol binary model, nol manajemen versi model, nol update payload raksasa.
4. **Jujur & sederhana.** Nol nilai bawaan yang menjanjikan model yang tidak ada; label mencerminkan backend nyata.
5. **Kontrak approval §13.6 tidak tersentuh.** AI tetap hanya mengusulkan; eksekusi tetap lewat terminal real-ssh existing.
6. **Konsisten dengan arah produk.** OAuth/Drive sudah membuka jalur network user-initiated; BYO endpoint = pola yang sama (user memilih endpoint), dibatasi loopback di v1.

**Empty state (kedua field kosong):**

- `ai-settings.json` belum ada / `endpoint` kosong / `model` kosong → `ai_chat` **harus** menolak dengan `ai:not-configured`; **0 network, 0 spawn, 0 execute**.
- Head-tag = `not configured`; input AI **disabled** + helper text + tombol **"Set up AI"** yang membuka Settings > AI Assistant.
- Settings > AI Assistant menampilkan field endpoint/model/key **kosong** (nol default) + status *"Not configured — AI requests are disabled until a model is set."*; tombol "Send test request" disabled.
- **Tidak ada proposal hardcoded** yang ditampilkan sebagai jika-nya backend jawab (lihat D-AI12).

### 1.3 Keputusan LOCKED (D-AI)

| ID | Decision |
|---|---|
| D-AI1 | Arah = BYO endpoint (C), **dikonfirmasi MEKL 13 Nov 2026**; **nol default endpoint/model**; v1 loopback-only (§1.2) |
| D-AI2 | Protokol = OpenAI-compatible `POST <endpoint>/chat/completions`; output JSON `{reply, proposal}` |
| D-AI3 | Endpoint = string user; **nol nilai default**. Field mulai **kosong**; placeholder HTML mencontohkan `http://127.0.0.1:11434/v1` (contoh, bukan nilai). Kosong = not configured. Alasan: hosterm tidak punya sponsor AI/model sendiri → nol bawaan yang bisa dipertanggungjawabkan |
| D-AI4 | Model = string user; **nol nilai default**. Field mulai **kosong**. Kosong = not configured (alasan sama dengan D-AI3) |
| D-AI5 | v1 **loopback-only**: endpoint non-loopback ditolak `write_ai_settings` (`ai-settings:remote-disabled`) dan `ai_chat` (`ai:remote-endpoint-disabled`); toggle "Allow remote endpoints" **dihapus** dari UI. Pertahanan berlapis (HTTPS-only, redirect none, validasi loopback) tetap di kode |
| D-AI6 | Key = OS keyring `hosterm`/`ai-api-key`; `has_key` only; nol config/log/argv |
| D-AI7 | AI settings = file app-data terpisah `ai-settings.json`; corrupt → defaults; `HOSTERM_DATA_DIR` override |
| D-AI8 | `ai_chat` proposal-only; nol execute; approval §13.6 FROZEN |
| D-AI9 | Validasi proposal: host `sftp_safe` + ada di config snapshot; command single-line ≤2048; desc ≤1024 |
| D-AI10 | Stateless; nol persistensi history |
| D-AI11 | Label jujur dari endpoint aktual; `on-device` dilarang di v1 |
| D-AI12 | Stub `AI_PROPOSALS` hardcoded dihapus; empty state + sample prompt, nol fake card |
| D-AI13 | Timeout 5–120 s default 30; caps §2.4 |
| D-AI14 | **Nol amendemen NFR-3** untuk jalur AI (loopback-only). Normalisasi NFR-3 untuk OAuth/Drive (v0.14.x) = kartu terpisah, bukan bagian epic AI (§7) |
| D-AI15 | Nol auto-run, nol pattern/glob, nol sync, nol streaming |

---

## 2. Kontrak IPC (BEKU — pola SDD §12.3/§13.5)

Conventions identik §12.3/§13.5: command args **camelCase** di boundary Tauri (auto-map ke snake_case Rust); JSON payload fields **snake_case**; **request/response only — no events**; rejection strings persis, frontend memetakan prefix → copy.

> **FROZEN:** kontrak approval §13.6 **TIDAK berubah**. `ai_chat` **tidak pernah** mengeksekusi command. Satu-satunya jalur eksekusi = (a) klik **Run once**, (b) exact `(host,command)` re-match pada allowlist (FR-AI5/6/7/9). AI hanya mengembalikan teks + proposal; state machine existing (`aiCardState`/`runAiOnce`/`alwaysAllowAi`/`rejectAi`/`executeAiCommand`) dipakai verbatim.

### 2.1 Commands

```ts
invoke("read_ai_settings"): Promise<AiSettings>
// Never throws: missing/corrupt ai-settings.json → DEFAULT_AI_SETTINGS (INV-24 class).
// `has_key` dibaca dari OS keyring; key itu sendiri TIDAK PERNAH dikembalikan.
interface AiSettings {
  version: number;          // 1
  endpoint: string;         // NO DEFAULT; starts empty = not configured (MEKL 13 Nov 2026)
  model: string;            // NO DEFAULT; starts empty = not configured
  remote_allowed: boolean;  // defense-in-depth only; UI toggle removed in v1 (D-AI5)
  has_key: boolean;         // keyring only; the key is never in this payload
}

invoke("write_ai_settings", { req: WriteAiSettingsReq }): Promise<null>
// Atomic whole-file write (temp → rename, 0600-class) ke ai-settings.json (app-data).
// `key`: undefined/null = biarkan key apa adanya; "" = hapus key; non-empty = simpan ke keyring.
// Rejects (frozen): ai-settings:invalid-endpoint | ai-settings:remote-disabled |
//   ai-settings:invalid-model | ai-settings:keyring-unavailable | ai-settings:write-failed.
// NEVER reads/writes ~/.ssh/config atau sidecar (INV-22 class).
interface WriteAiSettingsReq {
  endpoint: string;
  model: string;
  remote_allowed: boolean;
  key?: string | null;
}

invoke("ai_chat", { req: AiChatReq }): Promise<AiChatResp>
// Proposal-only. NEVER executes. Stateless (no server-side history).
interface AiChatReq {
  prompt: string;            // non-empty after trim; ≤ 8192 UTF-8 bytes
  host: string | null;       // scope alias dari panel AI; null = belum dispesifikkan
  history: AiMessage[];      // ≤ 20 turns, total ≤ 16384 UTF-8 bytes
  timeout_secs: number;      // 5..120; default 30 — backend REJECTS out of range, never clamps
}
interface AiMessage { role: "user" | "assistant"; content: string }

interface AiChatResp {
  reply: string;             // model text; ≤ 16384 bytes (truncated dengan penanda bila lewat)
  proposal: AiProposal | null;
  backend: "loopback" | "remote";  // sumber label jujur (FR-AI12)
  warning: string | null;    // frozen reason codes di bawah; null bila proposal valid
}
interface AiProposal {
  host: string;              // alias PERSIS, tervalidasi terhadap config snapshot
  command: string;           // single-line, ≤ 2048 UTF-8 bytes
  description: string;       // ≤ 1024 UTF-8 bytes
}
```

### 2.2 Frozen reason / error strings (exact)

**`ai_chat` rejections (`Err(...)`, 0 network/0 execute pada kondisi ini):**

| Err string | Condition |
|---|---|
| `ai:not-configured` | `model` kosong ATAU endpoint kosong (empty state) |
| `ai:empty-prompt` | `prompt` kosong/whitespace-only |
| `ai:prompt-too-large` | `prompt` > 8192 UTF-8 bytes |
| `ai:history-too-large` | `history` > 20 turns ATAU total > 16384 UTF-8 bytes |
| `ai:bad-timeout` | `timeout_secs` di luar 5..120 |
| `ai:bad-endpoint` | URL parse gagal / scheme bukan http(s) / userinfo ada / > 2048 bytes |
| `ai:remote-endpoint-disabled` | endpoint non-loopback saat `remote_allowed == false` |
| `ai:endpoint-unreachable` | DNS/connect/TLS gagal (pesan TIDAK PERNAH memuat key/URL ber-credential) |
| `ai:timeout` | request melewati `timeout_secs` |
| `ai:unauthorized` | HTTP 401/403 (key ditolak endpoint) |
| `ai:model-error` | HTTP non-2xx lain, body unparseable, atau `choices[0].message.content` absen |
| `ai:keyring-unavailable` | key diperlukan tapi keyring gagal dibaca → `ai_chat` menolak (fail-closed) |

**`ai_chat` warnings (`Ok` + `proposal: null` + `warning`):**

| warning | Condition |
|---|---|
| `ai:unsafe-host` | proposal host gagal `sftp_safe` (leading `-`, CR/LF) — INV-26 |
| `ai:unknown-host` | proposal host tidak ada di config snapshot |
| `ai:command-too-large` | proposal command > 2048 bytes |
| `ai:invalid-proposal` | JSON proposal gagal validasi lain (schema/description > 1024 bytes) |

**`write_ai_settings` rejections:** `ai-settings:invalid-endpoint`, `ai-settings:remote-disabled`, `ai-settings:invalid-model`, `ai-settings:keyring-unavailable`, `ai-settings:write-failed`.

### 2.3 Perilaku bila endpoint/model belum dikonfigurasi

1. `read_ai_settings` → `{ version:1, endpoint:"", model:"", remote_allowed:false, has_key:false }` — **kedua field kosong** (nol default).
2. Frontend: head-tag `not configured`; input disabled; tombol **Set up AI** → Settings > AI Assistant.
3. `ai_chat` → `Err("ai:not-configured")` — **0 network, 0 spawn, 0 execute**.
4. Settings: status *"Not configured"*; "Send test request" disabled.
5. **Nol proposal hardcoded** — thread kosong, nol fake card (D-AI12).

### 2.4 Timeout & batas ukuran (single source of truth)

| Name | Value | Meaning |
|---|---|---|
| `AI_PROMPT_MAX_BYTES` | 8192 | request prompt cap |
| `AI_HISTORY_MAX_TURNS` | 20 | prior turns cap |
| `AI_HISTORY_MAX_BYTES` | 16384 | total history cap |
| `AI_REPLY_MAX_BYTES` | 16384 | reply cap (truncate + penanda) |
| `AI_COMMAND_MAX_BYTES` | 2048 | proposal command cap |
| `AI_DESC_MAX_BYTES` | 1024 | proposal description cap |
| `AI_ENDPOINT_MAX_BYTES` | 2048 | endpoint URL cap |
| `AI_MODEL_MAX_BYTES` | 256 | model name cap |
| `timeout_secs` | default 30, range 5–120 | total request timeout; reject, never clamp |

### 2.5 Backend flow `ai_chat` (untuk architect/lead — kontrak, bukan implementasi)

1. Load `ai-settings.json` (missing/corrupt → defaults). `model` kosong ATAU `endpoint` kosong → `ai:not-configured`.
2. Validasi prompt/history/timeout (tabel §2.2).
3. Validasi endpoint: URL parse; scheme `http`/`https`; **userinfo dilarang**; loopback check (`localhost`/`127.0.0.1`/`[::1]`). Non-loopback: `remote_allowed` wajib `true` **dan** scheme wajib `https` (defense-in-depth — v1 UI selalu `remote_allowed=false`, D-AI5).
4. Bangun request `POST <endpoint>/chat/completions` (OpenAI-compatible) dengan system prompt + `history` + `prompt`. **TIDAK PERNAH** mengirim isi `~/.ssh/config`, key, sidecar, atau output terminal.
5. Header `Authorization: Bearer <key>` **hanya** bila `has_key` dan endpoint remote; key diambil dari keyring, **tidak pernah** dimasukkan ke log/error.
6. Timeout `timeout_secs`; redirect policy **none** (cegah SSRF/credential leak). Kirim, tangkap error → frozen string.
7. Parse `choices[0].message.content`. Coba parse JSON proposal `{reply, proposal{host,command,description}}`. Bila gagal → `reply` = teks mentah, `proposal: null`.
8. Validasi proposal: host `sftp_safe` + ada di config snapshot; command single-line + ≤2048; description ≤1024. Gagal → `warning` + `proposal: null`.
9. Kembalikan `AiChatResp`. **Nol execute.**

**System prompt contract (LOCKED):** menginstruksikan model membalas JSON `{reply, proposal}`; `proposal` hanya berisi command single-line + host alias + deskripsi dampak; **tidak pernah** mengeksekusi; **tidak pernah** mengaku mempunyai akses terminal; **tidak pernah** mengirim secret. Kata-kata persis = detail architect/lead, kontrak perilaku = beku.

### 2.6 Frontend behavior (kontrak)

- `sendAiStub` → `sendAiMessage`: baca input, trim, abort bila kosong; append user message; panggil `ai_chat`; render reply (escaped) + proposal card (client-generated `id`) via template existing.
- `ai:not-configured` → empty state + Set up AI.
- Error lain → pesan error (escaped) dengan copy mapper; **nol execute**.
- Head-tag di-set dari `AiChatResp.backend` (§4).
- Input + tombol send disabled selama request in-flight.

### 2.7 Mock fixture (copy-paste untuk frontend unit)

```json
{ "version": 1, "endpoint": "http://127.0.0.1:11434/v1", "model": "llama3.2",
  "remote_allowed": false, "has_key": false }
```

**IPC inventory:** **45 → 48** (`grep -rc "tauri::command" src-tauri/src/*.rs`; baseline live = `lib.rs` 43 + `fleet.rs` 2). §11.5 self-check + STATE.md verify loop diperbarui saat unit mendarat.

---

## 3. Permukaan Settings

**Lokasi:** Settings > **AI Assistant** (nav `data-section="ai"` sudah ada; section `#settings-approvals`). Section yang sama menampung **AI backend** (baru) + **Approvals** (existing, read/revoke list — FR-AI10 tidak berubah).

### 3.1 Field (blok "AI backend", di atas blok Approvals)

| Field | Elemen | Perilaku |
|---|---|---|
| Endpoint URL | `#ai-endpoint` (text) | **nol default**; field **kosong** saat pertama; placeholder `http://127.0.0.1:11434/v1` (contoh loopback, bukan nilai); validasi di backend |
| Model | `#ai-model` (text) | **nol default**; field **kosong** saat pertama; placeholder `llama3.2`; kosong = not configured |
| API key | `#ai-key` (password) | **opsional**; hint *"Stored in the OS keyring. Leave blank to keep the saved key."*; indikator `#ai-key-state` = `Saved` / `Not saved` / `Keyring unavailable` |
| Test | `#ai-test` (button) + `#ai-status` | "Send test request" → `ai_chat` dengan prompt tetap (bukan user prompt); **tidak** menyimpan apa pun; disabled saat not configured |
| Label | `#ai-backend-label` | preview head-tag jujur (§4) |

**Toggle "Allow remote endpoints" dihapus** dari UI (D-AI5) — v1 loopback-only; endpoint non-loopback ditolak backend.

### 3.2 Penyimpanan key (LOCKED)

- **OS keyring**, service `hosterm`, account `ai-api-key` — **entri terpisah** dari `google-refresh-token` (OAuth) dan `backup-data-key` (backup).
- **TIDAK PERNAH** di `~/.ssh/config`, **tidak** di `ai-settings.json`, **tidak** di log, **tidak** di argv, **tidak** di error string.
- `read_ai_settings` mengembalikan `has_key: boolean` saja; key **tidak pernah** crossing IPC response.
- Hapus key = `write_ai_settings` dengan `key: ""`.
- **Fail-closed:** bila keyring unavailable, `write_ai_settings` dengan key non-kosong **menolak atomik** (`ai-settings:keyring-unavailable`, nol write)； endpoint/model **tidak** tersimpan. Keyless loopback tetap bisa dipakai. **Tidak ada fallback file 0600** di v1 ( OQ-AI3 ).

### 3.3 Empty state (belum dikonfigurasi)

- Settings: status *"Not configured — AI requests are disabled until a model is set."*; test disabled.
- AI drawer: head-tag `not configured`; input disabled; helper + tombol **Set up AI** → Settings > AI Assistant.
- **Tidak ada fake command card.** `AI_PROPOSALS` hardcoded dihapus (D-AI12); thread mulai kosong dengan sample prompt (tanpa card).

### 3.4 Yang TIDAK disimpan

- Key (keyring saja) · prompt/response/history (stateless, nol persistensi) · token OAuth (entri keyring lain) · isi `~/.ssh/config`/sidecar (INV-22).

---

## 4. Kejujuran label (FR-AI12)

Head-tag `.head-tag` **harus** mencerminkan backend nyata, diturunkan dari endpoint aktual saat request — bukan string statis.

| State | Label yang DIPERBOLEHKAN | Label yang DILARANG |
|---|---|---|
| Belum dikonfigurasi (`endpoint` atau `model` kosong) | `not configured` | `on-device`, `local endpoint`, `remote endpoint`, `mock` |
| Endpoint loopback | `local endpoint` | `on-device`, `remote endpoint`, `cloud` |
| Endpoint remote | `remote endpoint` | `on-device`, `local endpoint`, `private`, `cloud` (kecuali memang cloud) |
| (Masa depan) on-device bundle | `on-device` | — (hanya bila epic on-device benar-benar ship) |

> **v1 = loopback-only (D-AI5):** state "remote" tidak dapat dicapai dari UI. Label `remote endpoint` dipertahankan di kontrak (`AiChatResp.backend`) sebagai defense-in-depth; UI v1 tidak pernah menghasilkannya.

**Aturan:**

1. **"on-device" dilarang di v1** — model tidak di-bundle di dalam hosterm; ia jalan di proses terpisah (Ollama/llama.cpp/dll). Loopback ≠ on-device.
2. Label di-set dari `AiChatResp.backend` / `read_ai_settings` + lookup loopback; **bukan** hardcoded.
3. Title/tooltip head-tag menjelaskan: mis. *"Connected to a local endpoint you configured."* / *"Connected to a remote endpoint you configured."*
4. Teks AI di thread **tidak boleh** mengklaim `on-device`/`private`/`local` bertentangan dengan state nyata.
5. Grep gate: `on-device` di `src/` = **0** (kecuali epic on-device ship).

---

## 5. Keamanan (jangan dilewat)

### 5.1 Hostile input → escaping (FR-AI11, INV-11 family)

- `reply`, `proposal.host`, `proposal.command`, `proposal.description`, dan semua pesan error = **input hostile** → render via `textContent`/`esc()`/`escAttr()`; **nol `innerHTML`** dengan data model. CSP (ADR-6) = backstop struktural.
- Command proposal dirender di `<code>` via `textContent`; eksekusi hanya lewat state machine existing (§13.6).

### 5.2 Nol auto-run implisit (FR-AI9)

- `ai_chat` **tidak pernah** mengeksekusi. Eksekusi hanya: (a) klik **Run once**, (b) exact `(host,command)` re-match allowlist yang user buat lewat **Always allow**.
- Allowlist = exact `(host,command)` byte-match (ADR-21); **nol pattern/glob** (OQ-6). Persist via `write_settings` existing — **tidak berubah**.

### 5.3 Key/token

- Keyring saja (§3.2); `has_key` boolean; nol key di config/log/argv/IPC response/error string.
- Key dikirim **hanya** ke endpoint yang dikonfigurasi, header `Authorization: Bearer`; redirect policy **none** (cekapat ke host lain = ditolak).
- Non-loopback wajib **HTTPS**; loopback boleh HTTP (defense-in-depth — v1 UI menolak non-loopback lebih awal, D-AI5). Userinfo di URL dilarang (`https://user:pass@…` → `ai:bad-endpoint`).

### 5.4 NFR-3 — jalur network yang DITAMBAH

| Jalur | Trigger | Eksternal? | User-initiated? |
|---|---|---|---|
| `ai_chat` | kirim pesan AI | **Tidak** (loopback-only, D-AI5) | Ya |
| "Send test request" | klik tombol test | **Tidak** (loopback) | Ya |
| — | idle/background/telemetry/model-download | **Tidak ada** | — |

> v1 **loopback-only**: jalur AI **nol jaringan eksternal** → **nol amendemen NFR-3** (§7). Endpoint non-loopback ditolak sebelum request (defense-in-depth).

- **Nol** network saat idle, nol telemetry, nol model download, nol background sync.
- Prompt + history (user-typed) + system prompt **satu-satunya** yang dikirim; **tidak** ada isi `~/.ssh/config`, key, sidecar, atau output terminal yang dikirim ke model.
- Host proposal divalidasi **lokal** terhadap config snapshot — alias list **tidak** dikirim ke model (privacy).

### 5.5 Logging

- Nol log prompt/response/key/endpoint-ber-credential. Error string hanya kode beku (§2.2).
- Nol `log::`/`eprintln!` di jalur AI yang memuat data model atau key.

---

## 6. Di luar scope (eksplisit)

- **On-device model bundling** (GGUF + sidecar llama.cpp) — epic terpisah bila MEKL memilih arah A.
- **Pattern/glob allowlist** (OQ-6) — exact `(host,command)` only.
- **Sync lintas-device** AI settings/key/history — machine-local (NFR-8 trade-off).
- **Streaming** response — v1 request/response only (§13.5 "no events" posture).
- **Auto-exec / tool-use oleh model** — eksekusi tetap human-in-the-loop via real-ssh terminal.
- **Manajemen model** (download/quantize/serve) — tanggung jawab user via layanan terpisah.
- **Injeksi konteks host otomatis** (alias list, output terminal) ke prompt — v1 tidak; model menebak dari prompt, backend memvalidasi host.

---

## 7. NFR-3 — TIDAK DIPERLUKAN untuk jalur AI (MEKL 13 Nov 2026)

**Keputusan:** v1 **loopback-only** (D-AI5) → jalur AI **nol jaringan eksternal** →
**nol amendemen NFR-3** untuk AI. `PRD.md` §5 NFR-3 **tidak diubah** oleh task ini.

**Normalisasi NFR-3 untuk OAuth/Drive (shipped v0.14.x)** — klausul (c) pada usulan lama —
adalah **utang dokumen pra-OQ-4**, bukan bagian dari epic AI. Dipindah ke **kartu
terpisah** (pm, doc-only, menunggu persetujuan MEKL): jangan dicampur ke epic ini.

---

## 8. Test plan / acceptance (EARS)

### 8.1 Smoke (frontend, `qa-reports/headless-smoke.mjs`)

1. WHEN Settings > AI Assistant dibuka THE SYSTEM SHALL menampilkan field endpoint/model/key (**endpoint & model kosong**, nol default) + status + label jujur (§3); toggle "Allow remote endpoints" **tidak ada** (D-AI5).
2. WHEN `model` kosong ATAU `endpoint` kosong THE SYSTEM SHALL menampilkan empty state + tombol Set up AI; input AI disabled; `ai_chat` → `ai:not-configured` (0 network) (§2.3).
3. WHEN endpoint loopback dikonfigurasi + mock server THE SYSTEM SHALL membalas reply (escaped) + proposal card via state machine §13.6 (§2.5/2.6).
4. WHEN output model/hostile fixture di-render THE SYSTEM SHALL escape (childCount 0, `__pwn` undefined, CSP 0) (§5.1).
5. WHEN proposal host leading `-`/CR/LF THE SYSTEM SHALL menolak (`ai:unsafe-host`) — nol card, nol execute (INV-26).
6. WHEN proposal host tak dikenal THE SYSTEM SHALL menolak (`ai:unknown-host`) — nol card, nol execute.
7. WHEN endpoint non-loopback + `remote_allowed=false` THE SYSTEM SHALL menolak (`ai:remote-endpoint-disabled`) — 0 network (§5.4).
8. WHEN key disimpan THE SYSTEM SHALL `has_key=true`; key **tidak** ada di `ai-settings.json` (grep) maupun log (§3.2).
9. WHEN Run once / Always allow / Reject THE SYSTEM SHALL berperilaku identik §13.6 AC4–7 (FROZEN).
10. WHEN `(host,command)` exact match allowlist THE SYSTEM SHALL render `ALWAYS ALLOWED` tanpa approval ulang (FR-AI7).
11. WHEN head-tag di-render THE SYSTEM SHALL memakai label state-nya (§4) — `on-device` = 0 di `src/`.

### 8.2 Cargo (`cargo test --lib`)

- `ai-settings` read/write round-trip; corrupt → defaults + bytes untouched (INV-24 class); config/sidecar byte-identity (INV-22 class).
- URL validation: loopback vs remote, scheme, userinfo, length; `remote-disabled`.
- Proposal validation: `sftp_safe`, config snapshot membership, command single-line/length, description length.
- Caps: prompt/history/timeout/command/reply/description/endpoint/model.
- Keyring save/delete/has_key (skip bila keyring unavailable).
- Error-string table (§2.2) — exact.

### 8.3 Grep gates

- `on-device` di `src/` = 0 · `ai-api-key` tidak di schema `settings.json`/`~/.ssh/config` · `sh -c|bash -c|cmd /C` = 0 di jalur AI · nol log key/prompt.

### 8.4 Verify loop

- `cargo test --lib` → hijau di baseline STATE.md (+ tes AI) · `npm run build` → 0 TS · smoke → hijau di baseline STATE.md (+ cek AI) · IPC **45 → 48** (live gate) · CSP 0 · baseline di-re-measure saat dispatch (NFR-5).

---

## 9. Open Questions (residual)

1. **OQ-AI1 (OQ-4) — RESOLVED (MEKL, 13 Nov 2026):** arah **C (BYO endpoint)**, **loopback-only**, **nol default endpoint/model**. Bukti: §1.2 (quote verbatim MEKL).
2. **OQ-AI2 — RESOLVED (MEKL, 13 Nov 2026):** toggle "Allow remote endpoints" **dihapus**; v1 **loopback-only**.
3. **OQ-AI3 (keyring fallback):** Bila OS keyring unavailable, apakah "key tak tersimpan, keyless loopback only" diterima? Rekomendasi pm: **ya, fail-closed**; nol fallback file 0600 di v1. **Rekomendasi PM tetap berlaku.**
4. **OQ-AI4 (host scope):** AI menebak host dari prompt + backend memvalidasi (v1), atau tambah selector host di panel AI? Rekomendasi pm: **validasi dulu**; selector hanya bila user benar-benar butuh. **Rekomendasi PM tetap berlaku.**
5. **OQ-AI5 (history):** Chat history dipersist? Rekomendasi pm: **tidak** (stateless, privacy). **Rekomendasi PM tetap berlaku.**
6. **OQ-AI6 (stub removal):** Hapus `AI_PROPOSALS` hardcoded saat backend ship? Rekomendasi pm: **ya** (kosongkan thread + sample prompt, tanpa fake card) — kejujuran label. **Rekomendasi PM tetap berlaku.**

---

## 10. Sources

- `docs/FEATURE-SPEC-redesign-dusk-v1.md` §4.5 (FR-AI1..13) + §6 OQ-4.
- `docs/SDD.md` §13.4 (store/allowlist) · §13.5 (IPC contract pattern) · §13.6 (approval FROZEN) · §13.8 (ADR-21/22, INV-25/26) · §13.10 (OQ-4 deferred).
- `docs/PRD.md` §5 NFR-3 · US-11 · §7 (Dusk row).
- `docs/EPIC-TSD-redesign-dusk-v1.md` §U-D7 + §"Settings nav".
- `docs/STATE.md` verify loop + hard invariants + Dusk invariants.
- Live code: `src/main.ts:188` (`AI_PROPOSALS`), `:1587` (`sendAiStub`), `:1443` (`renderAiThread`), `:1517` (`executeAiCommand`); `index.html:138` (`.head-tag`), `:215-222` (settings nav), `:294-304` (`#settings-approvals`); `src-tauri/src/lib.rs:837-976` (`Settings`/`read_settings`/`write_settings`), `:881` (`data_dir`); `src-tauri/Cargo.toml` (`reqwest`, `keyring`).

*Read 2026-11-13; patched 13 Nov 2026 untuk keputusan MEKL (card `t_5009863d`). PM = doc only; no commit — lead pushes after review.*
