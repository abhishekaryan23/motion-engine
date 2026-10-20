//! Ledger detectors for product UI (docs/DESIGN_RULES.md): run over
//! the lowered scene of every `device_frame` component (and, for type and
//! camera, every scene), sampling resolved frames. They read the
//! DeviceFrame lowering's layer ids: `<id>.fill<n>` / `<id>.plate` (the
//! container), `<id>.s<k>` (one content group per state), `<id>.touch`
//! (finger or pointer), `<id>.ripple<n>`, `<id>.screen`, labels ending in
//! `label`.

use motion_core::scene::{Color, Fill, LayerKind, MotionOp, Scene};
use motion_core::timeline::ResolvedLayer;
use motion_core::MotionProject;

use crate::ledger::GateFail;
use crate::palette::{contrast, oklch};
use crate::program::{ComponentBody, MotionProgram};

fn fail(out: &mut Vec<GateFail>, id: &str, detail: impl Into<String>) {
    if !out.iter().any(|f| f.id == id) {
        out.push(GateFail {
            id: id.to_string(),
            detail: detail.into(),
        });
    }
}

/// `<dev>.<kind><digits>`.
fn numbered(id: &str, dev: &str, kind: &str) -> bool {
    id.strip_prefix(dev)
        .and_then(|r| r.strip_prefix('.'))
        .and_then(|r| r.strip_prefix(kind))
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// One resolved layer with its accumulated opacity.
struct Flat<'a, 'b> {
    l: &'b ResolvedLayer<'a>,
    alpha: f32,
}

fn flatten<'a, 'b>(ls: &'b [ResolvedLayer<'a>], alpha: f32, out: &mut Vec<Flat<'a, 'b>>) {
    for l in ls {
        let a = alpha * l.opacity;
        out.push(Flat { l, alpha: a });
        flatten(&l.children, a, out);
    }
}

/// Canvas bounds of a layer's box `[x0, y0, x1, y1]`.
fn box_of(l: &ResolvedLayer<'_>) -> [f32; 4] {
    bounds_of(l, [0.0, 0.0, l.width, l.height])
}

fn bounds_of(l: &ResolvedLayer<'_>, b: [f32; 4]) -> [f32; 4] {
    let pts = [
        l.transform.apply(b[0], b[1]),
        l.transform.apply(b[2], b[1]),
        l.transform.apply(b[0], b[3]),
        l.transform.apply(b[2], b[3]),
    ];
    pts.iter()
        .fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |q, (x, y)| {
            [q[0].min(*x), q[1].min(*y), q[2].max(*x), q[3].max(*y)]
        })
}

/// Canvas bounds of a path layer's (morphed) geometry.
fn path_bounds(l: &ResolvedLayer<'_>) -> Option<[f32; 4]> {
    let LayerKind::Path { d, .. } = l.kind else {
        return None;
    };
    let geo = l
        .path
        .clone()
        .or_else(|| motion_core::path::parse(d).ok())?;
    let b = geo.bounds()?;
    Some(bounds_of(l, b))
}

fn solid_fill(l: &ResolvedLayer<'_>) -> Option<Color> {
    match l.kind {
        LayerKind::Path {
            fill: Some(Fill::Solid(c)),
            ..
        } => Some(*c),
        LayerKind::Rectangle { fill, .. } => Some(*fill),
        _ => None,
    }
}

/// Settle time (98 %) of a spring.
fn settle(s: &motion_core::SpringSpec) -> f64 {
    let m = s.mass.max(1e-3) as f64;
    let w = (s.stiffness as f64 / m).sqrt();
    let zeta = s.damping as f64 / (2.0 * (s.stiffness as f64 * m).sqrt());
    if zeta < 1.0 {
        50f64.ln() / (zeta * w)
    } else {
        5.83 / w * zeta
    }
}

