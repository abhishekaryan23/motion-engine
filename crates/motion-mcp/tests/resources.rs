//! Guidance resources and prompts (plan §10): through the public functions
//! per profile, and once through the stdio server with the rmcp client (fake
//! engine, as in `server_stdio.rs`).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rmcp::model::{CallToolRequestParams, GetPromptRequestParams, ReadResourceRequestParams};
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::{json, Map, Value};

use motion_core::intent::Format;
use motion_core::CreativeIntent;
use motion_mcp::lite::{self, LiteStory, Pictures};
use motion_mcp::pictures::PictureIndex;
use motion_mcp::profile::{Profile, ServerConfig};
use motion_mcp::resources::{
    self, ResourceCtx, EXAMPLES_PREFIX, INTENT_GUIDE_MAX_CHARS, LITE_EXAMPLES,
    LITE_GUIDE_MAX_CHARS, PUBLIC_EXAMPLES, URI_FULL_GUIDE, URI_INTENT_GUIDE, URI_LITE_GUIDE,
    URI_OPTIONS, URI_STYLES,
};
use motion_mcp::schema;

const PROFILES: [Profile; 3] = [Profile::Weak, Profile::Creator, Profile::Operator];

/// The repository these tests run in.
fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn config(profile: Profile) -> ServerConfig {
    ServerConfig::new(profile, repo())
}

/// Read `uri` as `profile` against this repository.
fn read_as(profile: Profile, uri: &str) -> Option<String> {
    let config = config(profile);
    let pictures = PictureIndex::load(&config.assets);
    resources::read(
        uri,
        &ResourceCtx {
            config: &config,
            pictures: &pictures,
        },
    )
}

fn args(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

fn uris(profile: Profile) -> Vec<String> {
    resources::list(profile)
        .into_iter()
        .map(|d| d.uri)
        .collect()
}

#[test]
fn weak_lists_the_guides_and_examples_but_nothing_for_creators() {
    let listed = uris(Profile::Weak);
    for want in [URI_LITE_GUIDE, URI_FULL_GUIDE, URI_STYLES] {
        assert!(listed.iter().any(|u| u == want), "{want} not listed");
    }
    for e in &LITE_EXAMPLES {
        assert!(listed.contains(&format!("{EXAMPLES_PREFIX}{}", e.name)));
    }
    // No creator resource is listed, and none can be read.
    assert!(!listed
        .iter()
        .any(|u| u == URI_INTENT_GUIDE || u == URI_OPTIONS));
    assert!(!listed.iter().any(|u| u.contains("intent-")), "{listed:?}");
    assert_eq!(read_as(Profile::Weak, URI_INTENT_GUIDE), None);
    assert_eq!(read_as(Profile::Weak, URI_OPTIONS), None);
    for (stem, _) in PUBLIC_EXAMPLES {
        let uri = format!("{EXAMPLES_PREFIX}intent-{stem}");
        assert_eq!(read_as(Profile::Weak, &uri), None, "{uri}");
    }
    // And the weak prompts do not include direct_video.
    let names: Vec<&str> = resources::prompts(Profile::Weak)
        .iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, ["make_explainer", "make_reel", "review_story"]);
    assert_eq!(
        resources::get_prompt(
            Profile::Weak,
            "direct_video",
            &args(&[("topic", json!("x"))])
        ),
        None
    );
}

#[test]
fn creator_and_operator_also_list_the_intent_guide_options_and_public_examples() {
    for profile in [Profile::Creator, Profile::Operator] {
        let listed = uris(profile);
        for want in [
            URI_LITE_GUIDE,
            URI_FULL_GUIDE,
            URI_STYLES,
            URI_INTENT_GUIDE,
            URI_OPTIONS,
        ] {
            assert!(listed.iter().any(|u| u == want), "{want} not listed");
        }
        for (stem, _) in PUBLIC_EXAMPLES {
            assert!(listed.contains(&format!("{EXAMPLES_PREFIX}intent-{stem}")));
        }
        let names: Vec<&str> = resources::prompts(profile).iter().map(|p| p.name).collect();
        assert_eq!(
            names,
            [
                "make_explainer",
                "make_reel",
                "review_story",
                "direct_video"
            ]
        );
    }
}

