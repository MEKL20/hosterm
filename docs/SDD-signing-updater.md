# hosterm — SDD v1 Extension: §2.4 Delivery & packaging — signing, updater, key custody (wave-2 M2/M3)

| Field | Value |
|---|---|
| Author | crew-coding-architect |
| Date | 2026-10-19 |
| Status | Draft — prerequisite for M2 implementation tasks (TSD §4.2/§7.2); input to wave-2 TSD update. **Rev. 2 (2026-10-20, MEKL): Authenticode vendor = SignPath Foundation** (was Azure Trusted Signing → now paid fallback) — see ADR-9 Rev. 2 |
| Extends | `docs/archive/SDD-hosterm-v1.md` (§2.4, §5 R-9, ADR-5) |
| Requirements source | `docs/archive/FEATURE-SPEC-wave-2-draft.md` §4 (FR-W2-5..10, AC-B1..B6, NFR-W2-6..9, HR-W2-1); PRD §6.2 Q4 (resolved), OQ-B1; TSD §4.2 milestones, §4.4 channel table, §7.2 |
| Version constraint | No commits by architect; docs only. Lead folds this into SDD v1 at the deferred docs commit (§E.10). |

---

## E.0 Why a separate extension file, not an in-place §2.4 edit

Decision: this content ships as `docs/SDD-signing-updater.md`, merged into `SDD-hosterm-v1.md` §2.4 by the lead at the deferred docs commit (wave-2 draft §10).

Reason: SDD v1 is frozen pending MEKL review and rides the single deferred docs commit (TSD §1 inputs, draft §10); editing it in place would mutate a review-pending document and blur its scope ("architecture of the shipped product") with wave-2 scope ("architecture we commit to building next"). A separate file gives M2 implementation tasks a self-contained, citable contract now. Sections here are prefixed `E.` to avoid colliding with SDD v1 numbering; ADRs continue v1's sequence (9–12).

---

## E.1 Context

hosterm v1 ships unsigned installers from a tag-triggered GitHub Actions workflow (`build-windows.yml`: `audit` → `build` (windows) → `release` (publish)). Wave-2 Section B (PRD §6.2 Q4, resolved) adds two stages, in fixed order (**HR-W2-1**):

- **Stage A (M2):** every released artifact is Authenticode-signed and carries Tauri updater artifacts (`.sig` + `latest.json`). No runtime behavior changes; the app still makes zero network calls (PRD NFR-3 unchanged until M3).
- **Stage B (M3):** the in-app updater is enabled (user-initiated check only), verifying downloads against a minisign public key baked into the app. HR-W2-1: this ships only after a fully signed release exists.

The threat being closed (SDD R-9): an unsigned auto-update channel converts the release pipeline into a silent RCE distribution network — any pipeline compromise (runner, token, action dependency) would push executable code to every user with no user-side trust anchor. Signing first makes the **keys**, not the **host**, the trust anchor: feed compromise stops being code execution.

Two facts reframe the feature spec's assumptions (recorded here because they change decisions):

1. **EV certificates no longer buy SmartScreen shortcut.** Microsoft removed EV's special reputation treatment from its Trusted Root Program in 2024; EV and OV now build reputation identically. FR-W2-5(a)'s "EV preferred" is therefore amended by this SDD to: **OV (cloud/HSM-hosted) is sufficient**; EV's cost premium is unjustified. SmartScreen reputation accrues per certificate across releases.
2. **Tauri's updater cannot be told to skip signature verification.** The plugin enforces minisign verification structurally — there is no config flag to disable it. HR-W2-1's "the updater can never distribute code the key did not sign" is thus a platform guarantee, not something we must re-assert in code.

Version floors (TSD §2 discipline): `tauri-plugin-updater` 2.x (plugins-workspace v2 line; current docs state Rust ≥ 1.90 — CI stable toolchain satisfies this; lockfile is the binding record). Locked shell stays Tauri 2.12.1.

---

## E.2 Architecture

### E.2.1 Release pipeline (Stage A / M2)

Signing is integrated **inside the existing build job** — no separate sign job, no second runner. Under the SignPath vendor (ADR-9 Rev. 2) the signing call is a service round-trip through the official SignPath action rather than a local CLI hook, but it stays a linear step sequence in the same job:

