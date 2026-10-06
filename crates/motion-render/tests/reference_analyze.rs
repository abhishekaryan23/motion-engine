//! End-to-end reference analysis on synthetic lavfi fixtures (0.7).
//!
//! Each fixture is a few seconds of ffmpeg-generated video (never committed)
//! with one known property; the evidence must report it. Also covers the
//! bundle layout, the no-intent boundary and reuse.

use std::path::{Path, PathBuf};
use std::process::Command;

use motion_core::reference::evidence::{
    ChangeKind, EvidencePolarity, EvidenceTemperature, ReferenceEvidence,
};
use motion_render::reference::{analyze_reference, AnalysisConfig, Reuse};

fn temp_dir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("motion_ref_analyze_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Encode lavfi inputs through an optional filter graph into `out` (h264).
fn encode(out: &Path, inputs: &[String], filter: Option<&str>) {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-nostdin", "-y"]);
    for i in inputs {
        cmd.args(["-f", "lavfi", "-i", i]);
    }
    if let Some(f) = filter {
        cmd.args(["-filter_complex", f]);
    }
    cmd.args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-r", "30"]);
    cmd.arg(out);
    let status = cmd.status().expect("run ffmpeg");
    assert!(status.success(), "ffmpeg failed for {}", out.display());
}

fn solid(color: &str, seconds: f32) -> String {
    format!("color=c={color}:s=180x320:r=30:d={seconds}")
}

fn analyze(name: &str, inputs: &[String], filter: Option<&str>) -> ReferenceEvidence {
    let dir = temp_dir(name);
    let video = dir.join("ref.mp4");
    encode(&video, inputs, filter);
    let config = AnalysisConfig {
        sample_budget: Some(6),
        ..Default::default()
    };
    analyze_reference(&video, &dir.join("bundle"), &config, None)
        .expect("analysis")
        .evidence
}

#[test]
fn reference_static_light_and_dark() {
    let light = analyze("static_light", &[solid("0xF2F2F2", 3.0)], None);
    assert_eq!(light.color.polarity.value, EvidencePolarity::Light);
    assert!(light.color.polarity.confidence >= 0.85);
    assert!(light.temporal.changes.is_empty());
    assert!(light.temporal.static_fraction > 0.95);

    let dark = analyze("static_dark", &[solid("0x101418", 3.0)], None);
    assert_eq!(dark.color.polarity.value, EvidencePolarity::Dark);
    assert!(dark.color.polarity.confidence >= 0.85);
    assert!(dark.color.dark_frame_fraction > 0.95);
}

#[test]
fn reference_fast_cuts_vs_slow_editorial() {
    let colors = ["0xE03020", "0x2040E0", "0xF0F0F0", "0x202020"];
    let inputs: Vec<String> = (0..12).map(|i| solid(colors[i % 4], 0.4)).collect();
    let concat = format!(
        "{}concat=n=12:v=1:a=0",
        (0..12).map(|i| format!("[{i}:v]")).collect::<String>()
    );
    let cuts = analyze("fast_cuts", &inputs, Some(&concat));
    assert!(
        cuts.temporal.cuts_per_10s > 15.0,
        "{}",
        cuts.temporal.cuts_per_10s
    );
    assert!(cuts.temporal.hard_cut_fraction > 0.8);
    assert!(cuts.temporal.median_hold_seconds < 0.6);

    // Two held compositions with a 0.6 s dissolve every 4 s.
    let slow = analyze(
        "slow_editorial",
        &[
            format!(
                "{},drawbox=x=20:y=60:w=140:h=40:color=0x202020:t=fill",
                solid("0xEDE6D8", 4.0)
            ),
            format!(
                "{},drawbox=x=30:y=200:w=60:h=90:color=0xC03020:t=fill",
                solid("0xEDE6D8", 4.0)
            ),
            format!(
                "{},drawbox=x=90:y=20:w=70:h=260:color=0x203040:t=fill",
                solid("0xEDE6D8", 4.0)
            ),
        ],
        Some(
            "[0:v]null[a];[a][1:v]xfade=transition=fade:duration=0.6:offset=3.4[ab];\
             [ab][2:v]xfade=transition=fade:duration=0.6:offset=6.8",
        ),
    );
    assert!(slow.temporal.cuts_per_10s < 1.0);
    assert!(
        slow.temporal.changes_per_10s < cuts.temporal.changes_per_10s / 4.0,
        "slow {} fast {}",
        slow.temporal.changes_per_10s,
        cuts.temporal.changes_per_10s
    );
    assert!(slow
        .temporal
        .changes
        .iter()
        .any(|c| c.kind != ChangeKind::Cut));
}

#[test]
fn reference_saturation_and_temperature() {
    let high = analyze("high_saturation", &[solid("0xFF2020", 3.0)], None);
    let low = analyze("low_saturation", &[solid("0x8A8A8A", 3.0)], None);
    assert!(high.color.saturation.p50 > 0.7);
    assert!(low.color.saturation.p50 < 0.05);

    let warm = analyze("warm", &[solid("0xE07020", 3.0)], None);
    let cool = analyze("cool", &[solid("0x2050D0", 3.0)], None);
    assert_eq!(warm.color.temperature.value, EvidenceTemperature::Warm);
    assert_eq!(cool.color.temperature.value, EvidenceTemperature::Cool);
    assert_eq!(low.color.temperature.value, EvidenceTemperature::Neutral);
}

#[test]
fn reference_dense_vs_sparse() {
    let dense = analyze("dense", &["testsrc2=s=180x320:r=30:d=3".to_string()], None);
    let sparse = analyze(
        "sparse",
        &[
            "color=c=white:s=180x320:r=30:d=3,drawbox=x=80:y=150:w=20:h=20:color=black:t=fill"
                .to_string(),
        ],
        None,
    );
    assert!(dense.complexity.edge_density.p50 > sparse.complexity.edge_density.p50);
    assert!(dense.complexity.occupied_ratio.p50 > sparse.complexity.occupied_ratio.p50);
    assert!(sparse.complexity.flat_area_fraction.p50 > 0.8);
}

#[test]
fn bundle_layout_boundary_and_reuse() {
    let dir = temp_dir("bundle");
    let video = dir.join("ref.mp4");
    encode(
        &video,
        &[
            "testsrc2=s=180x320:r=30:d=3".to_string(),
            "sine=d=3".to_string(),
        ],
        None,
    );
    let out = dir.join("bundle");
    let config = AnalysisConfig::default();
    let first = analyze_reference(&video, &out, &config, None).expect("analysis");
    assert_eq!(first.reused, Reuse::Fresh);
    let ev = &first.evidence;
    assert!(ev.reference_fingerprint.starts_with("rf1-"));
    assert!(
        ev.metadata.audio_present,
        "audio is recorded, never analysed"
    );
    assert!(!ev.samples.is_empty() && ev.samples.len() <= 30);
    for (i, s) in ev.samples.iter().enumerate() {
        assert_eq!(s.id, format!("s{:02}", i + 1));
        assert!(out.join(&s.image).is_file());
    }
    for f in [
        "evidence.json",
        "contact-sheet.png",
        "interpreter-request.json",
        "interpreter-prompt.md",
    ] {
        assert!(out.join(f).is_file(), "{f}");
    }
    // The bundle is independent of any target story.
    let request = std::fs::read_to_string(out.join("interpreter-request.json")).unwrap();
    for word in ["CreativeIntent", "\"beats\"", "statement"] {
        assert!(!request.contains(word), "request mentions {word}");
    }

    // Same video, same config: the existing bundle is reused byte-for-byte.
    let evidence_bytes = std::fs::read(out.join("evidence.json")).unwrap();
    let second = analyze_reference(&video, &out, &config, None).expect("reuse");
    assert_eq!(second.reused, Reuse::OutDir);
    assert_eq!(second.evidence, first.evidence);
    assert_eq!(
        std::fs::read(out.join("evidence.json")).unwrap(),
        evidence_bytes
    );

    // A different configuration is a different analysis.
    let other = AnalysisConfig {
        sample_budget: Some(5),
        ..Default::default()
    };
    let third = analyze_reference(&video, &out, &other, None).expect("reanalysis");
    assert_eq!(third.reused, Reuse::Fresh);
    assert_ne!(
        third.evidence.reference_fingerprint,
        first.evidence.reference_fingerprint
    );

    // Deterministic: a fresh directory reproduces the same evidence.
    let again = analyze_reference(&video, &dir.join("again"), &config, None).expect("again");
    assert_eq!(again.evidence, first.evidence);
}
