# Reference-interpreter benchmark (cheap / weak multimodal models)

Purpose: measure how well a cheap multimodal interpreter (Gemini Flash-class, GPT Luna-class, others) fills a
`ReferenceStyleProfile` using only the provider-neutral protocol
([REFERENCE_STYLE_INTERPRETER](../../docs/REFERENCE_STYLE_INTERPRETER.md),
[REFERENCE_STYLE_PROFILE](../../docs/REFERENCE_STYLE_PROFILE.md)). MotionEngine makes no model call and ships no
provider SDK; you run the model yourself and feed its text answer to `score.py`.

## Protocol (per model, per reference)

1. Generate the evidence bundle:
   `motion-engine reference-evidence VIDEO --out-dir BUNDLE --cache-dir output/reference-cache`
2. Give the model ONLY `BUNDLE/interpreter-prompt.md` + `BUNDLE/samples/*.jpg` (+ `BUNDLE/contact-sheet.png`).
   Never the repository, implementation source, schemas' Rust code, the target story, or expected values.
3. Save the returned text unchanged as `response.raw.txt`; if it is JSON also save it as `reference-style.json`.
4. Validate: `motion-engine validate-reference-style reference-style.json --bundle BUNDLE --json`.
   If invalid, add `--repair-request repair.json`, send that request to the model ONCE, and save the answer
   unchanged as `response.repair.raw.txt`. No second repair.
5. Score (below) and keep the result JSON next to the response.
6. Never tune the profile or the prompt per reference. A prompt/protocol change is a new benchmark generation.

## Scoring

```sh
cargo build --release -p motion-cli
python3 benchmarks/reference-interpreter/score.py BUNDLE response.raw.txt \
    --model "<model id>" [--expected benchmarks/reference-interpreter/expected/cal_1.json] \
    [--repair-response response.repair.raw.txt] [--intent I.json --style S.json] --out result.json
```

`score.py` runs `validate-reference-style --json` and `resolve-style --json` as subprocesses and reads only their
JSON. Alias folding, normalization, confidence thresholds and coverage stay in the engine; the scorer only counts
and compares. Coverage uses the DNA intent + neutral style by default (override with `--intent/--style`). No network.
The output follows `fixtures/result.template.json`; `null` means not applicable or not emitted by the engine.

| metric | where it comes from |
|---|---|
| `first_attempt_valid`, `repair_required`, `repair_valid`, `issues` | `validate-reference-style --json` (`valid`, `issues`) on the first / repair response |
| `completeness.style` / `.visual_language` | normalized dimension rows: applied = fidelity not `unknown`/`low_confidence`, over all rows (`visual.*` rows counted separately; `accent_hint` excluded) |
| `unknown_dimensions`, `low_confidence_dimensions` | rows with fidelity `unknown` / `low_confidence` |
| `aliases` | engine alias rewrites (`aliases`) |
| `coverage` | list sizes of `resolve-style --json` `coverage.{matched,partial,overridden,unsupported,unknown}` |
| `visual_language` | `present` = any applied `visual.*` row; `medium` / `medium_non_neutral` from the normalized principles; `composition_lists_disjoint` = engine `preferred`/`secondary`/`avoid` do not overlap; `weight`, `prefers_procedural` and `wants_images` come from the engine's `visual_policy` (validate-reference-style --json) |
| `agreement` | with `--expected`: per dimension, the profile-vocabulary value the engine reports is in the accepted list (`true`/`false`); `null` when the model emitted no value |

Compare models by aggregating result files: first-attempt validity rate, repair success rate, mean completeness,
coverage matched/(matched+partial+unknown+unsupported), agreement rate. Report counts, not a single score.

## Layout

- `expected/cal_{1,2,3}.json` ground truth for the three 0.6 calibration references (cal_1 dark technical,
  cal_2 playful print, cal_3 warm editorial), derived from `examples/taste/*.style.json`; only dimensions the source
  style states unambiguously. Bundles: `references/calibration/cal_N` (evidence only; videos are not committed).
- `fixtures/result.template.json` result shape. `fixtures/example.cal_1.result.json` is an EXAMPLE produced by
  scoring the committed `references/calibration/cal_1/reference-style.round1.json` (a v0.1 profile, so it has no
  `visual_language`); it is not a benchmark result.
- `score.py` the scorer (Python 3 standard library only).

## Related

- `scripts/regression_visual_language.sh` is the permanent DNA visual-language regression: one command, writes to a
  temporary output root, prints per-reference style/visual-language transfer, intent-hash check, automated proxies
  for scientific meaning and content leakage, and the data-story regression; exits non-zero on any FAIL/PARTIAL/CHECK.
  Usage: `scripts/regression_visual_language.sh` (`VL_REGRESSION_KEEP=1` keeps the temp output). It needs only the
  committed profiles + evidence under `references/benchmark071/`, never reference videos; a reference without them
  prints `SKIP — external reference media unavailable`. Human review of meaning stays authoritative
  (`benchmark-v071/dna-translesion-style-transfer-01/report.md`).
- `scripts/visual_language_benchmark.sh` (honours `VL_BENCH_OUT=<dir>`; default writes `output/visual_language_benchmark/`).
