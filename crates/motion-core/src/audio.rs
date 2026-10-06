//! Sound design layer (0.8): SFX library manifest, AudioPlan, MusicPlan and
//! the cue tables. See docs/SOUND_DESIGN.md.
//!
//! The AudioPlan is a separate artifact derived one-way FROM a compiled
//! MotionProject: the MotionScene schema and the Timeline never see audio.
//! Taste decides HOW a moment sounds (family, level); the scene lifecycle
//! decides WHEN (handoffs, ENTER of impact beats, EVOLVE arrivals). Weak models
//! never pick sounds, files, gains or times.
//!
//! Time is seconds on the project timeline (`t = frame / fps`), rounded to
//! milliseconds. A cue's `time` is where the sound's measured PEAK lands, never
//! where its file starts.

use serde::{Deserialize, Serialize};

use crate::compiler::taste::{
    CompositionRhythm, DensityLevel, ResolvedStyleProfile, TemperamentKind, TransitionFamily,
};
use crate::intent::Energy;
use crate::reference::evidence::{round_ms, Fnv64};
use crate::scene::{MotionProject, Scene};
use crate::speech::SpeechMap;

pub const SFX_LIBRARY_VERSION: &str = "0.1";
pub const SFX_CURATION_VERSION: &str = "0.1";
pub const AUDIO_PLAN_VERSION: &str = "0.1";
pub const MUSIC_PLAN_VERSION: &str = "0.1";
/// File name of the manifest inside a library root directory.
pub const LIBRARY_FILE: &str = "sfx-library.json";
/// Integrated loudness the mix aims for (LUFS).
pub const LOUDNESS_TARGET_LUFS: f64 = -16.0;
/// Limiter ceiling on the final mix (dBTP).
pub const TRUE_PEAK_LIMIT_DB: f64 = -1.0;
/// Motion starts closer than this belong to one information arrival
/// (same constant as the lifecycle QA).
pub const ARRIVAL_CLUSTER_SECONDS: f64 = 0.12;
/// Cue gains are clamped to this range (dB applied to the file).
pub const MAX_GAIN_DB: f64 = 18.0;
pub const MIN_GAIN_DB: f64 = -40.0;
/// (0.10) A cue's peak keeps at least this far from every spoken word onset.
pub const SPEECH_CLEARANCE: f64 = 0.080;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("audio json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported {what} version '{found}' (expected {expected})")]
    Version {
        what: &'static str,
        found: String,
        expected: &'static str,
    },
    #[error("invalid {0}")]
    Invalid(String),
}

// ---------------------------------------------------------------------------
// Families and tables (core-owned; never edited by choreography or music)
// ---------------------------------------------------------------------------

/// Closed set of sound families. A family is curation + measurement, never a
/// folder name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SfxFamily {
    WhooshSoft,
    WhooshHard,
    Swipe,
    HitSoft,
    HitHard,
    Subdrop,
    Riser,
    Click,
    Tick,
    Pop,
    Paper,
    Data,
    Notification,
    Glitch,
}

impl SfxFamily {
    pub const ALL: [SfxFamily; 14] = [
        SfxFamily::WhooshSoft,
        SfxFamily::WhooshHard,
        SfxFamily::Swipe,
        SfxFamily::HitSoft,
        SfxFamily::HitHard,
        SfxFamily::Subdrop,
        SfxFamily::Riser,
        SfxFamily::Click,
        SfxFamily::Tick,
        SfxFamily::Pop,
        SfxFamily::Paper,
        SfxFamily::Data,
        SfxFamily::Notification,
        SfxFamily::Glitch,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            SfxFamily::WhooshSoft => "whoosh_soft",
            SfxFamily::WhooshHard => "whoosh_hard",
            SfxFamily::Swipe => "swipe",
            SfxFamily::HitSoft => "hit_soft",
            SfxFamily::HitHard => "hit_hard",
            SfxFamily::Subdrop => "subdrop",
            SfxFamily::Riser => "riser",
            SfxFamily::Click => "click",
            SfxFamily::Tick => "tick",
            SfxFamily::Pop => "pop",
            SfxFamily::Paper => "paper",
            SfxFamily::Data => "data",
            SfxFamily::Notification => "notification",
            SfxFamily::Glitch => "glitch",
        }
    }

    /// Target sample peak (dBFS) a cue of this family is normalised to.
    pub fn peak_target_db(self) -> f64 {
        match self {
            SfxFamily::WhooshSoft | SfxFamily::WhooshHard => -14.0,
            SfxFamily::Swipe => -15.0,
            SfxFamily::HitHard => -8.0,
            SfxFamily::HitSoft => -11.0,
            SfxFamily::Subdrop => -12.0,
            SfxFamily::Riser => -14.0,
            SfxFamily::Click => -17.0,
            SfxFamily::Tick => -18.0,
            SfxFamily::Pop => -16.0,
            SfxFamily::Paper => -18.0,
            SfxFamily::Data => -20.0,
            SfxFamily::Notification => -18.0,
            SfxFamily::Glitch => -16.0,
        }
    }

    /// Beds (low sustained layers) sit under transients and are exempt from
    /// the minimum-spacing rule.
    pub fn is_bed(self) -> bool {
        matches!(self, SfxFamily::Subdrop | SfxFamily::Riser)
    }
}

/// Handoff (each scene start after the first): family and target peak dBFS.
pub fn handoff_cue(family: TransitionFamily) -> Option<(SfxFamily, f64)> {
    match family {
        TransitionFamily::Subtle => Some((SfxFamily::WhooshSoft, -24.0)),
        TransitionFamily::Editorial => Some((SfxFamily::WhooshSoft, -14.0)),
        TransitionFamily::Geometric => Some((SfxFamily::Swipe, -15.0)),
        TransitionFamily::Kinetic => Some((SfxFamily::WhooshHard, -14.0)),
        TransitionFamily::Hard => Some((SfxFamily::Click, -17.0)),
    }
}

/// Information arrival (EVOLVE arrivals): family and target peak dBFS by
/// temperament. Only short transients: a page-turn rustle (`paper`) is ~0.5 s
/// of noise that reads as an unrelated page change, so no taste uses it for
/// arrivals; restrained taste gets a quiet tick instead.
pub fn information_cue(kind: TemperamentKind, density: DensityLevel) -> Option<(SfxFamily, f64)> {
    match kind {
        TemperamentKind::Restrained if density == DensityLevel::Sparse => None,
        TemperamentKind::Restrained => Some((SfxFamily::Tick, -24.0)),
        TemperamentKind::Editorial => Some((SfxFamily::Click, -17.0)),
        TemperamentKind::Precise => Some((SfxFamily::Tick, -18.0)),
        TemperamentKind::Energetic => Some((SfxFamily::Pop, -16.0)),
    }
}

/// (0.10 Q) Accent cues peak this long before their word's onset.
pub const ACCENT_LEAD: f64 = 0.10;

