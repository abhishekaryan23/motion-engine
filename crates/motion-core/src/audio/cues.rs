//! (0.24 M5) Cue taste: which moments get a sound, which family (from the look's
//! palette in `assets/sfx/palettes.json`), how loud (normalised by LUFS per
//! role) and how they are spread (variety, breath). Pure and deterministic.
//! See docs/SOUND_DESIGN.md §3.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

use super::{
    accent_word, arrival_times, information_cap, nearest_downbeat, round1, AudioCue, CueKind,
    SfxFamily, SfxLibrary, SfxSound, MAX_GAIN_DB, MIN_GAIN_DB,
};
use crate::compiler::taste::{DensityLevel, TemperamentKind, TransitionFamily};
use crate::intent::Energy;
use crate::reference::evidence::{round_ms, Fnv64};
use crate::scene::{MotionProject, Scene};
use crate::speech::SpeechMap;

/// A video with at least this many cues keeps every family at or below
/// [`FAMILY_SHARE`] of them.
pub const SHARE_MIN_CUES: usize = 5;
pub const FAMILY_SHARE: f64 = 0.4;
/// A sound id does not repeat within this many cues.
pub const VARIETY_WINDOW: usize = 4;
/// Seconds without any transient cue before a hero.
pub const BREATH: f64 = 0.25;
/// Largest deterministic loudness jitter (dB).
pub const JITTER_DB: f64 = 0.9;
/// Small cues per beat.
pub const SMALL_PER_BEAT: usize = 2;
/// A cue's sample peak (after gain) stays at or below this (dBFS).
pub const PEAK_CEILING_DB: f64 = -2.0;
/// Lead of the riser's peak before the hero it builds to (s).
pub const RISER_LEAD: f64 = 0.02;

const PALETTES_JSON: &str = include_str!("../../../../assets/sfx/palettes.json");

#[derive(Debug, Clone, Deserialize)]
pub struct RoleLufs {
    pub hero: f64,
    pub transition: f64,
    pub small: f64,
    /// (0.24 M7) Loudness of the fastest tick of a count-up train.
    #[serde(default = "default_count_lufs")]
    pub count: f64,
}

fn default_count_lufs() -> f64 {
    -28.0
}

/// (0.24 M7) Per-role loudness (LUFS-equiv) under a voice-over.
#[derive(Debug, Clone, Deserialize)]
pub struct SpeakingLufs {
    pub hero: f64,
    pub transition: f64,
    pub small: f64,
    pub count: f64,
}

/// (0.24 M7) The levels SFX take when a voice-over is present.
#[derive(Debug, Clone, Deserialize)]
pub struct VoiceLevels {
    pub speaking: SpeakingLufs,
    /// Added in gaps between phrases.
    pub gap_boost_db: f64,
    /// Added to a cue that lands on a word's nucleus.
    pub nucleus_db: f64,
    /// Added to a count tick while a word is spoken.
    pub count_speech_db: f64,
    /// Rate cap (ticks/s) of a train that runs under speech.
    pub count_max_per_s_speech: f64,
    /// A silence shorter than this (s) between words is not a gap.
    pub phrase_gap_s: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Palette {
    #[serde(default)]
    pub looks: Vec<String>,
    #[serde(default)]
    pub temperaments: Vec<String>,
    #[serde(default)]
    pub open: Vec<SfxFamily>,
    #[serde(default)]
    pub hero: Vec<SfxFamily>,
    #[serde(default)]
    pub hero_big_with: Vec<SfxFamily>,
    #[serde(default)]
    pub riser: Vec<SfxFamily>,
    #[serde(default)]
    pub transition: Vec<SfxFamily>,
    #[serde(default)]
    pub small: Vec<SfxFamily>,
    #[serde(default)]
    pub close: Vec<SfxFamily>,
    /// (0.24 M7) Family of the count-up tick train.
    #[serde(default)]
    pub count: Vec<SfxFamily>,
    /// (0.24 M7) Soft hit on a count's settle when the beat's hero is elsewhere.
    #[serde(default)]
    pub count_lock: Vec<SfxFamily>,
    /// (0.24 M7) Added to the train's loudness (dB).
    #[serde(default)]
    pub count_offset_db: f64,
    /// (0.24 M7) Rate cap of the train (ticks/s); `None` = [`COUNT_MAX_PER_S`].
    #[serde(default)]
    pub count_max_per_s: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PaletteFile {
    pub version: String,
    pub role_lufs: RoleLufs,
    pub default: String,
    /// (0.24 M7) Levels under a voice-over; absent = M5 targets always.
    #[serde(default)]
    pub voice: Option<VoiceLevels>,
    pub palettes: BTreeMap<String, Palette>,
}

/// The embedded palette table (`None` only if the shipped JSON is broken; a
/// test guards that).
pub fn palette_file() -> Option<&'static PaletteFile> {
    static FILE: OnceLock<Option<PaletteFile>> = OnceLock::new();
    FILE.get_or_init(|| serde_json::from_str(PALETTES_JSON).ok())
        .as_ref()
}

/// Palette name and table for a look (when art direction ran) else the
/// temperament; the file's default otherwise.
pub fn palette_for(
    look: Option<&str>,
    temperament: TemperamentKind,
) -> Option<(&'static str, &'static Palette)> {
    let file = palette_file()?;
    let temp = format!("{temperament:?}").to_lowercase();
    let by_look = look.and_then(|l| {
        file.palettes
            .iter()
            .find(|(_, p)| p.looks.iter().any(|x| x == l))
    });
    let found = by_look.or_else(|| {
        file.palettes
            .iter()
            .find(|(_, p)| p.temperaments.contains(&temp))
    });
    let found = found.or_else(|| file.palettes.get_key_value(&file.default));
    found.map(|(k, v)| (k.as_str(), v))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Hero,
    Transition,
    Small,
}

impl Role {
    fn name(self) -> &'static str {
        match self {
            Role::Hero => "hero",
            Role::Transition => "transition",
            Role::Small => "small",
        }
    }
}

