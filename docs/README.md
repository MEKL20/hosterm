# hosterm — Documentation Index

Start here. Read top-to-bottom depending on what you need.

## Product & architecture (active)

| Doc | What it is |
|---|---|
| [PRD.md](PRD.md) | Product requirements: vision, users, use cases, epic roadmap (living doc) |
| [SDD.md](SDD.md) | Software design document: architecture, ADRs, invariants, frozen IPC contract |
| [SDD-signing-updater.md](SDD-signing-updater.md) | Design extension: Windows code signing (SignPath) + verified updater, key custody |
| [TSD.md](TSD.md) | Technical spec / delivery playbook: milestones, execution plan, gates |
| [STATE.md](STATE.md) | Living status for agents: architecture map, verify loop, risks & waivers — read before touching code |

## Feature & epic specs (per initiative)

| Doc | What it is |
|---|---|
| [FEATURE-SPEC-fleet-run-v1.md](FEATURE-SPEC-fleet-run-v1.md) | Fleet Run v1: run one command across many hosts, live output + summary |
| [FEATURE-SPEC-security-hardening-wave-1.md](FEATURE-SPEC-security-hardening-wave-1.md) | Security hardening wave 1: CSP, WebView2, IPC hardening (PRD) |
| [FEATURE-SPEC-startup-ssh-check.md](FEATURE-SPEC-startup-ssh-check.md) | Startup check for missing `ssh`/`sftp` + OpenSSH install guide (draft) |
| [EPIC-TSD-fleet-run-v1.md](EPIC-TSD-fleet-run-v1.md) | Fleet Run implementation playbook: units, gates, scheduling |
| [EPIC-TSD-redesign-arus-v1.md](EPIC-TSD-redesign-arus-v1.md) | UX redesign "Arus" + Abyss Emerald theme: implementation playbook |
| [EPIC-TSD-security-hardening-wave-1.md](EPIC-TSD-security-hardening-wave-1.md) | Security hardening wave 1: implementation playbook (shipped) |

## Research / spikes

| Doc | What it is |
|---|---|
| [SPIKE-csp-findings.md](SPIKE-csp-findings.md) | Findings: CSP behavior on Tauri v2 + CodeMirror style accommodation |

## UX design

- [UX/redesign-hosterm-v1.md](UX/redesign-hosterm-v1.md) — redesign direction, spec, fix list
- [UX/mockups/](UX/mockups/) — HTML mockups + theme variants

## Archive

Superseded documents, kept for history only — do not use as current truth:

- [archive/PRD-hosterm-v1.md](archive/PRD-hosterm-v1.md) — original PRD, superseded by PRD.md
- [archive/SDD-hosterm-v1.md](archive/SDD-hosterm-v1.md) — original SDD, superseded by SDD.md
- [archive/FEATURE-SPEC-wave-2-draft.md](archive/FEATURE-SPEC-wave-2-draft.md) — wave-2 draft, split into final specs
