# UI/UX Review — Google account, passphrase overlay, backup/restore toasts

- Date: 2026-11-07
- Author: crew-coding-uiux (design review only, no production source touched)
- Task: t_9f7d8fe8
- Mode: **AUDIT + implementation-exact spec.** Antislop usage-mode (DURING vs AFTER) was
  never answered by MEKL; this file is an evidence-based spec only. It does NOT choose a new
  visual direction and does NOT implement. Direction stays **Dusk** (SDD §13.3, tokens in
  `src/styles.css:1-36`). Every taste choice that genuinely needs MEKL is marked
  **[NEEDS-MEKL]**.

## Scope and review boundary

Reviewed by reading source only:
- `index.html:11-55` (nav rail + user chip + sign-in popover), `index.html:424-454`
  (dialog + passphrase dialog markup).
- `src/main.ts:320-371` (chip/popover render), `373-383` (popover open/close),
  `414-488` (passphrase prompt + drive status), `489-578` (backup/restore flow + init),
  `605-687` (overlay stack: Escape, focus trap, focus restore).
- `src/styles.css:1-36` (Dusk tokens), `315-390` (chip/popover/drive/passphrase styles),
  `875-876` (reduced-motion kill switch), `1206-1225` (fixed-overlay selector + modal box).

**Not reviewed / cannot verify:** the live rendered app and the user screenshot. The
screenshot endpoint returned provider 401; I did not see it. Every geometric claim below is
from CSS/DOM source, not from a rendered pixel. Runtime layout at a real DPI, actual focus
ring rendering, and WebView2-specific behavior are unverified and belong to QA.

## Dials (inferred from established Dusk identity, not invented)

Dusk is a calm, warm, local-first developer tool: muted honey-amber accent used sparingly,
single elevation token (`--shadow-drawer`), 120ms micro-transitions only, and a global
reduced-motion kill switch already in place (`styles.css:875-876`). That identity reads as:

- **ENERGY 1 (Calm).** Reason: Dusk uses one restrained accent and flat surfaces; account/
  backup chrome is utility, not a hero. It must not shout.
- **RHYTHM 1 (Uniform).** Reason: this is a settings/account surface, not a marketing page;
  predictable structure (chip -> popover -> actions -> status) is the correct rhythm.
- **MOTION 1 (Hover states + short functional transitions only).** Reason: the codebase
  already caps motion at 120ms transitions and kills all animation under reduced-motion;
  overlay entry/exit must stay inside that budget, no loops, no parallax.

These dials match the existing Dusk code; they are a reading of it, not a new direction.

---

## 1. Current-state audit (numbered findings)

Severity: HIGH = Hard-Gate rule or confirmed functional break; MED = Purpose-Gate / a11y
gap; LOW = consistency/polish.

### F-1 — Passphrase dialog renders in normal flow, shoves the shell [HIGH] [CONFIRMED BUG]
- Rule: R-27 / R-32 / C-4 (resilience; the overlay is structurally wrong).
- Evidence: `src/styles.css:1207` fixed-overlay selector is
  `#modal, #raw-modal, #key-modal, #dialog, #ssh-modal, #ansible-modal` — `#pass-dialog`
  is **absent**. `index.html:436` places `#pass-dialog` as a sibling **after** `#app`
  (`index.html:11`). With no `position: fixed; inset: 0`, when `passphrasePrompt`
  (`main.ts:480` -> `openOverlay("#pass-dialog", input)`) removes `.hidden`, the dialog
  enters normal document flow below `#app`, so the rail/panel/terminal shell is pushed and
  the box is not centered. This is the reported "breaks shell/sidebar, feels solid/abrupt".
- Note: `#app` still gets `inert` (`main.ts:655`) so the shell is correctly frozen, but it
  stays visually displaced. The fix is purely the missing overlay rule + entry transition.

### F-2 — No entry/exit transition on any overlay; appearance is abrupt [MED]
- Rule: R-19 (motion must serve a purpose; abrupt state change is a UX gap, not slop, but
  the fix is cheap and within MOTION 1).
- Evidence: overlays toggle `.hidden` (`display:none !important`, `styles.css:1213`) with
  no opacity/transform transition. `openOverlay`/`closeOverlay` (`main.ts:651-687`) flip the
  class synchronously. The "solid/abrupt" feel is this plus F-1.

