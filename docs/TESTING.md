# Testing

## The gate (once per phase end)

```
cargo fmt --check && cargo clippy --all-targets -q && cargo nextest run --workspace && cargo test --doc --workspace
```

`cargo nextest run` is the default runner (install once: `cargo install cargo-nextest --locked`).
It runs tests in parallel across binaries and prints one line per failure; nextest does not run
doctests, hence the second command. `cargo test --all` still works and runs the same tests.

## While iterating

Test only the crates you changed, and only the area:

```
cargo nextest run -p motion-program
cargo nextest run -p motion-core -E 'binary(timeline_motion)'
cargo test -p motion-core --test compiler choreography::
```

## Layout: few test binaries

Integration tests are grouped into one binary per area (`crates/<crate>/tests/<area>/main.rs`, each old
file is now a module). On this Mac the first launch of every freshly built executable waits ~30 s
(the system's security assessment, near 0% CPU, one binary at a time); with ~170 test binaries that
was ~110 of the old gate's 116 minutes. Compiling the whole workspace's tests after a motion-core
change takes ~70 s. The workspace now builds 25 test binaries; keep it that way (add a module to an
existing area instead of a new `tests/*.rs` file, and set `test = false` on targets without unit tests).

| crate | binaries |
|---|---|
| motion-core | `contract`, `timeline_motion`, `compiler`, `grammars`, `assets`, `audio_speech`, `qa_taste`, `reference`, `public_schema` |
| motion-render | `render_core`, `render_assets`, `render_audio`, `render_qa`, `golden`; `gpu_parity` builds only with `--features gpu` |
| motion-cli | `cli` |
| motion-mcp | `mcp`, `creator` (sets env vars, so it keeps its own process) |

`public_schema` and `golden` stay standalone because the regenerate commands name them.

## Gate wall time (touch motion-core, then the gate)

| | build | tests | total |
|---|---|---|---|
| before (`cargo test --all`, ~170 binaries) | 363 s | 6613 s | 116 min |
| after (`nextest` + doctests, 28 binaries) | 901 s* | 346 s + 132 s | 23 min |

*Mostly first-launch checks while nextest lists each binary; the compile itself is ~70 s (`--timings`).

## Build speed notes

- whisper.cpp / ONNX are opt-in features (`asr-local`, `asr-ctc`); normal builds compile no C++.
- The system linker is Apple's ld-prime (ld-1230), already a fast parallel linker; nothing to configure.
- Optional: adding the terminal app under System Settings → Privacy & Security →
  Developer Tools exempts freshly built binaries from the Gatekeeper scan.
