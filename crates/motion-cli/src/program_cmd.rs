//! (v2) `program validate|compile`: MotionEngine 2 programs
//! (`crates/motion-program`). `compile` writes a MotionScene that the usual
//! `render`, `inspect` and QA commands take, plus `<stem>.audio.json` (the
//! music bed) when the program names a track, so `render` mixes it.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::Subcommand;
use motion_core::audio::{
    AudioPlan, MusicBed, MusicPlan, AUDIO_PLAN_VERSION, LOUDNESS_TARGET_LUFS, TRUE_PEAK_LIMIT_DB,
};
use motion_program::program::{Aspect, GenKind};
use motion_program::MotionProgram;
use motion_providers::lockfile::{lock_path, resolve_all, LockAction, Lockfile};
use motion_providers::ImageSpec;

/// (v2.3) Generated asset id → absolute file (from the assets lockfile).
pub type GenPaths = std::collections::BTreeMap<String, String>;

#[derive(Subcommand)]
pub enum ProgramCmd {
    /// v2 QA: beat clock, overlap, fill, text contrast, prompt echo, and
    /// (rendered, one frame per beat) visible change and ink coverage.
    /// `--sheet` writes the per-beat contact sheet. Exit 1 on a failure.
    Qa {
        program: PathBuf,
        #[arg(long)]
        aspect: Option<String>,
        /// Per-beat contact sheet PNG (one small frame per beat).
        #[arg(long)]
        sheet: Option<PathBuf>,
        #[arg(long, default_value = "assets")]
        assets: PathBuf,
        /// Run the auto-repair loop (at most 2 passes) and write
        /// output/<name>.repaired.program.json.
        #[arg(long)]
        repair: bool,
    },
    /// QA → auto-repair (≤ 2 passes) → render; refuses to render on a
    /// remaining failure and prints `fail: [ids]`.
    Render {
        program: PathBuf,
        #[arg(long)]
        aspect: Option<String>,
        #[arg(long, default_value = "assets")]
        assets: PathBuf,
        /// Do not generate: every `assets` entry must already be in the lockfile.
        #[arg(long)]
        offline: bool,
    },
    /// (v2.3) Resolve the program's generated `assets` (lockfile → cache →
    /// provider fallback, MOSS last) and write `<stem>.assets.lock.json`
    /// beside the program. `--offline` only checks the lockfile.
    Assets {
        program: PathBuf,
        #[arg(long)]
        offline: bool,
        /// Asset library root (music analyses for `beat` references).
        #[arg(long, default_value = "assets")]
        assets: PathBuf,
    },
    /// Add a new mistake to the ledger: an entry in assets/qa/mistakes.json
    /// and a fixture stub in assets/qa/fixtures/<id>.json.
    Learn {
        #[arg(long)]
        id: String,
        /// Where the defect was seen: a frame path or `video.mp4@12.5` (recorded, not copied).
        #[arg(long)]
        from: String,
        #[arg(long)]
        note: String,
        /// `program`, `scene` or `rendered`.
        #[arg(long, default_value = "rendered")]
        area: String,
    },
    /// Check a program: vocabulary, timing, verb fit, channel overlaps, bans.
    Validate {
        program: PathBuf,
        /// Asset library root (music analyses for `beat` references).
        #[arg(long, default_value = "assets")]
        assets: PathBuf,
    },
    /// Compile a program to a MotionScene JSON.
    Compile {
        program: PathBuf,
        /// Output scene (default: output/<name>[_<aspect>].motion.json).
        #[arg(long, short)]
        output: Option<PathBuf>,
        /// Override the program's aspect: 16:9, 9:16 or 1:1.
        #[arg(long)]
        aspect: Option<String>,
        /// Asset library root (contains fonts/ and music/).
        #[arg(long, default_value = "assets")]
        assets: PathBuf,
    },
}

/// (v2.3) The resolved generated assets of the program at `path`, read
/// from its lockfile (never a provider call). Empty without `assets`.
pub fn gen_paths(p: &MotionProgram, path: &Path) -> Result<GenPaths> {
    if p.assets.is_empty() {
        return Ok(GenPaths::new());
    }
    Ok(Lockfile::load(&lock_path(path))?.paths())
}

