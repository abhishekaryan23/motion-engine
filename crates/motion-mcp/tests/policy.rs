//! Policy, auto-fix, profile gating, revise and job ids (plan §7, §6, §14),
//! against the real asset library.

mod common;

use motion_mcp::args::{BeatChange, MakeVideoArgs, ProductionOptions, ReviseVideoArgs};
use motion_mcp::job::{job_id, StoryKind};
use motion_mcp::policy::{self, Prepared};
use motion_mcp::profile::Profile;
use motion_mcp::reply::Fix;
use serde_json::{json, Value};

fn sentence(words: usize) -> String {
    let pool = [
        "the", "team", "built", "a", "small", "engine", "that", "now", "runs", "every", "night",
        "without", "any", "help", "from", "people",
    ];
    let mut out: Vec<&str> = (0..words).map(|i| pool[i % pool.len()]).collect();
    if let Some(first) = out.first_mut() {
        *first = "The";
    }
    out.join(" ") + "."
}

fn story(beats: Vec<Value>) -> Value {
    json!({ "title": "policy test", "beats": beats })
}

fn args(story: Value) -> MakeVideoArgs {
    MakeVideoArgs {
        story,
        ..Default::default()
    }
}

fn prepare(profile: Profile, a: &MakeVideoArgs) -> Result<Prepared, Vec<Fix>> {
    let config = common::config(profile, "policy");
    policy::prepare(a, &common::ctx(&config))
}

fn ok(profile: Profile, a: &MakeVideoArgs) -> Prepared {
    prepare(profile, a).unwrap_or_else(|f| panic!("needs_fix: {f:#?}"))
}

fn fixes(profile: Profile, a: &MakeVideoArgs) -> Vec<String> {
    match prepare(profile, a) {
        Ok(p) => panic!("expected fixes, got changed {:?}", p.changed),
        Err(f) => f.iter().map(ToString::to_string).collect(),
    }
}

fn two_beats() -> Vec<Value> {
    vec![
        json!({"say": "Costs fell sharply while the output almost tripled this year."}),
        json!({"say": "Nobody on the team expected the change to come this fast."}),
    ]
}

// ---------------------------------------------------------------------------
// Hard limits, with their fix-it wording
// ---------------------------------------------------------------------------

#[test]
fn beat_count_limits() {
    let one = fixes(
        Profile::Weak,
        &args(story(vec![json!({"say": sentence(10)})])),
    );
    assert_eq!(
        one,
        vec!["beats: need 2 to 12 beats (got 1) — write one beat per idea, at least 2"]
    );
    let many: Vec<Value> = (0..13).map(|_| json!({"say": sentence(8)})).collect();
    let f = fixes(Profile::Weak, &args(story(many)));
    assert_eq!(
        f,
        vec!["beats: need 2 to 12 beats (got 13) — merge or remove beats"]
    );
}

#[test]
fn say_length_limits() {
    let f = fixes(
        Profile::Weak,
        &args(story(vec![
            json!({"say": "Too short."}),
            json!({"say": sentence(41)}),
        ])),
    );
    assert_eq!(
        f,
        vec![
            "beat 1 say: has 2 words, needs 3 to 40 — write one full sentence for the narrator",
            "beat 2 say: has 41 words, at most 40 — shorten it or split it into two beats",
        ]
    );
    // A beat with nothing to say at all.
    let f = fixes(
        Profile::Weak,
        &args(story(vec![
            json!({"picture": "rocket"}),
            json!({"say": sentence(9)}),
        ])),
    );
    assert_eq!(
        f,
        vec!["beat 1 say: has 0 words, needs 3 to 40 — write one full sentence for the narrator"]
    );
}

#[test]
fn total_words_and_duration_limits() {
    // 8 × 38 = 304 words.
    let f = fixes(
        Profile::Weak,
        &args(story(
            (0..8).map(|_| json!({"say": sentence(38)})).collect(),
        )),
    );
    assert!(
        f.contains(
            &"say: the narration has 304 words, at most 300 — shorten the beats or remove some"
                .to_string()
        ),
        "{f:?}"
    );
    // 12 × 25 = 300 words (allowed), but 300 / 2.5 + 12 × 0.6 = 127 s.
    let f = fixes(
        Profile::Weak,
        &args(story(
            (0..12).map(|_| json!({"say": sentence(25)})).collect(),
        )),
    );
    assert_eq!(
        f,
        vec!["beats: the video would run about 127 s, at most 120 s — shorten the narration"]
    );
}

