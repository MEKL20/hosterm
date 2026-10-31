// Fresh independent rendered-UI probe for SDD §13.2.1 / INV-23.
// This captures DOM-observed accent/ink values only. WCAG math is performed
// separately by recompute_wcag.py; no production helper is imported or reused.
import { chromium } from "playwright";
import { createServer } from "node:http";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "../..");
const DIST = join(ROOT, "dist");
const OUT = process.env.EVIDENCE_JSON || join(HERE, "rendered-settings.json");
const SCREENSHOT = process.env.EVIDENCE_SCREENSHOT || join(HERE, "settings-rendered.png");
const TARGET_SHA = process.env.TARGET_SHA || "unknown";
const CSP = JSON.parse(readFileSync(join(ROOT, "src-tauri/tauri.conf.json"), "utf8")).app.security.csp;
const MIME = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml" };

const vectors = [
  { class: "curated", accent: "#e2a35c", expectedInk: "#241404" },
  { class: "curated", accent: "#5fbfae", expectedInk: "#241404" },
  { class: "curated", accent: "#e0869b", expectedInk: "#241404" },
  { class: "curated", accent: "#aec267", expectedInk: "#241404" },
  { class: "curated", accent: "#b99be3", expectedInk: "#241404" },
  { class: "free extreme", accent: "#000000", expectedInk: "#ffffff" },
  { class: "free existing dark sentinel", accent: "#101010", expectedInk: "#ffffff" },
  { class: "free former failure", accent: "#6f6f6f", expectedInk: "#ffffff" },
  { class: "free grayscale below switch", accent: "#757575", expectedInk: "#ffffff" },
  { class: "free closest 24-bit below switch", accent: "#766cb5", expectedInk: "#ffffff" },
  { class: "free closest 24-bit above switch", accent: "#cf0dcc", expectedInk: "#000000" },
  { class: "free grayscale above switch", accent: "#767676", expectedInk: "#000000" },
  { class: "free former failure", accent: "#777777", expectedInk: "#000000" },
  { class: "free extreme", accent: "#ffffff", expectedInk: "#000000" },
];

let storedSettings = { version: 1, theme: "dusk", accent: "#e2a35c", allowlist: [] };
const invokeCalls = [];
const writes = [];
let readSettingsCount = 0;
const invoke = async (cmd, args = {}) => {
  invokeCalls.push({ cmd, args });
  if (cmd === "read_ssh_config") return [];
  if (cmd === "check_binaries") return { ssh_missing: false, sftp_missing: false };
  if (cmd === "read_settings") {
    readSettingsCount += 1;
    return structuredClone(storedSettings);
  }
  if (cmd === "write_settings") {
    storedSettings = structuredClone(args.settings);
    writes.push(structuredClone(args.settings));
    return null;
  }
  throw new Error(`unexpected invoke in focused INV-23 probe: ${cmd}`);
};

const server = createServer((req, res) => {
  const rawPath = new URL(req.url || "/", "http://127.0.0.1").pathname;
  let path = join(DIST, rawPath === "/" ? "index.html" : rawPath.slice(1));
  if (!existsSync(path)) path = join(DIST, "index.html");
  res.setHeader("Content-Security-Policy", CSP);
  res.setHeader("Content-Type", MIME[extname(path)] || "application/octet-stream");
  res.end(readFileSync(path));
});
await new Promise((resolveListen, rejectListen) => {
  server.once("error", rejectListen);
  server.listen(0, "127.0.0.1", resolveListen);
});
const address = server.address();
if (!address || typeof address === "string") throw new Error("failed to bind QA server");
const url = `http://127.0.0.1:${address.port}/`;

const checks = [];
const check = (name, ok, detail = undefined) => {
  checks.push({ name, pass: Boolean(ok), ...(detail === undefined ? {} : { detail }) });
};
const waitForWrites = async (count) => {
  const deadline = Date.now() + 3000;
  while (writes.length < count && Date.now() < deadline) await new Promise((r) => setTimeout(r, 20));
  if (writes.length < count) throw new Error(`timed out waiting for write_settings call ${count}`);
};

let browser;
const evidence = {
  target_sha: TARGET_SHA,
  source: "rendered dist/ Settings <input type=color>; production helper not imported",
  vector_runs: [],
  former_failure_repeats: [],
  determinism: {},
  normalization: {},
  invoke_summary: {},
  console_errors: [],
  csp_violations: [],
  checks,
};

