# Changelog

All notable user-facing changes to MotionEngine. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions before 2.0 are the v1 engine
(`CreativeIntent` → compiler → narrated reels); 2.x adds MotionEngine 2 (`MotionProgram`).

## [Unreleased]

## [2.3.0] - 2026-10-20

### Added
- Generated assets in programs. A program can declare `assets: [{id, prompt, kind: plate|cutout,
  aspect, seed}]` and use any of them as `"gen:<id>"` wherever an image path is accepted (type fills,
  backdrops, photo layers).
- `motion-engine program assets` resolves declared assets through the provider fallback, caches them
  and pins them in `<stem>.assets.lock.json` beside the program. `program render` runs it first;
  `--offline` reads the lockfile only. Compile, QA and validate never call a provider.
- Local MOSS-Image provider (`providers generate moss`): a 0-credit image provider for a self-hosted
  MOSS-Image server, with explicit seeds, plate sizes up to 1024 px and magenta-keyed cutouts.
- `providers generate auto`: tries providers in the policy's `provider_order`, logs why each one was
  skipped, and shows the quote before any spend. `providers show` reports whether MOSS is reachable.
- Example `examples/v2/gen_assets`.
- Public documentation: `CONTRIBUTING.md`, `docs/DESIGN_RULES.md`, `docs/PROVIDERS.md`, a public
  `ARCHITECTURE.md`, and showcase previews.

### Changed
- Refreshed OpenArt tokens are saved after a fallback run.
- Programs are validated before any asset is generated.
- Samples use invented brand names.

## [2.2.0] - 2026-10-20

### Added
- Intentional layering: `Component.over` draws a sketch, shape, photo stack, backdrop or field above
  a named component and exempts that pair from the overlap gate. Type can never be layered over.
- Type styles on `title` and `kinetic_text`: gradient and image fills (text masks), outlines, and
  `case` (`preset`, `as_written`, `upper`). The contrast gate judges the worst gradient stop and the
  rendered median of image fills.
- Plate-less sketches (`plate: false`) and a `size` for sketches.
- `make --aspect square|portrait|landscape`.
- Thousands separators in bento stats and chart callouts, including count-ups.
- Ledger entry `eyebrow_kicker` (kickers and "est." pills on bento and lockups).
- Examples `examples/v2/layered_sketch` and `examples/v2/type_styles`.

### Changed
- Settled holds: after a component's last build step it may stay still for up to two beats (1 s),
  once per component and at most 15 % of beats program-wide.
- A title at `xl` size fills at least 55 % of the frame width in wide slots.
- Near-blank findings name the frame and the components on it.

### Fixed
- `end_beat` on a closing `kinetic_text` no longer blanks its last word.
- `beat_cuts: 0` on a sketch is reported as one clear error.

## [2.1.0] - 2026-10-20

### Added
- `sketch` component: natural-media strokes in the style of p5.brush (freeform, golden spiral, flow
  field, architectural hatch), with hatching and watercolour.
- `bento` metric grids, `photo_stack` (cutout collages in several layouts and presets) and five
  `lockup` presets (spline frame, monolith split, orbital seal, terminal prompt, editorial badge).
- Golden-ratio design tokens and 9:16 portrait layouts.
- `make_video` can edit an existing program: palette, fonts, ending and component patches by id.
- `motion-engine serve`: a local inspector for adjusting fonts, palette and copy.

### Removed
- The experimental 3D crate (`motion-3d`) and the `scene3d` component.

## [2.0.0] - 2026-10-14

### Added
- MotionEngine 2: the `MotionProgram` format (`schema/motion_program.json`), a closed vocabulary of
  components and verbs with validation and deterministic lowering to `MotionScene`.
- Components: backdrop, shape (point-matched morphs), title, kinetic text (poster cuts, glyph drop,
  type-on), chart (line, area, bar, donut, spring), field (halftone, grid, rings, tiles), HUD,
  diagram, code block, device frame with typed UI flows (payment, sign-up, upload, share, settings
  toggle), lockup.
- Beat sync: bundled music analyses; events and components are placed in beats and snapped to
  frames; the final hit lands on a downbeat.
- Camera (push, pan, frame, reset) and transitions (whip, colour flip, impact).
- Palette roles resolved in OKLCH per ground (ink, structure, accent, texture) with contrast targets;
  every component takes its colours from roles.
- Layout follows content: each step is solved from what is visible; nothing reserves room for an
  element that has not arrived.
- The mistake ledger (`assets/qa/mistakes.json`): static and rendered detectors with fixtures,
  automatic repair (at most two passes), and `program learn` to add entries.
- `motion-engine make --brief`: a brief becomes a beat-planned program through named recipes,
  with claim headlines derived from the content, then QA, render and critique, in one JSON line.
- `program validate|compile|qa|render|learn` subcommands.
- MCP profile `v2`: `make_video`, `edit_video`, `preview`, `critique`.
- Asset providers: an MCP client with OAuth, the OpenArt adapter, an allow-list and credit caps,
  a content-addressed cache and a spend log (`providers connect|show|tools|quote|generate`).
- Renderer: path layers with gradients and masks, instanced layers, contact shadows.

