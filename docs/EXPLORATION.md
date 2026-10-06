# Exploration (0.9)

`--explore N` (0–3, default 0) and `--seed S` are **operator** inputs
(`CompileOptions`), never CreativeIntent fields. They widen only curated,
Curated sets (`compiler/explore.rs`, `compiler/typography.rs`,
`assets/fonts/registry.json`). Level 0 is canonical and byte-identical to 0.8.

Every choice = hash(seed, level, story key, dimension) — no RNG, no clocks.
Same intent + style + seed + level (+ canvas) → byte-identical output.

## What each level may change
| level | dimensions (cumulative) |
|---|---|
| 1 | typography option within the emotion · accent (curated pair) · background field order · mirror · SFX sound ids inside their families |
| 2 | + adjacent emotion's typography · background grammar neighbour · transition family ±1 · scale contrast ±1 · motion amplitude ±1 · asset-family ties (equal-score catalog matches) |
| 3 | + any emotion listed for the tone · palette family neighbour · composition rhythm ±1 · density ±1 · scale contrast ±2 (oversized type) |

Neighbour tables: background PaperCollage↔PaperField↔CleanFlat, TechnicalGrid→CleanFlat,
PrintFields→PaperCollage; palette WarmPaper↔CoolPaper, DarkWarm↔DarkCool,
PrintBright→WarmPaper, PrintDark→DarkWarm; ordered scales for transition
(subtle, editorial, geometric, kinetic), scale contrast, amplitude, rhythm, density.

Never explored: statements, numbers, labels, entity identity, beat order, data /
number / structured grammars. Classic tone keeps its taste (only typography
energy/drama options, SFX and asset ties vary). Grammar alternates are recorded as
`unavailable`: `grammar::select` has no semantic tie table, and inventing one is
a design decision for a later sprint.

## Guardrails
Each explored compile is checked against the canonical (level 0) compile of the
same story, because canonical output itself has by-design exceptions (ghost words
bleeding off-canvas, a 4.0:1 accent button):
- text/ground contrast ≥ min(4.5, canonical − 0.25) for ink/paper and on-accent/accent;
- text size ≥ 20u px for layers that pass at level 0;
- `validate` clean; static text boxes inside the canvas (layers passing at level 0).
On failure every dimension of the highest explored level falls back to canonical
(recorded as `fallback: "guardrail: …"`) and the project recompiles one level
lower, until it passes. In sweeps ≈10 % of level-2/3 seeds fall back, mostly from
subtle scale contrast shrinking furniture labels below 20 px.

## Records
- `ProjectMeta.exploration` = `{level, seed, choices[{dimension, level, value, canonical, fallback?}]}`
  (absent at level 0).
- `theme.typography` = `{emotion, option, legacy?, fallback_from[]}` when the registry chose faces.

## Commands
```
motion-engine compile intent.json --style s.json --explore 2 --seed 7 -o out.motion.json
motion-engine explore intent.json --style s.json --explore 2 --variants 6 --seed 0 -o output/explore/
motion-engine plan-audio scene.motion.json ... --explore 2 --seed 7
motion-engine resolve-style intent.json --style s.json --explore 1 --seed 3 --json
```
`explore` writes `variant_<i>.motion.json` (seeds S..S+K−1), `sheet.png` (one row
per variant, READ/EVOLVE midpoint of each beat) and `explore.json`.