/// The punch word of beat `beat` in `speech`: the first number-like word of
/// its sentence, else (impact beats) the sentence's last word of 3+ letters.
pub fn accent_word(speech: &SpeechMap, beat: usize, energy: Energy) -> Option<(String, f64)> {
    let sentence = speech.sentences.iter().find(|s| s.beat == beat)?;
    let words = speech.words_in(sentence);
    let numeric =
        |w: &str| w.chars().any(|c| c.is_ascii_digit()) || w.contains(['%', '₹', '$', '€', '£']);
    if let Some(w) = words.iter().find(|w| numeric(&w.text)) {
        return Some((w.text.clone(), w.start));
    }
    if energy == Energy::Impact {
        return words
            .iter()
            .rev()
            .find(|w| w.text.chars().filter(|c| c.is_alphabetic()).count() >= 3)
            .map(|w| (w.text.clone(), w.start));
    }
    None
}

/// Accent family and level: the art direction's SFX palette when the project
/// has one (`ProjectMeta.art.sfx`), else by temperament. Quieter than an
/// impact: an accent marks a word, it does not land a scene.
pub fn accent_cue(project: &MotionProject, temperament: TemperamentKind) -> (SfxFamily, f64) {
    use crate::compiler::art_direction::SfxPalette;
    let palette = project
        .project
        .art
        .as_ref()
        .and_then(|a| SfxPalette::parse(&a.sfx));
    match palette {
        Some(SfxPalette::Soft) => (SfxFamily::Tick, -20.0),
        Some(SfxPalette::Crisp) => (SfxFamily::Data, -18.0),
        Some(SfxPalette::Bouncy) => (SfxFamily::Pop, -15.0),
        Some(SfxPalette::Punchy) => (SfxFamily::HitSoft, -14.0),
        Some(SfxPalette::Cinematic) => (SfxFamily::HitSoft, -15.0),
        None => match temperament {
            TemperamentKind::Restrained => (SfxFamily::Tick, -21.0),
            TemperamentKind::Editorial | TemperamentKind::Precise => (SfxFamily::Click, -17.0),
            TemperamentKind::Energetic => (SfxFamily::Pop, -15.0),
        },
    }
}

/// Family part of [`information_cue`].
pub fn information_family(kind: TemperamentKind, density: DensityLevel) -> Option<SfxFamily> {
    information_cue(kind, density).map(|(f, _)| f)
}

/// Maximum information cues per beat.
pub fn information_cap(density: DensityLevel) -> usize {
    match density {
        DensityLevel::Sparse => 1,
        DensityLevel::Balanced => 2,
        DensityLevel::Dense => 3,
    }
}

/// Entrance of an impact-energy beat.
pub fn impact_family(kind: TemperamentKind, transition: TransitionFamily) -> SfxFamily {
    if kind == TemperamentKind::Energetic || transition == TransitionFamily::Kinetic {
        SfxFamily::HitHard
    } else {
        SfxFamily::HitSoft
    }
}

/// Piece open (first scene's ENTER): only energetic and precise tastes.
pub fn open_family(kind: TemperamentKind) -> Option<SfxFamily> {
    match kind {
        TemperamentKind::Energetic => Some(SfxFamily::Subdrop),
        TemperamentKind::Precise => Some(SfxFamily::Riser),
        _ => None,
    }
}

/// Minimum seconds between two accepted transient (non-bed) cues.
pub fn min_spacing(rhythm: CompositionRhythm) -> f64 {
    match rhythm {
        CompositionRhythm::SlowBreathing => 0.6,
        CompositionRhythm::MeasuredEditorial => 0.45,
        CompositionRhythm::Progressive => 0.35,
        CompositionRhythm::Active => 0.25,
        CompositionRhythm::HighFrequency => 0.15,
    }
}

/// (Phase 3) Half-width of the window in which a handoff may snap to a downbeat.
pub fn snap_tolerance(rhythm: CompositionRhythm) -> f64 {
    match rhythm {
        CompositionRhythm::SlowBreathing => 0.35,
        CompositionRhythm::MeasuredEditorial => 0.28,
        CompositionRhythm::Progressive => 0.22,
        CompositionRhythm::Active => 0.16,
        CompositionRhythm::HighFrequency => 0.12,
    }
}

// ---------------------------------------------------------------------------
// SFX library manifest (sfx-library.json)
// ---------------------------------------------------------------------------

/// One measured sound. Times are seconds from the file start (ms-rounded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SfxSound {
    pub id: String,
    pub family: SfxFamily,
    /// Relative to the library root (the directory holding sfx-library.json).
    pub path: String,
    pub duration: f64,
    /// First 10 ms RMS window within 30 dB of the loudest window.
    pub onset: f64,
    /// Centre of the loudest 10 ms RMS window: the cue anchor.
    pub peak: f64,
    /// End of the last window within 40 dB of the loudest window.
    pub audible_end: f64,
    /// Sample peak in dBFS.
    pub peak_db: f64,
    /// Integrated loudness when measurable (very short sounds gate out).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lufs: Option<f64>,
    /// Lowercase hex SHA-256 of the file bytes.
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// sfx-library.json. Sounds sorted by id; ids unique.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SfxLibrary {
    pub version: String,
    pub sounds: Vec<SfxSound>,
}

impl SfxLibrary {
    /// Parse and [`validate`](Self::validate).
    pub fn from_json(json: &str) -> Result<Self, AudioError> {
        let lib: SfxLibrary = serde_json::from_str(json)?;
        lib.validate()?;
        Ok(lib)
    }

