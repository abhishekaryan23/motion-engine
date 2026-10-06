//! (0.23 C4b) The optional `music` word of `make_video` / `revise_video`
//! (`audio::MusicWord`: auto, none, calm, upbeat, serious, dramatic, playful,
//! neutral): read leniently, part of the job id only when it is not `auto`,
//! kept by `revise_video` unless changed, forwarded to the reel as
//! `--music-mood`, and answered with the bed and the reason (a `music_fit`
//! warning as a finding). Offline: the policy tests call `prepare` / `revise`
//! directly; the reply tests run the real `Service` and `SubprocessEngine`
//! over a stand-in engine script (no voice, no render, no network).

mod common;

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;

use motion_core::audio::MusicWord;
use motion_mcp::args::{MakeVideoArgs, ReviseVideoArgs};
use motion_mcp::engine::{self, EngineJob, SubprocessEngine};
use motion_mcp::job::{canonical_json, job_id, JobId, StoredRequest};
use motion_mcp::policy::{self, music_word_for, Prepared};
use motion_mcp::profile::{self, Profile};
use motion_mcp::reply::Fix;
use motion_mcp::service::{CallCtx, Service};
use serde_json::{json, Value};

fn two_beats() -> Value {
    json!({"title": "policy test", "beats": [
        {"say": "Costs fell sharply while the output almost tripled this year."},
        {"say": "Nobody on the team expected the change to come this fast."}
    ]})
}

fn make(music: Option<Value>, strict: bool) -> MakeVideoArgs {
    MakeVideoArgs {
        story: two_beats(),
        music,
        strict,
        ..Default::default()
    }
}

fn prepare(profile: Profile, a: &MakeVideoArgs) -> Result<Prepared, Vec<Fix>> {
    let config = common::config(profile, "music_word");
    policy::prepare(a, &common::ctx(&config))
}

fn ok(profile: Profile, music: Option<Value>) -> Prepared {
    prepare(profile, &make(music, false)).unwrap_or_else(|f| panic!("needs_fix: {f:#?}"))
}

fn id_of(p: &Prepared) -> String {
    job_id(&p.key).to_string()
}

fn revise(p: &Prepared, music: Option<Value>, strict: bool) -> Result<Prepared, Vec<Fix>> {
    let config = common::config(Profile::Weak, "music_word_revise");
    policy::revise(
        &p.request,
        &ReviseVideoArgs {
            job: id_of(p),
            style: Some(json!("cinematic")),
            music,
            strict,
            ..Default::default()
        },
        &common::ctx(&config),
    )
}

fn fixes(r: Result<Prepared, Vec<Fix>>) -> Vec<String> {
    match r {
        Ok(p) => panic!("expected fixes, got changed {:?}", p.changed),
        Err(f) => f.iter().map(ToString::to_string).collect(),
    }
}

fn music_notes(p: &Prepared) -> Vec<&String> {
    p.changed
        .iter()
        .filter(|c| c.starts_with("music "))
        .collect()
}

// ---------------------------------------------------------------------------
// The words
// ---------------------------------------------------------------------------

#[test]
fn the_eight_words_are_read_in_any_case_without_a_note() {
    let base = ok(Profile::Weak, None);
    for word in MusicWord::ALL {
        let name = word.as_str();
        for spelled in [
            name.to_string(),
            name.to_uppercase(),
            format!(" {} ", capitalised(name)),
        ] {
            for profile in [Profile::Weak, Profile::Creator, Profile::Operator] {
                let p = ok(profile, Some(json!(spelled)));
                assert_eq!(p.request.music, word, "{spelled:?} {}", profile.name());
                assert!(music_notes(&p).is_empty(), "{spelled:?}: {:?}", p.changed);
                // Strict reads an exact word (any case) too.
                let s = prepare(profile, &make(Some(json!(spelled)), true)).unwrap();
                assert_eq!(s.request.music, word, "{spelled:?} strict");
            }
        }
        let same = ok(Profile::Weak, Some(json!(name)));
        if word == MusicWord::Auto {
            assert_eq!(id_of(&same), id_of(&base));
        } else {
            assert_ne!(id_of(&same), id_of(&base), "{name} is another job");
        }
    }
}