## [0.24.0]

### Changed
- v1 looks drop default boxes and hairlines; backdrop families (ink, paper, colour block, light
  gradient) vary per video under `--variety`.
- Reveals follow the voice: slam cuts land on their spoken words; unspoken titles are not slammed.
- Speech QA judges every text layer for `reveal_before_speech`; new `reveal_unspoken` and
  `slam_density` checks.

## [0.23.0]

### Added
- Takes: seeded alternates for layouts, arrivals and camera plans, so the same story can render in
  several distinct versions.
- Mood-based music selection from the story's content, with silence when nothing fits.
- Voice normalisation, a speech-led music envelope, stems, and the `mix` command.

### Fixed
- Counters show their final value shortly after the spoken number and hold it.
- Content appears within 0.5 s of the start and of every beat; short state-change beats compile.
- Unreadable text over print fields and pictures in the variety looks.

## [0.22.0]

### Added
- Stat pairs (two pictures compared with both values), ranked horizontal bars, stat cards in every
  look.
- Story review in MCP (`mode: check`) returns the compiler's story warnings first.

### Changed
- `render` deletes its PNG frames once the MP4 is encoded (`--keep-frames` keeps them).

## [0.21.0]

### Added
- `motion-mcp`: an MCP server with `weak`, `creator` and `operator` profiles, a lite story format,
  lenient parsing with fix-it messages, bring-your-own images, resources and prompts.
- Brand colours per video (`StyleProfile.brand`, `--brand`).

## [0.20.0]

### Added
- Local word timing with whisper.cpp (`--features asr-local`) and CTC refinement (`asr-ctc`);
  `models list|fetch|path` with pinned checksums.
- Speech-aware scene lifecycles: reveals are anchored to the words that name them.
- Compile warnings for weak stories and timing QA (`speech_rate`, `reveal_before_speech`,
  `reveal_late`, `spoken_mismatch`).

## [0.17.0] – [0.19.0]

### Added
- Cinematic look with depth planes, rack focus and camera choreography; sphere gallery for
  collections; relationship stages for compare and contrast beats; environment plates.
- `tilt` and `revolve` motion ops, angled arrivals, continuous camera velocity.
- The `layers` subject: persistent labelled strata across beats.

## [0.14.0] – [0.16.0]

### Added
- Motion ops 2.0: shake, glyph cascade, path morph, echo, pulse, springs; camera shake, roll, dolly,
  orbit and perspective.
- CPU post effects (bloom, chromatic aberration, glitch, rays, blur, grain, vignette) and an optional
  wgpu backend (`--features gpu`, `--gpu`).
- Genres: street, documentary, hype and studio looks.
- `matte` (subject cut-outs with Apple Vision on macOS).

## [0.10.0] – [0.13.0]

### Added
- `motion-engine reel`: one command from a story to a voiced, captioned, scored video.
- `motion-voice`: single-take TTS through OpenRouter (free Fish voice by default) or macOS `say`,
  a content-addressed cache, and key management (`providers set-key|show|test`).
- Karaoke captions, speech-led beat timing, music beds by emotion, a −16 LUFS mix and speech QA.
- Art direction (`--art`), curated palettes and `--variety`; sprite loops; subject-first image
  layouts.

## [0.9.0] - 2026-10-01

### Added
- Emotion typography (a registry of bundled faces per emotion, `fonts list|specimen`).
- `explore`: render several style variants with a contact sheet.
- Responsive canvas (`--canvas`, `--aspect`) and layout QA.

## [0.8.0] - 2026-10-01

### Added
- Sound design: SFX cues, a music bed with light choreography, `music-index`, `plan-audio`.
- A bundled picture library.

## [0.7.0] - 2026-10-01

### Added
- Reference style interpreter: evidence from a reference video (`reference-evidence`), a validated
  `ReferenceStyleProfile`, and `--reference-style` on `compile` and `plan-assets`.
- Visual-language transfer (0.7.1): a reference can bias type-led versus image-led construction.

## [0.6.0] - 2026-09-30

### Added
- Taste fields on `StyleProfile` (`tone`, `polarity`, `temperature`, `temperament`, `density`),
  resolved by the Taste Director into colours, type, backgrounds, pacing and transitions.
- `style-preview` and `compare-styles`.

## [0.5.0]

### Added
- Generated-asset protocol: `asset-prompts`, `ingest-assets`, `validate-assets`; image analysis
  (alpha bounds, safe regions, head estimate) and subject-aware placement of type around pictures.
- Image treatments (monochrome, duotone, paper cut-out, muted documentary).

## [0.4.0] - 2026-09-30

### Added
- Scene lifecycle (enter, settle, read, evolve, anticipate, bridge) and composition grammars.
- Asset planning (`plan-assets`).

## [0.3.0]

### Added
- Layout bindings and motion languages (minimal, kinetic, parallax, sequential, data).
- CreativeIntent v0.2: collections, state changes, derived metrics.

## [0.2.0] - 2026-09-30

### Added
- The public consumer interface: `CreativeIntent` v0.1 and `StyleProfile` v0.1 schemas, the
  compiler to `MotionScene`, the CPU renderer and MP4 export.
