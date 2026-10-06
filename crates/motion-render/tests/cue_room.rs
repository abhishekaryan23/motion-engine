//! (0.22) Word cues make room instead of giving up silently.
//!
//! The Wallet Atlas story (`tests/fixtures/wallet_atlas`, compiled as the CLI
//! does with `--art auto --variety auto` and the real measured voice-over):
//! the documentary look's figure card on beat 1 ("80%", a Value anchor)
//! used to stay where the builder put it, readable a second before
//! "eighty", because its EVOLVE underline left the rigid cue shift no room.
//! Now the card's arrival waits for its word and the speech QA passes.
//! A group that cannot wait at all is reported as `cue_dropped`.

use std::path::{Path, PathBuf};

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::{ArtMode, Look};
use motion_core::compiler::speech_plan::{anchor_time, WORD_CUE_LEAD};
use motion_core::compiler::taste_rules::display_text;
use motion_core::compiler::{
    compile_with_report, ApproxMeasure, AssetLibrary, CompileOptions, CompileWarning, FontSet,
    WARN_CUE_DROPPED,
};
use motion_core::intent::CreativeIntent;
use motion_core::scene::MotionProject;
use motion_core::speech::{
    estimate_syllables, repair, statement_tokens, RevealRole, SpeechMap, SpeechSentence,
    SpeechWord, REVEAL_LEAD_MAX, SPEECH_VERSION,
};
use motion_core::style::StyleProfile;
use motion_render::reveal_qa::{first_readable_frames, scene_prefix, GroupRef};
use motion_render::speech_qa::{speech_report, CheckStatus, SpeechQaReport};
use motion_render::FontMeasure;

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn fixture(name: &str) -> String {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/wallet_atlas"
    ))
    .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn spoken_lines(intent: &CreativeIntent) -> Vec<String> {
    intent
        .beats
        .iter()
        .map(|b| display_text(b, true).spoken)
        .collect()
}

/// The CLI's `--variety auto` seed (FNV-1a over the title and statements).
fn story_seed(intent: &CreativeIntent) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut eat = |s: &str| {
        for b in s.bytes().chain([0u8]) {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    };
    eat(&intent.title);
    for b in &intent.beats {
        eat(&b.statement);
    }
    h
}

/// The Wallet Atlas story, its style and its repaired voice-over.
fn world_trade() -> (CreativeIntent, StyleProfile, SpeechMap) {
    let intent = CreativeIntent::from_json(&fixture("world_trade.intent.json")).expect("intent");
    let style: StyleProfile =
        serde_json::from_str(&fixture("world_trade.style.json")).expect("style");
    let map = SpeechMap::from_json(&fixture("world_trade.speech.json")).expect("speech");
    let speech = repair(&map, &spoken_lines(&intent)).0;
    (intent, style, speech)
}

