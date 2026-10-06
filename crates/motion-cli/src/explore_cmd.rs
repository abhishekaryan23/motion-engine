//! `explore` (0.9): compile K exploration variants of one story, render one
//! frame per beat at the READ/EVOLVE midpoint, and write a contact sheet
//! (one row per variant) plus `explore.json`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::Args;
use motion_core::assets::AssetManifest;
use motion_core::audio::beat_scenes;
use motion_core::compiler::explore::typography_value;
use motion_core::compiler::typography::Emotion;
use motion_core::compiler::{compile_with_options, resolve_taste, CompileOptions, FontSet};
use motion_core::{evaluate_frame, validate, AssetLibrary, MotionProject};
use motion_render::{CpuRenderer, FontMeasure, Renderer};
use serde_json::{json, Value};

#[derive(Args, Debug)]
pub struct ExploreArgs {
    pub intent: PathBuf,
    #[arg(long)]
    pub style: Option<PathBuf>,
    /// Exploration level 0..=3.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(0..=3))]
    pub explore: u8,
    /// Number of variants (seeds S..S+K-1).
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..=64))]
    pub variants: u32,
    /// First exploration seed.
    #[arg(long, default_value_t = 0)]
    pub seed: u64,
    /// Enable an asset family (repeatable).
    #[arg(long = "asset-family")]
    pub asset_family: Vec<String>,
    /// Typography emotion override (snake_case, e.g. `trust`, `playful_retro`).
    #[arg(long)]
    pub emotion: Option<String>,
    /// Asset library root (contains fonts/ and library/).
    #[arg(long, default_value = "assets")]
    pub assets: PathBuf,
    #[command(flatten)]
    pub canvas: crate::CanvasArgs,
    /// Output directory: variant_<i>.motion.json, sheet.png, explore.json.
    #[arg(long, short)]
    pub output: PathBuf,
}

fn parse_emotion(name: &str) -> Result<Emotion> {
    serde_json::from_value(Value::String(name.to_string())).map_err(|_| {
        let all: Vec<String> = Emotion::ALL
            .iter()
            .filter_map(|e| serde_json::to_value(e).ok())
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        anyhow::anyhow!("unknown emotion '{name}' (one of: {})", all.join(", "))
    })
}

pub fn cmd_explore(args: &ExploreArgs) -> Result<()> {
    let intent = crate::read_intent(&args.intent)?;
    let style = crate::read_style(args.style.as_deref())?;
    let emotion = args.emotion.as_deref().map(parse_emotion).transpose()?;
    let canvas = args.canvas.resolve()?;
    if !args.assets.is_dir() {
        bail!(
            "asset library '{}' not found (use --assets)",
            args.assets.display()
        );
    }
    let assets_abs = args.assets.canonicalize()?;
    let library = AssetLibrary::new(&assets_abs).with_families(args.asset_family.clone());
    std::fs::create_dir_all(&args.output)?;
    let out_abs = args.output.canonicalize()?;
    let asset_root = pathdiff::diff_paths(&assets_abs, &out_abs)
        .unwrap_or_else(|| assets_abs.clone())
        .to_string_lossy()
        .replace('\\', "/");

    let font_paths: Vec<PathBuf> = FontSet::for_style(&style)
        .faces
        .iter()
        .map(|f| assets_abs.join(f.path))
        .collect();
    let measure = FontMeasure::with_fonts(font_paths.iter().map(PathBuf::as_path))?;
    let legacy = typography_name(&resolve_taste(&intent, &style, None).typography);

    let mut rows: Vec<Vec<Thumb>> = Vec::new();
    let mut summary = Vec::new();
    for i in 0..args.variants {
        let seed = args.seed.wrapping_add(i as u64);
        let opts = CompileOptions {
            music_envelope: None,
            explore: args.explore,
            seed,
            emotion,
            canvas,
            speech: None,
            footage: None,
            art: None,
            variety: None,
            no_captions: false,
            take: 0,
            candidates: 0,
        };
        let mut project = compile_with_options(
            &intent,
            &style,
            None,
            &library,
            &measure,
            &AssetManifest::empty(),
            None,
            &opts,
        )?;
        project.asset_root = Some(asset_root.clone());
        validate(&project, Some(&out_abs))
            .map_err(|e| anyhow::anyhow!("variant {i} failed validation:\n{e}"))?;
        let path = out_abs.join(format!("variant_{i}.motion.json"));
        std::fs::write(&path, project.to_json_pretty() + "\n")
            .with_context(|| format!("writing {}", path.display()))?;

        rows.push(render_beats(&project, &out_abs)?);
        let typography = project
            .theme
            .typography
            .as_ref()
            .map(typography_value)
            .unwrap_or_else(|| legacy.clone());
        summary.push(json!({
            "variant": i,
            "seed": seed,
            "level": args.explore,
            "record": project.project.exploration,
            "typography": typography,
        }));
        println!(
            "variant {i}: seed {seed} -> {} ({typography})",
            path.display()
        );
    }

    let sheet = out_abs.join("sheet.png");
    write_sheet(&rows, &sheet)?;
    let json_path = out_abs.join("explore.json");
    std::fs::write(&json_path, serde_json::to_string_pretty(&summary)? + "\n")
        .with_context(|| format!("writing {}", json_path.display()))?;
    println!(
        "sheet {} ; summary {}",
        sheet.display(),
        json_path.display()
    );
    Ok(())
}

