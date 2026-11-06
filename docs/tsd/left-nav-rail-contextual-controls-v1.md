# TSD: hosterm left navigation rail and contextual controls (v1)

Status: buildable TSD. Owner: crew-coding-lead role, salvaged by nano after provider failure. Date: 2026-11-06.
Inputs: `docs/prd/left-nav-rail-contextual-controls-v1.md`, `docs/sdd/left-nav-rail-contextual-controls-v1.md` at `dbac2ad` plus `39b958c`.

This document specifies the frontend implementation. It does not change OAuth, Drive, Rust, IPC, CSP, or release state. Google login remains blocked by cloud client type under card `t_a7c6ee14`.

## 1. Baseline and scope

Live baseline at dispatch:

- Frontend build: `npm run build`, zero TypeScript errors.
- Rust baseline: 87 tests, backend must remain byte-unchanged for this feature.
- Canonical smoke: 343/343, count may only grow.
- IPC: 42 registered commands (`lib.rs:40`, `fleet.rs:2`).
- Dusk `:root` and INV-21 remain byte-exact.

Allowed implementation files:

- `index.html`
- `src/main.ts`
- `src/styles.css`
- `qa-reports/headless-smoke.mjs`
- feature docs and changelog after implementation

Forbidden implementation files:

- `src-tauri/**`
- `src-tauri/tauri.conf.json`
- Rust/Cargo manifests
- OAuth and Drive implementation
- credential, token, config, and secrets paths

Any forbidden-file diff is a review blocker.

## 2. Final shell structure

`#app` stays a flex row. The old single `#sidebar` is replaced by these siblings:

```text
#app
├─ #nav-rail
│  ├─ nav.rail-nav[aria-label="Sections"]
│  │  ├─ button.rail-item[data-view="host"]
│  │  ├─ button.rail-item[data-view="keys"]
│  │  └─ button.rail-item[data-view="ansible"]
│  ├─ #btn-expand.hidden
│  └─ .rail-foot
│     ├─ #user-chip
│     ├─ #google-chip-signed-out-template
│     ├─ #google-chip-signed-in-template
│     └─ #signin-popover
│        ├─ #signin-btn
│        ├─ #drive-actions
│        │  ├─ #drive-backup-btn
│        │  ├─ #drive-restore-btn
│        │  └─ #drive-status
│        ├─ #signout-btn
│        └─ #acct-settings-btn
├─ #section-panel
│  ├─ .panel-head
│  │  ├─ .brand (text: hosterm)
│  │  ├─ #btn-add
│  │  └─ #btn-collapse
│  ├─ #view-host.sb-view
│  │  ├─ #host-search
│  │  ├─ #host-list
│  │  └─ .panel-host-controls
│  │     ├─ #btn-raw
│  │     ├─ #btn-local
│  │     └─ #btn-reload
│  ├─ #view-keys.sb-view.hidden
│  └─ #view-ansible.sb-view.hidden
├─ #sb-resize
└─ #main
```

Markup rules:

1. Rail buttons are static `<button>` elements. Each keeps the existing relevant SVG, `data-view`, `aria-label`, and `title`. Hosts starts with `aria-current="page"`; others use `aria-current="false"`.
2. The visible text labels from `.sb-nav-label` are retired in the 56px rail. Accessible names are mandatory.
3. `#user-chip`, both Google templates, and the full popover move as one static block. Dynamic account content continues to use template cloning and `textContent`, never attacker-controlled `innerHTML`.
4. `#acct-settings-btn` is a static button outside `#drive-actions`, so it exists signed out and signed in.
5. The old `#open-settings`, `#sidebar-nav`, `.sidebar-footer`, and `.sidebar-foot` markup is deleted.
6. The account control remains the lowest rail item. `#btn-expand` stays above `.rail-foot`; it never displaces the account control from the bottom.

## 3. Navigation state machine

Add near the current sidebar view-switcher (`src/main.ts`, current `selectSidebarView` around line 2444):

```ts
type Section = "host" | "keys" | "ansible";
let activeSection: Section = "host";
```