```
 push v* tag
     │
     ▼
┌─ audit (ubuntu, unchanged) ────────────────┐
│  cargo-audit + npm audit prod gates        │
└───────────────┬────────────────────────────┘
                │ needs [audit]
                ▼
┌─ build (windows-latest) ───────────────────────────────────────────┐
│  1. npm install (unchanged)                                        │
│  2. npm run tauri build → UNSIGNED .exe + .msi                     │
│     • TAURI_SIGNING_PRIVATE_KEY env present → bundler emits        │
│       provisional *.exe.sig + *.msi.sig (createUpdaterArtifacts)   │
│     • key absent → bundler FAILS here (fail-closed, free)          │
│  3. Authenticode via SignPath Foundation (service round-trip):     │
│     • upload-artifact (unsigned) → signpath/github-action-         │
│       submit-signing-request (org/project/policy slugs +           │
│       SIGNPATH_API_TOKEN; wait-for-completion — this wait          │
│       INCLUDES the one manual SignPath approval) → download        │
│       signed files over the unsigned ones                          │
│     • submission denied / approval not granted → job FAILS here    │
│  4. REGENERATE *.exe.sig + *.msi.sig over the SIGNED binaries      │
│     (tauri signer sign) — .sig must cover signed bytes             │
│  5. GATE (bash, fail-closed):                                      │
│     • signtool verify /pa on .exe + .msi                           │
│     • every bundle has a sibling .sig; tauri signer verify passes  │
│  6. upload-artifact: *.msi, *.exe, *.sig   (globs extended)        │
└───────────────┬────────────────────────────────────────────────────┘
                │ needs [build, audit]
                ▼
┌─ release (ubuntu, unchanged topology) ─────────────────────────────┐
│  download-artifact → assemble latest.json (in-repo python script,  │
│  stdlib only) → softprops/action-gh-release                        │
│    files: **/*.msi  **/*.exe  **/*.sig  **/latest.json             │
│    fail_on_unmatched_files: true (a missing .sig or latest.json    │
│    fails the release — no unsigned/asset-less publish)             │
└────────────────────────────────────────────────────────────────────┘
                │
                ▼
   GitHub Releases page = the ONLY distribution feed
   (installer + .sig per installer + latest.json)
```

Ordering note (M2, Rev. 2): Authenticode signing rewrites the binary (cert table embedded in the PE), so the bundler's provisional `.sig` — generated before SignPath returns — is replaced in step 4 by a signature over the **signed** bytes. The step-5 `tauri signer verify` gate is what proves the ordering: a stale `.sig` over unsigned bytes fails it.

HR-W2-1 structural enforcement, clause by clause:

| HR-W2-1 clause | Mechanism | Test |
|---|---|---|
| No unsigned artifact reaches a release | `createUpdaterArtifacts: true` makes the bundler require `TAURI_SIGNING_PRIVATE_KEY` — missing secret = red build, before any artifact exists; a denied or unapproved SignPath submission fails the job pre-gates; the post-build `signtool verify /pa` + `tauri signer verify` gates run before upload; `fail_on_unmatched_files: true` covers the feed side | AC-B3.1 (dry-run without secrets fails pre-publish), AC-B3.2 (workflow audit: verify steps precede upload; `release` needs `build`) |
| Updater enabled only after signed release live | M2→M3 ordering. Honest note: this is a **one-shot ordering**, not a recurring invariant — a CI oracle ("check the previous release was signed") is over-engineering for a solo maintainer and is rejected. The *permanent* structural guarantees are the two rows above plus the plugin's non-disableable verification. M3's own PR carries the AC-B3.3 assert (updater config present only from this commit) | AC-B3.3, §E.9 |

### E.2.2 Update flow (Stage B / M3)

```
 user clicks "Check for updates"        ← the ONLY trigger (FR-W2-8; no polling,
     │                                     no background timer, D3)
     ▼
 tauri-plugin-updater (Rust side, NOT the WebView → no CSP surface change)
     │  GET https://github.com/<owner>/<repo>/releases/latest/download/latest.json
     │  (one endpoint; TLS enforced; single request, ≤ 5 s bound — NFR-W2-9)
     ▼
 version compare: strict forward-only, SemVer      ── downlevel → refuse (AC-B5.2iii)
     ▼ update available → user consents (dialog)   ── declined → nothing happens
     ▼
 download installer → minisign verify against baked pubkey (non-disableable)
     │                                              ── invalid/truncated → atomic refusal,
     ▼                                                 current app keeps running, zero
 NSIS silent install (installMode: "passive")          filesystem side effects (AC-B6.1)
     │                                                 (FR-W2-10)
     ▼
 plugin exits the app (Windows installer limitation) → installer runs → relaunch on new version
```

Notes that close spec ambiguities:

