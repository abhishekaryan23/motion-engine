//! The bar plan: content (or, for pure direction, a concept bank) laid on the
//! music's beat grid, one technique per bar, then realised as a program whose
//! every beat carries a visible event.
//!
//! Gallery defaults baked in here (V2_REVIEW §3): R2 an event on every beat
//! from a downbeat; R3 hook bar, one technique per bar, peak, end title on the
//! final downbeat; R4 nothing holds longer than a beat (an end-title hold of
//! one beat); R5 one accent dot carried through the type; R6 springs only, no
//! bounce; R10/R11 one accent, one display face; R12 no crossfade exits (cuts
//! on the beat), no free decoration.

use motion_program::program::Aspect;
use motion_program::MotionProgram;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{clip, Content, MakeRecipe, Section};

/// The beat grid in program seconds; beat 0 is a downbeat at `t ≈ 0`.
#[derive(Debug, Clone, PartialEq)]
pub struct Clock {
    /// Seconds into the track where the program starts (a downbeat).
    pub track_start: f64,
    /// Beat times, program seconds, from beat 0.
    pub beats: Vec<f64>,
}

impl Clock {
    /// A steady grid (tests, or a track without analysis).
    pub fn steady(bpm: f64, beats: usize) -> Clock {
        Clock {
            track_start: 0.0,
            beats: (0..beats).map(|i| i as f64 * 60.0 / bpm).collect(),
        }
    }

    pub fn beat_len(&self) -> f64 {
        match self.beats.as_slice() {
            [a, b, ..] => b - a,
            _ => 0.5,
        }
    }

    fn at(&self, beat: usize) -> f64 {
        self.beats
            .get(beat)
            .copied()
            .unwrap_or_else(|| self.beats.last().copied().unwrap_or(0.0))
    }
}

