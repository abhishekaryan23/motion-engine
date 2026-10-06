# CreativeIntent v0.2 and StyleProfile (creator and operator)

make_video `story` is a lite story (see motionengine://guide/lite-story) or a full CreativeIntent:
{"version": "0.2", "title": "short_name", "format": "vertical | square | landscape", "beats": [...]}.
You describe WHAT each beat communicates. The engine decides layout, timing, animation and rendering:
never write coordinates, sizes, durations, easing or z-order. Unknown fields are rejected.
Working files: examples/public (motionengine://examples/intent-...). Full reference: motionengine://guide/full.

## A beat
Required: `purpose`, `statement`, `primary`. Optional: `secondary`, `relationship`, `energy`,
`continuity`, `keyword`, `narration`.

- `purpose`: emphasize (one idea lands: bold headline + hero subject; opens a story or states a
  fact); compare (two subjects side by side, default relationship separate); contrast (two in
  tension, default compress); reveal (the payoff: the primary, best a number, is unveiled as the
  answer; usually the last beat, with energy impact); explain (the primary is the headline and the
  statement is read as body text; put pictures in `secondary`).
- `statement` is the on-screen TITLE, 3-8 words (reveal and explain up to about 12). Name the TOPIC,
  not the answer: the title is on screen before the narrator reaches the number, so "Light takes 8
  minutes" spoils the line while "Light from the Sun" does not. Longer than 12 words, it is treated
  as narration.
- `narration` is what the narrator SAYS: 1-2 natural sentences, 8-30 words, as you would tell a
  friend. ALL beats are read in ONE take by one voice, so write one continuous script: each line
  carries on from the previous one ("But...", "So...") and never restarts the topic. Things you name
  (a number, an object, a phrase on screen) appear when spoken, so say them in the order you want
  them seen. A sentence may run across beats (end a fragment with a comma). `""` is a silent beat.
- `keyword`: one word shown very large and faint behind the beat (default: the primary's meaning).
- `energy`: calm (unhurried) | building (default) | impact (punchy; a reveal with impact after
  another beat arrives on a full-screen accent transition).
- `continuity`: none (default) | carry_primary | carry_secondary: that subject stays on screen into
  the next beat (phrase, number and object subjects only).

## Subjects (`primary`, `secondary`): `kind` decides the fields
- phrase {value, meaning}: words, a name, a short claim.
- number {value "+40%", meaning "passengers"}: value is shown exactly as written, with its unit.
- object {asset "piggy_bank", value, meaning}: a lowercase snake_case noun. The engine finds the
  picture in its library (find_assets); with no match it shows `meaning` as text, so any concrete
  noun is safe. `value` is stamped next to the object in compare and contrast beats.
- collection {items: 2-6 of phrase | number | object, meaning}: items that belong together. With
  relationship accumulate they add up into a total; name the consequence in `secondary` if known.
- state_change {entity, from, to, meaning}: one thing moving between two states ("delivery time":
  "three days" to "same day"). A second state_change in `secondary` shows two changes at once.
- derived_metric {numerator {value, meaning}, denominator {value, meaning}, format, meaning}: give
  the raw JSON numbers, never the result; the denominator is not 0. format: percent (default) |
  decimal | per_thousand. Two metrics (primary + secondary) in a compare beat are compared directly.
- layers {layers [{name, note, boundary}], focus, meaning}: things stacked, top down (ocean zones,
  atmosphere, funnel stages). 2-6 layers, different names; `boundary: true` is a thin separator
  between two ordinary layers (never first or last). It is always the primary. Repeat the same
  list in every beat about it and change only `focus` (the name of the layer this beat is about);
  the `secondary` (picture, phrase, number) is shown inside the focused layer.

## relationship (compare and contrast beats)
grow (the secondary gains weight) | compress (the secondary squeezes the primary) | separate (they pull
apart, a visible divide) | replace (the secondary takes over) | carry (the primary holds steady into
the next beat) | accumulate (items add up into one consequence; a collection primary).

## Which structure fits the topic
- stacked things (zones, floors, stages): layers, repeated every beat, a focus each.
- A against B, a rate, old to new: compare or contrast with a secondary; derived_metric for "X out of Y"; state_change for before and after.
- steps in order: emphasize beats with the same primary and continuity carry_primary except the last.
- dated events: one collection of up to 6 dated phrases.
- Three or more picture-only beats that nothing ties together read as a slideshow
  (warning unrelated_beats): tie them with layers, a compare, relationship or continuity.
- data per country / team / product: a stat card is an object with value + meaning; two of
  them in a compare beat show side by side with both figures; a collection of stat cards with
  numeric values is a ranking (bar chart, rows in the order written, each bar grows when its
  name is said). End a comparison of N things on one beat that shows all N.

## Rules the compiler checks (fix them before rendering)
- Every picture is named in its own beat's narration (warning unnamed_picture).
- A keyword is a word the narrator says in that beat; otherwise it is not stamped.
- Say the figure the card shows: "$127" with "a hundred and twenty-seven dollars".

## style: a tone word, or a StyleProfile object
Tone words and when to use them: motionengine://styles. Taste dials, each `auto` = the tone decides:
polarity light | dark; temperature warm | cool; temperament restrained | balanced | energetic;
density sparse | balanced | dense. Explicit overrides, left at default they override nothing:
material paper | flat; typography_style grotesk_serif | condensed_mono; depth flat | layered;
camera_style static | slow_push | drift; motion_language auto | minimal | kinetic | parallax |
sequential | data; texture_style none | subtle_print | heavy_print; accent_role signal_red |
cobalt | acid; seed (an integer; same seed, same result). Set the taste dials, not the overrides.
Production options (look `art`, families, music, captions, aspect, variety): motionengine://options.

## Checklist
- 2-12 beats, one idea each; vary the structure from beat to beat; end on the takeaway.
- A title of 6 words or fewer; narration carries the figures and verdicts.
- Characters: Latin letters, digits, curly quotes, dashes and % + - × ° & # $ € £ ₹ ¥ render;
  emoji, checkmarks and signs like ≈ do not.
- Review: make_video with mode "check", then render, then view_frames, then revise_video.
