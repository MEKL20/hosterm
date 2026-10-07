# QA Report — hosterm ~/.ssh/config round-trip logic

Date: 2026-10-07
Target: /home/mekl/src/hosterm/src-tauri/src/lib.rs — parse_config / serialize_config / set_opt / get_opt / save_host / delete_host / read_ssh_config / read_config_raw / write_config_raw
Method: black-box via new #[cfg(test)] module `qa_roundtrip_tests` (30 tests added to src-tauri/src/lib.rs). Backend only.
Safety: every file-touching test sets HOSTERM_CONFIG to a unique file under std::env::temp_dir(); real ~/.ssh/config never touched. Env races serialized by static ENV_LOCK mutex; tests run with --test-threads=1.

## How to reproduce

```
export PATH="$HOME/.cargo/bin:$PATH"
cd /home/mekl/src/hosterm/src-tauri && cargo test --lib
```

Result: `test result: FAILED. 25 passed; 5 failed; 0 ignored; 0 measured` (20 pre-existing + 5 added pass-tests; 5 failures are all real bugs, reproduced identically in two consecutive runs — see evidence/repro-twice.txt). Full raw output: evidence/final-full.txt.

## Confirmed bugs

### BUG-1 (High) — Quoted/space aliases are truncated at first token; editing corrupts the block
- Repro: config contains `Host "my box"` (or unquoted `Host my box`). `read_ssh_config` returns `name = "my` (first whitespace token only; quotes are NOT stripped). `save_host(Some("my"), ...)` finds that block, rewrites its first pattern, producing:

```
Host renamed box"
    HostName 1.2.3.4
```

- Expected per spec: the host list shows the full alias (quoted alias `my box`); an edit must not mangle the pattern line.
- Actual: alias truncated to `"my`; after save the block header becomes `Host renamed box"` — corrupted alias with a stray trailing quote; the remaining fragment `box"` becomes part of the pattern list.
- Impact: any user with a quoted or multi-word alias who edits that host through the form permanently corrupts the config.
- Evidence: tests `qa_alias_with_space_no_corruption` (passes, prints corrupt output), `qa_unquoted_space_alias_dto_name_lossy` (FAILS: `left: "my"`, `right: "my box"`), `qa_alias_with_space_roundtrip_and_rename` (passes, prints before/after). Evidence/final-full.txt.
- Root cause (read-only observation): `parse_config` splits `Host` line by whitespace and `read_ssh_config` takes `patterns.first()`; quotes never parsed; `save_host` writes `patterns.join(" ")`.

### BUG-2 (Medium) — Round-trip is NOT byte-identical for CRLF, tabs, lowercase `host`, and empty `Host` line
Spec: "Round-trip (parse then serialize an unchanged config) must be byte-identical." Each case below fails that contract (all reproduced twice):
- CRLF file → all `\r\n` silently converted to `\n` (`qa_roundtrip_crlf_not_byte_identical` FAILED). Any save then rewrites the whole file LF-only.
- Tab-indented directives → re-emitted with 4 spaces (`qa_roundtrip_tab_indent_not_byte_identical` FAILED).
- Lowercase `host prod` → re-emitted as `Host prod` (`qa_roundtrip_lowercase_host_keyword_not_byte_identical` FAILED).
- `Host` with no patterns → re-emitted as `Host ` with trailing space (`qa_roundtrip_bare_host_keyword_no_patterns` FAILED: `"Host \n..."` vs `"Host\n..."`).
- Note: directive lines inside blocks are always re-emitted as `<4 spaces><Key> <value>`; original indentation/inter-token spacing lost (tabs case is the visible symptom).
- Severity rationale: silent whole-file reformatting on any edit; functionally harmless to ssh (except none of these change semantics) but violates the stated byte-identical contract and produces noisy diffs.

### BUG-3 (Medium) — Backend permits deleting the `Host *` catch-all block
- Repro: `delete_host("*")` on a config containing `Host *` returns Ok(()) and rewrites the file without the catch-all.
- Expected per spec: pattern blocks (containing `*`/`?`) "must never be rewritten" by host operations; deletion should be refused (UI shows them read-only, but the backend command is the trust boundary and enforces nothing).
- Evidence: test `qa_backend_allows_deleting_wildcard_block` (passes, prints `delete_host("*") = Ok(())` and resulting file). Evidence/final-full.txt.
- Impact: GUI guard bypass = data loss of global defaults; any IPC caller can delete `Host *`.

