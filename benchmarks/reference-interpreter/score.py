#!/usr/bin/env python3
"""Score one interpreter response against an evidence bundle.

    score.py BUNDLE RESPONSE [--model ID] [--expected FILE] [--repair-response FILE]
             [--intent FILE --style FILE] [--out FILE] [--engine PATH] [--note TEXT]

Provider-neutral: it never calls a model or the network. It runs the motion-engine CLI
(`validate-reference-style --json`, `resolve-style --json`) as a subprocess and reads ONLY that JSON.
Normalization, alias folding, confidence thresholds, visual-language weights and coverage are all
the engine's; nothing is re-implemented here. The result follows fixtures/result.template.json.

Exit code: 0 when a result was written (even for an invalid response), 2 on usage / engine errors.
"""
import argparse
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
DEFAULT_INTENT = "golden/fixtures/benchmarks/dna-translesion-style-transfer-01/intent/attempt-01.intent.json"
DEFAULT_STYLE = "golden/fixtures/benchmarks/dna-translesion-style-transfer-01/intent/neutral.style.json"
# Engine fidelity labels (from validate-reference-style --json) that mean "not applied".
NOT_APPLIED = ("unknown", "low_confidence")


def run_json(cmd):
    """Run an engine command; return (exit_code, parsed_json_or_None, stderr)."""
    p = subprocess.run(cmd, capture_output=True, text=True, cwd=REPO)
    try:
        data = json.loads(p.stdout)
    except Exception:
        data = None
    return p.returncode, data, p.stderr.strip()


def validate(engine, profile, bundle, repair_path=None):
    cmd = [engine, "validate-reference-style", profile, "--bundle", bundle, "--json"]
    if repair_path:
        cmd += ["--repair-request", repair_path]
    rc, data, err = run_json(cmd)
    if data is None:  # engine failed before producing JSON (e.g. unreadable / non-JSON file)
        data = {"valid": False, "issues": [{"path": "$", "message": err or "no engine output"}],
                "aliases": [], "normalized": None}
    return rc, data


def completeness(rows):
    def frac(sel):
        total = len(sel)
        applied = sum(1 for r in sel if r.get("fidelity") not in NOT_APPLIED)
        return {"applied": applied, "total": total, "fraction": round(applied / total, 3) if total else None}

    # accent_hint is a palette-derived evidence row without a confidence: not a graded dimension.
    style = [r for r in rows if not r["dimension"].startswith("visual.") and r["dimension"] != "accent_hint"]
    visual = [r for r in rows if r["dimension"].startswith("visual.")]
    return {"style": frac(style), "visual_language": frac(visual)}


def visual_section(rows, principles, policy):
    vrows = [r for r in rows if r["dimension"].startswith("visual.")]
    present = any(r.get("fidelity") not in NOT_APPLIED for r in vrows)
    v = (principles or {}).get("visual") or {}
    groups = [set(v.get("preferred") or []), set(v.get("secondary") or []), set(v.get("avoid") or [])]
    disjoint = all(not (groups[i] & groups[j]) for i in range(3) for j in range(i + 1, 3))
    medium = v.get("medium")
    return {
        "present": present,
        "medium": medium,
        "medium_non_neutral": (medium not in (None, "neutral")) if present else False,
        # Derived by the engine (validate-reference-style --json → visual_policy).
        "weight": (policy or {}).get("weight"),
        "composition_lists_disjoint": disjoint if present else None,
        "prefers_procedural": (policy or {}).get("prefers_procedural"),
        "wants_images": (policy or {}).get("wants_images"),
    }


def agreement(expected, rows):
    by_dim = {r["dimension"]: r for r in rows}
    out = {}
    for dim, accepted in (expected or {}).get("expected", {}).items():
        row = by_dim.get(dim)
        observed = row.get("reference") if row else None
        out[dim] = None if observed is None else (observed in accepted)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("bundle")
    ap.add_argument("response", help="response.raw.txt or reference-style.json, saved unchanged")
    ap.add_argument("--model", default="unspecified", help="free-form model id string")
    ap.add_argument("--expected", help="expected/<case>.json ground truth (optional)")
    ap.add_argument("--repair-response", help="the single repair response (response.repair.raw.txt), if any")
    ap.add_argument("--intent", default=DEFAULT_INTENT, help="intent used for the coverage run")
    ap.add_argument("--style", default=DEFAULT_STYLE, help="style used for the coverage run")
    ap.add_argument("--engine", default=os.environ.get("MOTION_ENGINE", os.path.join(REPO, "target/release/motion-engine")))
    ap.add_argument("--out", help="result JSON path (default: stdout)")
    ap.add_argument("--note", default="")
    a = ap.parse_args()

    if not os.path.exists(a.engine):
        print(f"engine not found: {a.engine} (cargo build --release -p motion-cli)", file=sys.stderr)
        return 2
    bundle = os.path.abspath(a.bundle)
    evidence_path = os.path.join(bundle, "evidence.json")
    try:
        fingerprint = json.load(open(evidence_path)).get("reference_fingerprint")
    except Exception as e:
        print(f"cannot read {evidence_path}: {e}", file=sys.stderr)
        return 2
    response = os.path.abspath(a.response)

    repair_path = (os.path.abspath(a.out) + ".repair-request.json") if a.out else None
    rc1, first = validate(a.engine, response, bundle, repair_path)
    first_valid = bool(first.get("valid"))
    issues = list(first.get("issues") or [])
    aliases = list(first.get("aliases") or [])
    repair_required = not first_valid
    repair_valid = None
    final, final_profile = first, response
    if repair_required and a.repair_response:
        _, second = validate(a.engine, os.path.abspath(a.repair_response), bundle)
        repair_valid = bool(second.get("valid"))
        if repair_valid:
            final, final_profile = second, os.path.abspath(a.repair_response)
            aliases = list(second.get("aliases") or [])
    if first_valid and repair_path and os.path.exists(repair_path):
        os.remove(repair_path)  # engine only writes it for invalid profiles; defensive

    normalized = final.get("normalized") if final.get("valid") else None
    rows = (normalized or {}).get("dimensions") or []
    result = {
        "model": a.model,
        "bundle_fingerprint": fingerprint,
        "first_attempt_valid": first_valid,
        "repair_required": repair_required,
        "repair_valid": repair_valid,
        "issues": issues,
        "completeness": completeness(rows) if normalized else None,
        "unknown_dimensions": [r["dimension"] for r in rows if r.get("fidelity") == "unknown"],
        "low_confidence_dimensions": [r["dimension"] for r in rows if r.get("fidelity") == "low_confidence"],
        "aliases": aliases,
        "coverage": None,
        "visual_language": visual_section(rows, (normalized or {}).get("principles"), final.get("visual_policy"))
        if normalized
        else None,
        "agreement": None,
        "notes": a.note,
    }
    if normalized:
        rc, cov, err = run_json([a.engine, "resolve-style", a.intent, "--style", a.style,
                                 "--reference-style", final_profile, "--json"])
        c = (cov or {}).get("coverage")
        if c is not None:
            result["coverage"] = {k: len(c.get(k) or []) for k in
                                  ("matched", "partial", "overridden", "unsupported", "unknown")}
        else:
            result["notes"] = (result["notes"] + " coverage unavailable: " + err).strip()
        if a.expected:
            result["agreement"] = agreement(json.load(open(a.expected)), rows)

    text = json.dumps(result, indent=2) + "\n"
    if a.out:
        with open(a.out, "w") as f:
            f.write(text)
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
