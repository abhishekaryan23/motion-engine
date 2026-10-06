#!/usr/bin/env bash
# MotionEngine 0.7.1 permanent visual-language regression ("DNA case").
#
# One command, no manual steps:
#   scripts/regression_visual_language.sh            # temp output is removed on success
#   VL_REGRESSION_KEEP=1 scripts/regression_visual_language.sh   # keep the temp output for inspection
#
# What it does
#   1. Checks preconditions: the byte-identical DNA CreativeIntent + neutral style, and for each of
#      reference A / B the committed profile (reference-style.json) + evidence.json under
#      references/benchmark071/reference_{a,b}/. A reference whose profile/evidence is missing prints
#      `SKIP — external reference media unavailable` and is skipped (not a failure). Reference VIDEOS are
#      never needed: profiles and evidence are committed.
#   2. Runs scripts/visual_language_benchmark.sh into a TEMP root under output/ (VL_BENCH_OUT), so the
#      committed fixtures under golden/fixtures/benchmarks/ are never overwritten.
#   3. Reads the temp result.json / qa.json / compiled scenes and prints a summary. It only CONSUMES
#      engine and benchmark outputs: it never recomputes type/visual dominance or style coverage.
#        CreativeIntent unchanged   sha256 of the intent vs the hash in the committed golden/fixtures/benchmarks result.json
#        Reference A / B            style transfer, visual-language transfer, pure-type ratio (benchmark gate)
#        scientific meaning         automated proxy: every intent phrase value appears as text in each reference
#                                   run's compiled scene AND no beat is type-dominant (qa.json)
#        content leakage           automated proxy: profiles pass validate-reference-style (closed schema) and
#                                   no compiled-scene string points into references/ or a samples/ dir
#        data regression            from result.json
#   4. Exit 0 only when no gate is FAIL/PARTIAL, the intent is unchanged and no proxy says CHECK.
#
# Human review remains the authority for meaning: benchmark-v071/dna-translesion-style-transfer-01/report.md.
# Related: benchmarks/reference-interpreter/README.md (cheap-interpreter benchmark harness).
set -uo pipefail
cd "$(dirname "$0")/.."

ME=target/release/motion-engine
CASE=dna-translesion-style-transfer-01
SRC=golden/fixtures/benchmarks/$CASE/intent
INTENT=$SRC/attempt-01.intent.json
STYLE=$SRC/neutral.style.json
COMMITTED=golden/fixtures/benchmarks/$CASE/result.json
REFDIR=references/benchmark071
TMP_ROOT=output/regression-visual-language
TMP_OUT=$TMP_ROOT/$CASE   # same depth as output/visual_language_benchmark/<case>: compiled scenes keep a relative asset_root
SKIP_MSG='SKIP — external reference media unavailable'

for f in "$INTENT" "$STYLE" "$COMMITTED"; do
  [ -f "$f" ] || { echo "FAIL: required fixture missing: $f"; exit 1; }
done

present=()   # references whose committed profile + evidence exist
for r in a b; do
  d=$REFDIR/reference_$r
  if [ -f "$d/reference-style.json" ] && [ -f "$d/evidence.json" ]; then present+=("$r")
  else echo "Reference $(echo "$r" | tr a-z A-Z): $SKIP_MSG"; fi
