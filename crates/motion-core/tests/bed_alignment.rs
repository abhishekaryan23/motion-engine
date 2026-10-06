//! (0.23 W5c) Bed alignment: with a voice-over the bed starts where its
//! downbeats meet the beat handoffs, and its end fade lands on a downbeat.
//! Speech still leads; only the bed's start moves.

use motion_core::audio::{
    align_bed, handoff_anchors, plan_audio, plan_audio_with_speech, AudioCue, AudioPlan, CueKind,
    MusicBed, MusicPlan, SfxFamily, SfxLibrary, SfxSound, AUDIO_PLAN_VERSION,
    BED_ALIGN_TOLERANCE_S, BED_START_MAX_S, MUSIC_PLAN_VERSION, SFX_LIBRARY_VERSION,
};
use motion_core::compiler::taste::resolve;
use motion_core::intent::Energy;
use motion_core::speech::{SpeechMap, SpeechWord, SPEECH_VERSION};
use motion_core::{MotionProject, StyleProfile};

/// Downbeats every `period` s from `phase`, covering `until`.
fn grid(phase: f64, period: f64, until: f64) -> Vec<f64> {
    let mut out = Vec::new();
    let mut k = 0.0;
    while phase + k * period <= until {
        out.push(((phase + k * period) * 1000.0).round() / 1000.0);
        k += 1.0;
    }
    out
}

/// Four handoffs on a 1.6 s downbeat grid (0, 1.6, 3.2 ...), each exactly on a
/// downbeat when the bed starts 1.23 s into the track: the offsets 1.17..=1.29
/// all put every handoff within 60 ms, so 1.23 is the centre of the plateau.
fn grid_handoffs() -> Vec<f64> {
    vec![1.97, 5.17, 8.37, 11.57]
}

#[test]
fn offset_1_23_aligning_four_of_four_handoffs_is_chosen() {
    let beats = grid(0.0, 1.6, 120.0);
    let a = align_bed(&grid_handoffs(), &beats, 120.0, 13.0);
    // The grid repeats every 1.6 s, so 2.83 (and 4.43 ...) score 4 of 4 too:
    // the earliest plateau wins.
    assert_eq!(a.start, 1.23);
    assert_eq!((a.aligned, a.handoffs), (4, 4));
    assert_eq!(a.fade_out_at, None);
}

#[test]
fn the_centre_of_the_plateau_is_chosen_not_its_edge() {
    let beats = grid(0.0, 1.6, 120.0);
    // The first handoff is 60 ms off at 1.23 and the others have room on one
    // side: the offsets 1.23..=1.26 score 4 of 4 and the centre is 1.24 (the
    // edge, 1.23, would leave the first handoff exactly 60 ms from its
    // downbeat).
    let a = align_bed(&[1.91, 5.17, 8.40, 11.55], &beats, 120.0, 13.0);
    assert_eq!((a.start, a.aligned), (1.24, 4));
    // One handoff 0.7 s before the downbeat at 3.2: 0.64..=0.76 reach it, the
    // centre is 0.70 (not the first reaching offset, 0.64).
    let a = align_bed(&[2.5], &beats, 120.0, 13.0);
    assert_eq!((a.start, a.aligned), (0.70, 1));
    // An even run of 12 steps (0.64..=0.75): the centre rounds down to the
    // 10 ms grid. A later plateau of the same score (at 4.8) never wins.
    let a = align_bed(&[2.505], &beats, 120.0, 13.0);
    assert_eq!((a.start, a.aligned), (0.69, 1));
    // Every aligned handoff of the chosen offset sits in the middle of its
    // window: at the centre of a symmetric plateau the distances are tiny.
    let a = align_bed(&grid_handoffs(), &beats, 120.0, 13.0);
    for h in grid_handoffs() {
        let nearest = beats
            .iter()
            .map(|b| (h + a.start - b).abs())
            .fold(f64::INFINITY, f64::min);
        assert!(
            nearest < 0.0101,
            "handoff {h} is {nearest} s from a downbeat"
        );
    }
}

#[test]
fn a_plateau_cut_short_by_the_range_is_centred_inside_the_range() {
    let beats = grid(0.0, 1.6, 120.0);
    // Handoffs already on the grid: the run of 4-of-4 offsets would be
    // -60..=+60 ms, the range starts at 0, so the run is 0..=60 ms and its
    // centre is 30 ms.
    let a = align_bed(&[1.6, 3.2, 6.4, 9.6], &beats, 120.0, 13.0);
    assert_eq!((a.start, a.aligned), (0.03, 4));
    // Handoffs 0.05 s after a downbeat reach it from offset 0 up to 10 ms.
    let a = align_bed(&[3.25, 6.45], &beats, 120.0, 13.0);
    assert_eq!((a.start, a.aligned), (0.0, 2));
}

