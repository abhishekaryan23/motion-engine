# MotionOp 2.0 (Timeline semantics)

What the Timeline (`evaluate_frame`) does with the 0.14 contract fields. Types live in
`crates/motion-core/src/scene.rs`; pixels are the renderer's job. Everything is a pure
function of `(project, frame)`: `t = frame / fps`, no accumulated time, all noise seeded
(`motion_core::noise`). Scenes and layers that do not use these fields resolve exactly as
before (finished or inactive ops add nothing to the output).

Units: seconds are scene-local unless stated; px are canvas px (layer space for layer ops);
degrees are clockwise-positive like `rotate`.

## Spring (`Motion.spring`)

`"spring": {"stiffness": 170, "damping": 26, "mass": 1}` replaces `easing` for that motion:
progress is `easing::spring_progress(elapsed, duration, spec)` (damped oscillator, exactly 0 at
the start and exactly 1 at `start + duration`; under-damped springs overshoot in between).
Applies wherever a motion's progress is computed (layer motions and motions on shared
elements). `count` motions never overshoot: with a spring they use OutQuint progress.

```json
{"target":"card","start":0.3,"duration":0.9,"op":"scale","from":0.6,"to":1.0,
 "spring":{"stiffness":300,"damping":14}}
```

## Layer ops

- `shake` (any layer): additive to the offset and rotation channels (coexists with `move` /
  `rotate`), only inside `[start, start + duration]`. With `e = t - start`:
  `dx = amplitude[0]*smooth(seed, frequency*e)`, `dy = amplitude[1]*smooth(seed ^ 0x9E3779B9, frequency*e)`,
  `drot = rotation*smooth(seed ^ 0x85EBCA6B, frequency*e)`, all times `exp(-decay*e)`.
  Several shakes on a layer sum. Bound layers shake inside the parent's box space like `move`.
  ```json
  {"target":"hit","start":1.2,"duration":0.6,"op":"shake","amplitude":[14,6],"rotation":2,
   "frequency":9,"decay":4,"seed":3}
  ```
- `glyph_cascade` (text layers): `n` = non-whitespace chars of the text shown this frame (the
  `count` text if any, else the layer text). Glyph `k` gets a start rank `r` from `order`:
  `forward` k, `backward` n-1-k, `center` by `|k-(n-1)/2|` (ties left first), `random` by
  `noise::hash(seed, k)`. Glyph rank `r` starts at `start + r*stagger` and runs
  `max(duration - (n-1)*stagger, 1e-3)` s with the motion's easing/spring; its pose is
  `lerp(from, REST, p)` per field (before its start `from`, after its end `REST`).
  `ResolvedLayer.glyphs = Some(poses)` (reading order, one per non-whitespace char) only while
  a pose differs from `GlyphPose::REST`; `None` otherwise.
  ```json
  {"target":"title","start":0.2,"duration":1.0,"op":"glyph_cascade","stagger":0.04,
   "order":"center","from":{"dy":40,"scale":0.6,"rotation":12,"opacity":0}}
  ```
- `path_morph` (polyline layers): the layer's own points and `to` are resampled to
  `N = max(len_a, len_b)` points by arc length (closed polylines resample the closed loop),
  then interpolated with the eased progress. `ResolvedLayer.points = Some(..)` from the motion
  start on (holding `to` after the end), `None` before it. Chained morphs start from the previous
  morph's `to`.
  ```json
  {"target":"line","start":0.5,"duration":0.8,"op":"path_morph","to":[[0,80],[60,10],[120,80]]}
  ```
- `echo`: while `t` is in `[start, start + duration]`, `count` ghost copies of the layer (and
  its children) are emitted, ghost `k` (1..=count) resolved at `t - k*spacing` with the current
  camera (the trail is world-locked), opacity `x decay^k`. Ghosts are drawn immediately behind
  the layer (same `z_index`, inserted before it, farthest first) and never echo themselves.
  A ghost whose time is before the scene start is skipped.
  ```json
  {"target":"ball","start":1.0,"duration":1.2,"op":"echo","count":4,"spacing":0.05,"decay":0.6}
  ```
- `pulse`: multiplies the layer's scale (both axes, on top of the `scale` channel) by
  `1 + gain * envelope(project time)` inside `[start, start + duration]`. `envelope` names a
  `MotionProject.envelopes` entry; an unknown id is a no-op (validation rejects it).
  ```json
  {"target":"logo","start":0,"duration":8,"op":"pulse","envelope":"beat","gain":0.12}
  ```
  with `"envelopes":[{"id":"beat","fps":30,"values":[0,0.2,1,0.6,0.1]}]` (sample `i` covers
  `[i/fps, (i+1)/fps)` of project time; past the end reads 0).

