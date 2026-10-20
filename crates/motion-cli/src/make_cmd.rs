//! (v2 Phase 4) `make`: a brief or a CreativeIntent → bar plan on the
//! recipe track's beat grid (`motion-bridge`) → program → scene → per-beat
//! QA sheet → MP4 → critique, in one call. Gates before the full render:
//! validation, beat coverage, prompt echo, component overlap and the per-beat
//! sheet (a visible change on every beat, no near-blank beat); a failing plan
//! is re-planned once with another concept. Prints one JSON line and nothing
//! else.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use clap::Args;
use motion_bridge::{Clock, Content, MakeRecipe, ReelPlan};
use motion_core::audio::MusicPlan;
use motion_program::program::{Aspect, ComponentBody, Face, Fonts, LockupPreset, Palette};
use motion_program::MotionProgram;
use motion_render::tiny_skia::{IntSize, Pixmap};
use motion_render::{CpuRenderer, Renderer};
use serde_json::{json, Value};

use crate::{audio_cmd, program_cmd, Quality, RenderArgs};

#[derive(Args)]
pub struct MakeArgs {
    /// A short brief. Direction ("make a dynamic 15 s showreel, go all out")
    /// sets length/aspect/recipe and is never shown; content (quoted text,
    /// numbers, plain statements) is. Line 1 is the title when it is content.
    #[arg(long, conflicts_with = "intent", required_unless_present = "intent")]
    pub brief: Option<String>,
    /// A CreativeIntent JSON file instead of a brief.
    #[arg(long)]
    pub intent: Option<PathBuf>,
    /// showreel, kinetic_poster, chart_reveal, ledger_ink, dot_story, or top5_countdown
    /// (default: picked from the content).
    #[arg(long)]
    pub recipe: Option<String>,
    /// 16:9, 9:16 or 1:1 (also landscape, portrait, square).
    #[arg(long)]
    pub aspect: Option<String>,
    /// Target seconds (snapped to the beat grid; content always fits).
    #[arg(long)]
    pub duration: Option<f64>,
    /// Brand palette override: JSON `{"background":"#...","ink":"#...","accent":"#...","muted":"#..."}`
    /// or 4 hex values `#BG,#INK,#ACCENT,#MUTED`.
    #[arg(long)]
    pub palette: Option<String>,
    /// Font pairing override: JSON `{"display":"...","serif":"...","mono":"..."}`
    /// or `display,serif[,mono]` CSV.
    #[arg(long)]
    pub fonts: Option<String>,
    /// Closing lockup preset override: `spline_frame`, `monolith_split`,
    /// `orbital_seal`, `terminal_prompt`, or `editorial_badge`.
    #[arg(long)]
    pub ending: Option<String>,
    /// Badge text for the closing lockup (used by `editorial_badge`).
    #[arg(long)]
    pub ending_badge: Option<String>,
    /// Pipe- or comma-separated specs for the closing lockup (`monolith_split` / `terminal_prompt`).
    #[arg(long)]
    pub ending_specs: Option<String>,
    /// Bundled music track override (`driving_pulse`, `tech_pulse`, `cinematic_strings`,
    /// `bright_upbeat`, `retro_groove`, `warm_piano`, `calm_ambient`).
    #[arg(long)]
    pub music_track: Option<String>,
    /// Output folder (default: output/make/<name>).
    #[arg(long)]
    pub out_dir: Option<PathBuf>,
    #[arg(long, default_value = "assets")]
    pub assets: PathBuf,
    /// Stop after the per-beat sheet and critique (no MP4).
    #[arg(long)]
    pub no_render: bool,
    /// Half-size proxy MP4 (fast look).
    #[arg(long)]
    pub proxy: bool,
}

/// The per-beat look: one small frame per beat, compared in order.
struct BeatSheet {
    path: PathBuf,
    beats: usize,
    /// Beats whose frame barely differs from the previous beat's.
    still: Vec<usize>,
    /// Beats with almost nothing but ground on screen.
    blank: Vec<usize>,
}