#[test]
fn nothing_to_align_keeps_offset_zero() {
    let beats = grid(0.0, 1.6, 120.0);
    let a = align_bed(&[], &beats, 120.0, 13.0);
    assert_eq!((a.start, a.aligned, a.handoffs), (0.0, 0, 0));
    let a = align_bed(&grid_handoffs(), &[], 120.0, 13.0);
    assert_eq!((a.start, a.fade_out_at, a.aligned), (0.0, None, 0));
}

#[test]
fn a_track_too_short_for_the_offset_falls_back() {
    let beats = grid(0.0, 1.6, 120.0);
    // 13 s of video needs 14 s of track at offset 0: a 13.5 s track fits nothing.
    let a = align_bed(&grid_handoffs(), &beats, 13.5, 13.0);
    assert_eq!(a.start, 0.0);
    // Room for 1.0 s only (track 15 s): the search stops there, and the best
    // offset inside [0, 1.0] is used (never the 1.23 that would not fit).
    let a = align_bed(&grid_handoffs(), &beats, 15.0, 13.0);
    assert!(a.start <= 1.0 + 1e-9, "start {}", a.start);
    assert!(a.aligned < 4, "{} of 4 inside [0, 1]", a.aligned);
    // Room for 1.23 s: the plateau 1.17..=1.23 is cut by the range, its centre
    // is 1.20 and the bed still covers the video (1.20 + 13 <= 15.23 - 1).
    let a = align_bed(&grid_handoffs(), &beats, 15.23, 13.0);
    assert_eq!((a.start, a.aligned), (1.20, 4));
    let a = align_bed(&grid_handoffs(), &beats, 15.22, 13.0);
    assert_eq!((a.start, a.aligned), (1.19, 4));
    // Room for 1.16 s: no offset reaches all four.
    let a = align_bed(&grid_handoffs(), &beats, 15.16, 13.0);
    assert!(a.aligned < 4 && a.start <= 1.16 + 1e-9);
}

#[test]
fn the_search_never_passes_eight_seconds() {
    // A single handoff that only lines up at 9.9 s (a 12 s grid): out of range.
    let a = align_bed(&[2.1], &grid(0.0, 12.0, 400.0), 400.0, 10.0);
    assert_eq!((a.start, a.aligned), (0.0, 0));
}

#[test]
fn the_end_fade_lands_on_the_last_downbeat_of_the_final_second() {
    // Video 9 s from offset 1.23: the bed ends at track time 10.23; the
    // downbeats in [9.23, 10.23) are 9.6 -> the fade ends at 9.6 - 1.23.
    let a = align_bed(&[1.97, 5.17, 8.37], &grid(0.0, 1.6, 120.0), 120.0, 9.0);
    assert_eq!(a.start, 1.23);
    assert_eq!(a.fade_out_at, Some(8.37));
    // One handoff at 2.0 s reaches the downbeat at 3.2 from 1.14 to 1.26: the
    // start is 1.20 and a 10 s video ends at track time 11.2. Two downbeats
    // inside the last second [10.2, 11.2): the later one ends the fade.
    let a = align_bed(&[2.0], &[1.6, 3.2, 10.3, 10.9], 120.0, 10.0);
    assert_eq!(a.start, 1.20);
    assert_eq!(a.fade_out_at, Some(9.7));
    // No downbeat in the last second: as today.
    let a = align_bed(&[2.0], &[1.6, 3.2, 8.0], 120.0, 10.0);
    assert_eq!((a.start, a.fade_out_at), (1.20, None));
    // A downbeat exactly at the end is the end itself: nothing to move.
    let a = align_bed(&[2.0], &[1.6, 3.2, 11.2], 120.0, 10.0);
    assert_eq!(a.fade_out_at, None);
    // The window is closed at its early end.
    let a = align_bed(&[2.0], &[1.6, 3.2, 10.2], 120.0, 10.0);
    assert_eq!(a.fade_out_at, Some(9.0));
}