struct Slot<'a> {
    scene: &'a Scene,
    kind: CueKind,
    role: Role,
    families: Vec<SfxFamily>,
    time: f64,
    bed: bool,
    ordinal: u64,
    what: String,
    suffix: &'static str,
}

impl Slot<'_> {
    fn is_hero(&self) -> bool {
        self.role == Role::Hero
    }
}

/// Inputs of the cue planner.
pub struct CueInputs<'a> {
    pub project: &'a MotionProject,
    pub beats: &'a [&'a Scene],
    pub beat_energy: &'a [Energy],
    pub temperament: TemperamentKind,
    pub density: DensityLevel,
    pub transition: TransitionFamily,
    pub library: &'a SfxLibrary,
    pub seed: u64,
    pub spacing: f64,
    pub duration: f64,
    pub speech: Option<&'a SpeechMap>,
    /// Downbeats of the bed as it plays (empty without music).
    pub grid: &'a [f64],
    pub has_music: bool,
}

fn rank(e: Energy) -> u8 {
    match e {
        Energy::Calm => 0,
        Energy::Building => 1,
        Energy::Impact => 2,
    }
}

fn is_backdrop_target(t: &str) -> bool {
    t.ends_with(".stage") || t.ends_with(".ghost")
}

/// Project seconds at which the beat's lead element has landed: the end of the
/// first motion on the hero layer (the focal layer of the art direction; else a
/// shared number, a `.hero`, a stamp, a zone, the headline), clamped into the
/// beat's ENTER..ANTICIPATE window. Scenes without such a layer fall back to the
/// lifecycle's settle.
pub fn hero_land(scene: &Scene, focal: Option<&str>) -> f64 {
    let s = scene.start_seconds;
    let (lo, hi) = match scene.lifecycle {
        Some(l) => (l.enter, l.anticipate - 0.001),
        None => (0.0, scene.duration_seconds),
    };
    let fallback = scene.lifecycle.map_or(0.0, |l| l.settle);
    let end_of = |pred: &dyn Fn(&str) -> bool| -> Option<f64> {
        scene
            .motions
            .iter()
            .filter(|m| {
                m.start.is_finite()
                    && m.duration.is_finite()
                    && m.start < hi
                    && !is_backdrop_target(&m.target)
                    && pred(&m.target)
            })
            .min_by(|a, b| {
                a.start
                    .total_cmp(&b.start)
                    .then(a.duration.total_cmp(&b.duration))
            })
            .map(|m| m.start + m.duration)
            .filter(|e| *e > lo)
    };
    let focal_pred = |t: &str| {
        focal.is_some_and(|f| t == f || (t.starts_with(f) && t[f.len()..].starts_with('.')))
    };
    let t = focal
        .and_then(|_| end_of(&focal_pred))
        .or_else(|| end_of(&|t| t.starts_with("shared.")))
        .or_else(|| end_of(&|t| t.contains(".hero")))
        .or_else(|| end_of(&|t| t.contains(".zone_")))
        .or_else(|| end_of(&|t| t.ends_with(".stamp")))
        .or_else(|| end_of(&|t| t.contains(".head.")))
        .unwrap_or(fallback);
    s + t.clamp(lo.min(hi), hi.max(lo))
}

