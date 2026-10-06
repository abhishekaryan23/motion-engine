//! Tool definitions, generated per profile (plan §4, §9).
//!
//! Same tool names everywhere; the INPUT SCHEMA differs: weak sees the lite
//! story and one tone word, creator and operator also see the full
//! CreativeIntent v0.2, a StyleProfile object and production options. Schemas
//! are compact and `$ref`-free (small models and Gemini-style function calling
//! handle them best); their enum lists are taken from motion-core's own
//! schemars output, so they cannot drift from the engine. Guidance lives in
//! MCP resources and prompts ([`crate::resources`]), not here.
//!
//! Budgets (`tests/tool_budget.rs`): weak ≤ 4.8k characters for all four
//! tools, creator ≤ 12k, operator ≤ 24k.

use motion_core::audio::MusicWord;
use serde_json::{json, Map, Value};

use crate::profile::{self, Profile};

/// One `tools/list` entry, transport-independent (the server converts it to
/// `rmcp::model::Tool`).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDef {
    pub name: &'static str,
    pub description: String,
    pub input_schema: Value,
}

impl ToolDef {
    /// The entry as sent on the wire (`name`, `description`, `inputSchema`).
    pub fn wire_json(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": self.input_schema,
        })
    }
}

/// Characters of the serialized tool list (what the budgets measure).
pub fn defs_chars(defs: &[ToolDef]) -> usize {
    defs.iter()
        .map(|d| d.wire_json().to_string().chars().count())
        .sum()
}

/// The tool list of a profile, in [`profile::tool_names`] order.
pub fn tool_defs(p: Profile, allow_scene_edits: bool) -> Vec<ToolDef> {
    profile::tool_names(p, allow_scene_edits)
        .into_iter()
        .map(|name| tool_def(p, name))
        .collect()
}

/// The definition of one tool in a profile (also for tools the profile does
/// not list, which callers should not do).
pub fn tool_def(p: Profile, name: &'static str) -> ToolDef {
    let (description, input_schema) = match name {
        profile::MAKE_VIDEO => (MAKE_VIDEO_DESC, make_video_schema(p)),
        profile::REVISE_VIDEO => (REVISE_VIDEO_DESC, revise_video_schema(p)),
        profile::GET_VIDEO => (
            "Status and result of a video job; waits up to wait_s seconds for it to finish. One call with wait_s waits: never poll in a loop.",
            object(
                [
                    ("job", string()),
                    ("wait_s", json!({"type": "integer", "minimum": 0, "maximum": 300})),
                ],
                &["job"],
            ),
        ),
        profile::FIND_ASSETS => (
            "Which library pictures exist for these words (one call for many words). Words with no picture are shown as text.",
            object(
                [
                    ("words", json!({"type": "array", "items": {"type": "string"}})),
                    ("k", json!({"type": "integer", "minimum": 1, "maximum": 5})),
                ],
                &["words"],
            ),
        ),
        profile::VIEW_FRAMES => (
            "Look at a video job: one contact sheet of every beat at its reading moment (default), or chosen beats or times. Review it before revise_video.",
            object(
                [
                    ("job", string()),
                    ("beats", json!({"type": "array", "items": {"type": "integer", "minimum": 1}})),
                    ("times", json!({"type": "array", "items": {"type": "number", "minimum": 0}})),
                    ("sheet", json!({"type": "boolean"})),
                ],
                &["job"],
            ),
        ),
        profile::EXPLORE_STYLES => (
            "Render k art-direction variants of one story as one sheet, one row per variant. Pass the chosen variant's art and variety to make_video options.",
            object(
                [
                    (
                        "story",
                        json!({"type": "object", "description": "the same story object as make_video"}),
                    ),
                    ("k", json!({"type": "integer", "minimum": 2, "maximum": 4})),
                    ("style", style_ref(p)),
                    ("brand", brand_ref()),
                    ("format", format_schema()),
                ],
                &["story"],
            ),
        ),
        profile::LIST_OPTIONS => (
            "Looks, asset families (with their medium), music beds and tones, with their best uses.",
            object(
                [(
                    "topic",
                    json!({"type": "string", "enum": ["all", "looks", "families", "music", "tones"]}),
                )],
                &[],
            ),
        ),
        profile::PLAN_ASSETS => (
            "Pictures a job is missing (shown as text), as image-generator prompts in the video's style. Make them elsewhere, then pass the files as file: pictures.",
            object([("job", string())], &["job"]),
        ),
        profile::RENDER_FRAME => (
            "Render one frame of a job as a PNG.",
            job_frame_schema(),
        ),
        profile::INSPECT_FRAME => (
            "The resolved state of one frame of a job as JSON (debugging).",
            job_frame_schema(),
        ),
        profile::QA_REPORT => (
            "The full QA report of a job (motion, layout, speech, audio).",
            object([("job", string())], &["job"]),
        ),
        profile::INGEST_ASSETS => (
            "Ingest generated images for a prompt set into an AssetManifest, with preflight QA.",
            file_schema(),
        ),
        profile::MATTE => (
            "Cut the subject out of a photo on-device (Apple Vision).",
            file_schema(),
        ),
        profile::MUSIC_INDEX => (
            "Analyse a music track (tempo, beats, downbeats) into a MusicPlan.",
            file_schema(),
        ),
        profile::SFX_INDEX => (
            "Measure a curated sound pack into an SFX library.",
            file_schema(),
        ),
        profile::REFERENCE_EVIDENCE => (
            "Analyse a reference video into an evidence bundle (samples, contact sheet).",
            file_schema(),
        ),
        profile::APPLY_SCENE_PATCH => (
            "Expert escape hatch: apply a JSON merge patch to a job's MotionScene, then validate, QA and re-render.",
            object(
                [("job", string()), ("patch", json!({"type": "object"}))],
                &["job", "patch"],
            ),
        ),
        other => panic!("unknown tool {other}"),
    };
    ToolDef {
        name,
        description: description.to_string(),
        input_schema,
    }
}