try {
  browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  page.on("pageerror", (error) => evidence.console_errors.push(`pageerror: ${String(error)}`));
  page.on("console", (message) => {
    if (message.type() === "error") evidence.console_errors.push(`console: ${message.text()}`);
  });
  await page.addInitScript(() => {
    window.__qaCallbacks = {};
    window.__qaEvents = {};
    window.__cspViolations = [];
    document.addEventListener("securitypolicyviolation", (event) => {
      window.__cspViolations.push(`${event.effectiveDirective}: ${event.blockedURI || event.sourceFile || "inline"}`);
    });
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
    window.__TAURI_INTERNALS__ = {
      transformCallback: (callback) => {
        const id = (window.__qaCallbackId = (window.__qaCallbackId || 0) + 1);
        window.__qaCallbacks[id] = callback;
        return id;
      },
      invoke: (cmd, args = {}) => {
        if (cmd === "plugin:event|listen") {
          (window.__qaEvents[args.event] = window.__qaEvents[args.event] || []).push(window.__qaCallbacks[args.handler]);
          return Promise.resolve(1);
        }
        if (cmd === "plugin:event|unlisten") return Promise.resolve(null);
        return globalThis.__qaInvoke(cmd, args);
      },
    };
  });
  await page.exposeFunction("__qaInvoke", invoke);
  const response = await page.goto(url, { waitUntil: "load" });
  check("dist served", response?.ok(), { status: response?.status() });
  await page.locator("#open-settings").waitFor({ state: "visible" });
  await page.waitForFunction(() => document.documentElement.style.getPropertyValue("--accent").trim() === "#e2a35c");

  const ensureSettingsOpen = async () => {
    const hidden = await page.locator("#settings-scrim").evaluate((element) => element.classList.contains("hidden"));
    if (hidden) {
      await page.locator("#open-settings").click();
      await page.locator("#settings-panel").waitFor({ state: "visible" });
    }
  };
  const capture = async (source, entered) => page.evaluate(({ source, entered }) => {
    const root = document.documentElement;
    const preview = document.getElementById("pv-mock");
    const input = document.getElementById("accent-input");
    const computed = getComputedStyle(root);
    return {
      source,
      entered,
      input_type: input.type,
      input_value: input.value,
      inline_accent: root.style.getPropertyValue("--accent").trim(),
      inline_ink: root.style.getPropertyValue("--accent-ink").trim(),
      computed_accent: computed.getPropertyValue("--accent").trim(),
      computed_ink: computed.getPropertyValue("--accent-ink").trim(),
      preview_accent: preview.style.getPropertyValue("--pa").trim(),
      preview_ink: preview.style.getPropertyValue("--pi").trim(),
      pressed_chips: [...document.querySelectorAll('.accent-chip[aria-pressed="true"]')].map((chip) => chip.dataset.accent),
    };
  }, { source, entered });
  const setViaInput = async (accent) => {
    await ensureSettingsOpen();
    await page.locator("#accent-input").evaluate((input, value) => {
      input.value = value;
      input.dispatchEvent(new Event("input", { bubbles: true }));
    }, accent);
    await page.waitForFunction((expected) => document.documentElement.style.getPropertyValue("--accent").trim() === expected, accent.toLowerCase());
    return capture("free-input", accent);
  };
  const setViaChip = async (accent) => {
    await ensureSettingsOpen();
    await page.locator(`.accent-chip[data-accent="${accent}"]`).click();
    await page.waitForFunction((expected) => document.documentElement.style.getPropertyValue("--accent").trim() === expected, accent);
    return capture("quick-chip", accent);
  };
  const persistAndReload = async (expectedAccent) => {
    const wantedWrites = writes.length + 1;
    const readsBefore = readSettingsCount;
    await page.locator("#settings-done").click();
    await waitForWrites(wantedWrites);
    await page.locator("#settings-scrim").waitFor({ state: "hidden" });
    const persisted = structuredClone(writes.at(-1));
    await page.reload({ waitUntil: "load" });
    await page.waitForFunction((expected) => document.documentElement.style.getPropertyValue("--accent").trim() === expected, expectedAccent);
    const boot = await capture("persist-reload", expectedAccent);
    return { persisted, boot, read_delta: readSettingsCount - readsBefore };
  };

  await ensureSettingsOpen();
  check("rendered control is input[type=color]", await page.locator("#accent-input").getAttribute("type") === "color");

  for (const vector of vectors) {
    const actual = await setViaInput(vector.accent);
    evidence.vector_runs.push({ ...vector, actual });
    check(`vector ${vector.accent} endpoint and no remap`,
      actual.input_value === vector.accent &&
      actual.inline_accent === vector.accent && actual.computed_accent === vector.accent &&
      actual.inline_ink === vector.expectedInk && actual.computed_ink === vector.expectedInk &&
      actual.preview_accent === vector.accent && actual.preview_ink === vector.expectedInk,
      actual);
  }

  for (let pass = 1; pass <= 2; pass += 1) {
    for (const probe of [
      { accent: "#6f6f6f", expectedInk: "#ffffff" },
      { accent: "#777777", expectedInk: "#000000" },
    ]) {
      const actual = await setViaInput(probe.accent);
      evidence.former_failure_repeats.push({ pass, ...probe, actual });
      check(`former failure ${probe.accent} repetition ${pass}`,
        actual.inline_accent === probe.accent && actual.input_value === probe.accent &&
        actual.inline_ink === probe.expectedInk && actual.preview_ink === probe.expectedInk,
        actual);
    }
  }

  const uppercase = await setViaInput("#E0869B");
  evidence.normalization.uppercase_curated = uppercase;
  check("uppercase valid value normalizes before mapping",
    uppercase.input_value === "#e0869b" && uppercase.inline_accent === "#e0869b" && uppercase.inline_ink === "#241404",
    uppercase);

  const curatedInput = await setViaInput("#5fbfae");
  await setViaChip("#e2a35c");
  const curatedChip = await setViaChip("#5fbfae");
  const curatedInputForPersist = await setViaInput("#5fbfae");
  const curatedReload = await persistAndReload("#5fbfae");
  evidence.determinism.curated = {
    accent: "#5fbfae",
    input: curatedInput,
    chip: curatedChip,
    input_before_persist: curatedInputForPersist,
    ...curatedReload,
  };
  check("curated input/chip/persist/reload deterministic",
    [curatedInput, curatedChip, curatedInputForPersist, curatedReload.boot].every((state) =>
      state.inline_accent === "#5fbfae" && state.inline_ink === "#241404" && state.preview_ink === "#241404") &&
    curatedReload.persisted.accent === "#5fbfae" && curatedReload.read_delta === 1,
    evidence.determinism.curated);

  const nonCuratedInput = await setViaInput("#777777");
  const nonCuratedReload = await persistAndReload("#777777");
  evidence.determinism.non_curated = {
    accent: "#777777",
    input: nonCuratedInput,
    ...nonCuratedReload,
  };
  check("non-curated input/persist/reload deterministic and unmodified",
    nonCuratedInput.inline_accent === "#777777" && nonCuratedInput.inline_ink === "#000000" &&
    nonCuratedReload.persisted.accent === "#777777" &&
    nonCuratedReload.boot.inline_accent === "#777777" && nonCuratedReload.boot.input_value === "#777777" &&
    nonCuratedReload.boot.inline_ink === "#000000" && nonCuratedReload.read_delta === 1,
    evidence.determinism.non_curated);

  await ensureSettingsOpen();
  await page.screenshot({ path: SCREENSHOT, fullPage: true });
  evidence.csp_violations = await page.evaluate(() => window.__cspViolations || []);
  evidence.invoke_summary = {
    read_settings: invokeCalls.filter((call) => call.cmd === "read_settings").length,
    write_settings: invokeCalls.filter((call) => call.cmd === "write_settings").length,
    commands: [...new Set(invokeCalls.map((call) => call.cmd))].sort(),
  };
  check("no console/page errors", evidence.console_errors.length === 0, evidence.console_errors);
  check("no CSP violations", evidence.csp_violations.length === 0, evidence.csp_violations);
} catch (error) {
  evidence.fatal_error = error?.stack || String(error);
  check("probe completed", false, evidence.fatal_error);
} finally {
  await browser?.close().catch(() => {});
  await new Promise((resolveClose) => server.close(resolveClose));
}

evidence.summary = {
  passed: checks.filter((entry) => entry.pass).length,
  failed: checks.filter((entry) => !entry.pass).length,
  total: checks.length,
};
writeFileSync(OUT, `${JSON.stringify(evidence, null, 2)}\n`);
console.log(JSON.stringify({ target_sha: TARGET_SHA, output: OUT, screenshot: SCREENSHOT, ...evidence.summary }));
process.exit(evidence.summary.failed === 0 && !evidence.fatal_error ? 0 : 1);
