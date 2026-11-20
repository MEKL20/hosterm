// Probe: #panel-toggle position vs opened sidebar (before/after click).
import { chromium } from "playwright";
import { createServer } from "http";
import { readFileSync, existsSync, readdirSync } from "fs";
import { extname, join } from "path";
import { fileURLToPath } from "url";

const DIST = fileURLToPath(new URL("../dist", import.meta.url));
const MIME = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css" };
const server = createServer((req, res) => {
  let p = join(DIST, req.url === "/" ? "index.html" : decodeURIComponent(req.url.split("?")[0]));
  if (!existsSync(p)) p = join(DIST, "index.html");
  const ext = extname(p);
  if (!MIME[ext]) { res.writeHead(404); res.end(); return; }
  res.writeHead(200, { "content-type": MIME[ext] });
  res.end(readFileSync(p));
});
await new Promise((r) => server.listen(4198, "127.0.0.1", r));

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
await page.addInitScript(() => {
  window.__cbMap = {};
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => { const id = (window.__cbN = (window.__cbN || 0) + 1); window.__cbMap[id] = cb; return id; },
    invoke: (cmd, args) => {
      if (cmd === "plugin:event|listen") return Promise.resolve(1);
      if (cmd === "plugin:event|unlisten") return Promise.resolve("");
      return Promise.resolve(null);
    },
  };
});
await page.goto("http://127.0.0.1:4198/");
await page.waitForFunction(() => document.getElementById("panel-toggle"));
await page.waitForTimeout(400);

const rects = () => page.evaluate(() => {
  const r = (id) => { const e = document.getElementById(id); const b = e.getBoundingClientRect();
    return { x: Math.round(b.x), y: Math.round(b.y), w: Math.round(b.width), h: Math.round(b.height), display: getComputedStyle(e).display }; };
  return { toggle: r("panel-toggle"), rail: r("nav-rail"), panel: r("section-panel"), tabbar: r("tabbar") };
});

const collapsed = await rects();
await page.locator("#panel-toggle").click();
await page.waitForTimeout(200);
const expanded = await rects();
await page.screenshot({ path: "qa-reports/after-toggle-expanded.png" });
await page.locator("#panel-toggle").click();
await page.waitForTimeout(200);
const recollapsed = await rects();
await page.screenshot({ path: "qa-reports/after-toggle-collapsed.png" });

console.log(JSON.stringify({ collapsed, expanded, recollapsed }, null, 2));
const gapExpanded = expanded.rail.x - (expanded.toggle.x + expanded.toggle.w);
const gapCollapsed = collapsed.tabbar.x - (collapsed.toggle.x + collapsed.toggle.w);
console.log("gap toggle->rail when expanded:", gapExpanded);
console.log("gap toggle->tabbar when collapsed:", gapCollapsed);
await browser.close();
server.close();
