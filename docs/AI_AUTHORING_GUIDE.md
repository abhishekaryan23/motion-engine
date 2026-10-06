# AI Authoring Guide — CreativeIntent v0.2

You write a small JSON file called a **CreativeIntent**. MotionEngine turns it into a finished motion-graphics video.

> **You describe WHAT the story should communicate.**
> **MotionEngine decides HOW it is laid out, animated and rendered.**

Contract files (they are exact and machine-checkable):
- `schema/creative-intent-v0.2.schema.json`: the intent you write. **v0.2 is the current, recommended contract.**
- `schema/creative-intent-v0.1.schema.json`: the legacy v0.1 contract. Existing v0.1 files keep working unchanged, but new v0.2 features (§4b, `accumulate`) require `"version": "0.2"`.
- `schema/style-profile-v0.1.schema.json`: optional visual style.
- `examples/public/`: working examples.

Validate your JSON against the schema before you hand it over.

---

## 1. File structure

```json
{
  "version": "0.2",
  "title": "train_example",
  "format": "vertical",
  "beats": [ { ...beat... }, { ...beat... } ]
}
```

| field | required | type | notes |
|---|---|---|---|
| `version` | yes | string | `"0.2"` (current). `"0.1"` is still accepted for legacy files, without the v0.2 subject kinds and `accumulate`. |
| `title` | yes | string | Short name. It also names the output folder and video file, so use only letters, digits, `_` and `-` (e.g. `bridge_story`). |
| `format` | no | enum | `vertical` (default, 9:16, the best-supported format), `square` (1:1), `landscape` (16:9). |
| `beats` | yes | array | At least 1 beat. Typical: 1–5 beats. One beat = one idea. |

Unknown fields are **rejected** everywhere: in the top level, beats, subjects and style. Do not invent fields.

## 2. Beat fields

| field | required | type | default |
|---|---|---|---|
| `purpose` | yes | enum | — |
| `statement` | yes | string | — |
| `primary` | yes | Subject | — |
| `secondary` | no | Subject | none |
| `relationship` | no | enum | depends on purpose (see §5) |
| `energy` | no | enum | `building` |
| `continuity` | no | enum | `none` |
| `keyword` | no | string | the primary's `meaning` (else its `value`) |
| `narration` | no | string | the narrator reads the `statement` |

**`statement`** is the on-screen TITLE of the beat — what the viewer reads in big type. Keep it short: 3–8 words for emphasize, compare and contrast; up to about 12 words for reveal and explain. Write it exactly as it should read; MotionEngine picks the case and line breaks. A statement longer than 12 words is treated as narration: the screen then shows a short derived title (your primary phrase or keyword) and the sentence goes to the voice-over/captions.

**`narration`** (optional, for voice-over videos) is what the narrator SAYS during the beat: 1–2 natural sentences, about 8–30 words, written the way you'd say it out loud to a friend — contractions, a question, a short aside are all welcome. It should expand on the title, not repeat it word for word. Captions show the narration word by word. Without `narration`, the narrator reads the statement.

| | good `statement` (title) | good `narration` (spoken) |
|---|---|---|
| hook | "Your 3 a.m. replay" | "It's midnight, you're staring at the ceiling, and your brain is replaying that one awkward moment." |
| fact | "Only 23% noticed" | "Researchers checked. The wearer guessed half the room noticed — it was barely one in four." |
| payoff | "The room moved on" | "Honestly? Nobody has time to obsess over your mistakes. They moved on hours ago." |

Never put a long spoken paragraph in `statement`; put it in `narration`. Write narration in English.

**One continuous script.** The whole voice-over is read in one take by one voice, so the narration lines must read as one script split across beats: each line continues the previous one ("But…", "So…", "That's why…"), keeps the same point of view and tense, and never restarts the topic or re-introduces the subject. Read all lines aloud in order before you finish: if it sounds like separate captions, rewrite it until it flows like one person talking.

**Fast cuts:** one spoken sentence may run across several beats. Give each beat a fragment of it — end a fragment with a comma, or start the next fragment in lowercase — and the narrator reads it as one sentence while the picture changes on each fragment:

```json
{ "statement": "Midnight", "narration": "It's midnight,", ... },
{ "statement": "Staring at the ceiling", "narration": "you're staring at the ceiling,", ... },
{ "statement": "The replay", "narration": "and your brain replays that one awkward moment.", ... }
```

`"narration": ""` makes a silent visual beat (the picture holds while the voice pauses). Things you name in the narration — a number like "50%", an object, a phrase on screen — appear on screen when they are spoken, so say them in the order you want them to appear.

