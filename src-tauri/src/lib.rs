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
        out.push_str("Host ");
        out.push_str(&h.patterns.join(" "));
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
    std::fs::write(path, content).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
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
    options: Vec<(String, String)>,
}

#[derive(Deserialize)]
struct HostInput {
    name: String,
    host_name: String,
    user: String,
    port: String,
    identity_file: String,
}

// ---------- SSH config commands ----------

#[tauri::command]
fn read_ssh_config() -> Result<Vec<HostDto>, String> {
    let path = config_path();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
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
        out.push(HostDto {
            name,
            host_name: get_opt(h, "HostName"),
            user: get_opt(h, "User"),
            port: get_opt(h, "Port"),
            identity_file: get_opt(h, "IdentityFile"),
            options,
        });
    }
    Ok(out)
}

#[tauri::command]
fn save_host(original_name: Option<String>, host: HostInput) -> Result<(), String> {
    if host.name.trim().is_empty() {
        return Err("Host alias cannot be empty".into());
    }
    let path = config_path();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
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
            set_opt(hb, "HostName", &host.host_name);
            set_opt(hb, "User", &host.user);
            set_opt(hb, "Port", &host.port);
            set_opt(hb, "IdentityFile", &host.identity_file);
        }
        None => {
            let mut hb = HostBlock {
                patterns: vec![host.name.clone()],
                lines: Vec::new(),
            };
            set_opt(&mut hb, "HostName", &host.host_name);
            set_opt(&mut hb, "User", &host.user);
            set_opt(&mut hb, "Port", &host.port);
            set_opt(&mut hb, "IdentityFile", &host.identity_file);
            cfg.hosts.push(hb);
        }
    }
    write_config_file(&path, &serialize_config(&cfg))
}

#[tauri::command]
fn delete_host(name: String) -> Result<(), String> {
    let path = config_path();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
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
    Ok(std::fs::read_to_string(config_path()).unwrap_or_default())
}

#[tauri::command]
fn write_config_raw(content: String) -> Result<(), String> {
    write_config_file(&config_path(), &content)
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
            pty_spawn,
            pty_write,
            pty_resize,
            pty_kill
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
