import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { basicSetup, EditorView } from "codemirror";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import "@xterm/xterm/css/xterm.css";

interface HostDto {
  name: string;
  host_name: string;
  user: string;
  port: string;
  identity_file: string;
  auth_method: string;
  options: [string, string][];
}

// xterm cannot read CSS custom properties; keep this identical to --font-mono in styles.css
const FONT_MONO = 'ui-monospace, "Cascadia Code", Menlo, monospace';

const svg = (p: string) =>
  `<svg viewBox="0 0 16 16" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${p}</svg>`;
const ICONS = {
  add: svg('<line x1="8" y1="4" x2="12" y2="4"/><line x1="10" y1="2" x2="10" y2="6"/>'),
  reload: svg('<path d="M13 8a5 5 0 1 1-1.7-3.75"/><polyline points="13,1.5 13,4.5 10,4.5"/>'),
  terminal: svg('<rect x="1.5" y="2.5" width="13" height="11" rx="2"/><polyline points="4,6 6,8 4,10"/><line x1="8" y1="10" x2="11" y2="10"/>'),
  arrows: svg('<line x1="5" y1="3" x2="5" y2="13"/><polyline points="3,5 5,3 7,5"/><line x1="11" y1="13" x2="11" y2="3"/><polyline points="9,11 11,13 13,11"/>'),
  pencil: svg('<line x1="3" y1="13" x2="12" y2="4"/><line x1="10.5" y1="5.5" x2="12" y2="7"/>'),
  close: svg('<line x1="4" y1="4" x2="12" y2="12"/><line x1="12" y1="4" x2="4" y2="12"/>'),
  folder: svg('<path d="M1.5,4 h4 l1.5,2 h7.5 v7 h-13 z"/>'),
  file: svg('<path d="M3.5,1.5 h6 l3,3 v10 h-9 z"/><polyline points="9.5,1.5 9.5,4.5 12.5,4.5"/>'),
  link: svg('<rect x="9" y="9" width="5" height="5" rx="1"/><polyline points="2,4 8,4 8,9"/><polyline points="6,7 8,9 6,11"/>'),
  download: svg('<line x1="8" y1="2" x2="8" y2="10"/><polyline points="5,7.5 8,10.5 11,7.5"/><line x1="3" y1="13.5" x2="13" y2="13.5"/>'),
  upload: svg('<line x1="8" y1="13" x2="8" y2="5"/><polyline points="5,7.5 8,4.5 11,7.5"/><line x1="3" y1="13.5" x2="13" y2="13.5"/>'),
  up: svg('<line x1="8" y1="13" x2="8" y2="4"/><polyline points="4.5,7.5 8,4 11.5,7.5"/>'),
};

const cmTheme = EditorView.theme({
  "&": { color: "var(--fg)", backgroundColor: "var(--bg)" },
  ".cm-content": { fontFamily: FONT_MONO, fontSize: "13px", caretColor: "var(--accent2)" },
  "&.cm-focused": { outline: "none" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--accent2)" },
  ".cm-gutters": {
    backgroundColor: "var(--bg2)",
    color: "var(--muted)",
    borderRight: "1px solid var(--border-soft)",
  },
  ".cm-activeLine": { backgroundColor: "#232430" },
  ".cm-activeLineGutter": { backgroundColor: "#282A37", color: "var(--fg)" },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": { backgroundColor: "#374465" },
  ".cm-matchingBracket": { outline: "1px solid var(--accent2)", backgroundColor: "transparent" },
}, { dark: true });

const cmHighlight = HighlightStyle.define([
  { tag: t.keyword, color: "#bb9af7" },
  { tag: t.string, color: "#9ece6a" },
  { tag: t.number, color: "#ff9e64" },
  { tag: t.comment, color: "#737aa2" },
  { tag: t.variableName, color: "#c8d3f5" },
  { tag: [t.function(t.variableName)], color: "#7aa2f7" },
  { tag: t.typeName, color: "#7dcfff" },
  { tag: t.propertyName, color: "#89ddff" },
  { tag: t.operator, color: "#89ddff" },
  { tag: t.punctuation, color: "#a9b1d6" },
]);

// ---------- state ----------
let hosts: HostDto[] = [];
let editingName: string | null = null; // null => adding new
let identityFiles: string[] = [];

interface Session {
  id: number;
  ptyId: number | null;
  host: string;
  term: Terminal;
  fit: FitAddon;
  unlisten: UnlistenFn[];
}
const sessions = new Map<number, Session>();
let nextTab = 1;
let activeTab: number | null = null;

// ---------- helpers ----------
const $ = <T extends HTMLElement>(sel: string) => document.querySelector(sel) as T;

