# Public interface (AI consumer boundary)

These files are the **complete, supported interface** for authoring and rendering MotionEngine inputs.
A model or human needs nothing else. (0.4 and 0.5 changed no schema. Images are planned by the engine itself (`motion-engine plan-assets`); generating them is an optional, vendor-neutral step for tool integrators described in `docs/GENERATED_ASSET_PROTOCOL.md` — intent authors never write image fields.)

```
PROJECT_NORTH_STAR.md
docs/AI_AUTHORING_GUIDE.md
docs/CLI_CONSUMER_GUIDE.md
schema/creative-intent-v0.2.schema.json   (current, recommended)
schema/creative-intent-v0.1.schema.json   (legacy, still supported)
schema/style-profile-v0.1.schema.json
schema/reference-style-profile-v0.2.schema.json   (current, adds visual_language)
schema/reference-style-profile-v0.1.schema.json   (legacy, still accepted)
docs/REFERENCE_EVIDENCE.md
docs/REFERENCE_STYLE_INTERPRETER.md
docs/REFERENCE_STYLE_PROFILE.md
docs/VISUAL_LANGUAGE.md
examples/public/minimal-emphasize.intent.json      (v0.1)
examples/public/minimal-contrast.intent.json       (v0.1)
examples/public/three-beat-story.intent.json       (v0.1)
examples/public/collection-accumulate.intent.json  (v0.2: collection + accumulate)
examples/public/state-change.intent.json           (v0.2: single and dual state_change)
examples/public/derived-metric.intent.json         (v0.2: two derived_metric ratios compared)
examples/public/layers.intent.json                 (v0.2: a layers stack, one beat per layer)
examples/public/minimal.style.json
examples/taste/*.style.json                        (0.6: three complete taste styles)
```

## 0.22: stat cards, rankings, warnings

No schema change. What the existing fields already said is now drawn in every look: an `object` with `value` + `meaning` is a stat card, two of them in a compare/contrast beat are shown as equals, and a `collection` of three or more items with numeric values is a ranking of horizontal bars in the written order (time series stay vertical). Described in `docs/AI_AUTHORING_GUIDE.md` (section 4d); DECISIONS 130–132. New compile warnings (stdout `warning[<code>]: beat N: ...`, MCP `findings`): `unnamed_picture` (a beat's picture its narration never names) and `cue_dropped` (a figure or stamp that could not wait for its word). With a voice-over, a keyword is stamped only when the narrator says it. CLI: `render` deletes its PNG frames after encoding the MP4 unless `--keep-frames` (`docs/CLI_CONSUMER_GUIDE.md`).

## 0.21: brand colours

Public (part of `schema/style-profile-v0.1.schema.json`, additive, optional): `brand` on StyleProfile, an object with `primary`, `secondary`, `background`, `text` as hex `#RRGGBB` / `#RGB` (all optional). Described in `docs/AI_AUTHORING_GUIDE.md` (section 8c); DECISIONS 129. A style without `brand` (or with `{}`) compiles exactly as before. The CLI flag `--brand` on `compile` and `reel` sets the same field. Unreadable colours are replaced and reported as the compile warning `brand_contrast`.

## 0.6: taste and 0.7: reference style

Public (part of `schema/style-profile-v0.1.schema.json`, additive, all optional, default `auto`):
`tone` (`auto`, `editorial`, `technical`, `playful`, and the 0.14 genres `street`, `documentary`, `hype`, `studio`, plus `cinematic` (0.17)), `polarity`, `temperature`, `temperament`, `density` on StyleProfile. They are described in `docs/AI_AUTHORING_GUIDE.md` (section 8b) and explained in `docs/TASTE_DIRECTOR.md`. An all-`auto` style compiles exactly as before 0.6.

Public schemas also include `schema/reference-style-profile-v0.2.schema.json` (current; 0.7.1 adds the optional `visual_language` table, see `docs/VISUAL_LANGUAGE.md`) and the frozen legacy `schema/reference-style-profile-v0.1.schema.json`. The reference workflow is documented in `docs/REFERENCE_EVIDENCE.md`, `docs/REFERENCE_STYLE_INTERPRETER.md`, and `docs/REFERENCE_STYLE_PROFILE.md`. CLI commands are `reference-evidence`, `validate-reference-style`, and `resolve-style`; `compile` and `plan-assets` accept `--reference-style` (see `docs/CLI_CONSUMER_GUIDE.md`).

`style-preview` and `compare-styles` remain supported review commands; their JSON outputs are informational, not schema-versioned.

Internal / unstable (may change without notice, not in any schema):
- `ResolvedStyleProfile` and everything it contains (palette family, background grammar, typography pairing, motion temperament, transition character, composition rhythm, scale contrast, layer activity, variation). Also the output of `style-preview --json`.
- `VisualLanguage` (`compiler/visual.rs`), the normalized visual-construction policy, and `Depiction`.
- `StyleFingerprint` and its dimensions (the dimension names printed by `compare-styles` may be extended).
- `ReferencePrinciples`, the normalized compiler input. The public input contract is `ReferenceStyleProfile` v0.2 (v0.1 accepted); the CLI validates and normalizes it at the consumer boundary.

Everything else is **implementation detail** from the consumer's point of view and may change without notice:
`crates/**`, `assets/**`, `golden/**`, `examples/*.json` (engine demos), compiled MotionScene files (`*.motion.json`), and all other docs.

Guarantees:
- The schemas are generated from the engine's Rust types. A test fails if they drift.
- CreativeIntent v0.1 is frozen: v0.1 documents are parsed by the unchanged v0.1 types and compile exactly like the same document declared as `"version": "0.2"`. Rendering quality improves across engine releases for both versions (0.4 added scene lifecycle and composition grammars without any schema change). New features (`collection`, `state_change`, `derived_metric`, `layers`, `accumulate`) require `"version": "0.2"`.
- A document the CreativeIntent schema accepts will compile. Tests enforce this for representative valid and invalid cases.
- The public examples are compiled, validated and rendered by an automated test (`crates/motion-cli/tests/public_interface.rs`) and by `scripts/public_smoke.sh`.
