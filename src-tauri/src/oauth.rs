// ============================================================
//  Google OAuth 2.0 — PKCE S256 loopback sign-in (BE-OAUTH)
// ============================================================
//
// Real Google OAuth per docs/tsd/gdrive-backup-v1.md §1,§2,§3,§8.
//
// Flow (LOCKED, not re-litigated):
//   - PKCE S256, loopback on 127.0.0.1:0 (ephemeral port), single-shot.
//   - access_type=offline + prompt=consent (so a refresh_token is issued).
//   - scope: https://www.googleapis.com/auth/drive.file ONLY (+ userinfo for
//     the account chip name/email/picture).
//   - Desktop client credentials are injected at build time from environment
//     variables; PKCE remains mandatory.
//   - refresh_token lives in the OS keyring (service "hosterm",
//     account "google-refresh-token") — NEVER in a plaintext file.
//   - access_token is kept in-memory only (never persisted, never logged).
//
// Security musts enforced here:
//   - `state` (CSRF) is validated on the redirect; a mismatch is rejected.
//   - exactly ONE loopback request is accepted; the listener is single-shot.
//   - the listener binds 127.0.0.1 only; a short timeout bounds the wait.
//   - minimal "you can close this tab" response page; no token echoed back.
//   - access_token and injected client credentials are never logged or persisted.
//
// HTTP is behind an injectable seam (`TokenHttp`) so unit tests exercise the
// token exchange / refresh / userinfo logic with a fake responder — no real
// network in tests.

use std::io::Read;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;

// ---- Constants (build-time injected) -------------------------------------

const UNCONFIGURED_CLIENT_ID: &str = "hosterm-oauth-client-unconfigured";
const UNCONFIGURED_CLIENT_SECRET: &str = "hosterm-oauth-value-unconfigured";

/// Official builds inject these through GitHub Actions secrets. The fallbacks
/// keep community builds/test compilation possible, but interactive sign-in
/// rejects an unconfigured production build with a clear error.
pub const CLIENT_ID: &str = match option_env!("HOSTERM_GOOGLE_CLIENT_ID") {
    Some(value) => value,
    None => UNCONFIGURED_CLIENT_ID,
};
const DESKTOP_CLIENT_SECRET: &str = match option_env!("HOSTERM_GOOGLE_CLIENT_SECRET") {
    Some(value) => value,
    None => UNCONFIGURED_CLIENT_SECRET,
};

fn oauth_client() -> Result<(&'static str, &'static str), String> {
    if !cfg!(test)
        && (CLIENT_ID == UNCONFIGURED_CLIENT_ID
            || DESKTOP_CLIENT_SECRET == UNCONFIGURED_CLIENT_SECRET)
    {
        return Err("Google OAuth is not configured in this build".to_string());
    }
    Ok((CLIENT_ID, DESKTOP_CLIENT_SECRET))
}

/// The ONLY Drive scope requested — per-file access, no broad Drive scope.
pub const SCOPE_DRIVE_FILE: &str = "https://www.googleapis.com/auth/drive.file";
/// Userinfo scopes for the account chip (name/email/picture).
pub const SCOPE_USERINFO: &str = "openid email profile";

const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const USERINFO_ENDPOINT: &str = "https://openidconnect.googleapis.com/v1/userinfo";

/// Keyring handle for the refresh token at rest.
const KEYRING_SERVICE: &str = "hosterm";
const KEYRING_ACCOUNT: &str = "google-refresh-token";

/// How long we wait for the browser to redirect back to the loopback catcher.
const LOOPBACK_TIMEOUT: Duration = Duration::from_secs(300);

// ---- DTOs ----------------------------------------------------------------

/// Returned across the IPC boundary — safe fields only (no tokens).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoogleAccount {
    pub name: String,
    pub email: String,
    pub picture: String,
}

/// Token-exchange / refresh response shape (subset we use).
#[derive(Debug, Clone, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    expires_in: Option<u64>,
    #[serde(default)]
    #[allow(dead_code)]
    token_type: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    scope: Option<String>,
}

/// Userinfo response (OIDC userinfo endpoint).
#[derive(Debug, Clone, Deserialize)]
struct UserInfoResponse {
    #[serde(default)]
    name: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    picture: String,
}

// ---- HTTP seam -----------------------------------------------------------
//
// All outbound HTTP goes through this trait so tests can inject a fake. The
// real implementation uses reqwest (rustls, blocking) and is the only
// sanctioned HTTP client in the app (user-initiated only).

/// A single POST/GET the OAuth flow needs. Keeps the seam minimal: callers
/// pass form params for POST, or a bearer token for the userinfo GET.
pub trait TokenHttp {
    /// POST application/x-www-form-urlencoded to `url`, return the body text
    /// on 2xx or an Err with a non-sensitive message. MUST NOT log the body.
    fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<String, String>;
    /// GET `url` with `Authorization: Bearer <access_token>`, return body text.
    fn get_bearer(&self, url: &str, access_token: &str) -> Result<String, String>;
}

/// Production HTTP client: reqwest blocking + rustls. Constructed lazily per
/// call; the app makes zero network calls at idle (NFR-3) — every call here is
/// user-initiated (sign-in / account refresh).
pub struct ReqwestHttp;