Replace `selectSidebarView` with one transition:

```ts
function selectSection(next: Section, focusPanel = true): void
```

It performs these steps in order:

1. Assign `activeSection = next`.
2. Set exactly one `.rail-item` to `aria-current="page"`; set the other two to `"false"`.
3. Toggle `.hidden` on `#view-host`, `#view-keys`, and `#view-ansible`.
4. Toggle both the `hidden` attribute and `.hidden` class on `#btn-add`. It is visible only for Hosts.
5. Host controls are children of `#view-host`; the hidden parent removes Raw config, Local, and Reload from layout and tab order on Keys/Ansible. Do not use opacity, visibility, or off-screen hiding.
6. Preserve lazy Keys rendering through `renderKeysList("#sb-keys-list", "#sb-keys-err")`.
7. If `focusPanel` is true, move focus to `#host-search`, `#sb-keys-add`, or `#sb-ansible-open` for the selected section.

Initialization calls `selectSection("host", false)`. It must not steal startup focus. Every later rail activation uses the default `focusPanel = true`.

No second active-section variable, CSS class, dataset, or per-control boolean may become a source of truth.

## 4. Google account popover and Settings

### 4.1 Non-modal invariant

The popover stays inside `#nav-rail`, which is inside `#app`. It must never call `openOverlay` or `closeOverlay`, because `openOverlay` sets `inert` on `#app` and would inert the popover itself.

Keep `setGooglePopover` as the open/close authority. Keep the single init-time document outside-click handler. No listener is added per open.

### 4.2 Shared focus-cycle helper

Extract the Tab-cycle code currently in `overlayKeydown` into:

```ts
function cycleFocusWithin(root: HTMLElement, event: KeyboardEvent): void
```

Candidate controls are buttons, inputs, textareas, and selects that are:

- not disabled;
- not under `.hidden`;
- not under an element with the `hidden` attribute;
- actually rendered (`getClientRects().length > 0`).

This rendered check is required because Google auth controls use the `hidden` attribute without always adding `.hidden`. A hidden sign-out or Drive control must never become the first or last focus target.

Both callers use the same helper:

- `overlayKeydown` calls it for the top modal overlay.
- The single account-popover keydown handler calls it while the popover is open.

Account handler behavior:

- Tab or Shift+Tab: call `cycleFocusWithin`.
- Escape: call `setGooglePopover(false, true)` and restore focus to `#user-chip`.
- Closed popover: no-op.

Outside-click closes without forcing focus back to the chip, because focus should follow the clicked target.

### 4.3 Settings wiring

Delete the `#open-settings` listener from `initSettings`. In `initGoogleAccount`, add one listener on `#acct-settings-btn`:

1. close the account popover without focus restore;
2. call the existing `openSettings()`.

`openSettings` remains unchanged and has no Google-auth dependency.

### 4.4 Keyboard shortcut guard

Ctrl+K focuses host search only when all conditions are true:

```ts
overlayStack.length === 0 &&
$("#signin-popover").hidden &&
activeSection === "host"
```

It is a no-op on Keys, Ansible, or while the account menu is open.

## 5. Panel collapse, narrow viewport, and resize

Rename the behavioral concept from sidebar collapse to panel collapse:

```ts
function setPanelCollapsed(collapsed: boolean): void
```

It must:

1. toggle `body.panel-collapsed`;
2. show `#btn-expand` only while collapsed;
3. persist `hosterm.panel-collapsed`;
4. call `setTimeout(fitActiveSessions, 0)` after every width-changing transition.

Migration:

- Read `hosterm.panel-collapsed` first.
- If absent, read legacy `hosterm.sb-collapsed` once and apply it.
- Future writes use only `hosterm.panel-collapsed`.
- Keep the existing `sb-width` storage key; it stores panel width and is already covered by smoke.

Controls:

- `#btn-collapse` remains in `.panel-head` and calls `setPanelCollapsed(true)`.
- `#btn-expand` lives on the rail and calls `setPanelCollapsed(false)`.
- The rail never collapses.

