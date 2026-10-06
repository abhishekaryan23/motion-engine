//! Guidance on demand (plan §10): MCP resources and prompts, never tool
//! descriptions.
//!
//! Resources, every profile:
//! * `motionengine://guide/lite-story`: ≤ 2,400 characters (≈ 600 tokens), the
//!   beat fields, the one-structure rule, the title rule and 3 examples
//!   (`docs/mcp/lite_story_guide.md`, embedded);
//! * `motionengine://guide/full`: `docs/AI_AUTHORING_GUIDE.md`, read from the
//!   repository at request time;
//! * `motionengine://examples/{name}`: the lite stories in `examples-lite/`
//!   (embedded; every profile) and, for creator and operator, the
//!   `examples/public/*.intent.json` files as `intent-<name>` (read from the
//!   repository);
//! * `motionengine://assets{?query}`: the same lookup as `find_assets`;
//! * `motionengine://styles`: the tone words, each with its best use.
//!
//! Creator and operator also get `motionengine://guide/intent` (the full
//! CreativeIntent and StyleProfile guide, `docs/mcp/intent_guide.md`) and
//! `motionengine://options` (the `list_options` data).
//!
//! Prompts: `make_explainer(topic, length_s?)` and `make_reel(topic, tone?)`
//! for every profile (a lite-story skeleton and the 5 key rules); creator and
//! operator also `direct_video(topic, look?, length_s?)` (a full-intent
//! skeleton, the check → render → view_frames → revise routine and what to
//! look for on a contact sheet).

use serde_json::{Map, Value};

use crate::creator;
use crate::pictures::PictureIndex;
use crate::profile::{Profile, ServerConfig};
use crate::schema;

/// A resource or resource template.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceDef {
    /// URI, or URI template for [`templates`].
    pub uri: String,
    pub name: String,
    pub description: String,
    pub mime: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PromptArg {
    pub name: &'static str,
    pub description: &'static str,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PromptDef {
    pub name: &'static str,
    pub description: String,
    pub arguments: Vec<PromptArg>,
}

/// What reading a resource may consult.
pub struct ResourceCtx<'a> {
    pub config: &'a ServerConfig,
    pub pictures: &'a PictureIndex,
}

pub const URI_LITE_GUIDE: &str = "motionengine://guide/lite-story";
pub const URI_FULL_GUIDE: &str = "motionengine://guide/full";
pub const URI_INTENT_GUIDE: &str = "motionengine://guide/intent";
pub const URI_OPTIONS: &str = "motionengine://options";
pub const URI_STYLES: &str = "motionengine://styles";
pub const EXAMPLES_PREFIX: &str = "motionengine://examples/";
pub const ASSETS_URI: &str = "motionengine://assets";
const EXAMPLES_TEMPLATE: &str = "motionengine://examples/{name}";
const ASSETS_TEMPLATE: &str = "motionengine://assets{?query}";

/// The lite guide's size limit (characters, ≈ 600 tokens).
pub const LITE_GUIDE_MAX_CHARS: usize = 2_400;
/// The intent guide's size limit (characters).
pub const INTENT_GUIDE_MAX_CHARS: usize = 8_000;

const MARKDOWN: &str = "text/markdown";
const TEXT: &str = "text/plain";
const JSON: &str = "application/json";

const LITE_GUIDE: &str = include_str!("../../../docs/mcp/lite_story_guide.md");
const INTENT_GUIDE: &str = include_str!("../../../docs/mcp/intent_guide.md");

/// A lite story, as the arguments of a `make_video` call.
pub struct LiteExample {
    pub name: &'static str,
    pub description: &'static str,
    pub text: &'static str,
}

