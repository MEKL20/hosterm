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

const sampleSftp = [
  { name: "notes.txt", is_dir: false, is_link: false, size: 142 },
  { name: "projects", is_dir: true, is_link: false, size: 4096 },
];
const sampleHosts = [
  { name: "prod-web", host_name: "203.0.113.10", user: "deploy", port: "", identity_file: "", auth_method: "key", options: [] },
];
const sampleConfig = `# demo
Host prod-web
    HostName 203.0.113.10
    User deploy

Host *`;

const calls = [];
const invoke = (cmd, args = {}) => {
  calls.push({ cmd, args });
  switch (cmd) {
    case "read_ssh_config": return Promise.resolve(sampleHosts);
    case "read_config_raw": return Promise.resolve(sampleConfig);
    case "write_config_raw": return Promise.resolve();
    case "list_identity_files": return Promise.resolve(["~/.ssh/id_test"]);
    case "sftp_list": return Promise.resolve(sampleSftp);
    case "sftp_read_file": return Promise.resolve("hello editor\nline two\n");
    case "sftp_write_file": return Promise.resolve();
    case "save_host": case "delete_host": case "create_identity_file": return Promise.resolve();
    default: return Promise.reject(`unmocked: ${cmd}`);
  }
};

const server = createServer((req, res) => {
  let p = join(DIST, req.url === "/" ? "index.html" : req.url.split("?")[0]);
  if (!existsSync(p)) p = join(DIST, "index.html");
  res.setHeader("content-type", MIME[extname(p)] || "text/plain");
  res.end(readFileSync(p));
});
await new Promise((r) => server.listen(4199, r));

const browser = await chromium.launch();
const page = await browser.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
await page.addInitScript(() => {
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => cb,
    invoke: (cmd, args) => globalThis.__invokeImpl(cmd, args),
  };
});
await page.exposeFunction("__invokeImpl", invoke);
await page.goto("http://localhost:4199/");
await page.waitForTimeout(400);

const results = [];
const check = (name, ok, extra = "") => results.push(`${ok ? "PASS" : "FAIL"} ${name}${extra ? " — " + extra : ""}`);

// 1. host list renders from mock config
const hosts = await page.locator(".host-item").count();
check("host list renders", hosts === 1, `count=${hosts}`);

// 2. sidebar action buttons are real buttons, visible (not opacity:0)
const sftpBtn = page.locator(".host-item .h-sftp");
check("sidebar sftp action is a button", (await sftpBtn.evaluate((el) => el.tagName)) === "BUTTON");
const vis = await sftpBtn.evaluate((el) => getComputedStyle(el).opacity);
check("action buttons not hover-hidden", vis === "1", `opacity=${vis}`);

// 3. open SFTP tab via button
await sftpBtn.click();
await page.waitForTimeout(500);
const sftpRows = await page.locator(".sftp-row").count();
check("sftp rows render from mock", sftpRows === 2, `rows=${sftpRows}`);
const dirSize = await page.locator(".sftp-row", { hasText: "projects" }).locator(".sz").textContent();
check("dir size cell empty (no em dash)", dirSize.trim() === "", JSON.stringify(dirSize));

// 4. click the file row -> editor tab opens with CodeMirror content
await page.locator(".sftp-row", { hasText: "notes.txt" }).locator(".nm").click();
await page.waitForTimeout(500);
const cmContent = await page.locator(".cm-content").textContent();
check("editor opens with mocked file", cmContent.includes("hello editor"), cmContent.slice(0, 40));

// 5. dirty dot appears after edit
await page.locator(".cm-content").click();
await page.keyboard.type("X");
await page.waitForTimeout(200);
const dirty = await page.locator(".tab .t-dot").count();
check("dirty dot appears on edit", dirty === 1, `dots=${dirty}`);

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

check("no console/page errors", errors.length === 0, errors.slice(0, 3).join(" | "));

console.log(results.join("\n"));
const fails = results.filter((r) => r.startsWith("FAIL")).length;
console.log(`\n${results.length - fails}/${results.length} passed`);
await browser.close();
server.close();
process.exit(fails ? 1 : 0);