Narrow window:

- Register one init-time resize or media-query listener.
- At viewport width 520px or lower, collapse the section panel.
- Never register this listener per section switch.
- Do not change `activeSection` during auto-collapse.

Resizer:

- Insert `#sb-resize` between `#section-panel` and `#main`.
- Clamp width using `Math.min(504, Math.max(160, event.clientX - 56))`.
- Persist the final width to `sb-width`.
- Mouseup removes drag state and calls `setTimeout(fitActiveSessions, 0)`.
- Replace the current main-session-only refit block. `fitActiveSessions` covers normal and split sessions.
- Manual mouse events remain mandatory; no HTML5 drag-and-drop.

## 6. CSS contract

Dusk `:root` lines remain byte-exact. No new palette literal is allowed.

### 6.1 Rail

- `#nav-rail`: fixed `56px`, `flex: 0 0 56px`, vertical flex, `var(--sidebar)` background, right `var(--border)` hairline.
- `.rail-nav`: column layout with 6 to 8px padding and 4px gap.
- `.rail-item`: 44 by 44px target, 16px icon, `var(--r-md)`, token colors, 120ms color/background transition.
- `.rail-item[aria-current="page"]`: `var(--surface)` background and accent icon.
- Do not copy the old `.sb-nav:focus-visible { outline:none }`. Use the global visible focus ring.
- `.rail-foot`: `margin-top:auto`, 8px padding, `position:relative`.

### 6.2 Context panel

- `#section-panel`: 248px default, 160px min, 504px max, column flex, `var(--sidebar)` background, border toward the workspace.
- `body.panel-collapsed #section-panel` and the inactive resizer are `display:none`.
- `.panel-head`: brand, Hosts-only Add host, and collapse control. Use existing 12px spacing rhythm.
- `.panel-host-controls`: flex row using existing footer button rules, inside `#view-host`.

### 6.3 Account popover

Re-anchor the existing popover from the old footer to the right of the rail:

- absolute within `.rail-foot`;
- `left: calc(100% + 8px)`;
- `bottom: 0`;
- fixed practical width around 230px;
- caret points left;
- keep the existing permitted `var(--shadow-drawer)` elevation.

No JS geometry math, gradient, glow, or extra shadow.

### 6.4 Retired rules

Remove rules for:

- `#sidebar-nav` and `.sb-nav`;
- `.sb-nav-label`;
- `.sidebar-footer` and `.sidebar-foot`;
- `body.sb-collapsed`;
- dead `.sidebar-foot #btn-keys` styling.

## 7. Smoke and click-through specification

The canonical suite remains one file: `qa-reports/headless-smoke.mjs`. Existing coverage is migrated, not deleted. The final pass count must be greater than 343.

Required named checks:

1. Default load selects Hosts, exactly one rail item has `aria-current="page"`, Hosts view is visible, and startup focus was not stolen.
2. Hosts exposes Add host, Raw config, Local, Reload config, search, and host list.
3. Keys exposes its list/add/error surfaces; all four Host controls are absent from the rendered focusable set.
4. Ansible exposes its runner control; all four Host controls are absent from the rendered focusable set.
5. Section switching moves focus to the first valid section control and never leaves focus stranded.
6. Ctrl+K focuses host search only on Hosts with no modal and a closed account menu.
7. Account control is a descendant of `.rail-foot` in `#nav-rail`.
8. Opening account popover leaves `#app` non-inert and does not change `overlayStack` length.
9. Tab and Shift+Tab cycle only over rendered controls for both signed-out and signed-in states.
10. Signed out shows Sign in and Settings. Signed in shows Backup, Restore, Sign out, and Settings.
11. Settings opens from the account menu in both auth states.
12. Escape closes and restores chip focus. Outside-click closes without forced chip focus.
13. `#open-settings` does not exist.
14. Collapse keeps rail visible, hides panel/resizer, and refits terminal sessions.
15. Expand restores the panel and refits sessions.
16. Resizer clamps to 160..504 after the 56px rail offset and persists `sb-width`.
17. Split terminals and normal terminals both receive refit/resize after width changes.
18. At 520px viewport, panel auto-collapses, rail remains, no horizontal overflow, and controls remain keyboard reachable.
19. Host empty/loading/error states, Keys error state, and Ansible content remain intact.
20. INV-21 rendered Dusk tokens remain byte-exact; source Abyss/glow scan remains zero.
21. No extra network request is introduced.
22. Hostile account and host text stays inert with no dynamic child creation or execution.

