# TSD: Real Google OAuth + zero-knowledge Drive backup/restore (v1)

Status: EXECUTION TRUTH for the gdrive-backup feature. Owner: crew-coding-lead.
Date: 2026-11-03. Repo: hosterm (Tauri v2 + WebView2 + Rust backend, xterm UI).
Grounds on: `docs/prd/discovery-gdrive-backup-v1.md` (PRD/AC + security gate).
This TSD answers HOW it is built: locked stack, pinned deps, IPC surface,
physical format, task breakdown, test strategy, rollout. Architecture rationale
(SDD) is deferred — nano's discovery doc carries the component design; this TSD
locks the buildable spec from it.

LOCKED DECISIONS (from MEKL, via discovery doc — NOT re-litigated here):
PKCE loopback (127.0.0.1 ephemeral), S256, `access_type=offline`+`prompt=consent`;
PUBLIC build-time CLIENT_ID `737778700861-dhtn1sc9miiqtsmtms3tmc8tk7jj3a80.apps.googleusercontent.com`
(try PKCE-only, no client_secret, first); scope `drive.file` ONLY; client-side
zero-knowledge encryption with user passphrase BEFORE upload; refresh_token in OS
keyring; `.htbk` container; restore writes real paths only after a timestamped
local pre-restore backup; wrong passphrase fails safe on the AEAD tag.

---

## 1. Locked stack + pinned deps (vetted, no typosquat)

New `src-tauri/Cargo.toml` dependencies. All versions confirmed on crates.io,
reputable publishers (RustCrypto/tarcieri, keyring-rs/brotskydotcom,
seanmonstar/reqwest). PIN EXACT (`=x.y.z`) to keep the lockfile reproducible and
block silent minor bumps in a security-sensitive surface.

| Crate | Pin | Why / features | Publisher |
|---|---|---|---|
| `reqwest` | `=0.12.9` | HTTPS client for token + Drive REST. **rustls only** (no OpenSSL): `default-features=false`, features `["rustls-tls","json","blocking"]`. ADR-1 spawns real ssh; this is the one sanctioned HTTP client, user-initiated only. | seanmonstar |
| `keyring` | `=3.6.3` | refresh_token at rest. Linux feature `["sync-secret-service","crypto-rust"]` (Secret Service, pure-Rust crypto, no system OpenSSL). Service `"hosterm"`, account `"google-refresh-token"`. | keyring-rs |
| `argon2` | `=0.5.3` | KDF passphrase→256-bit key. edition 2021, MSRV 1.65 (mature; 0.6 is edition-2024 churn — avoid). Features `["std"]`. | RustCrypto |
| `chacha20poly1305` | `=0.10.1` | AEAD (XChaCha20-Poly1305, 24-byte nonce). edition 2018, mature (0.11 is RC — avoid). Feature `["std"]`. | RustCrypto |
| `zeroize` | `=1.8.1` | Wipe passphrase + derived key + plaintext buffers from memory after use. `derive` feature for `#[derive(ZeroizeOnDrop)]`. | iqlusion |
| `sha2` | `=0.10.9` | `S256` code_challenge = BASE64URL(SHA256(verifier)). edition 2018 mature line (NOT 0.11 edition-churn). | RustCrypto |
| `base64` | `=0.22.1` | URL-safe-no-pad for PKCE + standard for manifest content. (0.23 exists; 0.22.1 is the widely-pinned stable line — either is fine, pin ONE.) | marshallpierce |
| `rand` | `=0.8.5` | CSPRNG for verifier/state/salt/nonce via `rand::rngs::OsRng`. edition 2018 mature line (NOT 0.9/0.10 API churn). | rust-random |
| `tiny_http` | `=0.12.0` | Single-shot loopback redirect catcher on 127.0.0.1:0. No TLS feature. | tiny-http |

Notes / guardrails for implementers:
- `open` crate is NOT added — hosterm is Tauri; open the auth URL with the
  existing `tauri-plugin-opener` / shell opener already in the app (BE-OAUTH
  checks `Cargo.toml` for the installed opener before adding anything). New
  dependency = the card must state why existing code can't do it.
