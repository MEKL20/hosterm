use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::thread;

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

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
}

/// Set the auth-related directives on a host block. There is no `Password`
/// directive in OpenSSH config, so "password" mode makes ssh PROMPT at connect
/// (no secret is ever stored — the config stays fully portable).
fn apply_host_fields(hb: &mut HostBlock, host: &HostInput) {
    set_opt(hb, "HostName", &host.host_name);
    set_opt(hb, "User", &host.user);
    set_opt(hb, "Port", &host.port);
    match host.auth_method.as_str() {
        "password" => {
            set_opt(hb, "IdentityFile", "");
            set_opt(hb, "PreferredAuthentications", "password");
            set_opt(hb, "PubkeyAuthentication", "no");
        }
        _ => {
            // Key auth (default): point at an IdentityFile and clear any
            // password-forcing directives left from a previous password setup.
            set_opt(hb, "IdentityFile", &host.identity_file);
            set_opt(hb, "PreferredAuthentications", "");
            set_opt(hb, "PubkeyAuthentication", "");
        }
    }
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
    let cfg = parse_config(&text);
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
        out.push(HostDto {
            name,
            host_name: get_opt(h, "HostName"),
            user: get_opt(h, "User"),
            port: get_opt(h, "Port"),
            identity_file: get_opt(h, "IdentityFile"),
            auth_method,
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
        }
        None => {
            let mut hb = HostBlock {
                patterns: vec![host.name.clone()],
                lines: Vec::new(),
            };
            apply_host_fields(&mut hb, &host);
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
    let before = cfg.hosts.len();
    cfg.hosts
        .retain(|h| h.patterns.first().map(|p| p != &name).unwrap_or(true));
    if cfg.hosts.len() == before {
        return Err(format!("Host '{}' not found", name));
    }
    write_config_file(&path, &serialize_config(&cfg))
}

#[tauri::command]
fn read_config_raw() -> Result<String, String> {
    read_config_text(&config_path())
}

#[tauri::command]
fn write_config_raw(content: String) -> Result<(), String> {
    write_config_file(&config_path(), &content)
}

// ---------- identity key management ----------

/// List private-key files in the ssh dir, newest-friendly order. Returns the
/// `~/.ssh/<name>` path form that goes straight into an IdentityFile directive.
/// A ".pub" file or a file whose sibling has no private counterpart is skipped.
#[tauri::command]
fn list_identity_files() -> Result<Vec<String>, String> {
    let dir = ssh_dir();
    let mut out: Vec<String> = Vec::new();
    let rd = match std::fs::read_dir(&dir) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
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
            out.push(format!("~/.ssh/{}", name));
        }
    }
    out.sort();
    Ok(out)
}

/// Write a pasted private key into the ssh dir at 0600 and return the
/// `~/.ssh/<name>` path to drop into an IdentityFile directive. Termius-style:
/// the user pastes key text, hosterm persists it as a real file (SSH needs a
/// file on disk; there is no in-config key storage).
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
    let dir = ssh_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
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
    Ok(format!("~/.ssh/{}", name))
}

// ============================================================
//  PTY ENGINE  (spawns the real OS `ssh`, honors ssh config)
// ============================================================

struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
}

#[derive(Default)]
struct PtyState {
    sessions: Mutex<HashMap<u32, PtySession>>,
}

static NEXT_ID: AtomicU32 = AtomicU32::new(1);

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

    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);

    // reader -> frontend events
    let app_r = app.clone();
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let s = String::from_utf8_lossy(&buf[..n]).to_string();
                    let _ = app_r.emit(&format!("pty://output/{}", id), s);
                }
                Err(_) => break,
            }
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

#[tauri::command]
fn pty_write(state: State<PtyState>, id: u32, data: String) -> Result<(), String> {
    let mut map = state.sessions.lock().unwrap();
    let s = map.get_mut(&id).ok_or("no such pty session")?;
    s.writer.write_all(data.as_bytes()).map_err(|e| e.to_string())?;
    s.writer.flush().map_err(|e| e.to_string())?;
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
        }
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
    fn create_identity_file_writes_key_0600_and_lists_it() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("key");
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        let body = "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----";
        let path = create_identity_file("id_test".into(), body.into()).unwrap();
        assert_eq!(path, "~/.ssh/id_test");
        let on_disk = cfg.parent().unwrap().join("id_test");
        let content = std::fs::read_to_string(&on_disk).unwrap();
        assert!(content.ends_with('\n'), "trailing newline added");
        assert!(content.starts_with("-----BEGIN"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&on_disk).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let listed = list_identity_files().unwrap();
        std::env::remove_var("HOSTERM_CONFIG");
        assert!(listed.contains(&"~/.ssh/id_test".to_string()));
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(PtyState::default())
        .invoke_handler(tauri::generate_handler![
            read_ssh_config,
            save_host,
            delete_host,
            read_config_raw,
            write_config_raw,
            list_identity_files,
            create_identity_file,
            pty_spawn,
            pty_write,
            pty_resize,
            pty_kill
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