// backend errors arrive as plain strings; keep only the detail, copy adds the prefix and next action
function msg(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

function esc(s: string): string {
  const d = document.createElement("div");
  d.textContent = s;
  return d.innerHTML;
}

function escAttr(s: string): string {
  return esc(s).replace(/"/g, "&quot;");
}

// ---------- overlays: Escape, focus trap, focus restore ----------
const overlayStack: { el: HTMLElement; restore: HTMLElement | null }[] = [];
let overlayKeydown: ((ev: KeyboardEvent) => void) | null = null;
let dialogResolve: ((id: string) => void) | null = null;
let dialogEscapeId = "cancel";

function focusables(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>("button, input, textarea, select")].filter(
    (el) => (el as HTMLButtonElement).disabled !== true && !el.closest(".hidden")
  );
}

function onOverlayEscape() {
  const top = overlayStack[overlayStack.length - 1];
  if (!top) return;
  if (top.el.id === "dialog") resolveDialog(dialogEscapeId);
  else if (top.el.id === "modal") cancelEditor();
  else if (top.el.id === "raw-modal") cancelRaw();
  else if (top.el.id === "key-modal") closeKey();
}

function openOverlay(sel: string, focusTarget?: HTMLElement) {
  const el = $(sel);
  if (!overlayStack.some((o) => o.el === el)) {
    overlayStack.push({ el, restore: document.activeElement as HTMLElement | null });
    if (overlayStack.length === 1) $("#app").setAttribute("inert", "");
  }
  el.classList.remove("hidden");
  if (!overlayKeydown) {
    overlayKeydown = (ev) => {
      const top = overlayStack[overlayStack.length - 1];
      if (!top) return;
      if (ev.key === "Escape") {
        ev.preventDefault();
        onOverlayEscape();
      } else if (ev.key === "Tab") {
        const f = focusables(top.el);
        if (!f.length) return;
        const first = f[0];
        const last = f[f.length - 1];
        const cur = document.activeElement;
        const outside = !top.el.contains(cur);
        if (ev.shiftKey && (cur === first || outside)) {
          ev.preventDefault();
          last.focus();
        } else if (!ev.shiftKey && (cur === last || outside)) {
          ev.preventDefault();
          first.focus();
        }
      }
    };
    window.addEventListener("keydown", overlayKeydown);
  }
  const target = focusTarget ?? focusables(el)[0];
  target?.focus();
}

function closeOverlay(sel: string) {
  const el = $(sel);
  el.classList.add("hidden");
  const i = overlayStack.findIndex((o) => o.el === el);
  if (i === -1) return;
  const [closed] = overlayStack.splice(i, 1);
  if (overlayStack.length === 0) {
    window.removeEventListener("keydown", overlayKeydown as (ev: KeyboardEvent) => void);
    overlayKeydown = null;
    $("#app").removeAttribute("inert");
  }
  closed.restore?.focus();
}

type DialogAction = { label: string; id: string; style?: "primary" | "danger" | "default" };

function dialog(
  title: string,
  message: string,
  actions: DialogAction[],
  focusId?: string,
  escapeId = "cancel"
): Promise<string> {
  return new Promise((resolve) => {
    dialogResolve = resolve;
    dialogEscapeId = escapeId;
    $("#dialog-title").textContent = title;
    $("#dialog-msg").textContent = message;
    const wrap = $("#dialog-actions");
    wrap.innerHTML = "";
    let first: HTMLButtonElement | null = null;
    let focusBtn: HTMLButtonElement | null = null;
    for (const a of actions) {
      const b = document.createElement("button");
      b.textContent = a.label;
      if (a.style === "primary") b.classList.add("primary");
      if (a.style === "danger") b.classList.add("danger-filled");
      b.addEventListener("click", () => resolveDialog(a.id));
      wrap.appendChild(b);
      first = first ?? b;
      if (a.id === focusId) focusBtn = b;
    }
    openOverlay("#dialog", focusBtn ?? first ?? undefined);
  });
}

function resolveDialog(id: string) {
  if (!dialogResolve) return;
  const r = dialogResolve;
  dialogResolve = null;
  closeOverlay("#dialog");
  r(id);
}

// ---------- sidebar ----------
async function loadHosts() {
  const list = $("#host-list");
  list.innerHTML = `<div class="list-state"><span class="spinner"></span>Loading hosts…</div>`;
  try {
    hosts = await invoke<HostDto[]>("read_ssh_config");
  } catch (e) {
    hosts = [];
    list.innerHTML = "";
    const err = document.createElement("div");
    err.className = "list-state err";
    err.textContent = `Could not read ~/.ssh/config: ${msg(e)}. Check the file, then press Reload.`;
    list.appendChild(err);
    console.error(e);
    return;
  }
  renderHostList();
}

function renderHostList() {
  const list = $("#host-list");
  list.innerHTML = "";
  if (hosts.length === 0) {
    const empty = document.createElement("div");
    empty.className = "list-state";
    empty.textContent = "No hosts in ~/.ssh/config yet. Click + to add one.";
    list.appendChild(empty);
    return;
  }
  for (const h of hosts) {
    if (h.name.includes("*") || h.name.includes("?")) continue; // skip pattern blocks
    const el = document.createElement("div");
    el.className = "host-item";
    el.setAttribute("role", "button");
    el.tabIndex = 0;
    el.setAttribute("aria-label", `Open terminal to ${h.name}`);
    const sub = [h.user && `${h.user}@`, h.host_name || "(no HostName)", h.port && `:${h.port}`]
      .filter(Boolean)
      .join("");
    el.innerHTML =
      `<div class="h-line"><span class="h-name">${esc(h.name)}</span>` +
      `<span class="h-acts">` +
      `<button class="h-sftp icon" title="SFTP">${ICONS.arrows}</button>` +
      `<button class="h-edit icon" title="Edit">${ICONS.pencil}</button>` +
      `</span></div>` +
      `<div class="h-sub">${esc(sub)}</div>`;
    (el.querySelector(".h-sftp") as HTMLElement).setAttribute("aria-label", `Open SFTP for ${h.name}`);
    (el.querySelector(".h-edit") as HTMLElement).setAttribute("aria-label", `Edit ${h.name}`);
    el.addEventListener("click", (ev) => {
      const t = ev.target as HTMLElement;
      if (t.closest(".h-edit")) openEditor(h);
      else if (t.closest(".h-sftp")) openSftp(h.name);
      else openTerminal(h.name);
    });
    el.addEventListener("keydown", (ev) => {
      if ((ev.target as HTMLElement).closest("button")) return;
      if (ev.key === "Enter" || ev.key === " ") {
        ev.preventDefault();
        openTerminal(h.name);
      }
    });
    list.appendChild(el);
  }
}

// ---------- host editor modal ----------
async function refreshIdentityDropdown(selected: string) {
  try {
    identityFiles = await invoke<string[]>("list_identity_files");
  } catch {
    identityFiles = [];
  }
  const sel = $("#f-identity") as HTMLSelectElement;
  const opts = ['<option value="">(none)</option>'];
  const known = new Set(identityFiles);
  if (selected && !known.has(selected)) known.add(selected); // keep an existing custom path
  for (const p of known) {
    opts.push(`<option value="${escAttr(p)}"${p === selected ? " selected" : ""}>${esc(p)}</option>`);
  }
  sel.innerHTML = opts.join("");
  sel.value = selected || "";
}

function applyAuthMode(method: string) {
  $("#row-identity").classList.toggle("hidden", method !== "key");
  $("#row-password-note").classList.toggle("hidden", method !== "password");
}

function modalValues(): string {
  return JSON.stringify([
    ($("#f-name") as HTMLInputElement).value,
    ($("#f-hostname") as HTMLInputElement).value,
    ($("#f-user") as HTMLInputElement).value,
    ($("#f-port") as HTMLInputElement).value,
    ($("#f-auth") as HTMLSelectElement).value,
    ($("#f-identity") as HTMLSelectElement).value,
  ]);
}

let hostSnapshot = "{}";

async function openEditor(h: HostDto | null) {
  editingName = h ? h.name : null;
  $("#modal-title").textContent = h ? `Edit ${h.name}` : "Add host";
  ($("#f-name") as HTMLInputElement).value = h?.name ?? "";
  ($("#f-hostname") as HTMLInputElement).value = h?.host_name ?? "";
  ($("#f-user") as HTMLInputElement).value = h?.user ?? "";
  ($("#f-port") as HTMLInputElement).value = h?.port ?? "";
  const method = h?.auth_method === "password" ? "password" : "key";
  ($("#f-auth") as HTMLSelectElement).value = method;
  applyAuthMode(method);
  await refreshIdentityDropdown(h?.identity_file ?? "");
  $("#modal-err").textContent = "";
  $("#m-delete").classList.toggle("hidden", !h);
  hostSnapshot = modalValues();
  openOverlay("#modal", $("#f-name"));
}

function closeEditor() {
  closeOverlay("#modal");
}

function modalDirty(): boolean {
  return modalValues() !== hostSnapshot;
}

async function dirtyDialog(save: () => Promise<void>, discard: () => void, name: string) {
  const choice = await dialog(
    "Unsaved changes",
    `${name} has unsaved changes.`,
    [
      { label: "Save and close", id: "save", style: "primary" },
      { label: "Discard changes", id: "discard", style: "danger" },
      { label: "Cancel", id: "cancel" },
    ],
    "save",
    "cancel"
  );
  if (choice === "save") await save();
  else if (choice === "discard") discard();
}

function cancelEditor() {
  if (!modalDirty()) return closeEditor();
  dirtyDialog(saveHost, closeEditor, editingName ?? "New host");
}

async function saveHost() {
  const method = ($("#f-auth") as HTMLSelectElement).value;
  const host = {
    name: ($("#f-name") as HTMLInputElement).value.trim(),
    host_name: ($("#f-hostname") as HTMLInputElement).value.trim(),
    user: ($("#f-user") as HTMLInputElement).value.trim(),
    port: ($("#f-port") as HTMLInputElement).value.trim(),
    identity_file: method === "key" ? ($("#f-identity") as HTMLSelectElement).value.trim() : "",
    auth_method: method,
  };
  try {
    await invoke("save_host", { originalName: editingName, host });
    closeEditor();
    await loadHosts();
  } catch (e) {
    $("#modal-err").textContent = `Failed to save host: ${msg(e)}. Fix the field and press Save again.`;
  }
}

// ---------- create identity key from pasted text ----------
function openKeyModal() {
  ($("#k-name") as HTMLInputElement).value = "";
  ($("#k-text") as HTMLTextAreaElement).value = "";
  $("#key-err").textContent = "";
  openOverlay("#key-modal", $("#k-name"));
}

function closeKey() {
  closeOverlay("#key-modal");
}

async function saveKey() {
  const name = ($("#k-name") as HTMLInputElement).value.trim();
  const privateKey = ($("#k-text") as HTMLTextAreaElement).value;
  try {
    const path = await invoke<string>("create_identity_file", { name, privateKey });
    closeKey();
    // select the freshly-created key in the host editor dropdown
    await refreshIdentityDropdown(path);
    ($("#f-auth") as HTMLSelectElement).value = "key";
    applyAuthMode("key");
  } catch (e) {
    $("#key-err").textContent = `Failed to save key: ${msg(e)}. Check the name and press Save key again.`;
  }
}

async function deleteHost() {
  if (!editingName) return;
  const choice = await dialog(
    "Delete host",
    `Delete host "${editingName}" from ~/.ssh/config? This cannot be undone.`,
    [
      { label: "Delete", id: "delete", style: "danger" },
      { label: "Cancel", id: "cancel" },
    ],
    "cancel",
    "cancel"
  );
  if (choice !== "delete") return;
  try {
    await invoke("delete_host", { name: editingName });
    closeEditor();
    await loadHosts();
  } catch (e) {
    $("#modal-err").textContent = `Failed to delete host: ${msg(e)}. Fix the problem and press Delete again.`;
  }
}

// ---------- raw config modal ----------
function rawDirty(): boolean {
  return ($("#raw-text") as HTMLTextAreaElement).value !== rawSnapshot;
}

let rawSnapshot = "";

async function openRaw() {
  try {
    const text = await invoke<string>("read_config_raw");
    ($("#raw-text") as HTMLTextAreaElement).value = text;
    $("#raw-err").textContent = "";
    rawSnapshot = text;
    openOverlay("#raw-modal", $("#raw-text"));
  } catch (e) {
    await dialog(
      "Could not open ~/.ssh/config",
      `Could not read ~/.ssh/config: ${msg(e)}. Check the file, then press Reload.`,
      [{ label: "OK", id: "ok" }],
      "ok",
      "ok"
    );
  }
}

function closeRaw() {
  closeOverlay("#raw-modal");
}

function cancelRaw() {
  if (!rawDirty()) return closeRaw();
  dirtyDialog(saveRaw, closeRaw, "The config");
}

async function saveRaw() {
  try {
    await invoke("write_config_raw", { content: ($("#raw-text") as HTMLTextAreaElement).value });
    closeRaw();
    await loadHosts();
  } catch (e) {
    $("#raw-err").textContent = `Failed to save config: ${msg(e)}. Fix the problem and press Save file again.`;
  }
}

// ---------- sftp browser ----------
type SftpSession = { host: string; path: string };
type SftpEntry = { name: string; is_dir: boolean; is_link: boolean; size: number };
const sftpSessions = new Map<number, SftpSession>();

function fmtSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1048576) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1073741824) return `${(n / 1048576).toFixed(1)} MB`;
  return `${(n / 1073741824).toFixed(1)} GB`;
}

