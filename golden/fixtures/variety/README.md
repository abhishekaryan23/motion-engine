# Variety bench speech fixtures (sprint 0.23)

One offline speech map per benchmark story, read by `scripts/variety_bench.sh`
(and `crates/motion-render/tests/sprint23_regressions.rs`) as `compile --speech`.
Only the `.speech.json` is committed; the synthesised wav is never kept
(`compile --speech` reads the JSON only, it does not need the wav).

| File | Story |
|---|---|
| `sleep_review.speech.json`, `money_review.speech.json` | `docs/plans/sprint_0_23/stories/*.intent.json` |
| `collection-accumulate`, `state-change`, `derived-metric`, `layers` | `examples/public/*.intent.json` |
| `ai_learns` | `examples/topics/ai_learns.intent.json` |
| `space` | `examples/cinematic/space.intent.json` |

## How they were made

On macOS, offline, no provider, no key:

```
motion-engine voice <story>.intent.json --tts-model say --align onset -o <tmp dir>
```

(no `--style`, so the default emotion `drama` picks the macOS voice **Daniel**
at 152 words per minute; one take per story, 48 kHz mono, word times estimated
from the audio by the onset aligner). Each `<tmp dir>/<title>.speech.json` is
copied here as `<story file name>.speech.json`; the wav stays in the temp dir
and is deleted.

Regenerate all eight with:

```
REGEN_FIXTURES=1 scripts/variety_bench.sh
```

The timings come from the audio `say` produced on the machine that ran it, so a
regeneration may move word times by a few milliseconds; commit the new files
only when the bench baseline is re-recorded with them.

These are test fixtures, not goldens: no hash in `golden/hashes.json` depends
on them.

## `tight/`: the continuous take (A4-short)

The owner's voice rule is one continuous take, so a real narration leaves 0.25-0.6 s
between sentences; the `say` fixtures above leave up to 1.2-1.3 s in places
(`state-change` 1.32 s, `layers` 1.24 / 1.32 s, `money_review` 1.24 s). `tight/` holds
the same eight maps with every gap between two consecutive sentences set to exactly
0.30 s (the words of a sentence move together, each keeps its duration, the sentences
keep their length, the first sentence stays at 0.35 s, `duration` is shortened by what
was removed and the 0.9 s tail after the last word is kept). A one-sentence story
(`collection-accumulate`, `derived-metric`) is unchanged.

Made by a small deterministic script, no clock, no randomness, no audio:

```
python3 docs/plans/sprint_0_23/tools/tighten_speech.py --all      # or: REGEN_TIGHT=1 scripts/variety_bench.sh
python3 docs/plans/sprint_0_23/tools/tighten_speech.py in.speech.json out.speech.json --gap 0.30
```

Run the bench on them with `SPEECH=tight scripts/variety_bench.sh` (report in
`output/sprint23/bench_tight/`, the report says `"speech": "tight"`). Regenerate the
tight files whenever the default fixtures are regenerated.
