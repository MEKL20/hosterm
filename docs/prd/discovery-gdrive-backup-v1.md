# Discovery: Real Google Sign-in + zero-knowledge Drive backup/restore (v1)

Status: grounding doc for crew. Author: nano (Tier-0 triage from real code + MEKL decisions).
Date: 2026-11-03. Repo: hosterm (Tauri v2 + WebView2 + Rust backend, xterm UI).

## What MEKL asked
Make hosterm's Google Sign-in REAL (currently mock). Any user can log in with
their own Google account, authorize Drive access, and use it to BACK UP and
RESTORE all their hosterm data to/from their own Google Drive.

## LOCKED DECISIONS (from MEKL — do not re-litigate)
1. **OAuth flow:** PKCE loopback (Authorization Code + PKCE, `S256`), NO client
   secret relied upon as secret. Desktop app. Loopback redirect `http://127.0.0.1:<ephemeral-port>`.
2. **Drive scope:** `https://www.googleapis.com/auth/drive.file` ONLY (app sees
   only files it created). NOT drive.appdata, NOT full drive.
3. **Backup contents (source of truth, real paths in code):**
   - `~/.ssh/config` — host list (env override `HOSTERM_CONFIG`; see `config_path()` lib.rs:34)
   - `~/.ssh/hosterm.secrets` — password sidecar `alias=pw` (`secrets_path()` lib.rs:231)
   - `settings.json` under app data dir (`settings_path()` lib.rs:740, `data_dir()` 731)
   - SSH PRIVATE KEYS referenced by `IdentityFile` in the config (files under ~/.ssh)
4. **Security (NON-NEGOTIABLE):** backup contains private keys + passwords, so it
   MUST be client-side encrypted with a user passphrase BEFORE upload
   (zero-knowledge). Google / anyone with Drive access sees only ciphertext.
   Decryption happens only locally with the passphrase; hosterm never uploads the
   passphrase or derived key, and never stores the passphrase at rest.

## Current state (confirmed by reading code)
- Google UI is 100% MOCK: `MOCK_GOOGLE_USER = { name: "Morgan Reyes" }`,
  `googleMockSignedIn` (main.ts:140-141), templates `#google-chip-signed-in/out`,
  popover text literally "UI-only mock. No Google session is connected."
- Backend has NO oauth/http-client/token store/Drive code. No `reqwest`, no
  `google`/`drive`/`oauth` deps in Cargo.toml. This is greenfield backend work.
- Secrets today live in a PLAINTEXT file `~/.ssh/hosterm.secrets` (`alias=pw`
  lines) — this is exactly why the backup MUST be encrypted before leaving the
  machine.

## Architecture (proposed; architect owns SDD, lead owns final stack/versions)

### A. OAuth PKCE loopback (backend, Rust)
- On "Sign in": backend generates `code_verifier` (43-128 char, high entropy) +
  `code_challenge = BASE64URL(SHA256(verifier))`, a random `state`, binds a
  loopback TCP listener on `127.0.0.1:0` (ephemeral), builds the Google auth URL
  (client_id, scope drive.file, redirect_uri=loopback, code_challenge, state,
  `access_type=offline`, `prompt=consent` to get a refresh_token), and opens it
  in the user's browser (tauri opener / `open` crate).
- The loopback server waits for the single redirect, verifies `state`, extracts
  `code`, then POSTs to the token endpoint with `code_verifier` to exchange for
  `access_token` + `refresh_token` + expiry.
- SECURITY: validate `state` (CSRF), accept exactly one request, bind to 127.0.0.1
  only, short timeout, render a minimal "you can close this tab" page.
- TOKEN STORAGE: refresh_token is a long-lived secret. Store it via the OS
  keyring (keyring crate: Secret Service on Linux / Keychain / Credential Mgr).
  Do NOT write refresh_token to a plaintext file. access_token kept in memory.
- Config: official builds inject the Desktop OAuth client ID and generated
  installed-app value from GitHub Actions secrets. Neither value is committed to
  source, docs, artifacts, or logs. The generated value is sent only during token
  exchange and refresh. PKCE S256 remains mandatory and is the authorization-code
  proof.
  **CONSOLE PREREQUISITES still owned by MEKL (verify before AC1 e2e):** Drive API
  enabled on the OAuth client's Google Cloud project; OAuth consent screen configured with scope
  `.../auth/drive.file`; MEKL's Google email added as a Test user (while app is in
  "Testing"). Without these, sign-in fails at Google even with a valid Client ID.

