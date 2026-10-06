//! (0.10) Caption QA: do the word-synced captions of a compiled project match
//! the speech and fit the canvas? Pure and deterministic.
//!
//! [`caption_report`] checks the caption scene (`captions`, built by
//! `compiler/captions.rs`) against the [`SpeechMap`] it was built from:
//!
//! | check | rule |
//! |---|---|
//! | `scene_missing` | a speech with words has a caption scene |
//! | `word_missing` | every spoken word has a text layer `cap.w{i}` |
//! | `word_timing` | the layer's fade-in starts within 1 frame (`1 / fps`) of the word start |
//! | `outside_safe` | every caption text box lies inside the safe area (box padding tolerated) |
//! | `too_many_lines` | a page has at most [`MAX_LINES`] lines |
//! | `line_too_long` | a line has at most [`MAX_CHARS_PER_LINE`] characters |
//!
//! Layout QA (`layout_report`) also samples the caption scene at every page's
//! settled time ([`sample_times`]).

use serde::{Deserialize, Serialize};

use crate::compiler::captions::{
    word_layer_id, CAPTION_LAYER_PREFIX, CAPTION_SCENE_ID, MAX_CHARS_PER_LINE, MAX_LINES,
};
use crate::compiler::layout_frame::LayoutFrame;
use crate::layout_qa::LayoutVerdict;
use crate::scene::{Layer, LayerKind, MotionOp, MotionProject, Scene};
use crate::speech::SpeechMap;

/// Seconds after a page's last word starts at which the page is settled.
const SETTLE_SECONDS: f64 = 0.14;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptionCheck {
    SceneMissing,
    WordMissing,
    WordTiming,
    OutsideSafe,
    TooManyLines,
    LineTooLong,
}

impl CaptionCheck {
    pub fn name(&self) -> &'static str {
        match self {
            CaptionCheck::SceneMissing => "scene_missing",
            CaptionCheck::WordMissing => "word_missing",
            CaptionCheck::WordTiming => "word_timing",
            CaptionCheck::OutsideSafe => "outside_safe",
            CaptionCheck::TooManyLines => "too_many_lines",
            CaptionCheck::LineTooLong => "line_too_long",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptionFinding {
    pub check: CaptionCheck,
    /// Layer or page id (empty for scene-level findings).
    pub layer: String,
    pub detail: String,
}

/// Timing of one spoken word's caption.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WordTiming {
    /// Index in `SpeechMap::words`.
    pub index: usize,
    pub text: String,
    /// The spoken start (seconds).
    pub start: f64,
    /// When the caption word starts appearing (seconds); `None` = no layer.
    pub appear: Option<f64>,
    /// `|appear - start|` in seconds.
    pub delta: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptionReport {
    pub verdict: LayoutVerdict,
    pub words: Vec<WordTiming>,
    /// Largest `|appear - start|` over the words (seconds).
    pub max_delta: f64,
    /// Most lines on any page.
    pub max_lines: usize,
    /// Most characters on any line.
    pub max_line_chars: usize,
    pub findings: Vec<CaptionFinding>,
}

impl CaptionReport {
    pub fn passed(&self) -> bool {
        self.verdict == LayoutVerdict::Pass
    }

    /// Human-readable text (one line per finding).
    pub fn to_text(&self) -> String {
        let mut out = String::from("captions\n");
        for f in &self.findings {
            out.push_str(&format!(
                "  FAIL {} {}: {}\n",
                f.layer,
                f.check.name(),
                f.detail
            ));
        }
        out.push_str(&format!(
            "captions: {} ({} word(s), max timing error {:.3}s, max {} line(s), max {} char(s)/line, {} finding(s))\n",
            if self.passed() { "PASS" } else { "FAIL" },
            self.words.len(),
            self.max_delta,
            self.max_lines,
            self.max_line_chars,
            self.findings.len()
        ));
        out
    }
}

fn find_layer<'a>(layers: &'a [Layer], id: &str) -> Option<&'a Layer> {
    for l in layers {
        if l.id == id {
            return Some(l);
        }
        if let LayerKind::Group { children } = &l.kind {
            if let Some(found) = find_layer(children, id) {
                return Some(found);
            }
        }
    }
    None
}