impl BeatSheet {
    fn passes(&self) -> bool {
        self.blank.len() <= 1 && self.still.len() * 10 <= self.beats
    }
}

struct Attempt {
    program: MotionProgram,
    resolved: MotionProgram,
    plan: ReelPlan,
    project: motion_core::MotionProject,
    findings: Vec<String>,
    sheet: BeatSheet,
}

/// `--aspect`: the ratio (`16:9`, `9:16`, `1:1`) or its name (`landscape`,
/// `portrait`, `square`, any case).
fn parse_aspect(s: &str) -> Result<Aspect> {
    let ratio = match s.trim().to_ascii_lowercase().as_str() {
        "landscape" => "16:9".to_string(),
        "portrait" => "9:16".to_string(),
        "square" => "1:1".to_string(),
        other => other.to_string(),
    };
    serde_json::from_value::<Aspect>(json!(ratio)).with_context(|| {
        format!("--aspect {s}: use 16:9, 9:16 or 1:1 (or landscape, portrait, square)")
    })
}

pub fn run(a: MakeArgs) -> Result<()> {
    crate::log::set_level(true, 0);
    let aspect = match &a.aspect {
        Some(s) => Some(parse_aspect(s)?),
        None => None,
    };
    let content = match (&a.brief, &a.intent) {
        (Some(b), _) => motion_bridge::parse_brief(b),
        (None, Some(p)) => {
            let text =
                std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
            let intent = motion_core::CreativeIntent::from_json(&text)
                .with_context(|| format!("parsing intent {}", p.display()))?;
            motion_bridge::from_intent(&intent)
        }
        (None, None) => anyhow::bail!("give --brief or --intent"),
    };
    let recipe_name = motion_bridge::pick_recipe(&content, a.recipe.as_deref());
    let mut recipe = motion_bridge::recipe(&recipe_name).map_err(|e| anyhow!(e))?;
    if let Some(track) = a.music_track.as_deref().filter(|s| !s.trim().is_empty()) {
        recipe.music = track.trim().to_string();
    }
    let music = program_cmd::music_plan(&a.assets, &recipe.music)?;
    let clock = clock_of(&music);
    let dir = a
        .out_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from("output/make").join(&content.name));
    std::fs::create_dir_all(&dir)?;
    let name = content.name.clone();
    let scene = dir.join(format!("{name}.motion.json"));
    let ctx = Ctx {
        a: &a,
        content: &content,
        recipe: &recipe,
        music: &music,
        clock: &clock,
        aspect,
        scene: &scene,
    };

    // One re-plan with another concept when a gate fails.
    // A UI flow plays on fixed seconds (its steps are not on the beat grid
    // yet), so when it breaks beat coverage it is dropped and named.
    let mut dropped_flow = false;
    let mut at = match attempt(&ctx, 0) {
        Err(e) if content.flow.is_some() && e.to_string().contains("beat_coverage") => {
            dropped_flow = true;
            let no_flow = Content {
                flow: None,
                ..content.clone()
            };
            attempt(
                &Ctx {
                    content: &no_flow,
                    ..ctx
                },
                0,
            )?
        }
        r => r?,
    };
    if !at.passes() {
        at = attempt(&ctx, 1).or_else(|_| {
            let no_flow = Content {
                flow: None,
                ..content.clone()
            };
            attempt(
                &Ctx {
                    content: &no_flow,
                    ..ctx
                },
                1,
            )
        })?;
    }
    if dropped_flow {
        at.findings.insert(
            0,
            "flow_skipped: the UI flow's steps are not on the beat grid yet".into(),
        );
    }
    // The mistake ledger: QA -> auto-repair (<= 2 passes) on the planned
    // program, as `program render` does; what still fails is reported.
    let (fixed, ledger) = program_cmd::qa_loop(
        at.resolved.clone(),
        Some(music.clone()),
        &a.assets,
        None,
        motion_program::repair::MAX_PASSES,
    )?;
    if fixed != at.resolved {
        at.project = program_cmd::compile_to(&fixed, Some(music.clone()), &scene, &a.assets)?;
        at.resolved = fixed.clone();
        at.program = fixed;
    }
    let mut ids: Vec<&str> = ledger.iter().map(|f| f.id.as_str()).collect();
    ids.dedup();
    if !ids.is_empty() {
        at.findings.insert(0, format!("ledger: {}", ids.join(", ")));
    }
    let program_path = dir.join(format!("{name}.program.json"));
    std::fs::write(&program_path, at.program.to_json() + "\n")?;

    let c = motion_core::critique_with(&at.project, &motion_core::SlopRules::bundled(), None);
    // Gate findings first, then the per-beat sheet, then critique; one per check.
    let mut top: Vec<String> = at.findings.clone();
    if !at.sheet.still.is_empty() {
        top.push(format!(
            "beat_still: {} of {} beats show no visible change (beats {:?})",
            at.sheet.still.len(),
            at.sheet.beats,
            &at.sheet.still[..at.sheet.still.len().min(6)]
        ));
    }
    if !at.sheet.blank.is_empty() {
        top.push(format!(
            "beat_blank: near-blank at beats {:?}",
            at.sheet.blank
        ));
    }
    for f in &c.findings {
        if !top.iter().any(|t| t.starts_with(&format!("{}:", f.check))) {
            top.push(format!("{}: {}", f.check, f.detail));
        }
    }
    top.truncate(3);
    let video = if a.no_render {
        None
    } else {
        render(&scene, a.proxy)?
    };
    let kinds: std::collections::BTreeSet<&str> = at
        .resolved
        .components
        .iter()
        .map(|c| c.body.kind_name())
        .collect();
    let out = json!({
        "video": video.map(|p| p.display().to_string()),
        "program": program_path.display().to_string(),
        "sheet": at.sheet.path.display().to_string(),
        "recipe": recipe_name,
        "concept": at.plan.concept,
        "seconds": at.resolved.meta.duration,
        "bars": at.plan.bars.len() + 1,
        "kinds": kinds.len(),
        "qa": if ids.is_empty() { json!("pass") } else { json!(ids) },
        "critique": (f64::from(c.total) * 10.0).round() / 10.0,
        "top": top,
    });
    println!("{out}");
    Ok(())
}