function baseName(p: string): string {
  return p.split(/[\\/]/).filter(Boolean).pop() ?? p;
}

function joinRemote(dir: string, name: string): string {
  const d = dir.replace(/\/+$/, "");
  return d ? `${d}/${name}` : name;
}

type StatusFn = (m: string, kind?: "" | "ok" | "err") => void;

async function openSftp(host: string) {
  const tabId = nextTab++;
  sftpSessions.set(tabId, { host, path: "" });
  const tab = makeTab(tabId, "sftp", host);
  $("#tabs").appendChild(tab);

  const panel = document.createElement("div");
  panel.className = "panel";
  panel.dataset.tab = String(tabId);
  panel.setAttribute("role", "tabpanel");
  panel.innerHTML = `
    <div class="sftp-wrap">
      <div class="sftp-toolbar">
        <button class="sftp-up icon" aria-label="Parent directory" title="Parent directory">${ICONS.up}</button>
        <input class="sftp-path" placeholder="remote path (empty = home)" />
        <button class="sftp-go">Go</button>
        <button class="sftp-refresh icon" aria-label="Reload directory" title="Reload directory">${ICONS.reload}</button>
      </div>
      <div class="xfer-bar"></div>
      <div class="sftp-list"></div>
      <div class="sftp-foot">
        <input class="sftp-local" placeholder="local path (download dir / file to upload)" />
        <button class="sftp-upload">Upload</button>
      </div>
      <div class="sftp-status"></div>
    </div>`;
  $("#panels").appendChild(panel);

  const q = (sel: string) => panel.querySelector(sel) as HTMLElement;
  const inp = q(".sftp-path") as HTMLInputElement;
  const loc = q(".sftp-local") as HTMLInputElement;
  const status: StatusFn = (m, kind = "") => {
    const el = q(".sftp-status");
    el.textContent = m;
    el.classList.toggle("ok", kind === "ok");
    el.classList.toggle("err", kind === "err");
  };
  const busy = (on: boolean) => {
    q(".xfer-bar").classList.toggle("on", on);
    (q(".sftp-upload") as HTMLButtonElement).disabled = on;
  };

  q(".sftp-go").addEventListener("click", () => {
    (sftpSessions.get(tabId) as SftpSession).path = inp.value.trim();
    renderSftp(tabId, status);
  });
  inp.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") (q(".sftp-go") as HTMLButtonElement).click();
  });
  q(".sftp-up").addEventListener("click", () => {
    const s = sftpSessions.get(tabId) as SftpSession;
    s.path = s.path.replace(/\/+[^/]*$/, "");
    inp.value = s.path;
    renderSftp(tabId, status);
  });
  q(".sftp-refresh").addEventListener("click", () => renderSftp(tabId, status));
  q(".sftp-upload").addEventListener("click", async () => {
    const localPath = loc.value.trim().replace(/[\/]+$/, "");
    if (!localPath) return status("Enter a local file path to upload, then press Upload.", "err");
    const remote = joinRemote((sftpSessions.get(tabId) as SftpSession).path, baseName(localPath));
    busy(true);
    status(`Uploading ${baseName(localPath)}…`);
    try {
      await invoke("sftp_upload", { host, localPath, remotePath: remote });
      status(`Uploaded to ${remote}`, "ok");
      renderSftp(tabId, status);
    } catch (e) {
      status(`Failed to upload ${baseName(localPath)}: ${msg(e)}. Check the local path and try again.`, "err");
    } finally {
      busy(false);
    }
  });

  activateTab(tabId);
  renderSftp(tabId, status);
}

