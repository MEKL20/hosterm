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
    el.innerHTML = `<div><span class="h-name">${esc(h.name)}</span>` +
      `<span class="h-act h-sftp" title="Open SFTP file browser">sftp</span>` +
      `<span class="h-act h-edit">edit</span></div>` +
      `<div class="h-sub">${esc(sub)}</div>`;
    el.addEventListener("click", (ev) => {
      const t = ev.target as HTMLElement;
      if (t.classList.contains("h-edit")) {
        openEditor(h);
      } else if (t.classList.contains("h-sftp")) {
        openSftp(h.name);
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

// ---------- sftp browser ----------
type SftpSession = { host: string; path: string };
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

async function openSftp(host: string) {
  const tabId = nextTab++;
  sftpSessions.set(tabId, { host, path: "" });

  const tab = document.createElement("div");
  tab.className = "tab";
  tab.dataset.tab = String(tabId);
  tab.innerHTML = `<span>⇅ ${esc(host)}</span><span class="x">×</span>`;
  tab.addEventListener("click", (ev) => {
    if ((ev.target as HTMLElement).classList.contains("x")) closeTab(tabId);
    else activateTab(tabId);
  });
  $("#tabs").appendChild(tab);

  const panel = document.createElement("div");
  panel.className = "panel";
  panel.dataset.tab = String(tabId);
  panel.innerHTML = `
    <div class="sftp-wrap">
      <div class="sftp-toolbar">
        <button class="sftp-up" title="Parent directory">↑</button>
        <input class="sftp-path" placeholder="remote path (empty = home)" />
        <button class="sftp-go">Go</button>
        <button class="sftp-refresh" title="Reload">⟳</button>
      </div>
      <div class="sftp-list"></div>
      <div class="sftp-foot">
        <input class="sftp-local" placeholder="local path (download dir / file to upload)" />
        <button class="sftp-upload">⬆ Upload</button>
      </div>
      <div class="sftp-status"></div>
    </div>`;
  $("#panels").appendChild(panel);

  const q = (sel: string) => panel.querySelector(sel) as HTMLElement;
  const inp = q(".sftp-path") as HTMLInputElement;
  const loc = q(".sftp-local") as HTMLInputElement;
  const status = (msg: string, ok = false) => {
    const el = q(".sftp-status");
    el.textContent = msg;
    el.classList.toggle("ok", ok);
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
    const localPath = loc.value.trim().replace(/[\\/]+$/, "");
    if (!localPath) return status("enter a local file path to upload");
    const remote = joinRemote((sftpSessions.get(tabId) as SftpSession).path, baseName(localPath));
    status(`uploading ${baseName(localPath)} …`);
    try {
      await invoke("sftp_upload", { host, localPath, remotePath: remote });
      status(`uploaded → ${remote}`, true);
      renderSftp(tabId, status);
    } catch (e) {
      status(String(e));
    }
  });

  activateTab(tabId);
  renderSftp(tabId, status);
}

async function renderSftp(tabId: number, status: (m: string, ok?: boolean) => void) {
  const s = sftpSessions.get(tabId);
  if (!s) return;
  const panel = document.querySelector(`.panel[data-tab="${tabId}"]`) as HTMLElement;
  const list = panel.querySelector(".sftp-list") as HTMLElement;
  const inp = panel.querySelector(".sftp-path") as HTMLInputElement;
  inp.value = s.path;
  list.innerHTML = `<div class="h-sub" style="padding:10px">loading …</div>`;
  try {
    const entries = await invoke<{ name: string; is_dir: boolean; is_link: boolean; size: number }[]>(
      "sftp_list",
      { host: s.host, path: s.path }
    );
    list.innerHTML = "";
    if (entries.length === 0) {
      list.innerHTML = `<div class="h-sub" style="padding:10px">(empty directory)</div>`;
    }
    for (const e of entries) {
      const row = document.createElement("div");
      row.className = "sftp-row";
      row.innerHTML = `<span class="nm">${e.is_dir ? "📁" : e.is_link ? "↪" : "📄"} ${esc(e.name)}</span>` +
        `<span class="sz">${e.is_dir ? "—" : fmtSize(e.size)}</span>` +
        (e.is_dir ? "" : `<span class="dl" title="Download">⬇</span>`);
      row.addEventListener("click", (ev) => {
        if ((ev.target as HTMLElement).classList.contains("dl")) {
          const localDir = (panel.querySelector(".sftp-local") as HTMLInputElement).value.trim().replace(/[\\/]+$/, "");
          if (!localDir) return status("set a local download dir in the bottom input first");
          const local = `${localDir}/${e.name}`;
          status(`downloading ${e.name} …`);
          invoke("sftp_download", { host: s.host, remotePath: joinRemote(s.path, e.name), localPath: local })
            .then(() => status(`saved → ${local}`, true))
            .catch((err) => status(String(err)));
        } else if (e.is_dir) {
          s.path = joinRemote(s.path, e.name);
          renderSftp(tabId, status);
        }
      });
      list.appendChild(row);
    }
    status(`${entries.length} item(s) · ${s.host}:${s.path || "~"}`, true);
  } catch (err) {
    list.innerHTML = "";
    status(String(err));
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
  if (s) {
    if (s.ptyId != null) await invoke("pty_kill", { id: s.ptyId }).catch(() => {});
    s.unlisten.forEach((u) => u());
    s.term.dispose();
    sessions.delete(tabId);
  } else if (sftpSessions.has(tabId)) {
    sftpSessions.delete(tabId);
  } else {
    return;
  }
  document.querySelector(`.tab[data-tab="${tabId}"]`)?.remove();
  document.querySelector(`.panel[data-tab="${tabId}"]`)?.remove();
  if (activeTab === tabId) {
    const remaining = [...sessions.keys(), ...sftpSessions.keys()];
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
