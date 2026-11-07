// ============================================================
//  backup.rs — backup/restore orchestration (BE-BACKUP owns)
// ============================================================
//
// Orchestrates the zero-knowledge Google Drive backup/restore feature per
// docs/tsd/gdrive-backup-v1.md §4,§5 and docs/prd/discovery-gdrive-backup-v1.md
// §C,§D. This module glues the three lower layers together WITHOUT adding any
// new business logic to lib.rs (which only gains four thin Tauri command
// wrappers, TSD §3):
//
//   BACKUP:  gather the four sources → build the JSON manifest →
//            `crypto::encode(passphrase)` → `drive::upload`.
//   RESTORE: `drive::download` → `crypto::decode(passphrase)` (abort on tag
//            failure, nothing touched) → PRE-RESTORE copy of every existing
//            target into `data_dir()/pre-restore-<UTC-compact>/` (abort if the
//            copy fails) → atomic write to the real paths (sibling temp →
//            chmod 0600 for secrets/keys/config → rename).
//
// The four backed-up sources (LOCKED, PRD §C):
//   1. ~/.ssh/config                 (super::config_path())
//   2. ~/.ssh/hosterm.secrets        (super::secrets_path())
//   3. settings.json                 (super::settings_path())
//   4. every SSH private key referenced by an IdentityFile in the parsed config
//
// SECURITY (audited HEAVY by SECURITY, TSD §3, PRD §D):
//   - passphrase is held in `Zeroizing<String>` from the IPC boundary inward,
//     never cloned into logs/errors, never persisted.
//   - ciphertext-at-rest: the bytes handed to Drive are the AEAD output of
//     `crypto::encode`; no host alias / key material appears in the blob.
//   - restore NEVER clobbers a real file before the pre-restore copy succeeds;
//     a wrong passphrase fails on the AEAD tag with zero writes (fail-safe).
//   - INV-1 preserved: config stays real-ssh-parseable, no secrets in config —
//     restore writes the user's own backed-up bytes verbatim, atomically.
//
// Drive I/O is injected through the `DriveBackend` trait so the integration
// tests drive a fully in-memory fake Drive (NFR-3: zero network in tests). The
// production path (`prod::*`) wires a real `drive::DriveClient` to the OAuth
// keyring refresh hook.

use std::collections::BTreeMap;
use std::path::PathBuf;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::crypto;

/// Manifest schema version (TSD §4 — also the cleartext Drive `htbk_schema`).
const SCHEMA: u32 = 1;

/// Length of the device data key (matches crypto KEY_LEN = 32).
const DEVICE_KEY_LEN: usize = 32;

// ---- Device-key store seam (OS keyring, v0.14.6) -------------------------
//
// Optional-passphrase backups encrypt with a random 32-byte device key held in
// the OS keyring under a DEDICATED entry, SEPARATE from the OAuth refresh_token
// (oauth.rs owns service "hosterm" / account "google-refresh-token"; the device
// backup key is service "hosterm" / account "backup-data-key"). The key is
// base64-encoded for keyring string storage and NEVER logged. The trait lets
// tests inject an in-memory mock (no real keyring); prod wraps `keyring::Entry`.

/// Keyring coordinates for the device backup key — distinct from OAuth's entry.
const DEVICE_KEY_SERVICE: &str = "hosterm";
const DEVICE_KEY_ACCOUNT: &str = "backup-data-key";

/// Abstracts read/write of the raw 32-byte device key. Production uses the OS
/// keyring; tests use an in-memory map so no real keyring is touched.
pub trait DeviceKeyStore {
    /// Read the stored device key, or `Ok(None)` if no entry exists.
    fn read_key(&self) -> Result<Option<Zeroizing<[u8; DEVICE_KEY_LEN]>>, String>;
    /// Persist the device key (overwrites any existing entry).
    fn store_key(&self, key: &[u8; DEVICE_KEY_LEN]) -> Result<(), String>;
}

/// Load the existing device key or generate+persist a fresh one. Used on the
/// backup path when the user supplies no passphrase.
fn load_or_create_device_key(
    store: &dyn DeviceKeyStore,
) -> Result<Zeroizing<[u8; DEVICE_KEY_LEN]>, String> {
    if let Some(k) = store.read_key()? {
        return Ok(k);
    }
    let key = crypto::random_device_key();
    store.store_key(&key)?;
    Ok(key)
}

/// Fixed permission for secrets / private keys / config on restore (0600).
#[cfg(unix)]
const RESTORE_MODE: u32 = 0o600;

// ---- DTOs returned across the IPC boundary (no secrets) ------------------

/// Result of a successful backup (returned to the FE for a toast).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupResult {
    pub file_id: String,
    pub name: String,
    pub bytes: u64,
    pub created_at: String,
}

/// One backup as listed from Drive's cleartext `appProperties` (no decrypt).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupMeta {
    pub file_id: String,
    pub name: String,
    pub created_at: String,
    pub host: String,
    pub schema: u32,
}

/// Result of a successful restore.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestoreResult {
    /// Absolute real paths that were written.
    pub restored: Vec<String>,
    /// The timestamped pre-restore backup directory.
    pub pre_restore_dir: String,
}

/// One known restore target + whether it currently exists on disk. Returned by
/// the preflight so the FE can warn about overwrites WITHOUT the passphrase
/// (the manifest is encrypted; this is the fixed known target set).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreflightTarget {
    pub path_role: String,
    pub path: String,
    pub exists: bool,
}

/// Preflight result: the fixed known target set + which currently exist.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestorePreflight {
    pub targets: Vec<PreflightTarget>,
}

// ---- Manifest (plaintext, pre-encryption — JSON, TSD §4) -----------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestFile {
    /// One of: ssh_config | secrets | settings | identity_key.
    pub path_role: String,
    /// Basename (config/secrets/settings) or the IdentityFile reference string
    /// as it appears in the config (for identity_key).
    pub rel: String,
    /// Unix mode to restore with (0600-class).
    pub mode: u32,
    /// Base64 (standard) of the file bytes.
    pub content_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub schema: u32,
    pub created_at: String,
    pub host: String,
    pub files: Vec<ManifestFile>,
}

// Path-role constants (stable strings baked into the manifest + preflight).
const ROLE_CONFIG: &str = "ssh_config";
const ROLE_SECRETS: &str = "secrets";
const ROLE_SETTINGS: &str = "settings";
const ROLE_IDENTITY: &str = "identity_key";

// ---- Drive backend seam --------------------------------------------------
//
// Abstracts the three Drive operations the orchestration needs. Production
// wraps `drive::DriveClient`; tests inject an in-memory fake (no network).

/// A listed backup as the backend reports it (cleartext facets only).
#[derive(Debug, Clone)]
pub struct RemoteBackup {
    pub file_id: String,
    pub name: String,
    pub created_at: String,
    pub host: String,
    pub schema: u32,
}

pub trait DriveBackend {
    /// Upload a `.htbk` blob with cleartext appProperties; return (file_id,
    /// name, byte count, created_at).
    fn upload(
        &mut self,
        name: &str,
        media: &[u8],
        schema: u32,
        created_at: &str,
        host: &str,
    ) -> Result<(String, String, u64, String), String>;

