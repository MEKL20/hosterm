# PRD: hosterm left navigation rail and contextual controls (v1)

Status: PRD (PM-owned doc). Author: crew-coding-pm. Date: 2026-11-06.
Repo: hosterm (Tauri v2 + TypeScript + xterm.js). Documentation only — no code here.
Owner decisions below are **LOCKED** (per task t_bfb6b3fc body) and are not reopened.

Antislop mode: **DURING** design and implementation. Keep ENERGY 1 / RHYTHM 2 /
MOTION 1 (DESIGN.md §1). Dusk identity and theme tokens (docs/STATE.md §Dusk) stay
byte-exact; INV-21 anti-Abyss and the no-network NFR (PRD NFR-3) are untouched by
this change.

Grounded against live code read 2026-11-06: `index.html` `#sidebar`, `src/main.ts`
`selectSidebarView` / `initGoogleAccount` / overlay stack, `src/styles.css` Dusk
`:root`. See §Further Notes for the current-state inventory this PRD migrates from.

---

## Problem Statement

The operator uses hosterm as a single-window SSH/SFTP tool. Today the left region
is **one column** that stacks everything: a brand head with Add-host and a
collapse chevron, a three-item nav strip (Hosts / Keys / Ansible), the current
section's body, and a persistent footer that always shows Raw config, Local,
Reload config, a Settings gear, and the Google account chip.

From the operator's seat this creates three concrete problems:

1. **Global navigation and section tasks share one surface.** Moving between
   Hosts, Keys, and Ansible lives in the same column as the controls that act on
   the current section, so "where I am" and "what I can do here" are visually
   interleaved. There is no persistent, always-visible anchor for switching
   sections — the nav strip scrolls and compresses with everything else.

2. **Section-specific controls are shown globally.** Raw config, Local, Reload
   config, and Add host are `~/.ssh/config`/Hosts concerns, but the footer renders
   Raw config / Local / Reload config on **every** section, and Add host lives in
   the head regardless of section. On Keys or Ansible these controls are noise:
   they act on hosts, not on the section the operator is looking at.

3. **Settings is a persistent footer gear competing with real navigation.**
   Settings is an app-wide preference surface, not a per-section action, yet it
   occupies permanent footer space next to section tools, and the account chip
   (sign-in, Drive backup/restore, sign-out) sits below it as a separate element.

The operator wants a shell where global navigation is a stable anchor, each
section shows only its own controls, and app/account concerns live in one place
out of the way of section work.

## Solution

Restructure the left region into **three columns**, left to right:

1. **Navigation rail** (narrow, far left): persistent, icon-first global
   navigation. Items top-to-bottom: **Hosts** (default), **Keys**, **Ansible**.
   A **Google account control** sits at the bottom of the rail and opens an
   account menu/popover. The rail is the one stable anchor for "where am I".

2. **Contextual section panel** (beside the rail): shows the current section's
   list/content and the controls that belong to that section only. Its header
   shows the brand `hosterm`. The header's primary action is section-scoped.

3. **Workspace** (right, the focal point): tabs, terminals, SFTP, editor, status
   bar — unchanged in purpose. It stays the loudest thing on screen (DESIGN.md §0).

This is the owner's chosen interpretation of a ChatGPT-style shell: a thin rail +
a contextual panel + a focal workspace. It is an **interaction reference, not a
visual clone** — do not copy ChatGPT visuals, and add no decorative assets.

Section-scoped visibility is the core behavior:

- **Hosts selected** (default): the contextual panel shows host search + host
  list plus **Raw config**, **Local**, **Reload config**. The panel header shows
  brand `hosterm` and **Add host**.
- **Keys or Ansible selected**: Raw config, Local, Reload config, and Add host are
  **not visible**. The panel shows only that section's existing controls/content.

**Settings** is removed from the persistent footer and becomes a real item inside
the **account menu/popover**. It must work **signed in and signed out**. The
account menu also preserves the current Google backup / restore / sign-out
actions.

All existing destinations are reused verbatim — this is an information-architecture
and visibility change, not new features and not a backend change.

## User Stories

### Shell / navigation rail