### B. Drive client (backend, Rust, reqwest + rustls)
- `drive.file` scope. Multipart upload (metadata + media) to create the backup
  object; list/get to find the latest backup for restore; update to overwrite.
- Name backups e.g. `hosterm-backup-<hostname>-<UTC-iso>.htbk`. Keep a small
  manifest inside the encrypted blob; also set Drive `appProperties` with a
  non-sensitive schema version + created_at for listing without decrypting.
- Handle 401 → refresh access_token via refresh_token; surface re-auth if the
  refresh_token is revoked.

### C. Backup archive format (encrypted, zero-knowledge)
- Collect the 4 sources into an in-memory manifest (tar or a simple JSON bundle:
  `{schema, created_at, host, files:[{path_rel, mode, content_b64}], secrets:{...}}`).
  Resolve `IdentityFile` keys from the config and include the private key files.
- ENCRYPT client-side:
  - KDF: Argon2id (or scrypt) over the user passphrase → 256-bit key. Store KDF
    params + random salt in a cleartext header (params are not secret).
  - AEAD: XChaCha20-Poly1305 (or AES-256-GCM) with a random nonce; authenticate
    the header. Output `.htbk` = `magic || version || kdf_params || salt || nonce || ciphertext || tag`.
  - The passphrase and derived key never leave the process; never logged.
- RESTORE: download blob → prompt passphrase → derive key → verify tag → decrypt →
  write files back to their real paths, with a PRE-RESTORE local safety backup of
  anything about to be overwritten (so a bad restore is recoverable). Never
  clobber ~/.ssh/config without a timestamped local copy first.

### D. Frontend (TS)
- Replace the mock chip/popover with real state: signed-out → "Sign in with
  Google" (invokes backend oauth), signed-in → real name/email/avatar from the
  Google userinfo endpoint (or id_token claims).
- Add Backup / Restore actions (passphrase prompt dialog — in-app, never native;
  mask input; confirm passphrase on backup; warn on restore that it overwrites).
- Progress + clear error states (auth failed, network, wrong passphrase = auth
  tag mismatch, revoked token).

## Acceptance criteria
- AC1 (auth): Real PKCE loopback sign-in works end-to-end with a real Google test
  user; `state` verified; refresh_token stored in OS keyring (NOT plaintext);
  access_token refresh works after expiry. Sign-out clears tokens.
- AC2 (scope): Only `drive.file` requested. App cannot see files it didn't create.
- AC3 (backup): Backup gathers config + secrets + settings + referenced private
  keys, encrypts client-side with the passphrase, uploads one `.htbk` to the
  user's Drive. The uploaded bytes are ciphertext (prove: cannot grep a known
  host alias / key material in the uploaded blob).
- AC4 (restore): Restore downloads the blob, decrypts with the correct passphrase,
  restores files to real paths; WRONG passphrase fails safe (auth tag mismatch,
  no partial write); a pre-restore local backup of overwritten files exists.
- AC5 (security): passphrase/derived key never written to disk or logs; refresh
  token only in keyring; no token/passphrase secret in plaintext; loopback bound to
  127.0.0.1 only; CSRF state enforced. Dependencies pinned, vetted (no typosquat).
- AC6 (regression): build 0 TS + cargo green; existing smoke stays green.

## Verifier gate (qa + security) — MANDATORY, security is heavy here
- security: audit OAuth (PKCE correctness, state/CSRF, redirect binding, token
  storage = keyring not file, scope minimalism), audit crypto (KDF params, AEAD
  choice, nonce uniqueness, no key/passphrase leakage, authenticated header),
  audit restore path (no clobber without local backup, fail-safe on bad passphrase),
  audit new deps (pinned versions, reputable crates, no typosquat).
- qa: e2e the auth flow against a real test user; prove ciphertext-at-rest
  (grep blob for plaintext markers = 0 hits); round-trip backup→restore equality;
  wrong-passphrase fails safe; extend smoke where feasible (mock the Drive HTTP).
- Direct-push to origin/main (completion_contract = local-only). Build 0 TS,
  cargo green, smoke green.

## BLOCKER (hard dependency on MEKL)
- OAuth **Client ID** from Google Cloud Console (Desktop app, Drive API enabled,
  consent screen with drive.file scope, MEKL's email as test user). Crew CANNOT
  complete AC1 end-to-end without it. Until then, implement everything behind the
  Client ID and verify with unit/integration + a documented manual-auth test.