    /// List `.htbk` backups the app created (filtered by schema, no decrypt).
    fn list(&mut self, schema: u32) -> Result<Vec<RemoteBackup>, String>;

    /// Download a backup blob by file id (the encrypted `.htbk` bytes).
    fn download(&mut self, file_id: &str) -> Result<Vec<u8>, String>;
}

// ---- Gather: build the manifest from the four sources --------------------

/// Resolve the private-key paths referenced by `IdentityFile` directives in the
/// config text. Expands a leading `~/` against `home`, dedups, and keeps only
/// entries that resolve to an existing regular file (public keys, missing
/// files and directories are skipped — restore never invents a key).
fn resolve_identity_files(config_text: &str, home: &PathBuf) -> Vec<(String, PathBuf)> {
    let mut seen: BTreeMap<String, PathBuf> = BTreeMap::new();
    for raw in config_text.lines() {
        let trimmed = raw.trim_start();
        let mut it = trimmed.splitn(2, char::is_whitespace);
        let key = it.next().unwrap_or("");
        if !key.eq_ignore_ascii_case("IdentityFile") {
            continue;
        }
        let val = it.next().unwrap_or("").trim();
        if val.is_empty() {
            continue;
        }
        // The `rel` we store is the directive string verbatim (so restore can
        // map it back); the resolved path is where we read bytes from now.
        let resolved = expand_tilde(val, home);
        if resolved.is_file() {
            seen.entry(val.to_string()).or_insert(resolved);
        }
    }
    seen.into_iter().collect()
}

/// Expand a leading `~/` (or bare `~`) against `home`. Anything else is taken
/// as given (absolute or relative to CWD — ssh semantics are ~ and absolute).
fn expand_tilde(p: &str, home: &PathBuf) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        home.join(rest)
    } else if p == "~" {
        home.clone()
    } else {
        PathBuf::from(p)
    }
}

/// Local hostname for the manifest + appProperties (non-secret). Windows uses
/// COMPUTERNAME; Unix prefers /etc/hostname. Env override stays first for tests.
fn hostname() -> String {
    for key in ["HOSTERM_BACKUP_HOST", "COMPUTERNAME"] {
        if let Ok(h) = std::env::var(key) {
            let h = h.trim();
            if !h.is_empty() {
                return h.to_string();
            }
        }
    }
    if let Some(host) = std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return host;
    }
    std::env::var("HOSTNAME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown-host".to_string())
}

/// RFC 3339 UTC timestamp (seconds). Env override for deterministic tests.
fn utc_now_iso() -> String {
    if let Ok(t) = std::env::var("HOSTERM_BACKUP_NOW") {
        return t;
    }
    // Minimal UTC formatter from the system clock (no chrono dependency).
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_iso_utc(secs)
}

/// A compact UTC stamp (`YYYYMMDDTHHMMSSZ`) for the pre-restore dir name.
fn utc_compact() -> String {
    if let Ok(t) = std::env::var("HOSTERM_BACKUP_NOW") {
        // Strip separators from the ISO value so the dir name is filesystem-safe.
        return t.replace([':', '-'], "");
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_iso_utc(secs).replace([':', '-'], "")
}

/// Convert a Unix timestamp (UTC) into `YYYY-MM-DDTHH:MM:SSZ` without pulling a
/// date crate. Civil-from-days algorithm (Howard Hinnant).
fn format_iso_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // days since 1970-01-01 → civil date.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as i64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y, m, d, hh, mm, ss
    )
}

/// Build the plaintext manifest by gathering the four sources. Reuses the
/// lib.rs path fns for the real locations. `config` is always present in the
/// manifest only if the file exists; `secrets`/`settings` likewise; identity
/// keys are those referenced by the config that resolve to real files.
///
/// Returns an error only on an unexpected I/O failure reading a file that
/// exists (a MISSING source is simply omitted — it is normal for a fresh
/// install to lack a secrets sidecar or settings store).
pub fn build_manifest() -> Result<Manifest, String> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let mut files: Vec<ManifestFile> = Vec::new();

    let config_path = crate::config_path();
    let secrets_path = crate::secrets_path();
    let settings_path = crate::settings_path();

    // 1. ssh config (also the source of IdentityFile refs).
    let config_text = match std::fs::read_to_string(&config_path) {
        Ok(s) => Some(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("read config failed: {e}")),
    };
    if let Some(ref text) = config_text {
        files.push(ManifestFile {
            path_role: ROLE_CONFIG.to_string(),
            rel: "config".to_string(),
            mode: 0o600,
            content_b64: B64.encode(text.as_bytes()),
        });
    }

    // 2. secrets sidecar.
    push_file_if_present(&mut files, &secrets_path, ROLE_SECRETS, "hosterm.secrets")?;

    // 3. settings.json.
    push_file_if_present(&mut files, &settings_path, ROLE_SETTINGS, "settings.json")?;

    // 4. SSH private keys referenced by IdentityFile.
    if let Some(ref text) = config_text {
        for (rel, path) in resolve_identity_files(text, &home) {
            let bytes = std::fs::read(&path)
                .map_err(|e| format!("read identity file failed: {e}"))?;
            files.push(ManifestFile {
                path_role: ROLE_IDENTITY.to_string(),
                rel,
                mode: 0o600,
                content_b64: B64.encode(&bytes),
            });
        }
    }

    Ok(Manifest {
        schema: SCHEMA,
        created_at: utc_now_iso(),
        host: hostname(),
        files,
    })
}

/// Read `path` and push it into the manifest if it exists. A missing file is
/// skipped silently; any other read error propagates.
fn push_file_if_present(
    files: &mut Vec<ManifestFile>,
    path: &PathBuf,
    role: &str,
    rel: &str,
) -> Result<(), String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            files.push(ManifestFile {
                path_role: role.to_string(),
                rel: rel.to_string(),
                mode: 0o600,
                content_b64: B64.encode(&bytes),
            });
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("read {role} failed: {e}")),
    }
}

// ---- Target resolution (restore + preflight) -----------------------------

/// Resolve the real on-disk path a manifest entry restores to.
fn target_path(role: &str, rel: &str, home: &PathBuf) -> PathBuf {
    match role {
        ROLE_CONFIG => crate::config_path(),
        ROLE_SECRETS => crate::secrets_path(),
        ROLE_SETTINGS => crate::settings_path(),
        ROLE_IDENTITY => expand_tilde(rel, home),
        // Unknown role: refuse by resolving to an obviously-inert path the
        // caller will never write (callers validate roles before writing).
        _ => PathBuf::from("/dev/null"),
    }
}

/// The FIXED known preflight target set (no passphrase, manifest encrypted).
/// Covers the three well-known stores. IdentityFile key paths are intentionally
/// NOT listed here — they live inside the encrypted manifest and are unknowable
/// without the passphrase; the FE warns on the three fixed stores + a generic
/// "plus any backed-up SSH keys" note.
pub fn preflight() -> RestorePreflight {
    let fixed = [
        (ROLE_CONFIG, crate::config_path()),
        (ROLE_SECRETS, crate::secrets_path()),
        (ROLE_SETTINGS, crate::settings_path()),
    ];
    let targets = fixed
        .into_iter()
        .map(|(role, path)| PreflightTarget {
            path_role: role.to_string(),
            exists: path.is_file(),
            path: path.to_string_lossy().into_owned(),
        })
        .collect();
    RestorePreflight { targets }
}

