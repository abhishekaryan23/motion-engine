//! The mistake ledger: every reported defect, as a hard gate.
//!
//! `assets/qa/mistakes.json` lists the entries (id, reviewer note, detector,
//! fix policy, repair, fixture). This module holds the *static* detectors
//! (over the program and its lowered scene); rendered detectors live with the
//! renderer in `motion-cli::program_cmd`. [`crate::repair`] turns failures
//! back into program edits; the CLI loops QA → repair → QA at most twice and
//! never ships a failing video silently.

use serde::{Deserialize, Serialize};

use crate::program::{Component, ComponentBody, KineticPreset, MotionProgram, Slot, TitlePreset};

/// One ledger entry (`assets/qa/mistakes.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub description: String,
    pub owner_note: String,
    /// The rule that detects it (a static detector id here, a rendered one
    /// in the CLI, or `v1:` for a v1 rule).
    pub detector: String,
    pub fix_policy: String,
    /// `default` (prevented by a compiler default), a repair name, or the
    /// owning session (`design_system`).
    pub repair: String,
    /// `program`, `scene`, `rendered` or `n/a`.
    pub checks: String,
    #[serde(default)]
    pub fixture: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ledger {
    pub version: String,
    pub entries: Vec<Entry>,
}

/// The bundled ledger.
pub fn ledger() -> Ledger {
    serde_json::from_str(include_str!("../../../assets/qa/mistakes.json")).expect("bundled ledger")
}

/// A failed gate: the ledger id and what was seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateFail {
    pub id: String,
    pub detail: String,
}

fn fail(id: &str, detail: impl Into<String>) -> GateFail {
    GateFail {
        id: id.to_string(),
        detail: detail.into(),
    }
}