/// Project seconds of the beat's first visible arrival.
fn first_arrival(scene: &Scene) -> f64 {
    let enter = scene.lifecycle.map_or(0.0, |l| l.enter);
    let first = scene
        .motions
        .iter()
        .filter(|m| !is_backdrop_target(&m.target) && m.start.is_finite())
        .map(|m| m.start)
        .min_by(|a, b| a.total_cmp(b));
    scene.start_seconds + first.unwrap_or(enter).max(0.0)
}

fn pick_hash(seed: u64, scene: &str, kind: CueKind, ordinal: u64, salt: u8) -> u64 {
    let mut h = Fnv64::default();
    h.write(&seed.to_le_bytes());
    h.write(scene.as_bytes());
    h.write(&[0xff]);
    h.write(kind.as_str().as_bytes());
    h.write(&[0xff]);
    h.write(&ordinal.to_le_bytes());
    if salt != 0 {
        h.write(&[0xee, salt]);
    }
    h.finish()
}

/// Plan the cues. Returns them sorted by (time, priority desc, sound id);
/// collisions, breath, family share and variety are already applied.
pub fn plan_cues(inp: &CueInputs) -> Vec<AudioCue> {
    let look = inp.project.project.art.as_ref().map(|a| a.look.as_str());
    let Some((pname, palette)) = palette_for(look, inp.temperament) else {
        return Vec::new();
    };
    let Some(file) = palette_file() else {
        return Vec::new();
    };
    let beats = inp.beats;
    let n_beats = beats.len();
    let speech_ctx = match (inp.speech, file.voice.as_ref()) {
        (Some(sp), Some(v)) => Some(SpeechCtx::new(sp, v)),
        _ => None,
    };
    // The video's biggest number: the highest-energy beat, the last on ties.
    let biggest = (0..n_beats).max_by_key(|&i| {
        (
            rank(inp.beat_energy.get(i).copied().unwrap_or(Energy::Building)),
            i,
        )
    });

    let mut slots: Vec<Slot> = Vec::new();
    let mut moving_cut = 0usize;
    for (i, scene) in beats.iter().copied().enumerate() {
        let s = scene.start_seconds;
        let energy = inp.beat_energy.get(i).copied().unwrap_or(Energy::Building);
        let focal = inp
            .project
            .project
            .art
            .as_ref()
            .and_then(|a| a.focal.get(&scene.id))
            .map(String::as_str);
        let mut ordinals = [0u64; 4];
        let mut ord = |k: CueKind| {
            let slot = &mut ordinals[match k {
                CueKind::Open => 0,
                CueKind::Handoff => 1,
                CueKind::Information => 2,
                CueKind::Impact => 3,
            }];
            let v = *slot;
            *slot += 1;
            v
        };
        let mut smalls = SMALL_PER_BEAT;

        if i == 0 && !palette.open.is_empty() {
            slots.push(Slot {
                scene,
                kind: CueKind::Open,
                role: Role::Transition,
                families: palette.open.clone(),
                time: first_arrival(scene),
                bed: false,
                ordinal: ord(CueKind::Open),
                what: "open".into(),
                suffix: "",
            });
            smalls -= 1;
        }

        // Transition: only cuts that move, at most one per two.
        if i >= 1 && !palette.transition.is_empty() {
            let prev_end = beats[i - 1].end_seconds();
            let overlap = (prev_end - s).max(0.0);
            let moves = overlap > 0.05 || inp.transition != TransitionFamily::Hard;
            if moves {
                if moving_cut.is_multiple_of(2) {
                    slots.push(Slot {
                        scene,
                        kind: CueKind::Handoff,
                        role: Role::Transition,
                        families: palette.transition.clone(),
                        time: s + overlap / 2.0,
                        bed: false,
                        ordinal: ord(CueKind::Handoff),
                        what: "cut".into(),
                        suffix: "",
                    });
                }
                moving_cut += 1;
            }
        }

        // Hero: the lead reveal, at its settle.
        let land = hero_land(scene, focal);
        let mut hero_t = land;
        let mut suffix = "";
        if energy == Energy::Impact && inp.has_music {
            if let Some(l) = scene.lifecycle {
                let lo = if i >= 1 {
                    let prev_end = beats[i - 1].end_seconds();
                    (s + (prev_end - s).max(0.0) / 2.0).min(s + l.enter)
                } else {
                    s + l.enter
                };
                if let Some(d) = nearest_downbeat(inp.grid, lo, s + l.settle, s + l.enter) {
                    hero_t = d;
                    suffix = " (on downbeat)";
                }
            }
        }
        let is_big = biggest == Some(i);
        if !palette.hero.is_empty() {
            slots.push(Slot {
                scene,
                kind: CueKind::Impact,
                role: Role::Hero,
                families: palette.hero.clone(),
                time: hero_t,
                bed: false,
                ordinal: ord(CueKind::Impact),
                what: if is_big {
                    "biggest reveal"
                } else {
                    "lead reveal"
                }
                .into(),
                suffix,
            });
        }
        let mut companions: Vec<(Vec<SfxFamily>, f64, &str)> = Vec::new();
        if is_big {
            if !palette.riser.is_empty() {
                companions.push((palette.riser.clone(), hero_t - RISER_LEAD, "riser"));
            }
            for f in &palette.hero_big_with {
                companions.push((vec![*f], hero_t, "hero bed"));
            }
        } else if i + 1 == n_beats && !palette.close.is_empty() {
            companions.push((palette.close.clone(), hero_t, "close"));
        }
        for (mut families, t, what) in companions {
            // A bed rides under the hero: only beds may share its instant.
            families.retain(|f| f.is_bed());
            if families.is_empty() {
                continue;
            }
            slots.push(Slot {
                scene,
                kind: CueKind::Impact,
                role: Role::Transition,
                families,
                time: t,
                bed: true,
                ordinal: ord(CueKind::Impact),
                what: what.into(),
                suffix: "",
            });
        }

        // Small: the punch word's accent, then secondary arrivals.
        if !palette.small.is_empty() {
            if let Some(sp) = inp.speech {
                if smalls > 0 {
                    if let Some((word, t)) = accent_word(sp, i, energy) {
                        slots.push(Slot {
                            scene,
                            kind: CueKind::Information,
                            role: Role::Small,
                            families: palette.small.clone(),
                            time: t - super::ACCENT_LEAD,
                            bed: false,
                            ordinal: ord(CueKind::Information),
                            what: format!("accent: '{word}'"),
                            suffix: "",
                        });
                        smalls -= 1;
                    }
                }
            }
            let silent = inp.temperament == TemperamentKind::Restrained
                && inp.density == DensityLevel::Sparse;
            if !silent {
                for a in arrival_times(scene)
                    .into_iter()
                    .take(information_cap(inp.density).min(smalls))
                {
                    slots.push(Slot {
                        scene,
                        kind: CueKind::Information,
                        role: Role::Small,
                        families: palette.small.clone(),
                        time: s + a,
                        bed: false,
                        ordinal: ord(CueKind::Information),
                        what: "arrival".into(),
                        suffix: "",
                    });
                }
            }
        }
    }

    // Only families the library has; drop times outside the project.
    for sl in &mut slots {
        sl.families.retain(|f| !inp.library.family(*f).is_empty());
    }
    slots.retain(|s| {
        !s.families.is_empty() && s.time.is_finite() && s.time >= 0.0 && s.time <= inp.duration
    });

    // Collisions: priority desc, time asc, hero first; beds are exempt.
    slots.sort_by(|a, b| {
        b.kind
            .priority()
            .cmp(&a.kind.priority())
            .then(b.is_hero().cmp(&a.is_hero()))
            .then(a.time.total_cmp(&b.time))
    });
    let mut accepted: Vec<Slot> = Vec::new();
    for sl in slots {
        if !sl.bed
            && accepted
                .iter()
                .any(|a| !a.bed && (a.time - sl.time).abs() < inp.spacing - 1e-9)
        {
            continue;
        }
        accepted.push(sl);
    }
    // Breath: no transient cue within BREATH before a hero.
    let hero_times: Vec<f64> = accepted
        .iter()
        .filter(|s| s.is_hero())
        .map(|s| s.time)
        .collect();
    accepted.retain(|s| {
        s.is_hero()
            || s.bed
            || !hero_times
                .iter()
                .any(|&h| s.time <= h && h - s.time < BREATH - 1e-9)
    });
    accepted.sort_by(|a, b| {
        a.time
            .total_cmp(&b.time)
            .then(b.kind.priority().cmp(&a.kind.priority()))
    });

    // Family per slot, in time order; the 40 % share caps later picks.
    let cap = if accepted.len() >= SHARE_MIN_CUES {
        (FAMILY_SHARE * accepted.len() as f64 + 1e-9).floor() as usize
    } else {
        usize::MAX
    };
    let mut counts: BTreeMap<SfxFamily, usize> = BTreeMap::new();
    let mut chosen: Vec<Option<SfxFamily>> = vec![None; accepted.len()];
    // Heroes first (they are never dropped), then the rest, each in time order.
    let mut order: Vec<usize> = (0..accepted.len()).collect();
    order.sort_by_key(|&i| (!accepted[i].is_hero(), i));
    for i in order {
        let sl = &accepted[i];
        let pick = sl
            .families
            .iter()
            .copied()
            .find(|f| counts.get(f).copied().unwrap_or(0) < cap)
            .or_else(|| sl.is_hero().then(|| sl.families[0]));
        if let Some(f) = pick {
            *counts.entry(f).or_default() += 1;
        }
        chosen[i] = pick;
    }
    let mut kept: Vec<(Slot, SfxFamily)> = accepted
        .into_iter()
        .zip(chosen)
        .filter_map(|(s, f)| f.map(|f| (s, f)))
        .collect();
    // Trim: no family above the share once there are enough cues.
    loop {
        let n = kept.len();
        if n < SHARE_MIN_CUES {
            break;
        }
        let mut tally: BTreeMap<SfxFamily, usize> = BTreeMap::new();
        for (_, f) in &kept {
            *tally.entry(*f).or_default() += 1;
        }
        let over = tally
            .iter()
            .find(|(_, c)| **c as f64 > FAMILY_SHARE * n as f64 + 1e-9)
            .map(|(f, _)| *f);
        let Some(over) = over else { break };
        let victim = kept
            .iter()
            .enumerate()
            .filter(|(_, (s, f))| *f == over && !s.is_hero())
            .max_by(|(_, (a, _)), (_, (b, _))| {
                (a.role == Role::Small)
                    .cmp(&(b.role == Role::Small))
                    .then(a.time.total_cmp(&b.time))
            })
            .map(|(i, _)| i);
        match victim {
            Some(i) => {
                kept.remove(i);
            }
            None => break,
        }
    }

    // Sounds: no id repeats within VARIETY_WINDOW cues; gain by role loudness.
    let mut out: Vec<AudioCue> = Vec::new();
    for (sl, family) in kept {
        let recent: Vec<&str> = out
            .iter()
            .rev()
            .take(VARIETY_WINDOW)
            .map(|c| c.sound_id.as_str())
            .collect();
        let target = match (&speech_ctx, file.voice.as_ref()) {
            (Some(ctx), Some(v)) => ctx.level(
                match sl.role {
                    Role::Hero => v.speaking.hero,
                    Role::Transition => v.speaking.transition,
                    Role::Small => v.speaking.small,
                },
                sl.time,
            ),
            _ => match sl.role {
                Role::Hero => file.role_lufs.hero,
                Role::Transition => file.role_lufs.transition,
                Role::Small => file.role_lufs.small,
            },
        };
        let lufs_of = |s: &SfxSound| s.lufs.unwrap_or(s.peak_db - 18.0);
        let all = inp.library.family(family);
        // Prefer sounds the role's loudness is reachable for without the peak
        // ceiling clipping the gain; a variety window beats that preference.
        let reachable: Vec<&SfxSound> = all
            .iter()
            .copied()
            .filter(|s| lufs_of(s) + (PEAK_CEILING_DB - s.peak_db) >= target - 0.1)
            .collect();
        let base = if reachable.is_empty() {
            all.clone()
        } else {
            reachable
        };
        let fresh: Vec<&SfxSound> = base
            .iter()
            .copied()
            .filter(|s| !recent.contains(&s.id.as_str()))
            .collect();
        let fresh = if fresh.is_empty() {
            all.iter()
                .copied()
                .filter(|s| !recent.contains(&s.id.as_str()))
                .collect()
        } else {
            fresh
        };
        let pool = if !fresh.is_empty() {
            fresh
        } else if sl.is_hero() {
            all
        } else {
            continue;
        };
        if pool.is_empty() {
            continue;
        }
        let h = pick_hash(inp.seed, &sl.scene.id, sl.kind, sl.ordinal, 0);
        let sound = pool[(h % pool.len() as u64) as usize];
        let jh = pick_hash(inp.seed, &sl.scene.id, sl.kind, sl.ordinal, 1);
        let mut jitter = ((jh % 1801) as f64 - 900.0) / 1000.0 * (JITTER_DB / 0.9);
        if speech_ctx.is_some() {
            // Under a voice-over the target is a ceiling: jitter only lowers.
            jitter = -jitter.abs();
        }
        let lufs = lufs_of(sound);
        let gain = (target - lufs + jitter)
            .min(PEAK_CEILING_DB - sound.peak_db)
            .clamp(MIN_GAIN_DB, MAX_GAIN_DB);
        out.push(AudioCue {
            time: round_ms(sl.time),
            scene: sl.scene.id.clone(),
            kind: sl.kind,
            family,
            sound_id: sound.id.clone(),
            gain_db: round1(gain),
            priority: sl.kind.priority(),
            reason: if sl.what.starts_with("accent") {
                format!(
                    "{} ({}, {pname}) → {}{}",
                    sl.what,
                    sl.role.name(),
                    family.as_str(),
                    sl.suffix
                )
            } else {
                format!(
                    "{}: {} {} ({pname}) → {}{}",
                    sl.kind.as_str(),
                    sl.role.name(),
                    sl.what,
                    family.as_str(),
                    sl.suffix
                )
            },
        });
    }
    let trains = count_trains(
        inp,
        palette,
        file,
        pname,
        speech_ctx.as_ref(),
        &hero_times,
        &out,
    );
    out.extend(trains);
    out
}

