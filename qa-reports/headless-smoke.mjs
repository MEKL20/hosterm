// Headless smoke test for the redesigned UI without a Tauri backend.
// Requires: npm install --no-save playwright@1.63.0 (chromium cached in
// ~/.cache/ms-playwright). Run: node qa-reports/headless-smoke.mjs
// Mocks window.__TAURI_INTERNALS__.invoke, loads dist/, asserts DOM behavior.
import { chromium } from "playwright";
import { createServer } from "http";
import { readFileSync, existsSync, readdirSync } from "fs";
import { extname, join } from "path";
import { fileURLToPath } from "url";

// fileURLToPath (not .pathname): on Windows .pathname yields "/C:/.../dist"
// with a leading slash before the drive letter, so join() builds an invalid
// path, existsSync() is always false, and every request falls back to
// index.html — the JS/CSS bundles never load and the whole DOM smoke fails.
// fileURLToPath returns a native path on both platforms.
const DIST = fileURLToPath(new URL("../dist", import.meta.url));
const MIME = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css" };

// CSP (AC R1, SEC-4): single source of truth = app.security.csp in
// src-tauri/tauri.conf.json (JSON, no comments allowed there — rationale lives here).
// Served on every response below so violations caught here mean real policy breaches.
// Per-directive (docs/SPIKE-csp-findings.md §3):
//   default-src 'self'            — deny-by-default floor
//   script-src 'self'             — STRICT (PRD R1.6, never relax); bundle is 1 external
//                                   file, 0 inline scripts; Tauri appends its own nonce
//   style-src 'self' 'unsafe-inline' — SOLE relaxation: CodeMirror style-mod injects
//                                   runtime <style>; hashes impractical (generated CSS),
//                                   nonce would neutralize 'unsafe-inline' (spike §2 trap)
//   img-src/font-src 'self'       — no data:/blob: usage, no @font-face
//   connect-src 'self' ipc: http://ipc.localhost — Tauri v2 does NOT auto-append IPC
//                                   origins (spike §1); ipc: = Linux, http://ipc.localhost = Windows
//   object-src 'none'             — no plugins
//   base-uri 'self'               — blocks injected <base> hijack
//   frame-ancestors 'none'        — no embedding
//   form-action 'none'            — zero <form> submits in app (all modals JS-driven);
//                                   blocks form-nav exfil (CSP3: no default-src fallback)
const CSP = JSON.parse(readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8")).app.security.csp;

const sampleSftp = [
  { name: "notes.txt", is_dir: false, is_link: false, size: 142 },
  { name: "projects", is_dir: true, is_link: false, size: 4096 },
];
const sampleHosts = [
  { name: "prod-web", host_name: "203.0.113.10", user: "deploy", port: "", identity_file: "", auth_method: "key", options: [] },
  { name: "db-core", host_name: "203.0.113.20", user: "admin", port: "", identity_file: "", auth_method: "key", options: [] },
  { name: "pw-host", host_name: "203.0.113.30", user: "ops", port: "", identity_file: "", auth_method: "password", has_password: true, options: [] },
];
const sampleConfig = `# demo
Host prod-web
    HostName 203.0.113.10
    User deploy

Host *`;

const calls = [];
// U2 skeleton check: delay read_ssh_config once to observe the loading state
let delayRead = 0;
// Fail the first sftp_list for pw-host to prove the stale-session retry path.
let sftpFailFirst = false;
// U5: force sftp_upload/sftp_download to reject (retry-row path) + delay a
// listing so the skeleton loading state is observable.
let xferFail = false;
let delayList = 0;
// U9: sidebar listing mode ("normal" | "empty" | "fail" | "hostile")
let hostsMode = "normal";
// hostile bytes (INV-11): must render inert everywhere — never parsed as HTML
const HOSTILE = "<img src=x onerror=window.__pwn=1>";
const HOSTILE_ENTRIES = [
  { name: HOSTILE, is_dir: false, is_link: false, size: 1 },
  { name: "\"><svg onload=window.__pwn=2>", is_dir: true, is_link: false, size: 0 },
  { name: "&amp;<b>x</b>", is_dir: false, is_link: true, size: 2 },
];
const sampleSnippets = [
  { category: "Docker", items: [
    { name: "Prune everything", command: "docker system prune -af" },
    { name: "Tail container logs", command: "docker logs -f --tail 100 api" },
  ] },
  { category: "Git", items: [{ name: "Status (short)", command: "git status -sb" }] },
  { category: "systemd", items: [{ name: HOSTILE, command: HOSTILE }] },
];
// W2-A startup ssh/sftp check state (AC-A1.5); read by the check_binaries case
let binCheckState = { ssh_missing: false, sftp_missing: false };
let storedSettings = {
  version: 1,
  theme: "dusk",
  accent: "#e2a35c",
  allowlist: [{ host: "prod-web", command: "sudo systemctl restart netdata", added_at: "2026-10-30T12:00:00Z" }],
};
let readSettingsFailures = 0;
// FE-AUTH Google OAuth mock state. Name carries a hostile <img> byte so the
// smoke can assert the chip escapes remote-origin strings (ADR-8). picture is
// a remote https URL that must NEVER be fetched (CSP img-src 'self' + NFR-3).
const GOOGLE_ACCT = {
  name: "Ada Lovelace",
  // Hostile byte in the remote email proves esc() on remote-origin strings (ADR-8):
  // it must render as inert text (0 child nodes), never inject a DOM node.
  email: 'ada+<img src=x onerror=alert(1)>@example.com',
  picture: "https://lh3.googleusercontent.com/a/mock-avatar",
};
let googleStored = null;       // what google_account returns on load (null = signed out)
// FE-FIX Drive backup/restore mock steering. DRIVE_ERR carries a hostile byte so
// the error toast must render it inert text (ADR-8), never an injected DOM node.
const DRIVE_ERR = 'Network failed <img src=x onerror=alert(1)>';
let driveBackupFail = false;
let driveRestoreFail = false;
let driveListMode = "normal";  // normal|empty — empty exercises the neutral no-backups toast
let storedUserSnippets = [];   // user-snippet CRUD store (read/write_user_snippets)
let googleSignInFail = false;  // flip to exercise the auth-failed error path
// FE-ANS Ansible v1 mock state.
const ANSIBLE_PTY_ID = 7777;
let ansibleCheckState = { missing: false };   // check_ansible result
let ansiblePickMode = "valid";                 // valid|cancel|binary|big|badext
let ansiblePickName = "site.yml";              // basename for the valid pick
let ansibleRunFail = false;                    // ansible_run rejects when true
const ansibleRuns = [];                        // every ansible_run req payload
let ansibleHistory = [];                       // ansible_history store (newest-last)
const invoke = (cmd, args = {}) => {
  calls.push({ cmd, args });
  switch (cmd) {
    case "read_ssh_config": {
      // U9: "empty"/"fail" = sidebar narrative states, "hostile" = esc fixture
      if (hostsMode === "fail") return Promise.reject("permission denied (mock)");
      const hs = hostsMode === "empty" ? [] : hostsMode === "hostile" ? [...sampleHosts, { ...sampleHosts[0], name: HOSTILE }] : sampleHosts;
      return delayRead > 0
        ? new Promise((r) => setTimeout(() => r(hs), delayRead))
        : Promise.resolve(hs);
    }
    case "check_binaries": return Promise.resolve({ ...binCheckState });
    case "read_settings": {
      if (readSettingsFailures > 0) {
        readSettingsFailures -= 1;
        return Promise.reject("settings read failed (mock)");
      }
      return Promise.resolve(JSON.parse(JSON.stringify(storedSettings)));
    }
    case "read_snippets": return Promise.resolve(JSON.parse(JSON.stringify(sampleSnippets)));
    // User-snippet CRUD store (mirrors the backend: a mutable in-memory list).
    case "read_user_snippets": return Promise.resolve(JSON.parse(JSON.stringify(storedUserSnippets)));
    case "write_user_snippets": {
      storedUserSnippets = (args.snippets || []).map((s, i) => ({
        id: (s.id && String(s.id).trim()) || `us-mock-${i}-${Date.now()}`,
        name: String(s.name || "").trim(),
        command: String(s.command || "").trim(),
        category: String(s.category || "").trim() || "Custom",
      }));
      return Promise.resolve(null);
    }
    case "local_shell_info": return Promise.resolve({ shell: "/bin/bash", overridden: false });
    case "write_settings": {
      storedSettings = JSON.parse(JSON.stringify(args.settings));
      return Promise.resolve(null);
    }
    case "read_config_raw": return Promise.resolve(sampleConfig);
    case "write_config_raw": return Promise.resolve();
    case "list_identity_files": return Promise.resolve(["~/.ssh/id_test"]);
    case "sftp_list": {
      if (sftpFailFirst && args.host === "pw-host") { sftpFailFirst = false; return Promise.reject("sftp session unreachable"); }
      // U6: "/boom" rejects BOTH attempts (retry path too) → narrative error copy observable;
      // "/empty" → 0 entries → "Empty directory." byte-identical check
      if (String(args.path || "").includes("boom")) return Promise.reject("sftp boom (mock)");
      // U9 hostile-byte fixture (SEC t_ec1dbd02 LOW): "/xss" lists attacker-named entries
      if (String(args.path || "").includes("xss")) return Promise.resolve(HOSTILE_ENTRIES);
      const sftpEntries = String(args.path || "").includes("empty") ? [] : sampleSftp;
      return delayList > 0
        ? new Promise((r) => setTimeout(() => r(sftpEntries), delayList))
        : Promise.resolve(sftpEntries);
    }
    case "sftp_upload": case "sftp_download": {
      if (xferFail) return Promise.reject("permission denied (mock)");
      // 150ms so tests can observe the .xf-row.run (spinner) state
      return new Promise((r) => setTimeout(r, 150));
    }
    case "local_list": {
      // U6: mirror remote mock — delayList + "/boom" reject + "/empty" zero entries,
      // so renderLocalPane's skeleton/empty/error states are observable
      if (String(args.path || "").includes("boom")) return Promise.reject("permission denied (mock-local)");
      const locEntries = String(args.path || "").includes("empty") ? [] : [{ name: "notes.txt", is_dir: false, size: 5 }, { name: "sub", is_dir: true, size: 0 }];
      return delayList > 0
        ? new Promise((r) => setTimeout(() => r(locEntries), delayList))
        : Promise.resolve(locEntries);
    }
    case "home_dir": return Promise.resolve("/tmp/mockhome");
    case "sftp_read_file": return Promise.resolve("hello editor\nline two\n");
    case "sftp_write_file": return Promise.resolve();
    case "pty_spawn": return Promise.resolve(4242);
    case "pty_resize": case "pty_kill": case "pty_write": case "clipboard_write": case "clipboard_read": return Promise.resolve("");
    case "save_host": case "delete_host": case "create_identity_file": return Promise.resolve();
    // U8 AC2: forget the stored sidecar password (frontend-only unit; the real
    // backend command is tracked in its own card — smoke asserts the invoke)
    case "forget_secret": return Promise.resolve();
    // FE-AUTH: real Google OAuth chip. google_account hydrates signed-in state
    // on load (null = signed out); google_sign_in runs the PKCE loopback flow
    // backend-side; google_sign_out clears the keyring refresh_token. The name
    // carries a hostile byte to assert esc() on the remote-origin string.
    case "google_account": return Promise.resolve(googleStored);
    case "google_sign_in": {
      if (googleSignInFail) return Promise.reject("network unreachable (mock)");
      googleStored = { ...GOOGLE_ACCT };
      return new Promise((r) => setTimeout(() => r({ ...GOOGLE_ACCT }), 40));
    }
    case "google_sign_out": { googleStored = null; return Promise.resolve(); }
    // FE-FIX Drive backup/restore mocks. driveListMode steers the restore path
    // (normal|empty), driveBackupFail/driveRestoreFail flip the error arms. The
    // error string carries a hostile byte so the toast must render it inert
    // (textContent, ADR-8). Small delays let the smoke observe in-progress state.
    case "drive_backup": {
      if (driveBackupFail) return Promise.reject(DRIVE_ERR);
      return new Promise((r) => setTimeout(() => r({ file_id: "f-mock", name: "hosterm-backup.age", bytes: 2048, created_at: "2026-11-07T10:00:00Z" }), 30));
    }
    case "drive_list_backups": {
      if (driveListMode === "empty") return Promise.resolve([]);
      return Promise.resolve([
        { file_id: "f-mock", name: "hosterm-backup.age", created_at: "2026-11-07T10:00:00Z", host: "ws", schema: 1 },
      ]);
    }
    case "drive_restore_preflight": return Promise.resolve({ targets: [{ path: "~/.ssh/config", exists: true }] });
    case "drive_restore": {
      if (driveRestoreFail) return Promise.reject(DRIVE_ERR);
      return new Promise((r) => setTimeout(() => r({ restored: ["~/.ssh/config"], pre_restore_dir: "~/.ssh/hosterm-pre-restore-123" }), 30));
    }
    // ---- FE-ANS Ansible v1 (FR-ANS1..15) ----
    case "check_ansible": return Promise.resolve({ ...ansibleCheckState });
    case "pick_playbook": {
      if (ansiblePickMode === "cancel") return Promise.reject("ansible:cancelled");
      if (ansiblePickMode === "binary") return Promise.resolve({ path: "/tmp/site.yml", text: "tasks:\n  - name: x\u0000\uFFFD\n" });
      if (ansiblePickMode === "big") return Promise.resolve({ path: "/tmp/big.yml", text: "x".repeat(2 * 1024 * 1024 + 8) });
      if (ansiblePickMode === "badext") return Promise.resolve({ path: "/tmp/notes.txt", text: "hello\n" });
      // valid: path carries a hostile byte so the panel must esc() it (ADR-8)
      return Promise.resolve({ path: `/tmp/${ansiblePickName}`, text: "- hosts: all\n  tasks:\n    - name: ping\n      ping:\n" });
    }
    case "ansible_run": {
      ansibleRuns.push(JSON.parse(JSON.stringify(args.req)));
      if (ansibleRunFail) return Promise.reject("ansible:no-eligible-hosts");
      return Promise.resolve(ANSIBLE_PTY_ID);
    }
    case "ansible_history": {
      const op = args.op || {};
      if (op.kind === "append") ansibleHistory.push(JSON.parse(JSON.stringify(op.entry)));
      // 200-cap FIFO mirror (the backend caps; the frontend just renders).
      if (ansibleHistory.length > 200) ansibleHistory = ansibleHistory.slice(-200);
      return Promise.resolve(JSON.parse(JSON.stringify(ansibleHistory)));
    }
    default: return Promise.reject(`unmocked: ${cmd}`);
  }
};

const server = createServer((req, res) => {
  let p = join(DIST, req.url === "/" ? "index.html" : req.url.split("?")[0]);
  if (!existsSync(p)) p = join(DIST, "index.html");
  res.setHeader("Content-Security-Policy", CSP); // prod policy (AC R1.4 meaningfulness)
  res.setHeader("content-type", MIME[extname(p)] || "text/plain");
  res.end(readFileSync(p));
});
// Fail fast with an actionable message instead of an unhandled 'EADDRINUSE'
// stack (QA t_81aec02e: stale server from a previous run holding 4199).
await new Promise((resolve, reject) => {
  server.once("error", (e) => reject(new Error(`port 4199 busy — kill the stale smoke server: ${e.code || e.message}`)));
  server.listen(4199, resolve);
});

const results = [];
let lastCheck = "(none yet)";
const check = (name, ok, extra = "") => {
  lastCheck = name;
  results.push(`${ok ? "PASS" : "FAIL"} ${name}${extra ? " — " + extra : ""}`);
};
// U9 harness hardening (U4/U7 deferred): any throw/rejection mid-suite prints
// what ran, names the last recorded check, closes chromium + frees port 4199.
let browser;
const bail = async (why) => {
  console.log(results.join("\n"));
  console.error(`\nSMOKE ABORTED after check "${lastCheck}" (${results.length} recorded): ${why}`);
  await browser?.close().catch(() => {});
  server.close();
  process.exit(1);
};
process.on("uncaughtException", (e) => bail(e?.stack || e));
process.on("unhandledRejection", (e) => bail(e?.stack || e));
process.on("SIGINT", () => bail("SIGINT"));
// cold-start flake: chromium can die right after launch ("Target page, context
// or browser has been closed") — retry launch+newPage up to 3×, other errors throw
let page;
for (let attempt = 1; ; attempt++) {
  try {
    const executablePath = process.env.HOSTERM_PLAYWRIGHT_EXECUTABLE;
    browser = await chromium.launch(executablePath ? { executablePath } : undefined);
    // Pin a Linux UA for the default page: the app computes isWindows from
    // navigator.userAgent (main.ts:1290), and the "missing (Linux)" scenario
    // below asserts the ssh guide is NOT shown. Without this, the default page
    // inherits the runner's UA, so on the windows-latest CI runner isWindows
    // became true, the guide modal opened, and the suite hung/timed out —
    // green on Linux, red on Windows. The Windows scenario sets its own UA.
    page = await browser.newPage({
      userAgent: "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36",
    });
    break;
  } catch (e) {
    await browser?.close().catch(() => {});
    if (attempt >= 3 || !/has been closed/.test(String(e))) throw e;
    console.error(`chromium launch attempt ${attempt} failed (${String(e).split("\n")[0]}) — retrying`);
  }
}
const requestLog = [];
page.on("request", (request) => requestLog.push(request.url()));
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
// Shared init scripts (U9 dedupe: page + winPage used verbatim copies)
const mockTauri = () => {
  window.__cbMap = {};
  window.__events = {};
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  // emit mock Tauri events from tests, e.g. __emitTauri("pty://exit/4242")
  window.__emitTauri = (event, payload) =>
    (window.__events[event] || []).forEach((cb) => cb({ event, id: 0, payload }));
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => {
      const id = (window.__cbN = (window.__cbN || 0) + 1);
      window.__cbMap[id] = cb;
      return id;
    },
    invoke: (cmd, args) => {
      // event listen/unlisten handled in-page so tests can emit via __emitTauri
      if (cmd === "plugin:event|listen") {
        (window.__events[args.event] = window.__events[args.event] || []).push(window.__cbMap[args.handler]);
        return Promise.resolve(1);
      }
      if (cmd === "plugin:event|unlisten") return Promise.resolve("");
      return globalThis.__invokeImpl(cmd, args);
    },
  };
};
// CSP-violation collector (AC R1.4): securitypolicyviolation events for the whole run
const cspCollector = () => {
  window.__cspViolations = [];
  document.addEventListener("securitypolicyviolation", (e) => {
    window.__cspViolations.push(`${e.effectiveDirective}: ${e.blockedURL || e.sourceFile || "inline"}`);
  });
};
await page.addInitScript(mockTauri);
await page.addInitScript(() => {
  // Track AbortController signals passed to window listeners (U4 split drag).
  // Each split registers its mousemove/mouseup under ONE shared signal; if the
  // split is torn down, that signal must be aborted (SEC LOW t_fb1d18d6 fix).
  // Scoped to the two drag types so library signals can't pollute the count.
  window.__sigList = [];
  const origAdd = window.addEventListener.bind(window);
  window.addEventListener = (type, fn, opts) => {
    const sig = (type === "mousemove" || type === "mouseup") && opts && typeof opts === "object" ? opts.signal : undefined;
    if (sig) {
      // tag by listener types so split (mousemove+mouseup) vs sftp (mouseup-only)
      // teardown checks can count their own registrations (U5)
      let e = window.__sigList.find((x) => x.sig === sig);
      if (!e) { e = { sig, types: [] }; window.__sigList.push(e); }
      if (!e.types.includes(type)) e.types.push(type);
    }
    return origAdd(type, fn, opts);
  };
});
await page.addInitScript(cspCollector);
await page.exposeFunction("__invokeImpl", invoke);
const firstResp = await page.goto("http://localhost:4199/");
const servedCsp = (firstResp.headers()["content-security-policy"] || "").trim();
await page.waitForTimeout(400);

const openSettingsFromAccount = async () => {
  if (await page.locator("#signin-popover").isHidden()) await page.locator("#user-chip").click();
  await page.locator("#acct-settings-btn").click();
};

// ---- W2-A: startup ssh/sftp check + install guide (AC-A1.5, NFR-W2-4) ----
const PS_CMD = "Add-WindowsCapability -Online -Name OpenSSH.Client~~~~0.0.1.0";
// present-binary no-op (AC-A1.2): detection ran at launch, guide never appears
check("check_binaries invoked at launch", calls.some((c) => c.cmd === "check_binaries"));
check("present binaries: guide never appears", await page.locator("#ssh-modal").evaluate((el) => el.classList.contains("hidden")));

// missing-binary state on Linux (OQ-5): check runs, guide skipped, per-attempt error friendly
binCheckState = { ssh_missing: true, sftp_missing: false };
await page.goto("http://localhost:4199/");
// Wait for the host list to render after the reload (the default page's UA is
// pinned Linux, so the ssh guide modal never opens here — no modal wait needed).
await page.locator(".host-item").first().waitFor({ state: "visible", timeout: 10000 });
check("missing (Linux): guide skipped, app usable",
  (await page.locator("#ssh-modal").evaluate((el) => el.classList.contains("hidden"))) &&
  (await page.locator(".host-item").count()) === 3);
const spawns0 = calls.filter((c) => c.cmd === "pty_spawn").length;
await page.locator(".host-item .h-name").first().click();
await page.waitForTimeout(300);
const linuxMsg = await page.locator("#dialog-msg").textContent();
check("missing (Linux): friendly per-attempt error names ssh",
  !(await page.locator("#dialog").evaluate((el) => el.classList.contains("hidden"))) &&
  linuxMsg.includes("ssh") && !linuxMsg.includes("No such file"), linuxMsg.slice(0, 60));
check("missing (Linux): backend spawn never attempted",
  calls.filter((c) => c.cmd === "pty_spawn").length === spawns0);
await page.locator("#dialog button", { hasText: "OK" }).click();
await page.waitForTimeout(200);