async function renderSftp(tabId: number, status: StatusFn) {
  const s = sftpSessions.get(tabId);
  if (!s) return;
  const panel = document.querySelector(`.panel[data-tab="${tabId}"]`) as HTMLElement;
  const list = panel.querySelector(".sftp-list") as HTMLElement;
  const inp = panel.querySelector(".sftp-path") as HTMLInputElement;
  inp.value = s.path;
  list.innerHTML = `<div class="list-state"><span class="spinner"></span>Listing ${esc(s.path || "~")}…</div>`;
  let entries: SftpEntry[];
  try {
    entries = await invoke<SftpEntry[]>("sftp_list", { host: s.host, path: s.path });
  } catch (err) {
    list.innerHTML = "";
    status(`Failed to list ${s.path || "~"}: ${msg(err)}. Fix the path and press Go, or go up one level.`, "err");
    return;
  }
  list.innerHTML = "";
  if (entries.length === 0) {
    list.innerHTML = `<div class="list-state">Empty directory.</div>`;
  }
  for (const e of entries) {
    const row = document.createElement("div");
    row.className = "sftp-row";
    const ico = e.is_dir ? ICONS.folder : e.is_link ? ICONS.link : ICONS.file;
    row.innerHTML =
      `<span class="f-ico${e.is_dir ? " dir" : ""}"${e.is_link ? ' title="Symbolic link"' : ""}>${ico}</span>` +
      `<span class="nm">${esc(e.name)}</span>` +
      `<span class="sz">${e.is_dir ? "" : fmtSize(e.size)}</span>`;
    if (!e.is_dir) {
      const dl = document.createElement("button");
      dl.className = "dl icon";
      dl.title = "Download";
      dl.setAttribute("aria-label", `Download ${e.name}`);
      dl.innerHTML = ICONS.download;
      dl.addEventListener("click", (ev) => {
        ev.stopPropagation();
        const localDir = (panel.querySelector(".sftp-local") as HTMLInputElement).value.trim().replace(/[\/]+$/, "");
        if (!localDir) return status("Set a local download directory in the bottom input first.", "err");
        const local = `${localDir}/${e.name}`;
        const bar = panel.querySelector(".xfer-bar") as HTMLElement;
        const up = panel.querySelector(".sftp-upload") as HTMLButtonElement;
        bar.classList.add("on");
        up.disabled = true;
        status(`Downloading ${e.name}…`);
        invoke("sftp_download", { host: s.host, remotePath: joinRemote(s.path, e.name), localPath: local })
          .then(() => status(`Saved to ${local}`, "ok"))
          .catch((err) => status(`Failed to download ${e.name}: ${msg(err)}. Check the local path and try again.`, "err"))
          .finally(() => {
            bar.classList.remove("on");
            up.disabled = false;
          });
      });
      row.appendChild(dl);
    }
    row.addEventListener("click", () => {
      if (e.is_dir) {
        s.path = joinRemote(s.path, e.name);
        renderSftp(tabId, status);
      } else {
        openRemoteEditor(s.host, joinRemote(s.path, e.name));
      }
    });
    list.appendChild(row);
  }
  status(`${entries.length} ${entries.length === 1 ? "item" : "items"} · ${s.host}:${s.path || "~"}`);
}