/// The track's grid with beat 0 on its first downbeat.
fn clock_of(m: &MusicPlan) -> Clock {
    let start = m
        .downbeat_times
        .first()
        .or(m.beat_times.first())
        .copied()
        .unwrap_or(0.0);
    Clock {
        track_start: start,
        beats: m
            .beat_times
            .iter()
            .map(|t| t - start)
            .filter(|t| *t > -1e-6)
            .map(|t| t.max(0.0))
            .collect(),
    }
}

#[derive(Clone, Copy)]
struct Ctx<'a> {
    a: &'a MakeArgs,
    content: &'a Content,
    recipe: &'a MakeRecipe,
    music: &'a MusicPlan,
    clock: &'a Clock,
    aspect: Option<Aspect>,
    scene: &'a Path,
}

impl Attempt {
    fn passes(&self) -> bool {
        self.findings.iter().all(|f| !f.starts_with("gate:")) && self.sheet.passes()
    }
}

fn parse_palette_arg(raw: &str, base: Option<&Palette>) -> Result<Palette> {
    let s = raw.trim();
    if s.starts_with('{') {
        let v: Value = serde_json::from_str(s).with_context(|| format!("--palette JSON: {s}"))?;
        let d = base.cloned().unwrap_or(Palette {
            background: "#0C0E14".into(),
            ink: "#F4F1EA".into(),
            accent: "#FF4D00".into(),
            muted: "#7A8194".into(),
            grounds: Vec::new(),
        });
        let str_or = |k: &str, def: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .filter(|x| !x.is_empty())
                .unwrap_or(def)
                .to_string()
        };
        let grounds = v
            .get("grounds")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or(d.grounds);
        return Ok(Palette {
            background: str_or("background", &d.background),
            ink: str_or("ink", &d.ink),
            accent: str_or("accent", &d.accent),
            muted: str_or("muted", &d.muted),
            grounds,
        });
    }
    let parts: Vec<&str> = s
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() != 4 {
        anyhow::bail!(
            "--palette '{s}': expected JSON object or 4 hex values '#BG,#INK,#ACCENT,#MUTED'"
        );
    }
    Ok(Palette {
        background: parts[0].to_string(),
        ink: parts[1].to_string(),
        accent: parts[2].to_string(),
        muted: parts[3].to_string(),
        grounds: base.map(|b| b.grounds.clone()).unwrap_or_default(),
    })
}

