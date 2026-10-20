// ============================================================
//  FLEET RUN v1  (SDD §12 — frozen IPC contract §12.3)
//
//  ALL fleet code lives here (§12.2). `lib.rs` contributes only:
//  `mod fleet;`, `.manage(FleetState::default())`, 2 generate_handler
//  entries, and the `pub(crate)` keyword on `sftp_safe` (finding #12).
//
//  Transport (ADR-13): real `ssh` via std::process, two argv shapes only,
//  never a local shell (INV-14). Runner (ADR-14): thread-per-host + N-token
//  std mpsc channel as semaphore + 50 ms deadline poll. `HOSTERM_SSH_BIN`
//  read fresh per spawn, fleet path only (ADR-16/§12.5). Backend writes
//  nothing (INV-16) and never reads secret VALUES (INV-15 — sidecar keys
//  only via `sidecar_aliases`, TSD finding #2).
// ============================================================

use std::collections::HashSet;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

// ---------- constants (§12.3 "Constants", single source of truth) ----------

pub const RETENTION_BYTES: u64 = 65536;
const CHUNK_READ: usize = 8192;
const POLL: Duration = Duration::from_millis(50);
pub const TEXT_MAX_BYTES: usize = 2_097_152;
const TIMEOUT_MIN: i64 = 5;
const TIMEOUT_MAX: i64 = 600;
const CONC_MIN: i64 = 1;
const CONC_MAX: i64 = 20;

// ---------- frozen reason codes (§12.3 table, exact strings) ----------

const R_UNSAFE: &str = "unsafe alias";
const R_UNKNOWN: &str = "unknown alias";
const R_WILDCARD: &str = "wildcard pattern";
const R_PASSWORD: &str = "password host (not supported in v1)";
const R_STOPPED: &str = "stopped by user";
const R_SPAWN: &str = "spawn failed";
const R_SIGNAL: &str = "killed by signal";

// ---------- frozen rejection error prefixes (§12.3, exact strings) ----------

const E_ALREADY: &str = "fleet:already-running";
const E_EMPTY_SELECTION: &str = "fleet:empty-selection";
const E_EMPTY_INPUT: &str = "fleet:empty-input";
const E_TEXT_TOO_LARGE: &str = "fleet:text-too-large";
const E_BAD_TIMEOUT: &str = "fleet:bad-timeout";
const E_BAD_CONCURRENCY: &str = "fleet:bad-concurrency";
const E_SSH_MISSING: &str = "fleet:ssh-missing";
const E_CONFIG_UNREADABLE: &str = "fleet:config-unreadable";

// ---------- DTOs (§12.3; payload fields snake_case on the wire) ----------

#[derive(Deserialize, Serialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FleetMode {
    Command,
    Script,
}

#[derive(Deserialize)]
pub struct FleetStartReq {
    pub run_id: String, // client-generated opaque (UUIDv4), max 64; never parsed (ADR-15)
    pub mode: FleetMode,
    pub text: String,
    pub aliases: Vec<String>,
    pub timeout_secs: i64,
    pub concurrency: i64,
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum PlanStatus {
    Queued,
    Skipped,
}

#[derive(Serialize, Clone)]
pub struct FleetHostPlan {
    pub alias: String,
    pub eligible: bool,
    pub status: PlanStatus,
    pub reason: Option<&'static str>,
}

#[derive(Serialize)]
pub struct FleetStartResp {
    pub run_id: String,
    pub hosts: Vec<FleetHostPlan>,
    pub finished: bool,
    pub summary: Option<FleetSummary>,
}

#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum HostStatus {
    Ok,
    Fail,
    Timeout,
    Skipped,
}

#[derive(Serialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Serialize)]
pub struct FleetOutputChunk {
    pub alias: String,
    pub seq: u32,           // starts 1, monotonic per host ACROSS both streams
    pub stream: Stream,
    pub bytes: String,      // from_utf8_lossy of ≤ 8192 raw bytes
    pub total_bytes: u64,   // stdout+stderr observed AFTER this chunk (pre-cap truth)
    pub truncated: bool,    // total_bytes > RETENTION_BYTES at this chunk
}

#[derive(Serialize)]
pub struct FleetHostDone {
    pub alias: String,
    pub status: HostStatus,
    pub exit_code: Option<i32>, // null when killed by timeout/Stop or signal death
    pub reason: Option<&'static str>,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub truncated: bool,
}

#[derive(Serialize, Clone)]
pub struct FleetSummary {
    pub run_id: String,
    pub ok: u32,
    pub fail: u32,
    pub timeout: u32,
    pub skipped: u32, // INCLUDES plan-skips; ok+fail+timeout+skipped == plan hosts
    pub elapsed_ms: u64,
    pub min_exit: Option<i32>, // min/max NON-ZERO exit among fail hosts
    pub max_exit: Option<i32>,
}

// ---------- pure functions (§12.4 — frozen rules) ----------

#[derive(Clone)]
pub enum Transport {
    /// one-line command text: passed as ONE ssh argv element (remote login
    /// shell interprets it there, never locally — ADR-13).
    Command(String),
    /// script mode: normalized bytes piped to stdin, remote side `bash -s`.
    Script(String),
}

/// §12.4 #2 VERBATIM: Script unless mode==Command AND text has no `\n`.
/// CRLF input contains `\n` → script (TSD finding #4; never normalize before
/// the transport choice — §12.4 #3 ordering is post-select).
pub fn pick_transport(mode: FleetMode, text: &str) -> Transport {
    if mode == FleetMode::Command && !text.contains('\n') {
        Transport::Command(text.to_string())
    } else {
        Transport::Script(normalize_script(text))
    }
}

