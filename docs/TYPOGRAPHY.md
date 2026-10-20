# Typography — emotion faces (0.9)

Weak models never choose fonts. The engine maps the resolved taste to one of
11 **emotions** and the emotion to curated face **options** stored as data in
`assets/fonts/registry.json` (curated). Code: `compiler/typography.rs`,
`compiler/theme.rs`; CLI: `fonts list`, `fonts specimen`.

## Roles
`display` headlines · `display_condensed` stacked/condensed display · `serif_emotional`
the voice (italic accents, quotes) · `body` · `mono` labels/sources · `number` figures.
Every option also loads `fallback_face` (IBM Plex Mono: ₹ € digits) and the four
`always_loaded` faces, so missing glyphs fall back to a full-coverage face.

## Emotion resolution (tone × temperament × temperature)
| tone | restrained | editorial | precise | energetic |
|---|---|---|---|---|
| editorial | calm | trust | trust | drama |
| technical | calm | precision | precision | energy |
| playful | warmth | joy | playful_retro | energy |
| classic | (GroteskSerif identity; reported as drama, energetic → energy) ||||

Editorial with a warm temperature turns trust into warmth. Operator override:
`compile --emotion <name>`.

## Defaults (0.24)
`compile`, `resolve-style` and `reel` (which runs `compile`) default to
`--explore 1`: a deterministic pick among the curated options of the resolved
emotion, hash(seed, story key), so videos stop all landing on the same few
legacy faces. The library `CompileOptions` default stays level 0.

## Level 0 is the 0.8 look
With `--explore 0` and no `--emotion`, the tone keeps its 0.8 pairing
(editorial HumanistSerif, technical PrecisionGrotesk, playful FriendlyGeometric,
classic GroteskSerif) byte-for-byte; three of those are option 1 of trust,
precision and joy. Reference-derived pairings stay fixed at every level.

## Category rule (explicit `typography_style`)
At level >= 1 an explicit `typography_style` (and the Classic tone's
GroteskSerif) is a category constraint, not a pin: only options whose face
`class` (registry.json: grotesk, condensed, geometric, serif, slab, sans, mono,
hand) fits the pairing are candidates. grotesk_serif = grotesk/condensed/
geometric display + serif/slab voice; condensed_mono = condensed/grotesk display.
If the level's emotions have no match the tone's other emotions are tried, then
the legacy pairing.