#[test]
fn every_listed_resource_reads_and_the_templates_cover_the_rest() {
    for profile in PROFILES {
        let defs = resources::list(profile);
        let mut seen = BTreeSet::new();
        for d in &defs {
            assert!(seen.insert(d.uri.clone()), "{} listed twice", d.uri);
            assert!(d.uri.starts_with("motionengine://"));
            assert!(!d.description.is_empty() && !d.name.is_empty());
            let text = read_as(profile, &d.uri)
                .unwrap_or_else(|| panic!("{} unreadable as {}", d.uri, profile.name()));
            assert!(!text.trim().is_empty(), "{} is empty", d.uri);
            if d.mime == "application/json" {
                serde_json::from_str::<Value>(&text)
                    .unwrap_or_else(|e| panic!("{} is not JSON: {e}", d.uri));
            }
        }
        let templates: Vec<String> = resources::templates(profile)
            .into_iter()
            .map(|d| d.uri)
            .collect();
        assert_eq!(
            templates,
            [
                "motionengine://examples/{name}",
                "motionengine://assets{?query}"
            ]
        );
    }
}

#[test]
fn the_lite_guide_is_short_and_says_the_rules() {
    for profile in PROFILES {
        let guide = read_as(profile, URI_LITE_GUIDE).unwrap();
        let chars = guide.chars().count();
        assert!(chars <= LITE_GUIDE_MAX_CHARS, "{chars} chars");
        // The beat fields, one structure per beat, the title rule, continuity of
        // the script and 3 examples.
        for want in [
            "say",
            "show",
            "6 words",
            "ONE continuous script",
            "One structure per beat",
            "keyword",
            "first words of say",
            "Number:",
            "List:",
            "Layers:",
        ] {
            assert!(guide.contains(want), "lite guide lacks {want:?}");
        }
    }
}

#[test]
fn the_intent_guide_covers_the_beat_and_stays_within_its_budget() {
    let guide = read_as(Profile::Creator, URI_INTENT_GUIDE).unwrap();
    assert!(guide.chars().count() <= INTENT_GUIDE_MAX_CHARS);
    for want in [
        "purpose",
        "statement",
        "narration",
        "relationship",
        "continuity",
        "energy",
        "state_change",
        "derived_metric",
        "collection",
        "layers",
        "StyleProfile",
        "motion_language",
    ] {
        assert!(guide.contains(want), "intent guide lacks {want:?}");
    }
    // Every enum word of the engine's own schema is spelled out in the guide.
    let e = schema::intent_enums();
    for word in e
        .purpose
        .iter()
        .chain(&e.relationship)
        .chain(&e.energy)
        .chain(&e.continuity)
        .chain(&e.subject_kinds)
    {
        assert!(guide.contains(word.as_str()), "intent guide lacks {word:?}");
    }
}

#[test]
fn the_full_guide_is_read_from_the_repository_at_request_time() {
    let guide = read_as(Profile::Weak, URI_FULL_GUIDE).unwrap();
    let on_disk = std::fs::read_to_string(repo().join("docs/AI_AUTHORING_GUIDE.md")).unwrap();
    assert_eq!(guide, on_disk);

    // A different checkout gives that checkout's text; a missing file gives a pointer.
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("resources-full-guide");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("docs")).unwrap();
    let config = ServerConfig::new(Profile::Weak, &root);
    let pictures = PictureIndex::load(&config.assets);
    let ctx = ResourceCtx {
        config: &config,
        pictures: &pictures,
    };
    let missing = resources::read(URI_FULL_GUIDE, &ctx).unwrap();
    assert!(
        missing.contains("motionengine://guide/lite-story"),
        "{missing}"
    );
    std::fs::write(root.join("docs/AI_AUTHORING_GUIDE.md"), "edited guide").unwrap();
    assert_eq!(
        resources::read(URI_FULL_GUIDE, &ctx).as_deref(),
        Some("edited guide")
    );
}