/// Every UI gate over the lowered scene.
pub fn gates(p: &MotionProgram, scene: &MotionProject) -> Vec<GateFail> {
    let mut out = Vec::new();
    // Type anywhere: glyphs the faces do not have.
    for s in &scene.scenes {
        visit(&s.layers, &mut |l| {
            if let LayerKind::Text(t) = &l.kind {
                if let Some(c) = crate::device::banned_glyph(&t.text) {
                    fail(
                        &mut out,
                        "glyph_fallback",
                        format!("'{c}' in {} ({})", l.id, t.text),
                    );
                }
            }
        });
    }
    // Cameras that move nothing.
    for s in &scene.scenes {
        let Some(cam) = &s.camera else { continue };
        // Grounds and overlays are pinned on purpose (camera::pin_grounds).
        let owner = p
            .components
            .iter()
            .find(|c| s.id == c.id || s.id.starts_with(&format!("{}.", c.id)));
        if owner.is_some_and(|c| {
            matches!(
                c.body,
                crate::program::ComponentBody::Backdrop { .. }
                    | crate::program::ComponentBody::Hud { .. }
                    | crate::program::ComponentBody::Field { .. }
                    | crate::program::ComponentBody::Title { .. }
                    | crate::program::ComponentBody::KineticText { .. }
            )
        }) {
            continue;
        }
        let moves = cam.motions.iter().any(|m| match &m.op {
            motion_core::scene::CameraOp::Push { from, to } => (to - from).abs() > 0.05,
            motion_core::scene::CameraOp::Track { from, to } => {
                (to[0] - from[0]).hypot(to[1] - from[1]) > 5.0
            }
            _ => false,
        });
        if moves && !s.layers.is_empty() && s.layers.iter().all(|l| l.depth == Some(0.0)) {
            fail(
                &mut out,
                "camera_dead",
                format!(
                    "{}: the camera moves but every layer is pinned (depth 0)",
                    s.id
                ),
            );
        }
    }
    let devices: Vec<&str> = p
        .components
        .iter()
        .filter(|c| matches!(c.body, ComponentBody::DeviceFrame(_)))
        .map(|c| c.id.as_str())
        .collect();
    for s in &scene.scenes {
        if devices.contains(&s.id.as_str()) {
            device(scene, s, &mut out);
        }
    }
    // Copy: point of view, one success at a time, app context, news.
    for c in &p.components {
        if let ComponentBody::DeviceFrame(f) = &c.body {
            let r = f.resolved();
            let actor = f.flow.as_ref().and_then(|x| x.actor.as_deref());
            for (id, msg) in crate::flow::copy_problems(r.device, &r.states, &r.steps, actor) {
                fail(&mut out, id, format!("{}: {msg}", c.id));
            }
        }
    }
    out
}

fn visit(ls: &[motion_core::scene::Layer], f: &mut dyn FnMut(&motion_core::scene::Layer)) {
    for l in ls {
        f(l);
        if let LayerKind::Group { children } = &l.kind {
            visit(children, f);
        }
    }
}

/// Per-frame facts about one device scene.
struct Sample {
    t: f64,
    /// Content groups shown (> 0.15) with their opacity and canvas box.
    groups: Vec<(String, f32, [f32; 4])>,
    /// Visible container fills: id, colour, alpha, geometry bounds.
    fills: Vec<(String, Color, f32, [f32; 4])>,
    /// Finger/pointer relative to the device (‰ of its box), its size
    /// (‰) and alpha; and its canvas position.
    touch: Option<([f32; 2], f32, f32)>,
    touch_px: Option<[f32; 2]>,
    /// Labels: (id, cap-centre offset / container h, x offset / container w).
    labels: Vec<(String, f32, f32)>,
    /// The phone screen's canvas box.
    screen: Option<[f32; 4]>,
    /// Rows (and cards' buttons) arriving: id, alpha, canvas box.
    arrivals: Vec<(String, f32, [f32; 4])>,
    /// Success ticks drawn (> half) and visible.
    ticks: Vec<String>,
}