### F-3 — Backup/restore feedback is a single in-popover status line, not a toast [MED]
- Rule: R-27 (states) + user request ("wants popup feedback for success and failure").
- Evidence: `setDriveStatus` (`main.ts:484-488`) writes `#drive-status`
  (`index.html:50`, `role="status" aria-live="polite"`). It lives **inside** the sign-in
  popover (`index.html:47-51`). If the user clicks away and the popover closes, the
  success/failure message is hidden. Errors (`main.ts:512`, `526`, `556`) and success
  (`510`, `552`) all land in this same transient, easily-dismissed spot. No persistent,
  shell-level feedback exists.

### F-4 — Error feedback uses `aria-live="polite"`, not `assertive` [MED]
- Rule: R-27 / R-32 (error states must be perceivable).
- Evidence: `#drive-status` is `aria-live="polite"` (`index.html:50`). A restore/backup
  **failure** is an error and should interrupt (`assertive`); a success is fine as polite.
  One live region cannot be both. Confirmed by source.

### F-5 — Passphrase dialog has no visible loading/busy state of its own [LOW]
- Rule: R-27.
- Evidence: after the user confirms a passphrase, `driveBackup`/`driveRestore` set
  `setDriveBusy(true)` and a status string (`main.ts:506-507`, `549`), but the passphrase
  dialog has already closed (`finish` -> `closeOverlay`, `main.ts:448-453`). The "encrypting
  / uploading" progress only appears in the popover status line (see F-3). No disabled/busy
  affordance bridges the gap between confirm and result.

### F-6 — Signed-in chip collapses name/caret; account identity is thin [LOW]
- Rule: R-20 (identity/hierarchy) + user request ("polish signed-in account state").
- Evidence: `.user-name, .chip-caret { display: none; }` (`styles.css:332`) — in the 56px
  collapsed rail the chip shows only a 24px initials avatar (`styles.css:325-331`). The
  signed-in identity (name, email) only exists inside the popover title/sub
  (`main.ts:346-360`). Acceptable for a 56px rail, but the popover header hierarchy is flat:
  title and sub are 12.5px/11.5px with little separation (`styles.css:349-350`).

### F-7 — `--faint` secondary text fails AA on raised surface [MED]
- Rule: R-25.
- Evidence (measured): `--faint #93836f` on `--raised #2a231c` = **4.22:1** (FAIL normal
  text). `--faint` is the placeholder/tertiary token (`styles.css:19`). Any new popover or
  toast secondary text must use `--dim #b9aa95` (6.82:1 on raised, PASS), never `--faint`.
  This is a constraint on the spec, flagged so implementers don't reach for `--faint`.

### F-8 — `--border` token is ~1.1:1 against its surfaces (non-text contrast) [MED]
- Rule: R-25 non-text (WCAG 1.4.11, 3:1 for component boundaries).
- Evidence (measured): `--border #332a21` vs `--raised #2a231c` = **1.10:1**; vs `--bg`
  = **1.29:1**. Existing component edges (popover, inputs, drive buttons) rely on `--border`
  and sit well under 3:1. This is a pre-existing Dusk property, not introduced here. The
  spec below does **not** silently "fix" it (that is a direction-wide decision = [NEEDS-MEKL]),
  but the toast and the focus ring MUST NOT depend on `--border` alone to be findable —
  they use `--accent` edges / shadow, which do pass.

---

## 2. Implementation-exact spec

All selectors, tokens, and geometry below are written so a frontend worker implements
without guessing. Owner of all three files is the frontend coder; this is a spec, not a diff.

### 2.1 — Fixed passphrase overlay + smooth, purposeful entry/exit

**Goal:** `#pass-dialog` becomes a true centered overlay like every other modal; the full
shell stays spatially visible behind a scrim (frozen via existing `inert`), and entry/exit
is a short, reduced-motion-safe fade+lift.

**CSS (`src/styles.css`):**
1. Add `#pass-dialog` to the fixed-overlay selector at line 1207:
   ```css
   #modal, #raw-modal, #key-modal, #dialog, #ssh-modal, #ansible-modal, #pass-dialog {
     position: fixed; inset: 0;
     background: rgba(0, 0, 0, .5);
     display: flex; align-items: center; justify-content: center;
     z-index: 50;
   }
   ```
   Reason: one rule already defines the scrim + centering contract; `#pass-dialog` must share
   it so the shell is dimmed-but-visible instead of shoved (R-31: reuse the single overlay
   contract, no new pattern).
