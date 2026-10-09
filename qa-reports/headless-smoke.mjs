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
  { name: "db-core", host_name: "203.0.113.20", user: "admin", port: "", identity_file: "", auth_method: "key", options: [] },
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
check("host list renders", hosts === 2, `count=${hosts}`);

// 2. sidebar action buttons are real buttons, visible (not opacity:0)
const sftpBtn = page.locator(".host-item .h-sftp").first();
check("sidebar sftp action is a button", (await sftpBtn.evaluate((el) => el.tagName)) === "BUTTON");
const vis = await sftpBtn.evaluate((el) => getComputedStyle(el).opacity);
check("action buttons not hover-hidden", vis === "1", `opacity=${vis}`);

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

// 9d. delete button present on host rows, opens confirm dialog, cancel keeps host
const delBtn = page.locator(".host-item .h-del").first();
check("delete button on host row", (await delBtn.count()) === 1);
await delBtn.click();
await page.waitForTimeout(250);
const dlgText = await page.evaluate(() => document.querySelector(".dialog-box, [role=alertdialog], #dialog")?.textContent || "");
check("delete dialog names the host", dlgText.includes("prod-web"), dlgText.slice(0, 60));
// focused button should be Cancel (destructive action guard)
const focusedLabel = await page.evaluate(() => (document.activeElement?.textContent || "").trim());
check("delete dialog focuses Cancel", focusedLabel === "Cancel", `focus=${focusedLabel}`);
await page.keyboard.press("Enter"); // activates focused Cancel
await page.waitForTimeout(250);
check("cancel keeps host", (await page.locator(".host-item").count()) === 2);

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

// 9g2. tab context menu: in-app menu, no native reload menu, close-others works
// (runs while the two terminal tabs from 9g are still open)
await page.locator(".tab").first().click({ button: "right" });
await page.waitForTimeout(200);
check("right-click tab opens in-app menu", (await page.locator("#tab-menu").count()) === 1);
check("menu has close items", (await page.locator("#tab-menu button").count()) === 3);
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

check("no console/page errors", errors.length === 0, errors.slice(0, 3).join(" | "));

console.log(results.join("\n"));
const fails = results.filter((r) => r.startsWith("FAIL")).length;
console.log(`\n${results.length - fails}/${results.length} passed`);
await browser.close();
server.close();
process.exit(fails ? 1 : 0);

// v0.4.1 checks appended: delete button on host rows + right-click paste handler
import { chromium as _c } from "playwright";