fn parse_fonts_arg(raw: &str, base: Fonts) -> Result<Fonts> {
    let s = raw.trim();
    if s.starts_with('{') {
        let v: Value = serde_json::from_str(s).with_context(|| format!("--fonts JSON: {s}"))?;
        let face_or = |k: &str, def: Face| -> Result<Face> {
            match v.get(k).and_then(Value::as_str).filter(|x| !x.is_empty()) {
                Some(name) => serde_json::from_value::<Face>(json!(name))
                    .with_context(|| format!("unknown font face '{name}'")),
                None => Ok(def),
            }
        };
        return Ok(Fonts {
            display: face_or("display", base.display)?,
            serif: face_or("serif", base.serif)?,
            mono: face_or("mono", base.mono)?,
        });
    }
    let parts: Vec<&str> = s
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() || parts.len() > 3 {
        anyhow::bail!("--fonts '{s}': expected JSON or 'display[,serif[,mono]]'");
    }
    let parse_face = |name: &str| -> Result<Face> {
        serde_json::from_value::<Face>(json!(name))
            .with_context(|| format!("unknown font face '{name}'"))
    };
    Ok(Fonts {
        display: parse_face(parts[0])?,
        serif: if parts.len() >= 2 {
            parse_face(parts[1])?
        } else {
            base.serif
        },
        mono: if parts.len() >= 3 {
            parse_face(parts[2])?
        } else {
            base.mono
        },
    })
}

fn apply_overrides(p: &mut MotionProgram, a: &MakeArgs) -> Result<()> {
    if let Some(raw_pal) = a.palette.as_deref().filter(|s| !s.trim().is_empty()) {
        let current = p.palette();
        p.palette = Some(parse_palette_arg(raw_pal, current.as_ref())?);
    }
    if let Some(raw_fonts) = a.fonts.as_deref().filter(|s| !s.trim().is_empty()) {
        let current = p.fonts();
        p.fonts = Some(parse_fonts_arg(raw_fonts, current)?);
    }
    let preset = match a.ending.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(name) => Some(
            serde_json::from_value::<LockupPreset>(json!(name.trim())).with_context(|| {
                format!(
                    "--ending '{name}': use spline_frame, monolith_split, orbital_seal, terminal_prompt, or editorial_badge"
                )
            })?,
        ),
        None => None,
    };
    let badge = a
        .ending_badge
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let specs: Option<Vec<String>> = a
        .ending_specs
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            let sep = if s.contains('|') { '|' } else { ',' };
            s.split(sep)
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .take(4)
                .map(|x| x.chars().take(36).collect())
                .collect()
        });

    if preset.is_some() || badge.is_some() || specs.is_some() {
        for c in &mut p.components {
            if let ComponentBody::Lockup(l) = &mut c.body {
                if let Some(pr) = preset {
                    l.preset = pr;
                }
                if let Some(b) = &badge {
                    l.badge = Some(b.chars().take(36).collect());
                } else if l.preset == LockupPreset::EditorialBadge && l.badge.is_none() {
                    l.badge = Some("EDITION // 2026".to_string());
                }
                if let Some(sp) = &specs {
                    l.specs = sp.clone();
                } else if matches!(
                    l.preset,
                    LockupPreset::MonolithSplit | LockupPreset::TerminalPrompt
                ) && l.specs.is_empty()
                {
                    l.specs = vec!["STATUS // VERIFIED".to_string()];
                }
            }
        }
    }
    Ok(())
}