## Options (✗ = face file not bundled yet → option falls back)
| option | display | display cond. | voice | body | mono | number |
|---|---|---|---|---|---|---|
| trust.1 | DM Serif Display | Barlow Condensed SemiBold | DM Serif Display Italic | Lato | IBM Plex Mono | DM Serif Display |
| trust.2 | Zodiak Bold | Zodiak Bold | Zodiak Italic | Satoshi | Necto Mono ✗ | Zodiak Bold |
| trust.3 | Instrument Serif | Instrument Serif | Instrument Serif Italic | Satoshi | IBM Plex Mono | Instrument Serif |
| warmth.1 | Instrument Serif | Instrument Serif | Instrument Serif Italic | Lato | IBM Plex Mono | Instrument Serif |
| warmth.2 | Zodiak | Zodiak Bold | Zilla Slab SemiBold Italic | Satoshi | Necto Mono ✗ | Zodiak |
| warmth.3 | Sprat Regular ✗ | Sprat Regular ✗ | Zodiak Italic | Satoshi | IBM Plex Mono | Sprat Regular ✗ |
| calm.1 | Zodiak Light | Zodiak | Zodiak Light Italic | Satoshi Light | Necto Mono ✗ | Zodiak Light |
| calm.2 | DM Serif Display | Barlow Condensed SemiBold | DM Serif Display Italic | Fira Sans | IBM Plex Mono | DM Serif Display |
| calm.3 | Instrument Serif | Instrument Serif | Instrument Serif Italic | Lato | Necto Mono ✗ | Instrument Serif |
| precision.1 | Barlow Condensed Bold | Barlow Condensed SemiBold | IBM Plex Serif Italic | Barlow Medium | IBM Plex Mono Medium | Barlow Condensed Bold |
| precision.2 | Cabinet Grotesk Bold | Cabinet Grotesk Bold | Zodiak Italic | Satoshi | Necto Mono ✗ | Cabinet Grotesk ExtraBold |
| precision.3 | Clash Display Medium | Clash Display Medium | IBM Plex Serif Italic | Cabinet Grotesk | Necto Mono ✗ | Clash Display Medium |
| joy.1 | Poppins ExtraBold | Poppins Bold | Zilla Slab SemiBold Italic | Poppins Medium | IBM Plex Mono SemiBold | Poppins Black |
| joy.2 | Cabinet Grotesk ExtraBold | Cabinet Grotesk Bold | Zilla Slab Italic | Satoshi Medium | Necto Mono ✗ | Cabinet Grotesk ExtraBold |
| joy.3 | Sprat Bold Bold ✗ | Sprat Bold Bold ✗ | Zodiak Italic | Poppins Medium | IBM Plex Mono | Sprat Bold Bold ✗ |
| energy.1 | Panchang Bold | Panchang Bold | Zodiak Italic | Satoshi Medium | Necto Mono ✗ | Panchang ExtraBold |
| energy.2 | Clash Display Bold | Clash Display Bold | Zodiak Italic | Cabinet Grotesk Medium | Necto Mono ✗ | Clash Display Bold |
| energy.3 | Archivo Black Black | Anton | DM Serif Display | Fira Sans SemiBold | Space Mono Bold | Archivo Black Black |
| drama.1 | Clash Display SemiBold | Clash Display SemiBold | Zodiak Italic | Satoshi | Necto Mono ✗ | Clash Display Bold |
| drama.2 | Mazius Display ✗ | Mazius Display ✗ | Mazius Display Italic Italic ✗ | Satoshi | Necto Mono ✗ | Mazius Display ✗ |
| drama.3 | Anton | Anton | DM Serif Display Italic | Lato | IBM Plex Mono | Anton |
| luxury.1 | Mazius Display ✗ | Mazius Display ✗ | Zodiak Light Italic | Satoshi Light | Necto Mono ✗ | Mazius Display ✗ |
| luxury.2 | Zodiak Thin | Zodiak Light | Zodiak Italic | Cabinet Grotesk Light | Necto Mono ✗ | Zodiak Light |
| luxury.3 | Sprat Light Light ✗ | Sprat Light Light ✗ | Instrument Serif Italic | Satoshi | IBM Plex Mono | Sprat Light Light ✗ |
| urgency.1 | Anton | Bebas Neue | DM Serif Display Italic | Barlow Medium | IBM Plex Mono Medium | Anton |
| urgency.2 | Panchang ExtraBold | Panchang Bold | Zodiak Italic | Cabinet Grotesk Bold | Necto Mono ✗ | Panchang ExtraBold |
| urgency.3 | Archivo Black Black | Anton | DM Serif Display Italic | Satoshi Bold | Space Mono Bold | Archivo Black Black |
| handmade.1 | Frantically | Zilla Slab SemiBold | Frantically | Lato | IBM Plex Mono | Zilla Slab SemiBold |
| handmade.2 | Zilla Slab SemiBold | Zilla Slab Bold | Frantically | Satoshi | IBM Plex Mono | Zilla Slab SemiBold |
| handmade.3 | Instrument Serif | Instrument Serif | Frantically | Lato | Necto Mono ✗ | Instrument Serif |
| playful_retro.1 | Sprat Bold Bold ✗ | Bebas Neue | Zilla Slab Italic | Poppins Medium | Space Mono | Bebas Neue |
| playful_retro.2 | Bebas Neue | Bebas Neue | Zilla Slab Italic | Barlow Medium | Space Mono | Bebas Neue |
| playful_retro.3 | Zilla Slab Bold | Zilla Slab Bold | Frantically | Poppins Medium | IBM Plex Mono | Zilla Slab Bold |

