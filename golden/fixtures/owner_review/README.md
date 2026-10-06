# Owner review fixtures (0.10 Q)

Intents written by a weak model during the owner's tests (2026-10-02). They
reproduce the reported problems and are regression inputs for the taste rules
(docs: DECISIONS 84–87):

| fixture | problem it reproduces |
|---|---|
| `spotlight_long_statements` | 20–30-word narration written into `statement` → giant headlines, robotic read |
| `celebrating_life_repetitive` | five `emphasize` + phrase beats with figures → identical layout five times, type over people |

Compile with `--asset-family people_everyday` (celebrating life) or `--art auto`
(variety path). All must pass layout QA including `headline_too_big`,
`text_over_subject`, `subject_too_small`, `frame_too_loose`, `asset_low_contrast`.

`celebrating_life_repetitive` is the regression input of
`crates/motion-render/tests/subject_first.rs`: compiled with
`--asset-family people_everyday` (warm_editorial and playful_print, 9:16 and
1:1, `variety` 1–4 and none, also with a voice-over) it must pass the four
subject checks, rotate its image layouts (consecutive image beats never share
one) and stay deterministic.