/// The lite examples: every file of `crates/motion-mcp/examples-lite/` (a test
/// checks the folder and this table agree).
pub const LITE_EXAMPLES: [LiteExample; 4] = [
    LiteExample {
        name: "number-reveal",
        description: "Lite story, 4 beats: a picture, a figure, a comparison and a closing number (documentary).",
        text: include_str!("../examples-lite/number-reveal.json"),
    },
    LiteExample {
        name: "list",
        description: "Lite story, 3 beats: a list of 5 pictures, a before-and-after change and a closing picture (technical).",
        text: include_str!("../examples-lite/list.json"),
    },
    LiteExample {
        name: "compare",
        description: "Lite story, 3 beats: a figure, a replace comparison and a closing number (editorial).",
        text: include_str!("../examples-lite/compare.json"),
    },
    LiteExample {
        name: "layers",
        description: "Lite story, 4 beats: one stack of three layers, a different focus in each beat (cinematic).",
        text: include_str!("../examples-lite/layers.json"),
    },
];

/// `examples/public/<stem>.intent.json` files offered to creator and operator
/// as `intent-<stem>` (a test checks the folder and this table agree).
pub const PUBLIC_EXAMPLES: [(&str, &str); 7] = [
    (
        "collection-accumulate",
        "Full CreativeIntent: a collection whose items add up into a total.",
    ),
    (
        "derived-metric",
        "Full CreativeIntent: a ratio the engine calculates from two numbers.",
    ),
    (
        "layers",
        "Full CreativeIntent: stacked layers, the same list in every beat, a focus each.",
    ),
    (
        "minimal-contrast",
        "Legacy v0.1 intent: the smallest contrast beat.",
    ),
    (
        "minimal-emphasize",
        "Legacy v0.1 intent: the smallest emphasize beat.",
    ),
    (
        "state-change",
        "Full CreativeIntent: one thing changing state, from and to.",
    ),
    (
        "three-beat-story",
        "Legacy v0.1 intent: a three-beat story (emphasize, contrast, reveal).",
    ),
];

/// The prefix of a public example's name.
const PUBLIC_PREFIX: &str = "intent-";

/// The best use of each tone word, one line (taken from the `Tone` doc comments
/// in `motion-core/src/style.rs`; ≤ 15 words each, tested).
pub const TONE_USES: [(&str, &str); 9] = [
    (
        "auto",
        "Classic warm editorial collage; the default when unsure.",
    ),
    (
        "editorial",
        "Magazine design: serif and sans contrast, generous space, measured pacing.",
    ),
    (
        "technical",
        "Structured grid, condensed and mono type, precise motion; data and how-it-works.",
    ),
    (
        "playful",
        "Bold type, big colour fields, energetic motion; light, friendly topics.",
    ),
    (
        "street",
        "The picture owns the frame, hard cuts, camera shake; music, sports, street culture.",
    ),
    (
        "documentary",
        "Vox-style evidence documents, stamps, highlights; facts, money, history, science.",
    ),
    (
        "hype",
        "Words and pictures slam in on the voice; short, punchy scripts and openers.",
    ),
    (
        "studio",
        "One big cutout on a bold colour disc; creator and brand stories.",
    ),
    (
        "cinematic",
        "3D depth with a flying camera; big ideas: technology, science, space, the future.",
    ),
];

fn def(uri: &str, name: &str, description: &str, mime: &'static str) -> ResourceDef {
    ResourceDef {
        uri: uri.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        mime,
    }
}

/// Fixed resources of a profile (including the concrete example URIs).
pub fn list(profile: Profile) -> Vec<ResourceDef> {
    let mut out = vec![
        def(
            URI_LITE_GUIDE,
            "lite-story guide",
            "How to write a story for make_video: beat fields, one structure per beat, the title rule, 3 examples (about 600 tokens).",
            MARKDOWN,
        ),
        def(
            URI_FULL_GUIDE,
            "full authoring guide",
            "The complete CreativeIntent v0.2 authoring guide (docs/AI_AUTHORING_GUIDE.md).",
            MARKDOWN,
        ),
    ];
    if profile.full_control() {
        out.push(def(
            URI_INTENT_GUIDE,
            "CreativeIntent and StyleProfile guide",
            "The full CreativeIntent v0.2 beat (purpose, statement vs narration, subjects, relationship, continuity, layers) and the StyleProfile dials.",
            MARKDOWN,
        ));
        out.push(def(
            URI_OPTIONS,
            "production options",
            "Looks, asset families, music beds, tones and their best uses (the list_options data).",
            JSON,
        ));
    }
    out.push(def(
        URI_STYLES,
        "tone words",
        "The tone words for `style`, each with its best use.",
        TEXT,
    ));
    for e in &LITE_EXAMPLES {
        out.push(def(
            &format!("{EXAMPLES_PREFIX}{}", e.name),
            &format!("example {}", e.name),
            e.description,
            JSON,
        ));
    }
    if profile.full_control() {
        for (stem, description) in PUBLIC_EXAMPLES {
            let name = format!("{PUBLIC_PREFIX}{stem}");
            out.push(def(
                &format!("{EXAMPLES_PREFIX}{name}"),
                &format!("example {name}"),
                description,
                JSON,
            ));
        }
    }
    out
}

