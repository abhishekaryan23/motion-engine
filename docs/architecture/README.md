# Architecture diagrams

Interactive diagrams of MotionEngine, built with
[Archify](https://github.com/tt-a1i/archify). Each `.html` file is
self-contained and works offline. Open it in a browser. Every box carries a
`SRC` badge listing the source lines it describes.

| Diagram | What it shows | Spec |
|---|---|---|
| [System architecture](system-architecture.html) | The crates, the network edge, and the data each stage hands to the next | [`system.architecture.json`](system.architecture.json) |
| [Reel pipeline](reel-workflow.html) | What `motion-engine reel` runs, step by step, including the fallbacks | [`reel.workflow.json`](reel.workflow.json) |
| [Voice-over sequence](voice-sequence.html) | Script pass, single take, free-voice fallback and word timing | [`voice.sequence.json`](voice.sequence.json) |

## Regenerating

The JSON specs are the source of truth; the HTML is generated. After editing a
spec, run Archify's `finalize`. It validates the spec, renders the HTML and
checks the page in a headless browser:

```bash
node <archify>/bin/archify.mjs finalize architecture docs/architecture/system.architecture.json docs/architecture/system-architecture.html --repo-root . --quality showcase
```

Use `workflow` or `sequence` as the diagram type for the other two specs. When
the referenced code moves, update each node's `sources` and
`meta.repository.revision`.

`finalize` also writes receipts next to the HTML (`*.finalize.json`,
`*.finalize-summary.json`, `*.browser-check.json`, `*.delivery.json`). They
contain local paths and are git-ignored.
