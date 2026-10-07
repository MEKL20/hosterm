// Render the REAL production frontend (dist/) in Chromium with the Tauri IPC
// mocked, so the screenshot shows hosterm's actual UI with sample data.
import { chromium } from "playwright";

const URL = "http://127.0.0.1:4178/";

const hosts = [
  { name: "prod-web",     host_name: "203.0.113.10",         user: "deploy",   port: "22",   identity_file: "~/.ssh/id_ed25519", options: [] },
  { name: "staging",      host_name: "staging.example.com",  user: "ubuntu",   port: "2222", identity_file: "",                   options: [] },
  { name: "db-primary",   host_name: "198.51.100.42",        user: "postgres", port: "",     identity_file: "~/.ssh/id_db",       options: [] },
  { name: "raspberry-pi", host_name: "192.168.1.50",         user: "pi",       port: "",     identity_file: "",                   options: [] },
];

const init = (hostsJson) => {
  const H = JSON.parse(hostsJson);
  window.__listeners = {};
  window.__TAURI_INTERNALS__ = {
    transformCallback(cb) {
      const id = Math.floor(Math.random() * 1e9);
      window["_" + id] = cb;
      return id;
    },
    invoke(cmd, args) {
      if (cmd === "read_ssh_config") return Promise.resolve(H);
      if (cmd === "pty_spawn") return Promise.resolve(1);
      if (cmd === "plugin:event|listen") {
        window.__listeners[args.event] = args.handler;
        return Promise.resolve(1);
      }
      return Promise.resolve();
    },
  };
  window.__fireOutput = (id, text) => {
    const cbId = window.__listeners["pty://output/" + id];
    if (cbId != null && window["_" + cbId]) {
      window["_" + cbId]({ event: "pty://output/" + id, id: 0, payload: text });
    }
  };
};

const session = [
  "\x1b[32mdeploy@prod-web\x1b[0m:\x1b[34m~\x1b[0m$ uptime\r\n",
  " 13:48:02 up 42 days,  3:17,  2 users,  load average: 0.08, 0.03, 0.01\r\n",
  "\x1b[32mdeploy@prod-web\x1b[0m:\x1b[34m~\x1b[0m$ systemctl is-active nginx\r\n",
  "\x1b[32mactive\x1b[0m\r\n",
  "\x1b[32mdeploy@prod-web\x1b[0m:\x1b[34m~\x1b[0m$ docker ps --format '{{.Names}}\\t{{.Status}}'\r\n",
  "web-1\tUp 6 days\r\n",
  "worker-1\tUp 6 days\r\n",
  "redis\tUp 6 days (healthy)\r\n",
  "\x1b[32mdeploy@prod-web\x1b[0m:\x1b[34m~\x1b[0m$ \x1b[7m \x1b[0m\r\n",
].join("");

const browser = await chromium.launch({ args: ["--force-color-profile=srgb"] });
const page = await browser.newPage({ viewport: { width: 1120, height: 720 }, deviceScaleFactor: 2 });
await page.addInitScript(init, JSON.stringify(hosts));
await page.goto(URL, { waitUntil: "networkidle" });
await page.waitForSelector(".host-item");
// open a terminal tab for prod-web, then feed it the sample session
await page.locator(".host-item .h-name", { hasText: "prod-web" }).first().click();
await page.waitForTimeout(400);
await page.evaluate((t) => window.__fireOutput(1, t), session);
await page.waitForTimeout(600);
await page.screenshot({ path: "scripts/shot-main.png" });

// also capture the add/edit host modal
await page.locator(".host-item .h-name", { hasText: "staging" }).first().hover();
await page.locator('.host-item:has-text("staging") .h-edit').first().click();
await page.waitForSelector("#modal:not(.hidden)");
await page.waitForTimeout(300);
await page.screenshot({ path: "scripts/shot-edit.png" });

await browser.close();
console.log("OK wrote scripts/shot-main.png + scripts/shot-edit.png");
