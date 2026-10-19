# PRD — hosterm Security Hardening, Wave 1

| Field | Value |
|---|---|
| Author | crew-coding-pm |
| Status | Draft — for MEKL review |
| Scope | 3 confirmed findings + 1 adjacent (CI permissions) |
| Out of scope | Implementation — design/architecture decisions belong to lead/architect |
| Version constraint | No git commits by PM; document only |

---

## 1. Problem statement

hosterm is a desktop SSH/SFTP client (Tauri v2, WebView2, Windows-first) that handles **user SSH credentials and private keys** (secrets sidecar `~/.ssh/hosterm.secrets`, 0600). Three confirmed gaps, verified in the repo today:

1. **No Content-Security-Policy.** `src-tauri/tauri.conf.json` line 24: `"csp": null`. The WebView runs with no CSP at all. The frontend renders remote-server-controlled data into the DOM — SFTP listings (filenames come from the connected server, which may be hostile), remote file contents in the editor, terminal output — via **21 `innerHTML` call sites** guarded by only **13 `esc()`/`escAttr()` call sites**. One missed escape = DOM XSS, and a successful script injection sits inside a Tauri webview where the IPC bridge can reach backend commands and the secrets sidecar. Today there is no second line of defense.
2. **Unpinned GitHub Actions.** `.github/workflows/build-windows.yml` references six actions by mutable refs: `actions/checkout@v4`, `actions/setup-node@v4`, `dtolnay/rust-toolchain@stable`, `swatinem/rust-cache@v2`, `actions/upload-artifact@v4`, and — not in the original finding list — **`softprops/action-gh-release@v2`, a third-party action**. The workflow grants `permissions: contents: write` at workflow level. A hijacked action tag = arbitrary code in CI holding a write token that **publishes installers to the public releases page**.
3. **No dependency vulnerability scanning.** No `cargo-audit`, no `npm audit` anywhere in CI. Known-CVE dependencies ship silently into installers users run with elevated trust.

Why now: the app is publicly released (v0.8.1) and positions itself as a credentials handler. All three gaps are cheap to close relative to the exposure they remove.

---

## 2. Goals / Non-goals

### Goals
- G1: Shipped WebView enforces a CSP that blocks injected script execution.
- G2: Every CI third-party action is immutable-pinned; the supply chain to the release artifacts is reproducible.
- G3: CI fails on known high/critical dependency CVEs, with a visible waiver process for exceptions.
- G4: Zero functional regressions — the app must keep working (this wave must NOT break the terminal, SFTP browser, or editor).

### Non-goals (this wave)
- Any change to `src-tauri/src/lib.rs`, auth flows, key management, or the secrets sidecar format.
- Migration off real `ssh`/`sftp` binaries (founding invariant, untouched).
- SBOM generation, artifact signing, auto-update infrastructure (candidate: wave 2).
- Full dependency upgrade campaign — audits gate new known-CVEs; upgrading is separate work.
- Rewriting `innerHTML` call sites (CSP is the compensating control for wave 1; DOM-hardening refactor is a separate, later proposal).

---

## 3. Personas

| Persona | Stake |
|---|---|
| **End user (SSH operator)** | Stores server credentials in hosterm. Trusts that a hostile remote server (filenames, MOTDs, file contents) cannot execute code inside the app and reach key material. |
| **MEKL (solo maintainer)** | Wants hardening that doesn't add CI toil or break releases; has limited Windows test time. |
| **Adversary A — malicious server** | Controls SFTP listings and remote file bytes rendered in the webview DOM. Goal: XSS → Tauri IPC → secrets. |
| **Adversary B — supply chain** | Compromises an action tag or a dependency version. Goal: run code in CI with `contents: write`, publish a backdoored installer to releases. |

---

## 4. Requirements

### R1 — CSP must be present and enforced (finding 1)

**WHAT:** The production WebView must run with a non-null, enforced Content-Security-Policy; the dev server (`localhost:1420`) must also be covered. The policy must deny-by-default everything not required to run the app.

**Hard constraint:** The policy must not break the three rendering surfaces: xterm.js terminal, SFTP browser, CodeMirror remote editor. This is a functional-compatibility requirement with equal weight to the security requirement.

