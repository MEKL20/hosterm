# SDD: hosterm left navigation rail and contextual controls (v1)

Status: SDD (architect-owned). Author: crew-coding-architect. Date: 2026-11-06.
Repo: hosterm (Tauri v2 + TypeScript + xterm.js). **Documentation only — no source code lands here.**
Consumes: `docs/prd/left-nav-rail-contextual-controls-v1.md` (PRD, commit `c0c2e92`, factual
correction `a54b222`). Feeds: lead TSD + implementation cards.

This SDD answers **HOW** the three-column shell is built, which DOM owns what, where the
source-of-truth state lives, and the exact invariants/ADRs QA and security verify. It does not
reopen the PRD's LOCKED product decisions (IA-1..IA-11). Where the PRD says "the SDD's call"
(collapse model, IA-9), the decision is frozen below.

Grounded against live code read 2026-11-06:
- `index.html` lines 11-101: `#app` → `#sidebar` (single column: `.sidebar-head` brand + `#btn-add`
  + `#btn-collapse`; `#sidebar-nav` with three `.sb-nav` buttons; `#view-host`/`#view-keys`/
  `#view-ansible`; `#btn-expand`; `.sidebar-footer` → `.sidebar-foot` with `#btn-raw`/`#btn-local`/
  `#btn-reload`/`#open-settings` + `#user-chip` + chip templates + `#signin-popover`).
- `src/main.ts`: `selectSidebarView` (L2444), `initGoogleAccount` (L562), `setGooglePopover`
  (chip+popover), overlay stack `openOverlay`/`closeOverlay`/`onOverlayEscape` (L597-671),
  `openSettings` (L964) wired to `#open-settings` (L980), `fitActiveSessions` (L1008),
  `setCollapsed` + `#btn-collapse`/`#btn-expand` + `#sb-resize` manual resizer (L3889, L3959-3972),
  `selectSidebarView` nav wiring (L3990), Ctrl+K gate on `overlayStack.length===0` (L3932).
- `src/styles.css`: Dusk `:root` 16 tokens + geometry (L1-36), `#sidebar` 248px/min160/max560 +
  `#sb-resize` (L182-192), `#sidebar-nav`/`.sb-nav` (L209-235), `.sb-view` (L237-250),
  `body.sb-collapsed` (L252-258), `.sidebar-footer` (L312).
- Dusk `:root` is byte-exact and frozen (INV-21). This change adds **zero** new palette literals.

---

## 1. Scope, non-scope, and the file-scope contract

### 1.1 In scope (frontend IA/visibility only)

Restructure the single `#sidebar` column into a three-column shell: a narrow icon-first **rail**,
a **contextual section panel**, the existing **workspace** (`#main`). Move the Google account
control to the rail bottom; move Settings into the account popover (works signed-out and signed-in);
make Add host / Raw config / Local / Reload config Hosts-only. Preserve every existing destination,
every real OAuth/Drive behavior, Dusk identity, and the terminal re-fit contract.

### 1.2 Out of scope (hard)

No backend change, no new `#[tauri::command]`, no IPC inventory change (stays **42**, STATE.md
§Verify loop), no `~/.ssh/config`/secrets-sidecar change, no Dusk `:root` edit, no new network call
(NFR-3), no new feature, no new section/destination, no ChatGPT visual clone, no `client_secret`,
no OAuth/Drive code edit. **ADR-NAV-5** makes backend files a review-blocker (§13).

### 1.3 Files the implementation MAY touch

| File | What it owns for this change |
|---|---|
| `index.html` | New rail/panel/account-popover **static** markup; retire `#sidebar-nav` horizontal strip + `.sidebar-footer` old positions. All templates stay static (ADR-8). |
| `src/main.ts` | Rail selection state machine (evolve `selectSidebarView`), visibility matrix application, account-popover open/close/focus (evolve `initGoogleAccount`/`setGooglePopover`), Settings-from-menu wiring, collapse/resize + terminal re-fit. DOM only. |
| `src/styles.css` | Rail + panel + account-popover CSS from existing Dusk tokens; retire horizontal-nav/footer rules. No new literal (INV-21). |
| `qa-reports/headless-smoke.mjs` | New behavior checks per §11; counts may only grow (AGENTS.md §4). |

### 1.4 Files the implementation MUST NOT touch (backend forbidden — ADR-NAV-5)

`src-tauri/src/lib.rs`, `src-tauri/src/fleet.rs`, `src-tauri/src/*.rs`, `src-tauri/tauri.conf.json`
(CSP frozen), `Cargo.toml`, any config/secrets path. Backend diff for this feature is **0**; the
`grep -rc "tauri::command" src-tauri/src/*.rs` gate stays `lib.rs:40  fleet.rs:2` = 42 registered.

### 1.5 Separate blocked card (not fixed here)

