# TSD: Ansible (v1) — technical specification + unit breakdown

| Field | Value |
|---|---|
| Author | crew-coding-lead |
| Status | Locked — unit cards fanned out (see §8). Buildable spec; frontend mocks §4 verbatim. |
| Upstream (CLOSED, do not relitigate) | `docs/FEATURE-SPEC-ansible-v1.md` (commit `9dae0b1`, nano-verified + MEKL-confirmed) · `docs/SDD.md §14` Ansible v1 (commit `28fba28`, ADR-A1..A3 + INV-28..31 + frozen IPC contract §14.4) |
| Scope of THIS doc | HOW it is built: locked stack, module/file boundaries, unit split + dependency order, per-unit acceptance, verifier gate (QA+SEC) per unit. No product re-scoping, no new ADRs. |
| Verify loop | per `docs/STATE.md §Verify loop` — re-measured at dispatch (cargo `--lib`, `npm run build`, headless smoke). This TSD does NOT hardcode cargo/smoke counts; it adds gates, never lowers a baseline. |
| Push model | direct-push `main`, `completion_contract = local-only`, workspace `dir:/home/mekl/src/hosterm`, identity Edwin Khoirudin / 69399477+MEKL20@users.noreply.github.com. Developers never commit/push; lead collects + re-verifies + pushes after QA+SEC gate. |

---

## 1. Locked stack (zero new deps)

- **Backend:** Rust, stdlib + `portable_pty` (already a dep) + `serde`/`serde_json` (already deps) + `dirs` (already a dep). **Zero new Rust crates.** No embedded python/ansible — `ansible-playbook` is a user-installed OS binary (NFR-ANS1).
- **Transport:** the existing `local_pty_spawn` PTY mechanism (`lib.rs:1768`) — `CommandBuilder` + the existing `PtyState`/`NEXT_ID`/`pump_pty`/`pty_kill`/`pty_write`/`pty_resize`. **No new event family, no new pty command** (SDD §14.2).
- **Frontend:** vanilla TS DOM in `src/main.ts`, static templates cloned from `index.html`, CSS in `src/styles.css` (Dusk tokens only). **Zero new JS deps.** All remote-origin render through `esc()`/`escAttr()` (ADR-8).
- **Stores:** app-data JSON under `dirs::config_dir()/hosterm/` (INV-22 family). Never `~/.ssh/config`, never the secrets sidecar (INV-29/30/31).

---

## 2. Module / file boundaries (binding — one active card per hotspot file)

| Artifact | Owner unit | Scope |
|---|---|---|
| `src-tauri/src/lib.rs` | BE-ANS1, BE-ANS2, BE-ANS3 (SERIALIZED — one active `lib.rs` card at a time) | 3 new `#[tauri::command]` thin wrappers (`check_ansible`, `ansible_run`, `ansible_history`) + pure cores. See LQ-1 decision below. |
| `src/main.ts` + `index.html` + `src/styles.css` | FE-ANS (single frontend card — serialized per repo hotspot rule) | Ansible panel, guide, pickers, dry-run gate state machine, PTY tab, history sub-panel. |
| `qa-reports/headless-smoke.mjs` | FE-ANS (extend) + QA-ANS (verify) | New smoke checks per SDD §14.8. |
| `docs/STATE.md` | BE-ANS1 (hygiene, same card) | Fix stale IPC count (see §7). |

### LQ-1 decision (unit split of backend) — DECIDED: keep in `lib.rs`, NO `mod ansible`.
The three commands are thin; the pure cores (`build_ansible_argv`, `build_inventory_ini`, `host_eligibility_ansible`, `ansible_missing`, `history_append_fifo`) are small, self-contained functions. Fleet got its own `fleet.rs` because it carried DTOs + aggregation logic; Ansible v1 reuses the existing PTY plumbing wholesale and adds no new event family, so the net `lib.rs` body stays well under the ~150-LOC threshold the architect flagged. **Keep everything in `lib.rs`.** If during BE-ANS3 the `ansible_run` body + helpers unexpectedly exceed ~150 LOC, the backend dev FLAGS it to lead (hotspot comment) rather than silently spinning up `mod ansible` — that would change the file boundary mid-wave. Command names/payloads (§4) are identical either way.

