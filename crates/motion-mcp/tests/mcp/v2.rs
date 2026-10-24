//! (v2 Phase 4) The `v2` profile: four tools, terse replies, and the KPI run
//! (brief → finished ~60 s video in ≤ 3 calls, ≤ 5k tokens of tool I/O).

use motion_mcp::profile::{budget_chars, tool_names, Profile};
use motion_mcp::schema::{defs_chars, tool_defs};
use motion_mcp::v2;
use serde_json::{json, Value};

use crate::common;

#[test]
fn v2_lists_four_tools_within_budget() {
    assert_eq!(tool_names(Profile::V2, true), v2::TOOLS.to_vec());
    let chars = defs_chars(&tool_defs(Profile::V2, true));
    assert!(chars <= budget_chars(Profile::V2), "{chars} chars");
}

#[test]
fn v2_refuses_bad_input_tersely() {
    let c = common::config(Profile::V2, "v2_refuse");
    let out = v2::call(&c, "make_video", json!({"brief": "x", "recipe": "nope"}));
    assert!(out.is_error && out.text.contains("kinetic_poster"));
    let out = v2::call(&c, "critique", json!({"path": "../etc/passwd"}));
    assert!(out.is_error && out.text.len() < 200);
}

#[test]
fn v2_exposes_guide_schema_and_patches_programs() {
    use motion_mcp::pictures::PictureIndex;
    use motion_mcp::resources::{self, ResourceCtx, URI_V2_GUIDE, URI_V2_SCHEMA};

    let c = common::config(Profile::V2, "v2_patch");
    let pictures = PictureIndex::load(&c.assets);
    let ctx = ResourceCtx {
        config: &c,
        pictures: &pictures,
    };
    let listed: Vec<String> = resources::list(Profile::V2)
        .into_iter()
        .map(|d| d.uri)
        .collect();
    assert!(listed.contains(&URI_V2_GUIDE.to_string()));
    assert!(listed.contains(&URI_V2_SCHEMA.to_string()));
    let guide = resources::read(URI_V2_GUIDE, &ctx).expect("v2 guide");
    assert!(guide.contains("top5_countdown") && guide.contains("monolith_split"));
    let schema = resources::read(URI_V2_SCHEMA, &ctx).expect("v2 schema");
    assert!(schema.contains("MotionProgram"));

    let mut prog = json!({
        "version": "2.0",
        "meta": {"name": "test_short", "aspect": "16:9", "duration": 12.0},
        "palette": {"background": "#000000", "ink": "#FFFFFF", "accent": "#FF0000", "muted": "#888888"},
        "components": [
            {"id": "hook", "kind": "title", "headline": "OLD HOOK", "start": 0.0, "duration": 4.0},
            {"id": "outro", "kind": "lockup", "preset": "spline_frame", "headline": "BRAND", "start": 8.0, "duration": 4.0}
        ]
    });
    v2::apply_program_edits(
        &mut prog,
        &json!({
            "aspect": "9:16",
            "palette": "#0B0F17,#F8FAFC,#10B981,#64748B",
            "fonts": "panchang_extrabold,instrument_serif_regular,space_mono_bold",
            "ending": "editorial_badge",
            "patch": {
                "components": [
                    {"id": "hook", "headline": "NEW HOOK"},
                    {"id": "outro", "subline": "Updated Subline"}
                ]
            }
        }),
    )
    .expect("apply edits");
    assert_eq!(prog["meta"]["aspect"], "9:16");
    assert_eq!(prog["palette"]["accent"], "#10B981");
    assert_eq!(prog["fonts"]["display"], "panchang_extrabold");
    assert_eq!(prog["components"][0]["headline"], "NEW HOOK");
    assert_eq!(prog["components"][1]["preset"], "editorial_badge");
    assert_eq!(prog["components"][1]["subline"], "Updated Subline");
    assert_eq!(prog["components"][1]["badge"], "EDITION // 2026");
}

/// ≈ 4 characters per token.
fn tokens(s: &str) -> usize {
    s.chars().count().div_ceil(4)
}

/// Needs a release engine (`MOTION_ENGINE`) and ffmpeg; renders 60 s.
/// `cargo test -p motion-mcp --test mcp v2::kpi -- --ignored --nocapture`
#[test]
#[ignore]
fn kpi_brief_to_60s_video_in_three_calls() {
    let c = common::config(Profile::V2, "v2_kpi");
    let brief = "Coffee prices, 60 s\nCoffee costs more every single year.\nArabica rose 38 52 61 74 90 cents a pound.\nFarmers see little of it.\nGrowers keep 7% of the retail price.\nRoasters and shops keep 85% of the price.\nShipping takes 4 6 5 8 weeks.\n\"Pay the grower\"\nFair trade adds 20 cents a pound.\nClimate cuts yields 12 18 25 30 percent by 2050.\nSmall farms grow 60% of the world's coffee.\nBuy direct from small roasters.\nDrink slower.";
    let mut io = 0;
    let mut call = |name: &str, args: Value| {
        let out = v2::call(&c, name, args.clone());
        let t_in = tokens(&args.to_string()) + tokens(name);
        let t_out = tokens(&out.text);
        eprintln!(
            "{name}: in {t_in} tok, out {t_out} tok, {} image(s)",
            out.images.len()
        );
        io += t_in + t_out;
        assert!(!out.is_error, "{name}: {}", out.text);
        out
    };
    let made = call("make_video", json!({"brief": brief}));
    let video = made.structured["video"]
        .as_str()
        .expect("video")
        .to_string();
    // 60 s asked: content without padding, snapped to the final downbeat.
    let secs = made.structured["seconds"].as_f64().unwrap();
    assert!((45.0..=62.0).contains(&secs), "{secs}");
    let top = made.structured["top"].to_string();
    assert!(!top.contains("gate:"), "{top}");
    let crit = call("critique", json!({"path": video}));
    let score = crit.structured["critique"].as_f64().unwrap();
    call("preview", json!({"path": video, "at": 30}));
    let defs = defs_chars(&tool_defs(Profile::V2, true)).div_ceil(4);
    eprintln!("KPI: 3 calls, tool I/O {io} tok (+ tools/list {defs} tok), critique {score}/40");
    assert!(io <= 5_000);
    eprintln!("make_video top: {top}");
    assert!(score >= 28.0);
    let _ = std::fs::remove_dir_all(common::repo().join("output/make/.preview"));
}