Not applied to shared elements: `shake`, `glyph_cascade`, `path_morph`, `echo` and `pulse` read
the scene motion index of scene layers; shared elements keep their move / scale / rotate /
fade / count / tint channels (now spring-aware).

## Camera ops

Applied to every layer equally, after the depth parallax (`view = translate(shake) *
rotate(roll) about the pivot`), also to free shared elements (their rotation gains the roll).
Absent = byte-identical to the plain camera.

- `roll` `{from, to}` degrees about the pivot, same easing/hold rules as `push`/`track`.
- `shake` `{trauma, frequency, decay, seed}`: only inside its window, with `e = t - start`,
  `g = trauma^2 * exp(-decay*e)`: `dx = g * 0.025 * canvas_width * smooth(seed, frequency*e)` px,
  `dy` the same with `seed ^ 0x9E3779B9`, and `g * 3 * smooth(seed ^ 0x85EBCA6B, frequency*e)`
  degrees of roll (same salts as layer shake). Shakes sum; their roll adds to the roll channel.

```json
"camera": {"motions": [
  {"start":0,"duration":3,"easing":"in_out_cubic","op":"roll","from":0,"to":6},
  {"start":1.0,"duration":0.7,"op":"shake","trauma":0.6,"frequency":10,"decay":3,"seed":7}
]}
```

## Perspective camera (`Camera.perspective`)

A scene camera with `perspective` is a true pinhole instead of the 2D multi-plane camera.
Without it nothing below applies and scenes resolve byte-identically. Implementation:
`timeline/perspective.rs`; all math in f64, a pure function of `(project, frame)`.

Units: px are canvas px; `z` is depth in px (0 = the focus plane at rest, positive = farther
from the camera, negative = nearer); degrees follow the same sign rules below.

```json
"camera": {
  "pivot": [540, 960],
  "perspective": {"fov_deg": 40, "focus_z": 0, "aperture": 3},
  "motions": [
    {"start":0,"duration":3,"easing":"in_out_cubic","op":"dolly","from":0,"to":600},
    {"start":0,"duration":3,"easing":"in_out_cubic","op":"orbit","from":[-8,2],"to":[10,-3]},
    {"start":0,"duration":1.2,"easing":"out_cubic","op":"focus","from":-1300,"to":-600}
  ]
}
```
```json
{"id":"card","type":"image","asset":"photo","x":540,"y":900,"width":600,"height":800,
 "anchor_x":0.5,"anchor_y":0.5,"z":-200,"tilt":[0,25]}
```

Pipeline for a world point `(x, y, z)` (pivot `c` = `camera.pivot` or the canvas centre):

1. `tilt` `[x°, y°]` (top-level layers): the layer's four box corners in rest canvas space (its
   own affine transform; the 2D `depth` factor is ignored) are rotated about the layer's
   anchor point, about the screen-aligned x and y axes (y first, then x), at depth `z`
   (default 0). Positive y tilt brings the right edge nearer, positive x tilt the top edge.
2. `track` pans the camera: subtract the pan, then `x - c.x, y - c.y`.
3. `orbit` `[yaw°, pitch°]` (eased like push/track, holding before/after): rotate about the
   pivot, yaw about y then pitch about x. Positive yaw brings the right side of the scene
   nearer (the camera swings right), positive pitch the top.
4. Projection with focal length `f = (canvas_height / 2) / tan(fov / 2)`. The camera starts at
   `z = -f` looking along +z, so `z = 0` maps 1:1 at rest. `dolly` `d` (px, eased, holding)
   moves it to `cam_z = -f + d`. With `dz = z' - cam_z`: `s = push * f / dz`,
   `p' = c + s * (x', y')`. `push` multiplies the projected scale.
5. `roll` and `shake` apply afterwards in screen space (`view`, as for the 2D camera).

Rules:

- The 2D `depth` factor is ignored. `z` and `tilt` are read on top-level scene layers only;
  on group children and shared elements they are ignored (the group's plane carries its
  children).
- Behind the camera: a layer with any box corner at `dz <= 1` is not drawn (a card swung
  across the camera plane is skipped whole). Edge-on cards (projected area < 0.001 px^2) are
  skipped too.
- Depth of field: `z_view = dz - f` for the box centre (distance from the rest focus plane);
  `ResolvedLayer.blur = min(24, aperture * |z_view - focus_z| / 100)` px, set on the top-level
  layer (also groups, where it blurs the composited children) only when `>= 0.5`.