    /// Version, unique ids, relative paths (no leading `/`, no `..`, no `\`),
    /// finite times with `0 <= onset <= peak <= audible_end <= duration`.
    pub fn validate(&self) -> Result<(), AudioError> {
        if self.version != SFX_LIBRARY_VERSION {
            return Err(AudioError::Version {
                what: "sfx library",
                found: self.version.clone(),
                expected: SFX_LIBRARY_VERSION,
            });
        }
        let mut ids = std::collections::BTreeSet::new();
        for s in &self.sounds {
            if !ids.insert(s.id.as_str()) {
                return Err(AudioError::Invalid(format!(
                    "duplicate sound id '{}'",
                    s.id
                )));
            }
            if !is_relative_path(&s.path) {
                return Err(AudioError::Invalid(format!(
                    "sound '{}': path '{}' must be relative to the library root",
                    s.id, s.path
                )));
            }
            let times = [s.onset, s.peak, s.audible_end, s.duration, s.peak_db];
            if times.iter().any(|t| !t.is_finite())
                || !(0.0 <= s.onset
                    && s.onset <= s.peak
                    && s.peak <= s.audible_end
                    && s.audible_end <= s.duration + 1e-6)
            {
                return Err(AudioError::Invalid(format!(
                    "sound '{}': times must satisfy 0 <= onset <= peak <= audible_end <= duration",
                    s.id
                )));
            }
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&SfxSound> {
        self.sounds.iter().find(|s| s.id == id)
    }

    /// Sounds of one family, sorted by id.
    pub fn family(&self, family: SfxFamily) -> Vec<&SfxSound> {
        let mut v: Vec<&SfxSound> = self.sounds.iter().filter(|s| s.family == family).collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).expect("library serializes")
    }
}

/// curation.json: which pack files enter the library and as which family.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SfxCuration {
    pub version: String,
    pub sounds: Vec<CurationEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurationEntry {
    pub id: String,
    pub family: SfxFamily,
    /// Relative to the pack root given on the command line.
    pub path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

fn is_relative_path(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && !p.contains('\\')
        && !p.contains(':')
        && p.split('/').all(|c| c != ".." && !c.is_empty())
}

// ---------------------------------------------------------------------------
// MusicPlan (Phase 3 input; frozen shape)
// ---------------------------------------------------------------------------

/// Offline analysis of one music track (`music-index`). Times in seconds from
/// the track start, ms-rounded, ascending.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MusicPlan {
    pub version: String,
    /// Relative to the music root (the directory holding the plan).
    pub track: String,
    pub duration: f64,
    pub bpm: f64,
    pub beat_times: Vec<f64>,
    /// Every 4th beat starting at the strongest phase.
    pub downbeat_times: Vec<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<f64>,
    /// Bed gain applied before ducking (dB).
    pub gain_db: f64,
    pub sha256: String,
    /// (0.23) Integrated loudness of the track (LUFS), measured by
    /// `music-index`; the voice-relative mix sets the bed from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lufs: Option<f64>,
    /// (0.23) Loudness range (LU), measured by `music-index`; beds above
    /// [`BED_LRA_COMPRESS_LU`] get a gentle compressor in the bed chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lra: Option<f64>,
}

// ---------------------------------------------------------------------------
// (0.23) Voice-relative mix levels
// ---------------------------------------------------------------------------

/// (0.23) Integrated loudness the voice-over is normalised to at mix time,
/// before makeup gain (LUFS). The voice cache keeps the raw audio.
pub const VOICE_LUFS: f64 = -18.0;
/// (0.23) True-peak ceiling of the normalised voice before the limiter (dBTP).
pub const VOICE_TRUE_PEAK_DB: f64 = -3.0;
/// (0.23) Beds with a loudness range above this (LU) are compressed gently.
pub const BED_LRA_COMPRESS_LU: f64 = 10.0;

/// (0.23) Where the music bed sits relative to the voice-over (dB, negative =
/// under the voice's integrated loudness), and how it moves between those
/// levels. The look table is [`MixLevels::for_style`]; the owner's listening
/// pass tunes the numbers here, never the code.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MixLevels {
    /// Bed level while the narrator speaks (and through gaps shorter than
    /// `long_gap_s`).
    pub under_speech_db: f64,
    /// Bed level in narration gaps of at least `long_gap_s`.
    pub long_gap_db: f64,
    /// Bed level before the first word and after the last.
    pub edge_db: f64,
    /// Ceiling of the SFX bus while the narrator speaks.
    pub sfx_under_speech_db: f64,
    /// A gap inside the narration at least this long (s) lets the bed rise.
    pub long_gap_s: f64,
    /// The bed is down this long (s) before a sentence's first word.
    pub ramp_down_s: f64,
    /// The bed starts rising this long (s) after a sentence's last word ends.
    pub ramp_up_s: f64,
}

impl MixLevels {
    /// The default table (plan §3 W5a): bed 18 dB under the voice while it
    /// speaks, 10 dB under in long gaps, 4 dB under before / after the
    /// narration, SFX at most 6 dB under.
    pub const STANDARD: MixLevels = MixLevels {
        under_speech_db: -18.0,
        long_gap_db: -10.0,
        edge_db: -4.0,
        sfx_under_speech_db: -6.0,
        long_gap_s: 0.8,
        ramp_down_s: 0.15,
        ramp_up_s: 0.5,
    };

    /// The look table: hype sits the bed 15 dB under the voice, documentary
    /// and calm (restrained) styles 20 dB, everything else [`Self::STANDARD`].
    pub fn for_style(taste: &ResolvedStyleProfile) -> MixLevels {
        use crate::compiler::taste::Genre;
        let under = match taste.genre {
            Genre::Hype => -15.0,
            Genre::Documentary => -20.0,
            _ if taste.motion.kind == TemperamentKind::Restrained => -20.0,
            _ => MixLevels::STANDARD.under_speech_db,
        };
        MixLevels {
            under_speech_db: under,
            ..MixLevels::STANDARD
        }
    }
}

impl Default for MixLevels {
    fn default() -> Self {
        MixLevels::STANDARD
    }
}

/// (0.23) The bed's gain automation, built from the SpeechMap word times (no
/// audio sidechain when a SpeechMap exists). Each point is (seconds on the
/// voice-over timeline, bed level relative to the voice's integrated loudness
/// in dB); linear between points, held before the first and after the last.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DuckEnvelope {
    pub points: Vec<(f64, f64)>,
}

impl DuckEnvelope {
    /// The envelope for a voice-over (pure, deterministic): `edge_db` before
    /// the first word, down to `under_speech_db` `ramp_down_s` before each
    /// sentence, held through gaps shorter than `long_gap_s`, up toward
    /// `long_gap_db` from `ramp_up_s` after a sentence that ends a long gap,
    /// and to `edge_db` after the last word.
    ///
    /// Spans are maximal runs of words whose gap to the next word is shorter
    /// than `long_gap_s`. The bed is at `edge_db` before the first span and
    /// ramps down to `under_speech_db`, reaching it at the span's first word
    /// start (`ramp_down_s` ahead); it holds through the span; after a span
    /// followed by a long gap it ramps up over `ramp_up_s` from the span's last
    /// word end to `long_gap_db`, then down again to reach `under_speech_db`
    /// at the next span's start; after the last span it ramps up over
    /// `ramp_up_s` to `edge_db` and holds. Points are on the voice timeline,
    /// ms-rounded, never below 0 s, with strictly increasing times. No words:
    /// `[(0.0, edge_db)]`.
    pub fn from_speech(speech: &SpeechMap, levels: &MixLevels) -> DuckEnvelope {
        // Word extents in start order (a map repaired by `speech::repair` is
        // already ordered; a hand-written one may not be).
        let mut words: Vec<(f64, f64)> = speech
            .words
            .iter()
            .filter(|w| w.start.is_finite() && w.end.is_finite())
            .map(|w| (w.start.max(0.0), w.end.max(w.start.max(0.0))))
            .collect();
        words.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));

        // Maximal runs separated by a gap of at least `long_gap_s` (compared
        // in whole milliseconds, the resolution of the word times).
        let long_gap = round_ms(levels.long_gap_s);
        let mut spans: Vec<(f64, f64)> = Vec::new();
        for &(start, end) in &words {
            match spans.last_mut() {
                Some(last) if round_ms(start - last.1) < long_gap => last.1 = last.1.max(end),
                _ => spans.push((start, end)),
            }
        }

        let mut points: Vec<(f64, f64)> = vec![(0.0, levels.edge_db)];
        // Appends a point, keeping the times strictly increasing (a later
        // point at the same millisecond, or before the last, replaces the
        // last point's level).
        let mut push = |t: f64, db: f64| {
            let t = round_ms(t.max(0.0));
            match points.last_mut() {
                Some(last) if t <= last.0 => last.1 = db,
                _ => points.push((t, db)),
            }
        };
        let mut prev_end: Option<f64> = None;
        for &(start, end) in &spans {
            match prev_end {
                None => {
                    push(start - levels.ramp_down_s, levels.edge_db);
                }
                Some(pe) => {
                    // The long gap before this span.
                    let up_end = pe + levels.ramp_up_s;
                    push(up_end.min(start), levels.long_gap_db);
                    push(
                        (start - levels.ramp_down_s).max(up_end.min(start)),
                        levels.long_gap_db,
                    );
                }
            }
            push(start, levels.under_speech_db);
            push(end, levels.under_speech_db);
            prev_end = Some(end);
        }
        if let Some(pe) = prev_end {
            push(pe + levels.ramp_up_s, levels.edge_db);
        }
        DuckEnvelope { points }
    }

    /// The level (dB relative to the voice) at `t` seconds on the voice
    /// timeline.
    pub fn level_at(&self, t: f64) -> f64 {
        let Some(&(t0, v0)) = self.points.first() else {
            return 0.0;
        };
        if t <= t0 {
            return v0;
        }
        for w in self.points.windows(2) {
            let ((ta, va), (tb, vb)) = (w[0], w[1]);
            if t <= tb {
                if tb - ta <= f64::EPSILON {
                    return vb;
                }
                return va + (vb - va) * (t - ta) / (tb - ta);
            }
        }
        self.points.last().map_or(v0, |p| p.1)
    }
}

