#!/usr/bin/env python3
"""MotionEngine 0.7.1 visual-language benchmark: result.json + report.md with the two-axis gate.

usage: visual_language_report.py <out-dir> --intent I --data-intent D --ref-a P --ref-b P

Reads what scripts/visual_language_benchmark.sh wrote under <out-dir>/{baseline,reference-a,reference-b}
(and data-regression/). Deterministic: fixed key order, no timestamps. No story-specific logic.
"""
import argparse
import hashlib
import json
import os
import sys

STYLE_DIMS = [
    "palette_character",
    "typography_character",
    "material_character",
    "motion_temperament",
    "transition_character",
    "composition_rhythm",
    "scale_contrast",
    "layer_activity",
]
RUNS = ["baseline", "reference-a", "reference-b"]
# Grammars whose beats are data / structured (a visual language must not change them).
STRUCTURED = {"data_story", "evidence_stack", "sequential_stack"}


def load(path):
    try:
        with open(path) as f:
            return json.load(f)
    except Exception:
        return None


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        h.update(f.read())
    return h.hexdigest()


def read_steps(run):
    steps = {}
    errors = {}
    try:
        with open(os.path.join(run, "steps.tsv")) as f:
            for line in f:
                name, rc = line.rstrip("\n").split("\t")
                steps[name] = int(rc)
    except Exception:
        pass
    for name, rc in steps.items():
        if rc != 0:
            err = ""
            try:
                with open(os.path.join(run, "logs", name + ".err")) as f:
                    err = f.read().strip()
            except Exception:
                pass
            errors[name] = err[:600]
    return steps, errors


def visual_weight(cov):
    """The engine's derived weight (`resolve-style --json` → visual_policy.weight)."""
    return ((cov or {}).get("visual_policy") or {}).get("weight") or "neutral"


def coverage_summary(cov):
    if not cov:
        return None
    c = cov.get("coverage", {})
    rows = c.get("rows", [])
    counts = {}
    for r in rows:
        counts[r["status"]] = counts.get(r["status"], 0) + 1
    return {
        "row_status_counts": dict(sorted(counts.items())),
        "matched": len(c.get("matched", [])),
        "partial": len(c.get("partial", [])),
        "overridden": len(c.get("overridden", [])),
        "unknown": len(c.get("unknown", [])),
        "unsupported": len(c.get("unsupported", [])),
        "style_dims": {r["dimension"]: r["status"] for r in rows if r["dimension"] in STYLE_DIMS},
    }


def plan_beats(plan):
    out = []
    for b in (plan or {}).get("beats", []):
        out.append(
            {
                "beat": b.get("beat"),
                "grammar": b.get("composition"),
                "decision_source": b.get("source"),
                "requests": b.get("requests", []),
            }
        )
    return out


def visual_qa(qa):
    v = (qa or {}).get("visual")
    if not v:
        return None
    return {
        "beats": [
            {
                "scene": b.get("scene"),
                "type_dominant": b.get("type_dominant"),
                "figure_share": round(b.get("figure_share", 0.0), 4),
            }
            for b in v.get("beats", [])
        ],
        "pure_type_ratio": round(v.get("pure_type_ratio", 0.0), 4),
        "verdict": v.get("verdict"),
    }


def ffprobe_info(path):
    d = load(path)
    if not d:
        return None
    vs = [s for s in d.get("streams", []) if s.get("codec_type") == "video"]
    if not vs:
        return None
    s = vs[0]
    num, _, den = s.get("r_frame_rate", "0/1").partition("/")
    try:
        fps = round(float(num) / float(den or 1), 3)
    except Exception:
        fps = None
    dur = s.get("duration") or d.get("format", {}).get("duration")
    return {
        "width": s.get("width"),
        "height": s.get("height"),
        "fps": fps,
        "duration_seconds": round(float(dur), 3) if dur else None,
        "codec": s.get("codec_name"),
    }


# ---------------------------------------------------------------- gate

def style_axis(summary):
    if not summary:
        return {"result": "N/A", "reason": "no coverage"}
    dims = summary["style_dims"]
    applicable = {k: v for k, v in dims.items() if v != "unknown"}
    n = len(applicable)
    if n == 0:
        return {"result": "PARTIAL", "reason": "no applicable style rows", "applicable": 0}
    good = sum(1 for v in applicable.values() if v in ("match", "partial"))
    overridden = sorted(k for k, v in applicable.items() if v == "overridden")
    frac = good / n
    if not overridden and frac >= 0.75:
        res = "PASS"
    elif frac < 0.5:
        res = "FAIL"
    else:
        res = "PARTIAL"
    return {
        "result": res,
        "applicable": n,
        "match_or_partial": good,
        "fraction": round(frac, 4),
        "overridden": overridden,
    }