fn capitalised(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

#[test]
fn synonyms_map_to_a_word_with_a_note() {
    let table: [(&str, MusicWord); 20] = [
        ("no music", MusicWord::None),
        ("No Music", MusicWord::None),
        ("without music", MusicWord::None),
        ("music off", MusicWord::None),
        ("silent", MusicWord::None),
        ("chill", MusicWord::Calm),
        ("relaxing music", MusicWord::Calm),
        ("calm and relaxing", MusicWord::Calm),
        ("epic", MusicWord::Dramatic),
        ("Epic cinematic", MusicWord::Dramatic),
        ("intense", MusicWord::Dramatic),
        ("happy", MusicWord::Upbeat),
        ("energetic", MusicWord::Upbeat),
        ("somber", MusicWord::Serious),
        ("sad music", MusicWord::Serious),
        ("fun", MusicWord::Playful),
        ("quirky", MusicWord::Playful),
        ("corporate", MusicWord::Neutral),
        ("background music", MusicWord::Neutral),
        ("default", MusicWord::Auto),
    ];
    for (said, word) in table {
        let p = ok(Profile::Weak, Some(json!(said)));
        assert_eq!(p.request.music, word, "{said:?}");
        let note = format!("music '{said}' → '{}'", word.as_str());
        assert!(
            p.changed.contains(&note),
            "{said:?}: wanted {note:?} in {:?}",
            p.changed
        );
        // The synonym is the same job as the word it stands for.
        assert_eq!(
            id_of(&p),
            id_of(&ok(Profile::Weak, Some(json!(word.as_str())))),
            "{said:?}"
        );
    }
    // JSON false is "no music"; true is the default.
    assert_eq!(
        ok(Profile::Weak, Some(json!(false))).request.music,
        MusicWord::None
    );
    assert_eq!(
        ok(Profile::Weak, Some(json!(true))).request.music,
        MusicWord::Auto
    );
    // A list of words reads as the phrase.
    assert_eq!(
        ok(Profile::Weak, Some(json!(["calm", "relaxing"])))
            .request
            .music,
        MusicWord::Calm
    );
}

#[test]
fn the_synonym_table_is_total_and_never_guesses_across_words() {
    // Direct: exact words, then phrases whose words all agree.
    assert_eq!(music_word_for("Calm"), Some((MusicWord::Calm, true)));
    assert_eq!(music_word_for("chill"), Some((MusicWord::Calm, false)));
    assert_eq!(music_word_for("no music"), Some((MusicWord::None, false)));
    assert_eq!(music_word_for("epic"), Some((MusicWord::Dramatic, false)));
    // Words that disagree, negate or mean nothing are unknown.
    for unknown in [
        "upbeat but serious",
        "not dramatic",
        "no dramatic",
        "xyz",
        "calm background",
        "music",
        "",
        "   ",
        "120 bpm",
    ] {
        assert_eq!(music_word_for(unknown), None, "{unknown:?}");
    }
}

#[test]
fn an_unknown_word_is_dropped_with_a_note() {
    let base = id_of(&ok(Profile::Weak, None));
    for v in [
        json!("xyz"),
        json!("upbeat but serious"),
        json!("not dramatic"),
        json!(5),
        json!({"mood": "calm"}),
        json!([1, 2]),
    ] {
        let p = ok(Profile::Weak, Some(v.clone()));
        assert_eq!(p.request.music, MusicWord::Auto, "{v}");
        assert_eq!(id_of(&p), base, "{v}");
        assert!(
            p.changed
                .iter()
                .any(|c| c.starts_with("music ") && c.ends_with("unknown, used auto")),
            "{v}: {:?}",
            p.changed
        );
    }
    let x = ok(Profile::Weak, Some(json!("xyz")));
    assert!(
        x.changed
            .contains(&"music 'xyz' unknown, used auto".to_string()),
        "{:?}",
        x.changed
    );
    // Empty and null mean "not given": no note.
    for v in [json!(""), json!("  "), Value::Null] {
        let p = ok(Profile::Weak, Some(v.clone()));
        assert_eq!(id_of(&p), base, "{v}");
        assert!(music_notes(&p).is_empty(), "{v}: {:?}", p.changed);
    }
}

#[test]
fn strict_refuses_a_synonym_and_an_unknown_word_with_a_fix() {
    let f = fixes(prepare(Profile::Weak, &make(Some(json!("xyz")), true)));
    assert_eq!(
        f,
        vec![
            "music: 'xyz' is not a music word — use \"calm\" or \"upbeat\" or \"dramatic\", or leave music out"
        ]
    );
    // A synonym is an auto-fix, so strict names the word to use instead.
    let f = fixes(prepare(Profile::Weak, &make(Some(json!("chill")), true)));
    assert_eq!(f, vec!["music: 'chill' is not a music word — use \"calm\""]);
    let f = fixes(prepare(Profile::Weak, &make(Some(json!("no music")), true)));
    assert_eq!(
        f,
        vec!["music: 'no music' is not a music word — use \"none\""]
    );
    // Every Fix line fits the reply's line cap.
    for bad in ["x".repeat(80), "xyz".to_string()] {
        for line in fixes(prepare(Profile::Weak, &make(Some(json!(bad)), true))) {
            assert!(line.chars().count() <= 110, "{line}");
        }
    }
}

// ---------------------------------------------------------------------------
// The job id and the stored request
// ---------------------------------------------------------------------------

#[test]
fn the_word_is_in_the_key_input_only_when_it_is_not_auto() {
    let auto = ok(Profile::Weak, None);
    let mut keys: Vec<&str> = auto
        .key
        .input
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["assets", "intent", "options", "style"]);
    for given in [json!("auto"), json!("Auto"), json!("default"), json!("")] {
        assert_eq!(
            canonical_json(&ok(Profile::Weak, Some(given)).key.input),
            canonical_json(&auto.key.input)
        );
    }
    let calm = ok(Profile::Weak, Some(json!("calm")));
    assert_eq!(calm.key.input["music"], json!("calm"));
    // Everything else is the auto key: the word is the only difference.
    let mut without = calm.key.input.clone();
    without.as_object_mut().unwrap().remove("music");
    assert_eq!(canonical_json(&without), canonical_json(&auto.key.input));
    // Each word is its own job, and a take is independent of it.
    let ids: std::collections::BTreeSet<String> = MusicWord::ALL
        .iter()
        .map(|w| id_of(&ok(Profile::Weak, Some(json!(w.as_str())))))
        .collect();
    assert_eq!(ids.len(), MusicWord::ALL.len());
    let mut both = make(Some(json!("calm")), false);
    both.take = Some(json!(2));
    let p = prepare(Profile::Weak, &both).unwrap();
    assert_eq!(p.key.input["music"], json!("calm"));
    assert_eq!(p.key.input["take"], json!(2));
    assert_eq!(p.request.music, MusicWord::Calm);
    assert_eq!(p.request.take, 2);
}