### LQ-2 decision (ephemeral cleanup trigger) — DECIDED: backend-side on PTY exit/kill.
The inventory + cfg temp paths are registered against the pty id at spawn; they are `unlink`ed in the same place the pty session is torn down (the pty exit path AND `pty_kill`). Cleanup must NOT be contingent on the frontend being alive (a crashed/closed renderer must still clean up). INV-29 "deleted when the run's PTY exits or is stopped" holds. Lives in BE-ANS3 (the `ansible_run` owner).

### LQ-3 decision (`ANSIBLE_NOCOLOR`) — DECIDED: YES, pin `ANSIBLE_NOCOLOR=1`.
Pin it on spawn alongside `ANSIBLE_HOST_KEY_CHECKING=True` and `ANSIBLE_CONFIG=<cfg>`. Cleaner `PLAY RECAP` for the frontend parser (no ANSI strip). Lives in BE-ANS3.

---

## 3. Physical realization (no DB — three app-data files)

Per SDD §14.6, realized concretely:

| File | Path | Mode | Lifetime | Writer |
|---|---|---|---|---|
| history | `dirs::config_dir()/hosterm/ansible-history.json` | 0600 | persistent | `ansible_history` (append op), atomic temp→chmod→rename |
| ephemeral inventory | `dirs::config_dir()/hosterm/ansible-inv-<rand>.ini` | 0600 | per-run, unlinked on pty exit/kill | `ansible_run` |
| ephemeral cfg | `dirs::config_dir()/hosterm/ansible-cfg-<rand>.ini` | 0600 | per-run, unlinked on pty exit/kill | `ansible_run` |

`<rand>` = a non-guessable suffix (reuse the existing random/id helper the codebase already uses for temp names; do NOT add a new RNG crate). Atomic write pattern = the EXACT `settings.json` pattern (sibling temp → chmod 0600 → rename; refuse to clobber on a failed write). Corrupt/missing history → `[]`, corrupt bytes untouched until a legitimate write (INV-24/INV-30).

Inventory INI line per eligible host (SDD §14.3):
```
<alias> ansible_host=<HostName> ansible_user=<User> ansible_port=<Port> ansible_ssh_private_key_file=<IdentityFile>
```
Absent config fields are OMITTED from the line. `[group]` headers from alias-prefix-before-first-`-` derivation (identical to Fleet FR-FL2, `main.ts:346`). Password-auth hosts are SKIPPED — never a line, never a group member (ADR-A2/INV-31).

Ephemeral cfg = pinned-safe defaults so a stray `ansible.cfg` in the user cwd is never honored (host key checking enforced via env; `nocolor = True` optional).

---

## 4. Frozen IPC contract (implement VERBATIM from SDD §14.4 — reproduced for the units)

Command args camelCase at the Tauri boundary; JSON payload fields snake_case. A shape change here is an SDD addendum revision, never implementation drift.

```ts
// 1. Binary presence — read-only, 0 spawn, 0 network (FR-ANS1/ANS2; ADR-A3)
invoke("check_ansible"): Promise<AnsibleCheck>
interface AnsibleCheck { missing: boolean }   // true ⇒ ansible-playbook not resolvable in PATH

// 2. Spawn a run (check or real) — returns a PTY id into the EXISTING pty session map
invoke("ansible_run", { req: AnsibleRunReq }): Promise<number>   // pty id; errors as Err("ansible:<code>")
interface AnsibleRunReq {
  mode: "check" | "real";    // check = --check --diff; real = no --check
  playbook: string;          // absolute path to picked *.yml/*.yaml (frontend validates UTF-8 ≤2 MiB BEFORE call)
  aliases: string[];         // picker selection; backend REPLANS authoritatively via host_eligibility_ansible
  limit: string;             // the --limit targets expression (ONE argv element; comma-joined alias/group names)
  cols: number; rows: number;// PTY geometry, like local_pty_spawn
}
//   • host_eligibility_ansible(aliases): key/agent → eligible; password/unsafe alias → skipped (NOT spawned)
//   • 0 eligible → Err("ansible:no-eligible-hosts"), 0 spawn, 0 files written
//   • generates ephemeral inventory+cfg (0600, data_dir()), argv as Vec<String> (INV-28), spawns PTY
//   • env on spawn: ANSIBLE_HOST_KEY_CHECKING=True, ANSIBLE_CONFIG=<ephemeral cfg>, ANSIBLE_NOCOLOR=1
//   • NO password/secret ever enters argv/env/payload (ADR-A2/INV-31)
//   • ephemeral files unlinked when this pty's exit fires or pty_kill is called (INV-29, backend-side)
//   • output/exit ride EXISTING pty://output/{id} + pty://exit/{id}; stop = existing pty_kill(id)

// 3. History — app-data JSON, atomic 0600, corrupt→empty, 200 FIFO (FR-ANS14/ANS15)
invoke("ansible_history", { op: AnsibleHistoryOp }): Promise<AnsibleHistoryEntry[]>
type AnsibleHistoryOp =
  | { kind: "read" }                                // → full array (newest-last), [] if missing/corrupt
  | { kind: "append"; entry: AnsibleHistoryEntry }  // → array after append + FIFO-cap 200; atomic whole-file write
interface AnsibleHistoryEntry {
  playbook: string; targets: string; mode: "check" | "real";
  started_at: string;   // ISO-8601 UTC
  duration_ms: number; rc: number;
  recap: string;        // trimmed PLAY RECAP block ONLY — NEVER full output, NEVER any secret
}
```

