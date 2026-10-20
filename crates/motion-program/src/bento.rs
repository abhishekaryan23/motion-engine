//! First-class Bento Metric & Architecture Grid component.
//!
//! Provides dense, structured metric and feature cards arranged in responsive
//! aspect-aware grids (16:9, 9:16, 1:1) with 2.5D card tilt arrivals,
//! automatic numeric count-ups, and music-synchronized step reveals.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use motion_core::easing::Easing;
use motion_core::scene::{FontRole, LayerKind, MotionOp, Stroke};

use crate::lower::{layer, scene, Ctx, Track};
use crate::program::{Aspect, Component, Slot};
use crate::taste::{self, Shape};

/// Bento layout preset archetypes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BentoPreset {
    #[default]
    Grid2x2,
    HeroLeft,
    Pillars,
    /// Recursive Golden Ratio (61.8% / 38.2%) Fibonacci subdivision grid.
    GoldenSpiral,
}

/// Bento metric & architecture grid component.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bento {
    #[serde(default)]
    pub preset: BentoPreset,
    /// Optional section header above the grid (at most 32 characters).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// 2..=6 metric/feature tiles.
    pub tiles: Vec<BentoTile>,
    /// With music: reveal the next tile every this many beats from `start_beat`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beat_cuts: Option<u32>,
    /// Optional 2.5D card tilt entrance angle `[pitch_deg, yaw_deg]` (default: `[-12.0, 8.0]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tilt_in: Option<[f32; 2]>,
    #[serde(default)]
    pub slot: Slot,
}

/// One metric or architecture card inside a [`Bento`] grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BentoTile {
    /// Primary hero metric or badge (1..=12 chars, e.g. `"40%"`, `"2X"`, `"1-MIN"`, `"vSAN ESA"`).
    pub stat: String,
    /// Concise label (1..=22 chars, e.g. `"LOWER TCO"`, `"VM DENSITY"`, `"RPO RECOVERY"`).
    pub title: String,
    /// Optional mono sub-caption (1..=28 chars, e.g. `"VS PUBLIC CLOUD"`, `"NVME TIERING"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Highlight this tile as an accent hero card.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub accent: bool,
}

impl Bento {
    /// Static validation of a Bento grid.
    pub fn check(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        let n = self.tiles.len();
        if !(2..=6).contains(&n) {
            out.push((".tiles", format!("at least 2 and at most 6 tiles, got {n}")));
        }
        if let Some(t) = &self.title {
            let count = t.chars().count();
            if t.trim().is_empty() || count > 32 || t.contains('\n') {
                out.push((
                    ".title",
                    format!("header must be 1..=32 characters on one line, got {count}"),
                ));
            }
            if let Some(g) = crate::device::banned_glyph(t) {
                out.push((
                    ".title",
                    format!("'{g}' has no glyph in the type faces (glyph_fallback)"),
                ));
            }
        }
        for (i, tile) in self.tiles.iter().enumerate() {
            let sc = tile.stat.chars().count();
            if tile.stat.trim().is_empty() || sc > 12 || tile.stat.contains('\n') {
                out.push((
                    ".tiles.stat",
                    format!("tile {i} stat must be 1..=12 characters on one line, got {sc}"),
                ));
            }
            if let Some(g) = crate::device::banned_glyph(&tile.stat) {
                out.push((
                    ".tiles.stat",
                    format!("'{g}' has no glyph in the type faces (glyph_fallback)"),
                ));
            }
            let tc = tile.title.chars().count();
            if tile.title.trim().is_empty() || tc > 22 || tile.title.contains('\n') {
                out.push((
                    ".tiles.title",
                    format!("tile {i} title must be 1..=22 characters on one line, got {tc}"),
                ));
            }
            if let Some(g) = crate::device::banned_glyph(&tile.title) {
                out.push((
                    ".tiles.title",
                    format!("'{g}' has no glyph in the type faces (glyph_fallback)"),
                ));
            }
            if let Some(d) = &tile.detail {
                let dc = d.chars().count();
                if d.trim().is_empty() || dc > 28 || d.contains('\n') {
                    out.push((
                        ".tiles.detail",
                        format!("tile {i} detail must be 1..=28 characters on one line, got {dc}"),
                    ));
                }
                if let Some(g) = crate::device::banned_glyph(d) {
                    out.push((
                        ".tiles.detail",
                        format!("'{g}' has no glyph in the type faces (glyph_fallback)"),
                    ));
                }
            }
        }
        if let Some([pitch, yaw]) = self.tilt_in {
            if !pitch.is_finite()
                || !yaw.is_finite()
                || !(-65.0..=65.0).contains(&pitch)
                || !(-65.0..=65.0).contains(&yaw)
            {
                out.push((
                    ".tilt_in",
                    "tilt angles must be finite and within -65.0..=65.0".to_string(),
                ));
            }
        }
        out
    }
}

