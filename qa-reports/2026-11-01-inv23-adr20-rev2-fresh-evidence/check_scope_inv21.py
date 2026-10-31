#!/usr/bin/env python3
"""Fresh scope and INV-21 regression evidence for INV-23 QA."""
from __future__ import annotations

import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TARGET = "f56e2ed04a2d744267fbd025afda999a34b46b78"
BASE = "ca3e86b25ed64a226fda262ada9c00e2e6f1d5f8"


def git(*args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=ROOT, text=True, check=True, capture_output=True
    ).stdout


head = git("rev-parse", "HEAD").strip()
commit_file_lines = [
    line for line in git("diff-tree", "--no-commit-id", "--name-status", "-r", TARGET).splitlines()
    if line
]
committed_backend_numstat = git("diff", "--numstat", f"{BASE}..{TARGET}", "--", "src-tauri")
working_backend_numstat = git("diff", "--numstat", "--", "src-tauri")

source_files = [ROOT / "src/main.ts", ROOT / "src/styles.css", ROOT / "index.html"]
forbidden = [
    "#08080a",
    "#050507",
    "#00d992",
    "#818cf8",
    "#0e2a20",
    "var(--glow)",
]
terminal_editor_forbidden = [
    "#020203",
    "#2fd6a1",
    "#f2f2f2",
    "#80d280",
    "#ffdd80",
    "#fd9c9f",
]
hits = []
for path in source_files:
    text = path.read_text(encoding="utf-8")
    for literal in forbidden + terminal_editor_forbidden:
        if literal.lower() in text.lower():
            hits.append({"path": str(path.relative_to(ROOT)), "literal": literal})

checks = {
    "head_is_target": head == TARGET,
    "commit_files_frontend_and_smoke_only": commit_file_lines
    == ["M\tqa-reports/headless-smoke.mjs", "M\tsrc/main.ts"],
    "committed_backend_diff_zero": committed_backend_numstat.strip() == "",
    "working_backend_diff_zero": working_backend_numstat.strip() == "",
    "inv21_forbidden_hits_zero": len(hits) == 0,
}
result = {
    "target_sha": head,
    "baseline_sha": BASE,
    "commit_files": commit_file_lines,
    "committed_backend_numstat": committed_backend_numstat.splitlines(),
    "working_backend_numstat": working_backend_numstat.splitlines(),
    "inv21": {
        "files_scanned": [str(path.relative_to(ROOT)) for path in source_files],
        "forbidden_literals": forbidden,
        "terminal_editor_forbidden_literals": terminal_editor_forbidden,
        "hits": hits,
    },
    "checks": checks,
    "pass": all(checks.values()),
}
out = Path(__file__).with_name("scope-inv21.json")
out.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
print(json.dumps({"output": str(out), "pass": result["pass"], "checks": checks}))
raise SystemExit(0 if result["pass"] else 1)