def visual_axis(cov, beats, qa):
    vis = ((cov or {}).get("resolved") or {}).get("visual") or {}
    weight = visual_weight(cov)
    if weight == "neutral" or not qa:
        return {"result": "N/A", "weight": weight, "reason": "neutral weight or no visual QA"}
    grammar = {b["beat"]: b["grammar"] for b in beats}
    qa_beats = []
    for b in qa["beats"]:
        try:
            idx = int(str(b["scene"]).rsplit("_", 1)[1])
        except Exception:
            continue
        qa_beats.append((idx, bool(b["type_dominant"])))
    want_type = weight == "type"
    # beats that carry the reference's dominant mode (visual: not type_dominant; type: type_dominant)
    carrying = [i for i, td in qa_beats if td == want_type]
    avoid = set(vis.get("avoid", []))
    preferred = set(vis.get("preferred", []))
    used = [grammar.get(i) for i, _ in qa_beats]
    carrying_grammars = sorted({grammar.get(i) for i in carrying if grammar.get(i)})
    checks = {
        "a_verdict_consistent": qa["verdict"] == "consistent",
        "b_two_distinct_grammars_in_carrying_beats": len(carrying_grammars) >= 2,
        "c_no_avoided_grammar": not (avoid & {g for g in used if g}),
        "d_preferred_grammar_used": bool(preferred & {g for g in used if g}),
        "e_three_carrying_beats": len(carrying) >= 3,
    }
    if all(checks.values()):
        res = "PASS"
    elif not checks["a_verdict_consistent"]:
        res = "FAIL"
    else:
        res = "PARTIAL"
    return {
        "result": res,
        "weight": weight,
        "checks": checks,
        "carrying_beats": carrying,
        "carrying_grammars": carrying_grammars,
        "avoid": sorted(avoid),
        "preferred": sorted(preferred),
    }


def overall(style, vis):
    s, v = style["result"], vis["result"]
    if v == "N/A":
        return s
    if s == "PASS" and v == "PASS":
        return "PASS"
    if "PASS" in (s, v) or "PARTIAL" in (s, v):
        return "PARTIAL"
    return "FAIL"


# ---------------------------------------------------------------- main

def collect_run(out, name, intent_sha, ref_path):
    run = os.path.join(out, name)
    steps, errors = read_steps(run)
    cov = load(os.path.join(run, "coverage.json"))
    plan = load(os.path.join(run, "asset-plan.json"))
    qa = visual_qa(load(os.path.join(run, "qa.json")))
    ref = load(ref_path) if ref_path else None
    summary = coverage_summary(cov) if ref else None
    beats = plan_beats(plan)
    r = {
        "intent_sha256": intent_sha,
        "reference_fingerprint": (ref or {}).get("reference_fingerprint"),
        "profile_version": (ref or {}).get("version"),
        "steps": dict(sorted(steps.items())),
        "step_errors": dict(sorted(errors.items())),
        "coverage": summary,
        "unsupported_traits": (cov or {}).get("coverage", {}).get("unsupported_traits", []) if ref else [],
        "beats": beats,
        "visual_qa": qa,
        "ffprobe": None,
    }
    mp4s = sorted(f for f in os.listdir(os.path.join(run, "render")) if f.endswith(".mp4")) if os.path.isdir(os.path.join(run, "render")) else []
    if mp4s:
        r["ffprobe"] = ffprobe_info(os.path.join(run, "ffprobe.json"))
    if ref:
        sa = style_axis(summary)
        va = visual_axis(cov, beats, qa)
        r["gate"] = {"style": sa, "visual_language": va, "overall": overall(sa, va)}
    return r


def data_regression(out):
    base = os.path.join(out, "data-regression")
    runs = {}
    for name in RUNS:
        run = os.path.join(base, name)
        steps, errors = read_steps(run)
        runs[name] = {
            "steps": dict(sorted(steps.items())),
            "step_errors": dict(sorted(errors.items())),
            "beats": plan_beats(load(os.path.join(run, "asset-plan.json"))),
        }
    base_beats = {b["beat"]: b for b in runs["baseline"]["beats"]}
    regression = False
    findings = []
    for name in ("reference-a", "reference-b"):
        for b in runs[name]["beats"]:
            b0 = base_beats.get(b["beat"])
            if not b0:
                continue
            changed = b0["grammar"] != b["grammar"]
            gained = b["decision_source"] == "generated_image" and b0["decision_source"] != "generated_image"
            if b0["grammar"] in STRUCTURED and (changed or gained):
                regression = True
                findings.append(
                    {
                        "run": name,
                        "beat": b["beat"],
                        "baseline": b0["grammar"],
                        "with_reference": b["grammar"],
                        "gained_generated_image": gained,
                    }
                )
            elif changed or gained:
                findings.append(
                    {
                        "run": name,
                        "beat": b["beat"],
                        "baseline": b0["grammar"],
                        "with_reference": b["grammar"],
                        "gained_generated_image": gained,
                        "note": "non-structured baseline beat (informational)",
                    }
                )
    return {"regression": regression, "structured_grammars": sorted(STRUCTURED), "findings": findings, "runs": runs}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("--intent", required=True)
    ap.add_argument("--data-intent", required=True)
    ap.add_argument("--ref-a", required=True)
    ap.add_argument("--ref-b", required=True)
    args = ap.parse_args()

    intent_sha = sha256(args.intent)
    refs = {"baseline": None, "reference-a": args.ref_a, "reference-b": args.ref_b}
    result = {
        "case": os.path.basename(os.path.normpath(args.out)),
        "intent": args.intent,
        "intent_sha256": intent_sha,
        "runs": {n: collect_run(args.out, n, intent_sha, refs[n]) for n in RUNS},
    }
    result["intent_identical_across_runs"] = len({r["intent_sha256"] for r in result["runs"].values()}) == 1
    result["data_regression"] = data_regression(args.out)
    result["data_regression"]["intent"] = args.data_intent
    result["data_regression"]["intent_sha256"] = sha256(args.data_intent)

    with open(os.path.join(args.out, "result.json"), "w") as f:
        json.dump(result, f, indent=2)
        f.write("\n")
    write_report(args.out, result)
    for n in ("reference-a", "reference-b"):
        g = result["runs"][n]["gate"]
        print(f"{n}: STYLE {g['style']['result']}  VISUAL {g['visual_language']['result']}  OVERALL {g['overall']}")
    print("data regression:", result["data_regression"]["regression"])


