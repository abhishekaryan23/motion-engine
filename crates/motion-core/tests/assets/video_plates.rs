//! (0.24 M6) Footage-plan plates: a video layer + scrim at the bottom of the
//! beat scene; nothing changes without a plan.

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::plates::WARN_PLATE_SKIPPED;
use motion_core::compiler::{
    compile_with_report, ApproxMeasure, AssetLibrary, CompileOptions, CompileWarning,
};
use motion_core::footage::FootagePlan;
use motion_core::intent::CreativeIntent;
use motion_core::scene::{AssetKind, LayerKind, MotionProject};
use motion_core::style::StyleProfile;

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn compile(
    footage: Option<&str>,
    canvas: Option<(u32, u32)>,
) -> (MotionProject, Vec<CompileWarning>) {
    let intent = CreativeIntent::from_json(
        &std::fs::read_to_string(repo().join("examples/editorial_demo.intent.json")).unwrap(),
    )
    .unwrap();
    let style: StyleProfile = serde_json::from_str(
        &std::fs::read_to_string(repo().join("examples/editorial_demo.style.json")).unwrap(),
    )
    .unwrap();
    let opts = CompileOptions {
        footage: footage.map(|f| FootagePlan::from_json(f).unwrap()),
        canvas,
        ..CompileOptions::default()
    };
    compile_with_report(
        &intent,
        &style,
        None,
        &AssetLibrary::new(repo().join("assets")),
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile")
}

const PLAN: &str = r#"{"version":"0.1","clips":[{"beat":2,"role":"plate","path":"/clips/a.mp4","frame_count":150,"fps":30.0,"in_s":1.0,"out_s":4.0}]}"#;

fn check(canvas: Option<(u32, u32)>) {
    let (base, _) = compile(None, canvas);
    let (with, warnings) = compile(Some(PLAN), canvas);
    assert!(warnings.iter().all(|w| w.code != WARN_PLATE_SKIPPED));
    // Scenes: [0] backdrop, [beat] (1-based). Only beat 2 changes.
    for (i, (a, b)) in base.scenes.iter().zip(&with.scenes).enumerate() {
        if i == 2 {
            continue;
        }
        assert_eq!(a, b, "scene {i} must not change");
    }
    let (a, b) = (&base.scenes[2], &with.scenes[2]);
    assert_eq!(b.layers.len(), a.layers.len() + 2);
    assert_eq!(&b.layers[2..], &a.layers[..]);
    let video = &b.layers[0];
    let LayerKind::Image {
        asset,
        treatment,
        playback,
        ..
    } = &video.kind
    else {
        panic!("first layer must be the video image");
    };
    let t = treatment.as_ref().expect("treatment");
    assert!((t.desaturate - 0.6).abs() < 1e-6);
    assert_eq!(t.tint.expect("tint").amount, 0.35);
    let pb = playback.as_ref().expect("playback");
    assert_eq!((pb.in_frame, pb.out_frame), (30, Some(119)));
    assert!(matches!(b.layers[1].kind, LayerKind::Rectangle { .. }));
    assert!(b.layers[1].opacity > 0.0 && b.layers[1].opacity < 1.0);
    assert!(video.z_index < b.layers[1].z_index);
    assert!(b.layers[2..]
        .iter()
        .all(|l| l.z_index > b.layers[1].z_index));
    let a = with.asset(asset).expect("video asset");
    assert_eq!(a.kind, AssetKind::Video);
    assert_eq!(a.sprite.as_ref().unwrap().frame_count, 150);
    assert_eq!(a.path, "/clips/a.mp4");
    // Full bleed.
    assert_eq!(
        (video.width as u32, video.height as u32),
        (with.canvas.width, with.canvas.height)
    );
    // Validates (no base dir: the file is not checked here).
    motion_core::validate::validate(&with, None).expect("valid");
}

#[test]
fn plate_beat_9x16() {
    check(None);
}

#[test]
fn plate_beat_16x9() {
    check(Some((1920, 1080)));
}

#[test]
fn unprobed_or_missing_beat_is_a_warning_not_a_plate() {
    let plan = r#"{"version":"0.1","clips":[
        {"beat":2,"role":"plate","path":"a.mp4"},
        {"beat":99,"role":"plate","path":"a.mp4","frame_count":30,"fps":30.0}]}"#;
    let (base, _) = compile(None, None);
    let (with, warnings) = compile(Some(plan), None);
    assert_eq!(base, with);
    assert_eq!(
        warnings
            .iter()
            .filter(|w| w.code == WARN_PLATE_SKIPPED)
            .count(),
        2
    );
}

#[test]
fn no_plan_is_identical() {
    assert_eq!(
        compile(None, None).0,
        compile(Some(r#"{"version":"0.1","clips":[]}"#), None).0
    );
}

#[test]
fn video_asset_extension_and_spec_are_validated() {
    let (mut p, _) = compile(Some(PLAN), None);
    let asset = p
        .assets
        .iter_mut()
        .find(|a| a.kind == AssetKind::Video)
        .unwrap();
    asset.path = "/clips/a.gif".into();
    let e = motion_core::validate::validate(&p, None)
        .unwrap_err()
        .to_string();
    assert!(e.contains("video extension not allowed"), "{e}");
    let asset = p
        .assets
        .iter_mut()
        .find(|a| a.kind == AssetKind::Video)
        .unwrap();
    asset.path = "/clips/a.mp4".into();
    asset.sprite = None;
    let e = motion_core::validate::validate(&p, None)
        .unwrap_err()
        .to_string();
    assert!(e.contains("needs a 'sprite' spec"), "{e}");
}