- Rack focus (0.17): the camera op `focus` `{from, to}` (px, eased, holding before/after like
  `dolly`) replaces `perspective.focus_z` while it is active. A plane is sharp when
  `z - dolly == focus`, so a move that dollies to `d` and racks to `-d` keeps the `z = 0` plane
  sharp. `focus` needs `perspective` (validation) and has its own non-overlapping span list.
- Echo ghosts project like their layer at the ghost time with the current camera.
- Free shared-element keys follow the `z = 0` plane (position and scale through the same
  camera; no tilt/orbit foreshortening of the element itself); bound keys follow the layer.

What the resolved frame carries (renderer contract):

- No tilt and no orbit: the projection is a uniform scale about the pivot plus a translation
  and is folded into `transform` / `content_transform` of the layer and of every descendant;
  `projective` is `None`.
- Tilt or orbit: every LEAF layer (non-group) of the tree gets `projective`, a row-major
  `[f32; 9]` homography (`h33 = 1`) from the leaf's LOCAL BOX space to canvas px, fitted
  (4-point DLT) to the leaf's own four box corners pushed through its top-level ancestor's
  plane, tilt, orbit and projection. It replaces `transform` for drawing; the leaf's
  `content_transform` is then only the box-relative content offset (e.g. `clip_reveal`).
  Groups are never given a `projective`. Their `transform` (and their descendants') hold the
  FLAT projection (no tilt/orbit foreshortening), a consistent non-warped fallback.
- `blur` is on the top-level layer (see above).

### (0.18) Revolve, billboard, depth sort, camera velocity

All four are pure functions of `(project, frame)` (f64 math, narrowed to f32 at the output);
absent they change nothing (goldens stay byte-identical).

**`revolve` (layer op)** `{radius, at: [lon, lat], from: [yaw, pitch], to: [yaw, pitch]}`: the layer
rides a sphere of `radius` px centred on its own position; `at` is the point of the sphere it sits
on (degrees; `[0, 0]` faces the camera, the nearest point; positive lat is up) and the sphere
turns by `[yaw, pitch]`. With `dir = (sin lon cos lat, -sin lat, -cos lon cos lat)`,
`Ry(a)(x, y, z) = (x cos a - z sin a, y, x sin a + z cos a)` and
`Rx(b)(x, y, z) = (x, y cos b + z sin b, -y sin b + z cos b)`:

```
[yaw, pitch] = lerp(from, to, p)            p = eased (or spring) progress
offset       = radius * Rx(pitch) * Ry(yaw) * dir
```

`[yaw, pitch] = [-lon, -lat]` brings the point `at` to the front (`offset = (0, 0, -radius)`).

- Channel `revolve` holds like every other layer channel: the most recently started revolve wins
  and holds its end; before the first one starts it holds its `from`. Overlaps on the channel are
  a validation error.
- `offset.x/y` add to the offset channel (after `move`, before `shake`): unbound, layout-bound
  and top-level layers alike, in every scene (with or without a perspective camera).
- `offset.z` adds to the plane depth (`z = layer.z + offset.z`; negative = nearer) of TOP-LEVEL
  layers of perspective scenes only; it is ignored for group children, in scenes without
  `camera.perspective`, and for shared elements (which do not take revolve).
- Layers do not turn with the sphere; only their anchor travels.

**`perspective.billboard`** (default false): a top-level layer without `tilt` is a sprite. The
orbit moves its box centre `(c.x, c.y, z)` (the box's middle in rest canvas space) through 3D (pan, orbit, projection, roll/shake: the
same camera as above, giving the canvas point `P` and view distance `dz`) but the plane itself
stays flat and uniformly scaled: a rest point `q` lands at `P + s * (q - c)` with
`s = push * f / dz` (the roll/shake rotation applies on top of it). `projective` is `None`, the
aspect ratio never shears. The blur uses this `dz` (`z_view = dz - f`), and a centre with
`dz <= 1` hides the layer. Explicit `tilt` still turns the plane; (0.19) with billboard and an
orbit it turns about the box centre with a local perspective (see `tilt` below) and never shears
with the orbit (0.18 warped it exactly as without billboard). Without an orbit billboard is the
plain flat projection (identical output).

**`perspective.depth_sort`** (default false): the scene's top-level layers with the same `z_index`
draw far to near by their plane's view distance `dz` (the value the blur is computed from: `z - cam_z`
for flat planes, the box centre's for billboards and warped planes), so layers
that travel in depth pass in front of and behind each other. The global draw order key is
`(z_index, scene index, depth rank, layer index)`; `depth rank = -round(dz * 1000)` in depth-sorted
scenes and 0 everywhere else (other scenes, shared elements), so ties keep layer order and nothing
else moves. `echo` ghosts take their layer's rank and stay right behind it. Layers that are not
drawn (behind the camera) are simply absent.

**`CameraMotion.velocity`** `[v0, v1]` (optional, replaces `easing`): the progress is the cubic
Hermite curve from 0 to 1 with normalised end slopes `v0`, `v1` (`d progress / d(t / duration)`),

```
h(u) = (u³ - 2u² + u) * v0 + (3u² - 2u³) + (u³ - u²) * v1        u = clamp((t - start) / duration)
```

`[1, 1]` is linear, `[0, 0]` is smoothstep `3u² - 2u³`; handing a move's end slope to the next
move's start slope chains camera moves without stopping. Slopes must be finite and in `0..=3`
(validation error otherwise); within that range progress stays monotonic.

### (0.19) Tilt

**`tilt` (layer op)** `{from: [pitch, yaw], to: [pitch, yaw]}` (degrees, each finite and within
±89°; validation error otherwise): the same angles as the static `Layer.tilt` (about the layer's
x and y axes, positive yaw brings the right side nearer, positive pitch the top), animated. Top-level
layers of perspective scenes only (ignored elsewhere, and for shared elements). It is a channel
like the others: the most recently started tilt wins, holds its end, and before the first one
starts holds its `from`; while a tilt motion is active it replaces the layer's static `tilt`.

```
[pitch, yaw] = lerp(from, to, p)            p = eased (or spring) progress
```

- Without billboard the plane turns about its anchor exactly like a static `tilt` (the warped
  projective path; the orbit warps it too).
- With `billboard` and an orbit (the cinematic look) the plane turns about its box centre `c`.
  The centre is placed like a sprite (point `P`, view distance `dz`, `focal = zoom * f`); a rest
  corner `q` is rotated about `c` by `(pitch, yaw)` into `(x, y, z)` and lands at
  `P + focal / (dz + z) * (x, y)`, then roll/shake. The orbit moves the centre but never shears
  the card, and a tilt of `[0, 0]` is the sprite's flat affine exactly, so a turn that settles to
  rest has no pop.
- The compiler (`cinematic3d`) pairs it with a `move` (id `arrival`) so a picture slides and turns
  in together; the FX director re-targets both from the stage child to its `.depth` wrapper (the
  plane the camera projects), like `revolve`.

## Post effects (`Scene.post`)

For every active scene (in order) and each `PostEffect` (in list order):
`strength = lerp(from, to, eased progress over [start, start+duration])` inside the window and
zero outside it (flashes); `duration <= 0` = persistent `to` from `start` to the scene end
(grain, vignette); clamped to 0..=1. Included in
`ResolvedFrame.post` when `strength > 1e-4`, with `time` = project time (per-frame noise seed).
`kind` points at the scene's `PostKind` (parameters give the look at strength 1; the renderer
applies them).

