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

/// Validate a managed-key filename and resolve it to its path inside keys_dir().
/// Rejects empty names, path separators, traversal, and (defence in depth) any
/// resolved path whose parent is not exactly keys_dir(). This is the single
/// trust-boundary gate shared by every managed-key mutation.
fn managed_key_path(name: &str) -> Result<(PathBuf, PathBuf), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Key name cannot be empty".into());
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err("Key name must be a plain filename (no path separators)".into());
    }
    let dir = keys_dir();
    let path = dir.join(name);
    if path.parent() != Some(dir.as_path()) {
        return Err("Refusing to operate outside the managed keys directory".into());
    }
    Ok((dir, path))
}

/// Atomically overwrite a key hosterm manages with new contents. Scoped to
/// `~/.ssh/keys/` ONLY (legacy ~/.ssh/* keys are never writable here). The key
/// must already exist — this is an edit, not a create (use create_identity_file
/// for a new key). Write is atomic: sibling temp → chmod 0600 → rename over the
/// target, so a crash mid-write leaves the original key intact. Contents are
/// never logged. LF-normalized with a guaranteed trailing newline (ssh is picky).
#[tauri::command]
fn write_identity_file(name: String, contents: String) -> Result<String, String> {
    let (dir, path) = managed_key_path(&name)?;
    if contents.trim().is_empty() {
        return Err("Private key text is empty".into());
    }
    if !path.exists() {
        return Err(format!("No key named '{}' in ~/.ssh/keys", name.trim()));
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    let mut body = contents.replace("\r\n", "\n");
    if !body.ends_with('\n') {
        body.push('\n');
    }
    // Atomic write: sibling temp → chmod 0600 → rename over the target.
    let tmp = {
        let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("key");
        let mut t = path.clone();
        t.set_file_name(format!(".{}.hosterm-tmp-{}", fname, std::process::id()));
        t
    };
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })?;
    Ok(format!("~/.ssh/keys/{}", name.trim()))
}

/// Rename a key hosterm manages. Both old and new names are validated against
/// the managed-keys trust boundary (bare filename, no traversal, resolved parent
/// == keys_dir()). The source must exist; the destination must NOT (never clobber
/// another key). A matching `<old>.pub` is renamed alongside. Legacy ~/.ssh/*
/// keys are never renamable here. Returns the new `~/.ssh/keys/<new>` path.
#[tauri::command]
fn rename_identity_file(old: String, new: String) -> Result<String, String> {
    let (dir, old_path) = managed_key_path(&old)?;
    let (_, new_path) = managed_key_path(&new)?;
    if !old_path.exists() {
        return Err(format!("No key named '{}' in ~/.ssh/keys", old.trim()));
    }
    if old_path == new_path {
        // No-op rename: nothing to do, report the stable path.
        return Ok(format!("~/.ssh/keys/{}", new.trim()));
    }
    if new_path.exists() {
        return Err(format!("A key named '{}' already exists", new.trim()));
    }
    std::fs::rename(&old_path, &new_path).map_err(|e| e.to_string())?;
    // Move the public half if present; its absence is not an error.
    let old_pub = dir.join(format!("{}.pub", old.trim()));
    if old_pub.exists() {
        let new_pub = dir.join(format!("{}.pub", new.trim()));
        let _ = std::fs::rename(&old_pub, &new_pub);
    }
    Ok(format!("~/.ssh/keys/{}", new.trim()))
}

/// Read the contents of a key hosterm manages so the Edit dialog can prefill the
/// textarea. Scoped to `~/.ssh/keys/` ONLY through the same `managed_key_path`
/// trust boundary as write/rename: legacy `~/.ssh/*` keys are NOT readable here
/// (consistent with legacy keys being read-only and having no Edit button), and
/// traversal / path separators / out-of-dir names are refused. The key must
/// exist. Contents are returned verbatim (no normalization) and NEVER logged.
#[tauri::command]
fn read_identity_file(name: String) -> Result<String, String> {
    let (_dir, path) = managed_key_path(&name)?;
    if !path.exists() {
        return Err(format!("No key named '{}' in ~/.ssh/keys", name.trim()));
    }
    std::fs::read_to_string(&path).map_err(|e| e.to_string())
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
    /// Returns None on timeout or EOF-without-match.
    fn wait_for(&self, needle: &str, timeout: std::time::Duration) -> Option<String> {
        let deadline = std::time::Instant::now() + timeout;
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
            if self.eof.load(Ordering::SeqCst) {
                return None;
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// Current buffered text without consuming it.
    fn snapshot(&self) -> String {
        String::from_utf8_lossy(&self.data.lock().unwrap()).to_string()
    }

    /// Consume and return everything currently buffered.
    fn drain(&self) -> String {
        let mut d = self.data.lock().unwrap();
        let s = String::from_utf8_lossy(&d).to_string();
        d.clear();
        s
    }

    /// Last `n` chars of the buffered text without consuming it.
    fn tail(&self, n: usize) -> String {
        let s = self.snapshot();
        let chars: Vec<char> = s.chars().collect();
        let start = chars.len().saturating_sub(n);
        chars[start..].iter().collect()
    }

    /// Drain once the buffer has been silent for `quiet`, bounded by `max_wait`.
    fn drain_idle(&self, quiet: std::time::Duration, max_wait: std::time::Duration) -> String {
        let deadline = std::time::Instant::now() + max_wait;
        let mut last_len = self.snapshot().len();
        let mut last_change = std::time::Instant::now();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(10));
            let cur = self.snapshot();
            if cur.len() != last_len {
                last_len = cur.len();
                last_change = std::time::Instant::now();
            }
            if last_change.elapsed() >= quiet || std::time::Instant::now() >= deadline {
                return self.drain();
            }
        }
    }
}

/// Format a raw PTY/output snippet for an error message: control characters
/// (CR from the tty, ANSI-ish noise) become spaces so the note stays on one
/// logical line. Empty input yields an empty note.
fn raw_note(tail: &str) -> String {
    let cleaned: String = tail
        .chars()
        .map(|c| if c.is_control() && c != '\n' && c != '\t' { ' ' } else { c })
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        String::new()
    } else {
        format!(" — sftp output: {}", cleaned)
    }
}

fn pty_tail_note(rx: &SftpBuf) -> String {
    raw_note(&rx.tail(400))
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

/// Line ending written to the sftp PTY. ConPTY's input parser expects CR for
/// Enter (the terminal encodes VK_RETURN as `\r`); a Unix PTY expects LF, and
/// its ICRNL would fold a CR to LF anyway. Writing LF on Windows would never
/// submit the line.
fn sftp_eol() -> &'static [u8] {
    if cfg!(windows) {
        b"\r"
    } else {
        b"\n"
    }
}

/// Spawn one interactive `sftp` in a PTY and wait for login to finish, typing
/// the stored password at each OpenSSH password prompt.
///
/// Login completion never relies on the interactive `sftp> ` prompt: Win32-
/// OpenSSH's sftp.exe does not print it. sftp.c compiles with the three
/// `allocate_standard_descriptor()` calls commented out, so `isatty(stdin)` is
/// false, `interactive` is false, and the `printf("sftp> ")` branch is dead.
/// Instead the login wait accepts either signal:
///   * the plain prompt (Linux/interactive — unchanged behavior), or
///   * the echo of a `pwd` probe, `sftp> pwd`, which BOTH modes print
///     (non-interactive: `printf("sftp> %s", cmd)`; interactive: prompt +
///     libedit/readline echo). sftp can only read the probe after login, so
///     its echo is a reliable login-complete marker on Windows too.
///
/// The stored password is supplied through `SSH_ASKPASS` (see below); the PTY
/// typing is only the fallback path when no helper could be resolved.
///
/// `SSH_ASKPASS` is set for password hosts. Win32-OpenSSH's `read_passphrase()`
/// consults it *instead* of the console and never falls back, and `ssh_askpass()`
/// launches it with `CreateProcess("\"%s\" \"%s\"")` (readpass.c:175,184) — one
/// quoted argument, no shell, so the helper must be a real `.exe`, never a
/// `.cmd` shim (a shim yields an empty password and three denials).
///
/// The console path is not usable under ConPTY: the UCRT's `_getch_nolock()`
/// initialises its console handle from `GetStdHandle(STD_INPUT_HANDLE)`, which
/// ConPTY hands the child as the input *pipe*, so `_getch()` returns EOF
/// immediately and the password read yields garbage. `choice.exe`/`pause` do
/// see ConPTY input (proven on Windows CI by `conpty_input_reaches_child` and
/// `conpty_choice_receives_input`) because they open `CONIN$` directly — only
/// the CRT's std-handle path is blind. The console fallback below is therefore
/// kept only as a best-effort secondary path; `SSH_ASKPASS` is the primary one.