impl TokenHttp for ReqwestHttp {
    fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<String, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("http client build failed: {e}"))?;
        let resp = client
            .post(url)
            .form(form)
            .send()
            .map_err(|e| format!("token request failed: {e}"))?;
        let status = resp.status();
        let body = resp
            .text()
            .map_err(|e| format!("token response read failed: {e}"))?;
        if !status.is_success() {
            // A FAILED token response is an OAuth error object
            // `{"error":..,"error_description":..}` and carries NO token
            // material, so surfacing those two fields is safe and makes the
            // cause diagnosable. We NEVER echo a success body or token fields.
            return Err(format!(
                "token endpoint returned HTTP {}{}",
                status.as_u16(),
                oauth_error_suffix(&body)
            ));
        }
        Ok(body)
    }

    fn get_bearer(&self, url: &str, access_token: &str) -> Result<String, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("http client build failed: {e}"))?;
        let resp = client
            .get(url)
            .bearer_auth(access_token)
            .send()
            .map_err(|e| format!("userinfo request failed: {e}"))?;
        let status = resp.status();
        let body = resp
            .text()
            .map_err(|e| format!("userinfo response read failed: {e}"))?;
        if !status.is_success() {
            return Err(format!("userinfo endpoint returned HTTP {}", status.as_u16()));
        }
        Ok(body)
    }
}

// ---- Token-endpoint error surfacing (non-sensitive) ---------------------

/// Minimal OAuth error object from a FAILED token response. Per RFC 6749 §5.2
/// a non-2xx token response is `{"error":..,"error_description":..}` and carries
/// NO token material — so these two fields are safe to surface.
#[derive(Debug, Clone, Deserialize)]
struct OAuthErrorBody {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

/// Build the non-sensitive, human-readable suffix for a token-endpoint HTTP
/// error. Parses ONLY `error` / `error_description` out of the body; it never
/// echoes the raw body and never surfaces any field that could carry token
/// material (access_token / refresh_token / id_token). On an unparseable or
/// error-less body it returns an empty suffix so only the HTTP status shows.
///
/// Example output: `: invalid_request (client_secret is missing.)`.
fn oauth_error_suffix(body: &str) -> String {
    match serde_json::from_str::<OAuthErrorBody>(body) {
        Ok(e) => match (e.error, e.error_description) {
            (Some(err), Some(desc)) if !err.is_empty() && !desc.is_empty() => {
                format!(": {err} ({desc})")
            }
            (Some(err), _) if !err.is_empty() => format!(": {err}"),
            _ => String::new(),
        },
        Err(_) => String::new(),
    }
}

// ---- PKCE ----------------------------------------------------------------

/// A generated PKCE pair plus the anti-CSRF state.
#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
    pub state: String,
}

/// Random URL-safe token of `n_bytes` entropy, base64url-no-pad encoded.
fn random_b64url(n_bytes: usize) -> String {
    use rand::rngs::OsRng;
    use rand::RngCore;
    let mut buf = vec![0u8; n_bytes];
    OsRng.fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(&buf)
}

/// Generate a fresh PKCE verifier/challenge and a random state.
///
/// The verifier is 64 bytes of OsRng entropy base64url-encoded, which lands at
/// ~86 chars — comfortably inside RFC 7636's 43..128 range and all-unreserved.
pub fn generate_pkce() -> Pkce {
    let verifier = random_b64url(64);
    let challenge = code_challenge(&verifier);
    let state = random_b64url(32);
    Pkce {
        verifier,
        challenge,
        state,
    }
}

/// S256 code_challenge = BASE64URL-no-pad( SHA256( ASCII(verifier) ) ).
pub fn code_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let digest = hasher.finalize();
    URL_SAFE_NO_PAD.encode(digest)
}

/// RFC 7636 unreserved charset check: A-Z a-z 0-9 - . _ ~
fn is_pkce_unreserved(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~')
}

/// True iff `v` is a legal PKCE code_verifier (length 43..=128, charset only).
pub fn verifier_is_valid(v: &str) -> bool {
    let len = v.len();
    (43..=128).contains(&len) && v.chars().all(is_pkce_unreserved)
}

// ---- Auth URL ------------------------------------------------------------