const MAKE_VIDEO_DESC: &str = "Create a narrated motion-graphics video (MP4) from a short story. Write 2-12 beats; each beat has what the narrator says and, optionally, a picture noun, a number, a list, a comparison or layers. The engine designs, voices, animates, renders and checks it. Fixable problems are fixed automatically and listed in `changed`.";

const REVISE_VIDEO_DESC: &str = "Change an earlier video: edit, add or remove beats, or change the style. Unchanged narration reuses the recorded voice. Returns a new job.";

fn string() -> Value {
    json!({"type": "string"})
}

fn str_enum(values: &[String]) -> Value {
    json!({"type": "string", "enum": values})
}

fn object<const N: usize>(props: [(&str, Value); N], required: &[&str]) -> Value {
    let mut map = Map::new();
    for (k, v) in props {
        map.insert(k.to_string(), v);
    }
    let mut o = json!({"type": "object", "properties": map});
    if !required.is_empty() {
        o["required"] = json!(required);
    }
    o
}

fn job_frame_schema() -> Value {
    object(
        [
            ("job", string()),
            ("frame", json!({"type": "integer", "minimum": 0})),
            ("time_s", json!({"type": "number", "minimum": 0})),
        ],
        &["job"],
    )
}

fn file_schema() -> Value {
    object(
        [
            (
                "path",
                json!({"type": "string", "description": "inside the repository or output/"}),
            ),
            ("extra", json!({"type": "object"})),
        ],
        &["path"],
    )
}

fn format_schema() -> Value {
    json!({"type": "string", "enum": ["vertical", "square", "wide"]})
}

/// (0.23) `take`, every profile: another version of the same story.
fn take_schema() -> Value {
    json!({"type": "integer", "minimum": 0, "maximum": 99, "description": "another version of the same story: 1, 2, 3 …"})
}

/// (0.23) `music`, every profile: the mood of the music bed (the engine reads
/// it from the story when it is `auto` or left out).
fn music_schema() -> Value {
    json!({"type": "string", "enum": MusicWord::ALL.map(MusicWord::as_str), "description": "music mood; auto reads the story, none = no music"})
}