// ---------- remote text editor ----------
type EditorSession = {
  host: string;
  path: string;
  view: EditorView;
  dirty: boolean;
  save: () => Promise<void>;
};
const editorSessions = new Map<number, EditorSession>();

function editorLabel(s: EditorSession): string {
  return baseName(s.path);
}

async function openRemoteEditor(host: string, path: string) {
  // reuse an existing editor tab for the same file
  for (const [id, s] of editorSessions) {
    if (s.host === host && s.path === path) return activateTab(id);
  }

  const tabId = nextTab++;
  const tab = makeTab(tabId, "editor", baseName(path));
  const sess: EditorSession = { host, path, view: null as unknown as EditorView, dirty: false, save: async () => {} };
  editorSessions.set(tabId, sess);

  const setTitle = () => {
    (tab.querySelector(".t-label") as HTMLElement).textContent = editorLabel(sess);
    tab.classList.toggle("dirty", sess.dirty);
  };

  $("#tabs").appendChild(tab);

  const panel = document.createElement("div");
  panel.className = "panel";
  panel.dataset.tab = String(tabId);
  panel.setAttribute("role", "tabpanel");
  panel.innerHTML = `
    <div class="ed-wrap">
      <div class="ed-bar">
        <span class="ed-path" title="${escAttr(path)}">${esc(host)}:${esc(path)}</span>
        <button class="ed-reload">Reload</button>
        <button class="ed-save primary">Save</button>
      </div>
      <div class="ed-editor"></div>
      <div class="ed-status"></div>
    </div>`;
  $("#panels").appendChild(panel);

  const q = (sel: string) => panel.querySelector(sel) as HTMLElement;
  const saveBtn = q(".ed-save") as HTMLButtonElement;
  const status: StatusFn = (m, kind = "") => {
    const el = q(".ed-status");
    el.textContent = m;
    el.classList.toggle("ok", kind === "ok");
    el.classList.toggle("err", kind === "err");
  };
  const save = async () => {
    saveBtn.disabled = true;
    status("Saving…");
    try {
      await invoke("sftp_write_file", { host, remotePath: path, content: sess.view.state.doc.toString() });
      sess.dirty = false;
      setTitle();
      status(`Saved ${new Date().toTimeString().slice(0, 8)}`, "ok");
    } catch (e) {
      status(`Failed to save: ${msg(e)}. Check the file and press Save again.`, "err");
    } finally {
      saveBtn.disabled = false;
    }
  };
  sess.save = save;
  saveBtn.disabled = true; // until the file is loaded
  saveBtn.addEventListener("click", save);
  q(".ed-reload").addEventListener("click", async () => {
    try {
      const text = await invoke<string>("sftp_read_file", { host, remotePath: path });
      sess.view.dispatch({ changes: { from: 0, to: sess.view.state.doc.length, insert: text } });
      sess.dirty = false;
      setTitle();
      status("Reloaded from server", "ok");
    } catch (e) {
      status(`Failed to reload: ${msg(e)}. Check the file exists, then press Reload again.`, "err");
    }
  });

  activateTab(tabId);
  status(`Opening ${baseName(path)}…`);
  try {
    const text = await invoke<string>("sftp_read_file", { host, remotePath: path });
    const view = new EditorView({
      doc: text,
      extensions: [
        basicSetup,
        EditorView.lineWrapping,
        cmTheme,
        syntaxHighlighting(cmHighlight),
        EditorView.updateListener.of((u) => {
          if (u.docChanged && !sess.dirty) {
            sess.dirty = true;
            setTitle();
          }
        }),
      ],
      parent: q(".ed-editor"),
    });
    sess.view = view;
    saveBtn.disabled = false;
    view.dom.addEventListener("keydown", (ev) => {
      if ((ev.ctrlKey || ev.metaKey) && ev.key === "s") {
        ev.preventDefault();
        save();
      }
    });
    const lines = text.split("\n").length;
    status(`${lines} ${lines === 1 ? "line" : "lines"} · ${host}:${path}`, "ok");
  } catch (e) {
    status(`Failed to open ${path}: ${msg(e)}. Check the file exists, then press Reload.`, "err");
  }
}