2. Entry/exit transition (MOTION 1, within the existing 120ms budget). Because `.hidden`
   uses `display:none !important`, a fade needs a class that is applied/removed around the
   display flip. Add an `.ov-enter` modifier the renderer toggles on the next frame:
   ```css
   #pass-dialog { opacity: 0; transition: opacity 120ms ease; }
   #pass-dialog .modal-box { transform: translateY(6px); transition: transform 120ms ease; }
   #pass-dialog.ov-enter { opacity: 1; }
   #pass-dialog.ov-enter .modal-box { transform: none; }
   ```
   - Scrim fades 0 -> 1 over 120ms; box lifts 6px -> 0. No scale, no bounce, no loop.
   - Reason (R-19): the motion signals "a layer opened over the frozen shell"; 6px + 120ms
     is the smallest gesture that reads as purposeful without drawing attention.
3. Reduced motion: already covered globally by `styles.css:875-876`
   (`*,*::before,*::after { animation:none!important; transition:none!important }`). Under
   reduced motion the dialog appears instantly at full opacity — correct, no extra rule
   needed. **Verify** the `.ov-enter` class still ends at `opacity:1`/`transform:none` so the
   instant state is the final state (it does, above).

**TS (`src/main.ts`, inside `openOverlay`/`closeOverlay`):**
- On open: `el.classList.remove("hidden")`, then on the next animation frame add `ov-enter`
  so the transition runs (`requestAnimationFrame(() => el.classList.add("ov-enter"))`).
- On close: remove `ov-enter`; after the 120ms `transitionend` (or a 140ms fallback timer),
  add `.hidden` back. This keeps the exit visible. Under reduced motion `transitionend` may
  not fire, so the fallback timer is mandatory.
- This is generic enough to apply to all overlays; **[NEEDS-MEKL]** whether to apply the
  fade to every overlay or scope it to `#pass-dialog` only this pass. Recommended: scope to
  `#pass-dialog` now (surgical, matches task), generalize later. Reason: AGENTS.md §0.3
  surgical-changes.

**Geometry:** `.modal-box` is `width:380px; max-height:90vh` (`styles.css:1220-1221`),
`.dialog-box` narrows to 340px (`styles.css:1225`). Keep `#pass-dialog`'s box on
`.dialog-box` (already in markup, `index.html:437`). Centered by the flex rule. No change.

### 2.2 — Signed-in rail chip + popover hierarchy (56px rail / 230px popover)

Constraints are fixed: rail is 56px wide, popover is 230px (`styles.css:336`). Do not widen.

**Chip (collapsed, 56px rail):**
- Signed-out: keep current (gmark + hidden label, `styles.css:332`). Chip is 44px
  (`styles.css:316`), meets the 44px tap target (R-03).
- Signed-in: show the 24px initials avatar (`styles.css:325-331`) as today. Add a 2px
  `--live #9dc489` ring OR a small accent dot ONLY if it marks real state; here it marks
  "account connected" which is a real state -> allowed (R-31). **[NEEDS-MEKL]** decide
  dot vs no dot; recommend **no dot** — the avatar presence already signals signed-in, and
  a decorative-looking dot risks the antislop status-dot tell. Reason: avoid a dot that
  marks nothing new.
- `avatar` text is `--accent` on `--raised` = 7.10:1 (measured, PASS). Keep.

**Popover (230px, signed-in) hierarchy:**
- Header: account name as `.pop-title` (currently 12.5px/600, `styles.css:349`). Bump to
  13px/650 and keep `--text` (12.88:1 on raised, PASS) so the name is the focal point.
  Reason (R-20): one clear focal point per screen — the identity.
- Sub: email as `.pop-sub` 11.5px `--dim` (6.82:1, PASS — currently correct,
  `styles.css:350`, `main.ts:352`). Must stay `--dim`, never `--faint` (F-7).
- Below header: `#drive-actions` (Back up / Restore, `index.html:47-51`) then `Sign out`
  then `Settings`. Order is correct. Keep the 230px width; `word-break:break-word` on
  sub/status (`styles.css:350,377`) handles long emails.