// ---- Atomic write (sibling temp → chmod 0600 → rename) -------------------
//
// Mirrors the lib.rs atomic-write discipline (INV-1/2): a crash mid-write
// leaves the original intact. Secrets/keys/config land 0600. Parent dirs are
// created as needed (0700 for the data dir, matching the store layout).

fn atomic_write(path: &PathBuf, bytes: &[u8], mode: u32) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Tighten the parent dir for ~/.ssh-class material; ignore failures
            // on dirs we do not own (e.g. the shared config dir root).
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }
    let tmp = {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("tmp");
        let mut t = path.clone();
        t.set_file_name(format!(".{}.hosterm-tmp-{}", name, std::process::id()));
        t
    };
    std::fs::write(&tmp, bytes).map_err(|e| format!("temp write failed: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    let _ = mode;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("rename failed: {e}")
    })?;
    Ok(())
}

// ---- BACKUP orchestration ------------------------------------------------

/// Encrypt the manifest. Production ALWAYS uses `crypto::encode` at the OWASP
/// argon2 floor (64 MiB / t=3). The ONLY deviation is a test-gated env escape
/// hatch: when `HOSTERM_TEST_KDF` is set (set exclusively by the backup test
/// harness under env_lock), a low-cost KDF is used so the backup roundtrip
/// tests do not pay ~30s per derive. The variable is never set in production;
/// if absent, behaviour is byte-identical to `crypto::encode`.
fn encode_manifest(passphrase: &str, plaintext: Vec<u8>) -> Result<Vec<u8>, crypto::CryptoError> {
    if std::env::var_os("HOSTERM_TEST_KDF").is_some() {
        // 256 KiB / t=1 / p=1 — same low-cost params the crypto unit tests use.
        crypto::encode_random_with_params(passphrase, plaintext, 256, 1, 1)
    } else {
        crypto::encode(passphrase, plaintext)
    }
}

/// Gather → manifest → encrypt → upload. `passphrase` is held in Zeroizing and
/// never logged. The uploaded media is the AEAD output of the chosen key mode
/// (ciphertext-at-rest: no plaintext host alias / key material in the blob).
///
/// Key mode (v0.14.6): a NON-empty passphrase → user-passphrase mode (argon2id,
/// kdf_id=1, unchanged). An EMPTY passphrase → device-keyring mode (kdf_id=2):
/// a random 32-byte key is loaded-or-created in the OS keyring (via `store`)
/// and used directly. The `.htbk` is encrypted in BOTH cases — a plaintext
/// backup is never produced, and the device key is never logged.
pub fn run_backup(
    backend: &mut dyn DriveBackend,
    passphrase: Zeroizing<String>,
    store: &dyn DeviceKeyStore,
) -> Result<BackupResult, String> {
    let manifest = build_manifest()?;
    let created_at = manifest.created_at.clone();
    let host = manifest.host.clone();

    // Serialize the manifest, then wrap the plaintext in Zeroizing so it is
    // wiped after encode (encode also zeroizes its own copy).
    let plaintext = Zeroizing::new(
        serde_json::to_vec(&manifest).map_err(|_| "manifest serialize failed".to_string())?,
    );

    let blob = if passphrase.is_empty() {
        // Device-keyring mode: load-or-create the device key, encrypt with it.
        let key = load_or_create_device_key(store)?;
        crypto::encode_with_key(&key, plaintext.to_vec()).map_err(|e| e.to_string())?
    } else {
        encode_manifest(passphrase.as_str(), plaintext.to_vec()).map_err(|e| e.to_string())?
    };

    let name = backup_filename(&host, &created_at);
    let (file_id, name, bytes, ret_created) =
        backend.upload(&name, &blob, SCHEMA, &created_at, &host)?;

    Ok(BackupResult {
        file_id,
        name,
        bytes,
        // Prefer the backend's echoed createdTime when present, else our stamp.
        created_at: if ret_created.is_empty() {
            created_at
        } else {
            ret_created
        },
    })
}

/// Drive file name (NOT secret — it is the file name visible in the user's own
/// Drive). `hosterm-backup-<host>-<iso>.htbk`.
fn backup_filename(host: &str, created_at: &str) -> String {
    // Keep it filesystem/Drive friendly: no colons in the name.
    let safe_ts = created_at.replace(':', "");
    let safe_host: String = host
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let safe_host = safe_host.trim_matches('-');
    let safe_host = if safe_host.is_empty() {
        "unknown-host"
    } else {
        safe_host
    };
    format!("hosterm-backup-{safe_host}-{safe_ts}.htbk")
}

// ---- LIST orchestration --------------------------------------------------

/// List `.htbk` backups via cleartext appProperties (no decrypt). Results are
/// sorted newest-first (`created_at` descending) at this boundary so the FE
/// restore picker renders them in the right order without re-sorting. The
/// `created_at` values are RFC-3339 UTC (`YYYY-MM-DDTHH:MM:SSZ`) so lexical
/// descending == chronological descending; legacy-root backups (whose
/// `created_at` may fall back to Drive's createdTime) still sort and remain
/// listable. Per-file metadata (file_id, name, host, created_at, schema) is the
/// full set the picker needs; the caller passes a chosen `file_id` to restore.
pub fn run_list(backend: &mut dyn DriveBackend) -> Result<Vec<BackupMeta>, String> {
    let remotes = backend.list(SCHEMA)?;
    let mut metas: Vec<BackupMeta> = remotes
        .into_iter()
        .map(|r| BackupMeta {
            file_id: r.file_id,
            name: r.name,
            created_at: r.created_at,
            host: r.host,
            schema: r.schema,
        })
        .collect();
    // Newest first. Ties (or empty created_at) break by file_id for a stable,
    // deterministic order.
    metas.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.file_id.cmp(&b.file_id))
    });
    Ok(metas)
}

// ---- RESTORE orchestration (fail-safe, TSD §5) ---------------------------

