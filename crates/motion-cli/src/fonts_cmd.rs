//! (0.9) `fonts list` / `fonts specimen` and the shared typography flags
//! (`--explore`, `--seed`, `--emotion`) used by `compile` and `resolve-style`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use motion_core::compiler::typography::{
    emotion_name, option_faces, option_fontset_in, option_ids, parse_emotion, role_name,
    validate_registry, Emotion, OptionReport,
};
use motion_core::compiler::CompileOptions;
use motion_core::scene::{Color, FontRole, TextAlign, TextStyle};
use motion_render::text::TextEngine;
use motion_render::tiny_skia::{Color as SkColor, FillRule, Paint, Pixmap, Rect, Transform};

fn emotion_arg(s: &str) -> std::result::Result<Emotion, String> {
    parse_emotion(s).ok_or_else(|| {
        format!(
            "unknown emotion '{s}' (one of: {})",
            Emotion::ALL.map(emotion_name).join(", ")
        )
    })
}

/// Operator typography flags shared by `compile` and `resolve-style`.
#[derive(Args, Debug, Clone)]
pub struct TypographyArgs {
    /// (0.9) Typography exploration level 0..=3 (0 = canonical 0.8 pairing).
    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=3))]
    pub explore: u8,
    /// (0.9) Exploration seed (independent of StyleProfile.seed).
    #[arg(long, default_value_t = 0)]
    pub seed: u64,
    /// (0.9) Force a typography emotion (snake_case, e.g. `trust`, `playful_retro`).
    #[arg(long, value_parser = emotion_arg)]
    pub emotion: Option<Emotion>,
}

impl TypographyArgs {
    pub fn options(&self) -> CompileOptions {
        CompileOptions {
            music_envelope: None,
            explore: self.explore,
            seed: self.seed,
            emotion: self.emotion,
            canvas: None,
            speech: None,
            footage: None,
            art: None,
            variety: None,
            no_captions: false,
            take: 0,
            candidates: 0,
        }
    }

    /// True when the flags can change the typography (otherwise output is 0.8).
    pub fn is_active(&self) -> bool {
        self.explore > 0 || self.emotion.is_some()
    }
}

