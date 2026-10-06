# MotionEngine over MCP (`motion-mcp`, Sprint 0.21)

`motion-mcp` exposes MotionEngine as MCP tools over stdio, so any model can make a finished,
narrated motion-graphics video. A small model writes one short story object; the engine designs,
voices, animates, renders and checks the video, and answers in one short line.

Plan and rationale: `docs/plans/SPRINT_0_21_MCP_SERVER.md`. Decisions 123–127 in `DECISIONS.md`.

## Set up

```
cargo build --release -p motion-cli --features asr-local,asr-ctc   # the engine (0.20 word timing)
cargo build --release -p motion-mcp                                # the MCP server
```

Register it with your client. For Claude Code, copy `docs/mcp/claude-code.mcp.json` to `.mcp.json`
in your project (or merge it into `~/.claude.json`) and replace `/ABSOLUTE/PATH/TO/MotionEngine`.
Any MCP client works the same way: start `motion-mcp --profile <weak|creator|operator> --repo
<MotionEngine checkout>` as a stdio server.

Check it without a model client:

```
python3 scripts/mcp_call.py --list                        # tools and their definition sizes
python3 scripts/mcp_call.py make_video '{"story": {...}}' # one call, progress on stderr
```

The voice uses the free OpenRouter TTS (Fish S2.1 Pro, pinned male narrator). The key is read by
the engine exactly as `motion-engine providers` stores it; the MCP server never reads, logs or
returns it, and the server itself makes no network calls.

## Profiles: control grows with the model, the contract does not

| | weak (default) | creator | operator |
|---|---|---|---|
| For | sub-4B up to Haiku-class | Flash-class multimodal | Opus-class or a human |
| Tools | `make_video`, `revise_video`, `get_video`, `find_assets` | + `view_frames`, `explore_styles`, `list_options`, `plan_assets` | + `render_frame`, `inspect_frame`, `qa_report`, `ingest_assets`, `matte`, `music_index`, `sfx_index`, `reference_evidence` (Phase 2), `apply_scene_patch` only with `--allow-scene-edits` |
| Tool definitions | 3.6k chars (budget 4.8k ≈ 1.2k tokens) | 9.4k chars (budget 12k) | 11.9k chars (budget 24k) |
| Story | lite story (or a full CreativeIntent) | full CreativeIntent v0.2 or lite | same |
| Style | one tone word | tone word or a full StyleProfile | same |
| Options | none | look, asset families, music bed, captions, aspect, variety | + paid voice opt-in, `force`, keep frames |

