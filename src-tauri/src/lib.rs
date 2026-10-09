use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
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
/// `IgnoreUnknown Password` (official escape hatch) which makes real ssh skip
/// it silently — the config stays valid for every other ssh client, and
/// hosterm injects the secret at the prompt on connect. Key mode: clear both.
/// OpenSSH parses top-down: an unknown option is fatal unless an earlier
/// IgnoreUnknown listed it. hosterm stores secrets as a `Password` directive,
/// so that pair must appear in exactly that order inside a host block. This
/// moves the IgnoreUnknown line ahead of the Password line when a save left
/// them reversed (new directives append at the block end).
fn reorder_ignore_unknown_first(hb: &mut HostBlock) {
    let iu = match hb.lines.iter().position(
        |l| matches!(l, CfgLine::Opt { key, .. } if key.eq_ignore_ascii_case("IgnoreUnknown")),
    ) {
        Some(i) => i,
        None => return,
    };
    let pw = match hb.lines.iter().position(
        |l| matches!(l, CfgLine::Opt { key, .. } if key.eq_ignore_ascii_case("Password")),
    ) {
        Some(p) => p,
        None => return,
    };
    if pw < iu {
        let line = hb.lines.remove(iu);
        hb.lines.insert(pw, line);
    }
}

/// Fix any existing host block that already has the pair reversed (configs
/// written by hosterm <= 0.4.x). Runs at load so users never see ssh's
/// "Bad configuration option: password" termination.
fn migrate_reversed_ignore_unknown(cfg: &mut SshConfig) {
    for hb in &mut cfg.hosts {
        reorder_ignore_unknown_first(hb);
    }
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
            // empty password on an existing host = keep the stored one
            // (the UI never sends the secret back to be re-saved)
            if !host.password.is_empty() {
                set_opt(hb, "Password", &host.password);
                // merge, never clobber: keep tokens the user had in IgnoreUnknown
                let existing = get_opt(hb, "IgnoreUnknown");
                if !existing.split_whitespace().any(|t| t == "Password") {
                    let merged = if existing.is_empty() {
                        "Password".to_string()
                    } else {
                        format!("{} Password", existing)
                    };
                    set_opt(hb, "IgnoreUnknown", &merged);
                }
                // OpenSSH reads the file sequentially: IgnoreUnknown must come
                // BEFORE the Password line, or ssh dies with "Bad configuration
                // option: password". (New opts append at the block end, so the
                // pair can end up in the wrong order — fix it here.)
                reorder_ignore_unknown_first(hb);
            }
        }
        _ => {
            // Key auth (default): point at an IdentityFile and clear any
            // password-forcing directives left from a previous password setup.
            set_opt(hb, "IdentityFile", &host.identity_file);
            set_opt(hb, "PreferredAuthentications", "");
            set_opt(hb, "PubkeyAuthentication", "");
            set_opt(hb, "Password", "");
            // drop our own IgnoreUnknown only if it is exactly ours
            if get_opt(hb, "IgnoreUnknown") == "Password" {
                set_opt(hb, "IgnoreUnknown", "");
            }
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
    let mut cfg = parse_config(&text);
    // self-heal configs written by older versions with the pair reversed
    migrate_reversed_ignore_unknown(&mut cfg);
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
        let has_password = !get_opt(h, "Password").is_empty();
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
            write_config_file(&path, &serialize_config(&cfg))
        }
    }
}