pub(crate) fn lower(
    cx: &Ctx,
    c: &Component,
    bento: &Bento,
    shown: usize,
    z: i32,
    length: f64,
) -> motion_core::scene::Scene {
    let id = c.id.as_str();
    let tempo = cx.tempo.factor();
    let n = bento.tiles.len().clamp(2, 6);
    let shown = shown.clamp(1, n);

    // Aspect-aware grid bounding box
    let (bw, bh) = match cx.aspect {
        Aspect::Portrait => {
            if bento.slot == Slot::Center {
                ((cx.w * 0.88).round(), (cx.h * 0.76).round())
            } else {
                ((cx.w * 0.86).round(), (cx.h * 0.58).round())
            }
        }
        Aspect::Landscape => ((cx.w * 0.84).round(), (cx.h * 0.62).round()),
        Aspect::Square => ((cx.w * 0.84).round(), (cx.h * 0.60).round()),
    };
    let bw = if matches!(bento.slot, Slot::Left | Slot::Right) {
        crate::taste::slot_width(bento.slot, cx.aspect)
    } else {
        bw
    };
    let center = if matches!(bento.slot, Slot::Left | Slot::Right) {
        crate::taste::slot_center(bento.slot, cx.aspect)
    } else {
        taste::slot_center(bento.slot, cx.aspect)
    };
    let grid_left = (center[0] - bw / 2.0).round();
    let grid_top = (center[1] - bh / 2.0).round();
    let gap = (cx.short * 0.024).round().max(16.0);

    let boxes: Vec<(f32, f32, f32, f32)> = match bento.preset {
        BentoPreset::Grid2x2 => {
            // Determine grid columns and rows
            let (cols, rows) = match (cx.aspect, n) {
                (Aspect::Portrait, 2) => (1, 2),
                (_, 2) => (2, 1),
                (Aspect::Portrait, 3) => (1, 3),
                (_, 3) => (3, 1),
                (_, 4) => (2, 2),
                (Aspect::Portrait, 5 | 6) => (2, 3),
                (_, 5 | 6) => (3, 2),
                _ => (2, 2),
            };

            let base_tw = ((bw - (cols as f32 - 1.0) * gap) / cols as f32).round();
            let raw_th = ((bh - (rows as f32 - 1.0) * gap) / rows as f32).round();
            let base_th = raw_th
                .min((base_tw * taste::INV_PHI).round())
                .max((cx.short * 0.18).round());
            let total_grid_h = rows as f32 * base_th + (rows as f32 - 1.0) * gap;
            let row_top = (center[1] - total_grid_h * 0.5).round();

            // Compute initial positions for the first `shown` tiles
            let mut bxs: Vec<(f32, f32, f32, f32)> = (0..shown)
                .map(|i| {
                    if n == 5 && cx.aspect == Aspect::Portrait && i == 4 {
                        let y = row_top + 2.0 * (base_th + gap) + base_th / 2.0;
                        (center[0], y, bw, base_th)
                    } else if n == 5 && cx.aspect != Aspect::Portrait && (i == 3 || i == 4) {
                        let row_w = 2.0 * base_tw + gap;
                        let start_x = center[0] - row_w / 2.0 + base_tw / 2.0;
                        let col_idx = (i - 3) as f32;
                        let x = start_x + col_idx * (base_tw + gap);
                        let y = row_top + (base_th + gap) + base_th / 2.0;
                        (x, y, base_tw, base_th)
                    } else {
                        let col = i % cols;
                        let row = i / cols;
                        let x = grid_left + col as f32 * (base_tw + gap) + base_tw / 2.0;
                        let y = row_top + row as f32 * (base_th + gap) + base_th / 2.0;
                        (x, y, base_tw, base_th)
                    }
                })
                .collect();

            // Center any incomplete row horizontally
            for r_idx in 0..rows {
                let row_indices: Vec<usize> =
                    (r_idx * cols..((r_idx + 1) * cols).min(shown)).collect();
                if row_indices.is_empty() || row_indices.len() == cols {
                    continue;
                }
                let mean_x = row_indices.iter().map(|&idx| bxs[idx].0).sum::<f32>()
                    / row_indices.len() as f32;
                let dx = center[0] - mean_x;
                for &idx in &row_indices {
                    bxs[idx].0 = (bxs[idx].0 + dx).round();
                }
            }

            // Center the entire shown subset around `center` when `shown < n`
            if shown < n {
                let min_x = bxs
                    .iter()
                    .map(|(x, _, w, _)| x - w / 2.0)
                    .fold(f32::MAX, f32::min);
                let max_x = bxs
                    .iter()
                    .map(|(x, _, w, _)| x + w / 2.0)
                    .fold(f32::MIN, f32::max);
                let min_y = bxs
                    .iter()
                    .map(|(_, y, _, h)| y - h / 2.0)
                    .fold(f32::MAX, f32::min);
                let max_y = bxs
                    .iter()
                    .map(|(_, y, _, h)| y + h / 2.0)
                    .fold(f32::MIN, f32::max);
                let dx = (center[0] - (min_x + max_x) / 2.0).round();
                let dy = (center[1] - (min_y + max_y) / 2.0).round();
                for b in &mut bxs {
                    b.0 += dx;
                    b.1 += dy;
                }
            }
            bxs
        }
        BentoPreset::HeroLeft => {
            let th_top = (bw * 0.34).clamp(cx.short * 0.20, bh * 0.44).round();
            if shown == 1 {
                let single_h = (bw * 0.38).clamp(cx.short * 0.24, bh * 0.50).round();
                vec![(center[0], center[1], bw, single_h)]
            } else {
                let k = (shown - 1) as f32;
                let rw = ((bw - (k - 1.0) * gap) / k).round();
                let rh = (rw * 0.66).clamp(cx.short * 0.18, bh * 0.42).round();
                let total_h = th_top + gap + rh;
                let top_y0 = (center[1] - total_h * 0.5).round();
                let mut bxs = vec![(center[0], (top_y0 + th_top * 0.5).round(), bw, th_top)];
                let bot_cy = (top_y0 + th_top + gap + rh * 0.5).round();
                for idx in 1..shown {
                    let rx = (grid_left + (idx - 1) as f32 * (rw + gap) + rw * 0.5).round();
                    bxs.push((rx, bot_cy, rw, rh));
                }
                bxs
            }
        }
        BentoPreset::Pillars => {
            if cx.aspect == Aspect::Portrait || matches!(bento.slot, Slot::Left | Slot::Right) {
                let k = shown as f32;
                let raw_ph = ((bh * 0.84 - (k - 1.0) * gap) / k).round();
                let ph = raw_ph
                    .min((bw * 0.28).round())
                    .max((cx.short * 0.14).round());
                let total_h = k * ph + (k - 1.0) * gap;
                let start_y = (center[1] - total_h * 0.5).round();
                (0..shown)
                    .map(|i| {
                        let py = (start_y + i as f32 * (ph + gap) + ph * 0.5).round();
                        (center[0], py, bw, ph)
                    })
                    .collect()
            } else {
                let k = shown as f32;
                let pw = ((bw - (k - 1.0) * gap) / k).round();
                let span_w = k * pw + (k - 1.0) * gap;
                let start_x = (center[0] - span_w * 0.5).round();
                let ph = (pw * taste::INV_PHI)
                    .min(bh * 0.64)
                    .max(cx.short * 0.22)
                    .round();
                (0..shown)
                    .map(|i| {
                        let px = (start_x + i as f32 * (pw + gap) + pw * 0.5).round();
                        (px, center[1], pw, ph)
                    })
                    .collect()
            }
        }
        BentoPreset::GoldenSpiral => {
            if shown == 1 {
                let single_h = (bw * taste::INV_PHI).clamp(cx.short * 0.24, bh).round();
                vec![(center[0], center[1], bw, single_h)]
            } else if shown == 2 {
                if cx.aspect == Aspect::Portrait {
                    let h0 = ((bh - gap) * taste::INV_PHI).round();
                    let h1 = (bh - gap - h0).round();
                    vec![
                        (center[0], grid_top + h0 * 0.5, bw, h0),
                        (center[0], grid_top + h0 + gap + h1 * 0.5, bw, h1),
                    ]
                } else {
                    let w0 = ((bw - gap) * taste::INV_PHI).round();
                    let w1 = (bw - gap - w0).round();
                    vec![
                        (grid_left + w0 * 0.5, center[1], w0, bh),
                        (grid_left + w0 + gap + w1 * 0.5, center[1], w1, bh),
                    ]
                }
            } else if cx.aspect == Aspect::Portrait {
                if shown == 3 {
                    // Tile 0 takes the top 46% full-width hero card
                    let th0 = ((bh - gap) * 0.46).round();
                    let bot_h = (bh - gap - th0).round();
                    let bot_y = (grid_top + th0 + gap).round();
                    let mut bxs = vec![(center[0], (grid_top + th0 * 0.5).round(), bw, th0)];
                    if n == 3 && bw >= 700.0 {
                        // Split 52% / 48% horizontally when 3 is the final tile count
                        let w1 = ((bw - gap) * 0.52).round();
                        let w2 = (bw - gap - w1).round();
                        bxs.push((grid_left + w1 * 0.5, bot_y + bot_h * 0.5, w1, bot_h));
                        bxs.push((
                            grid_left + w1 + gap + w2 * 0.5,
                            bot_y + bot_h * 0.5,
                            w2,
                            bot_h,
                        ));
                    } else {
                        // Stack vertically so Tile 1 stays centered at full width on the way to shown == 4
                        let h1 = ((bot_h - gap) * taste::INV_PHI).round();
                        let h2 = (bot_h - gap - h1).round();
                        bxs.push((center[0], bot_y + h1 * 0.5, bw, h1));
                        bxs.push((center[0], bot_y + h1 + gap + h2 * 0.5, bw, h2));
                    }
                    bxs
                } else {
                    // shown >= 4:
                    // Tile 0 takes top 38.2% full-width hero card
                    let th0 = ((bh - 2.0 * gap) * 0.382).round();
                    // Tile 1 takes middle 30.9% full-width card
                    let th1 = ((bh - 2.0 * gap) * 0.309).round();
                    // Tiles 2..shown split the bottom 30.9% row
                    let th2 = (bh - 2.0 * gap - th0 - th1).round();
                    let y0 = (grid_top + th0 * 0.5).round();
                    let y1 = (grid_top + th0 + gap + th1 * 0.5).round();
                    let y2 = (grid_top + th0 + gap + th1 + gap + th2 * 0.5).round();

                    let mut bxs = vec![(center[0], y0, bw, th0), (center[0], y1, bw, th1)];
                    let rem = shown - 2;
                    let rem_k = rem as f32;
                    let row_w = ((bw - (rem_k - 1.0) * gap) / rem_k).round();
                    for idx in 0..rem {
                        let card_w = if idx == rem - 1 {
                            (bw - idx as f32 * (row_w + gap)).round()
                        } else {
                            row_w
                        };
                        let tx = grid_left + idx as f32 * (row_w + gap) + card_w * 0.5;
                        bxs.push((tx.round(), y2, card_w, th2));
                    }
                    bxs
                }
            } else {
                // Landscape / Square when shown >= 3:
                // Tile 0: Left Golden Major column
                let w0 = ((bw - gap) * taste::INV_PHI).round();
                // Tiles 1..shown: k = shown - 1 right-hand cards in the 38.2% minor column
                let w1 = (bw - gap - w0).round();
                let x1 = (grid_left + w0 + gap + w1 * 0.5).round();
                let k = shown - 1;

                let heights: Vec<f32> = if k == 2 {
                    let h1 = ((bh - gap) * taste::INV_PHI).round();
                    let h2 = (bh - gap - h1).round();
                    vec![h1, h2]
                } else if k == 3 {
                    let h1 = ((bh - 2.0 * gap) * 0.42).round();
                    let h2 = ((bh - 2.0 * gap) * 0.31).round();
                    let h3 = (bh - 2.0 * gap - h1 - h2).round();
                    vec![h1, h2, h3]
                } else {
                    let total_gap = (k as f32 - 1.0) * gap;
                    let raw_h = ((bh - total_gap) / k as f32).round();
                    let mut hs = vec![raw_h; k - 1];
                    hs.push((bh - total_gap - raw_h * (k as f32 - 1.0)).round());
                    hs
                };

                let mut bxs = vec![(grid_left + w0 * 0.5, grid_top + bh * 0.5, w0, bh)];
                let mut cur_y = grid_top;
                for h in heights {
                    bxs.push((x1, (cur_y + h * 0.5).round(), w1, h));
                    cur_y += h + gap;
                }
                bxs
            }
        }
    };

    let top_edge = boxes
        .iter()
        .map(|(_, y, _, h)| y - h / 2.0)
        .fold(f32::MAX, f32::min);
    let top_edge = if top_edge.is_finite() {
        top_edge
    } else {
        grid_top
    };

    let mut layers = Vec::new();
    let mut track = Track::default();

    // Section header above the shown grid
    if let Some(ref hdr) = bento.title {
        let hdr_y = (top_edge - gap * 1.35).max(cx.short * 0.08).round();
        let base_hdr_sz = (cx.short * 0.036).round().clamp(16.0, 32.0);
        let mut hdr_sz = cx.fit(hdr, FontRole::Mono, 0.08, true, bw * 0.94, base_hdr_sz);
        while cx.line_width(hdr, FontRole::Mono, hdr_sz, 0.08, true) > bw * 0.94 && hdr_sz > 12.0 {
            hdr_sz -= 1.0;
        }
        let hdr_color = cx.roles(id).ink;
        let hdr_layer = cx.text_canvas(
            &format!("{id}.header"),
            hdr,
            FontRole::Mono,
            hdr_sz,
            hdr_color,
            0.08,
            true,
            [center[0], hdr_y],
            z + 1,
        );
        layers.push(hdr_layer);
    }

    // Timeline advances and arrival times
    let mut advances: Vec<f64> = c
        .timeline
        .iter()
        .filter(|e| matches!(e.verb, crate::program::Verb::Advance))
        .filter_map(|e| e.at)
        .collect();
    advances.sort_by(f64::total_cmp);

    let enter_at = c
        .timeline
        .iter()
        .find(|e| matches!(e.verb, crate::program::Verb::Enter { .. }))
        .and_then(|e| e.at)
        .unwrap_or(0.0);

    let r = cx.roles(id);

    for (i, tile) in bento.tiles.iter().enumerate().take(shown) {
        let (tx, ty, tw, th) = boxes[i];

        let at_i = if i == 0 {
            enter_at
        } else if let Some(&at) = advances.get(i - 1) {
            at
        } else {
            enter_at + 0.12 * i as f64
        };

        let (card_fill, stroke_col, stat_color, text_color, muted_color) = if tile.accent {
            let inv = cx.col.roles(r.ink);
            (r.ink, r.ink, inv.accent_text, inv.ink, inv.ink)
        } else {
            (r.ground, r.structure_strong, r.ink, r.ink, r.ink)
        };

        // 1. Background card path
        let radius = (tw.min(th) * 0.08).round().max(8.0);
        let bg_kind = LayerKind::Path {
            d: motion_core::path::rounded_rect(0.0, 0.0, tw, th, radius).to_svg(),
            fill: Some(motion_core::scene::Fill::Solid(card_fill)),
            stroke: Some(Stroke {
                color: stroke_col,
                width: 2.5,
                brush: None,
            }),
            fill_rule: motion_core::scene::PathFillRule::Nonzero,
            shadows: Vec::new(),
        };
        let bg = layer(
            &format!("{id}.tile{i}.bg"),
            bg_kind,
            [tw / 2.0, th / 2.0],
            [tw, th],
            1,
        );

        // 2. Hero stat text layer (large bare integers get thousands separators)
        let stat = crate::chart::group_thousands(&tile.stat);
        let max_stat_h = (th * 0.34).round();
        let base_stat_fs = cx.fit(
            &stat,
            FontRole::Display,
            -0.02,
            false,
            tw * 0.86,
            max_stat_h.max(16.0),
        );
        let mut stat_sz = base_stat_fs.min(max_stat_h).max(14.0);
        while cx.line_width(&stat, FontRole::Display, stat_sz, -0.02, false) > tw * 0.86
            && stat_sz > 12.0
        {
            stat_sz -= 1.0;
        }
        let stat_layer = cx.text(
            &format!("{id}.tile{i}.stat"),
            &stat,
            FontRole::Display,
            stat_sz,
            stat_color,
            -0.02,
            false,
            [tw * 0.5, th * 0.34],
            2,
        );

        // 3. Title text layer: Golden Ratio modular scale title_fs ≈ stat_fs / φ²
        let title_y = if tile.detail.is_some() {
            (th * taste::INV_PHI).round()
        } else {
            (th * 0.70).round()
        };
        let phi_title_target = taste::phi_step(stat_sz, -2).round().clamp(13.0, 32.0);
        let max_title_h = (th * 0.16).round().max(13.0).min(phi_title_target);
        let base_title_fs = cx.fit(
            &tile.title,
            FontRole::Body,
            0.04,
            true,
            tw * 0.86,
            max_title_h,
        );
        let mut title_sz = base_title_fs.min(max_title_h).max(12.0);
        while cx.line_width(&tile.title, FontRole::Body, title_sz, 0.04, true) > tw * 0.86
            && title_sz > 10.0
        {
            title_sz -= 1.0;
        }
        let title_layer = cx.text(
            &format!("{id}.tile{i}.title"),
            &tile.title,
            FontRole::Body,
            title_sz,
            text_color,
            0.04,
            true,
            [tw * 0.5, title_y],
            2,
        );

        // 4. Optional detail text layer: Golden Ratio modular scale detail_fs ≈ title_fs / φ
        let mut children = vec![bg, stat_layer, title_layer];
        if let Some(ref det) = tile.detail {
            let phi_det_target = taste::phi_step(title_sz, -1).round().clamp(11.0, 24.0);
            let max_det_h = (th * 0.13).round().max(11.0).min(phi_det_target);
            let base_det_fs = cx.fit(det, FontRole::Mono, 0.06, true, tw * 0.86, max_det_h);
            let mut det_sz = base_det_fs.min(max_det_h).max(10.0);
            while cx.line_width(det, FontRole::Mono, det_sz, 0.06, true) > tw * 0.86 && det_sz > 8.0
            {
                det_sz -= 1.0;
            }
            let detail_layer = cx.text(
                &format!("{id}.tile{i}.detail"),
                det,
                FontRole::Mono,
                det_sz,
                muted_color,
                0.06,
                true,
                [tw * 0.5, (th * 0.82).round()],
                2,
            );
            children.push(detail_layer);
        }

        // Group container
        let group_id = format!("{id}.tile{i}");
        let mut group = layer(
            &group_id,
            LayerKind::Group { children },
            [tx, ty],
            [tw, th],
            z + 2 + i as i32,
        );
        group.z = Some(if tile.accent { -18.0 } else { 0.0 });

        // Animations on arrival (at_i): MotionOp::Fade holds `from: 0.0` before `at_i`.
        if at_i > 0.0 {
            track.push(
                &group_id,
                at_i,
                0.15,
                Shape::Ease(Easing::OutCubic),
                MotionOp::Fade { from: 0.0, to: 1.0 },
            );
        }

        track.push(
            &group_id,
            at_i,
            (0.42 * tempo).min(length - at_i),
            Shape::Spring(taste::Spring::Pop),
            MotionOp::Scale {
                from: 0.78,
                to: 1.0,
                axis: Default::default(),
            },
        );

        let tilt_from = bento
            .tilt_in
            .unwrap_or([-12.0, if i % 2 == 0 { 8.0 } else { -8.0 }]);
        track.push(
            &group_id,
            at_i,
            (0.48 * tempo).min(length - at_i),
            Shape::Spring(taste::Spring::Settle),
            MotionOp::Tilt {
                from: tilt_from,
                to: [0.0, 0.0],
            },
        );

        // Numeric count-up if stat contains numbers
        if let Some((prefix, value, decimals, suffix)) = crate::chart::parse_number(&stat) {
            track.push(
                &format!("{id}.tile{i}.stat"),
                at_i,
                (0.45 * tempo).min(length - at_i),
                Shape::Ease(Easing::OutQuint),
                MotionOp::Count {
                    from: 0.0,
                    to: value,
                    decimals,
                    // Every intermediate value is grouped like the end value.
                    grouping: stat.contains(','),
                    prefix,
                    suffix,
                },
            );
        }

        layers.push(group);
    }

    scene(id, c.start, length, layers, track.motions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lower::{lower, Typesetter};
    use crate::program::MotionProgram;
    use crate::validate::validate;

    #[test]
    fn test_bento_golden_spiral_preset_serde() {
        let json = r#"{"preset":"golden_spiral","tiles":[{"stat":"40%","title":"SAVINGS"}]}"#;
        let bento: Bento = serde_json::from_str(json).unwrap();
        assert_eq!(bento.preset, BentoPreset::GoldenSpiral);
        let serialized = serde_json::to_string(&bento).unwrap();
        assert!(serialized.contains("\"golden_spiral\""));
    }

    #[test]
    fn test_golden_spiral_lowers_and_validates_landscape() {
        let program_json = r##"{
            "version": "2.0",
            "meta": {"name": "bento_test", "duration": 4, "aspect": "16:9"},
            "palette": {"background": "#101010", "ink": "#FFFFFF", "accent": "#00FFCC", "muted": "#888888"},
            "components": [
                {
                    "id": "bento",
                    "kind": "bento",
                    "preset": "golden_spiral",
                    "start": 0.0,
                    "tiles": [
                        {"stat": "61.8%", "title": "MAJOR SECTION", "accent": true},
                        {"stat": "38.2%", "title": "MINOR SECTION", "detail": "FIBONACCI"},
                        {"stat": "1.618", "title": "GOLDEN RATIO"}
                    ],
                    "timeline": [{"at": 0.0, "do": "enter", "style": "pop"}]
                }
            ]
        }"##;
        let p = MotionProgram::from_json(program_json).expect("validates json");
        validate(&p).expect("semantic validation passes");
        let project = lower(&p, None, &Typesetter::approx());
        motion_core::validate(&project, None)
            .expect("lowered project passes motion-core validation");

        let bento_scene = project
            .scenes
            .iter()
            .find(|s| s.id == "bento")
            .expect("bento scene found");
        let t0 = bento_scene
            .layers
            .iter()
            .find(|l| l.id == "bento.tile0")
            .expect("tile0 exists");
        let t1 = bento_scene
            .layers
            .iter()
            .find(|l| l.id == "bento.tile1")
            .expect("tile1 exists");
        let t2 = bento_scene
            .layers
            .iter()
            .find(|l| l.id == "bento.tile2")
            .expect("tile2 exists");
        assert_eq!(t0.width, 981.0);
        assert_eq!(t1.width, 606.0);
        assert_eq!(t2.width, 606.0);
    }

    #[test]
    fn test_golden_spiral_lowers_and_validates_portrait_4_tiles() {
        let program_json = r##"{
            "version": "2.0",
            "meta": {"name": "bento_portrait", "duration": 4, "aspect": "9:16"},
            "palette": {"background": "#101010", "ink": "#FFFFFF", "accent": "#00FFCC", "muted": "#888888"},
            "components": [
                {
                    "id": "bento",
                    "kind": "bento",
                    "preset": "golden_spiral",
                    "start": 0.0,
                    "tiles": [
                        {"stat": "HERO", "title": "TOP CARD", "accent": true},
                        {"stat": "61.8%", "title": "LEFT SECTION"},
                        {"stat": "0.1MS", "title": "STACK TOP", "detail": "FAST"},
                        {"stat": "99.9%", "title": "STACK BOT", "detail": "STABLE"}
                    ],
                    "timeline": [{"at": 0.0, "do": "enter", "style": "pop"}]
                }
            ]
        }"##;
        let p = MotionProgram::from_json(program_json).expect("validates json");
        validate(&p).expect("semantic validation passes");
        let project = lower(&p, None, &Typesetter::approx());
        motion_core::validate(&project, None)
            .expect("lowered project passes motion-core validation");

        let bento_scene = project
            .scenes
            .iter()
            .find(|s| s.id == "bento")
            .expect("bento scene found");
        assert_eq!(bento_scene.layers.len(), 4);
        let t0 = bento_scene
            .layers
            .iter()
            .find(|l| l.id == "bento.tile0")
            .expect("tile0");
        let t1 = bento_scene
            .layers
            .iter()
            .find(|l| l.id == "bento.tile1")
            .expect("tile1");
        let t2 = bento_scene
            .layers
            .iter()
            .find(|l| l.id == "bento.tile2")
            .expect("tile2");
        let t3 = bento_scene
            .layers
            .iter()
            .find(|l| l.id == "bento.tile3")
            .expect("tile3");
        assert_eq!(t0.width, 950.0);
        assert_eq!(t0.height, 537.0);
        assert_eq!(t1.width, 950.0);
        assert_eq!(t1.height, 435.0);
        assert_eq!(t2.width, 462.0);
        assert_eq!(t2.height, 435.0);
        assert_eq!(t3.width, 462.0);
        assert_eq!(t3.height, 435.0);
    }

    #[test]
    fn test_golden_spiral_portrait_3_tiles_layout() {
        let program_json = r##"{
            "version": "2.0",
            "meta": {"name": "bento_p3", "duration": 4, "aspect": "9:16"},
            "palette": {"background": "#101010", "ink": "#FFFFFF", "accent": "#00FFCC", "muted": "#888888"},
            "components": [
                {
                    "id": "bento",
                    "kind": "bento",
                    "preset": "golden_spiral",
                    "start": 0.0,
                    "tiles": [
                        {"stat": "HERO", "title": "TOP HERO", "accent": true},
                        {"stat": "48%", "title": "BOTTOM LEFT"},
                        {"stat": "52%", "title": "BOTTOM RIGHT"}
                    ],
                    "timeline": [{"at": 0.0, "do": "enter", "style": "pop"}]
                }
            ]
        }"##;
        let p = MotionProgram::from_json(program_json).expect("validates json");
        validate(&p).expect("semantic validation passes");
        let project = lower(&p, None, &Typesetter::approx());
        motion_core::validate(&project, None)
            .expect("lowered project passes motion-core validation");

        let bento_scene = project
            .scenes
            .iter()
            .find(|s| s.id == "bento")
            .expect("bento scene found");
        let t0 = bento_scene
            .layers
            .iter()
            .find(|l| l.id == "bento.tile0")
            .expect("tile0");
        let t1 = bento_scene
            .layers
            .iter()
            .find(|l| l.id == "bento.tile1")
            .expect("tile1");
        let t2 = bento_scene
            .layers
            .iter()
            .find(|l| l.id == "bento.tile2")
            .expect("tile2");
        assert_eq!(t0.width, 950.0);
        assert_eq!(t0.height, 659.0);
        assert_eq!(t1.width, 480.0);
        assert_eq!(t1.height, 774.0);
        assert_eq!(t2.width, 444.0);
        assert_eq!(t2.height, 774.0);
    }
}
