//! (0.23) Frozen names and thresholds of the QA checks added in sprint 0.23
//! (plan `docs/plans/SPRINT_0_23_VARIETY_QA_AUDIO.md` §1, §3 W4 / W5a, §4).
//! Every report, test and bench uses these constants; thresholds change only
//! through the coordinator.

/// Layout QA (rendered): every text layer readable at READ contrasts with the
/// rendered pixels around its ink box (median of a ring, glyphs excluded).
/// FAIL below [`TEXT_CONTRAST_DISPLAY`] for display text, below
/// [`TEXT_CONTRAST_BODY`] for smaller text.
pub const TEXT_LOCAL_CONTRAST: &str = "text_local_contrast";
/// Motion QA: the first readable element that is not backdrop or caption
/// appears within [`DEAD_AIR_FIRST_READABLE_S`], and no beat holds longer than
/// [`DEAD_AIR_MAX_HOLD_S`] after its start with only backdrop + captions.
pub const DEAD_AIR: &str = "dead_air";
/// Motion QA: a counting number reaches its final value within
/// [`COUNT_SETTLE_S`] of its anchor word (by READ without speech) and holds
/// it at least [`COUNT_HOLD_S`] before the exit.
pub const COUNT_UNSETTLED: &str = "count_unsettled";
/// Motion QA (WARN): two consecutive beats share template + entrance + camera
/// move where an alternative existed.
pub const REPEAT_TEMPLATE: &str = "repeat_template";
/// Speech QA (stems, voiced segments): voice loudness over the music bed.
/// PASS at or above [`VOICE_OVER_MUSIC_PASS_DB`], WARN between, FAIL below
/// [`VOICE_OVER_MUSIC_FAIL_DB`].
pub const VOICE_OVER_MUSIC: &str = "voice_over_music";
/// Speech QA (stems): the bed's momentary (400 ms) level in narration gaps
/// stays at or below voice + [`BED_OVER_VOICE_MAX_DB`]. FAIL above.
pub const BED_OVER_VOICE: &str = "bed_over_voice";
/// Speech QA (stems, WARN): the bed's 300–4000 Hz level under speech stays at
/// or below voice + [`SPEECH_BAND_MASKING_MAX_DB`].
pub const SPEECH_BAND_MASKING: &str = "speech_band_masking";
/// (0.23, owner decision) Layout QA: two readable, non-decorative text layers
/// overlap. Text over text is a defect in general; a deliberate impact overlap
/// is allowed only when one of the two is an impact layer (stamp, punchword,
/// slam unit, echo — `layout_qa::IMPACT_LAYERS`), neither is a value / figure,
/// and it lasts at most [`TEXT_OVERLAP_IMPACT_S`]. FAIL otherwise; an allowed
/// impact overlap is reported as WARN-free information.
pub const TEXT_OVERLAP: &str = "text_overlap";
/// (0.23, W8) Motion QA: a picture the story carries into the next beat
/// (`continuity: carry_*`) stays visible across the handoff — no frame
/// between its beat's READ and the next beat's READ without it.
pub const CARRY_CONTINUITY: &str = "carry_continuity";
/// Speech QA (`asr-local` builds): words recognised on the final mix against
/// the clean voice. PASS within [`MIX_INTELLIGIBILITY_PASS_POINTS`], FAIL
/// beyond [`MIX_INTELLIGIBILITY_FAIL_POINTS`] percentage points.
pub const MIX_INTELLIGIBILITY: &str = "mix_intelligibility";

/// WCAG contrast for display text (at least [`DISPLAY_TEXT_PX`] × u).
pub const TEXT_CONTRAST_DISPLAY: f64 = 3.0;
/// WCAG contrast for smaller text.
pub const TEXT_CONTRAST_BODY: f64 = 4.5;
/// Display text threshold in px at u = 1 (1080 short side).
pub const DISPLAY_TEXT_PX: f32 = 24.0;

/// Share of the smaller box two text layers may overlap before it counts.
pub const TEXT_OVERLAP_MIN_SHARE: f64 = 0.08;
/// Longest deliberate impact overlap (s).
pub const TEXT_OVERLAP_IMPACT_S: f64 = 0.8;

pub const DEAD_AIR_FIRST_READABLE_S: f64 = 0.5;
pub const DEAD_AIR_MAX_HOLD_S: f64 = 1.2;

pub const COUNT_SETTLE_S: f64 = 0.8;
pub const COUNT_HOLD_S: f64 = 1.0;

pub const VOICE_OVER_MUSIC_PASS_DB: f64 = 15.0;
pub const VOICE_OVER_MUSIC_FAIL_DB: f64 = 10.0;
/// Bed level in narration gaps relative to the voice (dB; bed <= voice - 6).
pub const BED_OVER_VOICE_MAX_DB: f64 = -6.0;
/// Bed speech-band level under speech relative to the voice (dB).
pub const SPEECH_BAND_MASKING_MAX_DB: f64 = -18.0;
pub const MIX_INTELLIGIBILITY_PASS_POINTS: f64 = 2.0;
pub const MIX_INTELLIGIBILITY_FAIL_POINTS: f64 = 5.0;