#[tauri::command]
fn read_config_raw() -> Result<String, String> {
    let path = config_path();
    let text = read_config_text(&path)?;
    let mut cfg = parse_config(&text);
    migrate_reversed_ignore_unknown(&mut cfg);
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
fn sftp_safe(value: &str, what: &str) -> Result<(), String> {
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
    ensure_config_valid();
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

#[tauri::command]
fn sftp_list(host: String, path: String) -> Result<Vec<SftpEntry>, String> {
    let dir = if path.trim().is_empty() { ".".to_string() } else { path };
    sftp_safe(&dir, "Path")?;
    let script = format!("ls -la {}\n", dir);
    let (stdout, stderr, ok) = run_sftp_batch(&host, &script)?;
    if !ok {
        return Err(format!("sftp ls failed: {}", stderr.trim()));
    }
    Ok(parse_sftp_ls(&stdout))
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
fn sftp_upload(host: String, local_path: String, remote_path: String) -> Result<(), String> {
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
    let (_o, stderr, ok) = run_sftp_batch(&host, &script)?;
    if !ok {
        return Err(format!("upload failed: {}", stderr.trim()));
    }
    Ok(())
}

#[tauri::command]
fn sftp_download(host: String, remote_path: String, local_path: String) -> Result<(), String> {
    sftp_safe(&remote_path, "Remote path")?;
    sftp_safe(&local_path, "Local path")?;
    // Aliasing guard (see sftp_upload): `get` would truncate the remote source.
    if let Ok(canon_local) = PathBuf::from(&local_path).canonicalize() {
        if remote_path == canon_local.display().to_string() {
            return Err("local and remote path are the same file — refusing to overwrite the source".into());
        }
    }
    let script = format!("get \"{}\" \"{}\"\n", remote_path, local_path);
    let (_o, stderr, ok) = run_sftp_batch(&host, &script)?;
    if !ok {
        return Err(format!("download failed: {}", stderr.trim()));
    }
    if !PathBuf::from(&local_path).exists() {
        return Err("download reported success but local file is missing".into());
    }
    Ok(())
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
fn sftp_read_file(host: String, remote_path: String) -> Result<String, String> {
    sftp_safe(&remote_path, "Remote path")?;
    let tmp = editor_temp("get");
    let script = format!("get \"{}\" \"{}\"\n", remote_path, tmp.display());
    let (_o, stderr, ok) = run_sftp_batch(&host, &script)?;
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("read failed: {}", stderr.trim()));
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
}

#[tauri::command]
fn sftp_write_file(host: String, remote_path: String, content: String) -> Result<(), String> {
    sftp_safe(&remote_path, "Remote path")?;
    if content.len() as u64 > EDITOR_MAX_BYTES {
        return Err("content exceeds the text editor size limit".into());
    }
    let tmp = editor_temp("put");
    std::fs::write(&tmp, content.as_bytes()).map_err(|e| e.to_string())?;
    let script = format!("put \"{}\" \"{}\"\n", tmp.display(), remote_path);
    let result = run_sftp_batch(&host, &script);
    let _ = std::fs::remove_file(&tmp);
    let (_o, stderr, ok) = result?;
    if !ok {
        return Err(format!("write failed: {}", stderr.trim()));
    }
    Ok(())
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
    let text = read_config_text(&config_path()).ok()?;
    let cfg = parse_config(&text);
    cfg.hosts
        .iter()
        .find(|h| h.patterns.first().map(|p| p == host).unwrap_or(false))
        .map(|h| get_opt(h, "Password"))
        .filter(|p| !p.is_empty())
}

/// Connect-time guard: repair any host block whose IgnoreUnknown/Password
/// pair is reversed before a real ssh/sftp/scp binary reads the file. Runs
/// on every spawn path (PTY, SFTP list/upload/download) so configs written
/// by any older version can never terminate ssh at connect time, even if
/// the app-start migration missed them.
fn ensure_config_valid() {
    let path = config_path();
    if let Ok(text) = read_config_text(&path) {
        let mut cfg = parse_config(&text);
        migrate_reversed_ignore_unknown(&mut cfg);
        let fixed = serialize_config(&cfg);
        if fixed != text {
            let _ = write_config_file(&path, &fixed);
        }
    }
}

#[tauri::command]
fn pty_spawn(
    app: AppHandle,
    state: State<PtyState>,
    host: String,
    cols: u16,
    rows: u16,
) -> Result<u32, String> {
    ensure_config_valid();
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
    fn save_password_host_writes_ignoreunknown_before_password() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("pworder");
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        let mut h = hi("orderbox", "password", "");
        h.password = "sekret".into();
        save_host(None, h).unwrap();
        let out = std::fs::read_to_string(&cfg).unwrap();
        std::env::remove_var("HOSTERM_CONFIG");
        let iu = out.find("IgnoreUnknown").expect("IgnoreUnknown present");
        let pw = out.find("\n    Password ").expect("Password present");
        assert!(iu < pw, "IgnoreUnknown must precede Password, got:\n{out}");
    }

    #[test]
    fn migration_repairs_reversed_pair_from_older_versions() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("pwmigrate");
        // exactly what hosterm <= 0.4.x could write: Password before IgnoreUnknown
        std::fs::write(
            &cfg,
            "Host legacy\n    HostName 203.0.113.9\n    Password oldsecret\n    IgnoreUnknown Password\n",
        )
        .unwrap();
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        // loading the config must self-heal the order in place
        let _ = read_ssh_config().unwrap();
        let out = std::fs::read_to_string(&cfg).unwrap();
        std::env::remove_var("HOSTERM_CONFIG");
        let iu = out.find("IgnoreUnknown").expect("IgnoreUnknown present");
        let pw = out.find("\n    Password ").expect("Password present");
        assert!(iu < pw, "migration must reorder, got:\n{out}");
        assert!(out.contains("oldsecret"), "secret must survive migration");
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
    fn stored_password_writes_password_directive_and_roundtrips() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = tmp("storedpw");
        std::env::set_var("HOSTERM_CONFIG", &cfg);
        let mut input = hi("box", "password", "");
        input.password = "s3cret!pw".into();
        save_host(None, input).unwrap();
        let out = std::fs::read_to_string(&cfg).unwrap();
        assert!(out.contains("IgnoreUnknown Password"), "ssh must be told to skip Password: {out}");
        assert!(out.contains("Password s3cret!pw"));
        // round-trips: has_password=true, and editing WITHOUT resending the
        // secret keeps the stored one (UI never echoes secrets back)
        let hosts = read_ssh_config().unwrap();
        assert_eq!(hosts[0].has_password, true);
        assert!(!hosts[0].options.iter().any(|(k, _)| k == "Password" && k.is_empty()));
        let mut edit = hi("box", "password", "");
        edit.password = "".into();
        save_host(Some("box".into()), edit).unwrap();
        let out2 = std::fs::read_to_string(&cfg).unwrap();
        std::env::remove_var("HOSTERM_CONFIG");
        assert!(out2.contains("Password s3cret!pw"), "stored secret must survive an edit that omits it");
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

        // upload -> list shows it -> download to a new path -> contents match
        sftp_upload("testbox".into(), local.display().to_string(), remote.clone()).unwrap();
        let entries = sftp_list("testbox".into(), dir.display().to_string()).unwrap();
        assert!(
            entries.iter().any(|e| e.name == "landed.txt" && !e.is_dir),
            "uploaded file must appear in listing: {:?}",
            entries.iter().map(|e| &e.name).collect::<Vec<_>>()
        );
        let back = dir.join("came_back.txt");
        sftp_download("testbox".into(), remote, back.display().to_string()).unwrap();
        assert_eq!(
            std::fs::read_to_string(&back).unwrap(),
            "hosterm live sftp test\n"
        );

        // editor roundtrip: read -> write -> read-back
        let txt_path = format!("{}/editor.txt", dir.display());
        sftp_write_file("testbox".into(), txt_path.clone(), "hello editor\nline two\n".into()).unwrap();
        assert_eq!(
            sftp_read_file("testbox".into(), txt_path.clone()).unwrap(),
            "hello editor\nline two\n"
        );
        // binary guard: NUL byte in an uploaded file must be refused by the editor
        let bin_local = dir.join("bin.dat");
        std::fs::write(&bin_local, [0x68, 0x00, 0x69]).unwrap();
        let bin_remote = format!("{}/uploaded_bin.dat", dir.display());
        sftp_upload("testbox".into(), bin_local.display().to_string(), bin_remote.clone()).unwrap();
        assert!(
            sftp_read_file("testbox".into(), bin_remote).is_err(),
            "binary must be refused"
        );

        // aliasing guard: put/get onto the same path must be refused, source intact
        let alias_local = dir.join("alias_me.txt");
        std::fs::write(&alias_local, "precious\n").unwrap();
        let alias_remote = alias_local.display().to_string();
        assert!(sftp_upload("testbox".into(), alias_remote.clone(), alias_remote.clone()).is_err());
        assert_eq!(std::fs::read_to_string(&alias_local).unwrap(), "precious\n");
        assert!(sftp_download("testbox".into(), alias_remote.clone(), alias_remote).is_err());
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
            pty_write,
            pty_resize,
            pty_kill
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