#[test]
fn the_stored_request_records_the_word_and_old_requests_read_as_auto() {
    let auto = ok(Profile::Weak, None);
    let stored = serde_json::to_value(&auto.request).unwrap();
    assert!(stored.get("music").is_none(), "auto is not written");
    let dramatic = ok(Profile::Weak, Some(json!("dramatic")));
    let stored = serde_json::to_value(&dramatic.request).unwrap();
    assert_eq!(stored["music"], json!("dramatic"));
    let back: StoredRequest = serde_json::from_value(stored).unwrap();
    assert_eq!(back, dramatic.request);
    // A request.json written before the word existed.
    let old: StoredRequest =
        serde_json::from_value(serde_json::to_value(&auto.request).unwrap()).unwrap();
    assert_eq!(old.music, MusicWord::Auto);
}

#[test]
fn raw_arguments_carry_the_word_and_every_profile_takes_it() {
    let config = common::config(Profile::Weak, "music_word_raw");
    let raw = json!({"story": two_beats(), "music": "Chill"});
    let (args, p) = policy::prepare_raw(&raw, &common::ctx(&config));
    assert_eq!(args.music, Some(json!("Chill")));
    let p = p.unwrap();
    assert_eq!(p.request.music, MusicWord::Calm);
    assert!(p.changed.contains(&"music 'Chill' → 'calm'".to_string()));
    // `null` is not given.
    let (args, p) = policy::prepare_raw(
        &json!({"story": two_beats(), "music": null}),
        &common::ctx(&config),
    );
    assert_eq!(args.music, None);
    assert_eq!(p.unwrap().request.music, MusicWord::Auto);
    for profile in [Profile::Weak, Profile::Creator, Profile::Operator] {
        let p = ok(profile, Some(json!("serious")));
        assert_eq!(p.request.music, MusicWord::Serious, "{}", profile.name());
        assert_ne!(id_of(&p), id_of(&ok(profile, None)));
    }
    // The creator's `options.music` (a bed id) is another thing: it does not
    // set the word.
    let mut creator = make(None, false);
    creator.options = Some(motion_mcp::args::ProductionOptions {
        music: Some("warm_piano".into()),
        ..Default::default()
    });
    let p = prepare(Profile::Creator, &creator).unwrap();
    assert_eq!(p.request.music, MusicWord::Auto);
    assert_eq!(p.options.music.as_deref(), Some("warm_piano"));
}

