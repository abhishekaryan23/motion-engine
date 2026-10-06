# motion-engine CLI: Consumer Guide

How to turn a CreativeIntent JSON into an MP4 using only the `motion-engine`
command line. You do not need to read Rust source. This guide covers four
commands: `compile`, `validate`, `render`, `inspect` (plus `plan-assets`, `qa`, and the
style tools `style-preview` and `compare-styles`, below).

- What to put in an intent: [AI_AUTHORING_GUIDE.md](AI_AUTHORING_GUIDE.md) and
  the JSON Schemas in [`schema/`](../schema/) (`creative-intent-v0.1.schema.json`,
  `style-profile-v0.1.schema.json`).
- The compiled MotionScene is an **opaque artifact**. Produce it with
  `compile`; pass it to `validate`, `render` and `inspect`. Do not edit it by hand.

## Prerequisites

- Rust toolchain (to build the binary).
- `ffmpeg` on `PATH`, needed only for full renders that produce an MP4.
  `ffprobe` (shipped with ffmpeg) is handy for checking the result.

Build once, from the repository root:

```bash
cargo build --release -p motion-cli
```

The binary is `target/release/motion-engine`. To skip the build step you can
instead run `cargo run --release -p motion-cli -- <args>`. The examples below
use a shell variable for brevity:

```bash
BIN=./target/release/motion-engine
```

**Run every command from the repository root.** `compile` looks up fonts and
textures in the asset library given by `--assets` (default `assets`, resolved
against the current directory). From any other directory it fails with
`asset library 'assets' not found (use --assets)`.

## Commands

All commands exit with status 0 on success and non-zero on any error
(1 for runtime errors, 2 for bad command-line usage).

### compile

```text
motion-engine compile <INTENT> [--style <STYLE>] --output <OUT> [--assets <DIR>]
```

- `<INTENT>`: a CreativeIntent JSON file.
- `--style <STYLE>`: optional StyleProfile JSON. Omit it to use the built-in defaults.
- `-o`, `--output <OUT>`: required; the MotionScene JSON to write. Missing parent
  directories are created.
- `--assets <DIR>`: asset library root; default `assets`.
- `--asset-manifest <FILE>`: optional. Delivered images for the requests `plan-assets` produced
  (AssetManifest JSON, PNG paths relative to the manifest file). Without it, compositions that
  could use an image are drawn with typography and procedural shapes.

The scene is validated before it is written; if validation fails nothing is written.

```bash
$BIN compile examples/public/minimal-emphasize.intent.json --style examples/public/minimal.style.json -o output/demo/train.motion.json
```

Success prints `compiled <n> beat(s) -> <path> (<s> scenes, <k> shared, <secs>s)`:

```text
compiled 1 beat(s) -> output/demo/train.motion.json (2 scenes, 0 shared, 5.20s)
```

**Keep the scene file where `compile` wrote it.** It stores the path to the asset
library relative to its own location, so moving or copying it elsewhere makes
`validate`/`render` fail with `missing asset file`. Re-run `compile` with the new
`--output` instead.

### plan-assets

```text
motion-engine plan-assets <INTENT> [--style <STYLE>] [-o <PLAN>] [--assets <DIR>]
```

Prints (or writes) an AssetPlan: per beat, whether an image would help (`none`, `procedural`,
`svg`, `user_asset`, `generated_image`), why, and semantic image requests (role, subject,
presentation — never coordinates or timing). It calls no image generator. Most beats need
no image.

### qa

```text
motion-engine qa <SCENE> [--step N] [--json]
```

Pacing diagnostic for a compiled scene: structural motion per sampled frame, and per beat the
activity of each lifecycle phase, evolve events and warnings such as dead holds. Informational.

```text
motion-engine qa <SCENE> [--step N] [--json] [--reference-style <PROFILE>]
```

With `--reference-style`, `qa` also prints a **visual language** section (JSON: a `visual` key). Per beat scene it reports `TYPE` or `VISUAL`, the figure share (`figure / (figure + text)`), text and figure areas, and the counts of images, svgs, polylines and figures, measured structurally on the visible frame at mid READ-to-ANTICIPATE (no OCR). Scenes it cannot measure are listed as `skipped`. It then prints the `pure_type_ratio` (share of type-dominant beats) and a verdict against the reference's visual language: a WARNING "visual-language transfer likely failed" when the reference is visual and `pure_type_ratio > 0.6`, or "type-led reference but the output is visual-heavy" when it is type-led and `pure_type_ratio < 0.4`. A reference without a visual-language preference gives no verdict, and type-dominant output is never a warning by itself. Without `--reference-style` the visual section reports measurements only. See [VISUAL_LANGUAGE.md](VISUAL_LANGUAGE.md).

### validate

```text
motion-engine validate <SCENE>
```

Checks that the scene parses and is internally consistent (including that the
asset files it references exist relative to the scene file).