Existing smoke locations that must be migrated include the sidebar collapse block, sidebar width drag, 248px sidebar assertion, old `#open-settings` clicks/focus restore, old `.sb-nav` block, and every Raw/Reload flow that now must select Hosts first.

Click-through ledger recorded by QA:

- Hosts, Keys, Ansible rail controls;
- Add host, Raw config, Local, Reload config;
- account open/close, Sign in visual path, Backup, Restore, Sign out, Settings in both auth states;
- Tab, Shift+Tab, Escape, outside click, Ctrl+K;
- collapse, expand, resize, narrow viewport;
- dark Dusk and all shipped appearance presets after structural changes;
- console and CSP violation count.

## 8. Source and security gates

Every implementation run records these checks:

- `git diff -- src-tauri` is empty.
- IPC raw and registered counts remain 42.
- Dusk `:root` is byte-exact.
- zero forbidden Abyss literals and zero `var(--glow)` under `src/`.
- no new dynamic `innerHTML` for account, host, or remote text.
- no `client_secret` added.
- no token field or successful token body logged or surfaced.
- staging excludes `target/`, `node_modules/`, `dist/`, and `.exe`.
- `npm run build` exits 0.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib` remains green.
- canonical smoke exits 0 with count greater than 343.
- browser console and CSP violation count remain zero.

## 9. Antislop delivery gate

Design read: developer desktop tool for technical users, Dusk terminal-native chrome, ENERGY 1 / RHYTHM 2 / MOTION 1.

Purpose record:

- 56px rail: persistent global navigation without competing with terminal work.
- 248px contextual panel: preserves familiar host-list density.
- account at rail bottom: app/account actions are stable and separate from section tasks.
- one Dusk accent: current section and focus only, not decoration.
- icon-first rail: fits three persistent destinations; accessible names replace visible labels.
- single permitted popover shadow: communicates elevation over the workspace.

Delivery requires PASS for:

- no dead interactive control;
- keyboard reachability and visible focus;
- no hidden-but-tabbable Host control;
- contrast AA in every shipped appearance preset;
- no mobile/narrow horizontal overflow;
- empty/loading/error states preserved;
- all controls clicked through with no console error;
- no new asset or fabricated content;
- no ChatGPT visual clone.

## 10. Unit and dependency order

The four hotspot files are tightly coupled. Parallel implementation would create false speed and merge conflicts. Use this serial graph:

1. **FE-NAV1: one integrated frontend implementation**
   - Owns `index.html`, `src/main.ts`, `src/styles.css`, and `qa-reports/headless-smoke.mjs`.
   - Implements the entire TSD and leaves all gates green.
2. **QA-NAV1: independent read-only verification**
   - Depends on FE-NAV1.
   - Runs full build/tests/smoke and click-through ledger. Does not modify source.
3. **SEC-NAV1: independent read-only security review**
   - Depends on FE-NAV1.
   - Reviews focus/inert behavior, dynamic rendering, listeners, OAuth non-change, tokens, CSP, and forbidden-file diff.
4. **LEAD-NAV1: fix/review gate**
   - Depends on QA-NAV1 and SEC-NAV1.
   - Applies required findings through a dedicated fix card if needed, reruns all gates, and hands the verified commit to nano.

Release is not part of these cards. Nano performs release versioning, push, tag, CI polling, asset verification, and board closure only after all implementation gates pass. The separate Google Cloud Desktop-client blocker may remain open; this UI feature must not claim Google login is fixed.
