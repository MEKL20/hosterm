//! Minimal SSH_ASKPASS helper for Windows.
//!
//! Win32-OpenSSH's `read_passphrase()` consults `SSH_ASKPASS` *instead* of the
//! console and launches it with `CreateProcess("\"%s\" \"%s\"")` — a single
//! argument (the prompt) and no shell, so a `.cmd` shim can never run. The
//! helper must be a real `.exe`.
//!
//! It prints exactly the stored password for `HOSTERM_ASKPASS_HOST` read from
//! the sidecar named by `HOSTERM_ASKPASS_FILE` and nothing else: `read_passphrase()`
//! takes everything up to the first newline as the password, so any extra byte
//! would corrupt it.
fn main() {
    let Ok(path) = std::env::var("HOSTERM_ASKPASS_FILE") else {
        std::process::exit(1);
    };
    let Ok(host) = std::env::var("HOSTERM_ASKPASS_HOST") else {
        std::process::exit(1);
    };
    // Optional support probe: when `HOSTERM_ASKPASS_LOG` is set, record what the
    // helper was asked and what it emitted. Nothing is written otherwise, so
    // ordinary use leaves no trace on disk.
    let log = std::env::var("HOSTERM_ASKPASS_LOG").ok();
    let note = |msg: &str| {
        if let Some(log) = log.as_deref() {
            use std::io::Write as _;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
                let _ = writeln!(f, "{msg}");
            }
        }
    };
    note(&format!(
        "invoked host={host} file={path} stdout_ft={:?}",
        stdout_file_type()
    ));
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            note(&format!("read-failed host={host}: {e}"));
            std::process::exit(1);
        }
    };
    let pw = text.lines().find_map(|line| {
        let (alias, secret) = line.split_once('=')?;
        (alias == host).then(|| secret.to_string())
    });
    let Some(pw) = pw else {
        note(&format!("no-answer host={host}"));
        std::process::exit(1);
    };
    use std::io::Write as _;
    let mut out = std::io::stdout();
    let wrote = out.write_all(pw.as_bytes()).and_then(|_| out.flush()).is_ok();
    note(&format!("emitted host={host} pw_len={} wrote={wrote}", pw.len()));
}

/// `GetFileType(GetStdHandle(STD_OUTPUT_HANDLE))`: 3 = pipe, 2 = character
/// device (console). Diagnostic only — it proves which handle the helper's
/// stdout really is when `ssh_askpass` launches it.
#[cfg(windows)]
fn stdout_file_type() -> Option<u32> {
    extern "system" {
        fn GetStdHandle(n: u32) -> *mut std::ffi::c_void;
        fn GetFileType(h: *mut std::ffi::c_void) -> u32;
    }
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    let h = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    if h.is_null() {
        return None;
    }
    Some(unsafe { GetFileType(h) })
}

#[cfg(not(windows))]
fn stdout_file_type() -> Option<u32> {
    None
}