fn tokens(t: &str) -> Vec<String> {
    t.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

/// Framing lines that talk about the content instead of being it.
const KICKER_PHRASES: [&str; 16] = [
    "in numbers",
    "by the numbers",
    "at a glance",
    "in review",
    "year in",
    "last year",
    "this year",
    "so far",
    "highlights",
    "recap",
    "key stats",
    "key numbers",
    "the numbers",
    "introducing",
    "presenting",
    "meet the",
];

/// A kicker line (owner-banned eyebrow): a framing phrase ("LAST YEAR, IN
/// NUMBERS"), an establishment line ("EST. 2019", "SINCE 1998") or a bare
/// date pill.
pub fn is_kicker(text: &str) -> bool {
    let tk = tokens(text);
    let year = |w: &String| {
        w.len() == 4
            && (w.starts_with("19") || w.starts_with("20"))
            && w.chars().all(|ch| ch.is_ascii_digit())
    };
    let founding = |w: &String| matches!(w.as_str(), "est" | "since" | "founded" | "established");
    // Phrases match whole words ("recap" is not "recapture").
    let phrase = |p: &str| {
        let pt = tokens(p);
        tk.windows(pt.len()).any(|w| w == pt.as_slice())
    };
    KICKER_PHRASES.iter().any(|p| phrase(p))
        || tk.iter().any(founding)
        // A bare date pill: a year alone (or with a founding word).
        || (tk.iter().any(year) && tk.iter().all(|w| year(w) || founding(w)))
}

/// Kicker-prone text a component sets beside its content: a bento's header,
/// a lockup's badge pill and spec lines.
pub fn component_kickers(c: &Component) -> Vec<&str> {
    match &c.body {
        ComponentBody::Bento(b) => b.title.iter().map(String::as_str).collect(),
        ComponentBody::Lockup(l) => l
            .badge
            .iter()
            .chain(l.specs.iter())
            .map(String::as_str)
            .collect(),
        _ => vec![],
    }
}

/// Whether two components are on screen together.
fn concurrent(
    p: &MotionProgram,
    a: &crate::program::Component,
    b: &crate::program::Component,
) -> bool {
    a.start < p.end_of(b) - 0.05 && b.start < p.end_of(a) - 0.05
}

/// Clichés that read as template filler.
const CLICHES: [&str; 4] = ["selected work", "selected works", "showreel", "portfolio"];

/// The static gates over a program and its lowered scene (type measured
/// with the approximate typesetter; see [`static_gates_with`]).
pub fn static_gates(p: &MotionProgram, scene: &motion_core::MotionProject) -> Vec<GateFail> {
    static_gates_with(p, scene, &crate::Typesetter::approx())
}

/// A component's visible leaf boxes at frame `f`, by layer id.
fn visible_boxes(scene: &motion_core::MotionProject, id: &str, f: u32) -> Vec<(String, [f32; 4])> {
    let Ok(rf) = motion_core::evaluate_frame(scene, f) else {
        return Vec::new();
    };
    let mine = |sid: Option<&str>| sid.is_some_and(|s| s == id || s.starts_with(&format!("{id}.")));
    let frame_area = (scene.canvas.width * scene.canvas.height) as f32;
    let mut out = Vec::new();
    let mut stack: Vec<(&motion_core::timeline::ResolvedLayer<'_>, f32)> =
        rf.layers.iter().map(|l| (l, 1.0)).collect();
    while let Some((l, a)) = stack.pop() {
        let op = a * l.opacity;
        stack.extend(l.children.iter().map(|c| (c, op)));
        if !mine(l.scene) || op < 0.05 || !l.children.is_empty() {
            continue;
        }
        // Type still waiting to type on is not shown.
        if l.glyphs
            .as_ref()
            .is_some_and(|g| g.iter().all(|p| p.opacity < 0.05))
        {
            continue;
        }
        let pts = [
            l.transform.apply(0.0, 0.0),
            l.transform.apply(l.width, 0.0),
            l.transform.apply(0.0, l.height),
            l.transform.apply(l.width, l.height),
        ];
        let b = pts
            .iter()
            .fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |q, (x, y)| {
                [q[0].min(*x), q[1].min(*y), q[2].max(*x), q[3].max(*y)]
            });
        let area = (b[2] - b[0]) * (b[3] - b[1]);
        if area <= 1.0 || area >= frame_area * 0.9 {
            continue;
        }
        out.push((l.id.to_string(), b));
    }
    out
}

fn union(boxes: &[[f32; 4]]) -> Option<[f32; 4]> {
    boxes.iter().copied().reduce(|q, b| {
        [
            q[0].min(b[0]),
            q[1].min(b[1]),
            q[2].max(b[2]),
            q[3].max(b[3]),
        ]
    })
}

/// Layout must never anticipate. At each event that brings content in, the
/// content already shown must make room for it (move) unless the arrival
/// neither lands inside the composition's existing extent nor pulls its
/// centre off: content that lands in a gap kept for it, or that extends a
/// composition laid out for it, fails. Measured on the program lowered
/// without camera moves (`opts` as given), so it is independent of framing.
pub fn anticipatory_space(p: &MotionProgram, opts: crate::LowerOptions) -> Vec<GateFail> {
    let opts = crate::LowerOptions {
        camera: false,
        ..opts
    };
    let scene = &crate::lower_with(p, None, &crate::Typesetter::approx(), opts);
    let fps = scene.canvas.fps as f64;
    let short = (scene.canvas.width.min(scene.canvas.height)) as f32;
    let mut out = Vec::new();
    for c in &p.components {
        let end = p.end_of(c);
        let mut times: Vec<f64> = c.timeline.iter().filter_map(|e| e.at).collect();
        times.sort_by(f64::total_cmp);
        times.dedup_by(|a, b| (*a - *b).abs() < 0.05);
        for at in times {
            let (t0, t1) = (
                c.start + at - crate::flip::LEAD - 0.02,
                c.start + at + crate::flip::RELAYOUT + 0.15,
            );
            if t0 < c.start + 0.15 || t1 > end - 0.05 {
                continue;
            }
            let f = |t: f64| (t * fps).round() as u32;
            let before = visible_boxes(scene, &c.id, f(t0));
            let after = visible_boxes(scene, &c.id, f(t1));
            let kept: Vec<(&[f32; 4], &[f32; 4])> = before
                .iter()
                .filter_map(|(id, a)| after.iter().find(|(j, _)| j == id).map(|(_, b)| (a, b)))
                .collect();
            let arrived: Vec<[f32; 4]> = after
                .iter()
                .filter(|(id, _)| !before.iter().any(|(j, _)| j == id))
                .map(|(_, b)| *b)
                .collect();
            let (Some(u0), Some(e)) = (
                union(&kept.iter().map(|(a, _)| **a).collect::<Vec<_>>()),
                union(&arrived),
            ) else {
                continue;
            };
            let ea = (e[2] - e[0]) * (e[3] - e[1]);
            if ea < 0.002 * (scene.canvas.width * scene.canvas.height) as f32 {
                continue;
            }
            let centre = |q: [f32; 4]| [(q[0] + q[2]) / 2.0, (q[1] + q[3]) / 2.0];
            let moved = kept
                .iter()
                .map(|(a, b)| {
                    let (ca, cb) = (centre(**a), centre(**b));
                    (ca[0] - cb[0]).hypot(ca[1] - cb[1])
                })
                .fold(0.0f32, f32::max);
            if moved > 0.01 * short {
                continue;
            }
            // Inside: the arrival lands within the extent already shown.
            let ix = (e[2].min(u0[2]) - e[0].max(u0[0])).max(0.0);
            let iy = (e[3].min(u0[3]) - e[1].max(u0[1])).max(0.0);
            // A label landing on its own shape (a card, a donut's hole) is
            // labelling, not kept room.
            let on_shape = kept.iter().any(|(_, b)| {
                let jx = (e[2].min(b[2]) - e[0].max(b[0])).max(0.0);
                let jy = (e[3].min(b[3]) - e[1].max(b[1])).max(0.0);
                jx * jy / ea > 0.8
            });
            let inside = if on_shape { 0.0 } else { ix * iy / ea };
            // Off-centre: what was shown sat off the composition's centre.
            let u1 = union(&[u0, e]).expect("two boxes");
            let (c0, c1) = (centre(u0), centre(u1));
            let drift = (c0[0] - c1[0]).hypot(c0[1] - c1[1]);
            if inside > 0.5 || drift > 0.03 * short {
                out.push(fail(
                    "anticipatory_space",
                    format!(
                        "{} kept room for what lands at {:.2}s (nothing moved; {:.0}% inside, centre {drift:.0}px off)",
                        c.id,
                        c.start + at,
                        inside * 100.0
                    ),
                ));
            }
        }
    }
    out
}

/// [`static_gates`] with the renderer's typesetter.
pub fn static_gates_with(
    p: &MotionProgram,
    scene: &motion_core::MotionProject,
    ts: &crate::Typesetter,
) -> Vec<GateFail> {
    use motion_core::scene::{LayerKind, MotionOp};
    let _ = ts;
    let mut out = anticipatory_space(p, Default::default());

    for w in crate::qa::prompt_echo(p, scene) {
        out.push(fail("echo_brief", format!("'{w}' copied from the brief")));
    }

    // Captions (type_on kinetic) that repeat labels/values already on screen.
    for c in &p.components {
        let ComponentBody::KineticText {
            preset: KineticPreset::TypeOn,
            words,
            ..
        } = &c.body
        else {
            continue;
        };
        let mut shown: Vec<String> = Vec::new();
        for o in p
            .components
            .iter()
            .filter(|o| o.id != c.id && concurrent(p, c, o))
        {
            match &o.body {
                ComponentBody::Chart {
                    labels, callout, ..
                } => {
                    labels.iter().for_each(|l| shown.extend(tokens(l)));
                    if let Some(t) = callout {
                        shown.extend(tokens(t));
                    }
                }
                ComponentBody::Diagram { nodes, .. } => {
                    nodes.iter().for_each(|l| shown.extend(tokens(l)));
                }
                _ => {}
            }
        }
        for w in words {
            if let Some(t) = tokens(w).into_iter().find(|t| shown.contains(t)) {
                out.push(fail(
                    "echo_labels",
                    format!("caption '{w}' repeats '{t}' already on screen ({})", c.id),
                ));
            }
        }
    }

    for m in crate::qa::meta_strings(p, scene) {
        out.push(fail("meta_text", format!("'{m}'")));
    }
    if let Some(m) = &p.music {
        let track: Vec<String> = tokens(&m.track.replace('_', " "))
            .into_iter()
            .filter(|t| t.len() >= 4)
            .collect();
        for s in &scene.scenes {
            for l in &s.layers {
                if let LayerKind::Text(st) = &l.kind {
                    let tk = tokens(&st.text);
                    if !track.is_empty() && track.iter().all(|t| tk.contains(t)) {
                        out.push(fail("meta_text", format!("track name '{}'", st.text)));
                    }
                }
            }
        }
    }
    for c in &p.components {
        if let ComponentBody::Hud { corners } = &c.body {
            if corners.iter().any(|t| !t.trim().is_empty()) {
                out.push(fail("meta_text", format!("hud corner labels ({})", c.id)));
            }
        }
    }

    // A typing reveal must leave the word whole for most of its shot.
    for s in &scene.scenes {
        for m in &s.motions {
            // Typing/drop reveals (letters fade in), not position snaps.
            if let MotionOp::GlyphCascade { from, .. } = &m.op {
                if from.opacity >= 0.5 {
                    continue;
                }
                if m.duration > 0.5 * s.duration_seconds && m.duration > 0.2 {
                    out.push(fail(
                        "partial_typing",
                        format!(
                            "{} types for {:.2}s of a {:.2}s shot",
                            s.id, m.duration, s.duration_seconds
                        ),
                    ));
                }
            }
        }
    }

    if let Err(e) = crate::validate(p) {
        for i in
            e.0.iter()
                .filter(|i| i.message.contains("slop_meaningless_shape"))
        {
            out.push(fail("meaningless_shape", i.path.clone()));
        }
    }

    // Eyebrows: a small label or chapter above a centred hero; clichés.
    for c in &p.components {
        let small_top = matches!(
            &c.body,
            ComponentBody::Title {
                preset: TitlePreset::Label | TitlePreset::Chapter,
                slot: Slot::Top | Slot::TopLeft,
                ..
            }
        );
        if small_top {
            let hero = p.components.iter().any(|o| {
                o.id != c.id
                    && concurrent(p, c, o)
                    && matches!(
                        &o.body,
                        ComponentBody::Title {
                            preset: TitlePreset::Display
                                | TitlePreset::Stacked
                                | TitlePreset::Serif,
                            slot: Slot::Center,
                            ..
                        }
                    )
            });
            if hero {
                out.push(fail(
                    "eyebrow",
                    format!("label above a hero title ({})", c.id),
                ));
            }
        }
        let texts: Vec<&str> = match &c.body {
            ComponentBody::Title { text, .. } => vec![text.as_str()],
            ComponentBody::KineticText { words, .. } => words.iter().map(String::as_str).collect(),
            _ => vec![],
        };
        let lower: Vec<String> = texts.iter().map(|t| t.to_lowercase()).collect();
        if lower.iter().any(|t| CLICHES.iter().any(|c| t.contains(c))) {
            out.push(fail("eyebrow", format!("cliché text ({})", c.id)));
        }
        if lower.len() >= 3 {
            let countdown = lower
                .windows(3)
                .any(|w| w[0] == "3" && w[1] == "2" && w[2] == "1");
            if countdown {
                out.push(fail("eyebrow", format!("3-2-1 countdown ({})", c.id)));
            }
        }
        if lower.iter().any(|t| {
            let tk = tokens(t);
            tk.iter().any(|w| {
                w.len() == 4 && w.starts_with("20") && w.chars().all(|ch| ch.is_ascii_digit())
            }) && tk.iter().any(|w| w == "designer")
        }) {
            out.push(fail("eyebrow", format!("kicker line ({})", c.id)));
        }
        // Kickers inside a component: a bento header, a lockup pill or spec.
        if let Some(k) = component_kickers(c).into_iter().find(|t| is_kicker(t)) {
            out.push(fail("eyebrow_kicker", format!("kicker '{k}' ({})", c.id)));
        }
    }

    // Boxes behind type that are not the content, pulses, unsettled counts.
    for s in &scene.scenes {
        let mut seen_assets = std::collections::BTreeSet::new();
        let mut reported_assets = std::collections::BTreeSet::new();
        for l in &s.layers {
            if let LayerKind::Image { asset, .. } = &l.kind {
                if !seen_assets.insert(asset.as_str()) && reported_assets.insert(asset.as_str()) {
                    out.push(fail(
                        "duplicate_cutout_in_shot",
                        format!("{} repeats {} in the same shot", s.id, asset),
                    ));
                }
            }
            if let LayerKind::Text(_) = &l.kind {
                if let Some(owner) = crate::qa::backing_owner(scene, s, l) {
                    let mine = owner == s.id || owner.split('.').next() == s.id.split('.').next();
                    if !mine {
                        out.push(fail(
                            "box_behind_type",
                            format!("{} sits on a box from {owner}", l.id),
                        ));
                    }
                }
            }
        }
        for m in &s.motions {
            match &m.op {
                MotionOp::Pulse { .. } | MotionOp::Shake { .. } => {
                    out.push(fail(
                        "pulse_motion",
                        format!("{} {}", m.target, m.op.op_name()),
                    ));
                }
                MotionOp::Count { .. } if m.duration > 0.45 => out.push(fail(
                    "count_unsettled",
                    format!("{} counts for {:.2}s", m.target, m.duration),
                )),
                _ => {}
            }
        }
    }

    for (id, ratio) in crate::qa::hairlines(scene) {
        out.push(fail("hairline", format!("{id} {ratio:.2}:1")));
    }
    out.extend(crate::ui_gates::gates(p, scene));
    out
}
