import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Terminal, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { basicSetup, EditorView } from "codemirror";
import { Compartment } from "@codemirror/state";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import palettes from "./theme-palettes.json";
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

type ThemePreset = "dusk" | "slate" | "ink" | "light";

interface AllowEntry {
  host: string;
  command: string;
  added_at: string;
}

interface AppSettings {
  version: number;
  theme: ThemePreset;
  accent: string;
  allowlist: AllowEntry[];
}

interface Snippet {
  name: string;
  command: string;
}

interface SnippetGroup {
  category: string;
  items: Snippet[];
}

interface AiProposal {
  id: string;
  prompt: string;
  response: string;
  host: string;
  command: string;
  description: string;
}

type AiDecision = "proposed" | "ran" | "rejected" | "refused";

// xterm cannot read CSS custom properties; keep this identical to --font-mono in styles.css
const FONT_MONO = '"JetBrains Mono", "Geist Mono", "Cascadia Code", "SF Mono", ui-monospace, "DejaVu Sans Mono", monospace';

// §13.3 motion: the xterm cursor blink is canvas-drawn, so the CSS
// reduced-motion kill (* { animation: none }) cannot reach it — blink is
// opted out at Terminal construction instead. Sessions already open keep
// their blink; the flag is re-read on every new terminal/split.
const REDUCED_MOTION = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

const svg = (p: string) =>
  `<svg viewBox="0 0 16 16" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${p}</svg>`;
const ICONS = {
  add: svg('<line x1="8" y1="4" x2="12" y2="4"/><line x1="10" y1="2" x2="10" y2="6"/>'),
  reload: svg('<path d="M13 8a5 5 0 1 1-1.7-3.75"/><polyline points="13,1.5 13,4.5 10,4.5"/>'),
  terminal: svg('<rect x="1.5" y="2.5" width="13" height="11" rx="2"/><polyline points="4,6 6,8 4,10"/><line x1="8" y1="10" x2="11" y2="10"/>'),
  arrows: svg('<line x1="5" y1="3" x2="5" y2="13"/><polyline points="3,5 5,3 7,5"/><line x1="11" y1="13" x2="11" y2="3"/><polyline points="9,11 11,13 13,11"/>'),
  pencil: svg('<line x1="3" y1="13" x2="12" y2="4"/><line x1="10.5" y1="5.5" x2="12" y2="7"/>'),
  trash: svg('<polyline points="3,4 13,4"/><path d="M4.5,4 V13 h7 V4"/><line x1="6.5" y1="6.5" x2="6.5" y2="11"/><line x1="9.5" y1="6.5" x2="9.5" y2="11"/><path d="M6,4 V2.5 h4 V4"/>'),
  close: svg('<line x1="4" y1="4" x2="12" y2="12"/><line x1="12" y1="4" x2="4" y2="12"/>'),
  folder: svg('<path d="M1.5,4 h4 l1.5,2 h7.5 v7 h-13 z"/>'),
  file: svg('<path d="M3.5,1.5 h6 l3,3 v10 h-9 z"/><polyline points="9.5,1.5 9.5,4.5 12.5,4.5"/>'),
  link: svg('<rect x="9" y="9" width="5" height="5" rx="1"/><polyline points="2,4 8,4 8,9"/><polyline points="6,7 8,9 6,11"/>'),
  download: svg('<line x1="8" y1="2" x2="8" y2="10"/><polyline points="5,7.5 8,10.5 11,7.5"/><line x1="3" y1="13.5" x2="13" y2="13.5"/>'),
  upload: svg('<line x1="8" y1="13" x2="8" y2="5"/><polyline points="5,7.5 8,4.5 11,7.5"/><line x1="3" y1="13.5" x2="13" y2="13.5"/>'),
  up: svg('<line x1="8" y1="13" x2="8" y2="4"/><polyline points="4.5,7.5 8,4 11.5,7.5"/>'),
  lock: svg('<rect x="3.5" y="7" width="9" height="6.5" rx="1"/><path d="M5.5 7V5.5a2.5 2.5 0 0 1 5 0V7"/>'),
  dots: svg('<circle cx="3.5" cy="8" r="1"/><circle cx="8" cy="8" r="1"/><circle cx="12.5" cy="8" r="1"/>'),
  copy: svg('<rect x="6" y="6" width="7.5" height="7.5" rx="1"/><path d="M3 10V3.5a1 1 0 0 1 1-1h6.5"/>'),
  check: svg('<polyline points="3,8 6.5,11.5 13,4.5"/>'),
};
// CodeMirror theme — structural rules read the preset CSS vars; token colors
// are JS literals (CSS vars can't reach them), built per preset from
// theme-palettes.json (SSOT, WCAG-gated by qa-reports/theme-contrast.mjs).
// Dusk is byte-identical to the former cmHighlight literal (SDD §13.2 pt 5):
// keyword/function=--accent, string=--live, comment=--faint, text=--text,
// type/property=cyan accent, number/operator warm neutrals.
const cmTheme = EditorView.theme({
  "&": { color: "var(--text)", backgroundColor: "var(--bg)" },
  ".cm-content": { fontFamily: FONT_MONO, fontSize: "13px", caretColor: "var(--accent)" },
  "&.cm-focused": { outline: "none" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--accent)" },
  ".cm-gutters": {
    backgroundColor: "var(--sidebar)",
    color: "var(--dim)",
    borderRight: "1px solid var(--border)",
  },
  ".cm-activeLine": { backgroundColor: "var(--surface)" },
  ".cm-activeLineGutter": { backgroundColor: "var(--surface)", color: "var(--text)" },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": { backgroundColor: "var(--accent-soft)" },
  ".cm-matchingBracket": { outline: "1px solid var(--accent)", backgroundColor: "transparent" },
}, { dark: true });

// cmHighlight slot per tag; the hex for each preset lives in theme-palettes.json.
const CM_TOKEN_ROLES = [
  { key: "keyword", tag: t.keyword },
  { key: "string", tag: t.string },
  { key: "number", tag: t.number },
  { key: "comment", tag: t.comment },
  { key: "variableName", tag: t.variableName },
  { key: "function", tag: t.function(t.variableName) },
  { key: "typeName", tag: t.typeName },
  { key: "propertyName", tag: t.propertyName },
  { key: "operator", tag: t.operator },
  { key: "punctuation", tag: t.punctuation },
] as const;

// Per-preset highlight styles; applyTheme swaps the active one into every open
// editor view through the compartment below — no EditorView recreation, so
// doc, cursor and dirty flag survive a preset change.
const CM_HIGHLIGHT = Object.fromEntries(
  (Object.keys(palettes) as ThemePreset[]).map((preset) => [
    preset,
    HighlightStyle.define(
      CM_TOKEN_ROLES.map(({ key, tag }) => ({ tag, color: palettes[preset].cm[key] })),
    ),
  ]),
) as Record<ThemePreset, HighlightStyle>;
const highlightCompartment = new Compartment();

// ---------- state ----------
let hosts: HostDto[] = [];
let editingName: string | null = null; // null => adding new
let identityFiles: string[] = [];

const DEFAULT_SETTINGS: AppSettings = {
  version: 1,
  theme: "dusk",
  accent: "#e2a35c",
  allowlist: [],
};
const QUICK_ACCENTS = new Set(["#e2a35c", "#5fbfae", "#e0869b", "#aec267", "#b99be3"]);
const CURATED_ACCENT_INK = "#241404";
const DARK_ACCENT_INK = "#000000";
const LIGHT_ACCENT_INK = "#ffffff";
let appSettings: AppSettings = { ...DEFAULT_SETTINGS, allowlist: [] };
let settingsDirty = false;
let settingsClosing = false;
let settingsReady: Promise<void> = Promise.resolve();
let snippetGroups: SnippetGroup[] = [];
// User-created snippets (CRUD) — persisted via read/write_user_snippets. Kept
// separate from the read-only bundled set (snippetGroups) so bundled snippets
// stay immutable (ADR-19) while user rows can be edited/deleted.
interface UserSnippet { id: string; name: string; command: string; category: string; }
let userSnippets: UserSnippet[] = [];
// id of the row being edited; "" = creating a new one when the form is open.
let snippetEditId: string | null = null;
const snippetCopyTimers = new WeakMap<HTMLButtonElement, number>();
// Real Google account state (BE-OAUTH: google_sign_in/out/account). The chip
// hydrates from google_account on load; sign-in runs the PKCE loopback flow
// backend-side. {name,email,picture} are remote-origin strings — every render
// routes through esc()/escAttr() (ADR-8). The picture URL is NOT fetched: CSP
// pins img-src 'self' and NFR-3 forbids idle network, so the avatar shows
// escaped initials and the URL is retained on a data-attribute only.
interface GoogleAccount { name: string; email: string; picture: string }
let googleAccount: GoogleAccount | null = null;
let googlePending = false;
let googleError = "";

// Drive backup/restore IPC result shapes (serde default snake_case, TSD §3).
interface BackupResult { file_id: string; name: string; bytes: number; created_at: string }
// `key_mode` is finalized by the backend card; until it ships we treat it as
// optional. "device-keyring" => the backup is protected by the OS keyring and
// restore needs no user passphrase; anything else (incl. undefined, legacy, or
// "user-passphrase") => prompt for the passphrase on restore.
interface BackupMeta { file_id: string; name: string; created_at: string; host: string; schema: number; key_mode?: string }
interface RestoreResult { restored: string[]; pre_restore_dir: string }
interface PreflightTarget { path_role: string; path: string; exists: boolean }
interface RestorePreflight { targets: PreflightTarget[] }
let drivePending = false;
const AI_PROPOSALS: AiProposal[] = [
  {
    id: "restart-netdata",
    prompt: "restart the metrics agent on prod-web",
    response: "This host-scoped command follows the exact-match approval contract shown below.",
    host: "prod-web",
    command: "sudo systemctl restart netdata",
    description: "Restarts the Netdata service on prod-web. Monitoring briefly pauses while the service restarts.",
  },
  {
    id: "prune-docker",
    prompt: "free up disk space on prod-web",
    response: "Docker cleanup can reclaim unused disk space. Review the exact command before it reaches the host terminal.",
    host: "prod-web",
    command: "docker system prune -af",
    description: "Removes stopped containers, unused networks, dangling images, and build cache. Running containers are not touched.",
  },
];
const aiDecisions = new Map<string, AiDecision>();
const aiResults = new Map<string, string>();
const aiExtraMessages: { role: "user" | "ai"; text: string }[] = [];

interface Session {
  id: number;
  ptyId: number | null;
  host: string;
  term: Terminal;
  fit: FitAddon;
  banner?: HTMLElement;
  unlisten: UnlistenFn[];
  // true = local shell (local_pty_spawn) rather than an ssh session to `host`.
  local?: boolean;
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

// Frontend mirror of the backend sftp_safe argv guard; it must run before pty_spawn.
function sftpSafe(value: string): boolean {
  return !value.startsWith("-") && !value.includes("\n") && !value.includes("\r");
}

function staticElement<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className = "",
  text?: string,
): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  if (className) element.className = className;
  if (text !== undefined) element.textContent = text;
  return element;
}

function staticTemplate(...children: Node[]): HTMLTemplateElement {
  const template = document.createElement("template");
  template.content.append(...children);
  return template;
}

const APPROVAL_ITEM_TEMPLATE = (() => {
  const item = staticElement("div", "approval-item");
  const host = staticElement("span", "approval-host");
  const command = staticElement("code", "approval-command");
  const revoke = staticElement("button", "approval-revoke", "Revoke");
  revoke.setAttribute("type", "button");
  revoke.setAttribute("data-index", "");
  revoke.setAttribute("aria-label", "Revoke approval");
  item.append(host, command, revoke);
  return staticTemplate(item);
})();

const AI_PROPOSAL_TEMPLATE = (() => {
  const user = staticElement("div", "msg msg-user");
  const assistant = staticElement("div", "msg msg-ai");
  const response = staticElement("p");
  const card = staticElement("div", "cmd-card");
  card.setAttribute("data-ai-id", "");

  const head = staticElement("div", "cmd-head");
  const scope = staticElement("span", "cmd-scope");
  const state = staticElement("span", "cmd-state");
  state.setAttribute("aria-live", "polite");
  head.append(scope, state);

  const command = staticElement("code", "cmd-line");
  const description = staticElement("p", "cmd-desc");
  const actions = staticElement("div", "cmd-actions");
  const run = staticElement("button", "btn-primary approval-run", "Run once");
  run.setAttribute("type", "button");
  const always = staticElement("button", "btn-secondary approval-always", "Always allow");
  always.setAttribute("type", "button");
  const reject = staticElement("button", "btn-ghost approval-reject", "Reject");
  reject.setAttribute("type", "button");
  actions.append(run, always, reject);

  const foot = staticElement(
    "div",
    "cmd-foot",
    "Allowlist stores this exact command only. Review patterns in Settings > Approvals.",
  );
  const output = staticElement("pre", "cmd-output");
  card.append(head, command, description, actions, foot, output);
  assistant.append(response, card);
  return staticTemplate(user, assistant);
})();

const AI_EXTRA_USER_TEMPLATE = staticTemplate(staticElement("div", "msg msg-user"));
const AI_EXTRA_RESPONSE_TEMPLATE = (() => {
  const assistant = staticElement("div", "msg msg-ai");
  assistant.append(staticElement("p"));
  return staticTemplate(assistant);
})();

// Initials from the remote account name; a safe DOM-escaped fallback for the
// avatar (the picture URL is never fetched — CSP img-src 'self' + NFR-3).
function googleInitials(name: string): string {
  return name.trim().split(/\s+/).filter(Boolean).slice(0, 2)
    .map((part) => part.charAt(0)).join("").toUpperCase() || "?";
}

