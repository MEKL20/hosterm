//! Minimal SSH_ASKPASS helper for Windows.
//!
//! Win32-OpenSSH's `read_passphrase()` consults `SSH_ASKPASS` *instead* of the
//! console and launches it with `CreateProcess("\"%s\" \"%s\"")` — a single
//! argument (the prompt) and no shell, so a `.cmd` shim can never run. The
//! helper must be a real `.exe`.
//!
//! It prints exactly one line: the stored password for `HOSTERM_ASKPASS_HOST`
//! read from the sidecar named by `HOSTERM_ASKPASS_FILE`. Nothing else is
//! written to stdout, because `read_passphrase()` takes everything up to the
//! first newline as the password.
fn main() {
    let Ok(path) = std::env::var("HOSTERM_ASKPASS_FILE") else {
        std::process::exit(1);
    };
    let Ok(host) = std::env::var("HOSTERM_ASKPASS_HOST") else {
        std::process::exit(1);
    };
    // Optional support probe: when `HOSTERM_ASKPASS_LOG` is set, record that the
    // helper was actually launched (and for which host). Nothing is written
    // otherwise, so ordinary use leaves no trace on disk.
    if let Ok(log) = std::env::var("HOSTERM_ASKPASS_LOG") {
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
            let _ = writeln!(f, "invoked host={host}");
        }
    }
    let Ok(text) = std::fs::read_to_string(&path) else {
        std::process::exit(1);
    };
    let pw = text.lines().find_map(|line| {
        let (alias, secret) = line.split_once('=')?;
        (alias == host).then(|| secret.to_string())
    });
    let Some(pw) = pw else {
        std::process::exit(1);
    };
    use std::io::Write as _;
    let mut out = std::io::stdout();
    let _ = out.write_all(pw.as_bytes());
    let _ = out.write_all(b"\n");
    let _ = out.flush();
}