- **Updater traffic bypasses the WebView entirely** (native reqwest inside the plugin). `app.security.csp` is untouched — `connect-src` stays locked to IPC origins. NFR-W2-6's network surface is exactly one user-initiated HTTPS GET + one download, both to `github.com`.
- **`releases/latest` semantics:** GitHub serves the latest **non-draft, non-prerelease** release. Policy falls out for free: prerelease tags are never pushed to updater users by accident. Do not mark updater-feed releases as prerelease/draft.
- **Windows install exits the app** (plugin limitation, documented). Safe here: config writes are atomic per operation (SDD §3.1), the secrets sidecar is read at use time (§3.2) — nothing is cached at consent time that a hard exit can lose.
- **Version floors / feed hygiene:** `latest.json` always points to a release whose artifacts carry `.sig` files (true from M2 onward, so the M3 flip has signed predecessors). Rollback stays fix-forward (TSD §4.5): the updater refuses downgrades by design; a "rollback" is a new forward version.

---

## E.3 Components and responsibilities

| Component | Location | Responsibility | Talks to |
|---|---|---|---|
| Authenticode signing step | `build-windows.yml` build job (M2+): upload unsigned → `signpath/github-action-submit-signing-request` → download signed | Submits `.exe` + `.msi` to SignPath.io (origin-verified: this repo's workflows, GitHub-hosted agents); waits out the manual approval; downloads signed files. Key material never touches the runner | SignPath.io service (SignPath Foundation cert) |
| Updater-artifact generation | `bundle.createUpdaterArtifacts: true` (M2+) | Bundler emits `.sig` siblings (minisign, key from `TAURI_SIGNING_PRIVATE_KEY` env — env only, `.env` files are ignored by the CLI) | GH secret `TAURI_SIGNING_PRIVATE_KEY`(+`_PASSWORD`) |
| Signature gates | `build-windows.yml` build job, post-build bash steps | `signtool verify /pa` both installers; `tauri signer verify` each `.sig` against the pubkey; fail = no upload | build outputs |
| `latest.json` assembler | New in-repo script (python3 stdlib), run in `release` job | Reads `.sig` contents + tag → writes latest.json (schema §E.4.2); no third-party action (keeps R2's action surface unchanged — tauri-action rejected, §ADR-10) | artifact files, `github.ref_name` |
| Release publish | `softprops/action-gh-release` (existing, SHA-pinned) | Attaches installers + `.sig` + `latest.json`; `fail_on_unmatched_files: true` extended to the new patterns | GitHub Releases |
| Updater runtime | `tauri-plugin-updater` (M3+) | User-initiated `check()` → version gate → consent → download → verify (non-disableable) → install → relaunch (`tauri-plugin-process`) | GitHub Releases endpoint, baked pubkey |
| Update UI | `src/main.ts` (M3, small) | "Check for updates" action; progress; explicit refusal error message (FR-W2-10 wording); routes any markup through `esc()`/`escAttr()` (SDD ADR-8) | updater JS API |
| Key ceremony record | `docs/KEY-CEREMONY.md` (new, created at M2) | Who/where/when of key generation; cert profile id + fingerprint; minisign **public** key; expiry; rotation history (AC-B1.1, NFR-W2-7) | this doc |

---

## E.4 Configuration contracts

### E.4.1 `tauri.conf.json` — stage-gated diff

**M2 (Stage A)** — add under `bundle`:

```json
{
  "bundle": {
    "createUpdaterArtifacts": true
  }
}
```

`plugins.updater` stays **absent** (AC-B3.3 asserts absence between M2 and M3).

Authenticode signing is **not** a `tauri.conf.json` key under the SignPath vendor (ADR-9 Rev. 2): it is build-job steps in `build-windows.yml` (§E.2.1: upload unsigned → `signpath/github-action-submit-signing-request` with `organization-id` / `project-slug` / `signing-policy-slug` / `api-token: ${{ secrets.SIGNPATH_API_TOKEN }}` → download signed → `.sig` regeneration). Nothing vendor-specific enters the app config; `tauri.conf.json` stays vendor-neutral, which keeps the Rev. 1 fallback (Azure, `signCommand`) a workflow-only change.

**M3 (Stage B)** — add under `plugins`:

```json
{
  "plugins": {
    "updater": {
      "pubkey": "<minisign public key string — inline, never a file path>",
      "endpoints": [
        "https://github.com/<owner>/<repo>/releases/latest/download/latest.json"
      ],
      "windows": {
        "installMode": "passive"
      }
    }
  }
}
```

- Exactly **one** endpoint (NFR-W2-6: no third-party update servers; the plugin's multi-endpoint fallback is unused).
- `installMode: "passive"` is the plugin default but is written explicitly — "quiet" would make installs invisible (violates consent posture); "basicUi" adds clicks for zero benefit.
- From the M3 commit, `plugins.updater.pubkey` joins TSD §2's frozen-strings list: any change = security review + key-ceremony cross-check (AC-B5.3), same treatment as the CSP string.

### E.4.2 `latest.json` — the feed contract

Static JSON (plugin's documented schema), assembled in the release job:

```json
{
  "version": "0.11.0",
  "notes": "one-line release note",
  "pub_date": "2026-11-01T12:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<full content of hosterm-setup.exe.sig>",
      "url": "https://github.com/<owner>/<repo>/releases/download/v0.11.0/hosterm-setup.exe"
    }
  }
}
```

- `signature` = verbatim content of the generated `.sig` (a string, not a path/URL — plugin requirement).
- `version` SemVer with or without leading `v`; the workflow writes the tag verbatim (literal preservation — no normalization).
- Keys we emit: `windows-x86_64` only (single shipped platform; adding linux/darwin keys is how a bad assembler would point users at 404s).
- `pub_date` RFC 3339 UTC.
- Assembler = ~30 lines of python3 stdlib in the release job; unit-testable as a fixture (happy path + missing-`.sig` failure asserted in the workflow).

### E.4.3 GitHub secrets inventory (complete)

| Secret / variable | Holds | Used in | Custody |
|---|---|---|---|
| `SIGNPATH_API_TOKEN` (GH **secret**) | SignPath API token (user with **submitter** permission on the project/signing policy) | build job (signing submission) | GH secret; revocable in the SignPath dashboard |
| `SIGNPATH_ORGANIZATION_ID` (GH **variable**, not secret) | SignPath org id (public identifier) | build job (submission) | plain Actions variable |
| `TAURI_SIGNING_PRIVATE_KEY` | minisign **private** key (password-encrypted) | build job (bundler env) | GH secret + offline encrypted backup (§E.5) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | key password | build job | GH secret + offline backup |

Project slug + signing-policy slug live in the workflow as plain config (not secrets). Deliberately **absent**: any Authenticode key material (the SignPath Foundation cert's private key is generated on and never leaves SignPath's HSM — nothing to store), any Azure/Entra credential (Rev. 1 fallback only). Accepted debt, tracked in STATE: `SIGNPATH_API_TOKEN` is **long-lived** (unlike Rev. 1's per-run OIDC token). Compensations: submitter-only scope (submits requests, exports nothing — the key never leaves the HSM), SignPath origin verification (submissions accepted only from GitHub workflow runs of **this repo** on GitHub-hosted agents, artifact uploaded before submission), and the per-release manual approval — a leaked token alone cannot produce a signed release silently.

---

## E.5 Key custody and threat model

### E.5.1 Assets and where they live

| Key | Created | Private half | Public half |
|---|---|---|---|
| Authenticode cert key | SignPath Foundation onboarding (repo + OSI-license verification; cert issued **atas nama SignPath Foundation**, not MEKL personally) | SignPath HSM, **non-exportable** — never exists outside SignPath | cert chain in every signed binary |
| Minisign keypair | M2 key ceremony: `tauri signer generate` run **offline** (maintainer machine, not CI) | password-encrypted GH secret + encrypted offline backup | key-ceremony doc (M2), baked into `tauri.conf.json` at M3 |

Key ceremony records (AC-B1.1): date, machine class, operator, key ids/fingerprints, expiry, backup location class. Grep gates: no private material in repo (`BEGIN PRIVATE KEY`, minisign secret-key prefix) in the pre-stage verify loop (AC-B1.2); first signed run's CI log eyeballed for masked-secret leakage (AC-B1.3).

### E.5.2 Threat model

| # | Asset / path | Threat | Control | Residual (accepted) |
|---|---|---|---|---|
| T1 | Authenticode signing authority | Signing-account abuse → attacker ships malware under the SignPath Foundation cert | Key non-exportable on SignPath HSM; a signing request needs ALL of: `SIGNPATH_API_TOKEN` (submitter scope), origin verification PASS (workflow run of this repo, GitHub-hosted agents, artifact uploaded pre-submission), and the manual release approval — three independent gates; fork PRs get no secrets by GitHub default | Token + approval-holder compromise = signing authority (single-maintainer: the approver is us). Recovery: revoke token, reject pending requests, incident note on the release. Solo-maintainer ceiling unchanged: no 4-eyes; every release publicly diffable |
| T2 | minisign private key | Exfiltration via malicious workflow change or log leak | Secrets unavailable to fork PRs; workflow edits are the review surface (SHA-pinned actions, R2); secret values auto-masked in logs; key password-encrypted so the secret alone is inert; offline backup enables rotation (NFR-W2-7) | Repo/secret-store compromise → malicious updates until rotation. Detection: users' baked pubkey mismatches rotated key → update refusals surface immediately |
| T3 | Release feed (Releases assets, `latest.json`) | Attacker swaps an asset or the feed | Requires `contents: write` — restricted to the release job (R4), tag-protected refs; even on success, the updater **verifies the minisign signature client-side against the baked key** — feed compromise yields refused installs, not RCE | Effectively none for code execution — this is the trust-anchor point of the whole design (HR-W2-1 rationale) |
| T4 | Build runner | Compromised `windows-latest` runner signs + publishes malware with *valid* signatures | Ephemeral runners; SHA-pinned actions (R2 — including the new SignPath submit action, pinned like every action); audit gates (R3); the only new third-party action is `signpath/github-action-submit-signing-request`, which can only submit artifacts for signature (it cannot publish — the release job and its gates stay ours) and is origin-verified; secret blast radius limited (T1/T2); the API token cannot export key material (HSM-held) | A sufficiently compromised runner can sign malware once — identical residual to any GitHub-hosted CI build; bounded by the same review-the-release discipline as any signed-from-CI product |
| T5 | Baked minisign pubkey | Silent swap in a merged PR → updater trusts attacker's key | Pubkey lands in exactly one auditable commit (M3 flip); frozen-strings treatment (§E.4.1); AC-B5.3 asserts the config value matches the ceremony record | Social engineering on the maintainer — out of technical scope, noted honestly |
| T6 | Version skew | Stale app on CVE'd version; or downgrade attack to old vulnerable version | Stage B gives a user-initiated check (the cure for staleness is *usable*, per US-W2-6); plugin enforces strict forward-only compare (AC-B5.2iii); downgrades refuse by design (TSD §4.5) | User who never clicks "Check for updates" stays stale — accepted (no polling is a product constraint, PRD NFR-3/D3) |

### E.5.3 Rotation runbook (NFR-W2-7; detailed doc lands with M2)

- **minisign, planned:** generate new pair offline → cut release `vN+1` **signed with the old key** whose build carries the **new** baked pubkey → wait one adoption cycle → retire old key. (Users' running apps verify `vN+1` against the old key — still baked in them — and adopt the new key for everything after.)
- **minisign, emergency (suspected T2 compromise):** same procedure, accelerated, plus a release note ("update manually from the Releases page this one time").
- **Authenticode:** certificate lifecycle (issuance/renewal/reissue) is owned by **SignPath Foundation**, not by us — there is no profile of ours to rotate. A Foundation-side reissue (expiry/compromise) just changes the chain in future signed releases; no app change. SmartScreen reputation is per-cert, so a reissue restarts accrual — outside our control, accepted (the minisign channel, which guards code execution, is unaffected).
- **pubkey-change rule:** a release that changes the baked pubkey must be signed by the *previous* key (the one users already trust). Never both in one flip.

---

## E.6 ADRs

### ADR-9 — Authenticode vendor = SignPath Foundation (Rev. 2, 2026-10-20); OV is sufficient

- **Decision (Rev. 2 — supersedes Rev. 1; MEKL, 20 Okt 2026, final).** Sign installers through the **SignPath Foundation** free OSS code-signing program: OV certificate issued **atas nama SignPath Foundation** (not MEKL personally), private key generated on and held by SignPath's HSM (non-exportable), signing executed by the SignPath.io service. Mechanism: the build job uploads unsigned artifacts and submits via the official `signpath/github-action-submit-signing-request` action (`SIGNPATH_API_TOKEN` + organization-id + project-slug + signing-policy-slug; origin-verified against this repo's GitHub workflows on GitHub-hosted agents — §E.2.1/§E.4.3). Each release requires **one manual approval** in the SignPath dashboard — policy gate, added to the release checklist (§E.8.4/.10).
- **Why the change.** Cost: **$0** (Foundation program: free for maintained OSI-licensed OSS — hosterm is MIT, public repo) vs **≈ $120/tahun** for Azure Trusted/Artifact Signing Basic, plus Azure's one-off identity validation. Identity: the Foundation vouches for the repo↔binary link, which is exactly the trust claim an OSS installer needs. Trade-offs accepted by MEKL: (1) publisher name in binary properties = "SignPath Foundation" — cosmetic; (2) one manual approval per release — deliberate friction that doubles as a release checkpoint; (3) SmartScreen reputation accrues per-cert, OV-class, gradually — parity with Azure Basic (no EV shortcut exists at all, §E.1 fact 1).
- **Rev. 1 (superseded, kept for the record).** Azure Trusted Signing (renamed "Azure Artifact Signing" in current docs), OIDC-federated Entra workload identity, invoked via Tauri's `bundle.windows.signCommand` hook. Demoted to **paid fallback**: if the Foundation program ever becomes unavailable (criteria change, program ends), revert to Rev. 1 — the architecture keeps that swap localized to workflow steps + one config key (§E.4.1).
- **Alternatives rejected (Rev. 2).** (a) Self-managed OV/EV USB-token certificate: incompatible with GitHub-hosted runners; single point of loss. (b) P12/PFX as a GH secret: key material on every runner — the exfiltration surface FR-W2-5(a) exists to avoid. (c) SSL.com eSigner & per-signature cloud-HSM vendors: valid but paid, for properties SignPath gives free. (d) Self-signed/unsigned: no SmartScreen path, no trust anchor. (e) Keep Azure despite cost: ≈$120/tahun buys no property the Foundation cert lacks for this product.
- **Consequence.** Signing stays tag-push-automatable modulo one human approval; custody reduces to "protect one SignPath org + token". The integration mechanism moves from a local CLI hook to an in-job service round-trip, which **amends ADR-10's "no artifact round-trip" clause** (below) and adds the `.sig`-regeneration step (§E.2.1 ordering note). Releases get slower by minutes and gain a human-dependent step; fail-closed is preserved (denied/unapproved submission = red job before any upload). Feature-spec amendment from Rev. 1 is retained: FR-W2-5(a) "EV preferred" → "OV cloud/HSM sufficient" — now satisfied by the Foundation cert.

### ADR-10 — Fail-closed signing inside the existing build job; `latest.json` assembled in-repo; no tauri-action

- **Decision.** (1) Signing happens inside the existing `build` job: the bundler invokes `signCommand` per artifact, and post-build bash gates (`signtool verify /pa`, `tauri signer verify` per `.sig`) run **before** `upload-artifact`. `release` keeps `needs: [build, audit]`, so a failed gate blocks publish structurally. (2) `latest.json` is assembled by a ~30-line in-repo python script in the `release` job from the downloaded `.sig` files + tag name. *(Rev. 1 wording — "bundler invokes `signCommand`" is superseded by the Rev. 2 note below.)*
- **Alternatives rejected.** (a) Separate `sign` job (download → sign → re-upload): extra artifact round-trip and a second Windows runner for zero added enforcement — the gates in `build` are just as structurally blocking (AC-B3.2's "publish needs sign" is satisfied as publish needs build-which-verifies). (b) `tauri-apps/tauri-action` for the whole release (it generates `latest.json` natively): a large third-party action surface that **rebuilds the app** inside the action — re-runs the entire toolchain outside our audited steps and would bypass the gates we just wrote; R2's posture is minimal, SHA-pinned action surface. The static JSON is 15 documented keys; a stdlib script with a fixture test is cheaper than the new trust problem.
- **Consequence.** Fail-closed is partly free (with `createUpdaterArtifacts: true`, a missing minisign secret fails the bundler before artifacts exist — AC-B3.1's evidence path); the rest is two assert steps. Diff to `build-windows.yml` stays small and reviewable: globs extended (`*.sig`, `latest.json`), two gate steps, one assembler step. Cost: the assembler is ours to maintain (mitigated by fixture test in-workflow).
- **Rev. 2 note (2026-10-20, SignPath vendor).** "No artifact round-trip to a separate sign job" is amended: the round-trip to SignPath (upload unsigned → submit → download signed) happens **inside the build job**, so "no separate sign job / no second runner" stands; the step sequence gains the `.sig` regeneration (§E.2.1). Rejected alternative (a) — the separate sign job — stays rejected. New rejection: driving the round-trip through `bundle.windows.signCommand` + a wrapper CLI — it would serialize a service wait (incl. the manual approval) inside the bundler and require custom wrapper code; the official action is vendor-supported and origin-verified.

### ADR-11 — Updater = official `tauri-plugin-updater`, single GitHub Releases feed, user-initiated only

- **Decision.** Stage B uses the official plugin (Rust-side HTTP + verify + install), configured exactly as §E.4.1 M3: one endpoint (`releases/latest/download/latest.json`), baked inline pubkey, `installMode: "passive"`, strict forward-only compare (plugin behavior), user-initiated check only. Frontend adds one action + result/refusal rendering.
- **Alternatives rejected.** (a) Custom updater (own check/download/verify code): re-implements signature verification and download hygiene the plugin already got right; verification bugs in hand-rolled updaters are a classic RCE class — violates the boring-and-proven rule. (b) Third-party update server / dynamic endpoint (CrabNebula etc.): violates NFR-W2-6 ("no third-party update servers") and adds a hosted dependency for a solo-maintainer product whose release cadence is measured in weeks. (c) Background polling or silent install: violates FR-W2-8/D3 and PRD's zero-background posture — non-negotiable product constraint, not an engineering preference.
- **Consequence.** Zero CSP impact (traffic never enters the WebView — `connect-src` untouched); zero background work; trust chain = baked pubkey → plugin-enforced verification. Costs/limits accepted: app exits on install (Windows plugin limitation — safe here, §E.2.2); no delta updates (full installer per update — fine at ~10 MB-class binaries); `releases/latest` serves non-prerelease only (feature, not bug).

### ADR-12 — Key custody: two keys, two lifetimes, offline-backup rotation

- **Decision.** Exactly two private keys exist: Authenticode (lives and dies inside SignPath's HSM; non-exportable; its "custody" is SignPath org hygiene + the release approval, ADR-9 Rev. 2) and minisign (generated offline at the M2 ceremony, password-encrypted, stored as a GH secret + encrypted offline backup; rotation per §E.5.3). The minisign **public** key is public material: recorded in the key-ceremony doc at M2, baked into `tauri.conf.json` only at the M3 flip (AC-B3.3 keeps the interim clean).
- **Alternatives rejected.** (a) Generating the minisign key in CI (`tauri signer generate` on a runner): the private key would transit a runner's memory and possibly disk caches; the ceremony is a one-time 5-minute offline step — no reason to accept the exposure. (b) Hardware token for minisign: every CI build would need the token — reintroduces ADR-9's rejected hosted-runner incompatibility for a key whose blast radius is already bounded by rotation. (c) Storing the pubkey nowhere until M3: loses the AC-B1.1 record and makes the M3 flip unverifiable against a ceremony — the record is the anchor.
- **Consequence.** Secret surface in GitHub = three secrets (§E.4.3), none of them Authenticode material. Rotation is rehearsed once (tabletop, NFR-W2-7) using §E.5.3 as the script. Cost: the key ceremony is a human step with a written record — deliberately so; it is the one ritual in the pipeline.

---

## E.7 Risks (amendments to SDD v1 §5)

| # | Risk | Impact | Mitigation / status |
|---|---|---|---|
| R-11 | SignPath org / API-token compromise → signed malware (T1) | High | Non-exportable HSM key, submitter-scoped token, origin verification (this repo, GitHub-hosted agents only), manual approval per release, fork-PR isolation, publicly diffable releases; recovery = token revocation + pending-request rejection + rotation runbook |
| R-12 | minisign secret leak → malicious updates until rotation (T2) | High | Encrypted secret + offline backup, fork isolation, log masking; baked pubkey makes leaks *visible* (refusals); tabletop-drilled rotation (NFR-W2-7) |
| R-13 | `latest.json` assembler bug points users at wrong/missing asset (T3-adjacent) | Medium | Plugin refuses on failed fetch/verify (no install, no side effects — FR-W2-10); fixture test on assembler; `fail_on_unmatched_files: true`; `windows-x86_64` key only |

---

## E.8 Verification steps (map to feature-spec ACs)

1. **Fail-closed proof (AC-B3.1):** workflow run with signing secrets absent → build fails at the bundler (missing `TAURI_SIGNING_PRIVATE_KEY`) before any artifact upload; release page unchanged.
2. **Workflow audit (AC-B3.2):** review shows gate steps precede `upload-artifact`; `release` job `needs: [build, audit]` unchanged; `fail_on_unmatched_files: true` covers `.sig` + `latest.json`.
3. **Updater-absence assert (AC-B3.3):** smoke/docs check — `plugins.updater` key absent from `tauri.conf.json` until the M3 flip commit; present with expected pubkey/endpoints from M3 on.
4. **Signed-release checklist (AC-B2.1/.2):** first M2 tag → release page carries `.exe`, `.msi`, both `.sig`, `latest.json`; `signtool verify /pa` passes in-workflow (runner) and once on a clean Windows VM (manual matrix). From M2 on, the release checklist gains a standing step: **grant the SignPath approval** in the dashboard when the submission appears — no approval, no signed artifacts, job stays red (fail-closed).
5. **Tamper fixture (AC-B2.3, AC-B5.2, AC-B6.x where headless-able):** CI step flips one byte in an artifact → `tauri signer verify` fails; fixtures for wrong-key signature, truncated artifact, downlevel `latest.json` version → all refuse; refusal path asserts zero filesystem side effects (smoke pre/post snapshot; extends per TSD §3.2).
6. **Network-surface audit (AC-B4.1/.2, NFR-W2-6):** smoke network log — zero outbound requests at idle and during normal use; the update check alone issues exactly one HTTPS request to the Releases endpoint (post-M3).
7. **Key-material grep (AC-B1.2/.3):** pre-stage grep (private-key markers) stays clean; first signed CI log inspected for secret masking.
8. **Timing (NFR-W2-9):** one-off measured pass at M3 (OQ-B2 posture): check→result ≤ 5 s, non-blocking UI.
9. **Key ceremony + runbook (AC-B1.1, NFR-W2-7):** `docs/KEY-CEREMONY.md` exists with all §E.5.1 fields; one tabletop rotation drill recorded.
10. **Vendor smoke (first M2 run, then spot-check):** SignPath submission shows origin-verification PASS (this repo, GitHub-hosted agents); the approval appears in the dashboard; publisher name in the signed binary's properties reads "SignPath Foundation" (documented trade-off — sanity-check once, not per release).

---

## E.9 Traceability

| Requirement | Where |
|---|---|
| HR-W2-1 (sign first; updater only after signed release live) | §E.1, §E.2.1 enforcement table, ADR-10 |
| FR-W2-5 (key material exists + protected) | §E.4.3, §E.5, ADR-9, ADR-12; EV amendment noted in ADR-9 |
| FR-W2-6 (CI signs every shipped artifact) | §E.2.1, §E.4.1 (M2 diff), ADR-10 |
| FR-W2-7 (unsigned-release gate) | §E.2.1 enforcement table, §E.8.1/.2, ADR-10 |
| FR-W2-8 (user-initiated check only) | §E.2.2, ADR-11; NFR-3 amendment dependency: OQ-B1 PRD edit must land with/before M3 (TSD §4.2) |
| FR-W2-9 (signature gates install; strict forward) | §E.2.2 (plugin non-disableable verify), §E.4.1 (pubkey bake), ADR-11 |
| FR-W2-10 (atomic refusal) | §E.2.2, §E.7 R-13, §E.8.5 |
| AC-B1.1/.2/.3 | §E.5.1, §E.8.7 |
| AC-B2.1/.2/.3 | §E.8.4/.5 |
| AC-B3.1/.2/.3 | §E.2.1, §E.8.1/.2/.3 |
| AC-B4.x, AC-B5.x, AC-B6.x | §E.2.2, §E.8.5/.6/.8 |
| NFR-W2-6 (minimal network surface) | §E.2.2, ADR-11 (single endpoint, no third-party servers) |
| NFR-W2-7 (rotation runbook) | §E.5.3, ADR-12 |
| NFR-W2-8 (verify loop unchanged) | §E.8 (all additions are workflow/docs/smoke-side; `cargo`/`npm` baselines untouched by this extension) |
| NFR-W2-9 (bounded, non-blocking check) | §E.8.8 |
| TSD §4.4 channel table | mirrored: Now→M2 manual-only / M3+ user-initiated signed feed (§E.2.2) |
| SDD v1 §2.4 / §5 R-9 / ADR-5 | this document is the §2.4 replacement content; R-9 gains R-11..R-13; ADR-5's "GitHub Releases distribution" gains the signing stage |

---

## E.10 Lead merge instruction (deferred docs commit)

When folding into `SDD-hosterm-v1.md`:

1. Replace §2.4 body with §E.2 (renumber locally; keep the existing Vite/CSP delivery paragraph as the intro — it remains true).
2. Append ADR-9..12 to the v1 ADR section (numbering continues).
3. Append R-11..R-13 to the risk table.
4. Add `plugins.updater.pubkey` to TSD §2's frozen-strings list at the M3 flip.
5. Mirror the vendor change into `docs/SDD.md` in the same docs commit: §2.2 stage-A paragraph, §3 components row, §7 ADR-9/ADR-10/ADR-12, §9 R-11 (v2 mirrors this extension verbatim today).
6. This file then becomes historical; the SDD is the single source.

---

*Sources: docs/archive/FEATURE-SPEC-wave-2-draft.md §4/§7/§8, docs/archive/PRD-hosterm-v1.md §6.2 (Q4, OQ-B1), docs/TSD.md §2/§4.2/§4.4/§7.2, docs/archive/SDD-hosterm-v1.md, .github/workflows/build-windows.yml, src-tauri/tauri.conf.json (all read 2026-10-19); Tauri v2 docs — plugin/updater + distribute/sign/windows (fetched 2026-10-19: createUpdaterArtifacts + env-key behavior, signCommand hook, latest.json schema, installMode, non-disableable verification, EV/OV SmartScreen parity 2024). Rev. 2 sources: signpath.org (Foundation program: free OSS signing, cert issued to the Foundation, HSM-held keys) + docs.signpath.io/trusted-build-systems/github (submit-signing-request action inputs SIGNPATH_API_TOKEN/organization-id/project-slug/signing-policy-slug; origin-verification checks: workflow-of-this-repo, artifact uploaded pre-submission, GitHub-hosted agents for OSS) (fetched 2026-10-20); Azure Trusted/Artifact Signing ≈ $120/tahun Basic (MEKL decision record, 20 Okt).*
