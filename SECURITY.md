# Security

## Reporting a problem

Report a suspected vulnerability privately through GitHub:
[open a security advisory](https://github.com/abhishekaryan23/motion-engine/security/advisories/new).
Do not put details in a public issue, a pull request or a commit.

## Secrets

MotionEngine can use two kinds of secret, both optional. Compiling and rendering never need either.

- An **OpenRouter API key** for the narration voice in v1 reels (and the paid opt-ins, such as
  speech recognition).
- **Asset-provider tokens** for generation platforms such as OpenArt, created by
  `motion-engine providers connect` (OAuth sign-in or a token read from stdin).

**Where secrets are read from:**

1. The `OPENROUTER_API_KEY` environment variable, if it is non-empty.
2. Files in the config directory: `$MOTION_CONFIG_DIR`, else `$XDG_CONFIG_HOME/motionengine`, else
   `~/.config/motionengine`. `providers.toml` may hold `[openrouter] api_key = "…"` or a top-level
   `OPENROUTER_API_KEY = "…"`; provider tokens live in `connections.json`.

**How they are protected:**

- `motion-engine providers set-key openrouter` and `providers connect --token-stdin` read secrets
  from stdin, never from command-line arguments, so they stay out of shell history and process
  listings.
- Secret files are written with mode 0600 inside a 0700 directory. If a key file is readable by
  others, the tool prints a `chmod 600` warning; it never changes permissions on its own.
- Secrets are never printed or logged. `providers show` prints a masked key (`sk-or-…` plus at most
  the last four characters) and connection names only.
- The OpenRouter key travels only in the `Authorization: Bearer` header of HTTPS requests to
  `openrouter.ai`. Provider tokens travel only to the platform they were issued for.
- Provider error messages are logged with every link removed, so account and key-management URLs
  never reach logs.
- Tests use fake keys only, such as `sk-or-v1-0123456789abcdefQRST`.

Never commit a key, a `providers.toml`, a `connections.json` or an `.env` file.

## Network boundary

- Only `motion-voice` (speech synthesis and the optional remote recognition) and `motion-providers`
  (asset generation) contain network code.
- `motion-core`, `motion-render`, `motion-program` and `motion-bridge` make no network calls.
  Compiling, QA and rendering are offline and deterministic.
- `motion-mcp` makes no network calls itself; it runs the engine.
- `reel --offline` refuses every provider call and uses only cached voice and recognition results;
  `program render --offline` uses only the generated-asset lockfile.
- Asset generation spends credits only within the caps of `providers.policy.json`, after printing a
  quote. See [docs/PROVIDERS.md](docs/PROVIDERS.md).
- **What leaves the machine:** narration text (for speech synthesis); with `--align asr`, the
  synthesized voice audio; for asset generation, the prompt and settings sent to the chosen
  provider. Intents, programs, frames and videos stay local. A local MOSS-Image server receives
  prompts over the URL you configure.

## Local data

- The voice and recognition caches live in `~/.cache/motionengine/voice/` (override with
  `MOTION_VOICE_CACHE`); generated assets in `~/.cache/motionengine/assets/` (override with
  `MOTION_ASSET_CACHE`). Entries are named by content hashes and never contain secrets.
- Renders go to `output/`, which is git-ignored, as are generated-asset lockfiles and local test media.

## Untrusted input

- Intents, styles and programs are parsed into typed structures that reject unknown fields, then
  validated.
- Library pictures are named by word, and the lookup accepts only `[A-Za-z0-9_-]` names, so an
  intent cannot point outside the asset library. Generated-asset ids accept lowercase letters,
  digits and `_`.
- FFmpeg, ffprobe, macOS `say` and the keying scripts run as direct processes with argument lists,
  never through a shell.

## Dependencies

Check dependencies against the RustSec advisory database with:

```bash
cargo install cargo-audit
cargo audit
```

The last recorded run (2026-10-02, 245 crates) found no vulnerabilities and one informational
warning: `ttf-parser` is unmaintained (RUSTSEC-2026-0192), pulled in through `cosmic-text → fontdb`.
The engine loads only bundled fonts and fonts the operator chooses.