- The popover arrow (`styles.css:342-348`) uses `--raised` + `--border` edges; `--border`
  is ~1.1:1 (F-8) so the arrow reads mostly by fill, not edge — acceptable for a decorative
  pointer, but do not rely on it as the only boundary. The popover body already carries
  `--shadow-drawer` (`styles.css:340`) which provides the real separation from the shell.

**States (chip/popover):**
- signed-out, signed-in, pending (`Signing in…`, `main.ts:366`), error
  (`pop-sub[data-tone="error"]` -> `--danger` 5.16:1 on raised, PASS, `styles.css:351`).
  All four already exist in `renderGoogleAccount`; no new state needed, only the hierarchy
  bump above.

### 2.3 — Accessible non-modal toast stack for backup/restore success + failure

**This is the new surface the user asked for.** It replaces the "feedback hidden inside a
popover" problem (F-3). The in-popover `#drive-status` can stay as inline progress while the
flow runs, but the terminal success/failure MUST also raise a shell-level toast so it
survives the popover closing.

**Placement + markup (`index.html`):** add ONE toast container as a sibling of `#app`,
same level as the overlays:
```html
<!-- Toast stack: non-modal backup/restore feedback. Cloned from template,
     never built from remote strings (ADR-8). -->
<div id="toast-stack" class="toast-stack" aria-live="polite" aria-relevant="additions"></div>
<template id="toast-template">
  <div class="toast" role="status">
    <span class="toast-icon" aria-hidden="true"></span>
    <div class="toast-body">
      <span class="toast-title"></span>
      <span class="toast-msg"></span>
    </div>
    <button class="toast-close" type="button" aria-label="Dismiss">
      <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor"
           stroke-width="1.5" stroke-linecap="round" aria-hidden="true"><path d="M3 3l10 10M13 3 3 13"/></svg>
    </button>
  </div>
</template>
```
- All dynamic text (title, msg) set via `textContent` on the cloned node (ADR-8, matches the
  existing chip/ansible clone pattern). Never `innerHTML`.

**ARIA live rules (resolves F-4):** success and error need different urgency, so use TWO
regions, not one:
- Container `#toast-stack` is `aria-live="polite"` for **success** toasts.
- Each **error** toast carries its own `role="alert"` (implicitly assertive) on the cloned
  `.toast` element — set `role="alert"` for errors, leave `role="status"` for success. Reason
  (R-27/R-32): a failed backup is an interrupting error; a success is a polite confirmation.
  Do NOT put `aria-live="assertive"` on the shared container or every success will interrupt.

**Severity variants:**
- success: `.toast--ok`, icon = check, left-accent = `--live #9dc489` (7.89:1 on raised).
- error: `.toast--error`, icon = alert, left-accent = `--danger #e07856` (5.16:1 on raised).
  Error text itself is `--text` (12.88:1), NOT danger-colored body copy — color is a signal,
  not the only signal (antislop-human: no color-only feedback). The icon + the word
  "failed" in the title carry the meaning too.

**Timing + persistence:**
- Success toast: auto-dismiss after **5000ms**. Reason: confirmation, low stakes.
- Error toast: **persistent** (no auto-dismiss). Reason (R-27/C-4): a failed backup/restore
  is actionable; the user must be able to read it and retry. Dismiss only via the close
  button or an explicit action.
- Hover/focus on any toast **pauses** its auto-dismiss timer (standard a11y: don't yank a
  message the user is reading). Resume on blur/mouseleave.
- Stack: newest on top (prepend), max **3** visible; if a 4th arrives, drop the oldest
  success (never auto-drop an error). Vertical gap 8px.

**Close / action behavior:**
- Every toast has a close button (focusable, 44px hit area via padding even if the glyph is
  14px — see geometry). Enter/Space activates (native `<button>`).
- Error toasts MAY carry one inline action ("Try again") that re-invokes the failed op.
  **[NEEDS-MEKL]** whether to add the retry action now; recommend **close-only** this pass to
  stay surgical, with a `// TODO` + no visible dead control. Reason: a retry button must wire
  to real behavior or not exist (R-26) — don't ship a dead "Try again".

