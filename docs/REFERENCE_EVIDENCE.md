# Reference evidence

`motion-engine reference-evidence VIDEO --out-dir DIR` analyzes a video locally and writes a bundle for a separate multimodal interpreter. The bundle contains `evidence.json`, sampled JPEG frames, a contact sheet, `interpreter-request.json`, and `interpreter-prompt.md`. It does not call a model or upload media.

Evidence v0.1 records stream metadata, deterministic color and visual-complexity measurements, and temporal activity/change measurements. Audio contributes only `audio_present`; no sound, speech, transcript, or beat analysis is performed. The analyzer uses raw visual event detection with noise-corrected activity measurements. Its algorithm version is `0.7.3`; the evidence document version remains `0.1`.

The `rf1-<16 hex>` reference fingerprint identifies the media bytes together with analyzer version and analysis configuration. A profile can copy this fingerprint to establish provenance. Do not treat evidence as a style decision: an external interpreter combines measurements and sampled images to produce a `ReferenceStyleProfile`.

See [REFERENCE_STYLE_INTERPRETER.md](REFERENCE_STYLE_INTERPRETER.md) for the file protocol and [CLI_CONSUMER_GUIDE.md](CLI_CONSUMER_GUIDE.md) for commands.

## Measurements and cost

| Area | Evidence |
|---|---|
| Metadata | Width, height, FPS, duration, frame count, aspect/orientation, codec, audio presence |
| Samples | Periodic, settled scene-change, high-motion and stable/read samples with ids and timestamps |
| Color | Luminance/contrast/saturation distributions, light/dark fraction, warm/cool balance, quantized palette, accent prevalence and stability |
| Temporal | Cuts and meaningful changes per 10 seconds, hold distribution, static/low/high activity fractions, transition tendency, foreground/background activity |
| Complexity | Edge density, entropy, flat-area fraction, large-region count and occupied ratio |

The analysis stream uses a 96-pixel short side, at most 30 fps and 3,600 frames. Representative samples use a 540-pixel short side. The default budget is 12–30 samples for clips up to 90 seconds, rising to at most 40 for longer clips; merging nearby samples may produce fewer. Spare sample slots include neighboring frames around motion peaks. The contact sheet labels each sample's timestamp and type.

These are lightweight visual proxies, not object tracking, optical flow, OCR or aesthetic judgments. Event detection uses raw RGB differences to retain isoluminant changes; activity statistics subtract a capped tenth-percentile noise floor. Slowly moving pictures and grain can remain ambiguous. Decoder version is recorded; reproducibility assumes the same FFmpeg decoding environment. Variable-rate source frame counts may be estimates.

`--cache-dir DIR` reuses complete evidence/sample bundles keyed by media bytes, analyzer version and analysis configuration. No source video copy is stored. `validate-reference-style --bundle DIR --cache-dir CACHE` additionally stores a validated profile. Missing, incomplete or unsafe bundle artifacts are cache misses. The prompt/request are regenerated from current protocol text on reuse; cached profiles are not silently applied to a new interpretation protocol.