**`keyword`** is one short word shown huge and faint in the background, e.g. `"ferry"`. Set it when the primary's `meaning` is longer than one word, or the background gets cluttered. If the keyword is also a word of the `statement`, kinetic motion may emphasize it inside the headline on building/impact beats.

Each beat lasts a few seconds. The time is chosen automatically from the amount of text and the energy. You cannot set durations, and you don't need to.

## 2b. Titles that don't spoil the line

The title (`statement`) is on screen from the first moment of the beat; the narrator reaches the answer a few seconds later. A title that already states the number or the verdict gives the line away: the viewer reads the punchline before hearing it.

> **The title names the TOPIC. The narration delivers the NUMBER or the verdict.**

| `statement` (title) | `narration` (spoken) | |
|---|---|---|
| "Light from the Sun" | "Sunlight has a long way to go before it reaches us. It takes eight minutes." | good: the topic; the number lands when it is said |
| "The ferry crossing" | "For years it meant forty minutes on the water. Every single day." | good: the topic; the number is spoken |
| "Light takes 8 minutes" | "Sunlight has a long way to go before it reaches us. It takes eight minutes." | **avoid**: the title shows "8" about five seconds before the narrator says it |

When a title does this the compiler prints `warning[title_states_conclusion]: beat N: title "Light takes 8 minutes" says "8" 4.9 s before the narrator does; keep the title to the topic ...`. Rewrite the `statement` as the topic and leave the figure in the `primary` and in the `narration`. The same applies to the `keyword` when the title contains it. A title may carry a figure the narrator says right away (or never says, like "Only 23% noticed" above): the warning fires only when the narrator reaches the word more than 0.8 s after the beat begins. The `cinematic`, `studio` and `documentary` looks hold the title until its word is spoken, so they never raise it; the others show it at once.

## 2c. Numbers on screen: countdowns and lists in order

Beats carry no number by default. A small rank (`03 — sharks`) appears only when the story counts: a countdown or things listed in order. The engine sees that from the opening words of each item beat's `statement`, else its `narration`:

| opening | rank |
|---|---|
| "Number three: …", "No. 3 …", "#3 …", "3. …", "3 — …" | 3 |
| "Step 2: …", "Fact 4 …", "Reason 1 …", "Day 3 …" | 2, 4, 1, 3 |
| "First, …", "Second, …", "Third, …" ("Finally, …" continues the run) | 1, 2, 3 |

At least three beats must be ranked, and the ranks must run by one in story order: up from 1 (steps in order) or down (a countdown, e.g. 5, 4, 3, 2, 1). Intro and outro beats without a rank stay unnumbered. If the ranks skip or repeat, nothing is numbered. A figure that is a value ("381 billion dollars", "70%", "2024: …") is never a rank.

## 3. Purpose: what the beat is for

| value | meaning | use when | do NOT use when |
|---|---|---|---|
| `emphasize` | One idea lands hard: the statement becomes a bold headline and the primary is shown as the hero element. A phrase or number `secondary` becomes a supporting line; an object `secondary` becomes a small supporting picture. | Opening a story, stating a key fact or figure. | You have two things to weigh against each other (use compare/contrast). |
| `compare` | Two subjects side by side, neutrally. Default relationship: `separate`. | "A vs B" where neither side is the villain. | There is tension or pressure between them (use contrast). |
| `contrast` | Two subjects in tension. Default relationship: `compress`. | One side is squeezing, overtaking or undermining the other. | There is only one subject. |
| `reveal` | The payoff: the primary (best as a number) is unveiled as the answer, with its `meaning` as a label and the statement as an emotional closing line. | Conclusions, results, "here's the real number". Usually the last beat. Pair with `energy: "impact"` for a dramatic entrance. | Setting up context at the start. |
| `explain` | The primary's text becomes the headline and the statement is set as body copy. An object `secondary` is shown as an illustration. | Definitions, "how it works", a sentence that needs to be read calmly. | You need a big number moment (use emphasize or reveal). |

A beat with `compare` or `contrast` should have a `secondary`. Without one, the second side is left empty.

In `reveal`, an optional `secondary` is shown small beneath the statement. In `explain`, the primary is used as **text** (its `value`, else `meaning`); an object primary is not pictured there, so put pictures in `secondary`.

## 4. Subjects: `primary` and `secondary`

```json
{ "kind": "number", "value": "120", "meaning": "passengers" }
```

| field | phrase | number | object |
|---|---|---|---|
| `kind` | required | required | required |
| `value` | the words shown (recommended) | the figure shown, with its unit or symbol, e.g. `"120"`, `"+40%"`, `"6 min"`, `"$9"` (recommended) | optional short figure. It is stamped next to the object in compare/contrast beats and not shown elsewhere. |
| `meaning` | optional: what it stands for | recommended: what the number counts, in 1–3 words | recommended |
| `asset` | ignored | ignored | **required**. It must be a name from the asset list below. |