The same tool names exist in every profile; each profile gets its own input schemas, generated in
`crates/motion-mcp/src/schema.rs` (enum lists come from motion-core's own schemas). No profile can
author pixels, coordinates or timings: layout, focus, timing and QA hold for every model. Budgets
and the "weak never sees creator fields" rule are tested (`crates/motion-mcp/tests/tool_budget.rs`).

## The lite story

```json
{ "title": "compound interest",
  "beats": [
    { "say": "Put one thousand dollars away today, and then leave it alone for thirty years.",
      "show": "Leave it alone", "picture": "piggy_bank" },
    { "say": "At seven percent a year, the interest starts earning interest of its own.",
      "number": "7%", "meaning": "every year" },
    { "say": "Time does the heavy lifting: the last ten years add more than the first twenty.",
      "compare": { "a": "first 20 years", "b": "last 10 years", "how": "grow" } },
    { "say": "So that one thousand dollars quietly grows into about seven thousand six hundred.",
      "show": "Patience pays", "number": "$7,600", "meaning": "after 30 years" } ] }
```

Every beat has `say` (what the narrator says, 8–30 words; all beats are read as one continuous take)
and at most one structure: `picture` (+ `picture2`), `number` (+ `meaning`), `list` (3–6),
`compare {a, b, how}`, `change {what, from, to}` or `layers {names, focus}`. `show` is the short
on-screen title (≤ 6 words); without it the title is the `keyword` or the first words of `say`.
The mapping to CreativeIntent v0.2 is deterministic (`crates/motion-mcp/src/lite.rs`, table in
its module docs).

## Tools and replies

- `make_video(story, style?, format?, assets?, mode?, strict?)` — `mode: auto` (default) checks,
  auto-fixes, renders and replies when the video is done; `check` replies in seconds with the plan
  (structure per beat, pictures found or shown as text, estimated duration) and renders nothing.
  `strict` turns auto-fixes into `needs_fix` answers.
- `revise_video(job, changes, style?)` — edit, add or remove beats, or change the style. A new job;
  narration that did not change reuses the recorded voice (the voice cache is keyed by the text).
- `get_video(job, wait_s)` — status, waiting up to `wait_s` (≤ 300) seconds. One call waits;
  never poll.
- `find_assets(words, k)` — which library pictures exist for these words, in one call.

Replies are one short text line plus the same as structured content, ≤ 200 tokens:

```
done · job j_9c0e07d4f5 · 18.9 s · qa pass · video output/jobs/j_9c0e07d4f5/video.mp4
next: Done. Call revise_video with changes if needed.
```

`video` and `preview` also come back as resource links (`file://` URIs). Problems come back as
fix-it lines that name the beat, the field and the values that would work:
`beat 2 picture: no picture for "berry_bush" — use "strawberry" or "blueberry", or leave it (shown as text)`.

## Where the videos are

Every job lives in `output/jobs/<job>/` (gitignored):

| file | what |
|---|---|
| `video.mp4` | the finished video (1080 × 1920 for vertical) |
| `preview_720p.mp4` | a 720p copy (≤ 30 MB) to share or send |
| `request.json`, `intent.json`, `style.json` | the normalised request and what the engine compiled |
| `status.json` | state, stage, QA verdict, findings |
| `log.txt` | the full engine output (never returned to the model) |
| `reel/` | scene, speech map, audio plan |

A job id is a hash of the normalised request, the engine version and the voice model: the same
request returns the same job instantly, without rendering again. The newest 50 jobs (≤ 20 GB) are
kept. Renders run one at a time; other calls queue.

## Your own images

Images never travel through the model. Put them in a folder under `assets/inbox/` (gitignored) and
pass references:

- `make_video(story, assets: "my_trip")` with `beat1.jpg`, `beat2_person.png`, `beat3_object.png`,
  `background.jpg` (the environment for beats without their own image);
- or per beat: `"picture": "file:my_trip/beach.jpg"`, or `{ "file": "my_trip/beach.jpg", "role": "place" }`;
- or a ready `my_trip/manifest.json` (AssetManifest).

Roles: person → `hero_subject` (cut out with Apple Vision when opaque), object → `hero_object`,
place → `environment`. Without a role, Apple Vision guesses on macOS. Absolute paths, `..`,
symlinks out of the folder, non-images and files over 40 MB or 8000 px are refused with a fix-it
line. `check` mode lists each image: role, size, person detected, cutout made.

## Guidance for models

Guidance lives in MCP resources and prompts, not in tool descriptions, so a small model pays no
tokens for it unless its client loads it.

| resource | what | profiles |
|---|---|---|
| `motionengine://guide/lite-story` | the beat fields, one structure per beat, the title rule, 3 examples (< 2,400 chars, about 600 tokens) | all |
| `motionengine://guide/full` | `docs/AI_AUTHORING_GUIDE.md`, read from the repository when asked | all |
| `motionengine://examples/{name}` | a ready `make_video` call: `number-reveal`, `list`, `compare`, `layers` | all |
| `motionengine://examples/intent-{name}` | the full-intent files of `examples/public/` | creator, operator |
| `motionengine://assets{?query}` | the same lookup as `find_assets`, e.g. `motionengine://assets?query=berry,rocket` | all |
| `motionengine://styles` | the tone words, each with its best use | all |
| `motionengine://guide/intent` | the full CreativeIntent v0.2 beat and the StyleProfile dials (< 8,000 chars) | creator, operator |
| `motionengine://options` | looks, asset families, music beds and tones (the `list_options` data) | creator, operator |

Prompts, each returning one user message with a story skeleton for the topic and the five key rules
(one structure per beat; `say` 8-30 words as one continuous script; `show` 6 words or fewer;
concrete picture nouns; call `make_video` once, then `revise_video`):

- `make_explainer(topic, length_s?)`: beats = `length_s` / 6, between 2 and 12 (default 30 s, 5 beats);
- `make_reel(topic, tone?)`: 4 punchy beats, tone `hype` unless another tone word is given;
- creator and operator: `direct_video(topic, look?, length_s?)`: a full-intent skeleton and the review
  routine: `make_video` with `mode: check`, render, `view_frames` (one contact sheet, one frame per
  beat at its reading moment), then `revise_video`. On the sheet look for: the main thing in focus
  and the largest, no text over faces, no empty or near-empty frames, no two beats that look the
  same, short titles.

The voice is the free pinned narrator (`--tts-model auto`). For tests and offline demos, weak and
creator also accept `--tts-model say` (the macOS offline voice); only the operator profile may name
a paid model. The end-to-end render test is
`cargo test -p motion-mcp --test e2e_render -- --ignored --nocapture` (needs the release engine).

## Brand colours

Every profile accepts `brand` on `make_video`, `revise_video` and `explore_styles`:
`{"primary": "#0A84FF", "secondary": "#FFD60A", "background": "#101828", "text": "#FFFFFF"}`
(all optional; a list or a comma string in that order also works, and `#` may be left out). The
look stays, the colours become the brand's, and text always stays readable (a colour that would
not read is replaced and reported in `findings`). A revision keeps the job's brand unless it
sends a new one. From the command line: `motion-engine reel story.json --brand "#0A84FF,#FFD60A"`
(or `--brand "primary=#0A84FF,background=#101828"`), or `brand` in the StyleProfile JSON.

## Countdowns and lists in order

Beats carry no number unless the story counts. Open each item's `say` with its rank — "Number
three: …", "Step 1: …", "First, …" — and those beats show `03 — label` (at least three ranked
beats, running by one, up from 1 or down). Intro and outro beats stay unnumbered.

## Status (Phase 1)

Phase 1 runs the existing `motion-engine reel` as a subprocess (progress per pipeline stage).
Phase 2 moves the pipeline into a library with frame-level progress and MCP Tasks, adds hybrid
picture/style retrieval, the creator's ≤ 20 s draft contact sheet and the operator tools. See
`BUILD_STATUS.md` for what is done.