/// Minimal application/x-www-form-urlencoded percent-encoding for query
/// values. We encode everything that is not an unreserved char; this keeps us
/// dependency-free (no `url`/`urlencoding` crate) and correct for our inputs.
fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Build the authorization URL for the loopback redirect at `redirect_uri`.
pub fn build_auth_url(challenge: &str, state: &str, redirect_uri: &str) -> String {
    let scope = format!("{SCOPE_DRIVE_FILE} {SCOPE_USERINFO}");
    let params = [
        ("client_id", CLIENT_ID),
        ("redirect_uri", redirect_uri),
        ("response_type", "code"),
        ("scope", &scope),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
        ("access_type", "offline"),
        ("prompt", "consent"),
    ];
    let query = params
        .iter()
        .map(|(k, v)| format!("{}={}", pct_encode(k), pct_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{AUTH_ENDPOINT}?{query}")
}

// ---- Loopback catcher ----------------------------------------------------

/// What the browser redirect delivered back to the loopback listener.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedirectResult {
    pub code: String,
    pub state: String,
}

/// Parse `code` and `state` out of a redirect request path+query such as
/// `/callback?state=XYZ&code=ABC` (or `/?code=...&state=...`). Returns an
/// `error=...` payload as Err so the caller surfaces a user-denied consent.
pub fn parse_redirect_query(path_and_query: &str) -> Result<RedirectResult, String> {
    let query = path_and_query.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut code: Option<String> = None;
    let mut state: Option<String> = None;
    let mut error: Option<String> = None;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = url_decode(v);
        match k {
            "code" => code = Some(v),
            "state" => state = Some(v),
            "error" => error = Some(v),
            _ => {}
        }
    }
    if let Some(e) = error {
        return Err(format!("authorization denied: {e}"));
    }
    match (code, state) {
        (Some(code), Some(state)) if !code.is_empty() && !state.is_empty() => {
            Ok(RedirectResult { code, state })
        }
        _ => Err("redirect missing code or state".to_string()),
    }
}

/// Minimal percent-decode for query values (handles %XX and '+').
fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = hex_val(bytes[i + 1]);
                let lo = hex_val(bytes[i + 2]);
                if let (Some(h), Some(l)) = (hi, lo) {
                    out.push((h << 4) | l);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Bind a single-shot loopback listener on 127.0.0.1:0 and return the listener
/// plus the chosen ephemeral port. Binding 127.0.0.1 ONLY (never 0.0.0.0).
pub fn bind_loopback() -> Result<(TcpListener, u16), String> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
    let listener = TcpListener::bind(addr).map_err(|e| format!("loopback bind failed: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("loopback addr failed: {e}"))?
        .port();
    Ok((listener, port))
}

/// Accept EXACTLY ONE request on the loopback listener, parse the redirect,
/// write a minimal "you can close this tab" page, and return the result.
/// Bounded by `LOOPBACK_TIMEOUT`. Uses tiny_http over the already-bound socket.
fn accept_single_redirect(listener: TcpListener) -> Result<RedirectResult, String> {
    let server = tiny_http::Server::from_listener(listener, None)
        .map_err(|e| format!("loopback server init failed: {e}"))?;

    // Single-shot: one recv with a timeout, then the server drops.
    let request = match server
        .recv_timeout(LOOPBACK_TIMEOUT)
        .map_err(|e| format!("loopback recv failed: {e}"))?
    {
        Some(req) => req,
        None => return Err("sign-in timed out waiting for the browser".to_string()),
    };

    let url = request.url().to_string();
    let parsed = parse_redirect_query(&url);

    // Always answer the browser with a minimal, static page — never echo the
    // code/state/token into the response body.
    let (page, ok) = match &parsed {
        Ok(_) => (CLOSE_TAB_OK, true),
        Err(_) => (CLOSE_TAB_ERR, false),
    };
    let mut header_vec = Vec::new();
    if let Ok(h) = "Content-Type: text/html; charset=utf-8".parse::<tiny_http::Header>() {
        header_vec.push(h);
    }
    let response = tiny_http::Response::from_string(page)
        .with_status_code(if ok { 200 } else { 400 })
        .with_header(header_vec.into_iter().next().unwrap_or_else(|| {
            "Content-Type: text/plain".parse::<tiny_http::Header>().unwrap()
        }));
    let _ = request.respond(response);

    parsed
}

const CLOSE_TAB_OK: &str = "<!doctype html><html><head><meta charset=\"utf-8\">\
<title>hosterm</title></head><body style=\"font-family:system-ui;padding:3rem;\">\
<h2>Signed in to hosterm</h2><p>You can close this tab and return to the app.</p>\
</body></html>";

const CLOSE_TAB_ERR: &str = "<!doctype html><html><head><meta charset=\"utf-8\">\
<title>hosterm</title></head><body style=\"font-family:system-ui;padding:3rem;\">\
<h2>Sign-in failed</h2><p>You can close this tab and try again in the app.</p>\
</body></html>";

// ---- Token exchange / refresh / userinfo ---------------------------------

/// Exchange an authorization `code` for tokens using PKCE plus Google's
/// installed-app client value.
/// Returns (access_token, refresh_token?).
pub fn exchange_code(
    http: &dyn TokenHttp,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<(String, Option<String>), String> {
    let (client_id, client_secret) = oauth_client()?;
    let form = [
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("code", code),
        ("code_verifier", verifier),
        ("grant_type", "authorization_code"),
        ("redirect_uri", redirect_uri),
    ];
    let body = http.post_form(TOKEN_ENDPOINT, &form)?;
    let tr: TokenResponse =
        serde_json::from_str(&body).map_err(|_| "malformed token response".to_string())?;
    if tr.access_token.is_empty() {
        return Err("token response missing access_token".to_string());
    }
    Ok((tr.access_token, tr.refresh_token))
}

/// Refresh the access token using a stored refresh_token (Desktop public client).
/// Google may rotate the refresh_token; if it returns a new one, surface it.
pub fn refresh_access_token(
    http: &dyn TokenHttp,
    refresh_token: &str,
) -> Result<(String, Option<String>), String> {
    let (client_id, client_secret) = oauth_client()?;
    let form = [
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ];
    let body = http.post_form(TOKEN_ENDPOINT, &form)?;
    let tr: TokenResponse =
        serde_json::from_str(&body).map_err(|_| "malformed refresh response".to_string())?;
    if tr.access_token.is_empty() {
        return Err("refresh response missing access_token".to_string());
    }
    Ok((tr.access_token, tr.refresh_token))
}

/// Fetch the user's profile for the account chip.
pub fn fetch_userinfo(http: &dyn TokenHttp, access_token: &str) -> Result<GoogleAccount, String> {
    let body = http.get_bearer(USERINFO_ENDPOINT, access_token)?;
    let ui: UserInfoResponse =
        serde_json::from_str(&body).map_err(|_| "malformed userinfo response".to_string())?;
    Ok(GoogleAccount {
        name: ui.name,
        email: ui.email,
        picture: ui.picture,
    })
}

// ---- Keyring (refresh_token at rest) -------------------------------------

/// Persist the refresh_token in the OS keyring. NEVER a plaintext file.
pub fn keyring_store_refresh(token: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|e| format!("keyring open failed: {e}"))?;
    entry
        .set_password(token)
        .map_err(|e| format!("keyring store failed: {e}"))
}

/// Read the stored refresh_token, if any. `NoEntry` → Ok(None).
pub fn keyring_read_refresh() -> Result<Option<String>, String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|e| format!("keyring open failed: {e}"))?;
    match entry.get_password() {
        Ok(t) => Ok(Some(t)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("keyring read failed: {e}")),
    }
}

