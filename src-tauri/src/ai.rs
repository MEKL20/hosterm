// ============================================================
//  AI BACKEND v1 — pure cores + DTOs + store + keyring + HTTP
//  (SDD §16.2–§16.8)
//
//  AI-1 (TSD §3) staged the eight frozen constants, the frozen IPC
//  DTOs, and the six pure validation/truncation cores. AI-2 (serial)
//  adds the ai-settings.json store, the OS keyring seam, the
//  OpenAI-compatible HTTP path, and the three Tauri commands that
//  consume every item below. The pure cores are unchanged; `lib.rs`
//  contributes `mod ai;` + the 3 generate_handler entries.
//
//  Reason codes are frozen strings (§16.8): return them verbatim, never
//  localize, log, or embed model data in them.
// ============================================================
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use crate::sftp_safe;

// ---------- constants (§16.2 — single source of truth; values frozen) ----------

pub(crate) const AI_PROMPT_MAX_BYTES: usize = 8192;
pub(crate) const AI_HISTORY_MAX_TURNS: usize = 20;
pub(crate) const AI_HISTORY_MAX_BYTES: usize = 16384;
pub(crate) const AI_REPLY_MAX_BYTES: usize = 16384;
pub(crate) const AI_COMMAND_MAX_BYTES: usize = 2048;
pub(crate) const AI_DESC_MAX_BYTES: usize = 1024;
pub(crate) const AI_ENDPOINT_MAX_BYTES: usize = 2048;
pub(crate) const AI_MODEL_MAX_BYTES: usize = 256;

/// Frozen reply-truncation marker (§16.4 #6 / LQ-16.3). The byte count is
/// part of the frozen text; do not template it.
const REPLY_TRUNCATED_MARKER: &str = "\n\n[truncated: exceeded 16384 bytes]";

// ---------- DTOs (§16.3 verbatim; snake_case wire fields) ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AiRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AiBackend {
    Loopback,
    Remote,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct AiSettings {
    pub(crate) version: u32,
    pub(crate) endpoint: String,
    pub(crate) model: String,
    pub(crate) remote_allowed: bool,
    pub(crate) has_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct WriteAiSettingsReq {
    pub(crate) endpoint: String,
    pub(crate) model: String,
    pub(crate) remote_allowed: bool,
    pub(crate) key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct AiChatReq {
    pub(crate) prompt: String,
    pub(crate) host: Option<String>,
    pub(crate) history: Vec<AiMessage>,
    pub(crate) timeout_secs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct AiMessage {
    pub(crate) role: AiRole,
    pub(crate) content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct AiChatResp {
    pub(crate) reply: String,
    pub(crate) proposal: Option<AiProposal>,
    pub(crate) backend: AiBackend,
    pub(crate) warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct AiProposal {
    pub(crate) host: String,
    pub(crate) command: String,
    pub(crate) description: String,
}

// ---------- pure cores (§16.4 #1–6) ----------

/// #1 prompt: trim → empty ⇒ `ai:empty-prompt`; UTF-8 bytes over cap ⇒
/// `ai:prompt-too-large`.
pub(crate) fn validate_prompt(prompt: &str) -> Result<(), &'static str> {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return Err("ai:empty-prompt");
    }
    if trimmed.len() > AI_PROMPT_MAX_BYTES {
        return Err("ai:prompt-too-large");
    }
    Ok(())
}

/// #2 history: turns over cap OR summed content UTF-8 bytes over cap ⇒
/// `ai:history-too-large`. The byte total counts message `content` only;
/// `role` is a fixed-size token and not part of the budget.
pub(crate) fn validate_history(history: &[AiMessage]) -> Result<(), &'static str> {
    if history.len() > AI_HISTORY_MAX_TURNS {
        return Err("ai:history-too-large");
    }
    let total: usize = history.iter().map(|m| m.content.len()).sum();
    if total > AI_HISTORY_MAX_BYTES {
        return Err("ai:history-too-large");
    }
    Ok(())
}

/// #3 timeout: outside 5..=120 ⇒ `ai:bad-timeout`. Reject, never clamp.
pub(crate) fn validate_timeout(timeout_secs: u32) -> Result<(), &'static str> {
    if !(5..=120).contains(&timeout_secs) {
        return Err("ai:bad-timeout");
    }
    Ok(())
}

/// #4 endpoint: URL parse fail / scheme ∉ {http,https} / userinfo present /
/// over byte cap ⇒ `ai:bad-endpoint`. Non-loopback with `remote_allowed ==
/// false` ⇒ `ai:remote-endpoint-disabled`; non-loopback with it true requires
/// `https` (otherwise `ai:bad-endpoint`). Loopback = `localhost` /
/// `127.0.0.1` / `[::1]` (hostname case-insensitive).
pub(crate) fn validate_endpoint(endpoint: &str, remote_allowed: bool) -> Result<(), &'static str> {
    if endpoint.len() > AI_ENDPOINT_MAX_BYTES {
        return Err("ai:bad-endpoint");
    }
    // Any userinfo form (including the bare `@host` form, where the parser
    // reports an empty username) is forbidden before parsing.
    if authority_has_userinfo(endpoint) {
        return Err("ai:bad-endpoint");
    }
    let url = reqwest::Url::parse(endpoint).map_err(|_| "ai:bad-endpoint")?;
    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        return Err("ai:bad-endpoint");
    }
    let host = match url.host_str() {
        Some(h) if !h.is_empty() => h,
        _ => return Err("ai:bad-endpoint"),
    };
    if !is_loopback_host(host) {
        if !remote_allowed {
            return Err("ai:remote-endpoint-disabled");
        }
        if scheme != "https" {
            return Err("ai:bad-endpoint");
        }
    }
    Ok(())
}

/// True when the URL authority component carries `@` (userinfo present).
/// Checks the raw authority substring so the empty-username `@host` form is
/// caught even though the URL parser reports `username() == ""`.
fn authority_has_userinfo(endpoint: &str) -> bool {
    let rest = match endpoint.find("://") {
        Some(i) => &endpoint[i + 3..],
        None => return false,
    };
    let end = rest
        .find(|c| c == '/' || c == '?' || c == '#')
        .unwrap_or(rest.len());
    rest[..end].contains('@')
}

/// Loopback host per §16.4 #4. `url` hands IPv6 hosts back without brackets,
/// so both forms are accepted; hostnames compare case-insensitively.
fn is_loopback_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    h == "localhost" || h == "127.0.0.1" || h == "::1" || h == "[::1]"
}

