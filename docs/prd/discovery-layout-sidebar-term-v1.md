# Discovery: Sidebar collapse ↔ terminal layout bugs (v1)

Status: grounding doc for crew. Author: nano (Tier-0 triage from real code).
Date: 2026-11-03. Repo: hosterm (Tauri + WebView2 + xterm.js 5.5, @xterm/addon-fit).

## Reported by MEKL (real usage, GUI)

- **BUG-L1 — terminal scroll control not clickable unless sidebar is minimized.**
  The right-side terminal scroll control (xterm scrollbar slider) cannot be
  clicked while the host-list sidebar is in its normal (non-collapsed) width.
  Minimizing (collapsing) the sidebar makes it clickable again.
- **BUG-L2 — collapsed sidebar leaves empty space / right margin; terminal not full width.**
  When the sidebar is collapsed (minimized), the terminal does NOT expand to
  fill the reclaimed width — there is leftover blank space / margin on the right.

## Code findings (confirmed by reading, not guessing)

### Layout structure (src/styles.css)
- `#app { display:flex; height:100vh }` (line 59) → sidebar + `#main` side by side.
- `#main { flex:1; display:flex; flex-direction:column; min-width:0 }` (60).
- `#workbench { flex:1; min-width:0; display:flex }` (321) → `#panels { flex:1; position:relative }` (322).
- `.term-host { width:100%; height:100%; padding:8px; overflow:hidden }` (326).
- Sidebar collapse state = `body.sb-collapsed` → `#sidebar { width:44px }` (133);
  children hidden except `#btn-expand` (134).

### ROOT CAUSE (confirmed) for BUG-L2 — missing fit() on collapse
`setCollapsed()` in src/main.ts (lines 2982–2986) is:
```
const setCollapsed = (c) => {
  document.body.classList.toggle("sb-collapsed", c);
  $("#btn-expand").classList.toggle("hidden", !c);
  try { localStorage.setItem("hosterm.sb-collapsed", c ? "1" : ""); } catch {}
};
```
It toggles the class + persists state but NEVER calls `fit()`/`pty_resize`.
Every OTHER resize path DOES re-fit the active sessions:
- window resize → `fitActiveSessions()` (main.ts:2887-2888)
- split divider drag → `s.fit.fit()` + `pty_resize` (2635-2636)
- sidebar width resizer drag → `s.fit.fit()` + `pty_resize` (2939-2940)
- tab activate → `s.fit.fit()` (2776, 2822)
So when the sidebar collapses/expands, the flex layout gives `#main` more/less
width, but xterm keeps its OLD cols/rows → the terminal canvas does not grow to
the new container width → blank margin on the right. This is BUG-L2.

### HYPOTHESIS (NOT yet confirmed visually) for BUG-L1 — scroll control
Likely the SAME root cause surfacing differently: with xterm geometry stale
(no re-fit), the `.xterm-viewport` scrollbar renders at the old width/position,
so its clickable hit-area ends up outside the visible terminal column (the blank
margin), i.e. not clickable. Collapsing the sidebar triggers... (still no fit,
but) a different width where the slider happens to land back inside the clickable
area — OR there is a separate z-index/overlay stealing pointer events over the
scrollbar at wide layout. Crew MUST verify BOTH possibilities before fixing:
  1. Re-fit on collapse (the L2 fix) and check whether L1 disappears too.
  2. If not, inspect `.xterm-viewport` / `.xterm-scroll-area` stacking +
     pointer-events, and any absolutely-positioned overlay over `#panels`
     (`.panel { position:absolute; inset:0 }` line 323 — panels stack; a hidden
     panel left capturing pointer events is a candidate).

## Acceptance criteria
- AC1 (L2): After collapse AND after expand, the active terminal(s) re-fit so the
  canvas fills the full `#main` width — no blank right margin. Verify cols/rows
  change and `pty_resize` is invoked. Also fit ALL visible sessions in a split.
- AC2 (L1): The terminal scroll control is clickable in BOTH sidebar states
  (normal width and collapsed). Scrolling via the slider works in both.
- AC3 (regression): window resize, split-divider drag, sidebar-width-resizer
  drag, and tab switch all still re-fit correctly (don't double-fire or thrash).
- AC4 (persistence): collapsed state still restored on restart (localStorage),
  and the restore path also fits (currently `setCollapsed` is called at startup
  line 2987 — restore must not leave a stale geometry either).

## Verifier gate (qa + security)
- qa: extend headless smoke to assert re-fit on collapse/expand (cols/rows or
  container width delta) + scroll-control hit-test in both states if feasible.
- Build 0 TS errors; existing smoke stays green (currently 359/359).
- Direct-push to origin/main (completion_contract = local-only).