function renderGoogleAccount() {
  const chip = $("#user-chip");
  const signedIn = googleAccount !== null;
  const template = $<HTMLTemplateElement>(signedIn
    ? "#google-chip-signed-in-template"
    : "#google-chip-signed-out-template");
  chip.replaceChildren(template.content.cloneNode(true));
  const chipName = chip.querySelector<HTMLElement>(".user-name");
  // textContent assignment escapes the remote name (ADR-8); never innerHTML.
  if (chipName) chipName.textContent = signedIn ? googleAccount!.name : "Google account";
  const avatar = chip.querySelector<HTMLElement>(".avatar");
  if (avatar && signedIn) {
    avatar.textContent = googleInitials(googleAccount!.name);
    // Retain the remote picture URL without fetching it; escAttr guards the attr.
    avatar.setAttribute("data-picture", escAttr(googleAccount!.picture));
  }
  chip.dataset.state = signedIn ? "signed-in" : "signed-out";
  chip.setAttribute("aria-label", signedIn
    ? `Signed in to Google as ${googleAccount!.name}`
    : "Sign in with Google");

  // Larger identity avatar in the popover (initials only; the remote picture is
  // never fetched — NFR-3 / CSP img-src 'self'). Hidden when signed out.
  const popAvatar = $("#pop-avatar");
  if (signedIn) {
    popAvatar.textContent = googleInitials(googleAccount!.name);
    popAvatar.hidden = false;
  } else {
    popAvatar.textContent = "";
    popAvatar.hidden = true;
  }

  $("#google-pop-title").textContent = signedIn
    ? googleAccount!.name
    : "Use hosterm with Google";
  // Popover sub-line: email when signed in, error when a flow failed, else prompt.
  const sub = $("#google-pop-sub");
  if (signedIn) {
    sub.textContent = googleAccount!.email;
    sub.dataset.tone = "";
  } else if (googleError) {
    sub.textContent = googleError;
    sub.dataset.tone = "error";
  } else {
    sub.textContent = "Connect a Google account to back up to Drive.";
    sub.dataset.tone = "";
  }

  const signin = $("#signin-btn") as HTMLButtonElement;
  signin.hidden = signedIn;
  signin.disabled = googlePending;
  const signinLabel = signin.querySelector<HTMLElement>(".signin-label");
  if (signinLabel) signinLabel.textContent = googlePending ? "Signing in…" : "Sign in with Google";
  ($("#signout-btn") as HTMLButtonElement).hidden = !signedIn;
  // Backup/Restore are only reachable with a connected account (they need a
  // live Drive token backend-side).
  $("#drive-actions").hidden = !signedIn;
}

function setGooglePopover(open: boolean, restoreFocus = false) {
  const chip = $("#user-chip");
  $("#signin-popover").hidden = !open;
  chip.setAttribute("aria-expanded", String(open));
  if (open) {
    const signedIn = googleAccount !== null;
    $(signedIn ? "#signout-btn" : "#signin-btn").focus();
  } else if (restoreFocus) {
    chip.focus();
  }
}

async function googleSignIn() {
  if (googlePending) return;
  googlePending = true;
  googleError = "";
  renderGoogleAccount();
  try {
    const acct = await invoke<GoogleAccount>("google_sign_in");
    googleAccount = acct;
    googleError = "";
  } catch (e) {
    googleAccount = null;
    googleError = `Sign-in failed: ${msg(e)}`;
  } finally {
    googlePending = false;
    renderGoogleAccount();
  }
}

async function googleSignOut() {
  try {
    await invoke("google_sign_out");
  } catch (e) {
    // Sign-out is idempotent backend-side; surface a failure but still clear UI.
    googleError = `Sign-out failed: ${msg(e)}`;
  }
  googleAccount = null;
  renderGoogleAccount();
}

// Prompt for a passphrase inside the app (never a native dialog, never
// persisted). `confirm` adds a second field (backup = set a new passphrase).
// Resolves to the entered string, or null if cancelled. The inputs are cleared
// on every close so the plaintext does not linger in the DOM.
function passphrasePrompt(
  title: string,
  message: string,
  okLabel: string,
  confirm: boolean,
  // When true an EMPTY passphrase is a valid submission (backup: protect the
  // backup with this device only). `helper` is a short static line shown under
  // the message (empty => hidden). The confirm field is only enforced when a
  // non-empty passphrase is actually typed.
  allowEmpty = false,
  helper = "",
): Promise<string | null> {
  return new Promise((resolve) => {
    const input = $("#pass-input") as HTMLInputElement;
    const confirmRow = $("#pass-confirm-row");
    const confirmInput = $("#pass-confirm") as HTMLInputElement;
    const eyeBtn = $("#pass-eye") as HTMLButtonElement;
    const confirmEyeBtn = $("#pass-confirm-eye") as HTMLButtonElement;
    const err = $("#pass-error");
    const helperEl = $("#pass-helper");
    input.value = "";
    confirmInput.value = "";
    err.hidden = true;
    err.textContent = "";
    confirmRow.hidden = !confirm;
    helperEl.textContent = helper;
    helperEl.hidden = !helper;
    $("#pass-title").textContent = title;
    $("#pass-msg").textContent = message;
    $("#pass-label").textContent = confirm ? "New passphrase" : "Passphrase";
    ($("#pass-ok") as HTMLButtonElement).textContent = okLabel;

    // Always open masked: the reveal is opt-in per prompt and per field, never
    // sticky, so a passphrase is never unmasked by a prior session's choice.
    // Each eye toggles ONLY its own field — the primary eye the passphrase, the
    // confirm eye the confirm field — so revealing one does not leak the other.
    // The plaintext stays in the input and never leaves it.
    const setFieldRevealed = (
      field: HTMLInputElement,
      btn: HTMLButtonElement,
      on: boolean,
    ) => {
      field.type = on ? "text" : "password";
      btn.setAttribute("aria-pressed", String(on));
      const label = on ? "Hide passphrase" : "Show passphrase";
      btn.setAttribute("aria-label", label);
      btn.setAttribute("title", label);
    };
    setFieldRevealed(input, eyeBtn, false);
    setFieldRevealed(confirmInput, confirmEyeBtn, false);
    const onEye = () =>
      setFieldRevealed(input, eyeBtn, eyeBtn.getAttribute("aria-pressed") !== "true");
    const onConfirmEye = () =>
      setFieldRevealed(confirmInput, confirmEyeBtn, confirmEyeBtn.getAttribute("aria-pressed") !== "true");

    const cleanup = () => {
      // Wipe plaintext from the inputs before the nodes go back to hidden, and
      // re-mask both fields so the next open starts hidden regardless of this
      // choice.
      input.value = "";
      confirmInput.value = "";
      setFieldRevealed(input, eyeBtn, false);
      setFieldRevealed(confirmInput, confirmEyeBtn, false);
      // Reset the optional-passphrase helper so a later prompt (e.g. restore)
      // does not inherit the backup copy.
      helperEl.textContent = "";
      helperEl.hidden = true;
      okBtn.removeEventListener("click", onOk);
      cancelBtn.removeEventListener("click", onCancel);
      eyeBtn.removeEventListener("click", onEye);
      confirmEyeBtn.removeEventListener("click", onConfirmEye);
      input.removeEventListener("keydown", onKey);
      confirmInput.removeEventListener("keydown", onKey);
    };
    const finish = (value: string | null) => {
      cleanup();
      closeOverlay("#pass-dialog");
      passDialogResolve = null;
      resolve(value);
    };
    const onOk = () => {
      const value = input.value;
      if (!value) {
        // Empty is a valid submission only when the prompt allows it (optional
        // backup passphrase = device-only protection). When empty is accepted,
        // the confirm field is irrelevant — no match check.
        if (allowEmpty) { finish(""); return; }
        err.textContent = "Enter a passphrase.";
        err.hidden = false;
        return;
      }
      if (confirm && value !== confirmInput.value) {
        err.textContent = "Passphrases do not match.";
        err.hidden = false;
        return;
      }
      finish(value);
    };
    const onCancel = () => finish(null);
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key === "Enter") { ev.preventDefault(); onOk(); }
    };

    const okBtn = $("#pass-ok") as HTMLButtonElement;
    const cancelBtn = $("#pass-cancel") as HTMLButtonElement;
    okBtn.addEventListener("click", onOk);
    cancelBtn.addEventListener("click", onCancel);
    eyeBtn.addEventListener("click", onEye);
    confirmEyeBtn.addEventListener("click", onConfirmEye);
    input.addEventListener("keydown", onKey);
    confirmInput.addEventListener("keydown", onKey);
    passDialogResolve = onCancel;
    openOverlay("#pass-dialog", input);
  });
}

function setDriveStatus(text: string, tone: "" | "error" | "ok" = "") {
  const el = $("#drive-status");
  el.textContent = text;
  el.dataset.tone = tone;
}

function setDriveBusy(busy: boolean) {
  drivePending = busy;
  ($("#drive-backup-btn") as HTMLButtonElement).disabled = busy;
  ($("#drive-restore-btn") as HTMLButtonElement).disabled = busy;
}

// ---------- shell-level toasts (non-modal backup/restore feedback) ----------
// Cloned from #toast-template; all dynamic strings set via textContent (ADR-8),
// never innerHTML. Toasts never join overlayStack, never trap focus, and are
// not dismissed by Escape (Escape stays owned by overlays — nano lock #4).
type ToastTone = "ok" | "error" | "info";
interface ToastOpts {
  tone: ToastTone;
  title: string;
  msg?: string;
  // Success/info auto-dismiss after this many ms; errors are always persistent.
  timeout?: number;
}
const TOAST_MAX = 3;
const SUCCESS_TOAST_MS = 5000;
// Per-toast dismiss timers so hover/focus can pause and blur can resume.
const toastTimers = new WeakMap<HTMLElement, { remaining: number; startedAt: number; id: number | null }>();

function dropToastNow(toast: HTMLElement) {
  const timer = toastTimers.get(toast);
  if (timer?.id !== null && timer?.id !== undefined) clearTimeout(timer.id);
  toastTimers.delete(toast);
  toast.remove();
}

function removeToast(toast: HTMLElement) {
  const timer = toastTimers.get(toast);
  if (timer?.id !== null && timer?.id !== undefined) clearTimeout(timer.id);
  toastTimers.delete(toast);
  toast.classList.remove("toast-in");
  // Let the exit transition run, then drop the node. Under reduced motion the
  // global kill switch means transitionend never fires, so the timer removes it.
  let done = false;
  const drop = () => { if (done) return; done = true; toast.remove(); };
  toast.addEventListener("transitionend", drop, { once: true });
  window.setTimeout(drop, 160);
}

function armToastTimer(toast: HTMLElement, ms: number) {
  const fire = () => removeToast(toast);
  const state = { remaining: ms, startedAt: Date.now(), id: window.setTimeout(fire, ms) };
  toastTimers.set(toast, state);
  const pause = () => {
    const t = toastTimers.get(toast);
    if (!t || t.id === null) return;
    clearTimeout(t.id);
    t.remaining = Math.max(0, t.remaining - (Date.now() - t.startedAt));
    t.id = null;
  };
  const resume = () => {
    const t = toastTimers.get(toast);
    if (!t || t.id !== null) return;
    t.startedAt = Date.now();
    t.id = window.setTimeout(fire, t.remaining);
  };
  toast.addEventListener("mouseenter", pause);
  toast.addEventListener("mouseleave", resume);
  toast.addEventListener("focusin", pause);
  toast.addEventListener("focusout", resume);
}

function showToast({ tone, title, msg = "", timeout }: ToastOpts) {
  const stack = $("#toast-stack");
  const tpl = $<HTMLTemplateElement>("#toast-template");
  const node = tpl.content.firstElementChild!.cloneNode(true) as HTMLElement;
  node.classList.add(`toast--${tone}`);
  // Errors interrupt (role=alert); success/neutral are polite (role=status).
  node.setAttribute("role", tone === "error" ? "alert" : "status");
  node.querySelector<HTMLElement>(".toast-title")!.textContent = title;
  node.querySelector<HTMLElement>(".toast-msg")!.textContent = msg;
  const close = node.querySelector<HTMLButtonElement>(".toast-close")!;
  close.addEventListener("click", () => removeToast(node));
  // Newest first (prepend). The stack is column-reverse so this reads as
  // stacking upward from the bottom-right corner.
  stack.prepend(node);

  // Eviction: at most TOAST_MAX visible. Never silently drop an error — evict
  // the oldest success/neutral first; if everything visible is an error, keep
  // them all rather than losing evidence. Overflow eviction is synchronous:
  // using the animated remover here would leave the victim in the DOM during
  // this loop, repeatedly select it, and block the UI thread forever.
  const toasts = () => [...stack.querySelectorAll<HTMLElement>(".toast")];
  while (toasts().length > TOAST_MAX) {
    const all = toasts();
    const evictable = all.filter((t) => !t.classList.contains("toast--error"));
    if (evictable.length === 0) break; // all errors — keep them
    dropToastNow(evictable[evictable.length - 1]); // oldest non-error (bottom of DOM)
  }

  // Run the entry transition on the next frame (after layout picks up the
  // initial opacity:0/translateX). Final state is toast-in = opacity:1.
  requestAnimationFrame(() => node.classList.add("toast-in"));

  // Errors are persistent; success/neutral auto-dismiss (pausable on hover/focus).
  const ms = timeout ?? (tone === "error" ? 0 : SUCCESS_TOAST_MS);
  if (ms > 0) armToastTimer(node, ms);
}

async function driveBackup() {
  if (drivePending || googleAccount === null) return;
  const pass = await passphrasePrompt(
    "Back up to Google Drive",
    "Choose a passphrase to encrypt this backup. Drive only ever stores the "
      + "encrypted file — without this passphrase the backup cannot be restored.",
    "Back up",
    true,
    true,
    "Leave empty to protect this backup with this device only.",
  );
  if (pass === null) return;
  setDriveBusy(true);
  setDriveStatus("Encrypting and uploading…");
  try {
    const res = await invoke<BackupResult>("drive_backup", { passphrase: pass });
    // Terminal outcome -> durable shell toast (survives the popover closing).
    // Clear the inline progress line so it does not double the message.
    setDriveStatus("");
    showToast({ tone: "ok", title: "Backup complete", msg: `Backed up ${res.name} (${res.bytes} bytes).` });
  } catch (e) {
    setDriveStatus("");
    // Backup outcome toasts auto-dismiss at 5000ms so the user never has to
    // click Close; the close button stays as an early-dismiss affordance.
    showToast({ tone: "error", title: "Backup failed", msg: msg(e), timeout: SUCCESS_TOAST_MS });
  } finally {
    setDriveBusy(false);
  }
}

// Format an ISO-8601 created_at for display in the local timezone. Falls back
// to the raw string if parsing fails (never throws into the renderer).
function formatBackupWhen(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, {
    year: "numeric", month: "short", day: "numeric",
    hour: "2-digit", minute: "2-digit",
  });
}