- reqwest pulls `tokio`. The app is Tauri (async runtime present). If a command
  needs blocking semantics inside a sync `#[tauri::command]`, use reqwest
  `blocking` on a spawned thread — do NOT block the Tauri main thread.
- SECURITY card re-vets every pin: exact-version in `Cargo.toml`, crate name
  spelled correctly, publisher matches the table, `cargo tree` shows no
  surprise transitive OpenSSL (rustls path only). A deviation is a review-block.

## 2. Backend module layout (file scope)

All new backend code lives in `src-tauri/src/`. ADR-1 (real ssh binaries) and the
config/secrets ownership in `lib.rs` are untouched. To keep `lib.rs` from
growing unreviewably and to keep the crypto PURE + unit-testable, split into new
modules; `lib.rs` only gains the thin `#[tauri::command]` wrappers + `mod`
declarations.

| File | Owns |
|---|---|
| `src-tauri/src/oauth.rs` (NEW) | PKCE helpers (verifier/challenge/state gen), loopback listener, auth-URL build, token exchange + refresh HTTP, keyring read/write/delete of refresh_token. Pure helpers unit-tested; HTTP behind a seam. |
| `src-tauri/src/crypto.rs` (NEW) | KDF (argon2id), AEAD seal/open (XChaCha20-Poly1305), `.htbk` encode/decode, zeroize. 100% PURE — no I/O, no network. Fully unit-tested. |
| `src-tauri/src/drive.rs` (NEW) | Drive REST (`drive.file`): multipart create, list (by appProperties), get/download, update. 401→refresh hook. reqwest behind a trait seam so tests mock HTTP. |
| `src-tauri/src/backup.rs` (NEW) | Orchestration: gather 4 sources → manifest → `crypto::encode` → `drive::upload`; restore: `drive::download` → `crypto::decode` → pre-restore local backup → write real paths. Reuses `lib.rs` path fns. |
| `src-tauri/src/lib.rs` (EDIT, lead-coordinated) | `mod oauth; mod crypto; mod drive; mod backup;` + the thin `#[tauri::command]` wrappers listed in §3. NO business logic here. The config/secrets/atomic-write code is SACRED — untouched. |

`lib.rs` is a hotspot (every backend card technically needs the `mod`+wrapper
lines). RULE: only **BE-OAUTH** edits `lib.rs` (adds all four `mod` lines + its
own wrappers in its unit); the later backend cards (BE-CRYPTO/DRIVE/BACKUP) add
ONLY their own `#[tauri::command]` wrapper lines, serialized after BE-OAUTH via
dependency link, and each quotes the IPC-count delta in its handoff. No two
backend cards touch `lib.rs` concurrently.

## 3. IPC surface (new `#[tauri::command]` — count is a gate)

Current baseline: `lib.rs`=25, `fleet.rs`=2 (STATE.md number of record). This
feature adds exactly **7** commands, all in `lib.rs` → new `lib.rs` count = 32.
Each card that adds a command MUST update STATE.md's IPC count in the same unit
and quote `grep -rc "tauri::command" src-tauri/src/*.rs`.

| Command | Card | Shape (secrets NEVER cross IPC) |
|---|---|---|
| `google_sign_in` | BE-OAUTH | `() -> Result<GoogleAccount,String>`. Runs PKCE loopback, stores refresh_token in keyring, returns `{name,email,picture}` from userinfo. Access token stays backend-side. |
| `google_sign_out` | BE-OAUTH | `() -> Result<(),String>`. Deletes keyring refresh_token, clears in-memory access token. |
| `google_account` | BE-OAUTH | `() -> Result<Option<GoogleAccount>,String>`. Signed-in state on app load (keyring has token → refresh → userinfo); `None` if signed out. |
| `drive_backup` | BE-BACKUP | `(passphrase:String) -> Result<BackupResult,String>`. Gather→encrypt→upload. Passphrase arrives from the in-app dialog, is zeroized after KDF, never logged/persisted. Returns `{file_id,name,bytes,created_at}`. |
| `drive_list_backups` | BE-BACKUP | `() -> Result<Vec<BackupMeta>,String>`. Lists `.htbk` via appProperties (no decrypt): `{file_id,name,created_at,host,schema}`. |
| `drive_restore` | BE-BACKUP | `(file_id:String, passphrase:String) -> Result<RestoreResult,String>`. Download→decrypt (fail-safe on tag)→pre-restore local backup→write real paths. Returns `{restored:[paths], pre_restore_dir}`. Passphrase zeroized. |
| `drive_restore_preflight` | BE-BACKUP | `(file_id:String) -> Result<RestorePreflight,String>`. Lists which real paths WOULD be overwritten (for the FE overwrite warning) WITHOUT needing the passphrase (reads manifest file list? No — manifest is encrypted; instead returns the fixed known target set + which currently exist). |