argv (frozen, SDD §14.2):
```
check:  ansible-playbook -i <inv> --check --diff --limit <targets> <playbook>
real:   ansible-playbook -i <inv>               --limit <targets> <playbook>
```
`<targets>` and `<playbook>` are each ONE argv element — never interpolated into a shell string (INV-28).

Error strings are opaque `ansible:<code>` tokens; the backend never embeds attacker-controlled host/path data in an error string.

---

## 5. Dry-run-WAJIB gate (frontend state machine — implement VERBATIM from SDD §14.5)

```
state: { playbookPath, targets, checkedKey: string|null }
checkedKey = `${playbookPath}\u0000${targets}`           // identity of the last SUCCESSFUL dry-run
Run-real ENABLED  ⇔  checkedKey === current(`${playbookPath}\u0000${targets}`)
  • change playbook OR targets → checkedKey := null → Run-real DISABLED (gate reset)
  • dry-run (mode:"check") pty exits rc==0 → checkedKey := current key → Run-real ENABLED
  • Run-real ALWAYS requires an explicit confirm dialog first (FR-ANS10); never auto-runs (INV-25 sibling)
```
This is a SECURITY layer, not just UX: the permission-widening mutating run is gated behind the cheap safe dry-run + an explicit human confirm, and is never the default auto-path.

---

## 6. Testing strategy (per unit — extend STATE.md baseline, never lower)

**cargo (`cargo test --manifest-path src-tauri/Cargo.toml --lib`; argon2 ~8 min, run in background):**
- `build_ansible_argv`: array shape (not a string); `--check --diff` present in check mode, absent in real; playbook + `--limit` each a single element (INV-28; marker-file fixture proving no shell interp).
- `build_inventory_ini`: alias→vars mapping; absent-field omission; `[group]` from prefix derivation; **password host skipped / no key bytes** (INV-31).
- `host_eligibility_ansible`: key/agent → eligible; password/unsafe alias → skipped+reason; mirrors fleet eligibility.
- `ansible_missing`: resolve logic mirrors `binaries_missing`.
- `history_append_fifo`: 200-cap FIFO; atomic write; corrupt→`[]` no-clobber (INV-30).
- **byte-identity**: an Ansible code path never mutates `~/.ssh/config` or sidecar (hash+mtime identical before/after; NFR-ANS4/INV-29); ephemeral inventory+cfg unlinked after run (INV-29).

**build:** `npm run build` → 0 TS errors.

**smoke (`qa-reports/headless-smoke.mjs`; `npm install --no-save playwright@1.63.0`, remove after per STATE.md):**
- Ansible panel renders.
- mock `check_ansible → {missing:true}` → per-OS guide shown, run blocked, re-check button present (W2-A pattern).
- dry-run-WAJIB gate: Run-real disabled until a mocked `mode:"check"` pty exits rc==0 for the current playbook+target; changing either resets it.
- confirm dialog appears before a real run; never auto-runs.
- hostile `PLAY RECAP` / task output fixture → 0 CSP violations, 0 injected `<script>` (ADR-8).
- grep `sh -c|bash -c|cmd /C` = 0 on the Ansible path (NFR-ANS5); `draggable` grep stays 0.

