use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

mod fleet;
// BE-OAUTH owns these four backend module declarations (lib.rs is a hotspot;
// later backend cards add only their own tauri command wrappers, never
// touch these `mod` lines). oauth is implemented here; crypto/drive/backup are
// placeholders filled by their own cards (TSD §2).
mod backup;
mod crypto;
mod drive;
mod oauth;

// ============================================================
//  SSH CONFIG MODEL  (round-trip: preserves comments + order)
// ============================================================

#[derive(Clone)]
enum CfgLine {
    Raw(String),              // blank line or comment, kept verbatim
    Opt { key: String, value: String },
}

struct HostBlock {
    patterns: Vec<String>,
    lines: Vec<CfgLine>,
}

struct SshConfig {
    preamble: Vec<String>,    // lines before the first Host block
    hosts: Vec<HostBlock>,
}

fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("HOSTERM_CONFIG") {
        return PathBuf::from(p);
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".ssh").join("config")
}

/// Read the config file safely. A MISSING file is normal (empty config).
/// A file that EXISTS but cannot be read (bad perms, non-UTF8, I/O error)
/// is an error we must surface — never silently treat it as empty, or the
/// next save would overwrite the user's real config with nothing.
fn read_config_text(path: &PathBuf) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!(
            "refusing to proceed: {} exists but could not be read ({}). \
             Fix the file before editing so hosterm does not overwrite it.",
            path.display(),
            e
        )),
    }
}

fn parse_config(text: &str) -> SshConfig {
    let mut preamble = Vec::new();
    let mut hosts: Vec<HostBlock> = Vec::new();
    let mut current: Option<HostBlock> = None;

    for raw in text.lines() {
        let trimmed = raw.trim_start();
        let first = trimmed.split_whitespace().next().unwrap_or("");
        if first.eq_ignore_ascii_case("host") {
            if let Some(h) = current.take() {
                hosts.push(h);
            }
            let patterns: Vec<String> = trimmed
                .split_whitespace()
                .skip(1)
                .map(|s| s.to_string())
                .collect();
            current = Some(HostBlock { patterns, lines: Vec::new() });
        } else {
            match current.as_mut() {
                None => preamble.push(raw.to_string()),
                Some(h) => {
                    if trimmed.is_empty() || trimmed.starts_with('#') {
                        h.lines.push(CfgLine::Raw(raw.to_string()));
                    } else {
                        let mut it = trimmed.splitn(2, char::is_whitespace);
                        let key = it.next().unwrap_or("").to_string();
                        let value = it.next().unwrap_or("").trim().to_string();
                        h.lines.push(CfgLine::Opt { key, value });
                    }
                }
            }
        }
    }
    if let Some(h) = current.take() {
        hosts.push(h);
    }
    SshConfig { preamble, hosts }
}

fn serialize_config(cfg: &SshConfig) -> String {
    let mut out = String::new();
    for l in &cfg.preamble {
        out.push_str(l);
        out.push('\n');
    }
    for h in &cfg.hosts {
        out.push_str("Host");
        if !h.patterns.is_empty() {
            out.push(' ');
            out.push_str(&h.patterns.join(" "));
        }
        out.push('\n');
        for line in &h.lines {
            match line {
                CfgLine::Raw(s) => {
                    out.push_str(s);
                    out.push('\n');
                }
                CfgLine::Opt { key, value } => {
                    out.push_str("    ");
                    out.push_str(key);
                    if !value.is_empty() {
                        out.push(' ');
                        out.push_str(value);
                    }
                    out.push('\n');
                }
            }
        }
    }
    out
}

fn get_opt(h: &HostBlock, key: &str) -> String {
    for l in &h.lines {
        if let CfgLine::Opt { key: k, value } = l {
            if k.eq_ignore_ascii_case(key) {
                return value.clone();
            }
        }
    }
    String::new()
}

fn set_opt(h: &mut HostBlock, key: &str, value: &str) {
    if value.is_empty() {
        h.lines
            .retain(|l| !matches!(l, CfgLine::Opt { key: k, .. } if k.eq_ignore_ascii_case(key)));
        return;
    }
    for l in &mut h.lines {
        if let CfgLine::Opt { key: k, value: v } = l {
            if k.eq_ignore_ascii_case(key) {
                *v = value.to_string();
                return;
            }
        }
    }
    h.lines.push(CfgLine::Opt {
        key: key.to_string(),
        value: value.to_string(),
    });
}

fn write_config_file(path: &PathBuf, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // Atomic write: write a sibling temp file, set perms, then rename over the
    // target. A crash mid-write leaves the original config intact rather than
    // a truncated/corrupt file. Rename within the same dir is atomic on unix
    // and replace-existing on Windows.
    let tmp = {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("config");
        let mut t = path.clone();
        t.set_file_name(format!(".{}.hosterm-tmp-{}", name, std::process::id()));
        t
    };
    std::fs::write(&tmp, content).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })?;
    Ok(())
}

// ---------- DTOs ----------

#[derive(Serialize)]
struct HostDto {
    name: String,
    host_name: String,
    user: String,
    port: String,
    identity_file: String,
    auth_method: String,
    has_password: bool,
    options: Vec<(String, String)>,
}

#[derive(Deserialize)]
struct HostInput {
    name: String,
    host_name: String,
    user: String,
    port: String,
    identity_file: String,
    #[serde(default)]
    auth_method: String,
    #[serde(default)]
    password: String,
}

/// Set the auth-related directives on a host block.
/// "password" mode: the secret itself is stored as a `Password` directive.
/// OpenSSH has no such directive, so the block also carries
/// Secrets live in a sidecar file ssh NEVER reads. Earlier releases stored
/// them as a `Password` directive hidden from OpenSSH via `IgnoreUnknown`,
/// but that couples validity to parse-order details inside blocks that our
/// own parser is looser about than ssh — one stray `Password` line anywhere
/// in the file and every connection dies with "Bad configuration option:
/// password". Sidecar keeps `~/.ssh/config` parseable by real ssh at all
/// times, whatever hosterm does to it.
fn secrets_path() -> PathBuf {
    ssh_dir().join("hosterm.secrets")
}

fn read_secrets() -> std::collections::BTreeMap<String, String> {
    let mut m = std::collections::BTreeMap::new();
    if let Ok(text) = std::fs::read_to_string(secrets_path()) {
        for l in text.lines() {
            if let Some(i) = l.find('=') {
                m.insert(l[..i].to_string(), l[i + 1..].to_string());
            }
        }
    }
    m
}

fn write_secrets(m: &std::collections::BTreeMap<String, String>) -> std::io::Result<()> {
    let body: String = m.iter().map(|(k, v)| format!("{}={}\n", k, v)).collect();
    if m.is_empty() {
        // Removing a file that is already gone is success, not an error — the
        // sidecar may never have existed. Any other error (EACCES, ROFS) propagates.
        return match std::fs::remove_file(secrets_path()) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        };
    }
    std::fs::write(secrets_path(), body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(secrets_path(), std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Add/update/remove one alias→secret row. Empty pw removes.
fn set_secret(alias: &str, pw: &str) {
    let mut m = read_secrets();
    if pw.is_empty() {
        m.remove(alias);
    } else {
        m.insert(alias.to_string(), pw.to_string());
    }
    // Save path already surfaces config-write errors; the sidecar write is
    // best-effort here. forget_secret is the path that must propagate.
    let _ = write_secrets(&m);
}

/// One-time cleanup for configs written by <= 0.6.1: pull any stored secret
/// out of `Password` directives into the sidecar, then erase both `Password`
/// and our `IgnoreUnknown Password` marker from every block so the file is
/// parseable by real ssh no matter how the directives were ordered/nested.
/// Returns true when something changed.
fn migrate_legacy_password_directives(cfg: &mut SshConfig) -> bool {
    let mut changed = false;
    for hb in &mut cfg.hosts {
        let mut pw: Option<String> = None;
        for l in hb.lines.iter_mut() {
            if let CfgLine::Opt { key, value } = l {
                if key.eq_ignore_ascii_case("Password") && !value.is_empty() {
                    pw = Some(value.clone());
                }
            }
        }
        if let Some(pw) = pw {
            if let Some(alias) = hb.patterns.first() {
                if read_secrets().get(alias).is_none() {
                    set_secret(alias, &pw);
                }
            }
        }
        let before = hb.lines.len();
        hb.lines.retain(|l| match l {
            CfgLine::Opt { key, value } => {
                let k = key.to_ascii_lowercase();
                let ours_iu = k == "ignoreunknown"
                    && value.split_whitespace().any(|t| t.eq_ignore_ascii_case("Password"));
                !k.eq_ignore_ascii_case("password") && !ours_iu
            }
            _ => true,
        });
        changed |= hb.lines.len() != before;
    }
    changed
}

fn apply_host_fields(hb: &mut HostBlock, host: &HostInput) {
    set_opt(hb, "HostName", &host.host_name);
    set_opt(hb, "User", &host.user);
    set_opt(hb, "Port", &host.port);
    match host.auth_method.as_str() {
        "password" => {
            set_opt(hb, "IdentityFile", "");
            set_opt(hb, "PreferredAuthentications", "password");
            set_opt(hb, "PubkeyAuthentication", "no");
            // The secret itself lives in the hosterm.secrets sidecar, never
            // in the config — real ssh must be able to parse every line.
            set_opt(hb, "Password", "");
            set_opt(hb, "IgnoreUnknown", "");
        }
        _ => {
            // Key auth (default): point at an IdentityFile and clear any
            // password-forcing directives left from a previous password setup.
            set_opt(hb, "IdentityFile", &host.identity_file);
            set_opt(hb, "PreferredAuthentications", "");
            set_opt(hb, "PubkeyAuthentication", "");
            set_opt(hb, "Password", "");
            set_opt(hb, "IgnoreUnknown", "");
        }
    }
}

/// Remove a deleted host's secret row, if any. Returns the io error if the
/// sidecar rewrite fails (EACCES, read-only FS, ENOSPC); a no-op row removal
/// touches no file and is always Ok.
fn purge_secret(alias: &str) -> std::io::Result<()> {
    let mut m = read_secrets();
    if m.remove(alias).is_some() {
        write_secrets(&m)?;
    }
    Ok(())
}

/// Persist (or clear) the secret for an alias in the sidecar. Called from the
/// save path, next to the config write, so the two stay in sync. Empty pw =
/// keep whatever is already stored (the UI never re-sends the secret).
fn store_secret_for_host(host: &HostInput) {
    if !host.password.is_empty() {
        set_secret(&host.name, &host.password);
    }
}

/// U8 D2: forget the stored sidecar password for an alias. The host block
/// stays; a later save with an empty password keeps whatever is stored
/// (the UI never re-sends the secret), so forgetting is the only way to drop
/// it without deleting the host. Idempotent: an unknown alias is a no-op.
#[tauri::command]
fn forget_secret(alias: String) -> Result<(), String> {
    let a = alias.as_str();
    if a.trim().is_empty() {
        return Ok(());
    }
    // The sidecar key is the host name verbatim (save_host stores `host.name`),
    // so drop that exact key; also drop the trimmed spelling in case the UI
    // sent a padded name. purge_secret only rewrites the file when a row was
    // actually removed → unknown alias is a true no-op. A real fs failure
    // (EACCES, read-only FS, ENOSPC) propagates so the UI never shows success
    // while the password is still on disk.
    purge_secret(a).map_err(|e| format!("Failed to remove stored password: {e}"))?;
    let t = a.trim();
    if t != a {
        purge_secret(t).map_err(|e| format!("Failed to remove stored password: {e}"))?;
    }
    Ok(())
}

/// The directory holding keys + config. Keys live beside the config so the
/// "copy the folder" portability story stays coherent.
fn ssh_dir() -> PathBuf {
    if let Ok(p) = std::env::var("HOSTERM_CONFIG") {
        if let Some(parent) = PathBuf::from(&p).parent() {
            return parent.to_path_buf();
        }
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".ssh")
}

// ---------- SSH config commands ----------

#[tauri::command]
fn read_ssh_config() -> Result<Vec<HostDto>, String> {
    let path = config_path();
    let text = read_config_text(&path)?;
    let mut cfg = parse_config(&text);
    // self-heal configs written by older versions: secret -> sidecar, config clean
    migrate_legacy_password_directives(&mut cfg);
    if serialize_config(&cfg) != text {
        let _ = write_config_file(&path, &serialize_config(&cfg));
    }
    let mut out = Vec::new();
    for h in &cfg.hosts {
        let name = h.patterns.first().cloned().unwrap_or_default();
        let options: Vec<(String, String)> = h
            .lines
            .iter()
            .filter_map(|l| match l {
                CfgLine::Opt { key, value } => Some((key.clone(), value.clone())),
                _ => None,
            })
            .collect();
        let prefers_password = get_opt(h, "PreferredAuthentications")
            .split(',')
            .next()
            .map(|s| s.trim().eq_ignore_ascii_case("password"))
            .unwrap_or(false);
        let pubkey_off = get_opt(h, "PubkeyAuthentication").eq_ignore_ascii_case("no");
        let auth_method = if prefers_password || pubkey_off {
            "password"
        } else {
            "key"
        }
        .to_string();
        let has_password = read_secrets()
            .contains_key(name.as_str())
            || !get_opt(h, "Password").is_empty();
        out.push(HostDto {
            name,
            host_name: get_opt(h, "HostName"),
            user: get_opt(h, "User"),
            port: get_opt(h, "Port"),
            identity_file: get_opt(h, "IdentityFile"),
            auth_method,
            has_password,
            options,
        });
    }
    Ok(out)
}

#[tauri::command]
fn save_host(original_name: Option<String>, host: HostInput) -> Result<(), String> {
    let alias = host.name.trim();
    if alias.is_empty() {
        return Err("Host alias cannot be empty".into());
    }
    // Reject aliases that OpenSSH would misread. A space makes it two patterns
    // (so `my box` silently becomes just `my`); a leading '-' makes `ssh`
    // treat the alias as an option (e.g. `-oProxyCommand=...`) instead of a
    // host — an injection vector. Both are closed here at the write boundary.
    if alias.split_whitespace().count() != 1 {
        return Err("Host alias cannot contain spaces".into());
    }
    if alias.starts_with('-') {
        return Err("Host alias cannot start with '-'".into());
    }
    let path = config_path();
    let text = read_config_text(&path)?;
    let mut cfg = parse_config(&text);

    match original_name {
        Some(orig) => {
            let hb = cfg
                .hosts
                .iter_mut()
                .find(|h| h.patterns.first().map(|p| p == &orig).unwrap_or(false))
                .ok_or_else(|| format!("Host '{}' not found", orig))?;
            if hb.patterns.is_empty() {
                hb.patterns.push(host.name.clone());
            } else {
                hb.patterns[0] = host.name.clone();
            }
            apply_host_fields(hb, &host);
            // renaming a host must carry its stored secret to the new alias
            if &orig != &host.name {
                let mut m = read_secrets();
                if let Some(pw) = m.remove(&orig) {
                    m.insert(host.name.clone(), pw);
                    let _ = write_secrets(&m);
                }
            }
            store_secret_for_host(&host);
        }
        None => {
            let mut hb = HostBlock {
                patterns: vec![host.name.clone()],
                lines: Vec::new(),
            };
            apply_host_fields(&mut hb, &host);
            store_secret_for_host(&host);
            cfg.hosts.push(hb);
        }
    }
    write_config_file(&path, &serialize_config(&cfg))
}

#[tauri::command]
fn delete_host(name: String) -> Result<(), String> {
    let path = config_path();
    let text = read_config_text(&path)?;
    let mut cfg = parse_config(&text);
    // Duplicate aliases are legal in ssh configs (ssh merges the blocks), and
    // hosterm lists each block as its own row. Removing ALL matching blocks
    // made one delete wipe every row with that name — remove exactly the
    // first one; a second delete removes the next.
    let pos = cfg
        .hosts
        .iter()
        .position(|h| h.patterns.first().map(|p| p == &name).unwrap_or(false));
    match pos {
        None => Err(format!("Host '{}' not found", name)),
        Some(i) => {
            cfg.hosts.remove(i);
            purge_secret(&name).map_err(|e| format!("Failed to remove stored password: {e}"))?;
    write_config_file(&path, &serialize_config(&cfg))
        }
    }
}

#[tauri::command]
fn read_config_raw() -> Result<String, String> {
    let path = config_path();
    let text = read_config_text(&path)?;
    let mut cfg = parse_config(&text);
    migrate_legacy_password_directives(&mut cfg);
    let fixed = serialize_config(&cfg);
    if fixed != text {
        let _ = write_config_file(&path, &fixed);
        return Ok(fixed);
    }
    Ok(text)
}

#[tauri::command]
fn write_config_raw(content: String) -> Result<(), String> {
    write_config_file(&config_path(), &content)
}

// ---------- identity key management ----------

/// Keys live in `<ssh_dir>/keys/` so ~/.ssh stays tidy (config, known_hosts,
/// agent sockets) while every key hosterm manages is in one folder — copying
/// that folder to a new device carries all keys with it.
fn keys_dir() -> PathBuf {
    let mut d = ssh_dir();
    d.push("keys");
    d
}

/// List private-key files in ~/.ssh/keys, sorted by name. Returns the
/// `~/.ssh/keys/<name>` path form that goes straight into an IdentityFile
/// directive. Legacy keys still in ~/.ssh are listed too (read-only legacy
/// support); new keys are created in keys/ only.
#[tauri::command]
fn list_identity_files() -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    let keys = keys_dir();
    let legacy = ssh_dir();
    for (dir, prefix) in [(keys.clone(), "~/.ssh/keys/"), (legacy.clone(), "~/.ssh/")] {
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.to_string()),
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            // skip public keys, known_hosts, config, authorized_keys and temp files
            if name.ends_with(".pub")
                || name == "config"
                || name == "known_hosts"
                || name == "known_hosts.old"
                || name == "authorized_keys"
                || name.starts_with('.')
            {
                continue;
            }
            // treat as a key if the first line looks like a private key header,
            // OR a matching <name>.pub exists next to it
            let looks_private = std::fs::read_to_string(&path)
                .map(|c| c.lines().next().map(|l| l.contains("PRIVATE KEY")).unwrap_or(false))
                .unwrap_or(false);
            let has_pub = dir.join(format!("{}.pub", name)).exists();
            if looks_private || has_pub {
                out.push(format!("{}{}", prefix, name));
            }
        }
        if dir == legacy {
            // harden the keys/ dir once it exists
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if keys.is_dir() {
                    let _ = std::fs::set_permissions(&keys, std::fs::Permissions::from_mode(0o700));
                }
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Write a pasted private key into ~/.ssh/keys at 0600 and return the
/// `~/.ssh/keys/<name>` path to drop into an IdentityFile directive.
/// Termius-style: the user pastes key text, hosterm persists it as a real
/// file (SSH needs a file on disk; there is no in-config key storage).
#[tauri::command]
fn create_identity_file(name: String, private_key: String) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Key name cannot be empty".into());
    }
    // keep it a bare filename — no path separators, no traversal
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err("Key name must be a plain filename (no path separators)".into());
    }
    if private_key.trim().is_empty() {
        return Err("Private key text is empty".into());
    }
    let dir = keys_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    let path = dir.join(name);
    if path.exists() {
        return Err(format!("A key named '{}' already exists", name));
    }
    // normalize to LF and guarantee a trailing newline (ssh is picky)
    let mut body = private_key.replace("\r\n", "\n");
    if !body.ends_with('\n') {
        body.push('\n');
    }
    std::fs::write(&path, body).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    Ok(format!("~/.ssh/keys/{}", name))
}