### BUG-4 (Low) — New option appended after the block's trailing blank line
- Repro: config where a block's lines end with the blank inter-block separator (i.e. the blank line sits inside the block, as the app itself writes between blocks — e.g. after `Port 2222` there is an empty line before the next `Host`). `save_host` adding a NEW option (e.g. `User` when block had none) pushes it after that blank line:

```
Host prod
    HostName 203.0.113.10
    Port 2222

    User root      <- landed after blank separator
Host staging
```

- Expected: new option stays inside the host block.
- Actual: option renders after the blank line — visually it "belongs" to nothing; still parsed as part of prod by ssh (blank lines don't terminate blocks), so cosmetic/structural, hence Low. But it shows `set_opt`/serialize do not model block boundaries, so any future layout change amplifies this.
- Evidence: `qa_append_new_option_lands_after_blank_separator` passes while printing the misplaced line; `qa_wildcard_preserved_on_other_host_edit` output shows the same artifact. Evidence/final-full.txt.

## Cases that PASSED (no bug found)

- Case 2 (partially): duplicate `Host dup` blocks — `delete_host("dup")` removes BOTH duplicate blocks (arguably intended; keeps `Host keep` intact, evidence printed); `save_host` rename updates only the FIRST dup block, second survives. No corruption. (If "remove only one" was intended, this is a design question, not flagged as bug.)
- Case 3: `Host *` and `Host 10.0.*` blocks preserved verbatim on round-trip and untouched when `save_host` edits a different host (`qa_wildcard_preserved_on_other_host_edit`, `qa_pattern_block_10_0_preserved` PASS).
- Case 4 (partially): bare keyword with no value (`ForwardX11`) round-trips byte-identical (`qa_roundtrip_bare_keyword_no_value` PASS).
- Case 5: missing config file — read returns empty list, delete errors, save creates file with mode 0600 (asserted, PASS); empty file parses/serializes to empty, write_config_raw("") OK.
- Case 6: Host block with no HostName — editing user keeps no HostName line; no phantom directive created (`qa_host_without_hostname_edit_keeps_it_absent` PASS).
- Case 7: `delete_host` on missing alias returns Err and leaves file byte-identical (`qa_delete_missing_alias_no_corruption` PASS).
- Case 8: rename — `save_host(Some("staging"), name="uat")` renames correct block, prod section byte-identical, `Host *` untouched; rename of missing original errors without touching file; empty/whitespace alias rejected (`qa_rename_*`, `qa_save_empty_alias_rejected` PASS).
- Case 9: `write_config_raw` arbitrary text (unicode, quotes, backslash, multiple blank lines) round-trips byte-identical through read_config_raw, perms 0600 (`qa_write_config_raw_roundtrip_arbitrary_text` PASS).
- Case 10: edit to one host — rest of file byte-identical incl. top comment, blank lines, inter-block order, and comment INSIDE the edited block (`qa_edit_one_host_rest_of_file_byte_identical`, `qa_edit_preserves_comment_inside_edited_block` PASS).

## Additional finding (design, unflagged)
- New hosts are appended at file END. If a `Host *` catch-all exists, the new block lands after it and is shadowed for options set in the catch-all (ssh first-match-wins). Confirmed by `qa_new_host_appended_after_catchall_is_shadowed` (pass, prints byte offsets). Functional for connect (alias-specific settings still win), but options intended as defaults may surprise. Not counted as a bug against the stated spec.

## Not tested (boundary)
- GUI/UI layer entirely — cannot render headless on this host (WebKit paints blank): host form validation, pattern-block read-only UI behavior, IPC wiring of tauri commands.
- Real `ssh` parsing/behavior against generated configs; PTY session layer.
- Concurrent saves (two processes racing on the file); file-locking behavior.
- Non-UTF8 config files (read_to_string would error → silently treated as empty config: read path `unwrap_or_default` would then make a save WIPE the file — flagged as a risk, not reproduced in a test).

## Evidence files
- evidence/final-full.txt — full `cargo test --lib -- --nocapture --test-threads=1` output (all 30 tests, file dumps)
- evidence/repro-twice.txt — two consecutive runs, identical failure set
- evidence/run3.txt — intermediate run with failure details

Test code: `mod qa_roundtrip_tests` in /home/mekl/src/hosterm/src-tauri/src/lib.rs (test-only; no app logic modified).