// Choose which backup to restore from a list (newest first). Returns the picked
// BackupMeta or null if cancelled. Rows are cloned from the static template and
// their host/time text is set via textContent — Drive-origin strings are
// hostile input (ADR-8), never interpolated into innerHTML. The radiogroup is
// keyboard operable (arrows move selection; the active row is pre-selected).
function restorePicker(backups: BackupMeta[]): Promise<BackupMeta | null> {
  return new Promise((resolve) => {
    const list = $("#restore-list");
    const tpl = $<HTMLTemplateElement>("#restore-row-template");
    const okBtn = $("#restore-ok") as HTMLButtonElement;
    const cancelBtn = $("#restore-cancel") as HTMLButtonElement;
    list.textContent = "";
    let selected = 0;
    const rows: HTMLButtonElement[] = [];

    const applySelection = () => {
      rows.forEach((row, i) => {
        const on = i === selected;
        row.setAttribute("aria-checked", String(on));
        row.tabIndex = on ? 0 : -1;
        row.classList.toggle("sel", on);
      });
    };
    const moveTo = (i: number) => {
      selected = Math.max(0, Math.min(rows.length - 1, i));
      applySelection();
      rows[selected].focus();
    };

    backups.forEach((meta, i) => {
      const row = tpl.content.firstElementChild!.cloneNode(true) as HTMLButtonElement;
      row.querySelector<HTMLElement>(".restore-host")!.textContent = meta.host || "(unknown host)";
      row.querySelector<HTMLElement>(".restore-when")!.textContent = formatBackupWhen(meta.created_at);
      row.addEventListener("click", () => { selected = i; applySelection(); });
      row.addEventListener("dblclick", () => { selected = i; applySelection(); onOk(); });
      row.addEventListener("keydown", (ev: KeyboardEvent) => {
        if (ev.key === "ArrowDown") { ev.preventDefault(); moveTo(selected + 1); }
        else if (ev.key === "ArrowUp") { ev.preventDefault(); moveTo(selected - 1); }
        else if (ev.key === "Enter" || ev.key === " ") { ev.preventDefault(); selected = i; applySelection(); }
      });
      rows.push(row);
      list.appendChild(row);
    });
    applySelection();

    const cleanup = () => {
      okBtn.removeEventListener("click", onOk);
      cancelBtn.removeEventListener("click", onCancel);
      restoreDialogResolve = null;
    };
    const finish = (value: BackupMeta | null) => {
      cleanup();
      closeOverlay("#restore-dialog");
      resolve(value);
    };
    function onOk() { finish(backups[selected] ?? null); }
    function onCancel() { finish(null); }

    okBtn.addEventListener("click", onOk);
    cancelBtn.addEventListener("click", onCancel);
    restoreDialogResolve = onCancel;
    openOverlay("#restore-dialog", rows[0] ?? okBtn);
  });
}

async function driveRestore() {
  if (drivePending || googleAccount === null) return;
  setDriveBusy(true);
  setDriveStatus("Finding backups…");
  try {
    const backups = await invoke<BackupMeta[]>("drive_list_backups");
    if (backups.length === 0) {
      // Neutral empty state, not an error (nano lock #5): persistent info toast
      // so the user can read it; copy is locked.
      setDriveStatus("");
      showToast({
        tone: "info",
        title: "Nothing to restore",
        msg: "No backups on Drive yet. Back up first to enable restore.",
        timeout: 0,
      });
      setDriveBusy(false);
      return;
    }
    // Newest first by created_at (ISO-8601 strings sort lexicographically).
    backups.sort((a, b) => (a.created_at < b.created_at ? 1 : -1));
    setDriveStatus("");
    // Let the user pick WHICH backup (there can be several).
    const chosen = await restorePicker(backups);
    if (chosen === null) { setDriveBusy(false); return; }
    const fileId = chosen.file_id;
    // Overwrite warning: which real files this restore would replace.
    setDriveStatus("Checking which files would be replaced…");
    const preflight = await invoke<RestorePreflight>("drive_restore_preflight", { fileId });
    const existing = preflight.targets.filter((t) => t.exists).map((t) => t.path);
    const warnBody = existing.length
      ? `This will overwrite ${existing.length} existing file(s):\n`
        + existing.join("\n")
        + "\n\nA timestamped copy of the current files is saved first."
      : "No existing files will be overwritten. A pre-restore copy is still made.";
    setDriveStatus("");
    // Only prompt for a passphrase when this backup is user-passphrase mode.
    // Device-keyring backups are restored via the OS keyring (no prompt). The
    // backend's key_mode drives this; absent/legacy => prompt (safe default).
    let passphrase: string | null = null;
    if (chosen.key_mode !== "device-keyring") {
      passphrase = await passphrasePrompt(
        "Restore from Google Drive",
        warnBody,
        "Restore",
        false,
      );
      if (passphrase === null) { setDriveBusy(false); return; }
    } else {
      // No passphrase prompt: confirm the overwrite warning before proceeding.
      const choice = await dialog(
        "Restore from Google Drive",
        warnBody + "\n\nThis backup is protected by this device — no passphrase is needed.",
        [
          { label: "Restore", id: "restore", style: "primary" },
          { label: "Cancel", id: "cancel" },
        ],
        "restore",
        "cancel",
      );
      if (choice !== "restore") { setDriveBusy(false); return; }
    }
    setDriveStatus("Downloading and decrypting…");
    // Device-keyring restore passes no passphrase; the backend uses the keyring.
    const res = await invoke<RestoreResult>("drive_restore", { fileId, passphrase });
    setDriveStatus("");
    showToast({
      tone: "ok",
      title: "Restore complete",
      msg: `Restored ${res.restored.length} file(s). Previous files saved to ${res.pre_restore_dir}.`,
    });
  } catch (e) {
    setDriveStatus("");
    // Restore outcome shares the 5000ms bounded lifetime (same helper), so the
    // terminal failure auto-closes too; close button remains for early dismiss.
    showToast({ tone: "error", title: "Restore failed", msg: msg(e), timeout: SUCCESS_TOAST_MS });
  } finally {
    setDriveBusy(false);
  }
}

function initGoogleAccount() {
  const chip = $("#user-chip");
  renderGoogleAccount();
  $("#drive-backup-btn").addEventListener("click", () => { void driveBackup(); });
  $("#drive-restore-btn").addEventListener("click", () => { void driveRestore(); });
  // Hydrate from the stored account (keyring refresh_token) on load.
  invoke<GoogleAccount | null>("google_account")
    .then((acct) => { googleAccount = acct ?? null; })
    .catch((e) => { googleAccount = null; googleError = `Account check failed: ${msg(e)}`; })
    .finally(renderGoogleAccount);
  chip.addEventListener("click", () => {
    setGooglePopover($("#signin-popover").hasAttribute("hidden"));
  });
  $("#signin-btn").addEventListener("click", () => {
    void googleSignIn();
  });
  $("#signout-btn").addEventListener("click", () => {
    void googleSignOut();
    setGooglePopover(false, true);
  });
  $("#acct-settings-btn").addEventListener("click", () => {
    setGooglePopover(false);
    chip.focus();
    void openSettings();
  });
  document.addEventListener("click", (event) => {
    const popover = $("#signin-popover");
    const target = event.target as Node;
    // When an overlay (e.g. the passphrase dialog) is open over the popover, the
    // overlay owns interaction (nano lock #4) — a click on an overlay control
    // (the Show passphrase toggle) sits outside the popover but must NOT close
    // it, or Escape's focus restore would target a hidden Backup button.
    if (overlayStack.length > 0) return;
    if (!popover.hidden && !chip.contains(target) && !popover.contains(target)) {
      setGooglePopover(false);
    }
  });
  window.addEventListener("keydown", (event) => {
    const popover = $("#signin-popover");
    if (popover.hidden) return;
    // When an overlay (e.g. the passphrase dialog) is open over the popover, the
    // overlay owns Tab and Escape (nano lock #4). The popover must not also close
    // on Escape or it would hide the drive button before focus is restored to it.
    if (overlayStack.length > 0) return;
    if (event.key === "Tab") cycleFocusWithin(popover, event);
    else if (event.key === "Escape") {
      event.preventDefault();
      setGooglePopover(false, true);
    }
  });
}

// ---------- overlays: Escape, focus trap, focus restore ----------
const overlayStack: { el: HTMLElement; restore: HTMLElement | null }[] = [];
let overlayKeydown: ((ev: KeyboardEvent) => void) | null = null;
let dialogResolve: ((id: string) => void) | null = null;
let dialogEscapeId = "cancel";
// Set while the passphrase dialog is open; Escape invokes it (= cancel).
let passDialogResolve: (() => void) | null = null;
// Set while the restore picker is open; Escape invokes it (= cancel).
let restoreDialogResolve: (() => void) | null = null;
// Pending pass-dialog exit-transition fallback timer (reduced-motion never
// fires transitionend); cleared on reopen so it cannot re-hide a live dialog.
let passDialogExitTimer: number | null = null;

function focusables(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>("button, input, textarea, select")].filter(
    (el) => (el as HTMLButtonElement).disabled !== true
      && !el.closest(".hidden")
      && !el.closest("[hidden]")
      && el.getClientRects().length > 0
  );
}

function cycleFocusWithin(root: HTMLElement, event: KeyboardEvent) {
  const candidates = focusables(root);
  if (!candidates.length) return;
  const first = candidates[0];
  const last = candidates[candidates.length - 1];
  const current = document.activeElement;
  const outside = !root.contains(current);
  if (event.shiftKey && (current === first || outside)) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && (current === last || outside)) {
    event.preventDefault();
    first.focus();
  }
}

function onOverlayEscape() {
  const top = overlayStack[overlayStack.length - 1];
  if (!top) return;
  if (top.el.id === "pass-dialog") { passDialogResolve?.(); }
  else if (top.el.id === "restore-dialog") { restoreDialogResolve?.(); }
  else if (top.el.id === "dialog") resolveDialog(dialogEscapeId);
  else if (top.el.id === "modal") cancelEditor();
  else if (top.el.id === "raw-modal") cancelRaw();
  else if (top.el.id === "keys-modal") closeKeys();
  else if (top.el.id === "key-modal") closeKey();
  else if (top.el.id === "key-edit-modal") closeEditKey();
  else if (top.el.id === "ssh-modal") closeOverlay("#ssh-modal"); // Escape = Continue anyway
  else if (top.el.id === "settings-scrim") void closeSettings();
}

function openOverlay(sel: string, focusTarget?: HTMLElement) {
  const el = $(sel);
  if (!overlayStack.some((o) => o.el === el)) {
    overlayStack.push({ el, restore: document.activeElement as HTMLElement | null });
    if (overlayStack.length === 1) $("#app").setAttribute("inert", "");
  }
  el.classList.remove("hidden");
  // Scoped fade+lift entry for the animated overlays (nano lock #2): the display
  // flip lands first, then .ov-enter runs the 120ms transition on the next
  // frame. A stale pending close timer is cleared so a rapid reopen does not
  // re-hide the dialog mid-entry. These dialogs default to opacity:0 in CSS, so
  // they MUST gain .ov-enter here or they render invisible (v0.14.7 regression).
  if (el.id === "pass-dialog" || el.id === "restore-dialog") {
    if (passDialogExitTimer !== null) { clearTimeout(passDialogExitTimer); passDialogExitTimer = null; }
    el.classList.remove("ov-enter");
    requestAnimationFrame(() => {
      // Guard: only animate in if the overlay is still open this frame.
      if (!el.classList.contains("hidden")) el.classList.add("ov-enter");
    });
  }
  if (!overlayKeydown) {
    overlayKeydown = (ev) => {
      const top = overlayStack[overlayStack.length - 1];
      if (!top) return;
      if (ev.key === "Escape") {
        ev.preventDefault();
        onOverlayEscape();
      } else if (ev.key === "Tab") {
        cycleFocusWithin(top.el, ev);
      }
    };
    window.addEventListener("keydown", overlayKeydown);
  }
  const target = focusTarget ?? focusables(el)[0];
  target?.focus();
}