/// Every picture is a library picture (the mapping drops nothing for lack of one).
struct Everything;
impl Pictures for Everything {
    fn is_picture(&self, _noun: &str) -> bool {
        true
    }
}

fn words(s: &str) -> usize {
    s.split_whitespace().count()
}

#[test]
fn lite_examples_are_valid_stories_that_map_without_dropping_anything() {
    for e in &LITE_EXAMPLES {
        let text = read_as(Profile::Weak, &format!("{EXAMPLES_PREFIX}{}", e.name)).unwrap();
        assert_eq!(text, e.text);
        let call: Value = serde_json::from_str(&text).unwrap();
        assert!(call["style"].is_string(), "{}: no style", e.name);
        let story: LiteStory = serde_json::from_value(call["story"].clone())
            .unwrap_or_else(|err| panic!("{}: {err}", e.name));
        assert!(!story.title.is_empty());
        assert!((2..=12).contains(&story.beats.len()), "{}", e.name);
        for (i, beat) in story.beats.iter().enumerate() {
            let at = format!("{} beat {}", e.name, i + 1);
            assert!((8..=30).contains(&words(&beat.say)), "{at}: say words");
            if let Some(show) = &beat.show {
                assert!(words(show) <= 6, "{at}: show words");
            }
        }
        // One structure per beat: nothing is dropped when mapping.
        let mapped = lite::to_intent(&story, Format::Vertical, &Everything);
        assert_eq!(mapped.notes, Vec::<String>::new(), "{}", e.name);
        mapped
            .intent
            .validate()
            .unwrap_or_else(|errs| panic!("{}: {errs:?}", e.name));
    }
    // The four structures the plan names are all shown.
    let structures: BTreeSet<&str> = LITE_EXAMPLES
        .iter()
        .flat_map(|e| {
            let v: Value = serde_json::from_str(e.text).unwrap();
            let story: LiteStory = serde_json::from_value(v["story"].clone()).unwrap();
            story
                .beats
                .iter()
                .map(|b| b.structure().name())
                .collect::<Vec<_>>()
        })
        .collect();
    for want in ["number", "list", "compare", "layers"] {
        assert!(structures.contains(want), "no {want} example");
    }
}

#[test]
fn example_tables_match_their_folders() {
    let stems = |dir: PathBuf, suffix: &str| -> BTreeSet<String> {
        std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .flatten()
            .filter_map(|f| {
                let name = f.file_name().to_string_lossy().into_owned();
                name.strip_suffix(suffix).map(String::from)
            })
            .collect()
    };
    let lite = stems(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples-lite"),
        ".json",
    );
    let table: BTreeSet<String> = LITE_EXAMPLES.iter().map(|e| e.name.to_string()).collect();
    assert_eq!(lite, table, "examples-lite/ and LITE_EXAMPLES differ");

    let public = stems(repo().join("examples/public"), ".intent.json");
    let table: BTreeSet<String> = PUBLIC_EXAMPLES.iter().map(|(s, _)| s.to_string()).collect();
    assert_eq!(public, table, "examples/public and PUBLIC_EXAMPLES differ");
}

#[test]
fn public_examples_are_read_from_the_repository_for_creators() {
    for profile in [Profile::Creator, Profile::Operator] {
        for (stem, _) in PUBLIC_EXAMPLES {
            let uri = format!("{EXAMPLES_PREFIX}intent-{stem}");
            let text = read_as(profile, &uri).unwrap();
            let on_disk =
                std::fs::read_to_string(repo().join(format!("examples/public/{stem}.intent.json")))
                    .unwrap();
            assert_eq!(text, on_disk);
            let intent = CreativeIntent::from_json(&text).unwrap();
            intent
                .validate()
                .unwrap_or_else(|errs| panic!("{stem}: {errs:?}"));
        }
    }
}

