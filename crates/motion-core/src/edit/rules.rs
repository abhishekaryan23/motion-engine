//! The deterministic rule planner: analysis -> EditPlan (E1 overlay kinds).

use std::collections::{BTreeMap, BTreeSet};

use super::analyze::{is_stopword, Analysis, EntityKind, Tok};
use super::cutaway::{fill_quiet, plan_cutaways, CutInput};
use super::layout::{face_union, zone};
use super::plan::{EditEvent, EditPlan, EventKind, FaceSeg, Side, EDIT_PLAN_VERSION};

/// Seconds between the starts of two number callouts.
pub const CALLOUT_GAP: f64 = 2.5;
/// At most one keyword per this many seconds.
pub const KEYWORD_WINDOW: f64 = 20.0;
/// At most one chapter card per this many seconds.
pub const CHAPTER_SPACING: f64 = 90.0;
/// Overlay text appears this long after its word starts (spec: 0..150 ms).
pub const TEXT_LAG: f64 = 0.05;
/// Overlays hold this long after their phrase ends.
pub const HOLD_AFTER: f64 = 0.8;
/// Seconds between two punch-in starts.
pub const PUNCH_SPACING: f64 = 20.0;
/// Clear time kept between two overlays.
const OVERLAY_GAP: f64 = 0.3;

pub struct PlanInput<'a> {
    pub toks: &'a [Tok],
    pub analysis: &'a Analysis,
    pub face_boxes: Vec<FaceSeg>,
    pub canvas: [u32; 2],
    pub fps: u32,
    pub duration: f64,
    pub source: &'a str,
    pub language: &'a str,
    pub lower_third_text: &'a str,
}

struct Builder<'a> {
    inp: &'a PlanInput<'a>,
    events: Vec<EditEvent>,
}

impl Builder<'_> {
    fn free(&self, start: f64, end: f64) -> bool {
        self.events.iter().all(|e| {
            matches!(e.kind, EventKind::Captions {} | EventKind::PunchIn {})
                || e.end + OVERLAY_GAP <= start
                || e.start >= end + OVERLAY_GAP
        })
    }

    /// The side with the wider free zone beside the face at `[t0, t1]`;
    /// alternates when both are comfortably wide.
    fn side(&self, t0: f64, t1: f64, k: usize) -> Side {
        let canvas = (self.inp.canvas[0] as f32, self.inp.canvas[1] as f32);
        let face = face_union(&self.inp.face_boxes, canvas, t0, t1);
        let width = |s| zone(canvas, face, s).map(|(a, b)| b - a).unwrap_or(0.0);
        let (l, r) = (width(Side::Left), width(Side::Right));
        if l >= 420.0 && r >= 420.0 {
            if k.is_multiple_of(2) {
                Side::Left
            } else {
                Side::Right
            }
        } else if r > l {
            Side::Right
        } else {
            Side::Left
        }
    }

    fn push(
        &mut self,
        kind: EventKind,
        at: usize,
        until: usize,
        start: f64,
        end: f64,
        side: Side,
    ) -> bool {
        let end = end.min(self.inp.duration);
        if end <= start || !self.free(start, end) {
            return false;
        }
        self.events.push(EditEvent {
            kind,
            at_word: at,
            until_word: until,
            start,
            end,
            side,
        });
        true
    }
}

/// Caption chunks `(first, last)` word indices: <= 5 words and 32 characters,
/// broken at sentence ends and pauses of 0.35 s or more.
pub fn caption_chunks(toks: &[Tok]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let (mut a, mut chars) = (0usize, 0usize);
    for i in 0..toks.len() {
        chars += toks[i].raw.chars().count() + 1;
        let next = toks.get(i + 1);
        let words = i + 1 - a;
        let brk = next.is_none()
            || words >= 5
            || chars > 32
            || toks[i].raw.ends_with(['.', '!', '?'])
            || next.is_some_and(|n| n.start - toks[i].end >= 0.35)
            || (words >= 3 && toks[i].raw.ends_with([',', ';', ':']));
        if brk {
            out.push((a, i));
            a = i + 1;
            chars = 0;
        }
    }
    out
}