/// Start of the fade-in (0 -> 1) of layer `id`, absolute seconds.
fn appear_time(scene: &Scene, id: &str) -> Option<f64> {
    scene
        .motions
        .iter()
        .filter(|m| m.target == id)
        .filter(|m| matches!(m.op, MotionOp::Fade { from, to } if from < to))
        .map(|m| scene.start_seconds + m.start)
        .fold(None, |acc: Option<f64>, t| {
            Some(acc.map_or(t, |a| a.min(t)))
        })
}

/// The caption scene of `project`, if any.
pub fn caption_scene_of(project: &MotionProject) -> Option<&Scene> {
    project.scenes.iter().find(|s| s.id == CAPTION_SCENE_ID)
}

/// Absolute times at which every caption page is fully shown (its last word
/// has settled, the page has not started to leave). Used by layout QA.
pub fn sample_times(scene: &Scene) -> Vec<f64> {
    let mut out = Vec::new();
    for page in &scene.layers {
        let LayerKind::Group { children } = &page.kind else {
            continue;
        };
        let word_ids: Vec<&str> = children
            .iter()
            .filter(|c| matches!(c.kind, LayerKind::Text(_)))
            .map(|c| c.id.as_str())
            .collect();
        let starts: Vec<f64> = word_ids
            .iter()
            .filter_map(|id| appear_time(scene, id))
            .collect();
        let Some(last) = starts.iter().copied().reduce(f64::max) else {
            continue;
        };
        let first = starts.iter().copied().fold(f64::MAX, f64::min);
        let exit = scene
            .motions
            .iter()
            .filter(|m| {
                m.target == page.id && matches!(m.op, MotionOp::Fade { from, to } if from > to)
            })
            .map(|m| m.start)
            .fold(f64::MAX, f64::min);
        let t = (last + SETTLE_SECONDS).min(exit - 0.005).max(first);
        out.push(scene.start_seconds + t);
    }
    out
}