fn sftp_spawn(host: &str) -> Result<SftpPtySession, String> {
    sftp_safe(host, "Host")?;
    let stored_pw = lookup_stored_password(host);
    // HOSTERM_SFTP_BIN: testability hook, same as the batch path
    let bin = std::env::var("HOSTERM_SFTP_BIN").unwrap_or_else(|_| "sftp".to_string());
    let mut cmd = CommandBuilder::new(bin);
    cmd.arg(host);
    #[cfg(windows)]
    if stored_pw.is_some() {
        // The helper reads the sidecar directly, so the password never crosses
        // the PTY (where it could be echoed back into the listing) or the IPC
        // boundary. `HOSTERM_ASKPASS_BIN` lets the Windows live test point at
        // a helper compiled on the fly, since `cargo test --lib` builds no bins.
        let helper = askpass_exe();
        if !helper.as_os_str().is_empty() {
            cmd.env("SSH_ASKPASS", helper);
            // `SSH_ASKPASS_REQUIRE=force` makes openssh-portable's
            // read_passphrase() take the askpass path even without a DISPLAY
            // (its `use_askpass && allow_askpass` gate is otherwise false on a
            // headless Windows console). Win32-OpenSSH ignores the variable and
            // keys off SSH_ASKPASS alone, so this is safe for both builds.
            cmd.env("SSH_ASKPASS_REQUIRE", "force");
            // Some Win32-OpenSSH builds gate the askpass branch behind
            // DISPLAY; a dummy value satisfies the gate (there is no X server,
            // but the helper is a plain console .exe).
            cmd.env("DISPLAY", "1");
            cmd.env("HOSTERM_ASKPASS_FILE", secrets_path());
            cmd.env("HOSTERM_ASKPASS_HOST", host);
        }
    }
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

    let sess = SftpPtySession {
        master: pair.master,
        writer,
        rx,
        alive,
    };
    // Windows console-password fallback (secondary: `SSH_ASKPASS` above is the
    // primary path, because the CRT's `_getch()` is blind under ConPTY — it
    // binds to `GetStdHandle(STD_INPUT_HANDLE)`, which ConPTY hands the child as
    // the input pipe, so the read yields EOF instead of a key). This fallback
    // only matters if no helper could be resolved. Win32-OpenSSH's
    // read_passphrase() also flushes the console input buffer right after
    // printing the prompt (`while (_kbhit()) _getch();`), so a sacrificial CR is
    // written when the prompt is *seen*; the real password follows once the
    // flush has drained. It must be written at prompt time: ConPTY can drop
    // input written before the child console is up, which is why a pre-spawn CR
    // never protected the password. If the sacrificial itself is read as the
    // password (a fresh prompt arrives before the password is sent) the flush
    // is not blocking, so we stop using it and send the password directly.
    #[cfg(windows)]
    let pw_grace = std::time::Duration::from_millis(120);
    #[cfg(windows)]
    let pw_quiet = std::time::Duration::from_millis(400);
    #[cfg(windows)]
    let mut sacrificial = stored_pw.is_some();
    #[cfg(windows)]
    let mut pending_pw: Option<std::time::Instant> = None;
    #[cfg(windows)]
    let mut last_pw_write: Option<std::time::Instant> = None;
    let timeout = if stored_pw.is_some() { 30 } else { 10 };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout);
    let spawned_at = std::time::Instant::now();
    let quiet = std::time::Duration::from_millis(250);
    let mut prompts_seen = 0usize;
    let mut probe_sent = false;
    let mut last_len = 0usize;
    let mut last_change = std::time::Instant::now();

    loop {
        if sess.rx.eof.load(Ordering::SeqCst) {
            sess.alive.store(false, Ordering::SeqCst);
            return Err(format!(
                "sftp: session ended before login (wrong password or host unreachable){}",
                pty_tail_note(&sess.rx)
            ));
        }
        let snap = sess.rx.snapshot();
        if snap.len() != last_len {
            last_len = snap.len();
            last_change = std::time::Instant::now();
        }
        // Answer every password prompt OpenSSH prints (it retries up to three
        // times). The prompt has no trailing newline, so count occurrences.
        let prompts = snap.matches("'s password:").count();
        if prompts > prompts_seen {
            prompts_seen = prompts;
            if let Some(pw) = stored_pw.as_deref() {
                #[cfg(windows)]
                {
                    // The flush that follows the prompt drains every key event
                    // already queued, so the first answer must be a throwaway
                    // CR; the real password goes out after it. From the second
                    // prompt on we already know the flush is not blocking, so
                    // the real password is sent directly (a sacrificial here
                    // would itself be taken as the password).
                    if prompts_seen >= 2 || pending_pw.is_some() {
                        sacrificial = false;
                        pending_pw = None;
                    }
                    if sacrificial {
                        let mut w = sess.writer.lock().unwrap();
                        let _ = w.write_all(b"\r");
                        let _ = w.flush();
                        pending_pw = Some(std::time::Instant::now() + pw_grace);
                    } else {
                        let mut w = sess.writer.lock().unwrap();
                        let _ = w.write_all(pw.as_bytes());
                        let _ = w.write_all(b"\r");
                        let _ = w.flush();
                        last_pw_write = Some(std::time::Instant::now());
                    }
                    #[cfg(windows)]
                    eprintln!(
                        "[winlive] pw prompt #{} at t={:?} sacrificial={} pending={}",
                        prompts,
                        spawned_at.elapsed(),
                        sacrificial,
                        pending_pw.is_some()
                    );
                }
                #[cfg(not(windows))]
                {
                    let mut w = sess.writer.lock().unwrap();
                    let _ = w.write_all(pw.as_bytes());
                    let _ = w.write_all(b"\r");
                    let _ = w.flush();
                }
            }
            // A probe sent before this prompt may have been swallowed by the
            // password reader (Windows flushes pending input first); arm a
            // fresh one so it goes out after the next quiet period.
            probe_sent = false;
        }
        #[cfg(windows)]
        if let Some(t) = pending_pw {
            if std::time::Instant::now() >= t {
                if let Some(pw) = stored_pw.as_deref() {
                    let mut w = sess.writer.lock().unwrap();
                    let _ = w.write_all(pw.as_bytes());
                    let _ = w.write_all(b"\r");
                    let _ = w.flush();
                    last_pw_write = Some(std::time::Instant::now());
                }
                pending_pw = None;
            }
        }
        // Linux/interactive: the plain prompt means login finished. Guard on
        // `!probe_sent` so a probe echo (`sftp> pwd`) is never mistaken for it.
        if !probe_sent && snap.contains("sftp> ") {
            let _ = sess.rx.wait_for("sftp> ", std::time::Duration::from_millis(50));
            let _ = sess.rx.drain();
            return Ok(sess);
        }
        // Windows/non-interactive: nothing is printed after a successful login,
        // so once the output has gone quiet, probe with `pwd` and wait for its
        // echo. Before any password prompt, only probe on a no-password host
        // (or after 5s, as a fallback for a stored password that is never
        // asked for): on a password host an early probe would be read as the
        // password.
        #[cfg(windows)]
        let may_probe = stored_pw.is_none()
            // Only probe once the password has been written AND the output has
            // gone quiet: a probe that races the login is read as the password.
            || last_pw_write
                .map(|t| t.elapsed() >= pw_quiet)
                .unwrap_or(false)
            || spawned_at.elapsed() >= std::time::Duration::from_secs(5);
        #[cfg(not(windows))]
        let may_probe = stored_pw.is_none()
            || prompts_seen > 0
            || spawned_at.elapsed() >= std::time::Duration::from_secs(5);
        if !probe_sent && last_change.elapsed() >= quiet && may_probe {
            let mut w = sess.writer.lock().unwrap();
            let _ = w.write_all(b"pwd");
            let _ = w.write_all(sftp_eol());
            let _ = w.flush();
            probe_sent = true;
            last_change = std::time::Instant::now();
            continue;
        }
        if probe_sent {
            if sess.rx.wait_for("sftp> pwd", std::time::Duration::from_millis(20)).is_some() {
                // Drain the probe output (and the next prompt, if any) so
                // every command starts from a clean buffer.
                let _ = sess.rx.drain_idle(
                    std::time::Duration::from_millis(250),
                    std::time::Duration::from_secs(2),
                );
                return Ok(sess);
            }
        }
        if std::time::Instant::now() >= deadline {
            sess.alive.store(false, Ordering::SeqCst);
            #[cfg(windows)]
            eprintln!(
                "[winlive] sftp_spawn gave up at t={:?}; full output: {:?}",
                spawned_at.elapsed(),
                sess.rx.snapshot()
            );
            return Err(format!(
                "sftp: no prompt after login (wrong password or host unreachable){}",
                pty_tail_note(&sess.rx)
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Run one command in the persistent session for `host`. Completion is the
/// echo of a `pwd` sentinel that follows the command — never the interactive
/// `sftp> ` prompt, which Win32-OpenSSH's sftp.exe does not print (see
/// `sftp_spawn`). Both modes echo the line sftp read, so the buffer always
/// contains `<cmd echo>` … output … `sftp> pwd`; the command output is the
/// text between the command echo and the sentinel echo. A dead or timed-out
/// session is discarded and the command is retried once on a fresh login.
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
        let g = sess.lock().unwrap();
        {
            let mut w = g.writer.lock().unwrap();
            if w.write_all(cmdline.as_bytes()).is_err()
                || w.write_all(sftp_eol()).is_err()
                || w.write_all(b"pwd").is_err()
                || w.write_all(sftp_eol()).is_err()
                || w.flush().is_err()
            {
                g.alive.store(false, Ordering::SeqCst);
                continue; // session died: respawn and retry
            }
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            if let Some(head) = g.rx.wait_for("sftp> pwd", std::time::Duration::from_millis(20)) {
                // Linux prints the next prompt right after the sentinel output;
                // Windows prints nothing more. Wait briefly for the prompt so
                // Linux keeps its zero-idle latency, then drain whatever
                // arrived so the next command starts from a clean buffer.
                let _ = g.rx.wait_for("sftp> ", std::time::Duration::from_millis(150));
                let raw = format!("{}{}", head, g.rx.drain());
                let out = strip_probe_echo(&raw, cmdline);
                return SftpBatchOutput { stdout: out, stderr: String::new(), ok: true };
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        g.alive.store(false, Ordering::SeqCst);
        // EOF or timeout: respawn and retry
    }
    SftpBatchOutput { stdout: String::new(), stderr: "sftp session unreachable".into(), ok: false }
}

/// Slice the command output out of the PTY buffer: strip the echo of the
/// command and the trailing sentinel echo (`sftp> pwd`).
///
/// Two echo layers can appear, in either order:
///   * the console's own input echo (`ENABLE_ECHO_INPUT` is on in ConPTY's
///     default input mode, and a Unix PTY echoes too), which prints the bare
///     line with no `sftp> ` prefix;
///   * sftp's own echo — Windows non-interactive `printf("sftp> %s", cmd)`,
///     Linux interactive prompt + libedit echo — which carries the prefix.
///
/// Anchoring on the prefixed form keeps the bare console echo from being
/// mistaken for the command echo. A trailing bare echo of the sentinel (it
/// lands just before the sentinel's prefixed echo when the console echoes on
/// read) is stripped too.
fn strip_probe_echo(raw: &str, cmdline: &str) -> String {
    // Skip the line ending that follows an anchor (`\n` or CRLF) so the
    // extraction works on both Windows and Unix PTY output.
    fn skip_eol(s: &str, pos: usize) -> usize {
        let b = s.as_bytes();
        let mut p = pos;
        if p < b.len() && b[p] == b'\r' {
            p += 1;
        }
        if p < b.len() && b[p] == b'\n' {
            p += 1;
        }
        p
    }
    let cmd_echo = format!("sftp> {}", cmdline);
    let sent_echo = "sftp> pwd";
    let start = match raw.find(&cmd_echo) {
        Some(p) => skip_eol(raw, p + cmd_echo.len()),
        None => {
            // Fallback for a console that echoes without the prefix: anchor
            // on the bare command line.
            raw.find(cmdline)
                .map(|p| skip_eol(raw, p + cmdline.len()))
                .unwrap_or(0)
        }
    };
    let end = raw[start..]
        .find(sent_echo)
        .map(|p| start + p)
        .unwrap_or(raw.len());
    let mut out = raw[start..end].to_string();
    // The console may echo the sentinel line just before its prefixed echo.
    for suf in ["pwd\r\n", "pwd\n"] {
        if out.ends_with(suf) {
            out.truncate(out.len() - suf.len());
            break;
        }
    }
    out.trim_end().to_string()
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
                // `put` has no exit code, so the listing is the only signal.
                // Surface the raw put output too — a permission/format failure
                // ("dest open …: Permission denied") is otherwise invisible.
                err = format!(
                    "upload verification failed: {} not found in {}{}",
                    name,
                    target_of(&lines[i]),
                    raw_note(outs.last().map(|s| s.as_str()).unwrap_or(""))
                );
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
    /// BE-ANS3 / LQ-2: ephemeral temp files (inventory + cfg) an Ansible run
    /// generated, keyed by pty id. Unlinked when the run's PTY exits (pump
    /// thread end) OR when `pty_kill` is called — backend-side, never contingent
    /// on the frontend being alive (INV-29).
    ephemeral: Mutex<HashMap<u32, Vec<PathBuf>>>,
}

/// LQ-2 ephemeral cleanup (INV-29): unlink every temp file registered against
/// `id` and drop the entry. Idempotent — a double call (exit then kill, or
/// kill then exit) is a no-op the second time. Errors are swallowed: a file
/// already gone is success. Called from BOTH the pty exit path and `pty_kill`.
fn cleanup_ephemeral(ephemeral: &Mutex<HashMap<u32, Vec<PathBuf>>>, id: u32) {
    let paths = ephemeral.lock().unwrap().remove(&id);
    if let Some(paths) = paths {
        for p in paths {
            let _ = std::fs::remove_file(&p);
        }
    }
}

/// Incremental UTF-8 decoder for a raw byte stream.
///
/// A PTY `read()` returns up to 4096 bytes at ARBITRARY boundaries, so a
/// multi-byte UTF-8 character (box-drawing, CJK, emoji) can be split across
/// two reads. Decoding each chunk independently with `String::from_utf8_lossy`
/// replaces the split halves with U+FFFD on both sides — the character is
/// destroyed permanently, and a full-screen TUI that repaints a burst of
/// Unicode (k9s/vim/htop/tmux switching views) is left garbled for good.
///
/// This helper instead carries the incomplete trailing bytes in `pending`
/// until the rest of the character arrives:
///
/// - `Ok`  -> the whole buffer is valid: emit it and clear `pending`.
/// - `Err` with `error_len() == None` -> an INCOMPLETE sequence at the end:
///   emit the valid prefix, keep the tail for the next call.
/// - `Err` with `Some(len)` -> genuinely invalid bytes: emit the valid prefix
///   plus ONE U+FFFD for those `len` bytes, then keep consuming.
///
/// `pending` is caller-owned and must be flushed with
/// `String::from_utf8_lossy` once the stream hits EOF.
fn decode_utf8_stream(pending: &mut Vec<u8>, incoming: &[u8]) -> String {
    pending.extend_from_slice(incoming);
    let mut out = String::new();
    loop {
        match std::str::from_utf8(pending) {
            Ok(s) => {
                out.push_str(s);
                pending.clear();
                return out;
            }
            Err(e) => {
                let valid = e.valid_up_to();
                // `valid_up_to()` is a char boundary by contract, so this is
                // lossless — `from_utf8_lossy` here can never substitute.
                out.push_str(&String::from_utf8_lossy(&pending[..valid]));
                match e.error_len() {
                    // Incomplete trailing sequence: hold it for the next chunk.
                    None => {
                        pending.drain(..valid);
                        return out;
                    }
                    // Genuinely invalid bytes: one replacement char, resume after.
                    Some(len) => {
                        out.push('\u{FFFD}');
                        pending.drain(..valid + len);
                    }
                }
            }
        }
    }
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
    // Undecoded trailing bytes carried across reads (see decode_utf8_stream).
    let mut pending: Vec<u8> = Vec::new();
    let mut injected = false;
    loop {
        match reader.read(&mut buf) {
            Ok(0) => {
                // EOF: flush whatever is left undecoded, best effort. A
                // character truncated by the stream ending still becomes
                // U+FFFD here — that is the documented fallback.
                if !pending.is_empty() {
                    on_chunk(&String::from_utf8_lossy(&pending));
                    pending.clear();
                }
                break;
            }
            Err(_) => break,
            Ok(n) => {
                let s = decode_utf8_stream(&mut pending, &buf[..n]);
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

/// Resolve the stored password for `host` out of the sidecar file named by
/// `HOSTERM_ASKPASS_FILE`. Shared by the in-process askpass entry point and the
/// `hosterm-askpass` helper binary so both honour the same contract.
fn askpass_password(host: &str) -> Option<String> {
    let path = std::env::var_os("HOSTERM_ASKPASS_FILE")?;
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .find_map(|line| line.split_once('=').filter(|(a, _)| *a == host))
        .map(|(_, pw)| pw.to_string())
        .filter(|p| !p.is_empty())
}

/// Windows `SSH_ASKPASS` entry point.
///
/// Win32-OpenSSH's `read_passphrase()` consults `SSH_ASKPASS` *instead* of the
/// console and never falls back, and `ssh_askpass()` launches the helper with
/// `CreateProcess("\"%s\" \"%s\"")` (readpass.c:175,184) — one quoted argument,
/// no shell, so the helper must be a real `.exe`. The console path is unusable
/// under ConPTY: the UCRT's `_getch_nolock()` initialises its console handle
/// from `GetStdHandle(STD_INPUT_HANDLE)`, which ConPTY hands the child as the
/// input *pipe*, so `_getch()` returns EOF immediately and the password read
/// yields garbage (three instant `Permission denied`). `choice.exe`/`pause`
/// work because they open `CONIN$` directly.
///
/// When this process is launched with exactly one argument that is a password
/// prompt, print the stored password for `HOSTERM_ASKPASS_HOST` and exit.
/// Returns normally when this is not an askpass invocation.
pub fn maybe_run_askpass() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 1 || std::env::var_os("HOSTERM_ASKPASS_FILE").is_none() {
        return;
    }
    if !args[0].trim_end().ends_with("password:") {
        return;
    }
    let host = std::env::var("HOSTERM_ASKPASS_HOST").unwrap_or_default();
    if let Some(pw) = askpass_password(&host) {
        use std::io::Write as _;
        let mut out = std::io::stdout();
        // No trailing newline: read_passphrase() takes everything up to the
        // first CR/LF as the password, and a reader that does not strip it
        // would otherwise authenticate with the newline included.
        let _ = out.write_all(pw.as_bytes());
        let _ = out.flush();
    }
    std::process::exit(0);
}

/// Path of the `SSH_ASKPASS` helper (Windows only).
///
/// Prefer the `hosterm-askpass` sidecar next to this binary (a second cargo bin
/// target); fall back to this binary, which answers the askpass invocation
/// itself via [`maybe_run_askpass`]. Never a `.cmd`/`.bat` shim: it cannot be
/// launched the way `ssh_askpass()` launches helpers.
#[cfg(windows)]
fn askpass_exe() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("HOSTERM_ASKPASS_BIN") {
        if !p.is_empty() {
            return std::path::PathBuf::from(p);
        }
    }
    let exe = std::env::current_exe().unwrap_or_default();
    let sibling = exe.with_file_name("hosterm-askpass.exe");
    if sibling.exists() {
        return sibling;
    }
    exe
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
    // LQ-2 (INV-29): unlink any ephemeral Ansible inventory/cfg registered for
    // this run. Backend-side, not contingent on the frontend being alive.
    cleanup_ephemeral(&state.ephemeral, id);
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

// ============================================================
//  ANSIBLE v1 — pure cores (BE-ANS2; no command wiring here)
// ============================================================
//
// INV-28/29/31. These are side-effect-free functions (no I/O, no spawn),
// exercised in isolation by the cargo `--lib` suite. BE-ANS3 wires them into
// `ansible_run`. They never touch `~/.ssh/config` or the secrets sidecar, and
// a secret value NEVER materializes in their output (only IdentityFile PATHS).

/// Why one host was dropped from an Ansible run (mirrors fleet `R_*`). Opaque,
/// attacker-data-free strings — never embed a host/path value here.
const R_ANS_UNSAFE: &str = "unsafe alias";
const R_ANS_PASSWORD: &str = "password host (not supported in v1)";

/// Build the `ansible-playbook` argv as a direct `Vec<String>` (INV-28). Each
/// of `<targets>` (the `--limit` expression) and `<playbook>` is a SINGLE argv
/// element — never interpolated into a shell string, so no `sh -c`/`bash -c`
/// path exists. The caller feeds this straight to `CommandBuilder` (the binary
/// name `ansible-playbook` is prepended by the spawn site, BE-ANS3).
///
///   check:  -i <inv> --check --diff --limit <targets> <playbook>
///   real:   -i <inv>               --limit <targets> <playbook>
fn build_ansible_argv(check: bool, inv_path: &str, limit: &str, playbook: &str) -> Vec<String> {
    let mut argv: Vec<String> = vec!["-i".to_string(), inv_path.to_string()];
    if check {
        argv.push("--check".to_string());
        argv.push("--diff".to_string());
    }
    argv.push("--limit".to_string());
    argv.push(limit.to_string()); // ONE element — never shell-split
    argv.push(playbook.to_string()); // ONE element — never shell-split
    argv
}

/// Group header derived from the alias prefix before the first `-`, identical
/// to Fleet FR-FL2 (`main.ts` group derivation). `prod-web-01` → `prod`; an
/// alias with no `-` (or a leading `-`, already rejected upstream) has no
/// group. Returns `None` when there is no non-empty prefix.
fn ansible_group_of(alias: &str) -> Option<&str> {
    match alias.find('-') {
        Some(0) => None,                       // leading '-' (unsafe; filtered upstream anyway)
        Some(i) => Some(&alias[..i]),
        None => None,                          // no '-' ⇒ ungrouped
    }
}

/// One eligible host's inventory facts (BE-ANS2 input DTO). Absent config
/// fields are empty strings and get OMITTED from the emitted INI line. Carries
/// only IdentityFile PATHS — NEVER key bytes (INV-31). Password hosts never
/// reach here (filtered by `host_eligibility_ansible` first).
struct AnsibleInvHost {
    alias: String,
    host_name: String,
    user: String,
    port: String,
    identity_file: String,
}

/// Build the ephemeral inventory INI from the already-eligible hosts (INV-29,
/// SDD §14.3). Per host one line:
///
///   <alias> ansible_host=<HostName> ansible_user=<User> ansible_port=<Port> \
///           ansible_ssh_private_key_file=<IdentityFile>
///
/// Any field absent in config (empty string) is OMITTED from the line. Hosts
/// are grouped under `[<prefix>]` headers (prefix-before-first-`-`); ungrouped
/// hosts go in a leading header-less block. A password-auth host NEVER appears
/// (the caller filters with `host_eligibility_ansible`), so no secret byte can
/// ever be written — only IdentityFile paths (INV-31).
fn build_inventory_ini(eligible_hosts: &[AnsibleInvHost]) -> String {
    // One INI line of host vars; absent (empty) fields omitted.
    fn line(h: &AnsibleInvHost) -> String {
        let mut s = h.alias.clone();
        if !h.host_name.is_empty() {
            s.push_str(&format!(" ansible_host={}", h.host_name));
        }
        if !h.user.is_empty() {
            s.push_str(&format!(" ansible_user={}", h.user));
        }
        if !h.port.is_empty() {
            s.push_str(&format!(" ansible_port={}", h.port));
        }
        if !h.identity_file.is_empty() {
            // IdentityFile PATH only — never key bytes (INV-31).
            s.push_str(&format!(" ansible_ssh_private_key_file={}", h.identity_file));
        }
        s
    }

    let mut out = String::new();
    // Ungrouped hosts first (stable order = input order).
    for h in eligible_hosts.iter().filter(|h| ansible_group_of(&h.alias).is_none()) {
        out.push_str(&line(h));
        out.push('\n');
    }
    // Grouped hosts under their prefix header, first-seen group order.
    let mut groups: Vec<&str> = Vec::new();
    for h in eligible_hosts {
        if let Some(g) = ansible_group_of(&h.alias) {
            if !groups.contains(&g) {
                groups.push(g);
            }
        }
    }
    for g in groups {
        out.push_str(&format!("[{}]\n", g));
        for h in eligible_hosts
            .iter()
            .filter(|h| ansible_group_of(&h.alias) == Some(g))
        {
            out.push_str(&line(h));
            out.push('\n');
        }
    }
    out
}

/// One host's Ansible eligibility verdict (INV-31). Eligible hosts flow into
/// the inventory; skipped hosts carry an opaque reason and are NEVER spawned,
/// NEVER written to the inventory.
enum AnsibleEligibility {
    Eligible,
    Skipped(&'static str),
}

/// Mirror of the fleet host eligibility (INV-31, ADR-A2): v1 supports KEY/AGENT
/// auth hosts ONLY. A password-auth host OR an unsafe alias (leading `-`, CR,
/// LF — via `sftp_safe` — or a wildcard `*`/`?`) is SKIPPED with an explicit
/// reason; it never spawns and never reaches the inventory. `is_password` is
/// decided by the caller the same way the fleet path does (PreferredAuth /
/// PubkeyAuthentication / sidecar presence / legacy Password directive).
fn host_eligibility_ansible(alias: &str, is_password: bool) -> AnsibleEligibility {
    // Spawn-safety first (same ordering as fleet `host_eligibility`): leading
    // '-'/CR/LF via sftp_safe, then wildcards, then auth method.
    if crate::sftp_safe(alias, "Alias").is_err() {
        return AnsibleEligibility::Skipped(R_ANS_UNSAFE);
    }
    if alias.contains('*') || alias.contains('?') {
        return AnsibleEligibility::Skipped(R_ANS_UNSAFE);
    }
    if is_password {
        return AnsibleEligibility::Skipped(R_ANS_PASSWORD);
    }
    AnsibleEligibility::Eligible
}

// ============================================================
//  ANSIBLE v1 — command wiring (BE-ANS3: ansible_run + ansible_history)
// ============================================================

/// Spawn request (SDD §14.4). Args camelCase at the Tauri boundary; backend
/// REPLANS eligibility authoritatively from config via `host_eligibility_ansible`
/// — the picker's `aliases` is advisory, the backend decides who actually runs.
#[derive(Deserialize)]
struct AnsibleRunReq {
    mode: String, // "check" | "real"
    playbook: String,
    aliases: Vec<String>,
    limit: String,
    cols: u16,
    rows: u16,
}

/// One persisted history row (FR-ANS14/ANS15). `recap` is the trimmed PLAY
/// RECAP block ONLY — NEVER full output, NEVER a secret (INV-31).
#[derive(Serialize, Deserialize, Clone)]
struct AnsibleHistoryEntry {
    playbook: String,
    targets: String,
    mode: String,
    started_at: String,
    duration_ms: u64,
    rc: i64,
    recap: String,
}

/// History op (SDD §14.4): read the full array, or append one row (FIFO-cap).
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum AnsibleHistoryOp {
    Read,
    Append { entry: AnsibleHistoryEntry },
}

fn ansible_history_path() -> PathBuf {
    data_dir().join("ansible-history.json")
}

/// A non-guessable per-run suffix for the ephemeral inventory/cfg file names.
/// Reuses the process id + a monotonic PTY id + a nanosecond timestamp — NO new
/// RNG crate (TSD §3). Hex-encoded so the name is filename-safe.
fn ephemeral_suffix(id: u32) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}-{:x}-{:x}", std::process::id(), id, nanos)
}

/// Read the persisted history array. NEVER throws (INV-24/INV-30): a missing
/// file, an I/O error, or unparseable JSON all yield `[]`, and the corrupt
/// file is left on disk untouched until a legitimate write.
fn read_history() -> Vec<AnsibleHistoryEntry> {
    match std::fs::read_to_string(ansible_history_path()) {
        Ok(text) => serde_json::from_str::<Vec<AnsibleHistoryEntry>>(&text).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Append `entry` to `history` and FIFO-cap at 200 (oldest dropped first,
/// newest-last order preserved). Pure — the atomic write is the caller's.
fn history_append_fifo(
    mut history: Vec<AnsibleHistoryEntry>,
    entry: AnsibleHistoryEntry,
) -> Vec<AnsibleHistoryEntry> {
    const CAP: usize = 200;
    history.push(entry);
    let len = history.len();
    if len > CAP {
        history.drain(..len - CAP);
    }
    history
}

/// Ansible run history (FR-ANS14/ANS15). `read` → full array newest-last (`[]`
/// if missing/corrupt, INV-30). `append` → add + FIFO-cap 200 + atomic 0600
/// whole-file write (EXACT `settings.json` pattern via `write_store_atomic`;
/// refuses to clobber on a failed write). NEVER touches ~/.ssh/config or the
/// sidecar (INV-22/INV-29). `recap` carries only the trimmed PLAY RECAP block.
#[tauri::command]
fn ansible_history(op: AnsibleHistoryOp) -> Result<Vec<AnsibleHistoryEntry>, String> {
    match op {
        AnsibleHistoryOp::Read => Ok(read_history()),
        AnsibleHistoryOp::Append { entry } => {
            let next = history_append_fifo(read_history(), entry);
            let body = serde_json::to_string_pretty(&next)
                .map_err(|_| "ansible:history-write-failed".to_string())?;
            write_store_atomic(&ansible_history_path(), &body)
                .map_err(|_| "ansible:history-write-failed".to_string())?;
            Ok(next)
        }
    }
}

/// Spawn an Ansible run (SDD §14.4). Returns a PTY id into the EXISTING pty
/// session map; errors are opaque `ansible:<code>` tokens (never embed host/path
/// data). Behavior (FROZEN):
///   • replan via `host_eligibility_ansible` + sidecar-alias OR → eligible/skipped;
///     0 eligible ⇒ `ansible:no-eligible-hosts`, 0 spawn, 0 files written.
///   • generate ephemeral inventory + cfg (0600, data_dir()); argv as Vec<String>
///     via `build_ansible_argv` (INV-28); spawn `ansible-playbook` into the SAME
///     PtyState/NEXT_ID map, reusing `pump_pty`.
///   • env: ANSIBLE_HOST_KEY_CHECKING=True, ANSIBLE_CONFIG=<cfg>, ANSIBLE_NOCOLOR=1.
///   • NO password/secret enters argv/env/payload/inventory (ADR-A2/INV-31).
///   • ephemeral files registered against the pty id and unlinked on exit AND on
///     pty_kill (LQ-2/INV-29), backend-side.
#[tauri::command]
fn ansible_run(
    app: AppHandle,
    state: State<PtyState>,
    req: AnsibleRunReq,
) -> Result<u32, String> {
    let check = req.mode == "check";

    // Authoritative replan from config text ONLY (no self-heal write; INV-29),
    // mirroring the fleet plan layer: parse facts per alias, OR sidecar presence
    // into password detection. Password/unsafe hosts are SKIPPED — never spawned,
    // never written to the inventory.
    let text = read_config_text(&config_path()).map_err(|_| "ansible:config-unreadable".to_string())?;
    let cfg = parse_config(&text);
    let sidecars = fleet::sidecar_aliases();

    let mut eligible: Vec<AnsibleInvHost> = Vec::new();
    for alias in &req.aliases {
        // password detection mirrors read_ssh_config/snapshot_hosts + sidecar OR.
        let host = cfg.hosts.iter().find(|h| h.patterns.first().map(|p| p == alias).unwrap_or(false));
        let is_password = match host {
            Some(h) => {
                let prefers_password = get_opt(h, "PreferredAuthentications")
                    .split(',')
                    .next()
                    .map(|s| s.trim().eq_ignore_ascii_case("password"))
                    .unwrap_or(false);
                let pubkey_off = get_opt(h, "PubkeyAuthentication").eq_ignore_ascii_case("no");
                let legacy_pw = !get_opt(h, "Password").is_empty();
                prefers_password || pubkey_off || legacy_pw
            }
            None => false,
        } || sidecars.contains(alias);

        if let AnsibleEligibility::Eligible = host_eligibility_ansible(alias, is_password) {
            if let Some(h) = host {
                eligible.push(AnsibleInvHost {
                    alias: alias.clone(),
                    host_name: get_opt(h, "HostName"),
                    user: get_opt(h, "User"),
                    port: get_opt(h, "Port"),
                    identity_file: get_opt(h, "IdentityFile"),
                });
            }
        }
    }

    // 0 eligible ⇒ error BEFORE any file is written or process spawned.
    if eligible.is_empty() {
        return Err("ansible:no-eligible-hosts".into());
    }

    // Allocate the pty id first so the ephemeral file names + the session key
    // share one monotonic id.
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let suffix = ephemeral_suffix(id);
    let dir = data_dir();
    let inv_path = dir.join(format!("ansible-inv-{}.ini", suffix));
    let cfg_path = dir.join(format!("ansible-cfg-{}.ini", suffix));

    // Generate the ephemeral inventory + pinned-safe cfg (0600). The cfg pins
    // safe defaults so a stray cwd ansible.cfg is never honored (host key
    // checking is also enforced via env below).
    let inv_body = build_inventory_ini(&eligible);
    let cfg_body = "[defaults]\nhost_key_checking = True\nnocolor = True\n";
    write_store_atomic(&inv_path, &inv_body).map_err(|_| "ansible:inv-write-failed".to_string())?;
    if write_store_atomic(&cfg_path, cfg_body).is_err() {
        let _ = std::fs::remove_file(&inv_path); // don't leak the inventory on a half-failure
        return Err("ansible:cfg-write-failed".into());
    }

    // Register ephemeral paths against the pty id BEFORE spawn, so cleanup fires
    // even if spawn fails below (we also clean up on the early-return paths).
    state
        .ephemeral
        .lock()
        .unwrap()
        .insert(id, vec![inv_path.clone(), cfg_path.clone()]);

    let argv = build_ansible_argv(check, &inv_path.to_string_lossy(), &req.limit, &req.playbook);

    let pty_system = native_pty_system();
    let pair = match pty_system.openpty(PtySize {
        rows: req.rows,
        cols: req.cols,
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(p) => p,
        Err(_) => {
            cleanup_ephemeral(&state.ephemeral, id);
            return Err("ansible:pty-failed".into());
        }
    };

    let mut cmd = CommandBuilder::new("ansible-playbook");
    cmd.args(&argv);
    // ADR-A1 + LQ-3: host key checking ON (fail-closed), pin the ephemeral cfg so
    // a stray ansible.cfg is ignored, and disable color for a clean PLAY RECAP.
    // NO secret ever enters the environment (INV-31).
    cmd.env("ANSIBLE_HOST_KEY_CHECKING", "True");
    cmd.env("ANSIBLE_CONFIG", cfg_path.as_os_str());
    cmd.env("ANSIBLE_NOCOLOR", "1");

    let mut child = match pair.slave.spawn_command(cmd) {
        Ok(c) => c,
        Err(_) => {
            cleanup_ephemeral(&state.ephemeral, id);
            return Err("ansible:spawn-failed".into());
        }
    };
    drop(pair.slave);

    let reader = match pair.master.try_clone_reader() {
        Ok(r) => r,
        Err(_) => {
            cleanup_ephemeral(&state.ephemeral, id);
            return Err("ansible:pty-failed".into());
        }
    };
    let writer = match pair.master.take_writer() {
        Ok(w) => w,
        Err(_) => {
            cleanup_ephemeral(&state.ephemeral, id);
            return Err("ansible:pty-failed".into());
        }
    };
    let writer = Arc::new(Mutex::new(writer));

    // reader -> frontend events over the EXISTING pty://output|exit/{id} family.
    // On exit, unlink the ephemeral files (LQ-2/INV-29) — backend-side.
    let app_r = app.clone();
    let w_inject = writer.clone();
    // Look the managed PtyState up again inside the thread (its `ephemeral` map
    // outlives any single run — PtyState is `.manage()`d for the app lifetime),
    // so exit-path cleanup drops the registration AND unlinks the files, making
    // a later pty_kill an idempotent no-op (LQ-2/INV-29).
    let app_state = app.clone();
    thread::spawn(move || {
        pump_pty(reader, w_inject, None, |s| {
            let _ = app_r.emit(&format!("pty://output/{}", id), s.to_string());
        });
        {
            use tauri::Manager;
            let st = app_state.state::<PtyState>();
            cleanup_ephemeral(&st.ephemeral, id);
        }
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

#[cfg(test)]
mod utf8_stream_tests {
    use super::*;

    /// Reader that hands back at most `chunk` bytes per `read()`.
    ///
    /// This is the only way to force a multi-byte UTF-8 character to straddle
    /// a read boundary DETERMINISTICALLY, instead of hoping the kernel PTY
    /// timing splits one on its own. A real PTY returns up to 4096 bytes at
    /// arbitrary boundaries; chunk=1 reproduces the worst case.
    struct ChunkReader {
        data: Vec<u8>,
        pos: usize,
        chunk: usize,
    }

    impl std::io::Read for ChunkReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.pos >= self.data.len() {
                return Ok(0);
            }
            let n = self.chunk.min(buf.len()).min(self.data.len() - self.pos);
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    /// Writer that records everything written to it, so the password-injection
    /// path can be asserted without a real PTY.
    #[derive(Clone, Default)]
    struct SharedBuf(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn pump_bytes(data: &[u8], chunk: usize) -> String {
        let reader = ChunkReader { data: data.to_vec(), pos: 0, chunk };
        let writer: Arc<Mutex<Box<dyn Write + Send>>> =
            Arc::new(Mutex::new(Box::new(std::io::sink())));
        let mut out = String::new();
        pump_pty(Box::new(reader), writer, None, |s| out.push_str(s));
        out
    }

    /// Every character class the corruption report names: 3-byte box-drawing,
    /// a 4-byte emoji, a 3-byte CJK char, plus ASCII around them.
    const MIXED: &str = "┌─┐│└┘ plain 🚀 漢字 k9s\r\n";

    /// THE regression test: 1-byte reads split EVERY multi-byte character.
    /// Per-chunk `from_utf8_lossy` would emit U+FFFD for both halves of each
    /// one; the fixed path must round-trip byte-for-byte.
    #[test]
    fn pump_pty_reassembles_utf8_split_one_byte_at_a_time() {
        let out = pump_bytes(MIXED.as_bytes(), 1);
        assert_eq!(out, MIXED, "1-byte reads must round-trip byte-for-byte");
        assert!(!out.contains('\u{FFFD}'), "no U+FFFD may survive");
    }

    /// Every chunk size from 1 to 24 must round-trip identically — a fix that
    /// only happens to work for some boundary alignment is not a fix.
    #[test]
    fn pump_pty_round_trips_at_every_chunk_size() {
        for chunk in 1..=24 {
            let out = pump_bytes(MIXED.as_bytes(), chunk);
            assert_eq!(out, MIXED, "chunk size {chunk} corrupted the stream");
            assert!(!out.contains('\u{FFFD}'), "chunk size {chunk} produced U+FFFD");
        }
    }

    /// Large burst (bigger than the 4096 read buffer) with multi-byte chars at
    /// the buffer edge — the real-world k9s repaint shape.
    #[test]
    fn pump_pty_reassembles_large_unicode_burst() {
        let mut s = String::new();
        for _ in 0..400 {
            s.push_str(MIXED);
        }
        let out = pump_bytes(s.as_bytes(), 4096);
        assert_eq!(out, s, "large burst must round-trip byte-for-byte");
        assert!(!out.contains('\u{FFFD}'));
    }

    /// The helper holds an incomplete trailing sequence across calls instead of
    /// emitting a replacement char for it.
    #[test]
    fn decode_utf8_stream_holds_incomplete_tail() {
        let rocket = "🚀".as_bytes(); // 4 bytes: F0 9F 9A 80
        let mut pending = Vec::new();
        assert_eq!(decode_utf8_stream(&mut pending, &rocket[..1]), "");
        assert_eq!(pending, rocket[..1], "1-byte head must be carried");
        assert_eq!(decode_utf8_stream(&mut pending, &rocket[1..2]), "");
        assert_eq!(decode_utf8_stream(&mut pending, &rocket[2..3]), "");
        assert_eq!(decode_utf8_stream(&mut pending, &rocket[3..]), "🚀");
        assert!(pending.is_empty(), "pending must be drained after completion");
    }

    /// Genuinely invalid bytes still become U+FFFD (one per invalid byte), and
    /// valid text after them is preserved.
    #[test]
    fn decode_utf8_stream_invalid_bytes_are_replaced_not_wedged() {
        let mut pending = Vec::new();
        let out = decode_utf8_stream(&mut pending, b"a\xFF\xFEb");
        assert_eq!(out, "a\u{FFFD}\u{FFFD}b");
        assert!(pending.is_empty());
    }

    /// A character truncated by EOF is flushed lossily on the EOF path — the
    /// documented best-effort fallback, not a silent drop.
    #[test]
    fn pump_pty_eof_flushes_truncated_tail_lossily() {
        let rocket = "🚀".as_bytes();
        let mut data = b"ok".to_vec();
        data.extend_from_slice(&rocket[..3]); // 3 of 4 bytes, then EOF
        let out = pump_bytes(&data, 1);
        assert_eq!(out, "ok\u{FFFD}", "truncated tail must be flushed best-effort");
    }

    /// The fix must not disturb the OpenSSH password-injection contract: the
    /// prompt is detected once and the secret written exactly once.
    #[test]
    fn pump_pty_still_injects_password_once() {
        let data: &[u8] = b"user@host's password: ";
        let reader = ChunkReader { data: data.to_vec(), pos: 0, chunk: 1 };
        let sink = SharedBuf::default();
        let writer: Arc<Mutex<Box<dyn Write + Send>>> =
            Arc::new(Mutex::new(Box::new(sink.clone())));
        let mut out = String::new();
        pump_pty(Box::new(reader), writer, Some("hunter2".into()), |s| out.push_str(s));
        assert_eq!(out, "user@host's password: ");
        let injected = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        assert_eq!(injected, "hunter2\r", "password must be injected exactly once");
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
        // interactive prompt/echo loop hosterm drives (prompt without a
        // newline, command echoed by libedit, CRLF line endings)
        let dir = std::env::temp_dir().join(format!("hosterm-sftpbin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("sftp");
        std::fs::write(
            &bin,
            "#!/bin/bash\n\
             read -r -s -p \"fakebox@127.0.0.1's password: \" pw\n\
             echo\n\
             if [ \"$pw\" != 's3cret-sftp-pw' ]; then echo Permission denied; exit 5; fi\n\
             stty -echo 2>/dev/null || true\n\
             printf 'sftp> '\n\
             while IFS= read -r line; do\n\
               printf '%s\\r\\n' \"$line\"\n\
               case \"$line\" in\n\
                 'ls -la'*) echo '-rw-r--r--    1 u u    42 Jan  1 00:00 live.txt';;\n\
                 'pwd') echo '/home/fakebox';;\n\
               esac\n\
               printf 'sftp> '\n\
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

    /// Live check of the promptless (Windows) path: a fake sftp that never
    /// prints `sftp> ` — only the `sftp> <cmd>` echo Win32-OpenSSH prints in
    /// non-interactive mode. Login must complete on the `pwd` probe echo and
    /// the listing must come back clean. Gated behind HOSTERM_LIVE_PTY=1.
    #[test]
    fn live_sftp_promptless_windows_mode_login_and_exec() {
        if std::env::var("HOSTERM_LIVE_PTY").as_deref() != Ok("1") {
            return;
        }
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("sftpwin");
        std::fs::write(
            &cfg,
            "Host fakeboxwin\n    HostName 127.0.0.1\n    PreferredAuthentications password\n",
        )
        .unwrap();
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        let secrets = cfg.parent().unwrap().join("hosterm.secrets");
        std::fs::write(&secrets, "fakeboxwin=s3cret-sftp-pw\n").unwrap();

        let dir = std::env::temp_dir().join(format!("hosterm-sftpwin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("sftp");
        std::fs::write(
            &bin,
            "#!/bin/bash\n\
             read -r -s -p \"fakeboxwin@127.0.0.1's password: \" pw\n\
             echo\n\
             if [ \"$pw\" != 's3cret-sftp-pw' ]; then echo \"Permission denied, please try again.\"; exit 5; fi\n\
             stty -echo 2>/dev/null || true\n\
             while IFS= read -r line; do\n\
               printf 'sftp> %s\\n' \"$line\"\n\
               case \"$line\" in\n\
                 'ls -la'*) echo '-rw-r--r--    1 u u    42 Jan  1 00:00 live.txt';;\n\
                 'pwd') echo '/home/fakeboxwin';;\n\
               esac\n\
             done\n"
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::env::set_var("HOSTERM_SFTP_BIN", &bin);

        let out1 = run_sftp_op("fakeboxwin", "ls -la\n");
        assert!(out1.ok, "op1 failed: {}", out1.stderr);
        assert!(out1.stdout.contains("live.txt"), "listing lost: {:?}", out1.stdout);
        assert!(!out1.stdout.contains("sftp>"), "echo leaked: {:?}", out1.stdout);
        assert!(!out1.stdout.contains("password:"), "login leaked: {:?}", out1.stdout);

        // op 2: reuses the SAME process — a second login would re-print the
        // password prompt into this round's output
        let out2 = run_sftp_op("fakeboxwin", "ls -la\n");
        assert!(out2.ok, "op2 failed: {}", out2.stderr);
        assert!(out2.stdout.contains("live.txt"));
        assert!(!out2.stdout.contains("password:"), "op2 logged in again: {:?}", out2.stdout);

        std::env::remove_var("HOSTERM_SFTP_BIN");
        std::env::remove_var("HOSTERM_CONFIG");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(cfg.parent().unwrap());
    }

    /// Live check of the ConPTY console-echo case: ConPTY's default input mode
    /// has ENABLE_ECHO_INPUT on, so the PTY echoes typed commands too. The
    /// buffer then carries BOTH the bare console echo and sftp's `sftp> <cmd>`
    /// echo. The extraction must anchor on the prefixed form and leak neither.
    /// Gated behind HOSTERM_LIVE_PTY=1.
    #[test]
    fn live_sftp_console_echo_login_and_exec() {
        if std::env::var("HOSTERM_LIVE_PTY").as_deref() != Ok("1") {
            return;
        }
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("sftpecho");
        std::fs::write(
            &cfg,
            "Host fakeboxecho\n    HostName 127.0.0.1\n    PreferredAuthentications password\n",
        )
        .unwrap();
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        let secrets = cfg.parent().unwrap().join("hosterm.secrets");
        std::fs::write(&secrets, "fakeboxecho=s3cret-sftp-pw\n").unwrap();

        let dir = std::env::temp_dir().join(format!("hosterm-sftpecho-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("sftp");
        // No `stty -echo`: the PTY echoes typed commands, like ConPTY.
        std::fs::write(
            &bin,
            "#!/bin/bash\n\
             read -r -s -p \"fakeboxecho@127.0.0.1's password: \" pw\n\
             echo\n\
             if [ \"$pw\" != 's3cret-sftp-pw' ]; then echo \"Permission denied\"; exit 5; fi\n\
             while IFS= read -r line; do\n\
               printf 'sftp> %s\\r\\n' \"$line\"\n\
               case \"$line\" in\n\
                 'ls -la'*) echo '-rw-r--r--    1 u u    42 Jan  1 00:00 live.txt';;\n\
                 'pwd') echo '/home/fakeboxecho';;\n\
               esac\n\
             done\n"
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::env::set_var("HOSTERM_SFTP_BIN", &bin);

        let out1 = run_sftp_op("fakeboxecho", "ls -la\n");
        assert!(out1.ok, "op1 failed: {}", out1.stderr);
        assert!(out1.stdout.contains("live.txt"), "listing lost: {:?}", out1.stdout);
        assert!(!out1.stdout.contains("sftp>"), "echo leaked: {:?}", out1.stdout);
        assert!(!out1.stdout.contains("password:"), "login leaked: {:?}", out1.stdout);

        let out2 = run_sftp_op("fakeboxecho", "ls -la\n");
        assert!(out2.ok, "op2 failed: {}", out2.stderr);
        assert!(out2.stdout.contains("live.txt"));
        assert!(!out2.stdout.contains("password:"), "op2 logged in again: {:?}", out2.stdout);

        std::env::remove_var("HOSTERM_SFTP_BIN");
        std::env::remove_var("HOSTERM_CONFIG");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(cfg.parent().unwrap());
    }

    #[test]
    fn probe_echo_extraction_handles_windows_and_linux_forms() {
        // Windows non-interactive: every command echoed as `sftp> <cmd>\n`
        let win = "sftp> ls -la\ntotal 0\n-rw-r--r-- 1 u u 42 Jan 1 00:00 live.txt\nsftp> pwd\n/home/w";
        assert_eq!(
            strip_probe_echo(win, "ls -la"),
            "total 0\n-rw-r--r-- 1 u u 42 Jan 1 00:00 live.txt"
        );
        // Linux interactive: prompt + libedit echo, CRLF endings; the command
        // echo carries no `sftp> ` prefix because the prompt was consumed at login
        let lin = "ls -la\r\ntotal 0\r\n-rw-r--r-- 1 u u 42 Jan 1 00:00 live.txt\r\nsftp> pwd\r\n/home/l";
        assert_eq!(
            strip_probe_echo(lin, "ls -la"),
            "total 0\r\n-rw-r--r-- 1 u u 42 Jan 1 00:00 live.txt"
        );
        // empty output
        assert_eq!(strip_probe_echo("sftp> ls -la\nsftp> pwd", "ls -la"), "");
        // quoted argument survives the echo strip
        let q = "sftp> ls -la \"/tmp/a b\"\n-rw-r--r-- 1 u u 1 Jan 1 00:00 x\nsftp> pwd";
        assert_eq!(strip_probe_echo(q, "ls -la \"/tmp/a b\""), "-rw-r--r-- 1 u u 1 Jan 1 00:00 x");
        // the sentinel itself as the command
        assert_eq!(strip_probe_echo("sftp> pwd\r\n/home/x\r\nsftp> pwd", "pwd"), "/home/x");
        // Windows + ConPTY console echo (ENABLE_ECHO_INPUT): the bare line is
        // echoed too. Ordering A — console echoes as the child reads, so the
        // sentinel's bare echo lands just before its prefixed echo.
        let win_echo_a = "ls -la\r\nsftp> ls -la\r\ntotal 0\r\n-rw-r--r-- 1 u u 42 Jan 1 00:00 live.txt\r\npwd\r\nsftp> pwd\r\n/home/w";
        assert_eq!(
            strip_probe_echo(win_echo_a, "ls -la"),
            "total 0\r\n-rw-r--r-- 1 u u 42 Jan 1 00:00 live.txt"
        );
        // Ordering B — the console echoes both lines up front.
        let win_echo_b = "ls -la\r\npwd\r\nsftp> ls -la\r\ntotal 0\r\n-rw-r--r-- 1 u u 42 Jan 1 00:00 live.txt\r\nsftp> pwd\r\n/home/w";
        assert_eq!(
            strip_probe_echo(win_echo_b, "ls -la"),
            "total 0\r\n-rw-r--r-- 1 u u 42 Jan 1 00:00 live.txt"
        );
        // Same, with the sentinel as the command.
        let win_echo_cmd = "pwd\r\nsftp> pwd\r\n/home/w\r\npwd\r\nsftp> pwd\r\n/home/w";
        assert_eq!(strip_probe_echo(win_echo_cmd, "pwd"), "/home/w");
    }

    #[test]
    fn raw_note_flattens_control_chars_and_marks_output() {
        // CR from the tty becomes a space; newlines/tabs survive
        assert_eq!(raw_note("a\rb\nc\td"), " — sftp output: a b\nc\td");
        // empty / whitespace-only -> no note at all
        assert_eq!(raw_note(""), "");
        assert_eq!(raw_note("   "), "");
        // hostile escape sequences are flattened, never interpreted
        assert_eq!(raw_note("bad\x1b[31mred"), " — sftp output: bad [31mred");
        // the note is prefixed so the cause reads as sftp output
        assert!(raw_note("denied").starts_with(" — sftp output: "));
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
    fn write_identity_file_overwrites_managed_0600_atomic() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("wkey");
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        create_identity_file("id_w".into(), "-----BEGIN OLD-----\nold\n".into()).unwrap();
        let on_disk = cfg.parent().unwrap().join("keys").join("id_w");
        // Overwrite with new contents; CRLF normalized, trailing newline added.
        let path = write_identity_file("id_w".into(), "-----BEGIN NEW-----\r\nnew".into()).unwrap();
        assert_eq!(path, "~/.ssh/keys/id_w");
        let content = std::fs::read_to_string(&on_disk).unwrap();
        assert_eq!(content, "-----BEGIN NEW-----\nnew\n", "LF-normalized, trailing newline, replaced");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&on_disk).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        // No temp residue left behind after the atomic rename.
        let residue: Vec<_> = std::fs::read_dir(cfg.parent().unwrap().join("keys"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("hosterm-tmp"))
            .collect();
        assert!(residue.is_empty(), "no temp file residue");
        std::env::remove_var("HOSTERM_CONFIG");
    }

    #[test]
    fn write_identity_file_rejects_missing_empty_and_traversal() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("wkeybad");
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        // Missing key: this is an edit, not a create.
        assert!(write_identity_file("no_such".into(), "PRIVATE KEY\n".into()).is_err());
        // Empty contents rejected.
        create_identity_file("id_e".into(), "PRIVATE KEY\n".into()).unwrap();
        assert!(write_identity_file("id_e".into(), "   ".into()).is_err());
        // Traversal / separators / empty name rejected before any write.
        assert!(write_identity_file("../evil".into(), "x\n".into()).is_err());
        assert!(write_identity_file("a/b".into(), "x\n".into()).is_err());
        assert!(write_identity_file("".into(), "x\n".into()).is_err());
        std::env::remove_var("HOSTERM_CONFIG");
    }

    #[test]
    fn write_identity_file_cannot_touch_legacy_ssh_key() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("wkeyleg");
        let dir = cfg.parent().unwrap();
        // A legacy key lives directly in ~/.ssh, NOT in ~/.ssh/keys.
        std::fs::write(dir.join("id_legacy"), "-----BEGIN LEGACY-----\norig\n").unwrap();
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        // write_identity_file only ever resolves inside keys_dir(); a legacy name
        // there does not exist, so the edit is refused and the legacy file on disk
        // is left untouched.
        assert!(write_identity_file("id_legacy".into(), "-----BEGIN PWN-----\nx\n".into()).is_err());
        let legacy = std::fs::read_to_string(dir.join("id_legacy")).unwrap();
        assert_eq!(legacy, "-----BEGIN LEGACY-----\norig\n", "legacy key unchanged");
        std::env::remove_var("HOSTERM_CONFIG");
    }

    #[test]
    fn rename_identity_file_moves_key_and_pub_and_guards() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("rnkey");
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        let keys = cfg.parent().unwrap().join("keys");
        create_identity_file("id_old".into(), "-----BEGIN OPENSSH PRIVATE KEY-----\n".into()).unwrap();
        std::fs::write(keys.join("id_old.pub"), "ssh-ed25519 AAAA\n").unwrap();
        // Rename moves both halves.
        let path = rename_identity_file("id_old".into(), "id_new".into()).unwrap();
        assert_eq!(path, "~/.ssh/keys/id_new");
        assert!(!keys.join("id_old").exists());
        assert!(!keys.join("id_old.pub").exists());
        assert!(keys.join("id_new").exists());
        assert!(keys.join("id_new.pub").exists());
        // Guards: missing source, traversal, clobber, bad names.
        assert!(rename_identity_file("gone".into(), "whatever".into()).is_err());
        assert!(rename_identity_file("id_new".into(), "../evil".into()).is_err());
        assert!(rename_identity_file("../evil".into(), "id_z".into()).is_err());
        // Clobber refused: create a second key, renaming onto it fails.
        create_identity_file("id_other".into(), "-----BEGIN B-----\n".into()).unwrap();
        assert!(rename_identity_file("id_new".into(), "id_other".into()).is_err());
        assert!(keys.join("id_new").exists(), "source intact after refused clobber");
        std::env::remove_var("HOSTERM_CONFIG");
    }

    #[test]
    fn rename_identity_file_cannot_target_or_source_legacy() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("rnkeyleg");
        let dir = cfg.parent().unwrap();
        std::fs::write(dir.join("id_legacy"), "-----BEGIN LEGACY-----\norig\n").unwrap();
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        // rename only resolves names inside keys_dir(); a legacy source name does
        // not exist there → refused, legacy file untouched.
        assert!(rename_identity_file("id_legacy".into(), "id_new".into()).is_err());
        let legacy = std::fs::read_to_string(dir.join("id_legacy")).unwrap();
        assert_eq!(legacy, "-----BEGIN LEGACY-----\norig\n", "legacy key unchanged");
        std::env::remove_var("HOSTERM_CONFIG");
    }

    #[test]
    fn read_identity_file_roundtrips_managed_and_guards() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("rdkey");
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        // Managed key reads back verbatim (create normalizes to LF + trailing \n).
        let body = "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n";
        create_identity_file("id_r".into(), body.into()).unwrap();
        assert_eq!(read_identity_file("id_r".into()).unwrap(), body);
        // Missing key: clear error, no panic.
        assert!(read_identity_file("no_such".into()).is_err());
        // Traversal / separators / absolute / empty name rejected before any read.
        assert!(read_identity_file("../evil".into()).is_err());
        assert!(read_identity_file("a/b".into()).is_err());
        assert!(read_identity_file("/etc/passwd".into()).is_err());
        assert!(read_identity_file("".into()).is_err());
        std::env::remove_var("HOSTERM_CONFIG");
    }

    #[test]
    fn read_identity_file_cannot_read_legacy_ssh_key() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("rdkeyleg");
        let dir = cfg.parent().unwrap();
        // A legacy key lives directly in ~/.ssh, NOT in ~/.ssh/keys.
        std::fs::write(dir.join("id_legacy"), "-----BEGIN LEGACY-----\norig\n").unwrap();
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        // read_identity_file only resolves names inside keys_dir(); the legacy name
        // does not exist there → refused, and its contents never cross IPC.
        assert!(read_identity_file("id_legacy".into()).is_err());
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

    // =========================================================
    //  BE-ANS3: ansible_history (atomic/FIFO/corrupt) + cleanup
    // =========================================================

    fn hist(playbook: &str) -> AnsibleHistoryEntry {
        AnsibleHistoryEntry {
            playbook: playbook.into(),
            targets: "web".into(),
            mode: "check".into(),
            started_at: "2026-11-05T00:00:00Z".into(),
            duration_ms: 1234,
            rc: 0,
            recap: "PLAY RECAP\nweb : ok=1 changed=0 unreachable=0 failed=0".into(),
        }
    }

    // ---- history_append_fifo: 200-cap FIFO (oldest dropped, newest-last) ----
    #[test]
    fn history_append_fifo_caps_at_200_dropping_oldest() {
        // Seed 200 entries tagged 0..200, then append one more → len stays 200,
        // the oldest (playbook "0") is gone, the newest ("200") is last.
        let mut v: Vec<AnsibleHistoryEntry> = (0..200).map(|i| hist(&i.to_string())).collect();
        assert_eq!(v.len(), 200);
        v = history_append_fifo(v, hist("200"));
        assert_eq!(v.len(), 200, "FIFO cap holds at 200");
        assert_eq!(v.first().unwrap().playbook, "1", "oldest entry dropped first");
        assert_eq!(v.last().unwrap().playbook, "200", "newest entry is last");
        // under the cap it simply appends
        let small = history_append_fifo(vec![hist("a")], hist("b"));
        assert_eq!(small.len(), 2);
        assert_eq!(small.last().unwrap().playbook, "b");
    }

    // ---- ansible_history: read missing → [], append atomic 0600, read back ----
    #[test]
    fn ansible_history_read_missing_then_append_atomic() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = data_root("ans-hist");
        std::env::set_var("HOSTERM_DATA_DIR", &root);

        // missing file → [], read creates nothing (INV-30/INV-24)
        let empty = ansible_history(AnsibleHistoryOp::Read).unwrap();
        assert!(empty.is_empty(), "missing history reads as []");
        let path = root.join("hosterm").join("ansible-history.json");
        assert!(!path.exists(), "a read must not create the file");

        // append → array after append; file lands in app-data dir
        let after = ansible_history(AnsibleHistoryOp::Append { entry: hist("site.yml") }).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].playbook, "site.yml");
        assert!(path.exists(), "append writes the store file");

        // 0600 on unix (atomic write discipline)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "history file is 0600");
        }

        // read back reflects the append (newest-last)
        let back = ansible_history(AnsibleHistoryOp::Read).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].playbook, "site.yml");

        std::env::remove_var("HOSTERM_DATA_DIR");
        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- INV-30: corrupt history → [] AND bytes untouched (no clobber on read) ----
    #[test]
    fn ansible_history_corrupt_reads_empty_without_clobber() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = data_root("ans-hist-corrupt");
        std::env::set_var("HOSTERM_DATA_DIR", &root);
        let path = root.join("hosterm").join("ansible-history.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let garbage = "{ not valid json ]]] \u{0} garbage";
        std::fs::write(&path, garbage).unwrap();
        let before = std::fs::read(&path).unwrap();

        // read → [] and the corrupt bytes are left on disk untouched
        let got = ansible_history(AnsibleHistoryOp::Read).unwrap();
        assert!(got.is_empty(), "corrupt history reads as []");
        let after = std::fs::read(&path).unwrap();
        assert_eq!(before, after, "corrupt bytes untouched until a legitimate write");

        std::env::remove_var("HOSTERM_DATA_DIR");
        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- LQ-2/INV-29: cleanup_ephemeral unlinks registered files, idempotent ----
    #[test]
    fn cleanup_ephemeral_unlinks_and_is_idempotent() {
        let root = data_root("ans-ephem");
        let inv = root.join("ansible-inv-test.ini");
        let cfg = root.join("ansible-cfg-test.ini");
        std::fs::write(&inv, "web ansible_host=10.0.0.1\n").unwrap();
        std::fs::write(&cfg, "[defaults]\n").unwrap();
        assert!(inv.exists() && cfg.exists());

        let map: Mutex<HashMap<u32, Vec<PathBuf>>> = Mutex::new(HashMap::new());
        map.lock().unwrap().insert(7, vec![inv.clone(), cfg.clone()]);

        cleanup_ephemeral(&map, 7);
        assert!(!inv.exists(), "inventory unlinked on cleanup");
        assert!(!cfg.exists(), "cfg unlinked on cleanup");
        assert!(map.lock().unwrap().get(&7).is_none(), "registration dropped");

        // second call (exit after kill, or vice versa) is a no-op, no panic
        cleanup_ephemeral(&map, 7);
        assert!(map.lock().unwrap().get(&7).is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- 0-eligible replan: all password/unsafe aliases → 0 inventory hosts ----
    // Proves the condition behind `ansible:no-eligible-hosts` (0 spawn, 0 files):
    // the authoritative replan drops every ineligible alias, so `eligible` is
    // empty and ansible_run returns the error BEFORE writing any file. The
    // command needs an AppHandle/State to spawn, so we exercise the replan
    // composition (the branch's predicate) directly here.
    #[test]
    fn replan_drops_password_and_unsafe_hosts_to_zero() {
        let aliases: Vec<String> = vec!["pw".into(), "-x".into(), "star*".into()];
        // pw = password host; -x/star* = unsafe/wildcard → all skipped.
        let is_password = |a: &str| a == "pw";
        let mut eligible: Vec<&str> = Vec::new();
        for a in &aliases {
            if let AnsibleEligibility::Eligible = host_eligibility_ansible(a, is_password(a)) {
                eligible.push(a);
            }
        }
        assert!(eligible.is_empty(), "no key/agent host ⇒ 0 eligible ⇒ ansible:no-eligible-hosts");
    }

    // ---- INV-31: the inventory built for an Ansible run carries NO secret ----
    // (only IdentityFile PATHS). Composition proof over build_inventory_ini,
    // the exact body ansible_run writes to the ephemeral inventory.
    #[test]
    fn ansible_inventory_body_never_contains_a_secret() {
        let hosts = vec![AnsibleInvHost {
            alias: "prod-web".into(),
            host_name: "203.0.113.10".into(),
            user: "root".into(),
            port: "2222".into(),
            identity_file: "~/.ssh/id_ed25519".into(),
        }];
        let ini = build_inventory_ini(&hosts);
        assert!(ini.contains("ansible_ssh_private_key_file=~/.ssh/id_ed25519"), "IdentityFile path present");
        assert!(!ini.to_lowercase().contains("password"), "no password token in inventory");
        assert!(!ini.contains("sekret"), "no secret bytes in inventory");
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

    /// Probe (and, if needed, repair) the CI fixture's `hosterm` account.
    ///
    /// sshd's Windows password check is `LogonUserW(user, L".", pw,
    /// LOGON32_LOGON_INTERACTIVE)` and then `LOGON32_LOGON_NETWORK`
    /// (Win32-OpenSSH `auth-passwd.c:343-361`). The fixture's
    /// `ValidateCredentials` accepts the password, but that path does not
    /// exercise the logon rights, so it cannot tell us whether sshd will.
    /// Probe both logon types directly, then clear the `UF_PASSWD_NOTREQD`
    /// flag (`PasswordRequired=False` was observed on the runner image) and
    /// give the account the Administrators group, which holds both logon
    /// rights by default. Both repairs are idempotent.
    #[cfg(windows)]
    fn ensure_windows_live_account() {
        if std::env::var("HOSTERM_WINLIVE").as_deref() != Ok("1") {
            return;
        }
        let Ok(password) = std::env::var("HOSTERM_WINLIVE_PASSWORD") else {
            return;
        };
        let user = std::env::var("HOSTERM_WINLIVE_USER").unwrap_or_else(|_| "hosterm".into());
        let dir = std::env::temp_dir().join(format!("hosterm-acct-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let ps = dir.join("logon-probe.ps1");
        // Written to a file rather than `-Command` so the C# here-string and the
        // quoting survive intact.
        std::fs::write(
            &ps,
            r#"param([string]$User, [string]$Password)
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class LogonProbe {
    [DllImport("advapi32.dll", SetLastError = true)]
    public static extern bool LogonUser(string user, string domain, string password, int logonType, int logonProvider, out IntPtr token);
}
'@
$t = [IntPtr]::Zero
$okI = [LogonProbe]::LogonUser($User, '.', $Password, 2, 0, [ref]$t)
$eI = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
$okN = [LogonProbe]::LogonUser($User, '.', $Password, 3, 0, [ref]$t)
$eN = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
"interactive=$okI err=$eI network=$okN err=$eN"
"#,
        )
        .ok();
        let probe = || -> String {
            match std::process::Command::new("powershell")
                .arg("-NoProfile")
                .arg("-ExecutionPolicy")
                .arg("Bypass")
                .arg("-File")
                .arg(&ps)
                .arg("-User")
                .arg(&user)
                .arg("-Password")
                .arg(&password)
                .output()
            {
                Ok(o) => format!(
                    "out={:?} err={:?}",
                    String::from_utf8_lossy(&o.stdout).trim(),
                    String::from_utf8_lossy(&o.stderr).trim()
                ),
                Err(e) => format!("probe failed to run: {e}"),
            }
        };
        let before = probe();
        eprintln!("[winlive] logon probe (before): {}", before);
        // Clear UF_PASSWD_NOTREQD, then make sure both logon rights are held.
        let _ = std::process::Command::new("net")
            .arg("user")
            .arg(&user)
            .arg("/passwordreq:yes")
            .output();
        let _ = std::process::Command::new("net")
            .arg("localgroup")
            .arg("Administrators")
            .arg(&user)
            .arg("/add")
            .output();
        let after = probe();
        eprintln!("[winlive] logon probe (after): {}", after);
        // Leave the evidence where the CI log can reach it even when the test
        // passes (cargo captures a passing test's stderr).
        let _ = std::fs::write(
            std::env::temp_dir().join("hosterm-logon-probe.txt"),
            format!("before: {before}\nafter: {after}\n"),
        );
        assert!(
            after.contains("interactive=True") || after.contains("network=True"),
            "CI fixture account still cannot log on: {}",
            after
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Windows-only proof of the fix: the REAL `sftp.exe` (Win32-OpenSSH) over
    /// a ConPTY, password host, listing loads. `sftp.exe` never prints the
    /// `sftp> ` prompt on Windows (sftp.c compiles the three
    /// allocate_standard_descriptor calls out, so isatty(stdin) is false and
    /// the prompt branch is dead), and ConPTY's default input mode has
    /// ENABLE_ECHO_INPUT on, so the console echoes typed commands too. The
    /// login/exec path must key off the command echo and write CR, not LF.
    /// Gated behind HOSTERM_WINLIVE=1: the CI job starts a local sshd, creates
    /// the `hosterm` user, and writes the `hostermlive` alias to
    /// %USERPROFILE%\.ssh\config (which the sftp.exe child reads; hosterm's own
    /// parser is pointed at a throwaway config via HOSTERM_CONFIG so the
    /// secrets sidecar stays out of the real ~/.ssh).
    #[cfg(windows)]
    #[test]
    fn windows_sftp_password_host_listing_loads() {
        if std::env::var("HOSTERM_WINLIVE").as_deref() != Ok("1") {
            return;
        }
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        ensure_windows_live_account();
        let user = std::env::var("HOSTERM_WINLIVE_USER").unwrap_or_else(|_| "hosterm".into());
        // Never hardcode a password in the tree: the workflow supplies it.
        let Ok(password) = std::env::var("HOSTERM_WINLIVE_PASSWORD") else {
            eprintln!("[winlive] HOSTERM_WINLIVE_PASSWORD not set; skipping");
            return;
        };
        let dir = std::env::temp_dir().join(format!("hosterm-winlive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("config");
        std::fs::write(
            &cfg,
            format!(
                "Host hostermlive\n    HostName 127.0.0.1\n    User {}\n    PreferredAuthentications password\n",
                user
            ),
        )
        .unwrap();
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        let secrets = dir.join("hosterm.secrets");
        std::fs::write(&secrets, format!("hostermlive={}\n", password)).unwrap();

        // `cargo test --lib` builds no bin targets, so compile the askpass helper
        // (std-only) with rustc. It must be a real .exe: ssh_askpass() launches
        // SSH_ASKPASS through CreateProcess with a single quoted argument and no
        // shell, so a .cmd shim can never run.
        let helper = dir.join("hosterm-askpass.exe");
        let helper_src =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/hosterm-askpass.rs");
        let built = std::process::Command::new("rustc")
            .arg("--edition")
            .arg("2021")
            .arg("-O")
            .arg(&helper_src)
            .arg("-o")
            .arg(&helper)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        eprintln!("[winlive] askpass helper built: {}", built);
        std::env::set_var("HOSTERM_ASKPASS_BIN", &helper);
        eprintln!("[winlive] askpass helper resolved: {}", askpass_exe().display());
        // Which OpenSSH build is on PATH? Win32-OpenSSH's read_passphrase keys
        // off SSH_ASKPASS alone; openssh-portable gates askpass behind
        // DISPLAY/WAYLAND_DISPLAY unless SSH_ASKPASS_REQUIRE=force.
        if let Ok(v) = std::process::Command::new("ssh").arg("-V").output() {
            eprintln!(
                "[winlive] ssh -V: {}",
                String::from_utf8_lossy(&v.stderr).trim()
            );
        }
        // The single most decisive fact: did sftp.exe/ssh.exe actually launch
        // the askpass helper? If this stays empty while the console prompt
        // appears, the helper was never consulted.
        let askpass_log = dir.join("askpass.log");
        std::env::set_var("HOSTERM_ASKPASS_LOG", &askpass_log);

        let out = run_sftp_op("hostermlive", "ls -la\n");

        eprintln!(
            "[winlive] stored password present: {} (len {})",
            lookup_stored_password("hostermlive").is_some(),
            password.len()
        );
        std::env::remove_var("HOSTERM_ASKPASS_BIN");
        std::env::remove_var("HOSTERM_CONFIG");
        let invoked = std::fs::read_to_string(&askpass_log).unwrap_or_default();
        eprintln!(
            "[winlive] askpass invoked: {} ({:?})",
            !invoked.trim().is_empty(),
            invoked.trim()
        );
        let _ = std::fs::remove_dir_all(&dir);

        assert!(built, "askpass helper failed to compile");
        assert!(out.ok, "Windows sftp listing failed: {}", out.stderr);
        assert!(!out.stdout.trim().is_empty(), "empty listing");
        assert!(!out.stdout.contains("sftp>"), "echo leaked: {:?}", out.stdout);
        assert!(
            !out.stdout.contains("password:"),
            "login leaked into the listing: {:?}",
            out.stdout
        );
    }

    /// Probe the Windows password path with `ssh.exe` (unlike sftp.exe it has a
    /// working console read under ConPTY). Three phases isolate the delivery
    /// from the account itself:
    ///   A: SSH_ASKPASS + the real password,
    ///   B: console typing + the real password,
    ///   C: console typing + a blank password.
    /// B failing means the fixture (account/sshd) is at fault, not the delivery;
    /// B working means the askpass delivery is what is broken.
    #[cfg(windows)]
    #[test]
    fn windows_ssh_askpass_authenticates() {
        if std::env::var("HOSTERM_WINLIVE").as_deref() != Ok("1") {
            return;
        }
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        ensure_windows_live_account();
        let user = std::env::var("HOSTERM_WINLIVE_USER").unwrap_or_else(|_| "hosterm".into());
        let Ok(password) = std::env::var("HOSTERM_WINLIVE_PASSWORD") else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("hosterm-sshask-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("config");
        std::fs::write(
            &cfg,
            format!(
                "Host hostermlive\n    HostName 127.0.0.1\n    User {}\n    PreferredAuthentications password\n",
                user
            ),
        )
        .unwrap();
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        let secrets = dir.join("hosterm.secrets");
        std::fs::write(&secrets, format!("hostermlive={password}\n")).unwrap();
        let helper = dir.join("hosterm-askpass.exe");
        let helper_src =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/hosterm-askpass.rs");
        let _ = std::process::Command::new("rustc")
            .arg("--edition")
            .arg("2021")
            .arg("-O")
            .arg(&helper_src)
            .arg("-o")
            .arg(&helper)
            .status();
        let log = dir.join("askpass.log");
        std::env::set_var("HOSTERM_ASKPASS_LOG", &log);

        // sshd's effective config, for the record: a disabled PasswordAuthentication
        // or a forced keyboard-interactive would explain a correct password being
        // rejected.
        for sshd in ["sshd", r"C:\Windows\System32\OpenSSH\sshd.exe"] {
            if let Ok(out) = std::process::Command::new(sshd).arg("-T").output() {
                let s = String::from_utf8_lossy(&out.stdout);
                let keep: Vec<&str> = s
                    .lines()
                    .filter(|l| {
                        l.contains("passwordauthentication")
                            || l.contains("kbdinteractive")
                            || l.contains("usepam")
                            || l.contains("logon")
                    })
                    .collect();
                eprintln!("[winlive] sshd -T: {:?}", keep);
                break;
            }
        }
        // Is the account really password-gated? The workflow reported
        // `PasswordRequired=False`; a broken account state would make every
        // password look acceptable.
        let ps = format!(
            "Add-Type -AssemblyName System.DirectoryServices.AccountManagement; $pc = New-Object System.DirectoryServices.AccountManagement.PrincipalContext([System.DirectoryServices.AccountManagement.ContextType]::Machine); $u = Get-LocalUser -Name '{user}'; 'enabled={{0}} lockedout={{1}} pwrequired={{2}} right={{3}} wrong={{4}}' -f $u.Enabled, $u.LockedOut, $u.PasswordRequired, $pc.ValidateCredentials('{user}','{password}'), $pc.ValidateCredentials('{user}','definitely-not-it')"
        );
        if let Ok(out) = std::process::Command::new("powershell")
            .arg("-NoProfile")
            .arg("-Command")
            .arg(ps)
            .output()
        {
            eprintln!(
                "[winlive] account check: {:?} err={:?}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }

        // One `ssh.exe hostermlive echo ACCESS_GRANTED` run under a ConPTY.
        // `askpass` toggles the SSH_ASKPASS env; `typed` is what to send at each
        // password prompt (None = send nothing).
        fn phase(
            cfg: &std::path::Path,
            helper: &std::path::Path,
            secrets: &std::path::Path,
            log: &std::path::Path,
            askpass: bool,
            typed: Option<&str>,
        ) -> String {
            let pty_system = native_pty_system();
            let pair = pty_system
                .openpty(PtySize { rows: 24, cols: 200, pixel_width: 0, pixel_height: 0 })
                .unwrap();
            let mut cmd = CommandBuilder::new("ssh");
            cmd.arg("-F");
            cmd.arg(cfg);
            cmd.arg("-o");
            cmd.arg("StrictHostKeyChecking=no");
            cmd.arg("-o");
            cmd.arg("UserKnownHostsFile=NUL");
            cmd.arg("hostermlive");
            cmd.arg("echo");
            cmd.arg("ACCESS_GRANTED");
            if askpass {
                cmd.env("SSH_ASKPASS", helper);
                cmd.env("SSH_ASKPASS_REQUIRE", "force");
                cmd.env("DISPLAY", "1");
                cmd.env("HOSTERM_ASKPASS_FILE", secrets);
                cmd.env("HOSTERM_ASKPASS_HOST", "hostermlive");
                cmd.env("HOSTERM_ASKPASS_LOG", log);
            }
            let _child = pair.slave.spawn_command(cmd).unwrap();
            drop(pair.slave);
            let mut reader = pair.master.try_clone_reader().unwrap();
            let mut writer = pair.master.take_writer().unwrap();
            let (tx, rx) = std::sync::mpsc::channel::<String>();
            std::thread::spawn(move || {
                let mut buf = [0u8; 1024];
                let mut acc = String::new();
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                            if tx.send(acc.clone()).is_err() {
                                break;
                            }
                        }
                    }
                }
            });
            let mut acc = String::new();
            let mut answered = 0usize;
            let start = std::time::Instant::now();
            while start.elapsed() < std::time::Duration::from_secs(20) {
                if let Ok(s) = rx.recv_timeout(std::time::Duration::from_millis(50)) {
                    acc = s;
                }
                if let Some(pw) = typed {
                    let seen = acc.matches("'s password:").count();
                    if seen > answered {
                        answered = seen;
                        use std::io::Write as _;
                        let _ = writer.write_all(pw.as_bytes());
                        let _ = writer.write_all(b"\r");
                        let _ = writer.flush();
                    }
                }
                if acc.contains("Permission denied")
                    || acc.contains("Connection closed")
                    || acc.contains("ACCESS_GRANTED")
                {
                    break;
                }
            }
            acc
        }

        let a = phase(&cfg, &helper, &secrets, &log, true, None);
        eprintln!("[winlive] phase A (askpass): {:?}", a);
        let b = phase(&cfg, &helper, &secrets, &log, false, Some(&password));
        eprintln!("[winlive] phase B (console, real pw): {:?}", b);
        let c = phase(&cfg, &helper, &secrets, &log, false, Some(""));
        eprintln!("[winlive] phase C (console, blank pw): {:?}", c);
        eprintln!(
            "[winlive] askpass log: {:?}",
            std::fs::read_to_string(&log).unwrap_or_default()
        );
        std::env::remove_var("HOSTERM_ASKPASS_LOG");
        std::env::remove_var("HOSTERM_CONFIG");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            !a.contains("Permission denied"),
            "ssh.exe askpass auth failed: {:?}",
            a
        );
    }

    #[cfg(windows)]
    #[test]
    fn conpty_env_reaches_child() {
        if std::env::var("HOSTERM_WINLIVE").as_deref() != Ok("1") {
            return;
        }
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize { rows: 24, cols: 200, pixel_width: 0, pixel_height: 0 })
            .unwrap();
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.arg("/c");
        cmd.arg("echo ENVPROBE=%HOSTERM_ENVPROBE%");
        cmd.env("HOSTERM_ENVPROBE", "PROBE_OK");
        let mut child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        // The ConPTY lives in `pair.master`; dropping it tears down the child's
        // console before the process can run, so it must outlive the read.
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            let mut acc = String::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                        if tx.send(acc.clone()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let mut out = String::new();
        let start = std::time::Instant::now();
        while !out.contains("ENVPROBE=PROBE_OK")
            && start.elapsed() < std::time::Duration::from_secs(15)
        {
            if let Ok(s) = rx.recv_timeout(std::time::Duration::from_millis(50)) {
                out = s;
            }
        }
        let _ = child.kill();
        drop(writer);
        eprintln!("[winlive] envprobe output: {:?}", out);
        assert!(
            out.contains("ENVPROBE=PROBE_OK"),
            "CommandBuilder::env did not reach the ConPTY child: {:?}",
            out
        );
    }

    /// Raw ConPTY input delivery: does a line written to the master reach the
    /// child's stdin and come back as echo + command output?
    #[cfg(windows)]
    #[test]
    fn conpty_input_reaches_child() {
        if std::env::var("HOSTERM_WINLIVE").as_deref() != Ok("1") {
            return;
        }
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize { rows: 24, cols: 200, pixel_width: 0, pixel_height: 0 })
            .unwrap();
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.arg("/c");
        cmd.arg("set /p V=TYPEHERE&call echo GOT:%%V%%");
        let _child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut writer = pair.master.take_writer().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            let mut acc = String::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                        if tx.send(acc.clone()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let mut acc = String::new();
        let start = std::time::Instant::now();
        while !acc.contains("TYPEHERE")
            && start.elapsed() < std::time::Duration::from_secs(10)
        {
            if let Ok(s) = rx.recv_timeout(std::time::Duration::from_millis(50)) {
                acc = s;
            }
        }
        eprintln!("[winlive] input probe pre: {:?}", acc);
        assert!(acc.contains("TYPEHERE"), "cmd.exe prompt never appeared: {:?}", acc);
        use std::io::Write as _;
        let _ = writer.write_all(b"hello\r");
        let _ = writer.flush();
        let start = std::time::Instant::now();
        while !acc.contains("GOT:hello")
            && start.elapsed() < std::time::Duration::from_secs(10)
        {
            if let Ok(s) = rx.recv_timeout(std::time::Duration::from_millis(50)) {
                acc = s;
            }
        }
        eprintln!("[winlive] input probe post: {:?}", acc);
        // %V% is only expanded at run time by `call echo`, so the text we
        // typed must show up after `GOT:`.
        assert!(
            acc.contains("GOT:") && acc.contains("hello"),
            "ConPTY input was not delivered: {:?}",
            acc
        );
    }

    /// What kind of handle does a ConPTY child actually get on STDIN? The CRT's
    /// `_getch()` (used by Win32-OpenSSH `read_passphrase()`) keys off
    /// `GetStdHandle(STD_INPUT_HANDLE)`; if that is the ConPTY input *pipe*
    /// rather than a character device, `ReadConsoleInput` cannot be used there
    /// and the CRT's own fallback decides whether the password can ever be read.
    #[cfg(windows)]
    #[test]
    fn conpty_stdin_handle_kind() {
        if std::env::var("HOSTERM_WINLIVE").as_deref() != Ok("1") {
            return;
        }
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("hm-conkind-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("kind.rs");
        std::fs::write(&src, r#"
use std::os::raw::{c_ulong, c_void};
extern "system" {
    fn GetStdHandle(n: u32) -> *mut c_void;
    fn GetFileType(h: *mut c_void) -> u32;
    fn GetConsoleMode(h: *mut c_void, mode: *mut c_ulong) -> i32;
    fn ReadFile(h: *mut c_void, buf: *mut u8, n: u32, read: *mut u32, ov: *mut c_void) -> i32;
}
fn main() {
    unsafe {
        let h = GetStdHandle((-10i32) as u32); // STD_INPUT_HANDLE
        let ft = GetFileType(h);
        let mut mode: c_ulong = 0;
        let cm = GetConsoleMode(h, &mut mode);
        let mut buf = [0u8; 64];
        let mut read: u32 = 0;
        let r = ReadFile(h, buf.as_mut_ptr(), 64, &mut read, std::ptr::null_mut());
        println!("filetype={} consolemode={} mode={} readfile={} read={} bytes={:?}",
            ft, cm, mode, r, read, &buf[..read as usize]);
    }
}
"#).unwrap();
        let exe = dir.join("kind.exe");
        let st = std::process::Command::new("rustc")
            .args(["-O", "-o"])
            .arg(&exe)
            .arg(&src)
            .output()
            .unwrap();
        eprintln!("[winlive] handle probe build: {}", st.status.success());
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize { rows: 24, cols: 200, pixel_width: 0, pixel_height: 0 })
            .unwrap();
        let cmd = CommandBuilder::new(&exe);
        let _child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut writer = pair.master.take_writer().unwrap();
        // Let the child initialise its std handles before typing.
        std::thread::sleep(std::time::Duration::from_millis(500));
        use std::io::Write as _;
        let _ = writer.write_all(b"SEKRET\r");
        let _ = writer.flush();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            let mut acc = String::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                        if tx.send(acc.clone()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let mut acc = String::new();
        let start = std::time::Instant::now();
        while !acc.contains("filetype=") && start.elapsed() < std::time::Duration::from_secs(15) {
            if let Ok(s) = rx.recv_timeout(std::time::Duration::from_millis(50)) {
                acc = s;
            }
        }
        eprintln!("[winlive] stdin handle probe: {:?}", acc);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(acc.contains("filetype="), "handle probe produced no output: {:?}", acc);
    }

    /// `set /p` reads the COOKED console line buffer (ReadConsole). The SFTP
    /// password path instead uses `_getch()` (ReadConsoleInput) via Win32-OpenSSH
    /// `read_passphrase()`, so prove that ConPTY feeds THAT API too. `pause`
    /// waits on a key event from the raw console input queue; if our write
    /// reaches it, `RESUMED` is printed.
    #[cfg(windows)]
    #[test]
    fn conpty_readconsoleinput_reaches_child() {
        if std::env::var("HOSTERM_WINLIVE").as_deref() != Ok("1") {
            return;
        }
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize { rows: 24, cols: 200, pixel_width: 0, pixel_height: 0 })
            .unwrap();
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.arg("/c");
        cmd.arg("pause&echo RESUMED");
        let _child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut writer = pair.master.take_writer().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            let mut acc = String::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                        if tx.send(acc.clone()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let mut acc = String::new();
        let start = std::time::Instant::now();
        while !acc.contains("Press any key") && start.elapsed() < std::time::Duration::from_secs(10) {
            if let Ok(s) = rx.recv_timeout(std::time::Duration::from_millis(50)) {
                acc = s;
            }
        }
        eprintln!("[winlive] readconsoleinput probe pre: {:?}", acc);
        assert!(
            acc.contains("Press any key"),
            "cmd.exe pause prompt never appeared: {:?}",
            acc
        );
        use std::io::Write as _;
        let _ = writer.write_all(b"x\r");
        let _ = writer.flush();
        let start = std::time::Instant::now();
        while !acc.contains("RESUMED") && start.elapsed() < std::time::Duration::from_secs(10) {
            if let Ok(s) = rx.recv_timeout(std::time::Duration::from_millis(50)) {
                acc = s;
            }
        }
        eprintln!("[winlive] readconsoleinput probe post: {:?}", acc);
        assert!(
            acc.contains("RESUMED"),
            "ConPTY input never reached the raw console input queue: {:?}",
            acc
        );
    }

    /// `choice.exe` consumes individual KEY_EVENTs from the raw console input
    /// queue (ReadConsoleInput), the same API Win32-OpenSSH's `_getch()` uses.
    /// `/t 5 /d y` makes it fall back to `y` (errorlevel 2) when no key arrives,
    /// so RC1 after we write `x` proves ConPTY feeds ReadConsoleInput too.
    #[cfg(windows)]
    #[test]
    fn conpty_choice_receives_input() {
        if std::env::var("HOSTERM_WINLIVE").as_deref() != Ok("1") {
            return;
        }
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize { rows: 24, cols: 200, pixel_width: 0, pixel_height: 0 })
            .unwrap();
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.arg("/c");
        cmd.arg("choice /c xy /n /t 5 /d y /m PICK&if errorlevel 2 (echo RC2) else if errorlevel 1 (echo RC1) else (echo RC0)");
        let _child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut writer = pair.master.take_writer().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            let mut acc = String::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                        if tx.send(acc.clone()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let mut acc = String::new();
        let start = std::time::Instant::now();
        while !acc.contains("PICK") && start.elapsed() < std::time::Duration::from_secs(10) {
            if let Ok(s) = rx.recv_timeout(std::time::Duration::from_millis(50)) {
                acc = s;
            }
        }
        eprintln!("[winlive] choice probe pre: {:?}", acc);
        assert!(acc.contains("PICK"), "choice.exe prompt never appeared: {:?}", acc);
        use std::io::Write as _;
        let _ = writer.write_all(b"x");
        let _ = writer.flush();
        let start = std::time::Instant::now();
        while !acc.contains("RC1") && !acc.contains("RC2") && start.elapsed() < std::time::Duration::from_secs(10)
        {
            if let Ok(s) = rx.recv_timeout(std::time::Duration::from_millis(50)) {
                acc = s;
            }
        }
        eprintln!("[winlive] choice probe post: {:?}", acc);
        assert!(
            acc.contains("RC1"),
            "choice.exe never saw the key event (ReadConsoleInput dead under ConPTY?): {:?}",
            acc
        );
    }

    /// Does input written IMMEDIATELY after spawn survive, or does ConPTY drop
    /// it before the child console is up? (The old sftp_spawn wrote its
    /// sacrificial CR here.) Diagnostic only: logs the outcome.
    #[cfg(windows)]
    #[test]
    fn conpty_early_input_survives() {
        if std::env::var("HOSTERM_WINLIVE").as_deref() != Ok("1") {
            return;
        }
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize { rows: 24, cols: 200, pixel_width: 0, pixel_height: 0 })
            .unwrap();
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.arg("/c");
        cmd.arg("set /p V=TYPEHERE&call echo GOT:%%V%%");
        let _child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut writer = pair.master.take_writer().unwrap();
        use std::io::Write as _;
        let _ = writer.write_all(b"\r");
        let _ = writer.flush();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            let mut acc = String::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                        if tx.send(acc.clone()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let mut acc = String::new();
        let start = std::time::Instant::now();
        while !acc.contains("GOT:")
            && start.elapsed() < std::time::Duration::from_secs(10)
        {
            if let Ok(s) = rx.recv_timeout(std::time::Duration::from_millis(50)) {
                acc = s;
            }
        }
        eprintln!("[winlive] early-CR outcome: {:?}", acc);
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

    // ---- BE-ANS2: build_ansible_argv (INV-28 array shape) ----

    #[test]
    fn argv_check_mode_has_check_diff() {
        let a = build_ansible_argv(true, "/tmp/inv.ini", "web,db", "/pb/site.yml");
        assert_eq!(
            a,
            vec![
                "-i", "/tmp/inv.ini",
                "--check", "--diff",
                "--limit", "web,db",
                "/pb/site.yml",
            ]
        );
        // --check and --diff present in check mode.
        assert!(a.contains(&"--check".to_string()));
        assert!(a.contains(&"--diff".to_string()));
    }

    #[test]
    fn argv_real_mode_omits_check_diff() {
        let a = build_ansible_argv(false, "/tmp/inv.ini", "web,db", "/pb/site.yml");
        assert_eq!(
            a,
            vec!["-i", "/tmp/inv.ini", "--limit", "web,db", "/pb/site.yml"]
        );
        assert!(!a.contains(&"--check".to_string()), "real mode has no --check");
        assert!(!a.contains(&"--diff".to_string()), "real mode has no --diff");
    }

    #[test]
    fn argv_playbook_and_limit_each_single_element() {
        // targets expression and playbook path carry spaces + shell metachars;
        // each must stay ONE argv element — never split, never shell-wrapped.
        let targets = "web host,db host";
        let playbook = "/pb/my site; rm -rf x.yml";
        let a = build_ansible_argv(true, "/tmp/inv.ini", targets, playbook);
        // --limit value is exactly the targets string, one element.
        let li = a.iter().position(|s| s == "--limit").unwrap();
        assert_eq!(a[li + 1], targets, "limit is one element, verbatim");
        // playbook is the LAST element, verbatim, one element.
        assert_eq!(a.last().unwrap(), playbook, "playbook is one element, verbatim");
        // No element is a shell invocation (INV-28 / NFR-ANS5): no sh -c wrapper.
        assert!(
            !a.iter().any(|s| s == "sh" || s == "bash" || s == "-c" || s == "cmd"),
            "argv is a direct array, never a shell string"
        );
    }

    /// INV-28 marker-file fixture: a file whose name is a shell metachar bomb.
    /// Because the argv is a direct array (never `sh -c "<interp>"`), the
    /// playbook element is the path VERBATIM — the metachars are inert data,
    /// never expanded. We assert the array element equals the on-disk path
    /// byte-for-byte, which proves no shell interpolation happened to it.
    #[test]
    fn argv_marker_file_proves_no_shell_interp() {
        // A playbook path whose name embeds shell metachars `;`, `$()`, `&&` — a
        // shell would mangle/expand any of these; a direct argv element keeps them
        // literal. We assert on the ARGV STRING, not on writing a real file: the
        // point is that `build_ansible_argv` never hands the value to a shell, and
        // these chars (plus Windows-illegal `|`/`:`) must never need to be a valid
        // filename for that proof. (A real file with `|`/`:` in its name is illegal
        // on Windows NTFS, so this test used to panic cross-platform.)
        let path_str = "pb;$(touch PWNED)&&echo x.yml".to_string();
        let a = build_ansible_argv(true, "/tmp/inv.ini", "all", &path_str);
        // The playbook element is the exact string — metachars preserved as data,
        // occupying a single argv slot (not split on `;`/whitespace by a shell).
        assert_eq!(a.last().unwrap(), &path_str);
        assert!(a.last().unwrap().contains("$("), "metachars preserved literally");
        assert!(a.last().unwrap().contains("&&"), "metachars preserved literally");
        // argv is a fixed array of discrete elements; the dangerous string is ONE
        // element, never expanded or re-split — nothing was handed to a shell.
        assert!(
            a.iter().filter(|e| e.contains("$(")).count() == 1,
            "metachar string stays a single argv element, never shell-split"
        );
    }

    // ---- BE-ANS2: build_inventory_ini (INV-29/31) ----

    fn invh(alias: &str, host: &str, user: &str, port: &str, key: &str) -> AnsibleInvHost {
        AnsibleInvHost {
            alias: alias.to_string(),
            host_name: host.to_string(),
            user: user.to_string(),
            port: port.to_string(),
            identity_file: key.to_string(),
        }
    }

    #[test]
    fn inventory_full_line_maps_all_vars() {
        let hosts = vec![invh("prod", "203.0.113.10", "root", "2222", "~/.ssh/id_ed25519")];
        let ini = build_inventory_ini(&hosts);
        assert_eq!(
            ini,
            "prod ansible_host=203.0.113.10 ansible_user=root ansible_port=2222 ansible_ssh_private_key_file=~/.ssh/id_ed25519\n"
        );
    }

    #[test]
    fn inventory_omits_absent_fields() {
        // No port, no key configured → those vars are absent from the line.
        let hosts = vec![invh("staging", "staging.example.com", "deploy", "", "")];
        let ini = build_inventory_ini(&hosts);
        assert_eq!(ini, "staging ansible_host=staging.example.com ansible_user=deploy\n");
        assert!(!ini.contains("ansible_port="), "absent port omitted");
        assert!(!ini.contains("ansible_ssh_private_key_file="), "absent key omitted");
    }

    #[test]
    fn inventory_group_headers_from_prefix() {
        // prod-web / prod-db share group [prod]; ungrouped `solo` leads; `db-1`
        // forms [db]. Group order = first-seen.
        let hosts = vec![
            invh("solo", "10.0.0.9", "root", "", "~/.ssh/k"),
            invh("prod-web", "10.0.0.1", "root", "", "~/.ssh/k"),
            invh("prod-db", "10.0.0.2", "root", "", "~/.ssh/k"),
            invh("db-1", "10.0.0.3", "root", "", "~/.ssh/k"),
        ];
        let ini = build_inventory_ini(&hosts);
        let expected = "\
solo ansible_host=10.0.0.9 ansible_user=root ansible_ssh_private_key_file=~/.ssh/k
[prod]
prod-web ansible_host=10.0.0.1 ansible_user=root ansible_ssh_private_key_file=~/.ssh/k
prod-db ansible_host=10.0.0.2 ansible_user=root ansible_ssh_private_key_file=~/.ssh/k
[db]
db-1 ansible_host=10.0.0.3 ansible_user=root ansible_ssh_private_key_file=~/.ssh/k
";
        assert_eq!(ini, expected);
    }

    #[test]
    fn inventory_never_carries_key_bytes_or_password() {
        // Only the IdentityFile PATH goes in — never key bytes, never any
        // password material (INV-31). Password hosts are filtered upstream, so
        // the inventory contains only the eligible host and only its path.
        let hosts = vec![invh("prod", "10.0.0.1", "root", "22", "/home/u/.ssh/id_ed25519")];
        let ini = build_inventory_ini(&hosts);
        assert!(ini.contains("/home/u/.ssh/id_ed25519"), "path present");
        assert!(!ini.contains("BEGIN OPENSSH PRIVATE KEY"), "no key bytes");
        assert!(!ini.contains("ansible_password"), "no password var");
        assert!(!ini.contains("ansible_ssh_pass"), "no ssh_pass var");
    }

    // ---- BE-ANS2: host_eligibility_ansible (INV-31) ----

    #[test]
    fn eligibility_key_and_agent_hosts_run() {
        assert!(matches!(
            host_eligibility_ansible("prod", false),
            AnsibleEligibility::Eligible
        ));
        assert!(matches!(
            host_eligibility_ansible("prod-web-01", false),
            AnsibleEligibility::Eligible
        ));
    }

    #[test]
    fn eligibility_password_host_skipped_with_reason() {
        match host_eligibility_ansible("legacybox", true) {
            AnsibleEligibility::Skipped(r) => assert_eq!(r, R_ANS_PASSWORD),
            _ => panic!("password host must be skipped"),
        }
    }

    #[test]
    fn eligibility_unsafe_alias_skipped() {
        // leading '-' (option injection), CR, LF, and wildcards all → skipped,
        // never spawned, never in inventory (INV-26/INV-31).
        for bad in ["-oProxyCommand=x", "evil\nalias", "host\rname", "web-*", "db-?"] {
            match host_eligibility_ansible(bad, false) {
                AnsibleEligibility::Skipped(r) => assert_eq!(r, R_ANS_UNSAFE, "alias {bad:?}"),
                _ => panic!("unsafe alias {bad:?} must be skipped"),
            }
        }
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
/// The browser/loopback/network flow is blocking and may wait up to five
/// minutes, so keep it off Tauri's UI thread.
#[tauri::command]
async fn google_sign_in() -> Result<oauth::GoogleAccount, String> {
    tauri::async_runtime::spawn_blocking(oauth::interactive_sign_in)
        .await
        .map_err(|e| format!("Google sign-in task failed: {e}"))?
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
            write_identity_file,
            rename_identity_file,
            read_identity_file,
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
            ansible_run,
            ansible_history,
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

#[cfg(test)]
mod askpass_tests {
    use super::*;

    /// The sidecar contract shared by the in-process askpass entry point and the
    /// `hosterm-askpass` helper binary: `alias=password` lines, first match wins,
    /// empty passwords and unknown hosts are not answers.
    #[test]
    fn askpass_password_reads_the_sidecar() {
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("hosterm-askpass-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sidecar = dir.join("hosterm.secrets");
        std::fs::write(&sidecar, "alpha=first\nbeta=second\ngamma=\n").unwrap();
        std::env::set_var("HOSTERM_ASKPASS_FILE", &sidecar);
        assert_eq!(askpass_password("alpha").as_deref(), Some("first"));
        assert_eq!(askpass_password("beta").as_deref(), Some("second"));
        // empty stored password is not a usable answer
        assert_eq!(askpass_password("gamma"), None);
        assert_eq!(askpass_password("delta"), None);
        std::env::remove_var("HOSTERM_ASKPASS_FILE");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `maybe_run_askpass` must not treat an ordinary launch (no sidecar env, or
    /// an argument that is not a password prompt) as an askpass invocation —
    /// otherwise the GUI would exit before it ever opens.
    #[test]
    fn maybe_run_askpass_ignores_ordinary_launches() {
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("HOSTERM_ASKPASS_FILE");
        // No sidecar env: must return normally (not exit, not print).
        maybe_run_askpass();
    }

    /// The helper must print exactly the stored password and nothing else:
    /// `ssh_askpass()` does one `ReadFile` up to the first newline and
    /// `read_passphrase()` truncates at the first CR/LF, so any extra output on
    /// stdout becomes part of the password. Compiled with `rustc` because the
    /// helper is std-only; skipped where no `rustc` is on PATH (the Windows live
    /// test compiles it for real).
    #[test]
    fn askpass_helper_prints_only_the_password() {
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("hosterm-askpass-bin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sidecar = dir.join("hosterm.secrets");
        std::fs::write(&sidecar, "box=hunter2\n").unwrap();
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/hosterm-askpass.rs");
        let exe = dir.join(if cfg!(windows) { "hosterm-askpass.exe" } else { "hosterm-askpass" });
        let compiled = std::process::Command::new("rustc")
            .arg("--edition")
            .arg("2021")
            .arg("-O")
            .arg(&src)
            .arg("-o")
            .arg(&exe)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !compiled {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let out = std::process::Command::new(&exe)
            .env("HOSTERM_ASKPASS_FILE", &sidecar)
            .env("HOSTERM_ASKPASS_HOST", "box")
            .arg("box@127.0.0.1's password: ")
            .output()
            .expect("run askpass helper");
        assert!(out.status.success(), "helper exited {}", out.status);
        assert_eq!(String::from_utf8_lossy(&out.stdout), "hunter2");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