/// §12.4 #3: \r\n → \n, then lone \r → \n, then ensure a final \n.
/// Never strips existing newlines/blank lines — they are content.
pub fn normalize_script(text: &str) -> String {
    let mut s = text.replace("\r\n", "\n").replace('\r', "\n");
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// Evidence a host's child lifecycle produced (§12.4 #4 input).
pub struct FinishEvidence {
    pub killed_by_timeout: bool,
    pub killed_by_stop: bool,
    pub exit: Option<i32>,
}

/// §12.4 #4 precedence (frozen): timeout > stop > exit 0 > exit n > None.
/// Natural exit always wins the race (§12.7 poll order), so a fast auth
/// failure stays `fail`, never `timeout`.
pub fn classify_status(ev: &FinishEvidence) -> (HostStatus, Option<&'static str>) {
    if ev.killed_by_timeout {
        return (HostStatus::Timeout, None);
    }
    if ev.killed_by_stop {
        return (HostStatus::Skipped, Some(R_STOPPED));
    }
    match ev.exit {
        Some(0) => (HostStatus::Ok, None),
        Some(_) => (HostStatus::Fail, None),
        None => (HostStatus::Fail, Some(R_SIGNAL)),
    }
}

pub enum Eligibility {
    Run,
    Skip(&'static str),
}

/// §12.4 #1 VERBATIM, precedence fixed in §12.3 row order (spawn-safety
/// first). Sidecar-alias OR-ing lives at the PLAN layer (finding #2), NOT
/// in this signature.
pub fn host_eligibility(alias: &str, snap: Option<&HostSnapshot>) -> Eligibility {
    if crate::sftp_safe(alias, "Alias").is_err() {
        return Eligibility::Skip(R_UNSAFE); // INV-19: same guard as INV-10 (sftp_safe reuse)
    }
    let Some(s) = snap else {
        return Eligibility::Skip(R_UNKNOWN);
    };
    if s.is_wildcard {
        return Eligibility::Skip(R_WILDCARD);
    }
    if s.auth_method == "password" || s.has_password {
        return Eligibility::Skip(R_PASSWORD);
    }
    Eligibility::Run
}

/// Read-only fleet view of one config host (§12.4 note). Built by
/// `snapshot_hosts` from CONFIG TEXT ONLY — never reads the sidecar
/// (INV-15), never writes anything (INV-16).
pub struct HostSnapshot {
    pub alias: String,
    pub auth_method: String,
    pub has_password: bool,
    pub is_wildcard: bool,
}

/// Pure wrapper over `crate::parse_config` with the migrate/self-heal write
/// branch REMOVED from the fleet call path (§12.4, INV-16; the normal
/// `read_ssh_config` command keeps its self-heal). auth detection mirrors
/// `read_ssh_config` exactly, minus the sidecar lookup.
pub fn snapshot_hosts(text: &str) -> Vec<HostSnapshot> {
    crate::parse_config(text)
        .hosts
        .iter()
        .map(|h| {
            let alias = h.patterns.first().cloned().unwrap_or_default();
            let prefers_password = crate::get_opt(h, "PreferredAuthentications")
                .split(',')
                .next()
                .map(|s| s.trim().eq_ignore_ascii_case("password"))
                .unwrap_or(false);
            let pubkey_off = crate::get_opt(h, "PubkeyAuthentication").eq_ignore_ascii_case("no");
            let auth_method = if prefers_password || pubkey_off { "password" } else { "key" }.to_string();
            // legacy `Password` directive only; sidecar presence is OR-ed at
            // plan level with keys-only data (finding #2).
            let has_password = !crate::get_opt(h, "Password").is_empty();
            let is_wildcard = alias.contains('*') || alias.contains('?');
            HostSnapshot { alias, auth_method, has_password, is_wildcard }
        })
        .collect()
}

/// TSD finding #2 / FR-FL21 / INV-15: presence of a sidecar entry, KEYS
/// ONLY. Reads lines, splits at the FIRST '=' (format `alias=value`,
/// lib.rs:233), keeps the alias, drops the value — a password string never
/// materializes in the fleet path. Reads NOTHING else, writes NOTHING.
pub fn sidecar_aliases_from_text(text: &str) -> HashSet<String> {
    let mut s = HashSet::new();
    for l in text.lines() {
        if let Some(i) = l.find('=') {
            s.insert(l[..i].to_string());
        }
    }
    s
}

pub fn sidecar_aliases() -> HashSet<String> {
    match std::fs::read_to_string(crate::secrets_path()) {
        Ok(t) => sidecar_aliases_from_text(&t),
        Err(_) => HashSet::new(), // missing/unreadable sidecar = no password hosts
    }
}

/// Authoritative re-plan (§12.3 rule 1): picker order preserved,
/// `host_eligibility` verbatim + sidecar-alias OR-ed into rule #4 here.
pub fn build_plan(
    aliases: &[String],
    snaps: &[HostSnapshot],
    sidecars: &HashSet<String>,
) -> Vec<FleetHostPlan> {
    aliases
        .iter()
        .map(|a| {
            let snap = snaps.iter().find(|s| &s.alias == a);
            let mut elig = host_eligibility(a, snap);
            if matches!(elig, Eligibility::Run) && sidecars.contains(a) {
                elig = Eligibility::Skip(R_PASSWORD);
            }
            match elig {
                Eligibility::Run => FleetHostPlan {
                    alias: a.clone(),
                    eligible: true,
                    status: PlanStatus::Queued,
                    reason: None,
                },
                Eligibility::Skip(r) => FleetHostPlan {
                    alias: a.clone(),
                    eligible: false,
                    status: PlanStatus::Skipped,
                    reason: Some(r),
                },
            }
        })
        .collect()
}

/// §12.4 #6 — the spawn's ONLY argv source (INV-18): BatchMode fixed,
/// command = one more argv element, script = `bash -s`. Nothing else is
/// EVER appended.
pub fn ssh_argv(bin: &str, alias: &str, t: &Transport) -> Vec<String> {
    let mut v = vec![bin.to_string(), "-o".to_string(), "BatchMode=yes".to_string(), alias.to_string()];
    match t {
        Transport::Command(text) => v.push(text.clone()),
        Transport::Script(_) => {
            v.push("bash".to_string());
            v.push("-s".to_string());
        }
    }
    v
}

/// §12.3 validation — reject-only, never clamp. §12.7: any failure here =
/// 0 spawn + 0 events (checked before state mutation and before any child).
pub fn validate(req: &FleetStartReq, active_run_id: Option<&str>) -> Result<(), &'static str> {
    // §12.3: "a run is active OR req.run_id == active run's id" — either way
    // some run is active. run_id itself is opaque and never parsed.
    if active_run_id.is_some() {
        return Err(E_ALREADY);
    }
    if req.aliases.is_empty() {
        return Err(E_EMPTY_SELECTION);
    }
    if req.text.trim().is_empty() {
        return Err(E_EMPTY_INPUT);
    }
    if req.text.len() > TEXT_MAX_BYTES {
        return Err(E_TEXT_TOO_LARGE);
    }
    if !(TIMEOUT_MIN..=TIMEOUT_MAX).contains(&req.timeout_secs) {
        return Err(E_BAD_TIMEOUT);
    }
    if !(CONC_MIN..=CONC_MAX).contains(&req.concurrency) {
        return Err(E_BAD_CONCURRENCY);
    }
    Ok(())
}

/// FR-FL32 presence probe (no spawn). With `HOSTERM_SSH_BIN` set the
/// override REPLACES the lookup entirely (ADR-16 / §12.5, sftp-hook parity
/// with lib.rs:721): the literal path must exist. Otherwise reuse the pure
/// `check_binaries` ssh flag — which never consults the override, so
/// `check_binaries` itself stays honest on test machines.
fn ssh_present() -> bool {
    if let Ok(bin) = std::env::var("HOSTERM_SSH_BIN") {
        return std::path::Path::new(&bin).is_file();
    }
    !crate::check_binaries().ssh_missing
}

// ---------- state (single-run lock, FR-FL31) ----------

pub struct ActiveRun {
    pub run_id: String,
    pub stop: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct FleetState {
    // None = no run active. Coordinator clears it through a Drop guard so a
    // panicking run never wedges the lock (§12.2 panic-safety).
    pub lock: Arc<Mutex<Option<ActiveRun>>>,
}

fn lock_recover(m: &Mutex<Option<ActiveRun>>) -> MutexGuard<'_, Option<ActiveRun>> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Clears FleetState.active on drop (panic-safe single-run release).
struct ActiveGuard {
    state: Arc<Mutex<Option<ActiveRun>>>,
    run_id: String,
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        let mut g = lock_recover(&self.state);
        if g.as_ref().map(|a| a.run_id == self.run_id).unwrap_or(false) {
            *g = None;
        }
    }
}

// ---------- runner (ADR-13/14) ----------

/// Event seam: (event_name, payload). Production wraps `app.emit` (§12.3
/// rule 2: delivery failure never kills a child or wedges state — errors
/// are swallowed here). Tests record, giving fixtures that replay the
/// frozen contract byte-for-byte without a webview.
type EventSink = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync + 'static>;

struct RunArgs {
    run_id: String,
    queued: Vec<String>,      // eligible aliases, plan order
    skipped_plan: u32,        // counted straight into the summary (no events)
    transport: Transport,
    timeout: Duration,        // per host, measured from spawn (FR-FL23)
    concurrency: usize,       // 1..=20, token channel capacity
    stop: Arc<AtomicBool>,
    sink: EventSink,
    state_lock: Arc<Mutex<Option<ActiveRun>>>,
}

struct HostOutcome {
    status: HostStatus,
    exit: Option<i32>,
}

fn emit(sink: &EventSink, name: &str, payload: &impl Serialize) {
    if let Ok(v) = serde_json::to_value(payload) {
        sink(name, v);
    }
}

/// Releases its token back to the channel on exit — including panic unwind
/// (§12.7: a spawn-failed host releases its permit to the queue).
struct PermitGuard(Option<Sender<()>>);
impl Drop for PermitGuard {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(());
        }
    }
}