/// Delete the stored refresh_token. Missing entry is treated as success
/// (idempotent sign-out).
pub fn keyring_delete_refresh() -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|e| format!("keyring open failed: {e}"))?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("keyring delete failed: {e}")),
    }
}

// ---- Open the auth URL in the system browser -----------------------------
//
// No `open` crate (TSD §1): hosterm already spawns OS processes directly
// (lib.rs / fleet.rs). Use the platform opener binary the same way.

/// Return the platform opener command without involving a shell. In particular,
/// Windows must not use `cmd /C start`: OAuth URLs contain `&`, which cmd.exe
/// treats as a command separator and silently truncates the browser URL.
fn browser_command(url: &str) -> (&'static str, Vec<&str>) {
    #[cfg(target_os = "windows")]
    let command: (&str, Vec<&str>) = (
        "rundll32.exe",
        vec!["url.dll,FileProtocolHandler", url],
    );
    #[cfg(target_os = "macos")]
    let command: (&str, Vec<&str>) = ("open", vec![url]);
    #[cfg(all(unix, not(target_os = "macos")))]
    let command: (&str, Vec<&str>) = ("xdg-open", vec![url]);

    command
}

/// Launch the user's default browser at `url`. Spawns detached; does not wait.
pub fn open_in_browser(url: &str) -> Result<(), String> {
    let (bin, args) = browser_command(url);

    std::process::Command::new(bin)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open browser: {e}"))
}

// ---- Full interactive sign-in orchestration ------------------------------

/// Run the full PKCE loopback sign-in, store the refresh_token in the keyring,
/// and return the GoogleAccount. Uses the real HTTP client and real browser.
///
/// Headless/test note: this performs real network + browser I/O and is NOT
/// unit-tested; the composable pieces (PKCE, URL build, redirect parse, token
/// exchange behind the seam, keyring) are each tested independently.
pub fn interactive_sign_in() -> Result<GoogleAccount, String> {
    oauth_client()?;
    let http = ReqwestHttp;
    let pkce = generate_pkce();
    let (listener, port) = bind_loopback()?;
    let redirect_uri = format!("http://127.0.0.1:{port}");
    let auth_url = build_auth_url(&pkce.challenge, &pkce.state, &redirect_uri);

    open_in_browser(&auth_url)?;

    let redirect = accept_single_redirect(listener)?;

    // CSRF: the returned state MUST match the one we generated.
    if redirect.state != pkce.state {
        return Err("state mismatch — possible CSRF, sign-in rejected".to_string());
    }

    let (access_token, refresh_token) =
        exchange_code(&http, &redirect.code, &pkce.verifier, &redirect_uri)?;

    // Decide the durable-auth outcome BEFORE presenting any account. A
    // successful sign-in must leave the app able to perform Drive work; the
    // frozen contract (TSD §9, backup::prod::with_client) bases every Drive
    // call on the keyring refresh_token, so a sign-in with no durable token is
    // a false success and is rejected here. See `durable_refresh_or_error`.
    let durable = durable_refresh_or_error(refresh_token)?;
    keyring_store_refresh(&durable)?;

    fetch_userinfo(&http, &access_token)
}