/// The search written the long way (a plain scan, no binary search): every
/// score first, then the earliest plateau at the top score and its centre.
fn reference(handoffs: &[f64], downbeats: &[f64], track: f64, video: f64) -> (f64, usize) {
    let ms = |t: f64| (t * 1000.0).round() as i64;
    let max = ms(BED_START_MAX_S).min(ms(track) - 1000 - ms(video)).max(0);
    let mut scores: Vec<usize> = Vec::new();
    let mut start = 0;
    while start <= max {
        let n = handoffs
            .iter()
            .filter(|&&h| {
                downbeats
                    .iter()
                    .any(|&d| (ms(h) + start - ms(d)).abs() <= ms(BED_ALIGN_TOLERANCE_S))
            })
            .count();
        scores.push(n);
        start += 10;
    }
    let top = scores.iter().copied().max().unwrap_or(0);
    if top == 0 {
        return (0.0, 0);
    }
    let first = scores.iter().position(|&s| s == top).expect("a top score");
    let mut last = first;
    while last + 1 < scores.len() && scores[last + 1] == top {
        last += 1;
    }
    (((first + last) / 2) as f64 * 0.010, top)
}

#[test]
fn the_search_matches_a_plain_scan_and_is_deterministic() {
    // A fixed linear congruential sequence: no clock, no RNG crate.
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as f64 / (1u64 << 31) as f64
    };
    for case in 0..60 {
        let period = 1.2 + 1.0 * next();
        let phase = period * next();
        let beats: Vec<f64> = grid(phase, period, 80.0)
            .into_iter()
            .map(|d| d + 0.05 * (next() - 0.5))
            .collect();
        let n = 2 + (next() * 6.0) as usize;
        let mut handoffs: Vec<f64> = (0..n).map(|_| 3.0 + 30.0 * next()).collect();
        handoffs.sort_by(|a, b| a.total_cmp(b));
        let video = handoffs[n - 1] + 4.0;
        let track = 60.0 + 20.0 * next();
        let a = align_bed(&handoffs, &beats, track, video);
        let (start, score) = reference(&handoffs, &beats, track, video);
        assert!(
            (a.start - start).abs() < 1e-9 && a.aligned == score,
            "case {case}: {} / {} vs {start} / {score}",
            a.start,
            a.aligned
        );
        assert_eq!(a, align_bed(&handoffs, &beats, track, video));
        assert!((0.0..=BED_START_MAX_S + 1e-9).contains(&a.start));
        assert!(a.start + video <= track - 1.0 + 1e-9 || a.start == 0.0);
    }
}

// ---------------------------------------------------------------------------
// The planner
// ---------------------------------------------------------------------------

fn project() -> MotionProject {
    // Four beat scenes whose overlaps put the handoff anchors at 3.25, 6.25
    // and 9.25 (each previous scene ends 0.5 s after the next one starts).
    let scene = |id: &str, start: f64, dur: f64| {
        format!(
            r#"{{"id":"{id}","start_seconds":{start},"duration_seconds":{dur},"layers":[],"motions":[],
               "lifecycle":{{"enter":0.2,"settle":0.3,"read":0.3,"evolve":1.0,"anticipate":2.5,"bridge":{dur}}}}}"#
        )
    };
    let json = format!(
        r##"{{"version":"0.2","project":{{"name":"t","duration_seconds":12.0}},
            "canvas":{{"width":108,"height":192,"fps":30,"background":"#FFFFFF"}},
            "scenes":[{},{},{},{}]}}"##,
        scene("a", 0.0, 3.5),
        scene("b", 3.0, 3.5),
        scene("c", 6.0, 3.5),
        scene("d", 9.0, 3.0),
    );
    MotionProject::from_json(&json).expect("fixture parses")
}

fn music(downbeats: Vec<f64>, duration: f64) -> MusicPlan {
    MusicPlan {
        version: MUSIC_PLAN_VERSION.to_string(),
        track: "bed.wav".into(),
        duration,
        bpm: 160.0,
        beat_times: vec![],
        downbeat_times: downbeats,
        sections: vec![],
        gain_db: -20.0,
        sha256: "0".repeat(64),
        lufs: None,
        lra: None,
    }
}

fn speech() -> SpeechMap {
    SpeechMap {
        version: SPEECH_VERSION.to_string(),
        audio: "v.wav".into(),
        sample_rate: 48_000,
        duration: 12.0,
        provider: "fixture".into(),
        model: "fixture/model".into(),
        voice: "fx".into(),
        words: vec![SpeechWord {
            text: "w".into(),
            start: 0.5,
            end: 0.9,
            confidence: 1.0,
        }],
        sentences: vec![],
        recognised: vec![],
        alignment: None,
    }
}