- **phrase**: words. Use for names and short claims.
- **number**: a figure. It is displayed exactly as written, so format it the way it should look (`"12,500"`, `"-27%"`, `"3.5 km"`).
- **object**: a pictured thing, named as a short lowercase snake_case noun (`piggy_bank`, `alarm_clock`, `coffee_cup`, `retro_tv`). The engine looks it up in its asset library (≈ 630 objects, figures and icons across 10 families); a name with no picture is shown as text (the object's `meaning`, else the name), so concrete everyday nouns are always safe. Give `meaning` in 1–3 words.

If `value` is missing, `meaning` is displayed instead. `meaning` also appears as a small label on the beat and is the default background keyword.

**Characters:** Latin letters (including accents), digits, curly quotes, dashes and the symbols `% + - × ° & # $ € £ ₹ ¥` render correctly. Emoji, checkmarks and math symbols such as `≈` do **not** render; they show as empty boxes. Non-Latin scripts are not supported.

## 4b. Structured subjects (v0.2)

Use these when a plain phrase or number would lose the meaning. Give MotionEngine the **facts**; it decides the layout, the order things appear in, the animation, and does all the arithmetic.

**`collection`**: several distinct items that belong together, 2–6 items. Each item is a `phrase`, `number` or `object` subject (items cannot be collections).

```json
{ "kind": "collection", "meaning": "drinks",
  "items": [ { "kind": "phrase", "value": "Coffee" }, { "kind": "phrase", "value": "Tea" },
             { "kind": "phrase", "value": "Juice" }, { "kind": "phrase", "value": "Water" } ] }
```
Each item stays individually visible. With relationship `accumulate` the items add up one after another into a total. If you know the consequence (e.g. `"₹2,400"` a month, or `"9 kg"`), put it in `secondary` as a number; otherwise MotionEngine shows the total itself: the sum when every item is a number with the same unit, else the count (`"4 drinks"`).

**`state_change`**: one thing moving from one state to another.

```json
{ "kind": "state_change", "entity": "delivery time", "from": "three days", "to": "same day", "meaning": "faster" }
```
`entity`, `from` and `to` are required, 1–4 words each. The viewer sees the old state turn into the new one. **Two things changing at the same time:** put one `state_change` in `primary` and another in `secondary`; they are shown side by side, changing together.

**`derived_metric`**: a value MotionEngine calculates from two numbers. Give the raw numbers, never the result.

```json
{ "kind": "derived_metric", "meaning": "sign-up rate", "format": "percent",
  "numerator": { "value": 80, "meaning": "sign-ups" },
  "denominator": { "value": 1000, "meaning": "visitors" } }
```
- `numerator` / `denominator`: `value` is a JSON number (`1000`, not `"1,000"`), `meaning` says what it counts. The denominator must not be 0.
- `operation`: only `"ratio"` (numerator ÷ denominator), the default. You may omit it.
- `format`: `percent` (default, `80 ÷ 1000` → `8%`), `decimal` (`1 ÷ 8` → `0.125`), `per_thousand` (`800 ÷ 60000` → `13.3 per 1,000`). Results are rounded half away from zero: percent to 2 decimals, per_thousand to 1, decimal to 3, trailing zeros dropped (`1000 ÷ 100000` → `1%`, `800 ÷ 60000` → `1.33%`); tiny non-zero values keep two significant digits.
- **Comparing two rates:** in a `compare` or `contrast` beat put one `derived_metric` in `primary` (before / first) and one in `secondary` (after / second). MotionEngine shows both calculations, compares the results, and states the direction (e.g. "fewer sales · higher conversion"). Say whether that is good or bad in your `statement`.

**`layers`**: things that are layered or stacked, named in order from the top (or first) down: the zones of the ocean, the layers of the atmosphere, the levels of a building, the stages of a funnel. Use it when the story is about WHERE something happens within a structure; a single picture cannot tell that.

```json
{ "kind": "layers", "meaning": "ocean water column",
  "layers": [ { "name": "Photic zone", "note": "sunlit, photosynthesis" },
              { "name": "Thermocline", "note": "density boundary", "boundary": true },
              { "name": "Aphotic zone", "note": "dark, decomposition" } ],
  "focus": "Thermocline" }
```
- **Use it as the `primary` of every beat that is about the same layers, and repeat the same `layers` list each time** (same names, notes and order). MotionEngine draws the layers once, names every one of them, and keeps them on screen from beat to beat: between beats the view glides down (or up) to the next layer, so the viewer always sees where the story is. Beats about other things in between start a new column later.
- `focus` is the `name` of the layer this beat is about (case does not matter). That layer is lit and the others dim. Leave it out for a beat about the whole stack. It must match one of the layers.
- The beat's **`secondary`** (a picture, phrase or number) is shown **inside the lit layer**, with its `meaning` as a caption: put what lives or happens there (`phytoplankton` in the sunlit zone, `"12 km"` for a layer's depth).
- `boundary: true` marks a thin separator rather than a layer of its own (a thermocline, a tropopause, a border). It must sit between two ordinary layers, so the first and last layer are never boundaries.
- `"relationship": "separate"` on a layers beat lights the dividers between the layers: use it for "kept apart", "does not mix", "cannot cross".
- 2 to 6 layers; names 1–4 words and different from each other; `note` up to about 6 plain words (no symbols such as `·`). `meaning` names the whole stack in 1–3 words.
- Layers are always the `primary`, never the `secondary`, and `continuity` is not needed: the layers persist by themselves.