// ---------------------------------------------------------------------------
// (0.23) Music catalog v0.2, story mood and bed selection
// ---------------------------------------------------------------------------

pub const MUSIC_CATALOG_VERSION: &str = "0.2";

/// Mood family of a bed and of a story's content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoodFamily {
    NeutralExplainer,
    Serious,
    Somber,
    Wonder,
    Upbeat,
    Playful,
    Dramatic,
    Tense,
    Calm,
    Inspiring,
}

impl MoodFamily {
    pub const ALL: [MoodFamily; 10] = [
        MoodFamily::NeutralExplainer,
        MoodFamily::Serious,
        MoodFamily::Somber,
        MoodFamily::Wonder,
        MoodFamily::Upbeat,
        MoodFamily::Playful,
        MoodFamily::Dramatic,
        MoodFamily::Tense,
        MoodFamily::Calm,
        MoodFamily::Inspiring,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            MoodFamily::NeutralExplainer => "neutral_explainer",
            MoodFamily::Serious => "serious",
            MoodFamily::Somber => "somber",
            MoodFamily::Wonder => "wonder",
            MoodFamily::Upbeat => "upbeat",
            MoodFamily::Playful => "playful",
            MoodFamily::Dramatic => "dramatic",
            MoodFamily::Tense => "tense",
            MoodFamily::Calm => "calm",
            MoodFamily::Inspiring => "inspiring",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TempoBand {
    Slow,
    Medium,
    Fast,
}

/// One bed of `assets/music/catalog.json` (v0.1 entries parse with the v0.2
/// fields empty).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogBed {
    pub id: String,
    pub file: String,
    /// MusicPlan file, relative to the catalog.
    pub plan: String,
    /// Typography emotions (v0.1 selection; kept for the creator tools).
    #[serde(default)]
    pub emotions: Vec<String>,
    /// (v0.2) Mood families this bed fits, best first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub moods: Vec<MoodFamily>,
    /// (v0.2) Energy 1 (still) ..= 5 (driving); 0 = untagged.
    #[serde(default)]
    pub energy: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tempo_band: Option<TempoBand>,
    pub bpm: f64,
    pub duration: f64,
    /// (v0.2) Measured loudness range (LU).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lra: Option<f64>,
    /// (v0.2) Measured 300–4000 Hz level relative to the bed's full-band
    /// level (dB): how much the bed sits in the speech band.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speech_band_db: Option<f64>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub source: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MusicCatalog {
    pub version: String,
    pub beds: Vec<CatalogBed>,
}

impl MusicCatalog {
    /// Parse a catalog (v0.1 or v0.2).
    pub fn from_json(json: &str) -> Result<Self, AudioError> {
        let cat: MusicCatalog = serde_json::from_str(json)?;
        if cat.version != "0.1" && cat.version != MUSIC_CATALOG_VERSION {
            return Err(AudioError::Version {
                what: "music catalog",
                found: cat.version,
                expected: MUSIC_CATALOG_VERSION,
            });
        }
        Ok(cat)
    }
}

/// The optional `music` word a weak model may give (MCP, `reel --music-mood`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MusicWord {
    /// Read the mood from the story (default).
    #[default]
    Auto,
    /// No music bed.
    None,
    Calm,
    Upbeat,
    Serious,
    Dramatic,
    Playful,
    Neutral,
}

impl MusicWord {
    pub const ALL: [MusicWord; 8] = [
        MusicWord::Auto,
        MusicWord::None,
        MusicWord::Calm,
        MusicWord::Upbeat,
        MusicWord::Serious,
        MusicWord::Dramatic,
        MusicWord::Playful,
        MusicWord::Neutral,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            MusicWord::Auto => "auto",
            MusicWord::None => "none",
            MusicWord::Calm => "calm",
            MusicWord::Upbeat => "upbeat",
            MusicWord::Serious => "serious",
            MusicWord::Dramatic => "dramatic",
            MusicWord::Playful => "playful",
            MusicWord::Neutral => "neutral",
        }
    }

    /// Exact word (trimmed, case-insensitive); lenient synonyms are the MCP
    /// policy's job.
    pub fn parse(s: &str) -> Option<MusicWord> {
        let s = s.trim().to_ascii_lowercase();
        MusicWord::ALL.into_iter().find(|w| w.as_str() == s)
    }
}

/// The mood read from a story's content (`story_mood`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoodProfile {
    /// The content's mood family.
    pub family: MoodFamily,
    /// Families a bed may have and still fit (includes `family`), best first.
    pub compatible: Vec<MoodFamily>,
    /// Energy 1..=5 from how the beats' energy is spread.
    pub energy: u8,
    /// Why, in a few words ("money story", "space and science").
    pub reason: String,
}

/// (0.23) The mood of a story, read deterministically from its content
/// (narration and statement lexicon, structures, energy spread), with the
/// tone as a secondary bias. Filled in by sprint 0.23 task C4-mood; until
/// then neutral.
pub fn story_mood(
    intent: &crate::intent::CreativeIntent,
    taste: &ResolvedStyleProfile,
) -> MoodProfile {
    crate::music_mood::read_mood(intent, taste)
}

/// The bed chosen for a story.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BedChoice {
    /// Catalog bed id; `None` = no bed (silence beats the wrong music).
    pub bed: Option<String>,
    /// The mood the choice was made for.
    pub mood: MoodFamily,
    /// For the reply: "tech_pulse (neutral, money story)".
    pub reason: String,
    /// `WARN_MUSIC_FIT` message when a forced `music` word conflicts with the
    /// content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// (0.23) Pick the bed: it must share a mood family with the content (the