// ---------------------------------------------------------------------------
// (0.24 M7) SFX under a voice-over
// ---------------------------------------------------------------------------

/// Where words are: phrases (words merged across short silences) and each
/// word's nucleus (its middle half).
struct SpeechCtx<'a> {
    levels: &'a VoiceLevels,
    phrases: Vec<(f64, f64)>,
    nuclei: Vec<(f64, f64)>,
    onsets: Vec<f64>,
}

impl<'a> SpeechCtx<'a> {
    fn new(sp: &SpeechMap, levels: &'a VoiceLevels) -> Self {
        let mut words: Vec<(f64, f64)> = sp
            .words
            .iter()
            .filter(|w| w.start.is_finite() && w.end.is_finite() && w.end >= w.start)
            .map(|w| (w.start, w.end))
            .collect();
        words.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut phrases: Vec<(f64, f64)> = Vec::new();
        for &(a, b) in &words {
            match phrases.last_mut() {
                Some(last) if a - last.1 < levels.phrase_gap_s => last.1 = last.1.max(b),
                _ => phrases.push((a, b)),
            }
        }
        let nuclei = words
            .iter()
            .map(|&(a, b)| (a + 0.25 * (b - a), b - 0.25 * (b - a)))
            .collect();
        let onsets = words.iter().map(|w| w.0).collect();
        SpeechCtx {
            levels,
            phrases,
            nuclei,
            onsets,
        }
    }