/// What one bar does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Technique {
    /// Bar 1: big poster words cut on every beat, the dot carried.
    Hook,
    /// Body words, one per beat.
    Words,
    /// One hero shape changing on every beat.
    Morph,
    /// A chart that draws, then pops its hero; caption words on the beats.
    Chart,
    /// A diagram that reveals one node per beat.
    Diagram,
    /// The biggest bar: poster words with ground flips.
    Peak,
    /// A product / UI flow (`ui_flow`): its steps land on the beats.
    Flow,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Bar {
    pub technique: Technique,
    /// Beats this bar lasts (1..=4; charts 6, morph bars up to 7).
    pub beats: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stress: Vec<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chart: Option<ChartContent>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChartContent {
    pub kind: &'static str,
    pub values: Vec<f64>,
    pub labels: Vec<String>,
    pub callout: Option<String>,
    /// The shot's claim headline, held beside the chart.
    pub claim: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReelPlan {
    pub concept: String,
    pub bars: Vec<Bar>,
    pub end_title: String,
}

impl ReelPlan {
    /// Beats from the program start to the end title's final downbeat.
    pub fn final_downbeat(&self) -> usize {
        let body: usize = self.bars.iter().map(|b| b.beats).sum();
        body + 2
    }
}

/// A concept for pure-direction briefs (no content given).
struct Concept {
    name: &'static str,
    hook: [&'static str; 4],
    words: [&'static str; 4],
    peak: [&'static str; 4],
    nodes: [&'static str; 4],
    chart: (
        &'static str,
        &'static [f64],
        &'static str,
        [&'static str; 4],
    ),
    end: &'static str,
}

/// Our own concept bank (motion craft). Words a brief used as direction are
/// filtered out by picking the next concept that avoids them.
const BANK: &[Concept] = &[
    Concept {
        name: "timing",
        hook: ["EVERY", "FRAME", "EARNS", "IT"],
        words: ["EASE", "IN", "HOLD", "SNAP"],
        peak: ["TIMING", "IS", "THE", "TASTE"],
        nodes: ["IDEA", "SKETCH", "SPRING", "SHIP"],
        chart: (
            "spring",
            &[0.0, 100.0],
            "0.4 S",
            ["SETTLED", "IN", "A", "BEAT"],
        ),
        end: "Frame by frame",
    },
    Concept {
        name: "rhythm",
        hook: ["CUT", "ON", "THE", "ONE"],
        words: ["LAND", "EACH", "HIT", "CLEAN"],
        peak: ["FEEL", "THE", "GRID", "BREATHE"],
        nodes: ["BEAT", "BAR", "PHRASE", "DROP"],
        chart: (
            "bar",
            &[9.0, 13.0, 18.0, 26.0],
            "2.9×",
            ["EVERY", "BAR", "HARDER", ""],
        ),
        end: "On the beat",
    },
    Concept {
        name: "form",
        hook: ["SHAPE", "BECOMES", "SIGNAL", "FAST"],
        words: ["SQUARE", "TO", "CIRCLE", "TO"],
        peak: ["ONE", "DOT", "CARRIES", "IT"],
        nodes: ["POINT", "LINE", "PLANE", "SPACE"],
        chart: (
            "line",
            &[3.0, 5.0, 4.0, 8.0, 12.0],
            "4×",
            ["FOUR", "TIMES", "LOUDER", ""],
        ),
        end: "Form in time",
    },
];

const SHAPES: [&str; 6] = ["square", "circle", "triangle", "diamond", "blob", "star"];

fn bar(technique: Technique, words: Vec<String>) -> Bar {
    Bar {
        technique,
        beats: words.len().clamp(1, 4),
        words,
        stress: Vec::new(),
        chart: None,
    }
}

fn owned(w: &[&str]) -> Vec<String> {
    w.iter().map(|s| s.to_string()).collect()
}

fn concept_for(c: &Content, variant: u64) -> &'static Concept {
    let n = BANK.len();
    let start = ((c.seed ^ variant.wrapping_mul(0x9e37_79b9)) % n as u64) as usize;
    let clean = |k: &Concept| {
        let all: Vec<&str> = k
            .hook
            .iter()
            .chain(&k.words)
            .chain(&k.peak)
            .chain(&k.nodes)
            .chain(&k.chart.3)
            .copied()
            .chain(k.end.split(' '))
            .collect();
        !all.iter()
            .any(|w| c.direction_words.contains(&w.to_lowercase()))
    };
    (0..n)
        .map(|i| &BANK[(start + i) % n])
        .find(|k| clean(k))
        .unwrap_or(&BANK[start])
}

/// Bars per second of target length.
fn target_bars(clock: &Clock, seconds: f64) -> usize {
    ((seconds - clock.beat_len() * 2.0) / (clock.beat_len() * 4.0))
        .round()
        .max(3.0) as usize
}

/// Lay the content on the grid. `duration` (or the brief's) sets the target
/// length; content always fits (the video grows to hold it, up to 120 s).
pub fn plan(
    c: &Content,
    r: &MakeRecipe,
    clock: &Clock,
    duration: Option<f64>,
    variant: u64,
) -> ReelPlan {
    let k = concept_for(c, variant);
    let mut content: Vec<Bar> = Vec::new();
    // `ends[i]`: content bar `i` closes its section (fillers go only there).
    let mut ends: Vec<bool> = Vec::new();
    let mut nodes: Vec<String> = Vec::new();
    for s in &c.sections {
        match s {
            Section::Words { words, stress } => {
                for (i, chunk) in words.chunks(4).enumerate() {
                    let mut b = bar(Technique::Words, chunk.to_vec());
                    b.stress = stress
                        .iter()
                        .filter(|j| **j / 4 == i)
                        .map(|j| j % 4)
                        .collect();
                    content.push(b);
                    ends.push(false);
                }
                if let Some(e) = ends.last_mut() {
                    *e = true;
                }
                if let Some(w) = words.first() {
                    nodes.push(clip(w, 18));
                }
            }
            Section::Stat {
                values,
                labels,
                callout,
                claim,
            } => {
                let single = values.len() == 2 && values[0] == 0.0;
                let mut b = bar(Technique::Chart, Vec::new());
                b.beats = 6;
                b.chart = Some(ChartContent {
                    kind: if single { "spring" } else { chart_kind(r) },
                    values: values.clone(),
                    labels: labels.clone(),
                    callout: callout.clone(),
                    claim: claim.clone(),
                });
                content.push(b);
                ends.push(true);
            }
        }
    }
    // A UI flow is content: whole bars right after the hook, played by a
    // device frame whose states and copy the core derives.
    if let Some(f) = c.flow.as_ref().and_then(crate::flow::ui_flow) {
        let mut b = bar(Technique::Flow, Vec::new());
        // The flow's steps are fixed seconds: whole bars that hold them.
        let beats = (crate::flow::seconds(&f) / clock.beat_len()).ceil() as usize;
        b.beats = beats.div_ceil(4) * 4;
        content.insert(0, b);
        ends.insert(0, true);
    }
    let is_countdown = c.recipe.as_deref() == Some("top5_countdown");
    if is_countdown && content.len() >= 2 {
        content.reverse();
        ends.reverse();
    }
    let direction_only = content.is_empty();
    // Headlines are claims, never the brief's own words: the hook states the
    // first claim (or opens on the first quoted line), the end title the
    // strongest one.
    let claims: Vec<String> = content
        .iter()
        .filter_map(|b| b.chart.as_ref().map(|c| c.claim.clone()))
        .collect();
    let hook_words: Vec<String> = if is_countdown {
        owned(&["TOP 5", "COUNTDOWN", "WAIT FOR", "NO. 01"])
    } else if direction_only {
        owned(&k.hook)
    } else if let Some(cl) = claims.first() {
        crate::kinetic_words(cl, 4)
    } else if let Some(i) = content.iter().position(|b| b.technique == Technique::Words) {
        ends.remove(i);
        content.remove(i).words.into_iter().take(4).collect()
    } else {
        owned(&k.hook)
    };
    let end_title = if direction_only {
        k.end.to_string()
    } else {
        claims.last().cloned().unwrap_or_else(|| {
            // The first quoted line in full (the hook shows its first words).
            c.sections
                .iter()
                .find_map(|s| match s {
                    Section::Words { words, .. } => Some(clip(&words.join(" "), 24)),
                    _ => None,
                })
                .unwrap_or_else(|| hook_words.join(" "))
        })
    };
    // Peak: the last words bar of the content, else the concept's.
    let peak_words = match content
        .iter()
        .rposition(|b| b.technique == Technique::Words)
    {
        Some(i) if !direction_only && i + 1 == content.len() => {
            ends.remove(i);
            if let Some(e) = ends.last_mut() {
                *e = true;
            }
            content.remove(i).words
        }
        _ if direction_only => owned(&k.peak),
        _ => crate::kinetic_words(&end_title, 4),
    };
    // Fillers: the recipe's techniques with the concept's (or content's) stuff.
    let seconds = duration
        .or(c.duration)
        .unwrap_or(if direction_only { 15.0 } else { 0.0 });
    let want = target_bars(clock, seconds.clamp(0.0, 120.0));
    // Recaps re-draw a multi-value stat as another chart kind (a spring
    // would only repeat itself).
    let stats: Vec<Bar> = content
        .iter()
        .filter(|b| b.chart.as_ref().is_some_and(|c| c.kind != "spring"))
        .cloned()
        .collect();
    let mut middle = content;
    let mut t = 0usize;
    let filler = |t: usize| -> Bar {
        let tech = r.techniques[t % r.techniques.len().max(1)];
        match tech {
            Technique::Chart if direction_only => {
                let mut b = bar(Technique::Chart, Vec::new());
                b.beats = 6;
                b.chart = Some(ChartContent {
                    kind: k.chart.0,
                    values: k.chart.1.to_vec(),
                    labels: Vec::new(),
                    callout: Some(k.chart.2.to_string()),
                    claim: k.chart.3.join(" ").trim().to_string(),
                });
                b
            }
            // Content recap: a stat again, as another chart kind.
            Technique::Chart if !stats.is_empty() => {
                let mut b = stats[(t / r.techniques.len().max(1)) % stats.len()].clone();
                if let Some(ch) = &mut b.chart {
                    ch.kind = match ch.kind {
                        "bar" => "line",
                        "line" | "area" => "bar",
                        k => k,
                    };
                }
                b
            }
            // A content diagram once (it is the map of the story).
            Technique::Diagram
                if direction_only || (nodes.len() >= 3 && t < r.techniques.len()) =>
            {
                let n: Vec<String> = if direction_only {
                    owned(&k.nodes)
                } else {
                    nodes.iter().take(4).cloned().collect()
                };
                let mut b = bar(Technique::Diagram, n);
                b.beats = b.words.len().clamp(2, 4);
                b
            }
            Technique::Words if direction_only => bar(Technique::Words, owned(&k.words)),
            _ => {
                let mut b = bar(Technique::Morph, Vec::new());
                b.beats = 4;
                b
            }
        }
    };
    // Interleave fillers between content bars until the target is met.
    let mut bars = vec![{
        let mut b = bar(Technique::Hook, hook_words);
        b.beats = b.words.len().clamp(1, 4);
        b
    }];
    let content_n = middle.len();
    let beats_of = |bars: &[Bar]| bars.iter().map(|b| b.beats).sum::<usize>();
    // Target beats for the body (hook..peak), the end title's two excluded.
    let want_beats = want * 4;
    if content_n == 0 {
        for _ in 0..want.saturating_sub(2).max(3) {
            bars.push(filler(t));
            t += 1;
        }
    } else {
        // Content fillers are finite: one morph per section, each stat
        // recapped once, the story map (diagram) once. Content never repeats
        // more than that; a sparse brief makes a shorter video.
        let sections = ends.iter().filter(|e| **e).count();
        let mut queue: Vec<Bar> = Vec::new();
        let (mut morphs, mut recaps, mut maps) = (0, 0, 0);
        for i in 0..r.techniques.len().max(1) * (sections + stats.len() + 1) {
            let b = filler(i);
            let ok = match b.technique {
                Technique::Morph => morphs < sections,
                Technique::Chart => recaps < stats.len(),
                Technique::Diagram => maps < 1,
                _ => false,
            };
            if ok {
                match b.technique {
                    Technique::Morph => morphs += 1,
                    Technique::Chart => recaps += 1,
                    _ => maps += 1,
                }
                queue.push(b);
            }
        }
        queue.reverse();
        let peak_beats = 4;
        let mut left = beats_of(&middle);
        for (b, end) in middle.drain(..).zip(ends.iter()) {
            left -= b.beats;
            bars.push(b);
            if *end && beats_of(&bars) + left + peak_beats < want_beats {
                if let Some(f) = queue.pop() {
                    bars.push(f);
                }
            }
        }
        // Still short: the rest of the queue before the peak.
        while beats_of(&bars) + peak_beats < want_beats {
            match queue.pop() {
                Some(f) => bars.push(f),
                None => break,
            }
        }
        let _ = t;
    }
    bars.push(bar(Technique::Peak, peak_words));
    // Pad so the end title's hit lands on a downbeat (beat count ≡ 2 mod 4).
    let body: usize = bars.iter().map(|b| b.beats).sum();
    let pad = (4 - (body + 2) % 4) % 4;
    if pad > 0 {
        // A shape needs two beats to morph: stretch the last morph bar, or
        // add one of `pad + 4` beats (still lands on the downbeat).
        match bars.iter().rposition(|b| b.technique == Technique::Morph) {
            Some(i) => bars[i].beats += pad,
            None => {
                let mut m = bar(Technique::Morph, Vec::new());
                m.beats = pad + 4;
                let at = bars.len() - 1;
                bars.insert(at, m);
            }
        }
    }
    ReelPlan {
        concept: if direction_only {
            k.name.into()
        } else {
            "content".into()
        },
        bars,
        end_title,
    }
}

fn chart_kind(r: &MakeRecipe) -> &'static str {
    match r.chart.as_str() {
        "paper" => "area",
        _ => "bar",
    }
}

fn ev(beat: usize, verb: Value) -> Value {
    let mut v = verb;
    v["beat"] = json!(beat);
    v
}

/// Program frames per second (the default `fps`).
const FPS: f64 = 30.0;

/// The frame time at or before `t`, floored to the microsecond: a scene
/// ends exclusively, so the next bar's first frame is not shared.
/// Frame-snapped and led by the core's cut lead, exactly like a resolved
/// `start_beat`, so a bar's end and the next bar's start share one frame.
fn snap(t: f64) -> f64 {
    let lead = motion_program::taste::CUT_LEAD_FRAMES / FPS;
    ((((t * FPS).round() / FPS - lead).max(0.0)) * 1e6 - 1e-3).floor() / 1e6
}

fn r3(t: f64) -> f64 {
    (t * 1000.0).round() / 1000.0
}

/// The plan as a program (validated).
pub fn realize(
    c: &Content,
    r: &MakeRecipe,
    plan: &ReelPlan,
    clock: &Clock,
    aspect: Option<Aspect>,
) -> Result<MotionProgram, String> {
    let fin = plan.final_downbeat();
    if clock.beats.len() < fin + 2 {
        return Err(format!(
            "track too short: needs {} beats, has {}",
            fin + 2,
            clock.beats.len()
        ));
    }
    // Under one beat of hold after the final hit (the core's `end_beat` rule).
    let lead = motion_program::taste::CUT_LEAD_FRAMES / FPS;
    let duration = r3(snap(clock.at(fin)) + lead + clock.beat_len() * 0.85);
    if duration > 120.0 {
        return Err(format!("plan is {duration:.0} s; at most 120 s"));
    }
    let mut comps: Vec<Value> = vec![json!({"id": "ground", "kind": "backdrop"})];
    if let Some(f) = &r.field {
        comps.push(json!({"id": "field", "kind": "field", "preset": f,
            "timeline": [{"at": 0.0, "do": "enter", "style": "fade"}]}));
    }
    let mut b0 = 0usize;
    let mut shape_i = 0usize;
    let mut morph_n = 0usize;
    let mut camera: Vec<Value> = Vec::new();
    for (i, bar) in plan.bars.iter().enumerate() {
        let b1 = b0 + bar.beats;
        // Frame-snapped like resolved `start_beat`s, so bars hand over on
        // one frame with no overlap.
        let end = snap(clock.at(b1));
        let id = |k: &str| format!("b{i}_{k}");
        match bar.technique {
            Technique::Hook | Technique::Words | Technique::Peak => {
                let poster = bar.technique != Technique::Words;
                let preset = if poster { &r.poster } else { &r.body };
                let mut tl = Vec::new();
                if bar.technique == Technique::Peak && preset == "poster_cuts" {
                    for j in (1..bar.words.len()).step_by(2) {
                        tl.push(ev(b0 + j, json!({"do": "cut", "flip": true})));
                    }
                }
                let mut stress = bar.stress.clone();
                if poster && stress.is_empty() {
                    stress.push(bar.words.len() - 1);
                }
                let k = json!({
                    "id": id("words"), "kind": "kinetic_text", "preset": preset,
                    "start_beat": b0, "end": end, "beat_cuts": 1,
                    "words": bar.words, "stress": stress, "dot": poster,
                    "timeline": tl,
                });
                comps.push(k);
            }
            Technique::Morph => {
                let first = SHAPES[(shape_i + c.seed as usize) % SHAPES.len()];
                let mut tl = vec![ev(b0, json!({"do": "enter", "style": "pop"}))];
                // Each morph bar starts from another place (no repeated layout).
                let home = ["center", "lead", "trail"][morph_n % 3];
                morph_n += 1;
                let mut moved = false;
                for j in 1..bar.beats {
                    let verb = if j + 1 == bar.beats {
                        json!({"do": "recolour", "to": "ink"})
                    } else if j % 2 == 1 {
                        shape_i += 1;
                        json!({"do": "morph", "to": SHAPES[(shape_i + c.seed as usize) % SHAPES.len()]})
                    } else {
                        moved = !moved;
                        let away = if home == "lead" { "trail" } else { "lead" };
                        json!({"do": "move", "to": if moved { away } else { home }})
                    };
                    tl.push(ev(b0 + j, verb));
                }
                shape_i += 1;
                comps.push(json!({
                    "id": id("shape"), "kind": "shape", "shape": first, "size": "xl",
                    "slot": home,
                    "start_beat": b0, "end": end, "timeline": tl,
                }));
            }
            Technique::Chart => {
                let ch = bar.chart.as_ref().expect("chart bar has a chart");
                let (values, hero) = if ch.kind == "spring" {
                    let v = ch.values.iter().cloned().fold(f64::MIN, f64::max);
                    (vec![0.0, v], 1)
                } else {
                    let h = (0..ch.values.len()).fold(0, |m, i| {
                        if ch.values[i] > ch.values[m] {
                            i
                        } else {
                            m
                        }
                    });
                    (ch.values.clone(), h)
                };
                let labels = if ch.labels.len() == values.len() {
                    ch.labels.clone()
                } else {
                    Vec::new()
                };
                // Story in two shots: the series builds on the downbeat and
                // its hero pops two beats later and hits; then the claim takes
                // the whole frame for two beats and hits on the last. No eyebrow
                // label over the chart.
                let mid = snap(clock.at(b0 + 4));
                // Bar i on beat i, then the hero lands (core `beat_cuts`);
                // a spring, a 2-point line/area, or a long series draws in one move instead.
                let stepped = ch.kind != "spring"
                    && values.len() <= 4
                    && !(matches!(ch.kind, "line" | "area") && values.len() <= 2);
                let mut chart = json!({
                    "id": id("chart"), "kind": "chart", "preset": r.chart, "chart": ch.kind,
                    "values": values, "labels": labels, "hero": hero, "callout": ch.callout,
                    "size": "xl", "start_beat": b0, "end": mid,
                    "timeline": [
                        ev(b0, json!({"do": "enter", "style": "draw"})),
                        ev(b0 + 2, json!({"do": "pop_label"})),
                    ],
                });
                if stepped {
                    chart["beat_cuts"] = json!(1);
                    chart["timeline"] = json!([]);
                }
                comps.push(chart);
                // The camera leans in on the off-beats and lets go on the cut.
                camera.push(ev(b0 + 1, json!({"do": "push", "to": 1.04})));
                camera.push(ev(b0 + 3, json!({"do": "push", "to": 1.1})));
                camera.push(ev(b0 + 4, json!({"do": "reset"})));
                // The claim as one poster line (fills the width), held two
                // beats; the second beat flips ground and ink.
                let claim = clip(&ch.claim, 24);
                comps.push(json!({
                    "id": id("claim"), "kind": "kinetic_text", "preset": "poster_cuts",
                    "words": [claim, claim], "start_beat": b0 + 4, "end": end,
                    "timeline": [ev(b0 + 5, json!({"do": "cut", "flip": true}))],
                }));
            }
            Technique::Flow => {
                let spec = c.flow.as_ref().expect("flow bar has a flow");
                let f = crate::flow::ui_flow(spec).expect("checked in plan");
                comps.push(crate::flow::component(
                    &id("flow"),
                    &f,
                    spec.device,
                    b0,
                    end,
                ));
                // Beside it a beat-cut type column keeps every beat alive
                // (the flow's own steps hold for reading).
                comps.push(json!({
                    "id": id("column"), "kind": "kinetic_text", "preset": r.body,
                    "slot": "trail", "start_beat": b0, "end": end, "beat_cuts": 1,
                    "words": crate::flow::column(
                        spec.flow,
                        &f,
                        spec.amount.as_deref(),
                        &c.direction_words,
                        &(b0..b1).map(|k| clock.at(k) - clock.at(b0)).collect::<Vec<_>>(),
                    ),
                }));
            }
            Technique::Diagram => {
                comps.push(json!({
                    "id": id("diagram"), "kind": "diagram", "preset": r.diagram,
                    "nodes": bar.words, "beat_cuts": 1, "start_beat": b0, "end": end,
                }));
            }
        }
        b0 = b1;
    }
    // End title: arrives half a bar before the final downbeat; the final hit
    // lands on it (poster: the ground flips, the carried dot comes to rest).
    let text = clip(&plan.end_title.to_uppercase(), 24);
    if r.poster == "poster_cuts" {
        comps.push(json!({
            "id": "end", "kind": "kinetic_text", "preset": "poster_cuts", "dot": true,
            "start_beat": b0, "words": [text, text], "stress": [1],
            "timeline": [ev(fin, json!({"do": "cut", "flip": true}))],
        }));
    } else {
        comps.push(json!({
            "id": "end", "kind": "title", "preset": "display", "size": "xl",
            "text": text, "start_beat": b0,
            "timeline": [
                ev(b0, json!({"do": "enter", "style": "wipe"})),
                ev(fin, json!({"do": "hit"})),
            ],
        }));
    }
    let mut p = json!({
        "version": motion_program::program::PROGRAM_VERSION,
        "meta": {"name": c.name, "duration": duration, "brief": c.brief},
        "music": {"track": r.music, "start": r3(clock.track_start)},
        "components": comps,
        "camera": camera,
    });
    if let Some(a) = aspect {
        p["meta"]["aspect"] = serde_json::to_value(a).expect("aspect");
    }
    for (k, v) in [
        ("recipe", &r.style),
        ("palette", &r.palette),
        ("fonts", &r.fonts),
    ] {
        if let Some(v) = v {
            p[k] = v.clone();
        }
    }
    if let Some(t) = &r.tempo {
        p["tempo"] = json!(t);
    }
    let mut prog: MotionProgram =
        serde_json::from_value(p).map_err(|e| format!("bridge built an invalid program: {e}"))?;
    let unresolved = prog.clone();
    prog.resolve_beats(&clock.beats);
    motion_program::validate(&prog).map_err(|e| e.to_string())?;
    // Keep the beat references in the written program (it stays readable
    // and re-resolvable); validation ran on the resolved copy.
    Ok(unresolved)
}