fn mode_schema() -> Value {
    json!({"type": "string", "enum": ["auto", "check", "render"], "description": "check: plan only, no render"})
}

/// The lite beat (every profile).
fn lite_beat() -> Value {
    let s = string;
    let mut beat = object(
        [
            (
                "say",
                json!({"type": "string", "description": "narrator's words, 8-30"}),
            ),
            (
                "show",
                json!({"type": "string", "description": "on-screen title, max 6 words"}),
            ),
            (
                "picture",
                json!({"type": "string", "description": "a noun like rocket, or file:folder/image.jpg"}),
            ),
            ("picture2", s()),
            (
                "number",
                json!({"type": "string", "description": "e.g. $381B, 40%"}),
            ),
            (
                "meaning",
                json!({"type": "string", "description": "what it stands for, 1-3 words"}),
            ),
            ("keyword", s()),
            (
                "list",
                json!({"type": "array", "items": {"type": "string"}, "minItems": 3, "maxItems": 6}),
            ),
            (
                "compare",
                object(
                    [
                        ("a", s()),
                        ("b", s()),
                        (
                            "how",
                            json!({"type": "string", "enum": ["separate", "grow", "compress", "replace"]}),
                        ),
                    ],
                    &["a", "b"],
                ),
            ),
            (
                "change",
                object(
                    [("what", s()), ("from", s()), ("to", s())],
                    &["what", "from", "to"],
                ),
            ),
            (
                "layers",
                object(
                    [
                        (
                            "names",
                            json!({"type": "array", "items": {"type": "string"}, "minItems": 2, "maxItems": 6}),
                        ),
                        ("focus", s()),
                    ],
                    &["names"],
                ),
            ),
            ("energy", str_enum(&intent_enums().energy)),
        ],
        &["say"],
    );
    if let Some(o) = beat.as_object_mut() {
        o.insert(
            "description".into(),
            json!("say, plus at most one of: picture, number, list, compare, change, layers"),
        );
    }
    beat
}

/// Enum values taken from motion-core's generated schemas.
#[derive(Debug, Clone)]
pub struct IntentEnums {
    pub purpose: Vec<String>,
    pub relationship: Vec<String>,
    pub energy: Vec<String>,
    pub continuity: Vec<String>,
    pub format: Vec<String>,
    pub subject_kinds: Vec<String>,
}