fn attempt(x: &Ctx, variant: u64) -> Result<Attempt> {
    let plan = motion_bridge::plan(x.content, x.recipe, x.clock, x.a.duration, variant);
    let mut program = motion_bridge::plan::realize(
        x.content,
        x.recipe,
        &plan,
        x.clock,
        x.aspect.or(x.content.aspect),
    )
    .map_err(|e| anyhow!(e))?;
    apply_overrides(&mut program, x.a)?;
    let mut resolved = program.clone();
    resolved.resolve_beats(&x.clock.beats);
    let mut findings = Vec::new();
    let shift = |v: &[f64]| -> Vec<f64> {
        v.iter()
            .map(|t| t - x.clock.track_start)
            .filter(|t| *t > -1e-6)
            .collect()
    };
    let cov = motion_program::qa::beat_coverage(
        &resolved,
        &shift(&x.music.beat_times),
        &shift(&x.music.downbeat_times),
    );
    if !cov.passes() {
        findings.push(format!("gate: {}", cov.report()));
    }
    let echo = motion_bridge::prompt_echo(x.content, &resolved);
    if !echo.is_empty() {
        findings.push(format!("gate: prompt words on screen: {}", echo.join(", ")));
    }
    let project = program_cmd::compile_to(&resolved, Some(x.music.clone()), x.scene, &x.a.assets)?;
    // The core's rendered-text checks: brief words and meta strings.
    let echo = motion_program::qa::prompt_echo(&resolved, &project);
    if !echo.is_empty() {
        findings.push(format!("gate: brief words on screen: {}", echo.join(", ")));
    }
    let meta = motion_program::qa::meta_strings(&resolved, &project);
    if !meta.is_empty() {
        findings.push(format!("gate: meta text on screen: {}", meta.join(", ")));
    }
    for o in motion_program::qa::component_overlap(&resolved, &project) {
        findings.push(format!(
            "overlap: {} and {} share {:.0}%",
            o.a,
            o.b,
            o.share * 100.0
        ));
    }
    let sheet = beat_sheet(&project, x.scene, x.clock, plan.final_downbeat())?;
    Ok(Attempt {
        program,
        resolved,
        plan,
        project,
        findings,
        sheet,
    })
}

const TILE_W: u32 = 240;
const COLS: u32 = 8;

/// Renders one frame per beat (60 % into the beat, after the cut has
/// settled), tiles them into `<stem>.beats.png` and measures change and ink.
fn beat_sheet(
    project: &motion_core::MotionProject,
    scene: &Path,
    clock: &Clock,
    final_beat: usize,
) -> Result<BeatSheet> {
    let base = scene.parent().unwrap_or(Path::new("."));
    let renderer = CpuRenderer::new(project, base)?;
    let fps = f64::from(project.canvas.fps);
    let (w, h) = (project.canvas.width, project.canvas.height);
    let tile_h = (TILE_W * h / w).max(1);
    let n = final_beat + 1;
    let rows = (n as u32).div_ceil(COLS);
    let mut sheet = vec![0u8; (TILE_W * COLS * tile_h * rows * 4) as usize];
    let mut prev: Option<Vec<u8>> = None;
    let mut ground: Option<[u8; 3]> = None;
    let (mut still, mut blank) = (Vec::new(), Vec::new());
    let last = project.frame_count().saturating_sub(1);
    let px = (TILE_W * tile_h) as usize;
    let differs =
        |a: &[u8], b: &[u8], p: usize| (0..3).any(|c| a[p * 4 + c].abs_diff(b[p * 4 + c]) > 24);
    for i in 0..n {
        let t = clock.beats.get(i).copied().unwrap_or(0.0) + clock.beat_len() * 0.6;
        let frame = ((t * fps).round() as u32).min(last);
        let resolved = motion_core::evaluate_frame(project, frame)?;
        let pm = renderer.render(&resolved)?;
        let tile = downscale(pm.data(), w, h, TILE_W, tile_h);
        let g = *ground.get_or_insert([tile[0], tile[1], tile[2]]);
        let ink = (0..px)
            .filter(|p| (0..3).any(|c| tile[p * 4 + c].abs_diff(g[c]) > 24))
            .count();
        if ink * 200 < px {
            blank.push(i);
        }
        if let Some(prev) = &prev {
            if (0..px).filter(|p| differs(&tile, prev, *p)).count() * 200 < px {
                still.push(i);
            }
        }
        let (cx, cy) = (i as u32 % COLS, i as u32 / COLS);
        let row = (TILE_W * 4) as usize;
        for y in 0..tile_h {
            let src = (y * TILE_W * 4) as usize;
            let dst = (((cy * tile_h + y) * TILE_W * COLS + cx * TILE_W) * 4) as usize;
            sheet[dst..dst + row].copy_from_slice(&tile[src..src + row]);
        }
        prev = Some(tile);
    }
    let size = IntSize::from_wh(TILE_W * COLS, tile_h * rows).ok_or(anyhow!("sheet size"))?;
    let pm = Pixmap::from_vec(sheet, size).ok_or(anyhow!("sheet pixmap"))?;
    let name = scene.file_name().unwrap_or_default().to_string_lossy();
    let stem = name.strip_suffix(".motion.json").unwrap_or(&name);
    let path = scene.with_file_name(format!("{stem}.beats.png"));
    pm.save_png(&path)
        .map_err(|e| anyhow!("writing sheet: {e}"))?;
    Ok(BeatSheet {
        path,
        beats: n,
        still,
        blank,
    })
}