/// #5 proposal: host must pass `sftp_safe` (no leading `-`, no CR/LF) **and**
/// exist in `known_hosts`; command must be single-line and within
/// `AI_COMMAND_MAX_BYTES`; description within `AI_DESC_MAX_BYTES`. Failure
/// returns the frozen warning string (§16.8).
pub(crate) fn validate_proposal(
    proposal: &AiProposal,
    known_hosts: &[String],
) -> Result<AiProposal, &'static str> {
    if sftp_safe(&proposal.host, "Host").is_err() {
        return Err("ai:unsafe-host");
    }
    if !known_hosts.iter().any(|h| h == &proposal.host) {
        return Err("ai:unknown-host");
    }
    if proposal.command.contains('\n') || proposal.command.contains('\r') {
        return Err("ai:invalid-proposal");
    }
    if proposal.command.len() > AI_COMMAND_MAX_BYTES {
        return Err("ai:command-too-large");
    }
    if proposal.description.len() > AI_DESC_MAX_BYTES {
        return Err("ai:invalid-proposal");
    }
    Ok(proposal.clone())
}

/// #6 reply truncation: over cap ⇒ the first `AI_REPLY_MAX_BYTES` bytes
/// (backed off to a char boundary so a codepoint is never split) plus the
/// frozen marker.
pub(crate) fn truncate_reply(reply: &str) -> String {
    if reply.len() <= AI_REPLY_MAX_BYTES {
        return reply.to_string();
    }
    let mut end = AI_REPLY_MAX_BYTES;
    while !reply.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = String::with_capacity(end + REPLY_TRUNCATED_MARKER.len());
    out.push_str(&reply[..end]);
    out.push_str(REPLY_TRUNCATED_MARKER);
    out
}

// ============================================================
//  AI-2 — store + keyring + HTTP wiring + the three commands
//  (SDD §16.3/§16.5/§16.7/§16.8; TSD AI-2)
//
//  The pure cores above are AI-1's frozen contract and are consumed
//  verbatim here. This section adds the ai-settings.json store, the OS
//  keyring seam, the OpenAI-compatible HTTP path, and the three Tauri
//  commands. Nol execute at any step (INV-36): a proposal is data only.
// ============================================================

// ---------- frozen reason codes (§16.8, exact strings) ----------

// `ai_chat` rejections (12).
const AI_ERR_NOT_CONFIGURED: &str = "ai:not-configured";
const AI_ERR_EMPTY_PROMPT: &str = "ai:empty-prompt";
const AI_ERR_PROMPT_TOO_LARGE: &str = "ai:prompt-too-large";
const AI_ERR_HISTORY_TOO_LARGE: &str = "ai:history-too-large";
const AI_ERR_BAD_TIMEOUT: &str = "ai:bad-timeout";
const AI_ERR_BAD_ENDPOINT: &str = "ai:bad-endpoint";
const AI_ERR_REMOTE_DISABLED: &str = "ai:remote-endpoint-disabled";
const AI_ERR_ENDPOINT_UNREACHABLE: &str = "ai:endpoint-unreachable";
const AI_ERR_TIMEOUT: &str = "ai:timeout";
const AI_ERR_UNAUTHORIZED: &str = "ai:unauthorized";
const AI_ERR_MODEL_ERROR: &str = "ai:model-error";
const AI_ERR_KEYRING_UNAVAILABLE: &str = "ai:keyring-unavailable";

// `ai_chat` warnings (4).
const AI_WARN_UNSAFE_HOST: &str = "ai:unsafe-host";
const AI_WARN_UNKNOWN_HOST: &str = "ai:unknown-host";
const AI_WARN_COMMAND_TOO_LARGE: &str = "ai:command-too-large";
const AI_WARN_INVALID_PROPOSAL: &str = "ai:invalid-proposal";

// `write_ai_settings` rejections (5).
const AIS_ERR_INVALID_ENDPOINT: &str = "ai-settings:invalid-endpoint";
const AIS_ERR_REMOTE_DISABLED: &str = "ai-settings:remote-disabled";
const AIS_ERR_INVALID_MODEL: &str = "ai-settings:invalid-model";
const AIS_ERR_KEYRING_UNAVAILABLE: &str = "ai-settings:keyring-unavailable";
const AIS_ERR_WRITE_FAILED: &str = "ai-settings:write-failed";

// ---------- ai-settings.json store (§16.3; the key NEVER persists) ----------

/// Store schema version. Reads coerce a foreign value back to it.
const AI_SETTINGS_VERSION: u32 = 1;

/// On-disk shape of `ai-settings.json`. Deliberately has NO key field: the
/// API key lives only in the OS keyring (INV-37). `has_key` is derived at
/// read time and never written.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AiSettingsFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    endpoint: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    remote_allowed: bool,
}

impl Default for AiSettingsFile {
    fn default() -> Self {
        AiSettingsFile {
            version: AI_SETTINGS_VERSION,
            endpoint: String::new(),
            model: String::new(),
            remote_allowed: false,
        }
    }
}

fn ai_settings_path() -> std::path::PathBuf {
    crate::data_dir().join("ai-settings.json")
}

/// Read the store. NEVER throws: missing, unreadable, or corrupt JSON all
/// yield `AiSettingsFile::default()`; a corrupt file is left untouched
/// (INV-24 class — no destructive self-heal on read).
fn read_ai_settings_file(path: &std::path::Path) -> AiSettingsFile {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let mut file: AiSettingsFile = serde_json::from_str(&text).unwrap_or_default();
            if file.version != AI_SETTINGS_VERSION {
                file.version = AI_SETTINGS_VERSION;
            }
            file
        }
        Err(_) => AiSettingsFile::default(),
    }
}

// ---------- keyring seam (§16.7 / INV-37) ----------
//
// The AI API key lives in the OS keyring under service `hosterm`, account
// `ai-api-key` — a dedicated entry, separate from `google-refresh-token`
// (OAuth) and `backup-data-key` (backup). The trait lets tests inject an
// in-memory fake so no real keyring is touched; production wraps
// `keyring::Entry`. The key value never crosses IPC, logs, argv, or error
// strings.

pub(crate) trait AiKeyring {
    /// `Ok(true)` when an entry exists, `Ok(false)` when it does not.
    fn has_key(&self) -> Result<bool, String>;
    /// Read the key, `Ok(None)` when no entry exists.
    fn read_key(&self) -> Result<Option<String>, String>;
    /// Persist (overwrite) the key.
    fn store_key(&self, key: &str) -> Result<(), String>;
    /// Delete the key; a missing entry is success (idempotent).
    fn delete_key(&self) -> Result<(), String>;
}

const AI_KEYRING_SERVICE: &str = "hosterm";
const AI_KEYRING_ACCOUNT: &str = "ai-api-key";

/// Production keyring backed by the OS credential store.
struct OsAiKeyring;

impl AiKeyring for OsAiKeyring {
    fn has_key(&self) -> Result<bool, String> {
        let entry = keyring::Entry::new(AI_KEYRING_SERVICE, AI_KEYRING_ACCOUNT)
            .map_err(|_| "keyring unavailable".to_string())?;
        match entry.get_password() {
            Ok(_) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(_) => Err("keyring unavailable".to_string()),
        }
    }