```bash
$BIN validate output/demo/train.motion.json
```

On success it prints one line and exits 0:

```text
ok: 2 scene(s), 0 shared element(s), 5.20s, 156 frames
```

On failure it exits 1 and prints `Error: <scene path>: N validation error(s):`
followed by one `- <location>: <problem>` line per problem, or a parse error such
as `missing field \`version\``.

### render

```text
motion-engine render <SCENE> [--out-dir <DIR>] [--frame <N>] [--no-video] [--keep-frames]
```

- Default output directory: `<directory of the scene file>/<intent title>/`
  (for `output/demo/train.motion.json` with title `train_example`:
  `output/demo/train_example/`). Override with `--out-dir`. Directories are created.
- Full render: writes `frame_000000.png`, `frame_000001.png`, ..., encodes
  `<intent title>.mp4` (H.264, yuv420p, 30 fps) in the same directory, then
  deletes the PNG frames (1-4 MB each). `--keep-frames` keeps them. Canvas size
  comes from the intent's `format`: `vertical` 1080x1920, `square` 1080x1080,
  `landscape` 1920x1080.
- `--no-video`: write the PNG frames only; ffmpeg is not run (and not required).
- `--frame <N>`: render that single PNG only, no video. Frame numbers start at 0;
  the frame count is duration x 30 (`validate` prints it, e.g. 5.20s = 156 frames,
  so valid indices are 0..155).
- **A full render (with or without `--no-video`) first deletes every
  `frame_*.png` in the output directory.** Other files there are left alone,
  including an older `.mp4`. Single-frame renders delete nothing.

```bash
$BIN render output/demo/train.motion.json
```

```text
rendered 156 frame(s) to output/demo/train_example in 7.6s
encoded output/demo/train_example/train_example.mp4
```

```bash
$BIN render output/demo/train.motion.json --frame 30 --out-dir output/demo/preview
```

```bash
$BIN render output/demo/train.motion.json --no-video --out-dir output/demo/frames_only
```

### inspect

```text
motion-engine inspect <SCENE> --frame <N>
```

Prints the resolved state of frame N (which scenes are active, per-layer
transforms, opacity, and so on) as pretty JSON on stdout. `--frame` is required.
This is a debugging aid: the output format is **not a stable contract**, so do
not build tooling that depends on its fields.

```bash
$BIN inspect output/demo/train.motion.json --frame 30
```

The output begins:

```text
{
  "frame": 30,
  "time_seconds": 1.0,
  "width": 1080,
  "height": 1920,
  "background": "#ECE3D2",
  "active_scenes": [
```

### style-preview

```text
motion-engine style-preview <STYLE> [-o <PNG>] [--assets <DIR>] [--json]
```

Shows what a StyleProfile resolves to, without compiling a story. Renders one
frame (palette swatches, typography specimen, backdrop, material) and prints a
description of the resolved design system, including the temporal character
(motion, transitions, density, rhythm, scale contrast, layer activity), which is
described in text, not animated. See [TASTE_DIRECTOR.md](TASTE_DIRECTOR.md).

- `<STYLE>`: a StyleProfile JSON file (required).
- `-o`, `--output <PNG>`: the PNG to write; default `output/style_preview.png`. Missing
  parent directories are created.
- `--assets <DIR>`: asset library root (fonts); default `assets`.
- `--json`: print the internal ResolvedStyleProfile as pretty JSON instead of the
  description. Not a stable contract.

Default output (stdout), for `examples/taste/dark_technical.style.json`:

```text
Tone: technical
Palette: dark_cool (dark, cool, steady)
Background: technical_grid
Typography: condensed_mono
Material: screen
Image treatment: monochrome
Motion temperament: precise (amplitude medium, settle crisp, overshoot low, stagger structured, camera controlled)
Transition character: geometric (wipe frequent, exit slide, overlap standard)
Density: balanced (annotation structured)
Composition rhythm: progressive
Scale contrast: moderate
Layer activity: fg balanced, mid structured, bg structured
Preview: output/style_preview.png
```

Exit codes: 0 on success; 1 for runtime errors (style file missing or not a valid
StyleProfile, e.g. an unknown field or enum value; asset library not found; a
render failure); 2 for bad command-line usage.

```bash
$BIN style-preview examples/taste/dark_technical.style.json -o output/demo/technical_preview.png
```

### compare-styles

```text
motion-engine compare-styles <A> <B> [--json]
```

Resolves two StyleProfile files and compares their design systems dimension by
dimension: palette, polarity, background, typography, material, image treatment,
motion character, transition style, density, composition rhythm, scale contrast
(11 dimensions). Accent color is deliberately not compared: changing only
`accent_role` is not a new style. Use it to check that styles for different
stories are genuinely different. There is no quality score.

```text
Palette              DIFFERENT
Polarity             DIFFERENT
...
Scale contrast       DIFFERENT
11/11 dimensions differ
```

