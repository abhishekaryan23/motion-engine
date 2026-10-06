# Security

## Reporting a problem

This repository is public. Report a suspected vulnerability directly to the
maintainer, the repository maintainers via GitHub Private Vulnerability Reporting. Do not put
details in an issue or a commit.

## Secrets

MotionEngine uses one secret: an OpenRouter API key. It enables the free
voice (and the paid opt-ins, such as speech recognition). Compiling and
rendering never need it.

**Where the key is read from, in order:**

1. The `OPENROUTER_API_KEY` environment variable, if it is non-empty.
2. `providers.toml` in `$MOTION_CONFIG_DIR`, else
   `$XDG_CONFIG_HOME/motionengine`, else `~/.config/motionengine`. The file
   may hold either `[openrouter] api_key = "…"` or a top-level
   `OPENROUTER_API_KEY = "…"`.

**How the key is protected:**

- `motion-engine providers set-key openrouter` reads the key from stdin, never
  from command-line arguments, so it stays out of shell history and process
  listings.
- The key file is written with mode 0600 inside a 0700 directory. If the file
  is readable by others, the tool prints a `chmod 600` warning. It never
  changes permissions on its own.
- Keys are never printed or logged. `providers show` prints a masked form:
  `sk-or-…` plus at most the last four characters.
- The key travels only in the `Authorization: Bearer` header of HTTPS requests
  to `openrouter.ai`.
- Provider error messages are logged with every link removed (`scrub_urls`),
  so account and key-management URLs never reach logs.
- Tests use fake keys only, such as `sk-or-v1-0123456789abcdefQRST`.

Never commit a key, a `providers.toml` or an `.env` file.

## Network boundary

- `motion-voice` is the only crate with network code. It calls exactly two
  OpenRouter endpoints: `/api/v1/audio/speech` and, only with
  `--align asr`, `/api/v1/audio/transcriptions`.
- `motion-core` and `motion-render` make no network calls. Compiling and
  rendering are fully offline and deterministic.
- `reel --offline` refuses every provider call and uses only cached voice and
  recognition results.
- **What leaves the machine:** the narration text (for speech synthesis) and,
  only with `--align asr`, the synthesized voice audio. Intents, images,
  frames and videos stay local.

## Local data

- The voice and recognition caches live in
  `~/.cache/motionengine/voice/` (override with `MOTION_VOICE_CACHE`). Entries
  are named by content hashes and never contain keys.
- Renders go to `output/`, which is git-ignored, as are the caches and local
  test media.

## Untrusted input

- Intents and styles are parsed into typed structures that reject unknown
  fields, then validated.
- An intent names library pictures by word, and the lookup accepts only
  `[A-Za-z0-9_-]` names, so an intent cannot point outside the asset library.
- FFmpeg, ffprobe and macOS `say` run as direct processes with argument
  lists, never through a shell.

## Dependencies

`cargo audit` was run against the RustSec advisory database on 2026-10-02,
covering 245 crates:

- **0 vulnerabilities.**
- **1 informational warning:** `ttf-parser` is unmaintained
  (RUSTSEC-2026-0192). It comes in through `cosmic-text → fontdb`. The project
  loads only bundled fonts and fonts the operator chooses.

To re-check:

```bash
cargo install cargo-audit
```

```bash
cargo audit
```
