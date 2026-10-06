//! Typesetting decisions: voices (typographic roles in context), size fitting
//! and balanced line breaking. Produces explicit Text layers.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use super::measure::TextMeasure;
use super::theme::FontSet;
use crate::motion::kinetic::{TextRun, UnitBox};
use crate::scene::{Color, FontRole, InkBounds, Layer, LayerKind, TextAlign, TextStyle};

/// A typographic voice: role plus the settings that make it read as intended.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Voice {
    pub role: FontRole,
    pub weight: u16,
    pub italic: bool,
    pub uppercase: bool,
    /// em
    pub letter_spacing: f32,
    /// multiple of font size
    pub line_height: f32,
}

impl Voice {
    pub const HEADLINE: Voice = Voice {
        role: FontRole::Display,
        weight: 900,
        italic: false,
        uppercase: true,
        letter_spacing: -0.02,
        line_height: 0.98,
    };
    pub const HERO_NUMBER: Voice = Voice {
        role: FontRole::Number,
        weight: 400,
        italic: false,
        uppercase: false,
        letter_spacing: -0.01,
        line_height: 1.0,
    };
    pub const GHOST: Voice = Voice {
        role: FontRole::DisplayCondensed,
        weight: 400,
        italic: false,
        uppercase: true,
        letter_spacing: -0.01,
        line_height: 1.0,
    };
    pub const SERIF: Voice = Voice {
        role: FontRole::SerifEmotional,
        weight: 400,
        italic: true,
        uppercase: false,
        letter_spacing: -0.01,
        line_height: 1.08,
    };
    pub const KICKER: Voice = Voice {
        role: FontRole::Mono,
        weight: 400,
        italic: false,
        uppercase: true,
        letter_spacing: 0.14,
        line_height: 1.2,
    };
    pub const LABEL: Voice = Voice {
        role: FontRole::Mono,
        weight: 700,
        italic: false,
        uppercase: true,
        letter_spacing: 0.08,
        line_height: 1.2,
    };
    pub const BODY: Voice = Voice {
        role: FontRole::Body,
        weight: 400,
        italic: false,
        uppercase: false,
        letter_spacing: 0.0,
        line_height: 1.3,
    };
}

/// Lines set at a size.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub lines: Vec<String>,
    pub size: f32,
    pub line_widths: Vec<f32>,
    pub voice: Voice,
}

impl Block {
    pub fn width(&self) -> f32 {
        self.line_widths.iter().copied().fold(0.0, f32::max)
    }
    pub fn line_advance(&self) -> f32 {
        self.size * self.voice.line_height
    }
    pub fn height(&self) -> f32 {
        self.line_advance() * self.lines.len() as f32
    }
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }
}

/// Reference size for measurement; widths scale linearly with size.
const REF_SIZE: f32 = 100.0;

/// (0.10 Q) The display headline budget of the beat being built
/// (`taste_rules::HEADLINE_MAX_*`). [`Typesetter::fit_block`] applies it to the
/// beat's headline text (and only that: subject phrases, cards and labels set
/// in the same voice are not headlines). A block within the budget is never
/// touched.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HeadlineBudget {
    /// The headline text (the beat's display title) the budget governs.
    pub text: String,
    /// Tallest the block may be (px).
    pub max_h: f32,
    /// Most lines.
    pub max_lines: usize,
    /// Smallest size (px) the shrink may reach; beyond it the text is cut.
    pub min_size: f32,
}

pub struct Typesetter<'a> {
    measure: &'a dyn TextMeasure,
    root: &'a Path,
    fonts: &'a FontSet,
    cache: RefCell<HashMap<(FontRole, bool, u16, String), f32>>,
    /// (0.6) Scale-contrast gains `(display, support)`; `(1, 1)` = neutral.
    gains: (f32, f32),
    /// (0.10 Q) Headline budget of the beat being built (`None` = unbudgeted).
    budget: RefCell<Option<HeadlineBudget>>,
}

impl<'a> Typesetter<'a> {
    pub fn new(measure: &'a dyn TextMeasure, root: &'a Path, fonts: &'a FontSet) -> Self {
        Typesetter {
            measure,
            root,
            fonts,
            cache: RefCell::new(HashMap::new()),
            gains: (1.0, 1.0),
            budget: RefCell::new(None),
        }
    }

