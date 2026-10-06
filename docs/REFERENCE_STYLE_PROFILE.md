# ReferenceStyleProfile v0.2 (v0.1 compatible)

`schema/reference-style-profile-v0.2.schema.json` is the current external contract (0.7.1); `schema/reference-style-profile-v0.1.schema.json` is the frozen legacy contract and is still accepted. The rest of this page applies to both. It is the supported external contract for expressing reusable style observations. Every trait uses a closed enum value (or `null`), a confidence from 0 to 1, and optional evidence links to sample ids, metric keys, and time ranges. Unknown fields and free-text fields are rejected. The profile carries no source copy, identity, geometry, or timing instructions.

Dimensions cover tone, polarity, temperature, contrast and palette character; background, typography, material and image treatment; visual density, composition rhythm, motion temperament, transitions, scale contrast, and layer activity. Approximate palette swatches and unsupported visible traits are evidence fields. Unsupported traits are recorded, not imitated.

`reference_fingerprint` is optional in the schema, but including the request's `rf1-…` value enables provenance validation against that bundle. A profile without a matching fingerprint can still be validated without bundle provenance checks.

The compiler normalizes a valid profile into internal `ReferencePrinciples`. These fill dimensions left open by `StyleProfile`; explicit style choices take precedence. Values below confidence `0.5`, unknown values, and unsupported observations do not drive style resolution. Material and image-treatment principles propagate through the compiler. Approximate source RGB values are not copied into output: they inform selection from curated palette families and accents.

Coverage reports one of `MATCH`, `PARTIAL`, `OVERRIDDEN`, `UNSUPPORTED`, or `UNKNOWN`. These are diagnostic states describing how requested principles relate to resolved output, not an aesthetic score or quality guarantee.

## v0.2: visual language (0.7.1)

v0.2 = v0.1 + one optional `visual_language` table describing what fills the frame, as opposed to how it looks:

| field | meaning |
|---|---|
| `medium` | type_only, type_led, image_led, object_led, diagrammatic, collage, interface_led, mixed |
| `asset_usage` | none, sparse, balanced, dense |
| `type_image_balance` | type_dominant, balanced, visual_dominant |
| `asset_character` | photographic, cutout, illustrative, diagrammatic, object_centric, collage, interface, procedural, mixed |
| `asset_roles` | up to 4 `{role, confidence}` |
| `composition_language` | `{preferred, secondary, avoid, confidence}` over the engine's composition grammars |
| `explanation_mode` | literal, diagrammatic, symbolic, metaphorical, evidence_based, mixed |

Traits follow the same `{value | null, confidence, evidence?}` rule: confidence below `0.5` is recorded, not applied. The table carries no free text, layouts, objects, characters, brands or images. How it is normalized and used (grammar bias, entity depiction, asset planning, QA) is specified in [VISUAL_LANGUAGE.md](VISUAL_LANGUAGE.md).

Compatibility: a v0.1 document is parsed by the frozen v0.1 types, converted losslessly, and has a neutral visual language, so it compiles exactly as before 0.7.1. A v0.1 document may not carry `visual_language`; use `"version": "0.2"`. `reference_fingerprint` and coverage behave identically for both versions.