#[test]
fn list_and_layer_rules() {
    let p = ok(
        Profile::Weak,
        &args(story(vec![
            json!({"say": sentence(10), "list": ["a1", "b2", "c3", "d4", "e5", "f6", "g7", "h8"]}),
            json!({"say": sentence(10), "list": ["cost", "output"]}),
            json!({"say": sentence(10), "layers": {"names": ["sunlit", "twilight", "midnight"], "focus": "twilite"}}),
            json!({"say": sentence(10), "layers": {"names": ["sunlit", "twilight", "midnight"], "focus": "abyss"}}),
        ])),
    );
    assert!(
        p.changed
            .contains(&"beat 1: list cut to 6 items (dropped 'g7', 'h8')".to_string()),
        "{:?}",
        p.changed
    );
    assert!(p
        .changed
        .contains(&"beat 2: list of 2 shown as compare 'cost' vs 'output'".to_string()));
    assert!(p
        .changed
        .contains(&"beat 3: focus 'twilite' → 'twilight'".to_string()));
    assert!(p
        .changed
        .contains(&"beat 4: focus 'abyss' dropped (not a layer name)".to_string()));
    let stored: policy::Parsed =
        policy::parse_story(&p.request.story).expect("stored story parses");
    let policy::Story::Lite(s) = stored.story else {
        panic!()
    };
    assert_eq!(s.beats[0].list.as_ref().map(Vec::len), Some(6));
    assert!(s.beats[1].list.is_none() && s.beats[1].compare.is_some());

    let f = fixes(
        Profile::Weak,
        &args(story(vec![
            json!({"say": sentence(10), "list": ["alone"]}),
            json!({"say": sentence(10), "layers": {"names": ["only"]}}),
        ])),
    );
    assert_eq!(
        f,
        vec![
            "beat 1 list: has 1 item(s), needs 3 to 6 — add items, or use picture or show instead",
            "beat 2 layers: has 1 layers, needs 2 to 6 — name 2 to 6 layers from top to bottom",
        ]
    );
}

#[test]
fn long_show_is_shortened() {
    let p = ok(
        Profile::Weak,
        &args(story(vec![
            json!({"say": sentence(10), "show": "First, there's the cue — a trigger that tells your brain"}),
            json!({"say": sentence(10), "show": "Costs fell while output tripled in the end"}),
        ])),
    );
    assert!(
        p.changed
            .contains(&"beat 1: show shortened to 'First, there's the cue'".to_string()),
        "{:?}",
        p.changed
    );
    assert_eq!(
        p.intent.beats[1].statement,
        "Costs fell while output tripled"
    );
    let mut strict = args(story(vec![
        json!({"say": sentence(10), "show": "Costs fell while output tripled in the end"}),
        json!({"say": sentence(10)}),
    ]));
    strict.strict = true;
    assert_eq!(
        fixes(Profile::Weak, &strict),
        vec!["beat 1 show: has 8 words, at most 6 — use \"Costs fell while output tripled\""]
    );
}

// ---------------------------------------------------------------------------
// Lenient parsing
// ---------------------------------------------------------------------------

#[test]
fn lenient_parsing_notes() {
    let p = ok(
        Profile::Weak,
        &args(story(vec![
            json!("Plain strings are read as what the narrator says out loud."),
            json!({"say": sentence(10), "number": 381, "colour": "red"}),
            json!({"narration": sentence(10), "list": "robot, microchip and clock"}),
            json!({"say": sentence(10), "compare": ["cost", "output"], "energy": "wobbly"}),
            json!({"say": sentence(10), "compare": {"a": "cost", "b": "output", "how": "versus"}}),
            json!({"show": "The narrator reads this"}),
            json!({"say": "  Control\u{0007} characters\n are   stripped here.  ", "picture": "Money Stack"}),
        ])),
    );
    let c = &p.changed;
    for want in [
        "beat 1: plain text read as 'say'",
        "beat 2: ignored unknown field 'colour'",
        "beat 2: number 381 read as text '381'",
        "beat 3: 'narration' read as 'say'",
        "beat 3: list written as text, split into 3 items",
        "beat 4: compare written as [a, b], read as {a, b}",
        "beat 4: energy 'wobbly' unknown (calm, building or impact), dropped",
        "beat 5: compare how 'versus' read as 'separate'",
        "beat 6: no 'say'; the narrator reads the 'show' text",
        "beat 7: picture 'Money Stack' → 'money_stack'",
    ] {
        assert!(c.iter().any(|l| l == want), "missing {want:?} in {c:#?}");
    }
    assert_eq!(p.intent.beats[1].primary.value(), Some("381"));
    assert_eq!(
        p.intent.beats[6].narration.as_deref(),
        Some("Control characters are stripped here.")
    );
    // Cosmetic notes come after structure fixes.
    let fix_at = c
        .iter()
        .position(|l| l.starts_with("beat 6: no 'say'"))
        .unwrap();
    let cosmetic_at = c
        .iter()
        .position(|l| l == "beat 1: plain text read as 'say'")
        .unwrap();
    assert!(fix_at < cosmetic_at);
}

