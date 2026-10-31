#!/usr/bin/env python3
"""Independent WCAG oracle for rendered INV-23 evidence.

Consumes only DOM-observed colors from rendered-settings.json. It neither
imports nor executes src/main.ts or the canonical smoke helper.
"""
from __future__ import annotations

import argparse
import json
import math
from pathlib import Path

CURATED = {"#e2a35c", "#5fbfae", "#e0869b", "#aec267", "#b99be3"}
SPEC_RATIOS = {
    "#e2a35c": 8.18060039728731,
    "#5fbfae": 8.115110618605044,
    "#e0869b": 6.842802495508517,
    "#aec267": 9.102683799795606,
    "#b99be3": 7.529220380096464,
    "#000000": 21.0,
    "#101010": 19.028110547666497,
    "#6f6f6f": 5.024865064757633,
    "#757575": 4.607518093747377,
    "#766cb5": 4.58257700240974,
    "#cf0dcc": 4.5825758149421025,
    "#767676": 4.6232848849972035,
    "#777777": 4.68949989000882,
    "#ffffff": 21.0,
}


def luminance(hex_color: str) -> float:
    channels = []
    for offset in (1, 3, 5):
        srgb = int(hex_color[offset : offset + 2], 16) / 255.0
        channels.append(
            srgb / 12.92
            if srgb <= 0.04045
            else ((srgb + 0.055) / 1.055) ** 2.4
        )
    return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2]


def contrast(first: str, second: str) -> float:
    high, low = sorted((luminance(first), luminance(second)), reverse=True)
    return (high + 0.05) / (low + 0.05)


def oracle_ink(accent: str) -> str:
    if accent in CURATED:
        return "#241404"
    black_ratio = contrast(accent, "#000000")
    white_ratio = contrast(accent, "#ffffff")
    return "#000000" if black_ratio >= white_ratio else "#ffffff"


def state_result(state: dict, expected_accent: str, expected_ink: str) -> dict:
    actual_accent = state["computed_accent"]
    actual_ink = state["computed_ink"]
    ratio = contrast(actual_accent, actual_ink)
    expected_ratio = SPEC_RATIOS.get(expected_accent)
    checks = {
        "accent_exact": actual_accent == expected_accent,
        "input_exact": state["input_value"] == expected_accent,
        "inline_matches_computed": (
            state["inline_accent"] == actual_accent
            and state["inline_ink"] == actual_ink
        ),
        "preview_matches": (
            state["preview_accent"] == actual_accent
            and state["preview_ink"] == actual_ink
        ),
        "spec_endpoint": actual_ink == expected_ink,
        "oracle_endpoint": actual_ink == oracle_ink(actual_accent),
        "ratio_at_least_4_5": ratio >= 4.5,
        "spec_ratio_exact_1e_12": (
            expected_ratio is None or abs(ratio - expected_ratio) < 1e-12
        ),
    }
    return {
        "accent": actual_accent,
        "actual_ink": actual_ink,
        "oracle_ink": oracle_ink(actual_accent),
        "luminance": luminance(actual_accent),
        "ratio": ratio,
        "spec_ratio": expected_ratio,
        "checks": checks,
        "pass": all(checks.values()),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("input", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()

    rendered = json.loads(args.input.read_text(encoding="utf-8"))
    vector_results = []
    for run in rendered["vector_runs"]:
        result = state_result(run["actual"], run["accent"], run["expectedInk"])
        vector_results.append(
            {"class": run["class"], "entered": run["accent"], **result}
        )

    repeat_results = []
    for run in rendered["former_failure_repeats"]:
        result = state_result(run["actual"], run["accent"], run["expectedInk"])
        repeat_results.append({"pass_number": run["pass"], **result})

    curated = rendered["determinism"]["curated"]
    curated_states = {
        "free_input": curated["input"],
        "chip": curated["chip"],
        "free_input_before_persist": curated["input_before_persist"],
        "persist_reload": curated["boot"],
    }
    curated_results = {
        path: state_result(state, "#5fbfae", "#241404")
        for path, state in curated_states.items()
    }
    curated_persist_exact = curated["persisted"]["accent"] == "#5fbfae"

    non_curated = rendered["determinism"]["non_curated"]
    non_curated_states = {
        "free_input": non_curated["input"],
        "persist_reload": non_curated["boot"],
    }
    non_curated_results = {
        path: state_result(state, "#777777", "#000000")
        for path, state in non_curated_states.items()
    }
    non_curated_persist_exact = non_curated["persisted"]["accent"] == "#777777"

    all_result_flags = [entry["pass"] for entry in vector_results]
    all_result_flags.extend(entry["pass"] for entry in repeat_results)
    all_result_flags.extend(entry["pass"] for entry in curated_results.values())
    all_result_flags.extend(entry["pass"] for entry in non_curated_results.values())
    all_result_flags.extend((curated_persist_exact, non_curated_persist_exact))

    output = {
        "target_sha": rendered["target_sha"],
        "oracle": {
            "formula": "WCAG sRGB, cutoff 0.04045; selected endpoint compares unrounded black/white ratios",
            "implementation_helper_reused": False,
            "switch_luminance": math.sqrt(0.05 * 1.05) - 0.05,
            "theoretical_floor": math.sqrt(21.0),
        },
        "vector_results": vector_results,
        "former_failure_repeats": repeat_results,
        "determinism": {
            "curated": {
                "accent": "#5fbfae",
                "states": curated_results,
                "persisted_accent_exact": curated_persist_exact,
                "pass": curated_persist_exact
                and all(entry["pass"] for entry in curated_results.values()),
            },
            "non_curated": {
                "accent": "#777777",
                "states": non_curated_results,
                "persisted_accent_exact": non_curated_persist_exact,
                "pass": non_curated_persist_exact
                and all(entry["pass"] for entry in non_curated_results.values()),
            },
        },
        "legacy_gap_context": {
            "#6f6f6f_with_forbidden_#f2e9dc": contrast("#6f6f6f", "#f2e9dc"),
            "#777777_with_forbidden_#241404": contrast("#777777", "#241404"),
        },
        "summary": {
            "vectors_passed": sum(entry["pass"] for entry in vector_results),
            "vectors_total": len(vector_results),
            "repeat_probes_passed": sum(entry["pass"] for entry in repeat_results),
            "repeat_probes_total": len(repeat_results),
            "all_pass": all(all_result_flags),
        },
    }
    args.output.write_text(json.dumps(output, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(output["summary"], sort_keys=True))
    return 0 if output["summary"]["all_pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