#[derive(Subcommand)]
pub enum FontsCmd {
    /// List every registry face and emotion option with availability.
    List {
        #[arg(long, default_value = "assets")]
        assets: PathBuf,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Render a specimen sheet (PNG) of emotion options with the real text engine.
    Specimen {
        /// One emotion's options.
        #[arg(long, value_parser = emotion_arg, conflicts_with = "all", required_unless_present = "all")]
        emotion: Option<Emotion>,
        /// Every emotion's options.
        #[arg(long)]
        all: bool,
        #[arg(long, short)]
        output: PathBuf,
        #[arg(long, default_value = "assets")]
        assets: PathBuf,
    },
}

pub fn run(cmd: FontsCmd) -> Result<()> {
    match cmd {
        FontsCmd::List { assets, json } => cmd_list(&assets, json),
        FontsCmd::Specimen {
            emotion,
            all,
            output,
            assets,
        } => cmd_specimen(emotion, all, &output, &assets),
    }
}

fn cmd_list(assets: &Path, json: bool) -> Result<()> {
    let report = validate_registry(assets);
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("faces ({}):", report.faces.len());
        for f in &report.faces {
            let cov = match f.coverage {
                Some(c) => format!(
                    "latin={} digits={} inr={} eur={}",
                    c.latin as u8, c.digits as u8, c.inr as u8, c.eur as u8
                ),
                None => "coverage=unknown".to_string(),
            };
            println!(
                "  {:<34} {:<9} {:<26} w{}{} {}",
                f.id,
                if f.file_exists {
                    "available"
                } else {
                    "MISSING"
                },
                f.family.as_deref().unwrap_or("-"),
                f.weight,
                if f.italic { " italic" } else { "" },
                cov
            );
        }
        println!("options ({}):", report.options.len());
        for o in &report.options {
            println!(
                "  {:<18} {}{}",
                o.id,
                if o.available {
                    "available"
                } else {
                    "unavailable"
                },
                if o.problems.is_empty() {
                    String::new()
                } else {
                    format!("  ({})", o.problems.join("; "))
                }
            );
        }
    }
    for e in &report.errors {
        eprintln!("registry error: {e}");
    }
    if !report.errors.is_empty() {
        bail!("registry has {} error(s)", report.errors.len());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// specimen
// ---------------------------------------------------------------------------

const CELL_W: u32 = 1200;
const CELL_H: u32 = 490;
const PAD: f32 = 36.0;
const PAPER: (u8, u8, u8) = (0xEC, 0xE3, 0xD2);
const INK: Color = Color::rgb(0x17, 0x15, 0x13);
const MUTED: Color = Color::rgb(0x6F, 0x66, 0x5A);

fn text_style(text: &str, role: FontRole, size: f32, weight: u16, italic: bool) -> TextStyle {
    TextStyle {
        text: text.to_string(),
        font_role: role,
        font_size: size,
        font_weight: weight,
        italic,
        color: INK,
        align: TextAlign::Left,
        line_height: 1.2,
        letter_spacing: 0.0,
        max_width: None,
        uppercase: false,
        ink: None,
    }
}

fn paint(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(SkColor::from_rgba8(c.r, c.g, c.b, c.a));
    p.anti_alias = true;
    p
}

/// Draw one text line with its top at `y`; returns the next y.
fn draw_line(
    pm: &mut Pixmap,
    engine: &mut TextEngine,
    family: &str,
    style: &TextStyle,
    color: Color,
    x: f32,
    y: f32,
) -> f32 {
    if let Some(path) = engine.outline(family, style, CELL_W as f32) {
        pm.fill_path(
            &path,
            &paint(color),
            FillRule::Winding,
            Transform::from_translate(x, y),
            None,
        );
    }
    y + style.font_size * style.line_height
}

fn fill_rect(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, c: Color) {
    if let Some(r) = Rect::from_xywh(x, y, w, h) {
        pm.fill_rect(r, &paint(c), Transform::identity(), None);
    }
}

fn draw_block(
    pm: &mut Pixmap,
    labels: &mut TextEngine,
    label_family: &str,
    assets: &Path,
    opt: &OptionReport,
    ox: f32,
    oy: f32,
) {
    let x = ox + PAD;
    let head = format!("{}  /  {}", opt.id, emotion_name(opt.emotion));
    let label = text_style(&head, FontRole::Mono, 24.0, 400, false);
    let mut y = oy + 18.0;
    y = draw_line(pm, labels, label_family, &label, MUTED, x, y) + 10.0;

    let loaded = if opt.available {
        load_option(assets, &opt.id)
    } else {
        Err(anyhow::anyhow!("not available"))
    };
    let (mut engine, families) = match loaded {
        Ok(v) => v,
        Err(e) => {
            let msg = if opt.available {
                format!("unavailable: {e}")
            } else {
                let files: Vec<String> = opt
                    .problems
                    .iter()
                    .map(|p| {
                        p.strip_prefix("missing file fonts/")
                            .unwrap_or(p)
                            .trim_end_matches(".otf")
                            .to_string()
                    })
                    .collect();
                format!("unavailable: {}", files.join(", "))
            };
            fill_rect(
                pm,
                ox + PAD,
                y,
                CELL_W as f32 - 2.0 * PAD,
                90.0,
                Color::rgb(0xB4, 0xB4, 0xB4),
            );
            let s = text_style(&msg, FontRole::Mono, 20.0, 400, false);
            draw_line(
                pm,
                labels,
                label_family,
                &s,
                Color::rgb(0x33, 0x33, 0x33),
                x + 18.0,
                y + 30.0,
            );
            return;
        }
    };
    let Some(faces) = option_faces(&opt.id) else {
        return;
    };
    let rows: [(FontRole, &str, f32); 6] = [
        (FontRole::Display, "Quiet hours, loud results", 68.0),
        (
            FontRole::DisplayCondensed,
            "Quiet hours, loud results",
            60.0,
        ),
        (FontRole::SerifEmotional, "the long way home", 52.0),
        (
            FontRole::Body,
            "Shoppers returned 13% more often in Q3.",
            34.0,
        ),
        (FontRole::Number, "\u{20B9}50,000 \u{2212}13%", 52.0),
        (FontRole::Mono, "SOURCE \u{2014} RBI 2026", 28.0),
    ];
    for (role, text, size) in rows {
        let (Some(face), Some(family)) = (faces.get(&role), families.get(&role)) else {
            continue;
        };
        let s = text_style(text, role, size, face.weight, face.italic);
        y = draw_line(pm, &mut engine, family, &s, INK, x, y) + 6.0;
    }
    // Tiny role legend so the sheet is self-explanatory.
    let legend = FontRole::ALL
        .iter()
        .filter_map(|r| {
            faces.get(r).map(|f| {
                format!(
                    "{}={}",
                    role_name(*r),
                    f.asset_id.trim_start_matches("font.")
                )
            })
        })
        .collect::<Vec<_>>()
        .join("  ");
    let s = text_style(&legend, FontRole::Mono, 12.0, 400, false);
    draw_line(
        pm,
        labels,
        label_family,
        &s,
        MUTED,
        x,
        oy + CELL_H as f32 - 34.0,
    );
}

type Families = std::collections::BTreeMap<FontRole, String>;

/// A TextEngine holding the option's complete FontSet (role faces + fallback
/// + always-loaded), exactly what the renderer would load for a project.
fn load_option(assets: &Path, option_id: &str) -> Result<(TextEngine, Families)> {
    let set = option_fontset_in(option_id, assets).context("option has no complete FontSet")?;
    let mut engine = TextEngine::new();
    let mut by_asset = std::collections::HashMap::new();
    for face in &set.faces {
        let family = engine
            .load(&assets.join(face.path))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        by_asset.insert(face.asset_id, family);
    }
    let families = set
        .roles
        .iter()
        .filter_map(|(role, face)| by_asset.get(face.asset_id).map(|f| (*role, f.clone())))
        .collect();
    Ok((engine, families))
}

fn cmd_specimen(emotion: Option<Emotion>, all: bool, output: &Path, assets: &Path) -> Result<()> {
    let report = validate_registry(assets);
    let emotions: Vec<Emotion> = match (all, emotion) {
        (true, _) => Emotion::ALL.to_vec(),
        (false, Some(e)) => vec![e],
        (false, None) => bail!("pass --emotion <name> or --all"),
    };
    let wanted: Vec<&OptionReport> = emotions
        .iter()
        .flat_map(|e| option_ids(*e))
        .filter_map(|id| report.options.iter().find(|o| o.id == id))
        .collect();
    if wanted.is_empty() {
        bail!("no options to draw");
    }
    let cols: u32 = if wanted.len() <= 3 { 1 } else { 3 };
    let rows = (wanted.len() as u32).div_ceil(cols);
    let mut pm = Pixmap::new(CELL_W * cols, CELL_H * rows).context("specimen canvas too large")?;
    pm.fill(SkColor::from_rgba8(PAPER.0, PAPER.1, PAPER.2, 255));

    // Labels use a fixed, always-present mono face (the fallback face).
    let label_path = assets.join("fonts/IBMPlexMono-Regular.ttf");
    let mut labels = TextEngine::new();
    let label_family = labels
        .load(&label_path)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .with_context(|| format!("loading label font {}", label_path.display()))?;

    for (i, opt) in wanted.iter().enumerate() {
        let col = i as u32 % cols;
        let row = i as u32 / cols;
        let (ox, oy) = ((col * CELL_W) as f32, (row * CELL_H) as f32);
        draw_block(&mut pm, &mut labels, &label_family, assets, opt, ox, oy);
        // Cell separator.
        fill_rect(
            &mut pm,
            ox,
            oy + CELL_H as f32 - 2.0,
            CELL_W as f32,
            2.0,
            Color::rgb(0xCF, 0xC4, 0xAF),
        );
    }
    if let Some(dir) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    pm.save_png(output)
        .map_err(|e| anyhow::anyhow!("writing {}: {e}", output.display()))?;
    let unavailable = wanted.iter().filter(|o| !o.available).count();
    println!(
        "specimen: {} option block(s) ({} unavailable) -> {}",
        wanted.len(),
        unavailable,
        output.display()
    );
    Ok(())
}
