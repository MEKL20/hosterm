# Manual Google OAuth sign-in (gdrive-backup v1)

This procedure lets MEKL exercise the **real** end-to-end PKCE loopback sign-in
(PRD AC1) once the Google Cloud Console prerequisites are confirmed. It is the
documented substitute for automated AC1 coverage, which is **console-gated** and
MUST NOT be claimed as passing by any agent until MEKL confirms the Console
state below and runs this procedure.

Grounding: `docs/tsd/gdrive-backup-v1.md` §1,§3,§7,§8;
`docs/prd/discovery-gdrive-backup-v1.md` §A. Backend: `src-tauri/src/oauth.rs`.

## What is built (no console needed)

- PKCE S256 generation, auth-URL build, loopback catcher, token
  exchange/refresh, userinfo, and keyring storage are implemented and unit
  tested (`cargo test --lib oauth` → 18 green, HTTP behind an injectable seam,
  no real network in tests).
- The three IPC commands exist and are registered:
  `google_sign_in`, `google_sign_out`, `google_account`.

## Console prerequisites (MEKL confirms — AC1 gate)

The flow runs against the **public** build-time client id
`737778700861-dhtn1sc9miiqtsmtms3tmc8tk7jj3a80.apps.googleusercontent.com`
(project `737778700861`). Before a live sign-in can succeed, confirm in the
Google Cloud Console for that project:

1. **Drive API enabled** on project `737778700861`.
2. **OAuth consent screen** configured with the scope
   `https://www.googleapis.com/auth/drive.file` (plus `openid email profile` for
   the account chip). Publishing status may stay **Testing**.
3. **MEKL's Google account is listed as a Test user** (required while the app is
   in Testing).
4. The OAuth client is of type **Desktop app** (installed-app PKCE, loopback
   redirect `http://127.0.0.1:<ephemeral>` — Google allows any loopback port for
   desktop clients, so no fixed redirect URI needs registering).

If PKCE-only (no client secret) is rejected by the token endpoint for this
client type, that is the signal the client was created as **Web** rather than
**Desktop**; recreate it as a Desktop client. The code tries PKCE-only first by
design (TSD LOCKED decision).

## Manual sign-in procedure

With a `cargo tauri dev` or an installed build running on MEKL's machine:

1. In the app, trigger **Sign in with Google** (FE-AUTH wires the chip; until
   then invoke `google_sign_in` directly from the dev console / a test harness).
2. The backend:
   - generates a PKCE verifier/challenge + random `state`,
   - binds a single-shot loopback listener on `127.0.0.1:0`,
   - opens the system browser at the Google authorization URL
     (`access_type=offline`, `prompt=consent`).
3. In the browser, pick MEKL's test-user account and grant the `drive.file`
   consent. Google redirects to `http://127.0.0.1:<port>/?code=...&state=...`.
4. The loopback catcher accepts **exactly one** request, validates `state`
   (CSRF — a mismatch aborts with "state mismatch … rejected"), shows a minimal
   "you can close this tab" page, and hands the code back.
5. The backend exchanges the code (PKCE, no secret) for tokens, stores the
   **refresh_token in the OS keyring** (service `hosterm`, account
   `google-refresh-token`), keeps the access token in memory only, and returns
   `{name,email,picture}` from userinfo.

### Expected results to verify

- `google_sign_in()` returns a `GoogleAccount` with MEKL's name/email/picture.
- The refresh_token is in the keyring, NOT in any file:
  - Linux Secret Service: `secret-tool search service hosterm` (or Seahorse)
    shows an entry for account `google-refresh-token`; `grep -r` across the home
    dir finds the token in **no** plaintext file.
- `google_account()` on a fresh app start returns `Some(account)` (it reads the
  keyring, refreshes, and fetches userinfo) — proving refresh works.
- `google_sign_out()` deletes the keyring entry; a subsequent `google_account()`
  returns `None`.

## Security checks during the manual run

- The access token appears in **no** log line (it is never logged by design).
- The loopback listener binds `127.0.0.1` only (never `0.0.0.0`); a second
  connection to the same port is not served (single-shot).
- A tampered `state` on the redirect is rejected before any token exchange.
- The public client id is a build-time constant; it is **not** written to the
  keyring or the secrets sidecar.

## Status

**AC1 (live end-to-end sign-in) is CONSOLE-GATED on MEKL.** Do not mark AC1 as
passed from CI or from an agent run. Everything else (PKCE correctness, URL
shape, CSRF guard, loopback single-shot, token exchange/refresh/userinfo behind
the HTTP seam, keyring read/write/delete) is covered by `cargo test --lib`.
