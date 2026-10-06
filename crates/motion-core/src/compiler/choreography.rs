//! Choreography v0 (0.8): the WHEN layer for music. Retimes beat durations so
//! scene handoffs land on downbeats, BEFORE lifecycle planning. Taste tables
//! are never edited; without a MusicPlan nothing here runs. See
//! docs/SOUND_DESIGN.md §7.

/// A snapped beat never shrinks below this fraction of its planned duration.
pub const MIN_STRETCH: f64 = 0.85;
/// A snapped beat never grows above this fraction of its planned duration.
pub const MAX_STRETCH: f64 = 1.20;
/// Absolute floor for a snapped beat (seconds).
pub const MIN_BEAT_SECONDS: f64 = 2.0;

/// Snap each handoff to the nearest downbeat within `tolerance` by changing
/// the duration of the beat BEFORE it. Pure and deterministic.
///
/// `durations[i]` and `overlaps_out[i]` are beat `i`'s planned duration and
/// outgoing overlap (seconds). Beat `i` starts at
/// `sum_{j<i} (durations[j] - overlaps_out[j])`; the handoff into beat `i ≥ 1`
/// is anchored at `start_i + overlaps_out[i-1] / 2` (the overlap midpoint,
/// the same anchor `plan_audio` uses). Handoffs are processed in order with
/// the already-snapped durations. A handoff snaps to the nearest downbeat `b`
/// with `|b - anchor| <= tolerance` (ties → the earlier downbeat) when the new
/// duration stays within `[MIN_STRETCH·d, MAX_STRETCH·d]` and `>=
/// MIN_BEAT_SECONDS`; otherwise it is left unchanged. Results are rounded to
/// milliseconds. Returns the new durations (same length).
pub fn snap_handoffs(
    durations: &[f64],
    overlaps_out: &[f64],
    downbeats: &[f64],
    tolerance: f64,
) -> Vec<f64> {
    let mut out = durations.to_vec();
    if downbeats.is_empty() || out.len() < 2 {
        return out;
    }
    let mut sorted: Vec<f64> = downbeats.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let overlap = |i: usize| overlaps_out.get(i).copied().unwrap_or(0.0);

    // Start of beat `i` from the (already snapped) preceding durations.
    let mut start = 0.0;
    for i in 1..out.len() {
        start += out[i - 1] - overlap(i - 1);
        let anchor = start + overlap(i - 1) / 2.0;
        // Nearest downbeat within tolerance; strict `<` keeps the earlier on ties.
        let mut best: Option<f64> = None;
        for &b in &sorted {
            let d = (b - anchor).abs();
            if d <= tolerance && best.is_none_or(|c| d < (c - anchor).abs()) {
                best = Some(b);
            }
        }
        let Some(b) = best else { continue };
        let planned = durations[i - 1];
        // The anchor moves one-for-one with the previous beat's duration.
        let new_d = round_ms(out[i - 1] + (b - anchor));
        if new_d >= MIN_BEAT_SECONDS
            && new_d >= planned * MIN_STRETCH - 1e-9
            && new_d <= planned * MAX_STRETCH + 1e-9
        {
            start += new_d - out[i - 1];
            out[i - 1] = new_d;
        }
    }
    out
}

fn round_ms(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}