/// Parses, resolves `beat` references against the track's analysis and
/// validates (generated assets included: they must be in the lockfile).
fn load(path: &Path, assets: &Path) -> Result<(MotionProgram, Option<MusicPlan>, GenPaths)> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut p = MotionProgram::from_json(&text)
        .with_context(|| format!("parsing program {}", path.display()))?;
    let music = match &p.music {
        Some(m) => {
            let file = assets.join("music").join(format!("{}.music.json", m.track));
            let plan: MusicPlan = serde_json::from_str(
                &std::fs::read_to_string(&file)
                    .with_context(|| format!("music analysis {}", file.display()))?,
            )
            .with_context(|| format!("parsing {}", file.display()))?;
            let beats: Vec<f64> = plan
                .beat_times
                .iter()
                .map(|t| t - m.start)
                .filter(|t| *t >= 0.0)
                .collect();
            p.resolve_beats(&beats);
            Some(plan)
        }
        None => None,
    };
    if let Err(e) = motion_program::validate(&p) {
        bail!("{} is not a valid program:\n{e}", path.display());
    }
    let gen = gen_paths(&p, path)?;
    if let Err(e) = motion_program::validate::check_gen_resolved(&p, &gen) {
        bail!("{} is not a valid program:\n{e}", path.display());
    }
    Ok((p, music, gen))
}

/// A music-led program keeps landing on beats to its final downbeat, and no
/// beat plays after it (`qa::beat_coverage`).
pub fn beat_gate(p: &MotionProgram, plan: &MusicPlan) -> Result<f64> {
    let m = p.music.as_ref().expect("music");
    let shift = |v: &[f64]| -> Vec<f64> {
        v.iter()
            .map(|t| t - m.start)
            .filter(|t| *t >= 0.0)
            .collect()
    };
    let cov = motion_program::qa::beat_coverage(
        p,
        &shift(&plan.beat_times),
        &shift(&plan.downbeat_times),
    );
    println!("{}", cov.report());
    if !cov.passes() {
        bail!("beat_coverage failed: {}", cov.report());
    }
    Ok(motion_program::qa::event_times(p)
        .last()
        .copied()
        .unwrap_or(p.meta.duration))
}

/// Lowers a validated program to `output` (a MotionScene) and, when it names
/// a track, writes the bed-only `<stem>.audio.json` beside it.
pub fn compile_to(
    p: &MotionProgram,
    music: Option<MusicPlan>,
    output: &Path,
    assets: &Path,
) -> Result<motion_core::MotionProject> {
    compile_to_gen(p, music, output, assets, &GenPaths::new())
}

/// [`compile_to`] with generated assets: `gen:<id>` image paths point at
/// their files (absolute paths in the user cache).
pub fn compile_to_gen(
    p: &MotionProgram,
    music: Option<MusicPlan>,
    output: &Path,
    assets: &Path,
    gen: &GenPaths,
) -> Result<motion_core::MotionProject> {
    // Music-led: gate the beat clock; the bed ends on the final hit.
    let final_hit = match (&p.music, &music) {
        (Some(_), Some(plan)) => Some(beat_gate(p, plan)?),
        _ => None,
    };
    let out_dir = output
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(out_dir)?;
    let out_abs = out_dir.canonicalize()?;
    let assets_abs = assets
        .canonicalize()
        .with_context(|| format!("assets dir {}", assets.display()))?;
    let rel = pathdiff::diff_paths(&assets_abs, &out_abs).unwrap_or(assets_abs.clone());
    let rel = rel.to_string_lossy().replace('\\', "/");
    // Type is measured with the renderer's own shaping.
    let measure = font_measure(p, &assets_abs)?;
    let ts = motion_program::Typesetter {
        measure: &measure,
        assets: assets_abs.clone(),
    };
    // The palette resolver re-derives what breaks the colour rules; say what.
    if let Some(pal) = p.palette() {
        for r in motion_program::palette::resolve(&pal).repairs {
            println!("palette: {r}");
        }
    }
    let mut project = motion_program::lower(p, Some(rel.clone()), &ts);
    let missing = motion_program::lower::resolve_gen_paths(&mut project, gen);
    if !missing.is_empty() {
        bail!(
            "generated asset(s) {} not resolved; run `program assets`",
            missing.join(", ")
        );
    }
    motion_core::validate(&project, Some(&out_abs))
        .map_err(|e| anyhow::anyhow!("lowered scene failed validation:\n{e}"))?;
    // No content element may sit on another (shared layout rule).
    let overlaps = motion_program::qa::component_overlap(p, &project);
    if !overlaps.is_empty() {
        let list: Vec<String> = overlaps
            .iter()
            .map(|o| {
                format!(
                    "{} x {} ({:.0}% at {:.2}s)",
                    o.a,
                    o.b,
                    o.share * 100.0,
                    o.at
                )
            })
            .collect();
        bail!("component_overlap: {}", list.join(", "));
    }
    std::fs::write(output, project.to_json_pretty() + "\n")?;
    if let (None, Some(sd)) = (&p.music, &p.sound) {
        let audio = sound_plan(p, sd, assets, &rel)?;
        let name = output.file_name().unwrap_or_default().to_string_lossy();
        let stem = name.strip_suffix(".motion.json").unwrap_or(&name);
        let audio_path = output.with_file_name(format!("{stem}.audio.json"));
        std::fs::write(&audio_path, serde_json::to_string_pretty(&audio)? + "\n")?;
    }
    if let (Some(m), Some(plan)) = (&p.music, music) {
        let audio = AudioPlan {
            version: AUDIO_PLAN_VERSION.to_string(),
            cues: Vec::new(),
            loudness_target: LOUDNESS_TARGET_LUFS,
            true_peak_limit: TRUE_PEAK_LIMIT_DB,
            min_spacing: 0.0,
            information_cap: 0,
            music: Some(MusicBed {
                track: format!("{rel}/music/{}", plan.track),
                gain_db: plan.gain_db,
                fade_in: 0.0,
                // Fade from the final hit to the end: no beat plays on.
                fade_out: final_hit.map_or(1.0, |h| (p.meta.duration - h).max(0.2)),
                duck: false,
                start: m.start,
                fade_out_at: Some(p.meta.duration),
            }),
            speech_adjustments: Vec::new(),
            levels: None,
        };
        let name = output.file_name().unwrap_or_default().to_string_lossy();
        let stem = name.strip_suffix(".motion.json").unwrap_or(&name);
        let audio_path = output.with_file_name(format!("{stem}.audio.json"));
        std::fs::write(&audio_path, serde_json::to_string_pretty(&audio)? + "\n")?;
    }
    Ok(project)
}