// ---------- tabs ----------
function makeTab(tabId: number, kind: "terminal" | "sftp" | "editor", label: string): HTMLElement {
  const tab = document.createElement("div");
  tab.className = "tab";
  tab.dataset.tab = String(tabId);
  tab.setAttribute("role", "tab");
  tab.tabIndex = 0;
  tab.setAttribute("aria-selected", "false");
  const ico = kind === "terminal" ? ICONS.terminal : kind === "sftp" ? ICONS.arrows : ICONS.pencil;
  tab.innerHTML =
    `<span class="t-ico">${ico}</span>` +
    `<span class="t-label">${esc(label)}</span>` +
    (kind === "editor" ? `<span class="t-dot" title="Unsaved changes" aria-label="Unsaved changes"></span>` : "") +
    `<button class="x icon" aria-label="Close tab" title="Close">${ICONS.close}</button>`;
  tab.addEventListener("click", (ev) => {
    if ((ev.target as HTMLElement).closest(".x")) closeTab(tabId);
    else activateTab(tabId);
  });
  tab.addEventListener("keydown", (ev) => {
    if ((ev.target as HTMLElement).closest("button")) return;
    if (ev.key === "Enter" || ev.key === " ") {
      ev.preventDefault();
      activateTab(tabId);
    } else if (ev.key === "ArrowRight" || ev.key === "ArrowLeft") {
      ev.preventDefault();
      const tabs = [...document.querySelectorAll<HTMLElement>(".tab")];
      const i = tabs.indexOf(tab);
      if (i === -1) return;
      const next = tabs[(i + (ev.key === "ArrowRight" ? 1 : tabs.length - 1)) % tabs.length];
      next.focus();
      activateTab(Number(next.dataset.tab));
    }
  });
  return tab;
}