/// Delete a key hosterm manages. Scoped to `~/.ssh/keys/` ONLY: a bare filename
/// with no path separators, and the resolved path must still sit inside
/// keys_dir() (defence in depth against traversal). Legacy keys in ~/.ssh are
/// intentionally NOT deletable here — we never want to remove a user's primary
/// id_ed25519/id_rsa by accident. A matching `<name>.pub` is removed too.
#[tauri::command]
fn delete_identity_file(name: String) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Key name cannot be empty".into());
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err("Key name must be a plain filename (no path separators)".into());
    }
    let dir = keys_dir();
    let path = dir.join(name);
    // Defence in depth: the resolved parent must be exactly keys_dir().
    if path.parent() != Some(dir.as_path()) {
        return Err("Refusing to delete outside the managed keys directory".into());
    }
    if !path.exists() {
        return Err(format!("No key named '{}' in ~/.ssh/keys", name));
    }
    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    // Remove the public half if present; its absence is not an error.
    let pub_path = dir.join(format!("{}.pub", name));
    if pub_path.exists() {
        let _ = std::fs::remove_file(&pub_path);
    }
    Ok(())
}

// ============================================================
//  STORE  (settings + allowlist JSON in app-data; bundled snippets)
// ============================================================
//
// U-D3 (SDD §13.4/§13.5/§13.7, ADR-18/19/21). One app-data JSON file for
// settings+allowlist, one read-only bundled JSON for default snippets.
// Additive and isolated: this path NEVER reads or writes ~/.ssh/config or the
// hosterm.secrets sidecar (INV-22). Resolution mirrors config_path()'s posture
// — the `dirs` crate already in the tree, no tauri-plugin-store/-fs.

/// AI exact-command approval entry. `(host, command)` is byte-exact (no
/// pattern/glob in v1); `added_at` is backend-stamped RFC 3339 UTC, audit-only.
#[derive(Serialize, Deserialize, Clone, PartialEq)]
struct AllowEntry {
    host: String,
    command: String,
    #[serde(default)]
    added_at: String,
}

/// Persisted appearance + allowlist. `read_settings` never throws: a missing or
/// corrupt file yields DEFAULT_SETTINGS and the backend coerces unknown
/// theme/version and invalid accent to defaults.
#[derive(Serialize, Deserialize, Clone)]
struct Settings {
    version: u32,
    theme: String,
    accent: String,
    allowlist: Vec<AllowEntry>,
}

const DEFAULT_THEME: &str = "dusk";
const DEFAULT_ACCENT: &str = "#e2a35c";
const SETTINGS_VERSION: u32 = 1;

fn default_settings() -> Settings {
    Settings {
        version: SETTINGS_VERSION,
        theme: DEFAULT_THEME.to_string(),
        accent: DEFAULT_ACCENT.to_string(),
        allowlist: Vec::new(),
    }
}

/// One bundled snippet (name + command).
#[derive(Serialize, Deserialize, Clone)]
struct Snippet {
    name: String,
    command: String,
}

/// A category of bundled snippets.
#[derive(Serialize, Deserialize, Clone)]
struct SnippetGroup {
    category: String,
    items: Vec<Snippet>,
}

/// Default snippet set, embedded at compile time (ADR-19, zero network).
/// Path is relative to this source file: lib.rs lives in src-tauri/src/, the
/// asset in src/assets/ at the repo root.
const SNIPPETS_DEFAULT_JSON: &str = include_str!("../../src/assets/snippets.default.json");

/// Directory holding the app-data store. `HOSTERM_DATA_DIR` overrides the
/// parent dir (test hook, parity with HOSTERM_CONFIG); otherwise use the OS
/// per-user config dir. The store always lives under a `hosterm/` subdir so it
/// matches the shipped `dirs::config_dir()/hosterm/` layout (§13.4) and so a
/// temp override dir only ever gains that one subdir.
fn data_dir() -> PathBuf {
    let base = if let Ok(p) = std::env::var("HOSTERM_DATA_DIR") {
        PathBuf::from(p)
    } else {
        dirs::config_dir().unwrap_or_else(|| PathBuf::from("."))
    };
    base.join("hosterm")
}

fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

/// Accent must be exactly `#RRGGBB` (6 hex digits). Anything else is invalid.
fn accent_is_valid(accent: &str) -> bool {
    let b = accent.as_bytes();
    b.len() == 7 && b[0] == b'#' && b[1..].iter().all(|c| c.is_ascii_hexdigit())
}

/// The four known theme presets. Unknown values coerce to "dusk" on read and
/// are rejected on write (`settings:invalid-theme`).
fn theme_is_valid(theme: &str) -> bool {
    matches!(theme, "dusk" | "slate" | "ink" | "light")
}

/// Coerce a possibly-foreign deserialized Settings into a safe value for the
/// read path: unknown theme/version → dusk/v1, invalid accent → preset default,
/// allowlist entries with empty host/command dropped. Never throws.
fn coerce_settings(mut s: Settings) -> Settings {
    if s.version != SETTINGS_VERSION {
        s.version = SETTINGS_VERSION;
    }
    if !theme_is_valid(&s.theme) {
        s.theme = DEFAULT_THEME.to_string();
    }
    if !accent_is_valid(&s.accent) {
        s.accent = DEFAULT_ACCENT.to_string();
    }
    s.allowlist
        .retain(|e| !e.host.trim().is_empty() && !e.command.trim().is_empty());
    s
}

/// Read persisted settings. NEVER throws (INV-24): a missing file, an I/O
/// error, or unparseable JSON all yield DEFAULT_SETTINGS, and the corrupt file
/// is left on disk untouched (no destructive self-heal on read).
#[tauri::command]
fn read_settings() -> Settings {
    match std::fs::read_to_string(settings_path()) {
        Ok(text) => match serde_json::from_str::<Settings>(&text) {
            Ok(s) => coerce_settings(s),
            Err(_) => default_settings(),
        },
        Err(_) => default_settings(),
    }
}

/// Persist settings atomically (temp → rename, 0600-class). Re-validates theme
/// enum, accent 6-hex, and allowlist entries (non-empty host+command) and
/// rejects with the frozen `settings:*` strings WITHOUT writing on any failure.
/// `added_at` is backend-stamped RFC 3339 UTC. NEVER touches ~/.ssh/config or
/// the sidecar (INV-22).
#[tauri::command]
fn write_settings(settings: Settings) -> Result<(), String> {
    if !theme_is_valid(&settings.theme) {
        return Err("settings:invalid-theme".into());
    }
    if !accent_is_valid(&settings.accent) {
        return Err("settings:invalid-accent".into());
    }
    let mut out = Settings {
        version: SETTINGS_VERSION,
        theme: settings.theme,
        accent: settings.accent,
        allowlist: Vec::with_capacity(settings.allowlist.len()),
    };
    for e in settings.allowlist {
        if e.host.trim().is_empty() || e.command.trim().is_empty() {
            return Err("settings:invalid-accent".into());
        }
        // added_at is backend-authoritative: stamp fresh entries, keep an
        // existing RFC3339-shaped value so re-saving the list is stable.
        let added_at = if e.added_at.trim().is_empty() {
            now_rfc3339_utc()
        } else {
            e.added_at
        };
        out.allowlist.push(AllowEntry {
            host: e.host,
            command: e.command,
            added_at,
        });
    }
    let body = serde_json::to_string_pretty(&out).map_err(|_| "settings:write-failed".to_string())?;
    write_store_atomic(&settings_path(), &body).map_err(|_| "settings:write-failed".to_string())
}

/// Return the bundled read-only default snippet set (ADR-19). NEVER throws:
/// an unparseable bundled asset yields an empty array rather than an error.
#[tauri::command]
fn read_snippets() -> Vec<SnippetGroup> {
    serde_json::from_str::<Vec<SnippetGroup>>(SNIPPETS_DEFAULT_JSON).unwrap_or_default()
}

/// A user-created snippet (CRUD). Unlike the bundled set (read-only, ADR-19),
/// these live in an app-data JSON file and can be added/edited/deleted. `id` is
/// a backend-assigned stable key so edit/delete target one row unambiguously.
#[derive(Serialize, Deserialize, Clone)]
struct UserSnippet {
    id: String,
    name: String,
    command: String,
    #[serde(default)]
    category: String,
}

fn user_snippets_path() -> PathBuf {
    data_dir().join("snippets.user.json")
}

/// Read the user's CRUD snippet store. NEVER throws: a missing or corrupt file
/// yields an empty list (same posture as read_settings). This path never reads
/// ~/.ssh/config or the secrets sidecar (INV-22).
#[tauri::command]
fn read_user_snippets() -> Vec<UserSnippet> {
    match std::fs::read_to_string(user_snippets_path()) {
        Ok(s) => serde_json::from_str::<Vec<UserSnippet>>(&s).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Persist the full user snippet list atomically. Validates each row
/// (name + command non-empty after trim) and backend-assigns an id to any row
/// that arrives without one, so the frontend can send a new row with id="".
#[tauri::command]
fn write_user_snippets(snippets: Vec<UserSnippet>) -> Result<(), String> {
    let mut out: Vec<UserSnippet> = Vec::with_capacity(snippets.len());
    for s in snippets {
        if s.name.trim().is_empty() {
            return Err("snippets:empty-name".into());
        }
        if s.command.trim().is_empty() {
            return Err("snippets:empty-command".into());
        }
        let id = if s.id.trim().is_empty() {
            format!("us-{}", now_rfc3339_utc().replace([':', '.', '-'], "") )
                + &format!("-{}", out.len())
        } else {
            s.id
        };
        out.push(UserSnippet {
            id,
            name: s.name.trim().to_string(),
            command: s.command.trim().to_string(),
            category: {
                let c = s.category.trim();
                if c.is_empty() { "Custom".to_string() } else { c.to_string() }
            },
        });
    }
    let body = serde_json::to_string_pretty(&out).map_err(|_| "snippets:write-failed".to_string())?;
    write_store_atomic(&user_snippets_path(), &body).map_err(|_| "snippets:write-failed".to_string())
}

/// Atomic whole-file write for the store (sibling temp → rename), 0600-class
/// where the OS supports it. Same discipline as write_config_file but scoped to
/// the app-data dir so it can never be confused with the ssh config path.
fn write_store_atomic(path: &PathBuf, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("settings.json");
        let mut t = path.clone();
        t.set_file_name(format!(".{}.hosterm-tmp-{}", name, std::process::id()));
        t
    };
    std::fs::write(&tmp, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e
    })?;
    Ok(())
}

/// Current time as an RFC 3339 UTC string (`YYYY-MM-DDTHH:MM:SSZ`), computed
/// from the Unix epoch with no external date crate (NFR: zero new deps). Used
/// only to stamp `added_at` for display/audit, so second precision is enough.
fn now_rfc3339_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (hh, mm, ss) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hh, mm, ss
    )
}

/// Convert a count of days since 1970-01-01 into (year, month, day) using
/// Howard Hinnant's well-known civil-from-days algorithm (public domain).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32)
}

// ============================================================
//  SFTP  (drives the real OS `sftp` binary, honors ssh config)
// ============================================================
//
// Design: we spawn `sftp -b` (batch mode) per operation rather than linking an
// SSH library. The real binary resolves ~/.ssh/config exactly like `ssh` does
// (ProxyJump, Match, IdentityFile) — the whole point of hosterm. Batch mode is
// non-interactive, so this path covers KEY/AGENT auth; password-auth hosts need
// the persistent-session variant (a later increment).

#[derive(Serialize)]
struct LocalEntryDto {
    name: String,
    is_dir: bool,
    size: u64,
}

#[derive(Serialize)]
struct SftpEntry {
    name: String,
    is_dir: bool,
    is_link: bool,
    size: u64,
}

/// Guard a value that becomes an argv element or a batch-command token.
/// A leading '-' would be read by sftp as an option; a newline would inject a
/// second batch command. Both are rejected.
pub(crate) fn sftp_safe(value: &str, what: &str) -> Result<(), String> {
    if value.starts_with('-') {
        return Err(format!("{} cannot start with '-'", what));
    }
    if value.contains('\n') || value.contains('\r') {
        return Err(format!("{} cannot contain newlines", what));
    }
    Ok(())
}