/// Fail-safe restore. Order is mandatory:
///   1. download → decode (abort on tag failure — NOTHING touched).
///   2. compute real targets.
///   3. pre-restore copy of every EXISTING target into a timestamped dir
///      (abort before any write if the copy fails).
///   4. atomic write of each manifest file to its real path (0600 for
///      secrets/keys/config).
pub fn run_restore(
    backend: &mut dyn DriveBackend,
    file_id: &str,
    passphrase: Zeroizing<String>,
    store: &dyn DeviceKeyStore,
) -> Result<RestoreResult, String> {
    // 1. Download + decode. A wrong passphrase / tampered blob / missing device
    //    key fails here and returns BEFORE anything on disk is touched.
    let blob = backend.download(file_id)?;

    // Dispatch on the cleartext header key mode (v0.14.6): device-keyring
    // containers read the key from the OS keyring; user-passphrase containers
    // derive from the supplied passphrase exactly as before.
    let plaintext = match crypto::header_key_mode(&blob).map_err(|e| e.to_string())? {
        crypto::KeyMode::DeviceKeyring => {
            let key = store.read_key()?.ok_or_else(|| {
                "restore failed: this backup is encrypted with a device key stored in \
this computer's keyring, but no such key was found (it was likely created on a \
different machine). Restore it on the original computer, or use a \
passphrase-protected backup."
                    .to_string()
            })?;
            Zeroizing::new(crypto::decode_with_key(&key, &blob).map_err(|e| e.to_string())?)
        }
        crypto::KeyMode::UserPassphrase => {
            Zeroizing::new(crypto::decode(passphrase.as_str(), &blob).map_err(|e| e.to_string())?)
        }
    };
    let manifest: Manifest = serde_json::from_slice(plaintext.as_slice())
        .map_err(|_| "restore failed: manifest in backup is not valid".to_string())?;
    if manifest.schema != SCHEMA {
        return Err(format!(
            "restore failed: unsupported backup schema {}",
            manifest.schema
        ));
    }

    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));

    // 2. Resolve targets and decode file bytes up front (so a malformed entry
    //    aborts before we write or copy anything).
    struct Pending {
        target: PathBuf,
        bytes: Zeroizing<Vec<u8>>,
        mode: u32,
    }
    let mut pending: Vec<Pending> = Vec::with_capacity(manifest.files.len());
    for f in &manifest.files {
        match f.path_role.as_str() {
            ROLE_CONFIG | ROLE_SECRETS | ROLE_SETTINGS | ROLE_IDENTITY => {}
            other => return Err(format!("restore failed: unknown path_role '{other}'")),
        }
        let target = target_path(&f.path_role, &f.rel, &home);
        let bytes = Zeroizing::new(
            B64.decode(f.content_b64.as_bytes())
                .map_err(|_| "restore failed: corrupt file content in backup".to_string())?,
        );
        // 0600 for every sensitive role; honor the stored mode but floor it to
        // 0600-class so we never restore a world-readable key.
        let mode = restore_mode(f.mode);
        pending.push(Pending {
            target,
            bytes,
            mode,
        });
    }

    // 3. PRE-RESTORE backup of every existing target. Abort the WHOLE restore
    //    if any copy fails — we must never clobber without a safety copy.
    let pre_dir = crate::data_dir().join(format!("pre-restore-{}", utc_compact()));
    std::fs::create_dir_all(&pre_dir)
        .map_err(|e| format!("restore aborted: could not create pre-restore dir: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&pre_dir, std::fs::Permissions::from_mode(0o700));
    }
    let mut copied_any = false;
    for p in &pending {
        if p.target.is_file() {
            let existing = std::fs::read(&p.target)
                .map_err(|e| format!("restore aborted: could not read target for pre-restore: {e}"))?;
            // Flatten into the pre-restore dir by basename; disambiguate
            // collisions (e.g. two keys with the same basename) with an index.
            let dest = unique_in_dir(&pre_dir, &p.target);
            atomic_write(&dest, &existing, 0o600)
                .map_err(|e| format!("restore aborted: pre-restore copy failed: {e}"))?;
            copied_any = true;
        }
    }
    let _ = copied_any; // empty-target restore is valid (fresh machine).

    // 4. Write the real paths atomically. The pre-restore copy already
    //    succeeded, so an overwrite here is recoverable from `pre_dir`.
    let mut restored: Vec<String> = Vec::with_capacity(pending.len());
    for p in &pending {
        atomic_write(&p.target, p.bytes.as_slice(), p.mode)?;
        restored.push(p.target.to_string_lossy().into_owned());
    }

    Ok(RestoreResult {
        restored,
        pre_restore_dir: pre_dir.to_string_lossy().into_owned(),
    })
}

/// Floor a stored mode to 0600-class (owner rw only) so restore never widens
/// perms on a private key. On non-unix the mode is ignored by atomic_write.
fn restore_mode(_stored: u32) -> u32 {
    #[cfg(unix)]
    {
        RESTORE_MODE
    }
    #[cfg(not(unix))]
    {
        0o600
    }
}

/// Pick a non-colliding path inside `dir` for the basename of `src`.
fn unique_in_dir(dir: &PathBuf, src: &PathBuf) -> PathBuf {
    let base = src
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    let mut candidate = dir.join(&base);
    let mut n = 1;
    while candidate.exists() {
        candidate = dir.join(format!("{base}.{n}"));
        n += 1;
    }
    candidate
}

// ==========================================================================
//  Production wiring — real DriveClient over the OAuth keyring refresh hook.
// ==========================================================================

/// Adapts `drive::DriveClient` to the `DriveBackend` seam, driving the real
/// Google Drive REST client. Built from the OAuth keyring: an initial access
/// token obtained by refreshing the stored refresh_token, plus a refresh hook
/// the Drive 401 path can call once.
pub mod prod {
    use super::*;
    use crate::drive::{DriveClient, ReqwestDriveHttp};
    use crate::oauth;

    /// Build a DriveClient from the keyring, run `f`, and map results into the
    /// IPC DTOs. Returns a re-auth error if no refresh_token is stored.
    fn with_client<T>(
        f: impl FnOnce(&mut dyn DriveBackend) -> Result<T, String>,
    ) -> Result<T, String> {
        let refresh = oauth::keyring_read_refresh()?
            .ok_or_else(|| "not signed in to Google — sign in first".to_string())?;
        let http = oauth::ReqwestHttp;
        // Initial access token: refresh once now so the Drive client starts
        // with a valid bearer. A rotated refresh_token is persisted.
        let (access_token, rotated) = oauth::refresh_access_token(&http, &refresh)?;
        if let Some(new_rt) = rotated {
            oauth::keyring_store_refresh(&new_rt)?;
        }

        let drive_http = ReqwestDriveHttp;
        // The Drive 401 hook refreshes again from the keyring (the token may
        // have expired between calls) and persists any rotation.
        let refresh_hook = Box::new(move || -> Result<String, String> {
            let rt = oauth::keyring_read_refresh()?
                .ok_or_else(|| "not signed in to Google".to_string())?;
            let h = oauth::ReqwestHttp;
            let (at, rotated) = oauth::refresh_access_token(&h, &rt)?;
            if let Some(new_rt) = rotated {
                oauth::keyring_store_refresh(&new_rt)?;
            }
            Ok(at)
        });
        let client = DriveClient::new(&drive_http, access_token, refresh_hook);
        let mut backend = ClientBackend { client };
        f(&mut backend)
    }