// ---------- terminals ----------
async function openTerminal(host: string) {
  const tabId = nextTab++;
  const term = new Terminal({
    cursorBlink: true,
    fontFamily: FONT_MONO,
    fontSize: 13,
    theme: {
      background: "#1a1b26",
      foreground: "#c8d3f5",
      cursor: "#7aa2f7",
      cursorAccent: "#1a1b26",
      selectionBackground: "#374465",
      selectionForeground: "#c8d3f5",
      selectionInactiveBackground: "#293046",
      // scrollbar sliders are not settable in xterm 5.5's ITheme; S-7 values
      // (#3b4261 / #6270a2 / #7aa2f7) apply when the addon support lands
      black: "#3b4261",
      red: "#f7768e",
      green: "#9ece6a",
      yellow: "#e0af68",
      blue: "#7aa2f7",
      magenta: "#bb9af7",
      cyan: "#7dcfff",
      white: "#a9b1d6",
      brightBlack: "#737aa2",
      brightRed: "#f7768e",
      brightGreen: "#9ece6a",
      brightYellow: "#e0af68",
      brightBlue: "#7aa2f7",
      brightMagenta: "#bb9af7",
      brightCyan: "#7dcfff",
      brightWhite: "#c0caf5",
    },
  });
  const fit = new FitAddon();
  term.loadAddon(fit);

  const tab = makeTab(tabId, "terminal", host);
  $("#tabs").appendChild(tab);

  // panel
  const panel = document.createElement("div");
  panel.className = "panel";
  panel.dataset.tab = String(tabId);
  panel.setAttribute("role", "tabpanel");
  const host_el = document.createElement("div");
  host_el.className = "term-host";
  panel.appendChild(host_el);
  const banner = document.createElement("div");
  banner.className = "term-banner";
  banner.innerHTML = `<span class="spinner"></span>Connecting to ${esc(host)}…`;
  panel.appendChild(banner);
  $("#panels").appendChild(panel);

  term.open(host_el);

  const session: Session = { id: tabId, ptyId: null, host, term, fit, unlisten: [] };
  sessions.set(tabId, session);
  activateTab(tabId);

  // user keystrokes -> pty
  term.onData((data) => {
    if (session.ptyId != null) invoke("pty_write", { id: session.ptyId, data });
  });

  // spawn pty
  try {
    fit.fit();
    const cols = term.cols;
    const rows = term.rows;
    const ptyId = await invoke<number>("pty_spawn", { host, cols, rows });
    session.ptyId = ptyId;

    const u1 = await listen<string>(`pty://output/${ptyId}`, (ev) => {
      banner.remove();
      session.term.write(ev.payload);
    });
    const u2 = await listen(`pty://exit/${ptyId}`, () => {
      session.term.write("\r\n\x1b[90m[session ended]\x1b[0m\r\n");
      session.ptyId = null;
    });
    session.unlisten.push(u1, u2);
  } catch (e) {
    banner.remove();
    term.write(`\r\n\x1b[31mFailed to start ssh: ${msg(e)}. Fix the host and reopen the tab.\x1b[0m\r\n`);
  }
}

