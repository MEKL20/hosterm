# Security Gate — Windows OAuth keyring and passphrase hotfix

Date: 2026-11-07
Kanban: `t_c8c33854`
Verdict: **CLEAN — 0 findings**

## Verified invariants

- PKCE S256 flow unchanged.
- CSRF `state` comparison unchanged.
- Loopback listener remains bound to `127.0.0.1` and single-shot.
- Google scope remains `drive.file`.
- OAuth credentials remain compile-time CI injection; no credential literals added.
- Refresh token storage now resolves to Windows Credential Manager through keyring's `windows-native` feature instead of the mock store.
- No token values or token-response bodies are logged.
- Passphrases remain confined to DOM inputs and the existing single IPC invocation; they are wiped and re-masked on cleanup and are not persisted.
- Added UI strings use static markup or `textContent`; no new dynamic `innerHTML` sink was introduced.
- Hostile error text remains inert under browser smoke/CSP checks.

## Dependency review

The lockfile delta for keyring 3.6.3 adds only its Windows-native transitive dependencies:

- `byteorder`
- `windows-sys 0.60.2`

The pinned keyring version and registry checksum are unchanged.

## Evidence

- Rust tests: 176/176 PASS, including OAuth and the new native-Windows-keyring regression guard.
- Browser smoke: 445/445 PASS, including passphrase lifecycle, fake-clock toast expiry, CSP, and unsafe-DOM checks.
- Windows-target Cargo feature tree explicitly contains `keyring/windows-native`.
- `git diff --check`: PASS.