    fn read_key(&self) -> Result<Option<String>, String> {
        let entry = keyring::Entry::new(AI_KEYRING_SERVICE, AI_KEYRING_ACCOUNT)
            .map_err(|_| "keyring unavailable".to_string())?;
        match entry.get_password() {
            Ok(k) => Ok(Some(k)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err("keyring unavailable".to_string()),
        }
    }

    fn store_key(&self, key: &str) -> Result<(), String> {
        let entry = keyring::Entry::new(AI_KEYRING_SERVICE, AI_KEYRING_ACCOUNT)
            .map_err(|_| "keyring unavailable".to_string())?;
        entry
            .set_password(key)
            .map_err(|_| "keyring unavailable".to_string())
    }

    fn delete_key(&self) -> Result<(), String> {
        let entry = keyring::Entry::new(AI_KEYRING_SERVICE, AI_KEYRING_ACCOUNT)
            .map_err(|_| "keyring unavailable".to_string())?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("keyring unavailable".to_string()),
        }
    }
}

// ---------- read / write settings commands (§16.3) ----------

/// `read_ai_settings` inner: NEVER throws. Missing/corrupt store → defaults;
/// the key is read from the keyring as a boolean only (INV-37). A keyring
/// failure degrades to `has_key: false`, never to an error.
pub(crate) fn read_ai_settings_inner(
    path: &std::path::Path,
    keyring: &dyn AiKeyring,
) -> AiSettings {
    let file = read_ai_settings_file(path);
    AiSettings {
        version: AI_SETTINGS_VERSION,
        endpoint: file.endpoint,
        model: file.model,
        remote_allowed: file.remote_allowed,
        has_key: keyring.has_key().unwrap_or(false),
    }
}

#[tauri::command]
pub(crate) fn read_ai_settings() -> AiSettings {
    read_ai_settings_inner(&ai_settings_path(), &OsAiKeyring)
}

/// `write_ai_settings` inner: validate the frozen rejects, apply key
/// semantics, then write the whole file atomically (temp → rename,
/// 0600-class). Keyring mutations happen BEFORE the file write so a keyring
/// failure leaves the store byte-identical (atomic reject, §3.2).
pub(crate) fn write_ai_settings_inner(
    req: WriteAiSettingsReq,
    path: &std::path::Path,
    keyring: &dyn AiKeyring,
) -> Result<(), String> {
    match validate_endpoint(&req.endpoint, req.remote_allowed) {
        Ok(()) => {}
        Err("ai:remote-endpoint-disabled") => return Err(AIS_ERR_REMOTE_DISABLED.into()),
        Err(_) => return Err(AIS_ERR_INVALID_ENDPOINT.into()),
    }
    let model = req.model.trim().to_string();
    if model.is_empty() || req.model.len() > AI_MODEL_MAX_BYTES {
        return Err(AIS_ERR_INVALID_MODEL.into());
    }
    match &req.key {
        None => {}
        Some(k) if k.is_empty() => keyring
            .delete_key()
            .map_err(|_| AIS_ERR_KEYRING_UNAVAILABLE.to_string())?,
        Some(k) => keyring
            .store_key(k)
            .map_err(|_| AIS_ERR_KEYRING_UNAVAILABLE.to_string())?,
    }
    let file = AiSettingsFile {
        version: AI_SETTINGS_VERSION,
        endpoint: req.endpoint.trim().to_string(),
        model,
        remote_allowed: req.remote_allowed,
    };
    let body =
        serde_json::to_string_pretty(&file).map_err(|_| AIS_ERR_WRITE_FAILED.to_string())?;
    let store = path.to_path_buf();
    crate::write_store_atomic(&store, &body).map_err(|_| AIS_ERR_WRITE_FAILED.to_string())
}

#[tauri::command]
pub(crate) fn write_ai_settings(req: WriteAiSettingsReq) -> Result<(), String> {
    write_ai_settings_inner(req, &ai_settings_path(), &OsAiKeyring)
}

// ---------- ai_chat (§16.5 nine steps, nol execute) ----------

/// Frozen system prompt (LQ-16.2 / §16.6) — VERBATIM. Do not reword.
const AI_SYSTEM_PROMPT: &str = r#"You are hosterm's command assistant. The user describes an operational task.
Reply with ONLY one JSON object, no prose outside it:
{ "reply": "<short user-facing explanation>",
  "proposal": { "host": "<exact host alias from the user's wording>",
                "command": "<single-line shell command>",
                "description": "<one sentence on the impact>" } }
Rules: never execute anything; you have no terminal or host access; never ask for
or include secrets/keys/credentials; the command must be a single line; if you
cannot propose a safe command, set "proposal": null."#;

fn endpoint_is_loopback(endpoint: &str) -> bool {
    match reqwest::Url::parse(endpoint) {
        Ok(url) => url.host_str().map(is_loopback_host).unwrap_or(false),
        Err(_) => false,
    }
}

fn role_wire(role: &AiRole) -> &'static str {
    match role {
        AiRole::User => "user",
        AiRole::Assistant => "assistant",
    }
}

/// Step 4: OpenAI-compatible request body — system prompt + history + prompt.
/// Never includes ~/.ssh/config contents, the key, sidecar bytes, or terminal
/// output (§16.5 step 4 / §5.4).
fn build_chat_body(model: &str, req: &AiChatReq) -> serde_json::Value {
    let mut messages = Vec::with_capacity(req.history.len() + 2);
    messages.push(serde_json::json!({
        "role": "system",
        "content": AI_SYSTEM_PROMPT,
    }));
    for m in &req.history {
        messages.push(serde_json::json!({
            "role": role_wire(&m.role),
            "content": m.content.clone(),
        }));
    }
    messages.push(serde_json::json!({
        "role": "user",
        "content": req.prompt.trim().to_string(),
    }));
    serde_json::json!({ "model": model, "messages": messages })
}

/// Step 7: extract `choices[0].message.content`. `None` ⇒ `ai:model-error`.
fn extract_message_content(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v.get("choices")?
        .get(0)?
        .get("message")?
        .get("content")?
        .as_str()
        .map(|s| s.to_string())
}

/// Model reply JSON: `{reply, proposal{host,command,description}}`. A missing
/// `proposal` is `null`; a missing/non-string `reply` fails the parse so the
/// caller falls back to the raw content (§16.5 step 7).
#[derive(Deserialize)]
struct ModelReply {
    reply: String,
    #[serde(default)]
    proposal: Option<AiProposal>,
}

