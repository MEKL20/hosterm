// ============================================================
//  AI BACKEND v1 — pure cores + DTOs (SDD §16.2/§16.3/§16.4)
//
//  Unit AI-1 (TSD §3): the eight frozen constants, the frozen IPC DTOs,
//  and the six pure validation/truncation cores. NO network, NO spawn,
//  NO filesystem here — HTTP wiring, the ai-settings.json store, keyring,
//  and the three Tauri commands arrive with AI-2. `lib.rs` contributes
//  only `mod ai;`.
//
//  Reason codes are frozen strings (§16.8): return them verbatim, never
//  localize, log, or embed model data in them.
// ============================================================
//
//  AI-1 stages the frozen contract only; AI-2 (serial) wires the commands,
//  store, keyring, and HTTP paths that consume every item below.
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
}