Board card **`t_a7c6ee14`** owns the Google Cloud Web-vs-Desktop OAuth client-type blocker
(`invalid_request (client_secret is missing.)`). This SDD does **not** fix, work around, or claim
resolution of that blocker. The account menu relocates the existing real sign-in / Drive /
sign-out surface verbatim; the blocker is orthogonal to IA.

---

## 2. Component boundaries and ownership (freeze #1)

The shell becomes three sibling flex columns inside `#app`. The current `#sidebar` splits into the
first two; `#main` is unchanged.

```
#app (flex row)
├─ #nav-rail            (COLUMN 1, narrow, fixed width — persistent global nav + account)
│   ├─ .rail-nav        <nav aria-label="Sections">  → 3× button.rail-item (Hosts/Keys/Ansible)
│   └─ .rail-foot       → #user-chip (account control, rail bottom)  [#signin-popover anchors here]
├─ #section-panel       (COLUMN 2, resizable/collapsible — contextual section content + controls)
│   ├─ .panel-head      brand `hosterm` + section-scoped primary action (#btn-add, Hosts only)
│   ├─ #view-host       host search (#host-search) + host list (#host-list) + host controls row
│   │     └─ .panel-host-controls → #btn-raw, #btn-local, #btn-reload  (Hosts only)
│   ├─ #view-keys       managed-key list + #sb-keys-add + note + #sb-keys-err (unchanged content)
│   └─ #view-ansible    note + #sb-ansible-open (unchanged content)
├─ #sb-resize           (COLUMN resizer handle — BETWEEN panel and workspace; manual mouse)
└─ #main                (COLUMN 3, workspace — tabs/terminals/SFTP/editor/status; UNCHANGED)
```

Ownership table (who renders, who owns state):

| Component | DOM id/class | Renders | State owner |
|---|---|---|---|
| Navigation rail | `#nav-rail` | 3 rail items + account control | — (static shell) |
| Rail item | `button.rail-item[data-view]` | icon + accessible label | `activeSection` (§3) via `aria-current` |
| Account control | `#user-chip` (moved to `.rail-foot`) | signed-out/in chip via existing templates | `googleAccount` (existing) |
| Account popover | `#signin-popover` (re-anchored) | existing sign-in/Drive/sign-out + **Settings item** | non-modal popover (NOT `overlayStack`) + `googleAccount` |
| Section panel | `#section-panel` | active section body + scoped controls | `activeSection` + panel-collapsed flag |
| Panel head | `.panel-head` | brand + `#btn-add` | visibility derived from `activeSection` |
| Host controls | `.panel-host-controls` | `#btn-raw`/`#btn-local`/`#btn-reload` | visibility derived from `activeSection` |
| Resizer | `#sb-resize` | drag handle | panel-width (localStorage, existing pattern) |
| Workspace | `#main` | tabs/terminals/SFTP/editor | existing, untouched |

Rail item, account popover, and panel controls are **static markup in `index.html`** cloned/toggled
by the renderer. No dynamic `innerHTML` builds any of them (ADR-8 / INV-27, §13).

---

## 3. Source-of-truth state (freeze #2 — no duplicated visibility state)

There is exactly **one** piece of navigation state and **one** panel-collapsed flag. Everything
visible is *derived* from them; nothing stores a second copy of "which section is current" or
"which control is visible."

### 3.1 `activeSection` — the single nav source of truth

```ts
type Section = "host" | "keys" | "ansible";
// single source of truth; NOT duplicated in DOM classes or a second variable.
let activeSection: Section = "host"; // default = Hosts (AC-H1)
```

The DOM derivations (all computed from `activeSection`, never stored independently):
- `aria-current="page"` on the matching `button.rail-item`, `"false"` on the others (preserves the
  current `selectSidebarView` semantics, main.ts L2447).
- `.hidden` toggled on `#view-host`/`#view-keys`/`#view-ansible` (existing mechanism, L2449-2451).
- `.panel-head` `#btn-add` and `.panel-host-controls` are shown **iff** `activeSection === "host"`.

The visibility matrix (§5) is a **pure function of `activeSection`**: one function recomputes all
derivations on every section change. No per-control boolean is persisted.

### 3.2 `selectSection(next)` — the one transition

Evolves the current `selectSidebarView`. Exact responsibilities, in order:

1. `activeSection = next`.
2. For each `.rail-item`: set `aria-current = (dataset.view === next) ? "page" : "false"`.
3. Toggle `.hidden` on the three `#view-*` bodies (show only the active one).
4. Apply the §5 visibility matrix: `host` → show `#btn-add` + `.panel-host-controls`; `keys`/
   `ansible` → **remove them from the accessibility tree AND tab order** (§5.2 mechanism).
5. Lazy section init preserved: `if (next === "keys") void renderKeysList(...)` (existing L2452).
6. Move focus into the panel's first sensible target (IA-6): Hosts → `#host-search`; Keys →
   `#sb-keys-add`; Ansible → `#sb-ansible-open`. Focus is never left stranded on the rail (R2).

