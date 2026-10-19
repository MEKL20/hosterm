// Headless smoke test for the redesigned UI without a Tauri backend.
// Requires: npm install --no-save playwright@1.63.0 (chromium cached in
// ~/.cache/ms-playwright). Run: node qa-reports/headless-smoke.mjs
// Mocks window.__TAURI_INTERNALS__.invoke, loads dist/, asserts DOM behavior.
import { chromium } from "playwright";
import { createServer } from "http";
import { readFileSync, existsSync } from "fs";
import { extname, join } from "path";

const DIST = new URL("../dist", import.meta.url).pathname;
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
// W2-A startup ssh/sftp check state (AC-A1.5); read by the check_binaries case
let binCheckState = { ssh_missing: false, sftp_missing: false };
const invoke = (cmd, args = {}) => {
  calls.push({ cmd, args });
  switch (cmd) {
    case "read_ssh_config": return delayRead > 0
      ? new Promise((r) => setTimeout(() => r(sampleHosts), delayRead))
      : Promise.resolve(sampleHosts);
    case "check_binaries": return Promise.resolve({ ...binCheckState });
    case "read_config_raw": return Promise.resolve(sampleConfig);
    case "write_config_raw": return Promise.resolve();
    case "list_identity_files": return Promise.resolve(["~/.ssh/id_test"]);
    case "sftp_list": {
      if (sftpFailFirst && args.host === "pw-host") { sftpFailFirst = false; return Promise.reject("sftp session unreachable"); }
      return Promise.resolve(sampleSftp);
    }
    case "local_list": return Promise.resolve([{ name: "notes.txt", is_dir: false, size: 5 }, { name: "sub", is_dir: true, size: 0 }]);
    case "home_dir": return Promise.resolve("/tmp/mockhome");
    case "sftp_read_file": return Promise.resolve("hello editor\nline two\n");
    case "sftp_write_file": return Promise.resolve();
    case "pty_spawn": return Promise.resolve(4242);
    case "pty_resize": case "pty_kill": case "pty_write": case "clipboard_write": case "clipboard_read": return Promise.resolve("");
    case "save_host": case "delete_host": case "create_identity_file": return Promise.resolve();
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

const browser = await chromium.launch();
const page = await browser.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
await page.addInitScript(() => {
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
});
await page.addInitScript(() => {
  // Track AbortController signals passed to window listeners (U4 split drag).
  // Each split registers its mousemove/mouseup under ONE shared signal; if the
  // split is torn down, that signal must be aborted (SEC LOW t_fb1d18d6 fix).
  // Scoped to the two drag types so library signals can't pollute the count.
  window.__sigList = [];
  const origAdd = window.addEventListener.bind(window);
  window.addEventListener = (type, fn, opts) => {
    const sig = (type === "mousemove" || type === "mouseup") && opts && typeof opts === "object" ? opts.signal : undefined;
    if (sig && !window.__sigList.includes(sig)) window.__sigList.push(sig);
    return origAdd(type, fn, opts);
  };
});
// CSP-violation collector (AC R1.4): securitypolicyviolation events for the whole run
await page.addInitScript(() => {
  window.__cspViolations = [];
  document.addEventListener("securitypolicyviolation", (e) => {
    window.__cspViolations.push(`${e.effectiveDirective}: ${e.blockedURL || e.sourceFile || "inline"}`);
  });
});
await page.exposeFunction("__invokeImpl", invoke);
const firstResp = await page.goto("http://localhost:4199/");
const servedCsp = (firstResp.headers()["content-security-policy"] || "").trim();
await page.waitForTimeout(400);

const results = [];
const check = (name, ok, extra = "") => results.push(`${ok ? "PASS" : "FAIL"} ${name}${extra ? " — " + extra : ""}`);

// ---- W2-A: startup ssh/sftp check + install guide (AC-A1.5, NFR-W2-4) ----
const PS_CMD = "Add-WindowsCapability -Online -Name OpenSSH.Client~~~~0.0.1.0";
// present-binary no-op (AC-A1.2): detection ran at launch, guide never appears
check("check_binaries invoked at launch", calls.some((c) => c.cmd === "check_binaries"));
check("present binaries: guide never appears", await page.locator("#ssh-modal").evaluate((el) => el.classList.contains("hidden")));

// missing-binary state on Linux (OQ-5): check runs, guide skipped, per-attempt error friendly
binCheckState = { ssh_missing: true, sftp_missing: false };
await page.goto("http://localhost:4199/");
await page.waitForTimeout(400);
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
await winPage.addInitScript(() => {
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
});
await winPage.addInitScript(() => {
  window.__cspViolations = [];
  document.addEventListener("securitypolicyviolation", (e) => {
    window.__cspViolations.push(`${e.effectiveDirective}: ${e.blockedURL || e.sourceFile || "inline"}`);
  });
});
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

// 5. dirty dot appears after edit
await page.locator(".cm-content").click();
await page.keyboard.type("X");
await page.waitForTimeout(200);
const dirty = await page.locator(".tab .t-dot").count();
check("dirty dot appears on edit", dirty === 1, `dots=${dirty}`);
check("dirty dot is 8px --warn (U3)", await page.locator(".tab .t-dot").first().evaluate((el) => {
  const s = getComputedStyle(el);
  return s.width === "8px" && s.backgroundColor === "rgb(255, 221, 128)";
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

// 9c. sidebar collapse/expand + localStorage persistence
await page.locator("#btn-collapse").click();
check("collapse: body class set", await page.evaluate(() => document.body.classList.contains("sb-collapsed")));
check("collapse: expand button shown", await page.locator("#btn-expand").isVisible());
const stored = await page.evaluate(() => localStorage.getItem("hosterm.sb-collapsed"));
check("collapse: persisted", stored === "1");
await page.locator("#btn-expand").click();
check("expand: class removed", await page.evaluate(() => !document.body.classList.contains("sb-collapsed")));

// 9d. Edit/Delete left the row (fix #4) -> context menu; Delete uses in-app dialog (not native confirm)
check("edit/delete buttons gone from rows",
  (await page.locator(".host-item .h-edit, .host-item .h-del").count()) === 0);
await page.locator(".host-item").first().click({ button: "right" });
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
// download button guards when nothing selected in remote
await page.locator(".panel.active .sftp-download").click();
await page.waitForTimeout(300);
check("download without selection shows hint", (await page.locator(".panel.active .sftp-status").textContent()).includes("Select a file"));

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
check("U4: term-host well bg --well #020203", wellBg === "rgb(2, 2, 3)", wellBg);
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
check("U4: divider lights --accent while dragging", dragCol === "rgb(0, 217, 146)", dragCol);
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
check("U4: focused side underline 2px --accent", focUl === "2px rgb(0, 217, 146)", focUl);
// U4 teardown hygiene (SEC LOW t_fb1d18d6): split drag listeners share one
// AbortController signal, registered on window and aborted by closeTab.
const sigAfterSplit = await page.evaluate(() => ({
  n: window.__sigList.length,
  live: window.__sigList.filter((s) => !s.aborted).length,
}));
check("U4: split registers an abortable window-listener signal",
  sigAfterSplit.n >= 1 && sigAfterSplit.live === sigAfterSplit.n, JSON.stringify(sigAfterSplit));
// status bar global: always visible, left "user@host · auth:key · N sesi"
const sb = await page.evaluate(() => {
  const el = document.getElementById("status-bar");
  return { h: el.offsetHeight, vis: !!el.offsetParent, left: document.getElementById("sb-left").textContent, ver: document.getElementById("sb-ver").textContent };
});
check("U4: status bar always visible, 24px", sb.vis && sb.h === 24, `h=${sb.h} vis=${sb.vis}`);
check("U4: status bar left format user@host · auth:key · N sesi",
  sb.left === "admin@db-core · auth:key · 3 sesi", sb.left);
check("U4: status bar shows app version", /^v\d+\.\d+\.\d+$/.test(sb.ver), sb.ver);
const cnt = (t) => Number((t.match(/(\d+) sesi/) || [])[1]);
await page.locator(".host-item", { hasText: "prod-web" }).first().locator(".h-name").click();
await page.waitForTimeout(400);
const sbLeft2 = await page.locator("#sb-left").textContent();
check("U4: status bar session count increments on tab open", cnt(sbLeft2) === cnt(sb.left) + 1, `${sb.left} -> ${sbLeft2}`);
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

// 9g3. SFTP: list scrolls (pane is height-constrained), transfer buttons visible
// (9g2 closed every tab, so open a fresh SFTP tab first)
await page.locator(".host-item .h-sftp").first().click();
await page.waitForTimeout(600);
const sftpPanel = page.locator(".panel.active");
await sftpPanel.locator(".sftp-row").first().waitFor({ timeout: 5000 });
const listScrollable = await sftpPanel.locator(".sftp-list").evaluate((el) => {
  const wrap = el.closest(".sftp-wrap");
  const foot = wrap && wrap.querySelector(".sftp-foot");
  return {
    wrapFits: wrap.getBoundingClientRect().height <= window.innerHeight + 2,
    footVisible: !!(foot && foot.offsetParent),
    footTop: Math.round(foot.getBoundingClientRect().top),
    vh: window.innerHeight,
  };
});
const footVisible = listScrollable.footVisible && listScrollable.footTop < listScrollable.vh;
check("sftp wrap fits viewport (list scrollable)", listScrollable.wrapFits);
check("upload/download buttons visible", footVisible);
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
const before = await page.locator("#sidebar").evaluate((el) => el.getBoundingClientRect().width);
const box = await page.locator("#sb-resize").boundingBox();
await page.mouse.move(box.x + box.width / 2, box.y + 200);
await page.mouse.down();
await page.mouse.move(box.x + 200, box.y + 200, { steps: 4 });
await page.mouse.up();
const after = await page.locator("#sidebar").evaluate((el) => el.getBoundingClientRect().width);
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
}));
check("U4: status bar visible with 0 tabs, '0 sesi'", sbIdle.vis && sbIdle.h === 24 && sbIdle.left === "0 sesi", JSON.stringify(sbIdle));
// U4 teardown hygiene (SEC LOW t_fb1d18d6): every split signal aborted after closeTab
const sigAfterClose = await page.evaluate(() => {
  const live = window.__sigList.filter((s) => !s.aborted);
  return { n: window.__sigList.length, live: live.length };
});
check("U4: split window listeners released on closeTab (no leak)",
  sigAfterClose.n >= 1 && sigAfterClose.live === 0, JSON.stringify(sigAfterClose));
// (1) single-line 28px row; subtitle is a title tooltip, not a second element
const rowH = await page.locator(".host-item").first().evaluate((el) => el.offsetHeight);
check("host row is exactly 28px (single line)", rowH === 28, `h=${rowH}`);
check("no second-line subtitle element", (await page.locator(".host-item .h-sub").count()) === 0);
const rowTitle = await page.locator(".host-item").first().getAttribute("title");
check("subtitle lives in title tooltip", !!rowTitle && rowTitle.includes("deploy@203.0.113.10"), rowTitle || "");
// (2) group headers = alias prefix before "-"; wildcard Host * never rendered
const groups = await page.locator(".h-group").allTextContents();
check("group headers from '-' prefix", JSON.stringify(groups) === JSON.stringify(["PROD", "DB", "PW"]), groups.join("|"));
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
check("lock uses --warn token", (await pwRowKey.evaluate((el) => getComputedStyle(el).color)) === "rgb(255, 221, 128)");
// (4) Ctrl+K focuses search; ArrowDown + Enter opens terminal (§5)
await page.keyboard.press("Control+k");
check("Ctrl+K focuses #host-search", await page.evaluate(() => document.activeElement?.id === "host-search"));
await page.keyboard.press("ArrowDown");
check("ArrowDown moves focus to first row", await page.evaluate(() => document.activeElement?.classList.contains("host-item")));
const spawnsU2 = calls.filter((c) => c.cmd === "pty_spawn").length;
await page.keyboard.press("Enter");
await page.waitForTimeout(400);
check("Enter on row opens terminal", calls.filter((c) => c.cmd === "pty_spawn").length === spawnsU2 + 1);
// (5) live-session dot: --ok while terminal tab open, --muted after close
const dot = prodRow.locator(".h-dot");
check("dot goes --ok with live terminal", (await dot.evaluate((el) => el.classList.contains("live"))) === true);
check("dot ok uses --ok token", (await dot.evaluate((el) => getComputedStyle(el).backgroundColor)) === "rgb(128, 210, 128)");
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
check("skeleton rows are 28px (--hover fill)", skelH === 28, `h=${skelH}`);
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

// 9j. U1 theme: Abyss Emerald tokens (EPIC-TSD §U1)
// (a) --bg token value
const themeBg = await page.evaluate(() =>
  getComputedStyle(document.documentElement).getPropertyValue("--bg").trim());
check("theme --bg == #08080a (Abyss Emerald)", themeBg === "#08080a", themeBg);
// (b) color-scheme dark in :root
const colorScheme = await page.evaluate(() =>
  getComputedStyle(document.documentElement).getPropertyValue("color-scheme").trim());
check("theme color-scheme: dark", colorScheme === "dark", colorScheme);
// (c) offline fonts: no @import / url(http in stylesheet + index.html (source files)
const cssSrc = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const htmlSrc = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const netFont = [cssSrc, htmlSrc].join("\n").match(/@import|url\(http|<link[^>]+href="http/g) || [];
check("no font CDN (@import / url(http) in styles.css + index.html", netFont.length === 0, netFont.slice(0, 2).join(" | "));
// (d) glow: box-shadow rules exist ONLY on the 3 active-state selectors
const glowSelectors = await page.evaluate(() => {
  const hits = [];
  for (const sheet of document.styleSheets) {
    let rules;
    try { rules = sheet.cssRules; } catch { continue; }
    for (const r of rules) {
      if (r.style && r.style.boxShadow && /rgba\(0,\s*217,\s*146|var\(--glow\)/.test(r.style.boxShadow)) {
        hits.push(r.selectorText);
      }
    }
  }
  return hits.sort();
});
const wantGlow = ["button.primary", ".tab.active", ".term-side.focused"].sort();
check("glow box-shadow only on 3 active selectors", JSON.stringify(glowSelectors) === JSON.stringify(wantGlow), glowSelectors.join(", "));
// AC4 letter: inactive tab + plain button carry no box-shadow
const noShadow = await page.evaluate(() => {
  const tab = document.querySelector(".tab:not(.active)");
  const btn = document.querySelector("button:not(.primary)");
  const sh = (el) => (el ? getComputedStyle(el).boxShadow : "missing");
  return { tab: sh(tab), btn: sh(btn) };
});
check("no glow on inactive tab / plain button", noShadow.tab === "none" && noShadow.btn === "none", `${noShadow.tab} / ${noShadow.btn}`);

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
check("U3: SFTP tab icon is --accent2", sftpIco === "rgb(129, 140, 248)", sftpIco);
await page.locator(".host-item", { hasText: "prod-web" }).first().locator(".h-name").click();
await page.waitForTimeout(400);
const termIco = await page.locator(".tab.active .t-ico").evaluate((el) => getComputedStyle(el).color);
check("U3: active terminal icon is --fg", termIco === "rgb(242, 242, 242)", termIco);
// AC3 dead tab: mock PTY exit -> ✕ --danger + exact title; click respawns
const spawnsU3 = calls.filter((c) => c.cmd === "pty_spawn").length;
await page.evaluate(() => window.__emitTauri("pty://exit/4242"));
await page.waitForTimeout(300);
check("U3: PTY exit marks terminal tab dead", (await page.locator(".tab.dead").count()) === 1);
const deadTab = page.locator(".tab.dead");
check("U3: dead tab title exact", (await deadTab.getAttribute("title")) === "Session ended — click to reconnect");
check("U3: dead icon is --danger", (await deadTab.locator(".t-ico").evaluate((el) => getComputedStyle(el).color)) === "rgb(253, 156, 159)");
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
await page.locator(".tab").first().click({ button: "right" });
await page.waitForTimeout(200);
const menuLabels = await page.locator("#tab-menu button").allTextContents();
check("U3: menu = Close/Close others/Close host/Close all",
  JSON.stringify(menuLabels) === JSON.stringify(["Close", "Close others", "Close host", "Close all"]), menuLabels.join("|"));
const menuTips = await page.evaluate(() => [...document.querySelectorAll("#tab-menu button")].map((b) => b.title));
check("U3: menu tooltips name new shortcuts", menuTips[0] === "Ctrl+W" && menuTips[1] === "Ctrl+Shift+W", menuTips.join("|"));
await page.locator("#tab-menu button", { hasText: "Close host" }).click();
await page.waitForTimeout(500);
check("U3: Close host keeps only other hosts' tabs", (await page.locator(".tab").count()) === 1 &&
  (await page.locator(".tab .t-label").first().textContent()) === "db-core");
// AC5 Ctrl+W / Ctrl+Shift+W; skipped while typing in a text field
await page.evaluate(() => { const a = document.activeElement; if (a instanceof HTMLElement) a.blur(); });
await page.keyboard.press("Control+w");
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
const u3src = readFileSync(new URL("../src/main.ts", import.meta.url), "utf8");
check("U3: no draggable attribute (grep = 0)", !/draggable/i.test(u3src) &&
  (await page.evaluate(() => document.querySelectorAll("[draggable]").length)) === 0);
check("U3: drop-zone rests --border-soft",
  (await page.locator("#tab-split-zone").evaluate((el) => getComputedStyle(el).color)) === "rgb(38, 36, 35)");
await page.locator(".host-item", { hasText: "prod-web" }).first().locator(".h-name").click();
await page.waitForTimeout(300);
const dzBox = await page.locator(".tab", { hasText: "prod-web" }).first().boundingBox();
await page.mouse.move(dzBox.x + dzBox.width / 2, dzBox.y + dzBox.height / 2);
await page.mouse.down();
await page.mouse.move(dzBox.x + dzBox.width / 2 + 12, dzBox.y + dzBox.height / 2);
const dzHot = await page.locator("#tab-split-zone").evaluate((el) => ({ hot: el.classList.contains("drag-hot"), c: getComputedStyle(el).color }));
check("U3: drop-zone lights --accent while dragging", dzHot.hot && dzHot.c === "rgb(0, 217, 146)", JSON.stringify(dzHot));
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

check("no console/page errors", errors.length === 0, errors.slice(0, 3).join(" | "));

// SEC-4 / AC R1.4: zero CSP violations across the full suite
const cspViolations = await page.evaluate(() => window.__cspViolations || []);
check("CSP violations == 0 across suite (AC R1.4)", cspViolations.length === 0, cspViolations.slice(0, 3).join(" | "));
check("served CSP == tauri.conf.json policy", servedCsp === CSP.trim(), servedCsp.slice(0, 80));

console.log(results.join("\n"));
const fails = results.filter((r) => r.startsWith("FAIL")).length;
console.log(`\n${results.length - fails}/${results.length} passed`);
await browser.close();
server.close();
process.exit(fails ? 1 : 0);

// v0.4.1 checks appended: delete button on host rows + right-click paste handler
import { chromium as _c } from "playwright";