/// Check the project's caption scene against `speech`.
pub fn caption_report(project: &MotionProject, speech: &SpeechMap) -> CaptionReport {
    let mut findings: Vec<CaptionFinding> = Vec::new();
    let mut words: Vec<WordTiming> = Vec::new();
    let (mut max_delta, mut max_lines, mut max_line_chars) = (0.0_f64, 0usize, 0usize);
    let finish = |findings: Vec<CaptionFinding>,
                  words: Vec<WordTiming>,
                  max_delta: f64,
                  max_lines: usize,
                  max_line_chars: usize| CaptionReport {
        verdict: if findings.is_empty() {
            LayoutVerdict::Pass
        } else {
            LayoutVerdict::Fail
        },
        words,
        max_delta,
        max_lines,
        max_line_chars,
        findings,
    };
    let Some(scene) = caption_scene_of(project) else {
        if !speech.words.is_empty() {
            findings.push(CaptionFinding {
                check: CaptionCheck::SceneMissing,
                layer: String::new(),
                detail: "the project has no caption scene".to_string(),
            });
        }
        return finish(findings, words, max_delta, max_lines, max_line_chars);
    };
    let fps = project.canvas.fps.max(1) as f64;
    let frame_tol = 1.0 / fps + 1e-6;

    // Per-word timing.
    for (index, w) in speech.words.iter().enumerate() {
        let id = word_layer_id(index);
        let present = find_layer(&scene.layers, &id).is_some();
        let appear = present.then(|| appear_time(scene, &id)).flatten();
        let delta = appear.map(|a| (a - w.start).abs());
        match (present, delta) {
            (false, _) => findings.push(CaptionFinding {
                check: CaptionCheck::WordMissing,
                layer: id.clone(),
                detail: format!(
                    "word {index} '{}' ({:.3}s) has no caption layer",
                    w.text, w.start
                ),
            }),
            (true, None) => findings.push(CaptionFinding {
                check: CaptionCheck::WordTiming,
                layer: id.clone(),
                detail: format!("word {index} '{}' has no fade-in", w.text),
            }),
            (true, Some(d)) => {
                max_delta = max_delta.max(d);
                if d > frame_tol {
                    findings.push(CaptionFinding {
                        check: CaptionCheck::WordTiming,
                        layer: id.clone(),
                        detail: format!(
                            "word {index} '{}' appears at {:.3}s, spoken at {:.3}s (off by {:.3}s > 1 frame)",
                            w.text,
                            appear.unwrap_or_default(),
                            w.start,
                            d
                        ),
                    });
                }
            }
        }
        words.push(WordTiming {
            index,
            text: w.text.clone(),
            start: w.start,
            appear,
            delta,
        });
    }

    // Geometry per page.
    let frame = LayoutFrame::new(project.canvas.width, project.canvas.height).ok();
    for page in &scene.layers {
        if !page.id.starts_with(CAPTION_LAYER_PREFIX) {
            continue;
        }
        let LayerKind::Group { children } = &page.kind else {
            continue;
        };
        let texts: Vec<&Layer> = children
            .iter()
            // `.on` layers are the active-word copies drawn over a word.
            .filter(|c| matches!(c.kind, LayerKind::Text(_)) && !c.id.ends_with(".on"))
            .collect();
        // Lines: cluster the word boxes by vertical centre.
        let mut ordered: Vec<(f32, f32, &Layer)> = texts
            .iter()
            .map(|l| (l.y + l.height / 2.0, l.x, *l))
            .collect();
        ordered.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        let mut lines: Vec<(f32, Vec<&Layer>)> = Vec::new();
        for (cy, _, l) in ordered {
            match lines.last_mut() {
                Some((c, members)) if (cy - *c).abs() < 1.0 => members.push(l),
                _ => lines.push((cy, vec![l])),
            }
        }
        max_lines = max_lines.max(lines.len());
        if lines.len() > MAX_LINES {
            findings.push(CaptionFinding {
                check: CaptionCheck::TooManyLines,
                layer: page.id.clone(),
                detail: format!("{} lines > {MAX_LINES}", lines.len()),
            });
        }
        for (_, members) in &lines {
            let mut members = members.clone();
            members.sort_by(|a, b| a.x.total_cmp(&b.x));
            let chars = members
                .iter()
                .map(|l| match &l.kind {
                    LayerKind::Text(t) => t.text.chars().count(),
                    _ => 0,
                })
                .sum::<usize>()
                + members.len().saturating_sub(1);
            max_line_chars = max_line_chars.max(chars);
            if chars > MAX_CHARS_PER_LINE {
                findings.push(CaptionFinding {
                    check: CaptionCheck::LineTooLong,
                    layer: page.id.clone(),
                    detail: format!("{chars} chars > {MAX_CHARS_PER_LINE}"),
                });
            }
        }
        if let Some(frame) = &frame {
            let sf = frame.safe;
            for l in &texts {
                // Same tolerance as layout QA: 2u plus the 2% + 2px box padding.
                let tol = 2.0 * frame.u + 0.02 * l.width + 2.0;
                if l.x < sf.x - tol
                    || l.y < sf.y - tol
                    || l.x + l.width > sf.x + sf.w + tol
                    || l.y + l.height > sf.y + sf.h + tol
                {
                    findings.push(CaptionFinding {
                        check: CaptionCheck::OutsideSafe,
                        layer: l.id.clone(),
                        detail: format!(
                            "box x {:.0}..{:.0} y {:.0}..{:.0} vs safe x {:.0}..{:.0} y {:.0}..{:.0}",
                            l.x,
                            l.x + l.width,
                            l.y,
                            l.y + l.height,
                            sf.x,
                            sf.x + sf.w,
                            sf.y,
                            sf.y + sf.h
                        ),
                    });
                }
            }
        }
    }
    finish(findings, words, max_delta, max_lines, max_line_chars)
}
