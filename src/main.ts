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
  has_password: boolean;
  options: [string, string][];
}

// xterm cannot read CSS custom properties; keep this identical to --font-mono in styles.css
const FONT_MONO = '"JetBrains Mono", "Geist Mono", "Cascadia Code", "SF Mono", ui-monospace, "DejaVu Sans Mono", monospace';

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
  lock: svg('<rect x="3.5" y="7" width="9" height="6.5" rx="1"/><path d="M5.5 7V5.5a2.5 2.5 0 0 1 5 0V7"/>'),
  dots: svg('<circle cx="3.5" cy="8" r="1"/><circle cx="8" cy="8" r="1"/><circle cx="12.5" cy="8" r="1"/>'),
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
  ".cm-activeLine": { backgroundColor: "var(--panel)" },
  ".cm-activeLineGutter": { backgroundColor: "var(--panel)", color: "var(--fg)" },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": { backgroundColor: "var(--sel)" },
  ".cm-matchingBracket": { outline: "1px solid var(--accent2)", backgroundColor: "transparent" },
}, { dark: true });

const cmHighlight = HighlightStyle.define([
  { tag: t.keyword, color: "#818cf8" },
  { tag: t.string, color: "#80d280" },
  { tag: t.number, color: "#ffb86c" },
  { tag: t.comment, color: "#7d7873" },
  { tag: t.variableName, color: "#f2f2f2" },
  { tag: [t.function(t.variableName)], color: "#818cf8" },
  { tag: t.typeName, color: "#2fd6a1" },
  { tag: t.propertyName, color: "#2fd6a1" },
  { tag: t.operator, color: "#b8b3b0" },
  { tag: t.punctuation, color: "#b8b3b0" },
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
  banner?: HTMLElement;
  unlisten: UnlistenFn[];
}
const sessions = new Map<number, Session>();
// second terminals created by Split (keyed by their tab)
const splitSessions = new Map<number, Session>();
// second terminals created by Split (keyed by their tab)
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
  else if (top.el.id === "ssh-modal") closeOverlay("#ssh-modal"); // Escape = Continue anyway
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

// ---------- startup ssh/sftp check + install guide (FR-W2-1..4) ----------
// Byte-exact install command; rendered AND copied from this single constant (AC-A2.1).
const PS_INSTALL_CMD = "Add-WindowsCapability -Online -Name OpenSSH.Client~~~~0.0.1.0";
type BinCheck = { ssh_missing: boolean; sftp_missing: boolean };
// Launch-lifetime detection result (FR-W2-1); null = check invoke failed → never
// intercept anything, the backend spawn error speaks for itself.
let binCheck: BinCheck | null = null;
const isWindows = navigator.userAgent.includes("Windows");

async function startupBinaryCheck() {
  try {
    binCheck = await invoke<BinCheck>("check_binaries");
    if (isWindows && (binCheck.ssh_missing || binCheck.sftp_missing)) openSshGuide();
  } catch (e) {
    console.error("check_binaries failed:", e);
  }
}

function openSshGuide() {
  if (!$("#ssh-modal").classList.contains("hidden")) return; // never stack (AC-A2.2)
  renderGuideState();
  openOverlay("#ssh-modal");
}

function renderGuideState() {
  if (!binCheck) return; // both call sites guard, but stay total (QA nit b)
  // dynamic content via textContent only (ADR-8); all static markup lives in index.html
  $("#ssh-what").textContent =
    `OpenSSH client (${[binCheck.ssh_missing && "ssh", binCheck.sftp_missing && "sftp"].filter(Boolean).join(" and ")}) not found on this machine.`;
}

// FR-W2-4: re-run detection in place — close when fixed (no restart needed),
// else update the same screen. Busy state on the button (AC-A4.3).
async function recheckBinaries() {
  const btn = $("#ssh-recheck") as HTMLButtonElement;
  btn.disabled = true;
  btn.textContent = "Re-checking…";
  try {
    binCheck = await invoke<BinCheck>("check_binaries");
    if (!binCheck.ssh_missing && !binCheck.sftp_missing) closeOverlay("#ssh-modal");
    else renderGuideState();
  } catch (e) {
    $("#ssh-what").textContent = `Re-check failed: ${msg(e)}. Fix the problem and press Re-check again.`;
  } finally {
    btn.disabled = false;
    btn.textContent = "Re-check";
  }
}

// FR-W2-3: friendly per-attempt error, shown immediately (no auto-recheck, OQ-A3).
// Returns true when the caller must not invoke the backend.
function binaryBlocked(kind: "ssh" | "sftp"): boolean {
  if (!binCheck || !(kind === "ssh" ? binCheck.ssh_missing : binCheck.sftp_missing)) return false;
  const actions = isWindows
    ? [{ label: "Open install guide", id: "guide", style: "primary" as const }, { label: "OK", id: "ok" }]
    : [{ label: "OK", id: "ok" }]; // guide content is Windows-specific (OQ-5)
  void dialog(
    "OpenSSH client missing",
    `The ${kind} binary was not found on this machine, so this action cannot run. Install the OpenSSH client, then try again.`,
    actions,
    actions[0].id,
    "ok"
  ).then((id) => {
    if (id === "guide") openSshGuide();
  });
  return true;
}

// ---------- sidebar ----------
async function loadHosts() {
  const list = $("#host-list");
  // loading = 3 static skeleton rows (--hover, pulse-free) instead of spinner-replace
  // (redesign §4.1/G2, fix #5 sidebar) — container height stays stable, no jump
  list.innerHTML = "";
  for (let i = 0; i < 3; i++) {
    const s = document.createElement("div");
    s.className = "skel";
    list.appendChild(s);
  }
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
  const q = ($("#host-search") as HTMLInputElement).value.trim().toLowerCase();
  const visible = hosts.filter(
    (h) => !h.name.includes("*") && !h.name.includes("?") && (!q || h.name.toLowerCase().includes(q))
  );
  if (hosts.length === 0) {
    const empty = document.createElement("div");
    empty.className = "list-state";
    empty.textContent = "No hosts in ~/.ssh/config yet. Click + to add one.";
    const add = document.createElement("button");
    add.textContent = "Add host";
    add.addEventListener("click", () => openEditor(null));
    empty.appendChild(add);
    list.appendChild(empty);
    return;
  }
  if (visible.length === 0) {
    const none = document.createElement("div");
    none.className = "list-state";
    none.textContent = `No host matches "${q}".`;
    list.appendChild(none);
    return;
  }
  // Single-line 28px rows (redesign §4.1): dot + name + (lock) + hover-only
  // ⇅ / ⋯ buttons. Edit/Delete left the row -> context menu (fix #4).
  let group = "";
  for (const h of visible) {
    const g = h.name.includes("-") ? h.name.slice(0, h.name.indexOf("-")) : h.name;
    if (g !== group) {
      group = g;
      const head = document.createElement("div");
      head.className = "h-group";
      head.textContent = g.toUpperCase();
      list.appendChild(head);
    }
    const el = document.createElement("div");
    el.className = "host-item";
    el.dataset.host = h.name;
    el.setAttribute("role", "button");
    el.tabIndex = 0;
    el.setAttribute("aria-label", `Open terminal to ${h.name}`);
    const sub = [h.user && `${h.user}@`, h.host_name || "(no HostName)", h.port && `:${h.port}`]
      .filter(Boolean)
      .join("");
    el.title = sub; // subtitle as tooltip, not a second line
    el.innerHTML =
      `<span class="h-dot${hostHasLiveSession(h.name) ? " live" : ""}"></span>` +
      `<span class="h-name">${esc(h.name)}</span>` +
      (h.has_password ? `<span class="h-key" title="Password stored (sidecar, 0600)">${ICONS.lock}</span>` : "") +
      `<span class="h-acts">` +
      `<button class="h-sftp icon" title="Open SFTP" aria-label="Open SFTP for ${escAttr(h.name)}">${ICONS.arrows}</button>` +
      `<button class="h-more icon" title="More actions (or right-click)" aria-label="More actions for ${escAttr(h.name)}">${ICONS.dots}</button>` +
      `</span>`;
    (el.querySelector(".h-more") as HTMLElement).addEventListener("click", (ev) => {
      ev.stopPropagation();
      const r = (ev.currentTarget as HTMLElement).getBoundingClientRect();
      showHostMenu({ clientX: r.left, clientY: r.bottom + 4 }, h.name);
    });
    el.addEventListener("click", (ev) => {
      const t = ev.target as HTMLElement;
      if (t.closest(".h-sftp")) openSftp(h.name);
      else if (t.closest(".h-more")) { /* handled by the button's own listener */ }
      else openTerminal(h.name);
    });
    el.addEventListener("contextmenu", (ev) => {
      ev.preventDefault();
      ev.stopPropagation();
      showHostMenu(ev as MouseEvent, h.name);
    });
    el.addEventListener("keydown", (ev) => {
      if ((ev.target as HTMLElement).closest("button")) return;
      if (ev.key === "Enter" || ev.key === " ") {
        ev.preventDefault();
        openTerminal(h.name);
      } else if (ev.key === "ArrowDown" || ev.key === "ArrowUp") {
        ev.preventDefault();
        moveRowFocus(el, ev.key === "ArrowDown" ? 1 : -1);
      }
    });
    list.appendChild(el);
  }
}

// live-session dot: --ok while the host has a terminal tab with a running PTY
function hostHasLiveSession(name: string): boolean {
  for (const s of sessions.values()) if (s.host === name && s.ptyId != null) return true;
  for (const s of splitSessions.values()) if (s.host === name && s.ptyId != null) return true;
  return false;
}

// toggle dot classes in place — no list rebuild, focus survives
function refreshHostDots() {
  document.querySelectorAll<HTMLElement>(".host-item[data-host]").forEach((el) => {
    el.querySelector(".h-dot")?.classList.toggle("live", hostHasLiveSession(el.dataset.host as string));
  });
}

// app version for the status bar; keep in sync with package.json / tauri.conf.json
const APP_VERSION = "v0.10.0";

// status bar updater (§4.3): kiri "user@host · auth:key|pw · N sesi" dari
// sesi aktif, kanan indikator transfer + versi. Dipanggil dari activateTab
// (semua jenis tab), openTerminal/splitTerminal/closeTab dan busy() SFTP.
function updateStatusBar() {
  const id = activeTab;
  const hostName =
    sessions.get(id as number)?.host ??
    splitSessions.get(id as number)?.host ??
    sftpSessions.get(id as number)?.host ??
    editorSessions.get(id as number)?.host;
  const h = hostName ? hosts.find((x) => x.name === hostName) : undefined;
  const n = sessions.size + splitSessions.size;
  const left: string[] = [];
  if (hostName) left.push(`${h?.user ?? "user"}@${hostName} · auth:${h?.has_password ? "pw" : "key"}`);
  left.push(`${n} sesi`);
  const el = document.getElementById("sb-left");
  if (el) el.textContent = left.join(" · ");
  const xfer = document.getElementById("sb-xfer");
  if (xfer) xfer.innerHTML = document.querySelectorAll(".xfer-bar.on").length > 0 ? '<span class="spinner"></span>⇅ transfer' : "⇅ idle";
}

function moveRowFocus(row: HTMLElement, dir: 1 | -1) {
  const rows = [...document.querySelectorAll<HTMLElement>(".host-item")];
  rows[rows.indexOf(row) + dir]?.focus();
}

// Host row context menu (redesign §4.1, fix #4): Open SFTP · Edit · Delete.
// Delete reuses the in-app confirm dialog, never native confirm.
function showHostMenu(at: { clientX: number; clientY: number }, name: string) {
  document.getElementById("host-menu")?.remove();
  const menu = document.createElement("div");
  menu.id = "host-menu";
  menu.setAttribute("role", "menu");
  const h = hosts.find((x) => x.name === name);
  const items: Array<[string, boolean, () => void]> = [
    ["Open SFTP", false, () => void openSftp(name)],
    ["Edit", false, () => { if (h) openEditor(h); }],
    ["Delete", true, () => void deleteHostByName(name)],
  ];
  for (const [label, danger, fn] of items) {
    const b = document.createElement("button");
    b.setAttribute("role", "menuitem");
    if (danger) b.className = "danger";
    b.textContent = label;
    b.addEventListener("click", () => { menu.remove(); fn(); });
    menu.appendChild(b);
  }
  document.body.appendChild(menu);
  const mw = menu.offsetWidth, mh = menu.offsetHeight;
  menu.style.left = Math.min(at.clientX, window.innerWidth - mw - 8) + "px";
  menu.style.top = Math.min(at.clientY, window.innerHeight - mh - 8) + "px";
  (menu.querySelector("button") as HTMLElement | null)?.focus();
  const close = () => { menu.remove(); document.removeEventListener("mousedown", onDown, true); };
  const onDown = (e: MouseEvent) => { if (!menu.contains(e.target as Node)) close(); };
  setTimeout(() => document.addEventListener("mousedown", onDown, true), 0);
  menu.addEventListener("keydown", (e) => { if (e.key === "Escape") close(); });
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

function applyAuthMode(method: string, hasPassword = false) {
  $("#row-identity").classList.toggle("hidden", method !== "key");
  $("#row-password").classList.toggle("hidden", method !== "password");
  const note = $("#row-password-note");
  const pw = $("#f-password") as HTMLInputElement;
  // U8 D2: badge chip + "Hapus password tersimpan" only while a password is stored
  $("#pw-stored-row").classList.toggle("hidden", !(method === "password" && hasPassword));
  // U8 D3 (PRD Q3): auto-typing is per-host opt-in — exactly while auth=password
  $("#row-password-note-d3").classList.toggle("hidden", method !== "password");
  if (method === "password") {
    note.textContent = hasPassword
      ? "A password is stored for this host. Leave the field empty to keep it; type to replace it."
      : "The password is stored in ~/.ssh/hosterm.secrets (file perms 0600), never in ~/.ssh/config, and filled in automatically at connect.";
    pw.placeholder = hasPassword ? "kept — type to replace" : "stored in ~/.ssh/hosterm.secrets";
  }
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
  applyAuthMode(method, h?.has_password ?? false);
  ($("#f-password") as HTMLInputElement).value = "";
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
    password: method === "password" ? ($("#f-password") as HTMLInputElement).value : "",
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

async function deleteHostByName(name: string) {
  const choice = await dialog(
    "Delete host",
    `Delete host "${name}" from ~/.ssh/config? This cannot be undone.`,
    [
      { label: "Delete", id: "delete", style: "danger" },
      { label: "Cancel", id: "cancel" },
    ],
    "cancel",
    "cancel"
  );
  if (choice !== "delete") return;
  try {
    await invoke("delete_host", { name });
    sftpHostCache.delete(name);
    await loadHosts();
  } catch (e) {
    await dialog(
      "Could not delete host",
      `Failed to delete host: ${msg(e)}. Check the config, then try again.`,
      [{ label: "OK", id: "ok" }],
      "ok",
      "ok"
    );
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
    sftpHostCache.delete(editingName);
    closeEditor();
    await loadHosts();
  } catch (e) {
    $("#modal-err").textContent = `Failed to delete host: ${msg(e)}. Fix the problem and press Delete again.`;
  }
}

// U8 D2: forget the stored sidecar password for the host being edited.
// Reuses the sidecar-forget path (`set_secret(alias, "")` semantics); the
// modal stays open — chip drops, note reverts to the empty-state version.
async function deleteHostPassword() {
  if (!editingName) return;
  const choice = await dialog(
    "Hapus password tersimpan",
    `Remove the stored password for "${editingName}" from ~/.ssh/hosterm.secrets? This cannot be undone.`,
    [
      { label: "Hapus", id: "delete", style: "danger" },
      { label: "Cancel", id: "cancel" },
    ],
    "cancel",
    "cancel"
  );
  if (choice !== "delete") return;
  try {
    await invoke("forget_secret", { alias: editingName });
    const h = hosts.find((x) => x.name === editingName);
    if (h) h.has_password = false;
    renderHostList(); // sidebar lock icon drops with the sidecar entry
    applyAuthMode(($("#f-auth") as HTMLSelectElement).value, false);
    $("#modal-err").textContent = "";
  } catch (e) {
    $("#modal-err").textContent = `Failed to remove stored password: ${msg(e)}. Fix the problem and press Hapus again.`;
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
// unlisten: teardown for the window-level drag listener (AbortController, U4
// invariant) + pending strip fade timers — closeTab must run it (SEC LOW t_fb1d18d6).
type SftpSession = { host: string; path: string; unlisten: (() => void)[] };
// Entries cached per host so N tabs on one host share one listing; invalidated
// on tab close (last tab of that host drops the cache). Rendering re-fetches
// via the persistent backend session — no extra login per tab or per refresh.
const sftpHostCache = new Map<string, SftpEntry[]>();
function sftpForgetHost(host: string) {
  if (![...sftpSessions.values()].some((s) => s.host === host)) sftpHostCache.delete(host);
}
type SftpEntry = { name: string; is_dir: boolean; is_link: boolean; size: number };
type LocalEntry = { name: string; is_dir: boolean; size: number };
const sftpSessions = new Map<number, SftpSession>();

// Local pane state per sftp tab (browser can't list dirs itself).
const localPane = new Map<number, string>(); // tabId -> local cwd

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

function joinLocal(dir: string, name: string): string {
  const d = dir.replace(/[\\/]+$/, "");
  if (!name) return d;
  return d ? `${d}/${name}` : name;
}

// U5 §4.4 F: single pending source per SFTP panel ("sumber"). Click-click
// stage 1 sets it; stage 2 (pick in the opposite pane) or a mouse drag drop
// consumes it. Stored on the panel so the module-level render fns can paint
// .sel + read it without threading state through every call site.
type SftpSide = "local" | "remote";
type SftpSel = { side: SftpSide; name: string; size: number };
const sftpSelOf = (panel: HTMLElement): SftpSel | null =>
  ((panel as any).__sel as SftpSel | undefined) || null;
const sftpSelIs = (panel: HTMLElement, side: SftpSide, name: string): boolean => {
  const s = sftpSelOf(panel);
  return !!s && s.side === side && s.name === name;
};

type StatusFn = (m: string, kind?: "" | "ok" | "err") => void;

async function openSftp(host: string) {
  if (binaryBlocked("sftp")) return;
  const tabId = nextTab++;
  sftpSessions.set(tabId, { host, path: "", unlisten: [] });
  const tab = makeTab(tabId, "sftp", host);
  $("#tab-split-zone").before(tab);

  const panel = document.createElement("div");
  panel.className = "panel";
  panel.dataset.tab = String(tabId);
  panel.setAttribute("role", "tabpanel");
  panel.innerHTML = `
    <div class="sftp-wrap">
      <div class="sftp-panes">
        <div class="sftp-pane">
          <div class="pane-head">
            <span class="pane-title">This PC — local</span><span class="chip"></span>
            <span class="ph-acts"><button class="sftp-download" title="Download ke pane ini — pilih file remote dulu, atau Ctrl+Enter">◂ Download</button></span>
          </div>
          <div class="sftp-toolbar">
            <button class="loc-up icon" aria-label="Parent local directory" title="Parent directory">${ICONS.up}</button>
            <input class="loc-path" placeholder="local directory" />
            <button class="loc-go">Go</button>
            <button class="loc-refresh icon" aria-label="Reload local directory" title="Reload directory">${ICONS.reload}</button>
          </div>
          <div class="loc-list"></div>
        </div>
        <div class="sftp-pane">
          <div class="pane-head">
            <span class="pane-title">${esc(host)} — remote</span><span class="chip"></span>
            <span class="ph-acts"><button class="sftp-upload" title="Upload ke pane ini — pilih file lokal dulu, atau Ctrl+Enter">Upload ▸</button></span>
          </div>
          <div class="sftp-toolbar">
            <button class="sftp-up icon" aria-label="Parent directory" title="Parent directory">${ICONS.up}</button>
            <input class="sftp-path" placeholder="remote path (empty = home)" />
            <button class="sftp-go">Go</button>
            <button class="sftp-refresh icon" aria-label="Reload directory" title="Reload directory">${ICONS.reload}</button>
          </div>
          <div class="sftp-list"></div>
        </div>
      </div>
      <div class="xfer-bar"></div>
      <div class="xfer-strip"><div class="xs-rows"></div><div class="sftp-status"></div></div>
      <div class="sftp-hint">Pilih file → klik pane tujuan / drag manual mouse — HTML5 DnD tidak dipakai</div>
    </div>`;
  $("#panels").appendChild(panel);

  const q = (sel: string) => panel.querySelector(sel) as HTMLElement;
  const inp = q(".sftp-path") as HTMLInputElement;
  const locinp = q(".loc-path") as HTMLInputElement;
  const status: StatusFn = (m, kind = "") => {
    const el = q(".sftp-status");
    el.textContent = m;
    el.classList.toggle("ok", kind === "ok");
    el.classList.toggle("err", kind === "err");
  };
  let xferN = 0; // concurrent transfers (drag one, then another) — counter, not flag
  const busy = (on: boolean) => {
    xferN = Math.max(0, xferN + (on ? 1 : -1));
    q(".xfer-bar").classList.toggle("on", xferN > 0);
    (q(".sftp-upload") as HTMLButtonElement).disabled = xferN > 0;
    (q(".sftp-download") as HTMLButtonElement).disabled = xferN > 0;
    updateStatusBar(); // indikator transfer di status bar global (§4.3)
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

  const goLocal = () => {
    localPane.set(tabId, locinp.value.trim());
    renderLocalPane(tabId, status);
  };
  locinp.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") goLocal();
  });
  q(".loc-go").addEventListener("click", goLocal);
  q(".loc-up").addEventListener("click", () => {
    const cur = localPane.get(tabId) || "";
    const parent = cur.replace(/[\\/][^\\/]*$/, "");
    localPane.set(tabId, parent || cur);
    const s2 = sftpSessions.get(tabId);
    locinp.value = localPane.get(tabId) || "";
    if (s2) renderLocalPane(tabId, status);
  });
  q(".loc-refresh").addEventListener("click", () => renderLocalPane(tabId, status));

  // ---- U5 §4.4: sumber/target chips + per-file transfer strip + 3 transfer paths ----
  const chipEls = {
    local: panel.querySelector(".sftp-pane:nth-child(1) .chip") as HTMLElement,
    remote: panel.querySelector(".sftp-pane:nth-child(2) .chip") as HTMLElement,
  };
  let sel: SftpSel | null = null;
  const setSel = (s: SftpSel | null) => {
    sel = s;
    (panel as any).__sel = s;
    (["local", "remote"] as SftpSide[]).forEach((side) => {
      const el = chipEls[side];
      el.classList.remove("src", "dst");
      el.textContent = "";
      if (!sel) return;
      if (sel.side === side) {
        el.classList.add("src");
        el.textContent = `▲ sumber: ${sel.name}`;
      } else {
        el.classList.add("dst");
        el.textContent = `▼ target: drop di sini / klik file lalu ${sel.side === "local" ? "Upload ▸" : "◂ Download"}`;
      }
    });
  };
  (panel as any).__clearSel = () => setSel(null);
  // A list re-render only invalidates a sumber that lived in THAT pane.
  (panel as any).__clearSelIf = (side: SftpSide) => {
    if (sel && sel.side === side) setSel(null);
  };

  // Transfer strip (§4.4): one row per file, "⇅ name → path · size · state".
  // ok rows auto-fade after 4s; failed rows persist with retry (AC5). Timers
  // tracked so tab close cancels pending fades.
  const rowsEl = panel.querySelector(".xs-rows") as HTMLElement;
  const fadeTimers = new Set<number>();
  const addXferRow = (
    dir: "up" | "down",
    name: string,
    path: string,
    size: number,
    retry: () => void,
  ) => {
    const row = document.createElement("div");
    row.className = "xf-row run";
    const ico = document.createElement("span");
    ico.className = "xf-ico";
    ico.setAttribute("aria-hidden", "true");
    ico.textContent = "⇅";
    const nm = document.createElement("span");
    nm.className = "xf-name";
    nm.textContent = name;
    const pth = document.createElement("span");
    pth.className = "xf-path";
    pth.textContent = `${dir === "up" ? "→" : "←"} ${path}`;
    const sz = document.createElement("span");
    sz.className = "xf-size";
    sz.textContent = size >= 0 ? fmtSize(size) : "";
    const st = document.createElement("span");
    st.className = "xf-state";
    row.append(ico, nm, pth, sz, st);
    rowsEl.appendChild(row);
    const later = (fn: () => void, ms: number) => {
      const t = window.setTimeout(() => {
        fadeTimers.delete(t);
        fn();
      }, ms);
      fadeTimers.add(t);
    };
    return {
      row,
      run(label: string) {
        const spin = document.createElement("span");
        spin.className = "spinner";
        spin.setAttribute("aria-hidden", "true");
        st.replaceChildren(spin, document.createTextNode(label));
      },
      ok() {
        row.classList.remove("run");
        row.classList.add("ok");
        st.textContent = "ok";
        later(() => {
          row.classList.add("out");
          later(() => row.remove(), 320);
        }, 4000);
      },
      fail(detail: string) {
        row.classList.remove("run");
        row.classList.add("fail");
        row.title = detail;
        const btn = document.createElement("button");
        btn.className = "xf-retry";
        btn.textContent = "coba lagi";
        // retry replaces its own row (fresh run row), else the strip would
        // keep the dead row forever (§4.4 max-3 + scroll cap).
        btn.addEventListener("click", () => {
          row.remove();
          retry();
        });
        st.replaceChildren(document.createTextNode("gagal"), btn);
      },
    };
  };

  // One transfer engine — all three paths (manual drag, click-click, head
  // buttons) route here. Narrative copies byte-identical to pre-U5 baseline,
  // only their home moved to the strip note.
  const transfer = (src: SftpSide, name: string, size: number, intoDir = "") => {
    const s = sftpSessions.get(tabId) as SftpSession;
    if (src === "local") {
      if (!name) return status("Select a file in the local pane first (single click).", "err");
      const localPath = joinLocal(localPane.get(tabId) || "", name);
      const remotePath = joinRemote(intoDir ? joinRemote(s.path, intoDir) : s.path, name);
      const job = () => {
        const r = addXferRow("up", name, `${host}:${remotePath}`, size, job);
        r.run("berjalan");
        busy(true);
        status(`Uploading ${name}…`);
        invoke("sftp_upload", { host, localPath, remotePath })
          .then(() => {
            r.ok();
            status(`Uploaded to ${remotePath}`, "ok");
            setSel(null);
            renderSftp(tabId, status);
          })
          .catch((e) => {
            const m = `Failed to upload ${name}: ${msg(e)}. Check the path and try again.`;
            r.fail(m);
            status(m, "err");
          })
          .finally(() => busy(false));
      };
      job();
      return;
    }
    if (!name) return status("Select a file in the remote pane first (single click).", "err");
    const localDir = localPane.get(tabId) || "";
    if (!localDir) return status("Open a local directory on the left first.", "err");
    const remotePath = joinRemote(s.path, name);
    const localPath = joinLocal(joinLocal(localDir, intoDir), name);
    const job = () => {
      const r = addXferRow("down", name, `${host}:${remotePath}`, size, job);
      r.run("berjalan");
      busy(true);
      status(`Downloading ${name}…`);
      invoke("sftp_download", { host, remotePath, localPath })
        .then(() => {
          r.ok();
          status(`Saved to ${localPath}`, "ok");
          setSel(null);
          renderLocalPane(tabId, status);
        })
        .catch((e) => {
          const m = `Failed to download ${name}: ${msg(e)}. Check the path and try again.`;
          r.fail(m);
          status(m, "err");
        })
        .finally(() => busy(false));
    };
    job();
  };

  // Click-click path (VSCode-style): stage 1 pick file = sumber; stage 2 pick
  // in the opposite pane = target (dir row → into that dir). Stage-1 clicks
  // never open the editor — they only travel the selection path (pitfall U5).
  const pickSource = (side: SftpSide, e: { name: string; is_dir: boolean; size: number }) => {
    if (e.is_dir) return setSel(null);
    setSel({ side, name: e.name, size: e.size });
  };
  (panel as any).__pickSource = pickSource;
  (panel as any).__onRowPick = (side: SftpSide, e: { name: string; is_dir: boolean; size: number }) => {
    if (sel && sel.side !== side) return transfer(sel.side, sel.name, sel.size, e.is_dir ? e.name : "");
    pickSource(side, e);
    if (!e.is_dir) status(side === "local" ? `Selected ${e.name} for upload.` : `Selected ${e.name} for download.`);
  };
  (panel as any).__onDrop = (side: SftpSide, intoDir: string, from: SftpSide, name: string, size: number) => {
    if (from === side || !name) return;
    transfer(from, name, size, intoDir);
  };
  (panel as any).__xferFrom = (side: SftpSide) => {
    if (sel) return transfer(sel.side, sel.name, sel.size);
    status(side === "local" ? "Select a file in the local pane first (single click)." : "Select a file in the remote pane first (single click).", "err");
  };

  // Head buttons live in the TARGET pane head (Upload → remote head, ◂
  // Download → local head) — §4.4 F; no transfer controls left in a foot.
  q(".sftp-upload").addEventListener("click", () => {
    if (sel && sel.side === "local") transfer("local", sel.name, sel.size);
    else status("Select a file in the local pane first (single click).", "err");
  });
  q(".sftp-download").addEventListener("click", () => {
    if (sel && sel.side === "remote") transfer("remote", sel.name, sel.size);
    else status("Select a file in the remote pane first (single click).", "err");
  });

  // Window-level mouseup clears a drag released anywhere (even off-panel).
  // AbortController teardown on tab close — U4 listener-registry invariant.
  const dragAc = new AbortController();
  window.addEventListener("mouseup", clearSftpDrag, { signal: dragAc.signal });
  (sftpSessions.get(tabId) as SftpSession).unlisten.push(() => {
    dragAc.abort();
    fadeTimers.forEach(clearTimeout);
    fadeTimers.clear();
    clearSftpDrag();
  });

  // drag & drop between panes (Termius): rows from the LOCAL pane drop on the
  // REMOTE pane = upload; rows from REMOTE drop on LOCAL = download. Manual
  // mouse events only — no HTML5 DnD (STATE.md invariant: unreliable in
  // WebView2; proven dead). The window-level mouseup safety-net lives in the
  // wiring above (AbortController, torn down on tab close).
  wirePane(q(".sftp-pane") as HTMLElement, "local");
  wirePane(q(".sftp-pane:nth-child(2)") as HTMLElement, "remote");

  // default local pane: user home (frontend knows it from the backend)
  try {
    localPane.set(tabId, await invoke<string>("home_dir"));
    locinp.value = localPane.get(tabId) || "";
  } catch {
    localPane.set(tabId, "");
  }
  activateTab(tabId);
  renderSftp(tabId, status);
  renderLocalPane(tabId, status);
}

// Manual mouse drag between SFTP panes (STATE.md invariant: HTML5 DnD is
// banned — dead in WebView2). One drag can be live at a time, so the grab
// state is module-level; the window mouseup net + pane mouseup call
// clearSftpDrag, which is also the AbortController-listener target.
let sftpDragSide: SftpSide | null = null;
let sftpDragName = "";
let sftpDragSize = -1;

function clearSftpDrag() {
  sftpDragSide = null;
  sftpDragName = "";
  sftpDragSize = -1;
  document.body.classList.remove("sftp-dragging");
  document.querySelectorAll(".sftp-pane.drop-target").forEach((p) => p.classList.remove("drop-target"));
}

function wirePane(pane: HTMLElement, side: SftpSide) {
  const panel = pane.closest(".panel") as HTMLElement;
  pane.addEventListener("mousedown", (ev) => {
    const row = (ev.target as HTMLElement).closest(".sftp-row") as HTMLElement | null;
    if (!row || ev.button !== 0) return;
    sftpDragSide = side;
    sftpDragName = row.dataset.name || "";
    sftpDragSize = Number(row.dataset.size ?? "-1");
    document.body.classList.add("sftp-dragging");
    // Opposite (target) pane gets the dashed drop outline for the whole drag
    // (§4.4 F); mousedown arms sumber so chips light up before the release.
    panel.querySelectorAll(".sftp-pane.drop-target").forEach((p) => p.classList.remove("drop-target"));
    panel.querySelector(side === "local" ? ".sftp-pane:nth-child(2)" : ".sftp-pane:nth-child(1)")?.classList.add("drop-target");
    // Arm sumber ONLY when no opposite-pane source is pending — otherwise the
    // mousedown of a stage-2 click-click would overwrite the source and the
    // transfer never fires (pitfall: stage-2 pick must reach __onRowPick).
    const cur = sftpSelOf(panel);
    if (!row.dataset.dir && (!cur || cur.side === side)) (panel as any).__pickSource?.(side, { name: sftpDragName, is_dir: false, size: sftpDragSize });
  });
  pane.addEventListener("mouseup", (ev) => {
    if (!sftpDragSide) return;
    const from = sftpDragSide;
    const name = sftpDragName;
    const size = sftpDragSize;
    clearSftpDrag();
    if (from === side || !name) return; // own pane = plain click
    const row = (ev.target as HTMLElement).closest(".sftp-row") as HTMLElement | null;
    const into = row && row.dataset.dir === "1" ? row.dataset.name || "" : "";
    (panel as any).__onDrop?.(side, into, from, name, size);
  });
  // Keyboard path (§4.4): Ctrl+Enter transfers the pending selection;
  // Tab from a row jumps focus to the opposite pane.
  pane.addEventListener("keydown", (ev) => {
    if (ev.target instanceof HTMLElement && ev.target.closest("input")) return;
    if ((ev.ctrlKey || ev.metaKey) && ev.key === "Enter") {
      ev.preventDefault();
      (panel as any).__xferFrom?.(side);
    } else if (ev.key === "Tab" && (ev.target as HTMLElement)?.closest?.(".sftp-row")) {
      ev.preventDefault();
      const other = panel.querySelector(side === "local" ? ".sftp-pane:nth-child(2)" : ".sftp-pane:nth-child(1)");
      (other?.querySelector(".sftp-row") as HTMLElement | null)?.focus();
    }
  });
}

function rowFor(
  list: HTMLElement,
  e: { name: string; is_dir: boolean; is_link?: boolean; size: number },
  onOpen: () => void,
  onSelect: () => void,
  isSelected: () => boolean
) {
  const row = document.createElement("div");
  row.className = "sftp-row";
  // dataset carries the entry to the drag path — rows are rebuilt on every
  // render, so the grab state must survive via attributes, not closures.
  row.dataset.name = e.name;
  row.dataset.size = String(e.size);
  if (e.is_dir) row.dataset.dir = "1";
  if (e.is_link) row.dataset.link = "1";
  row.setAttribute("role", "button");
  row.tabIndex = 0;
  // AC1: .sel paints on the CLICKED row (old code highlighted last-child —
  // wrong row when clicking mid-list), and only from the shared sumber state.
  const paint = () => {
    const on = isSelected();
    list.querySelectorAll(".sftp-row.sel").forEach((r) => r.classList.remove("sel"));
    row.classList.toggle("sel", on);
    if (on) row.setAttribute("aria-selected", "true");
    else row.removeAttribute("aria-selected");
  };
  if (isSelected()) paint();
  const ico = e.is_dir ? ICONS.folder : e.is_link ? ICONS.link : ICONS.file;
  row.innerHTML =
    `<span class="f-ico${e.is_dir ? " dir" : ""}"${e.is_link ? ' title="Symbolic link"' : ""}>${ico}</span>` +
    `<span class="nm">${esc(e.name)}</span>` +
    `<span class="sz">${e.is_dir ? "" : fmtSize(e.size)}</span>`;
  let clickTimer: number | null = null;
  row.addEventListener("click", () => {
    if (clickTimer != null) return; // the dblclick handler owns it
    clickTimer = window.setTimeout(() => {
      clickTimer = null;
      onSelect();
      paint();
    }, 250);
  });
  row.addEventListener("dblclick", () => {
    if (clickTimer != null) {
      clearTimeout(clickTimer);
      clickTimer = null;
    }
    onOpen();
  });
  row.addEventListener("keydown", (ev) => {
    // Ctrl/Cmd+Enter belongs to the transfer path (wirePane), not open-editor
    if (ev.key === "Enter" && !ev.ctrlKey && !ev.metaKey) onOpen();
    else if (ev.key === " ") {
      // Space = select (spec: double-click/Enter opens, no hidden click dbl)
      ev.preventDefault();
      onSelect();
      paint();
    }
  });
  list.appendChild(row);
}

// Loading state = skeleton rows (U2 .skel pattern, §4.4 — no spinner text).
function skelRows(list: HTMLElement, n = 3) {
  list.replaceChildren(
    ...Array.from({ length: n }, () => {
      const d = document.createElement("div");
      d.className = "skel";
      return d;
    })
  );
}

async function renderLocalPane(tabId: number, status: StatusFn) {
  const panel = document.querySelector(`.panel[data-tab="${tabId}"]`) as HTMLElement;
  if (!panel) return;
  const list = panel.querySelector(".loc-list") as HTMLElement;
  const dir = localPane.get(tabId) || "";
  (panel as any).__clearSelIf?.("local");
  skelRows(list);
  let entries: LocalEntry[];
  try {
    entries = await invoke<LocalEntry[]>("local_list", { path: dir });
  } catch (err) {
    list.innerHTML = "";
    status(`Failed to read ${dir || "home"}: ${msg(err)}. Fix the path and press Go.`, "err");
    return;
  }
  list.innerHTML = "";
  if (entries.length === 0) list.innerHTML = `<div class="list-state">Empty directory.</div>`;
  for (const e of entries) {
    rowFor(
      list,
      e,
      () => {
        if (e.is_dir) {
          const next = dir.replace(/[\\/]+$/, "") + "/" + e.name;
          localPane.set(tabId, next);
          const inp = panel.querySelector(".loc-path") as HTMLInputElement;
          inp.value = next;
          renderLocalPane(tabId, status);
        } else {
          status(`Selected ${e.name} for upload.`, "");
        }
      },
      () => (panel as any).__onRowPick?.("local", e),
      () => sftpSelIs(panel, "local", e.name)
    );
  }
  status(`${entries.length} local items · ${dir || "~"}`);
}

async function renderSftp(tabId: number, status: StatusFn) {
  const s = sftpSessions.get(tabId);
  if (!s) return;
  const panel = document.querySelector(`.panel[data-tab="${tabId}"]`) as HTMLElement;
  const list = panel.querySelector(".sftp-list") as HTMLElement;
  const inp = panel.querySelector(".sftp-path") as HTMLInputElement;
  inp.value = s.path;
  (panel as any).__clearSelIf?.("remote");
  skelRows(list);
  let entries: SftpEntry[];
  try {
    entries = await invoke<SftpEntry[]>("sftp_list", { host: s.host, path: s.path });
    sftpHostCache.set(s.host, entries);
  } catch (err) {
    // stale persistent session (server timeout etc.): backend respawns it;
    // one automatic retry before showing the error
    try {
      entries = await invoke<SftpEntry[]>("sftp_list", { host: s.host, path: s.path });
      sftpHostCache.set(s.host, entries);
    } catch (err2) {
      list.innerHTML = "";
      status(`Failed to list ${s.path || "~"}: ${msg(err2)}. Fix the path and press Go, or go up one level.`, "err");
      return;
    }
  }
  list.innerHTML = "";
  if (entries.length === 0) {
    list.innerHTML = `<div class="list-state">Empty directory.</div>`;
  }
  for (const e of entries) {
    rowFor(
      list,
      e,
      () => {
        if (e.is_dir) {
          s.path = joinRemote(s.path, e.name);
          renderSftp(tabId, status);
        } else {
          openRemoteEditor(s.host, joinRemote(s.path, e.name));
        }
      },
      () => (panel as any).__onRowPick?.("remote", e),
      () => sftpSelIs(panel, "remote", e.name)
    );
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
    // U7 E2: mirror the tab-dot dirty signal into the ed-bar (redesign §4.5) so the
    // second glance catches it — the dot alone was the missed "detak kedua".
    (panel.querySelector(".ed-dirty") as HTMLElement).classList.toggle("on", sess.dirty);
  };

  $("#tab-split-zone").before(tab);

  const panel = document.createElement("div");
  panel.className = "panel";
  panel.dataset.tab = String(tabId);
  panel.setAttribute("role", "tabpanel");
  panel.innerHTML = `
    <div class="ed-wrap">
      <div class="ed-bar">
        <span class="ed-path" title="${escAttr(path)}">${esc(host)}:${esc(path)}</span>
        <span class="ed-dirty">● unsaved</span>
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
  const reload = async () => {
    try {
      const text = await invoke<string>("sftp_read_file", { host, remotePath: path });
      sess.view.dispatch({ changes: { from: 0, to: sess.view.state.doc.length, insert: text } });
      sess.dirty = false;
      setTitle();
      status("Reloaded from server", "ok");
    } catch (e) {
      status(`Failed to reload: ${msg(e)}. Check the file exists, then press Reload again.`, "err");
    }
  };
  // U7 E2: Reload on a dirty buffer goes through the SAME in-app dirty dialog as
  // close (silent overwrite was the harm redesign §1.5 E2 names).
  (q(".ed-reload") as HTMLButtonElement).addEventListener("click", async () => {
    if (sess.dirty) {
      const choice = await dialog(
        "Unsaved changes",
        `${baseName(path)} has unsaved changes.`,
        [
          { label: "Save and reload", id: "save", style: "primary" },
          { label: "Discard changes", id: "discard", style: "danger" },
          { label: "Cancel", id: "cancel" },
        ],
        "save",
        "cancel"
      );
      if (choice === "cancel") return;
      if (choice === "save") {
        await save();
        if (sess.dirty) return; // save failed; keep the edit, don't overwrite it
      }
    }
    await reload();
  });
  // U7 E1: Ctrl+S on the PANEL, not on view.dom — focus anywhere inside the editor
  // (Save button, ed-status, Reload) saves. CodeMirror keydowns bubble up here.
  panel.addEventListener("keydown", (ev) => {
    if ((ev.ctrlKey || ev.metaKey) && ev.key.toLowerCase() === "s") {
      ev.preventDefault();
      if (sess.view && !saveBtn.disabled) void save();
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
    const lines = text.split("\n").length;
    status(`${lines} ${lines === 1 ? "line" : "lines"} · ${host}:${path}`, "ok");
  } catch (e) {
    status(`Failed to open ${path}: ${msg(e)}. Check the file exists, then press Reload.`, "err");
  }
}

// ---------- clipboard ----------
// Windows clipboard text is CRLF; a raw \r acts as Enter and the trailing \n
// adds a phantom blank line on top of the shell echo. Normalize to plain LF
// (bash/zsh pastes fine) and strip a single trailing newline.
function pasteNormalize(text: string): string {
  const lf = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  return lf.endsWith("\n") ? lf.slice(0, -1) : lf;
}

// ---------- tabs ----------
// Manual tab-drag state: mousedown arms it, moving >6px activates, mouseup
// over another TERMINAL tab splits. Plain clicks never activate it.
let tabDrag: { srcId: number; active: boolean; x: number; y: number } | null = null;

function tabFromPoint(x: number, y: number): HTMLElement | null {
  return (document.elementFromPoint(x, y)?.closest(".tab") as HTMLElement | null) ?? null;
}

window.addEventListener("mousemove", (ev) => {
  if (!tabDrag) return;
  if (!tabDrag.active) {
    if (Math.hypot(ev.clientX - tabDrag.x, ev.clientY - tabDrag.y) < 6) return;
    tabDrag.active = true;
    document.querySelector(`.tab[data-tab="${tabDrag.srcId}"]`)?.classList.add("dragging");
    document.getElementById("tab-split-zone")?.classList.add("drag-hot");
  }
  document.querySelectorAll(".tab.drop-target").forEach((t) => t.classList.remove("drop-target"));
  const over = tabFromPoint(ev.clientX, ev.clientY);
  const tid = Number(over?.dataset.tab);
  if (over && Number.isFinite(tid) && tid !== tabDrag.srcId && sessions.has(tid) && !splitSessions.has(tid)) {
    over.classList.add("drop-target");
  }
});

window.addEventListener("mouseup", (ev) => {
  const d = tabDrag;
  tabDrag = null;
  if (!d) return;
  document.querySelector(`.tab[data-tab="${d.srcId}"]`)?.classList.remove("dragging");
  document.getElementById("tab-split-zone")?.classList.remove("drag-hot");
  document.querySelectorAll(".tab.drop-target").forEach((t) => t.classList.remove("drop-target"));
  if (!d.active) return; // plain click — not a drag
  const over = tabFromPoint(ev.clientX, ev.clientY);
  const targetId = Number(over?.dataset.tab);
  if (!Number.isFinite(targetId) || targetId === d.srcId) return;
  if (!sessions.has(targetId) || splitSessions.has(targetId)) return;
  splitTerminal(targetId, d.srcId);
});

function makeTab(tabId: number, kind: "terminal" | "sftp" | "editor", label: string): HTMLElement {
  const tab = document.createElement("div");
  tab.className = `tab kind-${kind}`;
  tab.dataset.tab = String(tabId);
  tab.dataset.kind = kind;
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
    else if (tab.classList.contains("dead")) reconnectTab(tabId).catch(() => {}); // fail-safe: tab tetap dead
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
  // Manual tab drag (Termius/VSCode-style split trigger). HTML5 drag events
  // are unreliable inside WebView2/Tauri, so we track mouse movement instead.
  tab.addEventListener("mousedown", (ev) => {
    if (ev.button !== 0 || (ev.target as HTMLElement).closest("button")) return;
    tabDrag = { srcId: tabId, active: false, x: ev.clientX, y: ev.clientY };
  });
  // Tab context menu. Without this, right-click shows the webview's native
  // menu whose "Refresh" reloads the whole window — killing every tab at once.
  tab.addEventListener("contextmenu", (ev) => {
    ev.preventDefault();
    ev.stopPropagation();
    showTabMenu(ev as MouseEvent, tabId);
  });
  return tab;
}

// U3 §4.2: dead terminal tab = reload icon --danger (U9: was a 2nd ✕ beside
// the .x close glyph); clicking it respawns the PTY.
const DEAD_TITLE = "Session ended — click to reconnect";
function kindIcon(kind: string): string {
  return kind === "sftp" ? ICONS.arrows : kind === "editor" ? ICONS.pencil : ICONS.terminal;
}
function markTabDead(tabId: number, dead: boolean) {
  const tab = document.querySelector<HTMLElement>(`.tab[data-tab="${tabId}"]`);
  if (!tab) return;
  tab.classList.toggle("dead", dead);
  const ico = tab.querySelector(".t-ico");
  if (ico) ico.innerHTML = dead ? ICONS.reload : kindIcon(tab.dataset.kind || "terminal");
  if (dead) tab.title = DEAD_TITLE;
  else tab.removeAttribute("title");
}
async function reconnectTab(tabId: number) {
  activateTab(tabId);
  const s = sessions.get(tabId);
  if (!s || s.ptyId != null) return;
  await startPty(s);
  markTabDead(tabId, false);
}
// host a tab belongs to — Close host / Ctrl+Shift+W group by it
function tabHost(id: number): string | null {
  return sessions.get(id)?.host ?? splitSessions.get(id)?.host
    ?? sftpSessions.get(id)?.host ?? editorSessions.get(id)?.host ?? null;
}

// In-app tab context menu: Close / Close others / Close host / Close all.
function showTabMenu(ev: MouseEvent, tabId: number) {
  document.getElementById("tab-menu")?.remove();
  const menu = document.createElement("div");
  menu.id = "tab-menu";
  menu.setAttribute("role", "menu");
  const closeHost = () => {
    const h = tabHost(tabId);
    if (h) for (const id of openTabIds()) if (tabHost(id) === h) void closeTab(id);
  };
  const items: Array<[string, string | null, () => void]> = [
    ["Close", "Ctrl+W", () => void closeTab(tabId)],
    ["Close others", "Ctrl+Shift+W", () => {
      for (const id of openTabIds()) if (id !== tabId) void closeTab(id);
    }],
    ["Close host", null, closeHost],
    ["Close all", null, () => { for (const id of openTabIds()) void closeTab(id); }],
  ];
  for (const [label, tip, fn] of items) {
    const b = document.createElement("button");
    b.setAttribute("role", "menuitem");
    b.textContent = label;
    if (tip) b.title = tip;
    b.addEventListener("click", () => { menu.remove(); fn(); });
    menu.appendChild(b);
  }
  document.body.appendChild(menu);
  const mw = menu.offsetWidth, mh = menu.offsetHeight;
  menu.style.left = Math.min(ev.clientX, window.innerWidth - mw - 8) + "px";
  menu.style.top = Math.min(ev.clientY, window.innerHeight - mh - 8) + "px";
  const close = () => { menu.remove(); document.removeEventListener("mousedown", onDown, true); };
  const onDown = (e: MouseEvent) => { if (!menu.contains(e.target as Node)) close(); };
  setTimeout(() => document.addEventListener("mousedown", onDown, true), 0);
  menu.addEventListener("keydown", (e) => { if (e.key === "Escape") close(); });
}

function openTabIds(): number[] {
  return [...sessions.keys(), ...editorSessions.keys(), ...sftpSessions.keys()];
}

// ---------- terminals ----------
// shared xterm theme (primary + split terminals)
// Abyss Emerald ANSI remap (redesign §6.5); literals — xterm canvas can't read CSS vars
const TERMINAL_THEME = {
  background: "#020203",
  foreground: "#f2f2f2",
  cursor: "#00d992",
  cursorAccent: "#020203",
  selectionBackground: "#0e2a20",
  selectionForeground: "#f2f2f2",
  selectionInactiveBackground: "#0e2a20",
  // scrollbar sliders are not settable in xterm 5.5's ITheme; --border-soft /
  // --border / --accent apply when the addon support lands
  black: "#262423",
  red: "#fd9c9f",
  green: "#80d280",
  yellow: "#ffdd80",
  blue: "#818cf8",
  magenta: "#2fd6a1",
  cyan: "#00d992",
  white: "#b8b3b0",
  brightBlack: "#7d7873",
  brightRed: "#fd9c9f",
  brightGreen: "#80d280",
  brightYellow: "#ffdd80",
  brightBlue: "#818cf8",
  brightMagenta: "#2fd6a1",
  brightCyan: "#00d992",
  brightWhite: "#f2f2f2",
};


// Split view: the panel holds a flex row of two .term-host panes sharing one
// xterm Tab/term per pane; each pane gets its own PTY via openTerminalIn().
// Split view, Termius style: drag one terminal tab onto another and the
// dragged host opens beside the target in the same panel — its own xterm
// instance and its own PTY, fully independent of the first session.
async function splitTerminal(targetTabId: number, sourceTabId: number) {
  if (binaryBlocked("ssh")) return;
  const srcSession = sessions.get(sourceTabId);
  const panel = document.querySelector<HTMLElement>(`.panel[data-tab="${targetTabId}"]`);
  if (!srcSession || !panel || panel.classList.contains("split")) return;
  panel.classList.add("split");

  // Arus split layout (§4.3): flex row .term-split — sisi lama, divider 4px,
  // sisi baru. Drag manual mousedown/mousemove/mouseup (STATE.md #4, bukan
  // HTML5 DnD); double-click reset 50/50; klik sisi = fokus + underline.
  const termSplit = document.createElement("div");
  termSplit.className = "term-split";
  const leftSide = document.createElement("div");
  leftSide.className = "term-side focused";
  const rightSide = document.createElement("div");
  rightSide.className = "term-side";
  const divider = document.createElement("div");
  divider.className = "term-divider";
  divider.setAttribute("role", "separator");
  divider.setAttribute("aria-orientation", "vertical");
  divider.setAttribute("aria-label", "Resize split");
  divider.title = "Tarik untuk resize · double-click reset 50/50";
  termSplit.append(leftSide, divider, rightSide);
  while (panel.firstChild) leftSide.appendChild(panel.firstChild); // host_el + banner pindah
  panel.appendChild(termSplit);

  const setSplitFocus = (side: HTMLElement) => {
    leftSide.classList.toggle("focused", side === leftSide);
    rightSide.classList.toggle("focused", side === rightSide);
  };
  // window-level listeners must die with the tab, else they retain the detached
  // DOM (SEC LOW t_fb1d18d6): splitAbort is pushed into split_session.unlisten,
  // so the existing closeTab loop removes them.
  const splitAbort = new AbortController();
  const splitSig = { signal: splitAbort.signal };
  let dragX = false;
  divider.addEventListener("mousedown", (e) => {
    dragX = true;
    divider.classList.add("dragging");
    e.preventDefault();
  });
  window.addEventListener(
    "mousemove",
    (e) => {
      if (!dragX) return;
      const r = termSplit.getBoundingClientRect();
      const pct = Math.min(80, Math.max(20, ((e.clientX - r.left) / r.width) * 100));
      leftSide.style.flex = `0 0 ${pct}%`;
    },
    splitSig,
  );
  const fitSplit = () => {
    for (const s of [sessions.get(targetTabId), split_session]) {
      if (s) {
        s.fit.fit();
        if (s.ptyId != null) invoke("pty_resize", { id: s.ptyId, cols: s.term.cols, rows: s.term.rows });
      }
    }
  };
  window.addEventListener(
    "mouseup",
    () => {
      if (!dragX) return;
      dragX = false;
      divider.classList.remove("dragging");
      fitSplit();
    },
    splitSig,
  );
  divider.addEventListener("dblclick", () => {
    leftSide.style.flex = "";
    fitSplit();
  });
  leftSide.addEventListener("mousedown", () => {
    setSplitFocus(leftSide);
    sessions.get(targetTabId)?.term.focus();
  });
  rightSide.addEventListener("mousedown", () => {
    setSplitFocus(rightSide);
    split_session.term.focus();
  });

  const host = srcSession.host;

  const term = new Terminal({
    cursorBlink: true,
    fontFamily: FONT_MONO,
    fontSize: 13,
    theme: TERMINAL_THEME,
  });
  const fit = new FitAddon();
  term.loadAddon(fit);

  const host_el = document.createElement("div");
  host_el.className = "term-host";
  const banner = document.createElement("div");
  banner.className = "term-banner";
  banner.innerHTML = `<span class="spinner"></span>Connecting to ${esc(host)}…`;
  rightSide.appendChild(host_el);
  rightSide.appendChild(banner);
  term.open(host_el);

  host_el.addEventListener("mouseup", () => {
    const sel = term.getSelection();
    if (sel) {
      invoke("clipboard_write", { text: sel }).catch(() => {});
      term.clearSelection();
    }
  });
  host_el.addEventListener("contextmenu", (ev) => {
    ev.preventDefault();
    invoke("clipboard_read")
      .then((raw) => {
        const text = pasteNormalize(String(raw));
        if (text && split_session.ptyId != null) invoke("pty_write", { id: split_session.ptyId, data: text });
      })
      .catch(() => {});
  });

  const split_session: Session = { id: targetTabId, ptyId: null, host, term, fit, banner, unlisten: [] };
  split_session.unlisten.push(() => splitAbort.abort()); // drop window drag listeners (SEC LOW t_fb1d18d6)
  splitSessions.set(targetTabId, split_session);
  updateStatusBar();
  term.onData((data) => {
    if (split_session.ptyId != null) invoke("pty_write", { id: split_session.ptyId, data });
  });
  await startPty(split_session);
}

async function openTerminal(host: string) {
  if (binaryBlocked("ssh")) return;
  const tabId = nextTab++;
  const term = new Terminal({
    cursorBlink: true,
    fontFamily: FONT_MONO,
    fontSize: 13,
    theme: TERMINAL_THEME,
  });
  const fit = new FitAddon();
  term.loadAddon(fit);

  const tab = makeTab(tabId, "terminal", host);
  $("#tab-split-zone").before(tab);

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

  // Termius/Windows-terminal style clipboard: selecting text copies it
  // immediately; right-click pastes the clipboard into the terminal.
  host_el.addEventListener("mouseup", () => {
    const sel = term.getSelection();
    if (sel) {
      invoke("clipboard_write", { text: sel }).catch(() => {});
      term.clearSelection();
    }
  });
  host_el.addEventListener("contextmenu", (ev) => {
    ev.preventDefault();
    invoke("clipboard_read")
      .then((raw) => {
        const text = pasteNormalize(String(raw));
        if (text && session.ptyId != null) invoke("pty_write", { id: session.ptyId, data: text });
      })
      .catch(() => {});
  });

  const session: Session = { id: tabId, ptyId: null, host, term, fit, banner, unlisten: [] };
  sessions.set(tabId, session);
  activateTab(tabId);

  // user keystrokes -> pty
  term.onData((data) => {
    if (session.ptyId != null) invoke("pty_write", { id: session.ptyId, data });
  });

  await startPty(session);
}

// Spawn the PTY for a terminal session and wire output/exit listeners.
async function startPty(session: Session) {
  try {
    session.fit.fit();
    const cols = session.term.cols;
    const rows = session.term.rows;
    const ptyId = await invoke<number>("pty_spawn", { host: session.host, cols, rows });
    session.ptyId = ptyId;
    refreshHostDots();

    const u1 = await listen<string>(`pty://output/${ptyId}`, (ev) => {
      session.banner?.remove();
      session.term.write(ev.payload);
    });
    const u2 = await listen(`pty://exit/${ptyId}`, () => {
      session.term.write("\\r\\n\\x1b[90m[session ended]\\x1b[0m\\r\\n");
      session.ptyId = null;
      if (sessions.get(session.id) === session) markTabDead(session.id, true); // split sides share the tab
      refreshHostDots();
    });
    session.unlisten.push(u1, u2);
  } catch (e) {
    session.banner?.remove();
    session.term.write(`\\r\\n\\x1b[31mFailed to start ssh: ${msg(e)}. Fix the host and reopen the tab.\\x1b[0m\\r\\n`)
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
  // split fokus kembali ke sisi utama saat tab diaktifkan (underline ikut).
  // Query .term-side, bukan indeks children[] — tak tergantung posisi divider.
  const sides = document.querySelectorAll(`.panel[data-tab="${tabId}"] .term-side`);
  if (sides.length) {
    sides[0].classList.add("focused");
    sides[sides.length - 1].classList.remove("focused");
  }
  updateStatusBar();
  const fitOne = (s?: Session) => {
    if (!s) return;
    s.fit.fit();
    if (s.ptyId != null) invoke("pty_resize", { id: s.ptyId, cols: s.term.cols, rows: s.term.rows });
  };
  setTimeout(() => {
    fitOne(sessions.get(tabId));
    fitOne(splitSessions.get(tabId));
    sessions.get(tabId)?.term.focus();
  }, 0);
}

async function closeTab(tabId: number) {
  const sp = splitSessions.get(tabId);
  if (sp) {
    if (sp.ptyId != null) await invoke("pty_kill", { id: sp.ptyId }).catch(() => {});
    sp.unlisten.forEach((u) => u());
    sp.term.dispose();
    splitSessions.delete(tabId);
  }
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
    const s = sftpSessions.get(tabId) as SftpSession;
    s.unlisten.forEach((u) => u()); // window mouseup net + strip fade timers (U4 invariant)
    sftpSessions.delete(tabId);
    sftpForgetHost(s.host);
  } else {
    return;
  }
  document.querySelector(`.tab[data-tab="${tabId}"]`)?.remove();
  document.querySelector(`.panel[data-tab="${tabId}"]`)?.remove();
  refreshHostDots();
  updateStatusBar();
  if (activeTab === tabId) {
    const remaining = [...sessions.keys(), ...editorSessions.keys(), ...sftpSessions.keys()];
    if (remaining.length) activateTab(remaining[remaining.length - 1]);
    else activeTab = null;
  }
}

// resize active terminal with the window
window.addEventListener("resize", () => {
  if (activeTab != null) {
    for (const s of [sessions.get(activeTab), splitSessions.get(activeTab)]) {
      if (s) {
        s.fit.fit();
        if (s.ptyId != null) invoke("pty_resize", { id: s.ptyId, cols: s.term.cols, rows: s.term.rows });
      }
    }
  }
});

// ---------- wire up ----------
window.addEventListener("DOMContentLoaded", () => {
  // sidebar width resizer (drag the divider; width persists in localStorage)
  const sidebar = document.getElementById("sidebar") as HTMLElement;
  const divider = document.createElement("div");
  divider.id = "sb-resize";
  divider.setAttribute("role", "separator");
  divider.setAttribute("aria-orientation", "vertical");
  divider.setAttribute("aria-label", "Resize host list");
  sidebar.after(divider);
  const saved = localStorage.getItem("sb-width");
  if (saved) sidebar.style.width = saved + "px";
  let dragging = false;
  divider.addEventListener("mousedown", (e) => {
    dragging = true;
    divider.classList.add("dragging");
    e.preventDefault();
  });
  window.addEventListener("mousemove", (e) => {
    if (!dragging) return;
    const w = Math.min(560, Math.max(160, e.clientX));
    sidebar.style.width = w + "px";
  });
  window.addEventListener("mouseup", () => {
    if (!dragging) return;
    dragging = false;
    divider.classList.remove("dragging");
    const w = parseInt(sidebar.style.width, 10);
    if (w) localStorage.setItem("sb-width", String(w));
    if (activeTab != null) {
      const s = sessions.get(activeTab);
      if (s) {
        s.fit.fit();
        if (s.ptyId != null) invoke("pty_resize", { id: s.ptyId, cols: s.term.cols, rows: s.term.rows });
      }
    }
  });

  $("#btn-add").addEventListener("click", () => openEditor(null));
  $("#btn-reload").addEventListener("click", loadHosts);
  $("#btn-raw").addEventListener("click", openRaw);
  ($("#host-search") as HTMLInputElement).addEventListener("input", renderHostList);
  // Ctrl+K focuses host search (redesign §5); overlays keep their focus trap
  window.addEventListener("keydown", (ev) => {
    if ((ev.ctrlKey || ev.metaKey) && ev.key.toLowerCase() === "k" && overlayStack.length === 0) {
      ev.preventDefault();
      const s = $("#host-search") as HTMLInputElement;
      s.focus();
      s.select();
    }
  });
  // Ctrl+W closes the active tab; Ctrl+Shift+W closes the others (§5, FR-G3).
  // Ignored while focus sits in a text field so typing is never interrupted.
  window.addEventListener("keydown", (ev) => {
    if (!(ev.ctrlKey || ev.metaKey) || ev.key.toLowerCase() !== "w") return;
    if (activeTab == null || overlayStack.length > 0) return;
    const a = document.activeElement as HTMLElement | null;
    if (a && (a.tagName === "INPUT" || a.tagName === "TEXTAREA" || a.tagName === "SELECT" || a.isContentEditable)) return;
    ev.preventDefault();
    if (ev.shiftKey) {
      for (const id of openTabIds()) if (id !== activeTab) void closeTab(id);
    } else void closeTab(activeTab);
  });
  // ↑/↓ host navigation (§5): ArrowDown from search enters the list
  ($("#host-search") as HTMLInputElement).addEventListener("keydown", (ev) => {
    if (ev.key === "ArrowDown") {
      ev.preventDefault();
      document.querySelector<HTMLElement>(".host-item")?.focus();
    }
  });
  // sidebar collapse: remembered across restarts
  const setCollapsed = (c: boolean) => {
    document.body.classList.toggle("sb-collapsed", c);
    $("#btn-expand").classList.toggle("hidden", !c);
    try { localStorage.setItem("hosterm.sb-collapsed", c ? "1" : ""); } catch { /* private mode */ }
  };
  setCollapsed(localStorage.getItem("hosterm.sb-collapsed") === "1");
  $("#btn-collapse").addEventListener("click", () => setCollapsed(true));
  $("#btn-expand").addEventListener("click", () => setCollapsed(false));
  $("#m-cancel").addEventListener("click", cancelEditor);
  $("#m-save").addEventListener("click", saveHost);
  $("#m-delete").addEventListener("click", deleteHost);
  $("#m-delete-pw").addEventListener("click", deleteHostPassword);
  $("#raw-cancel").addEventListener("click", cancelRaw);
  $("#raw-save").addEventListener("click", saveRaw);
  $("#f-auth").addEventListener("change", (e) => {
    // U8: keep the chip/placeholder copy in sync with the edited host's real
    // stored-password state (toggling auth away and back must not drop it)
    const h = hosts.find((x) => x.name === editingName);
    applyAuthMode((e.target as HTMLSelectElement).value, h?.has_password ?? false);
  });
  $("#f-newkey").addEventListener("click", openKeyModal);
  $("#k-cancel").addEventListener("click", closeKey);
  $("#k-save").addEventListener("click", saveKey);
  // OpenSSH guide (FR-W2-2)
  $("#ssh-ps-cmd").textContent = PS_INSTALL_CMD;
  $("#ssh-copy").addEventListener("click", async () => {
    const btn = $("#ssh-copy");
    try {
      await invoke("clipboard_write", { text: PS_INSTALL_CMD });
      btn.textContent = "Copied!";
      setTimeout(() => { btn.textContent = "Copy"; }, 1500);
    } catch { /* clipboard unavailable; the command stays selectable text */ }
  });
  $("#ssh-recheck").addEventListener("click", recheckBinaries);
  $("#ssh-continue").addEventListener("click", () => closeOverlay("#ssh-modal"));
  // detection off the config-parse path: fire alongside, don't await (NFR-W2-2)
  $("#sb-ver").textContent = APP_VERSION;
  updateStatusBar();
  startupBinaryCheck();
  loadHosts();
});