**Known compatibility facts (evidence gathered):**
- All assets are bundled locally by Vite; no external `http(s)` origins found in `src/` or `index.html`.
- Fonts are system stacks (`var(--font-sans)` / `var(--font-mono)`); no `@font-face`, no `@import`, no font CDN → font loading imposes no constraint.
- xterm CSS is bundle-imported (`src/main.ts` line 8) — no runtime style injection from xterm.
- **CodeMirror 6 injects `<style>` elements at runtime** via `style-mod` (verified: `node_modules/style-mod/dist/style-mod.cjs` calls `doc.createElement("style")`). A `style-src` without an accommodation for this **will break the editor's rendering**. This is the #1 breakage risk of the whole wave.
- Zero inline `style=` attribute usage in `src/main.ts`.
- Playwright smoke tests run in Chromium, not WebView2 — CSP behavior is near-identical (both Chromium) but the injection mechanism (Tauri build-time CSP injection) is only exercised in real builds.

**Acceptance criteria:**
1. `"csp"` in `src-tauri/tauri.conf.json` is a concrete policy string, not `null`.
2. Verify loop fully green after the change: `cargo test --lib` → 48 pass; `npm run build` → no TS errors; `qa-reports/headless-smoke.mjs` → 43+ pass (extended per criterion 4).
3. Manual compatibility matrix on a real Windows build passes with **zero CSP violation entries in devtools console**: (a) open SSH session, run colored output (`ls --color`), resize terminal; (b) browse SFTP pane incl. a directory with unicode/special-char filenames; (c) open remote file in editor with syntax highlighting, edit, save.
4. Smoke suite gains an automated check that collects console CSP-violation events during the full run and asserts count == 0 (so the matrix stays covered in CI).
5. Demonstration of enforcement: an injected inline `<script>` in the built app is blocked by CSP (one-time manual proof during the spike, documented in the PR description).
6. Every directive relaxation relative to deny-by-default (e.g. any `'unsafe-inline'`) carries an inline comment or PR note naming the library that requires it (expected: CodeMirror/`style-mod` for styles). **`script-src` must not be relaxed beyond `'self'` + Tauri's injected IPC origins.**
7. A spike by lead/architect precedes implementation and records: exact Tauri v2 CSP injection behavior in dev vs prod (incl. whether Tauri auto-appends its IPC origins to `connect-src`), and the minimal style accommodation CodeMirror needs.

### R2 — CI actions pinned to immutable refs (finding 2)

**WHAT:** Every `uses:` reference in every file under `.github/workflows/` must point to a full-length commit SHA (immutable), not a mutable tag/branch. The referenced version is preserved as a comment for humans. Includes the six found refs — explicitly including third-party `softprops/action-gh-release@v2`, which is the highest-risk pin (it receives the `contents: write` token and publishes release artifacts).

**Acceptance criteria:**
1. `grep -nE "uses:\s*\S+@(v[0-9]+(\.[0-9]+)*|stable|master|main)" .github/workflows/*.yml` returns zero matches.
2. Every SHA is 40 hex chars and corresponds to the currently-used release tag of that action (tag recorded as a trailing comment on the same line).
3. A manual `workflow_dispatch` run of the workflow is green after pinning.
4. A real tag build (`v*`) still produces and uploads the `.msi`/`.exe` artifacts and attaches them to the release (end-to-end, verified on the next release cut).

### R3 — Dependency vulnerability gates in CI (finding 3)

**WHAT:** CI must run dependency vulnerability audits on every PR and every push to the default branch, and on release tag builds: (a) Rust dependency audit, (b) frontend dependency audit. Build must fail on high/critical known CVEs in production dependencies. Findings that cannot be fixed (no patched version) require a documented waiver in-repo: reason + review date.

**Acceptance criteria:**
1. Both audit steps exist in CI and execute on PR, push, and tag builds.
2. Negative test (verified once, then removed): temporarily declaring a dependency version with a known high/critical CVE makes CI red; reverting makes it green. Proves the gate bites.
3. Current default branch is green, or a waiver file lists each accepted finding with justification and a review date ≤ 90 days out.
4. Audit step output is visible in the job log (findings readable without rerunning locally).

### R4 — (adjacent, recommended to fold into wave 1) CI token scope minimized

**WHY include:** same file, minutes of effort, same review — removes the blast radius that makes R2 urgent. `permissions: contents: write` is currently granted workflow-wide.

