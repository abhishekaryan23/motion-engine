//! Creator tools (plan §9, step 1f): `view_frames`, `explore_styles`,
//! `list_options` / `options_json`, `plan_assets`.
//!
//! Pure checks (READ-frame computation, look ordering, options data, argument
//! errors) always run. Tests that need the real engine
//! (`<repo>/target/release/motion-engine`, or `$MOTION_ENGINE`) and ffmpeg skip
//! with a printed reason when they are missing. They write under
//! `CARGO_TARGET_TMPDIR`, never into the repository's own `output/`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use motion_core::compiler::art_direction::Look;
use motion_core::scene::MotionProject;
use motion_core::{CreativeIntent, StyleProfile};
use motion_mcp::creator::{self, auto_look, explore_looks, options_json, picks_for, read_moments};
use motion_mcp::profile::{self, Profile, ServerConfig};
use motion_mcp::reply::ToolOutput;
use serde_json::{json, Value};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/creator");
const JOB: &str = "j_c0ffee0001";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(Path::new(FIXTURES).join(name)).unwrap()
}

fn scene() -> MotionProject {
    MotionProject::from_json(&fixture("scene.motion.json")).unwrap()
}

/// The real engine, or None (the test then prints why and returns).
fn engine() -> Option<PathBuf> {
    let path = std::env::var_os("MOTION_ENGINE")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("target/release/motion-engine"));
    path.is_file().then_some(path)
}

fn has_ffmpeg() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Skip unless the engine and ffmpeg are there.
fn real_engine(test: &str) -> Option<PathBuf> {
    let Some(engine) = engine() else {
        eprintln!(
            "skipping {test}: no engine at {} (build it: cargo build --release -p motion-cli, or set MOTION_ENGINE)",
            repo().join("target/release/motion-engine").display()
        );
        return None;
    };
    if !has_ffmpeg() {
        eprintln!("skipping {test}: ffmpeg is not installed");
        return None;
    }
    Some(engine)
}

/// A temporary repository: `assets/` is the real library, `output/jobs/` is
/// empty.
struct Env {
    config: ServerConfig,
}