done
if [ ${#present[@]} -eq 0 ]; then echo "nothing to check"; exit 0; fi

cargo build --release -q -p motion-cli 2>/dev/null || { echo 'FAIL: build failed'; exit 1; }

BENCH_RAN=0
if [ ${#present[@]} -eq 2 ]; then
  rm -rf "$TMP_ROOT"; mkdir -p "$TMP_ROOT"
  echo "running benchmark into $TMP_OUT (committed fixtures untouched) ..."
  VL_BENCH_OUT=$TMP_OUT scripts/visual_language_benchmark.sh >"$TMP_ROOT/benchmark.log" 2>&1 \
    || echo "note: benchmark script exited non-zero (see $TMP_ROOT/benchmark.log); gates below decide"
  BENCH_RAN=1
else
  echo "note: the benchmark needs both references; only validating the present profile"
fi

python3 - "$ME" "$INTENT" "$COMMITTED" "$REFDIR" "$TMP_OUT" "$BENCH_RAN" "${present[*]}" <<'PY'
import hashlib, json, os, subprocess, sys

me, intent_path, committed_path, refdir, out, bench_ran, present = sys.argv[1:8]
bench_ran = bench_ran == "1"
present = present.split()
bad = []  # reasons for a non-zero exit


def load(path):
    try:
        with open(path) as f:
            return json.load(f)
    except Exception:
        return None


# -- CreativeIntent unchanged -------------------------------------------------
with open(intent_path, "rb") as f:
    actual = hashlib.sha256(f.read()).hexdigest()
recorded = (load(committed_path) or {}).get("intent_sha256")
unchanged = recorded is not None and actual == recorded
print(f"CreativeIntent unchanged: {'YES' if unchanged else 'NO'}")
if not unchanged:
    bad.append("CreativeIntent changed")

result = load(os.path.join(out, "result.json")) if bench_ran else None
if bench_ran and result is None:
    bad.append("benchmark produced no result.json")
    print("FAIL: benchmark produced no result.json")

# -- per-reference gates ------------------------------------------------------
phrases = []


def collect_phrases(n):
    if isinstance(n, dict):
        if n.get("kind") == "phrase" and isinstance(n.get("value"), str):
            phrases.append(n["value"])
        for v in n.values():
            collect_phrases(v)
    elif isinstance(n, list):
        for v in n:
            collect_phrases(v)


collect_phrases(load(intent_path))


def scene_texts(n, acc):
    if isinstance(n, dict):
        if n.get("type") == "text" and isinstance(n.get("text"), str):
            acc.append(n["text"])
        for v in n.values():
            scene_texts(v, acc)
    elif isinstance(n, list):
        for v in n:
            scene_texts(v, acc)


def strings(n, acc):
    if isinstance(n, str):
        acc.append(n)
    elif isinstance(n, dict):
        for v in n.values():
            strings(v, acc)
    elif isinstance(n, list):
        for v in n:
            strings(v, acc)


meaning_ok = True
leak_ok = True
for r in ("a", "b"):
    name = f"reference-{r}"
    label = f"Reference {r.upper()}"
    if r not in present:
        continue  # SKIP already printed by the shell wrapper
    profile = f"{refdir}/reference_{r}/reference-style.json"
    bundle = f"{refdir}/reference_{r}"
    # content leakage (a): closed-schema validation, engine verdict only
    v = subprocess.run([me, "validate-reference-style", profile, "--bundle", bundle, "--json"],
                       capture_output=True, text=True)
    if v.returncode != 0:
        leak_ok = False
    if not bench_ran or result is None:
        print(f"{label}: benchmark not run (profile validation exit {v.returncode})")
        continue
    run = (result.get("runs") or {}).get(name) or {}
    gate = run.get("gate") or {}
    style, vis = gate.get("style") or {}, gate.get("visual_language") or {}
    sres, vres = style.get("result", "FAIL"), vis.get("result", "FAIL")
    ratio = (run.get("visual_qa") or {}).get("pure_type_ratio")
    frac = f"{style.get('match_or_partial')}/{style.get('applicable')}"
    print(f"{label}: style transfer {sres} ({frac}), visual-language transfer {vres}, pure-type ratio {ratio}")
    if sres != "PASS" or vres != "PASS":
        bad.append(f"{label} gate {sres}/{vres}")
    if run.get("step_errors"):
        bad.append(f"{label} step errors {run['step_errors']}")
    # scientific meaning proxy: intent phrases as text + no type-dominant beat (engine qa.json)
    scene = load(os.path.join(out, name, "dna.motion.json"))
    qa = load(os.path.join(out, name, "qa.json"))
    if scene is None or qa is None:
        meaning_ok = leak_ok = False
        continue
    texts = []
    scene_texts(scene.get("scenes"), texts)
    words = {w.lower() for t in texts for w in t.split()}
    missing = [p for p in phrases if not all(w.lower() in words for w in p.split())]
    td = [b.get("scene") for b in (qa.get("visual") or {}).get("beats", []) if b.get("type_dominant")]
    if missing or td:
        meaning_ok = False
        print(f"  {label} meaning detail: missing phrases {missing}, type-dominant beats {td}")
    # content leakage (b): no compiled-scene string points into references/ or a samples/ dir
    strs = []
    strings(scene, strs)
    hits = [s for s in strs if "references/" in s or "samples/" in s]
    if hits:
        leak_ok = False
        print(f"  {label} leakage detail: scene paths into reference media {hits[:3]}")

if bench_ran:
    print("scientific meaning: " + ("PRESERVED" if meaning_ok else "CHECK")
          + " (automated proxy; human review: benchmark-v071 report.md)")
    if not meaning_ok:
        bad.append("scientific meaning proxy CHECK")
print("content leakage: " + ("NONE" if leak_ok else "CHECK")
      + " (automated proxy: closed-schema validation + no scene path into references/ or samples/)")
if not leak_ok:
    bad.append("content leakage proxy CHECK")

# -- data regression ----------------------------------------------------------
if bench_ran and result is not None:
    dr = result.get("data_regression") or {}
    reg = dr.get("regression")
    findings = dr.get("findings") or []
    print(f"data regression: {'YES' if reg else 'NONE'}" + (f" {findings}" if reg or findings else ""))
    if reg is not False:
        bad.append("data regression")

print("RESULT: " + ("PASS" if not bad else "FAIL — " + "; ".join(bad)))
sys.exit(0 if not bad else 1)
PY
rc=$?
if [ "$rc" -eq 0 ] && [ "${VL_REGRESSION_KEEP:-0}" != "1" ]; then rm -rf "$TMP_ROOT"; fi
exit "$rc"