1. As an operator, I want a narrow vertical rail pinned to the far left showing
   Hosts, Keys, and Ansible, so that I can switch sections from a stable anchor
   that never scrolls away with section content.
2. As an operator, I want **Hosts** selected by default on launch, so that the
   app opens on the surface I use most without a click.
3. As an operator, I want the selected rail item to be visibly current and exposed
   to assistive tech as the current item, so that I always know which section I am
   in, by sight and by screen reader.
4. As a keyboard operator, I want to reach every rail item and the account control
   by Tab in a predictable order with a visible focus ring, so that I can navigate
   the shell without a mouse.
5. As an operator, I want each rail item to carry a text label and/or an accessible
   name, so that an icon-only rail is never ambiguous about what a button does.
6. As an operator, I want selecting a rail item to swap the contextual panel to
   that section and move keyboard focus sensibly into the panel, so that section
   switching is one action and my focus is not stranded on the rail.

### Contextual section panel — Hosts

7. As an operator, I want the Hosts panel to show the host search field and the
   host list, so that finding and opening a host is unchanged from today.
8. As an operator, I want the Hosts panel header to show the brand `hosterm` and an
   **Add host** action, so that creating a host is one click from the section where
   hosts live.
9. As an operator, I want **Raw config**, **Local**, and **Reload config** visible
   in the Hosts contextual panel, so that the controls that act on
   `~/.ssh/config` are present exactly where host work happens.
10. As an operator, I want host-search focus (Ctrl+K), the empty state, loading
    skeletons, and the config read-error state to behave exactly as they do today,
    so that the restructure costs me no existing behavior.

### Contextual section panel — Keys

11. As an operator, I want selecting **Keys** to show only the keys section's
    existing content and controls (the managed-key list, its add-key action, and
    its note/error states), so that the panel is scoped to keys.
12. As an operator, I want Raw config, Local, Reload config, and Add host to be
    **absent** while Keys is selected, so that host-only controls do not clutter the
    keys surface.

### Contextual section panel — Ansible

13. As an operator, I want selecting **Ansible** to show only the Ansible section's
    existing content and controls (its note and the control that opens the runner),
    so that the panel is scoped to Ansible.
14. As an operator, I want Raw config, Local, Reload config, and Add host to be
    **absent** while Ansible is selected, so that host-only controls do not clutter
    the Ansible surface.

### Account menu (rail bottom)

15. As an operator, I want a Google account control at the bottom of the rail that
    opens an account menu/popover, so that app/account concerns are in one place out
    of the way of section work.
16. As a signed-out operator, I want the account menu to offer **Sign in with
    Google** and a working **Settings** item, so that I can reach settings and start
    sign-in before having an account.
17. As a signed-in operator, I want the account menu to show my account and offer
    **Back up to Drive**, **Restore from Drive**, **Sign out**, and **Settings**, so
    that every current account action and settings are reachable from one menu.
18. As an operator, I want **Settings** to open the existing Settings panel from the
    account menu in **both** signed-in and signed-out states, so that preferences are
    never gated behind sign-in.
19. As a keyboard operator, I want the account menu to open, trap focus while open,
    close on Escape, and restore focus to the account control on close, so that the
    menu follows the same overlay rules as the rest of the app (DESIGN.md §10).
20. As an operator, I want the account menu's Drive status, loading, and error
    messages to read exactly as they do today, so that backup/restore feedback is
    unchanged.

### Collapse / responsive / preservation

21. As an operator on a narrow window, I want the shell to stay usable at its
    minimum width with a defined collapse/resize behavior, so that the three columns
    do not break the layout when space is tight.
22. As an operator, I want collapse/resize to re-fit the active terminal(s) so the
    workspace fills reclaimed width with no blank margin, so that the known
    collapse-refit behavior (discovery-layout-sidebar-term-v1) is preserved, not
    regressed.
23. As an operator, I want every preserved surface (keyboard operation, visible
    focus, `aria-current`, empty/loading/error states) to behave as it does today
    unless a migration is explicitly documented, so that the restructure is safe.

## Implementation Decisions

These are product-level resolved decisions. The architect (SDD) and lead (TSD) own
component structure, DOM, and token mapping; this PRD describes contracts and
visibility, not file paths or code.