`selectSection` is idempotent and side-effect-complete: calling it is the *only* way `activeSection`
changes, so there is no path that updates the rail without updating the panel or vice versa.

### 3.3 Panel collapsed/width — the second (independent) flag

```ts
// existing pattern, retained: localStorage + body class. Renamed concept: it now collapses
// the SECTION PANEL, not the whole sidebar (the rail always stays visible — §7, ADR-NAV-3).
let panelCollapsed: boolean;   // mirror of body.classList.contains("panel-collapsed")
```

Panel width is stored in `localStorage` via the existing `#sb-resize` drag (main.ts L3889). The
collapsed flag is stored under a migrated key (§9). These two are orthogonal to `activeSection`:
collapsing the panel does not change which section is current, and switching section does not change
collapsed state.

---

## 4. Geometry using existing tokens (freeze #3)

All geometry uses Dusk `:root` tokens and the existing spacing instinct (4/8/12 px). **No new
`:root` token is added** (INV-21; the PRD's NFR). One-line purpose per decision (R-31):

| Decision | Value | Token/basis | Purpose (R-31) |
|---|---|---|---|
| Rail width | `56px` fixed | literal in `#nav-rail` rule (geometry, not palette) | icon-first anchor; wide enough for a 16px icon + hit area, narrow enough to stay a rail not a column. |
| Rail item hit area | `44×44px`, 16px icon | S-9 icon system | keyboard/pointer target meets min hit area; icon reads at 16px. |
| Rail bg | `var(--sidebar)` | existing | rail is chrome; recedes like the old sidebar did. |
| Rail right border | `1px solid var(--border)` | existing | hairline separates rail from panel (ENERGY 1, no shadow). |
| Panel default width | `248px` | current `#sidebar` width (L183) | preserve the width the operator already has; zero surprise. |
| Panel min width | `160px` | current `#sidebar` min (L184) | keep host rows legible; existing floor. |
| Panel max width | `504px` | `560 − 56` (old max minus rail) | keep total left region ≤ old max so workspace never starves. |
| Resizer | `#sb-resize` 5px, between panel and `#main` | existing L191 | reuse the proven manual-mouse resizer; no new component. |
| Panel collapsed width | `0` (hidden, `display:none`), rail stays | ADR-NAV-3 | reclaim full width for the workspace; rail remains the anchor. |
| Account control | `.rail-foot`, `margin-top:auto` | flexbox | pins account to rail bottom (AC-M1) without absolute positioning. |
| Narrow-window policy | below ~520px total, panel auto-collapses; rail persists | ADR-NAV-4 | three columns never break; rail + workspace are the irreducible minimum. |

Radius uses existing `--r-md`/`--r-sm`; motion stays MOTION 1 (120ms color-only; reduced-motion kill
already universal in styles.css). No gradient, no glow, no shadow except the already-permitted drawer/
popover shadow token `--shadow-drawer` (reused for the account popover, as today).

---

## 5. Header + control visibility matrix (freeze #4)

### 5.1 The matrix (driven entirely by `activeSection`, §3)

| `activeSection` | `.panel-head` brand | `#btn-add` | `#btn-raw` | `#btn-local` | `#btn-reload` | Panel body |
|---|---|---|---|---|---|---|
| `host` (default) | `hosterm` | **shown** | **shown** | **shown** | **shown** | `#host-search` + `#host-list` |
| `keys` | `hosterm` | removed | removed | removed | removed | `#view-keys` content (unchanged) |
| `ansible` | `hosterm` | removed | removed | removed | removed | `#view-ansible` content (unchanged) |

The brand `hosterm` is constant across sections (panel head). `#btn-add` lives in `.panel-head`;
`#btn-raw`/`#btn-local`/`#btn-reload` live in `.panel-host-controls` (a child of `#view-host`, so they
naturally hide with the host view — but §5.2 is still mandatory to kill tabbability).

### 5.2 "Hidden" means not perceivable AND not focusable (R1 — the no-hidden-but-tabbable gate)

Hidden host controls MUST satisfy **both**:
1. Not rendered to the accessibility tree / not visible: the simplest correct mechanism is that
   `#btn-add` and `.panel-host-controls` live where they are removed from layout when not Hosts. For
   `#btn-add` (in `.panel-head`, shared across sections) the renderer toggles `hidden` attribute +
   `.hidden` class (which sets `display:none`). For `#btn-raw/#btn-local/#btn-reload` nested in
   `#view-host`, the existing `.sb-view.hidden { display:none }` already removes them, but the
   renderer MUST NOT rely on that alone (see 2).
2. Not focusable: `display:none` removes elements from the tab order by spec, which is sufficient.
   The implementation MUST NOT use `visibility:hidden`, `opacity:0`, or off-screen positioning for
   these controls (all of those keep elements tabbable → R1 regression). The smoke gate asserts the
   controls are **not present as focusable** on Keys/Ansible (§11).

Rule for the implementer (ADR-NAV-2, §13): section-scoped controls are hidden via `display:none`
(class/attr), never via a technique that leaves them in the tab order.

---

## 6. Account menu structure + wiring (freeze #5)

The account control (`#user-chip`) and its popover (`#signin-popover`) move verbatim to
`.rail-foot`. The chip's two states reuse the **existing static templates** (`#google-chip-signed-
out-template` / `#google-chip-signed-in-template`, index.html L75-85) — no markup rebuilt.

### 6.1 Menu contents per auth state (preserves IA-7)

| State | Chip shows | Popover items (top→bottom) |
|---|---|---|
| Signed out (`googleAccount == null`) | signed-out template ("Google account") | **Sign in with Google** (`#signin-btn`), **Settings** (`#acct-settings-btn`, NEW item) |
| Signed in (`googleAccount != null`) | signed-in template (name/avatar) | **Back up to Drive** (`#drive-backup-btn`), **Restore from Drive** (`#drive-restore-btn`), **Sign out** (`#signout-btn`), **Settings** (`#acct-settings-btn`) |

The existing `#drive-actions` block (backup/restore/status, L93-97) and `#signout-btn` keep their
show/hide logic driven by `googleAccount` (existing `renderGoogleAccount`). Drive status/loading/
error copy is **byte-identical** to today (AC-M4) — no copy touched.

### 6.2 Settings item — new wiring, same target (IA-4, AC-M5, AC-M7)

- A new static `<button id="acct-settings-btn">Settings</button>` is added to `#signin-popover`,
  present in BOTH states (it is not inside `#drive-actions`, so it never hides with sign-out).
- Click handler: close the account popover, then call the existing `openSettings()` (main.ts L964).
  No new settings code; only the entry point moves.
- The old footer gear `#open-settings` (index.html L70-72) and its listener (L980) are **removed**.
  Settings is reachable ONLY via the account menu (AC-M7). The smoke gate asserts no persistent
  footer Settings control exists (§11).
- Signed-out Settings works because `openSettings()` has no auth dependency (verified: L964 reads
  `appSettings` from the local store, no `googleAccount` gate). R3 (Settings unreachable when signed
  out) is closed by placing the item outside `#drive-actions`.

### 6.3 Popover anchoring, outside-click, Escape, focus trap/restore (AC-M2, AC-M6)

The account popover is a **non-modal menu** and MUST NOT use `overlayStack`/`openOverlay`/
`closeOverlay`. This is forced by live code: `openOverlay` sets `inert` on all of `#app` when the
stack goes from empty to one (main.ts L627). `#signin-popover` lives inside `.rail-foot` → `#nav-rail`
→ `#app`, so routing it through `openOverlay` would make the popover **itself** inert — breaking the
mandated focus trap and every click inside the menu. The shared modal stack is therefore the wrong
tool here; the account popover keeps its own non-modal open/close path (evolving `initGoogleAccount`
L562-594), and `#app` is **never** made inert for this popover.

Design (ADR-NAV-1, frozen — one design, no fallback):

- **Anchor:** popover anchors to `#user-chip` at the rail bottom. Because the chip moves from the
  footer to `.rail-foot`, the popover's CSS position changes from "above the footer chip" to
  "to the right of / above the rail-bottom chip" — purpose: stay on-screen when the chip sits at
  bottom-left (R5). Positioning is CSS-only; no JS geometry math (keep it simple).
- **Open:** chip click toggles `#signin-popover.hidden` via the existing `setGooglePopover` path.
  Opening never pushes `overlayStack` and never sets `inert` on `#app`.
- **Shared focus-trap helper (anti-duplication):** the Tab-cycle algorithm currently inlined in
  `overlayKeydown` (main.ts L637-651) is extracted into one reusable helper, e.g.
  `cycleFocusWithin(root: HTMLElement, ev: KeyboardEvent)`, which wraps Tab/Shift+Tab focus within
  `focusables(root)`. `overlayKeydown` calls `cycleFocusWithin(top.el, ev)` and the account-popover
  keydown handler calls `cycleFocusWithin($("#signin-popover"), ev)`. There is exactly **one**
  focus-trap algorithm, reused — not a second or third copy.
- **One init-time keydown handler:** the account popover registers a **single** keydown handler at
  init (inside `initGoogleAccount`), NOT per open — so there is no listener registered/leaked on each
  open. While the popover is open (`!#signin-popover.hidden`): Tab calls `cycleFocusWithin` to trap
  focus inside the popover (AC-M2); Escape closes the popover and restores focus to `#user-chip`
  (AC-M6). While the popover is closed the handler is a no-op (guards on the `.hidden` state).
- **Outside-click close:** the existing document click handler (L582-588) is preserved — close when
  the target is outside both the chip and the popover. It is registered once (not per open, no leak)
  and restores focus only where appropriate (a click into the rest of the app moves focus to the
  clicked target, so the handler does not force focus back to `#user-chip` on outside-click; focus is
  restored to `#user-chip` only on the Escape path, which has no other focus target).
- The popover keeps `role="dialog"` + `aria-labelledby` (index.html L86).

Acceptance bar (AC-M2 + AC-M6): Tab stays inside the open popover, Escape closes and restores focus to
`#user-chip`, outside-click closes. There is no `inert` on `#app` for this popover and no second
focus-trap implementation.

---

## 7. Keyboard / focus model + semantics (freeze #6)

Tab order within the shell (IA-8), left region first:
1. `#nav-rail` items top→bottom: Hosts → Keys → Ansible (each `button.rail-item` with an accessible
   name; icon-first so `aria-label` + `title` are mandatory, S-9 rule).
2. Account control `#user-chip` (rail bottom).
3. Into `#section-panel`: head action first where present (`#btn-add` on Hosts), then
   `#host-search` / list / section controls; on Keys/Ansible the head action is absent so tab lands
   on the section's own first control.
4. Into `#main` (workspace).

Semantics:
- Selected rail item carries `aria-current="page"`; exactly one at a time (R2). Others `"false"`.
- Every rail item and the account control has an accessible name (`aria-label`; the current
  `.sb-nav` buttons already carry `title` + a visible `.sb-nav-label` — the rail keeps an accessible
  name even when the visible label is hidden by the icon-first layout; if the label text is visually
  hidden, an `aria-label` is mandatory per AC-S1).
- `:focus-visible` ring (existing global rule, DESIGN.md §11 / styles.css) applies to every rail and
  panel control. The current `.sb-nav:focus-visible { outline:none; border-color }` pattern is
  **replaced** by the global `:focus-visible` ring to avoid a removed-indicator-without-replacement
  regression on the new rail items (R-32 hard gate).
- Within the open account menu, Tab cycles inside the popover only, via the shared
  `cycleFocusWithin` helper (§6.3); the account popover is non-modal and is NOT on `overlayStack`.
- Ctrl+K (host search focus) is gated on **all three**: `overlayStack.length === 0` (no modal open,
  existing L3932) **AND** the account popover is closed (`#signin-popover` has `.hidden`) **AND**
  `activeSection === "host"` (AC-H5: it focuses `#host-search`, which only exists/visible on Hosts).
  The popover-closed clause is required precisely because the non-modal popover never enters
  `overlayStack`, so the `overlayStack.length === 0` check alone would not suppress Ctrl+K while the
  account menu is open.

---

## 8. Collapse / resize + terminal re-fit contract (freeze #7)

### 8.1 Collapse model migration (ADR-NAV-3 — explicit per IA-9)

**Today:** `body.sb-collapsed` narrows the whole `#sidebar` to 36px and hides everything but
`#btn-expand` (styles.css L252-253; `setCollapsed` main.ts L3959). **New model:** the **rail is
never collapsed** (it is the persistent anchor); the **section panel** is what collapses. This is a
deliberate migration (IA-9 authorizes it). Concretely:
- `body.sb-collapsed` → renamed concept `body.panel-collapsed` (§9 migration). When set,
  `#section-panel { display:none }` and `#sb-resize` is inactive; `#nav-rail` and `#main` remain.
- The collapse/expand affordance moves from the old sidebar head chevrons to a control on the rail
  (a rail-foot or panel-edge toggle); `#btn-collapse`/`#btn-expand` ids may be reused in their new
  position. The exact placement is a lead/implementation call within this contract; the invariant is
  "rail stays, panel toggles."

### 8.2 Terminal re-fit (R4 / AC-S3 — the proven contract, preserved)

Any width change to the left region — resizer drag end, collapse, expand, narrow-window auto-
collapse — MUST re-fit active terminal sessions so the workspace fills reclaimed width with no blank
right margin, exactly as `setSidePanel` does today. The mechanism is the existing one:

```ts
// after any left-region width mutation:
setTimeout(fitActiveSessions, 0);   // existing fn, main.ts L1008-1017
```

`fitActiveSessions` fits both `sessions` and `splitSessions` for the active tab and calls
`pty_resize` (L1010-1015). The `setTimeout(…,0)` defers the fit until after layout settles (the
proven `setSidePanel` pattern, L1035, and the current `setCollapsed` path, L3968). The global
`window.addEventListener("resize", fitActiveSessions)` (L3865) already covers viewport resize; the
new collapse/resize paths MUST call `fitActiveSessions` the same way so narrow-window auto-collapse
also re-fits. The terminal scroll control stays clickable in every rail/panel state (the known
discovery-layout-sidebar-term-v1 class of bug).

### 8.3 No listener leak (R4 cleanup)

The resizer and collapse use the existing manual-mouse pattern (mousedown/mousemove/mouseup) — NOT
HTML5 DnD (invariant #4). Any `window`/`document` listener added for resize drag MUST be removed on
mouseup (the existing `#sb-resize` drag already does this). The narrow-window auto-collapse uses a
single media-query / resize observer, not a per-section listener; it is registered once at init and
never per-`selectSection`. No detached-DOM listener retention (the SEC class from t_fb1d18d6).

---

## 9. CSS + selector/class migration plan (freeze #9)

Old → new, so QA can diff coverage and the horizontal `#sidebar-nav` behavior is retired cleanly:

| Old (retired) | New | Notes |
|---|---|---|
| `#sidebar` (single column, 248px) | split into `#nav-rail` (56px) + `#section-panel` (248px) | `#sidebar` id MAY be reused for `#section-panel` to minimize churn, OR replaced; lead's call. The 248/160/504 geometry (§4) lands on the panel. |
| `#sidebar-nav` horizontal strip (`display:flex` row, L209-214) + `.sb-nav` (flex row items) | `.rail-nav` vertical (`flex-direction:column`) + `.rail-item` (icon-first, 44×44) | horizontal nav behavior is **retired**; the three destinations move to the vertical rail. `data-view` attribute kept so `selectSection` keeps its 1:1 mapping. |
| `.sidebar-head` (brand + `#btn-add` + `#btn-collapse`) | `.panel-head` (brand + `#btn-add`) | `#btn-add` moves into the panel head; collapse chevron moves to the rail (§8.1). |
| `.sidebar-footer` / `.sidebar-foot` (`#btn-raw`/`#btn-local`/`#btn-reload`/`#open-settings` + `#user-chip`) | `.panel-host-controls` (raw/local/reload, inside `#view-host`) + `.rail-foot` (`#user-chip`) | footer is **dissolved**: host controls become Hosts-scoped panel controls; `#open-settings` is **deleted** (→ account menu); `#user-chip`/popover move to rail bottom. |
| `body.sb-collapsed` (whole sidebar → 36px) | `body.panel-collapsed` (panel → hidden, rail stays) | migrate the localStorage key `hosterm.sb-collapsed` → `hosterm.panel-collapsed`; read old key once for backward-compat, then write new (one-time migration, no data loss). |
| `.sb-nav:focus-visible { outline:none }` | global `:focus-visible` ring | remove the local outline-kill so rail items get the proven ring (R-32). |

Rendered Dusk `:root` stays byte-exact (INV-21); the migration adds only layout/positioning rules
using existing tokens. The source scan for Abyss literals / `var(--glow)` stays 0 under `src/`.

---

## 10. Empty / loading / error states retained (freeze #8)

Every section keeps its current empty/loading/error structure and copy (DESIGN.md §12, PRD IA-10).
No copy is rewritten by this change:
- **Hosts:** empty ("No hosts in ~/.ssh/config yet. Click + to add one.", main.ts L55), loading
  skeleton (`.skel` shimmer, styles.css L298, no layout jump), config read-error (`.list-state.err`,
  with a live Reload path). These move with `#view-host` into the panel unchanged.
- **Keys:** `#sb-keys-list` + `#sb-keys-err` note/error (index.html L50-51), rendered by the
  existing `renderKeysList` lazy-init on section switch (L2452).
- **Ansible:** `#view-ansible` note + `#sb-ansible-open` (L53-59), unchanged.

The Add-host affordance (`#btn-add`) remains the empty-state CTA target on Hosts.

---

## 11. Verification matrix mapped to every PRD AC (freeze #11)

Baselines are **re-measured at dispatch** (STATE.md §Verify loop): cargo 87/87 unchanged
(backend diff 0), build 0 TS errors, smoke current canonical 343/343 → grows by the new checks
below (smoke counts may only grow, AGENTS.md §4). CSP-violation counter stays 0; served CSP ==
`tauri.conf.json`. Dusk `:root` byte-exact (INV-21). Network log stays empty (NFR-3).

| PRD AC | Verification (real, reproducible) |
|---|---|
| AC-H1 default Hosts + aria-current | smoke: at load `activeSection==host`, Hosts `.rail-item[aria-current="page"]`, `#view-host` visible. |
| AC-H2 host search+list | smoke: `#host-search` + `#host-list` present in panel on Hosts. |
| AC-H3 brand + Add host | smoke: `.panel-head` text `hosterm`, `#btn-add` present + opens editor (existing). |
| AC-H4 raw/local/reload shown + behavior | smoke: all three present on Hosts; click-through invokes existing mock paths. |
| AC-H5 Ctrl+K focuses search | smoke: on Hosts with `overlayStack` empty AND account popover closed, Ctrl+K → `#host-search` focused; AND Ctrl+K is a no-op while the account popover is open (§7 gate). |
| AC-H6 error/empty states | smoke: reuse existing host-list error/empty fixtures (`boom`/`empty`) inside the panel. |
| AC-K1 keys content only | smoke: on Keys, `#sb-keys-list`/`#sb-keys-add` present. |
| AC-K2 no host controls focusable | smoke: on Keys, `#btn-add`/`#btn-raw`/`#btn-local`/`#btn-reload` **not present as focusable** (query focusables, assert absent). |
| AC-K3 aria-current Keys | smoke: Keys `.rail-item[aria-current="page"]`, exactly one current. |
| AC-A1/A2/A3 Ansible | smoke: mirror K1/K2/K3 for Ansible (`#sb-ansible-open` present; host controls not focusable; aria-current). |
| AC-M1 account control at rail bottom | smoke: `#user-chip` is a descendant of `.rail-foot` inside `#nav-rail`. |
| AC-M2 menu opens, focus trapped | smoke: activate chip → popover open, `#app` NOT `inert`, `overlayStack` length 0, Tab stays inside popover (shared `cycleFocusWithin`). |
| AC-M3 signed-out items | smoke: signed-out → `#signin-btn` + `#acct-settings-btn` present. |
| AC-M4 signed-in items + Drive copy | smoke: signed-in (mock `googleAccount`) → backup/restore/signout/settings present; Drive status copy byte-identical. |
| AC-M5 Settings in both states | smoke: click `#acct-settings-btn` signed-out AND signed-in → Settings panel opens. |
| AC-M6 Escape/outside-click close + restore | smoke: Escape closes + focus back on `#user-chip`; outside-click closes. |
| AC-M7 no footer Settings | smoke: `#open-settings` does not exist; no persistent Settings control outside the menu. |
| AC-S1 three rail items, order, names | smoke: exactly 3 `.rail-item` in Hosts→Keys→Ansible order, each with accessible name. |
| AC-S2 keyboard reachable + ring | smoke: Tab order per §7; `:focus-visible` ring present (computed outline). |
| AC-S3 collapse/resize re-fits terminals | smoke: extend discovery-layout-sidebar-term assertions — after collapse/resize, terminal fits, no blank margin, scroll control clickable; `fitActiveSessions`/`pty_resize` called. |
| AC-S4 Dusk byte-exact + no network | smoke: INV-21 rendered `:root` vector + network log empty; source Abyss-literal scan 0. |

New smoke checks extend the existing patterns (section views, U-D6 Google chip/popover, U-D4
Settings) — no new harness. The migration rewrites old checks that asserted the horizontal
`#sidebar-nav`/footer structure; coverage is migrated, not deleted (R6).

---

## 12. Security notes (freeze #12)

- **All dynamic account/host text rendered inert (ADR-8 / INV-27, §13).** The chip name, avatar,
  Drive status, host names, and any remote-origin string continue to route through the existing
  escape helpers / static templates. The account chip already uses static templates
  (`#google-chip-*-template`, U-D6 `7cccf6c` sink fix); relocating the chip MUST keep template
  cloning — no `innerHTML` from the Google account name or Drive status. New rail/panel/menu
  surfaces are static markup cloned/toggled, never built from attacker-influenced strings.
- **No token material logged or surfaced.** This change touches no OAuth/Drive code; refresh tokens
  stay in the OS keyring (backend), never cross IPC into the renderer, and are never written to the
  DOM, console, or any new field. The chip shows only the account display name/email already exposed
  today.
- **No `client_secret`, no credential workaround.** The relocation is visual/IA only; the blocked
  OAuth client-type issue stays with `t_a7c6ee14` (§1.5).
- **CSP unchanged (frozen).** No `tauri.conf.json` edit; `script-src 'self'`, scoped `connect-src`,
  `style-src` the only `'unsafe-inline'`. The smoke CSP-violation counter stays 0.
- **No new network call (NFR-3).** Zero outbound traffic from this change; the network log assertion
  stays empty.
- **No secrets in `~/.ssh/config`; no sidecar/config change** (invariants #1, #2) — backend diff 0.

---

## 13. ADRs and invariants for QA/security review (freeze #13)

### ADRs

- **ADR-NAV-1 — Account popover is a non-modal path, NOT the shared overlay stack (frozen, no
  fallback).** `#signin-popover` stays inside `.rail-foot`/`#nav-rail`/`#app` and is never passed to
  `openOverlay`/`closeOverlay`. Forcing reason: `openOverlay` sets `inert` on `#app` (main.ts L627);
  because the popover is a descendant of `#app`, the shared stack would inert the popover itself and
  kill the mandated focus trap and in-menu clicks. Design: (1) the popover keeps its own non-modal
  open/close (`setGooglePopover`, `initGoogleAccount` L562-594) and `#app` is never made inert for
  it; (2) the Tab-cycle algorithm in `overlayKeydown` (L637-651) is extracted into one reusable
  helper `cycleFocusWithin(root, ev)` that BOTH `overlayKeydown` and the account-popover keydown
  handler call — exactly one focus-trap algorithm, not a second/third copy; (3) the popover has ONE
  init-time keydown handler (not per open): while open, Tab → `cycleFocusWithin($("#signin-popover"))`
  traps focus (AC-M2); Escape closes and restores focus to `#user-chip` (AC-M6); (4) the existing
  outside-click close stays, registered once (no per-open listener, no leak), and restores focus only
  where appropriate (Escape restores to `#user-chip`; outside-click lets focus follow the clicked
  target); (5) Ctrl+K requires `overlayStack.length === 0` AND popover closed AND Hosts (§7).
  Rationale: a single frozen non-modal design satisfies AC-M2/AC-M6 without the `inert` self-defeat
  and without duplicating the focus trap. There is no "whichever path" fallback.
- **ADR-NAV-2 — Section-scoped controls hide via `display:none`, never a tabbable technique.**
  `#btn-add` and host controls are removed from layout (class/attr toggling `display:none`) when not
  Hosts, which removes them from the tab order by spec. `visibility:hidden`/`opacity:0`/off-screen
  are forbidden for these controls (they stay tabbable → R1). Rationale: close the no-hidden-but-
  tabbable risk structurally (§5.2).
- **ADR-NAV-3 — Collapse collapses the panel, not the rail.** The rail is the persistent anchor and
  is never collapsed; `body.panel-collapsed` hides `#section-panel` and leaves `#nav-rail` + `#main`.
  Rationale: a collapsible rail would remove the one stable navigation anchor the PRD requires
  (IA-9); the workspace reclaims the panel's width (§8.1).
- **ADR-NAV-4 — Narrow-window auto-collapse, rail persists.** Below ~520px total left+workspace
  budget, the panel auto-collapses (rail + workspace remain). Rationale: three columns never break
  layout at minimum width (AC-S3); rail + workspace are the irreducible shell (§4).
- **ADR-NAV-5 — Backend is forbidden for this feature.** No `src-tauri/**`, no new IPC, IPC gate
  stays 42; CSP frozen. Rationale: this is an IA/visibility change (PRD IA-11); any backend edit is
  out of scope and a review-blocker (§1.4).

### Invariants (new, named for QA/SEC gates)

- **INV-NAV-1 (single nav source of truth):** exactly one `activeSection` variable drives
  `aria-current`, view visibility, and the §5 matrix; no duplicated per-control visibility state.
  Gate: code review + smoke (exactly one `aria-current="page"` at all times; visibility matches
  `activeSection`).
- **INV-NAV-2 (no hidden-but-tabbable):** on Keys/Ansible, `#btn-add`/`#btn-raw`/`#btn-local`/
  `#btn-reload` are absent from the focusables set. Gate: smoke focusables query (AC-K2/AC-A2).
- **INV-NAV-3 (exactly-one-current + focus never stranded):** section switch keeps one current rail
  item and moves focus into the panel. Gate: smoke (AC-K3/AC-A3 + focus target assert).
- **INV-NAV-4 (Settings reachable in both auth states, no footer gear):** `#acct-settings-btn`
  present + functional signed-out and signed-in; `#open-settings` deleted. Gate: smoke (AC-M5/AC-M7).
- **INV-NAV-5 (terminal re-fit on every left-region width change):** resize/collapse/expand/narrow
  auto-collapse all call `fitActiveSessions` (→ `pty_resize`), no blank margin, scroll clickable, no
  listener leak. Gate: smoke (AC-S3) + listener-teardown probe (`window.__sigList`).
- **INV-27 (safe DOM for account/host text) — reaffirmed:** account chip/name/Drive status and host
  names render via static templates/escape helpers, never dynamic `innerHTML`. Gate: smoke hostile-
  text fixture + source scan (reuses U-D6 pattern). (Numbered 27 to continue the INV-21..26 Dusk
  series; QA/SEC treat it as the ADR-8 safe-render rule applied to this feature.)

> Existing invariants untouched and reaffirmed: INV-21 (anti-Abyss, `:root` byte-exact), INV-22..26
> (store isolation / accent ink / corrupt defaults / approval weight / alias guard), the six hard
> invariants (config parseable, round-trip, paste normalize, manual-mouse drag, staging grep,
> authorized_keys). Backend diff 0, IPC 42, CSP frozen, NFR-3 no-network.

---

## 14. OAuth blocker note (referenced, NOT fixed — freeze #14)

The released app already has real OAuth PKCE loopback and real Google Drive backup/restore. The open
blocker is **configuration**: the public client ID shipped in v0.13.3 is registered as a *Web*
application client, so Google's token endpoint rejects the secretless desktop exchange with
`invalid_request (client_secret is missing.)`. Board card **`t_a7c6ee14`** separately owns creating a
Desktop app client and swapping only the public client ID. **This SDD does not claim that blocker is
fixed**, changes no OAuth/Drive code, and introduces no `client_secret` or credential workaround. The
account menu preserves the current real sign-in / Drive backup / Drive restore / sign-out behavior
while relocating those actions to the rail-bottom menu.

---

## 15. Hand-off

SDD path: `docs/sdd/left-nav-rail-contextual-controls-v1.md`. Feeds the lead's TSD + implementation
cards (component build, exact token/class names within §4/§9 geometry, smoke finalization) and the
QA/security gate (§11 matrix, §13 ADRs/invariants). Architect owns the HOW (frozen above); lead owns
the build spec. Backend diff 0; no push by this task. Report commit + files to nano; nano re-verifies.