Other structured subjects do not carry into the next beat (`continuity` applies only to phrase, number and object subjects).

## 4c. Which structure fits which topic

Beats that relate to each other read as one explanation; beats that do not read as a slideshow of separate pictures. Decide the SHAPE of the topic first, then write the beats to fit it.

| the topic is... | use | when | what you write today |
|---|---|---|---|
| things stacked: zones, floors, stages | **layers** | the story moves down or up through a structure | a `layers` primary repeated in every beat, a `focus` per beat (§4b) |
| two things weighed, a rate, old → new | **compare / ratio** | "A vs B", "X out of Y", before → after | a `compare` / `contrast` beat with a `secondary`; `derived_metric` for a ratio; `state_change` for before → after (§4b) |
| steps in order | **steps** | a process, "first… then… finally…" | ordered beats that all keep the thing being processed on screen with `continuity` |
| a loop that returns | **cycle** | the water cycle, a sales loop | steps, and the last beat names the first subject again. A dedicated cycle structure is coming |
| parent → child | **hierarchy / zoom** | cell → nucleus, country → city | `layers` when the levels stack; otherwise outer-to-inner beats, each handing its child to the next. Dedicated structure coming |
| something moving, or stopped | **flow / blocked** | data through a firewall, blood through an artery | a `contrast` of two objects with `grow`, `replace` or `compress` (the `cinematic` tone draws the arrow); `"relationship": "separate"` on `layers` for "does not mix". Dedicated flow structure coming |
| dated events | **timeline** | history, a product's years | a `collection` of up to 6 dated phrases in one beat (they arrive in order). Dedicated timeline coming |

Layers (repeat the list in every beat; only `focus` changes):
```json
{ "purpose": "explain", "statement": "Where the light runs out",
  "primary": { "kind": "layers", "meaning": "ocean", "focus": "Twilight zone",
               "layers": [ { "name": "Sunlit zone" }, { "name": "Twilight zone" }, { "name": "Dark zone" } ] } }
```

Compare / ratio:
```json
{ "purpose": "compare", "statement": "Ferry against bridge",
  "primary":   { "kind": "number", "value": "40 min", "meaning": "by ferry" },
  "secondary": { "kind": "number", "value": "6 min",  "meaning": "by bridge" } }
```

Steps: give every step the same `primary` text and `continuity: "carry_primary"`, except the last beat (continuity on the last beat has no effect):
```json
{ "purpose": "emphasize", "statement": "Step 1: mix the dough", "primary": { "kind": "phrase", "value": "Dough" }, "continuity": "carry_primary" },
{ "purpose": "emphasize", "statement": "Step 2: let it rise",   "primary": { "kind": "phrase", "value": "Dough" }, "continuity": "carry_primary" },
{ "purpose": "emphasize", "statement": "Step 3: bake",          "primary": { "kind": "phrase", "value": "Dough" } }
```
(`carry_primary` has no effect on an `explain` beat, see §7: use `emphasize` beats for steps, or `carry_secondary` on `explain`.)

Cycle: write the steps above; the last beat names the first subject again, e.g. `"statement": "Rain falls, and it starts again"` with the same `primary` as the first step.

Hierarchy / zoom: the parent is the `secondary` and is carried into the next beat as its `primary`:
```json
{ "purpose": "explain", "statement": "A cell is mostly parts", "primary": { "kind": "phrase", "value": "Cell" },
  "secondary": { "kind": "phrase", "value": "Nucleus" }, "continuity": "carry_secondary" },
{ "purpose": "emphasize", "statement": "The nucleus holds the DNA", "primary": { "kind": "phrase", "value": "Nucleus" } }
```

