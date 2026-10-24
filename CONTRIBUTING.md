# Contributing to MotionEngine

Thank you for taking a look. This file covers the development setup, the checks every change must
pass, the rules that keep the engine deterministic, and the two most common kinds of change: a new
component and a new mistake-ledger entry.

MotionEngine is published under the MotionEngine Limited License (see [LICENSE](LICENSE)). By opening
a pull request you agree that your contribution may be distributed under the same terms.

## Development setup

1. Install a stable Rust toolchain (edition 2021; CI and local development use 1.92 or later).
2. Install FFmpeg (with libx264 and AAC) so that `ffmpeg` and `ffprobe` are on `PATH`.
3. Install cargo-nextest: `cargo install cargo-nextest --locked`.
4. Build: `cargo build --release -p motion-cli`.

Optional: `cmake` and `--features asr-local,asr-ctc` for local word timing; `--features gpu` for GPU
post effects. Neither is needed for the default build or tests.

Rendered output goes to `output/`, which is git-ignored. `render` deletes its PNG frames after the MP4
is encoded (`--keep-frames` keeps them). If you write single frames or contact sheets to inspect a
change, delete them afterwards.

## The gate

Every change must pass:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo nextest run --workspace
cargo test --doc --workspace
scripts/cli_smoke.sh
```

`cargo test --workspace` runs the same tests as nextest. The default test run needs no network, keys,
GPU or macOS-only tools; tests that do are `#[ignore]`d or gated by `cfg`. Do not add a test that
reaches the network by default.

Integration tests are grouped into one binary per area (`crates/<crate>/tests/<area>/main.rs`). Add a
module to an existing area rather than a new `tests/*.rs` file; see [docs/TESTING.md](docs/TESTING.md).

Visual changes need a look at rendered frames, not only green tests. A contact sheet from an MP4:

```bash
ffmpeg -i output/coffee_story/coffee_story.mp4 -vf "select='not(mod(n\,48))',scale=480:-2,tile=4x2" -frames:v 1 sheet.png
```

### Generated files

| file | regenerate after | command |
|---|---|---|
| `schema/creative-intent-*.json`, `schema/style-profile-*.json` | doc comments or fields in `intent.rs` / `style.rs` | `MOTION_UPDATE_SCHEMA=1 cargo test -p motion-core --test public_schema` |
| `schema/motion_program.json` | any `MotionProgram` type | `MOTION_UPDATE_SCHEMA=1 cargo test -p motion-program --test program` |
| `golden/` frames and scenes | an intentional visual change | `MOTION_UPDATE_GOLDEN=1 cargo test -p motion-render --test golden` |

Commit regenerated files in the same change, and say in the description why the goldens moved.

## Determinism rules

- Compilation, lowering and timeline evaluation are pure functions of their inputs. No wall-clock
  time, no unseeded randomness, no iteration over unordered maps where the order reaches the output.
- Seeds are explicit fields. Variation is hashed from the seed and stable keys.
- Time is computed from the frame number (`t = frame / fps`), never accumulated.
- The renderer consumes `ResolvedFrame` only. It makes no layout or semantic decisions.
- Only bundled fonts are loaded, so text renders the same on every machine. Optional faces fetched with
  `scripts/fetch_fonts.sh` replace their open fallbacks (`assets/fonts/RESTRICTED.md`); tests and goldens never need them.
- Network calls live only in `motion-voice` and `motion-providers`. `motion-core`, `motion-render`,
  `motion-program` and `motion-bridge` stay offline.
- Generated assets are resolved once into a lockfile; compile, QA and validate read the lockfile and
  never call a provider.

## Public interface

Small models write `CreativeIntent` or a brief, never pixels or timings. Do not add pixel or timing
fields to `intent.rs` or `style.rs`. The public schemas are generated from their doc comments, so the
doc comments are user-facing text. [docs/PUBLIC_INTERFACE.md](docs/PUBLIC_INTERFACE.md) lists what
consumers may rely on; keep [docs/AI_AUTHORING_GUIDE.md](docs/AI_AUTHORING_GUIDE.md) in sync with
recipe behaviour.

## Adding a component

Components live in `crates/motion-program`.

1. Add a variant to `ComponentBody` in `src/program.rs`. Use closed enums for presets and slots; give
   every field a doc comment (it becomes the schema description).
2. Validate it in `src/validate.rs`: required fields, timing fit (enough beats to read the content),
   which verbs it accepts.
3. Lower it in a new module (for example `src/my_component.rs`) and dispatch to it from
   `lower_component` in `src/lower.rs`. Take colours from palette roles (`Ctx::paint` / roles on the
   component's own ground), sizes and durations from the taste tables, and positions from slots. No
   hard-coded colours, no accumulated time.
4. Let layout follow content: solve each step from what is visible (`flip::follow`); never reserve
   space for an element that has not arrived.
5. Regenerate `schema/motion_program.json`, add an example under `examples/v2/`, and add tests: a
   round-trip, a determinism check, and `program qa` passing on the example.
6. Render the example at 16:9 and 9:16 and look at the frames.

## The mistake ledger

`assets/qa/mistakes.json` lists every design defect the engine has learned to catch. Each entry has an
`id`, a `description`, the note from the review that found it, a `detector`, a `fix_policy`, an
optional `repair`, the `checks` area, and a `fixture`.

- Detectors run statically on the program and the lowered scene (`motion-program/src/ledger.rs`,
  `ui_gates.rs`) or on rendered frames (`motion-program/src/qa/design.rs` and the rendered gates in
  `motion-cli`).
- Repairs implement `trait Repair` and are registered in `repair::repairs()`. QA → repair → QA runs at
  most twice. `program render` refuses to render while a gate fails; `make` and the MCP tools render
  and report the failing ids.
- Every entry names a fixture in `assets/qa/fixtures/` that must fail its detector before the fix and
  pass after it. The tests `ledger_fixtures_fail_then_pass` and
  `rendered_fixtures_fail_then_pass` enforce this.

To add a defect you found in a render:

```bash
./target/release/motion-engine program learn --id <snake_case_id> --from output/x/x.mp4@12.5 \
  --note "what is wrong, in one sentence" [--area program|scene|rendered]
```

This appends an entry and a fixture stub. Then write the detector, make the fixture fail, fix the
engine (never the sample), and make the fixture pass.

## Commits and pull requests

- One logical change per commit. Subject line in the imperative, at most about 72 characters, with an
  optional area prefix (`motion-program: settle holds after the last build step`). Explain why in the
  body when it is not obvious.
- Pull requests describe the change, the gate result, and for visual changes a contact sheet or a
  short clip.
- Update [CHANGELOG.md](CHANGELOG.md) under `Unreleased` for user-facing changes.
- Never commit keys, `providers.toml`, `.env` files, rendered output or personal paths.
- Samples use invented brands; do not present a real company as the subject of an ad.
- Report security problems privately (see [SECURITY.md](SECURITY.md)), not in an issue.