### IA-1 — Three-column shell

The left region splits into a **navigation rail** (far left) and a **contextual
section panel** beside it; the **workspace** stays the third column and the focal
point. The rail serves persistent global navigation; the contextual panel serves
current-section tasks (task body "Visual direction").

### IA-2 — Rail items are real destinations only

Rail items are exactly **Hosts**, **Keys**, **Ansible**, mapping 1:1 to the three
existing section views in the current single sidebar (the `host` / `keys` /
`ansible` views switched by the current `selectSidebarView`). **Hosts is the
default** selected item. No new destination is invented.

### IA-3 — Account control at rail bottom

The existing Google account chip moves to the **bottom of the rail**. Activating it
opens the existing account popover (the current sign-in / Drive / sign-out
surface), repositioned as the rail's account menu. Its anchor-and-dismiss, outside-
click close, and Escape-close behavior are preserved.

### IA-4 — Settings relocation

Settings is **removed** from the persistent footer (the current footer gear is
retired from its always-visible position) and becomes a **menu item inside the
account menu/popover**. It opens the existing Settings panel unchanged. The item is
present and functional in **both** the signed-out and signed-in account-menu
states. No other Settings behavior changes.

### IA-5 — Contextual control visibility matrix

The contextual panel renders section-scoped controls per the matrix below. "Host
controls" = Raw config, Local, Reload config. "Add host" = the panel header's
primary action.

| Selected section | Panel header | Add host | Raw config | Local | Reload config | Section body/controls |
|---|---|---|---|---|---|---|
| **Hosts** (default) | brand `hosterm` | **visible** | **visible** | **visible** | **visible** | host search + host list |
| **Keys** | brand `hosterm` | hidden | hidden | hidden | hidden | managed-key list + add-key + note/error (existing) |
| **Ansible** | brand `hosterm` | hidden | hidden | hidden | hidden | Ansible note + open-runner control (existing) |

Visibility is driven by the selected section. Hidden controls are not rendered or
are not perceivable/focusable while hidden (no hidden-but-tabbable controls).

### IA-6 — Selected-state behavior

Selecting a rail item: (a) marks that item current (`aria-current="page"` on the
selected item, cleared on the others — current `selectSidebarView` semantics
preserved); (b) swaps the contextual panel to that section's content and controls;
(c) applies the IA-5 visibility matrix; (d) moves keyboard focus into the panel in a
sensible first-focus target (e.g. host search on Hosts) rather than leaving focus on
the rail. Default on launch: Hosts selected, its panel shown.

### IA-7 — Account-menu states

| State | Account control shows | Menu items |
|---|---|---|
| Signed out | generic Google account affordance (current signed-out chip) | **Sign in with Google**, **Settings** |
| Signed in | account identity (current signed-in chip) | **Back up to Drive**, **Restore from Drive**, **Sign out**, **Settings** |

Drive status/loading/error copy is byte-identical to today. Settings appears in
both states (IA-4). The menu is a dialog-role popover with focus trap, Escape
close, outside-click close, and focus restore to the account control.

### IA-8 — Keyboard order and semantics

Tab order within the shell: rail items top-to-bottom (Hosts → Keys → Ansible), then
the account control at the rail bottom, then into the contextual panel's controls
(header action first where present, then search/list/section controls), then the
workspace. The selected rail item carries `aria-current="page"`; every rail control
and the account control has an accessible name; `:focus-visible` rings are visible
on every rail and panel control (DESIGN.md §11). Within the account menu, Tab cycles
inside the open popover only.

### IA-9 — Responsive / min-width / collapse