/// Resource templates of a profile.
pub fn templates(profile: Profile) -> Vec<ResourceDef> {
    let examples = if profile.full_control() {
        "A ready story: a lite example (number-reveal, list, compare, layers) or a full intent (intent-<name>)."
    } else {
        "A ready story for make_video: number-reveal, list, compare or layers."
    };
    vec![
        def(EXAMPLES_TEMPLATE, "example story", examples, JSON),
        def(
            ASSETS_TEMPLATE,
            "library pictures",
            "Which library pictures exist for these words (comma or space separated), as JSON; same as find_assets.",
            JSON,
        ),
    ]
}

/// The text of a resource (`None` = unknown URI or not in this profile).
pub fn read(uri: &str, ctx: &ResourceCtx) -> Option<String> {
    let full = ctx.config.profile.full_control();
    match uri {
        URI_LITE_GUIDE => return Some(LITE_GUIDE.to_string()),
        URI_FULL_GUIDE => return Some(full_guide(ctx.config)),
        URI_STYLES => return Some(styles_text()),
        URI_INTENT_GUIDE if full => return Some(INTENT_GUIDE.to_string()),
        URI_OPTIONS if full => {
            return serde_json::to_string_pretty(&creator::options_json(ctx.config)).ok()
        }
        _ => {}
    }
    if let Some(name) = uri.strip_prefix(EXAMPLES_PREFIX) {
        return example(name, ctx.config);
    }
    let query = match uri.strip_prefix(ASSETS_URI)? {
        "" => "",
        rest => rest.strip_prefix('?')?,
    };
    Some(assets_text(query, ctx.pictures))
}

/// `docs/AI_AUTHORING_GUIDE.md` of the repository.
fn full_guide(config: &ServerConfig) -> String {
    let path = config.repo.join("docs/AI_AUTHORING_GUIDE.md");
    std::fs::read_to_string(&path).unwrap_or_else(|_| {
        "The full guide (docs/AI_AUTHORING_GUIDE.md) is not in this repository checkout. \
         Use motionengine://guide/lite-story."
            .to_string()
    })
}

/// The tone words, each with its best use, one per line.
fn styles_text() -> String {
    let mut out = String::from(
        "Tone words for `style` (one per video; auto = the classic warm editorial collage):\n",
    );
    for tone in schema::tones() {
        let best = TONE_USES
            .iter()
            .find(|(t, _)| *t == tone)
            .map(|(_, u)| *u)
            .unwrap_or("See the StyleProfile schema.");
        out.push_str(&format!("{tone}: {best}\n"));
    }
    out
}

/// An example story by name (`None` = no such example in this profile).
fn example(name: &str, config: &ServerConfig) -> Option<String> {
    if let Some(e) = LITE_EXAMPLES.iter().find(|e| e.name == name) {
        return Some(e.text.to_string());
    }
    let stem = name.strip_prefix(PUBLIC_PREFIX)?;
    let plain = !stem.is_empty()
        && stem
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
    if !config.profile.full_control() || !plain {
        return None;
    }
    let path = config
        .repo
        .join("examples/public")
        .join(format!("{stem}.intent.json"));
    std::fs::read_to_string(path).ok()
}