/// §12.2 coordinator: walks the queued aliases, acquires a permit from the
/// N-token channel with `recv_timeout(50ms)` (Stop-aware while queued),
/// spawns one host task per started host, then aggregates + emits the
/// summary LAST (§12.3 rule 3).
fn coordinator(run: &Arc<RunArgs>) {
    let t0 = Instant::now();
    let (tx, rx) = mpsc_channel(run.concurrency);

    let mut handles: Vec<JoinHandle<HostOutcome>> = Vec::new();
    let mut unstarted = 0u32;
    for (i, alias) in run.queued.iter().enumerate() {
        if run.stop.load(Ordering::Acquire) {
            unstarted = (run.queued.len() - i) as u32;
            emit_stopped_remaining(run, i);
            break;
        }
        // acquire (stop-aware): ADR-14 — poll the token channel so Stop is
        // honored while queued.
        let acquired = loop {
            match rx.recv_timeout(POLL) {
                Ok(()) => break true,
                Err(RecvTimeoutError::Timeout) => {
                    if run.stop.load(Ordering::Acquire) {
                        break false;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break false,
            }
        };
        if !acquired {
            unstarted = (run.queued.len() - i) as u32;
            emit_stopped_remaining(run, i);
            break;
        }
        let alias = alias.clone();
        let tx2 = tx.clone();
        let run2 = Arc::clone(run);
        handles.push(std::thread::spawn(move || host_task(run2, alias, tx2)));
    }

    let mut s = FleetSummary {
        run_id: run.run_id.clone(),
        ok: 0,
        fail: 0,
        timeout: 0,
        // plan-skips + stop-killed unstarted queued hosts (§12.2/§12.7);
        // plan-skips are counted, never re-emitted as events.
        skipped: run.skipped_plan + unstarted,
        elapsed_ms: 0,
        min_exit: None,
        max_exit: None,
    };
    for h in handles {
        let outcome = match h.join() {
            Ok(o) => o,
            Err(_) => HostOutcome { status: HostStatus::Fail, exit: None }, // host task panicked
        };
        match outcome.status {
            HostStatus::Ok => s.ok += 1,
            HostStatus::Timeout => s.timeout += 1,
            HostStatus::Skipped => s.skipped += 1, // killed by stop
            HostStatus::Fail => {
                s.fail += 1;
                if let Some(n) = outcome.exit.filter(|e| *e != 0) {
                    s.min_exit = Some(s.min_exit.map_or(n, |m| m.min(n)));
                    s.max_exit = Some(s.max_exit.map_or(n, |m| m.max(n)));
                }
            }
        }
    }
    s.elapsed_ms = t0.elapsed().as_millis() as u64;
    emit(&run.sink, &format!("fleet://summary/{}", run.run_id), &s);
}

fn mpsc_channel(n: usize) -> (Sender<()>, std::sync::mpsc::Receiver<()>) {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    for _ in 0..n {
        let _ = tx.send(());
    }
    (tx, rx)
}

fn emit_stopped_remaining(run: &RunArgs, from: usize) {
    for alias in &run.queued[from..] {
        emit(
            &run.sink,
            &format!("fleet://host/{}", run.run_id),
            &FleetHostDone {
                alias: alias.clone(),
                status: HostStatus::Skipped,
                exit_code: None,
                reason: Some(R_STOPPED),
                stdout_bytes: 0,
                stderr_bytes: 0,
                truncated: false,
            },
        );
    }
}

#[derive(Default)]
struct HostCounters {
    out: AtomicU64,
    err: AtomicU64,
    seq: AtomicU32, // 0-based store; first chunk emits seq 1 (§12.3)
}

fn host_task(run: Arc<RunArgs>, alias: String, tx: Sender<()>) -> HostOutcome {
    let _permit = PermitGuard(Some(tx)); // token returns even on panic (§12.7)
    run_host(&run, &alias)
}

/// One host's full lifecycle (§12.2 host-task box): argv is built ONLY by
/// `ssh_argv`; poll precedence natural-exit > stop > timeout with a 50 ms
/// granularity; readers joined before the terminal host event so chunk→host
/// ordering per host is total.
fn run_host(run: &RunArgs, alias: &str) -> HostOutcome {
    let topic_host = format!("fleet://host/{}", run.run_id);
    // ADR-16/§12.5: HOSTERM_SSH_BIN read FRESH per spawn, fleet path only;
    // unset ⇒ plain "ssh".
    let bin = std::env::var("HOSTERM_SSH_BIN").unwrap_or_else(|_| "ssh".to_string());
    let argv = ssh_argv(&bin, alias, &run.transport);
    let mut cmd = std::process::Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let script_bytes: Option<Vec<u8>> = match &run.transport {
        // command mode: stdin is null — the text lives in argv (ADR-13)
        Transport::Command(_) => {
            cmd.stdin(std::process::Stdio::null());
            None
        }
        // script mode: normalized bytes piped to stdin, remote runs bash -s
        Transport::Script(s) => {
            cmd.stdin(std::process::Stdio::piped());
            Some(s.as_bytes().to_vec())
        }
    };
    // Windows parity with the proven sftp path (lib.rs:731): no console flash.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => {
            // §12.7: stable reason, NO raw spawn strings in any payload.
            emit(
                &run.sink,
                &topic_host,
                &FleetHostDone {
                    alias: alias.to_string(),
                    status: HostStatus::Fail,
                    exit_code: None,
                    reason: Some(R_SPAWN),
                    stdout_bytes: 0,
                    stderr_bytes: 0,
                    truncated: false,
                },
            );
            return HostOutcome { status: HostStatus::Fail, exit: None };
        }
    };

    let counters = Arc::new(HostCounters::default());
    let mut readers = Vec::new();
    if let Some(p) = child.stdout.take() {
        readers.push(spawn_reader(p, Stream::Stdout, alias.to_string(), run.run_id.clone(), Arc::clone(&run.sink), Arc::clone(&counters)));
    }
    if let Some(p) = child.stderr.take() {
        readers.push(spawn_reader(p, Stream::Stderr, alias.to_string(), run.run_id.clone(), Arc::clone(&run.sink), Arc::clone(&counters)));
    }
    let writer = script_bytes.and_then(|bytes| {
        child.stdin.take().map(|si| {
            std::thread::spawn(move || {
                // backpressure from a 2 MiB write must never stall the
                // deadline poll (ADR-14); write errors after a kill are
                // expected and ignored; drop closes stdin = EOF.
                let mut si = si;
                let _ = si.write_all(&bytes);
                drop(si);
            })
        })
    });

    // deadline measured FROM SPAWN — covers connect+exec (FR-FL23)
    let deadline = Instant::now() + run.timeout;
    let mut killed_by_timeout = false;
    let mut killed_by_stop = false;
    let mut natural: Option<std::process::ExitStatus> = None;
    loop {
        // precedence (frozen §12.2/§12.7): try_wait wins the race — a fast
        // auth failure is `fail`, never swallowed as timeout.
        match child.try_wait() {
            Ok(Some(st)) => {
                natural = Some(st);
                break;
            }
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break; // exit None → classify killed by signal
            }
        }
        if run.stop.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            killed_by_stop = true;
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            killed_by_timeout = true;
            break;
        }
        std::thread::sleep(POLL);
    }
    let exit = natural.map(|s| s.code()).flatten(); // post-kill code is meaningless → None (contract)

    for r in readers {
        let _ = r.join(); // exit/kill closes the pipes; readers hit EOF
    }
    if let Some(w) = writer {
        let _ = w.join();
    }

    let stdout_bytes = counters.out.load(Ordering::Relaxed);
    let stderr_bytes = counters.err.load(Ordering::Relaxed);
    let (status, reason) = classify_status(&FinishEvidence {
        killed_by_timeout,
        killed_by_stop,
        exit,
    });
    emit(
        &run.sink,
        &topic_host,
        &FleetHostDone {
            alias: alias.to_string(),
            status,
            exit_code: exit,
            reason,
            stdout_bytes,
            stderr_bytes,
            truncated: stdout_bytes + stderr_bytes > RETENTION_BYTES,
        },
    );
    HostOutcome { status, exit }
}

/// Reader thread (§12.6): one 8 KiB read → one chunk event → buffer dropped.
/// Backend memory is O(8 KiB) per host by construction; nothing buffers.
/// `seq` is one counter shared by BOTH readers of the host (monotonic
/// across streams, §12.3); `total_bytes` is the post-chunk observed sum.
fn spawn_reader<R: Read + Send + 'static>(
    mut pipe: R,
    stream: Stream,
    alias: String,
    run_id: String,
    sink: EventSink,
    counters: Arc<HostCounters>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf = [0u8; CHUNK_READ];
        let topic = format!("fleet://output/{}", run_id);
        loop {
            match pipe.read(&mut buf) {
                Ok(0) => break, // EOF: exit/kill closed the pipe
                Ok(n) => {
                    let own = if stream == Stream::Stdout { &counters.out } else { &counters.err };
                    own.fetch_add(n as u64, Ordering::Relaxed);
                    let seq = counters.seq.fetch_add(1, Ordering::AcqRel) + 1;
                    let total = counters.out.load(Ordering::Relaxed) + counters.err.load(Ordering::Relaxed);
                    emit(
                        &sink,
                        &topic,
                        &FleetOutputChunk {
                            alias: alias.clone(),
                            seq,
                            stream,
                            bytes: String::from_utf8_lossy(&buf[..n]).to_string(),
                            total_bytes: total,
                            truncated: total > RETENTION_BYTES,
                        },
                    );
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    })
}

