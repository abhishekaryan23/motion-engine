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

## Level 0 is the 0.8 look
With `--explore 0` (default) and no `--emotion`, the tone keeps its 0.8 pairing
(editorial HumanistSerif, technical PrecisionGrotesk, playful FriendlyGeometric,
classic GroteskSerif) byte-for-byte; three of those are option 1 of trust,
precision and joy. Reference-derived pairings and an explicit `typography_style`
stay fixed at every level.

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
Sans, IBM Plex Mono/Serif, Instrument Serif, Lato, Poppins, Space Mono, Zilla Slab),
ITF Free Font License (Fontshare: Zodiak, Satoshi, Clash Display, Cabinet Grotesk,
Panchang), 1001Fonts FFC (Frantically, words only — no digits). Sprat, Necto Mono
and Mazius Display (Collletttivo) need the foundry's download flow: drop the
static OTFs into `assets/fonts/` with the registry file names
(`Sprat-Light/Regular/Bold.otf`, `NectoMono-Regular.otf`,
`MaziusDisplay-Regular/Italic.otf`) and their options become available without
code changes.

₹ coverage: Satoshi, Panchang, Instrument Serif, DM Serif, Barlow and Archivo
Black have no ₹ glyph; it renders from the fallback face.
