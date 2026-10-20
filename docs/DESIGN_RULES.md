# Design rules

MotionEngine encodes its taste as rules the engine enforces, not as advice. Most rules below are a
gate in the mistake ledger (`assets/qa/mistakes.json`) with a detector and a fixture, or a compile gate; the id is given in
brackets where one exists. A program that fails a gate is repaired automatically when a
repair exists, and `program render` refuses to render while a gate still fails.

## Composition and motion

- **One main thing per beat.** Each beat has one subject, large and in focus. Supporting elements
  stay smaller and quieter.
- **Heroes fill the frame.** A hero covers at least 45 % of the frame after the camera move, and
  hero type spans at least 55 % of the frame width. Charts and diagrams are framed by default.
  [`small_composition`, `ui_underfill`]
- **Something changes on almost every beat.** A beat with no visible change is a defect, except a
  short settled hold after a component finishes building (at most two beats, capped program-wide).
  [`static_layout`, `beat_coverage`]
- **Cuts land on the music.** Every start and end snaps to a frame on the beat grid, two frames ahead
  of the beat. The final hit lands on a downbeat and holds for at most one beat.
- **Motion carries meaning.** Nothing pulses to the music for its own sake; no bounce, spin or shake
  without a reason. Shapes mark or anchor content (a ring around a value, an underline under a word).
  Counters settle within about 0.45 s. [`pulse_motion`, `meaningless_shape`, `count_unsettled`]
- **Layout follows content.** Nothing reserves space for an element that has not arrived yet; each
  step is laid out from what is visible, and arrivals make their own room. [`anticipatory_space`]
- **No overlaps by accident.** Components do not overlap unless one is declared `over` the other;
  type is never layered over anything. [`component_overlap`]
- **Type never covers the subject.** Text stays clear of people and of the picture's subject.

## Text

- On-screen text is a short title or a derived claim ("UP 2.6×"), never the narration and never the
  brief's own words. Only quoted text in a brief appears verbatim. [`echo_brief`]
- No meta text: no frame rates, BPM, track names, engine names or prompt titles. [`meta_text`]
- No clichés: no countdowns to "GO", no "selected work" or "designer / year" kickers, no eyebrow
  label above a hero title. [`eyebrow`, `eyebrow_kicker`]
- Captions in narrated reels are karaoke (the spoken word is highlighted) and never repeat labels or
  numbers already on screen.
- Typography uses heavy, low-contrast faces (for example Archivo Black, Anton, DM Serif, Fira,
  Barlow Condensed, Space Mono). Thin and extended faces are avoided.

## Colour roles

Every component takes its colours from roles resolved for the ground it sits on; no component
hard-codes a colour. The palette resolver works in OKLCH and re-derives a palette that breaks a rule.

| role | rule |
|---|---|
| ink | at least 7:1 against its ground (4.5:1 on saturated grounds) [`ink_contrast`] |
| strong structure | at least 3:1, for lines that carry meaning (axes, base lines, connectors) [`stroke_contrast`] |
| structure | about 2:1, for quiet framing |
| accent | at least 3:1; its area is budgeted per frame [`accent_area`] |
| texture | the ground's own hue, 1.08–1.35:1; never the ink |

- At most three hue families per video. [`hue_count`]
- No hairlines below 3:1. [`hairline`]
- Textures are never cut, cleared or faded around elements; legibility comes from the palette.
  [`texture_hole`, `text_on_texture`, `busy_under_type`]
- One art direction per video, and variety across videos (colour, layout). No stock clichés:
  holograms, glows, particle fields, gold bars.

## Product UI

- Show real app states: a card or sheet that changes, not a single floating element.
  [`empty_app_state`]
- One UI sans throughout (Satoshi). No emoji, check-mark or arrow glyphs as decoration.
- The cursor or tap lands on its target; one confirmation per flow. [`tap_misses_target`,
  `duplicate_confirmation`, `idle_cursor`]
- Point of view is consistent: the actor owns the device. [`pov_mismatch`]

## Sound

- Music fits the content; the bed stays under the voice.
- In narrated reels the bed sits about 18 dB under the voice (15 dB in the hype look); speech QA
  `voice_over_music` requires at least 15 dB.
- Sound effects stay at least 12 LU under the voice while a word is spoken.
- The final mix is −16 LUFS integrated with a true peak at or below −1 dBTP.

See [SOUND_DESIGN.md](SOUND_DESIGN.md) for the mixer and [MOTION_QA.md](MOTION_QA.md) for the
structural motion checks.

## Generated imagery

- Generated plates and cutouts are raw material: the engine decides placement, treatment and timing.
- Cutouts are generated on a solid magenta key, keyed, de-spilled and checked for leftover magenta.
- Within one video, keep one visual medium per group of related beats, and vary the medium across
  groups.
- Generation happens once per asset and is pinned in a lockfile; see [PROVIDERS.md](PROVIDERS.md).

## Samples and examples

- Samples use invented brands. A real company is never presented as the subject of an ad.
- Fix the engine, not the sample: when a sample shows a defect, the defect becomes a ledger entry and
  the engine is changed until the fixture passes.