fn device(scene: &MotionProject, s: &Scene, out: &mut Vec<GateFail>) {
    let dev = s.id.as_str();
    let fps = scene.canvas.fps as f64;
    let (w, h) = (scene.canvas.width as f32, scene.canvas.height as f32);
    let f0 = (s.start_seconds * fps).ceil() as u32;
    let f1 = ((s.start_seconds + s.duration_seconds) * fps).floor() as u32;
    let last = scene.frame_count().saturating_sub(1);
    let mut samples: Vec<Sample> = Vec::new();
    for f in (f0..f1.min(last + 1)).step_by(2) {
        let Ok(rf) = motion_core::evaluate_frame(scene, f) else {
            continue;
        };
        let mut flat = Vec::new();
        flatten(&rf.layers, 1.0, &mut flat);
        let mine: Vec<&Flat> = flat.iter().filter(|x| x.l.scene == Some(dev)).collect();
        let mut sm = Sample {
            t: f as f64 / fps - s.start_seconds,
            groups: Vec::new(),
            fills: Vec::new(),
            touch: None,
            touch_px: None,
            labels: Vec::new(),
            screen: None,
            arrivals: Vec::new(),
            ticks: Vec::new(),
        };
        for x in &mine {
            let id = x.l.id;
            if numbered(id, dev, "s") && x.alpha > 0.15 {
                sm.groups.push((id.to_string(), x.alpha, box_of(x.l)));
                // Labels centred in their container.
                if x.alpha > 0.99 {
                    let mut gb = box_of(x.l);
                    let mut ext: Option<[f32; 4]> = None;
                    let mut cap: Option<f32> = None;
                    let mut lid = String::new();
                    for c in &x.l.children {
                        if let LayerKind::Text(t) = c.kind {
                            // A button's label is centred in the button.
                            if let Some(own) =
                                c.id.strip_suffix(".label")
                                    .and_then(|b| x.l.children.iter().find(|o| o.id == b))
                            {
                                gb = box_of(own);
                            }
                            if c.id.ends_with("label")
                                && t.align == motion_core::scene::TextAlign::Center
                            {
                                let (top, bottom) =
                                    t.ink.map_or((0.0, c.height), |i| (i.top, i.bottom));
                                let desc = t.text.chars().any(|ch| "gjpqy,".contains(ch));
                                let cy = if desc {
                                    top + 0.36 * t.font_size
                                } else {
                                    (top + bottom) / 2.0
                                };
                                let (_, y) = c.transform.apply(0.0, cy);
                                cap = Some(y);
                                lid = c.id.to_string();
                                let b = box_of(c);
                                ext =
                                    Some(ext.map_or(b, |e| {
                                        [e[0].min(b[0]), e[1], e[2].max(b[2]), e[3]]
                                    }));
                            }
                        }
                        if c.id.ends_with(".icon") {
                            let b = box_of(c);
                            ext = Some(
                                ext.map_or(b, |e| [e[0].min(b[0]), e[1], e[2].max(b[2]), e[3]]),
                            );
                        }
                    }
                    let (gw, gh) = (gb[2] - gb[0], gb[3] - gb[1]);
                    if let (Some(y), Some(e)) = (cap, ext) {
                        let dy = (y - (gb[1] + gb[3]) / 2.0).abs() / gh.max(1.0);
                        let dx = ((e[0] + e[2]) / 2.0 - (gb[0] + gb[2]) / 2.0).abs() / gw.max(1.0);
                        sm.labels.push((lid, dy, dx));
                    }
                }
            }
            if (numbered(id, dev, "fill") || id == format!("{dev}.plate")) && x.alpha > 0.05 {
                if let (Some(c), Some(b)) = (solid_fill(x.l), path_bounds(x.l)) {
                    sm.fills.push((id.to_string(), c, x.alpha, b));
                }
            }
            if id.starts_with(&format!("{dev}.s")) && id.contains(".row") && id.ends_with(".g") {
                sm.arrivals.push((id.to_string(), x.alpha, box_of(x.l)));
            }
            if (id.ends_with(".tick")
                || id.rsplit('.').next().is_some_and(|r| r.starts_with("done")))
                && x.alpha > 0.3
                && x.l.trim.unwrap_or(1.0) > 0.5
            {
                sm.ticks.push(id.to_string());
            }
            if id == format!("{dev}.screen") {
                sm.screen = Some(box_of(x.l));
            }
            if id == format!("{dev}.touch") {
                // A finger is a circle (4 arcs); a pointer is a polygon
                // with its tip at the box origin.
                let disc = matches!(x.l.kind, LayerKind::Path { d, .. }
                    if motion_core::path::parse(d).is_ok_and(|p| p.contours.iter().map(|c| c.segs.len()).sum::<usize>() == 4));
                let p = if disc {
                    x.l.transform.apply(x.l.width / 2.0, x.l.height / 2.0)
                } else {
                    x.l.transform.apply(0.0, 0.0)
                };
                // Relative to the device, so the camera does not count as
                // the cursor moving.
                let dev_box = mine
                    .iter()
                    .find(|y| y.l.id == dev)
                    .map_or([0.0, 0.0, 1.0, 1.0], |y| box_of(y.l));
                let (dw, dh) = (
                    (dev_box[2] - dev_box[0]).max(1.0),
                    (dev_box[3] - dev_box[1]).max(1.0),
                );
                let b = box_of(x.l);
                sm.touch = Some((
                    [
                        (p.0 - dev_box[0]) / dw * 1000.0,
                        (p.1 - dev_box[1]) / dh * 1000.0,
                    ],
                    (b[2] - b[0]) / dw * 1000.0,
                    x.alpha,
                ));
                sm.touch_px = Some([p.0, p.1]);
            }
        }
        samples.push(sm);
    }

    // duplicate_confirmation: two success ticks on screen together.
    if let Some(sm) = samples.iter().find(|sm| sm.ticks.len() >= 2) {
        fail(
            out,
            "duplicate_confirmation",
            format!(
                "{} and {} confirm at once at {:.2}s",
                sm.ticks[0], sm.ticks[1], sm.t
            ),
        );
    }
    // ghost_crossfade: two states' content visible at once.
    if let Some(sm) = samples.iter().find(|sm| sm.groups.len() >= 2) {
        fail(
            out,
            "ghost_crossfade",
            format!(
                "{} and {} both shown at {:.2}s",
                sm.groups[0].0, sm.groups[1].0, sm.t
            ),
        );
    }
    // muddy_state_colour: two container colours mixed by opacity.
    for sm in &samples {
        let partial: Vec<&(String, Color, f32, [f32; 4])> =
            sm.fills.iter().filter(|f| f.2 > 0.1 && f.2 < 0.9).collect();
        let under = sm.fills.iter().find(|f| f.2 >= 0.9);
        if let (Some(top), Some(bot)) = (partial.first(), under.or(partial.get(1).copied())) {
            if top.1 != bot.1 {
                let a = top.2;
                let mix = |x: u8, y: u8| (x as f32 * a + y as f32 * (1.0 - a)).round() as u8;
                let m = Color::rgb(
                    mix(top.1.r, bot.1.r),
                    mix(top.1.g, bot.1.g),
                    mix(top.1.b, bot.1.b),
                );
                let (om, oa, ob) = (oklch(m), oklch(top.1), oklch(bot.1));
                if om.l < oa.l.min(ob.l) - 0.01 && om.c < oa.c.min(ob.c) - 0.01 {
                    fail(
                        out,
                        "muddy_state_colour",
                        format!(
                            "{} over {} at {:.2}s mixes to a dull {}",
                            top.0,
                            bot.0,
                            sm.t,
                            m.to_hex()
                        ),
                    );
                }
            }
        }
    }
    for m in &s.motions {
        if let MotionOp::Tint { color, from, to } = &m.op {
            if !numbered(&m.target, dev, "fill") || (to - from).abs() < 0.3 {
                continue;
            }
            if let Some(base) = find_fill(&s.layers, &m.target) {
                let k = ((from + to) / 2.0).clamp(0.0, 1.0);
                let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k).round() as u8;
                let c = Color::rgb(
                    mix(base.r, color.r),
                    mix(base.g, color.g),
                    mix(base.b, color.b),
                );
                let (om, oa, ob) = (oklch(c), oklch(base), oklch(*color));
                if om.l < oa.l.min(ob.l) - 0.01 && om.c < oa.c.min(ob.c) - 0.01 {
                    fail(
                        out,
                        "muddy_state_colour",
                        format!("{} tint lerps through {}", m.target, c.to_hex()),
                    );
                }
            }
        }
    }
    // anticipatory_space: a row lands in room the container already kept
    // for it (the container must grow to it as it arrives).
    let mut seen_rows: Vec<String> = Vec::new();
    for (i, sm) in samples.iter().enumerate() {
        for (rid, a, _) in &sm.arrivals {
            if *a < 0.5 || seen_rows.contains(rid) {
                continue;
            }
            seen_rows.push(rid.clone());
            let Some(prev) = i.checked_sub(3).map(|k| &samples[k]) else {
                continue;
            };
            let Some(fin) = samples
                .get(i + 8)
                .and_then(|x| x.arrivals.iter().find(|r| &r.0 == rid))
                .map(|r| r.2)
            else {
                continue;
            };
            // An element that stays where it was (the same row in the next
            // state) is not an arrival.
            let now = sm.arrivals.iter().find(|r| &r.0 == rid).map(|r| r.2);
            let just_before = &samples[i - 1];
            if let Some(nb) = now {
                let tol = 4.0 + 0.02 * (nb[2] - nb[0]);
                if just_before.arrivals.iter().any(|(o, a, b)| {
                    o != rid && *a > 0.5 && (0..4).all(|q| (b[q] - nb[q]).abs() < tol)
                }) {
                    continue;
                }
            }
            let before = prev
                .fills
                .iter()
                .filter(|f| f.2 > 0.9)
                .map(|f| f.3)
                .next_back();
            if let Some(b) = before {
                let inside = fin[0] >= b[0] - 2.0
                    && fin[2] <= b[2] + 2.0
                    && fin[1] >= b[1] - 2.0
                    && fin[3] <= b[3] + 2.0;
                if inside {
                    fail(
                        out,
                        "anticipatory_space",
                        format!(
                            "{rid} lands at {:.2}s in room the container kept for it",
                            sm.t
                        ),
                    );
                }
            }
        }
    }
    // content_overflow: content not masked to a morphing container.
    let morphs = s.motions.iter().any(|m| {
        matches!(m.op, MotionOp::Morph { .. })
            && (numbered(&m.target, dev, "fill") || m.target == format!("{dev}.plate"))
    });
    if morphs {
        if let Some(g) = groups_of(&s.layers, dev)
            .into_iter()
            .find(|g| g.mask.is_none())
        {
            fail(
                out,
                "content_overflow",
                format!("{} is not masked to the morphing container", g.id),
            );
        }
    }
    // text_off_centre_in_container.
    for sm in &samples {
        if let Some((id, dy, dx)) = sm.labels.iter().find(|l| l.1 > 0.025 || l.2 > 0.02) {
            fail(
                out,
                "text_off_centre_in_container",
                format!(
                    "{id} is {:.1} % off the container's centre (y), {:.1} % (x) at {:.2}s",
                    dy * 100.0,
                    dx * 100.0,
                    sm.t
                ),
            );
            break;
        }
    }
    // Presses: taps, ripples, dead clicks.
    let touch_id = format!("{dev}.touch");
    let presses: Vec<f64> = s
        .motions
        .iter()
        .filter(|m| {
            m.target == touch_id
                && matches!(m.op, MotionOp::Scale { from, to, .. } if from >= 0.99 && to < 0.99)
        })
        .map(|m| m.start)
        .collect();
    for (n, &t) in presses.iter().enumerate() {
        let Some(sm) = samples
            .iter()
            .min_by(|a, b| (a.t - t).abs().total_cmp(&(b.t - t).abs()))
        else {
            continue;
        };
        let Some(pt) = sm.touch_px else { continue };
        // The tapped shape: the smallest visible container/row/CTA under
        // the finger.
        let targets = tap_targets(scene, s, dev, t);
        let hit = targets
            .iter()
            .filter(|(_, b, _)| pt[0] >= b[0] && pt[0] <= b[2] && pt[1] >= b[1] && pt[1] <= b[3])
            .min_by(|a, b| area(&a.1).total_cmp(&area(&b.1)));
        match hit {
            None => fail(
                out,
                "tap_misses_target",
                format!("tap {n} at {t:.2}s lands on no shape"),
            ),
            Some((tid, b, colour)) => {
                let bh = b[3] - b[1];
                let margin = (pt[0] - b[0])
                    .min(b[2] - pt[0])
                    .min(pt[1] - b[1])
                    .min(b[3] - pt[1]);
                if margin < 0.25 * bh - 1.0 {
                    fail(
                        out,
                        "tap_misses_target",
                        format!("tap {n} at {t:.2}s is {margin:.0}px from the edge of {tid} (h {bh:.0})"),
                    );
                }
                let rid = format!("{dev}.ripple{n}");
                if let Some(rc) = find_fill(&s.layers, &rid) {
                    let c = contrast(Color { a: 255, ..rc }, *colour);
                    if c < 3.0 {
                        fail(out, "ripple_invisible", format!("{rid} {c:.2}:1 on {tid}"));
                    }
                }
            }
        }
        // Something answers within 0.5 s.
        let answers = s.motions.iter().any(|m| {
            m.target != touch_id
                && !m.target.contains(".ripple")
                && m.start >= t - 1e-6
                && m.start <= t + 0.5
        });
        if !answers {
            fail(out, "dead_click", format!("tap at {t:.2}s changes nothing"));
        }
    }
    // idle_cursor: visible and still for > 1 s.
    let mut still_from: Option<(f64, [f32; 2], f32)> = None;
    for sm in &samples {
        match sm.touch {
            Some((p, sz, a)) if a > 0.05 => match still_from {
                Some((t0, q, z))
                    if (p[0] - q[0]).abs() < 0.5
                        && (p[1] - q[1]).abs() < 0.5
                        && (sz - z).abs() < 0.5 =>
                {
                    if sm.t - t0 > 1.0 {
                        fail(
                            out,
                            "idle_cursor",
                            format!("the cursor sits still from {t0:.2}s to {:.2}s", sm.t),
                        );
                    }
                }
                _ => still_from = Some((sm.t, p, sz)),
            },
            _ => still_from = None,
        }
    }
    // read_time: each state's content shown long enough to read.
    let mut spans: Vec<(String, f64, f64)> = Vec::new();
    for sm in &samples {
        for (g, a, _) in &sm.groups {
            if *a < 0.95 {
                continue;
            }
            match spans.iter_mut().rev().find(|x| &x.0 == g) {
                Some(x) if sm.t - x.2 < 0.1 => x.2 = sm.t,
                _ => spans.push((g.clone(), sm.t, sm.t)),
            }
        }
    }
    for (g, t0, t1) in &spans {
        let lines = groups_of(&s.layers, dev)
            .iter()
            .find(|l| &l.id == g)
            .map_or(1, |l| text_count(l));
        let need = 0.4 + 0.3 * lines as f64 - 0.12;
        let end = s.duration_seconds - 0.1;
        if t1 - t0 < need && *t1 < end {
            fail(
                out,
                "read_time",
                format!(
                    "{g} reads for {:.2}s from {t0:.2}s; {lines} line(s) need {:.1}s",
                    t1 - t0,
                    need + 0.12
                ),
            );
        }
    }
    // ui_underfill: judged at the end of each state's hold.
    for (g, _, t1) in &spans {
        // Settled, before any press at the end of the hold.
        let Some(sm) = samples.iter().rev().find(|x| x.t <= *t1 - 0.15 + 1e-6) else {
            continue;
        };
        if !sm.groups.iter().any(|x| &x.0 == g) {
            continue;
        }
        let Some(b) = sm
            .fills
            .iter()
            .filter(|f| f.2 > 0.9)
            .map(|f| f.3)
            .next_back()
        else {
            continue;
        };
        let (bw, bh) = (b[2] - b[0], b[3] - b[1]);
        let width = bw.min(w) / w;
        let of_screen = sm.screen.map_or(0.0, |sc| {
            let area_share = bw * bh / area(&sc).max(1.0);
            let zoomed_phone = (sc[3] - sc[1]) > 1.8 * h && bw >= 0.25 * (sc[2] - sc[0]);
            if zoomed_phone {
                area_share.max(0.15)
            } else {
                area_share
            }
        });
        let of_frame = (bw.min(w) * bh.min(h)) / (w * h);
        if width < 0.45 && of_frame < 0.15 && of_screen < 0.15 {
            fail(
                out,
                "ui_underfill",
                format!(
                    "{g} fills {:.0} % of the frame width at {t1:.2}s",
                    width * 100.0
                ),
            );
        }
    }
    // flat_elevation: a hairline on the container, or no shadow and no
    // contrast with the screen.
    let screen = find_fill(&s.layers, &format!("{dev}.screen")).unwrap_or(scene.canvas.background);
    let shadowed = has_shadow_plate(&s.layers, dev);
    visit(&s.layers, &mut |l| {
        if !numbered(&l.id, dev, "fill") {
            return;
        }
        if let LayerKind::Path {
            fill: Some(Fill::Solid(c)),
            stroke,
            ..
        } = &l.kind
        {
            if stroke.is_some() {
                fail(
                    out,
                    "flat_elevation",
                    format!("{} is outlined with a hairline", l.id),
                );
            } else if !shadowed && contrast(*c, screen) < 1.3 {
                fail(
                    out,
                    "flat_elevation",
                    format!(
                        "{} {:.2}:1 on the screen with no shadow",
                        l.id,
                        contrast(*c, screen)
                    ),
                );
            }
        }
    });
    // dead_tail: > 1 s with nothing moving at the end.
    let last_move = s
        .motions
        .iter()
        .map(|m| m.start + m.duration)
        .chain(
            s.camera
                .iter()
                .flat_map(|c| c.motions.iter().map(|m| m.start + m.duration)),
        )
        .fold(0.0f64, f64::max);
    if s.duration_seconds - last_move > 1.0 {
        fail(
            out,
            "dead_tail",
            format!(
                "nothing moves for the last {:.2}s",
                s.duration_seconds - last_move
            ),
        );
    }
    // spring_truncated: a spring cut before it settles.
    for m in &s.motions {
        if let Some(sp) = &m.spring {
            let need = settle(sp);
            let cut = s
                .motions
                .iter()
                .filter(|o| {
                    o.target == m.target
                        && o.op.channel() == m.op.channel()
                        && o.start > m.start + 1e-6
                })
                .map(|o| o.start)
                .fold(m.start + m.duration, f64::min);
            if cut - m.start < need * 0.98 - 1e-3 {
                fail(
                    out,
                    "spring_truncated",
                    format!(
                        "{} {} cut at {:.2}s of {:.2}s",
                        m.target,
                        m.op.op_name(),
                        cut - m.start,
                        need
                    ),
                );
            }
        }
    }
    // Type: two weights at least, a small scale, one size per role.
    let mut weights = Vec::new();
    let mut sizes: Vec<f32> = Vec::new();
    let mut roles: Vec<(String, f32)> = Vec::new();
    for g in groups_of(&s.layers, dev) {
        visit(std::slice::from_ref(g), &mut |l| {
            if let LayerKind::Text(t) = &l.kind {
                if !weights.contains(&t.font_weight) {
                    weights.push(t.font_weight);
                }
                let sz = (t.font_size * 2.0).round() / 2.0;
                if !sizes.contains(&sz) {
                    sizes.push(sz);
                }
                let role = l.id.rsplit('.').next().unwrap_or("").to_string();
                if !roles.iter().any(|r| r.0 == role && r.1 == sz) {
                    roles.push((role, sz));
                }
            }
        });
    }
    let n_text: usize = groups_of(&s.layers, dev)
        .iter()
        .map(|g| text_count(g))
        .sum();
    if n_text >= 3 && weights.len() < 2 {
        fail(
            out,
            "single_weight_ui",
            format!("every UI label is weight {:?}", weights),
        );
    }
    sizes.sort_by(f32::total_cmp);
    // The hero number is outside the scale.
    if sizes.len() >= 2 && sizes[sizes.len() - 1] >= 2.0 * sizes[sizes.len() - 2] {
        sizes.pop();
    }
    if sizes.len() > 4 {
        fail(
            out,
            "ui_type_scale",
            format!("{} type sizes {:?}", sizes.len(), sizes),
        );
    }
    for (role, sz) in &roles {
        if roles.iter().any(|r| &r.0 == role && (r.1 - sz).abs() > 0.5) {
            fail(
                out,
                "ui_type_scale",
                format!("role '{role}' set at different sizes"),
            );
            break;
        }
    }
}

