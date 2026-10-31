# hosterm UI prototypes

Three standalone, one-screen mockups of the redesigned shell. Same layout in all three
(sidebar with host list, tab bar with split drop-zone, split terminal, status bar,
Terminal/Snippets/AI panel toggle). Open directly from disk, no build step, no network
assets. All colors verified WCAG AA with the antislop-human contrast checker.

| File | Direction | Palette (bg / surface / text / accent) | Shown open |
|---|---|---|---|
| `proto-a-dusk.html` | Warm paper-dark minimalist: brown-black surfaces, generous spacing, soft 10px radii, one honey-amber accent for focus + primary action | `#191512` / `#221c17` / `#f2e9dc` / `#e2a35c` | AI assistant with the command approval card |
| `proto-b-slate.html` | Cool glass-free precision: desaturated blue-gray, visible 1px edges, tight 30px rhythm, crisp cyan used only as a signal | `#0d1319` / `#151e28` / `#dce6f0` / `#56c8ea` | Snippets cheatsheet drawer |
| `proto-c-ink.html` | High-contrast near-monochrome: pure grayscale, typography-led hierarchy, one vermillion accent reserved for primary actions only (Run once, Reconnect) | `#0a0a0b` / `#131315` / `#f5f5f3` / `#ff4f32` | Clean terminal + Google account states |

Features across the set:

- Google login: signed-in chip (avatar, name, G mark) in every sidebar footer; the
  signed-out "Sign in with Google" popover is open in proto-c, present in all files.
- AI approval UX (in every file, open in proto-a): proposed command card with
  `Run once` / `Always allow` / `Reject`, plus a past always-allowed command with output.
- Snippets panel (in every file, open in proto-b): Docker / Git / systemd groups,
  filter input, per-item copy buttons.
- States: skeleton host row (loading), `backup-vault auth failed` error row, dead
  `staging-api` tab with Reconnect pill, unsaved-dot editor tab, live + idle host dots.

Full contrast ratios per file are documented in the HTML comment at the top of each file.