fn def_values(root: &Value, def: &str) -> Vec<String> {
    let d = &root["$defs"][def];
    if let Some(e) = d["enum"].as_array() {
        return e
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
    }
    d["oneOf"]
        .as_array()
        .map(|alts| {
            alts.iter()
                .filter_map(|a| {
                    a["const"]
                        .as_str()
                        .or_else(|| a["properties"]["kind"]["const"].as_str())
                        .map(String::from)
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn intent_enums() -> IntentEnums {
    let root = serde_json::to_value(schemars::schema_for!(motion_core::CreativeIntent))
        .expect("intent schema");
    IntentEnums {
        purpose: def_values(&root, "Purpose"),
        relationship: def_values(&root, "Relationship"),
        energy: def_values(&root, "Energy"),
        continuity: def_values(&root, "Continuity"),
        format: def_values(&root, "Format"),
        subject_kinds: def_values(&root, "Subject"),
    }
}

/// Tone words (the StyleProfile `tone` enum).
pub fn tones() -> Vec<String> {
    let root = serde_json::to_value(schemars::schema_for!(motion_core::StyleProfile))
        .expect("style schema");
    def_values(&root, "Tone")
}

/// The StyleProfile as a flat object of enums (creator and up). `family`
/// (no visible effect) is left out, and so is `brand`: every profile gets it
/// as the top-level `brand` argument ([`brand_schema`]).
fn style_object() -> Value {
    let root = serde_json::to_value(schemars::schema_for!(motion_core::StyleProfile))
        .expect("style schema");
    let mut props = Map::new();
    if let Some(p) = root["properties"].as_object() {
        for (name, prop) in p {
            if name == "family" || name == "brand" {
                continue;
            }
            let schema = match prop["$ref"].as_str() {
                Some(r) => str_enum(&def_values(&root, r.trim_start_matches("#/$defs/"))),
                None => json!({"type": "integer", "minimum": 0}),
            };
            props.insert(name.clone(), schema);
        }
    }
    json!({"type": "object", "properties": props})
}

/// (0.21) Brand colours (every profile; a colour, not a pixel or a timing).
fn brand_schema() -> Value {
    let hex = || json!({"type": "string"});
    let mut o = object(
        [
            ("primary", hex()),
            ("secondary", hex()),
            ("background", hex()),
            ("text", hex()),
        ],
        &[],
    );
    o["description"] = json!("brand colours as hex, e.g. {\"primary\": \"#0A84FF\"}");
    o
}

/// `brand` on tools other than `make_video`.
fn brand_ref() -> Value {
    json!({"type": "object", "description": "brand colours, as in make_video"})
}

fn style_schema(p: Profile) -> Value {
    let tone = json!({"type": "string", "enum": tones(), "description": "tone; default auto"});
    if p.full_control() {
        json!({"anyOf": [tone, style_object()]})
    } else {
        tone
    }
}

/// `style` on tools other than `make_video`: the same values, without
/// repeating the StyleProfile object (it is spelled out once, on make_video).
fn style_ref(p: Profile) -> Value {
    let tone = json!({"type": "string", "enum": tones()});
    if p.full_control() {
        json!({"anyOf": [tone, {"type": "object", "description": "a StyleProfile, as in make_video"}]})
    } else {
        tone
    }
}

/// One subject of a full-intent beat (compact, `$ref`-free).
fn subject_schema() -> Value {
    json!({
        "type": "object",
        "description": "kind decides the fields: phrase|number {value, meaning}; object {asset (snake_case noun), value, meaning}; collection {items: 2-6 phrase|number|object subjects, meaning}; state_change {entity, from, to, meaning}; derived_metric {numerator {value, meaning}, denominator {value, meaning}, format percent|decimal|per_thousand, meaning}; layers {layers [{name, note, boundary}], focus, meaning} (primary only)",
        "properties": {"kind": str_enum(&intent_enums().subject_kinds)},
        "required": ["kind"],
    })
}

/// A full CreativeIntent v0.2 beat (creator and up).
fn intent_beat() -> Value {
    let e = intent_enums();
    object(
        [
            ("purpose", str_enum(&e.purpose)),
            (
                "statement",
                json!({"type": "string", "description": "short on-screen title, 3-8 words"}),
            ),
            (
                "narration",
                json!({"type": "string", "description": "what the narrator says, 8-30 words; all beats are one continuous script"}),
            ),
            ("primary", subject_schema()),
            ("secondary", subject_schema()),
            ("relationship", str_enum(&e.relationship)),
            ("energy", str_enum(&e.energy)),
            ("continuity", str_enum(&e.continuity)),
            ("keyword", string()),
        ],
        &["purpose", "statement", "primary"],
    )
}

fn story_schema(p: Profile) -> Value {
    let beats =
        |items: Value| json!({"type": "array", "minItems": 2, "maxItems": 12, "items": items});
    if p.full_control() {
        json!({
            "type": "object",
            "description": "A lite story {title, beats: [lite beats]} or a full CreativeIntent v0.2 {version: \"0.2\", title, format, beats: [intent beats]}. Guide: motionengine://guide/intent",
            "properties": {
                "version": {"type": "string", "enum": ["0.2"]},
                "title": string(),
                "format": str_enum(&intent_enums().format),
                "beats": beats(json!({"anyOf": [lite_beat(), intent_beat()]})),
            },
            "required": ["beats"],
        })
    } else {
        json!({
            "type": "object",
            "properties": {"title": string(), "beats": beats(lite_beat())},
            "required": ["beats"],
        })
    }
}

fn options_schema(p: Profile) -> Value {
    let looks: Vec<String> = std::iter::once("auto".to_string())
        .chain(
            motion_core::compiler::art_direction::Look::ALL
                .iter()
                .map(|l| l.name().to_string()),
        )
        .collect();
    let mut o = object(
        [
            ("art", str_enum(&looks)),
            (
                "families",
                json!({"type": "array", "items": {"type": "string"}, "description": "asset families in order (list_options)"}),
            ),
            (
                "music",
                json!({"type": "string", "description": "auto, none or a bed id (list_options)"}),
            ),
            ("captions", json!({"type": "boolean"})),
            (
                "aspect",
                json!({"type": "string", "description": "story, portrait, square, landscape or a:b"}),
            ),
            (
                "variety",
                json!({"type": ["string", "integer"], "description": "auto, off or a seed from explore_styles"}),
            ),
        ],
        &[],
    );
    if p == Profile::Operator {
        let props = o["properties"].as_object_mut().expect("properties");
        props.insert(
            "tts_model".into(),
            json!({"type": "string", "enum": ["auto", "gemini", "mai", "deepgram"], "description": "paid voices are opt-in"}),
        );
        props.insert("keep_frames".into(), json!({"type": "boolean"}));
    }
    o
}

fn make_video_schema(p: Profile) -> Value {
    let mut s = object(
        [
            ("story", story_schema(p)),
            ("style", style_schema(p)),
            ("brand", brand_schema()),
            ("format", format_schema()),
            (
                "assets",
                json!({"type": "string", "description": "folder of the user's own images"}),
            ),
            ("take", take_schema()),
            ("music", music_schema()),
            ("mode", mode_schema()),
            (
                "strict",
                json!({"type": "boolean", "description": "never auto-fix"}),
            ),
        ],
        &["story"],
    );
    if p.full_control() {
        let props = s["properties"].as_object_mut().expect("properties");
        props.insert("options".into(), options_schema(p));
        props.insert(
            "describe".into(),
            json!({"type": "boolean", "description": "labels of the user's images"}),
        );
        if p == Profile::Operator {
            props.insert("force".into(), json!({"type": "boolean"}));
        }
    }
    s
}

fn revise_video_schema(p: Profile) -> Value {
    let change = json!({
        "type": "object",
        "description": "beat (1-based) and the fields to set (say, show, picture, number, list ...), or remove: true, or insert_after: N with a new beat's fields",
        "properties": {
            "beat": {"type": "integer", "minimum": 1},
            "remove": {"type": "boolean"},
            "insert_after": {"type": "integer", "minimum": 0},
        },
    });
    let mut s = object(
        [
            ("job", string()),
            ("changes", json!({"type": "array", "items": change})),
            ("style", style_ref(p)),
            ("brand", brand_ref()),
            ("take", take_schema()),
            ("music", music_schema()),
        ],
        &["job"],
    );
    if p.full_control() {
        let props = s["properties"].as_object_mut().expect("properties");
        props.insert(
            "patch".into(),
            json!({"type": "object", "description": "JSON merge patch on the stored story"}),
        );
        props.insert(
            "options".into(),
            json!({"type": "object", "description": "production options, as in make_video"}),
        );
        props.insert("format".into(), format_schema());
        props.insert("mode".into(), mode_schema());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_come_from_motion_core() {
        let e = intent_enums();
        assert!(e.purpose.contains(&"reveal".to_string()));
        assert!(e.subject_kinds.contains(&"layers".to_string()));
        assert_eq!(e.energy, ["calm", "building", "impact"]);
        assert!(tones().contains(&"cinematic".to_string()));
        assert!(style_object()["properties"]["motion_language"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("parallax")));
    }

    #[test]
    fn every_listed_tool_has_a_definition() {
        for p in [Profile::Weak, Profile::Creator, Profile::Operator] {
            for d in tool_defs(p, true) {
                assert!(!d.description.is_empty());
                assert_eq!(d.input_schema["type"], "object", "{}", d.name);
            }
        }
    }
}