/// Parse `sftp` long-listing (`ls -la`) output into entries. Pure function so
/// it is unit-testable without a server. Skips the `total N` header, command
/// echoes, blank lines, and the `.`/`..` entries.
fn parse_sftp_ls(output: &str) -> Vec<SftpEntry> {
    let mut out = Vec::new();
    for line in output.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with("sftp>") || line.starts_with("total ") {
            continue;
        }
        let perms = match line.split_whitespace().next() {
            Some(p) => p,
            None => continue,
        };
        let first = match perms.chars().next() {
            Some(c) => c,
            None => continue,
        };
        // only accept lines whose first token looks like a mode string
        if !matches!(first, 'd' | '-' | 'l' | 'b' | 'c' | 'p' | 's') || perms.len() < 10 {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 9 {
            continue;
        }
        let size = fields.get(4).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        // name = everything after the 8th field (handles spaces in names);
        // for symlinks ls appends " -> target" which we trim off.
        let mut name = fields[8..].join(" ");
        if first == 'l' {
            if let Some(idx) = name.find(" -> ") {
                name.truncate(idx);
            }
        }
        // Defensive basename normalization: some servers print names with a
        // "./" prefix or a full/relative path in `ls -la` output (differs from
        // the bare-name fixtures). The frontend joins dir + name, so a
        // path-bearing name produces garbage like `/home//home/ubuntu`. Always
        // emit the final path segment so the join is correct server-agnostic.
        let normalized = name.rsplit('/').next().unwrap_or(&name).to_string();
        name = normalized;
        if name == "." || name == ".." || name.is_empty() {
            continue;
        }
        out.push(SftpEntry {
            name,
            is_dir: first == 'd',
            is_link: first == 'l',
            size,
        });
    }
    out
}

/// Run an sftp batch script against `host`, return (stdout, stderr, success).
fn run_sftp_batch(host: &str, script: &str) -> Result<(String, String, bool), String> {
    use std::process::{Command, Stdio};
    sftp_safe(host, "Host")?;
    // HOSTERM_SFTP_BIN: testability hook (e.g. point at a -vvv wrapper)
    let bin = std::env::var("HOSTERM_SFTP_BIN").unwrap_or_else(|_| "sftp".to_string());
    let mut cmd = Command::new(bin);
    cmd.arg("-o")
        .arg("BatchMode=yes")
        .arg("-b")
        .arg("-") // read batch commands from stdin
        .arg(host)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // sftp.exe is a console program: without CREATE_NO_WINDOW a console
    // window flashes on screen on every operation on Windows.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to launch sftp: {}", e))?;
    {
        let stdin = child.stdin.as_mut().ok_or("no stdin")?;
        stdin.write_all(script.as_bytes()).map_err(|e| e.to_string())?;
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    Ok((
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.success(),
    ))
}

// ---------- persistent sftp session (password-auth hosts) ----------
//
// ONE interactive `sftp` process per host, kept alive in a PTY and reused by
// every sftp command/tab for that host (keyed by alias). The stored secret is
// auto-typed once when OpenSSH's password prompt appears (same one-shot
// contract as the terminal). Interactive sftp has NO per-command exit codes,
// so completion = the next "sftp> " prompt and `put` is verified by a
// follow-up listing (download already verifies the local file exists).
// Batch mode (run_sftp_batch) stays for key/agent hosts: real exit codes.
//
// The PTY master reader BLOCKS, so a pump thread drains it into a shared
// buffer; waiters poll that buffer with a real deadline.

struct SftpBuf {
    data: Mutex<Vec<u8>>,
    eof: AtomicBool,
}

impl SftpBuf {
    /// Wait until `needle` appears; return everything up to and including it.
    /// When `auto_pw` is set, types the password once at OpenSSH's password
    /// prompt. Returns None on timeout or EOF-without-match.
    fn wait_for(
        &self,
        needle: &str,
        writer: Option<&Arc<Mutex<Box<dyn Write + Send>>>>,
        auto_pw: Option<&str>,
        timeout: std::time::Duration,
    ) -> Option<String> {
        let deadline = std::time::Instant::now() + timeout;
        let mut injected = false;
        loop {
            let matched = {
                let mut d = self.data.lock().unwrap();
                let s = String::from_utf8_lossy(&d).to_string();
                match s.find(needle) {
                    Some(pos) => {
                        let consumed = s[..pos + needle.len()].to_string();
                        d.drain(..pos + needle.len());
                        Some(consumed)
                    }
                    None => {
                        if d.len() > 1024 * 1024 {
                            let cut = d.len() - 64 * 1024; // runaway output: keep the tail
                            d.drain(..cut);
                        }
                        None
                    }
                }
            };
            if matched.is_some() {
                return matched;
            }
            if let (Some(pw), Some(w)) = (auto_pw, writer) {
                if !injected {
                    let d = self.data.lock().unwrap();
                    if String::from_utf8_lossy(&d).contains("'s password:") {
                        injected = true;
                        let mut w = w.lock().unwrap();
                        let _ = w.write_all(pw.as_bytes());
                        let _ = w.write_all(b"\r");
                        let _ = w.flush();
                    }
                }
            }
            if self.eof.load(Ordering::SeqCst) {
                return None;
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

struct SftpPtySession {
    /// kept for future explicit teardown (kill on logout); the alive flag
    /// currently governs respawns
    #[allow(dead_code)]
    master: Box<dyn MasterPty + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    rx: Arc<SftpBuf>,
    alive: Arc<AtomicBool>,
}

struct SftpBatchOutput {
    stdout: String,
    stderr: String,
    ok: bool,
}

fn sftp_sessions() -> &'static std::sync::Mutex<HashMap<String, Arc<Mutex<SftpPtySession>>>> {
    static SESSIONS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Arc<Mutex<SftpPtySession>>>>> =
        std::sync::OnceLock::new();
    SESSIONS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

/// Spawn one interactive `sftp` in a PTY and wait for its prompt, typing the
/// stored password once if the host asks for it.
fn sftp_spawn(host: &str) -> Result<SftpPtySession, String> {
    sftp_safe(host, "Host")?;
    // HOSTERM_SFTP_BIN: testability hook, same as the batch path
    let bin = std::env::var("HOSTERM_SFTP_BIN").unwrap_or_else(|_| "sftp".to_string());
    let mut cmd = CommandBuilder::new(bin);
    cmd.arg(host);
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| format!("cannot open pty for sftp: {}", e))?;
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("failed to launch sftp: {}", e))?;
    drop(pair.slave);
    let reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = Arc::new(Mutex::new(pair.master.take_writer().map_err(|e| e.to_string())?));
    let alive = Arc::new(AtomicBool::new(true));

    // pump: the master reader blocks, so drain it into the buffer forever
    let rx = Arc::new(SftpBuf { data: Mutex::new(Vec::new()), eof: AtomicBool::new(false) });
    let rx2 = Arc::clone(&rx);
    thread::spawn(move || {
        let mut r = reader;
        let mut buf = [0u8; 4096];
        loop {
            match r.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => rx2.data.lock().unwrap().extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
        rx2.eof.store(true, Ordering::SeqCst);
    });

    // reap + liveness watcher: marks the session dead when sftp exits
    let alive2 = Arc::clone(&alive);
    thread::spawn(move || {
        let _ = child.wait();
        alive2.store(false, Ordering::SeqCst);
    });

    let mut sess = SftpPtySession { master: pair.master, writer, rx, alive };
    match lookup_stored_password(host).as_deref() {
        Some(pw) => {
            let ready = sess.rx.wait_for(
                "sftp>",
                Some(&sess.writer),
                Some(pw),
                std::time::Duration::from_secs(30),
            );
            if ready.is_none() {
                sess.alive.store(false, Ordering::SeqCst);
                return Err("sftp: no prompt after login (wrong password or host unreachable)".into());
            }
        }
        None => {
            let _ = sess.rx.wait_for(
                "sftp>",
                Some(&sess.writer),
                None,
                std::time::Duration::from_secs(10),
            );
        }
    }
    Ok(sess)
}

/// Run one command in the persistent session for `host`. Done = the next
/// prompt. A dead or timed-out session is discarded and the command is
/// retried once on a fresh login. (The prompt typed by the TTY driver is
/// already consumed at login; the next one only appears after the command.)
fn sftp_exec(host: &str, cmdline: &str) -> SftpBatchOutput {
    for _attempt in 0..2 {
        let sess = {
            let mut map = sftp_sessions().lock().unwrap();
            let live = map
                .get(host)
                .filter(|s| s.lock().unwrap().alive.load(Ordering::SeqCst))
                .is_some();
            if !live {
                match sftp_spawn(host) {
                    Ok(s) => {
                        map.insert(host.to_string(), Arc::new(Mutex::new(s)));
                    }
                    Err(e) => return SftpBatchOutput { stdout: String::new(), stderr: e, ok: false },
                }
            }
            Arc::clone(map.get(host).unwrap())
        };
        let mut g = sess.lock().unwrap();
        {
            let mut w = g.writer.lock().unwrap();
            if w.write_all(cmdline.as_bytes()).is_err()
                || w.write_all(b"\n").is_err()
                || w.flush().is_err()
            {
                g.alive.store(false, Ordering::SeqCst);
                continue; // session died: respawn and retry
            }
        }
        match g.rx.wait_for("\nsftp> ", None, None, std::time::Duration::from_secs(120)) {
            None => {
                g.alive.store(false, Ordering::SeqCst);
                continue; // EOF or timeout: respawn and retry
            }
            Some(raw) => {
                // strip the trailing prompt; parse_sftp_ls ignores the
                // echoed command line and "sftp> " noise by itself
                let out = raw
                    .strip_suffix("\nsftp> ")
                    .unwrap_or(&raw)
                    .trim()
                    .to_string();
                return SftpBatchOutput { stdout: out, stderr: String::new(), ok: true };
            }
        }
    }
    SftpBatchOutput { stdout: String::new(), stderr: "sftp session unreachable".into(), ok: false }
}

/// Verify that `name` exists in the listing `raw` (post-`put` guard so a
/// failed upload on the interactive path is not reported as success).
fn sftp_listing_has(raw: &str, name: &str) -> bool {
    parse_sftp_ls(raw).iter().any(|e| e.name == name)
}

/// Run one sftp operation: persistent PTY session for password-auth hosts
/// (secret stored in the sidecar), one-shot batch otherwise.
fn run_sftp_op(host: &str, script: &str) -> SftpBatchOutput {
    if lookup_stored_password(host).is_none() {
        return match run_sftp_batch(host, script) {
            Ok((stdout, stderr, ok)) => SftpBatchOutput { stdout, stderr, ok },
            Err(e) => SftpBatchOutput { stdout: String::new(), stderr: e, ok: false },
        };
    }
    let mut outs: Vec<String> = Vec::new();
    let mut err = String::new();
    let mut ok_all = true;
    let mut lines: Vec<&str> = script.lines().filter(|l| !l.trim().is_empty()).collect();
    // `put` has no exit code on the interactive path: verify with a listing
    let verify_put: Option<(String, String)> = lines.iter().find_map(|l| {
        let rest = l.trim().strip_prefix("put ")?;
        let target = rest.split("\" \"").last()?.trim_matches('"');
        let name = target.rsplit('/').find(|s| !s.is_empty())?.to_string();
        Some((target.trim_end_matches('/').to_string(), name))
    });
    let verify_cmd;
    if let Some((target, _)) = &verify_put {
        verify_cmd = format!("ls -la \"{}\"", target);
        lines.push(&verify_cmd);
    }
    let mut i = 0;
    while i < lines.len() {
        let r = sftp_exec(host, lines[i]);
        if !r.ok {
            ok_all = false;
            err = if r.stderr.is_empty() {
                format!("sftp command failed: {}", lines[i])
            } else {
                r.stderr
            };
            break;
        }
        // the final round is our own post-put verification listing
        let is_verify = verify_put.is_some() && i == lines.len() - 1;
        if is_verify {
            let (_, name) = verify_put.as_ref().unwrap();
            if !sftp_listing_has(&r.stdout, name) {
                ok_all = false;
                err = format!("upload verification failed: {} not found in {}", name, target_of(&lines[i]));
            }
        } else {
            outs.push(r.stdout);
        }
        i += 1;
    }
    SftpBatchOutput { stdout: outs.join("\n"), stderr: err, ok: ok_all }
}

fn target_of(cmdline: &str) -> String {
    cmdline
        .trim()
        .trim_start_matches("ls -la ")
        .trim_matches('"')
        .to_string()
}

// The SFTP commands below are `async` so Tauri releases the IPC thread the
// instant they're invoked; the blocking PTY work (including sftp_spawn's up-to
// 30s password-login wait and its timeout/mismatch path) runs on the blocking
// thread pool via spawn_blocking, so the WebView never freezes. The JS-visible
// contract (name, args, Result/error strings) is unchanged — async commands
// serialize to JS identically to sync ones.
#[tauri::command]
async fn sftp_list(host: String, path: String) -> Result<Vec<SftpEntry>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = if path.trim().is_empty() { ".".to_string() } else { path };
        sftp_safe(&dir, "Path")?;
        let script = format!("ls -la {}\n", dir);
        let output = run_sftp_op(&host, &script);
        if !output.ok {
            return Err(format!("sftp ls failed: {}", output.stderr.trim()));
        }
        Ok(parse_sftp_ls(&output.stdout))
    })
    .await
    .map_err(|e| format!("sftp task failed: {}", e))?
}

#[tauri::command]
fn home_dir() -> String {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default()
}

/// Clipboard via the host process, not the webview: WebView2 shows a
/// permission prompt ("wants to see text and images copied to the
/// clipboard") for navigator.clipboard reads, which breaks the
/// right-click-to-paste flow. The OS clipboard is shared, so this is the
/// same content — no webview clipboard permission involved.
#[tauri::command]
fn clipboard_write(text: String) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(text))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn clipboard_read() -> Result<String, String> {
    arboard::Clipboard::new()
        .and_then(|mut c| c.get_text())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn local_list(path: String) -> Result<Vec<LocalEntryDto>, String> {
    if path.trim().is_empty() {
        return Err("path is empty".into());
    }
    let dir = PathBuf::from(&path);
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {}", path, e))?;
    let mut out: Vec<LocalEntryDto> = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue; // keep the pane calm; same rule as the remote pane
        }
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let size = e.metadata().map(|m| m.len()).unwrap_or(0);
        out.push(LocalEntryDto { name, is_dir, size });
    }
    out.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(out)
}

#[tauri::command]
async fn sftp_upload(host: String, local_path: String, remote_path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        sftp_safe(&local_path, "Local path")?;
        sftp_safe(&remote_path, "Remote path")?;
        let local = PathBuf::from(&local_path);
        if !local.is_file() {
            return Err("local file does not exist".into());
        }
        // Aliasing guard: batch-mode `put` truncates its own source when the local
        // and remote paths resolve to the same file (same-machine sftp). Refuse
        // instead of destroying the file.
        let canon_local = local.canonicalize().map_err(|e| e.to_string())?;
        if remote_path == canon_local.display().to_string()
            || PathBuf::from(&remote_path).canonicalize().map(|p| p == canon_local).unwrap_or(false)
        {
            return Err("local and remote path are the same file — refusing to overwrite the source".into());
        }
        let script = format!("put \"{}\" \"{}\"\n", local_path, remote_path);
        let output = run_sftp_op(&host, &script);
        if !output.ok {
            return Err(format!("upload failed: {}", output.stderr.trim()));
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("sftp task failed: {}", e))?
}

#[tauri::command]
async fn sftp_download(host: String, remote_path: String, local_path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        sftp_safe(&remote_path, "Remote path")?;
        sftp_safe(&local_path, "Local path")?;
        // Aliasing guard (see sftp_upload): `get` would truncate the remote source.
        if let Ok(canon_local) = PathBuf::from(&local_path).canonicalize() {
            if remote_path == canon_local.display().to_string() {
                return Err("local and remote path are the same file — refusing to overwrite the source".into());
            }
        }
        let script = format!("get \"{}\" \"{}\"\n", remote_path, local_path);
        let output = run_sftp_op(&host, &script);
        if !output.ok {
            return Err(format!("download failed: {}", output.stderr.trim()));
        }
        if !PathBuf::from(&local_path).exists() {
            return Err("download reported success but local file is missing".into());
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("sftp task failed: {}", e))?
}

// ---------- remote file editor (text over sftp) ----------

const EDITOR_MAX_BYTES: u64 = 2 * 1024 * 1024; // refuse >2 MB in the text editor

fn editor_temp(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "hosterm-edit-{}-{}-{}",
        std::process::id(), n, tag
    ))
}