fn library() -> SfxLibrary {
    let mut sounds = Vec::new();
    for f in SfxFamily::ALL {
        sounds.push(SfxSound {
            id: format!("{}_a", f.as_str()),
            family: f,
            path: format!("sounds/{}/a.wav", f.as_str()),
            duration: 1.0,
            onset: 0.4,
            peak: 0.5,
            audible_end: 0.9,
            peak_db: -6.0,
            lufs: None,
            sha256: "0".repeat(64),
            tags: vec![],
        });
    }
    sounds.sort_by(|a, b| a.id.cmp(&b.id));
    SfxLibrary {
        version: SFX_LIBRARY_VERSION.to_string(),
        sounds,
    }
}

#[test]
fn the_handoff_anchors_are_the_overlap_midpoints() {
    // Not the scenes' raw starts (3.0, 6.0, 9.0): the midpoint of each overlap.
    let p = project();
    assert_eq!(handoff_anchors(&p), vec![3.25, 6.25, 9.25]);
    let beats = motion_core::audio::beat_scenes(&p);
    let starts: Vec<f64> = beats.iter().skip(1).map(|s| s.start_seconds).collect();
    assert_eq!(starts, vec![3.0, 6.0, 9.0]);
}

#[test]
fn a_planned_voice_over_starts_the_bed_where_the_handoffs_meet_the_downbeats() {
    // Downbeats every 1.5 s from 0.37: the anchors (3 s apart) share one
    // phase, 0.25 s after a multiple of 1.5, so the offsets 0.06..=0.18 line
    // up all three; the centre of that plateau is 0.12.
    let beats = grid(0.37, 1.5, 60.0);
    let m = music(beats.clone(), 60.0);
    let t = resolve(&StyleProfile::default());
    let lib = library();
    let p = project();
    let with = plan_audio_with_speech(&p, &t, &[], &lib, Some(&m), None, Some(&speech()));
    let bed = with.music.as_ref().expect("bed");
    assert_eq!(bed.start, 0.12);
    assert_eq!(
        bed.start,
        align_bed(&[3.25, 6.25, 9.25], &beats, 60.0, 12.0).start
    );
    // The end of the bed is at track time 12.12: no downbeat (10.87, 12.37)
    // in the last second, so the fade ends with the video.
    assert_eq!(bed.fade_out_at, None);
    // Nothing but the bed's `start` follows from the alignment: the cues are
    // those of the same plan without a bed, and the call is deterministic.
    let no_bed = plan_audio_with_speech(&p, &t, &[], &lib, None, None, Some(&speech()));
    assert_eq!(with.cues, no_bed.cues);
    assert_eq!(
        with,
        plan_audio_with_speech(&p, &t, &[], &lib, Some(&m), None, Some(&speech()))
    );
    let json = with.to_json_pretty();
    assert!(json.contains("\"start\": 0.12"), "{json}");
    // The plan round-trips.
    let back = AudioPlan::from_json(&json).expect("parses");
    assert_eq!(back, with);
}

#[test]
fn a_plan_without_speech_has_no_start_in_the_json() {
    let m = music(grid(0.37, 1.5, 60.0), 60.0);
    let t = resolve(&StyleProfile::default());
    let plan = plan_audio(&project(), &t, &[], &library(), Some(&m));
    let bed = plan.music.as_ref().expect("bed");
    assert_eq!((bed.start, bed.fade_out_at), (0.0, None));
    let json = plan.to_json_pretty();
    assert!(!json.contains("\"start\""), "{json}");
    assert!(!json.contains("fade_out_at"), "{json}");
    // The bed object is exactly the pre-0.23 one.
    let value: serde_json::Value = serde_json::from_str(&json).expect("json");
    let mut keys: Vec<&str> = value["music"]
        .as_object()
        .expect("music")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["duck", "fade_in", "fade_out", "gain_db", "track"]);
    // A bed with nothing to align to keeps offset 0, which stays absent from
    // the JSON even with a voice-over.
    let on_beat = music(vec![], 60.0);
    let plan = plan_audio_with_speech(
        &project(),
        &t,
        &[],
        &library(),
        Some(&on_beat),
        None,
        Some(&speech()),
    );
    let bed = plan.music.as_ref().expect("bed");
    assert_eq!(bed.start, 0.0);
    assert!(!plan.to_json_pretty().contains("\"start\""));
}