With `--json` it prints
`{"a": <path>, "b": <path>, "differing": <n>, "total": 11, "dimensions": [{"dimension": "palette", "different": true}, ...]}`.

Exit codes: 0 whenever both files were read and compared, **including when the
styles differ or are identical** (the difference is the result, not a failure);
1 if either file is missing or not a valid StyleProfile; 2 for bad usage.

```bash
$BIN compare-styles examples/taste/dark_technical.style.json examples/taste/playful_print.style.json
```

## Complete workflow

Copy-paste from the repository root, one step at a time; stop if any step exits non-zero.

```bash
cargo build --release -p motion-cli
```

```bash
BIN=./target/release/motion-engine
```

```bash
$BIN compile examples/public/three-beat-story.intent.json --style examples/public/minimal.style.json -o output/demo/bridge.motion.json
```

```bash
$BIN validate output/demo/bridge.motion.json
```

```bash
$BIN render output/demo/bridge.motion.json
```

```bash
ffprobe -v error -select_streams v:0 -show_entries stream=codec_name,pix_fmt,width,height,r_frame_rate,nb_frames -show_entries format=duration -of default=nw=1 output/demo/bridge_example/bridge_example.mp4
```

Expected for the vertical `three-beat-story` example (values come from the scene):

```text
codec_name=h264
width=1080
height=1920
pix_fmt=yuv420p
r_frame_rate=30/1
nb_frames=420
duration=14.000000
```

`nb_frames` should equal the frame count printed by `validate`. To look at a
single moment without rendering everything, use `render --frame N` and open the PNG.

## Errors

Invalid intents fail at `compile` (exit 1) with a message that names the problem
and its line and column. An unknown field:

```text
Error: parsing intent output/cli_guide_check/bad-field.intent.json

Caused by:
    unknown field `bogus`, expected one of `version`, `title`, `format`, `beats` at line 21 column 9
```

A bad enum value (here `"energy": "frantic"`):

```text
Error: parsing intent output/cli_guide_check/bad-enum.intent.json

Caused by:
    unknown variant `frantic`, expected one of `calm`, `building`, `impact` at line 18 column 25
```

Other common failures, all non-zero exit:

| Situation | Message (abridged) | Exit |
|---|---|---|
| Wrong working directory / bad `--assets` | `asset library 'assets' not found (use --assets)` | 1 |
| Input file missing | `reading <path>` / `No such file or directory` | 1 |
| Scene moved away from its asset library | `validate`: `N validation error(s): - assets[...].path: missing asset file '...'` | 1 |
| Missing required flag (`-o`, `inspect --frame`) or extra argument | clap usage message | 2 |
| `ffmpeg` not installed (full render) | `could not run ffmpeg (is it installed?)`; the PNG frames are already written | 1 |

Caveats:

- `--frame` (in `render` and `inspect`) is **not range-checked**. An index past the
  last frame exits 0: `inspect` prints an empty frame (`"active_scenes": []`,
  `"layers": []`) and `render` writes a PNG with only the background. Check the
  frame count from `validate` yourself.
- Trust the exit status, not the absence of output.
# Reference-style workflow (0.7)

Analyze a local reference video, then pass the resulting files to an external multimodal model:

```sh
motion-engine reference-evidence reference.mp4 --out-dir output/reference
```

The command writes `evidence.json`, `samples/`, `contact-sheet.png`, `interpreter-request.json`, and `interpreter-prompt.md`. It does not invoke a model. Save the model's JSON response as a profile and validate it against the same bundle:

```sh
motion-engine validate-reference-style output/reference-style.json --bundle output/reference
motion-engine resolve-style story.intent.json --reference-style output/reference-style.json
motion-engine compile story.intent.json --style style.json --reference-style output/reference-style.json --output output/story.motion.json
motion-engine plan-assets story.intent.json --style style.json --reference-style output/reference-style.json --output output/assets.json
```

Validation checks the profile contract (v0.2, or the legacy v0.1; v0.2 adds the optional `visual_language` table, see [VISUAL_LANGUAGE.md](VISUAL_LANGUAGE.md)) and, when supplied, its fingerprint against the bundle. If invalid, the command can write a repair-request file; an external orchestrator decides whether to call its model once with that request. The CLI never invokes a model. `resolve-style` prints resolved choices and coverage diagnostics (`MATCH`, `PARTIAL`, `OVERRIDDEN`, `UNSUPPORTED`, `UNKNOWN`); these are not quality scores. See the three reference docs for evidence, protocol, and profile details.

Use `--json` for machine-readable evidence, validation or resolution output. Explicit fields in `--style` override applicable reference dimensions; omit it or use an all-auto style to let the reference supply them. `--cache-dir output/reference-cache` is available on evidence generation and validation (validation also needs `--bundle`). A failed response can be sent for one repair using `--repair-request output/repair.json`; validate the repaired response without requesting another repair. No command automatically loops or calls a model.
