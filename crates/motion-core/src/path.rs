//! (v2) Filled Bezier paths: SVG path data parsing, trimming and
//! point-matched morphing. Pure geometry in a layer's box space (pixels);
//! the renderer only draws what this produces.
//!
//! Every segment is normalised to a cubic (lines and quadratics are degree
//! elevated, arcs are split into ≤ 90° cubics), so morphing is a lerp of
//! control points once both paths have the same contour and segment counts
//! (see [`morph`]).

use serde::Serialize;
use std::fmt;

/// One cubic segment from the previous end point.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Cubic {
    pub c1: [f32; 2],
    pub c2: [f32; 2],
    pub to: [f32; 2],
}

/// A sub-path: a start point and cubic segments, optionally closed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Contour {
    pub start: [f32; 2],
    pub segs: Vec<Cubic>,
    pub closed: bool,
}

/// A path of contours.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct BezPath {
    pub contours: Vec<Contour>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathError(pub String);

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "path data: {}", self.0)
    }
}

impl std::error::Error for PathError {}

fn lerp2(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

fn line(from: [f32; 2], to: [f32; 2]) -> Cubic {
    Cubic {
        c1: lerp2(from, to, 1.0 / 3.0),
        c2: lerp2(from, to, 2.0 / 3.0),
        to,
    }
}

impl Cubic {
    /// Point at parameter `t` from `p0`.
    pub fn at(&self, p0: [f32; 2], t: f32) -> [f32; 2] {
        let u = 1.0 - t;
        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        [
            a * p0[0] + b * self.c1[0] + c * self.c2[0] + d * self.to[0],
            a * p0[1] + b * self.c1[1] + c * self.c2[1] + d * self.to[1],
        ]
    }

    /// De Casteljau split at `t`: the two halves.
    pub fn split(&self, p0: [f32; 2], t: f32) -> (Cubic, Cubic) {
        let ab = lerp2(p0, self.c1, t);
        let bc = lerp2(self.c1, self.c2, t);
        let cd = lerp2(self.c2, self.to, t);
        let abc = lerp2(ab, bc, t);
        let bcd = lerp2(bc, cd, t);
        let m = lerp2(abc, bcd, t);
        (
            Cubic {
                c1: ab,
                c2: abc,
                to: m,
            },
            Cubic {
                c1: bcd,
                c2: cd,
                to: self.to,
            },
        )
    }

    /// Arc length by 16-step flattening (deterministic, plenty for trims).
    pub fn length(&self, p0: [f32; 2]) -> f32 {
        let mut len = 0.0;
        let mut prev = p0;
        for i in 1..=16 {
            let p = self.at(p0, i as f32 / 16.0);
            len += dist(prev, p);
            prev = p;
        }
        len
    }

    fn reversed(&self, p0: [f32; 2]) -> Cubic {
        Cubic {
            c1: self.c2,
            c2: self.c1,
            to: p0,
        }
    }
}

impl Contour {
    /// End points of every segment, starting with `start`.
    fn nodes(&self) -> Vec<[f32; 2]> {
        let mut v = vec![self.start];
        v.extend(self.segs.iter().map(|s| s.to));
        v
    }

    pub fn length(&self) -> f32 {
        let mut p0 = self.start;
        let mut len = 0.0;
        for s in &self.segs {
            len += s.length(p0);
            p0 = s.to;
        }
        len
    }

    /// Shoelace area of the control polygon (sign = winding; y down, so
    /// positive is clockwise on screen).
    pub fn signed_area(&self) -> f32 {
        let mut pts = vec![self.start];
        for s in &self.segs {
            pts.extend([s.c1, s.c2, s.to]);
        }
        let n = pts.len();
        let mut a = 0.0;
        for i in 0..n {
            let (p, q) = (pts[i], pts[(i + 1) % n]);
            a += p[0] * q[1] - q[0] * p[1];
        }
        a * 0.5
    }

    /// Mean of the segment end points (a closed contour's start is its last
    /// end, so it counts once).
    fn centroid(&self) -> [f32; 2] {
        let nodes: Vec<[f32; 2]> = if self.segs.is_empty() {
            vec![self.start]
        } else {
            self.segs.iter().map(|s| s.to).collect()
        };
        let n = nodes.len() as f32;
        let s = nodes
            .iter()
            .fold([0.0, 0.0], |acc, p| [acc[0] + p[0], acc[1] + p[1]]);
        [s[0] / n, s[1] / n]
    }

    /// The same contour drawn the other way round.
    fn reversed(&self) -> Contour {
        let nodes = self.nodes();
        let mut segs = Vec::with_capacity(self.segs.len());
        for (i, s) in self.segs.iter().enumerate().rev() {
            segs.push(s.reversed(nodes[i]));
        }
        Contour {
            start: *nodes.last().unwrap_or(&self.start),
            segs,
            closed: self.closed,
        }
    }

    /// Closed contours only: start at node `k` instead of node 0.
    fn rotated(&self, k: usize) -> Contour {
        let n = self.segs.len();
        if n == 0 || k.is_multiple_of(n) {
            return self.clone();
        }
        let nodes = self.nodes();
        let segs = (0..n).map(|i| self.segs[(k + i) % n]).collect();
        Contour {
            start: nodes[k % n],
            segs,
            closed: self.closed,
        }
    }

    /// Splits the longest segment in half until there are `n` segments.
    fn subdivided_to(&self, n: usize) -> Contour {
        let mut c = self.clone();
        if c.segs.is_empty() {
            c.segs = vec![line(c.start, c.start); n];
            return c;
        }
        while c.segs.len() < n {
            let mut p0 = c.start;
            let mut best = (0usize, -1.0f32, c.start);
            for (i, s) in c.segs.iter().enumerate() {
                let l = s.length(p0);
                if l > best.1 {
                    best = (i, l, p0);
                }
                p0 = s.to;
            }
            let (a, b) = c.segs[best.0].split(best.2, 0.5);
            c.segs[best.0] = a;
            c.segs.insert(best.0 + 1, b);
        }
        c
    }

    /// A contour of `n` zero-length segments at `p` (morph padding).
    fn point(p: [f32; 2], n: usize, closed: bool) -> Contour {
        Contour {
            start: p,
            segs: vec![
                Cubic {
                    c1: p,
                    c2: p,
                    to: p
                };
                n.max(1)
            ],
            closed,
        }
    }
}

impl BezPath {
    pub fn is_empty(&self) -> bool {
        self.contours.iter().all(|c| c.segs.is_empty())
    }

    pub fn length(&self) -> f32 {
        self.contours.iter().map(Contour::length).sum()
    }

    pub fn is_finite(&self) -> bool {
        self.contours.iter().all(|c| {
            c.start.iter().all(|v| v.is_finite())
                && c.segs.iter().all(|s| {
                    [s.c1, s.c2, s.to]
                        .iter()
                        .all(|p| p[0].is_finite() && p[1].is_finite())
                })
        })
    }

    /// Axis-aligned bounds of all control points `[min_x, min_y, max_x, max_y]`.
    pub fn bounds(&self) -> Option<[f32; 4]> {
        let mut b: Option<[f32; 4]> = None;
        for c in &self.contours {
            let mut pts = vec![c.start];
            for s in &c.segs {
                pts.extend([s.c1, s.c2, s.to]);
            }
            for p in pts {
                b = Some(match b {
                    None => [p[0], p[1], p[0], p[1]],
                    Some(r) => [
                        r[0].min(p[0]),
                        r[1].min(p[1]),
                        r[2].max(p[0]),
                        r[3].max(p[1]),
                    ],
                });
            }
        }
        b
    }

    /// The first `fraction` (0..=1) of the total length, contours in order.
    /// A closed contour cut short becomes open.
    pub fn trimmed(&self, fraction: f32) -> BezPath {
        let f = if fraction.is_nan() {
            0.0
        } else {
            fraction.clamp(0.0, 1.0)
        };
        if f >= 1.0 {
            return self.clone();
        }
        let mut left = self.length() * f;
        let mut out = BezPath::default();
        for c in &self.contours {
            if left <= 0.0 {
                break;
            }
            let mut segs = Vec::new();
            let mut p0 = c.start;
            let mut whole = true;
            for s in &c.segs {
                let l = s.length(p0);
                if l <= left {
                    segs.push(*s);
                    left -= l;
                    p0 = s.to;
                    continue;
                }
                // Cut inside this segment at the parameter whose flattened
                // length matches what is left.
                let t = param_at_length(s, p0, left);
                segs.push(s.split(p0, t).0);
                left = 0.0;
                whole = false;
                break;
            }
            out.contours.push(Contour {
                start: c.start,
                segs,
                closed: c.closed && whole,
            });
        }
        out
    }

    /// SVG path data (absolute `M`/`C`/`Z`), for scene files and debugging.
    pub fn to_svg(&self) -> String {
        let mut s = String::new();
        let n = |v: f32| {
            let r = (v * 100.0).round() / 100.0;
            if r == r.trunc() {
                format!("{}", r as i64)
            } else {
                format!("{r}")
            }
        };
        for c in &self.contours {
            s.push_str(&format!("M{} {}", n(c.start[0]), n(c.start[1])));
            for g in &c.segs {
                s.push_str(&format!(
                    "C{} {} {} {} {} {}",
                    n(g.c1[0]),
                    n(g.c1[1]),
                    n(g.c2[0]),
                    n(g.c2[1]),
                    n(g.to[0]),
                    n(g.to[1])
                ));
            }
            if c.closed {
                s.push('Z');
            }
        }
        s
    }
}

fn param_at_length(s: &Cubic, p0: [f32; 2], target: f32) -> f32 {
    let mut acc = 0.0;
    let mut prev = p0;
    for i in 1..=64 {
        let t = i as f32 / 64.0;
        let p = s.at(p0, t);
        let l = dist(prev, p);
        if acc + l >= target {
            let u = if l > 0.0 { (target - acc) / l } else { 0.0 };
            return (t - 1.0 / 64.0) + u / 64.0;
        }
        acc += l;
        prev = p;
    }
    1.0
}

// ---------------------------------------------------------------------------
// Morph
// ---------------------------------------------------------------------------

/// `a` and `b` brought to the same shape structure: equal contour counts
/// (missing contours become points at the partner's centroid, so they grow
/// from their centre), equal segment counts per contour (longest segments
/// split), closed contours rotated and, when windings differ, reversed so
/// nodes travel the shortest total distance. Deterministic.
pub fn match_paths(a: &BezPath, b: &BezPath) -> (BezPath, BezPath) {
    let n = a.contours.len().max(b.contours.len());
    let mut ra = BezPath::default();
    let mut rb = BezPath::default();
    for i in 0..n {
        let (ca, cb) = match (a.contours.get(i), b.contours.get(i)) {
            (Some(x), Some(y)) => (x.clone(), y.clone()),
            (Some(x), None) => (
                x.clone(),
                Contour::point(x.centroid(), x.segs.len(), x.closed),
            ),
            (None, Some(y)) => (
                Contour::point(y.centroid(), y.segs.len(), y.closed),
                y.clone(),
            ),
            (None, None) => continue,
        };
        let k = ca.segs.len().max(cb.segs.len()).max(1);
        let ca = ca.subdivided_to(k);
        let mut cb = cb.subdivided_to(k);
        if ca.closed && cb.closed {
            let degenerate = |c: &Contour| c.signed_area().abs() < 1e-3;
            if !degenerate(&ca)
                && !degenerate(&cb)
                && ca.signed_area().signum() != cb.signed_area().signum()
            {
                cb = cb.reversed();
            }
            let na = ca.nodes();
            let best = (0..k)
                .min_by(|&x, &y| {
                    let cost = |r: usize| {
                        let nb = cb.rotated(r).nodes();
                        na.iter().zip(&nb).map(|(p, q)| dist(*p, *q)).sum::<f32>()
                    };
                    cost(x).total_cmp(&cost(y))
                })
                .unwrap_or(0);
            cb = cb.rotated(best);
        }
        ra.contours.push(ca);
        rb.contours.push(cb);
    }
    (ra, rb)
}

/// The path between `a` (p = 0) and `b` (p = 1). Ends return the inputs
/// exactly; in between, both are matched with [`match_paths`] and control
/// points interpolate (p may overshoot for springs).
pub fn morph(a: &BezPath, b: &BezPath, p: f32) -> BezPath {
    if p <= 0.0 && p.abs() < f32::EPSILON {
        return a.clone();
    }
    if (p - 1.0).abs() < f32::EPSILON {
        return b.clone();
    }
    let (ma, mb) = match_paths(a, b);
    BezPath {
        contours: ma
            .contours
            .iter()
            .zip(&mb.contours)
            .map(|(x, y)| Contour {
                start: lerp2(x.start, y.start, p),
                segs: x
                    .segs
                    .iter()
                    .zip(&y.segs)
                    .map(|(s, t)| Cubic {
                        c1: lerp2(s.c1, t.c1, p),
                        c2: lerp2(s.c2, t.c2, p),
                        to: lerp2(s.to, t.to, p),
                    })
                    .collect(),
                closed: if p < 0.5 { x.closed } else { y.closed },
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parses SVG path data: `M L H V C S Q T A Z`, absolute and relative.
pub fn parse(d: &str) -> Result<BezPath, PathError> {
    let mut lx = Lexer {
        s: d.as_bytes(),
        i: 0,
    };
    let mut out = BezPath::default();
    let mut cur: Option<Contour> = None;
    let mut pos = [0.0f32, 0.0];
    let mut sub_start = [0.0f32, 0.0];
    let mut last_cmd = 0u8;
    let mut last_c2: Option<[f32; 2]> = None; // for S
    let mut last_q: Option<[f32; 2]> = None; // for T

    fn flush(out: &mut BezPath, cur: &mut Option<Contour>) {
        if let Some(c) = cur.take() {
            if !c.segs.is_empty() {
                out.contours.push(c);
            }
        }
    }

    loop {
        lx.skip_ws();
        if lx.i >= lx.s.len() {
            break;
        }
        let ch = lx.s[lx.i];
        let cmd = if ch.is_ascii_alphabetic() {
            lx.i += 1;
            ch
        } else {
            match last_cmd {
                0 => return Err(PathError("must start with a command".into())),
                b'M' => b'L',
                b'm' => b'l',
                b'Z' | b'z' => return Err(PathError("number after Z".into())),
                c => c,
            }
        };
        let rel = cmd.is_ascii_lowercase();
        let off = |p: [f32; 2], pos: [f32; 2]| {
            if rel {
                [p[0] + pos[0], p[1] + pos[1]]
            } else {
                p
            }
        };
        let up = cmd.to_ascii_uppercase();
        let mut c2_now = None;
        let mut q_now = None;
        match up {
            b'M' => {
                flush(&mut out, &mut cur);
                pos = off(lx.pair()?, pos);
                sub_start = pos;
                cur = Some(Contour {
                    start: pos,
                    segs: Vec::new(),
                    closed: false,
                });
            }
            b'Z' => {
                if let Some(c) = cur.as_mut() {
                    if dist(pos, sub_start) > 1e-4 {
                        c.segs.push(line(pos, sub_start));
                    }
                    c.closed = true;
                }
                flush(&mut out, &mut cur);
                pos = sub_start;
                cur = Some(Contour {
                    start: pos,
                    segs: Vec::new(),
                    closed: false,
                });
            }
            _ => {
                let c = cur.get_or_insert(Contour {
                    start: pos,
                    segs: Vec::new(),
                    closed: false,
                });
                match up {
                    b'L' => {
                        let p = off(lx.pair()?, pos);
                        c.segs.push(line(pos, p));
                        pos = p;
                    }
                    b'H' => {
                        let x = lx.num()? + if rel { pos[0] } else { 0.0 };
                        let p = [x, pos[1]];
                        c.segs.push(line(pos, p));
                        pos = p;
                    }
                    b'V' => {
                        let y = lx.num()? + if rel { pos[1] } else { 0.0 };
                        let p = [pos[0], y];
                        c.segs.push(line(pos, p));
                        pos = p;
                    }
                    b'C' => {
                        let c1 = off(lx.pair()?, pos);
                        let c2 = off(lx.pair()?, pos);
                        let p = off(lx.pair()?, pos);
                        c.segs.push(Cubic { c1, c2, to: p });
                        c2_now = Some(c2);
                        pos = p;
                    }
                    b'S' => {
                        let c1 = match (last_cmd.to_ascii_uppercase(), last_c2) {
                            (b'C' | b'S', Some(k)) => [2.0 * pos[0] - k[0], 2.0 * pos[1] - k[1]],
                            _ => pos,
                        };
                        let c2 = off(lx.pair()?, pos);
                        let p = off(lx.pair()?, pos);
                        c.segs.push(Cubic { c1, c2, to: p });
                        c2_now = Some(c2);
                        pos = p;
                    }
                    b'Q' | b'T' => {
                        let q = if up == b'Q' {
                            off(lx.pair()?, pos)
                        } else {
                            match (last_cmd.to_ascii_uppercase(), last_q) {
                                (b'Q' | b'T', Some(k)) => {
                                    [2.0 * pos[0] - k[0], 2.0 * pos[1] - k[1]]
                                }
                                _ => pos,
                            }
                        };
                        let p = off(lx.pair()?, pos);
                        c.segs.push(Cubic {
                            c1: lerp2(pos, q, 2.0 / 3.0),
                            c2: lerp2(p, q, 2.0 / 3.0),
                            to: p,
                        });
                        q_now = Some(q);
                        pos = p;
                    }
                    b'A' => {
                        let rx = lx.num()?;
                        let ry = lx.num()?;
                        let rot = lx.num()?;
                        let large = lx.flag()?;
                        let sweep = lx.flag()?;
                        let p = off(lx.pair()?, pos);
                        c.segs
                            .extend(arc_to_cubics(pos, rx, ry, rot, large, sweep, p));
                        pos = p;
                    }
                    other => return Err(PathError(format!("unknown command '{}'", other as char))),
                }
            }
        }
        last_c2 = c2_now;
        last_q = q_now;
        last_cmd = cmd;
    }
    flush(&mut out, &mut cur);
    if !out.is_finite() {
        return Err(PathError("non-finite coordinate".into()));
    }
    Ok(out)
}

struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
}

impl Lexer<'_> {
    fn skip_ws(&mut self) {
        while self.i < self.s.len()
            && (self.s[self.i].is_ascii_whitespace() || self.s[self.i] == b',')
        {
            self.i += 1;
        }
    }

    fn num(&mut self) -> Result<f32, PathError> {
        self.skip_ws();
        let start = self.i;
        let s = self.s;
        if self.i < s.len() && (s[self.i] == b'-' || s[self.i] == b'+') {
            self.i += 1;
        }
        let mut dot = false;
        while self.i < s.len() && (s[self.i].is_ascii_digit() || (s[self.i] == b'.' && !dot)) {
            dot |= s[self.i] == b'.';
            self.i += 1;
        }
        if self.i < s.len() && (s[self.i] == b'e' || s[self.i] == b'E') {
            self.i += 1;
            if self.i < s.len() && (s[self.i] == b'-' || s[self.i] == b'+') {
                self.i += 1;
            }
            while self.i < s.len() && s[self.i].is_ascii_digit() {
                self.i += 1;
            }
        }
        std::str::from_utf8(&s[start..self.i])
            .ok()
            .and_then(|t| t.parse::<f32>().ok())
            .ok_or_else(|| PathError(format!("expected a number at byte {start}")))
    }

    fn pair(&mut self) -> Result<[f32; 2], PathError> {
        Ok([self.num()?, self.num()?])
    }

    fn flag(&mut self) -> Result<bool, PathError> {
        self.skip_ws();
        match self.s.get(self.i) {
            Some(b'0') => {
                self.i += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.i += 1;
                Ok(true)
            }
            _ => Err(PathError(format!(
                "expected an arc flag at byte {}",
                self.i
            ))),
        }
    }
}

/// SVG endpoint arc → cubics of at most 90° each (W3C SVG 1.1 F.6.5).
fn arc_to_cubics(
    p0: [f32; 2],
    rx: f32,
    ry: f32,
    rot_deg: f32,
    large: bool,
    sweep: bool,
    p1: [f32; 2],
) -> Vec<Cubic> {
    let (mut rx, mut ry) = (rx.abs() as f64, ry.abs() as f64);
    if dist(p0, p1) < 1e-6 {
        return Vec::new();
    }
    if rx < 1e-9 || ry < 1e-9 {
        return vec![line(p0, p1)];
    }
    let phi = (rot_deg as f64).to_radians();
    let (sin, cos) = phi.sin_cos();
    let (x0, y0, x1, y1) = (p0[0] as f64, p0[1] as f64, p1[0] as f64, p1[1] as f64);
    let dx = (x0 - x1) / 2.0;
    let dy = (y0 - y1) / 2.0;
    let x1p = cos * dx + sin * dy;
    let y1p = -sin * dx + cos * dy;
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut coef = if den > 0.0 {
        (num / den).max(0.0).sqrt()
    } else {
        0.0
    };
    if large == sweep {
        coef = -coef;
    }
    let cxp = coef * rx * y1p / ry;
    let cyp = -coef * ry * x1p / rx;
    let cx = cos * cxp - sin * cyp + (x0 + x1) / 2.0;
    let cy = sin * cxp + cos * cyp + (y0 + y1) / 2.0;
    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let th1 = angle(1.0, 0.0, (x1p - cxp) / rx, (y1p - cyp) / ry);
    let mut dth = angle(
        (x1p - cxp) / rx,
        (y1p - cyp) / ry,
        (-x1p - cxp) / rx,
        (-y1p - cyp) / ry,
    );
    if !sweep && dth > 0.0 {
        dth -= std::f64::consts::TAU;
    } else if sweep && dth < 0.0 {
        dth += std::f64::consts::TAU;
    }
    let n = (dth.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = dth / n as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let pt = |t: f64| {
        let (s, c) = t.sin_cos();
        (
            cx + rx * c * cos - ry * s * sin,
            cy + rx * c * sin + ry * s * cos,
            -rx * s * cos - ry * c * sin,
            -rx * s * sin + ry * c * cos,
        )
    };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t0 = th1 + step * i as f64;
        let t1 = t0 + step;
        let (ax, ay, adx, ady) = pt(t0);
        let (bx, by, bdx, bdy) = pt(t1);
        let to = if i + 1 == n {
            p1
        } else {
            [bx as f32, by as f32]
        };
        out.push(Cubic {
            c1: [(ax + k * adx) as f32, (ay + k * ady) as f32],
            c2: [(bx - k * bdx) as f32, (by - k * bdy) as f32],
            to,
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Primitive shapes (box-space path data, used by v2 lowering and tests)
// ---------------------------------------------------------------------------

/// Kappa for a quarter circle as one cubic.
const KAPPA: f32 = 0.552_284_8;

/// A rounded rectangle `[x, y, w, h]` with corner radius `r` (clamped to
/// half the short side), clockwise from the top-left corner's end, as four
/// sides and four corner cubics (8 segments; `r = 0` keeps 8 segments so
/// rects, pills and circles morph cleanly into each other).
pub fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> BezPath {
    let r = r.clamp(0.0, w.min(h) / 2.0);
    let k = r * KAPPA;
    let (l, t, rt, b) = (x, y, x + w, y + h);
    let start = [l + r, t];
    let mut segs = Vec::with_capacity(8);
    let mut pos = start;
    let push_line = |segs: &mut Vec<Cubic>, pos: &mut [f32; 2], p: [f32; 2]| {
        segs.push(line(*pos, p));
        *pos = p;
    };
    push_line(&mut segs, &mut pos, [rt - r, t]);
    segs.push(Cubic {
        c1: [rt - r + k, t],
        c2: [rt, t + r - k],
        to: [rt, t + r],
    });
    pos = [rt, t + r];
    push_line(&mut segs, &mut pos, [rt, b - r]);
    segs.push(Cubic {
        c1: [rt, b - r + k],
        c2: [rt - r + k, b],
        to: [rt - r, b],
    });
    pos = [rt - r, b];
    push_line(&mut segs, &mut pos, [l + r, b]);
    segs.push(Cubic {
        c1: [l + r - k, b],
        c2: [l, b - r + k],
        to: [l, b - r],
    });
    pos = [l, b - r];
    push_line(&mut segs, &mut pos, [l, t + r]);
    segs.push(Cubic {
        c1: [l, t + r - k],
        c2: [l + r - k, t],
        to: start,
    });
    BezPath {
        contours: vec![Contour {
            start,
            segs,
            closed: true,
        }],
    }
}

/// Ellipse inscribed in `[x, y, w, h]`, clockwise from the top.
pub fn ellipse(x: f32, y: f32, w: f32, h: f32) -> BezPath {
    let (cx, cy, rx, ry) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    let top = [cx, cy - ry];
    let right = [cx + rx, cy];
    let bottom = [cx, cy + ry];
    let left = [cx - rx, cy];
    BezPath {
        contours: vec![Contour {
            start: top,
            segs: vec![
                Cubic {
                    c1: [cx + kx, top[1]],
                    c2: [right[0], cy - ky],
                    to: right,
                },
                Cubic {
                    c1: [right[0], cy + ky],
                    c2: [cx + kx, bottom[1]],
                    to: bottom,
                },
                Cubic {
                    c1: [cx - kx, bottom[1]],
                    c2: [left[0], cy + ky],
                    to: left,
                },
                Cubic {
                    c1: [left[0], cy - ky],
                    c2: [cx - kx, top[1]],
                    to: top,
                },
            ],
            closed: true,
        }],
    }
}

/// Closed polygon through `pts` (straight sides).
pub fn polygon(pts: &[[f32; 2]]) -> BezPath {
    let Some(&start) = pts.first() else {
        return BezPath::default();
    };
    let mut segs = Vec::with_capacity(pts.len());
    let mut pos = start;
    for &p in pts[1..].iter().chain(std::iter::once(&start)) {
        segs.push(line(pos, p));
        pos = p;
    }
    BezPath {
        contours: vec![Contour {
            start,
            segs,
            closed: true,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        dist(a, b) < 1e-3
    }

    #[test]
    fn parses_absolute_and_relative_commands() {
        let p = parse("M10 10 L20 10 l0 10 H10 v-10 Z").unwrap();
        assert_eq!(p.contours.len(), 1);
        let c = &p.contours[0];
        assert!(c.closed);
        assert_eq!(c.segs.len(), 4);
        assert!(close(c.segs[1].to, [20.0, 20.0]));
        assert!(close(c.segs[3].to, [10.0, 10.0]));
        // Implicit lineto after moveto, comma separators, exponents.
        let q = parse("m0,0 10,0 1e1 10").unwrap();
        assert!(close(q.contours[0].segs[1].to, [20.0, 10.0]));
    }

    #[test]
    fn smooth_quadratic_and_arc_segments() {
        let p = parse("M0 0 Q10 0 10 10 T10 30 C10 40 20 40 20 30 S30 20 30 30").unwrap();
        assert_eq!(p.contours[0].segs.len(), 4);
        // A half-circle arc of radius 10 ends where asked and bulges out.
        let a = parse("M0 0 A10 10 0 0 1 20 0").unwrap();
        let segs = &a.contours[0].segs;
        assert_eq!(segs.len(), 2);
        assert!(close(segs[1].to, [20.0, 0.0]));
        assert!((segs[0].to[1].abs() - 10.0).abs() < 1e-3);
    }

    #[test]
    fn rejects_bad_data() {
        assert!(parse("10 10").is_err());
        assert!(parse("M10").is_err());
        assert!(parse("M0 0 X1 1").is_err());
        assert!(parse("M0 0 A1 1 0 2 1 3 3").is_err());
    }

    #[test]
    fn round_trips_through_svg() {
        let p = rounded_rect(0.0, 0.0, 100.0, 50.0, 12.0);
        let q = parse(&p.to_svg()).unwrap();
        assert_eq!(q.contours[0].segs.len(), 8);
        assert!(q.contours[0].closed);
    }

    #[test]
    fn trim_cuts_by_length() {
        let p = parse("M0 0 L100 0 L100 100").unwrap();
        assert!((p.length() - 200.0).abs() < 1e-2);
        let half = p.trimmed(0.25);
        assert_eq!(half.contours[0].segs.len(), 1);
        assert!(close(half.contours[0].segs[0].to, [50.0, 0.0]));
        assert!(p.trimmed(0.0).contours.is_empty() || p.trimmed(0.0).is_empty());
        assert_eq!(p.trimmed(1.0), p);
        let sq = polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]);
        assert!(!sq.trimmed(0.5).contours[0].closed);
    }

    #[test]
    fn morph_ends_are_exact_and_middle_is_between() {
        let a = ellipse(0.0, 0.0, 100.0, 100.0);
        let b = rounded_rect(0.0, 0.0, 100.0, 100.0, 0.0);
        assert_eq!(morph(&a, &b, 0.0), a);
        assert_eq!(morph(&a, &b, 1.0), b);
        let m = morph(&a, &b, 0.5);
        assert_eq!(m.contours[0].segs.len(), 8);
        let [x0, y0, x1, y1] = m.bounds().unwrap();
        assert!(x0 >= -1.0 && y0 >= -1.0 && x1 <= 101.0 && y1 <= 101.0);
    }

    #[test]
    fn morph_pads_missing_contours_from_the_centre() {
        let a = ellipse(0.0, 0.0, 100.0, 100.0);
        let mut ring = ellipse(0.0, 0.0, 100.0, 100.0);
        ring.contours
            .push(ellipse(30.0, 30.0, 40.0, 40.0).contours[0].reversed());
        let (ma, mb) = match_paths(&a, &ring);
        assert_eq!(ma.contours.len(), 2);
        assert_eq!(ma.contours[1].segs.len(), mb.contours[1].segs.len());
        assert!(close(ma.contours[1].start, [50.0, 50.0]));
        // The hole keeps its own winding in the matched target.
        assert!(mb.contours[1].signed_area() * mb.contours[0].signed_area() < 0.0);
    }

    #[test]
    fn closed_contours_rotate_to_the_nearest_start() {
        let a = polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]);
        let b = polygon(&[[10.0, 10.0], [0.0, 10.0], [0.0, 0.0], [10.0, 0.0]]);
        let (_, mb) = match_paths(&a, &b);
        assert!(close(mb.contours[0].start, [0.0, 0.0]));
        // Opposite winding is reversed to match.
        let c = polygon(&[[0.0, 0.0], [0.0, 10.0], [10.0, 10.0], [10.0, 0.0]]);
        let (_, mc) = match_paths(&a, &c);
        assert!(mc.contours[0].signed_area() * a.contours[0].signed_area() > 0.0);
    }

    #[test]
    fn morph_is_deterministic() {
        let a = parse("M0 0 C10 20 30 20 40 0 Z").unwrap();
        let b = ellipse(0.0, 0.0, 40.0, 40.0);
        assert_eq!(morph(&a, &b, 0.37), morph(&a, &b, 0.37));
    }
}