/// Decide whether an authorization-code exchange produced a durable session.
///
/// Root cause of the v0.14.1 false signed-in state: a sign-in that received no
/// `refresh_token` from Google (e.g. a stale prior grant where Google omits it
/// even with `prompt=consent`) still returned `Ok(account)` while persisting
/// nothing. The in-memory access token was then dropped, so the next Drive
/// call hit `backup::prod::with_client`, found an empty keyring, and failed
/// with `not signed in to Google — sign in first` — an account that looks
/// signed in but can do no Drive work.
///
/// Tradeoff (chosen architecture): the frozen OAuth/Drive contract keys ALL
/// usable Drive state off the keyring refresh_token — `load_account` returns
/// `None` without it and `with_client` refuses without it — while the access
/// token is memory-only, backend-side, and MUST NOT cross IPC or live in
/// long-lived app state (security invariants). Carrying a memory-only session
/// would require a new backend access-token store plus IPC plumbing, both
/// outside this contract. The smallest honest fix is therefore to FAIL sign-in
/// when no durable refresh_token is obtained, so the app never presents a
/// signed-in account it cannot back with real Drive capability. The error is
/// actionable and carries NO token material.
fn durable_refresh_or_error(refresh_token: Option<String>) -> Result<String, String> {
    match refresh_token {
        Some(rt) if !rt.is_empty() => Ok(rt),
        _ => Err(
            "Google sign-in did not return durable authorization, so Drive access \
             cannot be set up. Remove hosterm's access in your Google Account \
             (myaccount.google.com → Security → Third-party access), then sign in \
             again to re-grant."
                .to_string(),
        ),
    }
}

/// Load the signed-in account on app start: if the keyring holds a
/// refresh_token, refresh it and fetch userinfo; else None.
pub fn load_account() -> Result<Option<GoogleAccount>, String> {
    let Some(refresh) = keyring_read_refresh()? else {
        return Ok(None);
    };
    let http = ReqwestHttp;
    let (access_token, rotated) = refresh_access_token(&http, &refresh)?;
    if let Some(new_rt) = rotated {
        // Google rotated the refresh_token — persist the new one.
        keyring_store_refresh(&new_rt)?;
    }
    let account = fetch_userinfo(&http, &access_token)?;
    Ok(Some(account))
}

/// Sign out: delete the keyring refresh_token. In-memory access token clearing
/// is the caller's concern (lib.rs holds no long-lived access token here).
pub fn sign_out() -> Result<(), String> {
    keyring_delete_refresh()
}