Passphrase transits IPC as a plain `String` arg (unavoidable — the dialog is in
the WebView, KDF is in Rust). Mitigation (SECURITY gate): the arg is moved into a
`Zeroizing<String>` on entry, never cloned into logs/errors, and the WebView
never persists it. This is documented and the SECURITY card audits it.

## 4. `.htbk` physical format (BE-CRYPTO owns; frozen here)

Binary container, little-endian lengths. Header is CLEARTEXT (params are not
secret) and is AEAD-**authenticated as associated data** so it can't be tampered.

```
offset  field                     bytes
0       magic   "HTBK"            4            0x48 0x54 0x42 0x4B
4       version u8                1            = 1
5       kdf_id  u8                1            1 = argon2id
6       aead_id u8                1            1 = XChaCha20-Poly1305
7       reserved u8               1            0
8       m_cost  u32 LE            4            argon2 memory KiB (default 65536 = 64 MiB)
12      t_cost  u32 LE            4            argon2 iterations (default 3)
16      p_cost  u32 LE            4            argon2 parallelism (default 1)
20      salt_len u16 LE           2            = 16
22      salt                      16           random (OsRng)
38      nonce                     24           XChaCha20 nonce, random (OsRng)
62      ct_len  u64 LE            8            ciphertext+tag length
70      ciphertext||tag           ct_len       AEAD over the plaintext manifest, AAD = header bytes [0..62)
```

- KDF: argon2id, output 32 bytes, params as stored in header (defaults above;
  m=64MiB/t=3/p=1 — SECURITY confirms ≥ OWASP floor). Derived key is
  `Zeroizing<[u8;32]>`.
- AEAD: XChaCha20-Poly1305, 24-byte nonce, **unique random nonce per backup**
  (never reused — fresh OsRng each encode). AAD = the 62-byte cleartext header.
- Plaintext manifest (pre-encryption), JSON:
  ```json
  {"schema":1,"created_at":"<UTC ISO8601>","host":"<hostname>",
   "files":[{"path_role":"ssh_config|secrets|settings|identity_key",
             "rel":"<basename or IdentityFile ref>","mode":384,"content_b64":"..."}]}
  ```
  `path_role` drives restore target resolution via `lib.rs` path fns
  (`config_path`/`secrets_path`/`settings_path`) + resolved IdentityFile paths.
- Drive `appProperties` (cleartext, for listing without decrypt):
  `{"htbk_schema":"1","created_at":"<ISO>","host":"<hostname>"}`.
  NO filenames, NO host aliases, NO key hints — appProperties must leak nothing.
- Wrong passphrase → wrong key → AEAD tag check fails → `decode` returns
  `Err("decryption failed")` BEFORE any plaintext is produced or written.

## 5. Restore safety (BE-BACKUP owns)

Order is mandatory and fail-safe:
1. Download blob, `crypto::decode(passphrase)` → manifest. If tag fails: abort,
   nothing touched, surface "wrong passphrase or corrupt backup".
2. Compute target real paths (via `lib.rs` path fns + IdentityFile resolution).
3. PRE-RESTORE BACKUP: copy every existing target that would be overwritten into
   a timestamped dir `data_dir()/pre-restore-<UTC-compact>/` preserving relative
   layout. If this copy fails, ABORT before writing anything.
4. Write files atomically (sibling temp → chmod 0600 for secrets/keys/config →
   rename), reusing the existing atomic-write discipline. `~/.ssh/config` must
   stay real-ssh-parseable (INV-1); secrets never land in config (INV-1).