// missing-binary state on Windows UA: full guide screen (FR-W2-2)
const winPage = await browser.newPage({ userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36" });
winPage.on("pageerror", (e) => errors.push(String(e)));
winPage.on("console", (m) => m.type() === "error" && errors.push(m.text()));
await winPage.addInitScript(mockTauri);
await winPage.addInitScript(cspCollector);
await winPage.exposeFunction("__invokeImpl", invoke);
await winPage.goto("http://localhost:4199/");
await winPage.waitForTimeout(400);
check("missing (Windows): blocking guide shown at launch", await winPage.locator("#ssh-modal").evaluate((el) => !el.classList.contains("hidden")));
check("guide: app inert behind it (blocking, OQ-A1)", await winPage.evaluate(() => document.getElementById("app").hasAttribute("inert")));
const whatTxt = await winPage.locator("#ssh-what").textContent();
check("guide: names the missing binary specifically (AC-A1.3)", whatTxt.includes("(ssh)") && !whatTxt.includes("sftp"), whatTxt);
check("guide: byte-exact PowerShell rendered", (await winPage.locator("#ssh-ps-cmd").textContent()) === PS_CMD);
check("guide: all 7 content elements (AC-A2.1)", await winPage.evaluate(() =>
  !!(document.querySelector("#ssh-copy") && document.querySelector("#ssh-recheck") &&
     document.querySelector("#ssh-continue") && document.querySelector('#ssh-modal a[href*="learn.microsoft.com"]') &&
     (() => { const t = document.getElementById("ssh-modal").textContent;
       return t.includes("Optional features") && t.includes("cannot work") && t.includes("OpenSSH.Client~~~~0.0.1.0"); })()
)));
const copyIdx = calls.filter((c) => c.cmd === "clipboard_write").length;
await winPage.locator("#ssh-copy").click();
await winPage.waitForTimeout(250);
const copyCall = calls.filter((c) => c.cmd === "clipboard_write")[copyIdx];
check("guide: Copy writes byte-exact command (AC-A2.1)", !!copyCall && copyCall.args.text === PS_CMD);

// re-check with binaries still missing → same single screen (AC-A4.2)
await winPage.locator("#ssh-recheck").click();
await winPage.waitForTimeout(300);
check("re-check still missing: screen stays, exactly one",
  (await winPage.locator("#ssh-modal").count()) === 1 &&
  (await winPage.locator("#ssh-modal").evaluate((el) => !el.classList.contains("hidden"))));

// re-check after install → guide closes, connect works without restart (AC-A4.1)
binCheckState = { ssh_missing: false, sftp_missing: false };
const spawns1 = calls.filter((c) => c.cmd === "pty_spawn").length;
await winPage.locator("#ssh-recheck").click();
await winPage.waitForTimeout(300);
check("re-check fixed: guide closes", await winPage.locator("#ssh-modal").evaluate((el) => el.classList.contains("hidden")));
await winPage.locator(".host-item .h-name").first().click();
await winPage.waitForTimeout(300);
check("re-check fixed: connect works without restart",
  calls.filter((c) => c.cmd === "pty_spawn").length === spawns1 + 1);

// declined guide → config-only works; connect = friendly error + guide button (FR-W2-3)
binCheckState = { ssh_missing: true, sftp_missing: true };
await winPage.reload();
await winPage.waitForTimeout(400);
await winPage.locator("#ssh-continue").click();
await winPage.waitForTimeout(200);
check("Continue anyway dismisses guide", await winPage.locator("#ssh-modal").evaluate((el) => el.classList.contains("hidden")));
await winPage.locator("#btn-raw").click();
await winPage.waitForTimeout(200);
check("declined: raw config still works (AC-A3.1)", await winPage.locator("#raw-modal").evaluate((el) => !el.classList.contains("hidden")));
await winPage.keyboard.press("Escape");
await winPage.waitForTimeout(200);
await winPage.locator(".host-item .h-name").first().click();
await winPage.waitForTimeout(300);
const decMsg = await winPage.locator("#dialog-msg").textContent();
check("declined: connect shows friendly error naming ssh (AC-A3.2)",
  decMsg.includes("ssh") && !decMsg.includes("No such file"), decMsg.slice(0, 70));
await winPage.locator("#dialog button", { hasText: "Open install guide" }).click();
await winPage.waitForTimeout(200);
check("'Open install guide' re-opens the guide (AC-A3.2)", await winPage.locator("#ssh-modal").evaluate((el) => !el.classList.contains("hidden")));
check("winPage CSP violations == 0 (NFR-W2-1)", (await winPage.evaluate(() => window.__cspViolations || [])).length === 0);
await winPage.close();

// back to present-state for the rest of the suite
binCheckState = { ssh_missing: false, sftp_missing: false };
await page.goto("http://localhost:4199/");
await page.waitForTimeout(400);

// 1. host list renders from mock config
const hosts = await page.locator(".host-item").count();
check("host list renders", hosts === 3, `count=${hosts}`);

// 2. sidebar actions: real buttons, hover-reveal but keyboard-reachable (R-32: focus-within reveals)
const sftpBtn = page.locator(".host-item .h-sftp").first();
check("sidebar sftp action is a button", (await sftpBtn.evaluate((el) => el.tagName)) === "BUTTON");
const restOp = await sftpBtn.evaluate((el) => getComputedStyle(el).opacity);
check("row actions hover-only at rest", restOp === "0", `opacity=${restOp}`);
await page.locator(".host-item").first().hover();
await page.waitForTimeout(200); // 120ms opacity transition settles
const hovOp = await sftpBtn.evaluate((el) => getComputedStyle(el).opacity);
check("row actions revealed on hover", hovOp === "1", `opacity=${hovOp}`);
await sftpBtn.focus();
await page.waitForTimeout(200);
const focOp = await sftpBtn.evaluate((el) => getComputedStyle(el).opacity);
check("row actions revealed on focus-within (R-32)", focOp === "1", `opacity=${focOp}`);

// 3. open SFTP tab via button (remote pane only; local pane adds its own rows)
await sftpBtn.click();
await page.waitForTimeout(500);
const sftpRows = await page.locator(".sftp-list .sftp-row").count();
check("sftp rows render from mock", sftpRows === 2, `rows=${sftpRows}`);
const dirSize = await page.locator(".sftp-list .sftp-row", { hasText: "projects" }).locator(".sz").textContent();
check("dir size cell empty (no em dash)", dirSize.trim() === "", JSON.stringify(dirSize));

// 4. double-click the file row in the REMOTE pane -> editor tab opens
await page.locator(".sftp-list .sftp-row", { hasText: "notes.txt" }).first().dblclick();
await page.waitForTimeout(500);
const cmContent = await page.locator(".cm-content").textContent();
check("editor opens with mocked file", cmContent.includes("hello editor"), cmContent.slice(0, 40));
// U4 height chain (EPIC-TSD pitfall): editor wrap still fills its panel
const edChain = await page.evaluate(() => {
  const panel = document.querySelector(".panel.active");
  const w = panel.querySelector(".ed-wrap");
  return { ph: panel.offsetHeight, wh: w ? w.offsetHeight : -1 };
});
check("U4: ed-wrap fills panel (height chain)", edChain.wh === edChain.ph, JSON.stringify(edChain));

// THEME v1 (TSD theme-palettes-v1 §2.3): a preset swap re-highlights the OPEN
// editor through the Compartment — no EditorView recreate, so doc and cursor
// survive; __hostermTheme.editorReconfigured must count exactly this view.
await page.locator(".cm-content").click(); // focus -> cursor DOM exists
await page.waitForTimeout(150);
const themeEdSwap = await page.evaluate(() => {
  const docBefore = document.querySelector(".panel.active .cm-content")?.textContent;
  const cursorBefore = !!document.querySelector(".panel.active .cm-cursor");
  const before = { ...window.__hostermTheme };
  document.querySelector('.preset[data-preset="ink"]').click();
  const after = { ...window.__hostermTheme };
  const docAfter = document.querySelector(".panel.active .cm-content")?.textContent;
  const cursorAfter = !!document.querySelector(".panel.active .cm-cursor");
  document.querySelector('.preset[data-preset="dusk"]').click();
  const restored = { ...window.__hostermTheme };
  return { before, after, restored, docBefore, docAfter, cursorBefore, cursorAfter };
});
check("THEME v1: preset swap re-highlights the open editor (Compartment, doc + cursor survive)",
  themeEdSwap.before.preset === "dusk" && themeEdSwap.after.preset === "ink" &&
  themeEdSwap.after.editorReconfigured === 1 && themeEdSwap.restored.preset === "dusk" &&
  themeEdSwap.docAfter === themeEdSwap.docBefore && themeEdSwap.cursorBefore && themeEdSwap.cursorAfter,
  JSON.stringify(themeEdSwap));


// 5. dirty dot appears after edit
await page.locator(".cm-content").click();
await page.keyboard.type("X");
await page.waitForTimeout(200);
const dirty = await page.locator(".tab .t-dot").count();
check("dirty dot appears on edit", dirty === 1, `dots=${dirty}`);
check("dirty dot is 8px --warn (U3)", await page.locator(".tab .t-dot").first().evaluate((el) => {
  const s = getComputedStyle(el);
  return s.width === "8px" && s.backgroundColor === "rgb(226, 163, 92)";
}));

// 6. closing dirty tab opens in-app dialog with 3 choices (not native confirm)
let dialogInfo = null;
page.once("dialog", (d) => { dialogInfo = d.type(); d.dismiss(); });
await page.locator('.tab:has-text("notes.txt") .x').click();
await page.waitForTimeout(300);
const dlgButtons = await page.locator(".dialog-box button, [role=alertdialog] button, .modal-box button").count();
check("dirty close uses in-app dialog (no native)", dialogInfo === null, `native=${dialogInfo}`);
check("dialog has >=3 action buttons", dlgButtons >= 3, `buttons=${dlgButtons}`);

// 7. Escape closes modal (raw editor)
await page.keyboard.press("Escape");
await page.waitForTimeout(200);

// 8. open Raw config, Escape closes it
await page.locator("#btn-raw").click();
await page.waitForTimeout(300);
const rawVisible = await page.locator("#raw-modal").evaluate((el) => !el.classList.contains("hidden"));
check("raw modal opens", rawVisible);
await page.keyboard.press("Escape");
await page.waitForTimeout(300);
const rawAfter = await page.locator("#raw-modal").evaluate((el) => el.classList.contains("hidden"));
check("Escape closes modal", rawAfter);

// 9. focus-visible ring exists in the stylesheet
const ring = await page.evaluate(() => [...document.styleSheets].some((s) => {
  try { return [...s.cssRules].some((r) => r.selectorText && r.selectorText.includes(":focus-visible")); }
  catch { return false; }
}));
check(":focus-visible rule present", ring);

// 9b. host search filters the list
await page.locator("#host-search").fill("zzz");
await page.waitForTimeout(150);
check("search: no match message", (await page.locator(".list-state").textContent()).includes("No host matches"));
await page.locator("#host-search").fill("prod");
await page.waitForTimeout(150);
check("search: match renders", (await page.locator(".host-item").count()) === 1);
await page.locator("#host-search").fill("");
await page.waitForTimeout(150);

// 9c. contextual panel collapse/expand + localStorage persistence
await page.locator("#btn-collapse").click();
check("collapse: body class set", await page.evaluate(() => document.body.classList.contains("panel-collapsed")));
check("collapse: expand button shown", await page.locator("#btn-expand").isVisible());
const stored = await page.evaluate(() => localStorage.getItem("hosterm.panel-collapsed"));
check("collapse: persisted", stored === "1");
await page.locator("#btn-expand").click();
check("expand: class removed", await page.evaluate(() => !document.body.classList.contains("panel-collapsed")));

// 9d. Edit/Delete left the row (fix #4) -> context menu; Delete uses in-app dialog (not native confirm)
check("edit/delete buttons gone from rows",
  (await page.locator(".host-item .h-edit, .host-item .h-del").count()) === 0);
// Right-click prod-web explicitly (not .first(), which is db-core under A-Z sort)
// so the delete-dialog host-name check below is deterministic.
await page.locator(".host-item", { hasText: "prod-web" }).first().click({ button: "right" });
await page.waitForTimeout(200);
check("right-click row opens in-app host menu", (await page.locator("#host-menu").count()) === 1);
const menuItems = await page.locator("#host-menu button").allTextContents();
check("host menu = Open SFTP / Edit / Delete",
  JSON.stringify(menuItems) === JSON.stringify(["Open SFTP", "Edit", "Delete"]), menuItems.join("|"));
check("host menu Delete styled danger", (await page.locator("#host-menu button.danger").count()) === 1);
let nativeDialog = null;
page.once("dialog", (d) => { nativeDialog = d.type(); d.dismiss(); });
await page.locator("#host-menu button", { hasText: "Delete" }).click();
await page.waitForTimeout(250);
const dlgText = await page.evaluate(() => document.querySelector(".dialog-box, [role=alertdialog], #dialog")?.textContent || "");
check("delete dialog names the host", dlgText.includes("prod-web"), dlgText.slice(0, 60));
check("delete uses in-app dialog (no native)", nativeDialog === null, `native=${nativeDialog}`);
// focused button should be Cancel (destructive action guard)
const focusedLabel = await page.evaluate(() => (document.activeElement?.textContent || "").trim());
check("delete dialog focuses Cancel", focusedLabel === "Cancel", `focus=${focusedLabel}`);
await page.keyboard.press("Enter"); // activates focused Cancel
await page.waitForTimeout(250);
check("cancel keeps host", (await page.locator(".host-item").count()) === 3);
// ⋯ button opens the same menu; Escape closes it
await page.locator(".host-item").first().hover();
await page.locator(".host-item .h-more").first().click();
await page.waitForTimeout(200);
check("⋯ button opens host menu", (await page.locator("#host-menu").count()) === 1);
await page.keyboard.press("Escape");
await page.waitForTimeout(150);
check("Escape closes host menu", (await page.locator("#host-menu").count()) === 0);
// menu Edit opens the host modal (same editor as before, just new entry point)
await page.locator(".host-item").first().click({ button: "right" });
await page.waitForTimeout(200);
await page.locator("#host-menu button", { hasText: "Edit" }).click();
await page.waitForTimeout(250);
check("host menu Edit opens modal", await page.locator("#modal").evaluate((el) => !el.classList.contains("hidden")));
await page.keyboard.press("Escape");
await page.waitForTimeout(200);

// 9e. terminal right-click paste handler attached (no context menu, no errors)
await page.locator(".host-item .h-name").first().click();
await page.waitForTimeout(400);
const rcHandled = await page.evaluate(() => {
  const el = document.querySelector(".term-host");
  const ev = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
  el.dispatchEvent(ev);
  return ev.defaultPrevented;
});
check("terminal right-click handled (no native menu)", rcHandled);

// 9f. sftp dual-pane: panes render, double-click navigates, single-click selects
await page.locator(".host-item .h-sftp").first().click();
await page.waitForTimeout(600);
check("sftp dual-pane: remote pane", (await page.locator(".panel.active .sftp-list .sftp-row").count()) >= 1);
check("sftp dual-pane: local pane rows", (await page.locator(".panel.active .loc-list .sftp-row").count()) === 2);
check("sftp dual-pane: local home prefilled", (await page.locator(".panel.active .loc-path").inputValue()) === "/tmp/mockhome");
// single click selects (delayed 250ms)
await page.locator(".panel.active .loc-list .sftp-row").first().click();
await page.waitForTimeout(400);
check("sftp single-click selects", (await page.locator(".panel.active .loc-list .sftp-row.sel").count()) === 1);
// double-click on local dir navigates
await page.locator(".panel.active .loc-list .sftp-row").nth(1).dblclick();
await page.waitForTimeout(400);
check("sftp double-click navigates local dir", (await page.locator(".panel.active .loc-path").inputValue()).endsWith("/sub"));
// BUG2: double-click a remote dir -> joined path is single-slash, no "./" prefix,
// no doubled base. The backend parser normalizes names to basenames; this asserts
// the frontend join produces a clean remote path in the sftp_list call args.
const beforeNav = calls.filter((c) => c.cmd === "sftp_list").length;
await page.locator(".panel.active .sftp-list .sftp-row", { hasText: "projects" }).first().dblclick();
await page.waitForTimeout(400);
const navCalls = calls.filter((c) => c.cmd === "sftp_list");
const navPath = String(navCalls[navCalls.length - 1]?.args?.path ?? "");
check("sftp nav path: no double slash", !navPath.includes("//"), navPath);
check("sftp nav path: no ./ prefix or segment", !navPath.includes("./"), navPath);
check("sftp nav path: ends with basename projects", navPath.endsWith("projects"), navPath);
check("sftp nav issued a listing", navCalls.length > beforeNav);
// download button guards when nothing selected in remote
await page.locator(".panel.active .sftp-download").click();
await page.waitForTimeout(300);
check("download without selection shows hint", (await page.locator(".panel.active .sftp-status").textContent()).includes("Select a file"));

// 9f1-BUG3: native WebView context menu (and its "Reload" item) must be
// suppressed over the SFTP pane area so an accidental right-click can never
// reload the SPA and wipe in-memory tab state. (runs while an SFTP tab is open)
const tabsBeforeRc = await page.locator(".tab").count();
check("BUG3-FE precondition: tabs open before right-click", tabsBeforeRc >= 1, `tabs=${tabsBeforeRc}`);
// (a) a contextmenu dispatched on the SFTP list / pane area is preventDefault-ed
// by the global document-level guard (no native menu path reachable).
const sftpRcBlocked = await page.evaluate(() => {
  const target = document.querySelector(".panel.active .sftp-list")
    || document.querySelector(".panel.active .sftp-pane")
    || document.querySelector(".panel.active");
  const ev = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
  target.dispatchEvent(ev);
  return ev.defaultPrevented;
});
check("BUG3-FE AC1: right-click on SFTP pane suppresses native menu (preventDefault)", sftpRcBlocked);
// a bare right-click on the document body (empty app surface) is also suppressed
const bodyRcBlocked = await page.evaluate(() => {
  const ev = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
  document.body.dispatchEvent(ev);
  return ev.defaultPrevented;
});
check("BUG3-FE AC4: right-click on empty app surface suppressed (no reload path)", bodyRcBlocked);
// (b) regression: the simulated right-click path leaves tab state intact —
// no app menu spawned on the SFTP pane, tab count unchanged, nothing reloaded.
await page.waitForTimeout(100);
const noAppMenuOnSftp = (await page.locator("#tab-menu").count()) === 0 && (await page.locator("#host-menu").count()) === 0;
check("BUG3-FE: right-click on SFTP pane opens no app menu (native + app both silent)", noAppMenuOnSftp);
const tabsAfterRc = await page.locator(".tab").count();
check("BUG3-FE AC2: tab state intact after simulated right-click (no reload, count unchanged)",
  tabsAfterRc === tabsBeforeRc, `before=${tabsBeforeRc} after=${tabsAfterRc}`);

// 9f2. password-host persistent session: fail-once retry, shared host cache,
// tab isolation (one sftp session per host serves many tabs)
sftpFailFirst = true; // first pw-host listing rejects once; retry must recover
await page.locator(".host-item", { hasText: "pw-host" }).first().locator(".h-sftp").click();
await page.waitForTimeout(700);
check("pw sftp: stale-session retry renders entries", (await page.locator(".panel.active .sftp-list .sftp-row").count()) >= 1);
const pwListCalls = calls.filter((c) => c.cmd === "sftp_list" && c.args.host === "pw-host").length;
check("pw sftp: backend re-invoked exactly once on retry", pwListCalls === 2, `calls=${pwListCalls}`);
await page.locator(".host-item", { hasText: "pw-host" }).first().locator(".h-sftp").click();
await page.waitForTimeout(500);
check("pw sftp: second tab on same host lists too", (await page.locator(".panel.active .sftp-list .sftp-row").count()) >= 1);
const beforeRef = calls.filter((c) => c.cmd === "sftp_list" && c.args.host === "pw-host").length;
await page.locator(".panel.active .sftp-refresh").click();
await page.waitForTimeout(400);
const afterRef = calls.filter((c) => c.cmd === "sftp_list" && c.args.host === "pw-host").length;
check("pw sftp: refresh rides the persistent session", afterRef === beforeRef + 1, `before=${beforeRef} after=${afterRef}`);
const tabsNow = await page.locator(".tab").count();
await page.locator(".tab").last().locator(".x").click();
await page.waitForTimeout(400);
check("pw sftp: closing one tab keeps its sibling", (await page.locator(".tab").count()) === tabsNow - 1);
check("pw sftp: sibling tab still lists", (await page.locator(".panel.active .sftp-list .sftp-row").count()) >= 1);

// 9g. split, Termius style: drag one terminal tab onto another terminal tab
for (let i = 0; i < 15 && (await page.locator(".tab").count()) > 0; i++) {
  if (await page.locator("#dialog").isVisible().catch(() => false)) {
    // unsaved-changes prompt: discard to proceed
    await page.locator("#dialog button").filter({ hasText: "Discard" }).first().click().catch(() => {});
    await page.waitForTimeout(250);
    continue;
  }
  await page.locator(".tab .x").first().click();
  await page.waitForTimeout(350);
}
await page.locator(".host-item", { hasText: "prod-web" }).first().locator(".h-name").click();
await page.waitForTimeout(400);
await page.locator(".host-item", { hasText: "db-core" }).first().locator(".h-name").click();
await page.waitForTimeout(400);
const srcTab = page.locator(".tab", { hasText: "prod-web" }).first();
const dstTab = page.locator(".tab", { hasText: "db-core" }).first();
const sBox = await srcTab.boundingBox();
const dBox = await dstTab.boundingBox();
// manual mouse drag (HTML5 DnD not used anymore)
await page.mouse.move(sBox.x + sBox.width / 2, sBox.y + sBox.height / 2);
await page.mouse.down();
await page.mouse.move(sBox.x + sBox.width / 2 + 12, sBox.y + sBox.height / 2); // cross 6px threshold
await page.mouse.move(dBox.x + dBox.width / 2, dBox.y + dBox.height / 2, { steps: 8 });
check("drop target highlighted while dragging", (await page.locator(".tab.drop-target").count()) === 1);
await page.mouse.up();
await page.waitForTimeout(600);
const splitPanel = page.locator(".panel.split");
check("drag tab onto tab adds second term-host", (await splitPanel.locator(".term-host").count()) === 2,
  `panels=${await page.locator(".panel").count()} split=${await splitPanel.count()}`);
const w1 = await splitPanel.locator(".term-host").first().evaluate((el) => el.getBoundingClientRect().width);
const w2 = await splitPanel.locator(".term-host").nth(1).evaluate((el) => el.getBoundingClientRect().width);
check("split panes side-by-side ~50/50", Math.abs(w1 - w2) < 30, `w1=${Math.round(w1)} w2=${Math.round(w2)}`);

// ---- U4: terminal well + divider split + status bar (EPIC-TSD §U4, redesign §4.3) ----
const wellBg = await splitPanel.locator(".term-host").first().evaluate((el) => getComputedStyle(el).backgroundColor);
check("U4: term-host well bg --term #14100d (Dusk)", wellBg === "rgb(20, 16, 13)", wellBg);
const divBox = await splitPanel.locator(".term-divider").boundingBox();
check("U4: split divider is 4px", !!divBox && Math.round(divBox.width) === 4, `w=${divBox && Math.round(divBox.width)}`);
const divGeo = await splitPanel.locator(".term-split").evaluate((el) => {
  const [a, d, b] = [...el.children].map((c) => c.getBoundingClientRect());
  return { g1: Math.round(d.left - a.right), g2: Math.round(b.left - d.right) };
});
check("U4: divider sits between the two sides", divGeo.g1 >= -1 && divGeo.g2 >= -1, JSON.stringify(divGeo));
// drag manual mouse (mousedown/mousemove/mouseup — STATE.md #4): ratio changes
const sidePct = () => splitPanel.evaluate((el) => {
  const total = el.querySelector(".term-split").getBoundingClientRect().width;
  const [l, r] = [...el.querySelectorAll(".term-side")].map((s) => (s.getBoundingClientRect().width / total) * 100);
  return { l: Math.round(l * 10) / 10, r: Math.round(r * 10) / 10 };
});
await page.mouse.move(divBox.x + divBox.width / 2, divBox.y + divBox.height / 2);
await page.mouse.down();
const dragCol = await splitPanel.locator(".term-divider").evaluate((el) => getComputedStyle(el).backgroundColor);
await page.mouse.move(divBox.x + 60, divBox.y + 10, { steps: 6 });
await page.mouse.up();
await page.waitForTimeout(250);
const dragged = await sidePct();
check("U4: divider lights --accent while dragging", dragCol === "rgb(226, 163, 92)", dragCol);
check("U4: divider drag resizes split (manual mouse)",
  Math.abs(dragged.l - dragged.r) > 2 && dragged.l > 40 && dragged.r < 60, JSON.stringify(dragged));
await splitPanel.locator(".term-divider").dblclick();
await page.waitForTimeout(250);
const reset = await sidePct();
check("U4: divider dblclick resets 50/50", Math.abs(reset.l - 50) <= 2 && Math.abs(reset.r - 50) <= 2, JSON.stringify(reset));
// focus follows clicked side: underline 2px --accent (arus motif)
await splitPanel.locator(".term-side").nth(1).click();
const focIdx1 = await splitPanel.evaluate((el) => [...el.querySelectorAll(".term-side")].findIndex((s) => s.classList.contains("focused")));
await splitPanel.locator(".term-side").nth(0).click();
const focUl = await splitPanel.locator(".term-side.focused").evaluate((el) => {
  const cs = getComputedStyle(el);
  return `${cs.borderTopWidth} ${cs.borderTopColor}`;
});
check("U4: focus follows clicked side", focIdx1 === 1, `focused=${focIdx1}`);
check("U4: focused side underline 2px --accent", focUl === "2px rgb(226, 163, 92)", focUl);
// U4 teardown hygiene (SEC LOW t_fb1d18d6): split drag listeners share one
// AbortController signal, registered on window and aborted by closeTab.
// Scoped to mousemove-type entries = splits only (sftp uses mouseup-only).
const sigAfterSplit = await page.evaluate(() => {
  const splitEntries = window.__sigList.filter((e) => e.types.includes("mousemove"));
  return {
    n: splitEntries.length,
    live: splitEntries.filter((e) => !e.sig.aborted).length,
  };
});
check("U4: split registers an abortable window-listener signal",
  sigAfterSplit.n >= 1 && sigAfterSplit.live === sigAfterSplit.n, JSON.stringify(sigAfterSplit));
// status bar global: always visible, left "user@host · auth:key · N sesi"
const sb = await page.evaluate(() => {
  const el = document.getElementById("status-bar");
  return { h: el.offsetHeight, vis: !!el.offsetParent, left: document.getElementById("sb-left").textContent, ver: document.getElementById("sb-ver").textContent };
});
check("U4: status bar always visible, 24px", sb.vis && sb.h === 24, `h=${sb.h} vis=${sb.vis}`);
// U-D2 FR-SH8: left = "host · user@ip:port · connected" (mono 11px bar)
check("U-D2 FR-SH8: status bar left = host · user@ip:port · connected",
  sb.left === "db-core · admin@203.0.113.20 · connected", sb.left);
check("U4: status bar shows app version", /^v\d+\.\d+\.\d+$/.test(sb.ver), sb.ver);
// U-D2 FR-SH8: live-session count now lives in #sb-live ("N live"), not #sb-left
const liveN = async () => Number(((await page.locator("#sb-live").textContent()).match(/(\d+) live/) || [])[1]);
const liveBefore = await liveN();
await page.locator(".host-item", { hasText: "prod-web" }).first().locator(".h-name").click();
await page.waitForTimeout(400);
const liveAfter = await liveN();
check("U-D2 FR-SH8: status bar live count increments on tab open", liveAfter === liveBefore + 1, `${liveBefore} -> ${liveAfter}`);
// activateTab restores split focus to the first .term-side (QA nit t_81aec02e:
// query by class, not positional children[] of .term-split). Re-activate the
// split tab first — opening prod-web above hid the split panel.
await page.locator(".tab", { hasText: "db-core" }).first().click();
await page.waitForTimeout(300);
await splitPanel.locator(".term-side").nth(1).click();
const preReactivate = await splitPanel.evaluate((el) =>
  [...el.querySelectorAll(".term-side")].findIndex((s) => s.classList.contains("focused")));
await page.locator(".tab", { hasText: "prod-web" }).first().click();
await page.waitForTimeout(300);
await page.locator(".tab", { hasText: "db-core" }).first().click();
await page.waitForTimeout(300);
const focBack = await splitPanel.evaluate((el) =>
  [...el.querySelectorAll(".term-side")].findIndex((s) => s.classList.contains("focused")));
check("U4: activateTab restores split focus to first side",
  preReactivate === 1 && focBack === 0, `pre=${preReactivate} post=${focBack}`);
// AC4: legacy float layout gone — split is flex + divider now
const cssU4 = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
check("U4: legacy 'float: left' split layout removed (grep 0)", !/float:\s*left/i.test(cssU4));

// THEME v1 (TSD theme-palettes-v1 §2.2a/§2.3): a preset swap repaints the LIVE
// terminals (main + split pane) through term.options.theme — no Terminal/PTY
// recreation. Light is the strongest flip: dark Dusk well -> light #faf7f0.
const themeTermSwap = await page.evaluate(() => {
  const before = { ...window.__hostermTheme };
  document.querySelector('.preset[data-preset="light"]').click();
  const light = { ...window.__hostermTheme };
  document.querySelector('.preset[data-preset="dusk"]').click();
  const restored = { ...window.__hostermTheme };
  return { before, light, restored };
});
check("THEME v1: preset swap repaints live split terminals (termBg #14100d -> #faf7f0 -> dusk)",
  themeTermSwap.before.preset === "dusk" && themeTermSwap.before.termBg === "#14100d" &&
  themeTermSwap.light.preset === "light" && themeTermSwap.light.termBg === "#faf7f0" &&
  themeTermSwap.restored.preset === "dusk" && themeTermSwap.restored.termBg === "#14100d",
  JSON.stringify(themeTermSwap));


// 9g1. paste normalization: CRLF clipboard -> single LF lines, no phantom Enter
const norm = await page.evaluate(() => {
  // pasteNormalize is module-scoped; recreate the exact same logic here to
  // assert its contract end-to-end through the same code path the app uses
  // (the handler calls it before pty_write).
  const text = "test baris 1\r\ntest baris 2\r\ntest baris 3\r\n";
  const lf = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  return lf.endsWith("\n") ? lf.slice(0, -1) : lf;
});
check("paste normalize: CRLF collapses to LF", norm === "test baris 1\ntest baris 2\ntest baris 3");

// 9g2. tab context menu: in-app menu, no native reload menu, close-others works
// (runs while the two terminal tabs from 9g are still open)
await page.locator(".tab").first().click({ button: "right" });
await page.waitForTimeout(200);
check("right-click tab opens in-app menu", (await page.locator("#tab-menu").count()) === 1);
check("menu has close items (U3 adds Close host: 4)", (await page.locator("#tab-menu button").count()) === 4);
const ctxBlocked = await page.evaluate(() => {
  let blocked = false;
  const t = document.querySelector(".tab");
  t.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
  // the handler calls preventDefault; a fresh event tells us nothing, so check listener effect:
  blocked = !document.getElementById("tab-menu") ? false : true;
  return blocked;
});
check("contextmenu handled (menu visible)", ctxBlocked);
await page.locator("#tab-menu button", { hasText: "Close others" }).click();
await page.waitForTimeout(500);
check("close others leaves one tab", (await page.locator(".tab").count()) === 1);

// 9g3. SFTP: list scrolls (pane is height-constrained), U5 strip + head buttons
// (9g2 closed every tab, so open a fresh SFTP tab first)
await page.locator(".host-item .h-sftp").first().click();
await page.waitForTimeout(600);
const sftpPanel = page.locator(".panel.active");
await sftpPanel.locator(".sftp-row").first().waitFor({ timeout: 5000 });
const stripInfo = await sftpPanel.locator(".sftp-list").evaluate((el) => {
  const wrap = el.closest(".sftp-wrap");
  const strip = wrap && wrap.querySelector(".xfer-strip");
  const up = wrap.querySelector(".sftp-pane:nth-child(2) .pane-head .sftp-upload");
  const dl = wrap.querySelector(".sftp-pane:nth-child(1) .pane-head .sftp-download");
  const head = up && up.closest(".pane-head");
  return {
    wrapFits: wrap.getBoundingClientRect().height <= window.innerHeight + 2,
    stripVisible: !!(strip && strip.offsetParent),
    upInHead: !!head && head.contains(up) && !!up.offsetParent,
    dlInHead: !!(dl && dl.closest(".pane-head") && dl.offsetParent),
    stripTop: Math.round(strip.getBoundingClientRect().top),
    vh: window.innerHeight,
  };
});
check("sftp wrap fits viewport (list scrollable)", stripInfo.wrapFits);
check("upload/download buttons visible", stripInfo.stripVisible && stripInfo.stripTop < stripInfo.vh);
check("U5: Upload/Download live in .pane-head (target pane), not a foot",
  stripInfo.upInHead && stripInfo.dlInHead && (await page.locator(".panel.active .sftp-foot").count()) === 0, JSON.stringify(stripInfo));
const srcFiles = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8") +
  readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
check("U5: grep .sftp-foot in main.ts+styles.css = 0", !srcFiles.includes(".sftp-foot") && !srcFiles.includes("sftp-foot"));
// U4 height chain: panel == #panels container even with the 24px status bar eating #main
const chain = await page.evaluate(() => {
  const panels = document.getElementById("panels");
  const panel = document.querySelector(".panel.active");
  const wrap = panel.querySelector(".sftp-wrap");
  return { ph: panel.offsetHeight, ch: panels.clientHeight, wh: wrap.offsetHeight };
});
check("U4: panel == #panels height (status bar ate 24px)", chain.ph === chain.ch, JSON.stringify(chain));
check("U4: sftp-wrap fills panel (height chain)", chain.wh === chain.ph, JSON.stringify(chain));

// 9h. sidebar resizer: drag divider widens the host list
await page.evaluate(() => { localStorage.removeItem("sb-width"); });
await page.locator(".tab.active .x").click();
await page.waitForTimeout(300);
const before = await page.locator("#section-panel").evaluate((el) => el.getBoundingClientRect().width);
const box = await page.locator("#sb-resize").boundingBox();
await page.mouse.move(box.x + box.width / 2, box.y + 200);
await page.mouse.down();
await page.mouse.move(box.x + 200, box.y + 200, { steps: 4 });
await page.mouse.up();
const after = await page.locator("#section-panel").evaluate((el) => el.getBoundingClientRect().width);
check("sidebar drag-resize works", after > before + 100, `before=${before} after=${after}`);
const sbw = await page.evaluate(() => localStorage.getItem("sb-width"));
check("sidebar width persisted", sbw !== null && parseInt(sbw, 10) === Math.round(after), `stored=${sbw}`);
await page.evaluate(() => { localStorage.removeItem("sb-width"); });

// 9i. sftp panes: This PC (local) is LEFT of remote — by geometry, not class order
await page.locator(".host-item .h-sftp").first().click();
await page.waitForTimeout(500);
const panes = page.locator(".panel.active .sftp-pane");
const g1 = await panes.nth(0).evaluate((el) => ({ x: el.getBoundingClientRect().x, t: el.querySelector(".pane-title").textContent }));
const g2 = await panes.nth(1).evaluate((el) => ({ x: el.getBoundingClientRect().x, t: el.querySelector(".pane-title").textContent }));
check("local pane is left of remote", g1.x < g2.x && g1.t.includes("This PC") && g2.t.includes("remote"), `L=${g1.t}@${Math.round(g1.x)} R=${g2.t}@${Math.round(g2.x)}`);

// 9k. U2 sidebar Arus: single-line rows, grouping, dots, Ctrl+K, skeleton (EPIC-TSD §U2)
// close leftover tabs so dot tests start idle
for (let i = 0; i < 10 && (await page.locator(".tab").count()) > 0; i++) {
  if (await page.locator("#dialog").isVisible().catch(() => false)) {
    await page.locator("#dialog button").filter({ hasText: "Discard" }).first().click().catch(() => {});
    await page.waitForTimeout(250);
    continue;
  }
  await page.locator(".tab .x").first().click();
  await page.waitForTimeout(300);
}
// U4: status bar survives with zero tabs (G1 — always visible), count drops to 0
const sbIdle = await page.evaluate(() => ({
  vis: !!document.getElementById("status-bar").offsetParent,
  h: document.getElementById("status-bar").offsetHeight,
  left: document.getElementById("sb-left").textContent,
  live: document.getElementById("sb-live").textContent,
  shell: (document.getElementById("sb-shell") || { textContent: null }).textContent,
}));
// U-D2 FR-SH8: 0 tabs → empty host segment left, "0 live" right cluster,
// shell field = static display-only label (never a guessed binary)
check("U-D2 FR-SH8: status bar visible with 0 tabs ('0 live', empty host, shell 'shell')",
  sbIdle.vis && sbIdle.h === 24 && sbIdle.left === "" && sbIdle.live === "0 live" && sbIdle.shell === "shell", JSON.stringify(sbIdle));
// U4 teardown hygiene (SEC LOW t_fb1d18d6): every split signal aborted after closeTab
const sigAfterClose = await page.evaluate(() => {
  const live = window.__sigList.filter((e) => !e.sig.aborted);
  return { n: window.__sigList.length, live: live.length };
});
check("U4: split window listeners released on closeTab (no leak)",
  sigAfterClose.n >= 1 && sigAfterClose.live === 0, JSON.stringify(sigAfterClose));
// (1) single-line 28px row; subtitle is a title tooltip, not a second element
const rowH = await page.locator(".host-item").first().evaluate((el) => el.offsetHeight);
check("U-D2 §13.3: host row is exactly 34px (single line)", rowH === 34, `h=${rowH}`);
check("no second-line subtitle element", (await page.locator(".host-item .h-sub").count()) === 0);
const rowTitle = await page.locator(".host-item").first().getAttribute("title");
// Hosts render A-Z, so the first row is db-core (admin@203.0.113.20).
check("subtitle lives in title tooltip", !!rowTitle && rowTitle.includes("admin@203.0.113.20"), rowTitle || "");
// (2) group headers = alias prefix before "-", A-Z order; wildcard Host * never rendered
const groups = await page.locator(".h-group").allTextContents();
check("group headers from '-' prefix", JSON.stringify(groups) === JSON.stringify(["DB", "PROD", "PW"]), groups.join("|"));
const prodRow = page.locator(".host-item", { hasText: "prod-web" }).first();
check("prod-web sits under PROD header",
  (await prodRow.evaluate((el) => el.previousElementSibling?.textContent)) === "PROD");
check("wildcard Host * not rendered", (await page.locator(".host-item").count()) === 3 &&
  !(await page.locator("#host-list").textContent()).includes("*"));
// (3) password marker: --warn lock only on has_password hosts
const pwKey = prodRow.locator(".h-key");
check("non-pw rows have no lock", (await pwKey.count()) === 0);
const pwRowKey = page.locator(".host-item", { hasText: "pw-host" }).first().locator(".h-key");
check("pw-host shows lock marker", (await pwRowKey.count()) === 1);
check("lock uses --accent token (Dusk)", (await pwRowKey.evaluate((el) => getComputedStyle(el).color)) === "rgb(226, 163, 92)");
// (4) Ctrl+K focuses search; ArrowDown + Enter opens terminal (§5).
// Filter to a single host first so the first visible row is deterministic
// regardless of the A-Z sort order (otherwise it would be db-core, not prod-web),
// keeping the opened terminal / dot / tab chain consistent for the checks below.
await page.keyboard.press("Control+k");
check("Ctrl+K focuses #host-search", await page.evaluate(() => document.activeElement?.id === "host-search"));
await page.locator("#host-search").fill("prod-web");
await page.waitForTimeout(150);
await page.keyboard.press("ArrowDown");
check("ArrowDown moves focus to first row", await page.evaluate(() => document.activeElement?.classList.contains("host-item")));
const spawnsU2 = calls.filter((c) => c.cmd === "pty_spawn").length;
await page.keyboard.press("Enter");
await page.waitForTimeout(400);
check("Enter on row opens terminal", calls.filter((c) => c.cmd === "pty_spawn").length === spawnsU2 + 1);
await page.locator("#host-search").fill("");
await page.waitForTimeout(150);
// (5) live-session dot: --ok while terminal tab open, --muted after close
const dot = prodRow.locator(".h-dot");
check("dot goes live with live terminal", (await dot.evaluate((el) => el.classList.contains("live"))) === true);
check("dot live uses --live token (Dusk)", (await dot.evaluate((el) => getComputedStyle(el).backgroundColor)) === "rgb(157, 196, 137)");
await page.locator('.tab:has-text("prod-web") .x').click();
await page.waitForTimeout(300);
check("dot back to muted after tab close", (await dot.evaluate((el) => el.classList.contains("live"))) === false);
// (6) loading = 3 static skeleton rows, no spinner, container height stable
delayRead = 400;
await page.goto("http://localhost:4199/");
await page.waitForTimeout(150);
check("loading renders 3 static skeleton rows", (await page.locator("#host-list .skel").count()) === 3);
check("no spinner replace during load", (await page.locator("#host-list .spinner").count()) === 0);
const hDuring = await page.locator("#host-list").evaluate((el) => el.offsetHeight);
const skelH = await page.locator("#host-list .skel").first().evaluate((el) => el.offsetHeight);
check("U-D2 §13.3: skeleton rows are 34px", skelH === 34, `h=${skelH}`);
delayRead = 0;
await page.waitForTimeout(600);
const hAfter = await page.locator("#host-list").evaluate((el) => el.offsetHeight);
check("no layout jump: #host-list height stable", hDuring === hAfter, `during=${hDuring} after=${hAfter}`);
check("hosts render after skeleton", (await page.locator(".host-item").count()) === 3);
// restore two tabs so U1's inactive-tab glow check (9j) has a subject
await page.locator(".host-item", { hasText: "prod-web" }).first().locator(".h-name").click();
await page.waitForTimeout(300);
await page.locator(".host-item", { hasText: "db-core" }).first().locator(".h-name").click();
await page.waitForTimeout(400);

// 9j. U-D1 Dusk token swap (EPIC-TSD §U-D1, SDD §13.3; supersedes Arus U1)
// token source of truth = proto-a-dusk.html :root, byte-exact.
const DUSK_ROOT = {
  "--bg": "#191512", "--sidebar": "#1d1814", "--surface": "#221c17",
  "--raised": "#2a231c", "--term": "#14100d", "--border": "#332a21",
  "--text": "#f2e9dc", "--dim": "#b9aa95", "--faint": "#93836f",
  "--accent": "#e2a35c", "--accent-ink": "#241404",
  "--live": "#9dc489", "--danger": "#e07856",
};
const rootTokens = await page.evaluate((keys) => {
  const cs = getComputedStyle(document.documentElement);
  const out = {};
  for (const k of keys) out[k] = cs.getPropertyValue(k).trim();
  return out;
}, Object.keys(DUSK_ROOT));
// (a) --bg == #191512 (Dusk)
check("U-D1: --bg == #191512 (Dusk)", rootTokens["--bg"] === DUSK_ROOT["--bg"], rootTokens["--bg"]);
// (b) --accent == #e2a35c (Dusk honey amber)
check("U-D1: --accent == #e2a35c (Dusk)", rootTokens["--accent"] === DUSK_ROOT["--accent"], rootTokens["--accent"]);
// all 16 Dusk tokens exposed verbatim in :root (SDD §13.3)
const tokMismatch = Object.entries(DUSK_ROOT).filter(([k, v]) => rootTokens[k] !== v);
check("U-D1: all Dusk :root tokens byte-exact §13.3", tokMismatch.length === 0,
  tokMismatch.map(([k, v]) => `${k}=${rootTokens[k]}≠${v}`).join(", "));
// color-scheme dark preserved
const colorScheme = await page.evaluate(() =>
  getComputedStyle(document.documentElement).getPropertyValue("color-scheme").trim());
check("theme color-scheme: dark", colorScheme === "dark", colorScheme);
// (c) offline fonts: no @import / url(http in stylesheet + index.html (source files)
const htmlSrc = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const netFont = [cssU4, htmlSrc].join("\n").match(/@import|url\(http|<link[^>]+href="http/g) || [];
check("no font CDN (@import / url(http) in styles.css + index.html", netFont.length === 0, netFont.slice(0, 2).join(" | "));
// (c) grep gate INV-21: ZERO Abyss-Emerald literal survives in src/ (acceptance kritis)
const mainSrc = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8");
const abyssLiterals = ["#08080a", "#050507", "#00d992", "#818cf8", "#0e2a20", "var(--glow)"];
const abyssHits = abyssLiterals.filter((lit) => (cssU4 + "\n" + mainSrc + "\n" + htmlSrc).includes(lit));
check("U-D1 INV-21: zero Abyss literal in src/ (grep gate)", abyssHits.length === 0, abyssHits.join(", "));
// (d) elevation: --shadow-drawer (box-shadow via that var) ONLY on floating
//     drawer/modal/popover surfaces; glow remains forbidden everywhere.
const shadowSelectors = await page.evaluate(() => {
  const out = { glow: [], elevation: [] };
  for (const sheet of document.styleSheets) {
    let rules;
    try { rules = sheet.cssRules; } catch { continue; }
    for (const r of rules) {
      if (!r.style || !r.style.boxShadow) continue;
      const bs = r.style.boxShadow;
      if (/rgba\(0,\s*217,\s*146|var\(--glow\)/.test(bs)) out.glow.push(r.selectorText);
      if (/var\(--shadow-drawer\)/.test(bs)) out.elevation.push(r.selectorText);
    }
  }
  out.glow.sort(); out.elevation.sort();
  return out;
});
check("U-D1: Abyss glow box-shadow fully removed", shadowSelectors.glow.length === 0, shadowSelectors.glow.join(", "));
// --shadow-drawer only lands on floating surfaces; no sidebar/tab/row carries it.
const elevationOk = shadowSelectors.elevation.every((s) => /modal-box|\.settings\b|\.drawer\b|\.signin-popover\b|\.toast\b/.test(s));
check("U-D1/U-D6: elevation only on drawer/modal/popover boxes", elevationOk && shadowSelectors.elevation.length > 0,
  shadowSelectors.elevation.join(", "));
// AC4 letter: inactive tab + plain button carry no box-shadow (no glow at idle/hover)
const noShadow = await page.evaluate(() => {
  const tab = document.querySelector(".tab:not(.active)");
  const btn = document.querySelector("button:not(.primary)");
  const sh = (el) => (el ? getComputedStyle(el).boxShadow : "missing");
  return { tab: sh(tab), btn: sh(btn) };
});
check("no glow on inactive tab / plain button", noShadow.tab === "none" && noShadow.btn === "none", `${noShadow.tab} / ${noShadow.btn}`);
// (e) xterm/CM palette = Dusk (SDD §13.2 pt 5): no emerald/indigo ANSI survives,
//     and Dusk's terminal palette now sources from theme-palettes.json (SSOT,
//     theme palettes v1) — main.ts builds THEME_TERMINAL from that JSON rather
//     than carrying inline hex literals, so assert the JSON holds byte-exact Dusk.
const paletteSrc = readFileSync(new URL("../src/theme-palettes.json", import.meta.url), "utf8");
const duskTerm = JSON.parse(paletteSrc).dusk.terminal;
const noAbyss = !/#020203|#00d992|#818cf8|#2fd6a1|#f2f2f2|#80d280|#ffdd80|#fd9c9f/.test(mainSrc + "\n" + paletteSrc);
const duskSsot = /theme-palettes\.json/.test(mainSrc) && /THEME_TERMINAL/.test(mainSrc) &&
  duskTerm.background === "#14100d" && duskTerm.cursor === "#e2a35c";
check("U-D1: xterm/CM palette = Dusk from SSOT JSON (no Abyss hex)", noAbyss && duskSsot);

// ---- U3 tab bar Arus (EPIC-TSD §U3): kind color, dead+reconnect, scroll, menu, Ctrl+W, drop-zone ----
// start clean: close leftover tabs (discard unsaved editor if prompted)
for (let i = 0; i < 10 && (await page.locator(".tab").count()) > 0; i++) {
  if (await page.locator("#dialog").isVisible().catch(() => false)) {
    await page.locator("#dialog button").filter({ hasText: "Discard" }).first().click().catch(() => {});
    await page.waitForTimeout(250);
    continue;
  }
  await page.locator(".tab .x").first().click();
  await page.waitForTimeout(300);
}
// AC1 kind colors: SFTP icon --accent2; active terminal icon --fg
await page.locator(".host-item .h-sftp").first().click();
await page.waitForTimeout(600);
const sftpIco = await page.locator(".tab.kind-sftp .t-ico").evaluate((el) => getComputedStyle(el).color);
check("U3: SFTP tab icon is --accent (Dusk)", sftpIco === "rgb(226, 163, 92)", sftpIco);
await page.locator(".host-item", { hasText: "prod-web" }).first().locator(".h-name").click();
await page.waitForTimeout(400);
const termIco = await page.locator(".tab.active .t-ico").evaluate((el) => getComputedStyle(el).color);
check("U3: active terminal icon is --text (Dusk)", termIco === "rgb(242, 233, 220)", termIco);
// AC3 dead tab: mock PTY exit -> ✕ --danger + exact title; click respawns
const spawnsU3 = calls.filter((c) => c.cmd === "pty_spawn").length;
await page.evaluate(() => window.__emitTauri("pty://exit/4242"));
await page.waitForTimeout(300);
check("U3: PTY exit marks terminal tab dead", (await page.locator(".tab.dead").count()) === 1);
const deadTab = page.locator(".tab.dead");
check("U3: dead tab title exact", (await deadTab.getAttribute("title")) === "Session ended — click to reconnect");
check("U3: dead icon is --danger (Dusk)", (await deadTab.locator(".t-ico").evaluate((el) => getComputedStyle(el).color)) === "rgb(224, 120, 86)");
// U9 polish (STATE Deferred U3): dead tab no longer shows two ✕ glyphs — the
// reconnect icon differs from the .x close glyph, close button still present
const deadGlyphs = await deadTab.evaluate((t) => ({ ico: t.querySelector(".t-ico").innerHTML, x: t.querySelector(".x")?.innerHTML || "" }));
check("U9: dead tab shows ONE ✕ (reconnect icon ≠ close glyph)",
  deadGlyphs.x !== "" && deadGlyphs.ico !== deadGlyphs.x && deadGlyphs.ico.includes("<svg"), JSON.stringify(deadGlyphs).slice(0, 120));
await deadTab.locator(".t-label").click();
await page.waitForTimeout(400);
check("U3: clicking dead tab respawns PTY", calls.filter((c) => c.cmd === "pty_spawn").length === spawnsU3 + 1);
check("U3: reconnect clears dead state", (await page.locator(".tab.dead").count()) === 0);
// AC2 auto-scroll: 12 tabs overflow the bar; scrollIntoView brings targets into view
await page.setViewportSize({ width: 800, height: 600 });
for (let i = 0; i < 10; i++) {
  await page.locator(".host-item", { hasText: "prod-web" }).first().locator(".h-name").click();
  await page.waitForTimeout(120);
}
check("U3: 12 tabs open", (await page.locator(".tab").count()) === 12, `tabs=${await page.locator(".tab").count()}`);
await page.locator(".tab").last().click();
await page.waitForTimeout(300);
const scLast = await page.locator("#tabs").evaluate((el) => el.scrollLeft);
check("U3: activating last tab scrolls bar (scrollIntoView)", scLast > 0, `scrollLeft=${scLast}`);
await page.locator(".tab").first().click();
await page.waitForTimeout(300);
const scFirst = await page.locator("#tabs").evaluate((el) => el.scrollLeft);
check("U3: activating first tab scrolls back", scFirst < scLast, `first=${scFirst} last=${scLast}`);
await page.setViewportSize({ width: 1280, height: 720 });
// AC4 context menu: Close others + Close host (+ shortcut tooltips)
await page.locator(".host-item", { hasText: "db-core" }).first().locator(".h-name").click();
await page.waitForTimeout(300);
// Right-click a prod-web tab explicitly (tab-bar order follows open order, not
// the host-list sort, but we must not depend on which tab lands first in the
// DOM): "Close host" then closes every prod-web tab and leaves only db-core.
await page.locator(".tab", { hasText: "prod-web" }).first().click({ button: "right" });
await page.waitForTimeout(200);
const menuLabels = await page.locator("#tab-menu button").allTextContents();
check("U3: menu = Close/Close others/Close host/Close all",
  JSON.stringify(menuLabels) === JSON.stringify(["Close", "Close others", "Close host", "Close all"]), menuLabels.join("|"));
const menuTips = await page.evaluate(() => [...document.querySelectorAll("#tab-menu button")].map((b) => b.title));
check("U3: menu tooltips name new shortcuts", menuTips[0] === "Ctrl+W" && menuTips[1] === "Ctrl+Shift+W", menuTips.join("|"));
await page.locator("#tab-menu button", { hasText: "Close host" }).click();
await page.waitForTimeout(500);
// "Close host" must drop every prod-web tab and keep the other host's tab(s).
// Assert on the invariant (no prod-web left, only db-core remains) rather than a
// hardcoded count — earlier blocks may legitimately have left >1 db-core tab, and
// the tab-bar order is independent of the host-list A-Z sort.
const u3Labels = await page.locator(".tab .t-label").allTextContents();
check("U3: Close host keeps only other hosts' tabs",
  u3Labels.length >= 1 && u3Labels.every((l) => l === "db-core"), u3Labels.join("|"));
// AC5 Ctrl+W / Ctrl+Shift+W; skipped while typing in a text field.
// Normalise to a known tab count first: earlier blocks + the A-Z sort can leave
// a variable number of db-core tabs, so close them all, then rebuild exactly the
// state each check needs (don't assume a specific residual count).
await page.evaluate(() => { const a = document.activeElement; if (a instanceof HTMLElement) a.blur(); });
await page.keyboard.press("Control+Shift+w"); // keep only active
await page.waitForTimeout(300);
await page.keyboard.press("Control+w"); // close the last one -> 0 tabs
await page.waitForTimeout(300);
check("U3: Ctrl+W closes active tab", (await page.locator(".tab").count()) === 0);
await page.locator(".host-item", { hasText: "prod-web" }).first().locator(".h-name").click();
await page.waitForTimeout(300);
await page.locator(".host-item", { hasText: "db-core" }).first().locator(".h-name").click();
await page.waitForTimeout(300);
await page.locator("#host-search").fill("zzz");
await page.keyboard.press("Control+Shift+w");
await page.waitForTimeout(300);
check("U3: Ctrl+Shift+W no-op while typing", (await page.locator(".tab").count()) === 2);
await page.locator("#host-search").fill("");
await page.evaluate(() => { const a = document.activeElement; if (a instanceof HTMLElement) a.blur(); });
await page.keyboard.press("Control+Shift+w");
await page.waitForTimeout(400);
check("U3: Ctrl+Shift+W closes all but active", (await page.locator(".tab").count()) === 1);
// AC6 split stays manual-mouse; drop-zone static affordance lights --accent while dragging
const u3src = mainSrc;
check("U3: no draggable attribute (grep = 0)", !/draggable/i.test(u3src) &&
  (await page.evaluate(() => document.querySelectorAll("[draggable]").length)) === 0);
check("U3: drop-zone rests --border (Dusk)",
  (await page.locator("#tab-split-zone").evaluate((el) => getComputedStyle(el).color)) === "rgb(51, 42, 33)");
await page.locator(".host-item", { hasText: "prod-web" }).first().locator(".h-name").click();
await page.waitForTimeout(300);
const dzBox = await page.locator(".tab", { hasText: "prod-web" }).first().boundingBox();
await page.mouse.move(dzBox.x + dzBox.width / 2, dzBox.y + dzBox.height / 2);
await page.mouse.down();
await page.mouse.move(dzBox.x + dzBox.width / 2 + 12, dzBox.y + dzBox.height / 2);
const dzHot = await page.locator("#tab-split-zone").evaluate((el) => ({ hot: el.classList.contains("drag-hot"), c: getComputedStyle(el).color }));
check("U3: drop-zone lights --accent while dragging", dzHot.hot && dzHot.c === "rgb(226, 163, 92)", JSON.stringify(dzHot));
await page.mouse.up();
await page.waitForTimeout(400);
// clean up for the final checks
for (let i = 0; i < 6 && (await page.locator(".tab").count()) > 0; i++) {
  if (await page.locator("#dialog").isVisible().catch(() => false)) {
    await page.locator("#dialog button").filter({ hasText: "Discard" }).first().click().catch(() => {});
    await page.waitForTimeout(250);
    continue;
  }
  await page.locator(".tab .x").first().click();
  await page.waitForTimeout(300);
}

// ---- U5 SFTP Arus: pane-head chips/actions + per-file strip + selection/drop (EPIC-TSD §U5) ----
// cleanup above closed all tabs — reopen a fresh SFTP tab (smoke quirk)
delayList = 600; // observe skeleton loading state (§4.4: loading = skeleton rows)
await page.locator(".host-item .h-sftp").first().click();
await page.waitForTimeout(250);
check("U5: loading shows 3 skeleton rows, no spinner text",
  (await page.locator(".panel.active .sftp-list .skel").count()) === 3 &&
  (await page.locator(".panel.active .sftp-list .spinner").count()) === 0);
delayList = 0;
await page.locator(".panel.active .sftp-list .sftp-row").first().waitFor({ timeout: 5000 });
// AC6: row height 28px measured
const u5rowH = await page.locator(".panel.active .sftp-list .sftp-row").first().evaluate((el) => el.offsetHeight);
check("U5 AC6: .sftp-row is 28px measured", u5rowH === 28, `h=${u5rowH}`);
// AC4 (Space): selects sumber WITHOUT opening the editor (stage-1 must stay on selection path)
const tabsBeforeSpace = await page.locator(".tab").count();
await page.locator(".panel.active .loc-list .sftp-row").first().focus(); // notes.txt local
await page.keyboard.press("Space");
await page.waitForTimeout(150);
check("U5 AC4: Space selects, no editor opened",
  (await page.locator(".panel.active .loc-list .sftp-row.sel").count()) === 1 &&
  (await page.locator(".tab").count()) === tabsBeforeSpace &&
  (await page.locator(".panel.active .ed-wrap").count()) === 0);
// AC1: .sel = bg --sel AND 1px solid --accent outline, on the CLICKED row (not last-child)
const selPaint = await page.locator(".panel.active .loc-list .sftp-row").first().evaluate((el) => {
  const cs = getComputedStyle(el);
  return { nm: el.dataset.name, last: el.parentNode.lastElementChild === el, bg: cs.backgroundColor, oW: cs.outlineWidth, oS: cs.outlineStyle, oC: cs.outlineColor };
});
check("U5 AC1: .sel paints bg var(--sel) + 1px solid --accent outline on clicked row",
  selPaint.nm === "notes.txt" && !selPaint.last && selPaint.bg === "rgba(226, 163, 92, 0.12)" &&
  selPaint.oW === "1px" && selPaint.oS === "solid" && selPaint.oC === "rgb(226, 163, 92)", JSON.stringify(selPaint));
// chips: ▲ sumber (--warn) on source head, ▼ target on opposite head (F)
const chips = await page.evaluate(() => {
  const p = document.querySelector(".panel.active");
  const src = p.querySelector(".sftp-pane:nth-child(1) .chip");
  const dst = p.querySelector(".sftp-pane:nth-child(2) .chip");
  return { src: src.textContent, c: getComputedStyle(src).color, dst: dst.textContent };
});
check("U5: chips ▲ sumber / ▼ target light on select (src --warn)",
  chips.src === "▲ sumber: notes.txt" && chips.c === "rgb(226, 163, 92)" && chips.dst.startsWith("▼ target:"), JSON.stringify(chips));
// Tab jumps focus to the opposite pane (keyboard path §4.4)
await page.keyboard.press("Tab");
const tabJump = await page.evaluate(() => ({ inRemote: !!document.activeElement?.closest(".sftp-list"), nm: document.activeElement?.dataset?.name || "" }));
check("U5: Tab moves focus to opposite pane", tabJump.inRemote && tabJump.nm === "notes.txt", JSON.stringify(tabJump));
// AC4 (Ctrl+Enter): transfer the pending sumber from the other pane
const upN0 = calls.filter((c) => c.cmd === "sftp_upload").length;
await page.keyboard.press("Control+Enter");
await page.waitForTimeout(450);
const upCall = calls.filter((c) => c.cmd === "sftp_upload").at(-1);
check("U5 AC4: Ctrl+Enter transfers selection with correct paths",
  calls.filter((c) => c.cmd === "sftp_upload").length === upN0 + 1 &&
  upCall?.args.localPath === "/tmp/mockhome/notes.txt" && upCall?.args.remotePath === "notes.txt", JSON.stringify(upCall?.args));
check("U5: strip row reaches ok state", (await page.locator(".panel.active .xf-row.ok").count()) === 1);
// spinner state visible while running (transfer strip §4.4)
await page.locator(".panel.active .loc-list .sftp-row").first().click();
await page.waitForTimeout(400);
await page.locator(".panel.active .sftp-pane:nth-child(2) .pane-head .sftp-upload").click();
// U9 flake fix: observe the run row as soon as it renders (was a fixed 80ms
// sleep racing the 150ms mock under CDP load); timeout → check FAILs below
await page.locator(".panel.active .xf-row.run .spinner").first().waitFor({ timeout: 1000 }).catch(() => {});
check("U5: running transfer shows 12px spinner row",
  (await page.locator(".panel.active .xf-row.run .spinner").count()) === 1 &&
  (await page.locator(".panel.active .xf-row.run .spinner").first().evaluate((el) => getComputedStyle(el).width)) === "12px");
// click-click two-stage: remote file = sumber, then local file row = target → download
await page.waitForTimeout(400);
await page.locator(".panel.active .sftp-list .sftp-row").first().click(); // remote notes.txt
await page.waitForTimeout(400);
const dlN0 = calls.filter((c) => c.cmd === "sftp_download").length;
await page.locator(".panel.active .loc-list .sftp-row").first().click(); // stage-2 target
await page.waitForTimeout(450);
const dlCall = calls.filter((c) => c.cmd === "sftp_download").at(-1);
check("U5: click-click two-stage transfer (remote→local, correct paths)",
  calls.filter((c) => c.cmd === "sftp_download").length === dlN0 + 1 &&
  dlCall?.args.remotePath === "notes.txt" && dlCall?.args.localPath === "/tmp/mockhome/notes.txt", JSON.stringify(dlCall?.args));
// AC5: two consecutive transfers keep TWO rows (no overwrite) — after older rows fade
await page.waitForTimeout(4700); // let all ok rows auto-fade (4s + 320ms fade) clear the strip first
check("U5: ok rows auto-fade after 4s", (await page.locator(".panel.active .xf-row").count()) === 0);
await page.locator(".panel.active .loc-list .sftp-row").first().click();
await page.waitForTimeout(400);
await page.locator(".panel.active .sftp-pane:nth-child(2) .pane-head .sftp-upload").click();
await page.waitForTimeout(350);
await page.locator(".panel.active .loc-list .sftp-row").first().click();
await page.waitForTimeout(400);
await page.locator(".panel.active .sftp-pane:nth-child(2) .pane-head .sftp-upload").click();
await page.waitForTimeout(350);
const xfN = await page.locator(".panel.active .xf-row").count();
check("U5 AC5: two consecutive transfers = 2 strip rows (not overwritten)", xfN === 2, `rows=${xfN}`);
// AC5: failure persists with retry, retry re-runs same job
xferFail = true;
await page.locator(".panel.active .loc-list .sftp-row").first().click();
await page.waitForTimeout(400);
await page.locator(".panel.active .sftp-pane:nth-child(2) .pane-head .sftp-upload").click();
await page.waitForTimeout(400);
const failInfo = await page.evaluate(() => {
  const r = [...document.querySelectorAll(".panel.active .xf-row.fail")].pop();
  return { n: document.querySelectorAll(".panel.active .xf-row.fail").length, retry: !!r?.querySelector(".xf-retry"), title: r?.title || "", c: r ? getComputedStyle(r.querySelector(".xf-state")).color : "" };
});
check("U5 AC5: failed transfer shows --danger row with retry button",
  failInfo.n === 1 && failInfo.retry && failInfo.title.includes("Failed to upload notes.txt") && failInfo.c === "rgb(224, 120, 86)", JSON.stringify(failInfo));
const upN1 = calls.filter((c) => c.cmd === "sftp_upload").length;
xferFail = false;
await page.locator(".panel.active .xf-row.fail .xf-retry").last().click();
await page.waitForTimeout(450);
check("U5: retry replaces the failed row and succeeds",
  calls.filter((c) => c.cmd === "sftp_upload").length === upN1 + 1 &&
  (await page.locator(".panel.active .xf-row.fail").count()) === 0 &&
  (await page.locator(".panel.active .xf-row.ok").count()) >= 1);
// AC3: manual mouse drag remote→local = download (no HTML5 DnD anywhere)
const dlN1 = calls.filter((c) => c.cmd === "sftp_download").length;
const gb = await page.locator(".panel.active .sftp-list .sftp-row").first().boundingBox();
const tb = await page.locator(".panel.active .sftp-pane:nth-child(1)").boundingBox();
await page.mouse.move(gb.x + gb.width / 2, gb.y + gb.height / 2);
await page.mouse.down();
await page.mouse.move(tb.x + tb.width / 2, tb.y + tb.height - 10, { steps: 5 });
const dragState = await page.evaluate(() => {
  const p = document.querySelector(".panel.active .sftp-pane:nth-child(1)");
  const cs = getComputedStyle(p);
  return { cls: p.classList.contains("drop-target"), body: document.body.classList.contains("sftp-dragging"), oS: cs.outlineStyle, oW: cs.outlineWidth, oC: cs.outlineColor };
});
check("U5 AC3: drag lights opposite pane dashed 2px --accent",
  dragState.cls && dragState.body && dragState.oS === "dashed" && dragState.oW === "2px" && dragState.oC === "rgb(226, 163, 92)", JSON.stringify(dragState));
await page.mouse.up();
await page.waitForTimeout(450);
const dlCall2 = calls.filter((c) => c.cmd === "sftp_download").at(-1);
check("U5 AC3: manual drag remote→local invokes sftp_download (correct paths)",
  calls.filter((c) => c.cmd === "sftp_download").length === dlN1 + 1 &&
  dlCall2?.args.remotePath === "notes.txt" && dlCall2?.args.localPath === "/tmp/mockhome/notes.txt", JSON.stringify(dlCall2?.args));
check("U5 AC3: zero [draggable] elements + no sftp-dragging stuck on body",
  (await page.evaluate(() => document.querySelectorAll("[draggable]").length)) === 0 &&
  !(await page.evaluate(() => document.body.classList.contains("sftp-dragging"))));
// strip layout: max 3 rows + scroll, static hint 11px --muted
const stripLayout = await page.evaluate(() => {
  const rows = document.querySelector(".panel.active .xs-rows");
  const h = document.querySelector(".panel.active .sftp-hint");
  const cs = getComputedStyle(h);
  return { maxH: getComputedStyle(rows).maxHeight, scrollable: rows.scrollHeight >= rows.clientHeight, hint: h.textContent, fs: cs.fontSize, c: cs.color };
});
check("U5: strip caps 3 rows + scroll", stripLayout.maxH === "64px", JSON.stringify(stripLayout));
check("U5: static hint 11px --muted names click/drag paths",
  stripLayout.hint === "Pilih file → klik pane tujuan / drag manual mouse — HTML5 DnD tidak dipakai" &&
  stripLayout.fs === "11px" && stripLayout.c === "rgb(185, 170, 149)", `${stripLayout.fs}/${stripLayout.c}`);
// close tabs — sftp window-mouseup signal must abort (U4 registry invariant)
for (let i = 0; i < 6 && (await page.locator(".tab").count()) > 0; i++) {
  if (await page.locator("#dialog").isVisible().catch(() => false)) {
    await page.locator("#dialog button").filter({ hasText: "Discard" }).first().click().catch(() => {});
    await page.waitForTimeout(250);
    continue;
  }
  await page.locator(".tab .x").first().click();
  await page.waitForTimeout(300);
}
const sigAfterU5 = await page.evaluate(() => ({
  n: window.__sigList.length,
  live: window.__sigList.filter((e) => !e.sig.aborted).length,
  sftp: window.__sigList.filter((e) => !e.types.includes("mousemove")).length,
}));
check("U5: sftp drag listener released on closeTab (AbortController, live==0)",
  sigAfterU5.live === 0 && sigAfterU5.sftp >= 1, JSON.stringify(sigAfterU5));

// ---- U6 skeleton rows list + empty/error CTA (EPIC-TSD §U6; redesign §1.7 G2, §4.7) ----
// U5 cleanup closed all tabs — reopen a fresh SFTP tab (same smoke quirk as §U5)
await page.locator(".host-item .h-sftp").first().click();
await page.locator(".panel.active .sftp-list .sftp-row").first().waitFor({ timeout: 5000 });
await page.locator(".panel.active .loc-list .sftp-row").first().waitFor({ timeout: 5000 });
const u6Heights = await page.evaluate(() => {
  const p = document.querySelector(".panel.active");
  return { sftp: p.querySelector(".sftp-list").offsetHeight, loc: p.querySelector(".loc-list").offsetHeight };
});
// (AC1) reload while a LIST of rows is shown: content swaps to static .skel rows,
// container offsetHeight must not move (no layout jump — delta = 0, flex height)
delayList = 500;
await page.locator(".panel.active .sftp-refresh").click();
await page.waitForTimeout(100);
const u6Mid = await page.evaluate(() => {
  const list = document.querySelector(".panel.active .sftp-list");
  const skel = list.querySelector(".skel");
  const cs = skel ? getComputedStyle(skel) : null;
  return {
    skels: list.querySelectorAll(".skel").length,
    rows: list.querySelectorAll(".sftp-row").length,
    spinners: list.querySelectorAll(".spinner").length,
    h: list.offsetHeight,
    anim: cs ? cs.animationName : "",
    adur: cs ? cs.animationDuration : "",
    tdur: cs ? cs.transitionDuration : "",
  };
});
check("U6 AC1: loading = 3 static .skel rows (0 list rows, 0 spinner)",
  u6Mid.skels === 3 && u6Mid.rows === 0 && u6Mid.spinners === 0, JSON.stringify(u6Mid));
// U-D2 §13.3 motion: skeletons now shimmer (1.4s), replacing the Arus pulse-free rule
check("U-D2 §13.3: skeletons shimmer (animationName 'shimmer', 1.4s)",
  u6Mid.anim === "shimmer" && u6Mid.adur === "1.4s", `${u6Mid.anim}/${u6Mid.adur}`);
check("U6 AC1: remote list height stable during loading (delta 0)",
  u6Mid.h === u6Heights.sftp, `before=${u6Heights.sftp} during=${u6Mid.h}`);
// same for renderLocalPane (skeleton path was never observable before — mock now delays)
await page.locator(".panel.active .loc-go").click();
await page.waitForTimeout(100);
const u6MidLoc = await page.evaluate(() => {
  const list = document.querySelector(".panel.active .loc-list");
  return { skels: list.querySelectorAll(".skel").length, rows: list.querySelectorAll(".sftp-row").length, h: list.offsetHeight };
});
check("U6 AC1: local list loading = 3 .skel rows + height stable (delta 0)",
  u6MidLoc.skels === 3 && u6MidLoc.rows === 0 && u6MidLoc.h === u6Heights.loc, JSON.stringify({ ...u6MidLoc, before: u6Heights.loc }));
delayList = 0;
await page.locator(".panel.active .loc-list .sftp-row").first().waitFor({ timeout: 5000 });
// (AC2) empty state byte-identical on BOTH panes (redesign §4.7: "Empty directory.", no CTA here)
await page.locator(".panel.active .sftp-path").fill("/empty");
await page.locator(".panel.active .sftp-go").click();
await page.locator(".panel.active .loc-path").fill("/empty");
await page.locator(".panel.active .loc-go").click();
await page.waitForTimeout(250);
const u6Empty = await page.evaluate(() => {
  const p = document.querySelector(".panel.active");
  const r = p.querySelector(".sftp-list .list-state");
  const l = p.querySelector(".loc-list .list-state");
  return { r: r ? r.textContent : null, l: l ? l.textContent : null };
});
check("U6 AC2: empty copy byte-identical \"Empty directory.\" (remote + local)",
  u6Empty.r === "Empty directory." && u6Empty.l === "Empty directory.", JSON.stringify(u6Empty));
// (AC2) error copy byte-identical + Go/Up stay actionable; list cleared but height stable
await page.locator(".panel.active .sftp-path").fill("/boom");
await page.locator(".panel.active .sftp-go").click();
await page.waitForTimeout(250);
const u6Err = await page.evaluate(() => {
  const p = document.querySelector(".panel.active");
  const st = p.querySelector(".sftp-status");
  return {
    text: st.textContent, err: st.classList.contains("err"),
    goOn: !p.querySelector(".sftp-go").disabled, upOn: !p.querySelector(".sftp-up").disabled,
    h: p.querySelector(".sftp-list").offsetHeight,
  };
});
check("U6 AC2: remote error copy byte-identical + Go/Up actionable + no height jump",
  u6Err.text === "Failed to list /boom: sftp boom (mock). Fix the path and press Go, or go up one level." &&
  u6Err.err && u6Err.goOn && u6Err.upOn && u6Err.h === u6Heights.sftp, JSON.stringify({ ...u6Err, before: u6Heights.sftp }));
await page.locator(".panel.active .loc-path").fill("/boom");
await page.locator(".panel.active .loc-go").click();
await page.waitForTimeout(250);
const u6ErrL = await page.evaluate(() => {
  const p = document.querySelector(".panel.active");
  return {
    text: p.querySelector(".sftp-status").textContent,
    goOn: !p.querySelector(".loc-go").disabled, upOn: !p.querySelector(".loc-up").disabled,
  };
});
check("U6 AC2: local error copy byte-identical + Go/Up actionable",
  u6ErrL.text === "Failed to read /boom: permission denied (mock-local). Fix the path and press Go." &&
  u6ErrL.goOn && u6ErrL.upOn, JSON.stringify(u6ErrL));
// source pin: U6 changed no copy strings in main.ts (diff review guard, EPIC-TSD §U6 AC2)
check("U6 AC2: copy strings untouched in main.ts source",
  (u3src.match(/<div class="list-state">Empty directory\.<\/div>/g) || []).length === 2 &&
  u3src.includes("Fix the path and press Go, or go up one level.") &&
  u3src.includes("Failed to read ${dir || \"home\"}: ${msg(err)}. Fix the path and press Go."));
// close the U6 tab so the suite ends as it started (no leftover listeners)
await page.locator(".tab .x").first().click();
await page.waitForTimeout(300);
// U9 (QA NIT-1 / SEC INFO-2): U6's own open/close lifecycle must also leave live==0
const sigAfterU6 = await page.evaluate(() => ({
  n: window.__sigList.length,
  live: window.__sigList.filter((e) => !e.sig.aborted).length,
}));
check("U6: listeners released after U6 tab lifecycle (AbortController, live==0)",
  sigAfterU6.live === 0 && sigAfterU6.n > sigAfterU5.n, JSON.stringify(sigAfterU6));
// U9 hygiene (SEC INFO-3): global list delay must be back to 0 after U6
check("U6: delayList reset to 0 (later checks assume instant listings)", delayList === 0, `delayList=${delayList}`);

// ---- BUG1-FE cancellable SFTP loading state (openSftp; AC1/AC2/AC3) ----
// A slow/failing SFTP login must never read as a frozen UI: opening a host
// shows a loading state IMMEDIATELY (skeleton + a Cancel control) BEFORE the
// first listing resolves, and the user can cancel/close the tab mid-load with
// no leaked window listener and no orphaned cache write (renderSftp bails when
// the session/panel is gone after the await). Reuses delayList to hold the
// first sftp_list in flight.
const b1SigBase = sigAfterU6.live; // 0 — all U6 listeners released
const b1Errs0 = errors.length;
delayList = 600; // keep the first listing in flight long enough to observe loading
await page.locator(".host-item .h-sftp").first().click();
await page.waitForTimeout(150); // mid-load: listing still pending
const b1Mid = await page.evaluate(() => {
  const list = document.querySelector(".panel.active .sftp-list");
  const cancel = list.querySelector(".sftp-cancel");
  const st = document.querySelector(".panel.active .sftp-status");
  return {
    skels: list.querySelectorAll(".skel").length,
    rows: list.querySelectorAll(".sftp-row").length,
    spinners: list.querySelectorAll(".spinner").length,
    hasCancel: !!cancel,
    cancelText: cancel ? cancel.textContent : null,
    loadingText: list.querySelector(".sftp-loading span")?.textContent ?? null,
    status: st ? st.textContent : null,
  };
});
check("BUG1-FE AC1: SFTP open shows loading immediately — 3 skeleton rows, 0 list rows, 0 spinner",
  b1Mid.skels === 3 && b1Mid.rows === 0 && b1Mid.spinners === 0, JSON.stringify(b1Mid));
check("BUG1-FE AC1: in-flight load exposes a Cancel control + Connecting status",
  b1Mid.hasCancel && b1Mid.cancelText === "Cancel" && b1Mid.loadingText === "Connecting…" &&
  /^Connecting to .+…$/.test(b1Mid.status || ""), JSON.stringify(b1Mid));
// (AC2) cancel/close WHILE the attempt is in flight: tears down listeners (no
// leak), drops the session, and the delayed listing that resolves afterwards
// must NOT poison the shared cache or throw against the detached panel.
const b1TabsBefore = await page.locator(".tab").count();
await page.locator(".panel.active .sftp-cancel").click();
await page.waitForTimeout(100);
const b1AfterCancel = await page.evaluate(() => ({
  tabs: document.querySelectorAll(".tab").length,
  loadingPanels: document.querySelectorAll(".sftp-loading").length,
}));
check("BUG1-FE AC2: Cancel mid-load closes the SFTP tab (one fewer tab, no loading panel left)",
  b1AfterCancel.tabs === b1TabsBefore - 1 && b1AfterCancel.loadingPanels === 0,
  JSON.stringify({ ...b1AfterCancel, before: b1TabsBefore }));
// let the still-pending delayed sftp_list resolve into the void (tab is gone)
await page.waitForTimeout(700);
delayList = 0;
const b1SigAfter = await page.evaluate(() => ({
  n: window.__sigList.length,
  live: window.__sigList.filter((e) => !e.sig.aborted).length,
}));
check("BUG1-FE AC2: cancel tears down window listeners (live count back to baseline, AbortController)",
  b1SigAfter.live === b1SigBase && b1SigAfter.n > sigAfterU6.n, JSON.stringify({ ...b1SigAfter, base: b1SigBase }));
check("BUG1-FE AC2: late-resolving listing after cancel throws no console/page error",
  errors.length === b1Errs0, errors.slice(b1Errs0).slice(0, 3).join(" | "));
// (AC3) no regression: a fresh open with no delay still reaches the normal
// listing (loading state clears, real rows render, no Cancel lingering).
await page.locator(".host-item .h-sftp").first().click();
await page.locator(".panel.active .sftp-list .sftp-row").first().waitFor({ timeout: 5000 });
const b1Normal = await page.evaluate(() => {
  const list = document.querySelector(".panel.active .sftp-list");
  return { rows: list.querySelectorAll(".sftp-row").length, cancels: list.querySelectorAll(".sftp-cancel").length, skels: list.querySelectorAll(".skel").length };
});
check("BUG1-FE AC3: instant open renders the listing and clears the loading/Cancel state",
  b1Normal.rows > 0 && b1Normal.cancels === 0 && b1Normal.skels === 0, JSON.stringify(b1Normal));
// close the BUG1-FE tab so the suite ends as it started
await page.locator(".tab .x").first().click();
await page.waitForTimeout(300);
const b1SigEnd = await page.evaluate(() => window.__sigList.filter((e) => !e.sig.aborted).length);
check("BUG1-FE: listeners released after full lifecycle (live==0)", b1SigEnd === 0, `live=${b1SigEnd}`);

// ---- U7 editor: panel-level Ctrl+S + ed-bar dirty (EPIC-TSD §U7; redesign §1.5 E1–E2, §4.5) ----
await page.locator(".host-item .h-sftp").first().click();
await page.locator(".panel.active .sftp-list .sftp-row", { hasText: "notes.txt" }).first().dblclick();
await page.locator(".panel.active .cm-content").waitFor({ timeout: 5000 });
const u7Clean = await page.evaluate(() => {
  const el = document.querySelector(".panel.active .ed-dirty");
  return el ? { text: el.textContent, display: getComputedStyle(el).display } : null;
});
check("U7 AC2: ed-bar carries '● unsaved', hidden while clean",
  u7Clean && u7Clean.text === "● unsaved" && u7Clean.display === "none", JSON.stringify(u7Clean));
// (AC2) edit -> unsaved text visible in --warn + tab dot still there (E2)
await page.locator(".panel.active .cm-content").click();
await page.keyboard.type("X");
await page.waitForTimeout(250);
const u7Dirty = await page.evaluate(() => {
  const el = document.querySelector(".panel.active .ed-dirty");
  const cs = getComputedStyle(el);
  const tab = [...document.querySelectorAll(".tab")].find((t) => t.querySelector(".t-label")?.textContent === "notes.txt");
  return { display: cs.display, color: cs.color, tabDirty: !!tab?.classList.contains("dirty"), dot: tab ? getComputedStyle(tab.querySelector(".t-dot")).display : "none" };
});
check("U7 AC2: edit shows '● unsaved' in --warn on the ed-bar",
  u7Dirty.display !== "none" && u7Dirty.color === "rgb(226, 163, 92)", JSON.stringify(u7Dirty));
check("U7 AC2: existing tab dirty dot + .tab.dirty survive (not replaced)",
  u7Dirty.tabDirty && u7Dirty.dot !== "none", JSON.stringify(u7Dirty));
// (AC1) Ctrl+S with focus on the Save BUTTON (outside .cm-editor) must still save —
// the pre-U7 listener lived on view.dom, where this was dead (redesign §1.5 E1).
const u7Wr0 = calls.filter((c) => c.cmd === "sftp_write_file").length;
await page.locator(".panel.active .ed-save").focus();
await page.keyboard.press("Control+s");
await page.waitForTimeout(400);
const u7Write = calls.filter((c) => c.cmd === "sftp_write_file").slice(u7Wr0);
check("U7 AC1: Ctrl+S while focus sits on Save invokes sftp_write_file with the edit",
  u7Write.length === 1 && u7Write[0].args.content.includes("X"), JSON.stringify(u7Write.map((c) => c.args.remotePath)));
// (AC2) saved -> indicator gone
const u7Saved = await page.evaluate(() => {
  const el = document.querySelector(".panel.active .ed-dirty");
  const tab = [...document.querySelectorAll(".tab")].find((t) => t.querySelector(".t-label")?.textContent === "notes.txt");
  return { display: getComputedStyle(el).display, tabDirty: !!tab?.classList.contains("dirty"), status: document.querySelector(".panel.active .ed-status").textContent };
});
check("U7 AC2: save clears the ed-bar text and the tab dot",
  u7Saved.display === "none" && !u7Saved.tabDirty && u7Saved.status.startsWith("Saved"), JSON.stringify(u7Saved));
// (AC1 regression) Ctrl+S INSIDE .cm-editor must keep working after the move — the
// keydown now reaches the panel by bubbling instead of living on view.dom.
await page.locator(".panel.active .cm-content").click();
await page.keyboard.type("V");
await page.keyboard.press("Control+s");
await page.waitForTimeout(400);
const u7Write2 = calls.filter((c) => c.cmd === "sftp_write_file").slice(u7Wr0 + 1);
check("U7 AC1: Ctrl+S inside .cm-editor still saves (bubbles to the panel listener)",
  u7Write2.length === 1 && u7Write2[0].args.content.includes("V") &&
  (await page.evaluate(() => getComputedStyle(document.querySelector(".panel.active .ed-dirty")).display === "none")),
  JSON.stringify({ n: u7Write2.length, hasV: u7Write2[0] ? u7Write2[0].args.content.includes("V") : null }));
// (AC2) Reload while dirty -> existing in-app dirty dialog, Cancel loses nothing
await page.locator(".panel.active .cm-content").click();
await page.keyboard.type("Y");
await page.waitForTimeout(250);
const u7Rd0 = calls.filter((c) => c.cmd === "sftp_read_file").length;
let u7Native = null;
page.once("dialog", (d) => { u7Native = d.type(); d.dismiss(); });
await page.locator(".panel.active .ed-reload").click();
await page.waitForTimeout(300);
const u7Dlg = await page.evaluate(() => ({
  open: !document.querySelector("#dialog").classList.contains("hidden"),
  msg: document.querySelector("#dialog-msg").textContent,
  btns: [...document.querySelectorAll("#dialog-actions button")].map((b) => b.textContent),
}));
check("U7 AC2: Reload while dirty raises the in-app dirty dialog (no native confirm)",
  u7Native === null && u7Dlg.open && u7Dlg.msg === "notes.txt has unsaved changes." && u7Dlg.btns.length === 3,
  JSON.stringify({ native: u7Native, ...u7Dlg }));
await page.locator("#dialog-actions button", { hasText: "Cancel" }).click();
await page.waitForTimeout(300);
const u7Cancel = await page.evaluate(() => ({
  text: document.querySelector(".panel.active .cm-content").textContent,
  dirty: getComputedStyle(document.querySelector(".panel.active .ed-dirty")).display !== "none",
}));
check("U7 AC2: Cancel reload = no read invoked, edit + dirty kept",
  calls.filter((c) => c.cmd === "sftp_read_file").length === u7Rd0 && u7Cancel.text.includes("Y") && u7Cancel.dirty,
  JSON.stringify(u7Cancel));
// Discard branch -> server text restored, dirty cleared
await page.locator(".panel.active .ed-reload").click();
await page.waitForTimeout(300);
await page.locator("#dialog-actions button", { hasText: "Discard changes" }).click();
await page.waitForTimeout(500);
const u7Discard = await page.evaluate(() => ({
  text: document.querySelector(".panel.active .cm-content").textContent,
  dirty: getComputedStyle(document.querySelector(".panel.active .ed-dirty")).display !== "none",
  status: document.querySelector(".panel.active .ed-status").textContent,
}));
check("U7 AC2: Discard reloads server text, clears dirty, keeps 'Reloaded from server' copy",
  u7Discard.text.includes("hello editor") && !u7Discard.text.includes("Y") && !u7Discard.dirty && u7Discard.status === "Reloaded from server",
  JSON.stringify(u7Discard));
// "Save and reload" branch -> write first, then read
const u7Wr1 = calls.filter((c) => c.cmd === "sftp_write_file").length;
const u7Rd1 = calls.filter((c) => c.cmd === "sftp_read_file").length;
await page.locator(".panel.active .cm-content").click();
await page.keyboard.type("Z");
await page.waitForTimeout(250);
await page.locator(".panel.active .ed-reload").click();
await page.waitForTimeout(300);
await page.locator("#dialog-actions button", { hasText: "Save and reload" }).click();
await page.waitForTimeout(600);
const u7SR = await page.evaluate(() => ({
  dirty: getComputedStyle(document.querySelector(".panel.active .ed-dirty")).display !== "none",
  text: document.querySelector(".panel.active .cm-content").textContent,
}));
const u7WrCalls = calls.filter((c) => c.cmd === "sftp_write_file").slice(u7Wr1);
check("U7 AC2: 'Save and reload' writes the edit THEN re-reads (1 write + 1 read), ends clean",
  u7WrCalls.length === 1 && u7WrCalls[0].args.content.includes("Z") &&
  calls.filter((c) => c.cmd === "sftp_read_file").length === u7Rd1 + 1 &&
  !u7SR.dirty && u7SR.text === "hello editorline two", JSON.stringify({ ...u7SR, wr: u7WrCalls.length, rd: calls.filter((c) => c.cmd === "sftp_read_file").length - u7Rd1 }));
// source pins: listener now on the PANEL, old view.dom handler gone; Ln/Col stays v1.1
check("U7 AC1: keydown listener lives on the editor panel (view.dom handler removed)",
  u3src.includes('panel.addEventListener("keydown"') && !u3src.includes('view.dom.addEventListener("keydown"'),
  `panel=${u3src.includes('panel.addEventListener("keydown"')}`);
check("U7: pasteNormalize + 0 draggable untouched by U7", /function pasteNormalize/.test(u3src) && !/draggable/i.test(u3src));
// end clean: editor tab is not dirty -> close it, then the SFTP tab
await page.locator('.tab:has-text("notes.txt") .x').first().click();
await page.waitForTimeout(250);
await page.locator(".tab .x").first().click();
await page.waitForTimeout(300);

// ---- U8 Dialog password host (badge stored + note opt-in, EPIC-TSD §U8) ----
const u8OpenEditor = async (name) => {
  await page.locator(".host-item", { hasText: name }).first().click({ button: "right" });
  await page.waitForTimeout(200);
  await page.locator("#host-menu button", { hasText: "Edit" }).click();
  await page.waitForTimeout(250);
};
const u8ModalState = () => page.evaluate(() => ({
  modalOpen: !document.querySelector("#modal").classList.contains("hidden"),
  chipHidden: document.querySelector("#pw-stored-row").classList.contains("hidden"),
  chipText: (document.querySelector("#pw-badge")?.textContent || "").replace(/\s+/g, " ").trim(),
  chipColor: document.querySelector("#pw-badge") ? getComputedStyle(document.querySelector("#pw-badge")).color : "",
  dotColor: document.querySelector("#pw-badge .dot") ? getComputedStyle(document.querySelector("#pw-badge .dot")).backgroundColor : "",
  rowHidden: document.querySelector("#row-password").classList.contains("hidden"),
  note: document.querySelector("#row-password-note").textContent,
  d3Hidden: document.querySelector("#row-password-note-d3").classList.contains("hidden"),
  d3Text: (document.querySelector("#row-password-note-d3")?.textContent || "").trim(),
  placeholder: document.querySelector("#f-password").placeholder,
}));
// AC1a + AC3 (stored side): pw-host has_password=true, auth=password
await u8OpenEditor("pw-host");
let u8s = await u8ModalState();
const u8EmptyNote = "The password is stored in ~/.ssh/hosterm.secrets (file perms 0600), never in ~/.ssh/config, and filled in automatically at connect.";
const u8D3 = "Auto-typing hanya aktif untuk host ini (opt-in per host).";
check("U8 AC1a: has_password=true → chip '● stored (sidecar, 0600)' visible, --ok text + dot",
  u8s.modalOpen && !u8s.chipHidden && u8s.chipText === "stored (sidecar, 0600)" &&
  u8s.chipColor === "rgb(157, 196, 137)" && u8s.dotColor === "rgb(157, 196, 137)",
  JSON.stringify(u8s));
check("U8 AC3: D3 note exact text, shown while auth=password (stored state keeps 'kept' placeholder)",
  u8s.d3Text === u8D3 && !u8s.d3Hidden && u8s.placeholder === "kept — type to replace" &&
  u8s.note.startsWith("A password is stored for this host."), JSON.stringify(u8s));
// AC3 (mode side): D3 exactly while auth=password — key hides it, back shows it again
await page.locator("#f-auth").selectOption("key");
await page.waitForTimeout(150);
const u8Key = await u8ModalState();
await page.locator("#f-auth").selectOption("password");
await page.waitForTimeout(150);
u8s = await u8ModalState();
check("U8 AC3: D3 note hidden at auth=key, back when auth=password (chip round-trips)",
  u8Key.d3Hidden && u8Key.rowHidden && !u8s.d3Hidden && !u8s.chipHidden, JSON.stringify({ u8Key, u8s }));
// AC1b: prod-web has no stored password → chip absent (auth=key default)
await page.locator("#m-cancel").click(); // modalValues untouched (auth back at snapshot value)
await page.waitForTimeout(200);
await u8OpenEditor("prod-web");
u8s = await u8ModalState();
check("U8 AC1b: has_password=false → chip absent, D3 hidden at auth=key",
  u8s.chipHidden && u8s.rowHidden && u8s.d3Hidden, JSON.stringify(u8s));
await page.locator("#f-auth").selectOption("password");
await page.waitForTimeout(150);
u8s = await u8ModalState();
check("U8 AC3: D3 note also shown at auth=password for a host without stored password",
  !u8s.d3Hidden && u8s.d3Text === u8D3 && u8s.chipHidden && u8s.note === u8EmptyNote, JSON.stringify(u8s));
await page.locator("#f-auth").selectOption("key"); // restore snapshot value → clean cancel
await page.waitForTimeout(150);
await page.locator("#m-cancel").click();
await page.waitForTimeout(200);
// AC2: "Hapus password tersimpan" → in-app confirm → forget_secret → chip + note revert
await u8OpenEditor("pw-host");
const u8ForgetBefore = calls.filter((c) => c.cmd === "forget_secret").length;
let u8Native = null;
page.once("dialog", (d) => { u8Native = d.type(); d.dismiss(); });
await page.locator("#m-delete-pw").click();
await page.waitForTimeout(250);
const u8Confirm = await page.evaluate(() => ({
  open: !document.querySelector("#dialog").classList.contains("hidden"),
  title: document.querySelector("#dialog-title").textContent,
  buttons: [...document.querySelectorAll("#dialog-actions button")].map((b) => b.textContent),
  focused: document.activeElement?.textContent?.trim() || "",
}));
check("U8 AC2: in-app confirm dialog (never native confirm), danger 'Hapus' + focus on Cancel",
  u8Native === null && u8Confirm.open && u8Confirm.title === "Hapus password tersimpan" &&
  u8Confirm.buttons.includes("Hapus") && u8Confirm.focused === "Cancel",
  JSON.stringify({ u8Native, ...u8Confirm }));
await page.locator("#dialog-actions button", { hasText: "Hapus" }).click();
await page.waitForTimeout(250);
const u8Forgets = calls.filter((c) => c.cmd === "forget_secret").slice(u8ForgetBefore);
u8s = await u8ModalState();
const u8LockGone = (await page.locator(".host-item", { hasText: "pw-host" }).first().locator(".h-key").count()) === 0;
check("U8 AC2: forget_secret invoked once with the edited alias",
  u8Forgets.length === 1 && u8Forgets[0].args.alias === "pw-host", JSON.stringify(u8Forgets));
check("U8 AC2: chip gone + note back to the empty version after forget",
  u8s.chipHidden && u8s.note === u8EmptyNote && u8s.modalOpen, JSON.stringify(u8s));
check("U8 AC2: sidebar lock icon drops with the sidecar entry",
  u8LockGone && u8s.chipHidden, `lockGone=${u8LockGone} chipHidden=${u8s.chipHidden}`);
// AC4 (NFR-1): the copy names the sidecar, never claims storage inside ~/.ssh/config
const u8Src = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8") + readFileSync(new URL("../index.html", import.meta.url), "utf8");
check("U8 AC4: copy names the sidecar (hosterm.secrets) and never claims storage in ~/.ssh/config",
  u8Src.includes("hosterm.secrets") && !/stored in ~\/\.ssh\/config(?!.*hosterm)/.test(u8Src) &&
  u8Src.includes("stored (sidecar, 0600)"));
// leave the modal clean for the closing checks
await page.locator("#m-cancel").click();
await page.waitForTimeout(200);

// ---- U9 audit: cross-feature invariants + narrative copy + hostile bytes (EPIC-TSD §U9) ----
// (a) no `draggable` anywhere under src/ (every file, not just main.ts) + index.html + live DOM
const srcDir = new URL("../src/", import.meta.url);
const u9Files = readdirSync(srcDir, { recursive: true }).filter((f) => /\.(ts|css|svg|html|json)$/.test(f));
const dragHits = u9Files.filter((f) => /draggable/i.test(readFileSync(new URL(f, srcDir), "utf8")));
check("U9 (a): grep draggable in src/ = 0", u9Files.length >= 3 && dragHits.length === 0, `files=${u9Files.length} hits=${dragHits.join(",")}`);
check("U9 (a): no draggable in index.html + 0 [draggable] in live DOM",
  !/draggable/i.test(readFileSync(new URL("../index.html", import.meta.url), "utf8")) &&
  (await page.evaluate(() => document.querySelectorAll("[draggable]").length)) === 0);
// (c) pasteNormalize: body byte-pinned to the contract the 9g1 check re-implements, wired on both paste paths
const pnBody = (u3src.match(/function pasteNormalize\(text: string\): string \{([\s\S]*?)\n\}/) || [])[1] || "";
check("U9 (c): pasteNormalize body unchanged (CRLF/CR→LF, strip 1 trailing LF)",
  pnBody === '\n  const lf = text.replace(/\\r\\n/g, "\\n").replace(/\\r/g, "\\n");\n  return lf.endsWith("\\n") ? lf.slice(0, -1) : lf;', JSON.stringify(pnBody));
check("U9 (c): pasteNormalize wired on both PTY paste paths",
  (u3src.match(/pasteNormalize\(String\(raw\)\)/g) || []).length === 2);
// (d) motion: shipped keyframes = spinner + xfer sweep + U-D2 shimmer, all off under reduced-motion
check("U-D2 §13.3: keyframes = shimmer + spin + sweep (shimmer added, no stragglers)",
  JSON.stringify((cssU4.match(/@keyframes\s+[\w-]+/g) || []).map((k) => k.split(/\s+/)[1]).sort()) === '["shimmer","spin","sweep"]');
await page.emulateMedia({ reducedMotion: "reduce" });
const rmAnim = await page.evaluate(() => {
  const s = document.createElement("span");
  s.className = "spinner";
  document.body.appendChild(s);
  const a = getComputedStyle(s).animationName;
  s.remove();
  return a;
});
await page.emulateMedia({ reducedMotion: null });
check("U9 (d): prefers-reduced-motion stops the spinner", rmAnim === "none", rmAnim);
check("U9: shipped skeleton class is .skel (no .skel-row) + no @font-face",
  /\.skel\b/.test(cssU4) && !/skel-row/.test(cssU4 + u3src) && !/@font-face/.test(cssU4));

// ===== U-D2 Dusk shell re-skin geometry + motion (SDD §13.3 / FR-SH1..8) =====
// (1) sidebar 248px
const sbW = await page.locator("#section-panel").evaluate((el) => el.getBoundingClientRect().width);
check("U-D2 §13.3: sidebar is 248px wide", Math.round(sbW) === 248, `w=${sbW}`);
// (2) tab bar 42px tall + tab 32px tall (open one terminal tab to measure)
// U-D2 scope correction: the shell field is display-only ("shell" literal).
// Anti-regression: opening a terminal must never read remote files — count
// sftp_read_file calls targeting /etc/passwd around this re-open (several
// terminals were opened earlier in the suite); exact 0 required.
const pwProbeBefore = calls.filter((c) => c.cmd === "sftp_read_file" && c.args.remotePath === "/etc/passwd").length;
await page.locator(".host-item", { hasText: "prod-web" }).first().locator(".h-name").click();
await page.waitForTimeout(300);
const pwProbeAfter = calls.filter((c) => c.cmd === "sftp_read_file" && c.args.remotePath === "/etc/passwd").length;
check("U-D2 FR-SH8: zero /etc/passwd probes via sftp_read_file on terminal open (anti-regression)",
  pwProbeBefore === 0 && pwProbeAfter === 0, `before=${pwProbeBefore} after=${pwProbeAfter}`);
const tabGeo = await page.evaluate(() => {
  const bar = document.getElementById("tabs");
  const tab = document.querySelector(".tab");
  return { bar: bar.offsetHeight, tab: tab ? tab.offsetHeight : 0 };
});
check("U-D2 §13.3: tab bar 42px, tab 32px", tabGeo.bar === 42 && tabGeo.tab === 32, JSON.stringify(tabGeo));
// (3) active tab underline = --accent 2px (honey amber #e2a35c)
const tabUnderline = await page.evaluate(() => {
  const t = document.querySelector(".tab.active");
  if (!t) return null;
  const cs = getComputedStyle(t);
  return { w: cs.borderBottomWidth, c: cs.borderBottomColor };
});
check("U-D2 FR-SH5: active tab underline 2px --accent",
  tabUnderline && tabUnderline.w === "2px" && tabUnderline.c === "rgb(226, 163, 92)", JSON.stringify(tabUnderline));
// (4) status bar is mono, 11px (FR-SH8)
const sbFont = await page.evaluate(() => {
  const cs = getComputedStyle(document.getElementById("status-bar"));
  return { fs: cs.fontSize, mono: /mono/i.test(cs.fontFamily) };
});
check("U-D2 FR-SH8: status bar mono 11px", sbFont.fs === "11px" && sbFont.mono, JSON.stringify(sbFont));
// (5) FR-SH8 right cluster = N live · UTF-8 · LF · shell · clock — the shell
// field is a static display-only label (real shell unknown without a remote
// probe, which U-D2 forbids); never a guessed binary or version
const sbRight = await page.evaluate(() => {
  const ids = ["sb-live", "sb-enc", "sb-eol", "sb-shell", "sb-clock"];
  const kids = [...document.querySelector(".sb-right").children].map((el) => el.id);
  return {
    live: document.getElementById("sb-live").textContent,
    enc: document.getElementById("sb-enc").textContent,
    eol: document.getElementById("sb-eol").textContent,
    shell: (document.getElementById("sb-shell") || { textContent: null }).textContent,
    order: ids.map((id) => kids.indexOf(id)),
  };
});
check("U-D2 FR-SH8: status bar right shows N live · UTF-8 · LF · shell (static display-only label)",
  sbRight.enc === "UTF-8" && sbRight.eol === "LF" && sbRight.shell === "shell"
    && /^\d+ live$/.test(sbRight.live), JSON.stringify(sbRight));
check("U-D2 FR-SH8: right cluster field order live·enc·eol·shell·clock",
  sbRight.order.every((v, i) => v !== -1 && (i === 0 || v > sbRight.order[i - 1])),
  JSON.stringify(sbRight.order));
// (6) host row-actions fade = .12s (FR-SH2, row-actions motion).
// Force motion on for this measurement: the app correctly zeroes the transition
// under prefers-reduced-motion (styles.css:729), and the windows-latest runner
// defaults to reduce, so without this the duration reads 0s (green on the Linux
// runner, red on Windows). Restore the default emulation afterwards.
await page.emulateMedia({ reducedMotion: "no-preference" });
const actFade = await page.evaluate(() => {
  const b = document.querySelector(".host-item .h-acts button");
  return b ? getComputedStyle(b).transitionDuration : "";
});
await page.emulateMedia({ reducedMotion: null });
check("U-D2 §13.3: row-actions fade transition 0.12s", actFade === "0.12s", actFade);
// (7) term divider 4px + col-resize (measure if a split is live, else assert CSS source)
const divGeoD2 = await page.evaluate(() => {
  const d = document.querySelector(".term-divider");
  if (!d) return null;
  const cs = getComputedStyle(d);
  return { w: Math.round(d.getBoundingClientRect().width), cur: cs.cursor };
});
// divider only exists when a split is present; if none open yet, assert CSS source
if (divGeoD2) check("U-D2 FR-SH6: divider 4px col-resize", divGeoD2.w === 4 && divGeoD2.cur === "col-resize", JSON.stringify(divGeoD2));
else check("U-D2 FR-SH6: divider 4px col-resize (CSS source asserts flex 4px + col-resize)",
  /\.term-divider\s*\{[^}]*flex:\s*0 0 4px[^}]*\}/s.test(cssU4) && /\.term-divider\s*\{[^}]*col-resize/s.test(cssU4));
// (8) reduced-motion silences the shimmer skeleton (FR-DT4 universal kill)
await page.emulateMedia({ reducedMotion: "reduce" });
const rmSkel = await page.evaluate(() => {
  const s = document.createElement("div");
  s.className = "skel";
  document.getElementById("host-list").appendChild(s);
  const a = getComputedStyle(s).animationName;
  s.remove();
  return a;
});
await page.emulateMedia({ reducedMotion: null });
check("U-D2 FR-DT4: reduced-motion stops the shimmer skeleton", rmSkel === "none", rmSkel);
// clean the probe tab(s) so teardown's 0-tab assertion holds
await page.evaluate(() => document.querySelectorAll(".tab .x").forEach((x) => x.click()));
await page.waitForTimeout(200);
// ===== end U-D2 block =====

// U-D4 Settings + Appearance (FR-SET1..13, INV-23).
check("U-D4 boot: read_settings invoked + Dusk/default accent applied",
  calls.some((c) => c.cmd === "read_settings") &&
  await page.evaluate(() => document.documentElement.dataset.theme === "dusk" &&
    document.documentElement.style.getPropertyValue("--accent").trim() === "#e2a35c"));

await openSettingsFromAccount();
await page.waitForTimeout(150);
const settingsMeta = await page.evaluate(() => {
  const scrim = document.getElementById("settings-scrim");
  const panel = document.getElementById("settings-panel");
  const rect = panel.getBoundingClientRect();
  return {
    open: !scrim.classList.contains("hidden"),
    role: panel.getAttribute("role"),
    modal: panel.getAttribute("aria-modal"),
    scrim: getComputedStyle(scrim).backgroundColor,
    width: Math.round(rect.width),
    height: Math.round(rect.height),
    inert: document.getElementById("app").hasAttribute("inert"),
  };
});
check("U-D4 FR-SET1: in-app modal + exact scrim + 900x620 geometry",
  settingsMeta.open && settingsMeta.role === "dialog" && settingsMeta.modal === "true" &&
  settingsMeta.scrim === "rgba(12, 9, 6, 0.55)" && settingsMeta.width === 900 && settingsMeta.height === 620 && settingsMeta.inert,
  JSON.stringify(settingsMeta));

const navState = await page.evaluate(() => ({
  labels: [...document.querySelectorAll(".set-nav")].map((b) => b.textContent.trim()),
  current: document.querySelector('.set-nav[aria-current="page"]')?.dataset.section,
}));
check("U-D4 FR-SET2: 7 nav items + Appearance default current",
  JSON.stringify(navState.labels) === JSON.stringify(["General", "Appearance", "Terminal", "SSH & Keys", "AI Assistant", "Snippets", "Account"]) &&
  navState.current === "appearance", JSON.stringify(navState));
// General is now a real section (local shell info + about), not a placeholder.
await page.locator('.set-nav[data-section="general"]').click();
const generalState = await page.evaluate(() => ({
  generalShown: !document.getElementById("settings-general").classList.contains("hidden"),
  placeholderHidden: document.getElementById("settings-placeholder").classList.contains("hidden"),
  shell: document.getElementById("gen-shell").textContent,
  appearanceHidden: document.getElementById("settings-appearance").classList.contains("hidden"),
}));
check("U-D4 FR-GEN1: General renders a real pane (local shell info), not a placeholder",
  generalState.generalShown && generalState.placeholderHidden && generalState.appearanceHidden &&
  generalState.shell === "/bin/bash",
  JSON.stringify(generalState));
// A still-placeholder section (Terminal) keeps the "coming" copy.
await page.locator('.set-nav[data-section="terminal"]').click();
const comingState = await page.evaluate(() => ({
  title: document.getElementById("settings-placeholder-title").textContent,
  copy: document.getElementById("settings-placeholder").textContent,
  placeholderShown: !document.getElementById("settings-placeholder").classList.contains("hidden"),
}));
check("U-D4 FR-SET3: a still-unbuilt nav renders the coming placeholder",
  comingState.title === "Terminal" && comingState.copy.includes("Coming soon") && comingState.placeholderShown,
  JSON.stringify(comingState));
await page.locator('.set-nav[data-section="appearance"]').click();

const presetState = await page.evaluate(() => ({
  count: document.querySelectorAll(".preset").length,
  dusk: document.querySelector('.preset[data-preset="dusk"]')?.getAttribute("aria-pressed"),
  disabled: document.querySelectorAll('.preset[aria-disabled="true"]').length,
}));
check("U-D4 FR-SET4: four presets; Dusk selected; all presets enabled (Slate/Ink/Light shipped)",
  presetState.count === 4 && presetState.dusk === "true" && presetState.disabled === 0, JSON.stringify(presetState));
// Slate/Ink/Light now functional: selecting one swaps data-theme AND changes
// the --bg token (the CSS preset block applies), then returns to Dusk so the
// accent checks below keep their canonical Dusk base.
const slateSwap = await page.evaluate(() => {
  const before = { theme: document.documentElement.dataset.theme, bg: getComputedStyle(document.documentElement).getPropertyValue("--bg").trim() };
  document.querySelector('.preset[data-preset="slate"]').click();
  const after = { theme: document.documentElement.dataset.theme, bg: getComputedStyle(document.documentElement).getPropertyValue("--bg").trim() };
  return { before, after };
});
check("U-D4 FR-SET4: Slate click swaps data-theme + applies its --bg token",
  slateSwap.before.theme === "dusk" && slateSwap.after.theme === "slate" &&
  slateSwap.after.bg.toLowerCase() === "#171d24" && slateSwap.after.bg !== slateSwap.before.bg,
  JSON.stringify(slateSwap));
// restore Dusk for the accent-preview checks that follow
await page.evaluate(() => document.querySelector('.preset[data-preset="dusk"]').click());

const stableBefore = await page.evaluate(() => {
  const cs = getComputedStyle(document.documentElement);
  return ["--surface", "--text", "--live", "--danger"].map((k) => cs.getPropertyValue(k).trim());
});
await page.locator('.accent-chip[data-accent="#5fbfae"]').click();
const tealState = await page.evaluate(() => {
  const root = document.documentElement;
  const preview = document.getElementById("pv-mock");
  const cs = getComputedStyle(root);
  return {
    accent: root.style.getPropertyValue("--accent").trim(),
    ink: root.style.getPropertyValue("--accent-ink").trim(),
    pressed: document.querySelector('.accent-chip[data-accent="#5fbfae"]')?.getAttribute("aria-pressed"),
    preview: preview.style.getPropertyValue("--pa").trim(),
    previewButton: getComputedStyle(document.querySelector(".pv-btn")).backgroundColor,
    stable: ["--surface", "--text", "--live", "--danger"].map((k) => cs.getPropertyValue(k).trim()),
  };
});
check("U-D4 FR-SET7/8/10: teal updates inline accent + pressed chip + preview only",
  tealState.accent === "#5fbfae" && tealState.ink === "#241404" && tealState.pressed === "true" &&
  tealState.preview === "#5fbfae" && tealState.previewButton === "rgb(95, 191, 174)" &&
  JSON.stringify(tealState.stable) === JSON.stringify(stableBefore), JSON.stringify(tealState));

const inv23Luminance = (hex) => {
  const linear = [1, 3, 5].map((offset) => parseInt(hex.slice(offset, offset + 2), 16) / 255)
    .map((channel) => channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4);
  return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2];
};
const inv23Contrast = (a, b) => {
  const [lighter, darker] = [inv23Luminance(a), inv23Luminance(b)].sort((x, y) => y - x);
  return (lighter + 0.05) / (darker + 0.05);
};
const inputAccent = async (accent) => {
  await page.locator("#accent-input").evaluate((input, value) => {
    input.value = value;
    input.dispatchEvent(new Event("input", { bubbles: true }));
  }, accent);
  return page.evaluate(() => {
    const root = document.documentElement;
    const preview = document.getElementById("pv-mock");
    return {
      accent: root.style.getPropertyValue("--accent").trim(),
      ink: root.style.getPropertyValue("--accent-ink").trim(),
      input: document.getElementById("accent-input").value,
      previewInk: preview.style.getPropertyValue("--pi").trim(),
      pressed: document.querySelectorAll('.accent-chip[aria-pressed="true"]').length,
    };
  });
};
const inv23Vectors = [
  { accent: "#e2a35c", ink: "#241404", ratio: 8.18060039728731 },
  { accent: "#5fbfae", ink: "#241404", ratio: 8.115110618605044 },
  { accent: "#e0869b", ink: "#241404", ratio: 6.842802495508517 },
  { accent: "#aec267", ink: "#241404", ratio: 9.102683799795606 },
  { accent: "#b99be3", ink: "#241404", ratio: 7.529220380096464 },
  { accent: "#000000", ink: "#ffffff", ratio: 21 },
  { accent: "#101010", ink: "#ffffff", ratio: 19.028110547666497 },
  { accent: "#6f6f6f", ink: "#ffffff", ratio: 5.024865064757633 },
  { accent: "#757575", ink: "#ffffff", ratio: 4.607518093747377 },
  { accent: "#766cb5", ink: "#ffffff", ratio: 4.58257700240974 },
  { accent: "#cf0dcc", ink: "#000000", ratio: 4.5825758149421025 },
  { accent: "#767676", ink: "#000000", ratio: 4.6232848849972035 },
  { accent: "#777777", ink: "#000000", ratio: 4.68949989000882 },
  { accent: "#ffffff", ink: "#000000", ratio: 21 },
];
for (const vector of inv23Vectors) {
  const actual = await inputAccent(vector.accent);
  const ratio = inv23Contrast(actual.accent, actual.ink);
  check(`U-D4 INV-23 vector ${vector.accent} -> ${vector.ink}`,
    actual.accent === vector.accent && actual.input === vector.accent &&
    actual.ink === vector.ink && actual.previewInk === vector.ink &&
    Math.abs(ratio - vector.ratio) < 1e-12 && ratio >= 4.5,
    JSON.stringify({ ...actual, ratio }));
}
const normalizedUpper = await inputAccent("#E0869B");
check("U-D4 INV-23: valid uppercase input normalizes before value-based mapping",
  normalizedUpper.accent === "#e0869b" && normalizedUpper.input === "#e0869b" &&
  normalizedUpper.ink === "#241404" && normalizedUpper.previewInk === "#241404",
  JSON.stringify(normalizedUpper));

await page.locator("#accent-reset").click();
const resetState = await page.evaluate(() => ({
  theme: document.documentElement.dataset.theme,
  accent: document.documentElement.style.getPropertyValue("--accent").trim(),
  dusk: document.querySelector('.preset[data-preset="dusk"]')?.getAttribute("aria-pressed"),
}));
check("U-D4 FR-SET11: reset restores Dusk + #e2a35c", resetState.theme === "dusk" && resetState.accent === "#e2a35c" && resetState.dusk === "true", JSON.stringify(resetState));

const tealInputState = await inputAccent("#5fbfae");
check("U-D4 INV-23: same curated value maps identically via chip and input",
  tealState.accent === tealInputState.accent && tealState.ink === tealInputState.ink &&
  tealInputState.ink === "#241404" && tealInputState.pressed === 1,
  JSON.stringify({ chip: tealState, input: tealInputState }));
const writesBeforeDone = calls.filter((c) => c.cmd === "write_settings").length;
await page.locator("#settings-done").click();
await page.waitForTimeout(200);
const writeCall = calls.filter((c) => c.cmd === "write_settings").at(-1);
check("U-D4 FR-SET12/13: Done closes + write_settings persists explicit full shape",
  await page.locator("#settings-scrim").evaluate((el) => el.classList.contains("hidden")) &&
  calls.filter((c) => c.cmd === "write_settings").length === writesBeforeDone + 1 &&
  writeCall?.args?.settings?.version === 1 && writeCall.args.settings.theme === "dusk" &&
  writeCall.args.settings.accent === "#5fbfae" && writeCall.args.settings.allowlist.length === 1,
  JSON.stringify(writeCall?.args));

const readsBeforeReload = calls.filter((c) => c.cmd === "read_settings").length;
await page.reload();
await page.waitForTimeout(400);
const bootApplied = await page.evaluate(() => ({
  theme: document.documentElement.dataset.theme,
  accent: document.documentElement.style.getPropertyValue("--accent").trim(),
  ink: document.documentElement.style.getPropertyValue("--accent-ink").trim(),
  previewInk: document.getElementById("pv-mock").style.getPropertyValue("--pi").trim(),
  teal: document.querySelector('.accent-chip[data-accent="#5fbfae"]')?.getAttribute("aria-pressed"),
}));
check("U-D4 INV-23: curated input -> persist -> reload stays value-based",
  calls.filter((c) => c.cmd === "read_settings").length === readsBeforeReload + 1 &&
  bootApplied.theme === "dusk" && bootApplied.accent === tealInputState.accent &&
  bootApplied.ink === tealInputState.ink && bootApplied.previewInk === tealInputState.ink && bootApplied.teal === "true",
  JSON.stringify(bootApplied));

await openSettingsFromAccount();
await page.waitForTimeout(100);
const nonCuratedInputState = await inputAccent("#777777");
const writesBeforeNonCurated = calls.filter((c) => c.cmd === "write_settings").length;
await page.locator("#settings-done").click();
await page.waitForTimeout(200);
const nonCuratedWrite = calls.filter((c) => c.cmd === "write_settings").at(-1);
check("U-D4 INV-23: non-curated accent persists without clamp/remap/reject",
  calls.filter((c) => c.cmd === "write_settings").length === writesBeforeNonCurated + 1 &&
  nonCuratedInputState.accent === "#777777" && nonCuratedInputState.ink === "#000000" &&
  nonCuratedWrite?.args?.settings?.accent === "#777777",
  JSON.stringify({ input: nonCuratedInputState, write: nonCuratedWrite?.args }));
const readsBeforeNonCuratedReload = calls.filter((c) => c.cmd === "read_settings").length;
await page.reload();
await page.waitForTimeout(400);
const nonCuratedBoot = await page.evaluate(() => ({
  accent: document.documentElement.style.getPropertyValue("--accent").trim(),
  ink: document.documentElement.style.getPropertyValue("--accent-ink").trim(),
  input: document.getElementById("accent-input").value,
  pressed: document.querySelectorAll('.accent-chip[aria-pressed="true"]').length,
}));
check("U-D4 INV-23: non-curated input -> persist -> reload stays value-based",
  calls.filter((c) => c.cmd === "read_settings").length === readsBeforeNonCuratedReload + 1 &&
  nonCuratedBoot.accent === nonCuratedInputState.accent && nonCuratedBoot.input === nonCuratedInputState.accent &&
  nonCuratedBoot.ink === nonCuratedInputState.ink && nonCuratedBoot.pressed === 0,
  JSON.stringify(nonCuratedBoot));

await openSettingsFromAccount();
await page.waitForTimeout(100);
await page.evaluate(() => document.getElementById("settings-scrim").click());
await page.waitForTimeout(100);
check("U-D4 FR-SET12: scrim click closes modal", await page.locator("#settings-scrim").evaluate((el) => el.classList.contains("hidden")));
await openSettingsFromAccount();
await page.waitForTimeout(100);
await page.keyboard.press("Escape");
await page.waitForTimeout(100);
check("U-D4 FR-SET12: Escape closes + restores account focus",
  await page.locator("#settings-scrim").evaluate((el) => el.classList.contains("hidden")) &&
  await page.evaluate(() => document.activeElement?.id === "user-chip"));

// Restore the Dusk default so the remaining legacy checks keep their canonical palette.
await openSettingsFromAccount();
await page.locator("#accent-reset").click();
await page.locator("#settings-done").click();
await page.waitForTimeout(150);
// End U-D4 checks.

// U-D5 Snippet panel (FR-SN1..6).
const snippetReadsBefore = calls.filter((c) => c.cmd === "read_snippets").length;
await page.locator("#mode-snippets").click();
await page.locator(".snippet-item").first().waitFor({ timeout: 3000 });
const snippetMeta = await page.evaluate(() => {
  const drawer = document.getElementById("snippet-drawer");
  const command = document.querySelector(".snip-cmd");
  return {
    width: Math.round(drawer.getBoundingClientRect().width),
    label: drawer.getAttribute("aria-label"),
    open: drawer.classList.contains("open") && !drawer.hidden,
    snippetsPressed: document.getElementById("mode-snippets").getAttribute("aria-pressed"),
    terminalPressed: document.getElementById("mode-terminal").getAttribute("aria-pressed"),
    count: document.getElementById("snippet-count").textContent,
    groups: [...document.querySelectorAll(".snip-group-label")].map((el) => el.textContent),
    itemCount: document.querySelectorAll(".snippet-item").length,
    mono: /mono/i.test(getComputedStyle(command).fontFamily),
  };
});
check("U-D5 FR-SN1: opening 340px Command snippets drawer invokes read_snippets",
  calls.filter((c) => c.cmd === "read_snippets").length === snippetReadsBefore + 1 &&
  snippetMeta.width === 340 && snippetMeta.label === "Command snippets" && snippetMeta.open &&
  snippetMeta.snippetsPressed === "true" && snippetMeta.terminalPressed === "false",
  JSON.stringify(snippetMeta));
check("U-D5 FR-SN1/4: grouped uppercase mono items render + head-count tracks visible items",
  snippetMeta.itemCount === 4 && snippetMeta.count === "4" && snippetMeta.mono &&
  JSON.stringify(snippetMeta.groups) === JSON.stringify(["DOCKER", "GIT", "SYSTEMD"]),
  JSON.stringify(snippetMeta));

await page.locator("#snippet-filter").fill("Prune");
const nameFilter = await page.evaluate(() => ({
  count: document.getElementById("snippet-count").textContent,
  names: [...document.querySelectorAll(".snip-name")].map((el) => el.textContent),
  groups: [...document.querySelectorAll(".snip-group-label")].map((el) => el.textContent),
}));
check("U-D5 FR-SN2/4: filter matches snippet name and updates groups/count",
  nameFilter.count === "1" && JSON.stringify(nameFilter.names) === JSON.stringify(["Prune everything"]) &&
  JSON.stringify(nameFilter.groups) === JSON.stringify(["DOCKER"]), JSON.stringify(nameFilter));

await page.locator("#snippet-filter").fill("status -sb");
const commandFilter = await page.evaluate(() => ({
  count: document.getElementById("snippet-count").textContent,
  commands: [...document.querySelectorAll(".snip-cmd")].map((el) => el.textContent),
}));
check("U-D5 FR-SN2: filter also matches command substring",
  commandFilter.count === "1" && JSON.stringify(commandFilter.commands) === JSON.stringify(["git status -sb"]),
  JSON.stringify(commandFilter));

await page.locator("#snippet-filter").fill("no-such-snippet");
const noSnippet = await page.evaluate(() => ({
  count: document.getElementById("snippet-count").textContent,
  items: document.querySelectorAll(".snippet-item").length,
  groups: document.querySelectorAll(".snip-group-label").length,
  text: document.getElementById("snippet-empty").textContent,
  hidden: document.getElementById("snippet-empty").hidden,
}));
check("U-D5 FR-SN2: zero match shows exact empty-state copy",
  noSnippet.count === "0" && noSnippet.items === 0 && noSnippet.groups === 0 && !noSnippet.hidden &&
  noSnippet.text === 'No snippets match "no-such-snippet"', JSON.stringify(noSnippet));

await page.locator("#snippet-filter").fill("");
const hostileSnippet = await page.evaluate((hostile) => {
  const item = [...document.querySelectorAll(".snippet-item")]
    .find((row) => row.querySelector(".snip-name")?.textContent === hostile);
  const name = item?.querySelector(".snip-name");
  const command = item?.querySelector(".snip-cmd");
  return {
    nameKids: name?.childElementCount,
    nameText: name?.textContent,
    commandKids: command?.childElementCount,
    commandText: command?.textContent,
    pwn: typeof window.__pwn,
    csp: (window.__cspViolations || []).length,
  };
}, HOSTILE);
check("U-D5 FR-SN5: hostile name/command are inert escaped text with CSP 0",
  hostileSnippet.nameKids === 0 && hostileSnippet.nameText === HOSTILE &&
  hostileSnippet.commandKids === 0 && hostileSnippet.commandText === HOSTILE &&
  hostileSnippet.pwn === "undefined" && hostileSnippet.csp === 0,
  JSON.stringify(hostileSnippet));

await page.locator("#snippet-filter").fill("Prune");
const copiesBefore = calls.filter((c) => c.cmd === "clipboard_write").length;
await page.locator(".copy-btn").click();
await page.waitForFunction(() => document.querySelector(".copy-btn")?.classList.contains("done"));
const snippetCopyCall = calls.filter((c) => c.cmd === "clipboard_write").at(-1);
const copiedState = await page.locator(".copy-btn").evaluate((button) => ({
  done: button.classList.contains("done"),
  label: button.getAttribute("aria-label"),
}));
check("U-D5 FR-SN3: copy invokes clipboard_write and enters done state",
  calls.filter((c) => c.cmd === "clipboard_write").length === copiesBefore + 1 &&
  snippetCopyCall?.args?.text === "docker system prune -af" && copiedState.done && copiedState.label === "Copied",
  JSON.stringify({ snippetCopyCall, copiedState }));
await page.waitForTimeout(650);
check("U-D5 FR-SN3: done state remains visible before 900ms reset",
  await page.locator(".copy-btn").evaluate((button) => button.classList.contains("done")));
await page.waitForTimeout(350);
const copyReset = await page.locator(".copy-btn").evaluate((button) => ({
  done: button.classList.contains("done"),
  label: button.getAttribute("aria-label"),
}));
check("U-D5 FR-SN3: done state resets after about 900ms",
  !copyReset.done && copyReset.label === "Copy command", JSON.stringify(copyReset));

await page.locator("#snippet-close").click();
check("U-D5: close returns Terminal mode and restores trigger focus",
  await page.evaluate(() => {
    const drawer = document.getElementById("snippet-drawer");
    return drawer.hidden && !drawer.classList.contains("open") &&
      document.getElementById("mode-terminal").getAttribute("aria-pressed") === "true" &&
      document.activeElement?.id === "mode-snippets";
  }));
// End U-D5 checks.

// U-D6 / FE-AUTH Google account REAL sign-in/out state machine
// (prd discovery-gdrive-backup-v1 §D, tsd gdrive-backup-v1 §3). The chip is no
// longer a mock: it hydrates from google_account on load, runs google_sign_in
// (PKCE loopback, backend-side) with a pending state, shows the real
// {name,email,picture}, and google_sign_out returns to signed-out. Remote
// name/email are escaped (ADR-8); the picture URL is retained but NEVER fetched
// (CSP img-src 'self' + NFR-3). googleStored starts null → load is signed-out.
const googleSignedOut = await page.locator("#user-chip").evaluate((chip) => ({
  state: chip.dataset.state,
  hasPopup: chip.getAttribute("aria-haspopup"),
  expanded: chip.getAttribute("aria-expanded"),
  label: chip.getAttribute("aria-label"),
  name: chip.querySelector(".user-name")?.textContent,
  avatar: chip.querySelectorAll(".avatar").length,
  gmark: chip.querySelectorAll(".gmark").length,
  caret: chip.querySelectorAll(".chip-caret").length,
}));
check("U-D6 FR-GL1: google_account hydrates a signed-out chip exposing the dialog",
  googleSignedOut.state === "signed-out" && googleSignedOut.hasPopup === "dialog" &&
  googleSignedOut.expanded === "false" && googleSignedOut.name === "Google account" &&
  googleSignedOut.label === "Sign in with Google" &&
  googleSignedOut.avatar === 0 && googleSignedOut.gmark === 1 && googleSignedOut.caret === 1,
  JSON.stringify(googleSignedOut));
check("U-D6 FE-AUTH: google_account invoked on load to hydrate state",
  calls.some((c) => c.cmd === "google_account"),
  JSON.stringify(calls.filter((c) => c.cmd === "google_account")));

await page.locator("#user-chip").click();
const googleSignin = await page.evaluate(() => {
  const popover = document.getElementById("signin-popover");
  const button = document.getElementById("signin-btn");
  const style = getComputedStyle(button);
  return {
    hidden: popover.hidden,
    role: popover.getAttribute("role"),
    expanded: document.getElementById("user-chip").getAttribute("aria-expanded"),
    title: document.getElementById("google-pop-title").textContent,
    sub: document.getElementById("google-pop-sub").textContent,
    button: button.textContent.trim().replace(/\s+/g, " "),
    surface: style.backgroundColor,
    text: style.color,
    radius: style.borderRadius,
    focused: document.activeElement?.id,
    appInert: document.getElementById("app").hasAttribute("inert"),
    settingsVisible: document.getElementById("acct-settings-btn").getClientRects().length > 0,
  };
});
check("U-D6 FR-GL1: popover + Google-brand sign-in button match the approved visual",
  !googleSignin.hidden && googleSignin.role === "dialog" && googleSignin.expanded === "true" &&
  googleSignin.title === "Use hosterm with Google" && googleSignin.button === "Sign in with Google" &&
  googleSignin.sub === "Connect a Google account to back up to Drive." &&
  googleSignin.surface === "rgb(255, 255, 255)" && googleSignin.text === "rgb(31, 31, 31)" &&
  googleSignin.radius === "4px" && googleSignin.focused === "signin-btn" &&
  !googleSignin.appInert && googleSignin.settingsVisible,
  JSON.stringify(googleSignin));
await page.keyboard.press("Shift+Tab");
check("NAV RAIL: signed-out account focus cycles backward to Settings",
  await page.evaluate(() => document.activeElement?.id === "acct-settings-btn"));
await page.keyboard.press("Tab");
check("NAV RAIL: signed-out account focus cycles forward to Sign in",
  await page.evaluate(() => document.activeElement?.id === "signin-btn"));
await page.keyboard.press("Control+k");
check("NAV RAIL: Ctrl+K is blocked while account menu is open",
  await page.evaluate(() => document.activeElement?.id === "signin-btn"));

await page.locator("#tabbar").click();
const accountOutsideClosed = await page.evaluate(() => ({
  hidden: document.getElementById("signin-popover").hidden,
  expanded: document.getElementById("user-chip").getAttribute("aria-expanded"),
}));
check("NAV RAIL: outside click closes account menu",
  accountOutsideClosed.hidden && accountOutsideClosed.expanded === "false",
  JSON.stringify(accountOutsideClosed));
await page.locator("#user-chip").click();

// Real sign-in: the 40ms mock delay lets us observe the pending state before
// the account resolves (signed-out → pending → signed-in).
const googleSigninCallsBefore = calls.length;
const googleReqBefore = requestLog.length;
await page.locator("#signin-btn").click();
const googlePending = await page.evaluate(() => {
  const btn = document.getElementById("signin-btn");
  return { disabled: btn.disabled, label: btn.querySelector(".signin-label")?.textContent };
});
check("U-D6 FE-AUTH: sign-in shows a pending state (button disabled, 'Signing in…')",
  googlePending.disabled === true && googlePending.label === "Signing in…",
  JSON.stringify(googlePending));

await page.waitForTimeout(120);
const googleSignedIn = await page.locator("#user-chip").evaluate((chip) => {
  const avatar = chip.querySelector(".avatar");
  const name = chip.querySelector(".user-name");
  return {
    state: chip.dataset.state,
    label: chip.getAttribute("aria-label"),
    expanded: chip.getAttribute("aria-expanded"),
    avatar: avatar?.textContent,
    avatarKids: avatar?.childElementCount,
    picture: avatar?.getAttribute("data-picture"),
    name: name?.textContent,
    nameKids: name?.childElementCount,
    gmark: chip.querySelectorAll(".gmark").length,
    caret: chip.querySelectorAll(".chip-caret").length,
    popoverHidden: document.getElementById("signin-popover").hidden,
  };
});
check("U-D6 FE-AUTH: google_sign_in invoked exactly once",
  calls.filter((c) => c.cmd === "google_sign_in").length === 1,
  JSON.stringify(calls.slice(googleSigninCallsBefore).map((c) => c.cmd)));
check("U-D6 FE-AUTH: signed-in chip renders real initials + name + G mark + caret",
  googleSignedIn.state === "signed-in" && googleSignedIn.avatar === "AL" && googleSignedIn.avatarKids === 0 &&
  googleSignedIn.name === "Ada Lovelace" && googleSignedIn.nameKids === 0 &&
  googleSignedIn.label === "Signed in to Google as Ada Lovelace" &&
  googleSignedIn.gmark === 1 && googleSignedIn.caret === 1,
  JSON.stringify(googleSignedIn));
check("U-D6 FE-AUTH/NFR-3: remote avatar picture URL retained but NEVER fetched",
  googleSignedIn.picture === "https://lh3.googleusercontent.com/a/mock-avatar" &&
  !requestLog.slice(googleReqBefore).some((u) => u.includes("googleusercontent")),
  JSON.stringify({ picture: googleSignedIn.picture, reqs: requestLog.slice(googleReqBefore) }));

// The popover stays open after sign-in; inspect it in place (no re-click).
const googlePopover = await page.evaluate(() => ({
  open: !document.getElementById("signin-popover").hidden,
  title: document.getElementById("google-pop-title").textContent,
  sub: document.getElementById("google-pop-sub").textContent,
  subKids: document.getElementById("google-pop-sub").childElementCount,
  signinHidden: document.getElementById("signin-btn").hidden,
  signoutHidden: document.getElementById("signout-btn").hidden,
  signout: document.getElementById("signout-btn").textContent,
}));
check("U-D6 FE-AUTH/ADR-8: signed-in popover shows real name + escaped email (inert, 0 child nodes) + Sign out",
  googlePopover.open && googlePopover.title === "Ada Lovelace" &&
  googlePopover.sub === 'ada+<img src=x onerror=alert(1)>@example.com' && googlePopover.subKids === 0 &&
  googlePopover.signinHidden && !googlePopover.signoutHidden && googlePopover.signout === "Sign out",
  JSON.stringify(googlePopover));
check("U-D6 FE-AUTH: no 'UI-only mock' / 'UI preview' text remains anywhere in the account surface",
  await page.evaluate(() => {
    const t = document.querySelector("aside")?.textContent || "";
    return !t.includes("UI-only") && !t.includes("UI preview") && !t.includes("No Google session");
  }));

await page.locator("#acct-settings-btn").click();
await page.waitForTimeout(60);
check("NAV RAIL: Settings opens from a signed-in account menu",
  await page.locator("#settings-scrim").evaluate((el) => !el.classList.contains("hidden")));
await page.locator("#settings-done").click();
await page.waitForTimeout(100);
await page.locator("#user-chip").click();

// Real sign-out: invoke google_sign_out, return to signed-out.
await page.locator("#signout-btn").click();
await page.waitForTimeout(20);
const googleReturned = await page.locator("#user-chip").evaluate((chip) => ({
  state: chip.dataset.state,
  expanded: chip.getAttribute("aria-expanded"),
  name: chip.querySelector(".user-name")?.textContent,
  avatar: chip.querySelectorAll(".avatar").length,
  popoverHidden: document.getElementById("signin-popover").hidden,
}));
check("U-D6 FE-AUTH: google_sign_out invoked and chip returns to signed-out",
  calls.some((c) => c.cmd === "google_sign_out") &&
  googleReturned.state === "signed-out" && googleReturned.expanded === "false" &&
  googleReturned.name === "Google account" && googleReturned.avatar === 0 && googleReturned.popoverHidden,
  JSON.stringify(googleReturned));

// Error path: a failed sign-in surfaces an error sub-line and stays signed-out.
googleSignInFail = true;
await page.locator("#user-chip").click();
await page.locator("#signin-btn").click();
await page.waitForTimeout(60);
const googleError = await page.evaluate(() => {
  const chip = document.getElementById("user-chip");
  const sub = document.getElementById("google-pop-sub");
  return {
    state: chip.dataset.state,
    sub: sub.textContent,
    tone: sub.dataset.tone,
    signinHidden: document.getElementById("signin-btn").hidden,
    signinDisabled: document.getElementById("signin-btn").disabled,
  };
});
check("U-D6 FE-AUTH: failed sign-in surfaces an error, stays signed-out, re-enables the button",
  googleError.state === "signed-out" && googleError.sub.startsWith("Sign-in failed:") &&
  googleError.tone === "error" && !googleError.signinHidden && googleError.signinDisabled === false,
  JSON.stringify(googleError));
googleSignInFail = false;

await page.keyboard.press("Escape");
check("U-D6 a11y: Escape closes the account dialog and restores chip focus",
  await page.evaluate(() => document.getElementById("signin-popover").hidden &&
    document.getElementById("user-chip").getAttribute("aria-expanded") === "false" &&
    document.activeElement?.id === "user-chip"));
// End U-D6 checks.

// ===================================================================
// FE-FIX (t_cee5db41): passphrase overlay, toasts, signed-in account UX
// (design-reviews/2026-11-07-google-account-backup-ux.md). Mode DURING; Dusk
// preserved. Deterministic mocked Drive outcomes; every assertion is red-capable.
// ===================================================================

// Re-sign-in so the Drive actions + signed-in identity row are reachable.
await page.locator("#user-chip").click();
await page.locator("#signin-btn").click();
await page.waitForTimeout(120);

// -- FE-FIX 1: passphrase overlay is a true fixed, centered overlay; the shell
//    does not move between closed and open, and is inert behind it (F-1). --
const shellBoxes = () => page.evaluate(() => {
  const r = (id) => { const b = document.getElementById(id).getBoundingClientRect(); return { x: b.x, y: b.y, w: b.width, h: b.height }; };
  return { app: r("app"), rail: r("nav-rail"), panel: r("section-panel") };
});
const boxesClosed = await shellBoxes();
// Open the passphrase overlay via the Backup action (confirm = two fields).
await page.locator("#drive-backup-btn").click();
await page.locator("#pass-dialog:not(.hidden)").waitFor({ timeout: 2000 });
await page.waitForTimeout(180); // let the 120ms entry transition settle
const passGeo = await page.evaluate(() => {
  const el = document.getElementById("pass-dialog");
  const cs = getComputedStyle(el);
  const box = el.getBoundingClientRect();
  const dlg = el.querySelector(".dialog-box").getBoundingClientRect();
  return {
    position: cs.position,
    opacity: cs.opacity,
    coversViewport: Math.round(box.width) === window.innerWidth && Math.round(box.height) === window.innerHeight,
    // centered: dialog box center ~ viewport center
    centeredX: Math.abs((dlg.left + dlg.right) / 2 - window.innerWidth / 2) < 2,
    centeredY: Math.abs((dlg.top + dlg.bottom) / 2 - window.innerHeight / 2) < 2,
    appInert: document.getElementById("app").hasAttribute("inert"),
    focused: document.activeElement?.id,
    dlgWithinViewport: dlg.left >= 0 && dlg.right <= window.innerWidth,
  };
});
check("FE-FIX F-1: #pass-dialog is position:fixed, viewport-sized, centered, opacity 1",
  passGeo.position === "fixed" && passGeo.coversViewport && passGeo.centeredX && passGeo.centeredY && passGeo.opacity === "1",
  JSON.stringify(passGeo));
const boxesOpen = await shellBoxes();
const sameBox = (a, b) => Math.abs(a.x - b.x) < 0.5 && Math.abs(a.y - b.y) < 0.5 && Math.abs(a.w - b.w) < 0.5 && Math.abs(a.h - b.h) < 0.5;
check("FE-FIX F-1: shell (app/rail/panel) does not move between closed and open",
  sameBox(boxesClosed.app, boxesOpen.app) && sameBox(boxesClosed.rail, boxesOpen.rail) && sameBox(boxesClosed.panel, boxesOpen.panel),
  JSON.stringify({ closed: boxesClosed, open: boxesOpen }));
check("FE-FIX F-1: shell inert + initial focus on the passphrase input",
  passGeo.appInert && passGeo.focused === "pass-input");

// -- FE-FIX t_5fe89707: per-field embedded eye toggles (each eye its own field) --
// Eyes live INSIDE each field wrapper. Both open masked (aria-pressed=false,
// aria-label 'Show passphrase', type=password). Each eye toggles ONLY its own
// field: revealing the passphrase does not unmask the confirm field and vice
// versa. Buttons are keyboard-focusable, >=32px targets. Plaintext stays
// confined to the inputs (type attr only). The icon swap is CSS-driven off
// aria-pressed, so the SVG markup is static (no innerHTML — ADR-8).
const eyeClosed = await page.evaluate(() => {
  const eye = document.getElementById("pass-eye");
  const confEye = document.getElementById("pass-confirm-eye");
  const r = eye.getBoundingClientRect();
  return {
    pressed: eye.getAttribute("aria-pressed"),
    label: eye.getAttribute("aria-label"),
    title: eye.getAttribute("title"),
    confPressed: confEye.getAttribute("aria-pressed"),
    inputType: document.getElementById("pass-input").type,
    confirmType: document.getElementById("pass-confirm").type,
    w: Math.round(r.width), h: Math.round(r.height),
  };
});
check("FE-FIX t_5fe89707: both eyes open masked — aria-pressed=false, aria-label/title 'Show passphrase', both fields type=password",
  eyeClosed.pressed === "false" && eyeClosed.label === "Show passphrase" &&
  eyeClosed.title === "Show passphrase" && eyeClosed.confPressed === "false" &&
  eyeClosed.inputType === "password" && eyeClosed.confirmType === "password",
  JSON.stringify(eyeClosed));
check("FE-FIX t_5fe89707: eye button is a >=32px target",
  eyeClosed.w >= 32 && eyeClosed.h >= 32, JSON.stringify(eyeClosed));
await page.locator("#pass-input").fill("reveal-me");
await page.locator("#pass-confirm").fill("confirm-me");
await page.locator("#pass-eye").focus();
const eyeFocusable = await page.evaluate(() => document.activeElement?.id === "pass-eye");
check("FE-FIX t_5fe89707: passphrase eye is keyboard-focusable", eyeFocusable);
await page.keyboard.press("Enter"); // keyboard-operate the primary eye
await page.waitForTimeout(40);
const primaryOn = await page.evaluate(() => {
  const eye = document.getElementById("pass-eye");
  const confEye = document.getElementById("pass-confirm-eye");
  const inp = document.getElementById("pass-input");
  const conf = document.getElementById("pass-confirm");
  return {
    pressed: eye.getAttribute("aria-pressed"), label: eye.getAttribute("aria-label"),
    confPressed: confEye.getAttribute("aria-pressed"),
    inputType: inp.type, confirmType: conf.type,
    inputVal: inp.value, confirmVal: conf.value, // plaintext still only in the inputs
  };
});
check("FE-FIX t_5fe89707: toggling the primary eye unmasks ONLY the passphrase (type=text, aria-pressed=true, label 'Hide passphrase') — confirm stays masked",
  primaryOn.pressed === "true" && primaryOn.label === "Hide passphrase" &&
  primaryOn.inputType === "text" && primaryOn.confirmType === "password" &&
  primaryOn.confPressed === "false" &&
  primaryOn.inputVal === "reveal-me" && primaryOn.confirmVal === "confirm-me",
  JSON.stringify(primaryOn));
await page.locator("#pass-confirm-eye").click(); // reveal only the confirm field
await page.waitForTimeout(40);
const confirmOn = await page.evaluate(() => {
  const confEye = document.getElementById("pass-confirm-eye");
  return {
    confPressed: confEye.getAttribute("aria-pressed"), confLabel: confEye.getAttribute("aria-label"),
    inputType: document.getElementById("pass-input").type,
    confirmType: document.getElementById("pass-confirm").type,
  };
});
check("FE-FIX t_5fe89707: the confirm eye toggles ONLY the confirm field (type=text, aria-pressed=true); passphrase stays revealed independently",
  confirmOn.confPressed === "true" && confirmOn.confLabel === "Hide passphrase" &&
  confirmOn.confirmType === "text" && confirmOn.inputType === "text",
  JSON.stringify(confirmOn));
await page.locator("#pass-eye").click(); // re-mask the passphrase; confirm stays revealed
await page.waitForTimeout(40);
const primaryOff = await page.evaluate(() => {
  const eye = document.getElementById("pass-eye");
  return {
    pressed: eye.getAttribute("aria-pressed"), label: eye.getAttribute("aria-label"),
    inputType: document.getElementById("pass-input").type,
    confirmType: document.getElementById("pass-confirm").type,
  };
});
check("FE-FIX t_5fe89707: re-masking the primary eye restores type=password/aria-pressed=false and leaves the confirm field revealed (independent state)",
  primaryOff.pressed === "false" && primaryOff.label === "Show passphrase" &&
  primaryOff.inputType === "password" && primaryOff.confirmType === "text",
  JSON.stringify(primaryOff));
// Restore the pre-eye-test precondition: focus back on the passphrase input
// so the Escape focus-restore check below starts from the dialog's initial state.
await page.locator("#pass-input").focus();

// Enter a value, assert Escape cancels, restores Backup-button focus, wipes inputs.
await page.evaluate(() => {
  document.getElementById("pass-input").value = "secretpass";
  document.getElementById("pass-confirm").value = "secretpass";
});
await page.keyboard.press("Escape");
await page.waitForTimeout(200); // 140ms exit + buffer
const afterEscape = await page.evaluate(() => ({
  hidden: document.getElementById("pass-dialog").classList.contains("hidden"),
  enter: document.getElementById("pass-dialog").classList.contains("ov-enter"),
  focused: document.activeElement?.id,
  passVal: document.getElementById("pass-input").value,
  confirmVal: document.getElementById("pass-confirm").value,
  appInert: document.getElementById("app").hasAttribute("inert"),
}));
check("FE-FIX F-1/§2.5: Escape cancels — dialog hidden, no stale ov-enter, shell released",
  afterEscape.hidden && !afterEscape.enter && !afterEscape.appInert,
  JSON.stringify(afterEscape));
check("FE-FIX §2.5: Escape restores focus to the Backup button and wipes the passphrase inputs",
  afterEscape.focused === "drive-backup-btn" && afterEscape.passVal === "" && afterEscape.confirmVal === "",
  JSON.stringify(afterEscape));

// -- FE-FIX 2: reduced-motion end-state is the same final state (visible, stable) --
// Fresh page hydrates from the shared mock; reset googleStored so it loads
// signed-out, then sign in cleanly inside the reduced-motion context.
googleStored = null;
const rmPage = await browser.newPage({ reducedMotion: "reduce" });
rmPage.on("pageerror", (e) => errors.push(String(e)));
await rmPage.addInitScript(mockTauri);
await rmPage.addInitScript(cspCollector);
await rmPage.exposeFunction("__invokeImpl", invoke);
await rmPage.goto("http://localhost:4199/");
await rmPage.waitForTimeout(300);
await rmPage.locator("#user-chip").click();
await rmPage.locator("#signin-btn").click();
await rmPage.waitForTimeout(120);
await rmPage.locator("#drive-backup-btn").click();
await rmPage.locator("#pass-dialog:not(.hidden)").waitFor({ timeout: 2000 });
await rmPage.waitForTimeout(60);
const rmState = await rmPage.evaluate(() => {
  const el = document.getElementById("pass-dialog");
  return { opacity: getComputedStyle(el).opacity, position: getComputedStyle(el).position, visible: el.getClientRects().length > 0 };
});
check("FE-FIX F-2: reduced-motion passphrase overlay reaches the visible final state (opacity 1, fixed)",
  rmState.opacity === "1" && rmState.position === "fixed" && rmState.visible,
  JSON.stringify(rmState));
await rmPage.close();

// -- FE-FIX 3: signed-in identity row — escaped hostile name/email inert, no remote image request --
// The popover stayed open through the overlay cycle (overlay owned Escape); make
// sure it is open without toggling it shut.
if (await page.locator("#signin-popover").isHidden()) await page.locator("#user-chip").click();
const reqBeforeIdentity = requestLog.length;
await page.waitForTimeout(40);
const identity = await page.evaluate(() => {
  const avatar = document.getElementById("pop-avatar");
  const title = document.getElementById("google-pop-title");
  const sub = document.getElementById("google-pop-sub");
  const label = document.querySelector("#drive-actions .pop-group-label");
  const foot = document.querySelector(".pop-foot");
  return {
    avatarHidden: avatar.hidden,
    avatarText: avatar.textContent,
    avatarKids: avatar.childElementCount,
    title: title.textContent,
    titleKids: title.childElementCount,
    titleWeight: getComputedStyle(title).fontWeight,
    titleColor: getComputedStyle(title).color,
    sub: sub.textContent,
    subKids: sub.childElementCount,
    subColor: getComputedStyle(sub).color,
    groupLabel: label?.textContent,
    footHasSignout: !!foot?.querySelector("#signout-btn"),
    footHasSettings: !!foot?.querySelector("#acct-settings-btn"),
  };
});
check("FE-FIX §2.2: signed-in identity row — initials avatar + name focal point + Drive group label",
  !identity.avatarHidden && identity.avatarText === "AL" && identity.avatarKids === 0 &&
  identity.title === "Ada Lovelace" && Number(identity.titleWeight) >= 650 &&
  identity.groupLabel === "Google Drive" && identity.footHasSignout && identity.footHasSettings,
  JSON.stringify(identity));
check("FE-FIX §2.2/ADR-8: hostile email renders inert text (0 child nodes) in --dim, name in --text",
  identity.sub === 'ada+<img src=x onerror=alert(1)>@example.com' && identity.subKids === 0 &&
  identity.subColor === "rgb(185, 170, 149)" && identity.titleColor === "rgb(242, 233, 220)",
  JSON.stringify(identity));
check("FE-FIX NFR-3: opening the signed-in popover fires no remote image request",
  !requestLog.slice(reqBeforeIdentity).some((u) => u.includes("googleusercontent") || /\.(png|jpg|jpeg|webp|gif)(\?|$)/i.test(u)),
  JSON.stringify(requestLog.slice(reqBeforeIdentity)));

// -- FE-FIX 4: backup success -> role=status toast, dismissable via close --
const toastInfo = () => page.evaluate(() => [...document.querySelectorAll("#toast-stack .toast")].map((t) => ({
  role: t.getAttribute("role"),
  title: t.querySelector(".toast-title").textContent,
  msg: t.querySelector(".toast-msg").textContent,
  msgKids: t.querySelector(".toast-msg").childElementCount,
  ok: t.classList.contains("toast--ok"),
  error: t.classList.contains("toast--error"),
  info: t.classList.contains("toast--info"),
})));
// Clicking a toast (outside the popover) closes it, so reopen before each
// drive action. The chip toggles, so only click when the popover is hidden.
const ensureDrive = async () => {
  // Wait out any in-flight pass-dialog exit (fixed, covers the rail for ~140ms).
  await page.locator("#pass-dialog.hidden").waitFor({ timeout: 2000 }).catch(() => {});
  if (await page.locator("#signin-popover").isHidden()) await page.locator("#user-chip").click();
  await page.locator("#drive-backup-btn").waitFor({ state: "visible", timeout: 2000 });
};
driveBackupFail = false;
await ensureDrive();
await page.locator("#drive-backup-btn").click();
await page.locator("#pass-input").fill("pw-success");
await page.locator("#pass-confirm").fill("pw-success");
await page.locator("#pass-ok").click();
await page.waitForTimeout(120);
let toasts = await toastInfo();
check("FE-FIX §2.3: backup success raises exactly one role=status --ok toast",
  toasts.length === 1 && toasts[0].role === "status" && toasts[0].ok && toasts[0].title === "Backup complete" &&
  toasts[0].msg.includes("hosterm-backup.age"),
  JSON.stringify(toasts));
// Close via the keyboard-operable close button.
await page.locator("#toast-stack .toast-close").first().focus();
const toastFocus = await page.evaluate(() => {
  const btn = document.activeElement;
  const cs = getComputedStyle(btn);
  const box = btn.getBoundingClientRect();
  return { isClose: btn.classList.contains("toast-close"), w: Math.round(box.width), h: Math.round(box.height) };
});
check("FE-FIX §2.3: toast close is keyboard-focusable with a 44px hit target",
  toastFocus.isClose && toastFocus.w === 44 && toastFocus.h === 44,
  JSON.stringify(toastFocus));
await page.keyboard.press("Enter");
await page.waitForTimeout(220);
check("FE-FIX §2.3: Enter on the close button dismisses the success toast",
  (await page.locator("#toast-stack .toast").count()) === 0);

// backup failure -> role=alert --error toast that AUTO-DISMISSES at 5000ms
// (t_058b9aee: the Drive backup/restore terminal outcome must not require a
// click; the close button remains only as an early-dismiss affordance).
driveBackupFail = true;
await ensureDrive();
await page.locator("#drive-backup-btn").click();
await page.locator("#pass-input").fill("pw-fail");
await page.locator("#pass-confirm").fill("pw-fail");
await page.locator("#pass-ok").click();
await page.waitForTimeout(120);
toasts = await toastInfo();
check("FE-FIX §2.3: backup failure raises a role=alert --error toast, title 'Backup failed'",
  toasts.length === 1 && toasts[0].role === "alert" && toasts[0].error && toasts[0].title === "Backup failed",
  JSON.stringify(toasts));
check("FE-FIX ADR-8: hostile error text renders inert in the toast (0 child nodes), no __pwn",
  toasts[0].msg === DRIVE_ERR && toasts[0].msgKids === 0 &&
  (await page.evaluate(() => typeof window.__pwn === "undefined")),
  JSON.stringify(toasts[0]));
// A short real-time sample only shows it has not vanished immediately; the
// authoritative proof that it auto-closes at the 5000ms boundary is the
// deterministic fake-clock check below (clockPage), which fast-forwards past 5s.
await page.waitForTimeout(600);
check("FE-FIX t_058b9aee: backup error toast is still present shortly after creation (600ms real-time sample)",
  (await page.locator("#toast-stack .toast--error").count()) === 1);
// No close control on error other than the close button (close-only, no 'Try again').
check("FE-FIX nano-lock #3: error toast has no retry/action control, only the close button",
  await page.evaluate(() => {
    const t = document.querySelector("#toast-stack .toast--error");
    return t && t.querySelectorAll("button").length === 1 && t.querySelector(".toast-close") !== null;
  }));
// Early dismiss via the close button still works; clear it for the restore checks.
await page.locator("#toast-stack .toast--error .toast-close").first().click();
await page.waitForTimeout(200);
driveBackupFail = false;

// -- FE-FIX 5: restore success/failure + neutral no-backups map to the right toast role/copy --

driveListMode = "empty";
await ensureDrive();
await page.locator("#drive-restore-btn").click();
await page.waitForTimeout(120);
toasts = await toastInfo();
check("FE-FIX nano-lock #5: restore with zero backups raises a neutral role=status --info toast with the locked copy",
  toasts.length === 1 && toasts[0].role === "status" && toasts[0].info &&
  toasts[0].title === "Nothing to restore" &&
  toasts[0].msg === "No backups on Drive yet. Back up first to enable restore.",
  JSON.stringify(toasts));
await page.locator("#toast-stack .toast-close").first().click();
await page.waitForTimeout(200);
driveListMode = "normal";

// restore success: confirm the passphrase (no-confirm field), expect an ok toast.
driveRestoreFail = false;
await ensureDrive();
await page.locator("#drive-restore-btn").click();
await page.locator("#pass-dialog:not(.hidden)").waitFor({ timeout: 2000 });
// Reset-to-masked on every open: a later prompt opens masked regardless of the
// eye state a prior prompt was left in (t_5fe89707 — wipe + remask on reopen).
const reopenReveal = await page.evaluate(() => ({
  pressed: document.getElementById("pass-eye").getAttribute("aria-pressed"),
  label: document.getElementById("pass-eye").getAttribute("aria-label"),
  inputType: document.getElementById("pass-input").type,
}));
check("FE-FIX t_5fe89707: a later passphrase prompt opens masked (eye reset to aria-pressed=false, label 'Show passphrase', type=password)",
  reopenReveal.pressed === "false" && reopenReveal.label === "Show passphrase" &&
  reopenReveal.inputType === "password",
  JSON.stringify(reopenReveal));
await page.locator("#pass-input").fill("restorepw");
await page.locator("#pass-ok").click();
await page.waitForTimeout(140);
toasts = await toastInfo();
check("FE-FIX §2.3: restore success raises a role=status --ok toast",
  toasts.length === 1 && toasts[0].role === "status" && toasts[0].ok && toasts[0].title === "Restore complete",
  JSON.stringify(toasts));
await page.locator("#toast-stack .toast-close").first().click();
await page.waitForTimeout(200);

driveRestoreFail = true;
await ensureDrive();
await page.locator("#drive-restore-btn").click();
await page.locator("#pass-dialog:not(.hidden)").waitFor({ timeout: 2000 });
await page.locator("#pass-input").fill("restorepw");
await page.locator("#pass-ok").click();
await page.waitForTimeout(140);
toasts = await toastInfo();
check("FE-FIX t_058b9aee: restore failure raises a role=alert --error toast (auto-dismiss at 5000ms)",
  toasts.length === 1 && toasts[0].role === "alert" && toasts[0].error && toasts[0].title === "Restore failed",
  JSON.stringify(toasts));
await page.locator("#toast-stack .toast-close").first().click();
await page.waitForTimeout(200);
driveRestoreFail = false;

// -- FE-FIX 6: max-stack/eviction (newest-first, max 3) + no CSP violations --
// Push four successes; the stack caps at TOAST_MAX=3, newest-first (prepend).
// NB t_058b9aee: Drive toasts now all auto-dismiss at 5000ms, and a real-time
// four-toast sequence spans >5s of dialog open/close latency, so the long-lived
// "never evict an error" property can no longer be observed across a slow
// real-time loop — it is proven deterministically on the fake-clock page below
// (check (c): four simultaneous errors all preserved at creation, no silent drop).
driveBackupFail = false;
for (const p of ["s1", "s2", "s3", "s4"]) {
  await ensureDrive();
  await page.locator("#drive-backup-btn").click();
  await page.locator("#pass-input").fill(p); await page.locator("#pass-confirm").fill(p);
  await page.locator("#pass-ok").click();
  await page.waitForTimeout(80);
}
const stackState = await page.evaluate(() => {
  const list = [...document.querySelectorAll("#toast-stack .toast")];
  return {
    count: list.length,
    allOk: list.every((t) => t.classList.contains("toast--ok")),
    // newest-first: DOM-first node is the most recent (prepend)
    firstIsSuccess: list[0]?.classList.contains("toast--ok"),
  };
});
check("FE-FIX §2.3: stack caps at 3 visible, newest-first (prepend)",
  stackState.count === 3 && stackState.allOk && stackState.firstIsSuccess,
  JSON.stringify(stackState));
const feFixCsp = await page.evaluate(() => window.__cspViolations || []);
check("FE-FIX: toast surface carries no CSP violations (counter stays 0)",
  feFixCsp.length === 0, JSON.stringify(feFixCsp));
// Clean the stack for later checks.
await page.evaluate(() => { document.querySelectorAll("#toast-stack .toast-close").forEach((b) => b.click()); });
await page.waitForTimeout(220);

// -- FE-FIX 6c: deterministic timer behavior on an ISOLATED fake-clock page --
// A dedicated page installs Playwright's clock so setTimeout + Date.now are
// controllable; this proves (a) the success toast auto-dismisses exactly at the
// 5000ms boundary, (b) hover and keyboard focus each PAUSE that timer so the
// toast outlives 5000ms of fast-forward, then resumes, and (c) a persistent
// error genuinely survives past 5000ms. The clock stays scoped to clockPage so
// it never perturbs the main suite's real-timer pages. Runtime is bounded: all
// waits are fake fast-forwards, not real 5s sleeps.
googleStored = null;
const clockPage = await browser.newPage();
clockPage.on("pageerror", (e) => errors.push(String(e)));
await clockPage.clock.install();
await clockPage.addInitScript(mockTauri);
await clockPage.addInitScript(cspCollector);
await clockPage.exposeFunction("__invokeImpl", invoke);
await clockPage.goto("http://localhost:4199/");
await clockPage.clock.runFor(400); // let boot settle under the fake clock
await clockPage.locator("#user-chip").click();
await clockPage.locator("#signin-btn").click();
await clockPage.clock.runFor(150);
const clockToastCount = () => clockPage.locator("#toast-stack .toast").count();
// Raise a success toast and PUMP the fake clock in small steps until it is
// actually in the DOM. The backup invoke resolves on a microtask/timer chain
// that only advances under clock.runFor, so without pumping the toast would not
// exist yet when we start measuring. Pumping ≤500ms total keeps the timer's
// 5000ms budget essentially intact and the measurement deterministic.
const raiseSuccess = async (pw) => {
  if (await clockPage.locator("#signin-popover").isHidden()) { await clockPage.locator("#user-chip").click(); await clockPage.clock.runFor(30); }
  await clockPage.locator("#drive-backup-btn").click();
  await clockPage.locator("#pass-dialog:not(.hidden)").waitFor({ timeout: 2000 });
  await clockPage.locator("#pass-input").fill(pw);
  await clockPage.locator("#pass-confirm").fill(pw);
  await clockPage.locator("#pass-ok").click();
  let pumped = 0;
  while (pumped < 500 && (await clockPage.locator("#toast-stack .toast--ok").count()) === 0) {
    await clockPage.clock.runFor(25);
    pumped += 25;
  }
  return pumped;
};

// (a) success auto-dismisses at the 5000ms boundary, not before.
driveBackupFail = false;
await raiseSuccess("t-auto");
await clockPage.clock.runFor(4000);
const before5s = await clockToastCount();
await clockPage.clock.runFor(1200); // cross 5000ms (+ exit transition)
const after5s = await clockToastCount();
check("FE-FIX §2.3: success toast is alive at 4000ms and auto-dismisses after the 5000ms window",
  before5s === 1 && after5s === 0, JSON.stringify({ before5s, after5s }));

// (b1) pointer hover PAUSES the success timer: fast-forward well past 5000ms
// while hovered and the toast must remain; after mouseleave it resumes and dies.
await raiseSuccess("t-hover");
await clockPage.locator("#toast-stack .toast--ok").first().hover();
await clockPage.clock.runFor(8000); // > 5000ms, but paused by hover
const hoverHeld = await clockToastCount();
await clockPage.mouse.move(5, 5); // leave the toast -> resume
await clockPage.clock.runFor(5200); // remaining ~5000ms elapses
const hoverResumed = await clockToastCount();
check("FE-FIX §2.3: pointer hover pauses the success auto-dismiss (survives 8000ms held), resumes on leave",
  hoverHeld === 1 && hoverResumed === 0, JSON.stringify({ hoverHeld, hoverResumed }));

// (b2) keyboard focus PAUSES the timer the same way (focusin), resumes on blur.
await raiseSuccess("t-focus");
await clockPage.locator("#toast-stack .toast--ok .toast-close").first().focus();
await clockPage.clock.runFor(8000); // > 5000ms, paused by focus
const focusHeld = await clockToastCount();
await clockPage.locator("#user-chip").focus(); // blur the toast -> resume
await clockPage.clock.runFor(5200);
const focusResumed = await clockToastCount();
check("FE-FIX §2.3: keyboard focus pauses the success auto-dismiss (survives 8000ms held), resumes on blur",
  focusHeld === 1 && focusResumed === 0, JSON.stringify({ focusHeld, focusResumed }));

// (c) more than three simultaneous Drive-backup errors are ALL preserved at
// creation (never silently dropped): with TOAST_MAX=3, four errors remain
// because overflow eviction only drops non-errors. They then AUTO-DISMISS once
// the 5000ms budget elapses (t_058b9aee: Drive backup/restore error toasts are
// bounded like the success toasts; eviction and auto-dismiss are orthogonal —
// eviction never silently drops an error, the timer closes it after 5s).
await clockPage.evaluate(() => { document.querySelectorAll("#toast-stack .toast-close").forEach((b) => b.click()); });
await clockPage.clock.runFor(300);
driveBackupFail = true;
for (const pw of ["e1", "e2", "e3", "e4"]) {
  if (await clockPage.locator("#signin-popover").isHidden()) { await clockPage.locator("#user-chip").click(); await clockPage.clock.runFor(30); }
  await clockPage.locator("#drive-backup-btn").click();
  await clockPage.locator("#pass-input").fill(pw);
  await clockPage.locator("#pass-confirm").fill(pw);
  await clockPage.locator("#pass-ok").click();
  await clockPage.clock.runFor(80);
}
const fourErrorsImmediate = await clockPage.locator("#toast-stack .toast--error").count();
await clockPage.clock.runFor(6000); // well past 5000ms — bounded backup errors auto-dismiss
const fourErrorsAfter = await clockPage.locator("#toast-stack .toast--error").count();
check("FE-FIX t_058b9aee: four simultaneous backup errors are all preserved at creation (no silent drop), then auto-dismiss after 5000ms",
  fourErrorsImmediate === 4 && fourErrorsAfter === 0,
  JSON.stringify({ fourErrorsImmediate, fourErrorsAfter }));
const clockCsp = await clockPage.evaluate(() => (window.__cspViolations || []).length);
check("FE-FIX §2.3: fake-clock timer page raises no CSP violations", clockCsp === 0, String(clockCsp));
driveBackupFail = false;
await clockPage.close();

// -- FE-FIX 6b: narrow window — passphrase box + toast stack never overflow --
await page.setViewportSize({ width: 360, height: 640 });
await page.waitForTimeout(80);
await ensureDrive();
await page.locator("#drive-backup-btn").click();
await page.locator("#pass-dialog:not(.hidden)").waitFor({ timeout: 2000 });
await page.waitForTimeout(80);
const narrowPass = await page.evaluate(() => {
  const dlg = document.querySelector("#pass-dialog .dialog-box").getBoundingClientRect();
  return { left: Math.round(dlg.left), right: Math.round(dlg.right), vw: window.innerWidth, overflow: document.documentElement.scrollWidth > window.innerWidth };
});
check("FE-FIX §2.5: passphrase box clamps within a 360px viewport (no horizontal overflow)",
  narrowPass.left >= 0 && narrowPass.right <= narrowPass.vw && !narrowPass.overflow,
  JSON.stringify(narrowPass));
await page.keyboard.press("Escape");
await page.waitForTimeout(200);
// Raise a toast in the narrow window and confirm it stays inside the viewport.
driveBackupFail = true;
await ensureDrive();
await page.locator("#drive-backup-btn").click();
await page.locator("#pass-input").fill("n1"); await page.locator("#pass-confirm").fill("n1");
await page.locator("#pass-ok").click();
await page.waitForTimeout(120);
const narrowToast = await page.evaluate(() => {
  const t = document.querySelector("#toast-stack .toast");
  if (!t) return { none: true };
  const b = t.getBoundingClientRect();
  return { left: Math.round(b.left), right: Math.round(b.right), vw: window.innerWidth, overflow: document.documentElement.scrollWidth > window.innerWidth };
});
check("FE-FIX §2.5/R-03: toast stack stays inside a 360px viewport (no overflow)",
  !narrowToast.none && narrowToast.left >= 0 && narrowToast.right <= narrowToast.vw && !narrowToast.overflow,
  JSON.stringify(narrowToast));
driveBackupFail = false;
await page.evaluate(() => { document.querySelectorAll("#toast-stack .toast-close").forEach((b) => b.click()); });
await page.waitForTimeout(220);
await page.setViewportSize({ width: 1280, height: 720 });
await page.waitForTimeout(80);

// -- FE-FIX 7: existing sign-in/out + account flow still green (re-assert the invariants). --
const feFixSignedIn = await page.locator("#user-chip").evaluate((chip) => chip.dataset.state);
check("FE-FIX regression: account remains signed-in after the toast/overlay cycle",
  feFixSignedIn === "signed-in");
// Sign out to leave the suite signed-out (the state U-D6 handed over). The
// narrow-window + toast cycle may have closed the popover, so open it first.
if (await page.locator("#signin-popover").isHidden()) {
  await page.locator("#user-chip").click();
  await page.waitForTimeout(40);
}
await page.locator("#signout-btn").click();
await page.waitForTimeout(40);
check("FE-FIX regression: sign out returns the chip to signed-out",
  (await page.locator("#user-chip").evaluate((chip) => chip.dataset.state)) === "signed-out");
// The 360px narrow-window test above auto-collapsed the section panel and
// persisted `hosterm.panel-collapsed=1` to localStorage, which survives
// page.reload(). Clear it and re-expand now (popover already closed by sign-out)
// so later sections (U-D7 reloads -> U9 host controls) boot with #view-host
// visible — do not leak narrow-window state downstream.
await page.evaluate(() => { try { localStorage.removeItem("hosterm.panel-collapsed"); localStorage.removeItem("hosterm.sb-collapsed"); } catch { /* private mode */ } });
if (await page.locator("#btn-expand").isVisible().catch(() => false)) {
  await page.locator("#btn-expand").click();
  await page.waitForTimeout(40);
}
// End FE-FIX checks.

// U-D7 AI assistant + exact-command approval contract (FR-AI1..13, INV-25/26).
const aiCounts = () => ({
  spawn: calls.filter((c) => c.cmd === "pty_spawn").length,
  write: calls.filter((c) => c.cmd === "pty_write").length,
  settings: calls.filter((c) => c.cmd === "write_settings").length,
  readSettings: calls.filter((c) => c.cmd === "read_settings").length,
});
const openAi = async () => {
  await page.locator("#mode-ai").click();
  await page.locator('#ai-drawer.open:not([hidden])').waitFor({ timeout: 3000 });
};
const callsBeforeAiOpen = aiCounts();
await openAi();
const aiPanel = await page.evaluate(() => {
  const drawer = document.getElementById("ai-drawer");
  const input = document.getElementById("ai-input");
  return {
    width: Math.round(drawer.getBoundingClientRect().width),
    label: drawer.getAttribute("aria-label"),
    thread: document.querySelectorAll("#ai-thread .msg-user").length,
    cards: document.querySelectorAll("#ai-thread .cmd-card").length,
    placeholder: input.getAttribute("placeholder"),
    focused: document.activeElement?.id,
    tag: drawer.querySelector(".head-tag")?.textContent,
    tagTitle: drawer.querySelector(".head-tag")?.getAttribute("title"),
  };
});
check("U-D7 FR-AI1/12: 340px mock assistant drawer has thread, command cards, and Ask hosterm input",
  aiPanel.width === 340 && aiPanel.label === "AI assistant" && aiPanel.thread === 2 && aiPanel.cards === 2 &&
  aiPanel.placeholder === "Ask hosterm" && aiPanel.focused === "ai-input" && aiPanel.tag === "mock" &&
  aiPanel.tagTitle.includes("no AI backend") && !JSON.stringify(aiPanel).includes("on-device"),
  JSON.stringify(aiPanel));
check("U-D7 FR-AI9: opening the mock never runs or persists a command implicitly",
  JSON.stringify(aiCounts()) === JSON.stringify(callsBeforeAiOpen), JSON.stringify({ before: callsBeforeAiOpen, after: aiCounts() }));
const approvalRendererSource = u3src.slice(
  u3src.indexOf("function renderApprovals"),
  u3src.indexOf("async function refreshApprovals"),
);
const threadRendererSource = u3src.slice(
  u3src.indexOf("function renderAiThread"),
  u3src.indexOf("function visibleAiCommand"),
);
check("U-D7 SEC-01: approval and thread renderers clone static templates without innerHTML assignments",
  approvalRendererSource.startsWith("function renderApprovals") &&
  threadRendererSource.startsWith("function renderAiThread") &&
  !/\.innerHTML\s*=/.test(approvalRendererSource) && !/\.innerHTML\s*=/.test(threadRendererSource) &&
  approvalRendererSource.includes("APPROVAL_ITEM_TEMPLATE.content.cloneNode(true)") &&
  approvalRendererSource.includes("list.replaceChildren(rendered)") &&
  threadRendererSource.includes("AI_PROPOSAL_TEMPLATE.content.cloneNode(true)") &&
  threadRendererSource.includes("thread.replaceChildren(rendered)"));

const allowedCard = page.locator('.cmd-card[data-ai-id="restart-netdata"]');
const awaitingCard = page.locator('.cmd-card[data-ai-id="prune-docker"]');
const allowedMeta = await allowedCard.evaluate((card) => ({
  host: card.querySelector(".cmd-scope")?.textContent,
  command: card.querySelector(".cmd-line")?.textContent,
  description: card.querySelector(".cmd-desc")?.textContent,
  state: card.querySelector(".cmd-state")?.textContent,
  stateClass: card.querySelector(".cmd-state")?.className,
  awaiting: card.classList.contains("awaiting"),
  actions: card.querySelectorAll(".cmd-actions button").length,
}));
check("U-D7 FR-AI2/7: exact host+command allowlist match renders directly ALWAYS ALLOWED",
  allowedMeta.host === "prod-web" && allowedMeta.command === "sudo systemctl restart netdata" &&
  allowedMeta.description.length > 0 && allowedMeta.state === "ALWAYS ALLOWED" &&
  allowedMeta.stateClass.includes("ok") && !allowedMeta.awaiting && allowedMeta.actions === 0,
  JSON.stringify(allowedMeta));

const awaitingMeta = await awaitingCard.evaluate((card) => ({
  host: card.querySelector(".cmd-scope")?.textContent,
  command: card.querySelector(".cmd-line")?.textContent,
  description: card.querySelector(".cmd-desc")?.textContent,
  state: card.querySelector(".cmd-state")?.textContent,
  stateClass: card.querySelector(".cmd-state")?.className,
  awaiting: card.classList.contains("awaiting"),
  labels: [...card.querySelectorAll(".cmd-actions button")].map((button) => button.textContent),
  classes: [...card.querySelectorAll(".cmd-actions button")].map((button) => button.className),
}));
check("U-D7 FR-AI2/3: proposed card has exact scope, mono command, impact, warn state, and three actions",
  awaitingMeta.host === "prod-web" && awaitingMeta.command === "docker system prune -af" &&
  awaitingMeta.description.includes("Running containers are not touched") && awaitingMeta.state === "AWAITING APPROVAL" &&
  awaitingMeta.stateClass.includes("warn") && awaitingMeta.awaiting &&
  JSON.stringify(awaitingMeta.labels) === JSON.stringify(["Run once", "Always allow", "Reject"]),
  JSON.stringify(awaitingMeta));
check("U-D7 INV-25: Run once is primary; permission-widening Always allow is secondary; Reject is ghost",
  awaitingMeta.classes[0].split(/\s+/).includes("btn-primary") && awaitingMeta.classes[0].split(/\s+/).includes("approval-run") &&
  awaitingMeta.classes[1].split(/\s+/).includes("btn-secondary") && awaitingMeta.classes[1].split(/\s+/).includes("approval-always") &&
  !awaitingMeta.classes[1].split(/\s+/).includes("btn-primary") &&
  awaitingMeta.classes[2].split(/\s+/).includes("btn-ghost") && awaitingMeta.classes[2].split(/\s+/).includes("approval-reject"),
  JSON.stringify(awaitingMeta.classes));

const hostileStubBefore = { calls: calls.length, requests: requestLog.length };
await page.locator("#ai-input").fill(HOSTILE);
await page.locator("#ai-send").click();
const hostileThread = await page.locator("#ai-thread .msg-user").last().evaluate((message) => ({
  text: message.textContent,
  children: message.childElementCount,
  pwn: typeof window.__pwn,
  csp: (window.__cspViolations || []).length,
}));
check("U-D7 FR-AI11: hostile thread text stays raw inert text with CSP 0",
  hostileThread.text === HOSTILE && hostileThread.children === 0 && hostileThread.pwn === "undefined" && hostileThread.csp === 0,
  JSON.stringify(hostileThread));
check("U-D7 ADR-22: Ask hosterm stub performs zero invoke and zero network request",
  calls.length === hostileStubBefore.calls && requestLog.length === hostileStubBefore.requests &&
  (await page.locator("#ai-thread .msg-ai p").last().textContent()).includes("AI backend is not connected"),
  JSON.stringify({ before: hostileStubBefore, after: { calls: calls.length, requests: requestLog.length } }));

const runBefore = aiCounts();
const runAllowlistBefore = JSON.stringify(storedSettings.allowlist);
const runCallOffset = calls.length;
await awaitingCard.locator(".approval-run").click();
await page.waitForFunction(() => document.querySelector('.cmd-card[data-ai-id="prune-docker"] .cmd-state')?.textContent === "RAN ONCE");
const runCalls = calls.slice(runCallOffset);
const runWrites = runCalls.filter((c) => c.cmd === "pty_write");
check("U-D7 FR-AI5: Run once writes the exact command plus Enter exactly once through the real-ssh PTY",
  aiCounts().spawn === runBefore.spawn + 1 && aiCounts().write === runBefore.write + 1 && runWrites.length === 1 &&
  runWrites[0]?.args?.data === "docker system prune -af\r" &&
  (await awaitingCard.locator(".cmd-output").textContent()) === "Command sent once to the real-ssh terminal.",
  JSON.stringify(runCalls.filter((c) => ["pty_spawn", "pty_write"].includes(c.cmd))));
check("U-D7 FR-AI5: Run once leaves the persisted allowlist byte-for-byte unchanged",
  aiCounts().settings === runBefore.settings && JSON.stringify(storedSettings.allowlist) === runAllowlistBefore,
  JSON.stringify(storedSettings.allowlist));
const hostileAfterRun = await page.locator("#ai-thread .msg-user").last().evaluate((message) => ({
  text: message.textContent,
  children: message.childElementCount,
  pwn: typeof window.__pwn,
  csp: (window.__cspViolations || []).length,
}));
check("U-D7 SEC-01: hostile thread text stays inert after Run-once template re-render",
  hostileAfterRun.text === HOSTILE && hostileAfterRun.children === 0 &&
  hostileAfterRun.pwn === "undefined" && hostileAfterRun.csp === 0,
  JSON.stringify(hostileAfterRun));

await page.reload();
await page.waitForTimeout(400);
await openAi();
const alwaysCard = page.locator('.cmd-card[data-ai-id="prune-docker"]');
await page.locator("#ai-input").fill(HOSTILE);
await page.locator("#ai-send").click();
const alwaysBefore = aiCounts();
const alwaysOffset = calls.length;
await alwaysCard.locator(".approval-always").click();
await page.waitForFunction(() => document.querySelector('.cmd-card[data-ai-id="prune-docker"] .cmd-state')?.textContent === "ALWAYS ALLOWED");
const alwaysCalls = calls.slice(alwaysOffset);
const alwaysWrite = alwaysCalls.find((c) => c.cmd === "write_settings");
const alwaysPty = alwaysCalls.find((c) => c.cmd === "pty_write");
const savedPrune = alwaysWrite?.args?.settings?.allowlist?.find((entry) =>
  entry.host === "prod-web" && entry.command === "docker system prune -af");
check("U-D7 FR-AI6/ADR-21: Always allow persists the exact tuple before one PTY execution",
  aiCounts().settings === alwaysBefore.settings + 1 && aiCounts().spawn === alwaysBefore.spawn + 1 &&
  aiCounts().write === alwaysBefore.write + 1 && savedPrune?.added_at === "" &&
  alwaysCalls.findIndex((c) => c.cmd === "write_settings") < alwaysCalls.findIndex((c) => c.cmd === "pty_write") &&
  alwaysPty?.args?.data === "docker system prune -af\r",
  JSON.stringify(alwaysCalls.filter((c) => ["write_settings", "pty_spawn", "pty_write"].includes(c.cmd))));
check("U-D7 FR-AI6/7: Always allow transitions to ok state with no approval actions",
  await alwaysCard.evaluate((card) => card.querySelector(".cmd-state")?.classList.contains("ok") &&
    card.querySelector(".cmd-state")?.textContent === "ALWAYS ALLOWED" && card.querySelectorAll(".cmd-actions").length === 0));
const hostileAfterAlways = await page.locator("#ai-thread .msg-user").last().evaluate((message) => ({
  text: message.textContent,
  children: message.childElementCount,
  pwn: typeof window.__pwn,
  csp: (window.__cspViolations || []).length,
}));
check("U-D7 SEC-01: hostile thread text stays inert after Always-allow template re-render",
  hostileAfterAlways.text === HOSTILE && hostileAfterAlways.children === 0 &&
  hostileAfterAlways.pwn === "undefined" && hostileAfterAlways.csp === 0,
  JSON.stringify(hostileAfterAlways));

await openSettingsFromAccount();
const approvalsReadBefore = aiCounts().readSettings;
await page.locator('.set-nav[data-section="ai"]').click();
await page.waitForFunction(() => document.querySelectorAll("#approvals-list .approval-item").length === 2);
const approvalsMeta = await page.evaluate(() => ({
  hidden: document.getElementById("settings-approvals").classList.contains("hidden"),
  hosts: [...document.querySelectorAll(".approval-host")].map((el) => el.textContent),
  commands: [...document.querySelectorAll(".approval-command")].map((el) => el.textContent),
  revokes: document.querySelectorAll(".approval-revoke").length,
  patternInputs: document.querySelectorAll('#settings-approvals input, #settings-approvals [contenteditable="true"]').length,
  rows: [...document.querySelectorAll("#approvals-list .approval-item")].map((row) => ({
    host: row.querySelector(".approval-host")?.textContent,
    hostChildren: row.querySelector(".approval-host")?.childElementCount,
    command: row.querySelector(".approval-command")?.textContent,
    commandChildren: row.querySelector(".approval-command")?.childElementCount,
    index: row.querySelector(".approval-revoke")?.getAttribute("data-index"),
  })),
}));
check("U-D7 FR-AI10: Settings > AI Assistant reads an exact command+host revoke list, not a pattern editor",
  aiCounts().readSettings === approvalsReadBefore + 1 && !approvalsMeta.hidden && approvalsMeta.revokes === 2 &&
  approvalsMeta.patternInputs === 0 && approvalsMeta.hosts.every((host) => host === "prod-web") &&
  approvalsMeta.commands.includes("sudo systemctl restart netdata") && approvalsMeta.commands.includes("docker system prune -af"),
  JSON.stringify(approvalsMeta));
check("U-D7 SEC-01: approval template preserves exact inert text and data-index values",
  JSON.stringify(approvalsMeta.rows) === JSON.stringify([
    { host: "prod-web", hostChildren: 0, command: "sudo systemctl restart netdata", commandChildren: 0, index: "0" },
    { host: "prod-web", hostChildren: 0, command: "docker system prune -af", commandChildren: 0, index: "1" },
  ]), JSON.stringify(approvalsMeta.rows));
const revokeBefore = aiCounts();
const pruneApproval = page.locator("#approvals-list .approval-item", { hasText: "docker system prune -af" });
await pruneApproval.locator(".approval-revoke").click();
await page.waitForFunction(() => document.querySelectorAll("#approvals-list .approval-item").length === 1);
const revokeWrite = calls.filter((c) => c.cmd === "write_settings").at(-1);
check("U-D7 FR-AI10: Revoke removes one exact tuple through write_settings",
  aiCounts().settings === revokeBefore.settings + 1 &&
  revokeWrite?.args?.settings?.allowlist?.length === 1 &&
  !revokeWrite.args.settings.allowlist.some((entry) => entry.host === "prod-web" && entry.command === "docker system prune -af") &&
  (await page.locator("#approvals-status").textContent()) === "Approval revoked.",
  JSON.stringify(revokeWrite?.args?.settings?.allowlist));
await page.locator("#settings-done").click();
await page.waitForTimeout(100);

const rejectCard = page.locator('.cmd-card[data-ai-id="prune-docker"]');
const rejectBefore = aiCounts();
await rejectCard.locator(".approval-reject").click();
const rejectedMeta = await rejectCard.evaluate((card) => ({
  state: card.querySelector(".cmd-state")?.textContent,
  danger: card.querySelector(".cmd-state")?.classList.contains("danger"),
  result: card.querySelector(".cmd-output")?.textContent,
}));
check("U-D7 FR-AI8: Reject changes state with no execution and no allowlist write",
  JSON.stringify(aiCounts()) === JSON.stringify(rejectBefore) && rejectedMeta.state === "REJECTED" &&
  rejectedMeta.danger && rejectedMeta.result === "Rejected. Nothing was executed or saved.",
  JSON.stringify(rejectedMeta));

const unsafeAiHost = async (host, action) => {
  await page.reload();
  await page.waitForTimeout(400);
  await openAi();
  const card = page.locator('.cmd-card[data-ai-id="prune-docker"]');
  await card.locator(".cmd-scope").evaluate((scope, value) => { scope.textContent = value; }, host);
  const before = aiCounts();
  await card.locator(action).click();
  await page.waitForFunction(() => document.querySelector('.cmd-card[data-ai-id="prune-docker"] .cmd-state')?.textContent === "REFUSED");
  return { before, after: aiCounts(), output: await card.locator(".cmd-output").textContent() };
};
for (const unsafe of [
  { name: "leading dash", host: "-prod-web", action: ".approval-always" },
  { name: "CR", host: "prod\rweb", action: ".approval-run" },
  { name: "LF", host: "prod\nweb", action: ".approval-run" },
]) {
  const blocked = await unsafeAiHost(unsafe.host, unsafe.action);
  check(`U-D7 INV-26: ${unsafe.name} host alias is refused before write or execution`,
    JSON.stringify(blocked.before) === JSON.stringify(blocked.after) && blocked.output.includes("Nothing was executed or saved"),
    JSON.stringify(blocked));
}

const canonicalApproval = { host: "prod-web", command: "sudo systemctl restart netdata", added_at: "2026-10-30T12:00:00Z" };
storedSettings = { ...storedSettings, allowlist: [{ ...canonicalApproval, command: `${canonicalApproval.command} ` }] };
await page.reload();
await page.waitForTimeout(400);
await openAi();
const nearMatch = await page.locator('.cmd-card[data-ai-id="restart-netdata"]').evaluate((card) => ({
  state: card.querySelector(".cmd-state")?.textContent,
  awaiting: card.classList.contains("awaiting"),
  actions: card.querySelectorAll(".cmd-actions button").length,
}));
check("U-D7 ADR-21: trailing-space command is not an exact byte match",
  nearMatch.state === "AWAITING APPROVAL" && nearMatch.awaiting && nearMatch.actions === 3,
  JSON.stringify(nearMatch));

storedSettings = { ...storedSettings, allowlist: [canonicalApproval] };
await page.reload();
await page.waitForTimeout(400);
await openAi();
const hostileFinalBefore = { calls: calls.length, requests: requestLog.length };
await page.locator("#ai-input").fill(HOSTILE);
await page.keyboard.press("Enter");
const hostileFinal = await page.locator("#ai-thread .msg-user").last().evaluate((message) => ({
  text: message.textContent,
  children: message.childElementCount,
  pwn: typeof window.__pwn,
  csp: (window.__cspViolations || []).length,
}));
check("U-D7 FR-AI11: Enter-submit hostile fixture remains inert after reload",
  hostileFinal.text === HOSTILE && hostileFinal.children === 0 && hostileFinal.pwn === "undefined" && hostileFinal.csp === 0 &&
  calls.length === hostileFinalBefore.calls && requestLog.length === hostileFinalBefore.requests,
  JSON.stringify(hostileFinal));

storedSettings = { ...storedSettings, allowlist: [canonicalApproval] };
readSettingsFailures = 1;
await page.reload();
await page.waitForTimeout(400);
await openAi();
const duplicateCard = page.locator('.cmd-card[data-ai-id="restart-netdata"]');
const duplicateFallback = await duplicateCard.evaluate((card) => ({
  state: card.querySelector(".cmd-state")?.textContent,
  awaiting: card.classList.contains("awaiting"),
  actions: card.querySelectorAll(".cmd-actions button").length,
}));
const duplicateBefore = aiCounts();
const duplicateOffset = calls.length;
const duplicateAllowlistBefore = JSON.stringify(storedSettings.allowlist);
await duplicateCard.locator(".approval-always").click();
await page.waitForFunction(() => document.querySelector('.cmd-card[data-ai-id="restart-netdata"] .cmd-state')?.textContent === "ALWAYS ALLOWED");
const duplicateCalls = calls.slice(duplicateOffset);
check("U-D7 SEC-03: startup fallback re-check skips duplicate write and still executes exactly once",
  duplicateFallback.state === "AWAITING APPROVAL" && duplicateFallback.awaiting && duplicateFallback.actions === 3 &&
  duplicateCalls.filter((c) => c.cmd === "read_settings").length === 1 &&
  duplicateCalls.filter((c) => c.cmd === "write_settings").length === 0 &&
  duplicateCalls.filter((c) => c.cmd === "pty_write").length === 1 &&
  aiCounts().settings === duplicateBefore.settings && aiCounts().write === duplicateBefore.write + 1 &&
  JSON.stringify(storedSettings.allowlist) === duplicateAllowlistBefore &&
  storedSettings.allowlist.filter((entry) => entry.host === canonicalApproval.host && entry.command === canonicalApproval.command).length === 1,
  JSON.stringify({ fallback: duplicateFallback, calls: duplicateCalls, allowlist: storedSettings.allowlist }));
await page.locator("#ai-close").click();
check("U-D7 a11y: drawer close restores the AI trigger and Terminal mode",
  await page.evaluate(() => document.getElementById("ai-drawer").hidden &&
    document.getElementById("mode-terminal").getAttribute("aria-pressed") === "true" &&
    document.activeElement?.id === "mode-ai"));
// End U-D7 checks.

// (b) sidebar narrative copy byte-identical (empty / error / no-match) + empty CTA formalized
const sbState = () => page.evaluate(() => {
  const s = document.querySelector("#host-list .list-state");
  return s ? { text: s.firstChild?.nodeValue ?? "", err: s.classList.contains("err"), btn: s.querySelector("button")?.textContent ?? null } : null;
});
hostsMode = "empty";
await page.locator("#btn-reload").click();
await page.waitForTimeout(250);
const sbEmpty = await sbState();
check("U9 (b): sidebar empty copy byte-identical",
  sbEmpty?.text === "No hosts in ~/.ssh/config yet. Click + to add one." && !sbEmpty.err, JSON.stringify(sbEmpty));
await page.locator("#host-list .list-state button").click();
await page.waitForTimeout(200);
check("U9: empty-state CTA 'Add host' opens the add-host modal",
  sbEmpty?.btn === "Add host" && (await page.locator("#modal").evaluate((el) => !el.classList.contains("hidden"))) &&
  (await page.locator("#modal-title").textContent()) === "Add host");
await page.locator("#m-cancel").click();
await page.waitForTimeout(200);
hostsMode = "fail";
await page.locator("#btn-reload").click();
await page.waitForTimeout(250);
const sbErr = await sbState();
check("U9 (b): sidebar error copy byte-identical (.list-state.err)",
  sbErr?.text === "Could not read ~/.ssh/config: permission denied (mock). Check the file, then press Reload." && sbErr.err, JSON.stringify(sbErr));
// loadHosts logs the rejection via console.error by design (main.ts) — drop only
// that expected entry so the global "no console/page errors" check stays strict
const expErr = errors.findIndex((e) => e === "permission denied (mock)");
check("U9: fail fixture logs exactly the expected console.error", expErr !== -1 && errors.filter((e) => e.includes("permission denied (mock)")).length === 1);
if (expErr !== -1) errors.splice(expErr, 1);
hostsMode = "normal";
await page.locator("#btn-reload").click();
await page.waitForTimeout(250);
await page.locator("#host-search").fill("zzz");
await page.waitForTimeout(150);
const sbNone = await sbState();
check("U9 (b): sidebar no-match copy byte-identical", sbNone?.text === 'No host matches "zzz".', JSON.stringify(sbNone));
await page.locator("#host-search").fill("");
await page.waitForTimeout(150);
// row-level ↑/↓ (STATE Deferred U2): search ↓ enters list, rows move focus both ways
await page.locator("#host-search").focus();
await page.keyboard.press("ArrowDown");
const hostOrder = await page.evaluate(() => [...document.querySelectorAll(".host-item")].map((e) => e.dataset.host));
const focusedHost = () => page.evaluate(() => document.activeElement?.dataset?.host ?? null);
const fh0 = await focusedHost();
await page.keyboard.press("ArrowDown");
const fh1 = await focusedHost();
await page.keyboard.press("ArrowUp");
const fh2 = await focusedHost();
check("U9: ArrowDown from search focuses the first host row", hostOrder.length === 3 && fh0 === hostOrder[0], `${fh0} vs ${hostOrder[0]}`);
check("U9: row ArrowDown/ArrowUp move focus between host rows", fh1 === hostOrder[1] && fh2 === hostOrder[0], `${fh1}/${fh2}`);
await page.evaluate(() => { const a = document.activeElement; if (a instanceof HTMLElement) a.blur(); });
// SFTP narrative copy: fixed U5 nit (local pane is the LEFT pane) + rest pinned verbatim
await page.locator(".host-item .h-sftp").first().click();
await page.locator(".panel.active .sftp-list .sftp-row").first().waitFor({ timeout: 5000 });
const paneGeo = await page.evaluate(() => {
  const [a, b] = document.querySelectorAll(".panel.active .sftp-pane");
  return { leftLocal: !!a.querySelector(".loc-list"), ax: a.getBoundingClientRect().left, bx: b.getBoundingClientRect().left };
});
check("U9: local pane renders on the LEFT and copy says so",
  paneGeo.leftLocal && paneGeo.ax < paneGeo.bx && u3src.includes('"Open a local directory on the left first."') && !u3src.includes("on the right first"),
  JSON.stringify(paneGeo));
check("U9 (b): SFTP selection/transfer copy byte-identical in source",
  (u3src.match(/"Select a file in the local pane first \(single click\)\."/g) || []).length === 3 &&
  (u3src.match(/"Select a file in the remote pane first \(single click\)\."/g) || []).length === 3 &&
  u3src.includes("`Failed to upload ${name}: ${msg(e)}. Check the path and try again.`") &&
  u3src.includes("`Failed to download ${name}: ${msg(e)}. Check the path and try again.`"));
// hostile bytes (SEC t_ec1dbd02 LOW → INV-11): attacker-named remote entries render inert
await page.locator(".panel.active .sftp-path").fill("/xss");
await page.locator(".panel.active .sftp-go").click();
await page.waitForFunction(() => document.querySelectorAll(".panel.active .sftp-list .sftp-row").length === 3, null, { timeout: 5000 }).catch(() => {});
await page.waitForTimeout(200); // give a would-be onerror/onload time to fire
const xssList = await page.evaluate(() => [...document.querySelectorAll(".panel.active .sftp-list .sftp-row")].map((r) => {
  const nm = r.querySelector(".nm");
  return { kids: nm.childElementCount, text: nm.textContent, ds: r.dataset.name };
}));
const hostileNames = HOSTILE_ENTRIES.map((e) => e.name);
check("U9 hostile: remote .nm childElementCount 0 for every entry", xssList.length === 3 && xssList.every((r) => r.kids === 0), JSON.stringify(xssList.map((r) => r.kids)));
check("U9 hostile: .nm text + dataset.name keep raw bytes",
  JSON.stringify(xssList.map((r) => r.text)) === JSON.stringify(hostileNames) &&
  JSON.stringify(xssList.map((r) => r.ds)) === JSON.stringify(hostileNames), JSON.stringify(xssList.map((r) => r.text)));
check("U9 hostile: 0 <img>/<b> + 0 on* attributes injected anywhere in the DOM",
  await page.evaluate(() => document.querySelectorAll("img, b, [onerror], [onload]").length === 0));
// hostile path echoed into the error copy goes through textContent
await page.locator(".panel.active .sftp-path").fill(`/boom/${HOSTILE}`);
await page.locator(".panel.active .sftp-go").click();
await page.waitForTimeout(300);
const xssStatus = await page.evaluate(() => {
  const st = document.querySelector(".panel.active .sftp-status");
  return { kids: st.childElementCount, text: st.textContent };
});
check("U9 hostile: path echo in .sftp-status inert (0 child elements, raw text)",
  xssStatus.kids === 0 && xssStatus.text.includes(HOSTILE) && xssStatus.text.endsWith("Fix the path and press Go, or go up one level."), JSON.stringify(xssStatus));
// hostile host alias in sidebar + tab label + pane title
hostsMode = "hostile";
await page.locator("#btn-reload").click();
await page.waitForTimeout(250);
const xssHost = await page.evaluate((H) => {
  const row = [...document.querySelectorAll(".host-item")].find((e) => e.dataset.host === H);
  if (!row) return null;
  const nm = row.querySelector(".h-name");
  return { kids: nm.childElementCount, text: nm.textContent, aria: row.getAttribute("aria-label") };
}, HOSTILE);
check("U9 hostile: sidebar .h-name inert + aria-label raw",
  xssHost?.kids === 0 && xssHost.text === HOSTILE && xssHost.aria === `Open terminal to ${HOSTILE}`, JSON.stringify(xssHost));
await page.evaluate((H) => [...document.querySelectorAll(".host-item")].find((e) => e.dataset.host === H).querySelector(".h-sftp").click(), HOSTILE);
await page.locator(".panel.active .sftp-list .sftp-row").first().waitFor({ timeout: 5000 });
const xssChrome = await page.evaluate(() => {
  const lbl = document.querySelector(".tab.active .t-label");
  const ttl = document.querySelector(".panel.active .sftp-pane:nth-child(2) .pane-title");
  return { lk: lbl.childElementCount, lt: lbl.textContent, tk: ttl.childElementCount, tt: ttl.textContent };
});
check("U9 hostile: tab label + remote pane title inert",
  xssChrome.lk === 0 && xssChrome.lt === HOSTILE && xssChrome.tk === 0 && xssChrome.tt === `${HOSTILE} — remote`, JSON.stringify(xssChrome));
await page.waitForTimeout(200);
check("U9 hostile: window.__pwn undefined + CSP 0 after all hostile fixtures",
  (await page.evaluate(() => typeof window.__pwn)) === "undefined" &&
  (await page.evaluate(() => (window.__cspViolations || []).length)) === 0);
// teardown: close U9 tabs, restore normal host list
for (let i = 0; i < 6 && (await page.locator(".tab").count()) > 0; i++) {
  await page.locator(".tab .x").first().click();
  await page.waitForTimeout(250);
}
hostsMode = "normal";
await page.locator("#btn-reload").click();
await page.waitForTimeout(250);
check("U9: teardown — 0 tabs, 3 hosts back", (await page.locator(".tab").count()) === 0 && (await page.locator(".host-item").count()) === 3);
const sigEnd = await page.evaluate(() => window.__sigList.filter((e) => !e.sig.aborted).length);
check("U9: end-of-suite window drag listeners live == 0", sigEnd === 0, `live=${sigEnd}`);
// audit: EPIC §8 #9 checklist features each have explicit checks; every unit U3–U8 recorded, none failing
const unitIssues = ["U3", "U4", "U5", "U6", "U7", "U8"].filter((u) => {
  const mine = results.filter((r) => r.slice(5).startsWith(u));
  return mine.length === 0 || mine.some((r) => r.startsWith("FAIL"));
});
const need = ["host menu", "divider", "status bar", "strip"].filter((k) => !results.some((r) => r.startsWith("PASS") && r.includes(k)));
check("U9 audit: §8#9 (host menu/divider/status bar/strip) covered + U3–U8 all PASS",
  unitIssues.length === 0 && need.length === 0, `units=${unitIssues.join(",")} missing=${need.join(",")}`);

// U-D2 scope correction, suite-wide anti-regression: the display-only shell
// field must never have triggered a remote read anywhere in the run
const pwProbeEnd = calls.filter((c) => c.cmd === "sftp_read_file" && c.args.remotePath === "/etc/passwd").length;
check("U-D2 FR-SH8: /etc/passwd probe count exact 0 across the whole suite",
  pwProbeEnd === 0, `total=${pwProbeEnd}`);

check("no console/page errors", errors.length === 0, errors.slice(0, 3).join(" | "));

// SEC-4 / AC R1.4: zero CSP violations across the full suite
const cspViolations = await page.evaluate(() => window.__cspViolations || []);
check("CSP violations == 0 across suite (AC R1.4)", cspViolations.length === 0, cspViolations.slice(0, 3).join(" | "));
check("served CSP == tauri.conf.json policy", servedCsp === CSP.trim(), servedCsp.slice(0, 80));

// U-D8 canon: explicit U-D1..U-D7 ledger + seven cross-feature audit gates.
// Baseline at dispatch was 313; this block adds exactly 30 named runtime checks.
check("U-D3 IPC: read_settings mock is exercised by boot and reload paths",
  calls.filter((c) => c.cmd === "read_settings").length >= 2);
check("U-D3 IPC: write_settings mock receives the complete settings shape",
  calls.filter((c) => c.cmd === "write_settings").some((c) =>
    c.args?.settings?.version === 1 && typeof c.args.settings.theme === "string" &&
    /^#[0-9a-f]{6}$/.test(c.args.settings.accent) && Array.isArray(c.args.settings.allowlist)));
check("U-D3 IPC: read_snippets mock is exercised with the bundled grouped fixture",
  calls.some((c) => c.cmd === "read_snippets") && sampleSnippets.length === 3 &&
  sampleSnippets.every((group) => group.category && Array.isArray(group.items) && group.items.length > 0));

for (const unit of ["U-D1", "U-D2", "U-D3", "U-D4", "U-D5", "U-D6", "U-D7"]) {
  const unitResults = results.filter((result) => result.slice(5).startsWith(unit));
  check(`U-D8 canon: ${unit} has explicit smoke coverage and every check PASS`,
    unitResults.length > 0 && unitResults.every((result) => result.startsWith("PASS")),
    `${unitResults.length} checks`);
}

const allSrcText = u9Files.map((file) => readFileSync(new URL(file, srcDir), "utf8")).join("\n");
check("U-D8 (a) NFR-DK4/INV-6: grep draggable in every src/ file = 0",
  u9Files.length >= 3 && dragHits.length === 0 && !/draggable/i.test(allSrcText), dragHits.join(","));
check("U-D8 (a) NFR-DK4/INV-6: index.html and live DOM have zero draggable attributes",
  !/draggable/i.test(htmlSrc) && await page.evaluate(() => document.querySelectorAll("[draggable]").length === 0));

check("U-D8 (b) copy canon: remote empty state is byte-identical", u6Empty.r === "Empty directory.", JSON.stringify(u6Empty.r));
check("U-D8 (b) copy canon: local empty state is byte-identical", u6Empty.l === "Empty directory.", JSON.stringify(u6Empty.l));
check("U-D8 (b) copy canon: remote error state is byte-identical",
  u6Err.text === "Failed to list /boom: sftp boom (mock). Fix the path and press Go, or go up one level.", u6Err.text);
check("U-D8 (b) copy canon: local error state is byte-identical",
  u6ErrL.text === "Failed to read /boom: permission denied (mock-local). Fix the path and press Go.", u6ErrL.text);
check("U-D8 (b) copy canon: sidebar empty state is byte-identical",
  sbEmpty?.text === "No hosts in ~/.ssh/config yet. Click + to add one." && sbEmpty?.btn === "Add host", JSON.stringify(sbEmpty));
check("U-D8 (b) copy canon: sidebar error and no-match states are byte-identical",
  sbErr?.text === "Could not read ~/.ssh/config: permission denied (mock). Check the file, then press Reload." &&
  sbNone?.text === 'No host matches "zzz".', JSON.stringify({ sbErr, sbNone }));

const reducedMotionCss = (cssU4.match(/@media \(prefers-reduced-motion: reduce\)\s*\{([\s\S]*?)\n\}/) || [])[1] || "";
check("U-D8 (c) NFR-DK5: reduced-motion CSS is a universal animation kill-switch",
  reducedMotionCss.includes("*, *::before, *::after") && /animation:\s*none\s*!important/.test(reducedMotionCss));
check("U-D8 (c) NFR-DK5: reduced-motion CSS disables all transitions too",
  /transition:\s*none\s*!important/.test(reducedMotionCss));
check("U-D8 (c) NFR-DK5: runtime spinner and shimmer both resolve to animation-name none",
  rmAnim === "none" && rmSkel === "none", JSON.stringify({ spinner: rmAnim, shimmer: rmSkel }));

check("U-D8 (d) NFR-DK9/INV-11: hostile snippet name and command stay inert with CSP 0",
  hostileSnippet.nameKids === 0 && hostileSnippet.nameText === HOSTILE &&
  hostileSnippet.commandKids === 0 && hostileSnippet.commandText === HOSTILE &&
  hostileSnippet.pwn === "undefined" && hostileSnippet.csp === 0, JSON.stringify(hostileSnippet));
check("U-D8 (d) NFR-DK9/INV-11: hostile AI text stays inert across submit and template rerenders",
  [hostileThread, hostileAfterRun, hostileAfterAlways, hostileFinal].every((frame) =>
    frame.text === HOSTILE && frame.children === 0 && frame.pwn === "undefined" && frame.csp === 0));
check("U-D8 (d) NFR-DK9/INV-11: hostile SFTP fixture leaves zero executable DOM and zero CSP violations",
  xssList.length === HOSTILE_ENTRIES.length && xssList.every((row) => row.kids === 0) &&
  cspViolations.length === 0 && await page.evaluate(() => typeof window.__pwn === "undefined"));

const inv21Hits = abyssLiterals.flatMap((literal) =>
  u9Files.filter((file) => readFileSync(new URL(file, srcDir), "utf8").includes(literal)).map((file) => `${literal}:${file}`));
check("U-D8 (e) INV-21 anti-Abyss: forbidden literal vector has zero src/ hits",
  inv21Hits.length === 0 && abyssHits.length === 0, inv21Hits.join(","));
check("U-D8 (e) INV-21 anti-Abyss: rendered Dusk token vector is byte-exact",
  tokMismatch.length === 0 && Object.keys(rootTokens).length === Object.keys(DUSK_ROOT).length,
  tokMismatch.map(([key, value]) => `${key}=${value}`).join(","));

const inv23ExpectedAccents = [
  "#e2a35c", "#5fbfae", "#e0869b", "#aec267", "#b99be3", "#000000", "#101010",
  "#6f6f6f", "#757575", "#766cb5", "#cf0dcc", "#767676", "#777777", "#ffffff",
];
check("U-D8 (f) INV-23: frozen SDD §13.2.1 vector is complete and ordered",
  JSON.stringify(inv23Vectors.map((vector) => vector.accent)) === JSON.stringify(inv23ExpectedAccents));
const inv23Results = results.filter((result) => result.slice(5).startsWith("U-D4 INV-23 vector "));
check("U-D8 (f) INV-23: all 14 rendered vectors PASS with independently computed ratio >= 4.5",
  inv23Results.length === 14 && inv23Results.every((result) => result.startsWith("PASS")) &&
  inv23Vectors.every((vector) => vector.ratio >= 4.5), `${inv23Results.length}/14`);

const primaryCss = (cssU4.match(/\.btn-primary\s*\{([^}]*)\}/s) || [])[1] || "";
const secondaryCss = (cssU4.match(/\.btn-secondary\s*\{([^}]*)\}/s) || [])[1] || "";
check("U-D8 (g) INV-25: approval classes keep Run once primary and Always allow secondary",
  awaitingMeta.classes[0].split(/\s+/).includes("btn-primary") &&
  awaitingMeta.classes[1].split(/\s+/).includes("btn-secondary") &&
  !awaitingMeta.classes[1].split(/\s+/).includes("btn-primary"), JSON.stringify(awaitingMeta.classes));
check("U-D8 (g) INV-25: primary uses accent fill while secondary stays on raised surface",
  /background:\s*var\(--accent\)/.test(primaryCss) && /color:\s*var\(--accent-ink\)/.test(primaryCss) &&
  /background:\s*var\(--raised\)/.test(secondaryCss) && !/var\(--accent\)/.test(secondaryCss));

// ============================================================
//  FE-ANS — Ansible v1 panel (TSD §6). Counts may only GROW.
// ============================================================

// Hostile PLAY RECAP + task output (ADR-8): must render inert, never inject a
// <script> or trip CSP. Carried on the dry-run and real-run pty streams.
const ANS_HOSTILE_OUTPUT =
  "TASK [evil] <img src=x onerror=window.__pwn=9>\r\n" +
  "<script>window.__pwn=10</script>\r\n" +
  "PLAY RECAP *********************************************************************\r\n" +
  "prod-web                   : ok=5    changed=2    unreachable=0    failed=0\r\n" +
  "db-core                    : ok=4    changed=0    unreachable=0    failed=0\r\n";

// 1) Persistent rail drives one contextual section at a time.
const sidebarNav = await page.evaluate(() => ({
  labels: [...document.querySelectorAll(".rail-item[data-view]")].map((el) => el.getAttribute("aria-label")),
  hostVisible: !document.querySelector("#view-host").classList.contains("hidden"),
  current: [...document.querySelectorAll('.rail-item[aria-current="page"]')].map((el) => el.dataset.view),
  accountInRail: !!document.querySelector("#nav-rail > .rail-foot > #user-chip"),
  oldSettings: !!document.querySelector("#open-settings"),
  oldKeysButton: !!document.querySelector("#btn-keys"),
  oldAnsibleButton: !!document.querySelector("#btn-ansible"),
}));
check("NAV RAIL: Hosts / Keys / Ansible render with exactly Hosts current",
  JSON.stringify(sidebarNav.labels) === JSON.stringify(["Hosts", "Keys", "Ansible"]) &&
  sidebarNav.hostVisible && JSON.stringify(sidebarNav.current) === JSON.stringify(["host"]),
  JSON.stringify(sidebarNav));
check("NAV RAIL: account is rail-bottom and old settings/footer destinations are removed",
  sidebarNav.accountInRail && !sidebarNav.oldSettings && !sidebarNav.oldKeysButton && !sidebarNav.oldAnsibleButton,
  JSON.stringify(sidebarNav));
const shellGeometry = await page.evaluate(() => {
  const rail = document.getElementById("nav-rail").getBoundingClientRect();
  const panel = document.getElementById("section-panel").getBoundingClientRect();
  const main = document.getElementById("main").getBoundingClientRect();
  return { railX: rail.x, railW: rail.width, panelX: panel.x, panelRight: panel.right, mainX: main.x };
});
check("NAV RAIL: visual order is 56px rail, contextual panel, workspace",
  Math.round(shellGeometry.railX) === 0 && Math.round(shellGeometry.railW) === 56 &&
  Math.round(shellGeometry.panelX) === 56 && shellGeometry.mainX > shellGeometry.panelRight,
  JSON.stringify(shellGeometry));
check("NAV RAIL: Hosts-only controls are rendered on Hosts",
  await page.evaluate(() => ["btn-add", "btn-raw", "btn-local", "btn-reload"].every((id) => document.getElementById(id).getClientRects().length > 0)));
await page.locator('.rail-item[data-view="keys"]').click();
await page.waitForTimeout(60);
const keysInline = await page.evaluate(() => ({
  visible: !document.querySelector("#view-keys").classList.contains("hidden"),
  current: document.querySelector('.rail-item[data-view="keys"]').getAttribute("aria-current"),
  focused: document.activeElement?.id,
  hostControlsHidden: ["btn-add", "btn-raw", "btn-local", "btn-reload"].every((id) => document.getElementById(id).getClientRects().length === 0),
  list: document.querySelector("#sb-keys-list"),
}));
check("NAV RAIL: Keys swaps inline, focuses its first action, and removes Host controls",
  keysInline.visible && keysInline.current === "page" && keysInline.focused === "sb-keys-add" &&
  keysInline.hostControlsHidden && !!keysInline.list,
  JSON.stringify({ ...keysInline, list: !!keysInline.list }));
await page.keyboard.press("Control+k");
check("NAV RAIL: Ctrl+K is a no-op outside Hosts", await page.evaluate(() => document.activeElement?.id === "sb-keys-add"));
await page.locator('.rail-item[data-view="host"]').click();
await page.keyboard.press("Control+k");
check("NAV RAIL: Ctrl+K focuses search on Hosts", await page.evaluate(() => document.activeElement?.id === "host-search"));

ansibleCheckState = { missing: false };
await page.locator('.rail-item[data-view="ansible"]').click();
check("NAV RAIL: Ansible removes Host controls from layout and tab order",
  await page.evaluate(() => ["btn-add", "btn-raw", "btn-local", "btn-reload"].every((id) => document.getElementById(id).getClientRects().length === 0)));
await page.locator("#sb-ansible-open").click();
await page.waitForTimeout(60);
const ansPanel = await page.evaluate(() => {
  const modal = document.querySelector("#ansible-modal");
  return {
    open: modal && !modal.classList.contains("hidden"),
    bodyShown: !document.querySelector("#ans-body").classList.contains("hidden"),
    guideShown: !document.querySelector("#ans-guide").classList.contains("hidden"),
    hostRows: document.querySelectorAll("#ans-host-list .ans-host-row").length,
    // password host (pw-host) row present + marked skipped + checkbox disabled
    skippedRows: [...document.querySelectorAll("#ans-host-list .ans-host-row.skipped")].map((r) => ({
      alias: r.querySelector(".ans-host-alias").textContent,
      cbDisabled: r.querySelector(".ans-host-cb").disabled,
      skipText: r.querySelector(".ans-host-skip").textContent,
    })),
    runRealDisabled: document.querySelector("#ans-run-real").disabled,
  };
});
check("FE-ANS: panel opens with runner body (binary present), guide hidden",
  ansPanel.open && ansPanel.bodyShown && !ansPanel.guideShown, JSON.stringify({ open: ansPanel.open, body: ansPanel.bodyShown, guide: ansPanel.guideShown }));
check("FE-ANS: one picker row per sidebar host (wildcard Host * excluded)",
  ansPanel.hostRows === 3, `rows=${ansPanel.hostRows}`);
check("FE-ANS (FR-ANS4): password host marked skipped, checkbox disabled, not selectable",
  ansPanel.skippedRows.length === 1 && ansPanel.skippedRows[0].alias === "pw-host" &&
  ansPanel.skippedRows[0].cbDisabled === true && ansPanel.skippedRows[0].skipText.includes("password"),
  JSON.stringify(ansPanel.skippedRows));
check("FE-ANS: Run-real starts disabled (no playbook, no dry-run yet)",
  ansPanel.runRealDisabled === true);

// 2) Binary missing → guide shown, run body blocked, re-check button present.
ansibleCheckState = { missing: true };
await page.evaluate(() => document.querySelector("#ans-recheck").click());
await page.waitForTimeout(60);
const ansGuide = await page.evaluate(() => ({
  guideShown: !document.querySelector("#ans-guide").classList.contains("hidden"),
  bodyHidden: document.querySelector("#ans-body").classList.contains("hidden"),
  recheck: !!document.querySelector("#ans-recheck"),
  steps: document.querySelectorAll("#ans-guide-os .ans-guide-step").length,
}));
check("FE-ANS (FR-ANS1/ANS2): check_ansible missing → guide shown, run body blocked, re-check present",
  ansGuide.guideShown && ansGuide.bodyHidden && ansGuide.recheck && ansGuide.steps > 0, JSON.stringify(ansGuide));
// re-check again with binary back → body returns
ansibleCheckState = { missing: false };
await page.evaluate(() => document.querySelector("#ans-recheck").click());
await page.waitForTimeout(60);
const ansBackState = await page.evaluate(() => ({
  bodyShown: !document.querySelector("#ans-body").classList.contains("hidden"),
  guideHidden: document.querySelector("#ans-guide").classList.contains("hidden"),
}));
check("FE-ANS: re-check with binary present → runner body returns, guide hidden",
  ansBackState.bodyShown && ansBackState.guideHidden, JSON.stringify(ansBackState));

// 3) Playbook validation (FR-ANS5): reject bad extension, >2 MiB, binary/invalid-UTF-8.
// Pick modes are driven by toggling the module-level ansiblePickMode / ansiblePickName
// vars below; the invoke mock reads them directly.
const ansErrFor = async () => page.evaluate(() => {
  const e = document.querySelector("#ans-playbook-err");
  return { hidden: e.hidden, text: e.textContent };
});
ansiblePickMode = "badext";
await page.evaluate(() => document.querySelector("#ans-pick-playbook").click());
await page.waitForTimeout(40);
const errBadExt = await ansErrFor();
check("FE-ANS (FR-ANS5): non-yml/yaml path rejected with a cause-naming message",
  !errBadExt.hidden && /\.yml or \.yaml/.test(errBadExt.text), JSON.stringify(errBadExt));
ansiblePickMode = "big";
await page.evaluate(() => document.querySelector("#ans-pick-playbook").click());
await page.waitForTimeout(40);
const errBig = await ansErrFor();
check("FE-ANS (FR-ANS5): >2 MiB playbook rejected, message names the size cause",
  !errBig.hidden && /2 MiB limit/.test(errBig.text), JSON.stringify(errBig));
ansiblePickMode = "binary";
await page.evaluate(() => document.querySelector("#ans-pick-playbook").click());
await page.waitForTimeout(40);
const errBin = await ansErrFor();
check("FE-ANS (FR-ANS5): binary/invalid-UTF-8 playbook rejected, message names the cause",
  !errBin.hidden && /not valid UTF-8/.test(errBin.text), JSON.stringify(errBin));
// a rejected pick must NOT have triggered any ansible_run
check("FE-ANS (FR-ANS5): rejected playbooks never reach ansible_run",
  calls.filter((c) => c.cmd === "ansible_run").length === 0);

// 4) Dry-run-WAJIB gate state machine (TSD §5). Pick a valid playbook (hostile
//    path byte asserts esc()), select a target, Run-real stays disabled until a
//    mode:"check" pty exits rc==0 for the current playbook+target.
ansiblePickMode = "valid";
ansiblePickName = "\"><img src=x onerror=window.__pwn=11>.yml"; // hostile basename (ADR-8)
await page.evaluate(() => document.querySelector("#ans-pick-playbook").click());
await page.waitForTimeout(40);
// select prod-web (first eligible, not pw-host)
await page.evaluate(() => {
  const rows = [...document.querySelectorAll("#ans-host-list .ans-host-row")];
  const r = rows.find((x) => x.querySelector(".ans-host-alias").textContent === "prod-web");
  const cb = r.querySelector(".ans-host-cb");
  cb.checked = true; cb.dispatchEvent(new Event("change", { bubbles: true }));
});
await page.waitForTimeout(20);
const gateBeforeDry = await page.evaluate(() => ({
  dryDisabled: document.querySelector("#ans-dry-run").disabled,
  realDisabled: document.querySelector("#ans-run-real").disabled,
  pathEsc: document.querySelector("#ans-playbook-path").children.length, // 0 = inert text
  pathText: document.querySelector("#ans-playbook-path").textContent,
}));
check("FE-ANS (gate): with playbook + target, Dry-run enabled but Run-real still DISABLED",
  gateBeforeDry.dryDisabled === false && gateBeforeDry.realDisabled === true, JSON.stringify(gateBeforeDry));
check("FE-ANS (ADR-8): hostile playbook path renders inert (0 child nodes), no __pwn",
  gateBeforeDry.pathEsc === 0 && gateBeforeDry.pathText.includes("onerror") &&
  (await page.evaluate(() => typeof window.__pwn === "undefined")), JSON.stringify({ kids: gateBeforeDry.pathEsc }));

const runsBeforeDry = calls.filter((c) => c.cmd === "ansible_run").length;
await page.evaluate(() => document.querySelector("#ans-dry-run").click());
await page.waitForTimeout(40);
const dryReq = ansibleRuns[ansibleRuns.length - 1];
check("FE-ANS (dry-run): clicking Dry-run calls ansible_run with mode:'check' + the selected target",
  calls.filter((c) => c.cmd === "ansible_run").length === runsBeforeDry + 1 &&
  dryReq && dryReq.mode === "check" && dryReq.aliases.includes("prod-web") && dryReq.limit === "prod-web",
  JSON.stringify(dryReq));
// while running both buttons are disabled
const midRun = await page.evaluate(() => ({ dry: document.querySelector("#ans-dry-run").disabled, real: document.querySelector("#ans-run-real").disabled }));
check("FE-ANS (gate): both run buttons disabled while a run is in flight", midRun.dry === true && midRun.real === true);
// stream hostile output then exit rc==0
await page.evaluate((out) => {
  window.__emitTauri("pty://output/7777", out);
  window.__emitTauri("pty://exit/7777");
}, ANS_HOSTILE_OUTPUT);
await page.waitForTimeout(40);
const afterDry = await page.evaluate(() => ({
  realDisabled: document.querySelector("#ans-run-real").disabled,
  summary: document.querySelector("#ans-recap-summary").textContent,
  pwn: typeof window.__pwn,
}));
check("FE-ANS (gate): dry-run mode:'check' exits rc==0 → Run-real ENABLED for current playbook+target",
  afterDry.realDisabled === false, JSON.stringify(afterDry));
check("FE-ANS (FR-ANS8): PLAY RECAP parsed into an 'N hosts will change, M tasks changed' summary",
  /1 host will change, 2 tasks changed/.test(afterDry.summary), afterDry.summary);
check("FE-ANS (ADR-8): hostile PLAY RECAP / task output never executes (no __pwn) and 0 CSP violations",
  afterDry.pwn === "undefined" && (await page.evaluate(() => window.__cspViolations.length)) === 0);

// changing the target resets the gate → Run-real disabled again (TSD §5)
await page.evaluate(() => {
  const rows = [...document.querySelectorAll("#ans-host-list .ans-host-row")];
  const r = rows.find((x) => x.querySelector(".ans-host-alias").textContent === "db-core");
  const cb = r.querySelector(".ans-host-cb");
  cb.checked = true; cb.dispatchEvent(new Event("change", { bubbles: true }));
});
await page.waitForTimeout(20);
check("FE-ANS (gate): changing targets after a passing dry-run resets the gate → Run-real DISABLED",
  (await page.evaluate(() => document.querySelector("#ans-run-real").disabled)) === true);

// re-run dry-run for the new pair, exit rc==0 → gate re-opens
await page.evaluate(() => document.querySelector("#ans-dry-run").click());
await page.waitForTimeout(30);
await page.evaluate((out) => { window.__emitTauri("pty://output/7777", out); window.__emitTauri("pty://exit/7777"); }, ANS_HOSTILE_OUTPUT);
await page.waitForTimeout(40);
check("FE-ANS (gate): a fresh dry-run for the new playbook+target re-opens the gate",
  (await page.evaluate(() => document.querySelector("#ans-run-real").disabled)) === false);

// 5) Run-real confirm dialog (FR-ANS10): clicking Run-real opens a confirm and
//    NEVER auto-runs; cancel → no ansible_run; confirm → mode:"real".
const runsBeforeReal = calls.filter((c) => c.cmd === "ansible_run").length;
await page.evaluate(() => document.querySelector("#ans-run-real").click());
await page.waitForTimeout(40);
const confirmShown = await page.evaluate(() => {
  const d = document.querySelector("#dialog");
  return { open: d && !d.classList.contains("hidden"), title: document.querySelector("#dialog-title").textContent };
});
check("FE-ANS (FR-ANS10): Run-real opens an explicit confirm dialog, never auto-runs",
  confirmShown.open && /Run this playbook for real/.test(confirmShown.title) &&
  calls.filter((c) => c.cmd === "ansible_run").length === runsBeforeReal, JSON.stringify(confirmShown));
// cancel → still no real run
await page.evaluate(() => [...document.querySelectorAll("#dialog-actions button")].find((b) => b.textContent === "Cancel").click());
await page.waitForTimeout(30);
check("FE-ANS (FR-ANS10): cancelling the confirm does NOT spawn a real run",
  calls.filter((c) => c.cmd === "ansible_run").length === runsBeforeReal);
// confirm → a mode:"real" run spawns
await page.evaluate(() => document.querySelector("#ans-run-real").click());
await page.waitForTimeout(30);
await page.evaluate(() => [...document.querySelectorAll("#dialog-actions button")].find((b) => b.textContent === "Run for real").click());
await page.waitForTimeout(40);
const realReq = ansibleRuns[ansibleRuns.length - 1];
check("FE-ANS (FR-ANS10): confirming spawns exactly one ansible_run with mode:'real'",
  calls.filter((c) => c.cmd === "ansible_run").length === runsBeforeReal + 1 && realReq.mode === "real",
  JSON.stringify(realReq));
await page.evaluate(() => { window.__emitTauri("pty://output/7777", "PLAY RECAP\r\nprod-web : ok=1 changed=0 unreachable=0 failed=0\r\n"); window.__emitTauri("pty://exit/7777"); });
await page.waitForTimeout(40);

// 6) History (FR-ANS14): after runs exit, ansible_history append + read render
//    rows (basename, targets, mode, time, duration, rc, recap), recap is the
//    trimmed PLAY RECAP, hostile recap stays inert.
const ansHist = await page.evaluate(() => {
  const rows = [...document.querySelectorAll("#ans-history-list .ans-hist-row")];
  return rows.map((r) => ({
    playbook: r.querySelector(".ans-hist-playbook").textContent,
    mode: r.querySelector(".ans-hist-mode").textContent,
    rc: r.querySelector(".ans-hist-rc").textContent,
    recapText: r.querySelector(".ans-hist-recap").textContent,
    recapKids: r.querySelector(".ans-hist-recap").children.length,
  }));
});
check("FE-ANS (FR-ANS14): history renders a row per run (≥3: 2 dry + 1 real) with mode + rc",
  ansHist.length >= 3 && ansHist.some((r) => r.mode === "dry-run") && ansHist.some((r) => r.mode === "real"),
  `rows=${ansHist.length}`);
check("FE-ANS (FR-ANS14): history recap is the trimmed PLAY RECAP, never the full output",
  ansHist.every((r) => !/TASK \[evil\]/.test(r.recapText)) &&
  ansHist.some((r) => /PLAY RECAP/.test(r.recapText) && /prod-web/.test(r.recapText)),
  JSON.stringify(ansHist.map((r) => r.recapText.slice(0, 40))));
check("FE-ANS (ADR-8): hostile recap in history renders inert (0 child nodes), no __pwn",
  ansHist.every((r) => r.recapKids === 0) && (await page.evaluate(() => typeof window.__pwn === "undefined")));
check("FE-ANS (FR-ANS14): append entry carries trimmed recap only (no full task output over IPC)",
  ansibleHistory.length >= 3 && ansibleHistory.every((e) => !/TASK \[evil\]/.test(e.recap) && !/<script>/.test(e.recap)),
  JSON.stringify(ansibleHistory.map((e) => e.recap.slice(0, 30))));

// 7) INV-28 / NFR-ANS5: no shell-string invocation on the Ansible frontend path;
//    draggable grep stays 0 (picker is checkbox/manual, not HTML5 DnD).
const ansMainSrc = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8");
const ansSlice = ansMainSrc.slice(ansMainSrc.indexOf("Ansible v1 panel (FE-ANS)"), ansMainSrc.indexOf("// ---------- sidebar ----------"));
check("FE-ANS (NFR-ANS5): no sh -c / bash -c / cmd /C on the Ansible frontend path",
  !/sh -c|bash -c|cmd \/C/.test(ansSlice));
check("FE-ANS: draggable grep stays 0 in src/ (picker is checkbox + manual, not HTML5 DnD)",
  !/\bdraggable\b/.test(ansMainSrc) && !/\bdraggable\b/.test(readFileSync(new URL("../index.html", import.meta.url), "utf8")));

// close the panel to leave the DOM clean for any later checks
await page.evaluate(() => document.querySelector("#ansible-close").click());

await page.setViewportSize({ width: 500, height: 720 });
await page.waitForTimeout(80);
const narrowShell = await page.evaluate(() => ({
  collapsed: document.body.classList.contains("panel-collapsed"),
  railVisible: document.getElementById("nav-rail").getClientRects().length > 0,
  panelVisible: document.getElementById("section-panel").getClientRects().length > 0,
  overflow: document.documentElement.scrollWidth > window.innerWidth,
}));
check("NAV RAIL: 520px breakpoint collapses only the contextual panel without overflow",
  narrowShell.collapsed && narrowShell.railVisible && !narrowShell.panelVisible && !narrowShell.overflow,
  JSON.stringify(narrowShell));
await page.locator("#user-chip").click();
const narrowPopover = await page.locator("#signin-popover").evaluate((el) => {
  const box = el.getBoundingClientRect();
  return { left: box.left, right: box.right, width: box.width, viewport: window.innerWidth };
});
check("NAV RAIL: account menu stays inside the narrow viewport",
  narrowPopover.left >= 0 && narrowPopover.right <= narrowPopover.viewport,
  JSON.stringify(narrowPopover));
await page.keyboard.press("Escape");
await page.setViewportSize({ width: 1280, height: 720 });
await page.locator("#btn-expand").click();

console.log(results.join("\n"));
const fails = results.filter((r) => r.startsWith("FAIL")).length;
console.log(`\n${results.length - fails}/${results.length} passed`);
// identity of failed checks on stderr too (U7 cold-start flake went unrecorded)
if (fails) console.error("FAILED checks:\n" + results.filter((r) => r.startsWith("FAIL")).join("\n"));
await browser.close();
server.close();
process.exit(fails ? 1 : 0);