/// The analysis of a bundled track.
pub fn music_plan(assets: &Path, track: &str) -> Result<MusicPlan> {
    let file = assets.join("music").join(format!("{track}.music.json"));
    serde_json::from_str(
        &std::fs::read_to_string(&file)
            .with_context(|| format!("music analysis {}", file.display()))?,
    )
    .with_context(|| format!("parsing {}", file.display()))
}

/// Rendered per-beat checks (and the optional sheet) for a compiled scene.
/// Returns `(beats without visible change, near-blank beats)`. Type and
/// colour are judged by the design detectors ([`design_gates`]).
pub fn beat_frames(
    scene: &motion_core::MotionProject,
    base: &Path,
    times: &[f64],
    sheet: Option<&Path>,
) -> Result<(Vec<f64>, Vec<f64>)> {
    use motion_render::tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform};
    use motion_render::Renderer;
    let r = motion_render::CpuRenderer::new(scene, base).map_err(|e| anyhow::anyhow!("{e}"))?;
    let fps = scene.canvas.fps as f64;
    let last = scene.frame_count().saturating_sub(1);
    let sample = |pm: &Pixmap| -> Vec<[u8; 3]> {
        let (w, h) = (pm.width(), pm.height());
        let mut v = Vec::new();
        for y in (0..h).step_by(12) {
            for x in (0..w).step_by(12) {
                let px = pm.pixel(x, y).expect("in bounds").demultiply();
                v.push([px.red(), px.green(), px.blue()]);
            }
        }
        v
    };
    let (tw, th) = (
        240u32,
        (240.0 * scene.canvas.height as f32 / scene.canvas.width as f32) as u32,
    );
    let cols = 8u32.min(times.len().max(1) as u32);
    let rows = (times.len() as u32).div_ceil(cols).max(1);
    let mut board = sheet.and_then(|_| Pixmap::new(tw * cols, th * rows));
    let (mut still, mut blank) = (Vec::new(), Vec::new());
    let mut prev: Option<Vec<[u8; 3]>> = None;
    for (i, &t) in times.iter().enumerate() {
        let f = ((t * fps).round() as u32 + 1).min(last);
        let rf = motion_core::evaluate_frame(scene, f).map_err(|e| anyhow::anyhow!("{e}"))?;
        let pm = r.render(&rf).map_err(|e| anyhow::anyhow!("{e}"))?;
        let s = sample(&pm);
        // Ink: share of samples away from the frame's most common colour.
        let mut counts: std::collections::HashMap<[u8; 3], usize> = Default::default();
        for px in &s {
            *counts.entry([px[0] / 8, px[1] / 8, px[2] / 8]).or_default() += 1;
        }
        let mode = counts.values().copied().max().unwrap_or(0);
        if (s.len() - mode) as f64 / (s.len().max(1) as f64) < 0.02 {
            blank.push(t);
        }
        if let Some(p0) = &prev {
            let diff = p0
                .iter()
                .zip(&s)
                .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 24))
                .count();
            if (diff as f64) / (s.len().max(1) as f64) < 0.003 {
                still.push(t);
            }
        }
        prev = Some(s);
        if let Some(b) = board.as_mut() {
            let (cx, cy) = ((i as u32 % cols) * tw, (i as u32 / cols) * th);
            let k = tw as f32 / pm.width() as f32;
            b.draw_pixmap(
                0,
                0,
                pm.as_ref(),
                &PixmapPaint {
                    quality: FilterQuality::Bilinear,
                    ..Default::default()
                },
                Transform::from_row(k, 0.0, 0.0, k, cx as f32, cy as f32),
                None,
            );
        }
    }
    if let (Some(b), Some(path)) = (board, sheet) {
        b.save_png(path).map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    Ok((still, blank))
}