fn area(b: &[f32; 4]) -> f32 {
    (b[2] - b[0]) * (b[3] - b[1])
}

fn groups_of<'a>(
    ls: &'a [motion_core::scene::Layer],
    dev: &str,
) -> Vec<&'a motion_core::scene::Layer> {
    let mut out = Vec::new();
    fn walk<'a>(
        ls: &'a [motion_core::scene::Layer],
        dev: &str,
        out: &mut Vec<&'a motion_core::scene::Layer>,
    ) {
        for l in ls {
            if numbered(&l.id, dev, "s") && matches!(l.kind, LayerKind::Group { .. }) {
                out.push(l);
            } else if let LayerKind::Group { children } = &l.kind {
                walk(children, dev, out);
            }
        }
    }
    walk(ls, dev, &mut out);
    out
}

fn text_count(l: &motion_core::scene::Layer) -> usize {
    let mut n = 0;
    visit(std::slice::from_ref(l), &mut |x| {
        if matches!(x.kind, LayerKind::Text(_)) {
            n += 1;
        }
    });
    n
}

fn find_fill(ls: &[motion_core::scene::Layer], id: &str) -> Option<Color> {
    let mut c = None;
    visit(ls, &mut |l| {
        if l.id == id {
            if let LayerKind::Path {
                fill: Some(Fill::Solid(f)),
                ..
            } = &l.kind
            {
                c = Some(*f);
            }
        }
    });
    c
}