/// The `motionengine://assets{?query}` answer: `find_assets` as compact JSON.
fn assets_text(query: &str, pictures: &PictureIndex) -> String {
    let mut words: Vec<String> = Vec::new();
    for pair in query.split('&') {
        let Some(("query", value)) = pair.split_once('=') else {
            continue;
        };
        for w in percent_decode(value)
            .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
            .map(str::trim)
            .filter(|w| !w.is_empty())
        {
            let w: String = w.chars().take(40).collect();
            if !words.contains(&w) && words.len() < 24 {
                words.push(w);
            }
        }
    }
    if words.is_empty() {
        return r#"{"hint":"add ?query=word1,word2 to look up pictures"}"#.to_string();
    }
    let found = pictures.find(&words, 3);
    serde_json::to_string(&found).unwrap_or_else(|_| "{}".to_string())
}

/// Percent-decoding of a URI query value (`+` is a space; bad escapes stay).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = bytes
                    .get(i + 1..i + 3)
                    .and_then(|h| std::str::from_utf8(h).ok())
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(b) => {
                        out.push(b);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn arg(name: &'static str, description: &'static str, required: bool) -> PromptArg {
    PromptArg {
        name,
        description,
        required,
    }
}

/// Prompts of a profile.
pub fn prompts(profile: Profile) -> Vec<PromptDef> {
    let mut out = vec![
        PromptDef {
            name: "make_explainer",
            description: "Plan and make a narrated explainer video about a topic with make_video."
                .to_string(),
            arguments: vec![
                arg("topic", "what the video is about", true),
                arg("length_s", "target length in seconds (default 30)", false),
            ],
        },
        PromptDef {
            name: "make_reel",
            description:
                "Plan and make a short, punchy vertical reel about a topic with make_video."
                    .to_string(),
            arguments: vec![
                arg("topic", "what the reel is about", true),
                arg("tone", "tone word for style (default hype)", false),
            ],
        },
    ];
    out.push(PromptDef {
        name: "review_story",
        description: "Check a story before rendering: pictures named when shown, spoken keywords and numbers, complete comparisons, one continuous script."
            .to_string(),
        arguments: vec![arg("topic", "what the video is about", false)],
    });
    if profile.full_control() {
        out.push(PromptDef {
            name: "direct_video",
            description: "Direct a video with a full CreativeIntent, then review it on a contact sheet and revise: check, render, view_frames, revise_video."
                .to_string(),
            arguments: vec![
                arg("topic", "what the video is about", true),
                arg("look", "art-direction look from list_options (default auto)", false),
                arg("length_s", "target length in seconds (default 36)", false),
            ],
        });
    }
    out
}

/// Spoken seconds a beat holds, on average (the beat count is length ÷ this).
const SECONDS_PER_BEAT: f64 = 6.0;
const DEFAULT_EXPLAINER_S: f64 = 30.0;
const DEFAULT_DIRECT_S: f64 = 36.0;
const DEFAULT_REEL_BEATS: usize = 4;
/// Spoken words in a beat of average length (2.5 words a second).
const WORDS_PER_BEAT: usize = 15;

/// Beats for a target length: length ÷ 6 s, clamped to 2-12.
pub fn beats_for(length_s: f64) -> usize {
    ((length_s / SECONDS_PER_BEAT).round() as usize).clamp(2, 12)
}

const RULES: &str = "Rules:
1. One structure per beat: picture, number (+ meaning), list, compare, change or layers. Never two.
2. say is 8-30 words, written to be spoken aloud. All beats are read as ONE continuous script: each line carries on from the last and never restarts the topic.
3. show (optional) is an on-screen title of 6 words or fewer: the topic, not the answer.
4. Pictures are concrete lowercase nouns you could photograph (piggy_bank, rocket), not ideas. Unsure? Call find_assets once with several words.
5. Call make_video once. For any change afterwards call revise_video with the job id and only the beats that change; never start a new story for a fix.
6. Name every picture in its own beat's say, and when you compare several things end on one beat that shows them all (a list with their numbers).";

/// (0.22) The story checks a model runs before rendering (the review step):
/// each one is a mistake seen in real consumer stories.
const REVIEW: &str = r#"Review your story before rendering. For each beat check:
1. Every picture is named in that beat's say (no flag or object before the narrator mentions it).
2. A keyword is a word the narrator actually says in that beat; otherwise leave it out.
3. Numbers on screen are the numbers said ("$127" with "a hundred and twenty-seven dollars", not "one twenty-six").
4. Things that relate are shown related: two things compared side by side (each with its number); steps or a countdown open each item's say with its rank ("Number three:").
5. If the story compares N things, all N appear, and one beat near the end shows them all (a list of the pictures with their numbers).
6. The beats read as one continuous script that builds to a payoff: the last beat lands the answer or asks the viewer a question.
Then call make_video with mode check, fix every finding it lists, and call it again with mode auto."#;

/// A prompt's text, as one user message (`None` = unknown prompt, or not in
/// this profile).
pub fn get_prompt(profile: Profile, name: &str, args: &Map<String, Value>) -> Option<String> {
    let topic = topic_of(args);
    match name {
        "make_explainer" => {
            let length = number_arg(args, "length_s").unwrap_or(DEFAULT_EXPLAINER_S);
            Some(explainer_prompt(&topic, length))
        }
        "make_reel" => Some(reel_prompt(&topic, word_arg(args, "tone").as_deref())),
        "review_story" => Some(REVIEW.to_string()),
        "direct_video" if profile.full_control() => {
            let length = number_arg(args, "length_s").unwrap_or(DEFAULT_DIRECT_S);
            Some(direct_prompt(
                &topic,
                word_arg(args, "look").as_deref(),
                length,
            ))
        }
        _ => None,
    }
}

/// A text argument: one line, trimmed, at most 200 characters.
fn text_arg(args: &Map<String, Value>, name: &str) -> Option<String> {
    let raw = match args.get(name)? {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let line: String = line.chars().take(200).collect();
    (!line.is_empty()).then_some(line)
}

/// A one-word argument (a tone or a look): letters, digits, `_` and `-` only,
/// at most 40 characters, so it can sit inside a JSON string.
fn word_arg(args: &Map<String, Value>, name: &str) -> Option<String> {
    let word: String = text_arg(args, name)?
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        .take(40)
        .collect();
    (!word.is_empty()).then_some(word)
}

fn topic_of(args: &Map<String, Value>) -> String {
    text_arg(args, "topic").unwrap_or_else(|| "(your topic)".to_string())
}

/// A positive number argument (prompt arguments arrive as strings).
fn number_arg(args: &Map<String, Value>, name: &str) -> Option<f64> {
    let n = match args.get(name)? {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    (n.is_finite() && n > 0.0).then_some(n)
}

/// Beat shapes cycled through the middle of a lite skeleton (neighbouring beats
/// should not look alike).
fn lite_middle(i: usize) -> &'static str {
    const SHAPES: [&str; 5] = [
        r#"{"say": "<spoken line that carries on and names the figure>", "number": "<figure, e.g. 40% or $2M>", "meaning": "<what it counts>"}"#,
        r#"{"say": "<spoken line that carries on and walks through the items>", "list": ["<item>", "<item>", "<item>"]}"#,
        r#"{"say": "<spoken line that sets two things against each other>", "compare": {"a": "<thing>", "b": "<thing>", "how": "grow"}}"#,
        r#"{"say": "<spoken line that carries on>", "show": "<title, 6 words or fewer>", "picture": "<concrete noun>"}"#,
        r#"{"say": "<spoken line about a before and after>", "change": {"what": "<thing>", "from": "<before>", "to": "<after>"}}"#,
    ];
    SHAPES[i % SHAPES.len()]
}

/// The `make_video` arguments for `n` beats: a picture hook, varied middle
/// beats, a closing number.
fn lite_skeleton(n: usize, style: &str) -> String {
    let mut beats: Vec<String> = Vec::with_capacity(n);
    for i in 0..n {
        let beat = if i == 0 {
            r#"{"say": "<hook: one or two spoken sentences that open the topic>", "show": "<title, 6 words or fewer>", "picture": "<concrete noun>"}"#
        } else if i + 1 == n {
            r#"{"say": "<the takeaway, spoken>", "show": "<title, 6 words or fewer>", "number": "<figure>", "meaning": "<what it counts>", "energy": "impact"}"#
        } else {
            lite_middle(i - 1)
        };
        beats.push(format!("    {beat}"));
    }
    format!(
        "{{\"story\": {{\"title\": \"<short name>\", \"beats\": [\n{}\n  ]}}, \"style\": \"{style}\", \"format\": \"vertical\"}}",
        beats.join(",\n")
    )
}

fn explainer_prompt(topic: &str, length_s: f64) -> String {
    let n = beats_for(length_s);
    let shown = if length_s > 12.0 * SECONDS_PER_BEAT {
        12.0 * SECONDS_PER_BEAT
    } else {
        length_s.max(2.0 * SECONDS_PER_BEAT)
    };
    format!(
        "Make a narrated explainer video about: {topic}\n\n\
         Target: about {shown:.0} s = {n} beats of about {WORDS_PER_BEAT} spoken words each. \
         Call make_video once with a story shaped like this. Replace every <...> and keep the mix of structures:\n\n\
         {skeleton}\n\n\
         {RULES}\n\n\
         Style: documentary for facts, money and history; technical for how things work; cinematic for science, space and the future; editorial for ideas; otherwise auto. \
         More help: resource motionengine://guide/lite-story.",
        skeleton = lite_skeleton(n, "<tone word>"),
    )
}

fn reel_prompt(topic: &str, tone: Option<&str>) -> String {
    let tones = schema::tones();
    let (style, note) = match tone.map(str::to_lowercase) {
        Some(t) if tones.contains(&t) => (t, String::new()),
        Some(t) => (
            "hype".to_string(),
            format!(
                "\n(\"{t}\" is not a tone word; using hype. Tone words: {}.)",
                tones.join(", ")
            ),
        ),
        None => ("hype".to_string(), String::new()),
    };
    format!(
        "Make a short, punchy vertical reel about: {topic}\n\n\
         Open on a hook in beat 1 (a surprising claim or a question), keep every line quick, end on one punchline. \
         About {n} beats. Call make_video once with a story shaped like this. Replace every <...>:\n\n\
         {skeleton}{note}\n\n\
         {RULES}\n\n\
         More help: resource motionengine://guide/lite-story; tone words: motionengine://styles.",
        n = DEFAULT_REEL_BEATS,
        skeleton = lite_skeleton(DEFAULT_REEL_BEATS, &style),
    )
}

/// Beat shapes of a full-intent skeleton.
fn intent_middle(i: usize) -> &'static str {
    const SHAPES: [&str; 5] = [
        r#"{"purpose": "emphasize", "statement": "<topic title, 3-8 words>", "narration": "<spoken line that carries on and names the figure>", "primary": {"kind": "number", "value": "<figure>", "meaning": "<what it counts>"}}"#,
        r#"{"purpose": "compare", "statement": "<A against B>", "narration": "<spoken line that sets them against each other>", "primary": {"kind": "phrase", "value": "<A>"}, "secondary": {"kind": "phrase", "value": "<B>"}, "relationship": "grow"}"#,
        r#"{"purpose": "emphasize", "statement": "<topic title>", "narration": "<spoken line that carries on>", "primary": {"kind": "collection", "meaning": "<what they are together>", "items": [{"kind": "object", "asset": "<noun>"}, {"kind": "object", "asset": "<noun>"}, {"kind": "object", "asset": "<noun>"}]}, "relationship": "accumulate"}"#,
        r#"{"purpose": "explain", "statement": "<topic title>", "narration": "<spoken line about a before and after>", "primary": {"kind": "state_change", "entity": "<thing>", "from": "<before>", "to": "<after>"}}"#,
        r#"{"purpose": "emphasize", "statement": "<topic title>", "narration": "<spoken line that carries on>", "primary": {"kind": "object", "asset": "<concrete noun>", "meaning": "<1-3 words>"}}"#,
    ];
    SHAPES[i % SHAPES.len()]
}

fn intent_skeleton(n: usize, look: &str) -> String {
    let mut beats: Vec<String> = Vec::with_capacity(n);
    for i in 0..n {
        let beat = if i == 0 {
            r#"{"purpose": "emphasize", "statement": "<hook title, 3-8 words>", "narration": "<spoken hook, 8-30 words>", "primary": {"kind": "object", "asset": "<concrete noun>", "meaning": "<1-3 words>"}, "energy": "calm"}"#
        } else if i + 1 == n {
            r#"{"purpose": "reveal", "statement": "<closing title>", "narration": "<the takeaway, spoken>", "primary": {"kind": "number", "value": "<figure>", "meaning": "<what it counts>"}, "energy": "impact"}"#
        } else {
            intent_middle(i - 1)
        };
        beats.push(format!("    {beat}"));
    }
    format!(
        "{{\"story\": {{\"version\": \"0.2\", \"title\": \"<short_name>\", \"format\": \"vertical\", \"beats\": [\n{}\n  ]}},\n \"style\": {{\"tone\": \"<tone word>\"}}, \"options\": {{\"art\": \"{look}\"}}, \"mode\": \"check\"}}",
        beats.join(",\n")
    )
}

fn direct_prompt(topic: &str, look: Option<&str>, length_s: f64) -> String {
    let n = beats_for(length_s);
    let look_line = match look {
        Some(l) => format!("\nLook: {l} (options.art; other looks: list_options)."),
        None => String::new(),
    };
    format!(
        "Direct a video about: {topic}{look_line}\n\n\
         1. Plan. Call make_video with mode \"check\" (nothing renders) and a full CreativeIntent of about {n} beats, shaped like this. Replace every <...>; use the structure that fits each beat and vary them; the narration is one continuous script (8-30 words a beat):\n\n\
         {skeleton}\n\n\
         Fields and rules: motionengine://guide/intent. Looks, families, music: motionengine://options.\n\
         2. Read the plan and fix what it names. Then make the same call without mode: it renders and returns a job.\n\
         3. Look. Call view_frames with the job: one contact sheet, one frame per beat at its reading moment.\n\
         4. On the sheet check, beat by beat:\n\
         - the main thing is in focus and the largest thing on screen;\n\
         - no text over faces;\n\
         - no empty or near-empty frames;\n\
         - no two beats look the same (vary structures and pictures);\n\
         - titles are short (6 words or fewer) and name the topic, not the answer.\n\
         5. Revise. Call revise_video with the job and only the beats that fail (a new job; unchanged narration reuses the voice), then view_frames again. Two rounds are usually enough. Never start a new story for a fix.",
        skeleton = intent_skeleton(n, look.unwrap_or("auto")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_values_decode() {
        assert_eq!(percent_decode("berry%20rocket"), "berry rocket");
        assert_eq!(percent_decode("a+b%2Cc"), "a b,c");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz%4"), "%zz%4");
        assert_eq!(percent_decode("caf%C3%A9"), "café");
    }

    #[test]
    fn beats_follow_the_length() {
        assert_eq!(beats_for(30.0), 5);
        assert_eq!(beats_for(36.0), 6);
        assert_eq!(beats_for(1.0), 2);
        assert_eq!(beats_for(600.0), 12);
        assert_eq!(beats_for(f64::NAN), 2);
    }

    #[test]
    fn skeleton_has_the_requested_beats() {
        for n in 2..=12 {
            let lite = lite_skeleton(n, "documentary");
            assert_eq!(lite.matches("\"say\"").count(), n);
            let intent = intent_skeleton(n, "auto");
            assert_eq!(intent.matches("\"purpose\"").count(), n);
        }
    }

    #[test]
    fn tone_uses_stay_short() {
        for (tone, use_) in TONE_USES {
            assert!(use_.split_whitespace().count() <= 15, "{tone}: {use_}");
        }
    }
}