/// snake_case name of a legacy typography pairing.
fn typography_name(p: &motion_core::compiler::taste::TypographyPairing) -> String {
    match serde_json::to_value(p) {
        Ok(Value::String(s)) => s,
        _ => String::from("legacy"),
    }
}

/// One downscaled RGB frame.
struct Thumb {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

const THUMB_W: u32 = 270;

/// Render each beat scene at the READ/EVOLVE midpoint and downscale it.
fn render_beats(project: &MotionProject, base: &Path) -> Result<Vec<Thumb>> {
    let renderer = CpuRenderer::new(project, base)?;
    let last = project.frame_count().saturating_sub(1);
    let mut out = Vec::new();
    for scene in beat_scenes(project) {
        let local = scene
            .lifecycle
            .map(|l| (l.read + l.evolve) / 2.0)
            .unwrap_or(scene.duration_seconds / 2.0);
        let t = scene.start_seconds + local;
        let frame = ((t * project.canvas.fps as f64).round() as u32).min(last);
        let resolved = evaluate_frame(project, frame)?;
        let pm = renderer.render(&resolved)?;
        out.push(downscale(pm.data(), pm.width(), pm.height()));
    }
    Ok(out)
}

/// Box-filter downscale of premultiplied RGBA (opaque canvas) to THUMB_W wide.
fn downscale(rgba: &[u8], w: u32, h: u32) -> Thumb {
    let tw = THUMB_W.min(w.max(1));
    let th = ((h as u64 * tw as u64) / w.max(1) as u64).max(1) as u32;
    let mut rgb = Vec::with_capacity((tw * th * 3) as usize);
    for ty in 0..th {
        let y0 = (ty as u64 * h as u64 / th as u64) as u32;
        let y1 = (((ty as u64 + 1) * h as u64 / th as u64) as u32).clamp(y0 + 1, h);
        for tx in 0..tw {
            let x0 = (tx as u64 * w as u64 / tw as u64) as u32;
            let x1 = (((tx as u64 + 1) * w as u64 / tw as u64) as u32).clamp(x0 + 1, w);
            let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = ((y * w + x) * 4) as usize;
                    if let Some(px) = rgba.get(i..i + 3) {
                        r += px[0] as u64;
                        g += px[1] as u64;
                        b += px[2] as u64;
                        n += 1;
                    }
                }
            }
            let n = n.max(1);
            rgb.extend_from_slice(&[(r / n) as u8, (g / n) as u8, (b / n) as u8]);
        }
    }
    Thumb {
        width: tw,
        height: th,
        rgb,
    }
}

const GAP: u32 = 6;
const SHEET_BG: [u8; 3] = [0x30, 0x30, 0x30];

/// Contact sheet: one row per variant, one column per beat.
fn write_sheet(rows: &[Vec<Thumb>], path: &Path) -> Result<()> {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0).max(1) as u32;
    let cell_w = rows
        .iter()
        .flatten()
        .map(|t| t.width)
        .max()
        .unwrap_or(THUMB_W);
    let cell_h = rows
        .iter()
        .flatten()
        .map(|t| t.height)
        .max()
        .unwrap_or(THUMB_W);
    let n_rows = rows.len().max(1) as u32;
    let width = GAP + cols * (cell_w + GAP);
    let height = GAP + n_rows * (cell_h + GAP);
    let mut rgb = SHEET_BG.repeat((width * height) as usize);
    for (r, row) in rows.iter().enumerate() {
        for (c, t) in row.iter().enumerate() {
            let ox = GAP + c as u32 * (cell_w + GAP);
            let oy = GAP + r as u32 * (cell_h + GAP);
            for y in 0..t.height {
                let src = (y * t.width * 3) as usize;
                let dst = (((oy + y) * width + ox) * 3) as usize;
                let len = (t.width * 3) as usize;
                rgb[dst..dst + len].copy_from_slice(&t.rgb[src..src + len]);
            }
        }
    }
    std::fs::write(path, encode_png(width, height, &rgb))
        .with_context(|| format!("writing {}", path.display()))
}

// ---------------------------------------------------------------------------
// Minimal PNG writer (RGB8, stored deflate blocks): no image dependency.
// ---------------------------------------------------------------------------

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

fn encode_png(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    // Scanlines, each prefixed with filter type 0.
    let stride = (width * 3) as usize;
    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    for y in 0..height as usize {
        raw.push(0);
        raw.extend_from_slice(&rgb[y * stride..(y + 1) * stride]);
    }
    // zlib: header, stored blocks of up to 65535 bytes, adler32.
    let mut z = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65535).peekable();
    if blocks.peek().is_none() {
        z.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(block) = blocks.next() {
        let last = blocks.peek().is_none();
        z.push(u8::from(last));
        let len = block.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit RGB
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}