## Exploration
`--explore 1`: options 1–3 of the resolved emotion · `2`: + options of adjacent
emotions (trust↔calm↔warmth↔handmade, trust↔precision, precision↔energy,
joy↔energy↔urgency, joy↔playful_retro↔handmade, drama↔luxury, drama↔urgency,
luxury↔calm) · `3`: + every emotion listed for the tone (`tone_emotions`).
Classic explores only energy and drama. Choice = hash(seed, level, story key).

## Fallback
An option is unavailable when any role face file is missing under the asset root
or its body/number face has no digits. The director then tries the next option of
the same emotion, option 1, then the legacy pairing, and records the skipped ids
in `theme.typography.fallback_from` (and `resolve-style --json`). A missing single
face with a registry `substitutes` entry is replaced role-wise instead
(`"trust.2:font.necto_mono->font.plex_mono"`): Necto Mono → IBM Plex Mono keeps 27
of 33 options available until the Collletttivo files land.

## Faces and licences
Static instances only (no variable fonts), licence texts in `assets/fonts/licenses/`:
OFL (google/fonts: Anton, Archivo Black, Barlow, Bebas Neue, DM Serif Display, Fira
Sans, IBM Plex Mono/Serif, Instrument Serif, Lato, Poppins, Space Mono, Zilla Slab).
Optional faces, not in the repository because their licences forbid redistribution
(`assets/fonts/RESTRICTED.md`): Fontshare Free Font License (Zodiak, Satoshi, Clash
Display, Cabinet Grotesk, Panchang) and 1001fonts freeware (Frantically, words only — no
digits). `scripts/fetch_fonts.sh` downloads the Fontshare families; Frantically is a
manual download. Without them every loader reads an open fallback from
`restricted_fallbacks` in the registry (Satoshi → Lato, Cabinet Grotesk → Fira Sans /
Barlow / Poppins / Archivo Black, Clash Display → Poppins / Archivo Black, Panchang →
Poppins, Zodiak → Instrument Serif / DM Serif Display, Frantically → Zilla Slab SemiBold
Italic). The typography choice, the gate and the compiled scene do not change (they
still name the original face); only the glyphs drawn do, so goldens and tests use open
faces and pass either way. Sprat, Necto Mono
and Mazius Display (Collletttivo) need the foundry's download flow: drop the
static OTFs into `assets/fonts/` with the registry file names
(`Sprat-Light/Regular/Bold.otf`, `NectoMono-Regular.otf`,
`MaziusDisplay-Regular/Italic.otf`) and their options become available without
code changes.

₹ coverage: Satoshi, Panchang, Instrument Serif, DM Serif, Barlow and Archivo
Black have no ₹ glyph; it renders from the fallback face.

## Beat markers (0.24)

There is no eyebrow kicker or kicker rule. `direction::decide_marker` picks per beat: nothing (default); a running index (`Voice::HERO_NUMBER`, display size, in the gutter opposite the headline, only on a counted story); a source/unit tag (`Voice::MARKER`: mono, 0.06 tracking, not caps) under the figure when the beat carries a unit or source the headline lacks (a unit such as "a year" or "per month", or a source such as "SmartAsset, 2024"; never what the picture already shows). No look parks tracked caps labels in a corner, and the beat's purpose word never renders. ClayPop/StudioPop show no text markers. A beat has at most one rule (`one_rule`, furniture). `slop_eyebrow_label` flags the old pattern.


## Motion-safe type system (0.24 M8)

Research basis: type in motion needs heavy or medium weights (thin strokes shimmer and break up under compression); low stroke contrast when small (high-contrast serifs only at display size); a large x-height for small text; tabular digits for counters (a count must not jitter sideways); open spacing for animated text; at most 2 families per piece plus a mono label.