#[test]
fn unknown_and_escaping_uris_are_none() {
    for profile in PROFILES {
        for uri in [
            "",
            "motionengine://",
            "motionengine://nope",
            "motionengine://guide",
            "motionengine://guide/lite-story/",
            "motionengine://guide/lite-story?x=1",
            "motionengine://styles/x",
            "motionengine://examples/",
            "motionengine://examples/missing",
            "motionengine://examples/number-reveal.json",
            "motionengine://examples/intent-",
            "motionengine://examples/intent-missing",
            "motionengine://examples/intent-../../../Cargo",
            "motionengine://examples/intent-..%2F..%2FCargo",
            "motionengine://examples/intent-Layers",
            "motionengine://assetsx",
            "motionengine://assets/x",
            "https://example.com/guide/lite-story",
            "file:///etc/passwd",
            "motionengine://options/x",
        ] {
            assert_eq!(read_as(profile, uri), None, "{uri} as {}", profile.name());
        }
    }
}

#[test]
fn styles_list_every_tone_with_a_short_use() {
    for profile in PROFILES {
        let text = read_as(profile, URI_STYLES).unwrap();
        for tone in schema::tones() {
            let line = text
                .lines()
                .find_map(|l| l.strip_prefix(&format!("{tone}: ")))
                .unwrap_or_else(|| panic!("tone {tone} missing:\n{text}"));
            assert!(words(line) >= 3, "{tone}: {line}");
            assert!(words(line) <= 15, "{tone}: {line} ({} words)", words(line));
        }
    }
}

#[test]
fn assets_template_answers_like_find_assets() {
    for profile in PROFILES {
        let text = read_as(
            profile,
            "motionengine://assets?query=rocket%20berry,piggy_bank",
        )
        .unwrap();
        let found: Map<String, Value> = serde_json::from_str(&text).unwrap();
        for word in ["rocket", "berry", "piggy_bank"] {
            let options = found
                .get(word)
                .unwrap_or_else(|| panic!("{word} missing in {text}"));
            assert!(options.as_array().is_some_and(|a| a.len() <= 3), "{text}");
        }
        assert!(!text.contains('\n'), "compact JSON");
        // The same words, found twice or in another form, give the same answer.
        assert_eq!(
            read_as(
                profile,
                "motionengine://assets?query=rocket+berry+piggy_bank+rocket"
            ),
            Some(text)
        );
        // No query: a hint, not an error.
        for uri in [
            "motionengine://assets",
            "motionengine://assets?query=",
            "motionengine://assets?other=x",
        ] {
            let hint = read_as(profile, uri).unwrap();
            assert!(hint.contains("?query="), "{uri}: {hint}");
        }
    }
}

#[test]
fn options_are_pretty_json_for_creators() {
    for profile in [Profile::Creator, Profile::Operator] {
        let text = read_as(profile, URI_OPTIONS).unwrap();
        serde_json::from_str::<Value>(&text).unwrap();
    }
}

#[test]
fn prompts_carry_the_topic_the_skeleton_and_the_five_rules() {
    let topic = "how vaccines train the immune system";
    for profile in PROFILES {
        for name in ["make_explainer", "make_reel"] {
            let text = resources::get_prompt(profile, name, &args(&[("topic", json!(topic))]))
                .unwrap_or_else(|| panic!("{name} missing"));
            assert!(text.contains(topic), "{name}: topic missing");
            for want in [
                "One structure per beat",
                "8-30 words",
                "ONE continuous script",
                "6 words or fewer",
                "concrete lowercase nouns",
                "make_video once",
                "revise_video",
                "\"beats\"",
                "\"say\"",
            ] {
                assert!(text.contains(want), "{name} lacks {want:?}");
            }
        }
        // Arguments are optional where documented; a missing topic is a placeholder.
        let text = resources::get_prompt(profile, "make_reel", &Map::new()).unwrap();
        assert!(text.contains("(your topic)"));
        assert_eq!(resources::get_prompt(profile, "nope", &Map::new()), None);
    }
}