fn has_shadow_plate(ls: &[motion_core::scene::Layer], dev: &str) -> bool {
    let mut yes = false;
    visit(ls, &mut |l| {
        if l.id == format!("{dev}.plate") {
            if let LayerKind::Path { shadows, .. } = &l.kind {
                yes = !shadows.is_empty();
            }
        }
    });
    yes
}

/// Tappable shapes at scene time `t`: visible container fills, rows and
/// CTAs, with their canvas bounds and colour.
fn tap_targets(
    scene: &MotionProject,
    s: &Scene,
    dev: &str,
    t: f64,
) -> Vec<(String, [f32; 4], Color)> {
    let fps = scene.canvas.fps as f64;
    let f = ((s.start_seconds + t) * fps).round() as u32;
    let Ok(rf) = motion_core::evaluate_frame(scene, f.min(scene.frame_count().saturating_sub(1)))
    else {
        return Vec::new();
    };
    let mut flat = Vec::new();
    flatten(&rf.layers, 1.0, &mut flat);
    flat.iter()
        .filter(|x| x.l.scene == Some(dev) && x.alpha > 0.5)
        .filter(|x| {
            numbered(x.l.id, dev, "fill")
                || x.l.id.ends_with(".cta")
                || x.l.id.rsplit('.').next().is_some_and(|r| {
                    r.strip_prefix("row")
                        .is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()))
                })
        })
        .filter_map(|x| Some((x.l.id.to_string(), path_bounds(x.l)?, solid_fill(x.l)?)))
        .collect()
}
