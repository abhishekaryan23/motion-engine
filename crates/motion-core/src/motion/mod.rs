//! Motion language building blocks.
//!
//! Engine-native behaviors (stagger, kinetic typography, data primitives) that
//! **expand into primitive MotionScene layers + motions**. They carry no
//! semantic meaning: the MotionCompiler decides *which* behavior expresses a
//! beat; these functions decide *how* the behavior moves. Everything is a pure,
//! deterministic function of its inputs, so the Timeline and Renderer never
//! learn about words, cascades or charts.

pub mod dataviz;
pub mod kinetic;
pub mod language;
pub mod lifecycle;
pub mod stagger;

use crate::scene::{Layer, Motion};

/// Result of expanding a behavior: layers to insert (in draw order) and the
/// scene-local motions that animate them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Expansion {
    pub layers: Vec<Layer>,
    pub motions: Vec<Motion>,
}

impl Expansion {
    pub fn extend(&mut self, other: Expansion) {
        self.layers.extend(other.layers);
        self.motions.extend(other.motions);
    }
}
