//! (0.23 B1) Takes: the optional `take` (0-99) of `make_video` / `revise_video`
//! is normalised leniently, is part of the job id only when it is not 0, is
//! kept by `revise_video` unless changed, and is mentioned once in the reply
//! of a finished video. Offline: policy tests call `prepare` / `revise`
//! directly; the reply tests run the real `Service` over a stand-in engine.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use motion_mcp::args::{MakeVideoArgs, ReviseVideoArgs};
use motion_mcp::engine::{Cancel, Engine, EngineJob, EngineOutput};
use motion_mcp::job::{canonical_json, job_id, JobId, Stage, StoredRequest};
use motion_mcp::policy::{self, Prepared};
use motion_mcp::profile::{self, Profile};
use motion_mcp::reply::{Fix, Qa};
use motion_mcp::service::{CallCtx, Service};
use serde_json::{json, Value};

fn two_beats() -> Value {
    json!({"title": "policy test", "beats": [
        {"say": "Costs fell sharply while the output almost tripled this year."},
        {"say": "Nobody on the team expected the change to come this fast."}
    ]})
}

fn make(take: Option<Value>, strict: bool) -> MakeVideoArgs {
    MakeVideoArgs {
        story: two_beats(),
        take,
        strict,
        ..Default::default()
    }
}

fn prepare(profile: Profile, a: &MakeVideoArgs) -> Result<Prepared, Vec<Fix>> {
    let config = common::config(profile, "takes");
    policy::prepare(a, &common::ctx(&config))
}

fn ok(profile: Profile, take: Option<Value>) -> Prepared {
    prepare(profile, &make(take, false)).unwrap_or_else(|f| panic!("needs_fix: {f:#?}"))
}

fn id_of(p: &Prepared) -> String {
    job_id(&p.key).to_string()
}