function closeOverlay(sel: string) {
  const el = $(sel);
  const i = overlayStack.findIndex((o) => o.el === el);
  if (i === -1) { el.classList.add("hidden"); return; }
  const [closed] = overlayStack.splice(i, 1);
  // Stack bookkeeping, inert release, and focus restore happen now so the exit
  // animation never races focus restoration (spec §2.1).
  if (overlayStack.length === 0) {
    window.removeEventListener("keydown", overlayKeydown as (ev: KeyboardEvent) => void);
    overlayKeydown = null;
    $("#app").removeAttribute("inert");
  }
  if (el.id === "restore-dialog") {
    // Restore-dialog closes SYNCHRONOUSLY (no deferred fade-out): the restore
    // flow opens #pass-dialog immediately after this close, and a deferred
    // .hidden left this dialog's fixed/inset:0 scrim on top for a frame — on
    // slower Windows CI that scrim intercepted the passphrase OK click and the
    // suite timed out (v0.14.8 Windows-only failure; raced green on Linux). We
    // keep the fade-IN (openOverlay adds .ov-enter) but drop the fade-OUT here.
    // Stripping .ov-enter returns it to opacity:0 while hidden so the next open
    // re-runs the entry transition cleanly.
    el.classList.remove("ov-enter");
    el.classList.add("hidden");
    closed.restore?.focus();
  } else if (el.id === "pass-dialog") {
    // Fade out, then reapply .hidden. Under reduced motion transitionend may not
    // fire, so a 140ms fallback timer reapplies .hidden deterministically. Focus
    // is restored once the dialog is actually hidden so the still-fixed scrim
    // cannot swallow it — deterministic, so it never races the exit (spec §2.1).
    el.classList.remove("ov-enter");
    let hidden = false;
    const hide = () => {
      if (hidden) return;
      hidden = true;
      if (passDialogExitTimer !== null) { clearTimeout(passDialogExitTimer); passDialogExitTimer = null; }
      el.classList.add("hidden");
      closed.restore?.focus();
    };
    el.addEventListener("transitionend", hide, { once: true });
    passDialogExitTimer = window.setTimeout(hide, 140);
  } else {
    el.classList.add("hidden");
    closed.restore?.focus();
  }
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

// Settings and appearance.
function validAccent(value: string): boolean {
  return /^#[0-9a-fA-F]{6}$/.test(value);
}

function relativeLuminance(hex: string): number {
  const channel = (offset: number) => {
    const value = parseInt(hex.slice(offset, offset + 2), 16) / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
}

function guardedAccentInk(accent: string): string {
  const normalized = accent.toLowerCase();
  if (QUICK_ACCENTS.has(normalized)) return CURATED_ACCENT_INK;
  const luminance = relativeLuminance(normalized);
  const darkContrast = (luminance + 0.05) / 0.05;
  const lightContrast = 1.05 / (luminance + 0.05);
  return darkContrast >= lightContrast ? DARK_ACCENT_INK : LIGHT_ACCENT_INK;
}

function applyTheme(preset: ThemePreset, markDirty: boolean) {
  document.documentElement.setAttribute("data-theme", preset);
  document.querySelectorAll<HTMLButtonElement>(".preset").forEach((button) => {
    button.setAttribute("aria-pressed", String(button.dataset.preset === preset));
  });
  appSettings.theme = preset;
  // Theme palettes v1: the terminal well and editor tokens follow the preset.
  // Live terminals repaint via the ITheme option (xterm repaints on set — no
  // Terminal/PTY recreation); open editors re-highlight through the Compartment
  // (no EditorView recreation, doc/cursor/dirty survive). Both loops are
  // no-op-safe while nothing is open (startup/reset path).
  const palette = THEME_TERMINAL[preset];
  const liveTerms = [...sessions.values(), ...splitSessions.values()];
  for (const sess of liveTerms) {
    sess.term.options.theme = palette;
  }
  let editorReconfigured = 0;
  for (const sess of editorSessions.values()) {
    if (!sess.view) continue; // tab exists but the EditorView hasn't opened yet
    sess.view.dispatch({
      effects: highlightCompartment.reconfigure(syntaxHighlighting(CM_HIGHLIGHT[preset])),
    });
    editorReconfigured += 1;
  }
  // Smoke hook (TSD theme-palettes-v1 §2.3): termBg is read back from the first
  // live Terminal when one exists, proving the option swap landed on a real
  // instance instead of only the palette table.
  const hook: { preset: ThemePreset; termBg: string; editorReconfigured: number } = {
    preset,
    termBg: liveTerms[0]?.term.options.theme?.background ?? palettes[preset].terminal.background,
    editorReconfigured,
  };
  (window as unknown as { __hostermTheme?: typeof hook }).__hostermTheme = hook;
  if (markDirty) settingsDirty = true;
}

function applyAccent(hex: string, markDirty: boolean) {
  if (!validAccent(hex)) return;
  const accent = hex.toLowerCase();
  const ink = guardedAccentInk(accent);
  const root = document.documentElement;
  root.style.setProperty("--accent", accent);
  root.style.setProperty("--accent-ink", ink);
  ($("#accent-input") as HTMLInputElement).value = accent;
  $("#accent-hex").textContent = accent;
  const preview = $("#pv-mock");
  preview.style.setProperty("--pa", accent);
  preview.style.setProperty("--pi", ink);
  document.querySelectorAll<HTMLButtonElement>(".accent-chip").forEach((button) => {
    button.setAttribute("aria-pressed", String(button.dataset.accent?.toLowerCase() === accent));
  });
  appSettings.accent = accent;
  if (markDirty) settingsDirty = true;
}

function selectSettingsSection(section: string) {
  document.querySelectorAll<HTMLButtonElement>(".set-nav").forEach((button) => {
    if (button.dataset.section === section) button.setAttribute("aria-current", "page");
    else button.removeAttribute("aria-current");
  });
  const appearance = $("#settings-appearance");
  const approvals = $("#settings-approvals");
  const general = $("#settings-general");
  const placeholder = $("#settings-placeholder");
  const showingAppearance = section === "appearance";
  const showingApprovals = section === "ai";
  const showingGeneral = section === "general";
  appearance.classList.toggle("hidden", !showingAppearance);
  approvals.classList.toggle("hidden", !showingApprovals);
  general.classList.toggle("hidden", !showingGeneral);
  placeholder.classList.toggle("hidden", showingAppearance || showingApprovals || showingGeneral);
  if (showingApprovals) {
    renderApprovals();
    void refreshApprovals();
  } else if (showingGeneral) {
    void populateGeneral();
  } else if (!showingAppearance) {
    const active = document.querySelector<HTMLButtonElement>(`.set-nav[data-section="${section}"]`);
    $("#settings-placeholder-title").textContent = active?.textContent?.trim() || "Settings";
  }
}

// Fill Settings → General: detected local shell + app/platform info.
async function populateGeneral() {
  $("#gen-version").textContent = APP_VERSION;
  $("#gen-platform").textContent = navigator.userAgent.includes("Windows")
    ? "Windows"
    : navigator.userAgent.includes("Mac")
      ? "macOS"
      : "Linux";
  try {
    const info = await invoke<{ shell: string; overridden: boolean }>("local_shell_info");
    $("#gen-shell").textContent = info.shell || "—";
    $("#gen-shell-src").textContent = info.overridden
      ? "HOSTERM_LOCAL_SHELL override"
      : "OS default";
  } catch {
    $("#gen-shell").textContent = "unavailable";
    $("#gen-shell-src").textContent = "—";
  }
}

async function readAndApplySettings() {
  let loaded = DEFAULT_SETTINGS;
  try {
    loaded = await invoke<AppSettings>("read_settings");
  } catch {
    loaded = DEFAULT_SETTINGS;
  }
  const theme: ThemePreset = ["dusk", "slate", "ink", "light"].includes(loaded.theme)
    ? loaded.theme
    : "dusk";
  const accent = validAccent(loaded.accent) ? loaded.accent.toLowerCase() : DEFAULT_SETTINGS.accent;
  appSettings = {
    version: 1,
    theme,
    accent,
    allowlist: Array.isArray(loaded.allowlist) ? loaded.allowlist : [],
  };
  applyTheme(theme, false);
  applyAccent(accent, false);
  settingsDirty = false;
}

async function persistSettings(): Promise<boolean> {
  if (!settingsDirty) return true;
  const status = $("#settings-status");
  const panel = $("#settings-panel");
  const controls = [...panel.querySelectorAll<HTMLButtonElement | HTMLInputElement>("button, input")];
  status.textContent = "";
  panel.setAttribute("aria-busy", "true");
  controls.forEach((control) => { control.disabled = true; });
  try {
    await invoke("write_settings", {
      settings: {
        version: 1,
        theme: appSettings.theme,
        accent: appSettings.accent,
        allowlist: appSettings.allowlist,
      },
    });
    settingsDirty = false;
    return true;
  } catch (e) {
    status.textContent = `Could not save settings: ${msg(e)}. Try again.`;
    return false;
  } finally {
    panel.removeAttribute("aria-busy");
    controls.forEach((control) => { control.disabled = false; });
  }
}

function settingsSnapshot(allowlist: AllowEntry[]): AppSettings {
  return {
    version: 1,
    theme: appSettings.theme,
    accent: appSettings.accent,
    allowlist,
  };
}

async function persistAllowlist(allowlist: AllowEntry[]) {
  await invoke("write_settings", { settings: settingsSnapshot(allowlist) });
  appSettings.allowlist = allowlist;
  try {
    const stored = await invoke<AppSettings>("read_settings");
    if (Array.isArray(stored.allowlist)) appSettings.allowlist = stored.allowlist;
  } catch { /* the write succeeded; keep the submitted list until the next read */ }
}

function renderApprovals() {
  const list = $("#approvals-list");
  const empty = $("#approvals-empty");
  const rendered = document.createDocumentFragment();
  appSettings.allowlist.forEach((entry, index) => {
    const item = APPROVAL_ITEM_TEMPLATE.content.cloneNode(true) as DocumentFragment;
    item.querySelector<HTMLElement>(".approval-host")!.textContent = entry.host;
    item.querySelector<HTMLElement>(".approval-command")!.textContent = entry.command;
    item.querySelector<HTMLButtonElement>(".approval-revoke")!.setAttribute("data-index", String(index));
    rendered.append(item);
  });
  list.replaceChildren(rendered);
  empty.hidden = appSettings.allowlist.length !== 0;
  list.querySelectorAll<HTMLButtonElement>(".approval-revoke").forEach((button) => {
    button.addEventListener("click", () => {
      const index = Number(button.dataset.index);
      if (Number.isInteger(index)) void revokeApproval(index, button);
    });
  });
}

async function refreshApprovals() {
  const status = $("#approvals-status");
  const section = $("#settings-approvals");
  section.setAttribute("aria-busy", "true");
  status.classList.remove("ok");
  status.textContent = "";
  try {
    const loaded = await invoke<AppSettings>("read_settings");
    appSettings.allowlist = Array.isArray(loaded.allowlist) ? loaded.allowlist : [];
    renderApprovals();
    renderAiThread();
  } catch (e) {
    status.textContent = `Could not load approvals: ${msg(e)}. Try again.`;
  } finally {
    section.removeAttribute("aria-busy");
  }
}

async function revokeApproval(index: number, button: HTMLButtonElement) {
  const status = $("#approvals-status");
  const next = appSettings.allowlist.slice();
  if (index < 0 || index >= next.length) return;
  next.splice(index, 1);
  button.disabled = true;
  status.classList.remove("ok");
  status.textContent = "";
  try {
    await persistAllowlist(next);
    status.classList.add("ok");
    status.textContent = "Approval revoked.";
    renderApprovals();
    renderAiThread();
  } catch (e) {
    button.disabled = false;
    status.textContent = `Could not revoke approval: ${msg(e)}. Try again.`;
  }
}

async function openSettings() {
  await settingsReady;
  selectSettingsSection("appearance");
  $("#settings-status").textContent = "";
  openOverlay("#settings-scrim", $("#settings-panel"));
}

async function closeSettings() {
  if (settingsClosing) return;
  settingsClosing = true;
  const saved = await persistSettings();
  settingsClosing = false;
  if (saved) closeOverlay("#settings-scrim");
}

function initSettings() {
  $("#settings-close").addEventListener("click", () => void closeSettings());
  $("#settings-done").addEventListener("click", () => void closeSettings());
  $("#settings-scrim").addEventListener("click", (event) => {
    if (event.target === event.currentTarget) void closeSettings();
  });
  document.querySelectorAll<HTMLButtonElement>(".set-nav").forEach((button) => {
    button.addEventListener("click", () => selectSettingsSection(button.dataset.section || "appearance"));
  });
  document.querySelectorAll<HTMLButtonElement>(".preset").forEach((button) => {
    button.addEventListener("click", () => {
      if (button.getAttribute("aria-disabled") === "true") return;
      applyTheme((button.dataset.preset || "dusk") as ThemePreset, true);
    });
  });
  document.querySelectorAll<HTMLButtonElement>(".accent-chip").forEach((button) => {
    button.addEventListener("click", () => applyAccent(button.dataset.accent || "", true));
  });
  ($("#accent-input") as HTMLInputElement).addEventListener("input", (event) => {
    applyAccent((event.target as HTMLInputElement).value, true);
  });
  $("#accent-reset").addEventListener("click", () => {
    applyTheme("dusk", true);
    applyAccent(DEFAULT_SETTINGS.accent, true);
  });
  settingsReady = readAndApplySettings();
}

function fitActiveSessions() {
  if (activeTab == null) return;
  for (const session of [sessions.get(activeTab), splitSessions.get(activeTab)]) {
    if (!session) continue;
    session.fit.fit();
    if (session.ptyId != null) {
      invoke("pty_resize", { id: session.ptyId, cols: session.term.cols, rows: session.term.rows });
    }
  }
}

type SidePanelMode = "terminal" | "snippets" | "ai";

function setSidePanel(mode: SidePanelMode) {
  const snippetOpen = mode === "snippets";
  const aiOpen = mode === "ai";
  const snippetDrawer = $("#snippet-drawer");
  const aiDrawer = $("#ai-drawer");
  snippetDrawer.hidden = !snippetOpen;
  snippetDrawer.classList.toggle("open", snippetOpen);
  aiDrawer.hidden = !aiOpen;
  aiDrawer.classList.toggle("open", aiOpen);
  $("#mode-terminal").setAttribute("aria-pressed", String(mode === "terminal"));
  $("#mode-snippets").setAttribute("aria-pressed", String(snippetOpen));
  $("#mode-snippets").setAttribute("aria-expanded", String(snippetOpen));
  $("#mode-ai").setAttribute("aria-pressed", String(aiOpen));
  $("#mode-ai").setAttribute("aria-expanded", String(aiOpen));
  setTimeout(fitActiveSessions, 0);
}

function setSnippetDrawer(open: boolean) {
  setSidePanel(open ? "snippets" : "terminal");
}

function setAiDrawer(open: boolean) {
  setSidePanel(open ? "ai" : "terminal");
}

function aiAllowlisted(host: string, command: string): boolean {
  return appSettings.allowlist.some((entry) => entry.host === host && entry.command === command);
}

function aiCardState(proposal: AiProposal) {
  const allowed = aiAllowlisted(proposal.host, proposal.command);
  const decision = aiDecisions.get(proposal.id) ?? "proposed";
  const result = aiResults.get(proposal.id) ?? "";
  let cardClass = "";
  let stateClass = "warn";
  let stateLabel = "AWAITING APPROVAL";
  let showActions = true;

  if (allowed) {
    stateClass = "ok";
    stateLabel = "ALWAYS ALLOWED";
    showActions = false;
  } else if (decision === "ran") {
    stateClass = "ok";
    stateLabel = "RAN ONCE";
    showActions = false;
  } else if (decision === "rejected") {
    cardClass = " rejected";
    stateClass = "danger";
    stateLabel = "REJECTED";
    showActions = false;
  } else if (decision === "refused") {
    cardClass = " refused";
    stateClass = "danger";
    stateLabel = "REFUSED";
    showActions = false;
  } else {
    cardClass = " awaiting";
  }

  return { cardClass, stateClass, stateLabel, showActions, result };
}

function renderAiThread() {
  const thread = $("#ai-thread");
  const rendered = document.createDocumentFragment();
  AI_PROPOSALS.forEach((proposal) => {
    const content = AI_PROPOSAL_TEMPLATE.content.cloneNode(true) as DocumentFragment;
    const cardState = aiCardState(proposal);
    content.querySelector<HTMLElement>(".msg-user")!.textContent = proposal.prompt;
    content.querySelector<HTMLElement>(".msg-ai > p")!.textContent = proposal.response;

    const card = content.querySelector<HTMLElement>(".cmd-card")!;
    card.setAttribute("class", `cmd-card${cardState.cardClass}`);
    card.setAttribute("data-ai-id", proposal.id);
    card.querySelector<HTMLElement>(".cmd-scope")!.textContent = proposal.host;
    const state = card.querySelector<HTMLElement>(".cmd-state")!;
    state.setAttribute("class", `cmd-state ${cardState.stateClass}`);
    state.textContent = cardState.stateLabel;
    card.querySelector<HTMLElement>(".cmd-line")!.textContent = proposal.command;
    card.querySelector<HTMLElement>(".cmd-desc")!.textContent = proposal.description;
    if (!cardState.showActions) {
      card.querySelector(".cmd-actions")!.remove();
      card.querySelector(".cmd-foot")!.remove();
    }
    const output = card.querySelector<HTMLElement>(".cmd-output")!;
    if (cardState.result) output.textContent = cardState.result;
    else output.remove();
    rendered.append(content);
  });

  aiExtraMessages.forEach((message) => {
    const template = message.role === "user" ? AI_EXTRA_USER_TEMPLATE : AI_EXTRA_RESPONSE_TEMPLATE;
    const content = template.content.cloneNode(true) as DocumentFragment;
    content.querySelector<HTMLElement>(message.role === "user" ? ".msg-user" : ".msg-ai > p")!.textContent = message.text;
    rendered.append(content);
  });
  thread.replaceChildren(rendered);

  thread.querySelectorAll<HTMLElement>(".cmd-card").forEach((card) => {
    const id = card.dataset.aiId;
    if (!id) return;
    card.querySelector<HTMLButtonElement>(".approval-run")?.addEventListener("click", () => void runAiOnce(id, card));
    card.querySelector<HTMLButtonElement>(".approval-always")?.addEventListener("click", () => void alwaysAllowAi(id, card));
    card.querySelector<HTMLButtonElement>(".approval-reject")?.addEventListener("click", () => rejectAi(id));
  });
}

function visibleAiCommand(card: HTMLElement): { host: string; command: string } {
  return {
    host: card.querySelector(".cmd-scope")?.textContent ?? "",
    command: card.querySelector(".cmd-line")?.textContent ?? "",
  };
}

function setAiCardBusy(card: HTMLElement, label: string) {
  card.setAttribute("aria-busy", "true");
  card.querySelectorAll<HTMLButtonElement>("button").forEach((button) => { button.disabled = true; });
  const state = card.querySelector(".cmd-state");
  if (state) {
    state.className = "cmd-state warn";
    state.textContent = label;
  }
}

function refuseAiCommand(id: string) {
  aiDecisions.set(id, "refused");
  aiResults.set(id, "Host alias failed the argv safety check. Nothing was executed or saved.");
  renderAiThread();
}

function liveTerminalForHost(host: string): Session | null {
  return [...sessions.values()].reverse().find((session) => session.host === host && session.ptyId != null)
    ?? [...splitSessions.values()].reverse().find((session) => session.host === host && session.ptyId != null)
    ?? null;
}

async function executeAiCommand(host: string, command: string) {
  if (!sftpSafe(host)) throw new Error("unsafe host alias");
  let session = liveTerminalForHost(host);
  if (!session) session = await openTerminal(host);
  if (!session || session.ptyId == null) throw new Error("SSH terminal did not start");
  activateTab(session.id);
  await invoke("pty_write", { id: session.ptyId, data: `${command}\r` });
}

async function runAiOnce(id: string, card: HTMLElement) {
  const { host, command } = visibleAiCommand(card);
  if (!sftpSafe(host)) {
    refuseAiCommand(id);
    return;
  }
  setAiCardBusy(card, "RUNNING ONCE");
  try {
    await executeAiCommand(host, command);
    aiDecisions.set(id, "ran");
    aiResults.set(id, "Command sent once to the real-ssh terminal.");
  } catch (e) {
    aiDecisions.set(id, "proposed");
    aiResults.set(id, `Could not run command: ${msg(e)}. Review the terminal and try again.`);
  }
  renderAiThread();
}

async function alwaysAllowAi(id: string, card: HTMLElement) {
  const { host, command } = visibleAiCommand(card);
  if (!sftpSafe(host)) {
    refuseAiCommand(id);
    return;
  }
  setAiCardBusy(card, "SAVING APPROVAL");
  let alreadyAllowed = aiAllowlisted(host, command);
  if (!alreadyAllowed) {
    try {
      const stored = await invoke<AppSettings>("read_settings");
      if (Array.isArray(stored.allowlist)) appSettings.allowlist = stored.allowlist;
      alreadyAllowed = aiAllowlisted(host, command);
    } catch { /* keep the current snapshot; persistAllowlist retains its existing error path */ }
  }
  if (!alreadyAllowed) {
    const next = [...appSettings.allowlist, { host, command, added_at: "" }];
    try {
      await persistAllowlist(next);
    } catch (e) {
      aiDecisions.set(id, "proposed");
      aiResults.set(id, `Could not save approval: ${msg(e)}. Nothing was executed.`);
      renderAiThread();
      return;
    }
  }

  aiDecisions.delete(id);
  try {
    await executeAiCommand(host, command);
    aiResults.set(id, "Approval saved. Command sent to the real-ssh terminal.");
  } catch (e) {
    aiResults.set(id, `Approval saved, but the command did not run: ${msg(e)}.`);
  }
  renderAiThread();
}

function rejectAi(id: string) {
  aiDecisions.set(id, "rejected");
  aiResults.set(id, "Rejected. Nothing was executed or saved.");
  renderAiThread();
}

function sendAiStub() {
  const input = $("#ai-input") as HTMLInputElement;
  if (!input.value.trim()) return;
  aiExtraMessages.push(
    { role: "user", text: input.value },
    { role: "ai", text: "The AI backend is not connected. This mock did not send or execute your request." },
  );
  input.value = "";
  renderAiThread();
  $("#ai-thread").scrollTop = $("#ai-thread").scrollHeight;
}

function initAiAssistant() {
  $("#mode-ai").addEventListener("click", () => {
    void settingsReady.then(() => {
      renderAiThread();
      setAiDrawer(true);
      ($("#ai-input") as HTMLInputElement).focus();
    });
  });
  $("#ai-close").addEventListener("click", () => {
    setAiDrawer(false);
    $("#mode-ai").focus();
  });
  $("#ai-send").addEventListener("click", sendAiStub);
  ($("#ai-input") as HTMLInputElement).addEventListener("keydown", (event) => {
    if (event.key !== "Enter" || event.isComposing) return;
    event.preventDefault();
    sendAiStub();
  });
  void settingsReady.then(renderAiThread);
}

async function copySnippet(button: HTMLButtonElement, command: string) {
  button.disabled = true;
  try {
    await invoke("clipboard_write", { text: command });
  } catch {
    button.disabled = false;
    return;
  }
  button.disabled = false;
  const previous = snippetCopyTimers.get(button);
  if (previous !== undefined) clearTimeout(previous);
  button.classList.add("done");
  button.innerHTML = ICONS.check;
  button.title = "Copied";
  button.setAttribute("aria-label", "Copied");
  const timer = window.setTimeout(() => {
    button.classList.remove("done");
    button.innerHTML = ICONS.copy;
    button.title = "Copy command";
    button.setAttribute("aria-label", "Copy command");
    snippetCopyTimers.delete(button);
  }, 900);
  snippetCopyTimers.set(button, timer);
}

// Group user snippets by category, preserving first-seen category order so the
// list is stable across edits.
function groupUserSnippets(rows: UserSnippet[]): { category: string; items: UserSnippet[] }[] {
  const order: string[] = [];
  const byCat = new Map<string, UserSnippet[]>();
  for (const r of rows) {
    const cat = r.category.trim() || "Custom";
    if (!byCat.has(cat)) { byCat.set(cat, []); order.push(cat); }
    byCat.get(cat)!.push(r);
  }
  return order.map((category) => ({ category, items: byCat.get(category)! }));
}

function renderSnippets() {
  const query = ($("#snippet-filter") as HTMLInputElement).value;
  const needle = query.toLowerCase();
  const commands: string[] = [];
  let visibleCount = 0;

  // Bundled, read-only groups (no edit/delete — ADR-19).
  const bundledMarkup = snippetGroups.map((group) => {
    const items = group.items.filter((item) =>
      item.name.toLowerCase().includes(needle) || item.command.toLowerCase().includes(needle)
    );
    if (items.length === 0) return "";
    visibleCount += items.length;
    const rows = items.map((item) => {
      commands.push(item.command);
      return `<div class="snippet-item">` +
        `<div class="snip-top"><span class="snip-name">${esc(item.name)}</span>` +
        `<button class="copy-btn" title="Copy command" aria-label="Copy command">${ICONS.copy}</button></div>` +
        `<code class="snip-cmd">${esc(item.command)}</code>` +
        `</div>`;
    }).join("");
    return `<div class="snip-group-label">${esc(group.category.toUpperCase())}</div>${rows}`;
  }).join("");

  // User groups (CRUD): copy + edit + delete per row.
  const userMarkup = groupUserSnippets(userSnippets).map((group) => {
    const items = group.items.filter((item) =>
      item.name.toLowerCase().includes(needle) || item.command.toLowerCase().includes(needle)
    );
    if (items.length === 0) return "";
    visibleCount += items.length;
    const rows = items.map((item) => {
      commands.push(item.command);
      return `<div class="snippet-item" data-user-id="${esc(item.id)}">` +
        `<div class="snip-top"><span class="snip-name">${esc(item.name)}</span>` +
        `<span class="snip-actions">` +
        `<button class="copy-btn" title="Copy command" aria-label="Copy command">${ICONS.copy}</button>` +
        `<button class="snip-edit" title="Edit snippet" aria-label="Edit snippet">${ICONS.pencil}</button>` +
        `<button class="snip-del" title="Delete snippet" aria-label="Delete snippet">${ICONS.trash}</button>` +
        `</span></div>` +
        `<code class="snip-cmd">${esc(item.command)}</code>` +
        `</div>`;
    }).join("");
    return `<div class="snip-group-label">${esc(group.category.toUpperCase())}</div>${rows}`;
  }).join("");

  $("#snippet-list").innerHTML = bundledMarkup + userMarkup;
  $("#snippet-count").textContent = String(visibleCount);
  const empty = $("#snippet-empty");
  empty.hidden = visibleCount !== 0;
  empty.innerHTML = visibleCount !== 0
    ? ""
    : query
      ? `No snippets match "${esc(query)}"`
      : "No snippets yet. Use + New to create one.";
  document.querySelectorAll<HTMLButtonElement>("#snippet-list .copy-btn").forEach((button, index) => {
    const command = commands[index];
    if (command !== undefined) button.addEventListener("click", () => void copySnippet(button, command));
  });
  // Wire edit/delete for user rows.
  document.querySelectorAll<HTMLElement>("#snippet-list .snippet-item[data-user-id]").forEach((row) => {
    const id = row.getAttribute("data-user-id") ?? "";
    row.querySelector<HTMLButtonElement>(".snip-edit")?.addEventListener("click", () => openSnippetForm(id));
    row.querySelector<HTMLButtonElement>(".snip-del")?.addEventListener("click", () => void deleteSnippet(id));
  });
}

// ---- Snippet CRUD form (create/edit) ----
function openSnippetForm(editId: string | null) {
  snippetEditId = editId;
  const form = $("#snippet-form") as HTMLFormElement;
  const nameEl = $("#snippet-f-name") as HTMLInputElement;
  const cmdEl = $("#snippet-f-cmd") as HTMLTextAreaElement;
  const catEl = $("#snippet-f-cat") as HTMLInputElement;
  const errEl = $("#snippet-form-err");
  errEl.hidden = true; errEl.textContent = "";
  if (editId) {
    const row = userSnippets.find((s) => s.id === editId);
    $("#snippet-form-title").textContent = "Edit snippet";
    nameEl.value = row?.name ?? "";
    cmdEl.value = row?.command ?? "";
    catEl.value = row?.category ?? "";
  } else {
    $("#snippet-form-title").textContent = "New snippet";
    nameEl.value = ""; cmdEl.value = ""; catEl.value = "";
  }
  form.hidden = false;
  nameEl.focus();
}

function closeSnippetForm() {
  snippetEditId = null;
  ($("#snippet-form") as HTMLFormElement).hidden = true;
}

async function saveSnippetForm() {
  const name = ($("#snippet-f-name") as HTMLInputElement).value.trim();
  const command = ($("#snippet-f-cmd") as HTMLTextAreaElement).value.trim();
  const category = ($("#snippet-f-cat") as HTMLInputElement).value.trim();
  const errEl = $("#snippet-form-err");
  if (!name) { errEl.textContent = "Name is required."; errEl.hidden = false; return; }
  if (!command) { errEl.textContent = "Command is required."; errEl.hidden = false; return; }

  let next: UserSnippet[];
  if (snippetEditId) {
    next = userSnippets.map((s) => s.id === snippetEditId ? { ...s, name, command, category } : s);
  } else {
    next = [...userSnippets, { id: "", name, command, category }];
  }
  try {
    await invoke("write_user_snippets", { snippets: next });
    userSnippets = await invoke<UserSnippet[]>("read_user_snippets");
    closeSnippetForm();
    renderSnippets();
  } catch (e) {
    errEl.textContent = `Could not save: ${msg(e)}`;
    errEl.hidden = false;
  }
}

async function deleteSnippet(id: string) {
  const row = userSnippets.find((s) => s.id === id);
  if (!row) return;
  if (!confirm(`Delete snippet "${row.name}"?`)) return;
  const next = userSnippets.filter((s) => s.id !== id);
  try {
    await invoke("write_user_snippets", { snippets: next });
    userSnippets = await invoke<UserSnippet[]>("read_user_snippets");
    if (snippetEditId === id) closeSnippetForm();
    renderSnippets();
  } catch {
    /* keep the row on failure; the list reflects last-known-good state */
  }
}

async function openSnippetDrawer() {
  setSnippetDrawer(true);
  const drawer = $("#snippet-drawer");
  drawer.setAttribute("aria-busy", "true");
  $("#snippet-list").innerHTML = "";
  $("#snippet-count").textContent = "0";
  const empty = $("#snippet-empty");
  empty.hidden = false;
  empty.textContent = "Loading snippets…";
  ($("#snippet-filter") as HTMLInputElement).focus();
  try {
    const [loaded, loadedUser] = await Promise.all([
      invoke<SnippetGroup[]>("read_snippets"),
      invoke<UserSnippet[]>("read_user_snippets"),
    ]);
    snippetGroups = Array.isArray(loaded) ? loaded : [];
    userSnippets = Array.isArray(loadedUser) ? loadedUser : [];
    renderSnippets();
  } catch {
    snippetGroups = [];
    userSnippets = [];
    empty.hidden = false;
    empty.textContent = "Could not load snippets. Close the panel and try again.";
  } finally {
    drawer.removeAttribute("aria-busy");
  }
}

function initSnippets() {
  $("#mode-snippets").addEventListener("click", () => void openSnippetDrawer());
  $("#mode-terminal").addEventListener("click", () => setSnippetDrawer(false));
  $("#snippet-close").addEventListener("click", () => {
    setSnippetDrawer(false);
    $("#mode-snippets").focus();
  });
  ($("#snippet-filter") as HTMLInputElement).addEventListener("input", renderSnippets);
  // CRUD form wiring.
  $("#snippet-new").addEventListener("click", () => openSnippetForm(null));
  $("#snippet-f-cancel").addEventListener("click", () => closeSnippetForm());
  ($("#snippet-form") as HTMLFormElement).addEventListener("submit", (e) => {
    e.preventDefault();
    void saveSnippetForm();
  });
  window.addEventListener("keydown", (event) => {
    if (event.key !== "/" || !$("#snippet-drawer").classList.contains("open")) return;
    const target = event.target as HTMLElement;
    if (target.matches("input, textarea, select") || target.isContentEditable) return;
    event.preventDefault();
    ($("#snippet-filter") as HTMLInputElement).focus();
  });
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

// ---------- Ansible v1 panel (FE-ANS) ----------
// Frozen IPC contract (TSD §4): three backend commands.
//   check_ansible(): { missing: boolean }
//   ansible_run({ req }): number (pty id); errors as "ansible:<code>"
//   ansible_history({ op }): AnsibleHistoryEntry[]
// The playbook picker needs a native file-open + UTF-8/size validation; the
// backend exposes that as pick_playbook() -> { path, text } (see FR-ANS5). The
// frontend re-validates the returned text (≤2 MiB, valid UTF-8) before any run.
interface AnsibleCheck { missing: boolean }
interface AnsibleRunReq {
  mode: "check" | "real";
  playbook: string;
  aliases: string[];
  limit: string;
  cols: number;
  rows: number;
}
interface AnsibleHistoryEntry {
  playbook: string;
  targets: string;
  mode: "check" | "real";
  started_at: string;
  duration_ms: number;
  rc: number;
  recap: string;
}
type AnsibleHistoryOp = { kind: "read" } | { kind: "append"; entry: AnsibleHistoryEntry };
// Result of the native playbook picker (backend pick_playbook).
interface PlaybookPick { path: string; text: string }

const ANS_MAX_PLAYBOOK_BYTES = 2 * 1024 * 1024; // 2 MiB (FR-ANS5)

// Dry-run-WAJIB gate state (TSD §5 / SDD §14.5, VERBATIM). checkedKey is the
// identity of the last SUCCESSFUL dry-run; Run-real is enabled iff it still
// matches the current playbook+targets. This is a SECURITY layer: the
// permission-widening real run is gated behind a cheap safe dry-run + an
// explicit human confirm, and never auto-runs.
const ansState = {
  checkAnsible: null as AnsibleCheck | null,
  playbookPath: "" as string,
  selected: new Set<string>(),      // chosen host aliases (eligible only)
  checkedKey: null as string | null, // last successful dry-run key, or null
  running: false as boolean,         // a run is in flight (disable both buttons)
};

// Current gate key: playbookPath + NUL + comma-joined sorted targets.
function ansCurrentKey(): string {
  const targets = [...ansState.selected].sort().join(",");
  return `${ansState.playbookPath}\u0000${targets}`;
}

// Reset the gate (TSD §5: any playbook OR target change nulls checkedKey).
function ansResetGate() {
  ansState.checkedKey = null;
  ansRenderGate();
}

async function openAnsible() {
  if (!$("#ansible-modal").classList.contains("hidden")) return; // never stack
  openOverlay("#ansible-modal");
  await ansRefreshCheck();
  ansRenderHosts();
  ansRenderGate();
  void ansLoadHistory();
}

// FR-ANS1/ANS2: on open, check the binary. missing → show the per-OS guide and
// BLOCK all runs; present → show the runner body. Mirrors the W2-A pattern.
async function ansRefreshCheck() {
  try {
    ansState.checkAnsible = await invoke<AnsibleCheck>("check_ansible");
  } catch (e) {
    // Treat a failed check as missing — fail-closed, never let a run proceed.
    ansState.checkAnsible = { missing: true };
    console.error("check_ansible failed:", e);
  }
  const missing = ansState.checkAnsible.missing;
  $("#ans-guide").classList.toggle("hidden", !missing);
  $("#ans-body").classList.toggle("hidden", missing);
  if (missing) ansRenderGuide();
}

// Per-OS install guide (FR-ANS1). Static text set via textContent (ADR-8).
function ansRenderGuide() {
  const host = $("#ans-guide-os");
  host.innerHTML = "";
  const ua = navigator.userAgent;
  const isMac = ua.includes("Mac OS X") || ua.includes("Macintosh");
  const isWin = ua.includes("Windows");
  const steps: { label: string; cmd: string }[] = isMac
    ? [{ label: "macOS — Homebrew", cmd: "brew install ansible" }]
    : isWin
      ? [{ label: "Windows — install under WSL (Ansible has no native Windows control node)", cmd: "wsl -- sudo apt install ansible" }]
      : [
          { label: "Debian / Ubuntu (apt)", cmd: "sudo apt install ansible" },
          { label: "Fedora / RHEL (dnf)", cmd: "sudo dnf install ansible" },
          { label: "Arch (pacman)", cmd: "sudo pacman -S ansible" },
          { label: "Any Unix (pipx)", cmd: "pipx install --include-deps ansible" },
        ];
  for (const s of steps) {
    const row = document.createElement("div");
    row.className = "ans-guide-step";
    const label = document.createElement("div");
    label.className = "ans-guide-label";
    label.textContent = s.label;
    const code = document.createElement("code");
    code.className = "ans-guide-cmd";
    code.textContent = s.cmd;
    row.appendChild(label);
    row.appendChild(code);
    host.appendChild(row);
  }
}

// FR-ANS4: one row per eligible sidebar host (wildcards excluded). Password-auth
// hosts are shown but marked skipped and NOT selectable. Backend replans
// authoritatively; the frontend only reflects.
function ansRenderHosts() {
  const list = $("#ans-host-list");
  list.innerHTML = "";
  const tpl = $("#ans-host-row-template") as HTMLTemplateElement;
  const rows = hosts
    .filter((h) => !h.name.includes("*") && !h.name.includes("?"))
    .sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
  // Drop selections that no longer map to an eligible host (picker race).
  const eligibleNow = new Set(rows.filter((h) => !h.has_password).map((h) => h.name));
  for (const a of [...ansState.selected]) if (!eligibleNow.has(a)) ansState.selected.delete(a);

  for (const h of rows) {
    const frag = tpl.content.cloneNode(true) as DocumentFragment;
    const row = frag.querySelector(".ans-host-row") as HTMLLabelElement;
    const cb = frag.querySelector(".ans-host-cb") as HTMLInputElement;
    const skipped = h.has_password; // v1: password-auth hosts are not eligible
    cb.checked = ansState.selected.has(h.name);
    cb.disabled = skipped;
    cb.setAttribute("aria-label", `Select ${h.name} as an Ansible target`);
    cb.addEventListener("change", () => {
      if (cb.checked) ansState.selected.add(h.name);
      else ansState.selected.delete(h.name);
      ansResetGate(); // target change resets the dry-run gate (TSD §5)
    });
    (frag.querySelector(".ans-host-alias") as HTMLElement).textContent = h.name;
    const conn = `${h.user || "?"}@${h.host_name || h.name}${h.port ? `:${h.port}` : ""}`;
    (frag.querySelector(".ans-host-conn") as HTMLElement).textContent = conn;
    (frag.querySelector(".ans-host-auth") as HTMLElement).textContent = h.auth_method || (h.has_password ? "password" : "key");
    if (skipped) {
      row.classList.add("skipped");
      const skip = frag.querySelector(".ans-host-skip") as HTMLElement;
      skip.hidden = false;
      skip.textContent = "skipped · password auth (key/agent only in v1)";
    }
    list.appendChild(frag);
  }
}

// Render the gate (TSD §5): Run-real enabled iff checkedKey === current key and
// a playbook + ≥1 target are chosen and no run is in flight.
function ansRenderGate() {
  const hasPlaybook = ansState.playbookPath.length > 0;
  const hasTargets = ansState.selected.size > 0;
  const dry = $("#ans-dry-run") as HTMLButtonElement;
  const real = $("#ans-run-real") as HTMLButtonElement;
  dry.disabled = ansState.running || !hasPlaybook || !hasTargets;
  const gateOpen = ansState.checkedKey !== null && ansState.checkedKey === ansCurrentKey();
  real.disabled = ansState.running || !hasPlaybook || !hasTargets || !gateOpen;
  const hint = $("#ans-gate-hint");
  if (!hasPlaybook || !hasTargets) {
    hint.textContent = "Choose a playbook and at least one target to begin.";
  } else if (gateOpen) {
    hint.textContent = "Dry-run passed for this playbook + targets — a real run is unlocked.";
  } else {
    hint.textContent = "A successful dry-run of the current playbook + targets is required before a real run. Changing either resets the gate.";
  }
}

// FR-ANS5: native playbook picker. Backend returns { path, text }; the frontend
// re-validates extension + UTF-8 + size and names the cause of any rejection
// BEFORE it is accepted (no ansible_run until a valid playbook is set).
async function ansPickPlaybook() {
  const err = $("#ans-playbook-err");
  err.hidden = true;
  err.textContent = "";
  let pick: PlaybookPick;
  try {
    pick = await invoke<PlaybookPick>("pick_playbook");
  } catch (e) {
    const m = msg(e);
    if (m === "ansible:cancelled" || m === "cancelled") return; // user dismissed picker
    err.textContent = `Could not open the playbook: ${m}.`;
    err.hidden = false;
    return;
  }
  const lower = pick.path.toLowerCase();
  if (!lower.endsWith(".yml") && !lower.endsWith(".yaml")) {
    err.textContent = "Rejected: the playbook must be a .yml or .yaml file.";
    err.hidden = false;
    return;
  }
  // Size: measure UTF-8 bytes, not code units (FR-ANS5 is a byte threshold).
  const bytes = new TextEncoder().encode(pick.text).length;
  if (bytes > ANS_MAX_PLAYBOOK_BYTES) {
    err.textContent = `Rejected: the playbook is ${(bytes / 1048576).toFixed(2)} MiB, over the 2 MiB limit.`;
    err.hidden = false;
    return;
  }
  // Invalid UTF-8 / binary: a NUL byte or a U+FFFD replacement char means the
  // bytes did not decode as clean UTF-8 text.
  if (pick.text.includes("\u0000") || pick.text.includes("\uFFFD")) {
    err.textContent = "Rejected: the file is not valid UTF-8 text (looks binary).";
    err.hidden = false;
    return;
  }
  ansState.playbookPath = pick.path;
  ($("#ans-playbook-path") as HTMLElement).textContent = pick.path;
  ansResetGate(); // playbook change resets the dry-run gate (TSD §5)
}

// Comma-joined --limit expression from the selection (one argv element backend-side).
function ansLimitExpr(): string {
  return [...ansState.selected].sort().join(",");
}

// Parse the PLAY RECAP block off streamed PTY output (FR-ANS8). The recap lines
// look like: "<host> : ok=5 changed=2 unreachable=0 failed=0 ...". Returns the
// trimmed recap text plus a "N hosts will change, M tasks changed" summary.
function ansParseRecap(output: string): { recap: string; summary: string } {
  const idx = output.lastIndexOf("PLAY RECAP");
  if (idx === -1) return { recap: "", summary: "" };
  const block = output.slice(idx);
  const lines = block.split(/\r?\n/);
  let hostsChanging = 0;
  let tasksChanged = 0;
  const recapLines: string[] = [];
  for (const line of lines) {
    const m = line.match(/^\s*(\S+)\s*:\s*(.*\bchanged=(\d+)\b.*)$/);
    if (!m) {
      // keep the "PLAY RECAP ***" header line, stop at the first blank after rows
      if (/^PLAY RECAP/.test(line)) recapLines.push(line.trim());
      else if (recapLines.length > 1 && line.trim() === "") break;
      continue;
    }
    recapLines.push(line.trim());
    const changed = Number(m[3]);
    tasksChanged += changed;
    if (changed > 0) hostsChanging += 1;
  }
  const summary = `${hostsChanging} host${hostsChanging === 1 ? "" : "s"} will change, ${tasksChanged} task${tasksChanged === 1 ? "" : "s"} changed`;
  return { recap: recapLines.join("\n"), summary };
}

// Open a PTY terminal tab bound to an ALREADY-SPAWNED pty id (returned by
// ansible_run). Reuses the local-terminal rendering path (openTerminal) but the
// backend owns the spawn, so we attach output/exit listeners to the given id.
// Accumulates raw output so the caller can parse the PLAY RECAP on exit.
function openAnsibleTab(ptyId: number, label: string, onExit: (output: string) => void) {
  const tabId = nextTab++;
  const term = new Terminal({
    cursorBlink: !REDUCED_MOTION,
    fontFamily: FONT_MONO,
    fontSize: 13,
    theme: THEME_TERMINAL[appSettings.theme],
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  const tab = makeTab(tabId, "terminal", label);
  $("#tab-split-zone").before(tab);
  const panel = document.createElement("div");
  panel.className = "panel";
  panel.dataset.tab = String(tabId);
  panel.setAttribute("role", "tabpanel");
  const host_el = document.createElement("div");
  host_el.className = "term-host";
  panel.appendChild(host_el);
  $("#panels").appendChild(panel);
  term.open(host_el);
  // Selecting copies; right-click pastes — same ergonomics as openTerminal.
  host_el.addEventListener("mouseup", () => {
    const sel = term.getSelection();
    if (sel) { invoke("clipboard_write", { text: sel }).catch(() => {}); term.clearSelection(); }
  });
  const session: Session = { id: tabId, ptyId, host: label, term, fit, banner: undefined, unlisten: [], local: true };
  sessions.set(tabId, session);
  activateTab(tabId);
  term.onData((data) => { if (session.ptyId != null) invoke("pty_write", { id: session.ptyId, data }); });

  let output = "";
  void (async () => {
    const u1 = await listen<string>(`pty://output/${ptyId}`, (ev) => {
      output += ev.payload;
      term.write(ev.payload);
    });
    const u2 = await listen(`pty://exit/${ptyId}`, () => {
      term.write("\r\n\x1b[90m[run ended]\x1b[0m\r\n");
      session.ptyId = null;
      if (sessions.get(session.id) === session) markTabDead(session.id, true);
      refreshHostDots();
      onExit(output);
    });
    session.unlisten.push(u1, u2);
  })();
  setTimeout(() => { fit.fit(); if (session.ptyId != null) invoke("pty_resize", { id: ptyId, cols: term.cols, rows: term.rows }); }, 0);
}

// Spawn a dry-run (mode:"check", FR-ANS8). On rc==0 exit the gate opens for the
// current playbook+targets; the recap summary is shown.
async function ansDryRun() {
  if (ansState.running) return;
  const keyAtLaunch = ansCurrentKey();
  ansState.running = true;
  ansRenderGate();
  const startedAt = new Date().toISOString();
  const started = Date.now();
  let ptyId: number;
  try {
    ptyId = await invoke<number>("ansible_run", {
      req: { mode: "check", playbook: ansState.playbookPath, aliases: [...ansState.selected], limit: ansLimitExpr(), cols: 80, rows: 24 } as AnsibleRunReq,
    });
  } catch (e) {
    ansState.running = false;
    ansRenderGate();
    void dialog("Dry-run failed", `Could not start the dry-run: ${ansErrText(msg(e))}`, [{ label: "OK", id: "ok", style: "primary" }], "ok", "ok");
    return;
  }
  const label = `ansible check · ${ansPlaybookBasename()}`;
  openAnsibleTab(ptyId, label, (out) => {
    ansState.running = false;
    const { recap, summary } = ansParseRecap(out);
    // Gate opens only if the dry-run succeeded (rc==0, implied by a PLAY RECAP
    // with no failed/unreachable) AND the key still matches the current state.
    const ok = /PLAY RECAP/.test(out) && !/failed=[1-9]/.test(out) && !/unreachable=[1-9]/.test(out);
    const rc = ok ? 0 : 1;
    if (ok && keyAtLaunch === ansCurrentKey()) ansState.checkedKey = keyAtLaunch;
    $("#ans-recap-summary").textContent = summary ? `Dry-run: ${summary}` : "Dry-run finished.";
    ansRenderGate();
    void ansAppendHistory({ playbook: ansState.playbookPath, targets: ansLimitExpr(), mode: "check", started_at: startedAt, duration_ms: Date.now() - started, rc, recap });
  });
}

// FR-ANS10: Run-real ALWAYS requires an explicit confirm dialog first and never
// auto-runs. The confirm shows the hosts + the dry-run change summary.
async function ansRunReal() {
  if (ansState.running) return;
  // Re-guard the gate at click time (defense in depth; the button is already
  // disabled when the gate is shut).
  if (ansState.checkedKey === null || ansState.checkedKey !== ansCurrentKey()) {
    ansRenderGate();
    return;
  }
  const targets = ansLimitExpr();
  const summary = $("#ans-recap-summary").textContent || "";
  const id = await dialog(
    "Run this playbook for real?",
    `This applies ${ansPlaybookBasename()} to: ${targets}. ${summary ? summary + "." : ""} This changes the target hosts.`,
    [
      { label: "Run for real", id: "run", style: "primary" },
      { label: "Cancel", id: "cancel" },
    ],
    "cancel", // focus Cancel first — the mutating action is never the default focus
    "cancel"
  );
  if (id !== "run") return;
  ansState.running = true;
  ansRenderGate();
  const startedAt = new Date().toISOString();
  const started = Date.now();
  let ptyId: number;
  try {
    ptyId = await invoke<number>("ansible_run", {
      req: { mode: "real", playbook: ansState.playbookPath, aliases: [...ansState.selected], limit: targets, cols: 80, rows: 24 } as AnsibleRunReq,
    });
  } catch (e) {
    ansState.running = false;
    ansRenderGate();
    void dialog("Run failed", `Could not start the run: ${ansErrText(msg(e))}`, [{ label: "OK", id: "ok", style: "primary" }], "ok", "ok");
    return;
  }
  openAnsibleTab(ptyId, `ansible run · ${ansPlaybookBasename()}`, (out) => {
    ansState.running = false;
    const { recap, summary: s } = ansParseRecap(out);
    const ok = /PLAY RECAP/.test(out) && !/failed=[1-9]/.test(out) && !/unreachable=[1-9]/.test(out);
    $("#ans-recap-summary").textContent = s ? `Run: ${s}` : "Run finished.";
    ansRenderGate();
    void ansAppendHistory({ playbook: ansState.playbookPath, targets, mode: "real", started_at: startedAt, duration_ms: Date.now() - started, rc: ok ? 0 : 1, recap });
  });
}

function ansPlaybookBasename(): string {
  const p = ansState.playbookPath;
  const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
  return i === -1 ? p : p.slice(i + 1);
}

// Map opaque ansible:<code> error tokens to a short human cause.
function ansErrText(code: string): string {
  switch (code) {
    case "ansible:no-eligible-hosts": return "none of the chosen hosts are eligible (key/agent auth only).";
    case "ansible:config-unreadable": return "~/.ssh/config could not be read.";
    case "ansible:missing": return "the ansible-playbook binary was not found.";
    default: return code;
  }
}

// History sub-panel (FR-ANS14). Read on open + after each run's exit. Entries
// render from a cloned static template; all fields set via textContent (ADR-8).
async function ansLoadHistory() {
  let entries: AnsibleHistoryEntry[];
  try {
    entries = await invoke<AnsibleHistoryEntry[]>("ansible_history", { op: { kind: "read" } as AnsibleHistoryOp });
  } catch (e) {
    console.error("ansible_history read failed:", e);
    entries = [];
  }
  ansRenderHistory(entries);
}

async function ansAppendHistory(entry: AnsibleHistoryEntry) {
  try {
    const entries = await invoke<AnsibleHistoryEntry[]>("ansible_history", { op: { kind: "append", entry } as AnsibleHistoryOp });
    ansRenderHistory(entries);
  } catch (e) {
    console.error("ansible_history append failed:", e);
  }
}

function ansRenderHistory(entries: AnsibleHistoryEntry[]) {
  const list = $("#ans-history-list");
  list.innerHTML = "";
  const empty = $("#ans-history-empty");
  empty.hidden = entries.length > 0;
  const tpl = $("#ans-history-row-template") as HTMLTemplateElement;
  // Newest-last from the store; show newest first.
  for (const e of [...entries].reverse()) {
    const frag = tpl.content.cloneNode(true) as DocumentFragment;
    const base = (() => { const i = Math.max(e.playbook.lastIndexOf("/"), e.playbook.lastIndexOf("\\")); return i === -1 ? e.playbook : e.playbook.slice(i + 1); })();
    (frag.querySelector(".ans-hist-playbook") as HTMLElement).textContent = base;
    const mode = frag.querySelector(".ans-hist-mode") as HTMLElement;
    mode.textContent = e.mode === "check" ? "dry-run" : "real";
    mode.classList.add(e.mode === "check" ? "mode-check" : "mode-real");
    const rc = frag.querySelector(".ans-hist-rc") as HTMLElement;
    rc.textContent = e.rc === 0 ? "ok" : `rc ${e.rc}`;
    rc.classList.add(e.rc === 0 ? "rc-ok" : "rc-bad");
    (frag.querySelector(".ans-hist-targets") as HTMLElement).textContent = e.targets;
    (frag.querySelector(".ans-hist-time") as HTMLElement).textContent = e.started_at;
    (frag.querySelector(".ans-hist-dur") as HTMLElement).textContent = `${(e.duration_ms / 1000).toFixed(1)}s`;
    (frag.querySelector(".ans-hist-recap") as HTMLElement).textContent = e.recap;
    list.appendChild(frag);
  }
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
  const visible = hosts
    .filter(
      (h) => !h.name.includes("*") && !h.name.includes("?") && (!q || h.name.toLowerCase().includes(q))
    )
    // Alphabetical A–Z, case-insensitive, so groups and rows read tidy.
    .sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
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
  // Single-line rows: dot + name + (lock) + hover-only ⇅ / ⋯ buttons.
  // Flat alphabetical list — no prefix grouping headers (v0.14.10).
  for (const h of visible) {
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
const APP_VERSION = "v0.15.5";

// status bar updater (§4.3): dipanggil dari activateTab (semua jenis tab),
// openTerminal/splitTerminal/closeTab dan busy() SFTP.
// FR-SH8 (Dusk U-D2): left = "host · user@ip:port · connected"; right cluster
// = "N live · UTF-8 · LF · shell · clock" (+ pre-existing xfer/version).
// Display-only, derived from the existing session maps — no session behavior
// changes.
function updateStatusBar() {
  const id = activeTab;
  const hostName =
    sessions.get(id as number)?.host ??
    splitSessions.get(id as number)?.host ??
    sftpSessions.get(id as number)?.host ??
    editorSessions.get(id as number)?.host;
  const h = hostName ? hosts.find((x) => x.name === hostName) : undefined;
  const n = sessions.size + splitSessions.size;
  const el = document.getElementById("sb-left");
  if (el) {
    el.textContent = "";
    if (hostName) {
      const addr = [h?.user && `${h.user}@`, h?.host_name || hostName, h?.port && `:${h.port}`]
        .filter(Boolean)
        .join("");
      const connected = hostHasLiveSession(hostName);
      const host = document.createElement("span");
      host.className = "sb-host";
      host.textContent = hostName;
      el.append(host, document.createTextNode(` · ${addr} · ${connected ? "connected" : "idle"}`));
    }
  }
  const live = document.getElementById("sb-live");
  if (live) live.textContent = `${n} live`;
  const clock = document.getElementById("sb-clock");
  if (clock) clock.textContent = new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
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
// The private-key dropdown stores the FULL path as the option value (that is what
// gets saved as identity_file), but shows only the file name: strip everything
// through the last "/". A custom path not in the listed set is reduced the same way.
function identityLabel(p: string): string {
  const i = p.lastIndexOf("/");
  return i === -1 ? p : p.slice(i + 1);
}

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
    opts.push(`<option value="${escAttr(p)}"${p === selected ? " selected" : ""}>${esc(identityLabel(p))}</option>`);
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
// True when the key modal was opened from the Keys manager (not the host
// editor): on save we refresh the keys list instead of the editor dropdown.
let keyModalReturnsToKeys = false;

function openKeyModal() {
  ($("#k-name") as HTMLInputElement).value = "";
  ($("#k-text") as HTMLTextAreaElement).value = "";
  ($("#k-file") as HTMLInputElement).value = "";
  $("#key-err").textContent = "";
  openOverlay("#key-modal", $("#k-name"));
}

function closeKey() {
  // Wipe key material on close: #k-text may hold a pasted/uploaded private key.
  ($("#k-text") as HTMLTextAreaElement).value = ""; // never retain key bytes
  ($("#k-name") as HTMLInputElement).value = "";
  closeOverlay("#key-modal");
  keyModalReturnsToKeys = false;
}

// Import a private key from a user-picked file into #k-text for review/edit.
// The webview can read a File the user chose directly (File.text()), so this
// needs no dialog/fs plugin and no filesystem permission: the backend still
// writes ~/.ssh/keys/<name> at 0600 from the textarea contents, and nothing is
// saved until "Save key" is pressed. The textarea is written only after the
// read fully resolves, so a failed read never leaves partial content behind.
async function importKeyFile(file: File) {
  const err = $("#key-err");
  err.textContent = "";
  try {
    const text = await file.text();
    if (!text) {
      err.textContent = `"${file.name}" is empty — pick a private key file, or paste the key below.`;
      return;
    }
    const textEl = $("#k-text") as HTMLTextAreaElement;
    textEl.value = text;
    textEl.focus();
  } catch (e) {
    err.textContent = `Could not read "${file.name}": ${msg(e)}. Paste the key below instead.`;
  }
}

async function saveKey() {
  const name = ($("#k-name") as HTMLInputElement).value.trim();
  const privateKey = ($("#k-text") as HTMLTextAreaElement).value;
  try {
    const path = await invoke<string>("create_identity_file", { name, privateKey });
    const fromKeys = keyModalReturnsToKeys;
    closeKey();
    if (fromKeys) {
      // Opened from the Keys manager (modal or sidebar view): refresh both.
      await refreshKeyViews();
    } else {
      // Opened from the host editor: select the new key in the dropdown.
      await refreshIdentityDropdown(path);
      ($("#f-auth") as HTMLSelectElement).value = "key";
      applyAuthMode("key");
    }
  } catch (e) {
    $("#key-err").textContent = `Failed to save key: ${msg(e)}. Check the name and press Save key again.`;
  }
}

// ---------- edit a managed identity key: rename + replace contents ----------
// The name we are editing (original basename). The textarea is PREFILLED with
// the current key on open (v0.15.1), so Save normally writes the same bytes
// back (idempotent); clearing it keeps the write-only-if-non-empty guard, which
// makes a blank textarea a rename-only operation. A changed name triggers a rename.
let keyEditOriginal = "";

async function openEditKey(name: string) {
  keyEditOriginal = name;
  ($("#ke-name") as HTMLInputElement).value = name;
  const text = $("#ke-text") as HTMLTextAreaElement;
  text.value = "";
  $("#key-edit-err").textContent = "";
  openOverlay("#key-edit-modal", $("#ke-name"));
  // Prefill the textarea with the current private key so small edits/renames
  // don't force a full re-paste (MEKL, v0.15.1). By design the key bytes are
  // shown in the DOM only while the user is editing; closeEditKey wipes them.
  try {
    text.value = await invoke<string>("read_identity_file", { name });
  } catch (e) {
    // Surface the failure instead of silently leaving the textarea blank.
    $("#key-edit-err").textContent = `Could not load key contents: ${msg(e)}. You can still rename, or paste to replace.`;
  }
}

function closeEditKey() {
  keyEditOriginal = "";
  ($("#ke-text") as HTMLTextAreaElement).value = ""; // never retain key bytes
  closeOverlay("#key-edit-modal");
}

async function saveEditKey() {
  const nextName = ($("#ke-name") as HTMLInputElement).value.trim();
  const contents = ($("#ke-text") as HTMLTextAreaElement).value;
  const err = $("#key-edit-err");
  err.textContent = "";
  if (!nextName) { err.textContent = "Key name is required."; return; }
  try {
    // Rename first (if changed), then overwrite contents on the current name.
    // The textarea is prefilled, so an untouched key writes the same bytes back
    // (idempotent); a cleared textarea skips the write and renames only.
    let current = keyEditOriginal;
    if (nextName !== keyEditOriginal) {
      await invoke<string>("rename_identity_file", { old: keyEditOriginal, new: nextName });
      current = nextName;
    }
    if (contents.trim()) {
      await invoke<string>("write_identity_file", { name: current, contents });
    }
    closeEditKey();
    await refreshKeyViews();
  } catch (e) {
    err.textContent = `Failed to save changes: ${msg(e)}. Check the name and try again.`;
  }
}

// ---------- keys manager: list + add + delete ----------
// Only keys under ~/.ssh/keys/ are deletable (the backend refuses anything
// else); legacy ~/.ssh/* keys are shown read-only so the user still sees them.
async function renderKeysList(listSel = "#keys-list", errSel = "#keys-err") {
  const list = $(listSel);
  const err = $(errSel);
  err.hidden = true;
  err.textContent = "";
  list.textContent = "Loading…";
  let paths: string[];
  try {
    paths = await invoke<string[]>("list_identity_files");
  } catch (e) {
    list.textContent = "";
    err.textContent = `Could not list keys: ${msg(e)}`;
    err.hidden = false;
    return;
  }
  list.textContent = "";
  if (paths.length === 0) {
    const empty = staticElement("div", "list-state", "No keys yet. Click Add key to paste one.");
    list.appendChild(empty);
    return;
  }
  for (const path of paths) {
    const managed = path.startsWith("~/.ssh/keys/");
    const name = path.slice(path.lastIndexOf("/") + 1);
    const row = staticElement("div", "key-row");
    const label = staticElement("span", "key-name");
    label.textContent = name; // basename only; textContent escapes (ADR-8)
    label.title = path; // full path on hover, not as the row text
    row.appendChild(label);
    if (managed) {
      const edit = staticElement("button", "key-edit", "Edit") as HTMLButtonElement;
      edit.type = "button";
      edit.setAttribute("aria-label", `Edit key ${name}`);
      edit.addEventListener("click", () => { void openEditKey(name); });
      row.appendChild(edit);
      const del = staticElement("button", "danger", "Delete") as HTMLButtonElement;
      del.type = "button";
      del.setAttribute("aria-label", `Delete key ${name}`);
      del.addEventListener("click", () => { void deleteKey(name); });
      row.appendChild(del);
    } else {
      const tag = staticElement("span", "key-legacy", "legacy (read-only)");
      row.appendChild(tag);
    }
    list.appendChild(row);
  }
}

function closeKeys() {
  closeOverlay("#keys-modal");
}

async function deleteKey(name: string) {
  const choice = await dialog(
    "Delete key",
    `Delete key "${name}" from ~/.ssh/keys? Any host pointing at it will lose its IdentityFile. This cannot be undone.`,
    [
      { label: "Cancel", id: "cancel" },
      { label: "Delete", id: "delete", style: "danger" },
    ],
    "cancel",
  );
  if (choice !== "delete") return;
  try {
    await invoke("delete_identity_file", { name });
    await refreshKeyViews();
  } catch (e) {
    // Surface the error in whichever key list is currently visible.
    const errSel = $("#view-keys").classList.contains("hidden") ? "#keys-err" : "#sb-keys-err";
    const err = $(errSel);
    err.textContent = `Failed to delete key: ${msg(e)}`;
    err.hidden = false;
  }
}

// Refresh every mounted keys list (modal + sidebar view) so a delete/add in one
// surface is reflected in the other.
async function refreshKeyViews() {
  if (!$("#keys-modal").classList.contains("hidden")) await renderKeysList();
  if (!$("#view-keys").classList.contains("hidden")) {
    await renderKeysList("#sb-keys-list", "#sb-keys-err");
  }
}

type Section = "host" | "keys" | "ansible";
let activeSection: Section = "host";
// Whether the contextual section panel is collapsed. Module-level so the rail
// click handler (toggle on the active item) and selectSection (expand on a
// section switch) share one source of truth. Mutated only via setPanelCollapsed.
let panelCollapsed = false;

// Collapse/expand the contextual section panel. Also keeps aria-expanded on the
// rail items in sync (a rail item "expands" the panel when active), persists the
// collapsed state, and refits terminals. Assigned the real impl in the wire-up;
// a no-op stub keeps early calls safe before DOMContentLoaded.
let setPanelCollapsed: (collapsed: boolean) => void = () => {};

// Reflect the collapsed state on every rail item's aria-expanded: the active
// item's panel is expanded iff the panel is not collapsed; inactive items point
// at a collapsed (for them) panel.
function syncRailExpanded() {
  document.querySelectorAll<HTMLButtonElement>(".rail-item[data-view]").forEach((btn) => {
    const active = btn.dataset.view === activeSection;
    btn.setAttribute("aria-expanded", String(active && !panelCollapsed));
  });
}

function selectSection(next: Section, focusPanel = true) {
  activeSection = next;
  document.querySelectorAll<HTMLButtonElement>(".rail-item[data-view]").forEach((btn) => {
    const active = btn.dataset.view === next;
    btn.setAttribute("aria-current", active ? "page" : "false");
  });
  $("#view-host").classList.toggle("hidden", next !== "host");
  $("#view-keys").classList.toggle("hidden", next !== "keys");
  $("#view-ansible").classList.toggle("hidden", next !== "ansible");
  const add = $("#btn-add") as HTMLButtonElement;
  add.hidden = next !== "host";
  add.classList.toggle("hidden", next !== "host");
  if (next === "keys") void renderKeysList("#sb-keys-list", "#sb-keys-err");
  syncRailExpanded();
  if (!focusPanel) return;
  const target = next === "host" ? "#host-search" : next === "keys" ? "#sb-keys-add" : "#sb-ansible-open";
  $(target).focus();
}

// Rail click behaviour (FR v0.14.6): clicking the ALREADY-ACTIVE rail item
// toggles the panel collapsed/expanded; clicking a DIFFERENT item switches
// section and ensures the panel is expanded.
function onRailClick(next: Section) {
  if (next === activeSection) {
    setPanelCollapsed(!panelCollapsed);
    return;
  }
  if (panelCollapsed) setPanelCollapsed(false);
  selectSection(next);
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
// loading: initial listing / session establishment still in flight. While true,
// renderSftp paints a cancellable loading state (skeleton + Cancel) so a slow
// login never looks frozen; closing the tab mid-load tears down cleanly (AC2).
type SftpSession = { host: string; path: string; unlisten: (() => void)[]; loading: boolean };
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
  // Belt-and-suspenders: an absolute name is the full target, not a segment to
  // append (guards against a server emitting a rooted path). The backend
  // parser normalizes names to basenames, so this is a secondary safeguard.
  if (name.startsWith("/")) return name;
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
  sftpSessions.set(tabId, { host, path: "", unlisten: [], loading: true });
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
  // BUG1-FE: loading state. The skeleton rows read as "working" immediately;
  // during the FIRST listing (session establishment / slow login) we also give
  // the user an explicit way to bail so it never reads as a hang. The Cancel
  // control is a panel-local button torn down with the panel on closeTab — no
  // window listener, so no leak (SEC LOW t_fb1d18d6 posture preserved).
  skelRows(list);
  if (s.loading) {
    status(`Connecting to ${s.host}…`);
    const bail = document.createElement("div");
    bail.className = "list-state sftp-loading";
    const label = document.createElement("span");
    label.textContent = "Connecting…";
    const cancel = document.createElement("button");
    cancel.className = "sftp-cancel";
    cancel.type = "button";
    cancel.textContent = "Cancel";
    cancel.addEventListener("click", () => void closeTab(tabId));
    bail.append(label, cancel);
    list.appendChild(bail);
  }
  let entries: SftpEntry[];
  try {
    entries = await invoke<SftpEntry[]>("sftp_list", { host: s.host, path: s.path });
    // tab closed (Cancel) or session gone while the listing was in flight:
    // bail before writing the shared cache or a detached DOM node (AC2 — no
    // orphaned state / no stale cache poisoning).
    if (sftpSessions.get(tabId) !== s || !panel.isConnected) return;
    sftpHostCache.set(s.host, entries);
  } catch (err) {
    if (sftpSessions.get(tabId) !== s || !panel.isConnected) return;
    // stale persistent session (server timeout etc.): backend respawns it;
    // one automatic retry before showing the error
    try {
      entries = await invoke<SftpEntry[]>("sftp_list", { host: s.host, path: s.path });
      if (sftpSessions.get(tabId) !== s || !panel.isConnected) return;
      sftpHostCache.set(s.host, entries);
    } catch (err2) {
      if (sftpSessions.get(tabId) !== s || !panel.isConnected) return;
      s.loading = false;
      list.innerHTML = "";
      status(`Failed to list ${s.path || "~"}: ${msg(err2)}. Fix the path and press Go, or go up one level.`, "err");
      return;
    }
  }
  s.loading = false;
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
        highlightCompartment.of(syntaxHighlighting(CM_HIGHLIGHT[appSettings.theme])),
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
// xterm theme per preset, built from theme-palettes.json (SSOT, WCAG-gated by
// qa-reports/theme-contrast.mjs). bg=--term, fg=--text, cursor=--accent; ANSI
// derived from each preset's chrome palette (Dusk: green=--live sage,
// red=--danger terracotta, yellow=--accent honey). Dusk is byte-identical to
// the former TERMINAL_THEME literal (SDD §13.2 pt 5). Live terminals repaint
// via term.options.theme in applyTheme; the constructors below read the ACTIVE
// preset at open time. Scrollbar sliders are not settable in xterm 5.5's
// ITheme — --border/--accent apply when the addon support lands.
const THEME_TERMINAL = Object.fromEntries(
  (Object.keys(palettes) as ThemePreset[]).map((preset) => [preset, palettes[preset].terminal]),
) as Record<ThemePreset, ITheme>;


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
    cursorBlink: !REDUCED_MOTION,
    fontFamily: FONT_MONO,
    fontSize: 13,
    theme: THEME_TERMINAL[appSettings.theme],
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
        // Bracketed-paste via term.paste() (see the main-terminal path) so multi-
        // line pastes keep their indentation; bytes exit through term.onData.
        if (text) term.paste(text);
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

async function openTerminal(host: string, opts: { local?: boolean } = {}): Promise<Session | null> {
  const isLocal = opts.local === true;
  // Local shell never needs ssh; only guard the remote path on a missing binary.
  if (!isLocal && binaryBlocked("ssh")) return null;
  const tabId = nextTab++;
  const term = new Terminal({
    cursorBlink: !REDUCED_MOTION,
    fontFamily: FONT_MONO,
    fontSize: 13,
    theme: THEME_TERMINAL[appSettings.theme],
  });
  const fit = new FitAddon();
  term.loadAddon(fit);

  const tab = makeTab(tabId, "terminal", isLocal ? "local" : host);
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
  banner.innerHTML = isLocal
    ? `<span class="spinner"></span>Starting local shell…`
    : `<span class="spinner"></span>Connecting to ${esc(host)}…`;
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
        // term.paste() wraps the text in bracketed-paste (ESC[200~ .. ESC[201~)
        // whenever the remote app enabled it (?2004h), so vim/nano/shell suspend
        // auto-indent for the paste and multi-line content (e.g. YAML) keeps its
        // exact indentation. The wrapped bytes flow out through term.onData ->
        // pty_write. Writing raw to the PTY (the old path) dropped the markers
        // and let the remote re-indent every line.
        if (text) term.paste(text);
      })
      .catch(() => {});
  });

  const session: Session = { id: tabId, ptyId: null, host, term, fit, banner, unlisten: [], local: isLocal };
  sessions.set(tabId, session);
  activateTab(tabId);

  // user keystrokes -> pty
  term.onData((data) => {
    if (session.ptyId != null) invoke("pty_write", { id: session.ptyId, data });
  });

  await startPty(session);
  return session.ptyId == null ? null : session;
}

// Spawn the PTY for a terminal session and wire output/exit listeners.
async function startPty(session: Session) {
  try {
    session.fit.fit();
    const cols = session.term.cols;
    const rows = session.term.rows;
    const ptyId = session.local
      ? await invoke<number>("local_pty_spawn", { cols, rows })
      : await invoke<number>("pty_spawn", { host: session.host, cols, rows });
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
    session.term.write(`\\r\\n\\x1b[31mFailed to start ${session.local ? "local shell" : "ssh"}: ${msg(e)}. ${session.local ? "Reopen the tab to retry." : "Fix the host and reopen the tab."}\\x1b[0m\\r\\n`)
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
window.addEventListener("resize", fitActiveSessions);

// BUG 3 (data-loss guard): the native WebView2 context menu carries a "Reload"
// item; one accidental click reloads the SPA and wipes every in-memory tab
// (terminals + SFTP sessions). Suppress the native menu everywhere by
// preventing the default on a single document-level contextmenu listener. The
// intentional in-app menus (tab 2464, host-row 1218, terminal paste 2690/2750)
// register their own contextmenu handlers on the target element; those run in
// the target phase BEFORE this bubble-phase guard and already call
// preventDefault + show the app's own #tab-menu/#host-menu, so this guard never
// swallows them — it only kills the native menu on non-interactive areas (the
// SFTP pane, empty space). Singleton: registered once at module top level
// (mirrors the resize singleton above), so it does not leak and is not torn
// down per-tab. Idempotent re-prevent is harmless for surfaces that already
// prevented default.
document.addEventListener("contextmenu", (ev) => {
  ev.preventDefault();
});

// ---------- wire up ----------
window.addEventListener("DOMContentLoaded", () => {
  const sectionPanel = $("#section-panel");
  const divider = document.createElement("div");
  divider.id = "sb-resize";
  divider.setAttribute("role", "separator");
  divider.setAttribute("aria-orientation", "vertical");
  divider.setAttribute("aria-label", "Resize section panel");
  sectionPanel.after(divider);
  const saved = localStorage.getItem("sb-width");
  if (saved) sectionPanel.style.width = Math.min(504, Math.max(160, parseInt(saved, 10))) + "px";
  let dragging = false;
  divider.addEventListener("mousedown", (e) => {
    dragging = true;
    divider.classList.add("dragging");
    e.preventDefault();
  });
  window.addEventListener("mousemove", (e) => {
    if (!dragging) return;
    const w = Math.min(504, Math.max(160, e.clientX - 56));
    sectionPanel.style.width = w + "px";
  });
  window.addEventListener("mouseup", () => {
    if (!dragging) return;
    dragging = false;
    divider.classList.remove("dragging");
    const w = parseInt(sectionPanel.style.width, 10);
    if (w) localStorage.setItem("sb-width", String(w));
    setTimeout(fitActiveSessions, 0);
  });

  $("#btn-add").addEventListener("click", () => openEditor(null));
  $("#btn-reload").addEventListener("click", loadHosts);
  $("#btn-raw").addEventListener("click", openRaw);
  initGoogleAccount();
  initSettings();
  initAiAssistant();
  initSnippets();
  ($("#host-search") as HTMLInputElement).addEventListener("input", renderHostList);
  selectSection("host", false);
  window.addEventListener("keydown", (ev) => {
    if ((ev.ctrlKey || ev.metaKey) && ev.key.toLowerCase() === "k"
      && overlayStack.length === 0 && $("#signin-popover").hidden && activeSection === "host") {
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
  setPanelCollapsed = (c: boolean) => {
    panelCollapsed = c;
    document.body.classList.toggle("panel-collapsed", c);
    syncRailExpanded();
    try { localStorage.setItem("hosterm.panel-collapsed", c ? "1" : ""); } catch { /* private mode */ }
    setTimeout(fitActiveSessions, 0);
  };
  const storedPanel = localStorage.getItem("hosterm.panel-collapsed");
  const initialPanelCollapsed = storedPanel === null
    ? localStorage.getItem("hosterm.sb-collapsed") === "1"
    : storedPanel === "1";
  setPanelCollapsed(initialPanelCollapsed);
  const narrowPanel = window.matchMedia("(max-width: 520px)");
  const collapseForNarrowWindow = () => { if (narrowPanel.matches) setPanelCollapsed(true); };
  narrowPanel.addEventListener("change", collapseForNarrowWindow);
  collapseForNarrowWindow();
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
  // Upload: the button opens the hidden file input; a picked file is read into
  // #k-text for review/edit (no auto-save). Reset the input so picking the same
  // file again still fires change.
  $("#k-upload").addEventListener("click", () => ($("#k-file") as HTMLInputElement).click());
  $("#k-file").addEventListener("change", () => {
    const input = $("#k-file") as HTMLInputElement;
    const file = input.files?.[0];
    if (file) void importKeyFile(file);
    input.value = "";
  });
  $("#ke-cancel").addEventListener("click", closeEditKey);
  $("#ke-save").addEventListener("click", () => { void saveEditKey(); });
  document.querySelectorAll<HTMLButtonElement>(".rail-item[data-view]").forEach((btn) => {
    btn.addEventListener("click", () => onRailClick((btn.dataset.view || "host") as Section));
  });
  // Inline "Add key" in the sidebar Keys view reuses the create-key modal.
  $("#sb-keys-add").addEventListener("click", () => { keyModalReturnsToKeys = true; openKeyModal(); });
  // Ansible is a wide runner; the sidebar view offers a launcher that opens it.
  $("#sb-ansible-open").addEventListener("click", () => { void openAnsible(); });
  // Local terminal (Termius-style): open a PTY on the OS default shell.
  $("#btn-local").addEventListener("click", () => { void openTerminal("local", { local: true }); });
  $("#ansible-close").addEventListener("click", () => closeOverlay("#ansible-modal"));
  $("#ans-recheck").addEventListener("click", () => { void ansRefreshCheck(); });
  $("#ans-pick-playbook").addEventListener("click", () => { void ansPickPlaybook(); });
  $("#ans-dry-run").addEventListener("click", () => { void ansDryRun(); });
  $("#ans-run-real").addEventListener("click", () => { void ansRunReal(); });
  $("#keys-close").addEventListener("click", closeKeys);
  $("#keys-add").addEventListener("click", () => { keyModalReturnsToKeys = true; openKeyModal(); });
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