/// Compile as `motion-engine compile --art auto --variety auto --speech`
/// does: the look's fonts measure the text.
fn compile_cli(
    intent: &CreativeIntent,
    style: &StyleProfile,
    speech: Option<&SpeechMap>,
) -> (MotionProject, Vec<CompileWarning>) {
    let assets = repo().join("assets");
    let font_paths: Vec<PathBuf> = FontSet::for_style(style)
        .faces
        .iter()
        .map(|f| assets.join(f.path))
        .collect();
    let measure =
        FontMeasure::with_fonts(font_paths.iter().map(PathBuf::as_path)).expect("load fonts");
    let opts = CompileOptions {
        art: Some(ArtMode::Auto),
        variety: Some(story_seed(intent)),
        speech: speech.cloned(),
        ..CompileOptions::default()
    };
    compile_with_report(
        intent,
        style,
        None,
        &AssetLibrary::new(&assets),
        &measure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile")
}

fn report(project: &MotionProject, speech: &SpeechMap) -> SpeechQaReport {
    speech_report(project, speech, Path::new("."), None, None, None, None).expect("speech qa")
}

fn status(r: &SpeechQaReport, name: &str) -> CheckStatus {
    r.checks
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no check {name}"))
        .status
}

#[test]
fn the_documentary_figure_lands_on_its_number() {
    let (intent, style, speech) = world_trade();
    let (project, warnings) = compile_cli(&intent, &style, Some(&speech));
    let art = project.project.art.as_ref().expect("art record");
    assert_eq!(
        art.look,
        Look::Dossier.name(),
        "tone documentary picks the dossier look"
    );

    // The figure card is beat 1's Value anchor ("80%").
    let anchors = art.reveals.get("beat_1").expect("beat 1 anchors");
    let figure = anchors
        .iter()
        .find(|a| a.role == RevealRole::Value)
        .expect("a Value anchor on beat 1");
    assert_eq!(figure.group, "figure");
    assert_eq!(figure.words, vec!["80%".to_string()]);

    // "eighty" as measured on the voice-over (scene 1 starts at 0).
    let scene = project
        .scenes
        .iter()
        .find(|s| s.id == "beat_1")
        .expect("beat 1 scene");
    let spoken: Vec<(String, f64)> = speech
        .words_in(&speech.sentences[0])
        .iter()
        .map(|w| (w.text.clone(), w.start - scene.start_seconds))
        .collect();
    let word = anchor_time(&spoken, &["80%"]).expect("\"80%\" is said");
    assert!((word - 1.8475).abs() < 1e-6, "eighty at {word}");

    // A word cue now moves the card onto its word, whole (not clamped).
    let cue = project
        .project
        .speech
        .as_ref()
        .expect("speech record")
        .word_cues
        .iter()
        .find(|c| c.beat == 0 && c.group == "figure")
        .expect("a word cue for the figure card");
    assert_eq!(cue.role, Some(RevealRole::Value));
    assert_eq!(cue.word, "eighty percent");
    assert!(!cue.clamped, "{cue:?}");
    assert!((cue.to - (word - WORD_CUE_LEAD)).abs() < 1e-3, "{cue:?}");
    assert!(
        warnings.iter().all(|w| w.code != WARN_CUE_DROPPED),
        "{warnings:?}"
    );

    // Readable (the speech QA's own rule, at frame resolution) within
    // REVEAL_LEAD_MAX before "eighty", and not after it ends.
    let group = GroupRef {
        scene: scene.id.clone(),
        prefix: scene_prefix(scene).expect("prefix"),
        group: "figure".into(),
    };
    let frame = first_readable_frames(&project, std::slice::from_ref(&group)).expect("frames")[0]
        .expect("the figure becomes readable");
    let at = motion_core::timeline::frame_time(project.canvas.fps, frame) - scene.start_seconds;
    assert!(
        at >= word - REVEAL_LEAD_MAX,
        "readable at {at}, eighty at {word}"
    );
    assert!(at <= 2.0475, "readable at {at}, after eighty ends");

    // The whole speech QA passes: no early reveal, and no layout finding.
    let r = report(&project, &speech);
    assert_eq!(
        status(&r, "reveal_before_speech"),
        CheckStatus::Pass,
        "{}",
        r.to_text()
    );
    assert_eq!(
        status(&r, "reveal_late"),
        CheckStatus::Pass,
        "{}",
        r.to_text()
    );
    assert_eq!(status(&r, "layout"), CheckStatus::Pass, "{}", r.to_text());
    assert!(r.passed(), "{}", r.to_text());
}

#[test]
fn without_a_voice_over_nothing_is_cued() {
    let (intent, style, _) = world_trade();
    let (project, warnings) = compile_cli(&intent, &style, None);
    assert!(project.project.speech.is_none());
    assert!(
        warnings.iter().all(|w| !w.code.starts_with("cue_")),
        "{warnings:?}"
    );
}

// ---------------------------------------------------------------------------
// cue_dropped through the compiler
// ---------------------------------------------------------------------------

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// One sentence per beat read at about 5 syllables per second, 0.3 s
/// between sentences (a real take's pause), repaired like the CLI's.
fn voice(intent: &CreativeIntent) -> SpeechMap {
    let lines = spoken_lines(intent);
    let (mut words, mut sentences) = (Vec::new(), Vec::new());
    let mut t = 0.4;
    for (beat, line) in lines.iter().enumerate() {
        let start = t;
        for tok in statement_tokens(line) {
            let len = 0.17 * estimate_syllables(&tok) as f64;
            words.push(SpeechWord {
                text: tok,
                start: round3(t),
                end: round3(t + len),
                confidence: 0.9,
            });
            t += len + 0.06;
        }
        sentences.push(SpeechSentence {
            beat,
            start: round3(start),
            end: round3(t),
        });
        t += 0.3;
    }
    let map = SpeechMap {
        recognised: Vec::new(),
        alignment: None,
        version: SPEECH_VERSION.to_string(),
        audio: "voice.wav".into(),
        sample_rate: 48_000,
        duration: round3(t + 1.0),
        provider: "fixture".into(),
        model: "fixture/model".into(),
        voice: "fixture".into(),
        words,
        sentences,
    };
    repair(&map, &lines).0
}

#[test]
fn a_stamp_that_cannot_wait_is_reported_and_only_warns() {
    // The space story in the documentary look: beat 3's stamp ("float") is
    // said in the beat's last second, after the read floor, and the stamp
    // is already planned as late as it can land.
    let read = |p: &str| std::fs::read_to_string(repo().join(p)).expect("read example");
    let intent =
        CreativeIntent::from_json(&read("examples/cinematic/space.intent.json")).expect("intent");
    let style: StyleProfile =
        serde_json::from_str(&read("examples/cinematic/space.style.json")).expect("style");
    let speech = voice(&intent);
    let opts = CompileOptions {
        art: Some(ArtMode::Force(Look::Dossier)),
        speech: Some(speech.clone()),
        ..CompileOptions::default()
    };
    let (project, warnings) = compile_with_report(
        &intent,
        &style,
        None,
        &AssetLibrary::new(repo().join("assets")),
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile");
    let dropped: Vec<&CompileWarning> = warnings
        .iter()
        .filter(|w| w.code == WARN_CUE_DROPPED)
        .collect();
    assert!(!dropped.is_empty(), "{warnings:?}");
    for w in &dropped {
        assert!(w.beat.is_some(), "{w:?}");
        assert!(
            w.message.contains("could not wait for") && w.message.contains("s early"),
            "{w:?}"
        );
    }
    let stamp = dropped
        .iter()
        .find(|w| w.beat == Some(2) && w.message.starts_with("\"stamp\""))
        .unwrap_or_else(|| panic!("beat 3 stamp not reported: {dropped:?}"));
    assert!(
        stamp
            .message
            .contains("\"float\" (no room before the beat ends)"),
        "{stamp:?}"
    );
    // No cue is recorded for it, and the speech QA only warns: the word
    // comes too late in its beat to be waited for.
    let cues = &project.project.speech.as_ref().expect("speech").word_cues;
    assert!(!cues.iter().any(|c| c.beat == 2 && c.group == "stamp"));
    let r = report(&project, &speech);
    let line = r
        .findings("reveal_before_speech")
        .iter()
        .find(|f| f.starts_with("beat 3: stamp"))
        .unwrap_or_else(|| panic!("{}", r.to_text()));
    assert!(
        line.ends_with("the word comes too late in the beat to wait for"),
        "{line}"
    );
}

#[test]
fn studio_pop_kinetic_words_fit_every_narrated_example() {
    // (0.22) The spoken-word rows: a word fitted to the full row width left
    // no room in the row clamp, and `f32::clamp` panicked (min > max).
    for name in [
        "cinematic/ai_age",
        "cinematic/ai_toolkit",
        "cinematic/space",
        "topics/ai_opener",
        "topics/vaccines",
        "topics/ai_learns",
        "voice_10/spotlight_effect",
        "facts/five_wait_what",
        "genre/face_your_fears",
        "genre/storytelling",
        "ocean/ocean_zones",
    ] {
        // voice_10 stories have no style file: the default style.
        let read = |ext: &str| {
            std::fs::read_to_string(repo().join(format!("examples/{name}.{ext}.json")))
                .unwrap_or_else(|_| "{}".into())
        };
        let intent = CreativeIntent::from_json(&read("intent")).expect("intent");
        let style: StyleProfile = serde_json::from_str(&read("style")).expect("style");
        let assets = repo().join("assets");
        let font_paths: Vec<PathBuf> = FontSet::for_style(&style)
            .faces
            .iter()
            .map(|f| assets.join(f.path))
            .collect();
        let measure =
            FontMeasure::with_fonts(font_paths.iter().map(PathBuf::as_path)).expect("fonts");
        let opts = CompileOptions {
            art: Some(ArtMode::Force(Look::StudioPop)),
            speech: Some(voice(&intent)),
            ..CompileOptions::default()
        };
        compile_with_report(
            &intent,
            &style,
            None,
            &AssetLibrary::new(&assets),
            &measure,
            &AssetManifest::empty(),
            None,
            &opts,
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}