/// The renderer's text measure for a program's fonts.
fn font_measure(p: &MotionProgram, assets_abs: &Path) -> Result<motion_render::FontMeasure> {
    let fonts = p.fonts();
    let faces: Vec<PathBuf> = [fonts.display, fonts.serif, fonts.mono]
        .iter()
        .map(|f| assets_abs.join(f.asset().1))
        .collect();
    motion_render::FontMeasure::with_fonts(faces.iter().map(PathBuf::as_path))
        .map_err(|e| anyhow::anyhow!("loading fonts: {e}"))
}

/// The design-system detectors (`motion_program::qa::design`, ledger
/// entries with repair `design_system`) on frames rendered at 2 fps.
pub fn design_gates(
    p: &MotionProgram,
    scene: &motion_core::MotionProject,
    base: &Path,
) -> Result<Vec<motion_program::ledger::GateFail>> {
    use motion_render::Renderer;
    let r = motion_render::CpuRenderer::new(scene, base).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut render = |rf: &motion_core::timeline::ResolvedFrame<'_>| {
        let pm = r.render(rf).map_err(|e| e.to_string())?;
        let mut rgb = Vec::with_capacity((pm.width() * pm.height() * 3) as usize);
        for px in pm.pixels() {
            let c = px.demultiply();
            rgb.extend_from_slice(&[c.red(), c.green(), c.blue()]);
        }
        Ok(motion_program::qa::design::Pixels {
            w: pm.width(),
            h: pm.height(),
            rgb,
        })
    };
    motion_program::qa::design::run(p, scene, 2.0, &mut render).map_err(|e| anyhow::anyhow!("{e}"))
}

pub fn run(cmd: ProgramCmd) -> Result<()> {
    match cmd {
        ProgramCmd::Qa {
            program,
            aspect,
            sheet,
            assets,
            repair,
        } => {
            let (mut p, music) = load_raw(&program, &assets)?;
            let gen = gen_paths(&p, &program)?;
            set_aspect(&mut p, aspect.as_deref())?;
            let passes = if repair {
                motion_program::repair::MAX_PASSES
            } else {
                0
            };
            let (p, fails) = qa_loop_gen(p, music, &assets, sheet.as_deref(), passes, &gen)?;
            report(&fails);
            if repair {
                let out = PathBuf::from(format!("output/{}.repaired.program.json", p.meta.name));
                std::fs::create_dir_all("output")?;
                std::fs::write(&out, p.to_json() + "\n")?;
            }
            if fails.is_empty() {
                Ok(())
            } else {
                bail!("v2 qa failed")
            }
        }
        ProgramCmd::Render {
            program,
            aspect,
            assets,
            offline,
        } => {
            let (mut p, music) = load_raw(&program, &assets)?;
            let suffix = aspect
                .as_deref()
                .map(|a| format!("_{}", a.replace(':', "x")))
                .unwrap_or_default();
            set_aspect(&mut p, aspect.as_deref())?;
            // Validated before anything is generated; plates follow --aspect.
            let gen = resolve_assets(&program, &p, offline)?;
            let (p, fails) = qa_loop_gen(
                p,
                music.clone(),
                &assets,
                None,
                motion_program::repair::MAX_PASSES,
                &gen,
            )?;
            report(&fails);
            if !fails.is_empty() {
                bail!("not rendered: QA failed after repair");
            }
            let out = PathBuf::from(format!("output/{}{suffix}.motion.json", p.meta.name));
            compile_to_gen(&p, music, &out, &assets, &gen)?;
            let exe = std::env::current_exe()?;
            let mut cmd = std::process::Command::new(exe);
            cmd.args(["render", "-q"]).arg(&out);
            if let Some(sd) = &p.sound {
                if !sd.hits.is_empty() {
                    cmd.arg("--sfx-library")
                        .arg(assets.join("sfx").join("library"));
                }
                // Authored sound is levelled to the plan's loudness target.
                cmd.args(["--makeup-gain", "auto"]);
            }
            let status = cmd.status()?;
            if !status.success() {
                bail!("render failed");
            }
            println!(
                "rendered {}",
                out.with_extension("").with_extension("").display()
            );
            Ok(())
        }
        ProgramCmd::Learn {
            id,
            from,
            note,
            area,
        } => learn(&id, &from, &note, &area),
        ProgramCmd::Assets {
            program,
            offline,
            assets,
        } => {
            let (p, _) = load_raw(&program, &assets)?;
            resolve_assets(&program, &p, offline)?;
            Ok(())
        }
        ProgramCmd::Validate { program, assets } => {
            let (p, music, _) = load(&program, &assets)?;
            if let Some(plan) = &music {
                beat_gate(&p, plan)?;
            }
            println!(
                "ok: {} component(s), {}s",
                p.components.len(),
                p.meta.duration
            );
            Ok(())
        }
        ProgramCmd::Compile {
            program,
            output,
            aspect,
            assets,
        } => {
            let (mut p, music, gen) = load(&program, &assets)?;
            let suffix = match aspect {
                Some(a) => {
                    p.meta.aspect =
                        serde_json::from_value::<Aspect>(serde_json::Value::String(a.clone()))
                            .with_context(|| format!("--aspect {a}: use 16:9, 9:16 or 1:1"))?;
                    format!("_{}", a.replace(':', "x"))
                }
                None => String::new(),
            };
            let output = output.unwrap_or_else(|| {
                PathBuf::from(format!("output/{}{suffix}.motion.json", p.meta.name))
            });
            let project = compile_to_gen(&p, music, &output, &assets, &gen)?;
            println!(
                "compiled {} component(s), {} frames -> {}",
                p.components.len(),
                project.frame_count(),
                output.display()
            );
            Ok(())
        }
    }
}

