# motion-mcp-eval

Dev-only eval harness for the MCP server (`docs/plans/SPRINT_0_21_MCP_SERVER.md` section 11):
how well does a small local model drive `motion-mcp` through its tools?

It starts **one** `motion-mcp` server (stdio, the rmcp client) for the whole run, hands the
server's tools to a model behind an OpenAI-compatible endpoint, plays every brief in
`briefs.jsonl` as a tool-calling conversation, and writes a JSON file and a markdown table.

It is a separate package on purpose. `motion-mcp` makes no network calls and no network code may
enter that crate; this harness is the only part that talks HTTP, and only to the endpoint you pass.

## Start the local model (LM Studio)

```
lms server start
lms load lfm2.5-2.6b-mlx --identifier lfm-eval -c 16384 -y     # any model; the identifier is the --model below
curl -s localhost:1234/v1/models                              # lfm-eval is listed
```

A 3-4B instruct model with tool calling is the target (plan section 1: success >= 85 % of 20 briefs).
Do not unload models other people are using; load yours under its own identifier.

## Run

```
cargo build --release -p motion-mcp            # the server
cargo build --release -p motion-cli            # the engine: the server refuses to start without it, even for --no-render

# fast: story quality only (every make_video is forwarded as mode "check", the run ends at "checked")
cargo run --release -p motion-mcp-eval -- --endpoint http://localhost:1234/v1 --model lfm-eval --no-render

# full: render, voice and QA, one brief at a time (minutes per brief)
cargo run --release -p motion-mcp-eval -- --endpoint http://localhost:1234/v1 --model lfm-eval

# a few briefs
cargo run --release -p motion-mcp-eval -- --model lfm-eval --no-render --only ai_agent_parts,sci_ocean_zones
```

Options (all but `--model` have defaults):

| flag | default | |
|---|---|---|
| `--endpoint` | `http://localhost:1234/v1` | OpenAI-compatible base URL (`/chat/completions` is appended) |
| `--model` | required | model id as the endpoint knows it (LM Studio: the `--identifier`) |
| `--profile` | `weak` | `weak`, `creator` or `operator`, passed to the server |
| `--briefs` | `crates/motion-mcp/eval/briefs.jsonl` | one brief per line: `id, topic, structure, brief, expect {beats: [min, max], key_terms}` |
| `--only` | all | comma-separated brief ids |
| `--server` | `target/release/motion-mcp` | the server binary |
| `--repo` | the enclosing checkout | passed to the server; point it at a checkout that has `target/release/motion-engine` if yours has none |
| `--jobs` | `<repo>/output/jobs` | the server's job store (check mode writes nothing, a temp dir is fine for `--no-render`) |
| `--no-render` | off | check-only run, see above |
| `--max-turns` | 6 | model requests per brief |
| `--temperature` | 0.2 | the request also sends `seed: 7` |
| `--max-tokens` | 3000 | cap on one model answer, stops a runaway generation |
| `--system` | `generic` | `generic` ("You are a helpful assistant. Use the available tools when they help.") or `none` |
| `--out` | `crates/motion-mcp/eval/results` | results directory |
| `--server-arg` | none | extra server flag, repeatable (`--server-arg=--reel-arg=--offline`) |
| `--date` | today (UTC) | date in the file names |

## What a brief does

The conversation is `[system?, user: the brief]`. Each turn posts `model`, `messages`, `tools`,
`temperature` and `seed` to `/chat/completions` and reads `usage`. For every tool call the model
makes:

- the arguments are parsed; invalid JSON (or a non-object) counts as `bad_args` and the model gets a
  tool message explaining the error (the server is not called);
- otherwise the MCP tool is called and its **text** reply goes back as a `tool` message (what most
  clients show the model);
- `--no-render` rewrites every `make_video` call's `mode` to `check` before forwarding.

The brief ends when a `make_video` / `revise_video` / `get_video` reply has status `done` or `failed`
(`checked` with `--no-render`), when the model answers without tool calls, at `--max-turns`, or when
the endpoint fails. A `running` / `queued` reply lets the model continue (it should call `get_video`);
if it walks away from a render the harness waits for the job itself so the next brief is not blocked
(not counted in the metrics).

Metrics per brief (JSON keys):

- `success`: final status `done` and `qa` pass and brief match; with `--no-render`, `checked` and brief match.
- `tool_calls` (every call the model made, usable or not), `bad_args`, `needs_fix` (fix retries),
  `repeated_calls` (word-for-word repeats of an earlier call), `tool_errors` (calls the MCP layer refused).
- `prompt_tokens`, `completion_tokens` (summed over turns), `first_prompt_tokens`, `wall_s`, `duration_s` (video length), `job`.
- `beats`, `changed` (the server's auto-fix lines), `findings`, `brief_match`.
- `calls`: the **raw tool arguments** of each call as the model wrote them, plus the reply it got.
- `note`: why a brief failed, in one line.

Brief match: the stored story (`<jobs>/<job>/request.json` -> `story`; with `--no-render` the
`story` argument the model sent, since check mode writes no job) has a beat count within
`expect.beats`, and every `expect.key_terms` term appears in the text of the beats' say, show,
number, list, compare, change and layers fields. Matching ignores case and the commas inside numbers
(`10,000` = `10000`).

## Output

`<out>/<YYYY-MM-DD>-<model>-<profile>[-norender].json` (all metrics, raw arguments) and the same
name with `.md` (one row per brief and a summary against the plan targets). Files are rewritten
after every brief, so an interrupted run keeps what it has.

A small local model is noisy: the same brief can succeed in one call in one run and loop on a
fix-it message in the next (`seed: 7` is sent, but LM Studio does not reproduce a run exactly).
Compare runs, not single briefs, and read the raw `arguments` in the JSON to see what the model
actually wrote. `repeated_calls` (a model stuck on a fix-it message) and `finish_reason` / `final_raw` (what the
endpoint sent when the model made no tool call) help with the two usual failure shapes.

Targets (plan section 1, weak profile):

| metric | target | how it is measured |
|---|---|---|
| success | >= 85 % of the briefs | as above |
| tool calls per video | average <= 1.5 | model tool calls per brief |
| tool definitions | <= 1.2k tokens | `prompt_tokens` of a request with the tools minus one without, on the model; also the chars of the MCP tool list |
| tool replies | <= 200 tokens | characters / 4 of every reply the model read; the largest is judged |

## Paid models need the owner's OK

The plan also names Haiku 4.5 (Anthropic) and a Flash-class model (OpenRouter) for the creator and
second weak-tier evals. They cost money, so they run **only with the owner's OK**. This harness
supports OpenAI-compatible endpoints only, sends no authorization header, and never reads
`OPENROUTER_API_KEY` or `ANTHROPIC_API_KEY` (or any other key). A paid run needs a deliberate
change to the harness (and the owner's approval first).

A rendered run makes the server's own voice calls through the engine (free OpenRouter TTS, one
pinned voice); that is the engine's existing behavior and the harness does not touch the key.

## Tests

`cargo test -p motion-mcp-eval` (no network, no server): tool conversion, argument parsing,
brief matching, the conversation loop against scripted model and server, summary maths, file names.