Flow / blocked (in the `cinematic` tone the two pictures are drawn with an arrow from one to the other):
```json
{ "purpose": "contrast", "relationship": "compress", "statement": "The firewall stops the traffic",
  "primary":   { "kind": "object", "asset": "cloud_data", "meaning": "traffic" },
  "secondary": { "kind": "object", "asset": "shield",     "meaning": "firewall" } }
```

Timeline:
```json
{ "purpose": "explain", "statement": "Nine centuries of firsts",
  "primary": { "kind": "collection", "meaning": "timeline",
               "items": [ { "kind": "phrase", "value": "1096: Oxford starts teaching" },
                          { "kind": "phrase", "value": "1325: Tenochtitlan is founded" },
                          { "kind": "phrase", "value": "1969: the first Moon landing" } ] } }
```

If the compiler prints `warning[unrelated_beats]: ...` your story has three or more beats that are pictures (object primaries), each one a separate subject, with no `layers`, collection, compare / contrast, `relationship`, `continuity` or repeated subject tying them together (a story told in phrases and numbers is narrative and never triggers it); the message names the rows above that fit your words. Do not ignore it: pick the row that fits the topic and rewrite the beats (for example give every beat the same `layers` primary, turn two beats into one `compare`, or carry the one thread through the steps with `continuity`).

## 4d. Stat cards, comparisons and rankings (0.22)

Data stories (prices, rates, scores per country, team or product) need no special kind: the fields you already have say it.

- **A stat card** is an `object` with a `value` and a `meaning`: the picture, its figure and its name, drawn as one unit in every look.
  `{ "kind": "object", "asset": "flag_nz", "value": "4.1%", "meaning": "New Zealand" }`
- **Two things compared** is a `compare` (or `contrast`) beat whose primary and secondary are both stat cards. Every look shows both pictures, both figures and both names as equals, with the relationship drawn between them.
- **A ranking** is a `collection` of stat cards whose values are numbers. It becomes a bar chart: one row per item with its picture, name, bar and figure; each bar grows as the narrator names that item. Rows appear in the order you write them, so write them in the order the narrator says them (highest first for "the leaders", lowest first for "the surprise leader"). A `number` item without a picture (e.g. "Euro area") may sit among them. Number items whose meanings are times ("2019", "Q3", "March") stay a vertical chart over time instead.
- **End a comparison of several things on one beat that shows them all**: a ranking of every item you compared. Do not leave one out ("five economies" means five rows).

Three rules keep these stories honest; the compiler warns when they are broken, and `make_video` with `mode: check` lists the warnings before you render:

1. **Every picture is named in its own beat's narration.** A flag on the intro beat whose narration never says the country (`warning[unnamed_picture]`) appears before the viewer knows why. Put the picture on the beat that names it.
2. **A `keyword` is a word the narrator says in that beat.** Documentary stamps and street tags slam on the spoken word; a keyword the narrator never says is not stamped.
3. **The numbers on screen are the numbers said.** Write the narration to say the figure the card shows: "$127" with "a hundred and twenty-seven dollars", "4.1%" with "four point one percent". Then the figure lands on its word.

## 5. Relationship: how primary and secondary relate

It is meaningful for `compare` and `contrast` beats. Omit it to get the purpose's default: `compare` uses `separate`, `contrast` uses `compress`. On other purposes, only `carry` has an effect (it keeps the primary on screen into the next beat).

| value | meaning | use when | do NOT use when |
|---|---|---|---|
| `grow` | The secondary gains weight and presence over the beat. | Something increases or becomes more important. | The primary should visibly lose ground (use compress or replace). |
| `compress` | The secondary creates pressure: the primary's space and power shrink while the secondary looms larger. | Squeeze, burden, crowding out, "costs rise while X stays". | The two are equals (use separate). |
| `separate` | The two pull apart, with a visible divide between them. | A gap or divergence; neutral comparisons. | One side is overtaking the other. |
| `replace` | The secondary takes over; the primary recedes. | Before → after, old → new, substitution. | Both should remain equally important. |
| `carry` | The primary holds steady **and stays on screen into the next beat**, like `continuity: "carry_primary"`. | The primary is the constant thread of the story. | You don't want the primary to persist. Use `none` continuity and another relationship. |
| `accumulate` (v0.2) | Separate items add up, one after another, into one larger consequence. Works on any purpose. | A `collection` primary whose items combine into a total or a burden. | The items are alternatives or unrelated (use a collection without `accumulate`). |