**Motion (MOTION 1):**
- Enter: slide-in 8px from the right + fade, 140ms ease. Exit: fade 120ms.
- No looping pulse, no bounce. Under reduced motion the global kill switch
  (`styles.css:875-876`) makes toasts appear/disappear instantly — correct. Ensure the
  instant end-state is `opacity:1; transform:none`.

**TS flow (`src/main.ts`):** route `driveBackup`/`driveRestore` terminal outcomes to a new
`showToast({tone, title, msg})` helper:
- backup success (`main.ts:510`) -> `showToast({tone:"ok", title:"Backup complete",
  msg: "Backed up <name> (<bytes> bytes)."})`.
- backup failure (`main.ts:512`) -> `showToast({tone:"error", title:"Backup failed",
  msg: <error>})` (persistent).
- restore success (`main.ts:551-554`) -> ok toast with restored count + pre-restore dir.
- restore failures (`main.ts:526` no backups, `556` exception) -> error toasts.
- Keep `#drive-status` as live **progress** text ("Encrypting and uploading…") so the user
  sees activity while the popover is still open; the toast is the durable terminal result.

### 2.4 — Loading / disabled / empty / error / success states (full matrix)

| Surface | Loading | Disabled | Empty | Error | Success |
|---|---|---|---|---|---|
| Sign-in button | `Signing in…` + disabled (`main.ts:364-366`) — keep | n/a | n/a | popover sub `--danger` (`main.ts:355`) — keep | chip -> signed-in, popover shows name/email |
| Backup | `#drive-status` "Encrypting and uploading…" + both drive btns disabled (`main.ts:506-507,492-493`) — keep | buttons `opacity .6` when `drivePending` (`styles.css:376`) — keep | n/a | **error toast (persistent)** + `#drive-status` cleared | **ok toast** |
| Restore | "Finding the latest backup…" then "Downloading and decrypting…" (`main.ts:521,549`) — keep | same as backup | **"No backups found on Drive"** -> currently an error status (`main.ts:526`); re-classify as a neutral **empty toast** (`role=status`, not alert), text "No backups on Drive yet. Back up first to enable restore." | error toast (persistent) | ok toast |
| Passphrase dialog | between confirm and result the dialog is already closed (F-5); the popover `#drive-status` + toast carry progress | OK button disabled while empty? currently not — keep OK enabled, validate on click (`main.ts:456-465`) | n/a | inline `#pass-error` for empty/mismatch (`index.html:448`, `main.ts:457-463`) — keep, `role="alert"` already set | resolves + closes |

- Empty-state reclassification (restore with zero backups) is the one behavior change in the
  state matrix. Reason (antislop-human): an empty state must name the cause and the next
  action, and "No backups found" styled as an error over-alarms. **[NEEDS-MEKL]** confirm the
  empty-state copy wording.

### 2.5 — Keyboard order, focus restoration, Escape, narrow window, 200% zoom

**Passphrase overlay (uses the existing overlay stack, `main.ts:605-687`):**
- Focus on open: `#pass-input` (`main.ts:480` passes it as `focusTarget`) — correct.
- Tab order: pass-input -> (confirm input if backup) -> Cancel -> OK. Matches DOM order
  (`index.html:441-451`). `cycleFocusWithin` (`main.ts:622-636`) traps within the box —
  correct.
- Escape: `onOverlayEscape` routes `#pass-dialog` -> `passDialogResolve()` = cancel
  (`main.ts:641`) — correct, keep.
- Focus restore: `closeOverlay` restores `closed.restore?.focus()` (`main.ts:686`) to the
  element focused before open (the drive button) — correct.
- Enter in either input submits (`main.ts:469-470`) — correct.

**Toast stack keyboard rules (new):**
- Toasts are **non-modal**: they do NOT trap focus and do NOT enter the overlay stack
  (`overlayStack`), so they never steal Tab from the shell. Reason: a toast that trapped
  focus would block the terminal — wrong for a passive notification.
- The close button is reachable by Tab in normal shell order (it is after `#app` in the DOM;
  confirm it comes late so it does not interrupt primary tab flow — append `#toast-stack`
  AFTER all overlays in `index.html` so live toasts are last in tab order).