#[test]
fn explainers_are_length_aware() {
    let beats = |a: &[(&str, Value)]| -> usize {
        resources::get_prompt(Profile::Weak, "make_explainer", &args(a))
            .unwrap()
            .matches("\"say\"")
            .count()
    };
    let t = ("topic", json!("tides"));
    assert_eq!(beats(std::slice::from_ref(&t)), 5, "default 30 s");
    assert_eq!(beats(&[t.clone(), ("length_s", json!(60))]), 10);
    assert_eq!(
        beats(&[t.clone(), ("length_s", json!("24"))]),
        4,
        "string argument"
    );
    assert_eq!(beats(&[t.clone(), ("length_s", json!(5))]), 2, "at least 2");
    assert_eq!(
        beats(&[t.clone(), ("length_s", json!(600))]),
        12,
        "at most 12"
    );
    assert_eq!(
        beats(&[t.clone(), ("length_s", json!("soon"))]),
        5,
        "garbage = default"
    );
    let text = resources::get_prompt(
        Profile::Weak,
        "make_explainer",
        &args(&[t.clone(), ("length_s", json!(60))]),
    )
    .unwrap();
    assert!(text.contains("about 60 s = 10 beats"), "{text}");
}

#[test]
fn reels_take_a_tone() {
    let text = resources::get_prompt(
        Profile::Weak,
        "make_reel",
        &args(&[("topic", json!("espresso")), ("tone", json!("Street"))]),
    )
    .unwrap();
    assert!(text.contains("\"style\": \"street\""), "{text}");
    let text = resources::get_prompt(
        Profile::Weak,
        "make_reel",
        &args(&[("topic", json!("espresso")), ("tone", json!("loud"))]),
    )
    .unwrap();
    assert!(text.contains("\"style\": \"hype\""));
    assert!(text.contains("\"loud\" is not a tone word"));
    for tone in schema::tones() {
        assert!(text.contains(&tone), "tone words listed: {tone}");
    }
}

#[test]
fn direct_video_has_the_full_intent_skeleton_and_the_review_routine() {
    for profile in [Profile::Creator, Profile::Operator] {
        let text = resources::get_prompt(
            profile,
            "direct_video",
            &args(&[
                ("topic", json!("the deep sea")),
                ("look", json!("cinematic3d")),
            ]),
        )
        .unwrap();
        assert!(text.contains("the deep sea"));
        assert!(text.contains("\"art\": \"cinematic3d\""), "look forwarded");
        for want in [
            "\"version\": \"0.2\"",
            "\"purpose\"",
            "\"narration\"",
            "mode \"check\"",
            "view_frames",
            "revise_video",
            "contact sheet",
            "reading moment",
            "in focus and the largest",
            "no text over faces",
            "no empty or near-empty frames",
            "no two beats look the same",
            "titles are short",
            "motionengine://guide/intent",
        ] {
            assert!(text.contains(want), "direct_video lacks {want:?}");
        }
        // check comes before the render, the render before the look, then the revision.
        let at = |s: &str| text.find(s).unwrap_or_else(|| panic!("{s}"));
        assert!(at("mode \"check\"") < at("renders and returns a job"));
        assert!(at("renders and returns a job") < at("Call view_frames"));
        assert!(at("Call view_frames") < at("Call revise_video"));
        let default = resources::get_prompt(profile, "direct_video", &Map::new()).unwrap();
        assert!(default.contains("\"art\": \"auto\""));
        // 36 s by default: 6 beats.
        assert_eq!(default.matches("\"purpose\"").count(), 6);
    }
}

// ---- --tts-model and the stdio server -------------------------------------

#[derive(Clone, Default)]
struct Client;
impl ClientHandler for Client {}

type Session = RunningService<RoleClient, Client>;

/// A temporary repository (`assets/` and a guide) and the fake engine's run log.
struct Env {
    repo: PathBuf,
    counter: PathBuf,
}