    /// (0.10 Q) Set (or clear) the headline budget for the beat being built.
    pub(crate) fn set_headline_budget(&self, budget: Option<HeadlineBudget>) {
        *self.budget.borrow_mut() = budget;
    }

    /// Apply the style's scale contrast: display voices (headlines, figures,
    /// ghost words) may grow, support voices (body, labels, serif asides)
    /// shrink — sizes stay bounded by the boxes callers fit into.
    pub fn with_scale(mut self, gains: (f32, f32)) -> Self {
        self.gains = gains;
        self
    }

    fn gain(&self, voice: &Voice) -> f32 {
        match voice.role {
            FontRole::Display | FontRole::DisplayCondensed | FontRole::Number => self.gains.0,
            FontRole::SerifEmotional | FontRole::Body | FontRole::Mono => self.gains.1,
        }
    }

    pub fn prepare(voice: &Voice, text: &str) -> String {
        let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if voice.uppercase {
            t.to_uppercase()
        } else {
            t
        }
    }

    /// Width of one line at `size`.
    pub fn width(&self, voice: &Voice, line: &str, size: f32) -> f32 {
        let key = (voice.role, voice.italic, voice.weight, line.to_string());
        if let Some(w) = self.cache.borrow().get(&key) {
            return w * size / REF_SIZE;
        }
        let face = self.fonts.role(voice.role);
        let path = self.root.join(face.path);
        let w = self.measure.line_width(
            &path,
            voice.weight,
            voice.italic,
            line,
            REF_SIZE,
            voice.letter_spacing,
        );
        self.cache.borrow_mut().insert(key, w);
        w * size / REF_SIZE
    }

    /// Vertical ink extent of the block's text (pixels from the box top), when
    /// the measure has glyph outlines.
    pub(crate) fn ink(&self, block: &Block) -> Option<InkBounds> {
        let face = self.fonts.role(block.voice.role);
        let path = self.root.join(face.path);
        self.measure
            .ink_bounds(
                &path,
                block.voice.weight,
                block.voice.italic,
                &block.text(),
                block.size,
                block.voice.line_height,
                block.voice.letter_spacing,
            )
            .map(|(top, bottom)| InkBounds { top, bottom })
    }

    /// Break a block into measured units (words, or glyph clusters when
    /// `glyphs`) for kinetic typography. Unit `x` is the measured width of the
    /// line up to and including the unit minus the unit's own width, so it
    /// carries the preceding spaces, letter spacing and kerning.
    pub(crate) fn text_run(
        &self,
        id: String,
        block: &Block,
        color: Color,
        origin: [f32; 2],
        glyphs: bool,
    ) -> TextRun {
        let voice = &block.voice;
        let lines = block
            .lines
            .iter()
            .map(|line| {
                let mut units = Vec::new();
                let mut ends: Vec<(usize, usize)> = Vec::new();
                if glyphs {
                    for (i, c) in line.char_indices() {
                        if !c.is_whitespace() {
                            ends.push((i, i + c.len_utf8()));
                        }
                    }
                } else {
                    let mut start = None;
                    for (i, c) in line.char_indices() {
                        match (c.is_whitespace(), start) {
                            (false, None) => start = Some(i),
                            (true, Some(s)) => {
                                ends.push((s, i));
                                start = None;
                            }
                            _ => {}
                        }
                    }
                    if let Some(s) = start {
                        ends.push((s, line.len()));
                    }
                }
                for (s, e) in ends {
                    let text = &line[s..e];
                    let width = self.width(voice, text, block.size);
                    let upto = self.width(voice, &line[..e], block.size);
                    units.push(UnitBox {
                        text: text.to_string(),
                        x: (upto - width).max(0.0),
                        width,
                    });
                }
                units
            })
            .collect();
        let ink = block.lines.first().and_then(|first| {
            self.ink(&Block {
                lines: vec![first.clone()],
                size: block.size,
                line_widths: vec![block.line_widths.first().copied().unwrap_or(0.0)],
                voice: block.voice,
            })
        });
        TextRun {
            id,
            style: TextStyle {
                text: String::new(),
                font_role: voice.role,
                font_size: round2(block.size),
                font_weight: voice.weight,
                italic: voice.italic,
                color,
                align: TextAlign::Left,
                line_height: voice.line_height,
                letter_spacing: voice.letter_spacing,
                max_width: None,
                uppercase: false,
                ink,
            },
            origin,
            line_advance: block.line_advance(),
            lines,
            ink,
        }
    }