5. Return `{restored:[...], pre_restore_dir}`.

INV-26 host-alias guard is NOT bypassed: restored config content is written
verbatim (it was the user's own backed-up config), but BE-BACKUP must not
re-derive approvals/PTY commands from it — it only restores files.

## 6. Test strategy (verify loop — AGENTS.md §4 baselines are number of record)

- **crypto.rs**: pure unit tests — KDF determinism (same salt+pass→same key),
  AEAD round-trip, wrong-passphrase fails on tag, tampered-header fails (AAD),
  tampered-ciphertext fails, nonce uniqueness across encodes, `.htbk` byte-layout
  golden test. NO network.
- **oauth.rs**: unit — verifier length/charset, challenge = BASE64URL(SHA256),
  state CSRF mismatch rejected, loopback binds 127.0.0.1 only + single-shot;
  token exchange/refresh behind an HTTP seam (inject a fake responder).
- **drive.rs**: unit against a mock HTTP seam — multipart body shape, list query
  filter, 401→refresh retry path, appProperties leak-check (no secret fields).
- **backup.rs**: integration with `HOSTERM_CONFIG`/`HOSTERM_DATA_DIR` temp
  overrides + a fake Drive + a tmp HOME: backup→restore round-trip equality;
  wrong-passphrase no partial write; pre-restore dir created and populated;
  ciphertext-at-rest grep (a known host alias in the source does NOT appear in
  the uploaded blob bytes = 0 hits).
- **smoke** (`qa-reports/headless-smoke.mjs`): extend with a MOCKED invoke map
  for the 7 new commands + the real sign-in chip/popover state machine +
  passphrase dialog (masked input, confirm-on-backup, overwrite warning on
  restore). CSP-violation counter stays 0; smoke count may only GROW. No real
  network from the WebView (NFR-3 network log stays clean — Drive/token HTTP is
  backend-only, user-initiated).
- Full loop: `cd src-tauri && cargo test --lib` green at the new baseline;
  `npm run build` 0 TS; Playwright smoke green then removed per AGENTS.md §4
  step 3 (`mv` to `$TMPDIR/pw-trash`, never `rm -rf` headless).

## 7. Rollout + BLOCKER

- completion_contract = **local-only**, direct-push to `origin/main` (NO PR).
  Workspace dir `/home/mekl/src/hosterm`, created-by default.
- Build everything NOW behind the public CLIENT_ID. AC1 live end-to-end sign-in
  is GATED on MEKL confirming Google Console prereqs (Drive API enabled on
  project 737778700861, consent screen with `drive.file`, MEKL's email as Test
  user). Until confirmed: cover auth with unit/integration tests + a documented
  manual-auth procedure (BE-OAUTH writes `docs/manual-auth-gdrive.md`). QA e2e
  of AC1 is itself blocked on the console; QA proves everything else now and
  flags AC1 as console-gated in its handoff.
- Version bump + release are the orchestrator's job (not these cards).

## 8. Dependency order (how cards are linked)

```
BE-OAUTH ──┐
           ├─► BE-DRIVE ──┐
BE-CRYPTO ─┴──────────────┼─► BE-BACKUP ─► FE-BACKUP-UI
           └──────────────┘
BE-OAUTH ─► FE-AUTH
```
- BE-OAUTH + BE-CRYPTO: file-disjoint (`oauth.rs` vs `crypto.rs`), run in
  PARALLEL. BE-OAUTH owns the `lib.rs` `mod` lines (serialization point).
- BE-DRIVE depends on BE-OAUTH (needs token/refresh + the `mod drive;` line
  landed after BE-OAUTH's `lib.rs` edit).
- BE-BACKUP depends on BE-CRYPTO + BE-DRIVE (uses both).
- FE-AUTH depends on BE-OAUTH (real sign-in/out/account commands).
- FE-BACKUP-UI depends on BE-BACKUP (backup/restore/list commands) AND FE-AUTH
  (shares the Google chip/popover surface in `main.ts` — serialize, same
  hotspot file).
- Each impl card has a linked QA child and a linked SECURITY child (parent =
  impl card). SECURITY is heavy and explicit per §6/PRD gate.