Describe the relationship only. Never try to specify how it is animated.

## 5b. Two pictures that relate (cinematic look)

In the `cinematic` tone a `compare` or `contrast` beat whose `primary` and `secondary` are both **objects** shows the two pictures together on the focus plane (side by side or stacked, whichever lets them be larger) and **draws the relationship between them**. Say what each picture is in its `meaning` (a label under the picture) and, when it carries a figure, in its `value` (stamped above it). The first picture arrives with the beat; the second flies in from the opposite side when the narrator names it, and the connector draws once it has landed.

| `relationship` | what the viewer sees | use when |
|---|---|---|
| `separate` (compare default) | a divider with a VS disc | "A versus B", two things with different properties (a year each, a verdict each) |
| `grow` | an arrow from A to B; B grows | A makes B bigger: heat → a tower that stretches, a cause → its effect |
| `replace` | an arrow from A to B; A steps back and dims | before → after, old → new |
| `compress` (contrast default) | an arrow from B onto A; A shrinks | B presses on A |
| `carry` | a link line; both stay | two things that belong together |
| `accumulate` | a plus disc | A and B add up |

```json
{ "purpose": "compare", "relationship": "separate",
  "statement": "Older than the Aztecs",
  "narration": "Oxford University is older than the Aztec Empire.",
  "primary":   { "kind": "object", "asset": "oxford_college", "meaning": "teaching began", "value": "1096" },
  "secondary": { "kind": "object", "asset": "aztec_pyramid",  "meaning": "empire rose",    "value": "1400s" } }
```

Name the second picture in the narration where it should appear. A beat that is not a `compare` / `contrast` of two objects keeps a hero with a supporting picture; a `collection` of three or more still turns on the sphere.

**Backgrounds.** In the `cinematic` tone each beat sits in a scene that fits the story: the engine matches the words of the beat (keyword, title, and the names, meanings and values of its subjects) against the environment plates of the enabled asset families (catalog role `texture`, tagged with the nouns they depict; at least two tags must match). So name concrete nouns (`shark`, `tree`, `eiffel_tower`) and the sea, the Paris sky or the market behind them come for free when the family has such a plate.

## 6. Energy: pace and intensity

| value | meaning | use when | do NOT use when |
|---|---|---|---|
| `calm` | Unhurried, gentle, reflective; beats blend softly. | Openings, context, empathy. | The beat is a punchline. |
| `building` | Steady editorial momentum (default). | Most middle beats. | — |
| `impact` | Punchy and fast. A **reveal** beat with `impact` that follows another beat arrives with a bold full-screen accent-color transition. | The payoff, usually one per story, usually the last beat. | Every beat. Overuse removes the punch. |

## 7. Continuity: keeping a subject on screen across beats

| value | meaning |
|---|---|
| `none` | Nothing carries over (default). |
| `carry_primary` | This beat's primary stays on screen and travels into the next beat. |
| `carry_secondary` | This beat's secondary stays on screen and travels into the next beat. |

How carrying works:
- **If the next beat features the same subject** (same `value`, or the same `meaning` if there is no value, or the same `asset`), the subject moves into its new role there instead of appearing twice. Repeat it with identical text in the next beat's `primary` or `secondary`.
- **If the next beat doesn't mention it**, it stays as a small reference at the top of the next beat. It survives even the accent-color transition, then leaves at the end of that beat (or stays until the end if that is the final beat).
- A subject carries **one beat forward**. To keep it longer, the next beat must also set `carry_primary` or `carry_secondary` for the role it now occupies.
- Continuity on the **last** beat has no effect.
- `carry_primary` has no effect on an `explain` beat. `carry_secondary` works there.

Use continuity for the one element that ties the story together, such as the figure you keep returning to. Use it at most once or twice per story.

## 8. Style profile (optional)

This is a separate JSON file. `{}` is valid and gives the default look. Every field is optional, and unknown fields are rejected.