/// Steps 7–9: parse the content, validate the proposal, build the response.
/// Parse failure ⇒ raw-text reply with `proposal: null` and no warning;
/// proposal validation failure ⇒ `warning` + `proposal: null`.
fn build_chat_resp(content: &str, backend: AiBackend, known_hosts: &[String]) -> AiChatResp {
    match serde_json::from_str::<ModelReply>(content) {
        Ok(model) => {
            let reply = truncate_reply(&model.reply);
            let (proposal, warning) = match model.proposal {
                Some(p) => match validate_proposal(&p, known_hosts) {
                    Ok(valid) => (Some(valid), None),
                    Err(code) => (None, Some(code.to_string())),
                },
                None => (None, None),
            };
            AiChatResp {
                reply,
                proposal,
                backend,
                warning,
            }
        }
        Err(_) => AiChatResp {
            reply: truncate_reply(content),
            proposal: None,
            backend,
            warning: None,
        },
    }
}

/// The nine frozen steps of §16.5. `known_hosts` is the local config snapshot
/// (the alias list is never sent to the model, §5.4). Nol execute at any step.
fn ai_chat_inner(
    req: AiChatReq,
    store_path: &std::path::Path,
    keyring: &dyn AiKeyring,
    known_hosts: &[String],
) -> Result<AiChatResp, String> {
    // 1. Load store (missing/corrupt → defaults). Empty model or endpoint ⇒
    //    not configured, 0 network.
    let settings = read_ai_settings_file(store_path);
    if settings.endpoint.trim().is_empty() || settings.model.trim().is_empty() {
        return Err(AI_ERR_NOT_CONFIGURED.into());
    }
    // 2. Validate prompt/history/timeout (frozen cores).
    validate_prompt(&req.prompt)?;
    validate_history(&req.history)?;
    validate_timeout(req.timeout_secs)?;
    // 3. Validate endpoint (frozen core).
    validate_endpoint(&settings.endpoint, settings.remote_allowed)?;
    let remote = !endpoint_is_loopback(&settings.endpoint);
    let backend = if remote {
        AiBackend::Remote
    } else {
        AiBackend::Loopback
    };
    // 4. Build POST <endpoint>/chat/completions.
    let url = format!(
        "{}/chat/completions",
        settings.endpoint.trim().trim_end_matches('/')
    );
    let body = build_chat_body(&settings.model, &req);
    // 6 (client): redirect policy none; timeout from the request.
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(req.timeout_secs as u64))
        .build()
        .map_err(|_| AI_ERR_ENDPOINT_UNREACHABLE.to_string())?;
    let mut builder = client.post(url.as_str()).json(&body);
    // 5. Authorization only when the endpoint is remote AND a key exists.
    //    A keyring failure on a remote call is fail-closed (INV-37).
    if remote {
        match keyring.read_key() {
            Ok(Some(key)) => {
                builder = builder.bearer_auth(key);
            }
            Ok(None) => {}
            Err(_) => return Err(AI_ERR_KEYRING_UNAVAILABLE.into()),
        }
    }
    // 6. Send; map transport errors to frozen codes (never the URL/key).
    let resp = builder.send().map_err(|e| {
        if e.is_timeout() {
            AI_ERR_TIMEOUT.to_string()
        } else {
            AI_ERR_ENDPOINT_UNREACHABLE.to_string()
        }
    })?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(AI_ERR_UNAUTHORIZED.into());
    }
    if !status.is_success() {
        return Err(AI_ERR_MODEL_ERROR.into());
    }
    let text = resp.text().map_err(|_| AI_ERR_MODEL_ERROR.to_string())?;
    // 7. Parse choices[0].message.content.
    let content = extract_message_content(&text).ok_or_else(|| AI_ERR_MODEL_ERROR.to_string())?;
    // 8 + 9. Validate proposal, return. Nol execute.
    Ok(build_chat_resp(&content, backend, known_hosts))
}

/// Async wrapper so the blocking HTTP call runs off the Tauri main/UI
/// thread (TSD §5 risk #1). `ai_chat_inner` stays sync, so the pure-path
/// tests are unaffected and the JS contract is unchanged (async commands
/// serialize identically — same note as `lib.rs` `sftp_*`).
#[tauri::command]
pub(crate) async fn ai_chat(req: AiChatReq) -> Result<AiChatResp, String> {
    tauri::async_runtime::spawn_blocking(move || {
        ai_chat_inner(
            req,
            &ai_settings_path(),
            &OsAiKeyring,
            &config_snapshot_hosts(),
        )
    })
    .await
    .map_err(|_| AI_ERR_ENDPOINT_UNREACHABLE.to_string())?
}

