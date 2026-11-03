# Discovery: SFTP bug triage v1 (MEKL report, 2026-11-03)

Grounding doc for the crew. nano did Tier-0 triage against the real code. Two
bugs have a confirmed root cause; one needs MEKL repro (OPEN QUESTION).

## Bug 1 — password-auth SFTP hangs the app → force-close  [SEVERE]

**Report:** opening an SFTP host that uses a stored password makes hosterm "not
responding"; must force-close.

**Confirmed root cause (nano, from code):**
- All SFTP commands are **synchronous** `#[tauri::command] fn sftp_list / sftp_read_file / sftp_write_file / sftp_upload / sftp_download` (`src-tauri/src/lib.rs:1306+`). Sync Tauri commands can block the IPC/UI.
- `sftp_spawn` (`lib.rs:1118`) for a password host calls `rx.wait_for("sftp>", …, Duration::from_secs(30))` — a **blocking wait up to 30s** for the login prompt. If it never matches, it blocks the full timeout.
- Frontend `openSftp` (`src/main.ts:1647`) awaits these with no UI-releasing guard.
- Net: opening a password SFTP host blocks the WebView for up to 30s (or indefinitely on a mismatch) → "not responding".

**Fix direction (for architect/lead to confirm):** make the SFTP commands
`async` (run the blocking PTY login off the UI/IPC thread, e.g.
`tauri::async_runtime::spawn_blocking`), and/or show a cancellable loading state
in `openSftp` so the UI never freezes. Verify the 30s login wait cannot run on
the thread that serves the WebView.

## Bug 3 — right-click "refresh" in SFTP reloads the whole app, all tabs vanish  [SEVERE — data loss]

**Report:** right-click → refresh inside the SFTP view reloads all of hosterm;
every tab disappears.

**Confirmed root cause (nano, from code):**
- Our own `.sftp-refresh` button correctly calls `renderSftp` (no reload) — NOT the culprit.
- There is **no document/window-level `contextmenu` guard** (`contextmenu` listeners exist only on tab / host-row / one element; grep confirms no `document`/`window` contextmenu handler).
- `src-tauri/tauri.conf.json` does **not** disable the native context menu or devtools.
- Net: right-clicking in the SFTP list/pane area shows the **native WebView2 context menu** with a standard **Reload**; clicking it reloads the SPA and all in-memory tab state is lost.

**Fix direction:** suppress the native reload path — a global `contextmenu`
`preventDefault` over the app surface (or disable the WebView context menu /
reload accelerator in Tauri config). Decide whether to keep a custom right-click
menu for SFTP rows. Guard against accidental full-app reload = protect tab state.

## Bug 2 — SFTP target path "broken / not easy to use"  [MEDIUM — OPEN QUESTION, needs MEKL repro]

**Report (ambiguous):** "sftp target path still broken, not easy to use."

**What the code shows:** `openSftp` seeds `path: ""` (= home); parent nav is
`s.path.replace(/\/+[^/]*$/, "")` (`main.ts:1719-1724`); path typed in `.sftp-path`.

**OPEN QUESTIONS for MEKL (clarify timed out — do not guess the fix):**
1. Does the remote path display wrong (double slash, unnormalized, home not resolving)?
2. Can't enter a folder (click doesn't navigate / parent wrong)?
3. Path empty / not pointing at home on first open?
4. Path is correct but tedious (too much manual typing) = UX, not a path bug?

Until MEKL answers, bug 2 stays in discovery — scope a fix only after repro.

## Suggested sequencing

- Bug 1 (backend async + frontend loading) and Bug 3 (contextmenu guard) are
  ready to spec now; file-disjoint-ish (lib.rs + main.ts shell vs main.ts SFTP).
- Bug 2 waits on MEKL repro.
- Every fix unit: QA + security review (verifier gate) before promote. Backend
  change (bug 1) needs cargo tests + smoke; bug 3 needs a smoke check that a
  simulated context-menu/reload path does not nuke tab state.