impl Env {
    fn new(name: &str, engine: Option<&Path>) -> Env {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("creator-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        let repo_dir = root.join("repo");
        std::fs::create_dir_all(repo_dir.join("output/jobs")).unwrap();
        std::os::unix::fs::symlink(
            repo().join("assets").canonicalize().unwrap(),
            repo_dir.join("assets"),
        )
        .unwrap();
        let mut config = ServerConfig::new(Profile::Creator, &repo_dir);
        if let Some(e) = engine {
            config.engine = e.to_path_buf();
        }
        Env { config }
    }

    fn jobs(&self) -> PathBuf {
        self.config.jobs.clone()
    }

    /// `output/jobs/<id>/` with the fixture's intent and style (and the story
    /// edits `edit` makes to the intent).
    fn job(&self, id: &str, edit: impl FnOnce(&mut Value)) -> PathBuf {
        let dir = self.jobs().join(id);
        std::fs::create_dir_all(&dir).unwrap();
        let mut intent: Value = serde_json::from_str(&fixture("intent.json")).unwrap();
        edit(&mut intent);
        std::fs::write(dir.join("intent.json"), intent.to_string()).unwrap();
        std::fs::write(dir.join("style.json"), fixture("style.json")).unwrap();
        dir
    }

    fn call(&self, tool: &str, args: Value) -> ToolOutput {
        let pictures = motion_mcp::pictures::PictureIndex::load(&self.config.assets);
        creator::call(&self.config, &pictures, tool, args)
    }
}

/// `job` with a compiled scene `reel/compound_interest.motion.json`.
fn compiled_job(env: &Env, engine: &Path) -> PathBuf {
    let dir = env.job(JOB, |_| {});
    let reel = dir.join("reel");
    std::fs::create_dir_all(&reel).unwrap();
    let out = Command::new(engine)
        .current_dir(&env.config.repo)
        .arg("compile")
        .arg(dir.join("intent.json"))
        .arg("--style")
        .arg(dir.join("style.json"))
        .arg("--assets")
        .arg(&env.config.assets)
        .args(["--art", "auto", "--variety", "auto", "--output"])
        .arg(reel.join("compound_interest.motion.json"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "compile failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    dir
}

fn png_size(path: &Path) -> (u32, u32) {
    let head = std::fs::read(path).unwrap();
    assert_eq!(
        &head[..8],
        b"\x89PNG\r\n\x1a\n",
        "{} is not a PNG",
        path.display()
    );
    (
        u32::from_be_bytes(head[16..20].try_into().unwrap()),
        u32::from_be_bytes(head[20..24].try_into().unwrap()),
    )
}

fn assert_fix(out: &ToolOutput, field: &str) {
    assert!(out.is_error, "{}", out.text);
    assert_eq!(out.structured["status"], "needs_fix", "{}", out.text);
    assert_eq!(out.structured["fixes"][0]["field"], field, "{}", out.text);
}

// ---------------------------------------------------------------- READ frames

#[test]
fn read_frames_are_scene_start_plus_lifecycle_read() {
    let project = scene();
    let moments = read_moments(&project);
    // The backdrop and the caption scene are not beats.
    assert_eq!(moments.len(), 4);
    assert_eq!(
        moments.iter().map(|m| m.beat).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    // start + read, at 30 fps, rounded.
    let want = [
        (0.0 + 1.57, 47),
        (4.959 + 1.683, 199),
        (8.857 + 1.883, 322),
        (13.631 + 3.544, 515),
    ];
    for (m, (t, frame)) in moments.iter().zip(want) {
        assert!((m.time_s - t).abs() < 1e-9, "{m:?}");
        assert_eq!(m.frame, frame, "{m:?}");
        assert_eq!(m.frame, (t * 30.0_f64).round() as u32);
    }
    // Frames stay inside the video (18.923 s = 568 frames).
    assert_eq!(project.frame_count(), 568);
    assert_eq!(
        motion_mcp::creator::frames::frame_at(&project, 1_000.0),
        567
    );
    assert_eq!(motion_mcp::creator::frames::frame_at(&project, -3.0), 0);
    assert_eq!(motion_mcp::creator::frames::frame_at(&project, f64::NAN), 0);
}

#[test]
fn a_scene_without_lifecycle_is_read_in_the_middle() {
    let mut project = scene();
    for s in &mut project.scenes {
        s.lifecycle = None;
    }
    // No lifecycle anywhere: every scene but the captions counts as a beat,
    // read in its middle (the backdrop included, as layout QA treats it).
    let moments = read_moments(&project);
    assert_eq!(moments.len(), 5);
    let beat_1 = moments[1];
    assert!((beat_1.time_s - 5.509 * 0.5).abs() < 1e-9);
}

#[test]
fn picks_default_to_every_beat_and_validate_requests() {
    let project = scene();
    let all = picks_for(&project, None, None).unwrap();
    assert_eq!(
        all.iter().map(|p| p.frame).collect::<Vec<_>>(),
        vec![47, 199, 322, 515]
    );
    assert_eq!(all[0].label, "beat 1");
    assert_eq!(all[0].beat, Some(1));

    // Chosen beats come back in video order, once each.
    let some = picks_for(&project, Some(&[3, 1, 3]), None).unwrap();
    assert_eq!(
        some.iter().map(|p| p.beat).collect::<Vec<_>>(),
        vec![Some(1), Some(3)]
    );
    // An empty list means "all".
    assert_eq!(picks_for(&project, Some(&[]), None).unwrap().len(), 4);

    // A beat that does not exist is a fix that says how many there are.
    for bad in [[0usize], [5]] {
        let fix = picks_for(&project, Some(&bad), None).unwrap_err();
        assert_eq!(fix.field, "beats");
        assert!(fix.to_string().contains("4 beats"), "{fix}");
    }

    // Times are clamped to the video; `times` wins over `beats`.
    let timed = picks_for(&project, Some(&[1]), Some(&[-2.0, 5.0, 999.0, 5.01])).unwrap();
    assert_eq!(
        timed.iter().map(|p| p.frame).collect::<Vec<_>>(),
        vec![0, 150, 567]
    );
    assert!(timed.iter().all(|p| p.beat.is_none()));
    assert_eq!(timed[1].label, "5.0 s");
    assert_eq!(
        picks_for(&project, None, Some(&[f64::NAN]))
            .unwrap_err()
            .field,
        "times"
    );
    assert_eq!(
        picks_for(&project, None, Some(&[1.0; 13]))
            .unwrap_err()
            .field,
        "times"
    );
}

// ----------------------------------------------------------------- explore

#[test]
fn explore_shows_the_auto_look_first_without_duplicates() {
    for auto in Look::ALL {
        for k in [0, 1, 2, 3, 4, 5, 9] {
            let looks = explore_looks(auto, k);
            assert_eq!(looks.len(), k.clamp(2, 4), "{auto:?} k={k}");
            assert_eq!(looks[0], auto);
            let names: std::collections::BTreeSet<_> = looks.iter().map(|l| l.name()).collect();
            assert_eq!(names.len(), looks.len(), "{auto:?} k={k}: {looks:?}");
        }
    }
}

#[test]
fn the_auto_look_is_the_one_the_compiler_records() {
    // The real compile of this story (art auto) recorded `classical_neon`.
    let intent = CreativeIntent::from_json(&fixture("intent.json")).unwrap();
    let style = StyleProfile::from_json(&fixture("style.json")).unwrap();
    let (look, emotion) = auto_look(&intent, &style);
    assert_eq!(look, Look::ClassicalNeon);
    assert_eq!(emotion, "drama");
}

// ----------------------------------------------------------------- options

#[test]
fn options_list_every_look_family_bed_and_tone() {
    let env = Env::new("options", None);
    let data = options_json(&env.config);

    let looks: Vec<&str> = data["looks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["name"].as_str().unwrap())
        .collect();
    assert_eq!(looks, Look::ALL.map(|l| l.name()).to_vec());
    for l in data["looks"].as_array().unwrap() {
        let use_ = l["best_use"].as_str().unwrap();
        assert!(
            !use_.is_empty() && use_.split_whitespace().count() <= 15,
            "{l}"
        );
        assert!(!l["families"].as_array().unwrap().is_empty(), "{l}");
    }

    // Every family folder of the real library, with its medium.
    let on_disk: std::collections::BTreeSet<String> =
        std::fs::read_dir(repo().join("assets/library"))
            .unwrap()
            .flatten()
            .filter(|e| e.path().join("catalog.json").is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
    let families = data["families"].as_array().unwrap();
    let listed: std::collections::BTreeSet<String> = families
        .iter()
        .map(|f| f["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(listed, on_disk);
    let clay = families
        .iter()
        .find(|f| f["name"] == "clay_props_3d")
        .unwrap();
    assert_eq!(clay["medium"], "clay 3D");
    assert!(clay["assets"].as_u64().unwrap() > 0);
    for f in families {
        assert_ne!(f["medium"], "mixed", "{f}: add the family to medium_of");
    }

    // Beds of assets/music/catalog.json, with their emotions.
    let beds = data["music"].as_array().unwrap();
    assert!(beds.len() >= 5, "{beds:?}");
    let warm = beds.iter().find(|b| b["id"] == "warm_piano").unwrap();
    assert!(warm["emotions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e == "trust"));

    // The tone words of the style schema, each with a best use.
    let tones: Vec<String> = data["tones"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(tones, motion_mcp::schema::tones());
    assert!(tones.iter().any(|t| t == "cinematic"));
}

#[test]
fn options_of_a_bare_library_are_empty_not_an_error() {
    let mut env = Env::new("options_bare", None);
    let bare = Path::new(env!("CARGO_TARGET_TMPDIR")).join("creator-options_bare/empty_assets");
    std::fs::create_dir_all(bare.join("library/odd_family")).unwrap();
    std::fs::write(
        bare.join("library/odd_family/catalog.json"),
        r#"{"assets": [{"id": "a"}, {"id": "b"}]}"#,
    )
    .unwrap();
    env.config.assets = bare;
    let data = options_json(&env.config);
    assert_eq!(data["music"], json!([]));
    assert_eq!(
        data["families"],
        json!([{"name": "odd_family", "assets": 2, "medium": "mixed"}])
    );
    assert_eq!(data["looks"].as_array().unwrap().len(), Look::ALL.len());
}

#[test]
fn list_options_stays_short_and_narrows_by_topic() {
    let env = Env::new("list_options", None);
    let budget = profile::reply_budget_chars(Profile::Creator);
    for topic in [
        None,
        Some("all"),
        Some("looks"),
        Some("families"),
        Some("music"),
        Some("tones"),
    ] {
        let args = match topic {
            Some(t) => json!({ "topic": t }),
            None => Value::Null,
        };
        let out = env.call(profile::LIST_OPTIONS, args);
        assert!(!out.is_error, "{topic:?}: {}", out.text);
        assert!(
            out.text.chars().count() <= 1_000 && out.text.chars().count() <= budget,
            "{topic:?}: {} chars",
            out.text.chars().count()
        );
        assert_eq!(out.structured["status"], "done");
        assert!(out.images.is_empty());
    }
    // `all` names every look, family and tone in the text.
    let all = env.call(profile::LIST_OPTIONS, Value::Null);
    for l in Look::ALL {
        assert!(
            all.text.contains(l.name()),
            "{} missing from: {}",
            l.name(),
            all.text
        );
    }
    assert!(all.text.contains("clay_props_3d") && all.text.contains("warm_piano"));
    assert!(all.structured["families"].as_array().unwrap().len() >= 10);
    // A topic gives that section only, with best uses.
    let looks = env.call(profile::LIST_OPTIONS, json!({"topic": "looks"}));
    assert!(looks.text.contains("dossier:") && !looks.text.contains("clay_props_3d"));
    assert!(looks.structured.get("families").is_none());
    assert_eq!(looks.structured["looks"].as_array().unwrap().len(), 10);
    let tones = env.call(profile::LIST_OPTIONS, json!({"topic": "tones"}));
    assert!(tones.text.contains("cinematic:"));
}

// ------------------------------------------------------- argument errors

#[test]
fn bad_arguments_are_fix_it_replies_not_errors() {
    let env = Env::new("arguments", None);
    for tool in [profile::VIEW_FRAMES, profile::PLAN_ASSETS] {
        for bad in ["", "../etc", "j_123", "J_C0FFEE0001", "j_c0ffee0001/"] {
            let out = env.call(tool, json!({ "job": bad }));
            assert_fix(&out, "job");
        }
        // A well-formed id that no job has.
        let out = env.call(tool, json!({ "job": "j_0000000000" }));
        assert_fix(&out, "job");
        assert!(out.text.contains("unknown job"), "{}", out.text);
        // The wrong shape of an argument.
        let out = env.call(tool, json!({ "job": 5 }));
        assert_fix(&out, "arguments");
        // No arguments at all.
        assert_fix(&env.call(tool, Value::Null), "job");
    }
    env.job(JOB, |_| {});
    assert_fix(
        &env.call(profile::VIEW_FRAMES, json!({"job": JOB, "beats": "all"})),
        "arguments",
    );

    let out = env.call(profile::LIST_OPTIONS, json!({"topic": "colours"}));
    assert_fix(&out, "topic");
    assert!(out.text.contains("looks"), "{}", out.text);

    let out = env.call(profile::EXPLORE_STYLES, Value::Null);
    assert_fix(&out, "story");
    let out = env.call(profile::EXPLORE_STYLES, json!({"story": {"beats": []}}));
    assert!(out.is_error, "{}", out.text);
    assert_eq!(out.structured["status"], "needs_fix");
    // An unknown format or tone word is not a fix: the story checks use
    // vertical / `auto` with a note (only `strict` refuses), so neither blocks.
    for (field, value) in [("format", "hexagonal"), ("style", "loud")] {
        let out = env.call(
            profile::EXPLORE_STYLES,
            json!({"story": serde_json::from_str::<Value>(&fixture("intent.json")).unwrap(), field: value}),
        );
        assert!(
            out.structured["fixes"]
                .as_array()
                .is_none_or(|f| f.iter().all(|f| f["field"] != field)),
            "{field}: {}",
            out.text
        );
    }
}

#[test]
fn unlisted_creator_names_are_not_available() {
    let env = Env::new("not_available", None);
    let out = env.call(profile::RENDER_FRAME, json!({}));
    assert!(
        out.is_error && out.text.contains("not available yet"),
        "{}",
        out.text
    );
}

#[test]
fn view_frames_before_the_scene_exists_says_still_rendering() {
    let env = Env::new("not_ready", None);
    let dir = env.job(JOB, |_| {});
    let out = env.call(profile::VIEW_FRAMES, json!({"job": JOB}));
    assert!(!out.is_error, "{}", out.text);
    assert_eq!(out.structured["status"], "running");
    assert!(out.text.contains("get_video"), "{}", out.text);
    assert!(out.images.is_empty());

    // A failed job says why instead.
    std::fs::write(
        dir.join("status.json"),
        json!({
            "job": JOB, "state": "failed", "error": "voice: provider unavailable",
            "created_unix": 1
        })
        .to_string(),
    )
    .unwrap();
    let out = env.call(profile::VIEW_FRAMES, json!({"job": JOB}));
    assert_eq!(out.structured["status"], "failed");
    assert!(out.text.contains("provider unavailable"), "{}", out.text);
}

// ------------------------------------------------------- the real engine

#[test]
fn view_frames_makes_one_sheet_within_1024_px() {
    let Some(engine) = real_engine("view_frames_makes_one_sheet_within_1024_px") else {
        return;
    };
    let env = Env::new("view_sheet", Some(&engine));
    let dir = compiled_job(&env, &engine);

    let started = Instant::now();
    let out = env.call(profile::VIEW_FRAMES, json!({"job": JOB}));
    let first = started.elapsed();
    assert!(!out.is_error, "{}", out.text);
    assert_eq!(out.structured["status"], "done");
    assert_eq!(out.images.len(), 1, "{}", out.text);
    assert_eq!(out.images[0].mime, "image/png");
    let sheet = &out.images[0].path;
    assert!(
        sheet.starts_with(&dir)
            && sheet
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("sheet_")
    );
    let (w, h) = png_size(sheet);
    assert!(w.max(h) <= 1024 && w.max(h) >= 512, "{w}x{h}");
    assert_eq!(out.structured["frames"].as_array().unwrap().len(), 4);
    assert!(
        out.text.contains("beats 1–4 at their reading moment"),
        "{}",
        out.text
    );
    assert!(out.text.contains("empty frames"), "{}", out.text);
    assert!(out.text.chars().count() <= 1_000);
    let rel = out.structured["sheet"].as_str().unwrap();
    assert!(rel.starts_with("output/jobs/j_c0ffee0001/sheet_"), "{rel}");
    // The frames were rendered at the READ moments of the compiled scene.
    let scene_path = dir.join("reel/compound_interest.motion.json");
    let project = MotionProject::from_json(&std::fs::read_to_string(&scene_path).unwrap()).unwrap();
    let want: Vec<u64> = read_moments(&project)
        .iter()
        .map(|m| m.frame as u64)
        .collect();
    let got: Vec<u64> = out.structured["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["frame"].as_u64().unwrap())
        .collect();
    assert_eq!(got, want);

    // Again: nothing is rendered twice.
    let started = Instant::now();
    let again = env.call(profile::VIEW_FRAMES, json!({"job": JOB}));
    let second = started.elapsed();
    assert_eq!(again.images[0].path, *sheet);
    assert!(
        second < first / 2 || second.as_millis() < 300,
        "{first:?} then {second:?}"
    );
    eprintln!("view_frames (4 beats): first {first:?}, cached {second:?}; sheet {w}x{h}");

    // Separate images, and one beat alone (full-size image, no sheet).
    let out = env.call(
        profile::VIEW_FRAMES,
        json!({"job": JOB, "beats": [2, 3], "sheet": false}),
    );
    assert!(!out.is_error, "{}", out.text);
    assert_eq!(out.images.len(), 2);
    for i in &out.images {
        let (w, h) = png_size(&i.path);
        assert!(w.max(h) <= 1024, "{w}x{h}");
    }
    assert!(out.structured.get("sheet").is_none());
    let one = env.call(profile::VIEW_FRAMES, json!({"job": JOB, "beats": [4]}));
    assert_eq!(one.images.len(), 1);
    assert_eq!(png_size(&one.images[0].path), (576, 1024));
    assert!(
        one.text.contains("beat 4 at its reading moment"),
        "{}",
        one.text
    );

    // Explicit times, and a beat the video does not have.
    let out = env.call(
        profile::VIEW_FRAMES,
        json!({"job": JOB, "times": [0.5, 9.0]}),
    );
    assert!(!out.is_error, "{}", out.text);
    assert!(out.text.contains("frames at 0.5 s, 9.0 s"), "{}", out.text);
    assert_fix(
        &env.call(profile::VIEW_FRAMES, json!({"job": JOB, "beats": [7]})),
        "beats",
    );
}

#[test]
fn view_frames_on_a_copy_of_a_real_job() {
    let Some(engine) = real_engine("view_frames_on_a_copy_of_a_real_job") else {
        return;
    };
    let source = std::env::var_os("MOTION_REAL_JOB").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from("output/jobs/test_job")
    });
    if !source.join("reel").is_dir() {
        eprintln!(
            "skipping view_frames_on_a_copy_of_a_real_job: no real job at {}",
            source.display()
        );
        return;
    }
    let env = Env::new("view_real", Some(&engine));
    // Copy the job's small files (not the 40 MB video); the scene finds
    // `assets/` four folders up: <repo>/output/jobs/<job>/reel/.
    let id = source.file_name().unwrap().to_string_lossy().into_owned();
    let dir = env.jobs().join(&id);
    std::fs::create_dir_all(dir.join("reel")).unwrap();
    for f in ["intent.json", "style.json", "request.json", "status.json"] {
        let _ = std::fs::copy(source.join(f), dir.join(f));
    }
    for e in std::fs::read_dir(source.join("reel")).unwrap().flatten() {
        if e.path().extension().is_some_and(|x| x == "json") {
            std::fs::copy(e.path(), dir.join("reel").join(e.file_name())).unwrap();
        }
    }
    let started = Instant::now();
    let out = env.call(profile::VIEW_FRAMES, json!({"job": id}));
    eprintln!("view_frames on the real job: {:?}", started.elapsed());
    assert!(!out.is_error, "{}", out.text);
    let (w, h) = png_size(&out.images[0].path);
    assert!(w.max(h) <= 1024, "{w}x{h}");
    assert!(out.structured["frames"].as_array().unwrap().len() >= 2);
}

#[test]
fn plan_assets_lists_only_what_the_library_cannot_show() {
    let Some(engine) = real_engine("plan_assets_lists_only_what_the_library_cannot_show") else {
        return;
    };
    let env = Env::new("plan", Some(&engine));

    // The fixture's nouns are all in the look's families: nothing to make.
    env.job(JOB, |_| {});
    let out = env.call(profile::PLAN_ASSETS, json!({"job": JOB}));
    assert!(!out.is_error, "{}", out.text);
    assert_eq!(out.structured["missing"], json!([]), "{}", out.text);
    assert!(out.text.contains("nothing missing"), "{}", out.text);
    assert!(out.structured["families"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f == "editorial_concepts"));

    // A noun no family has becomes a prompt, in the video's style.
    let other = "j_c0ffee0002";
    env.job(other, |intent| {
        intent["beats"][0]["primary"]["asset"] = json!("zorb");
    });
    let out = env.call(profile::PLAN_ASSETS, json!({"job": other}));
    assert!(!out.is_error, "{}", out.text);
    let missing = out.structured["missing"].as_array().unwrap();
    assert_eq!(missing.len(), 1, "{}", out.text);
    assert_eq!(missing[0]["beat"], 1);
    assert_eq!(missing[0]["role"], "hero_object");
    assert_eq!(missing[0]["subject"], "zorb");
    assert_eq!(missing[0]["file"], "beat1_object.png");
    assert!(missing[0]["prompt"]
        .as_str()
        .unwrap()
        .to_lowercase()
        .contains("zorb"));
    let lines: Vec<&str> = out.text.lines().collect();
    assert!(lines.len() <= 7, "{}", out.text);
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("beat 1 hero_object \"zorb\": ")),
        "{}",
        out.text
    );
    assert!(
        out.text.contains("assets/inbox/<folder>/ as beatN.png"),
        "{}",
        out.text
    );
    assert!(out.text.contains("assets: \"<folder>\""), "{}", out.text);
    // The same call again gives the same answer.
    assert_eq!(
        env.call(profile::PLAN_ASSETS, json!({"job": other})).text,
        out.text
    );

    // Pictures the user already supplied are not asked for again.
    let dir = env.jobs().join(other);
    std::fs::write(
        dir.join("assets.manifest.json"),
        json!({"version": "0.1", "assets": [{"id": "beat_1.hero_object", "path": "images/zorb.png"}]}).to_string(),
    )
    .unwrap();
    let out = env.call(profile::PLAN_ASSETS, json!({"job": other}));
    assert_eq!(out.structured["missing"], json!([]), "{}", out.text);
}

#[test]
fn explore_styles_returns_one_sheet_with_a_row_per_look() {
    let Some(engine) = real_engine("explore_styles_returns_one_sheet_with_a_row_per_look") else {
        return;
    };
    if !Command::new("say")
        .arg("-v")
        .arg("?")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("skipping explore_styles_returns_one_sheet_with_a_row_per_look: no `say` voice");
        return;
    }
    // The engine's voice cache is a temporary folder too.
    std::env::set_var(
        "MOTION_VOICE_CACHE",
        Path::new(env!("CARGO_TARGET_TMPDIR")).join("creator-voice-cache"),
    );
    let mut env = Env::new("explore", Some(&engine));
    env.config.tts_model = "say".to_string();
    let story: Value = serde_json::from_str(&fixture("intent.json")).unwrap();

    let started = Instant::now();
    let out = env.call(profile::EXPLORE_STYLES, json!({"story": story, "k": 3}));
    let first = started.elapsed();
    assert!(!out.is_error, "{}", out.text);
    assert_eq!(out.structured["status"], "done");
    assert_eq!(out.structured["cached"], false);
    assert_eq!(out.images.len(), 1);
    let (w, h) = png_size(&out.images[0].path);
    assert!(w.max(h) <= 1024, "{w}x{h}");
    // One row per look; the auto look first, then its neighbours.
    let lines: Vec<&str> = out.text.lines().filter(|l| l.starts_with("row ")).collect();
    assert_eq!(
        lines,
        vec![
            "row 1: classical_neon (auto)",
            "row 2: ornament_editorial",
            "row 3: halftone_cutout"
        ],
        "{}",
        out.text
    );
    assert!(
        out.text
            .contains("Pass options.art (the look) to make_video."),
        "{}",
        out.text
    );
    assert!(out.text.chars().count() <= 1_000);
    let rows = out.structured["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["art"], "classical_neon");
    assert_eq!(rows[0]["auto"], true);
    assert_eq!(rows[0]["beats"], json!([1, 2, 3, 4]));
    assert_eq!(
        out.structured["sheet"].as_str().unwrap().split('/').nth(2),
        Some("_explore")
    );

    // Every look compiled to its own scene in the work folder.
    let work = out.images[0].path.parent().unwrap().to_path_buf();
    for look in ["classical_neon", "ornament_editorial", "halftone_cutout"] {
        assert!(work.join(format!("{look}.motion.json")).is_file(), "{look}");
    }

    // The same request is answered from the work folder, instantly.
    let started = Instant::now();
    let again = env.call(profile::EXPLORE_STYLES, json!({"story": story, "k": 3}));
    let second = started.elapsed();
    assert_eq!(again.structured["cached"], true);
    assert_eq!(again.images[0].path, out.images[0].path);
    assert_eq!(again.text, out.text);
    assert!(second.as_millis() < 1_000, "{second:?}");
    eprintln!(
        "explore_styles (3 looks, say voice): first {first:?}, cached {second:?}; sheet {w}x{h}"
    );

    // k is clamped to 2-4; a different k is a different request.
    let two = env.call(profile::EXPLORE_STYLES, json!({"story": story, "k": 1}));
    assert!(!two.is_error, "{}", two.text);
    assert_eq!(
        two.text.lines().filter(|l| l.starts_with("row ")).count(),
        2
    );
}

#[test]
fn explore_styles_takes_a_lite_story_too() {
    let Some(engine) = real_engine("explore_styles_takes_a_lite_story_too") else {
        return;
    };
    if !Command::new("say")
        .arg("-v")
        .arg("?")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("skipping explore_styles_takes_a_lite_story_too: no `say` voice");
        return;
    }
    std::env::set_var(
        "MOTION_VOICE_CACHE",
        Path::new(env!("CARGO_TARGET_TMPDIR")).join("creator-voice-cache"),
    );
    let mut env = Env::new("explore_lite", Some(&engine));
    env.config.tts_model = "say".to_string();
    let story = json!({
        "title": "slow money",
        "beats": [
            {"say": "A savings account grows slowly, but it grows safely every single year.", "show": "Slow and safe"},
            {"say": "Seven percent a year doubles your money in about ten years.", "number": "7%", "meaning": "per year"},
            {"say": "So the best time to start was yesterday, and the second best is today.", "show": "Start today"}
        ]
    });
    let out = env.call(
        profile::EXPLORE_STYLES,
        json!({"story": story, "k": 2, "style": "editorial"}),
    );
    assert!(!out.is_error, "{}", out.text);
    assert_eq!(out.images.len(), 1);
    let (w, h) = png_size(&out.images[0].path);
    assert!(w.max(h) <= 1024, "{w}x{h}");
    assert_eq!(
        out.text.lines().filter(|l| l.starts_with("row ")).count(),
        2,
        "{}",
        out.text
    );
    assert!(
        out.text.lines().nth(1).unwrap().ends_with("(auto)"),
        "{}",
        out.text
    );
}
