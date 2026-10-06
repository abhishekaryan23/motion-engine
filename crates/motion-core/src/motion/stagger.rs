//! Deterministic, irregular stagger timing.
//!
//! Offsets are closed-form per index (never accumulated): for an element with
//! rank `r` in a pattern of length `L`,
//! `offset(r) = pattern[r % L] + (r / L) * cycle`, in frames at the 30 fps
//! reference, converted to seconds (`/ 30.0`) so timing is fps-independent.

use serde::{Deserialize, Serialize};

/// Named irregular rhythms (frames @30fps).
/// Editorial `[0, 4, 7, 13, 18]` cycle 22 · Calm `[0, 6, 11, 19]` cycle 25 ·
/// Impact `[0, 3, 5, 10]` cycle 12.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StaggerPreset {
    Calm,
    Editorial,
    Impact,
}

/// Which element goes first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StaggerOrder {
    #[default]
    Forward,
    Reverse,
    /// Middle element(s) first, then outwards (ties: lower index first).
    CenterOut,
    /// Both ends first, then inwards (ties: lower index first).
    EdgesIn,
}

/// The unit a stagger walks over (documentation / tooling; the caller decides
/// what the targets are).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StaggerUnit {
    Word,
    Line,
    Item,
    Layer,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StaggerSpec {
    pub preset: StaggerPreset,
    pub order: StaggerOrder,
}

/// Reference frame rate the patterns are authored at.
const REFERENCE_FPS: f64 = 30.0;

impl StaggerPreset {
    /// `(pattern, cycle)` in reference frames.
    pub fn pattern(self) -> (&'static [u32], u32) {
        match self {
            StaggerPreset::Editorial => (&[0, 4, 7, 13, 18], 22),
            StaggerPreset::Calm => (&[0, 6, 11, 19], 25),
            StaggerPreset::Impact => (&[0, 3, 5, 10], 12),
        }
    }

    /// Start offset in seconds of the element with rank `rank`. Closed form:
    /// `(pattern[rank % L] + (rank / L) * cycle) / 30`.
    pub fn offset_for_rank(self, rank: usize) -> f64 {
        let (pattern, cycle) = self.pattern();
        let len = pattern.len();
        let frames = pattern[rank % len] as f64 + (rank / len) as f64 * cycle as f64;
        frames / REFERENCE_FPS
    }
}

/// Rank of each index `0..n` under `order` (rank 0 moves first).
pub fn ranks(order: StaggerOrder, n: usize) -> Vec<usize> {
    match order {
        StaggerOrder::Forward => (0..n).collect(),
        StaggerOrder::Reverse => (0..n).map(|i| n - 1 - i).collect(),
        StaggerOrder::CenterOut => {
            // Distance from the center, doubled so it stays an integer.
            rank_by_key(n, |i| (2 * i).abs_diff(n.saturating_sub(1)))
        }
        StaggerOrder::EdgesIn => rank_by_key(n, |i| i.min(n - 1 - i)),
    }
}

/// Rank indices by `(key, index)` ascending (ties: lower index first).
fn rank_by_key(n: usize, key: impl Fn(usize) -> usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| (key(i), i));
    let mut ranks = vec![0; n];
    for (rank, &i) in order.iter().enumerate() {
        ranks[i] = rank;
    }
    ranks
}

/// Start offset in seconds for each index `0..n` (index order, not rank order).
pub fn offsets(spec: StaggerSpec, n: usize) -> Vec<f64> {
    ranks(spec.order, n)
        .into_iter()
        .map(|r| spec.preset.offset_for_rank(r))
        .collect()
}