Every registry face carries `weight`, `contrast` (low|medium|high), `width` (condensed|normal|extended), and measured `x_height` (x over H glyph height) and `tabular_digits` (all 0-9 advances within 1 %). `crates/motion-core/tests/motion_type.rs` fails if they drift from the font files.

`motion_safe(face, role, words)`: no weight < 400 anywhere; display/display_condensed/number need weight >= 600 unless the face is high-contrast and the role is `display`; body needs x_height >= 0.66 and low/medium contrast; number needs `tabular_digits` (Count re-lays out text each frame at the current width, so non-tabular figures jitter); `extended` faces only for display with <= 3 words. Options and pairings that fail are excluded at levels >= 1 (`motion-engine fonts` lists each with its reason). Level 0 and level-0 `--emotion` are not gated.

Pairing library v2 (`pairings` in registry.json; all pass the gate; the 7 legacy pairings are unchanged and ungated):
| id | display | voice | body | mono | number | use for |
|---|---|---|---|---|---|---|
| satoshi_editorial | Satoshi Bold | Instrument Serif Italic | Satoshi Medium | IBM Plex Mono | Lato Bold* | modern editorial, tech explainers, startups |
| clash_cabinet | Clash Display Semibold | DM Serif Italic | Cabinet Grotesk Medium | Space Mono | Archivo Black* | creator/hype, announcements, product |
| zodiak_documentary | Zodiak Bold | Zodiak Italic | Satoshi Regular | IBM Plex Mono | Lato Bold* | culture, history, long-form documentary |
| slab_news | Zilla Slab Bold | Zilla Slab SemiBold Italic | Lato Regular | IBM Plex Mono | Lato Bold* | warm news, local stories, explainers |
| instrument_cinema | Instrument Serif (display >= 8 % canvas height; not enforced yet) | Instrument Serif Italic | Satoshi Medium | Space Mono | Lato Bold* | slow cinematic, essays, luxury |

*Substituted: the planned figure faces (Satoshi/Clash/Zodiak/Zilla Bold, Barlow Condensed Bold) have proportional default digits and fail the tabular rule; the renderer does not apply `tnum`. instrument_cinema's display_condensed is Barlow Condensed SemiBold (Instrument Serif Regular fails the display_condensed weight rule).

Selection: level 0 = the tone's legacy pairing. Level 1 (no `--emotion`) = hash(seed, level, story key) over `usage[genre][tone]` pairings, narrowed by `use_for.temperaments` (Classic tone and explicit typography_style stay a category constraint and keep the legacy pairing as a candidate / widen to all pairings). Levels 2-3 add the gated emotion options. `--emotion` at level >= 1 draws only gated options (currently none pass, so it falls back to legacy).

Usage matrix (genre x tone -> pairings):
| genre | editorial | technical | playful | classic |
|---|---|---|---|---|
| standard | zodiak_documentary, slab_news, satoshi_editorial, instrument_cinema | satoshi_editorial, slab_news, clash_cabinet | clash_cabinet, slab_news, satoshi_editorial | satoshi_editorial, slab_news, clash_cabinet |
| documentary | zodiak_documentary, instrument_cinema | slab_news, satoshi_editorial (finance/news) | slab_news, clash_cabinet | zodiak_documentary, slab_news |
| street | clash_cabinet | clash_cabinet | clash_cabinet | clash_cabinet |
| hype | clash_cabinet, satoshi_editorial | same | same | same |
| studio | satoshi_editorial, clash_cabinet | same | same | same |
| cinematic | instrument_cinema, satoshi_editorial, zodiak_documentary | same | same | same |

Comparison sheet: `motion-engine fonts --sheet out.png [--format vertical|landscape]` renders one tile per pairing (7 legacy + 5 v2) through the CPU renderer.
