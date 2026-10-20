//! Overlay placement: free side zones that never touch the speaker's face box.

use super::plan::{FaceSeg, Side};

/// `[x, y, w, h]` in canvas pixels.
pub type Rect = [f32; 4];

/// Clear space kept between an overlay and the face box (at 1080 p; scales with height).
pub const FACE_MARGIN: f32 = 28.0;
/// Distance from the canvas edge.
pub const EDGE_INSET: f32 = 56.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Top,
    Mid,
    Bottom,
}

pub fn intersects(a: Rect, b: Rect) -> bool {
    a[0] < b[0] + b[2] && a[0] + a[2] > b[0] && a[1] < b[1] + b[3] && a[1] + a[3] > b[1]
}

/// Union of the face boxes whose segments overlap `[t0, t1]`; the central
/// third when there are none.
pub fn face_union(segs: &[FaceSeg], canvas: (f32, f32), t0: f64, t1: f64) -> Rect {
    let mut u: Option<[f32; 4]> = None; // x0 y0 x1 y1
    for s in segs.iter().filter(|s| s.end > t0 && s.start < t1) {
        let r = s.rect;
        let b = [r[0], r[1], r[0] + r[2], r[1] + r[3]];
        u = Some(match u {
            None => b,
            Some(a) => [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ],
        });
    }
    match u {
        Some(a) => [a[0], a[1], a[2] - a[0], a[3] - a[1]],
        None => [
            canvas.0 / 3.0,
            canvas.1 * 0.05,
            canvas.0 / 3.0,
            canvas.1 * 0.65,
        ],
    }
}

/// Horizontal free zone `(x0, x1)` on `side` of `face`; `None` if narrower than 200 px.
pub fn zone(canvas: (f32, f32), face: Rect, side: Side) -> Option<(f32, f32)> {
    let u = canvas.1 / 1080.0;
    let (inset, margin) = (EDGE_INSET * u, FACE_MARGIN * u);
    let (x0, x1) = match side {
        Side::Left => (inset, face[0] - margin),
        Side::Right => (face[0] + face[2] + margin, canvas.0 - inset),
    };
    (x1 - x0 >= 200.0 * u).then_some((x0, x1))
}

/// The rect for an overlay of `size` in `slot` on `side`, aligned to the
/// zone's outer edge. `None` when it does not fit beside the face.
pub fn place(
    canvas: (f32, f32),
    face: Rect,
    side: Side,
    slot: Slot,
    size: (f32, f32),
) -> Option<Rect> {
    let (x0, x1) = zone(canvas, face, side)?;
    if size.0 > x1 - x0 + 0.5 {
        return None;
    }
    let x = match side {
        Side::Left => x0,
        Side::Right => x1 - size.0,
    };
    let y = match slot {
        Slot::Top => canvas.1 * 0.14,
        Slot::Mid => canvas.1 * 0.36 - size.1 / 2.0,
        Slot::Bottom => canvas.1 * 0.80 - size.1,
    };
    let r = [x, y, size.0, size.1];
    (!intersects(r, face)).then_some(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: (f32, f32) = (1920.0, 1080.0);
    const FACE: Rect = [700.0, 60.0, 520.0, 640.0];

    #[test]
    fn placed_overlays_never_touch_the_face() {
        for side in [Side::Left, Side::Right] {
            for slot in [Slot::Top, Slot::Mid, Slot::Bottom] {
                let r = place(C, FACE, side, slot, (400.0, 200.0)).unwrap();
                assert!(!intersects(r, FACE), "{side:?} {slot:?}");
            }
        }
    }

    #[test]
    fn a_wide_face_leaves_no_zone() {
        let wide = [100.0, 0.0, 1700.0, 700.0];
        assert!(place(C, wide, Side::Left, Slot::Mid, (300.0, 100.0)).is_none());
        assert!(place(C, FACE, Side::Left, Slot::Mid, (900.0, 100.0)).is_none());
    }

    #[test]
    fn union_covers_overlapping_segments_and_falls_back() {
        let segs = vec![
            FaceSeg {
                start: 0.0,
                end: 5.0,
                rect: [700.0, 100.0, 400.0, 500.0],
                detected: true,
            },
            FaceSeg {
                start: 5.0,
                end: 10.0,
                rect: [800.0, 80.0, 500.0, 500.0],
                detected: true,
            },
        ];
        let u = face_union(&segs, C, 4.0, 6.0);
        assert_eq!(u, [700.0, 80.0, 600.0, 520.0]);
        assert_eq!(face_union(&[], C, 0.0, 1.0)[0], 640.0);
    }
}