#[test]
fn strings_are_capped() {
    let long = "word ".repeat(200);
    let p = policy::parse_story(&story(vec![
        json!({"say": long, "show": "x".repeat(900)}),
        json!({"say": sentence(9)}),
    ]))
    .unwrap();
    let policy::Story::Lite(s) = p.story else {
        panic!()
    };
    assert!(s.beats[0].say.chars().count() <= policy::MAX_STRING_CHARS);
    assert!(s.beats[0].show.as_ref().unwrap().chars().count() <= policy::MAX_STRING_CHARS);
}

#[test]
fn malformed_stories_get_fixes() {
    let f = fixes(Profile::Weak, &args(json!({"title": "no beats"})));
    assert_eq!(
        f,
        vec!["beats: missing — write 2 to 12 beats, each {\"say\": \"…\"}"]
    );
    let f = fixes(Profile::Weak, &args(json!(42)));
    assert_eq!(
        f,
        vec!["story: not a story object — write {\"title\": …, \"beats\": [{\"say\": …}, …]}"]
    );
    let f = fixes(
        Profile::Weak,
        &args(story(vec![
            json!({"say": sentence(8), "compare": ["only"]}),
            json!(7),
        ])),
    );
    assert_eq!(
        f,
        vec![
            "beat 1 compare: needs two sides — write compare {\"a\": …, \"b\": …}",
            "beat 2 beat: not a beat — write each beat as {\"say\": \"…\"}",
        ]
    );
}

// ---------------------------------------------------------------------------
// The lfm2.5 probe (coordinator): one weak call, no needs_fix
// ---------------------------------------------------------------------------

fn lfm_call() -> Value {
    json!({"title":"The Habit Loop: Cue, Routine & Reward","story":{"beats":[
        {"say":"The habit loop is a simple three-part cycle that explains how habits form and stick.","picture":"brain_network"},
        {"say":"First, there's the cue — a trigger that tells your brain it's time to act.","picture":"alarm_clock"},
        {"say":"Then comes the routine — the behavior you repeat automatically.","picture":"person_exercising"},
        {"say":"Finally, the reward — the positive feeling that reinforces the behavior and makes you want to repeat it.","picture":"smiley_face"},
        {"say":"When the cue leads to a routine that delivers a reward, the loop reinforces itself over time.","picture":"cycle_diagram"}],
        "mode":"render","style":"educational"},"assets":"assets:./images"})
}

#[test]
fn weak_model_call_shape_is_auto_fixed() {
    let config = common::config(Profile::Weak, "policy_lfm");
    let ctx = common::ctx(&config);
    let (args, res) = policy::prepare_raw(&lfm_call(), &ctx);
    let p = res.unwrap_or_else(|f| panic!("needs_fix: {f:#?}"));
    assert_eq!(args.mode, motion_mcp::args::Mode::Render);
    assert_eq!(p.intent.title, "the_habit_loop_cue_routine_reward");
    assert_eq!(p.intent.beats.len(), 5);
    let c = &p.changed;
    assert!(
        c.len() <= 5 || c[5..].iter().all(|l| !l.contains("→")),
        "{c:#?}"
    );
    for want in [
        "title moved into the story",
        "'mode', 'style' moved out of the story (tool arguments)",
        "style 'educational' → 'documentary'",
        "assets 'assets:./images' not found; using library pictures",
    ] {
        assert!(c.iter().any(|l| l == want), "missing {want:?} in {c:#?}");
    }
    assert_eq!(
        serde_json::to_value(&p.style).unwrap()["tone"],
        "documentary"
    );
    assert!(p.images.is_none());
    assert!(p.request.assets.is_none());
    // The same call through normalize_args + prepare_normalized agrees.
    let (a2, notes) = policy::normalize_args(&lfm_call());
    let p2 = policy::prepare_normalized(&a2, &notes, &ctx).unwrap();
    assert_eq!(job_id(&p.key), job_id(&p2.key));
    assert_eq!(p.changed, p2.changed);
    // Strict: the unknown style and the missing folder become fixes.
    let mut raw = lfm_call();
    raw["strict"] = json!(true);
    let (_, res) = policy::prepare_raw(&raw, &ctx);
    let f: Vec<String> = res.unwrap_err().iter().map(ToString::to_string).collect();
    assert!(
        f.iter()
            .any(|l| l.starts_with("style: unknown style 'educational' — use \"documentary\"")),
        "{f:#?}"
    );
    assert!(
        f.iter()
            .any(|l| l.starts_with("assets: 'assets:./images' not found")),
        "{f:#?}"
    );
}