    /// Set `text` on one line at the largest size <= `max_size` fitting `max_width`.
    pub fn fit_line(&self, voice: Voice, text: &str, max_width: f32, max_size: f32) -> Block {
        let line = Self::prepare(&voice, text);
        let w100 = self.width(&voice, &line, REF_SIZE).max(1.0);
        let size = (max_width / w100 * REF_SIZE)
            .min(max_size * self.gain(&voice))
            .max(1.0);
        let lw = self.width(&voice, &line, size);
        Block {
            lines: vec![line],
            size,
            line_widths: vec![lw],
            voice,
        }
    }

    /// Set `text` at a fixed size, breaking lines so none exceeds `max_width`
    /// (balanced, so lines are similar in length).
    pub fn set_wrapped(&self, voice: Voice, text: &str, size: f32, max_width: f32) -> Block {
        let size = size * self.gain(&voice);
        let prepared = Self::prepare(&voice, text);
        let words: Vec<&str> = prepared.split(' ').collect();
        for n in 1..=words.len().max(1) {
            let lines = self.balance(&voice, &words, n);
            let widths: Vec<f32> = lines.iter().map(|l| self.width(&voice, l, size)).collect();
            if widths.iter().all(|w| *w <= max_width) || n == words.len() {
                return Block {
                    lines,
                    size,
                    line_widths: widths,
                    voice,
                };
            }
        }
        Block {
            lines: vec![prepared],
            size,
            line_widths: vec![0.0],
            voice,
        }
    }

    /// Choose line count and size to maximize size within `max_w x max_h`.
    ///
    /// (0.10 Q) The headline voice obeys the beat's [`HeadlineBudget`] when one
    /// is set: a block taller than the budget or over its line cap is set again
    /// inside both (the size shrinks), and when even the floor size needs more
    /// lines than the cap the text is cut on a word boundary and ends with "…".
    /// A block inside the budget is returned exactly as before.
    pub fn fit_block(
        &self,
        voice: Voice,
        text: &str,
        max_w: f32,
        max_h: f32,
        max_size: f32,
        max_lines: usize,
    ) -> Block {
        let plain = self.fit_block_plain(voice, text, max_w, max_h, max_size, max_lines);
        // (budget height, budget lines, floor size) when this is the budgeted headline.
        let limits = (voice == Voice::HEADLINE)
            .then(|| {
                let guard = self.budget.borrow();
                let b = guard.as_ref()?;
                (Self::prepare(&voice, text) == b.text).then_some((
                    b.max_h,
                    b.max_lines,
                    b.min_size,
                ))
            })
            .flatten();
        let Some((budget_h, budget_lines, min_size)) = limits else {
            return plain;
        };
        if plain.lines.len() <= budget_lines && plain.height() <= budget_h + 0.5 {
            return plain;
        }
        let max_h = max_h.min(budget_h);
        let max_lines = max_lines.min(budget_lines).max(1);
        let floor = min_size.min(max_size * self.gain(&voice)) - 0.01;
        let block = self.fit_block_plain(voice, text, max_w, max_h, max_size, max_lines);
        if block.size >= floor {
            return block;
        }
        // Too long for the floor size: keep the leading words that fit, then "…".
        let prepared = Self::prepare(&voice, text);
        let words: Vec<&str> = prepared.split(' ').collect();
        for k in (1..words.len()).rev() {
            let cut = ellipsized(&words[..k]);
            let b = self.fit_block_plain(voice, &cut, max_w, max_h, max_size, max_lines);
            if b.size >= floor {
                return b;
            }
        }
        block
    }

    /// (0.10 Q) Body copy that stays small: set at `size`, shrunk toward
    /// `min_size` until it fits `max_lines`; if it still does not, cut on a
    /// word boundary and ended with "…".
    pub(crate) fn fit_body(
        &self,
        voice: Voice,
        text: &str,
        size: f32,
        min_size: f32,
        max_w: f32,
        max_lines: usize,
    ) -> Block {
        let mut s = size.max(min_size);
        loop {
            let b = self.set_wrapped(voice, text, s, max_w);
            if b.lines.len() <= max_lines {
                return b;
            }
            if s <= min_size {
                break;
            }
            s = (s * 0.94).max(min_size);
        }
        let prepared = Self::prepare(&voice, text);
        let words: Vec<&str> = prepared.split(' ').collect();
        for k in (1..words.len()).rev() {
            let b = self.set_wrapped(voice, &ellipsized(&words[..k]), min_size, max_w);
            if b.lines.len() <= max_lines {
                return b;
            }
        }
        self.set_wrapped(voice, text, min_size, max_w)
    }