    /// Wraps a live DriveClient as a DriveBackend.
    struct ClientBackend<'a> {
        client: DriveClient<'a>,
    }

    impl<'a> DriveBackend for ClientBackend<'a> {
        fn upload(
            &mut self,
            name: &str,
            media: &[u8],
            schema: u32,
            created_at: &str,
            host: &str,
        ) -> Result<(String, String, u64, String), String> {
            let props = crate::drive::build_app_properties(schema, created_at, host);
            let folder_id = self.client.ensure_backup_folder()?;
            let file = self
                .client
                .upload_in_folder(name, media, &props, &folder_id)?;
            let bytes = file
                .size
                .as_deref()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(media.len() as u64);
            let ct = file.created_time.clone().unwrap_or_default();
            Ok((file.id, file.name, bytes, ct))
        }

        fn list(&mut self, schema: u32) -> Result<Vec<RemoteBackup>, String> {
            let files = self.client.list_backups(schema)?;
            Ok(files
                .into_iter()
                .map(|f| {
                    let props = f.app_properties.unwrap_or_default();
                    let created_at = props
                        .get("created_at")
                        .cloned()
                        .or_else(|| f.created_time.clone())
                        .unwrap_or_default();
                    let host = props.get("host").cloned().unwrap_or_default();
                    let schema = props
                        .get("htbk_schema")
                        .and_then(|s| s.parse::<u32>().ok())
                        .unwrap_or(0);
                    RemoteBackup {
                        file_id: f.id,
                        name: f.name,
                        created_at,
                        host,
                        schema,
                    }
                })
                .collect())
        }

        fn download(&mut self, file_id: &str) -> Result<Vec<u8>, String> {
            self.client.download(file_id)
        }
    }

    /// Production device-key store backed by the OS keyring. The 32-byte key is
    /// stored base64-encoded (keyring entries are strings) under a DEDICATED
    /// entry, separate from the OAuth refresh_token. The key never appears in
    /// logs or errors.
    pub struct KeyringDeviceKeyStore;

    impl DeviceKeyStore for KeyringDeviceKeyStore {
        fn read_key(&self) -> Result<Option<Zeroizing<[u8; DEVICE_KEY_LEN]>>, String> {
            let entry = keyring::Entry::new(DEVICE_KEY_SERVICE, DEVICE_KEY_ACCOUNT)
                .map_err(|e| format!("keyring open failed: {e}"))?;
            match entry.get_password() {
                Ok(b64) => {
                    let raw = B64
                        .decode(b64.trim().as_bytes())
                        .map_err(|_| "device key in keyring is corrupt".to_string())?;
                    if raw.len() != DEVICE_KEY_LEN {
                        return Err("device key in keyring has wrong length".to_string());
                    }
                    let mut key = Zeroizing::new([0u8; DEVICE_KEY_LEN]);
                    key.copy_from_slice(&raw);
                    Ok(Some(key))
                }
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(e) => Err(format!("keyring read failed: {e}")),
            }
        }

        fn store_key(&self, key: &[u8; DEVICE_KEY_LEN]) -> Result<(), String> {
            let entry = keyring::Entry::new(DEVICE_KEY_SERVICE, DEVICE_KEY_ACCOUNT)
                .map_err(|e| format!("keyring open failed: {e}"))?;
            let b64 = B64.encode(key);
            entry
                .set_password(&b64)
                .map_err(|e| format!("keyring store failed: {e}"))
        }
    }

    /// `drive_backup` IPC body. An empty passphrase selects device-keyring mode.
    pub fn backup(passphrase: Zeroizing<String>) -> Result<BackupResult, String> {
        let store = KeyringDeviceKeyStore;
        with_client(|b| run_backup(b, passphrase, &store))
    }

    /// `drive_list_backups` IPC body.
    pub fn list_backups() -> Result<Vec<BackupMeta>, String> {
        with_client(run_list)
    }

    /// `drive_restore` IPC body.
    pub fn restore(
        file_id: String,
        passphrase: Zeroizing<String>,
    ) -> Result<RestoreResult, String> {
        let store = KeyringDeviceKeyStore;
        with_client(|b| run_restore(b, &file_id, passphrase, &store))
    }
}