```json
"post": [{"effect":"chromatic_aberration","shift":6,"angle_deg":0,
          "start":1.0,"duration":0.3,"easing":"out_cubic","from":1,"to":0},
         {"effect":"vignette","amount":0.45,"start":0,"duration":0,"from":1,"to":1}]
```

## Tests

`crates/motion-core/tests/motion_ops_2.rs` (one test per item above, determinism, spring
landing, glyph order, echo ordering/opacity, pulse sampling, post windows, "absent or inert fields
change nothing") plus unit tests in `timeline.rs` (glyph ranks, arc-length resampling, progress
shaping). `crates/motion-core/tests/perspective.rs` covers the perspective camera against an
independent f64 reference (rest 1:1, z shrink, dolly, tilt homography corners, orbit parallax,
DoF, behind-camera, groups, echo, shared, shake/roll, determinism).
`crates/motion-core/tests/revolve_3d.rs` covers the 0.18 additions (revolve formula, hold,
offset channel, z for top-level layers only, billboard, depth sort, camera velocity,
validation).
`crates/motion-core/tests/tilt_motion.rs` covers the 0.19 `tilt` op (validation, turn then hold,
pitch, billboard centre and orbit independence against a hand-computed local perspective, no pop
at rest, replacing a static tilt); `crates/motion-core/tests/cinematic_arrivals.rs` covers the
cinematic arrivals (no music pulse, a different side per picture, slide and turn in sync and at
rest, the supporting picture inside the frame and labelled).