fn r2(v: &[f64]) -> Vec<f64> {
    v.iter().map(|t| (t * 100.0).round() / 100.0).collect()
}

fn set_aspect(p: &mut MotionProgram, aspect: Option<&str>) -> Result<()> {
    if let Some(a) = aspect {
        p.meta.aspect = serde_json::from_value::<Aspect>(serde_json::Value::String(a.to_string()))
            .with_context(|| format!("--aspect {a}: use 16:9, 9:16 or 1:1"))?;
    }
    Ok(())
}

/// Parse + resolve beats (no validation: the QA loop validates).
fn load_raw(path: &Path, assets: &Path) -> Result<(MotionProgram, Option<MusicPlan>)> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut p = MotionProgram::from_json(&text)
        .with_context(|| format!("parsing program {}", path.display()))?;
    let music = match &p.music {
        Some(m) => {
            let plan = music_plan(assets, &m.track)?;
            let beats: Vec<f64> = plan
                .beat_times
                .iter()
                .map(|t| t - m.start)
                .filter(|t| *t >= 0.0)
                .collect();
            p.resolve_beats(&beats);
            Some(plan)
        }
        None => None,
    };
    Ok((p, music))
}

fn report(fails: &[motion_program::ledger::GateFail]) {
    if fails.is_empty() {
        println!("qa: pass");
        return;
    }
    let mut ids: Vec<&str> = fails.iter().map(|f| f.id.as_str()).collect();
    ids.dedup();
    let mut uniq: Vec<&str> = Vec::new();
    for i in ids {
        if !uniq.contains(&i) {
            uniq.push(i);
        }
    }
    println!("fail: [{}]", uniq.join(", "));
    for f in fails.iter().take(12) {
        println!("  {}: {}", f.id, f.detail);
    }
}