// ---------- IPC commands (§12.3 — inventory 21 → 23) ----------

/// Everything `fleet_start` does except building the app-event sink, split
/// out so cargo tests exercise the SHIPPING path without a webview
/// (`sink` records instead of emitting).
fn start_inner(
    req: FleetStartReq,
    state_lock: &Arc<Mutex<Option<ActiveRun>>>,
    sink: EventSink,
) -> Result<FleetStartResp, String> {
    let t0 = Instant::now();
    let mut g = lock_recover(state_lock);
    // §12.7: every rejection below = 0 spawn + 0 event (no child, no emit).
    validate(&req, g.as_ref().map(|a| a.run_id.as_str()))?;
    if !ssh_present() {
        return Err(E_SSH_MISSING.to_string());
    }
    // read-only snapshot: parse ONLY — no migrate/self-heal write (INV-16;
    // `read_ssh_config` keeps its self-heal, this path never does).
    let text = crate::read_config_text(&crate::config_path())
        .map_err(|_| E_CONFIG_UNREADABLE.to_string())?;
    let snaps = snapshot_hosts(&text);
    let sidecars = sidecar_aliases();
    let hosts = build_plan(&req.aliases, &snaps, &sidecars);
    let queued: Vec<String> = hosts.iter().filter(|h| h.eligible).map(|h| h.alias.clone()).collect();
    let skipped_plan = (hosts.len() - queued.len()) as u32;

    if queued.is_empty() {
        // All-skipped fast path (§12.3): finished:true + complete summary
        // from the plan, ZERO events ever, and the active lock is NOT held.
        return Ok(FleetStartResp {
            run_id: req.run_id.clone(),
            hosts,
            finished: true,
            summary: Some(FleetSummary {
                run_id: req.run_id.clone(),
                ok: 0,
                fail: 0,
                timeout: 0,
                skipped: skipped_plan,
                elapsed_ms: t0.elapsed().as_millis() as u64,
                min_exit: None,
                max_exit: None,
            }),
        });
    }

    let stop = Arc::new(AtomicBool::new(false));
    *g = Some(ActiveRun {
        run_id: req.run_id.clone(),
        stop: Arc::clone(&stop),
    });
    drop(g); // never hold the state lock while threads run

    let run = Arc::new(RunArgs {
        run_id: req.run_id.clone(),
        queued,
        skipped_plan,
        transport: pick_transport(req.mode, &req.text),
        timeout: Duration::from_secs(req.timeout_secs as u64),
        concurrency: req.concurrency as usize,
        stop,
        sink,
        state_lock: Arc::clone(state_lock),
    });
    // coordinator owns the single-run lock through the Drop guard: a
    // panicking run can never wedge it (§12.2 panic-safety).
    std::thread::spawn(move || {
        let _guard = ActiveGuard {
            state: Arc::clone(&run.state_lock),
            run_id: run.run_id.clone(),
        };
        coordinator(&run);
    });
    Ok(FleetStartResp {
        run_id: req.run_id,
        hosts,
        finished: false,
        summary: None,
    })
}

#[tauri::command]
pub(crate) fn fleet_start(
    app: AppHandle,
    state: State<'_, FleetState>,
    req: FleetStartReq,
) -> Result<FleetStartResp, String> {
    // §12.3 rule 2: emit failure is harmless — keep counting, keep running.
    let sink: EventSink = {
        let app = app.clone();
        Arc::new(move |name: &str, payload: serde_json::Value| {
            let _ = app.emit(name, payload);
        })
    };
    start_inner(req, &state.lock, sink)
}

/// Idempotent: unknown/finished run_id → Ok no-op, so the panel-close Stop
/// can always fire (FR-FL37 / §12.3).
#[tauri::command]
pub(crate) fn fleet_stop(state: State<'_, FleetState>, run_id: String) -> Result<(), String> {
    let g = lock_recover(&state.lock);
    if let Some(a) = g.as_ref() {
        if a.run_id == run_id {
            a.stop.store(true, Ordering::Release);
        }
    }
    Ok(())
}