    fn speaking(&self, t: f64) -> bool {
        self.phrases.iter().any(|&(a, b)| t >= a && t <= b)
    }

    fn on_nucleus(&self, t: f64) -> bool {
        self.nuclei.iter().any(|&(a, b)| t >= a && t <= b)
    }

    /// Whether any word is spoken inside `[a, b]`.
    fn overlaps(&self, a: f64, b: f64) -> bool {
        self.phrases.iter().any(|&(x, y)| x <= b && y >= a)
    }

    /// The loudness (LUFS-equiv) a cue at `t` takes given the speaking-table
    /// value of its role: gaps get the boost, a word's nucleus the cut.
    fn level(&self, speaking: f64, t: f64) -> f64 {
        if !self.speaking(t) {
            speaking + self.levels.gap_boost_db
        } else if self.on_nucleus(t) {
            speaking + self.levels.nucleus_db
        } else {
            speaking
        }
    }

    fn clear(&self, t: f64) -> bool {
        self.onsets
            .iter()
            .all(|o| (t - o).abs() >= super::SPEECH_CLEARANCE - 1e-9)
    }
}

/// (0.24 M7) Whether a cue is a tick of a count-up train. Trains are exempt
/// from the minimum spacing and the per-scene information cap, and the
/// speech planner never moves them (they are placed clear of word onsets).
pub fn is_count_tick(c: &AudioCue) -> bool {
    c.reason.starts_with("count tick")
}

