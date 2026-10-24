# Asset providers

MotionEngine renders from bundled assets by default and never needs a network connection to compile
or render. The provider layer (`crates/motion-providers`) is an optional way to generate images and
short videos, under a spending policy you control, and to pin the results so that later builds are
reproducible and offline.

## Providers

| provider | kind | cost | how it is reached |
|---|---|---|---|
| `openart` | images and videos from the models the platform hosts | credits | the platform's MCP server over HTTPS, with OAuth sign-in |
| any MCP URL | whatever the server's tools offer | depends | `providers connect <https URL>`; tools are discovered with `tools/list` |
| `moss` | images (plates and cutouts) from a self-hosted MOSS-Image server | 0 credits | local HTTP, default `http://127.0.0.1:7860` |

The OpenArt adapter maps generation verbs onto the tools the server lists, so new models appear
without engine changes, but only models on the allow-list (below) can be used.

### Connecting OpenArt (or another MCP platform)

```bash
motion-engine providers connect openart                 # browser sign-in (OAuth 2.1 with PKCE)
printf %s "$TOKEN" | motion-engine providers connect openart --token-stdin
motion-engine providers show                            # connections, balance, month spend, allow-list
motion-engine providers tools openart [--json]          # the tools the server exposes
```

Connections are stored in `connections.json` in the config directory with mode 0600. Tokens are
refreshed automatically and are never printed or logged.

### Running a local MOSS-Image server

The `moss` provider talks to a server that implements the asynchronous job API of
stable-diffusion.cpp's `sd-server` (`POST /sdcpp/v1/img_gen`, then polling the job) and serves the
MOSS-Image-SFT model with its 4-step turbo LoRA (`moss_4step_f16.safetensors`). MotionEngine never
starts the server; start it yourself before generating.

Point MotionEngine at it in `providers.toml` in the config directory (the default is shown):

```toml
[moss]
url = "http://127.0.0.1:7860"
```

Behaviour:

- Seeds are always explicit; the same prompt, seed and size give the same image.
- Plate sizes are multiples of 16 with the long side at most 1024 px: 768×432 (16:9), 432×768
  (9:16), 576×576 (1:1).
- Connecting times out after 3 s, and the provider is then reported as unavailable so the fallback
  moves on. A running job may take up to 180 s; the whole wait, including time queued behind other
  clients, at most 600 s. A typical plate takes 30–45 s.
- `providers show` reports whether the server is reachable.

## Cutouts

A cutout is a single object generated on flat magenta (`#FF00FF`) and keyed to a transparent PNG
with `scripts/key_cutout.py` and `scripts/despill_magenta.py` (Python 3 with numpy, Pillow and
SciPy). The result is rejected if magenta remains. Set `MOTION_SCRIPTS_DIR` when the engine runs
outside the repository.

## The policy file

`providers.policy.json` in the config directory decides what may be generated and how much may be
spent. It is created with defaults on first use:

```json
{
  "allow": [
    { "kind": "image", "model": "gpt-image-2.5-flare", "quality": "low" },
    { "kind": "image", "model": "gpt-image-2.5-sunburst", "quality": "low" },
    { "kind": "image", "model": "moss-image-sft" },
    { "kind": "video", "model": "fal-h3-max" },
    { "kind": "video", "model": "pixverseV6" },
    { "kind": "video", "model": "pixverse-c1" }
  ],
  "max_per_request": 40.0,
  "per_video_cap": 150.0,
  "monthly_cap": 1500.0,
  "provider_order": ["openart", "higgsfield", "moss"]
}
```

| field | meaning |
|---|---|
| `allow` | the only models that may be used. `model` is matched ignoring case and punctuation (`gpt-image-2.5-flare` matches `GPT Image 2.5 Flare`); `quality`, when set, must match too |
| `max_per_request` | credits a single request may cost |
| `per_video_cap` | credits one video (`--video <name>`) may spend in total |
| `monthly_cap` | credits per calendar month (UTC), all videos together |
| `provider_order` | the order `auto` tries providers in; names that are not configured are skipped |

Every paid request is quoted first. The quote is printed before anything is spent, and the request
is refused if it would break any cap. Each spend is appended to `spend.jsonl` beside the policy.

## Cache

Results are cached by a SHA-256 of provider, model, prompt and settings in
`~/.cache/motionengine/assets` (override with `MOTION_ASSET_CACHE`). A repeated request is a cache hit
and costs nothing. Cached files are never replaced silently.

## Generating from the command line

```bash
motion-engine providers quote openart --model gpt-image-2.5-flare --prompt "a brass compass on slate"
motion-engine providers generate openart --model gpt-image-2.5-flare --prompt "…" --aspect 1:1 --video demo
motion-engine providers generate moss --prompt "…" --seed 7 [--cutout] [--aspect 16:9] [--out plate.png]
motion-engine providers generate auto --prompt "…" --seed 7      # tries provider_order; may spend credits
```

`auto` walks `provider_order` and prints, for each provider it skips, why (not connected, not
reachable, no allowed image model, or a quote over a cap).

## Generated assets in programs

A program declares the images it needs and refers to them by id:

```json
"assets": [
  { "id": "sea_spray", "prompt": "macro photograph of ocean spray over a deep teal wave", "kind": "plate", "seed": 7 },
  { "id": "brass_key", "prompt": "a single antique brass key", "kind": "cutout", "aspect": "1:1", "seed": 3 }
],
"components": [
  { "id": "word", "kind": "title", "text": "Spray", "size": "xl",
    "style": { "fill": { "kind": "image", "image": "gen:sea_spray" } } }
]
```

- `id` uses lowercase letters, digits and `_`; `kind` is `plate` or `cutout`; `aspect` and `seed` are
  optional and explicit.
- `motion-engine program assets <program>` resolves every declared asset through the fallback and
  writes `<stem>.assets.lock.json` beside the program. `program render` runs it first unless
  `--offline` is given.
- Compile, QA and validate only read the lockfile. They never call a provider, and the output is
  byte-identical for the same lockfile.

### Lockfile

```json
{
  "version": 1,
  "assets": {
    "sea_spray": {
      "provider": "moss",
      "model": "moss-image-sft",
      "seed": 7,
      "prompt_hash": "<sha256 of prompt, cutout flag and aspect>",
      "sha256": "<sha256 of the image file>",
      "path": "<absolute path in the asset cache>"
    }
  }
}
```

An entry is reused while its file exists with the recorded hash and the prompt hash is unchanged;
changing the prompt, kind or aspect regenerates that asset only. Lockfiles hold local cache paths, so
they are git-ignored; commit the program, not the lockfile.

## Configuration directory

`$MOTION_CONFIG_DIR`, else `$XDG_CONFIG_HOME/motionengine`, else `~/.config/motionengine`. It holds
`providers.toml` (the OpenRouter voice key and the MOSS URL), `connections.json`,
`providers.policy.json` and `spend.jsonl`. Files with secrets are written with mode 0600. Nothing in
it belongs in the repository; see [SECURITY.md](../SECURITY.md).