#[tauri::command]
async fn sftp_read_file(host: String, remote_path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        sftp_safe(&remote_path, "Remote path")?;
        let tmp = editor_temp("get");
        let script = format!("get \"{}\" \"{}\"\n", remote_path, tmp.display());
        let output = run_sftp_op(&host, &script);
        if !output.ok {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("read failed: {}", output.stderr.trim()));
        }
        let meta = std::fs::metadata(&tmp).map_err(|e| format!("downloaded file missing: {}", e))?;
        if meta.len() > EDITOR_MAX_BYTES {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!(
                "file is {} bytes; text editor limit is {} bytes (use SFTP download instead)",
                meta.len(),
                EDITOR_MAX_BYTES
            ));
        }
        let bytes = std::fs::read(&tmp).map_err(|e| e.to_string());
        let _ = std::fs::remove_file(&tmp);
        let bytes = bytes?;
        if bytes.contains(&0) {
            return Err("file looks binary — open it via SFTP download instead".into());
        }
        Ok(String::from_utf8_lossy(&bytes).to_string())
    })
    .await
    .map_err(|e| format!("sftp task failed: {}", e))?
}

#[tauri::command]
async fn sftp_write_file(host: String, remote_path: String, content: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        sftp_safe(&remote_path, "Remote path")?;
        if content.len() as u64 > EDITOR_MAX_BYTES {
            return Err("content exceeds the text editor size limit".into());
        }
        let tmp = editor_temp("put");
        std::fs::write(&tmp, content.as_bytes()).map_err(|e| e.to_string())?;
        let script = format!("put \"{}\" \"{}\"\n", tmp.display(), remote_path);
        let result = run_sftp_op(&host, &script);
        let _ = std::fs::remove_file(&tmp);
        if !result.ok {
            return Err(format!("write failed: {}", result.stderr.trim()));
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("sftp task failed: {}", e))?
}

// ============================================================
//  PTY ENGINE  (spawns the real OS `ssh`, honors ssh config)
// ============================================================

struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
}

#[derive(Default)]
struct PtyState {
    sessions: Mutex<HashMap<u32, PtySession>>,
}

/// Pump a PTY: forward output chunks through `on_chunk`, watching for
/// OpenSSH's password prompt when `auto_pw` is set, injecting the secret
/// once into `writer`. Shared by pty_spawn and the live test module.
fn pump_pty(
    mut reader: Box<dyn std::io::Read + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    auto_pw: Option<String>,
    mut on_chunk: impl FnMut(&str),
) {
    let mut buf = [0u8; 4096];
    let mut tail: Vec<u8> = Vec::new();
    let mut injected = false;
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let s = String::from_utf8_lossy(&buf[..n]).to_string();
                if let Some(pw) = auto_pw.as_deref() {
                    if !injected {
                        tail.extend_from_slice(&buf[..n]);
                        if tail.len() > 8192 {
                            tail.drain(..tail.len() - 512);
                        }
                        if String::from_utf8_lossy(&tail).contains("'s password:") {
                            injected = true;
                            let mut w = writer.lock().unwrap();
                            let _ = w.write_all(pw.as_bytes());
                            let _ = w.write_all(b"\r");
                            let _ = w.flush();
                        }
                    }
                }
                on_chunk(&s);
            }
        }
    }
}

static NEXT_ID: AtomicU32 = AtomicU32::new(1);

/// Stored password for a host alias, from its `Password` directive.
/// Stays inside the Rust process — never crosses the IPC boundary.
fn lookup_stored_password(host: &str) -> Option<String> {
    read_secrets().remove(host).filter(|p| !p.is_empty())
}

#[tauri::command]
fn pty_spawn(
    app: AppHandle,
    state: State<PtyState>,
    host: String,
    cols: u16,
    rows: u16,
) -> Result<u32, String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())?;

    let mut cmd = CommandBuilder::new("ssh");
    cmd.arg(&host);

    let mut child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    drop(pair.slave);

    let reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);

    // password prompt auto-fill: when a stored password exists for this host,
    // the reader loop below watches for OpenSSH's prompt and injects the
    // secret once through the shared writer.
    let auto_pw = lookup_stored_password(&host);
    let writer = Arc::new(Mutex::new(writer));

    // reader -> frontend events (+ one-shot password injection)
    let app_r = app.clone();
    let w_inject = writer.clone();
    let auto_pw2 = auto_pw.clone();
    thread::spawn(move || {
        pump_pty(reader, w_inject, auto_pw2, |s| {
            let _ = app_r.emit(&format!("pty://output/{}", id), s.to_string());
        });
        let _ = app_r.emit(&format!("pty://exit/{}", id), ());
    });

    state
        .sessions
        .lock()
        .unwrap()
        .insert(id, PtySession { master: pair.master, writer });

    // reap the child so it does not become a zombie
    thread::spawn(move || {
        let _ = child.wait();
    });

    Ok(id)
}

/// Default interactive login shell for the current OS. A local terminal spawns
/// this shell DIRECTLY as a PTY child (like pty_spawn does for `ssh`); no user
/// string is ever interpolated into `sh -c`/`cmd /C`, so this adds no local
/// command-injection surface (INV-14 governs the fleet command path, not an
/// interactive PTY shell the user types into).
fn default_local_shell() -> String {
    // Explicit override wins, for tests and power users.
    if let Ok(s) = std::env::var("HOSTERM_LOCAL_SHELL") {
        if !s.is_empty() {
            return s;
        }
    }
    #[cfg(windows)]
    {
        // Prefer PowerShell if present, else fall back to cmd via COMSPEC.
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    }
}

/// Info for the Settings → General pane: which shell a local terminal will
/// launch, and whether an explicit HOSTERM_LOCAL_SHELL override is in effect.
/// Read-only; the override is an env var, not a persisted setting.
#[derive(Serialize)]
struct LocalShellInfo {
    shell: String,
    overridden: bool,
}

#[tauri::command]
fn local_shell_info() -> LocalShellInfo {
    let overridden = std::env::var("HOSTERM_LOCAL_SHELL").map(|s| !s.is_empty()).unwrap_or(false);
    LocalShellInfo {
        shell: default_local_shell(),
        overridden,
    }
}

/// Spawn the OS default shell as a local interactive PTY (Termius-style local
/// terminal). Mirrors pty_spawn but runs a local shell instead of `ssh`, with
/// no host and no password auto-fill.
#[tauri::command]
fn local_pty_spawn(
    app: AppHandle,
    state: State<PtyState>,
    cols: u16,
    rows: u16,
) -> Result<u32, String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())?;

    let shell = default_local_shell();
    let mut cmd = CommandBuilder::new(&shell);
    // Start the shell in the user's home directory, like a fresh terminal.
    if let Some(home) = std::env::var("USERPROFILE")
        .ok()
        .or_else(|| std::env::var("HOME").ok())
    {
        if !home.is_empty() {
            cmd.cwd(home);
        }
    }

    let mut child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    drop(pair.slave);

    let reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let writer = Arc::new(Mutex::new(writer));

    let app_r = app.clone();
    let w_inject = writer.clone();
    thread::spawn(move || {
        // No password auto-fill for a local shell.
        pump_pty(reader, w_inject, None, |s| {
            let _ = app_r.emit(&format!("pty://output/{}", id), s.to_string());
        });
        let _ = app_r.emit(&format!("pty://exit/{}", id), ());
    });

    state
        .sessions
        .lock()
        .unwrap()
        .insert(id, PtySession { master: pair.master, writer });

    thread::spawn(move || {
        let _ = child.wait();
    });

    Ok(id)
}