impl Env {
    fn new(name: &str) -> Env {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("resources-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("repo");
        std::fs::create_dir_all(repo.join("assets")).unwrap();
        std::fs::create_dir_all(repo.join("docs")).unwrap();
        std::fs::write(
            repo.join("docs/AI_AUTHORING_GUIDE.md"),
            "# Guide of this checkout\n",
        )
        .unwrap();
        Env {
            repo,
            counter: root.join("reel_runs.txt"),
        }
    }

    fn command(&self, args: &[&str]) -> tokio::process::Command {
        let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_engine.sh");
        let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_motion-mcp"));
        cmd.arg("--engine")
            .arg(fake)
            .arg("--repo")
            .arg(&self.repo)
            .args(args)
            .env("FAKE_ENGINE_COUNTER", &self.counter);
        cmd
    }

    async fn start(&self, args: &[&str]) -> Session {
        Client
            .serve(TokioChildProcess::new(self.command(args)).unwrap())
            .await
            .unwrap()
    }

    fn runs(&self) -> Vec<String> {
        std::fs::read_to_string(&self.counter)
            .map(|t| t.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }
}

async fn read_text(session: &Session, uri: &str) -> String {
    let result = session
        .read_resource(ReadResourceRequestParams::new(uri))
        .await
        .unwrap_or_else(|e| panic!("{uri}: {e}"));
    assert_eq!(result.contents.len(), 1);
    match &result.contents[0] {
        rmcp::model::ResourceContents::TextResourceContents { text, uri: got, .. } => {
            assert_eq!(got, uri);
            text.clone()
        }
        other => panic!("not text: {other:?}"),
    }
}

#[tokio::test]
async fn the_stdio_server_serves_guidance_per_profile() {
    let env = Env::new("stdio");
    for profile in PROFILES {
        let session = if profile == Profile::Weak {
            env.start(&[]).await
        } else {
            env.start(&["--profile", profile.name()]).await
        };
        let name = profile.name();

        // Resources: the same list as the public function, with their MIME types.
        let listed = session.list_all_resources().await.unwrap();
        let want = resources::list(profile);
        assert_eq!(listed.len(), want.len(), "{name}");
        for (got, want) in listed.iter().zip(&want) {
            assert_eq!(got.uri, want.uri);
            assert_eq!(got.mime_type.as_deref(), Some(want.mime), "{}", want.uri);
        }
        let creator_only = listed.iter().any(|r| r.uri == URI_INTENT_GUIDE);
        assert_eq!(creator_only, profile.full_control(), "{name}");

        // Templates.
        let templates = session.list_all_resource_templates().await.unwrap();
        let tpl: Vec<&str> = templates.iter().map(|t| t.uri_template.as_str()).collect();
        assert_eq!(
            tpl,
            [
                "motionengine://examples/{name}",
                "motionengine://assets{?query}"
            ]
        );

        // Reads: static guide, the repository's guide, an example, assets, styles.
        let guide = read_text(&session, URI_LITE_GUIDE).await;
        assert!(guide.chars().count() <= LITE_GUIDE_MAX_CHARS);
        assert_eq!(
            read_text(&session, URI_FULL_GUIDE).await,
            "# Guide of this checkout\n"
        );
        let example = read_text(&session, "motionengine://examples/compare").await;
        assert!(serde_json::from_str::<Value>(&example).is_ok());
        let assets = read_text(&session, "motionengine://assets?query=rocket").await;
        assert!(serde_json::from_str::<Value>(&assets).is_ok());
        assert!(read_text(&session, URI_STYLES)
            .await
            .contains("cinematic: "));

        // Profile-gated and unknown resources are errors.
        let denied = session
            .read_resource(ReadResourceRequestParams::new(URI_INTENT_GUIDE))
            .await;
        assert_eq!(denied.is_ok(), profile.full_control(), "{name}");
        let unknown = session
            .read_resource(ReadResourceRequestParams::new("motionengine://nope"))
            .await;
        assert!(unknown.is_err());

        // Prompts.
        let prompts = session.list_all_prompts().await.unwrap();
        let names: Vec<&str> = prompts.iter().map(|p| p.name.as_str()).collect();
        let want: Vec<&str> = resources::prompts(profile).iter().map(|p| p.name).collect();
        assert_eq!(names, want, "{name}");
        let reel = session
            .get_prompt(
                GetPromptRequestParams::new("make_reel")
                    .with_arguments(args(&[("topic", json!("espresso"))])),
            )
            .await
            .unwrap();
        assert_eq!(reel.messages.len(), 1);
        let text = &reel.messages[0].content.as_text().unwrap().text;
        assert!(text.contains("espresso") && text.contains("make_video"));
        let direct = session
            .get_prompt(
                GetPromptRequestParams::new("direct_video")
                    .with_arguments(args(&[("topic", json!("espresso"))])),
            )
            .await;
        assert_eq!(direct.is_ok(), profile.full_control(), "{name}");
        if let Ok(direct) = direct {
            assert!(direct.messages[0]
                .content
                .as_text()
                .unwrap()
                .text
                .contains("view_frames"));
        }
        session.cancel().await.unwrap();
    }
}

#[tokio::test]
async fn tts_model_reaches_the_engine_and_is_limited_by_profile() {
    let env = Env::new("tts");
    // `say` (the offline macOS voice) is forwarded to every reel.
    let session = env.start(&["--tts-model", "say"]).await;
    let story = json!({"story": {"title": "tts check", "beats": [
        {"say": "Octopuses have three hearts and blue blood running through their bodies.", "show": "Three hearts"},
        {"say": "Two of the hearts pump blood through the gills while one feeds the body.", "number": "2", "meaning": "gill hearts"}
    ]}});
    let result = tokio::time::timeout(
        Duration::from_secs(120),
        session.call_tool(
            CallToolRequestParams::new("make_video".to_string())
                .with_arguments(story.as_object().unwrap().clone()),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        result.structured_content.as_ref().unwrap()["status"],
        "done"
    );
    let runs = env.runs();
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert!(runs[0].contains("--tts-model say"), "{}", runs[0]);
    session.cancel().await.unwrap();

    // Weak and creator accept only auto or say; anything else exits 2 before serving.
    let exit = |args: &[&str]| -> (Option<i32>, String) {
        let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_engine.sh");
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_motion-mcp"))
            .arg("--engine")
            .arg(fake)
            .arg("--repo")
            .arg(&env.repo)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if started.elapsed() > Duration::from_secs(20) {
                let _ = child.kill();
                panic!("motion-mcp {args:?} did not exit");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let mut stderr = String::new();
        if let Some(mut pipe) = child.stderr.take() {
            let _ = std::io::Read::read_to_string(&mut pipe, &mut stderr);
        }
        (status.code(), stderr)
    };
    for args in [
        &["--tts-model", "gemini"][..],
        &["--tts-model", "fish"],
        &["--tts-model", "nonsense"],
        &["--profile", "weak", "--tts-model", "deepgram"],
        &["--profile", "creator", "--tts-model", "mai"],
    ] {
        let (code, stderr) = exit(args);
        assert_eq!(code, Some(2), "{args:?}: {stderr}");
        assert!(stderr.contains("--tts-model"), "{stderr}");
    }
    // The operator may name any voice model (the engine judges it); flags are refused.
    for args in [
        &["--profile", "operator", "--tts-model", "--evil"][..],
        &["--profile", "operator", "--tts-model", ""],
        &["--profile", "operator", "--tts-model", "a b"],
    ] {
        let (code, stderr) = exit(args);
        assert_eq!(code, Some(2), "{args:?}: {stderr}");
    }
    let (code, _) = exit(&["--profile", "operator", "--tts-model", "gemini"]);
    assert_ne!(code, Some(2), "operator accepts gemini");
}
