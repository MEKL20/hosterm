import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
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

async function loadHosts() {
  try {
    hosts = await invoke<HostDto[]>("read_ssh_config");
  } catch (e) {
    hosts = [];
    console.error(e);
  }
  renderHostList();
}

function renderHostList() {
  const list = $("#host-list");
  list.innerHTML = "";
  if (hosts.length === 0) {
    const empty = document.createElement("div");
    empty.className = "h-sub";
    empty.style.padding = "10px";
    empty.textContent = "No hosts in ~/.ssh/config yet. Click + to add one.";
    list.appendChild(empty);
    return;
  }
  for (const h of hosts) {
    if (h.name.includes("*") || h.name.includes("?")) continue; // skip pattern blocks
    const el = document.createElement("div");
    el.className = "host-item";
    const sub = [h.user && `${h.user}@`, h.host_name || "(no HostName)", h.port && `:${h.port}`]
      .filter(Boolean)
      .join("");
    el.innerHTML = `<div><span class="h-name">${esc(h.name)}</span><span class="h-edit">edit</span></div><div class="h-sub">${esc(sub)}</div>`;
    el.addEventListener("click", (ev) => {
      if ((ev.target as HTMLElement).classList.contains("h-edit")) {
        openEditor(h);
      } else {
        openTerminal(h.name);
      }
    });
    list.appendChild(el);
  }
}

function esc(s: string): string {
  const d = document.createElement("div");
  d.textContent = s;
  return d.innerHTML;
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
    opts.push(`<option value="${esc(p)}"${p === selected ? " selected" : ""}>${esc(p)}</option>`);
  }
  sel.innerHTML = opts.join("");
  sel.value = selected || "";
}

function applyAuthMode(method: string) {
  $("#row-identity").classList.toggle("hidden", method !== "key");
  $("#row-password-note").classList.toggle("hidden", method !== "password");
}

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
  $("#modal").classList.remove("hidden");
  ($("#f-name") as HTMLInputElement).focus();
}

function closeEditor() {
  $("#modal").classList.add("hidden");
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
    $("#modal-err").textContent = String(e);
  }
}

// ---------- create identity key from pasted text ----------
function openKeyModal() {
  ($("#k-name") as HTMLInputElement).value = "";
  ($("#k-text") as HTMLTextAreaElement).value = "";
  $("#key-err").textContent = "";
  $("#key-modal").classList.remove("hidden");
  ($("#k-name") as HTMLInputElement).focus();
}

async function saveKey() {
  const name = ($("#k-name") as HTMLInputElement).value.trim();
  const privateKey = ($("#k-text") as HTMLTextAreaElement).value;
  try {
    const path = await invoke<string>("create_identity_file", { name, privateKey });
    $("#key-modal").classList.add("hidden");
    // select the freshly-created key in the host editor dropdown
    await refreshIdentityDropdown(path);
    ($("#f-auth") as HTMLSelectElement).value = "key";
    applyAuthMode("key");
  } catch (e) {
    $("#key-err").textContent = String(e);
  }
}

async function deleteHost() {
  if (!editingName) return;
  if (!confirm(`Delete host "${editingName}" from ~/.ssh/config?`)) return;
  try {
    await invoke("delete_host", { name: editingName });
    closeEditor();
    await loadHosts();
  } catch (e) {
    $("#modal-err").textContent = String(e);
  }
}

// ---------- raw config modal ----------
async function openRaw() {
  try {
    const text = await invoke<string>("read_config_raw");
    ($("#raw-text") as HTMLTextAreaElement).value = text;
    $("#raw-err").textContent = "";
    $("#raw-modal").classList.remove("hidden");
  } catch (e) {
    alert(String(e));
  }
}
async function saveRaw() {
  try {
    await invoke("write_config_raw", { content: ($("#raw-text") as HTMLTextAreaElement).value });
    $("#raw-modal").classList.add("hidden");
    await loadHosts();
  } catch (e) {
    $("#raw-err").textContent = String(e);
  }
}

// ---------- terminals ----------
async function openTerminal(host: string) {
  const tabId = nextTab++;
  const term = new Terminal({
    cursorBlink: true,
    fontFamily: 'ui-monospace, "Cascadia Code", Menlo, monospace',
    fontSize: 13,
    theme: { background: "#1a1b26", foreground: "#c0caf5", cursor: "#c0caf5" },
  });
  const fit = new FitAddon();
  term.loadAddon(fit);

  // tab button
  const tab = document.createElement("div");
  tab.className = "tab";
  tab.dataset.tab = String(tabId);
  tab.innerHTML = `<span>${esc(host)}</span><span class="x">×</span>`;
  tab.addEventListener("click", (ev) => {
    if ((ev.target as HTMLElement).classList.contains("x")) {
      closeTab(tabId);
    } else {
      activateTab(tabId);
    }
  });
  $("#tabs").appendChild(tab);

  // panel
  const panel = document.createElement("div");
  panel.className = "panel";
  panel.dataset.tab = String(tabId);
  const host_el = document.createElement("div");
  host_el.className = "term-host";
  panel.appendChild(host_el);
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
      session.term.write(ev.payload);
    });
    const u2 = await listen(`pty://exit/${ptyId}`, () => {
      session.term.write("\r\n\x1b[90m[session ended]\x1b[0m\r\n");
      session.ptyId = null;
    });
    session.unlisten.push(u1, u2);
  } catch (e) {
    term.write(`\r\n\x1b[31mFailed to start ssh: ${e}\x1b[0m\r\n`);
  }
}

function activateTab(tabId: number) {
  activeTab = tabId;
  document.querySelectorAll(".tab").forEach((t) => {
    t.classList.toggle("active", (t as HTMLElement).dataset.tab === String(tabId));
  });
  document.querySelectorAll(".panel").forEach((p) => {
    p.classList.toggle("active", (p as HTMLElement).dataset.tab === String(tabId));
  });
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
  if (!s) return;
  if (s.ptyId != null) await invoke("pty_kill", { id: s.ptyId }).catch(() => {});
  s.unlisten.forEach((u) => u());
  s.term.dispose();
  document.querySelector(`.tab[data-tab="${tabId}"]`)?.remove();
  document.querySelector(`.panel[data-tab="${tabId}"]`)?.remove();
  sessions.delete(tabId);
  if (activeTab === tabId) {
    const remaining = [...sessions.keys()];
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
  $("#m-cancel").addEventListener("click", closeEditor);
  $("#m-save").addEventListener("click", saveHost);
  $("#m-delete").addEventListener("click", deleteHost);
  $("#raw-cancel").addEventListener("click", () => $("#raw-modal").classList.add("hidden"));
  $("#raw-save").addEventListener("click", saveRaw);
  $("#f-auth").addEventListener("change", (e) => applyAuthMode((e.target as HTMLSelectElement).value));
  $("#f-newkey").addEventListener("click", openKeyModal);
  $("#k-cancel").addEventListener("click", () => $("#key-modal").classList.add("hidden"));
  $("#k-save").addEventListener("click", saveKey);
  loadHosts();
});