// ============================================================
//  TESTS (§12.9 Unit-1+2 mapping). No network, no real ssh:
//  integration tests drive the SHIPPING runner through
//  `start_inner` + fake-ssh shell scripts via HOSTERM_SSH_BIN
//  (§12.5). Env-touching tests serialize on crate::TEST_ENV_LOCK
//  and use HOME-fixture dirs (HOSTERM_CONFIG) — the dev machine's
//  real ~/.ssh is NEVER touched (hard rule STATE.md #6).
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: &str = concat!(
        "Host prod-web\n",
        "    HostName 203.0.113.1\n",
        "    User deploy\n",
        "    IdentityFile ~/.ssh/id_ed25519\n",
        "Host pw-host\n",
        "    User x\n",
        "    PreferredAuthentications password\n",
        "    PubkeyAuthentication no\n",
        "Host legacy-pw\n",
        "    User y\n",
        "    Password s3cret-directive\n",
        "Host db-core\n",
        "    User root\n",
        "Host *\n",
        "    ServerAliveInterval 60\n",
    );

    fn snaps() -> Vec<HostSnapshot> {
        snapshot_hosts(CFG)
    }

    fn elig(alias: &str) -> Eligibility {
        let s = snaps();
        host_eligibility(alias, s.iter().find(|h| h.alias == alias))
    }

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // §12.4 #1 — precedence matrix (§12.3 row order) + exact reason strings.
    #[test]
    fn host_eligibility_precedence_matrix() {
        // rule #1 dominates: unsafe + unknown → unsafe alias
        assert!(matches!(host_eligibility("-x", None), Eligibility::Skip(R_UNSAFE)));
        assert!(matches!(host_eligibility("bad\ralias", Some(&snaps()[3])), Eligibility::Skip(R_UNSAFE)));
        assert!(matches!(host_eligibility("bad\nalias", None), Eligibility::Skip(R_UNSAFE)));
        // rule #2: unknown wins over wildcard-looking alias
        assert!(matches!(elig("nope*"), Eligibility::Skip(R_UNKNOWN)));
        // rule #3: wildcard (never spawnable)
        assert!(matches!(elig("*"), Eligibility::Skip(R_WILDCARD)));
        // rule #4: password METHOD and legacy Password directive both skip
        assert!(matches!(elig("pw-host"), Eligibility::Skip(R_PASSWORD)));
        assert!(matches!(elig("legacy-pw"), Eligibility::Skip(R_PASSWORD)));
        // clean key hosts run
        assert!(matches!(elig("prod-web"), Eligibility::Run));
        assert!(matches!(elig("db-core"), Eligibility::Run));
        // frozen strings (§12.3)
        assert_eq!(R_UNSAFE, "unsafe alias");
        assert_eq!(R_UNKNOWN, "unknown alias");
        assert_eq!(R_WILDCARD, "wildcard pattern");
        assert_eq!(R_PASSWORD, "password host (not supported in v1)");
        assert_eq!(R_STOPPED, "stopped by user");
        assert_eq!(R_SPAWN, "spawn failed");
        assert_eq!(R_SIGNAL, "killed by signal");
    }

    // §12.4 #2 — finding #4 verbatim: command mode = text WITHOUT any \n.
    #[test]
    fn pick_transport_three_fixtures() {
        assert!(matches!(pick_transport(FleetMode::Command, "uptime"), Transport::Command(t) if t == "uptime"));
        assert!(matches!(pick_transport(FleetMode::Command, "echo a\nexit 0"), Transport::Script(_)));
        match pick_transport(FleetMode::Command, "uptime\r\n") {
            // CRLF single-liner contains \n → SCRIPT, then normalized
            Transport::Script(s) => assert_eq!(s, "uptime\n"),
            _ => panic!("CRLF input must classify as script (TSD finding #4)"),
        }
        assert!(matches!(pick_transport(FleetMode::Script, "uptime"), Transport::Script(t) if t == "uptime\n"));
    }

    // §12.4 #3 — the four frozen cases.
    #[test]
    fn normalize_script_four_cases() {
        assert_eq!(normalize_script("a\r\nb\r\n"), "a\nb\n"); // CRLF file
        assert_eq!(normalize_script("a\rb"), "a\nb\n"); // lone CR
        assert_eq!(normalize_script("no tail"), "no tail\n"); // final \n appended
        assert_eq!(normalize_script("a\n\nb\n"), "a\n\nb\n"); // ends-with-newline untouched; blank line is content
    }

    // §12.4 #4 — matrix incl. 255 and None; frozen precedence.
    #[test]
    fn classify_status_matrix() {
        let ev = |to: bool, st: bool, ex: Option<i32>| FinishEvidence { killed_by_timeout: to, killed_by_stop: st, exit: ex };
        assert_eq!(classify_status(&ev(true, false, None)).0, HostStatus::Timeout);
        assert_eq!(classify_status(&ev(true, false, None)).1, None, "timeout carries null reason");
        assert_eq!(classify_status(&ev(true, true, Some(0))).0, HostStatus::Timeout, "timeout first in precedence");
        assert_eq!(classify_status(&ev(false, true, Some(0))), (HostStatus::Skipped, Some(R_STOPPED)));
        assert_eq!(classify_status(&ev(false, false, Some(0))), (HostStatus::Ok, None));
        assert_eq!(classify_status(&ev(false, false, Some(255))), (HostStatus::Fail, None), "255 is fail, exit-code-honest");
        assert_eq!(classify_status(&ev(false, false, Some(2))), (HostStatus::Fail, None));
        assert_eq!(classify_status(&ev(false, false, None)), (HostStatus::Fail, Some(R_SIGNAL)));
    }

    // §12.4 #6 / INV-18 — the spawn's ONLY argv source, property over combos.
    #[test]
    fn ssh_argv_is_the_only_argv_source() {
        let cmd = ssh_argv("/usr/bin/ssh", "prod-web", &Transport::Command("uptime -p".into()));
        assert_eq!(cmd, vec!["/usr/bin/ssh", "-o", "BatchMode=yes", "prod-web", "uptime -p"]);
        let scr = ssh_argv("ssh", "db-core", &Transport::Script("echo hi\n".into()));
        assert_eq!(scr, vec!["ssh", "-o", "BatchMode=yes", "db-core", "bash", "-s"]);
        // frozen shape: exactly 5 (command) or 6 (script) — nothing is EVER appended
        for bin in ["ssh", "/opt/x/ssh.exe"] {
            for alias in ["a", "weird.alias-1", "x_9"] {
                for t in [
                    Transport::Command("echo".into()),
                    Transport::Command("".into()),
                    Transport::Script("s\n".into()),
                ] {
                    let v = ssh_argv(bin, alias, &t);
                    let n = if matches!(t, Transport::Script(_)) { 6 } else { 5 };
                    assert_eq!(v.len(), n);
                    assert_eq!(&v[0..4], [bin, "-o", "BatchMode=yes", alias]);
                    for arg in &v[..4] {
                        for forbidden in ["StrictHostKeyChecking", "UserKnownHostsFile", "-A", "-X", "-t", "sudo"] {
                            assert!(!arg.contains(forbidden), "INV-18: {forbidden} must never enter the fixed argv zone");
                        }
                    }
                }
            }
        }
    }

    // §12.4 note — snapshot is pure over config TEXT (no sidecar, no write).
    #[test]
    fn snapshot_hosts_parses_text_only() {
        let s = snaps();
        let names: Vec<&str> = s.iter().map(|h| h.alias.as_str()).collect();
        assert_eq!(names, ["prod-web", "pw-host", "legacy-pw", "db-core", "*"]);
        assert_eq!(s.iter().find(|h| h.alias == "pw-host").unwrap().auth_method, "password");
        let legacy = s.iter().find(|h| h.alias == "legacy-pw").unwrap();
        assert!(legacy.has_password, "legacy Password directive visible in text snapshot");
        assert_eq!(legacy.auth_method, "key", "method detection mirrors read_ssh_config");
        let key = s.iter().find(|h| h.alias == "prod-web").unwrap();
        assert_eq!(key.auth_method, "key");
        assert!(!key.has_password, "pure snapshot never consults the sidecar (INV-15)");
        assert!(s.iter().find(|h| h.alias == "*").unwrap().is_wildcard);
        assert!(!s.iter().find(|h| h.alias == "db-core").unwrap().is_wildcard);
    }

    // TSD finding #2 + INV-15: keys-only sidecar, OR-ed at PLAN layer.
    #[test]
    fn sidecar_keys_only_plan_or() {
        let set = sidecar_aliases_from_text("prod-web=hunter2\nweird==\nnoequalsline\nx=y");
        let want: HashSet<String> = ["prod-web", "weird", "x"].iter().map(|s| s.to_string()).collect();
        assert_eq!(set, want, "split at FIRST '='; value dropped; no-'=' lines ignored");
        let s = snaps();
        let plan = build_plan(&a(&["prod-web", "pw-host", "ghost", "db-core"]), &s, &set);
        assert!(!plan[0].eligible);
        assert_eq!(plan[0].reason, Some(R_PASSWORD), "key-auth host WITH sidecar entry is skipped (FR-FL21)");
        assert_eq!(plan[1].reason, Some(R_PASSWORD));
        assert_eq!(plan[2].reason, Some(R_UNKNOWN), "precedence preserved: unknown (#2) before password (#4)");
        assert_eq!(plan[3].reason, None);
        assert!(plan[3].eligible);
        // serialized plan carries ALIAS only — secret values never materialize
        let j = serde_json::to_string(&plan).unwrap();
        assert!(!j.contains("hunter2"), "INV-15: 0 secret bytes in fleet payloads");
    }

    // §12.3 rejection table — exact strings, reject-only (never clamp).
    #[test]
    fn validate_rejects_exact_fleet_prefixes() {
        assert_eq!(validate(&req("r-1", &["x"], FleetMode::Command, "uptime", 60, 5), None), Ok(()));
        assert_eq!(validate(&req("r-1", &["x"], FleetMode::Command, "uptime", 60, 5), Some("r-0")), Err(E_ALREADY));
        assert_eq!(validate(&req("r-1", &[], FleetMode::Command, "uptime", 60, 5), None), Err(E_EMPTY_SELECTION));
        assert_eq!(validate(&req("r-1", &["x"], FleetMode::Command, "  \t ", 60, 5), None), Err(E_EMPTY_INPUT));
        assert_eq!(validate(&req("r-1", &["x"], FleetMode::Command, &"x".repeat(TEXT_MAX_BYTES + 1), 60, 5), None), Err(E_TEXT_TOO_LARGE));
        assert_eq!(validate(&req("r-1", &["x"], FleetMode::Command, &"x".repeat(TEXT_MAX_BYTES), 60, 5), None), Ok(()), "cap inclusive at 2 MiB");
        for bad in [4, 0, -5, 601] {
            assert_eq!(validate(&req("r-1", &["x"], FleetMode::Command, "uptime", bad, 5), None), Err(E_BAD_TIMEOUT), "{bad} rejected, never clamped");
        }
        for bad in [0, -1, 21] {
            assert_eq!(validate(&req("r-1", &["x"], FleetMode::Command, "uptime", 60, bad), None), Err(E_BAD_CONCURRENCY));
        }
        assert_eq!(
            (E_ALREADY, E_EMPTY_SELECTION, E_EMPTY_INPUT, E_TEXT_TOO_LARGE, E_BAD_TIMEOUT, E_BAD_CONCURRENCY, E_SSH_MISSING, E_CONFIG_UNREADABLE),
            (
                "fleet:already-running",
                "fleet:empty-selection",
                "fleet:empty-input",
                "fleet:text-too-large",
                "fleet:bad-timeout",
                "fleet:bad-concurrency",
                "fleet:ssh-missing",
                "fleet:config-unreadable"
            )
        );
    }

    // §12.7: validation rejection = 0 spawn + 0 events + lock untouched.
    // (fails in validate() before any probe or child, so no env needed)
    #[test]
    fn validation_rejections_emit_zero_events() {
        let (sink, log) = recorder();
        let state = Arc::new(Mutex::new(None));
        let bad = [
            req("v-1", &[], FleetMode::Command, "uptime", 60, 5),
            req("v-2", &["h"], FleetMode::Command, " ", 60, 5),
            req("v-3", &["h"], FleetMode::Command, "uptime", 4, 5),
            req("v-4", &["h"], FleetMode::Command, "uptime", 60, 21),
        ];
        for r in bad {
            assert!(start_inner(r, &state, Arc::clone(&sink)).is_err());
        }
        assert_eq!(log.lock().unwrap().len(), 0, "rejections emit ZERO events");
        assert!(state.lock().unwrap().is_none(), "rejections never take the run lock");
    }

    fn req(run: &str, aliases: &[&str], mode: FleetMode, text: &str, timeout: i64, conc: i64) -> FleetStartReq {
        FleetStartReq {
            run_id: run.into(),
            mode,
            text: text.into(),
            aliases: aliases.iter().map(|s| s.to_string()).collect(),
            timeout_secs: timeout,
            concurrency: conc,
        }
    }

    type EventLog = Arc<Mutex<Vec<(String, serde_json::Value)>>>;

    fn recorder() -> (EventSink, EventLog) {
        let log: EventLog = Arc::new(Mutex::new(Vec::new()));
        let log2 = Arc::clone(&log);
        (
            Arc::new(move |name: &str, payload: serde_json::Value| {
                log2.lock().unwrap_or_else(|e| e.into_inner()).push((name.to_string(), payload));
            }),
            log,
        )
    }

    fn events(log: &EventLog, topic: &str) -> Vec<serde_json::Value> {
        log.lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(n, _)| n == topic)
            .map(|(_, p)| p.clone())
            .collect()
    }

    fn wait_event(log: &EventLog, topic: &str, secs: u64) -> Vec<serde_json::Value> {
        let t0 = Instant::now();
        loop {
            let v = events(log, topic);
            if !v.is_empty() {
                return v;
            }
            assert!(t0.elapsed() < Duration::from_secs(secs), "event {topic} never arrived");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    // ---- fake-ssh integration (Linux; §12.5 fixtures; env-serialized) ----

    #[cfg(unix)]
    mod fake_ssh {
        use super::*;
        use std::os::unix::fs::PermissionsExt;
        use std::path::PathBuf;

        pub(super) fn env_lock() -> MutexGuard<'static, ()> {
            crate::TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
        }

        fn tmp(tag: &str) -> PathBuf {
            std::env::temp_dir().join(format!("hosterm-fleet-{}-{}", std::process::id(), tag))
        }

        /// Writes `<tmp>/{config,hosterm.secrets?,fake-ssh}` and exports
        /// HOSTERM_CONFIG + HOSTERM_SSH_BIN. HOME fixture ONLY — the real
        /// ~/.ssh is never referenced. Teardown is panic-safe: the returned
        /// `Fixture` guard clears both env vars + removes the temp dir on
        /// Drop, so a failing assertion can never leak the fixture env into
        /// later tests (QA MINOR-3, t_dc94d129).
        pub(super) fn fixture(tag: &str, config: &str, secrets: Option<&str>, body: &str) -> Fixture {
            let d = tmp(tag);
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            let cfgp = d.join("config");
            std::fs::write(&cfgp, config).unwrap();
            if let Some(s) = secrets {
                std::fs::write(d.join("hosterm.secrets"), s).unwrap();
            }
            let fake = d.join("fake-ssh");
            std::fs::write(&fake, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::env::set_var("HOSTERM_CONFIG", &cfgp);
            std::env::set_var("HOSTERM_SSH_BIN", &fake);
            Fixture {
                _guard: FixtureGuard(tag.to_string()),
                fake,
                cfgp,
            }
        }

        /// Returned by `fixture`; Drop restores the env + removes the temp dir
        /// whether the test passes, fails, or panics (QA MINOR-3).
        pub(super) struct Fixture {
            _guard: FixtureGuard,
            pub fake: PathBuf,
            pub cfgp: PathBuf,
        }

        struct FixtureGuard(String);

        impl Drop for FixtureGuard {
            fn drop(&mut self) {
                std::env::remove_var("HOSTERM_SSH_BIN");
                std::env::remove_var("HOSTERM_CONFIG");
                let _ = std::fs::remove_dir_all(tmp(&self.0));
            }
        }

        pub(super) const CFG6: &str = concat!(
            "Host a1\n    HostName 10.0.0.1\n    User u\n",
            "Host a2\n    User u\n",
            "Host a3\n    User u\n",
            "Host a4\n    User u\n",
            "Host a5\n    User u\n",
            "Host a6\n    User u\n",
            "Host pw1\n    User u\n    PreferredAuthentications password\n",
        );

        /// Full shipping path: fixture → start_inner → run. run_id = tag.
        /// The returned `Fixture` guard MUST be bound by the test (third
        /// tuple element) so the fixture env + temp dir outlive the async
        /// coordinator spawns; Drop cleans up even on assertion failure.
        fn start(
            tag: &str,
            cfg: &str,
            secrets: Option<&str>,
            body: &str,
            aliases: &[&str],
            mode: FleetMode,
            text: &str,
            timeout: i64,
            conc: i64,
        ) -> (EventLog, Arc<Mutex<Option<ActiveRun>>>, Fixture) {
            let fx = fixture(tag, cfg, secrets, body);
            let (sink, log) = recorder();
            let state = Arc::new(Mutex::new(None));
            let resp = start_inner(req(tag, aliases, mode, text, timeout, conc), &state, sink).unwrap();
            assert!(!resp.finished, "fixture expected at least one queued host");
            (log, state, fx)
        }

        /// §12.9: concurrency ceiling asserted by CHILD-LIFETIME OVERLAP —
        /// the fake logs start/exit timestamps; no more than N may overlap.
        #[test]
        fn fleet_concurrency_ceiling_never_exceeds_n() {
            let _env = env_lock();
            let (log, _state, _fx) = start(
                "ceil",
                CFG6,
                None,
                "echo \"START $(date +%s%N)\"\nsleep 0.4\necho \"EXIT $(date +%s%N)\"\nexit 0",
                &["a1", "a2", "a3", "a4", "a5", "a6"],
                FleetMode::Command,
                "uptime",
                60,
                2,
            );
            let sum = wait_event(&log, "fleet://summary/ceil", 30);
            assert_eq!(sum[0]["ok"].as_u64(), Some(6));
            let mut starts: std::collections::HashMap<String, u128> = std::collections::HashMap::new();
            let mut intervals = Vec::new();
            for e in events(&log, "fleet://output/ceil") {
                let alias = e["alias"].as_str().unwrap().to_string();
                for line in e["bytes"].as_str().unwrap().lines() {
                    if let Some(t) = line.strip_prefix("START ") {
                        starts.insert(alias.clone(), t.trim().parse().unwrap());
                    } else if let Some(t) = line.strip_prefix("EXIT ") {
                        if let Some(s0) = starts.remove(&alias) {
                            intervals.push((s0, t.trim().parse::<u128>().unwrap()));
                        }
                    }
                }
            }
            assert_eq!(intervals.len(), 6, "all six hosts logged start+exit pairs");
            let mut pts = Vec::new();
            for (s, e) in intervals {
                pts.push((s, 1i32));
                pts.push((e, -1i32));
            }
            pts.sort_by(|x, y| x.0.cmp(&y.0).then(x.1.cmp(&y.1)));
            let (mut cur, mut maxc) = (0, 0);
            for (_, d) in pts {
                cur += d;
                maxc = maxc.max(cur);
            }
            assert!(maxc <= 2, "concurrency ceiling breached: overlap = {maxc} > N=2");
            assert!(maxc >= 2, "test vacuous: never had 2 overlapping children");
        }

        /// §12.9: timeout kill — fake sleeps 30 s vs the frozen 5 s floor;
        /// the run must end at the deadline with `timeout` + null exit.
        #[test]
        fn fleet_timeout_kills_child() {
            let _env = env_lock();
            let t0 = Instant::now();
            let (log, _state, _fx) = start(
                "tmo",
                CFG6,
                None,
                "echo MARK\nexec sleep 30\n",
                &["a1"],
                FleetMode::Command,
                "sleep 30",
                5,
                1,
            );
            let sum = wait_event(&log, "fleet://summary/tmo", 30);
            let elapsed = t0.elapsed();
            assert!(elapsed < Duration::from_secs(25), "deadline never fired: {elapsed:?}");
            let hosts = events(&log, "fleet://host/tmo");
            assert_eq!(hosts.len(), 1);
            assert_eq!(hosts[0]["status"].as_str(), Some("timeout"));
            assert!(hosts[0]["exit_code"].is_null(), "post-kill exit code is null per §12.3");
            assert!(hosts[0]["reason"].is_null(), "timeout carries null reason");
            assert_eq!(sum[0]["timeout"].as_u64(), Some(1));
        }

        /// exit 255 (connect/auth/host-key) is `fail` with the RAW code —
        /// never phrased as timeout (§12.7, OQ-2 posture).
        #[test]
        fn fleet_exit_255_is_fail_not_timeout() {
            let _env = env_lock();
            let (log, _state, _fx) = start(
                "e255",
                CFG6,
                None,
                "echo 'Host key verification failed.' >&2\nexit 255",
                &["a1"],
                FleetMode::Command,
                "uptime",
                60,
                1,
            );
            let hosts = wait_event(&log, "fleet://host/e255", 20);
            assert_eq!(hosts[0]["status"].as_str(), Some("fail"));
            assert_eq!(hosts[0]["exit_code"].as_i64(), Some(255));
            assert!(hosts[0]["reason"].is_null());
            assert!(hosts[0]["stderr_bytes"].as_u64().unwrap() >= 29);
            let sum = wait_event(&log, "fleet://summary/e255", 5);
            assert_eq!(sum[0]["fail"].as_u64(), Some(1));
            assert_eq!(sum[0]["min_exit"].as_i64(), Some(255));
            assert_eq!(sum[0]["max_exit"].as_i64(), Some(255));
        }

        /// FR-FL30 / §12.7: stop kills the running child (skipped,"stopped
        /// by user") AND marks unstarted queued hosts the same via the
        /// coordinator; exactly one summary; lock released after.
        #[test]
        fn fleet_stop_kills_and_marks_remaining_skipped() {
            let _env = env_lock();
            let (log, state, _fx) = start(
                "stp",
                CFG6,
                None,
                "echo HELD\nexec sleep 10\n",
                &["a1", "a2", "a3"],
                FleetMode::Command,
                "sleep 10",
                60,
                1,
            );
            std::thread::sleep(Duration::from_millis(400)); // a1 is definitely spawned
            {
                let g = state.lock().unwrap_or_else(|e| e.into_inner());
                g.as_ref().expect("run active while stopped").stop.store(true, Ordering::Release);
            }
            let sum = wait_event(&log, "fleet://summary/stp", 15);
            let hosts = events(&log, "fleet://host/stp");
            assert_eq!(hosts.len(), 3, "started-kill + 2 unstarted: one event each");
            for h in &hosts {
                assert_eq!(h["status"].as_str(), Some("skipped"));
                assert_eq!(h["reason"].as_str(), Some(R_STOPPED));
                assert!(h["exit_code"].is_null());
            }
            assert_eq!(sum[0]["skipped"].as_u64(), Some(3));
            let t0 = Instant::now();
            loop {
                if state.lock().unwrap_or_else(|e| e.into_inner()).is_none() {
                    break;
                }
                assert!(t0.elapsed() < Duration::from_secs(5), "ActiveGuard never cleared the lock");
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        /// §12.3 all-skipped fast path: finished:true + full summary in the
        /// RESPONSE, zero events EVER, run lock never held.
        #[test]
        fn fleet_finished_true_fast_path_zero_events() {
            let _env = env_lock();
            let _fx = fixture("fin", CFG6, None, "exit 0");
            let (sink, log) = recorder();
            let state = Arc::new(Mutex::new(None));
            let resp = start_inner(req("fin", &["pw1", "*"], FleetMode::Script, "echo hi", 60, 5), &state, Arc::clone(&sink)).unwrap();
            assert!(resp.finished);
            let s = resp.summary.expect("finished ⇒ summary present");
            assert_eq!(s.skipped, 2);
            assert_eq!((s.ok, s.fail, s.timeout), (0, 0, 0));
            assert!(s.min_exit.is_none() && s.max_exit.is_none());
            std::thread::sleep(Duration::from_millis(150)); // window for any stray thread
            assert_eq!(log.lock().unwrap().len(), 0, "finished:true = 0 events, ever");
            assert!(state.lock().unwrap().is_none(), "fast path never takes the lock");
            // …so the next start is NOT fleet:already-running
            let r2 = start_inner(req("fin2", &["a1"], FleetMode::Command, "uptime", 60, 5), &state, Arc::clone(&sink));
            assert!(r2.is_ok());
            wait_event(&log, "fleet://summary/fin2", 20);
        }

        /// INV-16/NFR-FL8 + finding #2 end-to-end: config & sidecar bytes,
        /// mtime and dir listing IDENTICAL across a multi-host run; the
        /// sidecar VALUE never appears in any fleet payload.
        #[test]
        fn fleet_writes_nothing_config_or_sidecar() {
            let _env = env_lock();
            let sec = "pw1=SEKRETFENCE-never-in-any-payload\n";
            let fx = fixture("nw", CFG6, Some(sec), "echo ran\nexit 0");
            let (fake, cfgp) = (fx.fake.clone(), fx.cfgp.clone());
            let secretp = fake.parent().unwrap().join("hosterm.secrets");
            let fingerprint = |p: &std::path::Path| -> (Vec<u8>, std::time::SystemTime, u64) {
                let m = std::fs::metadata(p).unwrap();
                (std::fs::read(p).unwrap(), m.modified().unwrap(), m.len())
            };
            let dir_entries = || -> Vec<String> {
                let mut v: Vec<String> = std::fs::read_dir(cfgp.parent().unwrap())
                    .unwrap()
                    .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().to_string()))
                    .collect();
                v.sort();
                v
            };
            let before_cfg = fingerprint(&cfgp);
            let before_sec = fingerprint(&secretp);
            let before_entries = dir_entries();
            let (sink, log) = recorder();
            let state = Arc::new(Mutex::new(None));
            let resp = start_inner(req("nw", &["a1", "a2", "pw1"], FleetMode::Command, "uptime", 60, 2), &state, sink).unwrap();
            assert_eq!(resp.hosts[2].reason, Some(R_PASSWORD), "sidecar presence skips via keys-only OR (finding #2)");
            wait_event(&log, "fleet://summary/nw", 20);
            assert_eq!(fingerprint(&cfgp), before_cfg, "INV-16: config bytes+mtime untouched");
            assert_eq!(fingerprint(&secretp), before_sec, "INV-16: sidecar bytes+mtime untouched");
            assert_eq!(dir_entries(), before_entries, "fleet created no new files");
            let dump = serde_json::to_string(&log.lock().unwrap_or_else(|e| e.into_inner()).clone()).unwrap();
            assert!(!dump.contains("SEKRETFENCE"), "INV-15: sidecar VALUE never crosses the fleet IPC surface");
        }

        /// NFR-FL3 cargo half (finding #5): 10 MiB stream through the real
        /// runner. Backend buffers nothing (O(8 KiB) by construction — code
        /// audit); counters + truncated flags asserted on the events.
        #[test]
        fn fleet_streams_10mib_with_honest_counters() {
            let _env = env_lock();
            let (log, _state, _fx) = start(
                "big",
                CFG6,
                None,
                "dd if=/dev/zero bs=8192 count=1280 2>/dev/null\nexit 0",
                &["a1"],
                FleetMode::Command,
                "cat",
                120,
                1,
            );
            let sum = wait_event(&log, "fleet://summary/big", 60);
            assert_eq!(sum[0]["ok"].as_u64(), Some(1));
            let chunks = events(&log, "fleet://output/big");
            assert!(!chunks.is_empty());
            let total = 10_485_760u64;
            for c in &chunks {
                assert!(c["bytes"].as_str().unwrap().len() <= CHUNK_READ, "chunk over 8 KiB");
            }
            let mut seqs: Vec<u64> = chunks.iter().map(|c| c["seq"].as_u64().unwrap()).collect();
            assert_eq!(seqs.iter().min(), Some(&1), "seq starts at 1");
            seqs.sort();
            seqs.dedup();
            assert_eq!(seqs.len(), chunks.len(), "seq monotonic per host (no dupes)");
            let last = chunks.last().unwrap();
            assert_eq!(last["total_bytes"].as_u64(), Some(total));
            assert_eq!(last["truncated"].as_bool(), Some(true), "FR-FL25: > 64 KiB sets truncated");
            assert_eq!(chunks[0]["truncated"].as_bool(), Some(false));
            let host = &events(&log, "fleet://host/big")[0];
            assert_eq!(host["stdout_bytes"].as_u64(), Some(total));
            assert_eq!(host["truncated"].as_bool(), Some(true));
        }

        /// Script mode end-to-end: argv is `... bash -s` (fake validates it),
        /// the NORMALIZED script arrives on stdin, stdin close = EOF.
        #[test]
        fn fleet_script_mode_pipes_stdin_with_frozen_argv() {
            let _env = env_lock();
            let (log, _state, _fx) = start(
                "scr",
                CFG6,
                None,
                "[ \"$4\" = bash ] && [ \"$5\" = -s ] || { echo BADARGV; exit 9; }\ncat\nexit 0",
                &["a1"],
                FleetMode::Script,
                "echo hi\r\necho bye",
                60,
                1,
            );
            let hosts = wait_event(&log, "fleet://host/scr", 20);
            assert_eq!(hosts[0]["status"].as_str(), Some("ok"), "fake validated bash -s argv (no BADARGV)");
            let body: String = events(&log, "fleet://output/scr")
                .iter()
                .map(|c| c["bytes"].as_str().unwrap().to_string())
                .collect();
            assert!(body.contains("echo hi\necho bye\n"), "CRLF normalized to LF and delivered via stdin; got: {body:?}");
            assert!(!body.contains("BADARGV"));
        }

        /// Guard call-site (INV-19 == sftp_safe reuse): unsafe aliases are
        /// plan-skipped and NEVER spawned; only the clean host gets events.
        #[test]
        fn fleet_guard_call_site_skips_unsafe_alias() {
            let _env = env_lock();
            let (log, _state, _fx) = start(
                "grd",
                CFG6,
                None,
                "echo ran\nexit 0",
                &["-evil", "a1", "bad\rhost"],
                FleetMode::Command,
                "uptime",
                60,
                2,
            );
            wait_event(&log, "fleet://summary/grd", 20);
            let hosts = events(&log, "fleet://host/grd");
            assert_eq!(hosts.len(), 1, "skipped rows never emit events; unsafe alias never spawned");
            assert_eq!(hosts[0]["alias"].as_str(), Some("a1"));
            assert!(
                events(&log, "fleet://output/grd").iter().all(|c| c["alias"].as_str() == Some("a1")),
                "no output chunks may carry a skipped (unsafe) alias"
            );
        }

        /// QA MINOR-1 (t_dc94d129): §12.3 summary invariant checked on ONE
        /// MIXED run — plan-skip + fail + ok in a single summary. The frozen
        /// contract demands ok+fail+timeout+skipped == |plan| (the FL-3 row
        /// renderer relies on it).
        #[test]
        fn fleet_summary_invariant_holds_on_mixed_run() {
            let _env = env_lock();
            // command-mode argv = [bin,-o,BatchMode=yes,ALIAS,text] → $3 is the alias
            let (log, _state, _fx) = start(
                "mix",
                CFG6,
                None,
                "[ \"$3\" = a2 ] && exit 2\necho \"ok $3\"\nexit 0",
                &["a1", "a2", "pw1", "ghost"],
                FleetMode::Command,
                "uptime",
                60,
                1,
            );
            let sum = wait_event(&log, "fleet://summary/mix", 20);
            let s = &sum[0];
            assert_eq!(s["ok"].as_u64(), Some(1));
            assert_eq!(s["fail"].as_u64(), Some(1));
            assert_eq!(s["timeout"].as_u64(), Some(0));
            assert_eq!(s["skipped"].as_u64(), Some(2), "pw1 (rule #4) + ghost (unknown) plan-skips counted");
            let total = ["ok", "fail", "timeout", "skipped"]
                .iter()
                .map(|k| s[*k].as_u64().unwrap())
                .sum::<u64>();
            assert_eq!(total, 4, "ok+fail+timeout+skipped == |plan|");
            // exactly the 2 STARTED hosts emit terminal events (plan-skips never do)
            let hosts = events(&log, "fleet://host/mix");
            assert_eq!(hosts.len(), 2);
            let a2 = hosts.iter().find(|h| h["alias"].as_str() == Some("a2")).unwrap();
            assert_eq!(a2["status"].as_str(), Some("fail"));
            assert_eq!(a2["exit_code"].as_i64(), Some(2));
            assert_eq!(s["min_exit"].as_i64(), Some(2));
            assert_eq!(s["max_exit"].as_i64(), Some(2));
        }

        /// QA MINOR-2 (t_dc94d129): spawn-failure arm — ssh exists (probe is
        /// mode-blind `is_file()`) but is NOT executable → one terminal host
        /// event `fail` + frozen reason "spawn failed", exit null, OS strings
        /// never leak (§12.7).
        #[test]
        fn fleet_spawn_failure_reports_frozen_reason() {
            let _env = env_lock();
            let fx = fixture("spf", CFG6, None, "exit 0");
            std::fs::set_permissions(&fx.fake, std::fs::Permissions::from_mode(0o644)).unwrap();
            let (sink, log) = recorder();
            let state = Arc::new(Mutex::new(None));
            let resp = start_inner(req("spf", &["a1"], FleetMode::Command, "uptime", 60, 1), &state, sink).unwrap();
            assert!(!resp.finished);
            let hosts = wait_event(&log, "fleet://host/spf", 10);
            assert_eq!(hosts.len(), 1);
            assert_eq!(hosts[0]["status"].as_str(), Some("fail"));
            assert!(hosts[0]["exit_code"].is_null());
            assert_eq!(hosts[0]["reason"].as_str(), Some(R_SPAWN));
            let sum = wait_event(&log, "fleet://summary/spf", 10);
            assert_eq!(sum[0]["fail"].as_u64(), Some(1));
            let dump = serde_json::to_string(&*log.lock().unwrap_or_else(|e| e.into_inner())).unwrap();
            assert!(!dump.contains("Permission denied"), "raw OS spawn strings never cross the IPC surface");
        }

        /// Rejection arms that need the env: dangling override + unreadable
        /// config — both 0 spawn 0 events.
        #[test]
        fn fleet_probe_and_config_rejections() {
            let _env = env_lock();
            let (sink, log) = recorder();
            let state = Arc::new(Mutex::new(None));
            // dangling HOSTERM_SSH_BIN → ssh-missing (override REPLACES the
            // lookup entirely, ADR-16 / sftp-hook parity)
            let d = fixture("prb", CFG6, None, "exit 0");
            std::env::set_var("HOSTERM_SSH_BIN", d.fake.parent().unwrap().join("no-such-bin"));
            let e = start_inner(req("prb", &["a1"], FleetMode::Command, "uptime", 60, 5), &state, Arc::clone(&sink)).err().unwrap();
            assert_eq!(e, E_SSH_MISSING);
            // override VALID again, but config path is a DIRECTORY → unreadable
            std::env::set_var("HOSTERM_SSH_BIN", &d.fake);
            std::env::set_var("HOSTERM_CONFIG", d.fake.parent().unwrap());
            let e = start_inner(req("prb", &["a1"], FleetMode::Command, "uptime", 60, 5), &state, Arc::clone(&sink)).err().unwrap();
            assert_eq!(e, E_CONFIG_UNREADABLE);
            assert_eq!(log.lock().unwrap().len(), 0, "rejections = 0 events");
            assert!(state.lock().unwrap().is_none());
        }
    }
}