fn content_words(toks: &[Tok], a: usize, b: usize) -> BTreeSet<String> {
    toks[a..b.min(toks.len())]
        .iter()
        .filter(|t| !is_stopword(&t.norm) && t.norm.chars().all(|c| c.is_alphabetic()))
        .map(|t| t.norm.clone())
        .collect()
}

/// Topic-shift candidates: `(word index after the pause, shift 0..1)`.
fn chapter_candidates(inp: &PlanInput) -> Vec<(usize, f64)> {
    const SPAN: usize = 60;
    let toks = inp.toks;
    let mut out = Vec::new();
    for p in inp
        .analysis
        .pauses
        .iter()
        .filter(|p| p.end - p.start >= 0.7)
    {
        let at = p.after + 1;
        if at < SPAN || at + SPAN > toks.len() {
            continue;
        }
        let before = content_words(toks, at - SPAN, at);
        let after = content_words(toks, at, at + SPAN);
        let inter = before.intersection(&after).count() as f64;
        let union = before.union(&after).count().max(1) as f64;
        out.push((at, 1.0 - inter / union));
    }
    out
}

fn chapter_title(toks: &[Tok], at: usize) -> String {
    let mut freq: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for (k, t) in toks[at..(at + 60).min(toks.len())].iter().enumerate() {
        if !is_stopword(&t.norm) && t.norm.chars().all(|c| c.is_alphabetic()) && t.norm.len() > 3 {
            let e = freq.entry(t.norm.as_str()).or_insert((0, k));
            e.0 += 1;
        }
    }
    let mut v: Vec<_> = freq.into_iter().collect();
    v.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(a.1 .1.cmp(&b.1 .1)));
    let mut top: Vec<(usize, &str)> = v.iter().take(2).map(|(w, (_, k))| (*k, *w)).collect();
    top.sort();
    let t = top
        .iter()
        .map(|(_, w)| w.to_uppercase())
        .collect::<Vec<_>>()
        .join(" ");
    if t.is_empty() {
        "NEXT".into()
    } else {
        t
    }
}

/// Digital reframes (1.00 -> 1.12 on the face) on the strongest stresses.
fn punch_ins(b: &mut Builder) {
    let inp = b.inp;
    let toks = inp.toks;
    let f = inp.fps as f64;
    let q = |t: f64| (t * f).round() / f;
    let mut stresses: Vec<(usize, f64)> = inp
        .analysis
        .stresses
        .iter()
        .copied()
        .filter(|&(w, _)| !is_stopword(&toks[w].norm) && toks[w].start >= 6.0)
        .collect();
    stresses.sort_by(|a, c| {
        c.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&c.0))
    });
    let mut starts: Vec<f64> = Vec::new();
    for (w, _) in stresses {
        let s = q((toks[w].start - 0.15).max(0.0));
        if starts.iter().any(|x| (x - s).abs() < PUNCH_SPACING) {
            continue;
        }
        // Back to full frame on the first pause 1.8..3.6 s later.
        let end = inp
            .analysis
            .pauses
            .iter()
            .filter(|p| p.start >= s + 1.8 && p.start <= s + 3.6)
            .map(|p| (p.start + p.end) / 2.0)
            .next()
            .unwrap_or(s + 2.4);
        let end = q(end).min(inp.duration);
        if end <= s + 1.0 || !b.free(s - 0.4, end + 0.4) {
            continue;
        }
        starts.push(s);
        b.events.push(EditEvent {
            kind: EventKind::PunchIn {},
            at_word: w,
            until_word: w,
            start: s,
            end,
            side: Side::Left,
        });
    }
}

