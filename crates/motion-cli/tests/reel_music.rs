//! (0.23 C4b) `reel` picks the music bed from the story's content
//! (`audio::story_mood` + `audio::select_bed`), `reel --music-mood <word>`
//! forces a mood with a `warning[music_fit]` when it conflicts, and no
//! compatible bed means no bed, said plainly.
//!
//! Offline and cheap: `reel --plan-only` chooses and prints the music (the
//! `[3/5] music:` line, its `warning[music_fit]` and the MusicPlan path) and
//! stops before the voice, the compile and the render, so nothing is spoken,
//! rendered or written, and no provider is called.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .canonicalize()
        .expect("workspace root")
}

fn motion(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_motion-engine"))
        .current_dir(root())
        .args(args)
        .output()
        .expect("run motion-engine")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

const COMPOUND: &str = "golden/fixtures/music_mood/compound_interest.intent.json";
const FLOOD: &str = "golden/fixtures/music_mood/flood_toll.intent.json";
const WEEKLY: &str = "examples/demo_04/weekly_basket.intent.json";
const WEEKLY_STYLE: &str = "examples/demo_04/weekly_basket.style.json";
const SPACE: &str = "examples/cinematic/space.intent.json";
const SPACE_STYLE: &str = "examples/cinematic/space.style.json";
const NAP: &str = "examples/voice_10/why_we_nap.intent.json";

/// What `reel --plan-only` printed for the music step.
#[derive(Debug)]
struct Plan {
    /// The text after `[3/5] music: `.
    line: String,
    /// The bed id (`None` when the line says `none`).
    bed: Option<String>,
    /// The MusicPlan path of the `music plan:` line.
    plan: Option<String>,
    /// The messages of the `warning[music_fit]: ` lines.
    warnings: Vec<String>,
    /// Every stdout line.
    all: Vec<String>,
}

fn plan_of(args: &[&str]) -> Plan {
    let mut full = vec!["reel"];
    full.extend_from_slice(args);
    // The way the spec asks: offline, the say voice (never reached with
    // --plan-only, which stops before the voice step).
    full.extend_from_slice(&["--plan-only", "--tts-model", "say", "--offline"]);
    let o = motion(&full);
    assert!(
        o.status.success(),
        "reel {args:?} failed:\n{}{}",
        stdout(&o),
        stderr(&o)
    );
    let text = stdout(&o);
    let all: Vec<String> = text.lines().map(str::to_string).collect();
    let line = all
        .iter()
        .find_map(|l| l.strip_prefix("[3/5] music: "))
        .unwrap_or_else(|| panic!("no music line in:\n{text}"))
        .to_string();
    let bed = line
        .split_whitespace()
        .next()
        .filter(|w| *w != "none")
        .map(str::to_string);
    let plan = all
        .iter()
        .find_map(|l| l.strip_prefix("music plan: "))
        .map(str::to_string);
    let warnings = all
        .iter()
        .filter_map(|l| l.strip_prefix("warning[music_fit]: "))
        .map(str::to_string)
        .collect();
    // Plan-only stops before every other step.
    for step in ["[1/5]", "[2/5]", "[4/5]", "[5/5]", "video:"] {
        assert!(
            !all.iter().any(|l| l.starts_with(step)),
            "plan-only ran {step}:\n{text}"
        );
    }
    Plan {
        line,
        bed,
        plan,
        warnings,
        all,
    }
}

fn bed_id(p: &Plan) -> &str {
    p.bed.as_deref().unwrap_or("(none)")
}

/// The v0.2 catalog: bed id → its mood families.
fn catalog() -> Vec<(String, Vec<String>)> {
    let text = std::fs::read_to_string(root().join("assets/music/catalog.json")).expect("catalog");
    let cat: Value = serde_json::from_str(&text).expect("catalog json");
    cat["beds"]
        .as_array()
        .expect("beds")
        .iter()
        .map(|b| {
            (
                b["id"].as_str().expect("id").to_string(),
                b["moods"]
                    .as_array()
                    .map(|m| {
                        m.iter()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
            )
        })
        .collect()
}

fn moods_of(bed: &str) -> Vec<String> {
    catalog()
        .into_iter()
        .find(|(id, _)| id == bed)
        .map(|(_, m)| m)
        .unwrap_or_else(|| panic!("bed {bed} is not in the catalog"))
}

/// Scratch directory unique to one test, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("reel_music_{}", std::process::id()))
            .join(tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Scratch(dir)
    }
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------------------
// 1. Beds follow the content
// ---------------------------------------------------------------------------

#[test]
fn reel_picks_the_bed_from_the_story_not_from_the_emotion() {
    // compound_interest (tone auto): a neutral explainer about money.
    let p = plan_of(&[COMPOUND]);
    assert_eq!(bed_id(&p), "tech_pulse", "{}", p.line);
    assert_eq!(p.line, "tech_pulse (neutral explainer, money story)");
    assert!(p.warnings.is_empty());
    let plan = p.plan.as_deref().expect("music plan line");
    assert!(
        root().join(plan).is_file() && plan.ends_with("tech_pulse.music.json"),
        "{plan}"
    );

    // weekly_basket (documentary): prices rising, serious; never warm piano.
    let p = plan_of(&[WEEKLY, "--style", WEEKLY_STYLE]);
    assert_ne!(bed_id(&p), "warm_piano", "{}", p.line);
    let moods = moods_of(bed_id(&p));
    assert!(
        moods
            .iter()
            .any(|m| m == "neutral_explainer" || m == "serious"),
        "{}: {moods:?}",
        p.line
    );

    // space (cinematic): calm_ambient or cinematic_strings.
    let p = plan_of(&[SPACE, "--style", SPACE_STYLE]);
    assert!(
        ["calm_ambient", "cinematic_strings"].contains(&bed_id(&p)),
        "{}",
        p.line
    );

    // flood_toll (a somber disaster story): calm_ambient, or no bed with the
    // plain line; never strings, never a bright bed.
    let p = plan_of(&[FLOOD]);
    match p.bed.as_deref() {
        Some(bed) => assert_eq!(bed, "calm_ambient", "{}", p.line),
        None => assert!(p.line.contains("no bed fits"), "{}", p.line),
    }
}

#[test]
fn the_choice_is_deterministic_and_follows_the_take() {
    for story in [COMPOUND, FLOOD, NAP] {
        for take in ["0", "1", "2", "3"] {
            let a = plan_of(&[story, "--take", take]);
            let b = plan_of(&[story, "--take", take]);
            assert_eq!(a.line, b.line, "{story} take {take}");
            assert_eq!(a.plan, b.plan);
        }
        // Without variety the compile has no seed and the bed has none.
        let a = plan_of(&[story, "--no-variety"]);
        let b = plan_of(&[story, "--no-variety", "--take", "5"]);
        assert_eq!(a.line, b.line, "{story}: --no-variety ignores the take");
    }
    // A take never leaves the fitting beds: compound_interest stays neutral.
    for take in ["0", "1", "2", "3", "7"] {
        let p = plan_of(&[COMPOUND, "--take", take]);
        let moods = moods_of(bed_id(&p));
        assert!(
            moods
                .iter()
                .any(|m| ["neutral_explainer", "serious", "calm"].contains(&m.as_str())),
            "take {take}: {}",
            p.line
        );
        assert_ne!(bed_id(&p), "cinematic_strings", "take {take}");
    }
}

#[test]
fn named_beds_and_none_behave_as_before() {
    let p = plan_of(&[COMPOUND, "--music", "retro_groove"]);
    assert_eq!(p.line, "retro_groove (as asked)");
    assert!(p.warnings.is_empty());
    assert!(p
        .plan
        .as_deref()
        .is_some_and(|x| x.ends_with("retro_groove.music.json") && root().join(x).is_file()));

    let p = plan_of(&[COMPOUND, "--music", "none"]);
    assert_eq!(p.line, "none");
    assert_eq!(p.plan, None);

    // A mood word is read only with --music auto: said, not applied.
    let p = plan_of(&[COMPOUND, "--music", "none", "--music-mood", "calm"]);
    assert_eq!(p.line, "none");
    assert!(
        p.all
            .iter()
            .any(|l| l.contains("--music-mood calm ignored (--music none)")),
        "{:?}",
        p.all
    );
    let p = plan_of(&[
        COMPOUND,
        "--music",
        "warm_piano",
        "--music-mood",
        "dramatic",
    ]);
    assert_eq!(p.line, "warm_piano (as asked)");
    assert!(p.warnings.is_empty(), "a named bed is never second-guessed");
    assert!(
        p.all
            .iter()
            .any(|l| l.contains("--music-mood dramatic ignored")),
        "{:?}",
        p.all
    );

    // An unknown bed fails the way it always did.
    let o = motion(&["reel", COMPOUND, "--music", "nope", "--plan-only"]);
    assert!(!o.status.success());
    assert!(
        stderr(&o).contains("--music 'nope': not in"),
        "{}",
        stderr(&o)
    );
}

// ---------------------------------------------------------------------------
// 2. --music-mood
// ---------------------------------------------------------------------------

#[test]
fn a_forced_mood_that_conflicts_warns_and_plays_as_asked() {
    // compound_interest reads as a neutral explainer; "dramatic" conflicts.
    let p = plan_of(&[COMPOUND, "--music-mood", "dramatic"]);
    assert_eq!(bed_id(&p), "cinematic_strings", "{}", p.line);
    assert_eq!(p.warnings.len(), 1, "{:?}", p.all);
    let w = &p.warnings[0];
    assert!(
        w.contains("'dramatic'") && w.contains("neutral explainer"),
        "{w}"
    );
    // The warning is printed in the shape of the compile's story warnings.
    assert!(p
        .all
        .iter()
        .any(|l| l.starts_with("warning[music_fit]: the music word 'dramatic'")));

    // why_we_nap reads as calm; "calm" agrees: no warning.
    let p = plan_of(&[NAP, "--music-mood", "calm"]);
    assert!(p.warnings.is_empty(), "{:?}", p.all);
    assert!(p.bed.is_some(), "{}", p.line);
    assert!(!p.all.iter().any(|l| l.contains("warning[")), "{:?}", p.all);

    // The same word also agrees with a story it fits: neutral on money.
    let p = plan_of(&[COMPOUND, "--music-mood", "neutral"]);
    assert!(p.warnings.is_empty(), "{:?}", p.all);
    assert_eq!(bed_id(&p), "tech_pulse");

    // `none` is silence on request, with no warning; `auto` is the default.
    let p = plan_of(&[COMPOUND, "--music-mood", "none"]);
    assert_eq!(p.bed, None);
    assert_eq!(p.line, "none (no music requested)");
    assert_eq!(p.plan, None);
    assert!(p.warnings.is_empty());
    assert_eq!(
        plan_of(&[COMPOUND, "--music-mood", "auto"]).line,
        plan_of(&[COMPOUND]).line
    );

    // Words are exact: the synonyms are the MCP policy's job.
    let o = motion(&["reel", COMPOUND, "--music-mood", "loud", "--plan-only"]);
    assert!(!o.status.success());
    let err = stderr(&o);
    assert!(
        err.contains("--music-mood 'loud'")
            && err.contains("auto, none, calm, upbeat, serious, dramatic, playful, neutral"),
        "{err}"
    );
}

// ---------------------------------------------------------------------------
// 3. No compatible bed: no bed, said plainly
// ---------------------------------------------------------------------------

/// A copy of `assets/music/catalog.json` keeping only `keep` (and, with
/// `strip_moods`, without the v0.2 tags), under a scratch assets folder.
fn assets_with(scratch: &Scratch, keep: &[&str], strip_moods: bool) -> String {
    let text = std::fs::read_to_string(root().join("assets/music/catalog.json")).expect("catalog");
    let mut cat: Value = serde_json::from_str(&text).expect("catalog json");
    let beds: Vec<Value> = cat["beds"]
        .as_array()
        .expect("beds")
        .iter()
        .filter(|b| keep.contains(&b["id"].as_str().unwrap_or_default()))
        .cloned()
        .map(|mut b| {
            if strip_moods {
                let o = b.as_object_mut().expect("bed object");
                for k in ["moods", "energy", "tempo_band", "lra", "speech_band_db"] {
                    o.remove(k);
                }
            }
            b
        })
        .collect();
    assert_eq!(beds.len(), keep.len(), "bed ids {keep:?}");
    cat["beds"] = Value::Array(beds);
    if strip_moods {
        cat["version"] = Value::String("0.1".into());
    }
    let assets = scratch.path("assets");
    std::fs::create_dir_all(format!("{assets}/music")).expect("assets/music");
    std::fs::write(
        format!("{assets}/music/catalog.json"),
        serde_json::to_string_pretty(&cat).expect("json"),
    )
    .expect("write catalog");
    assets
}

#[test]
fn no_compatible_bed_means_no_bed_and_the_line_says_so() {
    let scratch = Scratch::new("no_bed");
    // A library with only the neutral / serious bed has nothing for grief.
    let assets = assets_with(&scratch, &["tech_pulse"], false);
    let out = scratch.path("out");
    let p = plan_of(&[FLOOD, "--assets", &assets, "-o", &out]);
    assert_eq!(p.bed, None, "{}", p.line);
    assert_eq!(p.line, "none (no bed fits a somber story; silence)");
    assert_eq!(p.plan, None);
    assert!(p.warnings.is_empty());
    assert!(
        !Path::new(&out).exists(),
        "plan-only writes nothing: {out} exists"
    );
    // The same library still serves a story it fits.
    let p = plan_of(&[COMPOUND, "--assets", &assets]);
    assert_eq!(bed_id(&p), "tech_pulse");

    // A forced mood with no bed for it: no bed, the plain line and the warning.
    let p = plan_of(&[COMPOUND, "--assets", &assets, "--music-mood", "playful"]);
    assert_eq!(p.bed, None, "{}", p.line);
    assert!(p.line.contains("no bed fits a playful story"), "{}", p.line);
    assert_eq!(p.warnings.len(), 1, "{:?}", p.all);

    // No catalog at all is no bed, said plainly (as it always was).
    let empty = scratch.path("bare");
    std::fs::create_dir_all(&empty).expect("bare assets");
    let p = plan_of(&[COMPOUND, "--assets", &empty]);
    assert_eq!(p.bed, None);
    assert!(
        p.line.starts_with("none (no music catalog at "),
        "{}",
        p.line
    );
}

#[test]
fn a_catalog_without_mood_tags_still_picks_by_emotion() {
    let scratch = Scratch::new("v01");
    let assets = assets_with(&scratch, &["tech_pulse", "warm_piano"], true);
    let p = plan_of(&[COMPOUND, "--assets", &assets]);
    assert!(p.bed.is_some(), "{}", p.line);
    assert!(p.line.contains("catalog without moods"), "{}", p.line);
    assert!(p.plan.is_some());
    assert!(p.warnings.is_empty());
}

// ---------------------------------------------------------------------------
// The help text
// ---------------------------------------------------------------------------

#[test]
fn reel_lists_the_new_flags() {
    let o = motion(&["reel", "--help"]);
    assert!(o.status.success());
    let help = stdout(&o);
    assert!(help.contains("--music-mood <MUSIC_MOOD>"), "{help}");
    assert!(help.contains("--plan-only"), "{help}");
    assert!(help.contains("music_fit"), "{help}");
}