/// Read-only snapshot of the ssh config aliases for proposal validation
/// (§16.4 #5). Wildcard patterns are excluded: a proposal host must be an
/// exact alias. Never writes the config (unlike `read_ssh_config`, which
/// self-heals); an unreadable config yields an empty snapshot.
fn config_snapshot_hosts() -> Vec<String> {
    let path = crate::config_path();
    let text = match crate::read_config_text(&path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    crate::parse_config(&text)
        .hosts
        .iter()
        .filter_map(|h| h.patterns.first().cloned())
        .filter(|p| {
            !p.is_empty() && !p.contains('*') && !p.contains('?') && !p.starts_with('!')
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: AiRole, content: &str) -> AiMessage {
        AiMessage {
            role,
            content: content.to_string(),
        }
    }

    fn proposal(host: &str, command: &str, description: &str) -> AiProposal {
        AiProposal {
            host: host.to_string(),
            command: command.to_string(),
            description: description.to_string(),
        }
    }

    #[test]
    fn ai_constants_match_frozen_spec_values() {
        assert_eq!(AI_PROMPT_MAX_BYTES, 8192);
        assert_eq!(AI_HISTORY_MAX_TURNS, 20);
        assert_eq!(AI_HISTORY_MAX_BYTES, 16384);
        assert_eq!(AI_REPLY_MAX_BYTES, 16384);
        assert_eq!(AI_COMMAND_MAX_BYTES, 2048);
        assert_eq!(AI_DESC_MAX_BYTES, 1024);
        assert_eq!(AI_ENDPOINT_MAX_BYTES, 2048);
        assert_eq!(AI_MODEL_MAX_BYTES, 256);
    }

    #[test]
    fn ai_prompt_accepts_normal_after_trim() {
        assert!(validate_prompt("hello").is_ok());
        assert!(validate_prompt("  hello  ").is_ok());
        assert!(validate_prompt(&"a".repeat(AI_PROMPT_MAX_BYTES)).is_ok());
    }

    #[test]
    fn ai_prompt_rejects_empty_and_oversize() {
        assert_eq!(validate_prompt(""), Err("ai:empty-prompt"));
        assert_eq!(validate_prompt("   "), Err("ai:empty-prompt"));
        assert_eq!(validate_prompt("\n\t "), Err("ai:empty-prompt"));
        assert_eq!(
            validate_prompt(&"a".repeat(AI_PROMPT_MAX_BYTES + 1)),
            Err("ai:prompt-too-large")
        );
    }

    #[test]
    fn ai_prompt_measures_utf8_bytes_not_chars() {
        assert!(validate_prompt(&"é".repeat(4096)).is_ok()); // 8192 bytes
        assert_eq!(
            validate_prompt(&"é".repeat(4097)),
            Err("ai:prompt-too-large")
        ); // 8194 bytes
    }

    #[test]
    fn ai_history_rejects_too_many_turns() {
        let mut h: Vec<AiMessage> = (0..AI_HISTORY_MAX_TURNS)
            .map(|_| msg(AiRole::User, "ab"))
            .collect();
        assert!(validate_history(&h).is_ok());
        h.push(msg(AiRole::Assistant, "ab"));
        assert_eq!(validate_history(&h), Err("ai:history-too-large"));
    }

    #[test]
    fn ai_history_rejects_total_byte_overflow() {
        let h = vec![msg(AiRole::User, &"a".repeat(AI_HISTORY_MAX_BYTES + 1))];
        assert_eq!(validate_history(&h), Err("ai:history-too-large"));
    }

    #[test]
    fn ai_history_total_bytes_boundary_is_inclusive() {
        let h = vec![
            msg(AiRole::User, &"a".repeat(AI_HISTORY_MAX_BYTES - 5)),
            msg(AiRole::Assistant, "12345"),
        ];
        assert!(validate_history(&h).is_ok());
    }

    #[test]
    fn ai_timeout_rejects_out_of_range_never_clamps() {
        assert!(validate_timeout(5).is_ok());
        assert!(validate_timeout(30).is_ok());
        assert!(validate_timeout(120).is_ok());
        for bad in [0, 4, 121, u32::MAX] {
            assert_eq!(validate_timeout(bad), Err("ai:bad-timeout"));
        }
    }

    #[test]
    fn ai_endpoint_accepts_loopback_forms() {
        for ep in [
            "http://localhost",
            "http://localhost:11434/v1",
            "http://127.0.0.1:11434/v1",
            "http://[::1]:11434/v1",
            "https://LOCALHOST:8443",
            "HTTP://127.0.0.1/v1/chat/completions",
        ] {
            assert!(validate_endpoint(ep, false).is_ok(), "{}", ep);
        }
    }

    #[test]
    fn ai_endpoint_rejects_userinfo_and_remote() {
        assert_eq!(
            validate_endpoint("http://user:pass@127.0.0.1", false),
            Err("ai:bad-endpoint")
        );
        assert_eq!(
            validate_endpoint("http://@127.0.0.1", false),
            Err("ai:bad-endpoint")
        );
        assert_eq!(
            validate_endpoint("https://user@example.com", false),
            Err("ai:bad-endpoint")
        );
        assert_eq!(
            validate_endpoint("http://example.com", false),
            Err("ai:remote-endpoint-disabled")
        );
        assert_eq!(
            validate_endpoint("https://example.com", false),
            Err("ai:remote-endpoint-disabled")
        );
    }

    #[test]
    fn ai_endpoint_requires_https_for_remote_when_allowed() {
        assert_eq!(
            validate_endpoint("http://example.com", true),
            Err("ai:bad-endpoint")
        );
        assert!(validate_endpoint("https://example.com", true).is_ok());
        // loopback stays allowed over http even when remote is allowed
        assert!(validate_endpoint("http://127.0.0.1:11434", true).is_ok());
    }

    #[test]
    fn ai_endpoint_rejects_bad_scheme_parse_fail_and_oversize() {
        for bad in [
            "",
            "not a url",
            "ftp://127.0.0.1",
            "file:///etc/passwd",
            "javascript:alert(1)",
        ] {
            assert_eq!(validate_endpoint(bad, true), Err("ai:bad-endpoint"), "{}", bad);
        }
        let huge = format!("http://127.0.0.1/{}", "a".repeat(AI_ENDPOINT_MAX_BYTES));
        assert_eq!(validate_endpoint(&huge, false), Err("ai:bad-endpoint"));
    }

    #[test]
    fn ai_endpoint_byte_cap_is_inclusive() {
        let base = "http://127.0.0.1/";
        let pad = AI_ENDPOINT_MAX_BYTES - base.len();
        let exact = format!("{}{}", base, "a".repeat(pad));
        assert_eq!(exact.len(), AI_ENDPOINT_MAX_BYTES);
        assert!(validate_endpoint(&exact, false).is_ok());
    }

    #[test]
    fn ai_proposal_accepts_known_safe_host() {
        let hosts = vec!["web-01".to_string(), "db".to_string()];
        let p = proposal("web-01", "ls -la", "List files on web-01");
        let ok = validate_proposal(&p, &hosts).expect("valid proposal");
        assert_eq!(ok.host, "web-01");
        assert_eq!(ok.command, "ls -la");
        assert_eq!(ok.description, "List files on web-01");
    }

    #[test]
    fn ai_proposal_requires_known_safe_host() {
        let hosts = vec!["web-01".to_string()];
        // unsafe: leading '-'
        assert_eq!(
            validate_proposal(&proposal("-evil", "ls", "d"), &hosts).unwrap_err(),
            "ai:unsafe-host"
        );
        // unsafe: CR/LF in host
        assert_eq!(
            validate_proposal(&proposal("web\r01", "ls", "d"), &hosts).unwrap_err(),
            "ai:unsafe-host"
        );
        // safe but unknown
        assert_eq!(
            validate_proposal(&proposal("prod", "ls", "d"), &hosts).unwrap_err(),
            "ai:unknown-host"
        );
        // unsafe wins over unknown
        assert_eq!(
            validate_proposal(&proposal("-unknown", "ls", "d"), &hosts).unwrap_err(),
            "ai:unsafe-host"
        );
    }

    #[test]
    fn ai_proposal_command_and_description_caps() {
        let hosts = vec!["web-01".to_string()];
        // multi-line command
        assert_eq!(
            validate_proposal(&proposal("web-01", "ls\nls", "d"), &hosts).unwrap_err(),
            "ai:invalid-proposal"
        );
        assert_eq!(
            validate_proposal(&proposal("web-01", "ls\rls", "d"), &hosts).unwrap_err(),
            "ai:invalid-proposal"
        );
        // command over cap
        assert_eq!(
            validate_proposal(
                &proposal("web-01", &"c".repeat(AI_COMMAND_MAX_BYTES + 1), "d"),
                &hosts
            )
            .unwrap_err(),
            "ai:command-too-large"
        );
        // command at cap ok
        assert!(
            validate_proposal(&proposal("web-01", &"c".repeat(AI_COMMAND_MAX_BYTES), "d"), &hosts)
                .is_ok()
        );
        // description over cap
        assert_eq!(
            validate_proposal(
                &proposal("web-01", "ls", &"d".repeat(AI_DESC_MAX_BYTES + 1)),
                &hosts
            )
            .unwrap_err(),
            "ai:invalid-proposal"
        );
        // description at cap ok
        assert!(
            validate_proposal(&proposal("web-01", "ls", &"d".repeat(AI_DESC_MAX_BYTES)), &hosts)
                .is_ok()
        );
    }

    #[test]
    fn ai_reply_passthrough_at_or_under_cap() {
        assert_eq!(truncate_reply(""), "");
        assert_eq!(truncate_reply("hello"), "hello");
        let exact = "a".repeat(AI_REPLY_MAX_BYTES);
        assert_eq!(truncate_reply(&exact), exact);
    }

    #[test]
    fn ai_reply_truncates_with_marker() {
        let long = "a".repeat(AI_REPLY_MAX_BYTES + 1);
        let out = truncate_reply(&long);
        assert!(out.starts_with(&"a".repeat(AI_REPLY_MAX_BYTES)));
        assert!(out.ends_with("\n\n[truncated: exceeded 16384 bytes]"));
        assert_eq!(
            out.len(),
            AI_REPLY_MAX_BYTES + "\n\n[truncated: exceeded 16384 bytes]".len()
        );
    }

    #[test]
    fn ai_reply_truncation_respects_char_boundary() {
        // 3-byte chars: 16384 / 3 = 5461 r 1 → body must stop at 16383 bytes.
        let long = "€".repeat(6000);
        assert_eq!(long.len(), 18000);
        let out = truncate_reply(&long);
        let body = &out[..AI_REPLY_MAX_BYTES - 1];
        assert_eq!(body.len(), 16383);
        assert_eq!(body.chars().count(), 5461);
        assert!(out.ends_with("\n\n[truncated: exceeded 16384 bytes]"));
    }

    #[test]
    fn ai_dto_roundtrips_snake_case() {
        let settings = AiSettings {
            version: 1,
            endpoint: "http://127.0.0.1:11434/v1".to_string(),
            model: "llama3.2".to_string(),
            remote_allowed: false,
            has_key: true,
        };
        let v = serde_json::to_value(&settings).unwrap();
        assert_eq!(v["version"], 1);
        assert_eq!(v["remote_allowed"], false);
        assert_eq!(v["has_key"], true);

        let resp = AiChatResp {
            reply: "hi".to_string(),
            proposal: None,
            backend: AiBackend::Loopback,
            warning: None,
        };
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["backend"], "loopback");

        let parsed: AiMessage =
            serde_json::from_str(r#"{"role":"assistant","content":"yo"}"#).unwrap();
        assert!(matches!(parsed.role, AiRole::Assistant));
        assert_eq!(parsed.content, "yo");

        let parsed: AiChatReq =
            serde_json::from_str(r#"{"prompt":"p","host":null,"history":[],"timeout_secs":30}"#)
                .unwrap();
        assert_eq!(parsed.timeout_secs, 30);
    }

    // ---------- AI-2: store / keyring / HTTP-path tests ----------
    //
    // No socket bind, no real network: store/keyring/validation run against
    // fakes and temp dirs; the HTTP path is exercised only up to the pre-send
    // rejections and through the parse/validate fixture cores. The loopback
    // mock smoke is AI-5's job.

    use std::cell::RefCell;

    struct FakeKeyring {
        key: RefCell<Option<String>>,
        fail: bool,
    }

    impl FakeKeyring {
        fn new() -> Self {
            Self {
                key: RefCell::new(None),
                fail: false,
            }
        }
        fn with_key(k: &str) -> Self {
            Self {
                key: RefCell::new(Some(k.to_string())),
                fail: false,
            }
        }
        fn failing() -> Self {
            Self {
                key: RefCell::new(None),
                fail: true,
            }
        }
    }

    impl AiKeyring for FakeKeyring {
        fn has_key(&self) -> Result<bool, String> {
            if self.fail {
                return Err("fake keyring down".into());
            }
            Ok(self.key.borrow().is_some())
        }
        fn read_key(&self) -> Result<Option<String>, String> {
            if self.fail {
                return Err("fake keyring down".into());
            }
            Ok(self.key.borrow().clone())
        }
        fn store_key(&self, key: &str) -> Result<(), String> {
            if self.fail {
                return Err("fake keyring down".into());
            }
            *self.key.borrow_mut() = Some(key.to_string());
            Ok(())
        }
        fn delete_key(&self) -> Result<(), String> {
            if self.fail {
                return Err("fake keyring down".into());
            }
            *self.key.borrow_mut() = None;
            Ok(())
        }
    }

    fn temp_store(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "hosterm-ai2-{}-{}-{}",
            tag,
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn chat_req(prompt: &str, timeout_secs: u32) -> AiChatReq {
        AiChatReq {
            prompt: prompt.to_string(),
            host: None,
            history: Vec::new(),
            timeout_secs,
        }
    }

    fn write_store_raw(path: &std::path::Path, endpoint: &str, model: &str, remote: bool) {
        let body = format!(
            r#"{{"version":1,"endpoint":"{}","model":"{}","remote_allowed":{}}}"#,
            endpoint, model, remote
        );
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn ai_settings_roundtrip_atomic_corrupt_defaults() {
        let dir = temp_store("roundtrip");
        let path = dir.join("ai-settings.json");
        let kr = FakeKeyring::new();

        // Missing file → defaults, never throws.
        let s = read_ai_settings_inner(&path, &kr);
        assert_eq!(s.version, 1);
        assert_eq!(s.endpoint, "");
        assert_eq!(s.model, "");
        assert!(!s.remote_allowed);
        assert!(!s.has_key);

        // Write (non-empty key → keyring), read back.
        write_ai_settings_inner(
            WriteAiSettingsReq {
                endpoint: "http://127.0.0.1:11434/v1".into(),
                model: "llama3.2".into(),
                remote_allowed: false,
                key: Some("sk-live-secret".into()),
            },
            &path,
            &kr,
        )
        .unwrap();
        assert_eq!(kr.key.borrow().as_deref(), Some("sk-live-secret"));
        let s = read_ai_settings_inner(&path, &kr);
        assert_eq!(s.endpoint, "http://127.0.0.1:11434/v1");
        assert_eq!(s.model, "llama3.2");
        assert!(s.has_key);

        // The key never enters the store.
        let bytes = std::fs::read_to_string(&path).unwrap();
        assert!(!bytes.contains("sk-live-secret"));
        assert!(!bytes.contains("ai-api-key"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "store must be 0600-class");
        }

        // Corrupt file → defaults, bytes untouched (no self-heal on read).
        std::fs::write(&path, "{ not json").unwrap();
        let before = std::fs::read_to_string(&path).unwrap();
        let s = read_ai_settings_inner(&path, &kr);
        assert_eq!(s.endpoint, "");
        assert_eq!(s.model, "");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn ai_chat_rejects_not_configured_zero_network() {
        let dir = temp_store("notcfg");
        let path = dir.join("ai-settings.json");
        let kr = FakeKeyring::new();

        // Missing store → defaults → not configured.
        assert_eq!(
            ai_chat_inner(chat_req("hello", 30), &path, &kr, &[]).unwrap_err(),
            AI_ERR_NOT_CONFIGURED
        );

        // Endpoint present, model empty.
        write_store_raw(&path, "http://127.0.0.1:11434/v1", "", false);
        assert_eq!(
            ai_chat_inner(chat_req("hello", 30), &path, &kr, &[]).unwrap_err(),
            AI_ERR_NOT_CONFIGURED
        );

        // Model present, endpoint empty.
        write_store_raw(&path, "", "llama3.2", false);
        assert_eq!(
            ai_chat_inner(chat_req("hello", 30), &path, &kr, &[]).unwrap_err(),
            AI_ERR_NOT_CONFIGURED
        );
    }

    #[test]
    fn ai_chat_rejects_bad_prompt_history_timeout_before_network() {
        let dir = temp_store("validate");
        let path = dir.join("ai-settings.json");
        write_store_raw(&path, "http://127.0.0.1:11434/v1", "llama3.2", false);
        let kr = FakeKeyring::new();

        assert_eq!(
            ai_chat_inner(chat_req("   ", 30), &path, &kr, &[]).unwrap_err(),
            AI_ERR_EMPTY_PROMPT
        );
        assert_eq!(
            ai_chat_inner(
                chat_req(&"a".repeat(AI_PROMPT_MAX_BYTES + 1), 30),
                &path,
                &kr,
                &[]
            )
            .unwrap_err(),
            AI_ERR_PROMPT_TOO_LARGE
        );
        let mut req = chat_req("hello", 30);
        req.history = (0..=AI_HISTORY_MAX_TURNS)
            .map(|_| msg(AiRole::User, "x"))
            .collect();
        assert_eq!(
            ai_chat_inner(req, &path, &kr, &[]).unwrap_err(),
            AI_ERR_HISTORY_TOO_LARGE
        );
        assert_eq!(
            ai_chat_inner(chat_req("hello", 4), &path, &kr, &[]).unwrap_err(),
            AI_ERR_BAD_TIMEOUT
        );
    }

    #[test]
    fn ai_chat_rejects_bad_endpoint_and_remote_before_network() {
        let dir = temp_store("endpoint");
        let path = dir.join("ai-settings.json");
        let kr = FakeKeyring::new();

        write_store_raw(&path, "not a url", "llama3.2", false);
        assert_eq!(
            ai_chat_inner(chat_req("hi", 30), &path, &kr, &[]).unwrap_err(),
            AI_ERR_BAD_ENDPOINT
        );

        write_store_raw(&path, "http://example.com/v1", "llama3.2", false);
        assert_eq!(
            ai_chat_inner(chat_req("hi", 30), &path, &kr, &[]).unwrap_err(),
            AI_ERR_REMOTE_DISABLED
        );

        // remote_allowed=true but plain http → bad endpoint.
        write_store_raw(&path, "http://example.com/v1", "llama3.2", true);
        assert_eq!(
            ai_chat_inner(chat_req("hi", 30), &path, &kr, &[]).unwrap_err(),
            AI_ERR_BAD_ENDPOINT
        );
    }

    #[test]
    fn ai_chat_proposal_unsafe_host_warns() {
        let body = r#"{"choices":[{"message":{"content":"{\"reply\":\"ok\",\"proposal\":{\"host\":\"-evil\",\"command\":\"ls\",\"description\":\"d\"}}"}}]}"#;
        let content = extract_message_content(body).expect("content");
        let resp = build_chat_resp(&content, AiBackend::Loopback, &["web-01".to_string()]);
        assert_eq!(resp.reply, "ok");
        assert!(resp.proposal.is_none());
        assert_eq!(resp.warning.as_deref(), Some(AI_WARN_UNSAFE_HOST));
        assert!(matches!(resp.backend, AiBackend::Loopback));
    }

    #[test]
    fn ai_chat_proposal_unknown_host_warns() {
        let body = r#"{"choices":[{"message":{"content":"{\"reply\":\"r\",\"proposal\":{\"host\":\"prod\",\"command\":\"ls\",\"description\":\"d\"}}"}}]}"#;
        let content = extract_message_content(body).unwrap();
        let resp = build_chat_resp(&content, AiBackend::Loopback, &["web-01".to_string()]);
        assert_eq!(resp.warning.as_deref(), Some(AI_WARN_UNKNOWN_HOST));
        assert!(resp.proposal.is_none());
    }

    #[test]
    fn ai_chat_proposal_valid_fixture_returns_proposal() {
        let body = r#"{"choices":[{"message":{"content":"{\"reply\":\"r\",\"proposal\":{\"host\":\"web-01\",\"command\":\"ls -la\",\"description\":\"list\"}}"}}]}"#;
        let content = extract_message_content(body).unwrap();
        let resp = build_chat_resp(&content, AiBackend::Loopback, &["web-01".to_string()]);
        assert!(resp.warning.is_none());
        let p = resp.proposal.expect("proposal");
        assert_eq!(p.host, "web-01");
        assert_eq!(p.command, "ls -la");
        assert_eq!(p.description, "list");
    }

    #[test]
    fn ai_chat_non_json_content_becomes_raw_reply() {
        let body = r#"{"choices":[{"message":{"content":"plain text answer"}}]}"#;
        let content = extract_message_content(body).unwrap();
        let resp = build_chat_resp(&content, AiBackend::Loopback, &[]);
        assert_eq!(resp.reply, "plain text answer");
        assert!(resp.proposal.is_none());
        assert!(resp.warning.is_none());
    }

    #[test]
    fn ai_keyring_fail_closed() {
        let dir = temp_store("failclosed");
        let path = dir.join("ai-settings.json");
        // Remote https endpoint with remote_allowed=true passes endpoint
        // validation, so a key is required — and the keyring is down. This
        // rejects BEFORE any request is sent.
        write_store_raw(&path, "https://example.com/v1", "llama3.2", true);
        let kr = FakeKeyring::failing();
        assert_eq!(
            ai_chat_inner(chat_req("hi", 30), &path, &kr, &[]).unwrap_err(),
            AI_ERR_KEYRING_UNAVAILABLE
        );
    }

    #[test]
    fn ai_write_settings_key_semantics_and_atomic_rejects() {
        let dir = temp_store("write");
        let path = dir.join("ai-settings.json");
        let kr = FakeKeyring::with_key("old-key");

        // None leaves the key untouched.
        write_ai_settings_inner(
            WriteAiSettingsReq {
                endpoint: "http://127.0.0.1:11434/v1".into(),
                model: "m".into(),
                remote_allowed: false,
                key: None,
            },
            &path,
            &kr,
        )
        .unwrap();
        assert_eq!(kr.key.borrow().as_deref(), Some("old-key"));

        // "" deletes.
        write_ai_settings_inner(
            WriteAiSettingsReq {
                endpoint: "http://127.0.0.1:11434/v1".into(),
                model: "m".into(),
                remote_allowed: false,
                key: Some(String::new()),
            },
            &path,
            &kr,
        )
        .unwrap();
        assert!(kr.key.borrow().is_none());

        // Non-empty stores.
        write_ai_settings_inner(
            WriteAiSettingsReq {
                endpoint: "http://127.0.0.1:11434/v1".into(),
                model: "m".into(),
                remote_allowed: false,
                key: Some("new-key".into()),
            },
            &path,
            &kr,
        )
        .unwrap();
        assert_eq!(kr.key.borrow().as_deref(), Some("new-key"));

        // Keyring failure with a non-empty key rejects and writes nothing.
        let failing = FakeKeyring::failing();
        let fresh = dir.join("fresh.json");
        let err = write_ai_settings_inner(
            WriteAiSettingsReq {
                endpoint: "http://127.0.0.1:11434/v1".into(),
                model: "m".into(),
                remote_allowed: false,
                key: Some("k".into()),
            },
            &fresh,
            &failing,
        )
        .unwrap_err();
        assert_eq!(err, AIS_ERR_KEYRING_UNAVAILABLE);
        assert!(!fresh.exists(), "no write on keyring failure");

        // Frozen validation rejects, no file.
        let err = write_ai_settings_inner(
            WriteAiSettingsReq {
                endpoint: "not a url".into(),
                model: "m".into(),
                remote_allowed: false,
                key: None,
            },
            &fresh,
            &kr,
        )
        .unwrap_err();
        assert_eq!(err, AIS_ERR_INVALID_ENDPOINT);
        assert!(!fresh.exists());

        let err = write_ai_settings_inner(
            WriteAiSettingsReq {
                endpoint: "http://example.com".into(),
                model: "m".into(),
                remote_allowed: false,
                key: None,
            },
            &fresh,
            &kr,
        )
        .unwrap_err();
        assert_eq!(err, AIS_ERR_REMOTE_DISABLED);

        let err = write_ai_settings_inner(
            WriteAiSettingsReq {
                endpoint: "http://127.0.0.1:11434/v1".into(),
                model: "  ".into(),
                remote_allowed: false,
                key: None,
            },
            &fresh,
            &kr,
        )
        .unwrap_err();
        assert_eq!(err, AIS_ERR_INVALID_MODEL);
        assert!(!fresh.exists());

        // Oversize model.
        let err = write_ai_settings_inner(
            WriteAiSettingsReq {
                endpoint: "http://127.0.0.1:11434/v1".into(),
                model: "m".repeat(AI_MODEL_MAX_BYTES + 1),
                remote_allowed: false,
                key: None,
            },
            &fresh,
            &kr,
        )
        .unwrap_err();
        assert_eq!(err, AIS_ERR_INVALID_MODEL);
    }

    #[test]
    fn ai_error_string_table_exact() {
        // 12 ai_chat rejections.
        assert_eq!(AI_ERR_NOT_CONFIGURED, "ai:not-configured");
        assert_eq!(AI_ERR_EMPTY_PROMPT, "ai:empty-prompt");
        assert_eq!(AI_ERR_PROMPT_TOO_LARGE, "ai:prompt-too-large");
        assert_eq!(AI_ERR_HISTORY_TOO_LARGE, "ai:history-too-large");
        assert_eq!(AI_ERR_BAD_TIMEOUT, "ai:bad-timeout");
        assert_eq!(AI_ERR_BAD_ENDPOINT, "ai:bad-endpoint");
        assert_eq!(AI_ERR_REMOTE_DISABLED, "ai:remote-endpoint-disabled");
        assert_eq!(AI_ERR_ENDPOINT_UNREACHABLE, "ai:endpoint-unreachable");
        assert_eq!(AI_ERR_TIMEOUT, "ai:timeout");
        assert_eq!(AI_ERR_UNAUTHORIZED, "ai:unauthorized");
        assert_eq!(AI_ERR_MODEL_ERROR, "ai:model-error");
        assert_eq!(AI_ERR_KEYRING_UNAVAILABLE, "ai:keyring-unavailable");
        // 4 warnings.
        assert_eq!(AI_WARN_UNSAFE_HOST, "ai:unsafe-host");
        assert_eq!(AI_WARN_UNKNOWN_HOST, "ai:unknown-host");
        assert_eq!(AI_WARN_COMMAND_TOO_LARGE, "ai:command-too-large");
        assert_eq!(AI_WARN_INVALID_PROPOSAL, "ai:invalid-proposal");
        // 5 write rejections.
        assert_eq!(AIS_ERR_INVALID_ENDPOINT, "ai-settings:invalid-endpoint");
        assert_eq!(AIS_ERR_REMOTE_DISABLED, "ai-settings:remote-disabled");
        assert_eq!(AIS_ERR_INVALID_MODEL, "ai-settings:invalid-model");
        assert_eq!(
            AIS_ERR_KEYRING_UNAVAILABLE,
            "ai-settings:keyring-unavailable"
        );
        assert_eq!(AIS_ERR_WRITE_FAILED, "ai-settings:write-failed");

        // The AI-1 pure cores agree with the frozen table.
        assert_eq!(validate_prompt("").unwrap_err(), AI_ERR_EMPTY_PROMPT);
        assert_eq!(
            validate_prompt(&"a".repeat(AI_PROMPT_MAX_BYTES + 1)).unwrap_err(),
            AI_ERR_PROMPT_TOO_LARGE
        );
        assert_eq!(
            validate_history(&[msg(
                AiRole::User,
                &"a".repeat(AI_HISTORY_MAX_BYTES + 1)
            )])
            .unwrap_err(),
            AI_ERR_HISTORY_TOO_LARGE
        );
        assert_eq!(validate_timeout(4).unwrap_err(), AI_ERR_BAD_TIMEOUT);
        assert_eq!(
            validate_endpoint("ftp://127.0.0.1", false).unwrap_err(),
            AI_ERR_BAD_ENDPOINT
        );
        assert_eq!(
            validate_endpoint("http://example.com", false).unwrap_err(),
            AI_ERR_REMOTE_DISABLED
        );
        assert_eq!(
            validate_proposal(&proposal("-evil", "ls", "d"), &["web-01".to_string()]).unwrap_err(),
            AI_WARN_UNSAFE_HOST
        );
        assert_eq!(
            validate_proposal(&proposal("prod", "ls", "d"), &["web-01".to_string()]).unwrap_err(),
            AI_WARN_UNKNOWN_HOST
        );
        assert_eq!(
            validate_proposal(
                &proposal("web-01", &"c".repeat(AI_COMMAND_MAX_BYTES + 1), "d"),
                &["web-01".to_string()]
            )
            .unwrap_err(),
            AI_WARN_COMMAND_TOO_LARGE
        );
        assert_eq!(
            validate_proposal(
                &proposal("web-01", "ls", &"d".repeat(AI_DESC_MAX_BYTES + 1)),
                &["web-01".to_string()]
            )
            .unwrap_err(),
            AI_WARN_INVALID_PROPOSAL
        );
    }
}