pub fn plan_edit(inp: &PlanInput) -> EditPlan {
    let toks = inp.toks;
    let n = toks.len();
    let mut b = Builder {
        inp,
        events: Vec::new(),
    };
    if n > 0 {
        // 1. lower third, once, at the start.
        let start = toks[0].start.max(0.8);
        let at = toks.iter().position(|t| t.start >= start).unwrap_or(0);
        b.push(
            EventKind::LowerThird {
                text: inp.lower_third_text.to_string(),
            },
            at,
            at,
            start,
            start + 4.0,
            Side::Left,
        );

        // 1b. cutaways and number stacks own the screen first; the overlays
        // below fit around them.
        let cuts = plan_cutaways(&CutInput {
            toks,
            analysis: inp.analysis,
            fps: inp.fps,
            duration: inp.duration,
            earliest: (start + 4.0).max(8.0),
        });
        b.events.extend(cuts.cutaways);
        for p in cuts.panels {
            if b.free(p.start, p.end) {
                b.events.push(p);
            }
        }

        // 2. chapters (<= 1 per 90 s, none in the first 45 s).
        let mut cands = chapter_candidates(inp);
        cands.retain(|(at, shift)| *shift >= 0.8 && toks[*at].start >= 45.0);
        cands.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        let mut chosen: Vec<usize> = Vec::new();
        for (at, _) in cands {
            if chosen
                .iter()
                .all(|c| (toks[*c].start - toks[at].start).abs() >= CHAPTER_SPACING)
            {
                chosen.push(at);
            }
        }
        chosen.sort();
        for (i, at) in chosen.into_iter().enumerate() {
            let s = toks[at].start;
            let side = b.side(s, s + 2.6, i);
            b.push(
                EventKind::ChapterCard {
                    title: chapter_title(toks, at),
                    index: i + 1,
                },
                at,
                at,
                s,
                s + 2.6,
                side,
            );
        }

        // 3. number callouts on spoken numbers.
        let mut last_start = f64::NEG_INFINITY;
        let mut k = 0;
        for e in inp
            .analysis
            .entities
            .iter()
            .filter(|e| e.kind == EntityKind::Number)
        {
            let s = toks[e.first].start + TEXT_LAG;
            if s - last_start < CALLOUT_GAP {
                continue;
            }
            let end = (toks[e.last].end + HOLD_AFTER).max(s + 2.0).min(s + 4.5);
            let side = b.side(s, end, k);
            if b.push(
                EventKind::NumberCallout {
                    value: e.text.clone(),
                },
                e.first,
                e.last,
                s,
                end,
                side,
            ) {
                last_start = s;
                k += 1;
            }
        }

        // 4. keywords: the stressed content word of a sentence with a claim entity.
        let claim_in = |a: usize, z: usize| {
            inp.analysis.entities.iter().any(|e| {
                matches!(
                    e.kind,
                    EntityKind::Number
                        | EntityKind::Comparison
                        | EntityKind::ListCue
                        | EntityKind::Proper
                ) && e.first >= a
                    && e.last <= z
            })
        };
        let in_entity = |i: usize| {
            inp.analysis
                .entities
                .iter()
                .any(|e| e.kind == EntityKind::Number && i >= e.first && i <= e.last)
        };
        let mut best: BTreeMap<u64, (usize, f64)> = BTreeMap::new();
        for &(w, score) in &inp.analysis.stresses {
            if is_stopword(&toks[w].norm)
                || in_entity(w)
                || !toks[w].norm.chars().all(|c| c.is_alphabetic())
            {
                continue;
            }
            let Some(&(a, z)) = inp
                .analysis
                .sentences
                .iter()
                .find(|(a, z)| w >= *a && w <= *z)
            else {
                continue;
            };
            if !claim_in(a, z) {
                continue;
            }
            let bucket = (toks[w].start / KEYWORD_WINDOW) as u64;
            let cur = best.entry(bucket).or_insert((w, score));
            if score > cur.1 {
                *cur = (w, score);
            }
        }
        let mut kk = 0;
        for (_, (w, _)) in best {
            let s = toks[w].start + TEXT_LAG;
            let side = b.side(s, s + 1.8, kk + 1);
            if b.push(
                EventKind::Keyword {
                    text: toks[w].norm.to_uppercase(),
                },
                w,
                w,
                s,
                s + 1.8,
                side,
            ) {
                kk += 1;
            }
        }

        // 4b. a key-idea panel wherever 25 s would pass with nothing on screen.
        let busy: Vec<(f64, f64)> = b
            .events
            .iter()
            .filter(|e| !matches!(e.kind, EventKind::Captions {} | EventKind::PunchIn {}))
            .map(|e| (e.start, e.end))
            .collect();
        for p in fill_quiet(toks, inp.analysis, inp.fps, inp.duration, &busy) {
            let side = b.side(p.start, p.end, p.at_word);
            let mut p = p;
            p.side = side;
            if b.free(p.start, p.end) {
                b.events.push(p);
            }
        }

        // 4c. punch-ins on stresses (<= 1 per 20 s), clear of every other event.
        punch_ins(&mut b);

        // 5. captions for the whole talk.
        b.events.push(EditEvent {
            kind: EventKind::Captions {},
            at_word: 0,
            until_word: n - 1,
            start: 0.0,
            end: inp.duration,
            side: Side::Left,
        });
    }
    b.events.sort_by(|x, y| {
        x.start
            .partial_cmp(&y.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    EditPlan {
        version: EDIT_PLAN_VERSION.to_string(),
        source: inp.source.to_string(),
        canvas: inp.canvas,
        fps: inp.fps,
        duration: inp.duration,
        language: inp.language.to_string(),
        words: n,
        face_boxes: inp.face_boxes.clone(),
        events: b.events,
    }
}

#[cfg(test)]
mod tests {
    use super::super::analyze::analyse;
    use super::*;

    fn talk() -> Vec<Tok> {
        let text =
            "Hello everyone and welcome back today we talk about trade. The deficit was ₹40 lakh crore last year. \
                    Growth fell to 3.5% in March. Compare 2016 vs 2026 for me.";
        text.split_whitespace()
            .enumerate()
            .map(|(i, w)| Tok::new(w, 1.0 + i as f64 * 0.4, 1.0 + i as f64 * 0.4 + 0.3))
            .collect()
    }

    fn run() -> EditPlan {
        let toks = talk();
        let a = analyse(&toks, &[]);
        plan_edit(&PlanInput {
            toks: &toks,
            analysis: &a,
            face_boxes: vec![],
            canvas: [1920, 1080],
            fps: 30,
            duration: 20.0,
            source: "t.mp4",
            language: "en",
            lower_third_text: "—",
        })
    }

    #[test]
    fn plan_is_deterministic_and_valid() {
        let (a, b) = (run(), run());
        assert_eq!(a, b);
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
        assert!(a.problems().is_empty(), "{:?}", a.problems());
        let back = EditPlan::from_json(&serde_json::to_string_pretty(&a).unwrap()).unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn callouts_sit_on_their_numbers() {
        let p = run();
        let toks = talk();
        // A number is a callout, a number stack or a figure cutaway.
        let calls: Vec<_> = p
            .events
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    EventKind::NumberCallout { .. }
                        | EventKind::SidePanel { .. }
                        | EventKind::Cutaway { .. }
                )
            })
            .collect();
        assert!(!calls.is_empty(), "{calls:?}");
        for c in calls {
            assert!((c.start - toks[c.at_word].start).abs() <= 0.35);
        }
        assert_eq!(p.count("lower_third"), 1);
        assert_eq!(p.count("captions"), 1);
    }

    #[test]
    fn captions_chunk_on_limits() {
        let toks = talk();
        let ch = caption_chunks(&toks);
        assert_eq!(ch.first().unwrap().0, 0);
        assert_eq!(ch.last().unwrap().1, toks.len() - 1);
        assert!(ch.iter().all(|(a, z)| z - a < 5));
        assert!(ch.windows(2).all(|w| w[0].1 + 1 == w[1].0));
    }
}