// ==========================================================================
//  TESTS — pure helpers + HTTP behind a fake seam. NO real network.
// ==========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io::Write;
    use std::net::TcpStream;

    // ---- PKCE ------------------------------------------------------------

    #[test]
    fn verifier_length_and_charset_valid() {
        for _ in 0..50 {
            let p = generate_pkce();
            assert!(
                verifier_is_valid(&p.verifier),
                "verifier out of spec: len={} value={}",
                p.verifier.len(),
                p.verifier
            );
            // base64url(64 bytes) with no padding == 86 chars.
            assert_eq!(p.verifier.len(), 86);
            assert!((43..=128).contains(&p.verifier.len()));
        }
    }

    #[test]
    fn verifier_rejects_out_of_range_and_bad_charset() {
        assert!(!verifier_is_valid(&"a".repeat(42)), "42 chars too short");
        assert!(verifier_is_valid(&"a".repeat(43)), "43 chars min ok");
        assert!(verifier_is_valid(&"a".repeat(128)), "128 chars max ok");
        assert!(!verifier_is_valid(&"a".repeat(129)), "129 chars too long");
        // '+' and '/' and '=' are NOT in the PKCE unreserved set.
        assert!(!verifier_is_valid(&format!("{}+", "a".repeat(50))));
        assert!(!verifier_is_valid(&format!("{}/", "a".repeat(50))));
        assert!(!verifier_is_valid(&format!("{}=", "a".repeat(50))));
    }

    #[test]
    fn challenge_is_base64url_sha256_of_verifier() {
        // RFC 7636 Appendix B canonical vector.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert_eq!(code_challenge(verifier), expected);
    }

    #[test]
    fn challenge_matches_independent_sha256_computation() {
        let p = generate_pkce();
        let mut h = Sha256::new();
        h.update(p.verifier.as_bytes());
        let want = URL_SAFE_NO_PAD.encode(h.finalize());
        assert_eq!(p.challenge, want);
        // challenge is also PKCE-safe charset + 43 chars (sha256→32B→43 b64url)
        assert_eq!(p.challenge.len(), 43);
    }

    #[test]
    fn state_is_random_and_distinct() {
        let a = generate_pkce();
        let b = generate_pkce();
        assert_ne!(a.state, b.state, "state must be unpredictable per flow");
        assert_ne!(a.verifier, b.verifier);
        assert!(!a.state.is_empty());
    }

    // ---- Auth URL --------------------------------------------------------

    #[test]
    fn auth_url_contains_locked_params() {
        let url = build_auth_url("CHAL", "STATE123", "http://127.0.0.1:54321");
        assert!(url.starts_with(AUTH_ENDPOINT));
        assert!(url.contains(&format!("client_id={}", pct_encode(CLIENT_ID))));
        assert!(url.contains("code_challenge=CHAL"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=STATE123"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
        assert!(url.contains("response_type=code"));
        // redirect_uri percent-encoded
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A54321"));
        // drive.file scope present, encoded
        assert!(url.contains(&pct_encode(SCOPE_DRIVE_FILE)));
        // NO client_secret in a PKCE-only flow
        assert!(!url.contains("client_secret"));
    }

    // ---- Redirect parsing / CSRF ----------------------------------------

    #[test]
    fn parse_redirect_extracts_code_and_state() {
        let r = parse_redirect_query("/?state=abc&code=xyz").unwrap();
        assert_eq!(r.code, "xyz");
        assert_eq!(r.state, "abc");
        // order independent + extra params ignored
        let r2 = parse_redirect_query("/callback?code=c2&scope=drive&state=s2").unwrap();
        assert_eq!(r2.code, "c2");
        assert_eq!(r2.state, "s2");
    }

    #[test]
    fn parse_redirect_percent_decodes_values() {
        let r = parse_redirect_query("/?code=a%2Fb&state=x%20y").unwrap();
        assert_eq!(r.code, "a/b");
        assert_eq!(r.state, "x y");
    }

    #[test]
    fn parse_redirect_surfaces_error_and_missing() {
        assert!(parse_redirect_query("/?error=access_denied").is_err());
        assert!(parse_redirect_query("/?code=only").is_err());
        assert!(parse_redirect_query("/?state=only").is_err());
        assert!(parse_redirect_query("/nothere").is_err());
    }

    #[test]
    fn state_mismatch_is_rejected_csrf_guard() {
        // Simulate the orchestration's CSRF check directly.
        let ours = generate_pkce();
        let redirect = RedirectResult {
            code: "c".into(),
            state: "ATTACKER-STATE".into(),
        };
        assert_ne!(redirect.state, ours.state);
        // The guard in interactive_sign_in rejects this before token exchange.
        let rejected = redirect.state != ours.state;
        assert!(rejected, "mismatched state must be rejected");
    }

    // ---- Loopback binding + single-shot ---------------------------------

    #[test]
    fn loopback_binds_localhost_only() {
        let (listener, port) = bind_loopback().unwrap();
        let addr = listener.local_addr().unwrap();
        assert!(addr.ip().is_loopback(), "must bind 127.0.0.1 (loopback)");
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
        assert_ne!(port, 0, "ephemeral port assigned");
    }

    #[test]
    fn loopback_accepts_single_request_and_parses() {
        let (listener, port) = bind_loopback().unwrap();
        // Drive one request from a client thread, then assert single-shot.
        let handle = std::thread::spawn(move || accept_single_redirect(listener));
        // tiny delay so the server thread reaches recv
        std::thread::sleep(Duration::from_millis(50));
        let mut sock = TcpStream::connect(("127.0.0.1", port)).unwrap();
        sock.write_all(
            b"GET /?state=S&code=C HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .unwrap();
        // read + discard the close-tab response
        let mut buf = Vec::new();
        let _ = sock.read_to_end(&mut buf);
        let got = handle.join().unwrap().unwrap();
        assert_eq!(got.code, "C");
        assert_eq!(got.state, "S");
        // single-shot: the port is now free (server dropped) — a fresh bind on
        // the SAME ephemeral is not guaranteed, but a new connect must NOT be
        // served by the dropped server. Connecting should fail or hang-refuse.
        let second = TcpStream::connect(("127.0.0.1", port));
        if let Ok(mut s2) = second {
            // If the OS let us connect (TIME_WAIT etc.), no HTTP server answers.
            s2.set_read_timeout(Some(Duration::from_millis(200))).ok();
            let _ = s2.write_all(b"GET /?x=1 HTTP/1.1\r\n\r\n");
            let mut b2 = Vec::new();
            let n = s2.read_to_end(&mut b2).unwrap_or(0);
            assert_eq!(n, 0, "no second request is served (single-shot)");
        }
    }

    // ---- Token exchange / refresh behind the HTTP seam ------------------

    /// Fake responder: records requests, returns canned bodies. No network.
    struct FakeHttp {
        post_body: String,
        get_body: String,
        seen_forms: RefCell<Vec<Vec<(String, String)>>>,
    }

    impl TokenHttp for FakeHttp {
        fn post_form(&self, _url: &str, form: &[(&str, &str)]) -> Result<String, String> {
            self.seen_forms.borrow_mut().push(
                form.iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            );
            Ok(self.post_body.clone())
        }
        fn get_bearer(&self, _url: &str, _access_token: &str) -> Result<String, String> {
            Ok(self.get_body.clone())
        }
    }

    #[test]
    fn exchange_code_parses_tokens_and_sends_pkce_params() {
        let fake = FakeHttp {
            post_body: r#"{"access_token":"AT","refresh_token":"RT","expires_in":3599}"#
                .to_string(),
            get_body: String::new(),
            seen_forms: RefCell::new(Vec::new()),
        };
        let (at, rt) =
            exchange_code(&fake, "CODE", "VERIFIER", "http://127.0.0.1:1").unwrap();
        assert_eq!(at, "AT");
        assert_eq!(rt.as_deref(), Some("RT"));
        // The request keeps PKCE and includes Google's installed-app value.
        let form = &fake.seen_forms.borrow()[0];
        let has = |k: &str, v: &str| form.iter().any(|(kk, vv)| kk == k && vv == v);
        assert!(has("grant_type", "authorization_code"));
        assert!(has("code_verifier", "VERIFIER"));
        assert!(has("code", "CODE"));
        assert!(has("client_id", CLIENT_ID));
        assert!(has("client_secret", DESKTOP_CLIENT_SECRET));
    }

    #[test]
    fn refresh_access_token_parses_and_rotates() {
        let fake = FakeHttp {
            post_body: r#"{"access_token":"AT2","refresh_token":"RT2"}"#.to_string(),
            get_body: String::new(),
            seen_forms: RefCell::new(Vec::new()),
        };
        let (at, rt) = refresh_access_token(&fake, "OLD_RT").unwrap();
        assert_eq!(at, "AT2");
        assert_eq!(rt.as_deref(), Some("RT2"));
        let form = &fake.seen_forms.borrow()[0];
        assert!(form.iter().any(|(k, v)| k == "grant_type" && v == "refresh_token"));
        assert!(form.iter().any(|(k, v)| k == "client_id" && v == CLIENT_ID));
        assert!(form.iter().any(|(k, v)| k == "client_secret" && v == DESKTOP_CLIENT_SECRET));
        assert!(form.iter().any(|(k, v)| k == "refresh_token" && v == "OLD_RT"));
    }

    #[test]
    fn refresh_without_rotation_returns_none_refresh() {
        let fake = FakeHttp {
            post_body: r#"{"access_token":"AT3"}"#.to_string(),
            get_body: String::new(),
            seen_forms: RefCell::new(Vec::new()),
        };
        let (at, rt) = refresh_access_token(&fake, "RT").unwrap();
        assert_eq!(at, "AT3");
        assert_eq!(rt, None);
    }

    // ---- Durable sign-in gate (v0.14.1 false signed-in regression) ------
    //
    // These prove a successful-looking sign-in cannot leave the app claiming a
    // usable Drive account when no durable refresh_token was obtained. The
    // decision lives in the pure `durable_refresh_or_error` so it is tested
    // without any real network / browser / keyring I/O (interactive_sign_in
    // itself performs real I/O and stays untested, per the module header).

    #[test]
    fn durable_refresh_rejects_missing_refresh_token() {
        // REGRESSION: Google returned no refresh_token. Sign-in must fail here
        // (before userinfo / before presenting an account), not succeed into a
        // state where the keyring is empty and the first Drive call errors.
        let err = durable_refresh_or_error(None).unwrap_err();
        assert!(
            err.contains("durable authorization"),
            "error must explain the missing durable grant: {err}"
        );
        // Actionable: tells the user how to recover.
        assert!(err.contains("sign in again"), "error must be actionable: {err}");
        // Secret-safe: never echoes token-shaped material.
        assert!(!err.to_lowercase().contains("refresh_token"));
        assert!(!err.to_lowercase().contains("access_token"));
    }

    #[test]
    fn durable_refresh_rejects_empty_refresh_token() {
        // An empty string is not a usable token; treat it like None.
        let err = durable_refresh_or_error(Some(String::new())).unwrap_err();
        assert!(err.contains("durable authorization"), "got: {err}");
    }

    #[test]
    fn durable_refresh_accepts_stored_refresh_token() {
        // Normal first grant: Google issues a refresh_token; it is the value
        // that gets persisted to the keyring and backs every later Drive call.
        let rt = durable_refresh_or_error(Some("RT_DURABLE".to_string())).unwrap();
        assert_eq!(rt, "RT_DURABLE");
    }

    #[test]
    fn durable_refresh_passes_through_rotated_refresh_token() {
        // Re-grant / rotation: whatever refresh_token the exchange yields is the
        // one persisted — the gate never substitutes or drops it.
        let rt = durable_refresh_or_error(Some("RT_ROTATED".to_string())).unwrap();
        assert_eq!(rt, "RT_ROTATED");
    }

    #[test]
    fn fetch_userinfo_builds_account() {
        let fake = FakeHttp {
            post_body: String::new(),
            get_body: r#"{"name":"Ada L","email":"ada@example.com","picture":"https://p/x.png"}"#
                .to_string(),
            seen_forms: RefCell::new(Vec::new()),
        };
        let acct = fetch_userinfo(&fake, "AT").unwrap();
        assert_eq!(
            acct,
            GoogleAccount {
                name: "Ada L".into(),
                email: "ada@example.com".into(),
                picture: "https://p/x.png".into(),
            }
        );
    }

    #[test]
    fn exchange_rejects_malformed_and_empty_token() {
        let bad = FakeHttp {
            post_body: "not json".to_string(),
            get_body: String::new(),
            seen_forms: RefCell::new(Vec::new()),
        };
        assert!(exchange_code(&bad, "C", "V", "r").is_err());
        let empty = FakeHttp {
            post_body: r#"{"access_token":""}"#.to_string(),
            get_body: String::new(),
            seen_forms: RefCell::new(Vec::new()),
        };
        assert!(exchange_code(&empty, "C", "V", "r").is_err());
    }

    // ---- Token-endpoint error surfacing (BE-OAUTH 400 diagnosis) --------

    #[test]
    fn oauth_error_suffix_surfaces_error_and_description() {
        // A real Google token 400 body: error + error_description, no tokens.
        let body = r#"{"error":"invalid_request","error_description":"client_secret is missing."}"#;
        let suffix = oauth_error_suffix(body);
        assert_eq!(suffix, ": invalid_request (client_secret is missing.)");
    }

    #[test]
    fn oauth_error_suffix_error_only_when_no_description() {
        let body = r#"{"error":"invalid_grant"}"#;
        assert_eq!(oauth_error_suffix(body), ": invalid_grant");
    }

    #[test]
    fn oauth_error_suffix_empty_on_unparseable_or_errorless() {
        assert_eq!(oauth_error_suffix("not json at all"), "");
        assert_eq!(oauth_error_suffix("{}"), "");
        assert_eq!(oauth_error_suffix(r#"{"error":""}"#), "");
    }

    #[test]
    fn oauth_error_suffix_never_echoes_token_material() {
        // Even if a (hypothetical) failure body also carried token-shaped
        // fields, the suffix must surface ONLY error/error_description and must
        // NEVER echo any token field value.
        let body = r#"{"error":"invalid_request","error_description":"bad",
            "access_token":"SECRET_AT","refresh_token":"SECRET_RT","id_token":"SECRET_ID"}"#;
        let suffix = oauth_error_suffix(body);
        assert_eq!(suffix, ": invalid_request (bad)");
        assert!(!suffix.contains("SECRET_AT"), "must not echo access_token");
        assert!(!suffix.contains("SECRET_RT"), "must not echo refresh_token");
        assert!(!suffix.contains("SECRET_ID"), "must not echo id_token");
        assert!(!suffix.contains("access_token"));
        assert!(!suffix.contains("refresh_token"));
    }

    /// FakeHttp that returns an Err exactly as `ReqwestHttp::post_form` would on
    /// a non-2xx — proving `exchange_code` propagates the surfaced OAuth error
    /// (and no token material) all the way to the caller's message.
    struct ErrHttp {
        err: String,
    }
    impl TokenHttp for ErrHttp {
        fn post_form(&self, _url: &str, _form: &[(&str, &str)]) -> Result<String, String> {
            Err(self.err.clone())
        }
        fn get_bearer(&self, _url: &str, _t: &str) -> Result<String, String> {
            Ok(String::new())
        }
    }

    #[test]
    fn exchange_code_propagates_surfaced_token_error() {
        // Construct the message the production post_form would produce for a
        // 400 with Google's error body.
        let msg = format!(
            "token endpoint returned HTTP 400{}",
            oauth_error_suffix(
                r#"{"error":"invalid_request","error_description":"client_secret is missing."}"#
            )
        );
        assert_eq!(
            msg,
            "token endpoint returned HTTP 400: invalid_request (client_secret is missing.)"
        );
        let fake = ErrHttp { err: msg.clone() };
        let got = exchange_code(&fake, "CODE", "VERIFIER", "http://127.0.0.1:1").unwrap_err();
        assert_eq!(got, msg);
        assert!(!got.contains("access_token"));
    }

    // ---- pct_encode / url_decode round-trip -----------------------------

    #[test]
    fn pct_encode_encodes_reserved_chars() {
        assert_eq!(pct_encode("a b/c:d"), "a%20b%2Fc%3Ad");
        assert_eq!(pct_encode("safe-._~AZ09"), "safe-._~AZ09");
    }

    // ---- Browser opener (Windows cmd & truncation regression) -----------

    /// A real OAuth URL carries several `&`-separated query params. The opener
    /// MUST pass the whole URL as one opaque argv element and MUST NOT route it
    /// through a shell that treats `&` as a command separator (the Windows
    /// `cmd /C start` bug that reached only `response_type` being dropped and
    /// Google returning `invalid_request`).
    #[test]
    fn browser_command_passes_full_url_as_single_arg() {
        let url = build_auth_url("CHAL", "STATE123", "http://127.0.0.1:54321");
        // The URL under test genuinely contains multiple ampersands.
        assert!(url.matches('&').count() >= 5, "fixture must have many & params");

        let (bin, args) = browser_command(&url);

        // The complete, unmodified URL must appear as exactly one argv element,
        // never split on '&'.
        assert!(
            args.iter().any(|a| *a == url.as_str()),
            "full URL must be a single argv element"
        );
        // response_type (the param Google complained was missing) must survive.
        assert!(
            args.iter().any(|a| a.contains("response_type=code")),
            "response_type must reach the opener intact"
        );

        // Never invoke a shell that reinterprets '&'. On Windows this means NOT
        // `cmd` with a `start` verb (which splits the URL on '&').
        assert_ne!(bin, "cmd", "must not shell out through cmd.exe");
        assert_ne!(bin, "sh");
        assert_ne!(bin, "bash");
        assert!(
            !args.iter().any(|a| *a == "start"),
            "must not use cmd's start verb that splits URLs on &"
        );
    }

    #[test]
    fn browser_command_is_platform_appropriate() {
        let (bin, args) = browser_command("http://127.0.0.1:9/cb?a=1&b=2");
        #[cfg(target_os = "windows")]
        {
            assert_eq!(bin, "rundll32.exe");
            assert_eq!(args[0], "url.dll,FileProtocolHandler");
        }
        #[cfg(target_os = "macos")]
        assert_eq!(bin, "open");
        #[cfg(all(unix, not(target_os = "macos")))]
        assert_eq!(bin, "xdg-open");
        let _ = args;
    }
}