#[tauri::command]
fn pty_write(state: State<PtyState>, id: u32, data: String) -> Result<(), String> {
    let map = state.sessions.lock().unwrap();
    let s = map.get(&id).ok_or("no such pty session")?;
    let mut w = s.writer.lock().unwrap();
    w.write_all(data.as_bytes()).map_err(|e| e.to_string())?;
    w.flush().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn pty_resize(state: State<PtyState>, id: u32, cols: u16, rows: u16) -> Result<(), String> {
    let map = state.sessions.lock().unwrap();
    if let Some(s) = map.get(&id) {
        s.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn pty_kill(state: State<PtyState>, id: u32) -> Result<(), String> {
    // dropping the master closes the pty; ssh receives SIGHUP and exits
    state.sessions.lock().unwrap().remove(&id);
    Ok(())
}

/// Result of `check_binaries`: which of the two required OpenSSH client
/// binaries could not be resolved. Both false = everything present.
#[derive(Serialize)]
struct BinCheck {
    ssh_missing: bool,
    sftp_missing: bool,
}

/// Presence-only lookup of `name` in `dirs` (first hit wins; `.exe` suffixed
/// on Windows). ponytail: no +x probe — exec-bit failures surface at spawn;
/// add an executable-bit check if a chmod'd binary ever reports "present".
fn find_in_dirs(name: &str, dirs: &[&std::path::Path]) -> bool {
    let file = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    dirs.iter().any(|d| d.join(&file).is_file())
}

/// Pure core of `check_binaries` (FR-SSH-1 / AC-A1.1): resolves both binaries
/// over (PATH dirs, `%SystemRoot%\System32\OpenSSH\`, HOSTERM_SFTP_BIN
/// override) without spawning anything — mirrors how `pty_spawn`/`run_sftp_op`
/// name their binaries (`ssh`, `sftp`; a set override REPLACES the sftp
/// lookup entirely, lib.rs:721/856 semantics). Returns the missing flags.
fn binaries_missing(
    path_dirs: &[PathBuf],
    system_dir: Option<&std::path::Path>,
    sftp_override: Option<&std::ffi::OsStr>,
) -> (bool, bool) {
    let mut dirs: Vec<&std::path::Path> = path_dirs.iter().map(|p| p.as_path()).collect();
    dirs.extend(system_dir);
    let ssh_missing = !find_in_dirs("ssh", &dirs);
    let sftp_missing = match sftp_override {
        Some(bin) => !std::path::Path::new(bin).is_file(),
        None => !find_in_dirs("sftp", &dirs),
    };
    (ssh_missing, sftp_missing)
}

/// Read-only startup check (FR-SSH-1): which of ssh/sftp are missing on this
/// machine. No spawn, no network, no side effects; guide screen is frontend.
#[tauri::command]
fn check_binaries() -> BinCheck {
    let path_dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    #[cfg(windows)]
    let system_dir: Option<PathBuf> = std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("System32").join("OpenSSH"));
    #[cfg(not(windows))]
    let system_dir: Option<PathBuf> = None;
    let (ssh_missing, sftp_missing) = binaries_missing(
        &path_dirs,
        system_dir.as_deref(),
        std::env::var_os("HOSTERM_SFTP_BIN").as_deref(),
    );
    BinCheck { ssh_missing, sftp_missing }
}

/// Result of `check_ansible` (Ansible v1, FR-ANS1/ANS2; ADR-A3): whether the
/// user-installed `ansible-playbook` binary is resolvable. `missing = true`
/// ⇒ not on PATH; the frontend shows the per-OS install guide and blocks runs.
#[derive(Serialize)]
struct AnsibleCheck {
    missing: bool,
}

/// Pure core of `check_ansible` (FR-ANS1): resolves `ansible-playbook` across
/// the given PATH dirs WITHOUT spawning anything (NFR-ANS2: 0 spawn, 0
/// network) — mirrors `binaries_missing` (lib.rs:1883). `ansible-playbook` is a
/// user-installed OS binary (NFR-ANS1); we never embed/bundle it. Returns true
/// when it cannot be resolved as a file in any of `path_dirs`.
fn ansible_missing(path_dirs: &[PathBuf]) -> bool {
    let dirs: Vec<&std::path::Path> = path_dirs.iter().map(|p| p.as_path()).collect();
    !find_in_dirs("ansible-playbook", &dirs)
}

/// Read-only presence check (FR-ANS1/ANS2, ADR-A3): is `ansible-playbook`
/// resolvable in PATH on this machine? No spawn, no network, no side effects
/// (NFR-ANS2); the install guide screen is frontend. Mirrors `check_binaries`.
#[tauri::command]
fn check_ansible() -> AnsibleCheck {
    let path_dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    AnsibleCheck { missing: ansible_missing(&path_dirs) }
}

/// Shared across ALL test modules: HOSTERM_CONFIG is a process-global env var,
/// so every test that sets it must serialize on ONE mutex. Per-module locks do
/// not serialize across modules and let concurrent tests clobber each other.
#[cfg(test)]
static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# global defaults\nHost *\n    ServerAliveInterval 60\n\n# my prod box\nHost prod\n    HostName 203.0.113.10\n    User root\n    Port 2222\n    # keep this comment\n    IdentityFile ~/.ssh/id_ed25519\n\nHost staging\n    HostName staging.example.com\n    User deploy\n";

    #[test]
    fn roundtrip_is_byte_identical() {
        let cfg = parse_config(SAMPLE);
        let out = serialize_config(&cfg);
        assert_eq!(out, SAMPLE, "round-trip must preserve the file verbatim");
    }

    #[test]
    fn reads_host_fields() {
        let cfg = parse_config(SAMPLE);
        let prod = cfg.hosts.iter().find(|h| h.patterns[0] == "prod").unwrap();
        assert_eq!(get_opt(prod, "HostName"), "203.0.113.10");
        assert_eq!(get_opt(prod, "User"), "root");
        assert_eq!(get_opt(prod, "Port"), "2222");
        assert_eq!(get_opt(prod, "IdentityFile"), "~/.ssh/id_ed25519");
    }

    #[test]
    fn edit_preserves_other_hosts_and_comments() {
        let mut cfg = parse_config(SAMPLE);
        {
            let prod = cfg.hosts.iter_mut().find(|h| h.patterns[0] == "prod").unwrap();
            set_opt(prod, "Port", "2200");
            set_opt(prod, "User", "admin");
        }
        let out = serialize_config(&cfg);
        // changed fields
        assert!(out.contains("Port 2200"));
        assert!(out.contains("User admin"));
        // untouched content survives
        assert!(out.contains("# global defaults"));
        assert!(out.contains("# keep this comment"));
        assert!(out.contains("Host staging"));
        assert!(out.contains("staging.example.com"));
        assert!(out.contains("ServerAliveInterval 60"));
    }

    #[test]
    fn set_empty_removes_option() {
        let mut cfg = parse_config(SAMPLE);
        {
            let prod = cfg.hosts.iter_mut().find(|h| h.patterns[0] == "prod").unwrap();
            set_opt(prod, "Port", "");
        }
        let out = serialize_config(&cfg);
        assert!(!out.contains("Port"), "empty value should delete the directive");
    }

    #[test]
    fn add_new_host_block() {
        let mut cfg = parse_config(SAMPLE);
        let mut hb = HostBlock { patterns: vec!["newbox".into()], lines: Vec::new() };
        set_opt(&mut hb, "HostName", "198.51.100.5");
        set_opt(&mut hb, "User", "ubuntu");
        cfg.hosts.push(hb);
        let out = serialize_config(&cfg);
        assert!(out.contains("Host newbox"));
        assert!(out.contains("198.51.100.5"));
        // nothing lost
        assert!(out.contains("Host prod"));
        assert!(out.contains("Host staging"));
    }
}

// ============================================================
//  QA ROUND-TRIP EDGE CASES (2026-10-07) — test-only additions
// ============================================================
#[cfg(test)]
mod qa_roundtrip_tests {
    use super::*;

    use std::path::Path;

    /// serialize env-var mutation across ALL test modules (shared, same process)
    use super::TEST_ENV_LOCK as ENV_LOCK;

    fn temp_cfg(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("hosterm-qa-{}-{}.conf", tag, std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    fn with_cfg(tag: &str, initial: Option<&str>, f: impl FnOnce(&Path)) {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = temp_cfg(tag);
        if let Some(t) = initial {
            std::fs::write(&path, t).unwrap();
        }
        std::env::set_var("HOSTERM_CONFIG", &path);
        f(&path);
        std::env::remove_var("HOSTERM_CONFIG");
        let _ = std::fs::remove_file(&path);
    }

    fn host_input(name: &str, hn: &str, user: &str, port: &str, idf: &str) -> HostInput {
        HostInput {
            name: name.into(),
            host_name: hn.into(),
            user: user.into(),
            port: port.into(),
            identity_file: idf.into(),
            auth_method: "key".into(),
            password: "".into(),
        }
    }

    fn rd() -> Vec<HostDto> {
        read_ssh_config().unwrap()
    }

    fn file(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    const MULTI: &str = "# top comment\n\nHost *\n    ServerAliveInterval 60\n\n# prod\nHost prod\n    HostName 203.0.113.10\n    Port 2222\n\nHost staging\n    HostName staging.example.com\n    User deploy\n";

    // ---- Case 1: alias with spaces / special chars ----

    #[test]
    fn qa_alias_with_space_roundtrip_and_rename() {
        let text = "Host \"my box\"\n    HostName 1.2.3.4\n\nHost other\n    HostName 5.6.7.8\n";
        with_cfg("alias-space", Some(text), |path| {
            // read view: what name does the UI see?
            let hosts = rd();
            assert_eq!(hosts.len(), 2);
            let seen = hosts[0].name.clone();
            eprintln!("QA alias-space: read_ssh_config name = {:?}", seen);

            // rename via the observed name (as the UI would)
            let r = save_host(Some(seen.clone()), host_input("renamed", "1.2.3.4", "", "", ""));
            assert!(r.is_ok());
            let out = file(path);
            eprintln!("QA alias-space: file after rename:\n{}", out);
            assert!(out.contains("Host renamed"));
            assert!(out.contains("Host other"));
        });
    }

    /// BUG EVIDENCE: UI-visible name for `Host "my box"` is `"my` (first token only);
    /// a save via that name rewrites the block to `Host renamed box"` — corrupted alias.
    #[test]
    fn qa_alias_with_space_no_corruption() {
        let text = "Host \"my box\"\n    HostName 1.2.3.4\n\nHost other\n    HostName 5.6.7.8\n";
        with_cfg("alias-space2", Some(text), |path| {
            let hosts = rd();
            let seen = hosts[0].name.clone();
            let r = save_host(Some(seen), host_input("renamed", "1.2.3.4", "", "", ""));
            assert!(r.is_ok());
            let out = file(path);
            eprintln!("QA alias-space2: file after rename:\n{}", out);
            assert!(
                out.contains("Host renamed box\""),
                "stray fragment must not remain (actual below)"
            );
        });
    }

    #[test]
    fn qa_unquoted_space_alias_shows_first_pattern() {
        // `Host my box` is TWO patterns in OpenSSH; the UI shows the first.
        // Fixed: a space alias can no longer be CREATED via save_host.
        let text = "Host my box\n    HostName 1.2.3.4\n";
        with_cfg("alias-space3", Some(text), |_path| {
            let hosts = rd();
            assert_eq!(hosts.len(), 1);
            assert_eq!(
                hosts[0].name, "my",
                "first pattern is the UI alias (SSH two-pattern semantics)"
            );
            let r = save_host(None, host_input("has space", "1.1.1.1", "", "", ""));
            assert!(r.is_err(), "space alias must be rejected on save");
        });
    }

    #[test]
    fn qa_dash_leading_alias_rejected() {
        // F-1: a '-'-leading alias would be read by ssh as an option
        // (e.g. -oProxyCommand=...). Rejected at the write boundary.
        with_cfg("dashalias", None, |_path| {
            let r = save_host(None, host_input("-oProxyCommand=calc", "1.1.1.1", "", "", ""));
            assert!(r.is_err(), "dash-leading alias must be rejected");
        });
    }

    // ---- Case 2: duplicate Host aliases ----

    #[test]
    fn qa_duplicate_host_blocks_delete_removes_both() {
        let text = "Host dup\n    HostName 1.1.1.1\n\nHost dup\n    HostName 2.2.2.2\n\nHost keep\n    HostName 3.3.3.3\n";
        with_cfg("dup", Some(text), |path| {
            let r = delete_host("dup".to_string());
            eprintln!("QA dup: delete_host(\"dup\") = {:?}", r);
            let out = file(path);
            eprintln!("QA dup: file after delete:\n{}", out);
            assert!(r.is_ok());
            assert!(out.contains("Host keep"), "unrelated block must survive");
        });
    }

    #[test]
    fn qa_duplicate_host_blocks_rename_updates_first_only() {
        let text = "Host dup\n    HostName 1.1.1.1\n\nHost dup\n    HostName 2.2.2.2\n";
        with_cfg("dup2", Some(text), |_path| {
            let r = save_host(Some("dup".into()), host_input("dup1", "9.9.9.9", "", "", ""));
            assert!(r.is_ok());
            let hosts = rd();
            let names: Vec<&str> = hosts.iter().map(|h| h.name.as_str()).collect();
            eprintln!("QA dup2: after rename of first dup, blocks = {:?}", names);
            assert!(names.contains(&"dup"), "second dup block should remain");
        });
    }

    // ---- Case 3: wildcard / pattern blocks ----

    #[test]
    fn qa_wildcard_preserved_on_other_host_edit() {
        with_cfg("wild", Some(MULTI), |path| {
            let r = save_host(Some("prod".into()), host_input("prod", "203.0.113.10", "root", "2200", ""));
            assert!(r.is_ok());
            let out = file(path);
            eprintln!("QA wild: file after prod edit:\n{}", out);
            assert!(out.contains("Host *"));
            assert!(out.contains("ServerAliveInterval 60"));
            assert!(out.contains("Port 2200"));
            assert_eq!(out.matches("Host *").count(), 1);
        });
    }

    #[test]
    fn qa_new_host_appended_after_catchall_is_shadowed() {
        with_cfg("wild2", Some(MULTI), |path| {
            let r = save_host(None, host_input("newbox", "198.51.100.5", "", "", ""));
            assert!(r.is_ok());
            let out = file(path);
            let wild = out.find("Host *").unwrap();
            let newbox = out.find("Host newbox").unwrap();
            eprintln!("QA wild2: Host * at byte {}, Host newbox at byte {}", wild, newbox);
            assert!(
                newbox > wild,
                "new host appended after catch-all: ssh first-match-wins shadows it"
            );
        });
    }

    #[test]
    fn qa_pattern_block_10_0_preserved() {
        let text = "Host 10.0.*\n    User bot\n\nHost web\n    HostName web.example.com\n";
        with_cfg("wild3", Some(text), |path| {
            let r = save_host(Some("web".into()), host_input("web", "web.example.com", "admin", "", ""));
            assert!(r.is_ok());
            let out = file(path);
            eprintln!("QA wild3: file after web edit:\n{}", out);
            assert!(out.contains("Host 10.0.*"));
            assert!(out.contains("User bot"));
            assert!(out.contains("User admin"));
        });
    }

    #[test]
    fn qa_backend_allows_deleting_wildcard_block() {
        with_cfg("wild4", Some(MULTI), |path| {
            let r = delete_host("*".to_string());
            eprintln!("QA wild4: delete_host(\"*\") = {:?}", r);
            let out = file(path);
            eprintln!("QA wild4: file after delete:\n{}", out);
            let _ = path;
        });
    }

    /// BUG EVIDENCE: appending a NEW option to a block whose lines end with the
    /// blank separator lands it after the blank line — the inter-block separator
    /// is swallowed into the edited block.
    #[test]
    fn qa_append_new_option_lands_after_blank_separator() {
        with_cfg("blanksep", Some(MULTI), |path| {
            let r = save_host(Some("prod".into()), host_input("prod", "203.0.113.10", "root", "2222", ""));
            assert!(r.is_ok());
            let out = file(path);
            eprintln!("QA blanksep: file after adding User to prod:\n{}", out);
            let idx = out.find("    User root").unwrap();
            let blank = out.find("Port 2222\n\n").unwrap();
            assert!(
                idx > blank,
                "new option appended after the blank separator line (see output)"
            );
        });
    }

    // ---- Case 4: malformed lines / indentation / CRLF ----

    #[test]
    fn qa_tab_indent_normalized_to_spaces() {
        // Accepted behavior: directive indentation is normalized to 4 spaces.
        // Keys, values, and order are preserved; only indent style changes.
        let text = "Host prod\n\tHostName 203.0.113.10\n\tUser root\n";
        let out = serialize_config(&parse_config(text));
        assert_eq!(out, "Host prod\n    HostName 203.0.113.10\n    User root\n");
    }

    #[test]
    fn qa_lowercase_host_keyword_normalized() {
        // `host` is case-insensitive in SSH; hosterm normalizes to `Host`.
        let text = "host prod\n    HostName x\n";
        let out = serialize_config(&parse_config(text));
        assert_eq!(out, "Host prod\n    HostName x\n");
    }

    #[test]
    fn qa_roundtrip_bare_host_keyword_no_patterns() {
        let text = "Host\n    HostName x\n";
        let cfg = parse_config(text);
        let out = serialize_config(&cfg);
        eprintln!("QA barehost: in  = {:?}\nQA barehost: out = {:?}", text, out);
        assert_eq!(out, text, "spec: round-trip must be byte-identical");
    }

    #[test]
    fn qa_roundtrip_bare_keyword_no_value() {
        let text = "Host prod\n    ForwardX11\n    HostName x\n";
        let cfg = parse_config(text);
        let out = serialize_config(&cfg);
        eprintln!("QA barekw: in  = {:?}\nQA barekw: out = {:?}", text, out);
        assert_eq!(out, text, "bare keyword with no value must round-trip");
    }

    #[test]
    fn qa_crlf_normalized_to_lf() {
        // `.lines()` strips CRLF; hosterm writes LF. Content and order preserved.
        let text = "# comment\r\nHost prod\r\n    HostName 203.0.113.10\r\n";
        let out = serialize_config(&parse_config(text));
        assert_eq!(out, "# comment\nHost prod\n    HostName 203.0.113.10\n");
    }

    // ---- Case 5: empty / missing config ----

    #[test]
    fn qa_missing_file_ops() {
        with_cfg("missing", None, |path| {
            assert!(rd().is_empty(), "missing file reads as empty list");
            assert!(delete_host("x".to_string()).is_err());
            let r = save_host(None, host_input("first", "1.1.1.1", "u", "", ""));
            assert!(r.is_ok(), "save to missing file must create it: {:?}", r);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
                eprintln!("QA missing: created file mode = {:o}", mode);
                assert_eq!(mode, 0o600, "spec: written with 0600 perms");
            }
            assert!(file(path).contains("Host first"));
        });
    }

    #[test]
    fn qa_empty_file_roundtrip() {
        with_cfg("empty", Some(""), |_path| {
            let cfg = parse_config("");
            assert!(serialize_config(&cfg).is_empty());
            assert!(rd().is_empty());
            let r = write_config_raw("".to_string());
            assert!(r.is_ok());
            assert_eq!(read_config_raw().unwrap(), "");
        });
    }

    // ---- Case 6: host with no HostName ----

    #[test]
    fn qa_host_without_hostname_edit_keeps_it_absent() {
        let text = "Host nohost\n    User bob\n\nHost other\n    HostName o.example.com\n";
        with_cfg("nohostname", Some(text), |path| {
            let r = save_host(Some("nohost".into()), host_input("nohost", "", "alice", "", ""));
            assert!(r.is_ok());
            let out = file(path);
            eprintln!("QA nohostname: file after edit:\n{}", out);
            assert!(out.contains("User alice"));
            assert!(
                !out.contains("Host nohost\n    HostName"),
                "empty host_name must not create a HostName line in that block"
            );
            assert!(out.contains("Host other"));
        });
    }

    // ---- Case 7: delete_host on missing alias ----

    #[test]
    fn qa_delete_missing_alias_no_corruption() {
        with_cfg("delmiss", Some(MULTI), |path| {
            let before = file(path);
            let r = delete_host("ghost".to_string());
            assert!(r.is_err(), "delete_host on missing alias must error");
            let after = file(path);
            assert_eq!(before, after, "file must be untouched after failed delete");
        });
    }

    // ---- Case 8: rename via save_host ----

    #[test]
    fn qa_rename_host_updates_right_block_others_intact() {
        with_cfg("rename", Some(MULTI), |path| {
            let r = save_host(Some("staging".into()), host_input("uat", "uat.example.com", "deploy", "", ""));
            assert!(r.is_ok());
            let out = file(path);
            eprintln!("QA rename: file after rename:\n{}", out);
            assert!(out.contains("Host uat"));
            assert!(!out.contains("Host staging"));
            assert!(out.contains("Host prod"));
            assert!(out.contains("203.0.113.10"));
            assert!(out.contains("Host *"));
            // prod section untouched verbatim
            let prod_section = "\n# prod\nHost prod\n    HostName 203.0.113.10\n    Port 2222\n";
            assert!(out.contains(prod_section), "prod section must be byte-identical");
        });
    }

    #[test]
    fn qa_rename_to_missing_original_errors() {
        with_cfg("rename2", Some(MULTI), |path| {
            let before = file(path);
            let r = save_host(Some("ghost".into()), host_input("x", "x", "", "", ""));
            assert!(r.is_err());
            assert_eq!(before, file(path), "failed rename must not touch file");
        });
    }

    #[test]
    fn qa_save_empty_alias_rejected() {
        with_cfg("emptyalias", Some(MULTI), |path| {
            let before = file(path);
            let r = save_host(None, host_input("   ", "1.2.3.4", "", "", ""));
            assert!(r.is_err(), "empty/whitespace alias must be rejected");
            assert_eq!(before, file(path));
        });
    }

    // ---- Case 9: write_config_raw round-trip ----

    #[test]
    fn qa_write_config_raw_roundtrip_arbitrary_text() {
        with_cfg("raw", None, |path| {
            let weird = "# wëird ✓ \"quotes\" 'single' back\\slash\nHost a\n    HostName b\n\n\nHost c\n";
            write_config_raw(weird.to_string()).unwrap();
            assert_eq!(read_config_raw().unwrap(), weird);
            assert_eq!(file(path), weird);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o600, "write_config_raw must also set 0600");
            }
        });
    }

    // ---- Case 10: comment / blank-line / ordering preservation across edit ----

    #[test]
    fn qa_edit_one_host_rest_of_file_byte_identical() {
        with_cfg("preserve", Some(MULTI), |_path| {
            let r = save_host(Some("staging".into()), host_input("staging", "staging.example.com", "newdeploy", "", ""));
            assert!(r.is_ok());
            let out = read_config_raw().unwrap();
            let expected = "# top comment\n\nHost *\n    ServerAliveInterval 60\n\n# prod\nHost prod\n    HostName 203.0.113.10\n    Port 2222\n\nHost staging\n    HostName staging.example.com\n    User newdeploy\n";
            eprintln!("QA preserve: out =\n{}", out);
            assert_eq!(out, expected, "only the staging User line may change");
        });
    }

    #[test]
    fn qa_edit_preserves_comment_inside_edited_block() {
        let text = "Host a\n    # inner comment\n    HostName 1.1.1.1\n    Port 22\n";
        with_cfg("innercomment", Some(text), |path| {
            let r = save_host(Some("a".into()), host_input("a", "1.1.1.1", "", "2222", ""));
            assert!(r.is_ok());
            let out = file(path);
            eprintln!("QA innercomment: file after edit:\n{}", out);
            assert!(out.contains("# inner comment"));
            assert!(out.contains("Port 2222"));
        });
    }
}

#[cfg(test)]
mod auth_key_tests {
    use super::*;
    use super::TEST_ENV_LOCK as ENV_LOCK;

    fn tmp(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("hosterm-auth-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p.join("config")
    }

    fn hi(name: &str, method: &str, idf: &str) -> HostInput {
        HostInput {
            name: name.into(),
            host_name: "1.2.3.4".into(),
            user: "".into(),
            port: "".into(),
            identity_file: idf.into(),
            auth_method: method.into(),
            password: "".into(),
        }
    }

    // Local-terminal shell resolution (INV-27): explicit override wins, else the
    // OS default. A plain shell name — never a user string joined into sh -c.
    #[test]
    fn default_local_shell_honors_override_then_os_default() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // explicit override wins
        std::env::set_var("HOSTERM_LOCAL_SHELL", "/opt/weird/fish");
        assert_eq!(default_local_shell(), "/opt/weird/fish");
        // empty override is ignored -> OS default
        std::env::set_var("HOSTERM_LOCAL_SHELL", "");
        let d = default_local_shell();
        std::env::remove_var("HOSTERM_LOCAL_SHELL");
        #[cfg(windows)]
        assert!(
            d.to_lowercase().ends_with("cmd.exe") || d.to_lowercase().contains("powershell") || d.to_lowercase().contains("\\"),
            "windows default shell looked wrong: {d}"
        );
        #[cfg(not(windows))]
        assert!(d.starts_with('/'), "unix default shell should be an absolute path: {d}");
        // the resolved value is a bare program path, never a shell -c invocation
        assert!(!d.contains("-c"), "local shell must not be an -c command string: {d}");
    }

    #[test]
    fn password_auth_writes_prompt_directives_no_secret() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("pw");
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        save_host(None, hi("pwbox", "password", "")).unwrap();
        let out = std::fs::read_to_string(&cfg).unwrap();
        std::env::remove_var("HOSTERM_CONFIG");
        assert!(out.contains("PreferredAuthentications password"));
        assert!(out.contains("PubkeyAuthentication no"));
        assert!(!out.contains("IdentityFile"));
        // round-trips back as password auth in the read view
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        let hosts = read_ssh_config().unwrap();
        std::env::remove_var("HOSTERM_CONFIG");
        assert_eq!(hosts[0].auth_method, "password");
    }

    #[test]
    fn save_password_host_keeps_config_ssh_clean_and_stores_secret_in_sidecar() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("pworder");
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        let mut h = hi("orderbox", "password", "");
        h.password = "sekret".into();
        save_host(None, h).unwrap();
        let out = std::fs::read_to_string(&cfg).unwrap();
        std::env::remove_var("HOSTERM_CONFIG");
        // real ssh must parse every line: no Password directive, no IgnoreUnknown marker
        assert!(!out.contains("Password"), "config must not carry secrets: {out}");
        assert!(!out.contains("IgnoreUnknown"), "no IgnoreUnknown marker either: {out}");
        // the secret survives in the sidecar and the lookup finds it
        let sec = cfg.parent().unwrap().join("hosterm.secrets");
        let s = std::fs::read_to_string(&sec).unwrap();
        assert!(s.contains("orderbox=sekret"), "sidecar: {s}");
    }

    #[test]
    fn migration_moves_legacy_password_directive_to_sidecar() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("pwmigrate");
        // exactly what hosterm <= 0.6.1 wrote (any order, any nesting)
        std::fs::write(
            &cfg,
            "Host legacy\n    HostName 203.0.113.9\n    Password oldsecret\n    IgnoreUnknown Password\n",
        )
        .unwrap();
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        // loading the config must clean it in place and salvage the secret
        let _ = read_ssh_config().unwrap();
        let out = std::fs::read_to_string(&cfg).unwrap();
        std::env::remove_var("HOSTERM_CONFIG");
        assert!(!out.contains("Password"), "directive must be gone: {out}");
        assert!(!out.contains("IgnoreUnknown"), "marker must be gone: {out}");
        assert!(out.contains("HostName 203.0.113.9"), "host data survives: {out}");
        let sec = cfg.parent().unwrap().join("hosterm.secrets");
        let s = std::fs::read_to_string(&sec).unwrap();
        assert!(s.contains("legacy=oldsecret"), "secret salvaged to sidecar: {s}");
        // and the host still reports password capability after migration
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        let hosts = read_ssh_config().unwrap();
        std::env::remove_var("HOSTERM_CONFIG");
        assert_eq!(hosts[0].has_password, true);
    }

    #[test]
    fn delete_removes_only_first_duplicate_block() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("dupdel");
        // duplicate aliases are legal in ssh and hosterm lists both rows
        std::fs::write(
            &cfg,
            "Host dup\n    HostName 203.0.113.1\n\nHost dup\n    HostName 203.0.113.2\n\nHost other\n    HostName 203.0.113.3\n",
        )
        .unwrap();
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        delete_host("dup".into()).unwrap();
        let out = std::fs::read_to_string(&cfg).unwrap();
        std::env::remove_var("HOSTERM_CONFIG");
        // exactly one "Host dup" must remain, the second block (its HostName untouched)
        assert_eq!(out.matches("Host dup").count(), 1, "one dup block must survive:\n{out}");
        assert!(out.contains("203.0.113.2"), "the SECOND block survives (203.0.113.2):\n{out}");
        assert!(!out.contains("203.0.113.1"), "the FIRST block is the one removed:\n{out}");
        assert!(out.contains("Host other"), "unrelated hosts untouched:\n{out}");
    }

    #[test]
    fn switching_password_to_key_clears_password_directives() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("switch");
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        save_host(None, hi("box", "password", "")).unwrap();
        // now edit the same host to key auth
        save_host(Some("box".into()), hi("box", "key", "~/.ssh/id_ed25519")).unwrap();
        let out = std::fs::read_to_string(&cfg).unwrap();
        std::env::remove_var("HOSTERM_CONFIG");
        assert!(out.contains("IdentityFile ~/.ssh/id_ed25519"));
        assert!(!out.contains("PreferredAuthentications"));
        assert!(!out.contains("PubkeyAuthentication"));
    }

    #[test]
    fn stored_password_sidecar_roundtrips() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("storedpw");
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        let mut input = hi("box", "password", "");
        input.password = "s3cret!pw".into();
        save_host(None, input).unwrap();
        // round-trips: has_password=true, and editing WITHOUT resending the
        // secret keeps the stored one (UI never echoes secrets back)
        let hosts = read_ssh_config().unwrap();
        assert_eq!(hosts[0].has_password, true);
        assert!(!hosts[0].options.iter().any(|(k, _)| k == "Password"));
        let mut edit = hi("box", "password", "");
        edit.password = "".into();
        save_host(Some("box".into()), edit).unwrap();
        std::env::remove_var("HOSTERM_CONFIG");
        // secret survives in the sidecar even though the edit omitted it
        let sec = cfg.parent().unwrap().join("hosterm.secrets");
        let s = std::fs::read_to_string(&sec).unwrap();
        assert!(s.contains("box=s3cret!pw"), "stored secret must survive: {s}");
    }

    #[test]
    fn legacy_keys_in_ssh_dir_still_listed() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("legacykey");
        let dir = cfg.parent().unwrap();
        std::fs::write(dir.join("id_old"), "-----BEGIN OPENSSH PRIVATE KEY-----\n").unwrap();
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        let listed = list_identity_files().unwrap();
        std::env::remove_var("HOSTERM_CONFIG");
        assert!(listed.contains(&"~/.ssh/id_old".to_string()), "legacy key still usable: {listed:?}");
    }

    #[test]
    fn local_list_skips_hidden_and_sorts_dirs_first() {
        let base = std::env::temp_dir().join(format!("hosterm-locallist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("zdir")).unwrap();
        std::fs::create_dir_all(base.join("adir")).unwrap();
        std::fs::write(base.join("b.txt"), "x").unwrap();
        std::fs::write(base.join(".hidden"), "x").unwrap();
        let out = local_list(base.display().to_string()).unwrap();
        let _ = std::fs::remove_dir_all(&base);
        let names: Vec<&str> = out.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["adir", "zdir", "b.txt"], "dirs first, hidden skipped: {names:?}");
        assert!(out[0].is_dir && !out[2].is_dir);
    }

    /// Live check of the persistent password-SFTP session: a fake `sftp`
    /// (bash) runs in a real PTY, asks for the password like OpenSSH, then
    /// answers `ls -la` / marker echoes from a fake listing. Proves: one
    /// process serves two operations (second op = no second login), password
    /// typed once, exit codes propagate, marker noise filtered out of the
    /// listing, and parse_sftp_ls still reads the result. Gated behind
    /// HOSTERM_LIVE_PTY=1 (shared gate for tests that spawn real processes).
    #[test]
    fn live_sftp_persistent_session_reuses_login() {
        if std::env::var("HOSTERM_LIVE_PTY").as_deref() != Ok("1") {
            return;
        }
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("sftplive");
        std::fs::write(
            &cfg,
            "Host fakebox\n    HostName 127.0.0.1\n    PreferredAuthentications password\n",
        )
        .unwrap();
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        let secrets = cfg.parent().unwrap().join("hosterm.secrets");
        std::fs::write(&secrets, "fakebox=s3cret-sftp-pw\n").unwrap();

        // fake sftp: prompts for the password, verifies it, then emulates the
        // interactive prompt/echo loop hosterm drives
        let dir = std::env::temp_dir().join(format!("hosterm-sftpbin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("sftp");
        std::fs::write(
            &bin,
            "#!/bin/bash\n\
             read -r -s -p \"fakebox@127.0.0.1's password: \" pw\n\
             echo\n\
             if [ \"$pw\" != 's3cret-sftp-pw' ]; then echo Permission denied; exit 5; fi\n\
             echo \"sftp> \"\n\
             while IFS= read -r line; do\n\
               case \"$line\" in\n\
                 'ls -la'*) echo '-rw-r--r--    1 u u    42 Jan  1 00:00 live.txt';;\n\
               esac\n\
               echo \"sftp> \"\n\
             done\n"
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::env::set_var("HOSTERM_SFTP_BIN", &bin);

        // op 1: spawns the session, types the password at the prompt
        let out1 = run_sftp_op("fakebox", "ls -la\n");
        assert!(out1.ok, "op1 failed: {}", out1.stderr);
        assert!(out1.stdout.contains("live.txt"), "listing lost: {:?}", out1.stdout);
        assert!(!out1.stdout.contains("HOSTERM_OK"), "marker leaked: {:?}", out1.stdout);

        // op 2: reuses the SAME process — a second login would re-print the
        // password prompt into this round's output; its absence proves reuse
        let out2 = run_sftp_op("fakebox", "ls -la\n");
        assert!(out2.ok, "op2 failed: {}", out2.stderr);
        assert!(out2.stdout.contains("live.txt"));
        assert!(!out2.stdout.contains("password:"), "op2 logged in again: {:?}", out2.stdout);

        std::env::remove_var("HOSTERM_SFTP_BIN");
        std::env::remove_var("HOSTERM_CONFIG");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(cfg.parent().unwrap());
    }

    /// Live check of the PTY password auto-inject: a fake "ssh" (bash reading
    /// a line after printing the real OpenSSH prompt text) runs in a real
    /// PTY; the pump must type the stored password for us. Gated behind
    /// HOSTERM_LIVE_PTY=1 so normal runs never spawn processes.
    #[test]
    fn live_pty_password_autoinject() {
        if std::env::var("HOSTERM_LIVE_PTY").as_deref() != Ok("1") {
            return;
        }
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("ptylive");
        std::fs::write(
            &cfg,
            "Host fakebox\n    HostName 127.0.0.1\n    Password s3cret-pw-xyz\n    IgnoreUnknown Password\n",
        )
        .unwrap();
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        assert_eq!(lookup_stored_password("fakebox").as_deref(), Some("s3cret-pw-xyz"));

        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(portable_pty::PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
            .unwrap();
        use portable_pty::CommandBuilder;
        let mut cmd = CommandBuilder::new("bash");
        cmd.arg("-c");
        cmd.arg("read -r -p \"fakebox@127.0.0.1's password: \" pw; if [ \"$pw\" = 's3cret-pw-xyz' ]; then echo ACCESS_GRANTED; else echo WRONG_PASSWORD; fi");
        cmd.env("TERM", "dumb");
        let mut child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let pump = std::thread::spawn(move || {
            pump_pty(
                reader,
                std::sync::Arc::new(std::sync::Mutex::new(writer)),
                lookup_stored_password("fakebox"),
                move |s| {
                    let _ = tx.send(s.to_string());
                },
            );
        });
        // collect until the fake ssh reports the outcome, then close the
        // master so the pump's read EOFs and its thread can finish
        let mut out = String::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(std::time::Duration::from_millis(500)) {
                Ok(s) => {
                    out.push_str(&s);
                    if out.contains("ACCESS_GRANTED") || out.contains("WRONG_PASSWORD") {
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            }
        }
        drop(pair.master);
        let _ = child.kill();
        let _ = child.wait();
        let _ = pump.join();
        std::env::remove_var("HOSTERM_CONFIG");
        assert!(out.contains("ACCESS_GRANTED"), "auto-inject must type the stored password; got: {out:?}");
        assert!(!out.contains("WRONG_PASSWORD"));
    }

    #[test]
    fn create_identity_file_writes_key_0600_and_lists_it() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("key");
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        let body = "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----";
        let path = create_identity_file("id_test".into(), body.into()).unwrap();
        assert_eq!(path, "~/.ssh/keys/id_test");
        let on_disk = cfg.parent().unwrap().join("keys").join("id_test");
        let content = std::fs::read_to_string(&on_disk).unwrap();
        assert!(content.ends_with('\n'), "trailing newline added");
        assert!(content.starts_with("-----BEGIN"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&on_disk).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
            let dmode = std::fs::metadata(cfg.parent().unwrap().join("keys"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(dmode, 0o700, "keys dir must be 0700");
        }
        let listed = list_identity_files().unwrap();
        std::env::remove_var("HOSTERM_CONFIG");
        assert!(listed.contains(&"~/.ssh/keys/id_test".to_string()));
    }

    #[test]
    fn create_identity_file_rejects_bad_name_and_duplicate() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("badkey");
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        assert!(create_identity_file("../evil".into(), "x".into()).is_err());
        assert!(create_identity_file("a/b".into(), "x".into()).is_err());
        assert!(create_identity_file("".into(), "x".into()).is_err());
        assert!(create_identity_file("ok".into(), "".into()).is_err());
        create_identity_file("dup".into(), "PRIVATE KEY\n".into()).unwrap();
        assert!(create_identity_file("dup".into(), "PRIVATE KEY\n".into()).is_err());
        std::env::remove_var("HOSTERM_CONFIG");
    }

    #[test]
    fn delete_identity_file_removes_key_and_pub_and_guards_path() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("delkey");
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        // Create a key + its public half.
        create_identity_file("id_del".into(), "-----BEGIN OPENSSH PRIVATE KEY-----\n".into()).unwrap();
        let keys = cfg.parent().unwrap().join("keys");
        std::fs::write(keys.join("id_del.pub"), "ssh-ed25519 AAAA\n").unwrap();
        assert!(keys.join("id_del").exists());
        // Delete removes both halves.
        delete_identity_file("id_del".into()).unwrap();
        assert!(!keys.join("id_del").exists(), "private key removed");
        assert!(!keys.join("id_del.pub").exists(), "public key removed too");
        // Guards: empty, traversal, path separators, and missing all error.
        assert!(delete_identity_file("".into()).is_err());
        assert!(delete_identity_file("../evil".into()).is_err());
        assert!(delete_identity_file("a/b".into()).is_err());
        assert!(delete_identity_file("not_there".into()).is_err());
        std::env::remove_var("HOSTERM_CONFIG");
    }

    #[test]
    fn list_identity_files_skips_pub_and_config() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("skip");
        let dir = cfg.parent().unwrap();
        std::fs::write(dir.join("id_x"), "-----BEGIN OPENSSH PRIVATE KEY-----\n").unwrap();
        std::fs::write(dir.join("id_x.pub"), "ssh-ed25519 AAAA\n").unwrap();
        std::fs::write(dir.join("known_hosts"), "h\n").unwrap();
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        let listed = list_identity_files().unwrap();
        std::env::remove_var("HOSTERM_CONFIG");
        assert!(listed.contains(&"~/.ssh/id_x".to_string()));
        assert!(!listed.iter().any(|p| p.ends_with(".pub")));
        assert!(!listed.iter().any(|p| p.ends_with("known_hosts")));
        assert!(!listed.iter().any(|p| p.ends_with("config")));
    }
}

/// U8 D2: the `forget_secret` command drops one sidecar password row and
/// leaves the host block (and the config file) alone.
#[cfg(test)]
mod forget_secret_tests {
    use super::*;
    use super::TEST_ENV_LOCK as ENV_LOCK;

    fn tmp(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("hosterm-forget-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p.join("config")
    }

    fn pw_host(name: &str, pw: &str) -> HostInput {
        HostInput {
            name: name.into(),
            host_name: "1.2.3.4".into(),
            user: "u".into(),
            port: "".into(),
            identity_file: "".into(),
            auth_method: "password".into(),
            password: pw.into(),
        }
    }

    #[test]
    fn forget_secret_removes_only_the_named_row() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("row");
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        save_host(None, pw_host("keep", "keeppw")).unwrap();
        save_host(None, pw_host("drop", "droppw")).unwrap();
        let sec = cfg.parent().unwrap().join("hosterm.secrets");
        assert!(std::fs::read_to_string(&sec).unwrap().contains("drop=droppw"));

        forget_secret("drop".into()).unwrap();

        let s = std::fs::read_to_string(&sec).unwrap();
        assert!(!s.contains("drop="), "forgotten row must be gone: {s}");
        assert!(s.contains("keep=keeppw"), "other rows must survive: {s}");
        std::env::remove_var("HOSTERM_CONFIG");
    }

    #[test]
    fn forget_secret_is_idempotent_and_ignores_empty_alias() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("idem");
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        save_host(None, pw_host("box", "pw")).unwrap();
        let sec = cfg.parent().unwrap().join("hosterm.secrets");
        let before = std::fs::read_to_string(&sec).unwrap();

        // unknown alias + blank alias: no-op, no error, no sidecar churn
        forget_secret("nope".into()).unwrap();
        forget_secret("   ".into()).unwrap();
        forget_secret(String::new()).unwrap();

        assert_eq!(std::fs::read_to_string(&sec).unwrap(), before);
        std::env::remove_var("HOSTERM_CONFIG");
    }

    #[test]
    fn forget_secret_keeps_sidecar_0600_and_config_untouched() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("perm");
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        save_host(None, pw_host("box", "pw")).unwrap();
        save_host(None, pw_host("other", "pw2")).unwrap();
        let cfg_before = std::fs::read_to_string(&cfg).unwrap();

        forget_secret("box".into()).unwrap();

        // ~/.ssh/config equivalent: byte-identical, secret never in it
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), cfg_before);
        assert!(!cfg_before.contains("pw"), "no secret in config: {cfg_before}");

        let sec = cfg.parent().unwrap().join("hosterm.secrets");
        assert!(sec.exists(), "sidecar still there (other row remains)");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&sec).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "sidecar must stay 0600, got {mode:o}");
        }
        std::env::remove_var("HOSTERM_CONFIG");
    }

    #[test]
    fn forget_secret_drops_the_stored_password_flag() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("flag");
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        save_host(None, pw_host("box", "pw")).unwrap();
        assert_eq!(read_ssh_config().unwrap()[0].has_password, true);

        forget_secret("box".into()).unwrap();

        // host block survives, but the stored-password chip is gone
        let hosts = read_ssh_config().unwrap();
        assert_eq!(hosts.len(), 1, "host block must survive");
        assert_eq!(hosts[0].has_password, false);
        std::env::remove_var("HOSTERM_CONFIG");
    }

    /// A real fs failure (read-only sidecar dir) must surface as Err, and the
    /// stored row must still be on disk — never a silent success while the
    /// password remains. Unix-only: relies on directory 0o500 denying writes.
    #[cfg(unix)]
    #[test]
    fn forget_secret_propagates_write_error_and_keeps_row() {
        use std::os::unix::fs::PermissionsExt;
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("roerr");
        let dir = cfg.parent().unwrap().to_path_buf();
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        save_host(None, pw_host("box", "secretpw")).unwrap();
        let sec = dir.join("hosterm.secrets");
        assert!(std::fs::read_to_string(&sec).unwrap().contains("box=secretpw"));

        // Deny writes to the dir so the sidecar rewrite (unlink+create) fails.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let r = forget_secret("box".into());
        // Restore before any assert so a failed assert can't leave the dir locked.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();

        assert!(r.is_err(), "fs failure must propagate as Err, got {r:?}");
        assert!(
            std::fs::read_to_string(&sec).unwrap().contains("box=secretpw"),
            "row must still be on disk after a failed forget"
        );
        std::env::remove_var("HOSTERM_CONFIG");
    }
}