def write_report(out, result):
    L = []
    L.append(f"# Visual-language benchmark: {result['case']}")
    L.append("")
    L.append(f"Intent `{result['intent']}` sha256 `{result['intent_sha256']}` (identical across runs: {result['intent_identical_across_runs']})")
    L.append("")
    L.append("## Gate")
    L.append("")
    L.append("| reference | STYLE | VISUAL-LANGUAGE | OVERALL | weight |")
    L.append("|---|---|---|---|---|")
    for n in ("reference-a", "reference-b"):
        g = result["runs"][n]["gate"]
        L.append(f"| {n} | {g['style']['result']} ({g['style'].get('match_or_partial')}/{g['style'].get('applicable')}) | {g['visual_language']['result']} | **{g['overall']}** | {g['visual_language'].get('weight')} |")
    L.append("")
    for n in ("reference-a", "reference-b"):
        g = result["runs"][n]["gate"]
        v = g["visual_language"]
        L.append(f"### {n} details")
        L.append("")
        s = g["style"]
        L.append(f"- style: {s['result']}, applicable {s.get('applicable')}, match/partial {s.get('match_or_partial')}, overridden {s.get('overridden')}")
        for k, ok in (v.get("checks") or {}).items():
            L.append(f"- visual {k}: {'ok' if ok else 'FAIL'}")
        if v.get("checks"):
            L.append(f"- carrying beats {v['carrying_beats']}, grammars {v['carrying_grammars']}; preferred {v['preferred']}; avoid {v['avoid']}")
        L.append("")
    L.append("## Runs")
    for n in RUNS:
        r = result["runs"][n]
        L.append("")
        L.append(f"### {n}")
        L.append("")
        L.append(f"reference `{r['reference_fingerprint']}` profile v{r['profile_version']}")
        L.append("")
        qa = r["visual_qa"]
        share = {}
        if qa:
            for b in qa["beats"]:
                share[str(b["scene"]).rsplit("_", 1)[-1]] = b
        L.append("| beat | grammar | source | requests | type_dominant | figure_share |")
        L.append("|---|---|---|---|---|---|")
        for b in r["beats"]:
            q = share.get(str(b["beat"]), {})
            L.append(f"| {b['beat']} | {b['grammar']} | {b['decision_source']} | {', '.join(b['requests']) or '-'} | {q.get('type_dominant', '-')} | {q.get('figure_share', '-')} |")
        L.append("")
        if qa:
            L.append(f"pure_type_ratio {qa['pure_type_ratio']}, verdict `{qa['verdict']}`")
        else:
            L.append("no visual QA verdict (no reference)" if n == "baseline" else "visual QA missing")
        if r["ffprobe"]:
            f = r["ffprobe"]
            L.append(f"- video {f['width']}x{f['height']} @ {f['fps']} fps, {f['duration_seconds']}s, {f['codec']}")
        if r["unsupported_traits"]:
            L.append(f"- unsupported traits: {', '.join(r['unsupported_traits'])}")
        if r["step_errors"]:
            L.append(f"- FAILED steps: {', '.join(r['step_errors'])}")
        L.append(f"- sheet: `{out}/{n}/sheet.png`, beat frames `{out}/{n}/beat<N>.png`")
    d = result["data_regression"]
    L.append("")
    L.append("## Data regression (derived-metric story)")
    L.append("")
    L.append(f"intent `{d['intent']}`; **regression: {str(d['regression']).lower()}**")
    L.append("")
    L.append("| run | grammars (source) |")
    L.append("|---|---|")
    for n in RUNS:
        bs = d["runs"][n]["beats"]
        L.append(f"| {n} | " + ", ".join(f"{b['grammar']} ({b['decision_source']})" for b in bs) + " |")
    for f in d["findings"]:
        L.append(f"- {f}")
    L.append("")
    with open(os.path.join(out, "report.md"), "w") as f:
        f.write("\n".join(L))


if __name__ == "__main__":
    sys.exit(main())