---

## 7. Hygiene fixes (architect-flagged — land in BE-ANS1)

1. **STATE.md IPC count stale.** `docs/STATE.md §Verify loop` line 68 still reads IPC=30 (lib.rs 28 + fleet.rs 2) and cargo 107/107 — both stale against live HEAD. **Live gate at HEAD = 40** (`grep -rc "tauri::command" src-tauri/src/*.rs` → lib.rs 37 + fleet.rs 2 + backup.rs 1 false-positive). The three new Ansible commands land in `lib.rs`, moving the gate **40 → 43** (lib.rs 37→40). BE-ANS1 updates STATE.md to the number the gate ACTUALLY emits at that card's dispatch (re-measure; do not copy this doc's 40 blindly — other cards may have landed). Record both the raw grep number and the registered-command number.
2. **`backup.rs` `#[tauri::command]`-in-comment false-positive.** `src-tauri/src/backup.rs:8` prose ("four thin `#[tauri::command]` wrappers") makes `grep -rc` count backup.rs as 1 though it registers 0 real commands. BE-ANS1 MAY reword that comment so the gate no longer matches it (e.g. drop the backticked attribute or write "four thin Tauri command wrappers"), making gate == registered. This is a hygiene nicety, NOT a blocker — if done, update STATE.md's gate number accordingly (the backup.rs 1 disappears). Touching `backup.rs` is a one-line comment edit only; it does not make backup.rs a code hotspot for this wave.

> Both fixes are docs/comment-only. They do NOT require the full code gate chain; nano verification + lead suffices (AGENTS.md §6). They ride BE-ANS1 so there is no separate card churn.

---

## 8. Unit breakdown + dependency order

Six units. Dependency order driven by the frozen contract: the frontend mocks §4/§5 so it is NOT hard-blocked on the backend, but the integration units (dry-run gate wired to real `ansible_run`, history sub-panel wired to real `ansible_history`) need BE merged. **`lib.rs` is a serialized hotspot** — BE-ANS1 → BE-ANS2 → BE-ANS3 run one at a time, linked in series. FE-ANS is a single frontend card (one active `main.ts`/`index.html` card per repo rule) and depends on the backend being complete for its integration pieces, so it is linked as a child of BE-ANS3.

| Unit | Assignee | Owns (files) | Depends on |
|---|---|---|---|
| BE-ANS1 | crew-coding-backend | `lib.rs` (`check_ansible` + `ansible_missing` pure core + tests) + `docs/STATE.md` + `backup.rs` comment | — (start first) |
| BE-ANS2 | crew-coding-backend | `lib.rs` (`build_ansible_argv` + `build_inventory_ini` + `host_eligibility_ansible` pure cores + tests) | BE-ANS1 (serialize lib.rs) |
| BE-ANS3 | crew-coding-backend | `lib.rs` (`ansible_run` spawn + ephemeral cleanup wiring + `ansible_history` read/append atomic + `history_append_fifo` + tests) | BE-ANS2 (serialize lib.rs) |
| FE-ANS | crew-coding-frontend | `src/main.ts` + `index.html` + `src/styles.css` + `qa-reports/headless-smoke.mjs` (Ansible panel: guide, pickers, dry-run gate, PTY tab, history, smoke) | BE-ANS3 (integration) |
| QA-ANS | crew-coding-qa | verify only (no code) | BE-ANS3 + FE-ANS |
| SEC-ANS | crew-coding-security | scan only (no code) | BE-ANS3 + FE-ANS |

QA-ANS + SEC-ANS are the verifier gate (children of the implementation they verify). No implementation unit self-promotes; QA+SEC approve, lead collects + re-verifies + pushes, nano re-verifies aggregate, MEKL final gate.

### Acceptance per unit

**BE-ANS1** — `check_ansible() -> AnsibleCheck{missing}` (read-only PATH resolve, 0 spawn, 0 network, mirrors `binaries_missing:1883`); pure `ansible_missing()` with cargo coverage. STATE.md IPC count corrected to live gate number; backup.rs comment false-positive optionally neutralized. Gate: cargo `--lib` green at STATE.md baseline + new `check_ansible`/`ansible_missing` tests; `grep -rc "tauri::command"` documented.