// ---------------------------------------------------------------------------
// revise_video
// ---------------------------------------------------------------------------

#[test]
fn revise_keeps_the_word_unless_a_new_one_is_given() {
    let dramatic = ok(Profile::Weak, Some(json!("dramatic")));
    let kept = revise(&dramatic, None, false).unwrap();
    assert_eq!(kept.request.music, MusicWord::Dramatic);
    assert_eq!(kept.key.input["music"], json!("dramatic"));
    let changed = revise(&dramatic, Some(json!("calm")), false).unwrap();
    assert_eq!(changed.request.music, MusicWord::Calm);
    assert_eq!(changed.key.input["music"], json!("calm"));
    assert_ne!(id_of(&kept), id_of(&changed));
    // Back to auto: the key has no word, like a fresh request.
    let auto = revise(&dramatic, Some(json!("auto")), false).unwrap();
    assert_eq!(auto.request.music, MusicWord::Auto);
    assert!(auto.key.input.get("music").is_none());
    assert_eq!(
        id_of(&auto),
        id_of(&revise(&ok(Profile::Weak, None), None, false).unwrap())
    );
    // A job without a word takes one on revise.
    let from_auto = revise(&ok(Profile::Weak, None), Some(json!("epic")), false).unwrap();
    assert_eq!(from_auto.request.music, MusicWord::Dramatic);
    assert!(from_auto
        .changed
        .contains(&"music 'epic' → 'dramatic'".to_string()));
    // A word that does not parse leaves the job's.
    let bad = revise(&dramatic, Some(json!("xyz")), false).unwrap();
    assert_eq!(bad.request.music, MusicWord::Dramatic);
    assert!(bad
        .changed
        .contains(&"music 'xyz' unknown, used dramatic".to_string()));
    let f = fixes(revise(&dramatic, Some(json!("xyz")), true));
    assert_eq!(f.len(), 1);
    assert!(
        f[0].starts_with("music: 'xyz' is not a music word"),
        "{f:?}"
    );
    // The take and the word are kept independently.
    let mut both = make(Some(json!("calm")), false);
    both.take = Some(json!(3));
    let p = prepare(Profile::Weak, &both).unwrap();
    let r = revise(&p, None, false).unwrap();
    assert_eq!((r.request.take, r.request.music), (3, MusicWord::Calm));
}

// ---------------------------------------------------------------------------
// The reel's command line and output
// ---------------------------------------------------------------------------