// ==========================================================================
//  TESTS — in-memory fake Drive + env-overridden HOME/CONFIG/DATA_DIR.
//  No network, no real keyring. (NFR-3: zero network in tests.)
// ==========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::MutexGuard;

    // HOSTERM_CONFIG / HOME / HOSTERM_* are process-global env vars also mutated
    // by lib.rs tests. A per-module lock does NOT serialize across modules, so we
    // MUST share the one crate-wide mutex (same as fleet.rs) or a concurrent
    // lib.rs test can repoint HOSTERM_CONFIG mid-backup and break the roundtrip.
    // Poison-tolerant: a prior panic while holding it is harmless to recover from
    // since the lock only serializes env mutation.
    fn env_lock() -> MutexGuard<'static, ()> {
        let g = crate::TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Backup roundtrip tests drive the real run_backup/run_restore path,
        // which encrypts with crypto::encode (OWASP 64 MiB argon2 ~30s/derive).
        // Flag the test-only low-cost KDF so the suite stays fast; never set in
        // production. Scoped to the test process, serialized by this lock.
        std::env::set_var("HOSTERM_TEST_KDF", "1");
        g
    }

    /// A unique scratch sandbox for one test: HOME, CONFIG, DATA_DIR all point
    /// inside it via env overrides.
    struct Sandbox {
        root: PathBuf,
    }

    impl Sandbox {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "hosterm-backup-test-{}-{}-{:?}",
                tag,
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            // ~/.ssh lives under HOME; config + secrets resolve from CONFIG.
            std::fs::create_dir_all(root.join(".ssh").join("keys")).unwrap();
            std::env::set_var("HOSTERM_CONFIG", root.join(".ssh").join("config"));
            std::env::set_var("HOSTERM_DATA_DIR", root.join("data"));
            std::env::set_var("HOME", &root);
            std::env::set_var("HOSTERM_BACKUP_HOST", "test-box");
            std::env::set_var("HOSTERM_BACKUP_NOW", "2026-11-04T00:00:00Z");
            Sandbox { root }
        }
        fn ssh(&self, name: &str) -> PathBuf {
            self.root.join(".ssh").join(name)
        }
        fn data(&self) -> PathBuf {
            self.root.join("data").join("hosterm")
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            std::env::remove_var("HOSTERM_CONFIG");
            std::env::remove_var("HOSTERM_DATA_DIR");
            std::env::remove_var("HOSTERM_BACKUP_HOST");
            std::env::remove_var("HOSTERM_BACKUP_NOW");
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// In-memory fake Drive: upload stores the blob under a generated id; list
    /// echoes them back with their appProperties; download returns the stored
    /// bytes. Entirely offline.
    #[derive(Default)]
    struct FakeDrive {
        store: RefCell<Vec<(String, Vec<u8>, u32, String, String, String)>>, // id, blob, schema, created, host, name
        next_id: RefCell<u32>,
    }

    impl DriveBackend for FakeDrive {
        fn upload(
            &mut self,
            name: &str,
            media: &[u8],
            schema: u32,
            created_at: &str,
            host: &str,
        ) -> Result<(String, String, u64, String), String> {
            let mut n = self.next_id.borrow_mut();
            *n += 1;
            let id = format!("FAKE{n}");
            self.store.borrow_mut().push((
                id.clone(),
                media.to_vec(),
                schema,
                created_at.to_string(),
                host.to_string(),
                name.to_string(),
            ));
            Ok((id, name.to_string(), media.len() as u64, created_at.to_string()))
        }
        fn list(&mut self, schema: u32) -> Result<Vec<RemoteBackup>, String> {
            Ok(self
                .store
                .borrow()
                .iter()
                .filter(|e| e.2 == schema)
                .map(|e| RemoteBackup {
                    file_id: e.0.clone(),
                    name: e.5.clone(),
                    created_at: e.3.clone(),
                    host: e.4.clone(),
                    schema: e.2,
                })
                .collect())
        }
        fn download(&mut self, file_id: &str) -> Result<Vec<u8>, String> {
            self.store
                .borrow()
                .iter()
                .find(|e| e.0 == file_id)
                .map(|e| e.1.clone())
                .ok_or_else(|| "fake drive: no such file".to_string())
        }
    }

    /// In-memory device-key store for tests (no real OS keyring). Mirrors the
    /// keyring's load-or-create and missing-entry semantics.
    #[derive(Default)]
    struct MockKeyStore {
        key: RefCell<Option<[u8; DEVICE_KEY_LEN]>>,
    }

    impl MockKeyStore {
        /// A store that already holds no key (fresh machine).
        fn empty() -> Self {
            Self::default()
        }
    }

    impl DeviceKeyStore for MockKeyStore {
        fn read_key(&self) -> Result<Option<Zeroizing<[u8; DEVICE_KEY_LEN]>>, String> {
            Ok(self.key.borrow().map(Zeroizing::new))
        }
        fn store_key(&self, key: &[u8; DEVICE_KEY_LEN]) -> Result<(), String> {
            *self.key.borrow_mut() = Some(*key);
            Ok(())
        }
    }

    const PASS: &str = "correct horse battery staple";

    fn seed_sources(sb: &Sandbox) {
        // config with a real IdentityFile pointing at a key we create.
        let key_path = sb.ssh("keys/id_test");
        std::fs::write(&key_path, b"-----BEGIN OPENSSH PRIVATE KEY-----\nSECRETKEYBYTES\n").unwrap();
        let cfg = format!(
            "# my hosts\nHost prod-secret-alias\n    HostName 203.0.113.9\n    User root\n    IdentityFile {}\n",
            key_path.display()
        );
        std::fs::write(sb.ssh("config"), cfg).unwrap();
        std::fs::write(sb.ssh("hosterm.secrets"), b"prod-secret-alias=hunter2\n").unwrap();
        // settings.json under data dir.
        std::fs::create_dir_all(sb.data()).unwrap();
        std::fs::write(sb.data().join("settings.json"), br#"{"version":1,"theme":"dusk"}"#).unwrap();
    }

    #[test]
    fn windows_computername_is_used_when_no_test_override_exists() {
        let _g = env_lock();
        let old_override = std::env::var_os("HOSTERM_BACKUP_HOST");
        let old_computername = std::env::var_os("COMPUTERNAME");
        std::env::remove_var("HOSTERM_BACKUP_HOST");
        std::env::set_var("COMPUTERNAME", "MEKL-PC");
        assert_eq!(hostname(), "MEKL-PC");
        match old_override {
            Some(v) => std::env::set_var("HOSTERM_BACKUP_HOST", v),
            None => std::env::remove_var("HOSTERM_BACKUP_HOST"),
        }
        match old_computername {
            Some(v) => std::env::set_var("COMPUTERNAME", v),
            None => std::env::remove_var("COMPUTERNAME"),
        }
    }

    #[test]
    fn backup_filename_sanitizes_host_component() {
        assert_eq!(
            backup_filename("MEKL PC/Desk", "2026-11-07T05:05:21Z"),
            "hosterm-backup-MEKL-PC-Desk-2026-11-07T050521Z.htbk"
        );
    }

    #[test]
    fn backup_restore_roundtrip_equality() {
        let _g = env_lock();
        let sb = Sandbox::new("roundtrip");
        seed_sources(&sb);

        // Capture original bytes.
        let orig_cfg = std::fs::read(sb.ssh("config")).unwrap();
        let orig_sec = std::fs::read(sb.ssh("hosterm.secrets")).unwrap();
        let orig_set = std::fs::read(sb.data().join("settings.json")).unwrap();
        let orig_key = std::fs::read(sb.ssh("keys/id_test")).unwrap();

        let mut drive = FakeDrive::default();
        let ks = MockKeyStore::empty();
        let res = run_backup(&mut drive, Zeroizing::new(PASS.to_string()), &ks).unwrap();
        assert!(res.bytes > 0, "backup produced bytes");
        assert!(res.name.contains("test-box"), "filename carries host: {}", res.name);

        // Mutate the real files so restore must actually overwrite them.
        std::fs::write(sb.ssh("config"), b"Host CLOBBERED\n").unwrap();
        std::fs::write(sb.ssh("hosterm.secrets"), b"x=y\n").unwrap();
        std::fs::write(sb.data().join("settings.json"), b"{}").unwrap();
        std::fs::write(sb.ssh("keys/id_test"), b"CLOBBERED").unwrap();

        let rr = run_restore(&mut drive, &res.file_id, Zeroizing::new(PASS.to_string()), &ks).unwrap();
        assert!(!rr.restored.is_empty());
        assert!(PathBuf::from(&rr.pre_restore_dir).is_dir(), "pre-restore dir exists");

        // Byte-for-byte equality after restore.
        assert_eq!(std::fs::read(sb.ssh("config")).unwrap(), orig_cfg, "config restored");
        assert_eq!(std::fs::read(sb.ssh("hosterm.secrets")).unwrap(), orig_sec, "secrets restored");
        assert_eq!(std::fs::read(sb.data().join("settings.json")).unwrap(), orig_set, "settings restored");
        assert_eq!(std::fs::read(sb.ssh("keys/id_test")).unwrap(), orig_key, "identity key restored");
    }

    #[test]
    fn wrong_passphrase_no_partial_write() {
        let _g = env_lock();
        let sb = Sandbox::new("wrongpass");
        seed_sources(&sb);

        let mut drive = FakeDrive::default();
        let ks = MockKeyStore::empty();
        let res = run_backup(&mut drive, Zeroizing::new(PASS.to_string()), &ks).unwrap();

        // Clobber targets, snapshot the clobbered state.
        std::fs::write(sb.ssh("config"), b"Host UNTOUCHED\n").unwrap();
        let clobbered_cfg = std::fs::read(sb.ssh("config")).unwrap();
        std::fs::write(sb.ssh("hosterm.secrets"), b"keep=this\n").unwrap();
        let clobbered_sec = std::fs::read(sb.ssh("hosterm.secrets")).unwrap();

        // Count pre-restore dirs before the failed attempt.
        let pre_before = count_pre_restore(&sb);

        let err = run_restore(&mut drive, &res.file_id, Zeroizing::new("WRONG".to_string()), &ks)
            .unwrap_err();
        assert!(
            err.contains("wrong passphrase") || err.contains("corrupt"),
            "fail-safe message: {err}"
        );

        // NOTHING was written: targets unchanged AND no pre-restore dir created.
        assert_eq!(std::fs::read(sb.ssh("config")).unwrap(), clobbered_cfg, "config untouched on bad pass");
        assert_eq!(std::fs::read(sb.ssh("hosterm.secrets")).unwrap(), clobbered_sec, "secrets untouched on bad pass");
        assert_eq!(count_pre_restore(&sb), pre_before, "no pre-restore dir on bad pass (abort before step 3)");
    }

    #[test]
    fn pre_restore_dir_created_and_populated() {
        let _g = env_lock();
        let sb = Sandbox::new("prerestore");
        seed_sources(&sb);

        let mut drive = FakeDrive::default();
        let ks = MockKeyStore::empty();
        let res = run_backup(&mut drive, Zeroizing::new(PASS.to_string()), &ks).unwrap();

        // Leave the originals in place so they all get copied into pre-restore.
        let rr = run_restore(&mut drive, &res.file_id, Zeroizing::new(PASS.to_string()), &ks).unwrap();
        let pre = PathBuf::from(&rr.pre_restore_dir);
        assert!(pre.is_dir());
        // config + secrets + settings + key = 4 existing targets copied.
        let copied: Vec<_> = std::fs::read_dir(&pre).unwrap().filter_map(|e| e.ok()).collect();
        assert_eq!(copied.len(), 4, "every existing target pre-backed-up: {:?}",
            copied.iter().map(|e| e.file_name()).collect::<Vec<_>>());
        // The copied config matches the pre-restore original (CLOBBER check:
        // here originals were intact so content equals the restored content).
        let saved_cfg = pre.join("config");
        assert!(saved_cfg.is_file(), "config copied into pre-restore dir");
    }

    #[test]
    fn ciphertext_at_rest_no_plaintext_alias_in_blob() {
        let _g = env_lock();
        let sb = Sandbox::new("ciphertext");
        seed_sources(&sb);

        let mut drive = FakeDrive::default();
        let ks = MockKeyStore::empty();
        let _ = run_backup(&mut drive, Zeroizing::new(PASS.to_string()), &ks).unwrap();

        // Grab the uploaded blob from the fake Drive.
        let blob = drive.store.borrow()[0].1.clone();

        // The known host alias + the secret value + key bytes must NOT appear
        // in the uploaded ciphertext (zero-knowledge at rest).
        for needle in [
            b"prod-secret-alias".as_slice(),
            b"hunter2".as_slice(),
            b"SECRETKEYBYTES".as_slice(),
            b"203.0.113.9".as_slice(),
        ] {
            assert!(
                blob.windows(needle.len()).all(|w| w != needle),
                "plaintext '{}' leaked into the uploaded blob",
                String::from_utf8_lossy(needle)
            );
        }
        // Sanity: the blob IS a valid .htbk (magic present) and decodes back.
        assert_eq!(&blob[0..4], b"HTBK");
        let pt = crypto::decode(PASS, &blob).unwrap();
        let m: Manifest = serde_json::from_slice(&pt).unwrap();
        assert_eq!(m.host, "test-box");
        // The alias DOES appear once we base64-decode the config file content
        // stored in the manifest (files are kept base64 in `content_b64`, so the
        // alias is NOT a plaintext substring of the raw manifest JSON — only of
        // the decoded config body).
        let cfg_file = m
            .files
            .iter()
            .find(|f| f.path_role == ROLE_CONFIG)
            .expect("config present in manifest");
        let cfg_body = B64.decode(cfg_file.content_b64.as_bytes()).unwrap();
        let cfg_text = String::from_utf8_lossy(&cfg_body);
        assert!(
            cfg_text.contains("prod-secret-alias"),
            "alias present in decoded config after decrypt"
        );
    }

    #[test]
    fn list_backups_reports_cleartext_facets() {
        let _g = env_lock();
        let sb = Sandbox::new("list");
        seed_sources(&sb);

        let mut drive = FakeDrive::default();
        let ks = MockKeyStore::empty();
        let res = run_backup(&mut drive, Zeroizing::new(PASS.to_string()), &ks).unwrap();
        let metas = run_list(&mut drive).unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].file_id, res.file_id);
        assert_eq!(metas[0].host, "test-box");
        assert_eq!(metas[0].schema, SCHEMA);
        assert_eq!(metas[0].created_at, "2026-11-04T00:00:00Z");
    }

    #[test]
    fn run_list_sorts_newest_first() {
        // Seed three backups out of chronological order into the fake Drive and
        // assert run_list returns them created_at descending (newest first).
        let _g = env_lock();
        let _sb = Sandbox::new("list-sort");
        let mut drive = FakeDrive::default();
        // Manually push (bypassing run_backup) so we control created_at.
        for (id, ts) in [
            ("2026-11-01T00:00:00Z"),
            ("2026-11-05T00:00:00Z"),
            ("2026-11-03T00:00:00Z"),
        ]
        .iter()
        .enumerate()
        {
            drive.store.borrow_mut().push((
                format!("F{id}"),
                vec![0u8; 8],
                SCHEMA,
                ts.to_string(),
                "box".to_string(),
                format!("hosterm-backup-box-{ts}.htbk"),
            ));
        }
        let metas = run_list(&mut drive).unwrap();
        let order: Vec<&str> = metas.iter().map(|m| m.created_at.as_str()).collect();
        assert_eq!(
            order,
            vec!["2026-11-05T00:00:00Z", "2026-11-03T00:00:00Z", "2026-11-01T00:00:00Z"],
            "list sorted newest-first"
        );
    }

    #[test]
    fn preflight_reports_fixed_targets_and_existence() {
        let _g = env_lock();
        let sb = Sandbox::new("preflight");
        // Only config + secrets exist; settings does NOT.
        std::fs::write(sb.ssh("config"), b"Host x\n").unwrap();
        std::fs::write(sb.ssh("hosterm.secrets"), b"x=y\n").unwrap();

        let pf = preflight();
        assert_eq!(pf.targets.len(), 3, "fixed three-target set");
        let by_role: BTreeMap<_, _> =
            pf.targets.iter().map(|t| (t.path_role.as_str(), t.exists)).collect();
        assert_eq!(by_role[ROLE_CONFIG], true, "config exists");
        assert_eq!(by_role[ROLE_SECRETS], true, "secrets exists");
        assert_eq!(by_role[ROLE_SETTINGS], false, "settings absent");
    }

    #[test]
    fn restore_floors_key_perms_to_0600() {
        let _g = env_lock();
        let sb = Sandbox::new("perms");
        seed_sources(&sb);
        let mut drive = FakeDrive::default();
        let ks = MockKeyStore::empty();
        let res = run_backup(&mut drive, Zeroizing::new(PASS.to_string()), &ks).unwrap();
        std::fs::write(sb.ssh("keys/id_test"), b"CLOBBER").unwrap();
        run_restore(&mut drive, &res.file_id, Zeroizing::new(PASS.to_string()), &ks).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(sb.ssh("keys/id_test")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "restored key is 0600");
            let cfgmode = std::fs::metadata(sb.ssh("config")).unwrap().permissions().mode();
            assert_eq!(cfgmode & 0o777, 0o600, "restored config is 0600");
        }
    }

    #[test]
    fn empty_sources_backup_restore_is_valid() {
        let _g = env_lock();
        // Keep the sandbox alive (its Drop restores env) but we seed no sources.
        let _sb = Sandbox::new("empty");
        // No config / secrets / settings / keys at all.
        let mut drive = FakeDrive::default();
        let ks = MockKeyStore::empty();
        let res = run_backup(&mut drive, Zeroizing::new(PASS.to_string()), &ks).unwrap();
        // Manifest has zero files but is still a valid encrypted backup.
        let rr = run_restore(&mut drive, &res.file_id, Zeroizing::new(PASS.to_string()), &ks).unwrap();
        assert!(rr.restored.is_empty(), "nothing to restore on an empty backup");
        assert!(PathBuf::from(&rr.pre_restore_dir).is_dir());
    }

    // ---- device-keyring (optional-passphrase) backup/restore (v0.14.6) --

    #[test]
    fn empty_passphrase_device_key_roundtrip() {
        let _g = env_lock();
        let sb = Sandbox::new("devkey-roundtrip");
        seed_sources(&sb);

        let orig_cfg = std::fs::read(sb.ssh("config")).unwrap();
        let orig_sec = std::fs::read(sb.ssh("hosterm.secrets")).unwrap();
        let orig_key = std::fs::read(sb.ssh("keys/id_test")).unwrap();

        let mut drive = FakeDrive::default();
        let ks = MockKeyStore::empty();
        // Empty passphrase → device-keyring mode. A fresh key is generated and
        // stored in the mock keyring.
        let res = run_backup(&mut drive, Zeroizing::new(String::new()), &ks).unwrap();
        assert!(res.bytes > 0, "device-key backup produced bytes");
        assert!(ks.key.borrow().is_some(), "device key persisted to keyring");

        // The uploaded blob is encrypted (kdf_id=2) — NEVER plaintext.
        let blob = drive.store.borrow()[0].1.clone();
        assert_eq!(&blob[0..4], b"HTBK");
        assert_eq!(blob[5], 2, "device-keyring kdf_id in header");
        assert_eq!(
            crypto::header_key_mode(&blob).unwrap(),
            crypto::KeyMode::DeviceKeyring
        );

        // Clobber, then restore reads the key from the same keyring.
        std::fs::write(sb.ssh("config"), b"Host CLOBBERED\n").unwrap();
        std::fs::write(sb.ssh("keys/id_test"), b"CLOBBERED").unwrap();
        let rr = run_restore(&mut drive, &res.file_id, Zeroizing::new(String::new()), &ks).unwrap();
        assert!(!rr.restored.is_empty());

        assert_eq!(std::fs::read(sb.ssh("config")).unwrap(), orig_cfg, "config restored (device key)");
        assert_eq!(std::fs::read(sb.ssh("hosterm.secrets")).unwrap(), orig_sec, "secrets restored (device key)");
        assert_eq!(std::fs::read(sb.ssh("keys/id_test")).unwrap(), orig_key, "key restored (device key)");
    }

    #[test]
    fn device_key_reused_across_backups() {
        let _g = env_lock();
        let sb = Sandbox::new("devkey-reuse");
        seed_sources(&sb);
        let mut drive = FakeDrive::default();
        let ks = MockKeyStore::empty();

        let _ = run_backup(&mut drive, Zeroizing::new(String::new()), &ks).unwrap();
        let key_after_first = ks.key.borrow().unwrap();
        let _ = run_backup(&mut drive, Zeroizing::new(String::new()), &ks).unwrap();
        let key_after_second = ks.key.borrow().unwrap();
        assert_eq!(key_after_first, key_after_second, "same device key reused, not regenerated");
    }

    #[test]
    fn device_key_restore_without_key_fails_cleanly() {
        let _g = env_lock();
        let sb = Sandbox::new("devkey-missing");
        seed_sources(&sb);

        // Snapshot the untouched targets so we can assert no write happened.
        let orig_cfg = std::fs::read(sb.ssh("config")).unwrap();
        let pre_before = count_pre_restore(&sb);

        let mut drive = FakeDrive::default();
        let backup_ks = MockKeyStore::empty();
        let res = run_backup(&mut drive, Zeroizing::new(String::new()), &backup_ks).unwrap();

        // Simulate restoring on a DIFFERENT machine: an empty keyring, no key.
        let other_ks = MockKeyStore::empty();
        let err = run_restore(&mut drive, &res.file_id, Zeroizing::new(String::new()), &other_ks)
            .unwrap_err();
        assert!(
            err.contains("device key") && err.contains("keyring"),
            "actionable missing-key error: {err}"
        );

        // Fail-safe: nothing on disk touched, no pre-restore dir created.
        assert_eq!(std::fs::read(sb.ssh("config")).unwrap(), orig_cfg, "config untouched when key missing");
        assert_eq!(count_pre_restore(&sb), pre_before, "no pre-restore dir when key missing");
    }

    #[test]
    fn user_passphrase_roundtrip_still_works_after_device_mode_added() {
        // Regression: adding device-key mode must not break existing
        // user-passphrase containers (kdf_id=1). Backup with a passphrase, then
        // restore with it; the device-key store is present but unused.
        let _g = env_lock();
        let sb = Sandbox::new("devkey-pw-compat");
        seed_sources(&sb);
        let orig_cfg = std::fs::read(sb.ssh("config")).unwrap();

        let mut drive = FakeDrive::default();
        let ks = MockKeyStore::empty();
        let res = run_backup(&mut drive, Zeroizing::new(PASS.to_string()), &ks).unwrap();
        // Passphrase backups must NOT touch the device key.
        assert!(ks.key.borrow().is_none(), "passphrase mode never creates a device key");

        let blob = drive.store.borrow()[0].1.clone();
        assert_eq!(blob[5], 1, "user-passphrase kdf_id=1 in header");
        assert_eq!(
            crypto::header_key_mode(&blob).unwrap(),
            crypto::KeyMode::UserPassphrase
        );

        std::fs::write(sb.ssh("config"), b"Host CLOBBERED\n").unwrap();
        run_restore(&mut drive, &res.file_id, Zeroizing::new(PASS.to_string()), &ks).unwrap();
        assert_eq!(std::fs::read(sb.ssh("config")).unwrap(), orig_cfg, "passphrase restore still works");
    }

    #[test]
    fn iso_formatter_matches_known_epoch() {
        // 2026-11-04T00:00:00Z == 1793750400 (sanity for the civil-date math).
        assert_eq!(format_iso_utc(1_793_750_400), "2026-11-04T00:00:00Z");
        // Unix epoch.
        assert_eq!(format_iso_utc(0), "1970-01-01T00:00:00Z");
    }

    fn count_pre_restore(sb: &Sandbox) -> usize {
        let d = sb.data();
        if !d.is_dir() {
            return 0;
        }
        std::fs::read_dir(&d)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| {
                        e.file_name()
                            .to_string_lossy()
                            .starts_with("pre-restore-")
                    })
                    .count()
            })
            .unwrap_or(0)
    }
}