- Escape does NOT dismiss toasts (Escape is owned by the overlay stack). Dismiss is the
  close button or the timer. **[NEEDS-MEKL]** if MEKL wants Escape-to-dismiss-newest-toast,
  that needs a guarded global handler that yields to the overlay stack when an overlay is
  open; recommend not adding it this pass (keeps Escape semantics single-owner).

**Narrow window geometry:**
- Popover is `left: calc(100% + 8px)` of the 56px rail (`styles.css:335`), width 230px -> it
  needs ~294px of horizontal room. On a very narrow window the popover could clip the right
  edge. **[NEEDS-MEKL] / QA:** verify at the app's minimum window width; if it clips, the
  popover should flip or clamp. Not changing placement blind — flag for QA measurement.
- Toast stack: fix to `bottom: 16px; right: 16px; max-width: min(360px, calc(100vw - 32px))`
  so it never overflows horizontally (R-03). Toasts stack upward.

**200% zoom:**
- Passphrase box is `width:380px` fixed with `max-height:90vh; overflow-y:auto`
  (`styles.css:1220-1221`) — at 200% zoom the box stays 380 CSS px (zoom scales it), content
  scrolls inside, no clipping. Acceptable. **QA to confirm** the box does not exceed viewport
  width at 200% on a narrow window; if it does, change `.dialog-box` to
  `width: min(340px, calc(100vw - 24px))`. Flag, do not change blind.
- Toast `max-width: min(360px, calc(100vw - 32px))` already reflows at zoom.

---

## 3. Exact CSS / token constraints + measured contrast

All ratios measured with `python3 ~/.hermes/skills/creative/antislop-human/contrast-check.py`
against the Dusk `:root` tokens (`styles.css:8-26`). Toast/popover surface is `--raised
#2a231c`.

| Pairing (new or load-bearing) | Ratio | Normal AA (4.5) | Use |
|---|---|---|---|
| `--text #f2e9dc` on `--raised #2a231c` | 12.88:1 | PASS | toast title, error body, popover name |
| `--dim #b9aa95` on `--raised #2a231c` | 6.82:1 | PASS | toast msg, popover email/sub |
| `--faint #93836f` on `--raised #2a231c` | **4.22:1** | **FAIL** | **banned for toast/popover text (F-7)** |
| `--danger #e07856` on `--raised #2a231c` | 5.16:1 | PASS | error icon/accent (not body text) |
| `--live #9dc489` on `--raised #2a231c` | 7.89:1 | PASS | success icon/accent |
| `--accent #e2a35c` on `--raised #2a231c` | 7.10:1 | PASS | focus ring, avatar initials |
| `--accent-ink #241404` on `--accent #e2a35c` | 8.18:1 | PASS | text on accent button |
| `--text #f2e9dc` on `--bg #191512` | 15.09:1 | PASS | overlay-over-shell baseline |
| `--border #332a21` vs `--raised #2a231c` | **1.10:1** | **FAIL (non-text 3:1)** | do NOT make it the sole boundary of toast/focus (F-8) |

**Token constraints:**
- Toast/popover secondary text: `--dim` only, never `--faint` (F-7).
- Toast surface: `--raised`; separation from shell via `--shadow-drawer` (the existing
  single elevation token), NOT a new shadow. Reason (R-12): one elevation marker, Dusk uses
  exactly one.
- Toast left-accent bar: `--live` (success) / `--danger` (error) — carries severity, so it
  is a signal not decoration (allowed vs the antislop "colored left stripe" tell). Reason
  (R-31): the stripe marks real state.
- Focus ring on close button / dialog controls: use the existing accent focus style; do NOT
  set `outline:none` without replacement (R-32). `--accent` on `--raised` = 7.10:1 (PASS as
  a 2px ring; non-text 3:1 met with margin).
- Radius: reuse `--r-lg/--r-md/--r-sm` (`styles.css:28`); toast = `--r-md 8px` to match
  popover family. No new radius token (R-11).
- No new color, no gradient, no glow, no blur anywhere (keeps Dusk flat; R-01/R-10/R-13).

---

## 4. Acceptance checklist (frontend + QA)

Frontend (implementation):
- [ ] `#pass-dialog` added to the fixed-overlay selector (`styles.css:1207`); dialog renders
      centered over a dimmed, spatially-intact shell (F-1).