fn revise(p: &Prepared, take: Option<Value>, strict: bool) -> Result<Prepared, Vec<Fix>> {
    let config = common::config(Profile::Weak, "takes_revise");
    policy::revise(
        &p.request,
        &ReviseVideoArgs {
            job: id_of(p),
            style: Some(json!("cinematic")),
            take,
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

// ---------------------------------------------------------------------------
// The job id
// ---------------------------------------------------------------------------

#[test]
fn same_take_same_id_different_take_different_id() {
    let t1 = ok(Profile::Weak, Some(json!(1)));
    assert_eq!(id_of(&t1), id_of(&ok(Profile::Weak, Some(json!(1)))));
    let t0 = ok(Profile::Weak, None);
    assert_ne!(id_of(&t1), id_of(&t0), "take 1 vs 0");
    assert_ne!(id_of(&t1), id_of(&ok(Profile::Weak, Some(json!(2)))));
    // Take 0 given explicitly is the request without a take.
    for zero in [json!(0), json!("0"), json!(0.0), Value::Null] {
        assert_eq!(
            id_of(&t0),
            id_of(&ok(Profile::Weak, Some(zero.clone()))),
            "{zero}"
        );
    }
    assert_eq!(t0.request.take, 0);
    assert_eq!(t1.request.take, 1);
}

#[test]
fn the_take_is_in_the_key_input_only_when_it_is_not_zero() {
    let t0 = ok(Profile::Weak, None);
    let mut keys: Vec<&str> = t0
        .key
        .input
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["assets", "intent", "options", "style"]);
    let t5 = ok(Profile::Weak, Some(json!(5)));
    assert_eq!(t5.key.input["take"], json!(5));
    // Everything else is the take-0 key: the take is the only difference.
    let mut without = t5.key.input.clone();
    without.as_object_mut().unwrap().remove("take");
    assert_eq!(canonical_json(&without), canonical_json(&t0.key.input));
}

#[test]
fn the_stored_request_records_the_take_and_old_requests_read_as_take_zero() {
    let t0 = ok(Profile::Weak, None);
    let stored = serde_json::to_value(&t0.request).unwrap();
    assert!(stored.get("take").is_none(), "take 0 is not written");
    let t3 = ok(Profile::Weak, Some(json!(3)));
    let stored = serde_json::to_value(&t3.request).unwrap();
    assert_eq!(stored["take"], json!(3));
    let back: StoredRequest = serde_json::from_value(stored).unwrap();
    assert_eq!(back, t3.request);
    // A request.json written before takes existed.
    let old: StoredRequest =
        serde_json::from_value(serde_json::to_value(&t0.request).unwrap()).unwrap();
    assert_eq!(old.take, 0);
}

#[test]
fn every_profile_takes_a_take() {
    for profile in [Profile::Weak, Profile::Creator, Profile::Operator] {
        let p = ok(profile, Some(json!(1)));
        assert_eq!(p.request.take, 1, "{}", profile.name());
        assert!(p.changed.is_empty(), "{:?}", p.changed);
        assert_ne!(id_of(&p), id_of(&ok(profile, None)));
    }
}

#[test]
fn raw_arguments_carry_the_take() {
    let config = common::config(Profile::Weak, "takes_raw");
    let raw = json!({"story": two_beats(), "take": "2"});
    let (args, p) = policy::prepare_raw(&raw, &common::ctx(&config));
    assert_eq!(args.take, Some(json!("2")));
    let p = p.unwrap();
    assert_eq!(p.request.take, 2);
    assert!(
        !p.changed.iter().any(|c| c.contains("take")),
        "a string take is read quietly: {:?}",
        p.changed
    );
    assert_eq!(id_of(&p), id_of(&ok(Profile::Weak, Some(json!(2)))));
}

// ---------------------------------------------------------------------------
// Lenient parsing
// ---------------------------------------------------------------------------

#[test]
fn a_numeric_string_is_a_take() {
    let two = id_of(&ok(Profile::Weak, Some(json!(2))));
    for v in [
        json!("2"),
        json!(" 2 "),
        json!(2.0),
        json!("2.0"),
        json!("02"),
    ] {
        let p = ok(Profile::Weak, Some(v.clone()));
        assert_eq!(p.request.take, 2, "{v}");
        assert_eq!(id_of(&p), two, "{v}");
        // Strict is no stricter about a number written as text.
        let strict = prepare(Profile::Weak, &make(Some(v.clone()), true)).unwrap();
        assert_eq!(strict.request.take, 2, "{v}");
    }
    assert_eq!(ok(Profile::Weak, Some(json!(99))).request.take, 99);
}

#[test]
fn out_of_range_is_clamped_with_a_note() {
    let high = ok(Profile::Weak, Some(json!(100)));
    assert_eq!(high.request.take, 99);
    assert_eq!(id_of(&high), id_of(&ok(Profile::Weak, Some(json!(99)))));
    assert!(
        high.changed
            .contains(&"take 100 → 99 (takes are 0 to 99)".to_string()),
        "{:?}",
        high.changed
    );
    let huge = ok(Profile::Weak, Some(json!("1e30")));
    assert_eq!(huge.request.take, 99);
    let low = ok(Profile::Weak, Some(json!(-1)));
    assert_eq!(low.request.take, 0);
    assert_eq!(id_of(&low), id_of(&ok(Profile::Weak, None)));
    assert!(
        low.changed
            .contains(&"take -1 → 0 (takes are 0 to 99)".to_string()),
        "{:?}",
        low.changed
    );
}

#[test]
fn not_a_number_is_dropped_with_a_note() {
    let base = id_of(&ok(Profile::Weak, None));
    for v in [
        json!("x"),
        json!(true),
        json!(1.5),
        json!("NaN"),
        json!([1]),
        json!({}),
    ] {
        let p = ok(Profile::Weak, Some(v.clone()));
        assert_eq!(p.request.take, 0, "{v}");
        assert_eq!(id_of(&p), base, "{v}");
        assert!(
            p.changed
                .iter()
                .any(|c| c.starts_with("take ") && c.contains("ignored")),
            "{v}: {:?}",
            p.changed
        );
    }
    let x = ok(Profile::Weak, Some(json!("x")));
    assert!(
        x.changed
            .contains(&"take 'x' ignored (not a whole number from 0 to 99)".to_string()),
        "{:?}",
        x.changed
    );
}

#[test]
fn strict_refuses_a_bad_take_with_a_fix() {
    let want = |v: Value, text: &str| {
        let f = fixes(prepare(Profile::Weak, &make(Some(v.clone()), true)));
        assert_eq!(
            f,
            vec![format!(
                "take: {text} is not a take number (a whole number from 0 to 99) — use \"1\" or \"2\" or \"3\", or leave take out"
            )],
            "{v}"
        );
    };
    want(json!(100), "100");
    want(json!(-1), "-1");
    want(json!("x"), "'x'");
    want(json!(1.5), "1.5");
    // A good take under strict is fine.
    assert_eq!(
        prepare(Profile::Weak, &make(Some(json!(7)), true))
            .unwrap()
            .request
            .take,
        7
    );
}

// ---------------------------------------------------------------------------
// revise_video
// ---------------------------------------------------------------------------

#[test]
fn revise_keeps_the_stored_take_unless_it_is_changed() {
    let t2 = ok(Profile::Weak, Some(json!(2)));
    let kept = revise(&t2, None, false).unwrap();
    assert_eq!(kept.request.take, 2, "no take given keeps the job's");
    assert_eq!(kept.key.input["take"], json!(2));
    let changed = revise(&t2, Some(json!(3)), false).unwrap();
    assert_eq!(changed.request.take, 3);
    assert_eq!(changed.key.input["take"], json!(3));
    // Back to the default take: the key has no take, like a fresh request.
    let zero = revise(&t2, Some(json!(0)), false).unwrap();
    assert_eq!(zero.request.take, 0);
    assert!(zero.key.input.get("take").is_none());
    // The same revision of a take-0 job and of a take-3 job are different
    // jobs, and taking a take on revise works from take 0.
    let t0 = ok(Profile::Weak, None);
    assert_eq!(revise(&t0, None, false).unwrap().request.take, 0);
    let from0 = revise(&t0, Some(json!("4")), false).unwrap();
    assert_eq!(from0.request.take, 4);
    assert_ne!(id_of(&from0), id_of(&revise(&t0, None, false).unwrap()));
    assert_ne!(id_of(&kept), id_of(&changed));
    // A revision that changes nothing but the take is the take's own job.
    let same_story = revise(&t2, Some(json!(2)), false).unwrap();
    assert_eq!(id_of(&same_story), id_of(&kept));
}

#[test]
fn revise_ignores_a_bad_take_and_keeps_the_jobs() {
    let t2 = ok(Profile::Weak, Some(json!(2)));
    let bad = revise(&t2, Some(json!("x")), false).unwrap();
    assert_eq!(bad.request.take, 2, "a dropped take leaves the job's");
    assert!(bad
        .changed
        .iter()
        .any(|c| c == "take 'x' ignored (not a whole number from 0 to 99)"));
    let high = revise(&t2, Some(json!(100)), false).unwrap();
    assert_eq!(high.request.take, 99);
    let f = fixes(revise(&t2, Some(json!("x")), true));
    assert_eq!(f.len(), 1);
    assert!(f[0].starts_with("take: 'x' is not a take number"), "{f:?}");
}

// ---------------------------------------------------------------------------
// The reply of a finished video (a stand-in engine; nothing renders)
// ---------------------------------------------------------------------------

struct Quick {
    runs: AtomicUsize,
}

impl Engine for Quick {
    fn run(
        &self,
        _config: &motion_mcp::ServerConfig,
        job: &EngineJob,
        stage: &dyn Fn(Stage),
        _cancel: &Arc<Cancel>,
    ) -> Result<EngineOutput, String> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        stage(Stage::Render);
        let video = job.dir.join(motion_mcp::job::VIDEO_MP4);
        std::fs::write(&video, b"mp4").map_err(|e| e.to_string())?;
        Ok(EngineOutput {
            video,
            preview: None,
            duration_s: Some(8.0),
            qa: Some(Qa::Pass),
            findings: Vec::new(),
        })
    }
}

fn service(name: &str) -> (Service, Arc<Quick>, std::path::PathBuf) {
    let mut config = common::config(Profile::Weak, name);
    config.max_wait_s = 30;
    let jobs = config.jobs.clone();
    let _ = std::fs::remove_dir_all(&jobs);
    let engine = Arc::new(Quick {
        runs: AtomicUsize::new(0),
    });
    (Service::new(config, engine.clone()).unwrap(), engine, jobs)
}

fn next_of(structured: &Value) -> String {
    structured["next"].as_str().unwrap_or_default().to_string()
}

#[test]
fn a_finished_video_mentions_the_next_take_once() {
    let (svc, engine, jobs) = service("takes_reply");
    let ctx = CallCtx::new();
    let call = |args: Value| svc.call(profile::MAKE_VIDEO, args, &ctx);

    // Take 0 (nothing given): "take: 1 for another version", once.
    let first = call(json!({"story": two_beats()}));
    assert_eq!(first.structured["status"], "done", "{}", first.text);
    let next = next_of(&first.structured);
    assert!(
        next.starts_with("Done. Call revise_video with changes if needed."),
        "{next}"
    );
    assert_eq!(
        next.matches("take: 1 for another version").count(),
        1,
        "{next}"
    );
    assert_eq!(first.text.matches("take:").count(), 1, "{}", first.text);
    let job0 = first.structured["job"].as_str().unwrap().to_string();

    // Take 1: its own job, which points at take 2.
    let second = call(json!({"story": two_beats(), "take": 1}));
    let job1 = second.structured["job"].as_str().unwrap().to_string();
    assert_ne!(job1, job0);
    assert!(next_of(&second.structured).contains("take: 2 for another version"));
    assert_eq!(engine.runs.load(Ordering::SeqCst), 2);

    // The same request again: the same job, no new render, the same hint.
    let again = call(json!({"story": two_beats(), "take": "1"}));
    assert_eq!(again.structured["job"], job1.as_str());
    assert_eq!(engine.runs.load(Ordering::SeqCst), 2);
    assert!(next_of(&again.structured).contains("take: 2 for another version"));

    // The request file records the take; take 0 writes none.
    let request = |job: &str| -> Value {
        let text = std::fs::read_to_string(jobs.join(job).join("request.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    };
    assert_eq!(request(&job1)["take"], json!(1));
    assert!(request(&job0).get("take").is_none());

    // get_video of a finished job says the same.
    let got = svc.call(profile::GET_VIDEO, json!({"job": job1, "wait_s": 0}), &ctx);
    assert!(next_of(&got.structured).contains("take: 2 for another version"));

    // revise_video keeps the take (and so points at the following one), and
    // a given take changes it.
    let kept = svc.call(
        profile::REVISE_VIDEO,
        json!({"job": job1, "style": "cinematic"}),
        &ctx,
    );
    assert_eq!(kept.structured["status"], "done", "{}", kept.text);
    let kept_job = kept.structured["job"].as_str().unwrap().to_string();
    assert!(next_of(&kept.structured).contains("take: 2 for another version"));
    assert_eq!(request(&kept_job)["take"], json!(1));
    let moved = svc.call(
        profile::REVISE_VIDEO,
        json!({"job": job1, "style": "cinematic", "take": 5}),
        &ctx,
    );
    let moved_job = moved.structured["job"].as_str().unwrap().to_string();
    assert!(next_of(&moved.structured).contains("take: 6 for another version"));
    assert_eq!(request(&moved_job)["take"], json!(5));
    assert_ne!(moved_job, kept_job);
    assert_eq!(
        JobId::parse(&moved_job).unwrap().as_str(),
        moved_job.as_str()
    );

    // Past the last take there is nothing to suggest; other statuses carry
    // no take text.
    let last = call(json!({"story": two_beats(), "take": 99}));
    assert_eq!(last.structured["status"], "done");
    assert!(!next_of(&last.structured).contains("take"));
    let checked = call(json!({"story": two_beats(), "take": 3, "mode": "check"}));
    assert_eq!(checked.structured["status"], "checked");
    assert!(!next_of(&checked.structured).contains("take"));
    let bad = call(json!({"story": two_beats(), "take": "x", "strict": true}));
    assert_eq!(bad.structured["status"], "needs_fix");
    assert!(
        bad.text.contains("take: 'x' is not a take number"),
        "{}",
        bad.text
    );
    assert!(!next_of(&bad.structured).contains("another version"));
    svc.shutdown();
    let _ = std::fs::remove_dir_all(&jobs);
}