/// Every ledger gate on one program (static + rendered).
fn gates(
    p: &MotionProgram,
    music: &Option<MusicPlan>,
    assets: &Path,
    sheet: Option<&Path>,
    gen: &GenPaths,
) -> Result<Vec<motion_program::ledger::GateFail>> {
    use motion_program::ledger::GateFail;
    let g = |id: &str, d: String| GateFail {
        id: id.to_string(),
        detail: d,
    };
    if let Err(e) = motion_program::validate(p) {
        // Meaningless shapes are a gate; other validation errors stop QA.
        let meaningless: Vec<GateFail> =
            e.0.iter()
                .filter(|i| i.message.contains("slop_meaningless_shape"))
                .map(|i| g("meaningless_shape", i.path.clone()))
                .collect();
        if meaningless.len() == e.0.len() {
            return Ok(meaningless);
        }
        bail!("not a valid program:\n{e}");
    }
    if let Err(e) = motion_program::validate::check_gen_resolved(p, gen) {
        bail!("not a valid program:\n{e}");
    }
    let mut fails = Vec::new();
    if let (Some(m), Some(plan)) = (&p.music, music) {
        let shift = |v: &[f64]| -> Vec<f64> {
            v.iter()
                .map(|t| t - m.start)
                .filter(|t| *t >= 0.0)
                .collect()
        };
        let cov = motion_program::qa::beat_coverage(
            p,
            &shift(&plan.beat_times),
            &shift(&plan.downbeat_times),
        );
        if !cov.passes() {
            fails.push(g("beats_dropped", cov.report()));
        }
    }
    let tmp = PathBuf::from(format!("output/{}.qa.motion.json", p.meta.name));
    let audio = tmp.with_file_name(format!("{}.qa.audio.json", p.meta.name));
    // Lower without the compile gates (QA reports them as ledger ids).
    let mut lowered = p.clone();
    lowered.music = None;
    let scene = compile_to_gen(&lowered, None, &tmp, assets, gen);
    let scene = match scene {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    let assets_abs = assets.canonicalize()?;
    let measure = font_measure(p, &assets_abs)?;
    let ts = motion_program::Typesetter {
        measure: &measure,
        assets: assets_abs,
    };
    fails.extend(motion_program::ledger::static_gates_with(p, &scene, &ts));
    let times: Vec<f64> = match (&p.music, music) {
        (Some(m), Some(plan)) => plan
            .beat_times
            .iter()
            .map(|t| t - m.start)
            .filter(|t| *t >= 0.0 && *t < p.meta.duration - 0.05)
            .collect(),
        _ => (0..)
            .map(|k| k as f64 * 0.5)
            .take_while(|t| *t < p.meta.duration - 0.05)
            .collect(),
    };
    let base = tmp.parent().unwrap_or(Path::new(".")).to_path_buf();
    let rendered = beat_frames(&scene, &base, &times, sheet);
    let design = design_gates(p, &scene, &base);
    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(&audio);
    let (still, blank) = rendered?;
    fails.extend(design?);
    // The closing hold after the last move (≤ CLOSING_HOLD) is allowed to
    // be still, like the hold after a music-led program's final hit.
    let hold_from = closing_hold_start(p);
    let still: Vec<f64> = still
        .into_iter()
        .filter(|t| !(p.meta.duration - hold_from <= CLOSING_HOLD && *t >= hold_from))
        .collect();
    // A settled read-hold after a component's last build step is allowed
    // (once per component, ≤ 2 beats or 1 s, never in the opening).
    let beat_len = match (&p.music, music) {
        (Some(_), Some(plan)) if plan.beat_times.len() > 1 => Some(
            (plan.beat_times[plan.beat_times.len() - 1] - plan.beat_times[0])
                / (plan.beat_times.len() - 1) as f64,
        ),
        _ => None,
    };
    let still = motion_program::qa::excuse_settled_holds(
        &motion_program::qa::settled_holds(p, beat_len),
        &still,
        &times,
    );
    if !still.is_empty() {
        fails.push(g(
            "static_layout",
            format!("no visible change at {:?}", r2(&still)),
        ));
    }
    // Pixel ink counts dot grids; content coverage is the gate.
    let _ = blank;
    let blank = motion_program::qa::near_blank(p, &scene, &times);
    if !blank.is_empty() {
        let at: Vec<String> = blank.iter().map(|b| b.describe()).collect();
        fails.push(g(
            "static_layout",
            format!("near-blank at {}", at.join(", ")),
        ));
    }
    let (fill, thin) = motion_program::qa::fill(p, &scene, &times);
    if fill < 0.9 {
        fails.push(g(
            "small_composition",
            format!("hero too small at {:?}", r2(&thin)),
        ));
    }
    Ok(fails)
}

/// QA → repair → QA, at most `passes` repairs.
pub(crate) fn qa_loop(
    p: MotionProgram,
    music: Option<MusicPlan>,
    assets: &Path,
    sheet: Option<&Path>,
    passes: usize,
) -> Result<(MotionProgram, Vec<motion_program::ledger::GateFail>)> {
    qa_loop_gen(p, music, assets, sheet, passes, &GenPaths::new())
}

/// [`qa_loop`] for a program with generated assets.
pub(crate) fn qa_loop_gen(
    mut p: MotionProgram,
    music: Option<MusicPlan>,
    assets: &Path,
    sheet: Option<&Path>,
    passes: usize,
    gen: &GenPaths,
) -> Result<(MotionProgram, Vec<motion_program::ledger::GateFail>)> {
    let mut fails = gates(&p, &music, assets, sheet, gen)?;
    for pass in 0..passes {
        if fails.is_empty() || !motion_program::repair::repair(&mut p, &fails) {
            break;
        }
        info!(
            "repair pass {}: {} failure(s) addressed",
            pass + 1,
            fails.len()
        );
        fails = gates(&p, &music, assets, sheet, gen)?;
    }
    Ok((p, fails))
}

/// Scaffold a ledger entry and a fixture stub for a newly reported mistake.
fn learn(id: &str, from: &str, note: &str, area: &str) -> Result<()> {
    if !id.chars().all(|c| c.is_ascii_lowercase() || c == '_') || id.is_empty() {
        bail!("--id: lowercase letters and _");
    }
    let path = Path::new("assets/qa/mistakes.json");
    let text = std::fs::read_to_string(path).context("assets/qa/mistakes.json")?;
    let ledger: serde_json::Value = serde_json::from_str(&text)?;
    let entries = ledger["entries"].as_array().context("entries")?;
    if entries.iter().any(|e| e["id"] == id) {
        bail!("ledger already has '{id}'");
    }
    let fixture = format!("assets/qa/fixtures/{id}.json");
    // Appended as text: the file keeps its key order and layout.
    let q = |v: &str| serde_json::to_string(v).expect("string");
    let fields = [
        ("id", q(id)),
        ("description", q(note)),
        ("owner_note", q(&format!("{note} (seen in {from})"))),
        ("detector", q("TODO: rule id")),
        ("fix_policy", q("TODO")),
        ("repair", q("TODO")),
        ("checks", q(area)),
        ("fixture", q(&fixture)),
    ];
    let body = format!(
        "  {{\n{}\n  }}",
        fields
            .iter()
            .map(|(k, v)| format!("   \"{k}\": {v}"))
            .collect::<Vec<_>>()
            .join(",\n")
    );
    let end = text.rfind(']').context("entries end")?;
    let head = text[..end].trim_end();
    // The closing bracket keeps its own line indent.
    let indent = &text[text[..end].rfind('\n').map_or(end, |n| n + 1)..end];
    let out = format!("{head},\n{body}\n{indent}{}", &text[end..]);
    serde_json::from_str::<serde_json::Value>(&out).context("ledger after append")?;
    std::fs::write(path, out)?;
    let stub = serde_json::json!({
        "id": id,
        "note": note,
        "from": from,
        "kind": area,
        "program": {"version": "2.0", "meta": {"name": "fixture", "duration": 2.0},
            "palette": {"background": "#FFFFFF", "ink": "#111111", "accent": "#FF4D00", "muted": "#777777"},
            "components": [{"id": "todo", "kind": "backdrop"}]},
        "expect_detector": id,
    });
    std::fs::write(&fixture, serde_json::to_string_pretty(&stub)? + "\n")?;
    println!("learned '{id}': ledger entry + {fixture} (fill in the detector, fix policy and fixture program)");
    Ok(())
}

/// Longest still ending a program that the visible-change gate accepts.
const CLOSING_HOLD: f64 = 1.5;

/// When the last move ends (component starts, verbs with their durations,
/// camera moves and transitions).
fn closing_hold_start(p: &MotionProgram) -> f64 {
    let tempo = p.tempo();
    let mut end: f64 = 0.0;
    for c in &p.components {
        end = end.max(c.start);
        for e in &c.timeline {
            if let Some(at) = e.at {
                end = end.max(c.start + at + motion_program::taste::duration(&e.verb, tempo));
            }
        }
    }
    for e in p.camera.iter().filter_map(|e| e.at) {
        end = end.max(e + 0.7 * tempo.factor());
    }
    for e in p.transitions.iter().filter_map(|e| e.at) {
        end = end.max(e + 0.4);
    }
    end
}

/// The AudioPlan of a program's `sound`: the bed, and each hit as a cue at
/// its family's target peak (plus the hit's own offset). Families resolve
/// to their first sound in the SFX library (`assets/sfx/library`).
fn sound_plan(
    p: &MotionProgram,
    sd: &motion_program::program::SoundDesign,
    assets: &Path,
    rel: &str,
) -> Result<AudioPlan> {
    use motion_core::audio::{AudioCue, CueKind, SfxLibrary, LIBRARY_FILE};
    let music = match &sd.bed {
        Some(b) => {
            let plan = music_plan(assets, &b.track)?;
            Some(MusicBed {
                track: format!("{rel}/music/{}", plan.track),
                gain_db: plan.gain_db + b.gain_db,
                fade_in: 0.0,
                fade_out: b.fade_out.clamp(0.05, p.meta.duration),
                duck: false,
                start: b.start,
                fade_out_at: Some(p.meta.duration),
            })
        }
        None => None,
    };
    let mut cues = Vec::new();
    if !sd.hits.is_empty() {
        let lib_path = assets.join("sfx").join("library").join(LIBRARY_FILE);
        let lib = SfxLibrary::from_json(
            &std::fs::read_to_string(&lib_path)
                .with_context(|| format!("SFX library {}", lib_path.display()))?,
        )
        .map_err(|e| anyhow::anyhow!("{}: {e}", lib_path.display()))?;
        for h in &sd.hits {
            let s = lib
                .sounds
                .iter()
                .find(|s| s.id == h.sound)
                .or_else(|| lib.sounds.iter().find(|s| s.family.as_str() == h.sound))
                .with_context(|| format!("sound '{}': no such id or family", h.sound))?;
            let scene = p
                .components
                .iter()
                .rev()
                .find(|c| c.start <= h.at && h.at < p.end_of(c))
                .map_or("program".to_string(), |c| c.id.clone());
            cues.push(AudioCue {
                time: (h.at * 1000.0).round() / 1000.0,
                scene,
                kind: CueKind::Impact,
                family: s.family,
                sound_id: s.id.clone(),
                gain_db: ((s.family.peak_target_db() - s.peak_db + h.gain_db) * 10.0).round()
                    / 10.0,
                priority: 3,
                reason: "authored hit".to_string(),
            });
        }
        cues.sort_by(|a, b| {
            a.time
                .total_cmp(&b.time)
                .then(b.priority.cmp(&a.priority))
                .then(a.sound_id.cmp(&b.sound_id))
        });
    }
    Ok(AudioPlan {
        version: AUDIO_PLAN_VERSION.to_string(),
        cues,
        loudness_target: LOUDNESS_TARGET_LUFS,
        true_peak_limit: TRUE_PEAK_LIMIT_DB,
        min_spacing: 0.0,
        information_cap: 0,
        music,
        speech_adjustments: Vec::new(),
        levels: None,
    })
}

/// (v2.3) `program assets`: every `GenAsset` through the lockfile, the cache
/// and the provider fallback (policy order, MOSS last); writes the lockfile
/// beside the program and returns id → file.
/// `p` is the parsed (beat-resolved, aspect-adjusted) program read from
/// `program`; it is validated first, so an invalid program spends nothing.
pub fn resolve_assets(program: &Path, p: &MotionProgram, offline: bool) -> Result<GenPaths> {
    if p.assets.is_empty() {
        return Ok(GenPaths::new());
    }
    if let Err(e) = motion_program::validate(p) {
        // Meaningless shapes are a QA gate (repairable), not a stop.
        let real: Vec<motion_program::Issue> =
            e.0.into_iter()
                .filter(|i| !i.message.contains("slop_meaningless_shape"))
                .collect();
        if !real.is_empty() {
            bail!(
                "{} is not a valid program (nothing generated):\n{}",
                program.display(),
                motion_program::Issues(real)
            );
        }
    }
    let aspect_str = |a: Aspect| {
        serde_json::to_value(a)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "16:9".into())
    };
    let specs: Vec<(String, ImageSpec)> = p
        .assets
        .iter()
        .map(|a| {
            let cutout = a.kind == GenKind::Cutout;
            let aspect = a.aspect.unwrap_or(if cutout {
                Aspect::Square
            } else {
                p.meta.aspect
            });
            (
                a.id.clone(),
                ImageSpec {
                    prompt: a.prompt.clone(),
                    cutout,
                    aspect: aspect_str(aspect),
                    seed: a.seed,
                },
            )
        })
        .collect();
    let lock_file = lock_path(program);
    let mut lock = Lockfile::load(&lock_file)?;
    let before = lock.clone();
    let video = p.meta.name.clone();
    let done = resolve_all(&mut lock, &specs, offline, &mut |id, spec| {
        let t0 = std::time::Instant::now();
        let r = crate::gen_providers_cmd::fallback_image(None, spec, &video)?;
        info!(
            "asset {id}: {} {} {} in {:.1}s",
            r.provider,
            r.model,
            if r.cache_hit {
                "cache hit"
            } else {
                "generated"
            },
            t0.elapsed().as_secs_f64()
        );
        Ok(r)
    })?;
    let reused = done
        .iter()
        .filter(|(_, a)| matches!(a, LockAction::Reused))
        .count();
    if lock != before {
        lock.save(&lock_file)?;
    }
    info!(
        "assets: {} resolved ({reused} from the lockfile) -> {}",
        done.len(),
        lock_file.display()
    );
    Ok(lock.paths())
}