- [ ] Passphrase overlay enter = scrim fade 120ms + box lift 6px->0; exit fades out before
      `.hidden` reapplies; reduced-motion shows instant final state (F-2).
- [ ] Toast stack sibling of `#app`, appended AFTER all overlays; cloned from
      `#toast-template` via `textContent` only (no innerHTML, ADR-8).
- [ ] Success toast auto-dismiss 5000ms, `role="status"`; error toast persistent,
      `role="alert"`; hover/focus pauses the timer (F-3, F-4).
- [ ] Backup/restore terminal outcomes (`main.ts:510,512,551,556`) raise toasts; restore
      "no backups" is a neutral empty toast, not an error (§2.4).
- [ ] `#drive-status` kept as inline progress during the flow.
- [ ] All new text uses `--text`/`--dim`; `--faint` NOT used on `--raised` (F-7).
- [ ] Toast/focus boundaries do not rely on `--border` alone (shadow + accent carry it, F-8).
- [ ] Popover name bumped to focal point (13px/650 `--text`); email stays `--dim` (§2.2).
- [ ] No new color/gradient/glow/blur; radius from existing tokens; one elevation token.
- [ ] Toasts are non-modal: no focus trap, not in `overlayStack`, close button in tab order.

QA / verify loop (AGENTS.md §4):
- [ ] `cd src-tauri && cargo test --lib` green at STATE.md baseline.
- [ ] `npm run build` 0 TS errors.
- [ ] Smoke (`qa-reports/headless-smoke.mjs`) extended: assert `#pass-dialog` computed
      `position:fixed`; assert toast container + template exist; assert error toast carries
      `role="alert"`, success `role="status"`; CSP-violation counter == 0.
- [ ] Measured contrast re-confirmed for any pairing the implementer actually ships.
- [ ] Keyboard: open passphrase (focus input), Tab cycle traps, Escape cancels + restores
      focus to the drive button; toast close reachable by Tab, Enter/Space dismisses.
- [ ] Narrow window: popover does not clip at min width; toast stack does not overflow (R-03).
- [ ] 200% zoom: passphrase box + toasts do not clip or force horizontal scroll (R-35).
- [ ] Reduced motion: no transitions run; overlays/toasts appear/disappear instantly.
- [ ] INV-21 anti-Abyss source scan still clean (no new banned literals / `var(--glow)`).

---

## 5. Taste choices — RESOLVED by nano (2026-11-06 19:34 board comment)

Antislop usage-mode (DURING vs AFTER) went unanswered by MEKL, so the six items below were
flagged for decision. Nano reviewed this report end-to-end, accepted it as the implementation
gate, and — per MEKL's "continue" instruction — applied the recommended DURING defaults. All
six resolutions match this report's own recommendations; they are now **settled directives**
for the frontend and QA workers, not open questions.

1. **RESOLVED — no connected dot.** The 24px initials avatar already signals signed-in; a
   decorative-looking dot risks the antislop status-dot tell. Chip stays avatar-only.
2. **RESOLVED — animate `#pass-dialog` only.** Scope the fade entry/exit to the passphrase
   overlay this pass (surgical, AGENTS.md §0.3); do not generalize to all overlays now.
3. **RESOLVED — close-only error toasts.** No inline "Try again" action this pass; ship no
   dead control (R-26). A `// TODO` may mark the future retry hook, but nothing visible.
4. **RESOLVED — Escape stays overlay-only.** Escape remains owned by the overlay stack; it
   does NOT dismiss toasts. Toasts dismiss via close button or timer only.
5. **RESOLVED — neutral no-backups copy.** Restore with zero backups is a neutral empty toast
   (`role="status"`, not `alert`): "No backups on Drive yet. Back up first to enable restore."
6. **RESOLVED — QA measures/clamps narrow geometry.** Popover flip/clamp is deferred to a QA
   min-width measurement; frontend does not change placement blind. QA measures at the app's
   minimum window width and clamps only if it clips (§2.5).

Screenshot note (unchanged): the live rendered app and the user screenshot remain
**unverified** — the screenshot endpoint returned provider 401 and was never seen. Every
geometric claim in this report is from CSS/DOM source, not a rendered pixel; runtime layout,
focus-ring rendering, and WebView2 behavior are QA's to confirm.

No production source was modified. This review file is the only artifact. Not committed,
not pushed.
