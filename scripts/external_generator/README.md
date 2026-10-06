# External fixture generator

A **procedural stand-in for an AI image generator**. It lives outside the Rust
engine on purpose: it consumes `asset-prompts.json` (the vendor-neutral request)
and writes a delivery directory (the vendor-neutral response) that
`motion-engine ingest-assets` ingests. It draws no photographs and calls no
model; sidecars say `"kind": "procedural stand-in, not an AI image"`.
A real generator only has to honour the same contract
(`docs/GENERATED_ASSET_PROTOCOL.md`, "Delivery contract").

Requires Python 3 with Pillow and numpy. No network.

```
python3 scripts/external_generator/fixture_generator.py <asset-prompts.json> <delivery-dir> \
    [--defect none|head_cropped|too_small|no_alpha] [--only SPEC_ID] [--with-head-metadata]
target/release/motion-engine ingest-assets <asset-prompts.json> <delivery-dir> -o manifest.json
```

Per spec it writes `<file_stem>.png` (when `output.alpha_required`, real alpha
cutout) or `<file_stem>.jpg` (quality 90), plus `<file_stem>.json`:
`{fingerprint, generator{tool, kind, spec_id, framing, seed}}`. Face/head bounds
are omitted by default (the engine estimates them from alpha);
`--with-head-metadata` adds `head_bounds` for person framings.

Framings: `half_figure` 1024x1365, `head_and_shoulders` 1024x1280,
`full_figure` 1024x1536 (an office worker); `object` 1024x1024 (coffee mug);
`scene` 1080x1440 and `artifact` 900x1200 (opaque JPEG).

Defects (QA demos): `head_cropped` pushes the head through the top edge;
`too_small` writes a 400 px short side; `no_alpha` writes an opaque white PNG.
The defect is also recorded in `generator.defect`.

Deterministic: the numpy RNG is seeded from the spec fingerprint, there is no
time-dependent data, and PNG/JPEG encoder settings are fixed, so the same input
gives byte-identical files. Everything is drawn at 2x and downsampled (LANCZOS,
premultiplied) for clean anti-aliased cutout edges. A person takes ~20 s.