/// Default ceiling of a train's rate (ticks/s).
pub const COUNT_MAX_PER_S: f64 = 18.0;
/// Rate a train aims for at the fastest point (ticks/s).
pub const COUNT_PEAK_PER_S: f64 = 16.0;
/// Nearest two ticks may be (s).
pub const COUNT_MIN_GAP: f64 = 0.07;
/// The last tick is this clear of the settle (s).
pub const COUNT_SETTLE_CLEAR: f64 = 0.25;
/// Counts shorter than this (s) get no train.
const COUNT_MIN_DURATION: f64 = 0.5;
/// Quietest a slow tick may sit under the fastest one (dB).
const COUNT_SLOW_DB: f64 = 6.0;
/// The last three ticks' extra attenuation (dB), earliest first.
const COUNT_DECRESCENDO: [f64; 3] = [2.0, 4.0, 6.0];

fn count_shape(m: &crate::scene::Motion, u: f64) -> f64 {
    // The timeline lands springs on OutQuint for counters.
    let e = if m.spring.is_some() {
        crate::easing::Easing::OutQuint
    } else {
        m.easing
    };
    e.apply(u)
}

/// Normalised time `u` in `[0, 1]` where the eased progress first reaches
/// `target` (bisection; deterministic).
fn count_cross(m: &crate::scene::Motion, target: f64) -> f64 {
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..48 {
        let mid = 0.5 * (lo + hi);
        if count_shape(m, mid) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// Tick trains (one per `Count` motion) plus the soft lock on a count's
/// settle when no hero lands there. See docs/SOUND_DESIGN.md §3.
fn count_trains(
    inp: &CueInputs,
    palette: &Palette,
    file: &PaletteFile,
    pname: &str,
    ctx: Option<&SpeechCtx>,
    hero_times: &[f64],
    placed: &[AudioCue],
) -> Vec<AudioCue> {
    let lib = inp.library;
    let Some(family) = palette
        .count
        .iter()
        .copied()
        .find(|f| !lib.family(*f).is_empty())
    else {
        return Vec::new();
    };
    let lock_family = palette
        .count_lock
        .iter()
        .copied()
        .find(|f| !lib.family(*f).is_empty());
    let mut out: Vec<AudioCue> = Vec::new();
    let others = |t: f64, out: &[AudioCue], within: f64| {
        placed
            .iter()
            .chain(out.iter().filter(|c| !is_count_tick(c)))
            .any(|c| !c.family.is_bed() && (c.time - t).abs() < within)
    };
    for scene in inp.beats.iter().copied() {
        let anticipate = scene.lifecycle.map_or(scene.end_seconds(), |l| {
            scene.start_seconds + l.anticipate - 0.001
        });
        for (mi, m) in scene.motions.iter().enumerate() {
            let crate::scene::MotionOp::Count { from, to, .. } = &m.op else {
                continue;
            };
            if !(m.start.is_finite() && m.duration.is_finite())
                || m.duration < COUNT_MIN_DURATION
                || (to - from).abs() < 1e-9
            {
                continue;
            }
            let t0 = scene.start_seconds + m.start;
            let settle = t0 + m.duration;
            let under_speech = ctx.is_some_and(|c| c.overlaps(t0, settle));
            let mut cap = palette.count_max_per_s.unwrap_or(COUNT_MAX_PER_S);
            if under_speech {
                if let Some(v) = file.voice.as_ref() {
                    cap = cap.min(v.count_max_per_s_speech);
                }
            }
            let gap_min = COUNT_MIN_GAP.max(1.0 / cap);
            // Fastest progress rate (fraction per normalised time).
            let steps = 400;
            let pmax = (0..steps)
                .map(|i| {
                    let (a, b) = (i as f64 / steps as f64, (i + 1) as f64 / steps as f64);
                    (count_shape(m, b) - count_shape(m, a)) * steps as f64
                })
                .fold(0.0_f64, f64::max);
            if pmax <= 1e-6 {
                continue;
            }
            let peak = COUNT_PEAK_PER_S.min(cap);
            let n = ((peak * m.duration / pmax).round() as usize).clamp(4, 80);
            let latest = (settle - COUNT_SETTLE_CLEAR).min(anticipate);
            // Candidate crossings 1/N .. (N-1)/N: the Nth is the settle.
            let mut kept: Vec<(f64, f64)> = Vec::new(); // (time, rate 0..1)
            let mut prev = f64::NEG_INFINITY;
            for k in 1..n {
                let u = count_cross(m, k as f64 / n as f64);
                let t = t0 + u * m.duration;
                let h = 1.0e-3;
                let rate = ((count_shape(m, (u + h).min(1.0)) - count_shape(m, (u - h).max(0.0)))
                    / (((u + h).min(1.0)) - ((u - h).max(0.0))).max(1e-9)
                    / pmax)
                    .clamp(0.0, 1.0);
                // Nudge off word onsets (the 80 ms rule), staying in order.
                let mut found: Option<f64> = None;
                for d in 0..=24i32 {
                    for sign in [1i32, -1] {
                        if d == 0 && sign == -1 {
                            continue;
                        }
                        let c = round_ms(t + f64::from(sign * d) * 0.005);
                        let fits = c >= t0
                            && c <= latest
                            && c >= 0.0
                            && c <= inp.duration
                            && c - prev >= gap_min - 1e-9
                            && ctx.is_none_or(|x| x.clear(c))
                            && !others(c, &out, 0.06)
                            && !hero_times.iter().any(|&h| c <= h && h - c < BREATH - 1e-9);
                        if fits {
                            found = Some(c);
                            break;
                        }
                    }
                    if found.is_some() {
                        break;
                    }
                }
                if let Some(c) = found {
                    prev = c;
                    kept.push((c, rate));
                }
            }
            if kept.len() < 3 {
                continue;
            }
            let all = lib.family(family);
            let h = pick_hash(inp.seed, &scene.id, CueKind::Impact, 0x1000 + mi as u64, 3);
            let total = kept.len();
            for (j, &(t, rate)) in kept.iter().enumerate() {
                // Round-robin over the family so one sample never machine-guns.
                let sound = all[((h % all.len() as u64) as usize + j) % all.len()];
                let speaking = ctx.is_some_and(|c| c.speaking(t));
                let mut level = match (ctx, file.voice.as_ref()) {
                    (Some(c), Some(v)) => {
                        let mut l = c.level(v.speaking.count, t);
                        if speaking {
                            l += v.count_speech_db;
                        }
                        l
                    }
                    _ => file.role_lufs.count,
                } + palette.count_offset_db;
                level -= (1.0 - rate) * COUNT_SLOW_DB;
                let from_end = total - 1 - j;
                if from_end < COUNT_DECRESCENDO.len() {
                    level -= COUNT_DECRESCENDO[COUNT_DECRESCENDO.len() - 1 - from_end];
                }
                let lufs = sound.lufs.unwrap_or(sound.peak_db - 18.0);
                let gain = (level - lufs)
                    .min(PEAK_CEILING_DB - sound.peak_db)
                    .clamp(MIN_GAIN_DB, MAX_GAIN_DB);
                out.push(AudioCue {
                    time: t,
                    scene: scene.id.clone(),
                    kind: CueKind::Impact,
                    family,
                    sound_id: sound.id.clone(),
                    gain_db: round1(gain),
                    priority: CueKind::Impact.priority(),
                    reason: format!(
                        "count tick {}/{total} ({pname}) → {}",
                        j + 1,
                        family.as_str()
                    ),
                });
            }
            // The settle: the beat's hero when it lands there, else a soft lock.
            let hero_here = hero_times.iter().any(|&t| (t - settle).abs() < 0.15);
            if let (false, Some(lf)) = (hero_here, lock_family) {
                if settle <= anticipate
                    && settle <= inp.duration
                    && !others(settle, &out, inp.spacing)
                {
                    let all = lib.family(lf);
                    let sound = all[(h % all.len() as u64) as usize];
                    let level = match (ctx, file.voice.as_ref()) {
                        (Some(c), Some(v)) => c.level(v.speaking.transition, settle),
                        _ => file.role_lufs.transition,
                    };
                    let lufs = sound.lufs.unwrap_or(sound.peak_db - 18.0);
                    let gain = (level - lufs)
                        .min(PEAK_CEILING_DB - sound.peak_db)
                        .clamp(MIN_GAIN_DB, MAX_GAIN_DB);
                    out.push(AudioCue {
                        time: round_ms(settle),
                        scene: scene.id.clone(),
                        kind: CueKind::Impact,
                        family: lf,
                        sound_id: sound.id.clone(),
                        gain_db: round1(gain),
                        priority: CueKind::Impact.priority(),
                        reason: format!("count lock ({pname}) → {}", lf.as_str()),
                    });
                }
            }
        }
    }
    out
}