The shell keeps a defined minimum usable width and a collapse/resize behavior. The
rail stays visible as the persistent anchor; the contextual panel is the element
that collapses/narrows under width pressure. Any collapse or width change **must
re-fit active terminal sessions** so the workspace fills reclaimed width with no
blank right margin, and the scroll control stays clickable in every rail/panel state
(preserving the fixes/behavior tracked in discovery-layout-sidebar-term-v1). If the
migration must change the current single-sidebar collapse model (e.g. "collapse the
contextual panel" vs "collapse the whole sidebar"), that change is **documented
explicitly** in the SDD as a deliberate migration; otherwise current
collapse/resize semantics are preserved.

### IA-10 — Empty / loading / error states preserved

Every section's empty, loading (skeleton), and error states keep their current
structure and copy (DESIGN.md §12 state matrix; PRD FR state rules). Specifically:
the Hosts empty state ("No hosts in ~/.ssh/config yet…"), the host-list loading
skeleton with no layout jump, the config read-error state with a live Reload path;
the Keys list note/error; the Ansible note. Copy is not rewritten by this PRD.

### IA-11 — No backend / no new IPC

This is a frontend IA/visibility change. No backend command, no new IPC, no change
to `~/.ssh/config` / secrets sidecar handling, no change to the Dusk `:root` tokens,
no new network call. The IPC inventory count (docs/STATE.md §Verify loop) is
unchanged.

## Acceptance Criteria (EARS)

### Hosts

- AC-H1: WHEN the app launches THE SYSTEM SHALL select **Hosts** by default, mark
  the Hosts rail item `aria-current="page"`, and show the Hosts contextual panel.
- AC-H2: WHILE Hosts is selected THE SYSTEM SHALL show the host search field and the
  host list in the contextual panel.
- AC-H3: WHILE Hosts is selected THE SYSTEM SHALL show the panel header brand
  `hosterm` and an **Add host** action that opens the existing host editor.
- AC-H4: WHILE Hosts is selected THE SYSTEM SHALL show **Raw config**, **Local**,
  and **Reload config**, each invoking its existing behavior (open raw
  `~/.ssh/config` editor, open a local terminal tab, reload the host list).
- AC-H5: WHEN the operator presses Ctrl+K while Hosts is selected THE SYSTEM SHALL
  focus the host search field (existing behavior preserved).
- AC-H6: WHEN `~/.ssh/config` cannot be read THE SYSTEM SHALL show the existing
  config read-error state with a live Reload path, and WHEN there are no hosts THE
  SYSTEM SHALL show the existing empty-state copy and the Add-host affordance.

### Keys

- AC-K1: WHEN the operator selects **Keys** THE SYSTEM SHALL show only the keys
  section's existing content and controls (managed-key list, add-key, note, error)
  in the contextual panel.
- AC-K2: WHILE Keys is selected THE SYSTEM SHALL NOT show Add host, Raw config,
  Local, or Reload config (not rendered and not focusable).
- AC-K3: WHILE Keys is selected THE SYSTEM SHALL mark the Keys rail item
  `aria-current="page"` and clear it from the others.

### Ansible

- AC-A1: WHEN the operator selects **Ansible** THE SYSTEM SHALL show only the
  Ansible section's existing content and controls (note + open-runner control) in
  the contextual panel.
- AC-A2: WHILE Ansible is selected THE SYSTEM SHALL NOT show Add host, Raw config,
  Local, or Reload config (not rendered and not focusable).
- AC-A3: WHILE Ansible is selected THE SYSTEM SHALL mark the Ansible rail item
  `aria-current="page"` and clear it from the others.

### Account menu

- AC-M1: THE SYSTEM SHALL place the Google account control at the **bottom of the
  rail**.
- AC-M2: WHEN the operator activates the account control THE SYSTEM SHALL open the
  account menu/popover with focus trapped inside it.
- AC-M3: WHILE signed out THE SYSTEM SHALL offer **Sign in with Google** and
  **Settings** in the account menu.
- AC-M4: WHILE signed in THE SYSTEM SHALL offer **Back up to Drive**, **Restore
  from Drive**, **Sign out**, and **Settings** in the account menu, with Drive
  status/loading/error copy byte-identical to today.
- AC-M5: WHEN the operator activates **Settings** from the account menu in either
  state THE SYSTEM SHALL open the existing Settings panel.
- AC-M6: WHEN the operator presses Escape or clicks outside the open account menu
  THE SYSTEM SHALL close it and restore focus to the account control.
- AC-M7: THE SYSTEM SHALL NOT render a persistent Settings control in the footer
  (Settings is reachable only via the account menu).

### Shell / a11y / responsive

- AC-S1: THE SYSTEM SHALL expose exactly three rail items — Hosts, Keys, Ansible —
  in that top-to-bottom order, each with an accessible name.
- AC-S2: THE SYSTEM SHALL make every rail item and the account control reachable by
  keyboard with a visible `:focus-visible` ring, in the order defined in IA-8.
- AC-S3: WHEN the shell collapses or is resized THE SYSTEM SHALL re-fit active
  terminal session(s) so the workspace fills the available width with no blank
  right margin, and keep the terminal scroll control clickable in every state.
- AC-S4: THE SYSTEM SHALL keep the rendered Dusk `:root` byte-exact (INV-21) and
  make zero network calls as a result of this change (NFR-3).

## Testing Decisions

- **Test external behavior, not DOM internals.** Good tests assert what the operator
  perceives: which section is current (`aria-current`), which controls are
  present/absent per section, the account-menu item set per state, and focus/Escape
  behavior — not specific class names or node structure.
- **Modules exercised:** the section-switch + visibility logic (the evolution of the
  current `selectSidebarView`), the account-menu open/close/focus logic (evolution
  of the current Google-popover init), and the Settings-open path from the account
  menu.
- **Extend the headless smoke** (`qa-reports/headless-smoke.mjs`) in the same change
  that implements this, per AGENTS.md §4. New checks to add (illustrative, lead/QA
  finalize exact assertions):
  - Default section = Hosts; Hosts rail item `aria-current="page"` at load.
  - Visibility matrix (IA-5): on Keys and on Ansible, assert Add host / Raw config /
    Local / Reload config are not present and not focusable; on Hosts assert all four
    present.
  - Account control is at the rail bottom; activating it opens a focus-trapped menu;
    Settings item present and opens Settings in **both** signed-out and signed-in
    account-menu states; Drive/sign-out items present when signed in.
  - No persistent footer Settings control (AC-M7).
  - `aria-current` moves correctly across Hosts/Keys/Ansible selection.
  - Collapse/resize re-fits terminals (reuse/extend the
    discovery-layout-sidebar-term smoke assertions) with no blank margin.
  - Rendered `:root` stays byte-exact Dusk (INV-21) and the network log stays empty
    (NFR-3).
- **Prior art:** the existing smoke suite already covers section views, the Google
  chip/popover (U-D6), Settings/Appearance (U-D4), reduced-motion, CSP-hostile, and
  INV-21/23/25 gates — extend those patterns rather than inventing a new harness.
- **Baselines are not hardcoded here.** The canonical smoke/cargo counts live in
  docs/STATE.md §Verify loop and are re-measured at dispatch; smoke counts may only
  grow (AGENTS.md §4).

## Out of Scope

Explicit non-goals (per task body):

- **No OAuth credential workaround.** This PRD does not fix, enable, or work around
  Google OAuth. The account control and its Sign-in item remain the existing
  **visual-only** surface (OQ-3). See §Further Notes "OAuth blocker".
- **No new backend API / no new IPC.** This is a frontend IA/visibility change;
  backend diff is 0 and the IPC inventory count is unchanged.
- **No client_secret in the binary.** No embedded OAuth client secret, no change to
  credential handling, no new secret storage.
- **No unrelated visual redesign.** Dusk tokens, ENERGY/RHYTHM/MOTION dials, and the
  existing component styling stay as-is except for the structural rail/panel split
  and the control-visibility/relocation changes specified here. No decorative assets.
- **No ChatGPT visual cloning.** ChatGPT is an interaction reference for the
  rail+panel+workspace shape only.
- **No change to section features themselves.** Hosts, Keys, and Ansible keep their
  current content/behavior; only where their controls appear changes.
- **No new section/destination.** Exactly the three existing destinations.

## Affected user flows and regression risks

Affected flows (what an operator does that this change touches):

1. **Section switching** (Hosts ↔ Keys ↔ Ansible) — moves from the top nav strip to
   the far-left rail; selection + `aria-current` semantics must carry over.
2. **Host CRUD and connect** — Add host moves to the Hosts panel header; host
   search/list and open-terminal behavior must be unchanged.
3. **Config operations** — Raw config / Reload config become Hosts-scoped (hidden on
   Keys/Ansible); their behavior is unchanged where shown.
4. **Local terminal** — the Local control becomes Hosts-scoped; opening a local
   terminal tab is otherwise unchanged.
5. **Settings** — reached via the account menu instead of the footer gear; must work
   signed in and out.
6. **Google account** — sign-in / Drive backup / Drive restore / sign-out move into
   the rail-bottom account menu; copy and behavior preserved.
7. **Collapse/resize** — the collapse model may change shape (rail persists, panel
   collapses); terminal re-fit must be preserved.

Regression risks to watch (hand to QA/SEC as the verify focus):

- **R1 — Hidden-but-tabbable controls.** Keys/Ansible must not leave Add
  host/Raw/Local/Reload focusable while hidden (keyboard trap / unexpected action).
- **R2 — Lost `aria-current` / focus management.** Section switch must keep exactly
  one current item and not strand keyboard focus on the rail.
- **R3 — Settings unreachable when signed out.** Moving Settings into the account
  menu must not gate it behind sign-in (AC-M5 in both states).
- **R4 — Terminal geometry on collapse.** The new collapse model must re-fit
  sessions (no blank margin, scroll control clickable) — the exact class of bug in
  discovery-layout-sidebar-term-v1.
- **R5 — Google popover regressions.** Repositioning the chip to the rail bottom
  must keep outside-click close, Escape close, and focus restore working.
- **R6 — Smoke baseline drift.** Existing smoke checks that assert the old footer /
  nav-strip structure will need migration, not deletion of coverage; counts may only
  grow (AGENTS.md §4).
- **R7 — INV-21 / NFR-3 drift.** A restructure that touches styles must keep the
  rendered `:root` byte-exact Dusk and introduce no network call.

## Further Notes

### OAuth blocker (referenced, not fixed)

Google OAuth remains an open blocker (docs/STATE.md / PRD OQ-3: the Google account
surface is **visual-only** in v1; no OAuth/session/network backend is wired — U-D6
landed the accessible chip/popover explicitly as UI-only). This PRD **does not**
claim OAuth is fixed and **does not** introduce a credential workaround, a backend,
or a client secret. The account menu's Sign-in item stays the existing visual-only
affordance; Drive backup/restore/sign-out keep their current behavior. Any real
OAuth work is a separate, out-of-scope effort.

### Current-state inventory this PRD migrates from (read 2026-11-06)

For the architect/lead, the live starting point:

- **Single sidebar** (`#sidebar`): head with brand `hosterm` + Add-host (`#btn-add`)
  + collapse chevron; a top nav strip (`#sidebar-nav`) with Hosts/Keys/Ansible
  (`.sb-nav` buttons, `data-view`, `aria-current`); three section views
  (`#view-host` / `#view-keys` / `#view-ansible`) swapped by `selectSidebarView`;
  a persistent footer (`.sidebar-footer`) with Raw config (`#btn-raw`), Local
  (`#btn-local`), Reload config (`#btn-reload`), a **Settings gear**
  (`#open-settings`), and the **Google account chip** (`#user-chip`) + its popover
  (`#signin-popover`: Sign in / Back up to Drive / Restore from Drive / Sign out).
- **Section destinations are all real:** Hosts = `#host-search` (Ctrl+K) +
  `#host-list`; Keys = `#sb-keys-list` + add-key; Ansible = note +
  `#sb-ansible-open` → `openAnsible()` modal.
- **Settings panel** is the existing `#settings-panel` dialog (`openSettings`),
  reachable today only via the footer gear — this PRD moves that entry point into the
  account menu and makes it work signed-out too.
- **Collapse** today = `body.sb-collapsed` toggled by `setCollapsed`; the whole
  sidebar narrows to a thin strip. The new model keeps the rail as the persistent
  anchor and collapses the contextual panel; the exact migration is the SDD's call
  (IA-9) and must preserve terminal re-fit.

### Hand-off

PRD path: `docs/prd/left-nav-rail-contextual-controls-v1.md`. This feeds the
architect's SDD (component structure, DOM contract, token mapping, collapse-model
decision) and the lead's TSD/cards. PM owns the WHAT; architect owns the HOW. No
Kanban card is created by PM. Report to nano; nano re-verifies.