fn job_of(request: &StoredRequest) -> EngineJob {
    EngineJob {
        id: JobId::parse("j_0123456789").unwrap(),
        dir: PathBuf::from("/repo/output/jobs/j_0123456789"),
        request: request.clone(),
        title: "t".into(),
        has_manifest: false,
    }
}

fn line(request: &StoredRequest) -> String {
    let config = common::config(Profile::Weak, "music_word_args");
    engine::reel_args(&config, &job_of(request))
        .iter()
        .map(|a: &OsString| a.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn the_reel_gets_music_mood_only_when_the_word_is_not_auto() {
    let auto = ok(Profile::Weak, None);
    assert!(!line(&auto.request).contains("--music-mood"));
    for word in MusicWord::ALL.into_iter().filter(|w| *w != MusicWord::Auto) {
        let p = ok(Profile::Weak, Some(json!(word.as_str())));
        let l = line(&p.request);
        assert_eq!(l.matches("--music-mood").count(), 1, "{l}");
        assert!(
            l.contains(&format!("--music-mood {}", word.as_str())),
            "{l}"
        );
        // The bed option of the creator profile stays `--music auto`.
        assert!(l.contains("--music auto"), "{l}");
    }
    // The take and the word reach the command line together.
    let mut both = make(Some(json!("playful")), false);
    both.take = Some(json!(4));
    let l = line(&prepare(Profile::Weak, &both).unwrap().request);
    assert!(l.contains("--take 4 --music-mood playful"), "{l}");
}

fn out(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|l| l.to_string()).collect()
}

#[test]
fn the_reply_line_names_the_bed_and_the_reason() {
    let stdout = out(&[
        "[1/5] voice (auto · emotion precision)",
        "[3/5] music: tech_pulse (neutral explainer, money story)",
        "[2/5] compile (art auto, variety auto)",
    ]);
    assert_eq!(
        engine::music_finding(&stdout).as_deref(),
        Some("music: tech_pulse (neutral explainer, money story)")
    );
    // No bed fits: said plainly.
    let none = out(&["[3/5] music: none (no bed fits a tense story; silence)"]);
    assert_eq!(
        engine::music_finding(&none).as_deref(),
        Some("music: none (no bed fits a tense story; silence)")
    );
    // The word `none` explains itself; a bare `none` says nothing new.
    assert_eq!(
        engine::music_finding(&out(&["[3/5] music: none (no music requested)"])).as_deref(),
        Some("music: none (no music requested)")
    );
    assert_eq!(engine::music_finding(&out(&["[3/5] music: none"])), None);
    assert_eq!(engine::music_finding(&out(&["[2/5] compile"])), None);
}

#[test]
fn music_fit_is_a_finding_without_its_tail() {
    let stdout = out(&[
        "warning[carry_ignored]: beat 2: a carry was dropped",
        "warning[music_fit]: the music word 'dramatic' asks for dramatic music, but this story reads neutral explainer; using dramatic music as asked",
        "warning[cue_clamped]: beat 1: a cue moved",
    ]);
    let got = engine::compile_warnings(&stdout);
    // Story-level warnings first (music_fit among them), the rest after.
    assert_eq!(
        got,
        [
            "beat 2: a carry was dropped",
            "the music word 'dramatic' asks for dramatic music, but this story reads neutral explainer",
            "beat 1: a cue moved",
        ]
    );
}

// ---------------------------------------------------------------------------
// The reply of a finished video (a stand-in engine script)
// ---------------------------------------------------------------------------

/// A stand-in `motion-engine`: `reel` is `tests/fixtures/fake_engine.sh` with
/// its `[3/5] music: none` line replaced by the lines of `music_lines.txt`,
/// and every reel's arguments appended to `reel_args.txt`.
struct Stub {
    dir: PathBuf,
}

impl Stub {
    fn new(name: &str) -> Stub {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("music_word_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fake = common::fixtures().join("fake_engine.sh");
        let script = format!(
            r#"#!/usr/bin/env bash
here="$(cd "$(dirname "$0")" && pwd)"
if [ "${{1:-}}" = "reel" ]; then
  echo "$*" >> "$here/reel_args.txt"
  out=$("{fake}" "$@") || exit $?
  while IFS= read -r l; do
    if [ "$l" = "[3/5] music: none" ] && [ -s "$here/music_lines.txt" ]; then
      cat "$here/music_lines.txt"
    else
      printf '%s\n' "$l"
    fi
  done <<< "$out"
else
  exec "{fake}" "$@"
fi
"#,
            fake = fake.display()
        );
        let path = dir.join("engine.sh");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        Stub { dir }
    }

    fn says(&self, lines: &[&str]) {
        std::fs::write(self.dir.join("music_lines.txt"), lines.join("\n") + "\n").unwrap();
    }

    fn reel_runs(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.join("reel_args.txt"))
            .map(|t| t.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }
}

fn has_ffmpeg() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn findings(structured: &Value) -> Vec<String> {
    structured["findings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| f.as_str().map(str::to_string))
        .collect()
}

#[test]
fn the_done_reply_names_the_bed_the_reason_and_a_music_fit_warning() {
    if !has_ffmpeg() {
        eprintln!("skipping: no ffmpeg (the stand-in engine makes a 1 s mp4)");
        return;
    }
    let stub = Stub::new("reply");
    let mut config = common::config(Profile::Weak, "music_word_service");
    config.engine = stub.dir.join("engine.sh");
    config.max_wait_s = 60;
    let jobs = config.jobs.clone();
    let _ = std::fs::remove_dir_all(&jobs);
    let svc = Service::new(config, Arc::new(SubprocessEngine)).unwrap();
    let ctx = CallCtx::new();

    // 1. music auto: the bed and the reason, from the reel's own line.
    stub.says(&["[3/5] music: tech_pulse (neutral explainer, money story)"]);
    let first = svc.call(profile::MAKE_VIDEO, json!({"story": two_beats()}), &ctx);
    assert_eq!(first.structured["status"], "done", "{}", first.text);
    assert_eq!(
        findings(&first.structured),
        ["music: tech_pulse (neutral explainer, money story)"],
        "{}",
        first.text
    );
    assert!(first
        .text
        .contains("music: tech_pulse (neutral explainer, money story)"));
    assert!(
        !stub.reel_runs()[0].contains("--music-mood"),
        "{:?}",
        stub.reel_runs()
    );
    let job0 = first.structured["job"].as_str().unwrap().to_string();

    // 2. A word that conflicts: the bed, the reason and the music_fit warning.
    stub.says(&[
        "[3/5] music: cinematic_strings (dramatic, as asked)",
        "warning[music_fit]: the music word 'dramatic' asks for dramatic music, but this story reads neutral explainer; using dramatic music as asked",
    ]);
    let second = svc.call(
        profile::MAKE_VIDEO,
        json!({"story": two_beats(), "music": "epic"}),
        &ctx,
    );
    assert_eq!(second.structured["status"], "done", "{}", second.text);
    let job1 = second.structured["job"].as_str().unwrap().to_string();
    assert_ne!(job1, job0, "the word is part of the job");
    let f = findings(&second.structured);
    assert_eq!(
        f,
        [
            "the music word 'dramatic' asks for dramatic music, but this story reads neutral explainer",
            "music: cinematic_strings (dramatic, as asked)",
        ],
        "{}",
        second.text
    );
    // The synonym was noted, and the word reached the reel.
    let changed: Vec<&str> = second.structured["changed"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        changed.contains(&"music 'epic' → 'dramatic'"),
        "{changed:?}"
    );
    let runs = stub.reel_runs();
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert!(runs[1].contains("--music-mood dramatic"), "{}", runs[1]);

    // 3. get_video of the finished job says the same.
    let got = svc.call(profile::GET_VIDEO, json!({"job": job1, "wait_s": 0}), &ctx);
    assert_eq!(findings(&got.structured), f);

    // 4. revise_video keeps the word (so the reel gets it again) and may
    //    change it.
    stub.says(&["[3/5] music: cinematic_strings (dramatic, as asked)"]);
    let kept = svc.call(
        profile::REVISE_VIDEO,
        json!({"job": job1, "style": "cinematic"}),
        &ctx,
    );
    assert_eq!(kept.structured["status"], "done", "{}", kept.text);
    let runs = stub.reel_runs();
    assert_eq!(runs.len(), 3, "{runs:?}");
    assert!(runs[2].contains("--music-mood dramatic"), "{}", runs[2]);
    stub.says(&["[3/5] music: calm_ambient (calm, as asked)"]);
    let moved = svc.call(
        profile::REVISE_VIDEO,
        json!({"job": job1, "style": "cinematic", "music": "calm"}),
        &ctx,
    );
    assert_eq!(moved.structured["status"], "done", "{}", moved.text);
    let runs = stub.reel_runs();
    assert_eq!(runs.len(), 4, "{runs:?}");
    assert!(runs[3].contains("--music-mood calm"), "{}", runs[3]);
    assert_eq!(
        findings(&moved.structured),
        ["music: calm_ambient (calm, as asked)"]
    );

    // 5. Silence is said plainly; a bare `none` adds no line.
    stub.says(&["[3/5] music: none (no bed fits a somber story; silence)"]);
    let quiet = svc.call(
        profile::MAKE_VIDEO,
        json!({"story": two_beats(), "music": "serious"}),
        &ctx,
    );
    assert_eq!(
        findings(&quiet.structured),
        ["music: none (no bed fits a somber story; silence)"]
    );
    stub.says(&["[3/5] music: none"]);
    let bare = svc.call(
        profile::MAKE_VIDEO,
        json!({"story": two_beats(), "music": "none"}),
        &ctx,
    );
    assert_eq!(bare.structured["status"], "done", "{}", bare.text);
    assert!(findings(&bare.structured).is_empty(), "{}", bare.text);

    svc.shutdown();
    let _ = std::fs::remove_dir_all(&jobs);
    let _ = std::fs::remove_dir_all(&stub.dir);
}

// ---------------------------------------------------------------------------
// list_options and the v0.2 catalog
// ---------------------------------------------------------------------------

#[test]
fn list_options_shows_the_moods_and_energy_of_every_bed() {
    let config = common::config(Profile::Creator, "music_word_options");
    let data = motion_mcp::creator::options_json(&config);
    let beds = data["music"].as_array().expect("music beds");
    assert!(beds.len() >= 5, "{beds:?}");
    for b in beds {
        assert!(b["emotions"].is_array(), "{b}: the v0.1 emotions are kept");
        assert!(
            b["moods"].as_array().is_some_and(|m| !m.is_empty()),
            "{b}: v0.2 moods"
        );
        assert!(
            b["energy"].as_u64().is_some_and(|e| (1..=5).contains(&e)),
            "{b}: v0.2 energy"
        );
    }
    let tech = beds.iter().find(|b| b["id"] == "tech_pulse").unwrap();
    assert_eq!(tech["moods"], json!(["neutral_explainer", "serious"]));
    assert!(
        data["use"]
            .as_str()
            .unwrap()
            .contains("A mood word as music"),
        "{}",
        data["use"]
    );
    // The text of the music topic says what each bed is for.
    let topic_config = common::config(Profile::Creator, "music_word_options_topic");
    let jobs = topic_config.jobs.clone();
    let _ = std::fs::remove_dir_all(&jobs);
    let svc = Service::new(topic_config, Arc::new(SubprocessEngine)).unwrap();
    let out = svc.call(
        profile::LIST_OPTIONS,
        json!({"topic": "music"}),
        &CallCtx::new(),
    );
    assert!(!out.is_error, "{}", out.text);
    assert!(out.text.contains("tech_pulse:"), "{}", out.text);
    assert!(
        out.text.contains("neutral_explainer, serious"),
        "{}",
        out.text
    );
    assert!(out.text.contains("energy 3"), "{}", out.text);
    svc.shutdown();
    let _ = std::fs::remove_dir_all(&jobs);
}