**WHAT:** Workflows run with read-only token by default; write permission is granted only where artifact release attachment requires it, scoped to the narrowest unit (job/step).

**Acceptance criteria:**
1. Top-level `permissions:` in the workflow is read-only.
2. Non-tag builds (manual dispatch) complete successfully with read-only tokens.
3. Tag build still attaches artifacts to the release.

---

## 5. Priorities

| Req | Security value | Effort | Breakage risk | Ship order |
|---|---|---|---|---|
| R2 pin actions | High (supply chain) | ~30 min | None | **1** |
| R4 token scope | Medium | < 30 min | Low (tag build must be re-verified) | **1** (same PR as R2) |
| R3 audit gates | High | Low–medium | CI churn (finding triage) | **2** |
| R1 CSP | **Highest** (XSS → secrets containment) | Medium–high (spike + matrix) | **Highest** (CodeMirror styles) | **3** — spike starts immediately in parallel |

Rationale: R2/R4/R3 are cheap, zero-UI-risk wins shipped immediately. R1 carries the most security value and the only real regression risk, so its **spike** starts at once but it ships last, gated by the compatibility matrix.

---

## 6. Success metrics

- 100% of CI action refs immutable-pinned (AC R2.1 grep = 0).
- Both audit gates green on default branch; negative-test proven (AC R3.2).
- CSP enforced in shipped builds; 0 CSP violations across smoke suite + manual Windows matrix (AC R1.3–1.4).
- No regression: 48 cargo tests, 43+ smoke checks, all three rendering surfaces verified on Windows.
- Zero new CI minutes pain: release build latency increase ≤ 5 min from audit steps (constraint for architect's design).

---

## 7. Rollout (PM-level phasing)

1. **Immediately:** R2+R4 PR (one file), verified by manual dispatch + next tag build.
2. **Same week:** R3 audit steps land; CVE triage session with MEKL for whatever surfaces.
3. **Parallel:** R1 spike (architect) → CSP PR → manual Windows matrix → ship with next regular release. Wave 1 is DONE when R1 ships.

---

## 8. OPEN QUESTIONS (for MEKL — each with PM recommendation)

| # | Question | Recommendation |
|---|---|---|
| Q1 | CodeMirror requires a `style-src` accommodation for runtime `<style>` injection. How strict: hash/nonce-per-injection vs pragmatic `'unsafe-inline'` for **styles only**? | Pragmatic first: strict everywhere, `'unsafe-inline'` limited to `style-src` if spike proves hashes impractical for `style-mod`'s generated CSS. `script-src` stays strict regardless. Architect decides post-spike. |
| Q2 | Does Tauri v2 auto-append its IPC origins (`ipc:` / `http://ipc.localhost`) to `connect-src` when `csp` is set, or must the config carry them? | Architect verifies against Tauri v2 docs in the spike; spike AC includes this explicitly (R1.7). |
| Q3 | Audit failure threshold: block on all CVEs, or high+critical only? Dev-dependency vulns blocking too? | Block on high+critical in production deps; dev-only vulns advisory (log, don't fail) — avoids CI rot from transitive dev chains. |
| Q4 | Waiver process weight: ignore-entry in the audit config with reason+date, or separate `SECURITY_WAIVERS.md`? | Ignore-entry with inline comment + review date; one file fewer, waivers live next to the config that needs them. |
| Q5 | Auto-update pinned SHAs: enable Dependabot (github-actions ecosystem) or manual quarterly review? | Dependabot, PRs reviewed and merged by MEKL — pinning without an update path rots into "pinned 3 years ago". |
| Q6 | Fold R4 (token scope) into wave 1 or defer? | Fold in: same file, trivial, and it shrinks the exact blast radius R2 addresses. |
| Q7 | Playwright smoke runs in Chromium, not WebView2 — is the automated CSP check (R1.4) sufficient, or is a one-time manual WebView2 pass also required before tagging? | Keep manual one-time pass on real Windows before the wave-1 release tag (R1.3), then rely on the automated check for regression. WebView2 is Chromium-based, so drift risk after the first verified pass is low. |

---

## 9. Future waves (parking lot, not commitments)

- DOM-hardening refactor: replace `innerHTML` composition with DOM APIs / tagged-template sanitization (21 sites).
- Installer signing + auto-update trust chain.
- SBOM per release.