/// Box-averaged RGBA downscale (every other source pixel; opaque frames).
fn downscale(src: &[u8], w: u32, h: u32, tw: u32, th: u32) -> Vec<u8> {
    let mut out = vec![0u8; (tw * th * 4) as usize];
    for ty in 0..th {
        let (y0, y1) = (ty * h / th, ((ty + 1) * h / th).max(ty * h / th + 1));
        for tx in 0..tw {
            let (x0, x1) = (tx * w / tw, ((tx + 1) * w / tw).max(tx * w / tw + 1));
            let mut acc = [0u32; 4];
            let mut k = 0u32;
            for y in (y0..y1).step_by(2) {
                for x in (x0..x1).step_by(2) {
                    let i = ((y * w + x) * 4) as usize;
                    for (c, v) in acc.iter_mut().enumerate() {
                        *v += u32::from(src[i + c]);
                    }
                    k += 1;
                }
            }
            let o = ((ty * tw + tx) * 4) as usize;
            for (c, v) in acc.iter().enumerate() {
                out[o + c] = (v / k.max(1)) as u8;
            }
        }
    }
    out
}

fn render(scene: &Path, proxy: bool) -> Result<Option<PathBuf>> {
    let quality = if proxy {
        Quality::Proxy
    } else {
        Quality::Delivery
    };
    let mp4 = crate::cmd_render(
        scene,
        RenderArgs {
            out_dir: None,
            frame: None,
            no_video: false,
            keep_frames: false,
            gpu: false,
            brand_mark: None,
            quality,
            jobs: None,
        },
    )?;
    if let Some(mp4) = &mp4 {
        audio_cmd::mix_after_render(
            scene,
            mp4,
            &audio_cmd::RenderAudio {
                sfx_library: None,
                audio_plan: None,
                makeup: None,
                speech: None,
                music: None,
            },
        )?;
    }
    Ok(mp4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspect_accepts_ratios_and_names() {
        assert_eq!(parse_aspect("16:9").unwrap(), Aspect::Landscape);
        assert_eq!(parse_aspect("9:16").unwrap(), Aspect::Portrait);
        assert_eq!(parse_aspect("1:1").unwrap(), Aspect::Square);
        assert_eq!(parse_aspect("square").unwrap(), Aspect::Square);
        assert_eq!(parse_aspect("portrait").unwrap(), Aspect::Portrait);
        assert_eq!(parse_aspect("landscape").unwrap(), Aspect::Landscape);
        assert_eq!(parse_aspect(" Square ").unwrap(), Aspect::Square);
    }

    #[test]
    fn aspect_rejects_unknown_with_the_accepted_forms() {
        let e = parse_aspect("wide").unwrap_err().to_string();
        assert!(e.contains("square"), "{e}");
        assert!(parse_aspect("4:3").is_err());
    }
}