/// tone only chooses among compatible beds), the `music` word may force a
/// mood (with a `music_fit` warning on conflict), the seed rotates among
/// compatible beds (`direction::choice(seed, 0, Dim::Bed)`), and no
/// compatible bed means no bed. Filled in by sprint 0.23 task C4-mood; until
/// then no bed.
pub fn select_bed(
    catalog: &MusicCatalog,
    mood: &MoodProfile,
    word: MusicWord,
    seed: u64,
) -> BedChoice {
    crate::music_mood::choose_bed(catalog, mood, word, seed)
}

// ---------------------------------------------------------------------------
// AudioPlan (<name>.audio.json)
// ---------------------------------------------------------------------------

/// Why a cue exists. Priority: impact = open (3) > handoff (2) > information (1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CueKind {
    Open,
    Handoff,
    Information,
    Impact,
}

impl CueKind {
    pub fn priority(self) -> u8 {
        match self {
            CueKind::Open | CueKind::Impact => 3,
            CueKind::Handoff => 2,
            CueKind::Information => 1,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            CueKind::Open => "open",
            CueKind::Handoff => "handoff",
            CueKind::Information => "information",
            CueKind::Impact => "impact",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioCue {
    /// Project seconds where the sound's PEAK lands (ms-rounded).
    pub time: f64,
    /// Scene id the cue belongs to.
    pub scene: String,
    pub kind: CueKind,
    pub family: SfxFamily,
    pub sound_id: String,
    /// Gain applied to the file (dB, 0.1-rounded) so its peak hits the family target.
    pub gain_db: f64,
    pub priority: u8,
    pub reason: String,
}

/// Music bed in the mix (present only when a MusicPlan was given).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MusicBed {
    /// Relative to the music root.
    pub track: String,
    pub gain_db: f64,
    pub fade_in: f64,
    pub fade_out: f64,
    /// Sidechain-duck the bed under the SFX bus.
    pub duck: bool,
    /// (0.23 W5c) Seconds into the track where the bed starts (the mix trims
    /// the track at this offset); chosen by [`align_bed`] when a voice-over is
    /// planned so the beat handoffs land on downbeats. Absent (= 0) in plans
    /// without a voice-over.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub start: f64,
    /// (0.23 W5c) Project seconds at which the end fade finishes (the bed is
    /// silent after it): the last downbeat inside the final
    /// [`BED_END_WINDOW_S`] of the bed. Absent = the fade ends with the video.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_out_at: Option<f64>,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

// ---------------------------------------------------------------------------
// (0.23 W5c) Bed alignment: where in the track the bed starts
// ---------------------------------------------------------------------------

/// Largest start offset searched (s).
pub const BED_START_MAX_S: f64 = 8.0;
/// Step of the start search (s).
pub const BED_START_STEP_S: f64 = 0.010;
/// A handoff counts as on the beat when a downbeat lies within this of it (s).
pub const BED_ALIGN_TOLERANCE_S: f64 = 0.060;
/// The bed must still have this much track after the video's end (s).
pub const BED_TAIL_MARGIN_S: f64 = 1.0;
/// The end fade lands on the last downbeat inside this window before the end
/// of the bed (s).
pub const BED_END_WINDOW_S: f64 = 1.0;

/// The result of [`align_bed`].
#[derive(Debug, Clone, PartialEq)]
pub struct BedAlignment {
    /// Seconds into the track (a multiple of [`BED_START_STEP_S`]).
    pub start: f64,
    /// Project seconds at which the end fade finishes; `None` = with the video.
    pub fade_out_at: Option<f64>,
    /// Handoffs within [`BED_ALIGN_TOLERANCE_S`] of a downbeat at `start`.
    pub aligned: usize,
    /// Handoffs scored.
    pub handoffs: usize,
}

/// The handoff times [`align_bed`] scores, in project seconds: for each beat
/// scene after the first, its handoff anchor, the overlap midpoint
/// `start + max(previous end - start, 0) / 2`. That is the engine's own notion
/// of "the handoff": the time `plan_audio` puts the handoff cue's peak at, the
/// anchor `choreography::snap_handoffs` moves onto a downbeat in a compile
/// without a voice-over, and the one the QA's `handoff_to_downbeat` measures.
/// It is not the scene's raw `start_seconds`: in a speech-led compile the
/// scenes overlap by about half a second, so the two differ by about a quarter
/// of a second, far more than the 60 ms the alignment tolerates, and the new
/// beat reads as arrived in the middle of the crossfade, where the handoff
/// sound lands.
pub fn handoff_anchors(project: &MotionProject) -> Vec<f64> {
    let beats = beat_scenes(project);
    (1..beats.len())
        .map(|i| {
            let s = beats[i].start_seconds;
            s + (beats[i - 1].end_seconds() - s).max(0.0) / 2.0
        })
        .collect()
}

/// (0.23 W5c) Pick where in the track the bed starts. Speech leads (no visual
/// retiming), so instead the bed's start moves.
///
/// `handoffs` are the handoff anchors in project seconds
/// ([`handoff_anchors`]: the overlap midpoints, not the raw scene starts),
/// `downbeats` the track's downbeats in track seconds. Each offset in
/// `[0, BED_START_MAX_S]` in [`BED_START_STEP_S`] steps is scored by the number
/// of handoffs that, shifted by it, lie within [`BED_ALIGN_TOLERANCE_S`] of a
/// downbeat. Among the offsets with the maximal score the **earliest plateau**
/// (the first run of consecutive steps that all have it) is taken and its
/// **centre** chosen, rounded down to the step grid, so the aligned handoffs sit
/// in the middle of their windows rather than on the edge; a later plateau with
/// the same score never wins. A maximal score of 0 (no handoffs, no downbeats,
/// nothing reachable) gives offset 0. A plateau cut short by the range (at 0 or
/// at the largest offset that fits) is centred inside the range.
///
/// The bed must still cover the video with [`BED_TAIL_MARGIN_S`] to spare
/// (`start + video_duration <= track_duration - 1 s`), so the search stops at
/// the largest offset that fits (0 when none does). When a downbeat lies in the
/// last [`BED_END_WINDOW_S`] before `start + video_duration`, the latest such
/// downbeat ends the fade-out. Pure and deterministic (integer milliseconds
/// inside).
pub fn align_bed(
    handoffs: &[f64],
    downbeats: &[f64],
    track_duration: f64,
    video_duration: f64,
) -> BedAlignment {
    let ms = |t: f64| (t * 1000.0).round() as i64;
    let step = ms(BED_START_STEP_S).max(1);
    let tol = ms(BED_ALIGN_TOLERANCE_S);
    let mut beats: Vec<i64> = downbeats
        .iter()
        .copied()
        .filter(|d| d.is_finite())
        .map(ms)
        .collect();
    beats.sort_unstable();
    let times: Vec<i64> = handoffs
        .iter()
        .copied()
        .filter(|t| t.is_finite())
        .map(ms)
        .collect();
    let tail = ms(BED_TAIL_MARGIN_S);
    let max_start = ms(BED_START_MAX_S)
        .min(ms(track_duration) - tail - ms(video_duration))
        .max(0);
    // A downbeat within `tol` of `t`, by binary search.
    let near = |t: i64| {
        let i = beats.partition_point(|&b| b < t - tol);
        beats.get(i).is_some_and(|&b| b <= t + tol)
    };
    // The score of every step 0, step, 2 step ... <= max_start.
    let scores: Vec<usize> = if beats.is_empty() {
        Vec::new()
    } else {
        (0..=max_start / step)
            .map(|k| times.iter().filter(|&&t| near(t + k * step)).count())
            .collect()
    };
    let top = scores.iter().copied().max().unwrap_or(0);
    let mut best = 0i64;
    if top > 0 {
        // The earliest plateau at the top score: steps `first..=last`.
        let first = scores.iter().position(|&s| s == top).unwrap_or(0);
        let run = scores[first..].iter().take_while(|&&s| s == top).count();
        let last = first + run.max(1) - 1;
        best = ((first + last) / 2) as i64 * step;
    }
    // The end fade: the last downbeat in [end - window, end). One exactly at
    // the end is the end itself (nothing to move).
    let end = best + ms(video_duration);
    let fade_out_at = beats
        .iter()
        .copied()
        .filter(|&d| d >= end - ms(BED_END_WINDOW_S) && d < end)
        .max()
        .map(|d| d - best);
    BedAlignment {
        start: best as f64 / 1000.0,
        fade_out_at: fade_out_at.map(|t| t as f64 / 1000.0),
        aligned: top,
        handoffs: times.len(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioPlan {
    pub version: String,
    /// Sorted by (time, priority desc, sound_id).
    pub cues: Vec<AudioCue>,
    /// Integrated LUFS target for the mix.
    pub loudness_target: f64,
    /// Limiter ceiling (dBTP).
    pub true_peak_limit: f64,
    /// The style's minimum transient spacing (s) the plan was built with (QA checks it).
    pub min_spacing: f64,
    /// The style's information cues per beat cap (QA checks it).
    pub information_cap: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music: Option<MusicBed>,
    /// (0.10) Cues moved (or dropped) to keep their peak [`SPEECH_CLEARANCE`]
    /// away from every word onset. Empty without speech.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speech_adjustments: Vec<SpeechAdjustment>,
    /// (0.23) Voice-relative bed levels for the mix ([`MixLevels::for_style`]),
    /// written by `plan-audio` when a voice-over is planned; absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<MixLevels>,
}

/// (0.10) One cue the speech-aware planner moved off a word onset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeechAdjustment {
    pub scene: String,
    pub kind: CueKind,
    pub sound_id: String,
    /// Planned peak time (s).
    pub from: f64,
    /// New peak time; `None` = the cue was dropped (no free time in its window).
    pub to: Option<f64>,
    /// The word onset that was too close (s).
    pub onset: f64,
}

impl AudioPlan {
    pub fn from_json(json: &str) -> Result<Self, AudioError> {
        let plan: AudioPlan = serde_json::from_str(json)?;
        if plan.version != AUDIO_PLAN_VERSION {
            return Err(AudioError::Version {
                what: "audio plan",
                found: plan.version,
                expected: AUDIO_PLAN_VERSION,
            });
        }
        Ok(plan)
    }
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).expect("audio plan serializes")
    }
}

/// Information arrivals of one scene in scene-local seconds: motion starts in
/// `[life.evolve, life.anticipate)`, excluding targets ending in `.stage` or
/// `.ghost`, clustered (a start within [`ARRIVAL_CLUSTER_SECONDS`] of the
/// previous cluster member joins it; the cluster time is its first start).
/// Scenes without a lifecycle have no arrivals.
pub fn arrival_times(scene: &Scene) -> Vec<f64> {
    let Some(life) = scene.lifecycle else {
        return Vec::new();
    };
    let mut starts: Vec<f64> = scene
        .motions
        .iter()
        .filter(|m| !m.target.ends_with(".stage") && !m.target.ends_with(".ghost"))
        .map(|m| m.start)
        .filter(|&t| t.is_finite() && t >= life.evolve && t < life.anticipate)
        .collect();
    starts.sort_by(|a, b| a.total_cmp(b));
    let mut out: Vec<f64> = Vec::new();
    let mut last: Option<f64> = None;
    for t in starts {
        match last {
            Some(prev) if t - prev < ARRIVAL_CLUSTER_SECONDS => {}
            _ => out.push(t),
        }
        last = Some(t);
    }
    out
}

/// Plan every cue for a compiled project. Pure and deterministic; see
/// docs/SOUND_DESIGN.md §3 for the algorithm.
///
/// `beat_energy[i]` is the energy of the i-th beat scene ([`beat_scenes`]);
/// missing entries count as `Building`. The style seed is
/// `taste.effective.seed`.
pub fn plan_audio(
    project: &MotionProject,
    taste: &ResolvedStyleProfile,
    beat_energy: &[Energy],
    library: &SfxLibrary,
    music: Option<&MusicPlan>,
) -> AudioPlan {
    plan_audio_explored(project, taste, beat_energy, library, music, None)
}

/// (0.9) [`plan_audio`] with an exploration seed. `None` is byte-identical to
/// `plan_audio`; `Some(s)` mixes `s` into the FNV key that picks a sound inside
/// a family (the family, cue kinds and times are unchanged).
pub fn plan_audio_explored(
    project: &MotionProject,
    taste: &ResolvedStyleProfile,
    beat_energy: &[Energy],
    library: &SfxLibrary,
    music: Option<&MusicPlan>,
    explore_seed: Option<u64>,
) -> AudioPlan {
    plan_audio_with_speech(
        project,
        taste,
        beat_energy,
        library,
        music,
        explore_seed,
        None,
    )
}

/// (0.10) [`plan_audio_explored`] with an optional [`SpeechMap`]. `None` is
/// byte-identical to `plan_audio_explored`. With speech, after collisions any
/// cue whose peak lies within [`SPEECH_CLEARANCE`] of a word onset moves to the
/// nearest time at least that far from every onset (earlier on ties), inside
/// its scene and its own lifecycle window and clear of the other transient
/// cues by `min_spacing`; when no such time exists it is dropped. Both are
/// recorded in `speech_adjustments`.
pub fn plan_audio_with_speech(
    project: &MotionProject,
    taste: &ResolvedStyleProfile,
    beat_energy: &[Energy],
    library: &SfxLibrary,
    music: Option<&MusicPlan>,
    explore_seed: Option<u64>,
    speech: Option<&SpeechMap>,
) -> AudioPlan {
    let seed = match explore_seed {
        None => taste.effective.seed,
        Some(s) => crate::compiler::mix(taste.effective.seed ^ 0x0005_FCE8, s),
    };
    let temperament = taste.motion.kind;
    let density = taste.density.level;
    let transition = taste.transition.family;
    let duration = project.duration_seconds();

    // (0.23 W5c) With a voice-over the bed starts `start` s into the track, so
    // the track's downbeats are at `d - start` on the project timeline. The
    // alignment is chosen once, here; the impact snap below uses the shifted
    // grid and the bed is built from the same alignment.
    let align: Option<BedAlignment> = match (music, speech) {
        (Some(m), Some(_)) => Some(align_bed(
            &handoff_anchors(project),
            &m.downbeat_times,
            m.duration,
            duration,
        )),
        _ => None,
    };
    let bed_start = align.as_ref().map_or(0.0, |a| a.start);
    let grid: Vec<f64> = match music {
        Some(m) if bed_start != 0.0 => m
            .downbeat_times
            .iter()
            .map(|d| d - bed_start)
            .filter(|d| *d >= 0.0)
            .collect(),
        Some(m) => m.downbeat_times.clone(),
        None => Vec::new(),
    };

    let mut cands: Vec<Candidate> = Vec::new();
    let mut counters = Counters::default();
    let mut first_impact_done = false;

    let beats = beat_scenes(project);
    for (i, scene) in beats.iter().copied().enumerate() {
        let s = scene.start_seconds;
        let enter = scene.lifecycle.map(|l| l.enter).unwrap_or(0.0);
        let energy = beat_energy.get(i).copied().unwrap_or(Energy::Building);
        counters.reset();

        // open
        if i == 0 {
            if let Some(f) = open_family(temperament) {
                cands.push(make_candidate(
                    library,
                    seed,
                    scene,
                    CueKind::Open,
                    f,
                    f.peak_target_db(),
                    s + enter,
                    &mut counters,
                    format!("open: {} → {}", name_of(&temperament), f.as_str()),
                ));
            }
        }

        // handoff
        if i >= 1 {
            let prev_end = beats[i - 1].end_seconds();
            let t = s + (prev_end - s).max(0.0) / 2.0;
            if let Some((f, target)) = handoff_cue(transition) {
                cands.push(make_candidate(
                    library,
                    seed,
                    scene,
                    CueKind::Handoff,
                    f,
                    target,
                    t,
                    &mut counters,
                    format!("handoff: {} → {}", name_of(&transition), f.as_str()),
                ));
            }
        }

        // impact
        if energy == Energy::Impact {
            let f = impact_family(temperament, transition);
            // (Phase 3) With a MusicPlan the hit moves to the nearest downbeat
            // inside [handoff anchor, S + settle] (the anchor is S + enter for
            // the first beat), so a hit can land with a snapped handoff.
            // (0.23 W5c) The downbeats are those of the bed as it plays: the
            // track's, shifted by the bed's start.
            let on_beat = match (music, scene.lifecycle) {
                (Some(_), Some(l)) => {
                    let lo = if i >= 1 {
                        let prev_end = beats[i - 1].end_seconds();
                        (s + (prev_end - s).max(0.0) / 2.0).min(s + l.enter)
                    } else {
                        s + l.enter
                    };
                    nearest_downbeat(&grid, lo, s + l.settle, s + l.enter)
                }
                _ => None,
            };
            let impact_t = on_beat.unwrap_or(s + enter);
            cands.push(make_candidate(
                library,
                seed,
                scene,
                CueKind::Impact,
                f,
                f.peak_target_db(),
                impact_t,
                &mut counters,
                format!(
                    "impact: {} / {} → {}{}",
                    name_of(&temperament),
                    name_of(&transition),
                    f.as_str(),
                    if on_beat.is_some() {
                        " (on downbeat)"
                    } else {
                        ""
                    }
                ),
            ));
            if !first_impact_done {
                first_impact_done = true;
                let t = impact_t;
                let near = cands.iter().any(|c| {
                    c.family == SfxFamily::Subdrop && (c.time_raw - t).abs() <= 1.0 + 1e-9
                });
                if !near {
                    let f = SfxFamily::Subdrop;
                    cands.push(make_candidate(
                        library,
                        seed,
                        scene,
                        CueKind::Impact,
                        f,
                        f.peak_target_db(),
                        t,
                        &mut counters,
                        format!("impact: first impact bed → {}", f.as_str()),
                    ));
                }
            }
        }

        // (0.10 Q) Accent: with a voice-over, the beat's punch word (its first
        // number, else the payoff word of an impact beat) gets one cue that
        // peaks ACCENT_LEAD before the word starts — synchronised with the
        // word without masking it.
        if let Some(sp) = speech {
            if let Some((word, t)) = accent_word(sp, i, energy) {
                let (f, target) = accent_cue(project, temperament);
                cands.push(make_candidate(
                    library,
                    seed,
                    scene,
                    CueKind::Information,
                    f,
                    target,
                    t - ACCENT_LEAD,
                    &mut counters,
                    format!("accent: '{word}' → {}", f.as_str()),
                ));
            }
        }

        // information
        if let Some((f, target)) = information_cue(temperament, density) {
            for a in arrival_times(scene)
                .into_iter()
                .take(information_cap(density))
            {
                cands.push(make_candidate(
                    library,
                    seed,
                    scene,
                    CueKind::Information,
                    f,
                    target,
                    s + a,
                    &mut counters,
                    format!("information: {} → {}", name_of(&temperament), f.as_str()),
                ));
            }
        }
    }

    // Candidates without a sound (family missing) or outside the project drop out.
    let mut live: Vec<AudioCue> = cands
        .into_iter()
        .filter_map(|c| c.cue)
        .filter(|c| c.time >= 0.0 && c.time <= duration)
        .collect();

    // Collisions: accept in (priority desc, time asc, sound id asc).
    live.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then(a.time.total_cmp(&b.time))
            .then(a.sound_id.cmp(&b.sound_id))
    });
    let spacing = min_spacing(taste.rhythm);
    let mut accepted: Vec<AudioCue> = Vec::new();
    for c in live {
        if !c.family.is_bed()
            && accepted
                .iter()
                .any(|a| !a.family.is_bed() && (a.time - c.time).abs() < spacing - 1e-9)
        {
            continue;
        }
        accepted.push(c);
    }
    accepted.sort_by(|a, b| {
        a.time
            .total_cmp(&b.time)
            .then(b.priority.cmp(&a.priority))
            .then(a.sound_id.cmp(&b.sound_id))
    });

    let speech_adjustments = match speech {
        Some(sp) => adjust_for_speech(&mut accepted, project, sp, spacing, duration),
        None => Vec::new(),
    };

    AudioPlan {
        version: AUDIO_PLAN_VERSION.to_string(),
        cues: accepted,
        loudness_target: LOUDNESS_TARGET_LUFS,
        true_peak_limit: TRUE_PEAK_LIMIT_DB,
        min_spacing: spacing,
        information_cap: information_cap(density),
        // (0.23 W5c) With a voice-over the bed starts where its downbeats meet
        // the handoffs ([`align_bed`], above); without one it plays from its
        // first second.
        music: music.map(|m| MusicBed {
            track: m.track.clone(),
            gain_db: m.gain_db,
            fade_in: 0.5,
            fade_out: 1.0,
            duck: true,
            start: bed_start,
            fade_out_at: align.as_ref().and_then(|a| a.fade_out_at),
        }),
        speech_adjustments,
        levels: None,
    }
}

/// The word onset closest to `t` (`None` without words).
fn nearest_onset(onsets: &[f64], t: f64) -> Option<f64> {
    onsets
        .iter()
        .copied()
        .min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
}

/// Move cues off word onsets (see [`plan_audio_with_speech`]). `cues` is the
/// final sorted list; it is re-sorted and returned without dropped cues.
fn adjust_for_speech(
    cues: &mut Vec<AudioCue>,
    project: &MotionProject,
    speech: &SpeechMap,
    spacing: f64,
    duration: f64,
) -> Vec<SpeechAdjustment> {
    let onsets: Vec<f64> = speech
        .words
        .iter()
        .map(|w| w.start)
        .filter(|t| t.is_finite())
        .collect();
    let clear = |t: f64| {
        onsets
            .iter()
            .all(|o| (t - o).abs() >= SPEECH_CLEARANCE - 1e-9)
    };
    let mut adjustments = Vec::new();
    let mut dropped: Vec<usize> = Vec::new();
    for i in 0..cues.len() {
        let cue = cues[i].clone();
        let Some(onset) = nearest_onset(&onsets, cue.time) else {
            break;
        };
        if clear(cue.time) {
            continue;
        }
        // The cue's window on the project timeline.
        let scene = project.scenes.iter().find(|s| s.id == cue.scene);
        let (mut lo, mut hi) = (0.0_f64, duration);
        if let Some(s) = scene {
            lo = lo.max(s.start_seconds);
            hi = hi.min(s.end_seconds());
            if let Some(l) = s.lifecycle {
                match cue.kind {
                    CueKind::Handoff => {}
                    CueKind::Information => {
                        lo = lo.max(s.start_seconds + l.evolve);
                        hi = hi.min(s.start_seconds + l.anticipate - 0.001);
                    }
                    CueKind::Open | CueKind::Impact => {
                        hi = hi.min(s.start_seconds + l.anticipate - 0.001);
                    }
                }
            }
        }
        let free = |t: f64| {
            clear(t)
                && (cue.family.is_bed()
                    || cues.iter().enumerate().all(|(j, o)| {
                        j == i
                            || dropped.contains(&j)
                            || o.family.is_bed()
                            || (o.time - t).abs() >= spacing - 1e-9
                    }))
        };
        // Nearest free millisecond to the planned time, earlier first.
        let t0 = (cue.time * 1000.0).round() as i64;
        let (lo_ms, hi_ms) = ((lo * 1000.0).ceil() as i64, (hi * 1000.0).floor() as i64);
        let mut found: Option<f64> = None;
        let max_d = (hi_ms - lo_ms).max(0);
        for d in 1..=max_d {
            for ms in [t0 - d, t0 + d] {
                if ms >= lo_ms && ms <= hi_ms && free(ms as f64 / 1000.0) {
                    found = Some(ms as f64 / 1000.0);
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        adjustments.push(SpeechAdjustment {
            scene: cue.scene.clone(),
            kind: cue.kind,
            sound_id: cue.sound_id.clone(),
            from: cue.time,
            to: found,
            onset: round_ms(onset),
        });
        match found {
            Some(t) => cues[i].time = round_ms(t),
            None => dropped.push(i),
        }
    }
    let mut keep = 0usize;
    cues.retain(|_| {
        let k = !dropped.contains(&keep);
        keep += 1;
        k
    });
    cues.sort_by(|a, b| {
        a.time
            .total_cmp(&b.time)
            .then(b.priority.cmp(&a.priority))
            .then(a.sound_id.cmp(&b.sound_id))
    });
    adjustments
}

/// The downbeat closest to `target` inside `[lo, hi]` (ties → the earlier).
fn nearest_downbeat(downbeats: &[f64], lo: f64, hi: f64, target: f64) -> Option<f64> {
    let mut best: Option<f64> = None;
    for &b in downbeats {
        if b < lo - 1e-9 || b > hi + 1e-9 {
            continue;
        }
        best = match best {
            None => Some(b),
            Some(cur) => {
                let (db, dc) = ((b - target).abs(), (cur - target).abs());
                if db < dc || (db == dc && b < cur) {
                    Some(b)
                } else {
                    Some(cur)
                }
            }
        };
    }
    best
}

/// Beat scenes in order: the scenes carrying a lifecycle (the compiler's
/// whole-piece backdrop has none). A hand-authored project without any
/// lifecycle treats every scene as a beat.
pub fn beat_scenes(project: &MotionProject) -> Vec<&Scene> {
    let with_life: Vec<&Scene> = project
        .scenes
        .iter()
        .filter(|s| s.lifecycle.is_some())
        .collect();
    if with_life.is_empty() {
        project.scenes.iter().collect()
    } else {
        with_life
    }
}

struct Candidate {
    family: SfxFamily,
    /// Unrounded time (used for the 1 s subdrop proximity rule).
    time_raw: f64,
    cue: Option<AudioCue>,
}

/// Per-scene ordinal counters per cue kind.
#[derive(Default)]
struct Counters {
    open: u64,
    handoff: u64,
    information: u64,
    impact: u64,
}

impl Counters {
    fn reset(&mut self) {
        *self = Counters::default();
    }
    fn next(&mut self, kind: CueKind) -> u64 {
        let slot = match kind {
            CueKind::Open => &mut self.open,
            CueKind::Handoff => &mut self.handoff,
            CueKind::Information => &mut self.information,
            CueKind::Impact => &mut self.impact,
        };
        let v = *slot;
        *slot += 1;
        v
    }
}

fn name_of<T: std::fmt::Debug>(v: &T) -> String {
    format!("{v:?}").to_lowercase()
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

#[allow(clippy::too_many_arguments)]
fn make_candidate(
    library: &SfxLibrary,
    seed: u64,
    scene: &Scene,
    kind: CueKind,
    family: SfxFamily,
    target_db: f64,
    time: f64,
    counters: &mut Counters,
    reason: String,
) -> Candidate {
    let ordinal = counters.next(kind);
    let sounds = library.family(family);
    let cue = if sounds.is_empty() {
        None
    } else {
        let mut h = Fnv64::default();
        h.write(&seed.to_le_bytes());
        h.write(scene.id.as_bytes());
        h.write(&[0xff]);
        h.write(kind.as_str().as_bytes());
        h.write(&[0xff]);
        h.write(&ordinal.to_le_bytes());
        let idx = (h.finish() % sounds.len() as u64) as usize;
        let sound = sounds[idx];
        Some(AudioCue {
            time: round_ms(time),
            scene: scene.id.clone(),
            kind,
            family,
            sound_id: sound.id.clone(),
            gain_db: round1((target_db - sound.peak_db).clamp(MIN_GAIN_DB, MAX_GAIN_DB)),
            priority: kind.priority(),
            reason,
        })
    };
    Candidate {
        family,
        time_raw: time,
        cue,
    }
}