function activateTab(tabId: number) {
  activeTab = tabId;
  document.querySelectorAll(".tab").forEach((t) => {
    const on = (t as HTMLElement).dataset.tab === String(tabId);
    t.classList.toggle("active", on);
    t.setAttribute("aria-selected", String(on));
  });
  document.querySelectorAll(".panel").forEach((p) => {
    p.classList.toggle("active", (p as HTMLElement).dataset.tab === String(tabId));
  });
  const tabEl = document.querySelector(`.tab[data-tab="${tabId}"]`) as HTMLElement | null;
  tabEl?.scrollIntoView({ inline: "nearest" });
  const s = sessions.get(tabId);
  if (s) {
    setTimeout(() => {
      s.fit.fit();
      s.term.focus();
      if (s.ptyId != null) invoke("pty_resize", { id: s.ptyId, cols: s.term.cols, rows: s.term.rows });
    }, 0);
  }
}

async function closeTab(tabId: number) {
  const s = sessions.get(tabId);
  if (s) {
    if (s.ptyId != null) await invoke("pty_kill", { id: s.ptyId }).catch(() => {});
    s.unlisten.forEach((u) => u());
    s.term.dispose();
    sessions.delete(tabId);
  } else if (editorSessions.has(tabId)) {
    const ed = editorSessions.get(tabId) as EditorSession;
    if (ed.dirty) {
      const choice = await dialog(
        "Unsaved changes",
        `${baseName(ed.path)} has unsaved changes.`,
        [
          { label: "Save and close", id: "save", style: "primary" },
          { label: "Discard changes", id: "discard", style: "danger" },
          { label: "Cancel", id: "cancel" },
        ],
        "save",
        "cancel"
      );
      if (choice === "cancel") return;
      if (choice === "save") {
        await ed.save();
        if (ed.dirty) return; // save failed; keep the tab open
      }
    }
    ed.view?.destroy();
    editorSessions.delete(tabId);
  } else if (sftpSessions.has(tabId)) {
    sftpSessions.delete(tabId);
  } else {
    return;
  }
  document.querySelector(`.tab[data-tab="${tabId}"]`)?.remove();
  document.querySelector(`.panel[data-tab="${tabId}"]`)?.remove();
  if (activeTab === tabId) {
    const remaining = [...sessions.keys(), ...editorSessions.keys(), ...sftpSessions.keys()];
    if (remaining.length) activateTab(remaining[remaining.length - 1]);
    else activeTab = null;
  }
}

// resize active terminal with the window
window.addEventListener("resize", () => {
  if (activeTab != null) {
    const s = sessions.get(activeTab);
    if (s) {
      s.fit.fit();
      if (s.ptyId != null) invoke("pty_resize", { id: s.ptyId, cols: s.term.cols, rows: s.term.rows });
    }
  }
});

// ---------- wire up ----------
window.addEventListener("DOMContentLoaded", () => {
  $("#btn-add").addEventListener("click", () => openEditor(null));
  $("#btn-reload").addEventListener("click", loadHosts);
  $("#btn-raw").addEventListener("click", openRaw);
  $("#m-cancel").addEventListener("click", cancelEditor);
  $("#m-save").addEventListener("click", saveHost);
  $("#m-delete").addEventListener("click", deleteHost);
  $("#raw-cancel").addEventListener("click", cancelRaw);
  $("#raw-save").addEventListener("click", saveRaw);
  $("#f-auth").addEventListener("change", (e) => applyAuthMode((e.target as HTMLSelectElement).value));
  $("#f-newkey").addEventListener("click", openKeyModal);
  $("#k-cancel").addEventListener("click", closeKey);
  $("#k-save").addEventListener("click", saveKey);
  loadHosts();
});