#[test]
fn an_old_plan_without_start_parses_to_zero() {
    let json = r#"{"track":"bed.wav","gain_db":-20.0,"fade_in":0.5,"fade_out":1.0,"duck":true}"#;
    let bed: MusicBed = serde_json::from_str(json).expect("old bed parses");
    assert_eq!((bed.start, bed.fade_out_at), (0.0, None));
    let plan = AudioPlan {
        version: AUDIO_PLAN_VERSION.to_string(),
        cues: vec![],
        loudness_target: -16.0,
        true_peak_limit: -1.0,
        min_spacing: 0.15,
        information_cap: 3,
        music: Some(MusicBed {
            start: 2.5,
            fade_out_at: Some(8.4),
            ..bed
        }),
        speech_adjustments: vec![],
        levels: None,
    };
    let text = plan.to_json_pretty();
    assert!(text.contains("\"start\": 2.5") && text.contains("\"fade_out_at\": 8.4"));
    assert_eq!(AudioPlan::from_json(&text).expect("parses"), plan);
}

/// The impact cue the planner snapped to a downbeat (reason ends in
/// "(on downbeat)"), if any.
fn snapped_impact(plan: &AudioPlan) -> Option<&AudioCue> {
    plan.cues
        .iter()
        .find(|c| c.kind == CueKind::Impact && c.reason.ends_with("(on downbeat)"))
}

fn on_a_downbeat(time_in_track: f64, downbeats: &[f64]) -> bool {
    downbeats.iter().any(|d| (d - time_in_track).abs() <= 0.001)
}

#[test]
fn an_impact_snaps_to_the_downbeats_of_the_bed_as_it_plays() {
    // Beat 1 (scene "b", start 3.0, enter 0.2, settle 0.3) is an impact: its
    // hit may move to a downbeat in [3.2, 3.3] of the PROJECT timeline.
    let energies = [Energy::Building, Energy::Impact, Energy::Building];
    let t = resolve(&StyleProfile::default());
    let lib = library();
    let p = project();

    // Track downbeats at 0.37 + 1.5 k: with the voice-over the bed starts at
    // 0.12 s (see the test above), so on the project timeline they are at
    // 0.25 + 1.5 k and 3.25 is inside the window; in track time 3.37 is not.
    let beats = grid(0.37, 1.5, 60.0);
    let m = music(beats.clone(), 60.0);
    let with = plan_audio_with_speech(&p, &t, &energies, &lib, Some(&m), None, Some(&speech()));
    let bed = with.music.as_ref().expect("bed");
    assert_eq!(bed.start, 0.12);
    let hit = snapped_impact(&with).expect("the impact snapped to a downbeat of the bed");
    assert_eq!(hit.time, 3.25);
    assert!(
        on_a_downbeat(hit.time + bed.start, &beats),
        "hit at {} + start {} is not on a track downbeat",
        hit.time,
        bed.start
    );
    // The other impact-side cues are unchanged by the shift: the subdrop that
    // goes with the first impact sits on the same time.
    assert!(with
        .cues
        .iter()
        .filter(|c| c.kind == CueKind::Impact)
        .all(|c| (c.time - 3.25).abs() < 1e-9));

    // Without a voice-over the bed starts at 0 and the snap uses the track's
    // own downbeats: 3.37 is outside [3.2, 3.3], so the hit stays at S + enter.
    let without = plan_audio(&p, &t, &energies, &lib, Some(&m));
    assert_eq!(without.music.as_ref().expect("bed").start, 0.0);
    assert!(snapped_impact(&without).is_none());
    let hit = without
        .cues
        .iter()
        .find(|c| c.kind == CueKind::Impact)
        .expect("an impact");
    assert_eq!(hit.time, 3.2);

    // A track whose own downbeat (3.25) is inside the window: the unshifted
    // grid snaps to it when there is no voice-over.
    let own = music(grid(0.25, 1.5, 60.0), 60.0);
    let without = plan_audio(&p, &t, &energies, &lib, Some(&own));
    let hit = snapped_impact(&without).expect("snapped on the track's own grid");
    assert_eq!(hit.time, 3.25);
    assert!(on_a_downbeat(hit.time, &own.downbeat_times));
    // With a voice-over the bed starts at 0.03 s (the plateau of offsets
    // 0..=0.06 is cut at 0): the hit follows the shifted grid, 3.22.
    let with = plan_audio_with_speech(&p, &t, &energies, &lib, Some(&own), None, Some(&speech()));
    let bed = with.music.as_ref().expect("bed");
    assert_eq!(bed.start, 0.03);
    let hit = snapped_impact(&with).expect("snapped on the shifted grid");
    assert_eq!(hit.time, 3.22);
    assert!(on_a_downbeat(hit.time + bed.start, &own.downbeat_times));
}