#[test]
fn misplaced_arguments_and_unknown_words() {
    let (a, notes) = policy::normalize_args(&json!({
        "story": {"beats": two_beats(), "format": "square", "strict": "true", "tone": "fun"},
        "format": "wide",
        "bogus": 1,
        "mode": "preview",
    }));
    assert_eq!(a.format.as_deref(), Some("wide"));
    assert!(a.strict);
    assert_eq!(a.style, Some(json!("fun")));
    assert_eq!(a.mode, motion_mcp::args::Mode::Check);
    assert!(
        notes.contains(&"ignored unknown argument 'bogus'".to_string()),
        "{notes:?}"
    );
    assert!(
        notes.contains(&"'tone', 'strict' moved out of the story (tool arguments)".to_string()),
        "{notes:?}"
    );
    assert!(
        notes.contains(&"'format' inside the story ignored (given outside it)".to_string()),
        "{notes:?}"
    );
    // Beats given without a story object.
    let (a, notes) = policy::normalize_args(&json!({"title": "loose", "beats": two_beats()}));
    assert_eq!(a.story["title"], "loose");
    assert!(notes[0].contains("outside 'story'"));

    let mut m = args(story(two_beats()));
    m.style = Some(json!("fun"));
    m.format = Some("reel".into());
    let p = ok(Profile::Weak, &m);
    assert!(p.changed.contains(&"style 'fun' → 'playful'".to_string()));
    assert!(p
        .changed
        .contains(&"format 'reel' → 'vertical'".to_string()));
    m.style = Some(json!("glorpy"));
    let p = ok(Profile::Weak, &m);
    assert!(p
        .changed
        .contains(&"style 'glorpy' unknown, used auto".to_string()));
    m.strict = true;
    let f = fixes(Profile::Weak, &m);
    assert!(
        f[0].starts_with("style: unknown style 'glorpy' — use"),
        "{f:?}"
    );
}