**BE-ANS2** — pure cores `build_ansible_argv` (INV-28 array shape, check vs real), `build_inventory_ini` (INV-29 line format, group derivation, absent-field omission), `host_eligibility_ansible` (INV-31 password-host skip + reason). No command wiring yet (cores only). Gate: cargo `--lib` green + argv/inventory/eligibility tests incl. INV-28/29/31 markers.

**BE-ANS3** — `ansible_run(req) -> pty id` (generates ephemeral inventory+cfg 0600, builds argv via BE-ANS2 cores, spawns PTY into existing `PtyState` map, pins `ANSIBLE_HOST_KEY_CHECKING=True`/`ANSIBLE_CONFIG`/`ANSIBLE_NOCOLOR=1`, 0-eligible → `ansible:no-eligible-hosts`, backend-side ephemeral unlink on pty exit/kill — LQ-2); `ansible_history(op)` read/append atomic 0600 + `history_append_fifo` 200-cap, corrupt→`[]`. Gate: cargo `--lib` green + byte-identity (INV-29 NFR-ANS4) + history atomic/FIFO/corrupt (INV-30) + ephemeral-cleanup tests; IPC gate now 43.

**FE-ANS** — Ansible sidebar tab + panel; binary-missing guide (reuse `openSshGuide`/`recheckBinaries` W2-A pattern, mocks `check_ansible`); playbook file-picker (`*.yml`/`*.yaml`, UTF-8 ≤2 MiB validation BEFORE `ansible_run`, FR-ANS5); host/group picker (reuse fleet picker, password rows marked skipped FR-ANS4); dry-run → `PLAY RECAP` parse + "N hosts will change, M tasks changed" summary; the WAJIB gate state machine (§5) wired to real `ansible_run`; Run-real confirm dialog (FR-ANS10, never auto-run) + PTY terminal tab (reuse `openTerminal` local path); history sub-panel reading `ansible_history`. All remote-origin render via `esc()`/`escAttr()`; markup cloned from static `index.html` templates (ADR-8, no new `innerHTML` sink). Extend `headless-smoke.mjs` per §6. Gate: `npm run build` 0 TS + smoke green at STATE.md baseline + new Ansible checks.

**QA-ANS** — verify the REAL merged diff (not self-reports): run `cargo test --manifest-path src-tauri/Cargo.toml --lib` (background, argon2 ~8 min), `npm run build`, `npm install --no-save playwright@1.63.0 && node qa-reports/headless-smoke.mjs` (remove Playwright after). Confirm every AC above + the verify-loop baselines only grew. Confirm all FR-ANS acceptance criteria exercised. Quote real command output. `request-changes` on any failure.

**SEC-ANS** — scan the Ansible path for: INV-28 (`sh -c|bash -c|cmd /C` = 0; argv is an array; playbook/`--limit` single elements), INV-31 (no secret in argv/env/inventory/history/IPC; password hosts skipped; only IdentityFile paths), INV-29 (no `~/.ssh/config`/sidecar read/write; byte-identity; ephemeral 0600 + unlinked), host key checking ON fail-closed (ADR-A1), ADR-8 hostile `PLAY RECAP`/output render (no new unescaped `innerHTML`), no idle/background network (NFR-ANS3). CLEAN verdict required (0 crit/high/med) before gate.

---

## 9. Non-negotiables carried into every unit

- Dry-run `--check --diff` WAJIB before any real run; Run-real disabled until dry-run succeeds once for the current playbook+target (security layer, SDD §14.5).
- v1 = key/agent hosts ONLY; password hosts → `skipped` with reason (MEKL-confirmed, ADR-A2).
- Zero new deps (stdlib + `portable_pty`); zero embedded python/ansible.
- Argv is a direct `Vec<String>` array → `CommandBuilder`; never `sh -c`/shell string (INV-14/INV-27/INV-28).
- Secrets never cross argv/env/IPC/inventory/history (INV-31); app-data stores isolated from config+sidecar (INV-29/30).
- All remote-origin render through `esc()`/`escAttr()`; static templates cloned, no dynamic `innerHTML` (ADR-8).