    fn fit_block_plain(
        &self,
        voice: Voice,
        text: &str,
        max_w: f32,
        max_h: f32,
        max_size: f32,
        max_lines: usize,
    ) -> Block {
        let prepared = Self::prepare(&voice, text);
        let words: Vec<&str> = prepared.split(' ').collect();
        let mut best: Option<Block> = None;
        for n in 1..=max_lines.min(words.len()).max(1) {
            let lines = self.balance(&voice, &words, n);
            let widest = lines
                .iter()
                .map(|l| self.width(&voice, l, REF_SIZE))
                .fold(1.0, f32::max);
            let by_width = max_w / widest * REF_SIZE;
            let by_height = max_h / (n as f32 * voice.line_height);
            let size = by_width.min(by_height).min(max_size * self.gain(&voice));
            // Prefer fewer lines unless more lines buy a clearly bigger size.
            let better = match &best {
                None => true,
                Some(b) => size > b.size * 1.12,
            };
            if better {
                let widths = lines.iter().map(|l| self.width(&voice, l, size)).collect();
                best = Some(Block {
                    lines,
                    size,
                    line_widths: widths,
                    voice,
                });
            }
        }
        best.unwrap_or_else(|| self.fit_line(voice, text, max_w, max_size))
    }

    /// Break `words` into exactly `n` lines minimizing the widest line.
    fn balance(&self, voice: &Voice, words: &[&str], n: usize) -> Vec<String> {
        let n = n.clamp(1, words.len().max(1));
        let len = words.len();
        let seg = |i: usize, j: usize| words[i..j].join(" ");
        // best[k][i] = (widest, next break) for words[i..] in k lines.
        let mut best = vec![vec![(f32::INFINITY, len); len + 1]; n + 1];
        best[0][len] = (0.0, len);
        for k in 1..=n {
            for i in (0..len).rev() {
                for j in (i + 1)..=len {
                    let (rest, _) = best[k - 1][j];
                    if !rest.is_finite() {
                        continue;
                    }
                    let w = self.width(voice, &seg(i, j), REF_SIZE).max(rest);
                    if w < best[k][i].0 {
                        best[k][i] = (w, j);
                    }
                }
            }
        }
        let mut lines = Vec::with_capacity(n);
        let (mut i, mut k) = (0, n);
        while i < len && k > 0 {
            let j = best[k][i].1;
            lines.push(seg(i, j));
            i = j;
            k -= 1;
        }
        if lines.is_empty() {
            lines.push(words.join(" "));
        }
        lines
    }
}

/// `words` joined, trailing clause punctuation dropped, ending in "…".
fn ellipsized(words: &[&str]) -> String {
    let joined = words.join(" ");
    format!("{}…", joined.trim_end_matches([',', ';', ':', '.']))
}

/// Build a Text layer for a block, top-left at `(x, y)`, anchor as given.
pub fn text_layer(id: String, block: &Block, color: Color, align: TextAlign) -> Layer {
    // Small safety margin so the box never clips glyph overhangs.
    let width = block.width() * 1.02 + 2.0;
    Layer {
        tilt: None,
        z: None,
        id,
        x: 0.0,
        y: 0.0,
        width,
        height: block.height(),
        scale_x: 1.0,
        scale_y: 1.0,
        rotation_degrees: 0.0,
        anchor_x: 0.0,
        anchor_y: 0.0,
        opacity: 1.0,
        z_index: 0,
        visible: true,
        clip: None,
        depth: None,
        layout: None,
        kind: LayerKind::Text(TextStyle {
            text: block.text(),
            font_role: block.voice.role,
            font_size: round2(block.size),
            font_weight: block.voice.weight,
            italic: block.voice.italic,
            color,
            align,
            line_height: block.voice.line_height,
            letter_spacing: block.voice.letter_spacing,
            max_width: None,
            uppercase: false,
            ink: None,
        }),
    }
}

pub fn round2(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}