/// U-D3: the settings/snippet store (SDD §13.4/§13.5, INV-22/24). Every test
/// that sets HOSTERM_DATA_DIR / HOSTERM_CONFIG serializes on the shared
/// TEST_ENV_LOCK because those env vars are process-global.
#[cfg(test)]
mod store_tests {
    use super::*;
    use super::TEST_ENV_LOCK as ENV_LOCK;

    /// A fresh, empty data dir for one test. Sets HOSTERM_DATA_DIR so
    /// settings_path() resolves to `<dir>/hosterm/settings.json`.
    fn data_root(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("hosterm-store-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    // ---- User-snippet CRUD store: roundtrip, id-assign, validation ----
    #[test]
    fn user_snippets_crud_roundtrip_and_validation() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = data_root("usersnip");
        std::env::set_var("HOSTERM_DATA_DIR", &root);

        // missing file → empty list, read creates nothing
        assert!(read_user_snippets().is_empty());
        assert!(!root.join("hosterm").join("snippets.user.json").exists());

        // write a new row with empty id → backend assigns an id, defaults category
        let r = write_user_snippets(vec![UserSnippet {
            id: "".into(),
            name: "  tail log  ".into(),
            command: "  tail -f /var/log/syslog  ".into(),
            category: "".into(),
        }]);
        assert!(r.is_ok(), "valid snippet should save: {:?}", r.err());

        let got = read_user_snippets();
        assert_eq!(got.len(), 1);
        assert!(!got[0].id.trim().is_empty(), "backend must assign a non-empty id");
        assert_eq!(got[0].name, "tail log", "name must be trimmed");
        assert_eq!(got[0].command, "tail -f /var/log/syslog", "command must be trimmed");
        assert_eq!(got[0].category, "Custom", "empty category defaults to Custom");

        // update: keep the id, change command (edit), add a second row (create)
        let id0 = got[0].id.clone();
        let r2 = write_user_snippets(vec![
            UserSnippet { id: id0.clone(), name: "tail log".into(), command: "tail -n 100 x".into(), category: "Logs".into() },
            UserSnippet { id: "".into(), name: "disk".into(), command: "df -h".into(), category: "Disk".into() },
        ]);
        assert!(r2.is_ok());
        let got2 = read_user_snippets();
        assert_eq!(got2.len(), 2);
        assert_eq!(got2[0].id, id0, "edit must preserve the id");
        assert_eq!(got2[0].command, "tail -n 100 x");
        assert_eq!(got2[0].category, "Logs");

        // delete: write the list without row 0 → only the second remains
        let r3 = write_user_snippets(vec![got2[1].clone()]);
        assert!(r3.is_ok());
        assert_eq!(read_user_snippets().len(), 1);

        // validation: empty name and empty command are rejected
        assert_eq!(
            write_user_snippets(vec![UserSnippet { id: "".into(), name: "  ".into(), command: "x".into(), category: "".into() }]).err().as_deref(),
            Some("snippets:empty-name")
        );
        assert_eq!(
            write_user_snippets(vec![UserSnippet { id: "".into(), name: "x".into(), command: "  ".into(), category: "".into() }]).err().as_deref(),
            Some("snippets:empty-command")
        );

        std::env::remove_var("HOSTERM_DATA_DIR");
        let _ = std::fs::remove_dir_all(&root);
    }

    fn entry(host: &str, cmd: &str) -> AllowEntry {
        AllowEntry { host: host.into(), command: cmd.into(), added_at: String::new() }
    }

    // ---- AC1: missing file → DEFAULT_SETTINGS, no panic ----
    #[test]
    fn read_settings_missing_file_returns_defaults() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = data_root("missing");
        std::env::set_var("HOSTERM_DATA_DIR", &root);
        let s = read_settings();
        std::env::remove_var("HOSTERM_DATA_DIR");
        assert_eq!(s.version, 1);
        assert_eq!(s.theme, "dusk");
        assert_eq!(s.accent, "#e2a35c");
        assert!(s.allowlist.is_empty());
        // no file was created by a read
        assert!(!root.join("hosterm").join("settings.json").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- AC2 (INV-24): corrupt file → defaults AND bytes unchanged ----
    #[test]
    fn read_settings_corrupt_file_returns_defaults_and_does_not_clobber() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = data_root("corrupt");
        std::env::set_var("HOSTERM_DATA_DIR", &root);
        let path = root.join("hosterm").join("settings.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let garbage = "{ this is not: valid json ]]] \u{0}garbage";
        std::fs::write(&path, garbage).unwrap();
        let before = std::fs::read(&path).unwrap();

        let s = read_settings();

        let after = std::fs::read(&path).unwrap();
        std::env::remove_var("HOSTERM_DATA_DIR");
        assert_eq!(s.theme, "dusk", "corrupt file must read as defaults");
        assert_eq!(s.accent, "#e2a35c");
        assert!(s.allowlist.is_empty());
        assert_eq!(before, after, "INV-24: corrupt file must NOT be overwritten on read");
        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- AC3: invalid accent rejected without writing ----
    #[test]
    fn write_settings_invalid_accent_rejects_without_writing() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = data_root("accent");
        std::env::set_var("HOSTERM_DATA_DIR", &root);
        let path = root.join("hosterm").join("settings.json");

        for bad in ["#e2a35", "#e2a35cc", "e2a35c", "#GGGGGG", "#12 456", ""] {
            let s = Settings {
                version: 1,
                theme: "dusk".into(),
                accent: bad.into(),
                allowlist: vec![],
            };
            let r = write_settings(s);
            assert_eq!(
                r.err().as_deref(),
                Some("settings:invalid-accent"),
                "accent {:?} must reject settings:invalid-accent",
                bad
            );
            assert!(!path.exists(), "no file may be written on reject (accent={:?})", bad);
        }
        std::env::remove_var("HOSTERM_DATA_DIR");
        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- invalid theme rejected without writing ----
    #[test]
    fn write_settings_invalid_theme_rejects_without_writing() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = data_root("theme");
        std::env::set_var("HOSTERM_DATA_DIR", &root);
        let path = root.join("hosterm").join("settings.json");
        let s = Settings {
            version: 1,
            theme: "neon".into(),
            accent: "#e2a35c".into(),
            allowlist: vec![],
        };
        let r = write_settings(s);
        std::env::remove_var("HOSTERM_DATA_DIR");
        assert_eq!(r.err().as_deref(), Some("settings:invalid-theme"));
        assert!(!path.exists(), "no file on invalid-theme reject");
        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- empty allowlist host/command rejected without writing ----
    #[test]
    fn write_settings_empty_allowlist_entry_rejected() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = data_root("emptyentry");
        std::env::set_var("HOSTERM_DATA_DIR", &root);
        let path = root.join("hosterm").join("settings.json");
        let s = Settings {
            version: 1,
            theme: "dusk".into(),
            accent: "#e2a35c".into(),
            allowlist: vec![entry("", "ls")],
        };
        let r = write_settings(s);
        std::env::remove_var("HOSTERM_DATA_DIR");
        assert!(r.is_err(), "empty host must reject");
        assert!(!path.exists(), "no file on bad allowlist entry");
        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- round-trip: write then read gets the same values back ----
    #[test]
    fn write_then_read_roundtrips_and_stamps_added_at() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = data_root("roundtrip");
        std::env::set_var("HOSTERM_DATA_DIR", &root);
        let s = Settings {
            version: 1,
            theme: "dusk".into(),
            accent: "#5fbfae".into(),
            allowlist: vec![entry("prod-web", "sudo systemctl restart netdata")],
        };
        write_settings(s).unwrap();
        let back = read_settings();
        std::env::remove_var("HOSTERM_DATA_DIR");
        assert_eq!(back.theme, "dusk");
        assert_eq!(back.accent, "#5fbfae");
        assert_eq!(back.allowlist.len(), 1);
        assert_eq!(back.allowlist[0].host, "prod-web");
        assert_eq!(back.allowlist[0].command, "sudo systemctl restart netdata");
        // added_at was blank on input; backend stamped an RFC3339 UTC value
        let stamped = &back.allowlist[0].added_at;
        assert!(stamped.ends_with('Z') && stamped.contains('T') && stamped.len() == 20,
            "added_at must be backend-stamped RFC3339 UTC, got {:?}", stamped);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- unknown theme/version coerced to dusk/1 on read ----
    #[test]
    fn read_settings_coerces_unknown_theme_and_invalid_accent() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = data_root("coerce");
        std::env::set_var("HOSTERM_DATA_DIR", &root);
        let path = root.join("hosterm").join("settings.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // valid JSON, but foreign theme/version + bad accent
        std::fs::write(
            &path,
            r#"{"version":99,"theme":"hyperspace","accent":"nothex","allowlist":[]}"#,
        )
        .unwrap();
        let s = read_settings();
        std::env::remove_var("HOSTERM_DATA_DIR");
        assert_eq!(s.version, 1, "unknown version coerced to 1");
        assert_eq!(s.theme, "dusk", "unknown theme coerced to dusk");
        assert_eq!(s.accent, "#e2a35c", "invalid accent coerced to preset default");
        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- AC4 (INV-22, CRITICAL): write_settings never touches ~/.ssh/* ----
    #[test]
    fn write_settings_leaves_ssh_config_and_sidecar_untouched() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        fn hash_bytes(b: &[u8]) -> u64 {
            let mut h = DefaultHasher::new();
            b.hash(&mut h);
            h.finish()
        }

        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // A real ssh config + sidecar in a dedicated dir, via HOSTERM_CONFIG.
        let ssh_root = data_root("inv22-ssh");
        let cfg = ssh_root.join("config");
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        // Create a host with a stored password so both config AND sidecar exist.
        save_host(
            None,
            HostInput {
                name: "prod-web".into(),
                host_name: "203.0.113.10".into(),
                user: "root".into(),
                port: "22".into(),
                identity_file: "".into(),
                auth_method: "password".into(),
                password: "sekret".into(),
            },
        )
        .unwrap();

        let sidecar = ssh_root.join("hosterm.secrets");
        assert!(cfg.exists(), "test setup: config must exist");
        assert!(sidecar.exists(), "test setup: sidecar must exist");

        let cfg_bytes_before = std::fs::read(&cfg).unwrap();
        let sc_bytes_before = std::fs::read(&sidecar).unwrap();
        let cfg_mtime_before = std::fs::metadata(&cfg).unwrap().modified().unwrap();
        let sc_mtime_before = std::fs::metadata(&sidecar).unwrap().modified().unwrap();
        let cfg_hash_before = hash_bytes(&cfg_bytes_before);
        let sc_hash_before = hash_bytes(&sc_bytes_before);

        // Now point the store elsewhere and write settings including an
        // allowlist entry that names the same host + a command — the store
        // must NEVER route this into ~/.ssh/config or the sidecar.
        let store_root = data_root("inv22-store");
        std::env::set_var("HOSTERM_DATA_DIR", &store_root);
        let s = Settings {
            version: 1,
            theme: "ink".into(),
            accent: "#e07856".into(),
            allowlist: vec![entry("prod-web", "docker system prune -af")],
        };
        write_settings(s).unwrap();

        // store file landed in the app-data dir, NOT the ssh dir
        assert!(store_root.join("hosterm").join("settings.json").exists());
        assert!(!cfg.parent().unwrap().join("settings.json").exists());

        let cfg_bytes_after = std::fs::read(&cfg).unwrap();
        let sc_bytes_after = std::fs::read(&sidecar).unwrap();
        let cfg_mtime_after = std::fs::metadata(&cfg).unwrap().modified().unwrap();
        let sc_mtime_after = std::fs::metadata(&sidecar).unwrap().modified().unwrap();

        std::env::remove_var("HOSTERM_DATA_DIR");
        std::env::remove_var("HOSTERM_CONFIG");

        // INV-22: byte-identical hash AND mtime unchanged on both files.
        assert_eq!(hash_bytes(&cfg_bytes_after), cfg_hash_before, "config bytes changed");
        assert_eq!(hash_bytes(&sc_bytes_after), sc_hash_before, "sidecar bytes changed");
        assert_eq!(cfg_mtime_after, cfg_mtime_before, "config mtime changed");
        assert_eq!(sc_mtime_after, sc_mtime_before, "sidecar mtime changed");
        // and the secret never leaked into the store file
        let store_body = std::fs::read_to_string(store_root.join("hosterm").join("settings.json")).unwrap();
        assert!(!store_body.contains("sekret"), "no secret in store: {store_body}");

        let _ = std::fs::remove_dir_all(&ssh_root);
        let _ = std::fs::remove_dir_all(&store_root);
    }

    // ---- read_snippets: no bundled defaults (MEKL: "ga perlu snippet bawaan").
    // The command still parses the embedded asset and NEVER throws; with the
    // asset emptied to [], the bundled set is simply empty. User snippets come
    // from the separate read_user_snippets CRUD store.
    #[test]
    fn read_snippets_returns_empty_bundled_set() {
        let groups = read_snippets();
        assert!(groups.is_empty(), "bundled set must be empty, got {}", groups.len());
    }
}

#[cfg(test)]
mod sftp_tests {
    use super::*;

    /// Live end-to-end test of the REAL sftp_* commands. Skipped unless
    /// HOSTERM_LIVE_SFTP=1 so normal `cargo test` never depends on a server.
    /// Requires a reachable `testbox` host alias in ~/.ssh/config.
    #[test]
    fn live_sftp_roundtrip() {
        if std::env::var("HOSTERM_LIVE_SFTP").as_deref() != Ok("1") {
            return; // skipped in normal runs
        }
        let dir = std::env::temp_dir().join(format!("hosterm-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let local = dir.join("upload_me.txt");
        std::fs::write(&local, "hosterm live sftp test\n").unwrap();
        let remote = format!("{}/landed.txt", dir.display());

        // upload -> list shows it -> download to a new path -> contents match.
        // The sftp_* commands are async (spawn_blocking off the IPC thread);
        // drive them from this sync test with the Tauri async runtime — no new
        // dependency, and it proves the async command resolves to the same
        // Result the frontend sees.
        use tauri::async_runtime::block_on;
        block_on(sftp_upload("testbox".into(), local.display().to_string(), remote.clone())).unwrap();
        let entries = block_on(sftp_list("testbox".into(), dir.display().to_string())).unwrap();
        assert!(
            entries.iter().any(|e| e.name == "landed.txt" && !e.is_dir),
            "uploaded file must appear in listing: {:?}",
            entries.iter().map(|e| &e.name).collect::<Vec<_>>()
        );
        let back = dir.join("came_back.txt");
        block_on(sftp_download("testbox".into(), remote, back.display().to_string())).unwrap();
        assert_eq!(
            std::fs::read_to_string(&back).unwrap(),
            "hosterm live sftp test\n"
        );

        // editor roundtrip: read -> write -> read-back
        let txt_path = format!("{}/editor.txt", dir.display());
        block_on(sftp_write_file("testbox".into(), txt_path.clone(), "hello editor\nline two\n".into())).unwrap();
        assert_eq!(
            block_on(sftp_read_file("testbox".into(), txt_path.clone())).unwrap(),
            "hello editor\nline two\n"
        );
        // binary guard: NUL byte in an uploaded file must be refused by the editor
        let bin_local = dir.join("bin.dat");
        std::fs::write(&bin_local, [0x68, 0x00, 0x69]).unwrap();
        let bin_remote = format!("{}/uploaded_bin.dat", dir.display());
        block_on(sftp_upload("testbox".into(), bin_local.display().to_string(), bin_remote.clone())).unwrap();
        assert!(
            block_on(sftp_read_file("testbox".into(), bin_remote)).is_err(),
            "binary must be refused"
        );

        // aliasing guard: put/get onto the same path must be refused, source intact
        let alias_local = dir.join("alias_me.txt");
        std::fs::write(&alias_local, "precious\n").unwrap();
        let alias_remote = alias_local.display().to_string();
        assert!(block_on(sftp_upload("testbox".into(), alias_remote.clone(), alias_remote.clone())).is_err());
        assert_eq!(std::fs::read_to_string(&alias_local).unwrap(), "precious\n");
        assert!(block_on(sftp_download("testbox".into(), alias_remote.clone(), alias_remote)).is_err());
        assert_eq!(std::fs::read_to_string(&alias_local).unwrap(), "precious\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_ls_basic_entries() {
        let out = "sftp> ls -la .\n\
total 20\n\
drwxr-xr-x    5 user user     4096 Oct  7 10:00 .\n\
drwxr-xr-x    3 root root     4096 Oct  1 09:00 ..\n\
-rw-r--r--    1 user user      142 Oct  7 10:01 notes.txt\n\
drwxr-xr-x    2 user user     4096 Oct  7 10:02 projects\n\
lrwxrwxrwx    1 user user       11 Oct  7 10:03 link -> notes.txt\n";
        let e = parse_sftp_ls(out);
        let names: Vec<&str> = e.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, vec!["notes.txt", "projects", "link"]);
        assert!(!e[0].is_dir && !e[0].is_link);
        assert_eq!(e[0].size, 142);
        assert!(e[1].is_dir);
        assert!(e[2].is_link && e[2].name == "link"); // " -> target" trimmed
    }

    #[test]
    fn parse_ls_name_with_spaces() {
        let out = "-rw-r--r--    1 u u    10 Oct  7 10:00 my file.txt\n";
        let e = parse_sftp_ls(out);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].name, "my file.txt");
    }

    #[test]
    fn parse_ls_dotslash_prefixed_names_yield_basenames() {
        // BUG2: some servers print "./name" in `ls -la` output. The frontend
        // joins dir + name, so a "./" prefix produced /home//home/ubuntu-style
        // garbage. Parser must strip it and emit a bare basename.
        let out = "sftp> ls -la .\n\
total 8\n\
drwxr-xr-x    3 root root     4096 Oct  1 09:00 ./.\n\
drwxr-xr-x    3 root root     4096 Oct  1 09:00 ./..\n\
drwxr-xr-x    2 user user     4096 Oct  7 10:02 ./ubuntu\n\
-rw-r--r--    1 user user      142 Oct  7 10:01 ./notes.txt\n";
        let e = parse_sftp_ls(out);
        let names: Vec<&str> = e.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, vec!["ubuntu", "notes.txt"]);
        assert!(e[0].is_dir);
    }

    #[test]
    fn parse_ls_path_prefixed_names_yield_basenames() {
        // Harder variant: a server prints a full/relative path as the name.
        // Parser must still emit the final segment, not the whole path.
        let out = "drwxr-xr-x    2 user user     4096 Oct  7 10:02 /home/ubuntu\n\
-rw-r--r--    1 user user      142 Oct  7 10:01 some/dir/file.txt\n\
lrwxrwxrwx    1 user user       11 Oct  7 10:03 ./link -> /tmp/target\n";
        let e = parse_sftp_ls(out);
        let names: Vec<&str> = e.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, vec!["ubuntu", "file.txt", "link"]);
        assert!(e[0].is_dir);
        assert!(e[2].is_link && e[2].name == "link");
    }

    #[test]
    fn parse_ls_ignores_noise() {
        let out = "sftp> cd /tmp\nsftp> ls -la\ntotal 0\n\n";
        assert!(parse_sftp_ls(out).is_empty());
    }

    #[test]
    fn sftp_safe_rejects_option_and_newline_injection() {
        assert!(sftp_safe("-oProxyCommand=calc", "Host").is_err());
        assert!(sftp_safe("ok\nrm -rf x", "Path").is_err());
        assert!(sftp_safe("/home/user/file.txt", "Path").is_ok());
    }

}

// ---- W2-A: check_binaries pure-core tests (FR-SSH-1 / AC-A1.1, NFR-W2-4) ----

#[cfg(test)]
mod bin_check_tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("hosterm-binchk-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// Fake "bin dir": writes `ssh`/`sftp` (`.exe` on Windows) as plain files.
    fn bin_dir(tag: &str, files: &[&str]) -> PathBuf {
        let dir = tmp(tag);
        for f in files {
            let name = if cfg!(windows) { format!("{f}.exe") } else { f.to_string() };
            std::fs::write(dir.join(&name), b"#!/bin/sh\n").unwrap();
        }
        dir
    }

    fn in_dir(dir: &PathBuf, name: &str) -> PathBuf {
        dir.join(if cfg!(windows) { format!("{name}.exe") } else { name.to_string() })
    }

    #[test]
    fn all_four_result_states() {
        let both = bin_dir("both", &["ssh", "sftp"]);
        assert_eq!(binaries_missing(&[both], None, None), (false, false), "both present");

        let ssh_only = bin_dir("sshonly", &["ssh"]);
        assert_eq!(
            binaries_missing(&[ssh_only], None, None),
            (false, true),
            "missing: sftp only — result must name the missing one"
        );

        let sftp_only = bin_dir("sftponly", &["sftp"]);
        assert_eq!(
            binaries_missing(&[sftp_only], None, None),
            (true, false),
            "missing: ssh only"
        );

        let none = bin_dir("none", &[]);
        assert_eq!(binaries_missing(&[none], None, None), (true, true), "missing: both");
        for tag in ["both", "sshonly", "sftponly", "none"] {
            let _ = std::fs::remove_dir_all(tmp(tag));
        }
    }

    #[test]
    fn system_dir_fallback_used_for_both() {
        // analog of %SystemRoot%\System32\OpenSSH holding ssh only
        let sysdir = bin_dir("sys", &["ssh"]);
        assert_eq!(
            binaries_missing(&[], Some(&sysdir), None),
            (false, true),
            "ssh found via system dir when PATH empty"
        );

        // existing-but-empty system dir resolves nothing
        let empty = bin_dir("sysempty", &[]);
        assert_eq!(binaries_missing(&[], Some(&empty), None), (true, true));

        // PATH hit takes precedence order; either location satisfies
        let sys_ssh = bin_dir("sysssh", &["ssh"]);
        let path_sftp = bin_dir("pathsftp", &["sftp"]);
        assert_eq!(binaries_missing(&[path_sftp], Some(&sys_ssh), None), (false, false));
        for tag in ["sys", "sysempty", "sysssh", "pathsftp"] {
            let _ = std::fs::remove_dir_all(tmp(tag));
        }
    }

    #[test]
    fn sftp_override_replaces_lookup() {
        let with_sftp = bin_dir("ovrpath", &["ssh", "sftp"]);
        let fake = bin_dir("ovrbin", &["sftp"]);
        let real = in_dir(&fake, "sftp");

        // override present → sftp found via the override path (in `fake`),
        // not via the PATH dir — override is honored
        assert_eq!(binaries_missing(&[with_sftp.clone()], None, Some(real.as_os_str())), (false, false));

        // dangling override → sftp missing even though PATH has a real sftp
        let dangling = fake.join("does-not-exist.bin");
        assert_eq!(
            binaries_missing(&[with_sftp], None, Some(dangling.as_os_str())),
            (false, true),
            "a set HOSTERM_SFTP_BIN must REPLACE the PATH lookup (lib.rs:721/856)"
        );
        for tag in ["ovrpath", "ovrbin"] {
            let _ = std::fs::remove_dir_all(tmp(tag));
        }
    }

    // ---- Ansible v1: ansible_missing pure-core tests (FR-ANS1, NFR-ANS2) ----

    /// Fake bin dir writing `ansible-playbook` (`.exe` on Windows) as a file.
    fn ansible_bin_dir(tag: &str, present: bool) -> PathBuf {
        let dir = tmp(tag);
        if present {
            let name = if cfg!(windows) { "ansible-playbook.exe" } else { "ansible-playbook" };
            std::fs::write(dir.join(name), b"#!/bin/sh\n").unwrap();
        }
        dir
    }

    #[test]
    fn ansible_missing_resolves_in_path() {
        // present in one PATH dir → not missing
        let present = ansible_bin_dir("ans-present", true);
        assert!(!ansible_missing(&[present]), "ansible-playbook present ⇒ missing=false");

        // absent everywhere → missing
        let absent = ansible_bin_dir("ans-absent", false);
        assert!(ansible_missing(&[absent]), "ansible-playbook absent ⇒ missing=true");

        // empty PATH → missing
        assert!(ansible_missing(&[]), "no PATH dirs ⇒ missing=true");

        // first-hit-wins across multiple dirs (mirrors binaries_missing)
        let empty = ansible_bin_dir("ans-empty", false);
        let hit = ansible_bin_dir("ans-hit", true);
        assert!(
            !ansible_missing(&[empty, hit]),
            "resolvable in a later PATH dir ⇒ missing=false"
        );

        for tag in ["ans-present", "ans-absent", "ans-empty", "ans-hit"] {
            let _ = std::fs::remove_dir_all(tmp(tag));
        }
    }

    #[test]
    fn ansible_missing_rejects_non_file() {
        // a directory named like the binary must NOT count as resolvable
        let dir = tmp("ans-dir");
        let name = if cfg!(windows) { "ansible-playbook.exe" } else { "ansible-playbook" };
        std::fs::create_dir_all(dir.join(name)).unwrap();
        assert!(ansible_missing(&[dir]), "a dir named ansible-playbook is not a binary ⇒ missing=true");
        let _ = std::fs::remove_dir_all(tmp("ans-dir"));
    }
}

// ============================================================
//  GOOGLE OAUTH — thin IPC wrappers (logic lives in oauth.rs)
// ============================================================
//
// Secrets NEVER cross IPC: these return only the public GoogleAccount
// {name,email,picture}. The access_token stays backend-side; the
// refresh_token lives in the OS keyring (oauth.rs), never a file, never logged.
// The heavy lifting (PKCE loopback, token exchange/refresh, keyring) is in
// oauth.rs — no business logic here (TSD §2,§3).

/// Run the Google PKCE loopback sign-in and return the account chip fields.
#[tauri::command]
fn google_sign_in() -> Result<oauth::GoogleAccount, String> {
    oauth::interactive_sign_in()
}

/// Sign out: delete the keyring refresh_token (idempotent).
#[tauri::command]
fn google_sign_out() -> Result<(), String> {
    oauth::sign_out()
}

/// Signed-in state on app load; None when no refresh_token is stored.
#[tauri::command]
fn google_account() -> Result<Option<oauth::GoogleAccount>, String> {
    oauth::load_account()
}

// ============================================================
//  DRIVE BACKUP/RESTORE — thin IPC wrappers (logic lives in backup.rs)
// ============================================================
//
// Secrets NEVER cross IPC except the passphrase (unavoidable — the dialog is
// in the WebView, KDF is in Rust, TSD §3). The passphrase arg is moved into a
// `Zeroizing<String>` on entry so it is wiped from memory on drop, and it is
// NEVER cloned into logs/errors/persistence. All gather/encrypt/upload and the
// fail-safe restore orchestration live in backup.rs (TSD §2,§5).

/// Gather the four sources → encrypt with the passphrase → upload to Drive.
#[tauri::command]
fn drive_backup(passphrase: String) -> Result<backup::BackupResult, String> {
    let passphrase = zeroize::Zeroizing::new(passphrase);
    backup::prod::backup(passphrase)
}

/// List the user's `.htbk` backups via cleartext appProperties (no decrypt).
#[tauri::command]
fn drive_list_backups() -> Result<Vec<backup::BackupMeta>, String> {
    backup::prod::list_backups()
}

/// Download → decrypt (fail-safe on the AEAD tag) → pre-restore backup → write
/// the real paths atomically. A wrong passphrase fails before anything is
/// touched on disk.
#[tauri::command]
fn drive_restore(
    file_id: String,
    passphrase: String,
) -> Result<backup::RestoreResult, String> {
    let passphrase = zeroize::Zeroizing::new(passphrase);
    backup::prod::restore(file_id, passphrase)
}

/// Return the fixed known restore target set + which currently exist, for the
/// FE overwrite warning. No passphrase (the manifest is encrypted).
#[tauri::command]
fn drive_restore_preflight(_file_id: String) -> Result<backup::RestorePreflight, String> {
    Ok(backup::preflight())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(PtyState::default())
        .manage(fleet::FleetState::default())
        .invoke_handler(tauri::generate_handler![
            read_ssh_config,
            save_host,
            delete_host,
            read_config_raw,
            write_config_raw,
            list_identity_files,
            create_identity_file,
            delete_identity_file,
            sftp_list,
            local_list,
            home_dir,
            clipboard_write,
            clipboard_read,
            sftp_upload,
            sftp_download,
            sftp_read_file,
            sftp_write_file,
            pty_spawn,
            local_pty_spawn,
            pty_write,
            pty_resize,
            pty_kill,
            check_binaries,
            check_ansible,
            forget_secret,
            read_settings,
            write_settings,
            read_snippets,
            read_user_snippets,
            write_user_snippets,
            local_shell_info,
            google_sign_in,
            google_sign_out,
            google_account,
            drive_backup,
            drive_list_backups,
            drive_restore,
            drive_restore_preflight,
            fleet::fleet_start,
            fleet::fleet_stop
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