| field | values (default first) |
|---|---|
| `family` | `editorial_collage`, `minimal`. Accepted, but **currently has no visible effect**. |
| `material` | `paper` (warm textured paper), `flat` (plain light background) |
| `typography_style` | `grotesk_serif` (heavy headlines + italic serif voice), `condensed_mono` (condensed display + monospaced body) |
| `depth` | `layered` (collage halftone patches, slowly drifting background word), `flat` |
| `camera_style` | `slow_push` (gentle push-in), `static`, `drift` (slow sideways drift with a barely perceptible push-in) |
| `motion_language` | `auto` (the engine picks per beat), `minimal` (restrained, no bounce), `kinetic` (words cascade in; the beat's `keyword` is enlarged or punched), `parallax` (background, subject and foreground move at different depths), `sequential` (lines and items arrive one after another), `data` (numbers count up, quantities grow). Leave it `auto` unless you want one consistent feel for the whole video. |
| `texture_style` | `subtle_print`, `none`, `heavy_print` |
| `accent_role` | `signal_red`, `cobalt`, `acid` (yellow-green) |
| `seed` | non-negative integer, default `0`. It only varies the texture patterns. Write it as a plain integer (`7`, not `7.0` or `7e0`); the schema cannot catch this, but the engine rejects it. |

### 8b. Taste fields (0.6): name a tendency, not a look

Five optional style fields let you steer the whole design with a few words. The engine's TasteDirector turns them into a complete, coherent system: palette, background, typography, motion character, transitions, pacing and density. You never choose colors, fonts or timings.

| field | values (default first) | what it means |
|---|---|---|
| `tone` | `auto`, `editorial`, `technical`, `playful`, `street`, `documentary`, `hype`, `studio`, `cinematic` | the overall design character. `auto` = the classic warm editorial collage; the last five are genres (below) |
| `polarity` | `auto`, `light`, `dark` | dark text on light, or light text on dark |
| `temperature` | `auto`, `warm`, `cool` | warm or cool neutrals and accents |
| `temperament` | `auto`, `restrained`, `balanced`, `energetic` | how motion broadly feels: quiet and soft, balanced, or big and lively |
| `density` | `auto`, `sparse`, `balanced`, `dense` | how much supporting detail surrounds the one primary focus (text always stays readable) |

**Genre tones** (each picks its own look, layout grammar and finishing effects; `reel` uses them by default):

| tone | use it for | what the engine does | best inputs |
|---|---|---|---|
| `street` | music, sports, street culture, hype about places | the picture owns ~75 % of the frame, a giant stencil word behind it, tape strips, a sticker, halftone photo grounds, camera shake and colour flashes on the hits. With `"polarity": "light"` it becomes a paper poster: red web, red banner headline, object from the top and a person at the bottom | an `object` (or a delivered photo/cutout) per beat; environment photos make it richer |
| `documentary` | explainers about money, history, science, facts (Vox style) | evidence documents on a desk: clipping with redacted lines, taped photo, a figure card with a drawn underline, a stamp and a highlighter; slow 3D camera with depth of field | numbers or `$`/`%` values in `primary`, a picture per beat |
| `hype` | 10–20 s openers, punchy lists | every beat becomes fast hard cuts, one or two words (or the picture) slamming in, colour inversions, shake and glitch | short statements, strong keywords |
| `studio` | creator / brand storytelling | one big cutout (best a person) over a bold colour disc on a clean studio ground; the spoken words appear large behind the subject's head as they are said (no subtitle bar) | a person picture per beat (delivered cutout) and a `narration` per beat |
| `cinematic` | tech, science, space, big-idea trailers | a true 3D scene per beat: the camera flies in out of a blur, racks focus from a giant background word to the hero, then dollies, trucks, cranes or orbits through depth planes (glow, dust, a prop, the hero, the title, foreground bokeh) and flies through into the next beat with a zoom-blur light burst; a `collection` primary of 3–6 items becomes a turning sphere where each item comes into focus in turn | an `object` per beat (`primary`, plus a `secondary` object as the midground prop); a short `statement` |

Pictures: name any concrete noun as an `object` (`"asset": "robot"`, `"money_bag"`, `"microscope"`); the library covers AI, tech, science, psychology and business props in clay, photo and marble styles plus people at work. Your own photos/cutouts go in an asset manifest (`beat_N.hero_subject`, `beat_N.hero_object`, `beat_N.environment`); `motion-engine matte` cuts photos out on a Mac.

`auto` means "let the tone decide". Three complete examples (in `examples/taste/`):

```json
{ "tone": "editorial", "polarity": "light", "temperature": "warm", "temperament": "restrained", "density": "balanced" }
```
Magazine feel: serif headlines, quiet paper ground, soft small motion, generous space.

```json
{ "tone": "technical", "polarity": "dark", "temperature": "cool", "temperament": "balanced", "density": "balanced" }
```
Dark data feel: condensed and monospaced type, a drifting measurement grid, crisp precise motion, sliding panel wipes.

```json
{ "tone": "playful", "polarity": "light", "temperature": "warm", "temperament": "energetic", "density": "dense" }
```
Playful print feel: poster type, large color fields that recompose per beat, springy cascading motion, bold stickers.

Guidance:
- **Prefer `tone` plus at most one or two other fields.** `{ "tone": "technical" }` or `{ "tone": "editorial", "polarity": "dark" }` is usually all you need; every field you leave `auto` gets a value chosen to fit the rest.
- Setting any of the four other fields without a `tone` behaves as `"tone": "editorial"`. `{}` (everything `auto`) is unchanged from earlier versions.
- **Do not set the legacy fields** (`material`, `typography_style`, `depth`, `camera_style`, `texture_style`, `motion_language`) unless you are deliberately overriding what the tone chose. A legacy field left at its default never fights the tone; one you set explicitly always wins over it.
- **Changing `accent_role` (red to cobalt to acid) is not a new style.** It only changes the accent color. To make two videos look genuinely different, change `tone`, `polarity` or `temperament`.
- Do not describe taste with time or geometry ("faster cuts", "0.3 s wipes"): the taste fields are the vocabulary. Pacing comes from `energy` on each beat plus `temperament`.

### 8c. Brand colours (0.21)

When the piece must wear a brand, give its colours in `brand` (hex `#RRGGBB`); every field is optional:

```json
{ "tone": "editorial", "brand": { "primary": "#0A84FF", "secondary": "#FFD60A", "background": "#101828", "text": "#FFFFFF" } }
```

- `primary`: highlights, rules, transitions and key figures (the accent). `secondary`: large colour fields and supporting accents. `background`: the ground (it replaces the look's paper texture plate). `text`: the text colour.
- The look and its layout stay; only the colours change. Use `tone` (and `polarity`) as usual to pick the design.
- Text is always readable: a `text` colour that does not read on the `background` (contrast under 4.5:1) is replaced by near-black or near-white, and a `primary` that is nearly invisible on the background (under 2:1, e.g. yellow on white) hands the lead to a readable `secondary` or is darkened just enough. Each change is reported as `warning[brand_contrast]`. Colours that read are used exactly.
- Give only what the brand defines. One colour (`primary`) is enough for most pieces.

## 9. Abstraction rule: never author these

The contract has **no fields** for any of the following. Do not add them, and do not put them into `statement` or `value` as instructions:

coordinates, positions, sizes, font sizes, font families, colors (other than choosing `accent_role`), frame numbers, timestamps, durations, easing, spring values, z-index or layers, masks or mask geometry, keyframes, transitions, camera moves, renderer instructions, and code of any kind (Rust, React, CSS, HTML, SVG).

If you want something "bigger", "faster" or "more dramatic", express it through `purpose`, `relationship` and `energy`.

## 10. Checklist

1. `"version": "0.2"` (or legacy `"0.1"` without v0.2 features), a filename-safe `title`, at least one beat.
2. Every beat has `purpose`, `statement` and `primary`, and every subject has `kind`.
3. Only enum values listed in this guide. Only field names listed in this guide.
4. `object` subjects use a concrete snake_case noun for `asset` and a short `meaning`.
5. compare/contrast beats have a `secondary`.
6. Statements are short titles (≤ 8 words, ≤ 12 for reveal/explain); spoken sentences go in `narration`. Numbers are formatted exactly as they should appear.
7. `keyword` is set to a single word when `meaning` is long.
8. At most one or two `impact` beats, usually a closing `reveal`.
9. No key appears twice in the same object (the engine rejects duplicate keys, and schema validators can't detect them).
10. The file validates against `schema/creative-intent-v0.2.schema.json`.

## 11. Complete example

```json
{
  "version": "0.2",
  "title": "bridge_example",
  "format": "vertical",
  "beats": [
    {
      "purpose": "emphasize",
      "statement": "The ferry crossing took forty minutes",
      "narration": "For years, getting across the river meant a forty-minute ferry ride. Every single day.",
      "primary": { "kind": "number", "value": "40 min", "meaning": "crossing time" },
      "energy": "calm",
      "continuity": "carry_primary",
      "keyword": "ferry"
    },
    {
      "purpose": "contrast",
      "statement": "Then the bridge opened",
      "primary": { "kind": "number", "value": "40 min", "meaning": "crossing time" },
      "secondary": { "kind": "number", "value": "6 min", "meaning": "by bridge" },
      "relationship": "replace",
      "keyword": "bridge"
    },
    {
      "purpose": "reveal",
      "statement": "Crossing the river is now almost seven times faster.",
      "primary": { "kind": "number", "value": "6.7x", "meaning": "faster crossing" },
      "energy": "impact",
      "keyword": "faster"
    }
  ]
}
```

What happens: "40 min" appears as the hero of beat 1 and carries into beat 2, where it becomes the side that gets replaced by "6 min". Beat 3 arrives with an accent-color transition and reveals "6.7x".

To compile and render your file, see `docs/CLI_CONSUMER_GUIDE.md`.