#[test]
fn assets_outside_the_root_are_always_fixes() {
    for bad in ["../secrets", "/etc", "a/../../b"] {
        let mut m = args(story(two_beats()));
        m.assets = Some(bad.into());
        let f = fixes(Profile::Weak, &m);
        assert!(
            f.iter()
                .any(|l| l.starts_with("assets: '") && l.contains("use a path inside")),
            "{bad}: {f:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Pictures: auto-fix vs strict
// ---------------------------------------------------------------------------

/// A noun with no picture but a strong suggestion under these families.
fn strong_miss(families: &[String]) -> (String, String) {
    let pics = common::pictures();
    for noun in [
        "piggybank",
        "berries",
        "coins",
        "rockets",
        "clocks",
        "brains",
        "chess_kings",
        "books",
    ] {
        if pics.is_picture_in(noun, families) {
            continue;
        }
        if let Some((top, s)) = pics.suggest_scored(noun, 3, families).first() {
            if s.is_strong() {
                return (noun.to_string(), top.clone());
            }
        }
    }
    panic!("no strong miss among the candidates for {families:?}");
}

#[test]
fn pictures_auto_fix_or_strict_retry() {
    let base = ok(Profile::Weak, &args(story(two_beats())));
    let families = policy::families_for(&base.intent, &base.style, &base.options);
    let (noun, top) = strong_miss(&families);
    let beats = |pic: &str| {
        vec![
            json!({"say": sentence(10), "picture": pic}),
            json!({"say": sentence(10), "picture": "zorbtastic"}),
        ]
    };
    let p = ok(Profile::Weak, &args(story(beats(&noun))));
    assert_eq!(p.changed[0], format!("beat 1: picture '{noun}' → '{top}'"));
    assert!(p
        .changed
        .contains(&"beat 2: picture 'zorbtastic' shown as text".to_string()));
    assert_eq!(p.intent.beats[0].primary.asset(), Some(top.as_str()));
    assert!(
        p.plan[0].starts_with(&format!("1 picture {top}")),
        "{:?}",
        p.plan
    );
    assert!(p.plan[1].contains("zorbtastic (text)"));
    // Strict: a fix with options, and one retry with an option succeeds.
    let mut strict = args(story(beats(&noun)));
    strict.strict = true;
    let f = prepare(Profile::Weak, &strict).unwrap_err();
    let first = &f[0];
    assert_eq!(first.beat, Some(1));
    assert_eq!(first.field, "picture");
    assert_eq!(first.options[0], top);
    assert_eq!(
        first.otherwise.as_deref(),
        Some("leave it with strict off (shown as text)")
    );
    assert!(f[1]
        .to_string()
        .starts_with("beat 2 picture: no picture for 'zorbtastic'"));
    let mut retry = args(story(vec![
        json!({"say": sentence(10), "picture": first.options[0]}),
        json!({"say": sentence(10), "picture": "rocket"}),
    ]));
    retry.strict = true;
    retry.style = Some(json!("cinematic"));
    let p = prepare(Profile::Weak, &retry);
    // `rocket` may not be in the cinematic families; check the option only.
    match p {
        Ok(p) => assert!(p.changed.is_empty()),
        Err(f) => assert!(f.iter().all(|x| x.beat != Some(1)), "{f:?}"),
    }
    let mut retry = args(story(vec![
        json!({"say": sentence(10), "picture": first.options[0]}),
        json!({"say": sentence(10)}),
    ]));
    retry.strict = true;
    let p = ok(Profile::Weak, &retry);
    assert!(p.changed.is_empty(), "{:?}", p.changed);
}

#[test]
fn many_text_pictures_merge_into_one_line() {
    let p = ok(
        Profile::Weak,
        &args(story(vec![
            json!({"say": sentence(10), "picture": "zorbtastic"}),
            json!({"say": sentence(10), "picture": "glimmerfax"}),
            json!({"say": sentence(10), "picture": "quonkle"}),
        ])),
    );
    assert!(
        p.changed.contains(
            &"beats 1-3: pictures shown as text: zorbtastic, glimmerfax, quonkle".to_string()
        ),
        "{:?}",
        p.changed
    );
}

// ---------------------------------------------------------------------------
// Profile gating
// ---------------------------------------------------------------------------

fn gated_args() -> MakeVideoArgs {
    let mut a = args(story(two_beats()));
    a.style = Some(json!({"tone": "cinematic", "density": "dense"}));
    a.options = Some(ProductionOptions {
        art: Some("dossier".into()),
        tts_model: Some("gemini".into()),
        keep_frames: true,
        ..Default::default()
    });
    a.describe = true;
    a.force = true;
    a
}

#[test]
fn weak_profile_ignores_strong_fields() {
    let p = ok(Profile::Weak, &gated_args());
    for want in [
        "options ignored (not offered to this model)",
        "describe ignored (not offered to this model)",
        "force ignored (operator only)",
        "style: only the tone word is used",
    ] {
        assert!(
            p.changed.iter().any(|l| l == want),
            "missing {want:?}: {:?}",
            p.changed
        );
    }
    assert!(p.options.is_empty());
    let style = serde_json::to_value(&p.style).unwrap();
    assert_eq!(style["tone"], "cinematic");
    assert_eq!(style["density"], "auto", "only the tone word is used");
}

#[test]
fn creator_and_operator_gating() {
    let p = ok(Profile::Creator, &gated_args());
    assert!(p
        .changed
        .contains(&"force ignored (operator only)".to_string()));
    assert!(p
        .changed
        .contains(&"options.tts_model ignored (the free narrator is used)".to_string()));
    assert!(p
        .changed
        .contains(&"options.keep_frames ignored (operator only)".to_string()));
    assert_eq!(p.options.art.as_deref(), Some("dossier"));
    assert!(p.options.tts_model.is_none() && !p.options.keep_frames);
    assert_eq!(serde_json::to_value(&p.style).unwrap()["density"], "dense");

    let p = ok(Profile::Operator, &gated_args());
    assert!(p.changed.is_empty(), "{:?}", p.changed);
    assert_eq!(p.options.tts_model.as_deref(), Some("gemini"));
    assert!(p.options.keep_frames);

    let mut bad = gated_args();
    bad.options = Some(ProductionOptions {
        art: Some("vaporwave".into()),
        families: vec!["clay_props_3d".into(), "nope".into()],
        ..Default::default()
    });
    let p = ok(Profile::Creator, &bad);
    assert!(p
        .changed
        .contains(&"options.art 'vaporwave' unknown, used auto".to_string()));
    assert!(p
        .changed
        .contains(&"unknown asset families ignored: nope".to_string()));
    assert_eq!(p.options.families, vec!["clay_props_3d".to_string()]);
}

// ---------------------------------------------------------------------------
// Job ids
// ---------------------------------------------------------------------------

#[test]
fn job_ids_are_stable_and_change_with_the_story() {
    let a = args(story(two_beats()));
    let first = job_id(&ok(Profile::Weak, &a).key);
    assert_eq!(first, job_id(&ok(Profile::Weak, &a).key));
    let mut beats = two_beats();
    beats[1] = json!({"say": "Nobody on the team expected the change to come this quickly."});
    assert_ne!(first, job_id(&ok(Profile::Weak, &args(story(beats))).key));
    let mut styled = args(story(two_beats()));
    styled.style = Some(json!("hype"));
    assert_ne!(first, job_id(&ok(Profile::Weak, &styled).key));
    // Cosmetic differences that normalise to the same story share the id.
    let messy = args(story(vec![
        json!({"narration": "  Costs fell sharply while the output almost tripled this year. "}),
        json!("Nobody on the team expected the change to come this fast."),
    ]));
    assert_eq!(first, job_id(&ok(Profile::Weak, &messy).key));
}

/// (0.23 B1) A request without a take has the id it had before takes
/// existed: pinned for this story, profile, engine version (`test-engine`)
/// and voice (`auto`). Take 0 given explicitly is the same request; any other
/// take is another job.
#[test]
fn take_zero_job_ids_are_unchanged() {
    let a = args(story(two_beats()));
    let p = ok(Profile::Weak, &a);
    assert_eq!(p.key.engine, "test-engine");
    assert_eq!(job_id(&p.key).as_str(), "j_e13e87e6df");
    for zero in [json!(0), json!("0"), json!(null)] {
        let mut t = a.clone();
        t.take = Some(zero);
        assert_eq!(job_id(&ok(Profile::Weak, &t).key), job_id(&p.key));
    }
    let mut t = a.clone();
    t.take = Some(json!(1));
    let taken = ok(Profile::Weak, &t);
    assert_ne!(job_id(&taken.key), job_id(&p.key));
    assert_eq!(taken.request.take, 1);
}

/// (0.23 C4b) A request without a `music` word has the id it had before the
/// word existed: the same pinned id as the take-0 request above (so `auto`,
/// left out, `null`, empty or any case of "auto" is today's job), and every
/// other word is another job.
#[test]
fn music_auto_job_ids_are_unchanged() {
    let a = args(story(two_beats()));
    let p = ok(Profile::Weak, &a);
    assert_eq!(job_id(&p.key).as_str(), "j_e13e87e6df");
    for auto in [
        json!("auto"),
        json!("AUTO"),
        json!(" auto "),
        json!(null),
        json!(""),
    ] {
        let mut t = a.clone();
        t.music = Some(auto);
        assert_eq!(job_id(&ok(Profile::Weak, &t).key), job_id(&p.key));
    }
    let mut t = a.clone();
    t.music = Some(json!("calm"));
    let calm = ok(Profile::Weak, &t);
    assert_ne!(job_id(&calm.key), job_id(&p.key));
    assert_eq!(calm.request.music, motion_core::audio::MusicWord::Calm);
}

/// (0.23 C4b) `revise_video` keeps the job's music word unless one is given.
#[test]
fn revise_keeps_the_music_word_unless_a_new_one_is_given() {
    let mut a = args(story(two_beats()));
    a.music = Some(json!("dramatic"));
    let p = ok(Profile::Weak, &a);
    let revise_with = |music: Option<Value>| {
        revise(
            Profile::Weak,
            &p,
            ReviseVideoArgs {
                job: job_id(&p.key).to_string(),
                style: Some(json!("cinematic")),
                music,
                ..Default::default()
            },
        )
        .unwrap()
    };
    use motion_core::audio::MusicWord;
    assert_eq!(revise_with(None).request.music, MusicWord::Dramatic);
    assert_eq!(
        revise_with(Some(json!("Calm"))).request.music,
        MusicWord::Calm
    );
    assert_eq!(
        revise_with(Some(json!("no music"))).request.music,
        MusicWord::None
    );
    assert_eq!(
        revise_with(Some(json!("auto"))).request.music,
        MusicWord::Auto
    );
}

/// (0.23 B1) `revise_video` keeps the job's take unless one is given.
#[test]
fn revise_keeps_the_take_unless_a_new_one_is_given() {
    let mut a = args(story(two_beats()));
    a.take = Some(json!(2));
    let p = ok(Profile::Weak, &a);
    let revise_with = |take: Option<Value>| {
        revise(
            Profile::Weak,
            &p,
            ReviseVideoArgs {
                job: job_id(&p.key).to_string(),
                style: Some(json!("cinematic")),
                take,
                ..Default::default()
            },
        )
        .unwrap()
    };
    assert_eq!(revise_with(None).request.take, 2);
    assert_eq!(revise_with(Some(json!("5"))).request.take, 5);
    assert_eq!(revise_with(Some(json!(0))).request.take, 0);
}

// ---------------------------------------------------------------------------
// Full intents pass through
// ---------------------------------------------------------------------------

fn intent_json() -> Value {
    json!({
        "version": "0.2", "title": "full_intent", "format": "square",
        "beats": [
            {"purpose": "emphasize", "statement": "Cash is king", "narration": sentence(12),
             "primary": {"kind": "number", "value": "$381B", "meaning": "cash"}},
            {"purpose": "compare", "statement": "Then and now", "relationship": "grow", "narration": sentence(10),
             "primary": {"kind": "phrase", "value": "then"}, "secondary": {"kind": "object", "asset": "zorbtastic"}}
        ]
    })
}

#[test]
fn a_full_intent_passes_through() {
    let raw = intent_json();
    let p = ok(Profile::Creator, &args(raw.clone()));
    assert_eq!(p.request.kind, StoryKind::Intent);
    assert_eq!(
        serde_json::to_value(&p.intent).unwrap()["beats"],
        serde_json::to_value(motion_core::CreativeIntent::from_value(raw.clone()).unwrap())
            .unwrap()["beats"]
    );
    assert_eq!(p.request.format, "square");
    assert!(p
        .changed
        .contains(&"beat 2: picture 'zorbtastic' shown as text".to_string()));
    let mut wide = args(raw.clone());
    wide.format = Some("wide".into());
    assert_eq!(ok(Profile::Weak, &wide).request.format, "wide");
    // Errors name the beat.
    let mut broken = raw;
    broken["beats"][1]["purpose"] = json!("shout");
    let f = prepare(Profile::Creator, &args(broken)).unwrap_err();
    assert_eq!(f[0].beat, Some(2));
    let mut invalid = intent_json();
    invalid["beats"][0]["primary"] =
        json!({"kind": "layers", "layers": [{"name": "a"}, {"name": "b"}], "focus": "c"});
    let f = prepare(Profile::Creator, &args(invalid)).unwrap_err();
    assert_eq!(
        f[0].to_string(),
        "beat 1 layers: beat 1: primary.focus 'c' is not the name of any layer"
    );
}

// ---------------------------------------------------------------------------
// revise
// ---------------------------------------------------------------------------

fn revise(profile: Profile, p: &Prepared, r: ReviseVideoArgs) -> Result<Prepared, Vec<Fix>> {
    let config = common::config(profile, "policy_revise");
    policy::revise(&p.request, &r, &common::ctx(&config))
}

fn change(beat: Option<usize>, set: Value) -> BeatChange {
    BeatChange {
        beat,
        remove: false,
        insert_after: None,
        set: set.as_object().cloned().unwrap_or_default(),
    }
}

fn four_beats() -> MakeVideoArgs {
    args(story(vec![
        json!({"say": "Warren Buffett is sitting on 381 billion dollars in cash.", "number": "$381B", "show": "Cash pile"}),
        json!({"say": "He has been selling stocks for three straight years now.", "number": "36 months"}),
        json!({"say": "Costs fell sharply while the output almost tripled this year.", "compare": {"a": "cost", "b": "output"}}),
        json!({"say": "Nobody on the team expected the change to come this fast.", "keyword": "surprise"}),
    ]))
}

#[test]
fn revising_only_show_keeps_every_narration() {
    let p = ok(Profile::Weak, &four_beats());
    let r = revise(
        Profile::Weak,
        &p,
        ReviseVideoArgs {
            job: job_id(&p.key).to_string(),
            changes: vec![change(Some(2), json!({"show": "Three years of selling"}))],
            ..Default::default()
        },
    )
    .unwrap();
    let narr = |x: &Prepared| {
        x.intent
            .beats
            .iter()
            .map(|b| b.narration.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        narr(&p),
        narr(&r),
        "the voice is keyed by the narration text"
    );
    assert_eq!(r.intent.beats[1].statement, "Three years of selling");
    assert_ne!(job_id(&p.key), job_id(&r.key));
    assert_eq!(r.style, p.style);
    assert_eq!(r.request.format, p.request.format);
}

#[test]
fn revise_edits_removes_and_inserts_in_order() {
    let p = ok(Profile::Weak, &four_beats());
    let job = job_id(&p.key).to_string();
    let r = revise(
        Profile::Weak,
        &p,
        ReviseVideoArgs {
            job: job.clone(),
            changes: vec![
                BeatChange { beat: Some(1), remove: true, ..Default::default() },
                change(Some(2), json!({"number": null, "picture": "rocket", "colour": "red"})),
                BeatChange { insert_after: Some(3), ..change(None, json!({"say": "And then everything changed for the whole company at once."})) },
                BeatChange { insert_after: Some(0), ..change(None, json!({"say": "Here is a story about money and patience."})) },
            ],
            style: Some(json!("cinematic")),
            ..Default::default()
        },
    )
    .unwrap();
    let says: Vec<&str> = r
        .intent
        .beats
        .iter()
        .map(|b| b.narration.as_deref().unwrap_or(""))
        .collect();
    assert_eq!(
        says,
        vec![
            "Here is a story about money and patience.",
            "He has been selling stocks for three straight years now.",
            "Costs fell sharply while the output almost tripled this year.",
            "And then everything changed for the whole company at once.",
            "Nobody on the team expected the change to come this fast.",
        ]
    );
    // Beat 2 lost its number (null clears) and now shows a picture.
    assert_eq!(r.intent.beats[1].primary.kind_name(), "object");
    assert!(
        r.changed
            .contains(&"beat 2: change: ignored unknown field 'colour'".to_string()),
        "{:?}",
        r.changed
    );
    assert_eq!(serde_json::to_value(&r.style).unwrap()["tone"], "cinematic");

    // Out of range.
    let f = revise(
        Profile::Weak,
        &p,
        ReviseVideoArgs {
            job: job.clone(),
            changes: vec![change(Some(9), json!({"show": "x"}))],
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        f[0].to_string(),
        "changes: beat 9 is out of range — use 1 to 4"
    );
    let f = revise(
        Profile::Weak,
        &p,
        ReviseVideoArgs {
            job,
            changes: vec![BeatChange {
                insert_after: Some(5),
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        f[0].to_string(),
        "changes: insert_after 5 is out of range — use 0 to 4"
    );
}

#[test]
fn revise_patch_is_creator_only() {
    let p = ok(Profile::Weak, &four_beats());
    let patch = json!({"title": "patched title", "beats": null});
    let r = revise(
        Profile::Weak,
        &p,
        ReviseVideoArgs {
            job: job_id(&p.key).to_string(),
            patch: Some(json!({"title": "patched title"})),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(r
        .changed
        .contains(&"patch ignored (not offered to this model)".to_string()));
    assert_eq!(r.intent.title, p.intent.title);
    let r = revise(
        Profile::Creator,
        &p,
        ReviseVideoArgs {
            job: job_id(&p.key).to_string(),
            patch: Some(json!({"title": "patched title"})),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(r.intent.title, "patched_title");
    // A patch removing the beats is caught by the checks.
    assert!(revise(
        Profile::Creator,
        &p,
        ReviseVideoArgs {
            job: job_id(&p.key).to_string(),
            patch: Some(patch),
            ..Default::default()
        },
    )
    .is_err());
}

#[test]
fn revise_a_full_intent() {
    let p = ok(Profile::Creator, &args(intent_json()));
    let r = revise(
        Profile::Creator,
        &p,
        ReviseVideoArgs {
            job: job_id(&p.key).to_string(),
            changes: vec![
                change(Some(1), json!({"show": "Cash is still king", "energy": "impact"})),
                change(Some(2), json!({"say": "Now the numbers tell a very different story than before.", "list": ["a"]})),
                BeatChange { insert_after: Some(2), ..change(None, json!({"say": "So what happens next is anyone's guess, really.", "picture": "rocket"})) },
            ],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(r.intent.beats[0].statement, "Cash is still king");
    assert_eq!(r.intent.beats[0].narration, p.intent.beats[0].narration);
    assert_eq!(
        r.intent.beats[1].narration.as_deref(),
        Some("Now the numbers tell a very different story than before.")
    );
    assert_eq!(r.intent.beats.len(), 3);
    assert_eq!(r.intent.beats[2].primary.asset(), Some("rocket"));
    assert!(
        r.changed
            .iter()
            .any(|l| l.contains("'list' cannot be set on a full intent")),
        "{:?}",
        r.changed
    );
}

// ---------------------------------------------------------------------------
// (0.21) Brand colours

#[test]
fn brand_colours_every_shape_and_every_profile() {
    for brand in [
        json!({"primary": "#0a84ff", "background": "101828"}),
        json!(["0A84FF", "#FFD60A", "#101828"]),
        json!("#0A84FF, #FFD60A, #101828"),
        json!({"main": "#0af", "bg": "#101828"}),
    ] {
        for profile in [Profile::Weak, Profile::Creator] {
            let mut a = args(story(two_beats()));
            a.brand = Some(brand.clone());
            let p = ok(profile, &a);
            let b = p.style.brand.clone().expect("brand kept");
            assert!(
                matches!(b.primary.as_deref(), Some("#0A84FF") | Some("#00AAFF")),
                "{brand}: {b:?}"
            );
            assert_eq!(b.background.as_deref(), Some("#101828"), "{brand}");
            // The brand reaches the job id: a different brand is a different video.
            assert_ne!(
                job_id(&p.key),
                job_id(&ok(profile, &args(story(two_beats()))).key)
            );
        }
    }
    // A weak model may also tuck it inside `story`; it is moved out.
    let raw = json!({"story": {"beats": two_beats(), "brand": {"primary": "#FF375F"}}});
    let config = common::config(Profile::Weak, "policy");
    let (_, p) = policy::prepare_raw(&raw, &common::ctx(&config));
    assert_eq!(
        p.unwrap().style.brand.unwrap().primary.as_deref(),
        Some("#FF375F")
    );
}

#[test]
fn a_bad_brand_colour_is_noted_or_strictly_refused() {
    let mut a = args(story(two_beats()));
    a.brand = Some(json!({"primary": "blue-ish", "secondary": "#FFD60A"}));
    let p = ok(Profile::Weak, &a);
    let b = p.style.brand.clone().unwrap();
    assert_eq!((b.primary, b.secondary.as_deref()), (None, Some("#FFD60A")));
    assert!(
        p.changed.iter().any(|c| c.contains("not a hex colour")),
        "{:?}",
        p.changed
    );
    a.strict = true;
    let f = fixes(Profile::Weak, &a);
    assert!(
        f.iter().any(|f| f.starts_with("brand: primary 'blue-ish'")),
        "{f:?}"
    );
}

#[test]
fn revise_keeps_the_brand_unless_a_new_one_is_given() {
    let mut a = args(story(two_beats()));
    a.brand = Some(json!({"primary": "#0A84FF"}));
    let p = ok(Profile::Weak, &a);
    let kept = revise(
        Profile::Weak,
        &p,
        ReviseVideoArgs {
            job: "j_0000000000".into(),
            style: Some(json!("cinematic")),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(kept.style.brand, p.style.brand);
    let changed = revise(
        Profile::Weak,
        &p,
        ReviseVideoArgs {
            job: "j_0000000000".into(),
            brand: Some(json!({"primary": "#FF375F"})),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        changed.style.brand.unwrap().primary.as_deref(),
        Some("#FF375F")
    );
}
