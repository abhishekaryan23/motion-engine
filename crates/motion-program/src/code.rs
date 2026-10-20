//! CodeBlock: mono lines typed in with a caret, coloured by a tiny
//! per-language tokenizer from the palette. Steps: `type` a line, `retype`
//! it (deleted back to its start, then typed again), `dissolve` it into ink
//! speckles (optionally becoming a serif italic word) and `highlight` it.
//!
//! Every syntax run is one text layer on a shared baseline; typing is a
//! per-glyph cascade whose glyphs pop in one after another (discrete, like
//! keys), deleting is a mask that shrinks right to left. Nothing loops: the
//! caret stays solid while it works and is put away at the end.

use motion_core::easing::Easing;
use motion_core::path;
use motion_core::scene::{
    Color, Fill, FontRole, GlyphOrder, GlyphPose, Layer, LayerKind, LayerMask, MotionOp,
    PathFillRule, Scene, TextAlign, TextStyle,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::lower::{layer, r6, scene, Ctx, Track, LINE_HEIGHT};
use crate::program::{Component, EnterStyle, ExitStyle, Size, Slot, Verb};
use crate::taste::{self, Shape, Spring};

/// Mono code typed in with a caret: lines, a language for colouring and
/// steps (`type`, `retype`, `dissolve`, `highlight`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CodeBlock {
    #[serde(default)]
    pub preset: CodePreset,
    /// 1..=14 lines of at most 48 characters (no tabs).
    pub lines: Vec<String>,
    #[serde(default)]
    pub language: Language,
    #[serde(default)]
    pub slot: Slot,
    #[serde(default)]
    pub size: Size,
    /// In time order; `at` is seconds from the component start. Without any
    /// `type` step every line types in, one after another, from 0.3 s;
    /// otherwise lines without a `type` step are shown from the start.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<CodeStep>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CodePreset {
    /// Ink on the page, muted line numbers, accent keywords.
    #[default]
    PaperMono,
    /// A dark window (ink) with light type and a title bar.
    TerminalDark,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    Plain,
    Rust,
    Python,
    Js,
    Shell,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CodeStep {
    /// Seconds from the component start (fallback when no `beat`/`bar`).
    #[serde(default)]
    pub at: f64,
    /// On music beat n instead (program beat grid, snapped to a frame).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beat: Option<u32>,
    /// On the downbeat of bar n (beat 4n) instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bar: Option<u32>,
    #[serde(flatten)]
    pub op: CodeOp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum CodeOp {
    /// Type line `line` (0-based) at 0.05 s a character.
    Type { line: usize },
    /// Delete line `line` back to its start, then type `to` in its place.
    Retype { line: usize, to: String },
    /// Line `line` crumbles into ink speckles; with `to`, a serif italic
    /// word rises in its place.
    Dissolve {
        line: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<String>,
    },
    /// A band sweeps in behind line `line` (once per line).
    Highlight { line: usize },
}

impl CodeOp {
    fn line(&self) -> usize {
        match self {
            CodeOp::Type { line }
            | CodeOp::Retype { line, .. }
            | CodeOp::Dissolve { line, .. }
            | CodeOp::Highlight { line } => *line,
        }
    }
}

pub const MAX_LINES: usize = 14;
pub const MAX_CHARS: usize = 48;

impl CodeBlock {
    /// Content problems as `(path suffix, message)`.
    pub fn check(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let ok_line = |s: &str| s.chars().count() <= MAX_CHARS && !s.contains(['\n', '\t']);
        if self.lines.is_empty() || self.lines.len() > MAX_LINES {
            out.push((
                ".lines".into(),
                format!("1..={MAX_LINES} lines, got {}", self.lines.len()),
            ));
        }
        for (i, l) in self.lines.iter().enumerate() {
            if !ok_line(l) {
                out.push((
                    format!(".lines[{i}]"),
                    format!("at most {MAX_CHARS} characters, no tabs or newlines"),
                ));
            }
        }
        let mut last = f64::NEG_INFINITY;
        let mut lit = Vec::new();
        for (i, s) in self.steps.iter().enumerate() {
            let p = format!(".steps[{i}]");
            if !(s.at.is_finite() && s.at >= 0.0) || s.at < last {
                out.push((format!("{p}.at"), "times are >= 0 and in order".into()));
            }
            last = s.at;
            if s.beat.is_some() || s.bar.is_some() {
                out.push((
                    format!("{p}.beat"),
                    "beat not in the music analysis (or no music)".into(),
                ));
            }
            if s.op.line() >= self.lines.len() {
                out.push((format!("{p}.line"), "no such line".into()));
            }
            match &s.op {
                CodeOp::Retype { to, .. } if !ok_line(to) || to.trim().is_empty() => out.push((
                    format!("{p}.to"),
                    format!("1..={MAX_CHARS} characters, one line"),
                )),
                CodeOp::Dissolve { to: Some(to), .. }
                    if to.trim().is_empty() || to.chars().count() > 24 =>
                {
                    out.push((format!("{p}.to"), "1..=24 characters".into()))
                }
                CodeOp::Highlight { line } => {
                    if lit.contains(line) {
                        out.push((p.to_string(), "one highlight per line".into()));
                    }
                    lit.push(*line);
                }
                _ => {}
            }
        }
        out
    }
}

/// Seconds per typed character at normal tempo; deleting runs twice as fast.
const CHAR: f64 = 0.05;
const ROWS: f32 = 1.55;

// ---------------------------------------------------------------------------
// Tokens

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tok {
    Plain,
    Keyword,
    Str,
    Num,
    Comment,
    Punct,
}

fn keywords(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Plain => &[],
        Language::Rust => &[
            "fn", "let", "mut", "pub", "struct", "enum", "impl", "use", "match", "if", "else",
            "for", "in", "while", "loop", "return", "self", "Self", "mod", "const", "true",
            "false", "async", "await", "move", "where", "trait", "type",
        ],
        Language::Python => &[
            "def", "class", "return", "import", "from", "as", "if", "elif", "else", "for", "in",
            "while", "with", "lambda", "None", "True", "False", "and", "or", "not", "yield",
            "async", "await", "print",
        ],
        Language::Js => &[
            "const", "let", "var", "function", "return", "if", "else", "for", "of", "in", "import",
            "from", "export", "default", "await", "async", "new", "class", "true", "false", "null",
            "=>",
        ],
        Language::Shell => &[
            "cd", "ls", "git", "cargo", "npm", "echo", "export", "sudo", "brew", "curl", "run",
        ],
    }
}

/// Splits a line into coloured runs (adjacent runs of one class merged;
/// whitespace joins the run before it).
pub(crate) fn tokenize(line: &str, lang: Language) -> Vec<(Tok, String)> {
    let kw = keywords(lang);
    let comment = match lang {
        Language::Python | Language::Shell => "#",
        Language::Plain => "\u{0}",
        _ => "//",
    };
    let cs: Vec<char> = line.chars().collect();
    let mut out: Vec<(Tok, String)> = Vec::new();
    let push = |t: Tok, s: String, out: &mut Vec<(Tok, String)>| match out.last_mut() {
        Some((lt, ls)) if *lt == t || s.trim().is_empty() => ls.push_str(&s),
        _ => out.push((t, s)),
    };
    let mut i = 0;
    while i < cs.len() {
        let rest: String = cs[i..].iter().collect();
        let c = cs[i];
        if lang != Language::Plain && rest.starts_with(comment) {
            push(Tok::Comment, rest, &mut out);
            break;
        }
        if lang != Language::Plain && (c == '"' || c == '\'' || c == '`') {
            let mut j = i + 1;
            while j < cs.len() && cs[j] != c {
                j += 1;
            }
            let j = (j + 1).min(cs.len());
            push(Tok::Str, cs[i..j].iter().collect(), &mut out);
            i = j;
            continue;
        }
        if c.is_alphanumeric() || c == '_' || (lang == Language::Shell && c == '-') {
            let mut j = i;
            while j < cs.len()
                && (cs[j].is_alphanumeric() || cs[j] == '_' || cs[j] == '.' && c.is_ascii_digit())
            {
                j += 1;
            }
            let j = j.max(i + 1);
            let w: String = cs[i..j].iter().collect();
            let t = if c.is_ascii_digit() {
                Tok::Num
            } else if kw.contains(&w.as_str()) {
                Tok::Keyword
            } else {
                Tok::Plain
            };
            push(t, w, &mut out);
            i = j;
            continue;
        }
        if c.is_whitespace() {
            push(Tok::Plain, c.to_string(), &mut out);
        } else if lang == Language::Js && rest.starts_with("=>") {
            push(Tok::Keyword, "=>".into(), &mut out);
            i += 2;
            continue;
        } else {
            push(
                if lang == Language::Plain {
                    Tok::Plain
                } else {
                    Tok::Punct
                },
                c.to_string(),
                &mut out,
            );
        }
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Lowering

fn alpha(c: Color, a: u8) -> Color {
    Color::rgba(c.r, c.g, c.b, a)
}

struct Look {
    panel: Option<Color>,
    text: Color,
    keyword: Color,
    string: Color,
    number: Color,
    comment: Color,
    punct: Color,
    gutter: Color,
    caret: Color,
    band: Color,
}

impl Look {
    /// Syntax colours from the roles of whatever the code sits on (the
    /// component's ground, or the terminal panel): ink, the secondary ink
    /// for quieter tokens, the accent (as type) for keywords and numbers.
    /// Every token is type, so every token clears the type band.
    fn of(cx: &Ctx, id: &str, preset: CodePreset) -> Look {
        let g = cx.roles(id);
        let (panel, r) = match preset {
            CodePreset::PaperMono => (None, g),
            CodePreset::TerminalDark => (Some(g.ink), cx.col.roles(g.ink)),
        };
        // Small type: the 7:1 band.
        let fs = 0.0;
        let string = r.text(crate::palette::mix_oklab(r.ink, r.accent_text, 0.5), fs);
        Look {
            panel,
            text: r.ink,
            keyword: r.text(r.accent_text, fs),
            string,
            number: r.text(r.accent_text, fs),
            comment: r.ink_secondary,
            punct: r.ink,
            gutter: r.ink_secondary,
            caret: r.accent,
            band: alpha(r.accent, 28),
        }
    }

    fn colour(&self, t: Tok) -> Color {
        match t {
            Tok::Plain => self.text,
            Tok::Keyword => self.keyword,
            Tok::Str => self.string,
            Tok::Num => self.number,
            Tok::Comment => self.comment,
            Tok::Punct => self.punct,
        }
    }
}

/// A mono text layer whose box starts at `left` and is centred on row `y`.
fn mono(id: &str, text: &str, size: f32, adv: f32, left: f32, y: f32, colour: Color) -> Layer {
    let n = text.chars().count() as f32;
    let w = (n * adv).ceil().max(1.0);
    let mut l = layer(
        id,
        LayerKind::Text(TextStyle {
            text: text.to_string(),
            font_role: FontRole::Mono,
            font_size: size,
            font_weight: 400,
            italic: false,
            color: colour,
            align: TextAlign::Left,
            line_height: LINE_HEIGHT,
            letter_spacing: 0.0,
            max_width: None,
            uppercase: false,
            ink: None,
            fill: None,
            outline: None,
        }),
        [left, y],
        [w + size * 0.3, (size * LINE_HEIGHT).round()],
        0,
    );
    l.anchor_x = 0.0;
    l
}

fn rect(id: &str, x: f32, y: f32, w: f32, h: f32, r: f32, colour: Color) -> Layer {
    let mut l = layer(
        id,
        LayerKind::Path {
            d: path::rounded_rect(0.0, 0.0, w, h, r).to_svg(),
            fill: Some(Fill::Solid(colour)),
            stroke: None,
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        [x, y],
        [w, h],
        0,
    );
    l.anchor_x = 0.0;
    l.anchor_y = 0.0;
    l
}

/// Types `layers` (runs of one line, left to right) from `t0`; returns the
/// time the last glyph lands.
fn type_runs(track: &mut Track, runs: &[(String, String)], t0: f64, ch: f64, fps: f64) -> f64 {
    let mut t = t0;
    for (id, text) in runs {
        let n = text.chars().filter(|c| !c.is_whitespace()).count();
        let spaces = text.chars().count() - n;
        if n > 0 {
            track.push(
                id,
                t,
                (n as f64 - 1.0) * ch + 1.0 / fps,
                Shape::Ease(Easing::Linear),
                MotionOp::GlyphCascade {
                    stagger: ch,
                    from: GlyphPose {
                        opacity: 0.0,
                        ..GlyphPose::REST
                    },
                    order: GlyphOrder::Forward,
                    seed: 0,
                },
            );
        }
        t += (n + spaces) as f64 * ch;
    }
    t
}

pub(crate) fn lower(cx: &Ctx, c: &Component, b: &CodeBlock, z: i32, length: f64) -> Vec<Scene> {
    let id = c.id.as_str();
    let look = Look::of(cx, &c.id, b.preset);
    let ch = CHAR * cx.tempo.factor();
    let fps = cx.fps;

    // Measure: the longest line (and every retype target) fits the slot.
    let longest = b
        .lines
        .iter()
        .chain(b.steps.iter().filter_map(|s| match &s.op {
            CodeOp::Retype { to, .. } => Some(to),
            _ => None,
        }))
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(1)
        .max(12) as f32;
    let gutter_cols = if b.preset == CodePreset::PaperMono {
        3.0
    } else {
        0.0
    };
    let adv100 = cx.line_width("MMMMMMMMMM", FontRole::Mono, 100.0, 0.0, false) / 10.0;
    let max_size = cx.short
        * match b.size {
            Size::S => 0.026,
            Size::M => 0.034,
            Size::L => 0.044,
            Size::Xl => 0.056,
        };
    let max_card_w = if matches!(b.slot, Slot::Left | Slot::Right) {
        crate::taste::slot_width(b.slot, cx.aspect)
    } else {
        cx.w * 0.8
    };
    let width = taste::slot_width(b.slot, cx.aspect).min(max_card_w);
    let pad = if look.panel.is_some() { 2.0 } else { 0.0 };
    let size = (width / ((longest + gutter_cols + 2.0 * pad) * adv100 / 100.0))
        .min(max_size)
        .floor();
    let adv = adv100 * size / 100.0;
    let row = (size * ROWS).round();
    let n = b.lines.len() as f32;
    let bar = if look.panel.is_some() { row * 1.2 } else { 0.0 };
    let block_w = (longest + gutter_cols + 2.0 * pad) * adv;
    let block_h = n * row + bar + 2.0 * pad * adv * 0.6;
    let centre = if matches!(b.slot, Slot::Left | Slot::Right) {
        crate::taste::slot_center(b.slot, cx.aspect)
    } else {
        taste::slot_center(b.slot, cx.aspect)
    };
    let x0 = (centre[0] - block_w / 2.0).round();
    let y0 = (centre[1] - block_h / 2.0).round();
    let text_x = x0 + (pad + gutter_cols) * adv;
    let row_y = |i: usize| y0 + bar + pad * adv * 0.6 + (i as f32 + 0.5) * row;

    let mut layers: Vec<Layer> = Vec::new();
    let mut track = Track::default();

    if let Some(panel) = look.panel {
        layers.push(rect(
            &format!("{id}.panel"),
            x0,
            y0,
            block_w,
            block_h,
            (size * 0.6).round(),
            panel,
        ));
        for k in 0..3 {
            let d = (row * 0.32).round();
            let mut dot = layer(
                &format!("{id}.light{k}"),
                LayerKind::Path {
                    d: path::ellipse(0.0, 0.0, d, d).to_svg(),
                    fill: Some(Fill::Solid(cx.col.roles(panel).structure)),
                    stroke: None,
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                [
                    x0 + pad * adv + k as f32 * d * 1.7 + d / 2.0,
                    y0 + bar * 0.55,
                ],
                [d, d],
                0,
            );
            dot.z_index = 1;
            layers.push(dot);
        }
    }

    // Default schedule: every line types in turn.
    let mut steps = b.steps.clone();
    if !steps.iter().any(|s| matches!(s.op, CodeOp::Type { .. })) && b.steps.is_empty() {
        let mut t = 0.3;
        for (i, l) in b.lines.iter().enumerate() {
            steps.push(CodeStep {
                at: r6(t),
                beat: None,
                bar: None,
                op: CodeOp::Type { line: i },
            });
            t += l.chars().count() as f64 * ch + 0.25;
        }
    }
    let typed: Vec<usize> = steps
        .iter()
        .filter_map(|s| match s.op {
            CodeOp::Type { line } => Some(line),
            _ => None,
        })
        .collect();

    // Line numbers (paper preset).
    if gutter_cols > 0.0 {
        for i in 0..b.lines.len() {
            let mut g = mono(
                &format!("{id}.no{i}"),
                &format!("{:>2}", i + 1),
                size,
                adv,
                x0,
                row_y(i),
                look.gutter,
            );
            g.z_index = 2;
            layers.push(g);
        }
    }

    // Each line's current runs: (layer id, text).
    let mut runs: Vec<Vec<(String, String)>> = Vec::new();
    for (i, line) in b.lines.iter().enumerate() {
        let mut col = 0usize;
        let mut v = Vec::new();
        for (k, (tok, text)) in tokenize(line, b.language).into_iter().enumerate() {
            let rid = format!("{id}.l{i}.{k}");
            let mut l = mono(
                &rid,
                &text,
                size,
                adv,
                text_x + col as f32 * adv,
                row_y(i),
                look.colour(tok),
            );
            l.z_index = 3;
            col += text.chars().count();
            layers.push(l);
            v.push((rid, text));
        }
        runs.push(v);
    }

    // Caret: a solid bar that follows the typing.
    let caret_id = format!("{id}.caret");
    let caret_h = (size * 1.05).round();
    let caret_w = (size * 0.1).max(3.0).round();
    let home = [text_x, row_y(0) - caret_h / 2.0];
    let mut caret = rect(
        &caret_id, home[0], home[1], caret_w, caret_h, 0.0, look.caret,
    );
    caret.z_index = 5;
    caret.opacity = 0.0;
    let mut caret_at = [0.0f32, 0.0];
    let mut caret_shown = false;
    let mut caret_busy = 0.0f64;
    let spot = |line: usize, col: usize| [col as f32 * adv + caret_w * 0.5, row_y(line) - row_y(0)];
    let caret_go = |track: &mut Track, t: f64, d: f64, to: [f32; 2], at: &mut [f32; 2]| {
        let d = d.max(1.0 / fps);
        track.push(
            &caret_id,
            t,
            d,
            Shape::Ease(Easing::Linear),
            MotionOp::Move {
                from: *at,
                to,
                via: None,
            },
        );
        *at = to;
    };

    let mut extra = 0usize;
    let mut last_end = 0.0f64;
    for s in &steps {
        let line = s.op.line();
        if line >= runs.len() {
            continue;
        }
        let at = s.at.max(caret_busy);
        match &s.op {
            CodeOp::Type { line } => {
                let n = runs[*line]
                    .iter()
                    .map(|r| r.1.chars().count())
                    .sum::<usize>();
                if !caret_shown {
                    caret_go(
                        &mut track,
                        (at - 1.0 / fps).max(0.0),
                        0.0,
                        spot(*line, 0),
                        &mut caret_at,
                    );
                    track.push(
                        &caret_id,
                        at,
                        1.0 / fps,
                        Shape::Ease(Easing::Linear),
                        fade(0.0, 1.0),
                    );
                    caret_shown = true;
                } else {
                    caret_go(&mut track, at, 0.0, spot(*line, 0), &mut caret_at);
                }
                let end = type_runs(&mut track, &runs[*line], at, ch, fps);
                caret_go(
                    &mut track,
                    at + 1.0 / fps,
                    end - at - 1.0 / fps,
                    spot(*line, n),
                    &mut caret_at,
                );
                caret_busy = end;
                last_end = last_end.max(end);
            }
            CodeOp::Retype { line, to } => {
                let line = *line;
                let n = runs[line]
                    .iter()
                    .map(|r| r.1.chars().count())
                    .sum::<usize>();
                let del = ch * 0.5;
                caret_go(&mut track, at, 0.0, spot(line, n), &mut caret_at);
                // Delete right to left: each run's mask shrinks to its left edge.
                let mut t = at + 1.0 / fps;
                let mut col = n;
                for (rid, text) in runs[line].iter().rev() {
                    let k = text.chars().count();
                    let l = layers.iter_mut().find(|l| &l.id == rid).expect("run");
                    let (w, h) = (l.width, l.height);
                    l.mask = Some(LayerMask {
                        d: path::rounded_rect(-w, -h, 3.0 * w, 3.0 * h, 0.0).to_svg(),
                        invert: false,
                    });
                    let d = k as f64 * del;
                    track.push(
                        rid,
                        t,
                        d.max(1.0 / fps),
                        Shape::Ease(Easing::Linear),
                        MotionOp::MaskMorph {
                            to: path::rounded_rect(-w, -h, w, 3.0 * h, 0.0).to_svg(),
                        },
                    );
                    t += d;
                    col -= k;
                }
                let _ = col;
                caret_go(
                    &mut track,
                    at + 1.0 / fps,
                    t - at - 1.0 / fps,
                    spot(line, 0),
                    &mut caret_at,
                );
                // Type the replacement.
                let t1 = t + 0.18;
                let mut col = 0usize;
                let mut v = Vec::new();
                for (tok, text) in tokenize(to, b.language) {
                    let rid = format!("{id}.r{extra}");
                    extra += 1;
                    let mut l = mono(
                        &rid,
                        &text,
                        size,
                        adv,
                        text_x + col as f32 * adv,
                        row_y(line),
                        look.colour(tok),
                    );
                    l.z_index = 3;
                    col += text.chars().count();
                    layers.push(l);
                    v.push((rid, text));
                }
                let end = type_runs(&mut track, &v, t1, ch, fps);
                caret_go(
                    &mut track,
                    t1 + 1.0 / fps,
                    end - t1 - 1.0 / fps,
                    spot(line, col),
                    &mut caret_at,
                );
                runs[line] = v;
                caret_busy = end;
                last_end = last_end.max(end);
            }
            CodeOp::Dissolve { line, to } => {
                let line = *line;
                let d = 0.45 * cx.tempo.factor();
                let drift = (size * 0.35).round();
                let mut dots: Vec<[f32; 3]> = Vec::new();
                let mut col = 0usize;
                let mut seed = crate::code::seed(id) ^ (line as u64 + 1);
                for (rid, text) in &runs[line] {
                    track.push(rid, at, d, Shape::Ease(Easing::InCubic), fade(1.0, 0.0));
                    track.push(
                        rid,
                        at,
                        d,
                        Shape::Ease(Easing::InCubic),
                        MotionOp::Move {
                            from: [0.0, 0.0],
                            to: [0.0, drift * 0.5],
                            via: None,
                        },
                    );
                    for (j, c) in text.chars().enumerate() {
                        if c.is_whitespace() {
                            continue;
                        }
                        for _ in 0..3 {
                            let u = next(&mut seed);
                            let v = next(&mut seed);
                            let s = 0.6 + next(&mut seed) * 0.9;
                            dots.push([
                                (col + j) as f32 * adv + u * adv,
                                (v - 0.5) * size * 0.7,
                                s,
                            ]);
                        }
                    }
                    col += text.chars().count();
                }
                if !dots.is_empty() {
                    let did = format!("{id}.dust{line}");
                    let r = (size * 0.045).max(1.5);
                    let w = (col as f32 * adv).max(1.0);
                    let mut l = layer(
                        &did,
                        LayerKind::Instances {
                            d: path::ellipse(-r, -r, 2.0 * r, 2.0 * r).to_svg(),
                            fill: Fill::Solid(look.text),
                            at: dots,
                        },
                        [text_x, row_y(line) - row / 2.0],
                        [w, row],
                        4,
                    );
                    l.anchor_x = 0.0;
                    l.anchor_y = 0.0;
                    // Box space: shift the dots onto the row's centre line.
                    if let LayerKind::Instances { at, .. } = &mut l.kind {
                        for p in at.iter_mut() {
                            p[1] += row / 2.0;
                        }
                    }
                    l.opacity = 0.0;
                    track.push(
                        &did,
                        at,
                        1.0 / fps,
                        Shape::Ease(Easing::Linear),
                        fade(0.0, 1.0),
                    );
                    track.push(
                        &did,
                        at,
                        d * 1.8,
                        Shape::Ease(Easing::OutCubic),
                        MotionOp::Move {
                            from: [0.0, 0.0],
                            to: [drift * 0.4, drift * 1.6],
                            via: None,
                        },
                    );
                    track.push(
                        &did,
                        at + d * 0.3,
                        d * 1.5,
                        Shape::Ease(Easing::InOutCubic),
                        MotionOp::Trim { from: 1.0, to: 0.0 },
                    );
                    layers.push(l);
                }
                if let Some(word) = to {
                    let wid = format!("{id}.w{extra}");
                    extra += 1;
                    let ssize = (size * 1.35).round();
                    let mut l = layer(
                        &wid,
                        LayerKind::Text(TextStyle {
                            text: word.clone(),
                            font_role: FontRole::SerifEmotional,
                            font_size: ssize,
                            font_weight: 400,
                            italic: false,
                            color: look.keyword,
                            align: TextAlign::Left,
                            line_height: LINE_HEIGHT,
                            letter_spacing: 0.0,
                            max_width: None,
                            uppercase: false,
                            ink: None,
                            fill: None,
                            outline: None,
                        }),
                        [text_x, row_y(line)],
                        [
                            cx.line_width(word, FontRole::SerifEmotional, ssize, 0.0, false)
                                + ssize * 0.3,
                            (ssize * LINE_HEIGHT).round(),
                        ],
                        3,
                    );
                    l.anchor_x = 0.0;
                    let t = at + d * 0.55;
                    let n = word.chars().filter(|c| !c.is_whitespace()).count().max(1);
                    let st = 0.035 * cx.tempo.factor();
                    track.push(
                        &wid,
                        t,
                        (n as f64 - 1.0) * st + 0.5,
                        Shape::Spring(Spring::Settle),
                        MotionOp::GlyphCascade {
                            stagger: st,
                            from: GlyphPose {
                                dy: ssize * 0.28,
                                opacity: 0.0,
                                scale: 0.92,
                                ..GlyphPose::REST
                            },
                            order: GlyphOrder::Forward,
                            seed: 0,
                        },
                    );
                    layers.push(l);
                }
                runs[line].clear();
                last_end = last_end.max(at + d);
            }
            CodeOp::Highlight { line } => {
                let bid = format!("{id}.band{line}");
                let bx = text_x - adv * 0.6;
                let bw = (x0 + block_w - pad * adv * 0.5) - bx;
                let mut l = rect(
                    &bid,
                    bx,
                    row_y(*line) - row * 0.5,
                    bw,
                    row,
                    (row * 0.12).round(),
                    look.band,
                );
                l.z_index = 1;
                let hd = (0.4 * cx.tempo.factor()).min((length - at).max(1.0 / fps));
                track.push(
                    &bid,
                    at,
                    hd,
                    Shape::Ease(Easing::OutQuint),
                    MotionOp::Scale {
                        from: 0.0,
                        to: 1.0,
                        axis: motion_core::scene::Axis::X,
                    },
                );
                layers.push(l);
            }
        }
    }
    let _ = typed;
    let _ = last_end;
    layers.push(caret);

    // Whole-block enter/exit through a group around everything.
    let mut group = layer(
        id,
        LayerKind::Group { children: layers },
        [cx.w / 2.0, cx.h / 2.0],
        [cx.w, cx.h],
        z,
    );
    group.depth = Some(1.0);
    for e in &c.timeline {
        let Some(at) = e.at else { continue };
        let d = taste::duration(&e.verb, cx.tempo);
        match e.verb {
            Verb::Enter { style } => {
                if style == EnterStyle::Rise {
                    track.push(
                        id,
                        at,
                        d,
                        Shape::Spring(Spring::Settle),
                        MotionOp::Move {
                            from: [0.0, (cx.short * 0.04).round()],
                            to: [0.0, 0.0],
                            via: None,
                        },
                    );
                }
                track.push(
                    id,
                    at,
                    d.min(0.4),
                    Shape::Ease(Easing::OutCubic),
                    fade(0.0, 1.0),
                );
            }
            Verb::Exit { style } => {
                let _ = style == ExitStyle::Fade;
                track.push(id, at, d, Shape::Ease(Easing::InCubic), fade(1.0, 0.0));
            }
            _ => {}
        }
    }
    group.opacity = 1.0;
    vec![scene(id, c.start, length, vec![group], track.motions)]
}

fn fade(from: f32, to: f32) -> MotionOp {
    MotionOp::Fade { from, to }
}

pub(crate) fn seed(id: &str) -> u64 {
    id.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// xorshift64* in 0..1.
pub(crate) fn next(x: &mut u64) -> f32 {
    *x |= 1;
    *x ^= *x >> 12;
    *x ^= *x << 25;
    *x ^= *x >> 27;
    (x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32 / (1u64 << 24) as f32
}

/// Resolves `beat`/`bar` steps to component seconds (frame-snapped beats).
pub(crate) fn resolve_steps(steps: &mut [CodeStep], beats: &[f64], start: f64, fps: f64) {
    for s in steps {
        let n = s.beat.or(s.bar.map(|b| b * 4));
        if let Some(t) = n.and_then(|n| beats.get(n as usize)) {
            s.at = (((t * fps).round() / fps - start) * 1e6).round() / 1e6;
            s.beat = None;
            s.bar = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_rust() {
        let t = tokenize("let x = \"hi\"; // note", Language::Rust);
        assert_eq!(t[0], (Tok::Keyword, "let ".into()));
        assert!(t
            .iter()
            .any(|r| r.0 == Tok::Str && r.1.starts_with("\"hi\"")));
        assert_eq!(t.last().unwrap().0, Tok::Comment);
        let joined: String = t.iter().map(|r| r.1.as_str()).collect();
        assert_eq!(joined, "let x = \"hi\"; // note");
    }
}
