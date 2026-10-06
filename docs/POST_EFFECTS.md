# Post effects and layer effects (CPU renderer, 0.14-0.16)

The Timeline decides *what* happens (`ResolvedFrame.post`, `ResolvedLayer.glyphs / points /
blur / projective`); the CPU renderer only draws it. Nothing here makes semantic decisions.
Code: `motion-render/src/postfx.rs` (post effects), `blur.rs` (layer blur), `warp.rs`
(homography warp), `cpu.rs` (layer integration), `text.rs` (per-glyph outlines).

## Post effects

`postfx::apply_post(&mut Pixmap, &[ResolvedPost])` runs after the whole frame is composited,
in list order. `CpuRenderer::render` calls it; an empty list is a no-op, so frames without
post are byte-identical to before.

Common rules:

- **Strength.** `strength` is clamped to 1. Strength 0, negative or NaN skips the effect
  (the pixels are bit-identical). Parameters give the look at strength 1; each effect says
  what strength scales.
- **Deterministic.** Pure functions of the pixels, parameters, strength and, for noise,
  `seed` plus the frame `time` (via `motion_core::noise::hash`). Rows run in parallel
  (rayon) but each output pixel depends only on a snapshot of the input, so the thread
  count never changes the result.
- **Alpha.** The frame is normally opaque. Colour is always kept `<= alpha`, so the pixmap
  stays valid premultiplied RGBA. Bad parameters (non-finite, out of range) skip the effect.

| effect | parameters | what it does | strength scales |
|---|---|---|---|
| `bloom` | `threshold` 0..1 luma, `radius` px | Bright pass (`w = clamp((luma - threshold) / (1 - threshold), 0, 1)` times the colour, luma = Rec.709 of the premultiplied RGB), dual-filter pyramid blur, added back and clamped. | glow intensity |
| `chromatic_aberration` | `shift` px, `angle_deg` | Red is **sampled** at `p + d` and blue at `p - d`, `d = shift * strength` along `angle_deg` (bilinear, edge clamped); so red *content* appears displaced by `-d` and blue by `+d`. Negative `shift` swaps them. Green and alpha untouched. | the offset |
| `glitch` | `bands`, `max_shift` px, `seed` | Time bucket `b = floor(time * 12)`. For band `i` (1..=`bands`, cap 256): `y`, `height` (1%..8% of the frame) and the signed whole-pixel offset (25%..100% of `max_shift * strength`) come from `hash(seed ^ b, 4i + k)`. Each band shifts horizontally with wrap and gets an RGB split (red and blue sampled `max(1, 12% of the offset)` px either side). | maximum displacement |
| `rays` | `center` canvas px, `length` 0..1 | 24 samples along the segment from each pixel toward `center` over `length * strength` of that distance (nearest texel), averaged, then screen-blended over the frame with weight `strength`. Not luma-gated: it lifts the whole frame, so use low strength or dark frames. | sample reach and blend weight |
| `directional_blur` | `angle_deg`, `length` px | Centred box blur along `angle_deg` over `length * strength` px (>= 8 bilinear samples, at most 1 px apart along the dominant axis, trapezoid end weights, edge clamped). Lengths under about 1 px are a no-op. | blur length |
| `grain` | `amount` 0..1, `seed` | Per-pixel uniform noise from `hash(seed ^ floor(time * 24), y * width + x)`, the same value on R, G and B, shifting luma by up to `amount * strength * 0.25` of full scale (scaled by alpha). | amplitude |
| `vignette` | `amount` 0..1 | Multiplies RGB by `1 - amount * strength * smoothstep(0.45, 1.0, r)`, `r` = distance from the frame centre normalised so the corners are 1. | darkening |

### Bloom radius

`radius` is calibrated so the glow reaches about `radius` px: against a large bright block
the glow falls below 1% of full scale at roughly 0.9 x `radius` (radius 32 -> 27 px, 64 ->
52 px, 128 -> 98 px; below about 8 px the minimum glow is 3-4 px wide). The pyramid has
`round(log2(radius)) - 1` levels (1..=7, fewer on tiny canvases) with equal energy per
octave, giving a tight core plus a wide halo. At strength 1 the glow adds at most the
blurred bright pass at gain 1, so thin text glows softly and big bright areas blow out.

### Cost (1080 x 1920, release, Apple M4, 10 threads / 1 thread)

Whole effect on a clone of a busy opaque frame, best of several runs on a loaded machine:

| effect | 10 threads | 1 thread |
|---|---|---|
| bloom (radius 24 or 100) | 11 ms | 50 ms |
| chromatic aberration | 2 ms | 9 ms |
| glitch (12 bands) | 2 ms | 2 ms |
| rays (length 0.5) | 11 ms | 58 ms |
| directional blur, 0 deg, 60 px | 8 ms | 39 ms |
| directional blur, 30 deg, 60 px | 21 ms | 118 ms |
| directional blur, 30 deg, 200 px | 72 ms | 391 ms |
| grain | 2 ms | 5 ms |
| vignette | 2 ms | 7 ms |

Directional blur costs `O(length)` per pixel (taps are row accumulations: two per sample on
the dominant axis). `export::render_frames` already renders frames in parallel, so
single-thread cost is what limits throughput. Reproduce with
`cargo test --release -p motion-render --test postfx_timing -- --ignored --nocapture`.

## GPU backend (wgpu + WGSL, `--features gpu`)

An optional backend that runs the same seven effects as WGSL compute passes
(`motion_render::gpu::GpuPost`, code in `motion-render/src/gpu/`). The CPU code above stays the
**reference**: it defines the look, the goldens and every default build. Without the feature
nothing changes and nothing new compiles (wgpu, pollster and bytemuck are optional
dependencies).

### Build and run

```
cargo build --release -p motion-cli --features gpu
motion-engine render scene.motion.json --gpu          # also: motion-engine reel ... --gpu
cargo test -p motion-render --features gpu            # includes tests/gpu_parity.rs
```

`--gpu` is accepted by every build. It switches the post stage of `render` (and `reel`, which
forwards it) to the GPU; the default is the CPU path, byte-identical to before. If the
build has no `gpu` feature, no adapter is found, or the GPU fails mid-render (the failing
frame and all later ones are redone on the CPU), the command prints one line on stdout and
carries on with the CPU effects:

```
gpu: unavailable (<reason>), using CPU post effects
```

Library use: `GpuPost::new() -> Result<GpuPost, GpuError>` opens the high-performance headless
adapter (`WGPU_BACKEND` can force one) and `gpu.apply(&mut pixmap, &frame.post)` is the twin of
`postfx::apply_post`. `CpuRenderer::with_gpu_post(gpu)` routes a renderer's post stage through
it and `export::render_frames_with(.., RenderOptions { gpu_post: true })` does the same for
whole renders. On error `apply` leaves the pixmap untouched.

### How it works

- **One upload, one readback per frame.** The finished premultiplied RGBA8 pixmap is uploaded
  once, every effect in the list runs in order as compute passes inside a single command
  buffer (ping-pong between two frame buffers), and the result is read back once. An empty
  list, or a list where every effect is skipped (strength 0 or NaN/negative, invalid
  parameters), does not touch the GPU or the pixmap.
- **Storage buffers, not textures.** Frames are packed `u32` RGBA8 in storage buffers, so the
  8-bit quantisation between effects is exactly the CPU's (each effect re-rounds to u8), there
  is no 256-byte row padding on readback, and bilinear taps are evaluated by hand in f32 (the
  CPU's weights, not the texture unit's 8-bit weights). Bloom's pyramid is f32 `vec4` per
  texel. Uniforms for all passes of a frame are written once and selected with dynamic offsets.
- **Same arithmetic.** Parameter handling and derived values (chromatic offsets, directional
  blur tap list, bloom pyramid depth and gains, ray step, vignette centre) are computed on the
  host with the CPU's f32 expressions (`gpu/plan.rs`); the shaders port the per-pixel formulas
  one to one. The lowbias32 hash is ported with identical u32 arithmetic, so glitch band
  positions, heights, offsets and signs and every grain value are the CPU's. Glitch bands apply
  sequentially per row exactly like the CPU (a later band tears what an earlier band left).
- **Lazy, reused resources.** Pipelines compile on first use; frame, readback and pyramid
  buffers are created per canvas size and reused until the size changes.
- **Fast-math caveat.** Metal compiles WGSL with fast-math, which may fuse or reassociate float
  ops. `rays` positions are accumulated like the CPU (`q += v`) and a one-ulp change flips the
  nearest texel at exact integer coordinates, so that shader passes the step and the running
  position through an identity the compiler cannot see through (`opaque`, an XOR with a runtime
  zero). Other effects have no such cliffs: fused ops change a value by at most a rounding step.

### Parity (Apple M4 / Metal; `tests/gpu_parity.rs`)

Every effect is compared with `apply_post` on 256 x 256 gradients, hard-edged shapes, thin
antialiased strokes, semi-transparent premultiplied content and hash noise, over 33 parameter
sets (strengths, angles, seeds, time buckets, off-canvas centres, offsets past the canvas).
Pass criteria: max per-channel difference <= 3 and mean <= 0.5. Measured worst cases:

| effect | max diff (of 255) | mean diff |
|---|---|---|
| bloom (r 6 / 24 / 100 / 300) | 1 | < 0.0001 |
| chromatic aberration | 1 | < 0.0001 |
| glitch (3 to 40 bands, 5 seed/time sets) | 0 (identical) | 0 |
| rays | 0 (identical) | 0 |
| directional blur | 1 | 0.0005 |
| grain | 0 (identical) | 0 |
| vignette | 1 | < 0.0001 |
| chain of all 7 in list order (and reversed) | 1 | < 0.0001 |

Glitch and grain are additionally checked over 24 seeds on a ramp image (every row/pixel
distinct), so a different random choice would show as a large error. Also tested: strength 0 /
negative / NaN and an empty list are bit-identical no-ops, strength clamps to 1, odd and tiny
canvases (1 x 1, 5 x 300, ...) and resizing between frames, `CpuRenderer::with_gpu_post`
against the plain renderer on a text frame (frames 0, 7, 21; no-post frames byte-identical),
and `render_frames_with` over 30 PNG frames. An end-to-end render of
`golden/hero_object.motion.json` with all seven effects across both scenes (162 frames at
1080 x 1920) differed from the CPU render by at most 2 levels in any channel (mean 6e-6).
Other GPUs may round differently by a level or so; tests allow 3. Tests skip (print and
return) when no adapter is available.

### Timings (1080 x 1920, release, Apple M4 / Metal)

Whole effect on a clone of the busy opaque frame, median of 7, **including** upload, readback
and the frame clone (so these are what a frame actually pays). Upload + readback
alone costs about 2.2 ms (about 0.8 ms of it queuing the upload, 0.2 ms copying out); that is
the floor for any GPU effect.

| effect | CPU 10 threads | CPU 1 thread | GPU |
|---|---|---|---|
| bloom (r 24 / 100) | 11 / 10 ms | 49 ms | 3.6 / 3.7 ms |
| chromatic aberration | 2.1 ms | 9.4 ms | 2.4 ms |
| glitch (12 bands) | 2.1 ms | 1.9 ms | 3.8 ms |
| rays (length 0.5) | 10 ms | 56 ms | 3.2 ms |
| directional blur 0 deg, 60 px | 7 ms | 38 ms | 5.1 ms |
| directional blur 30 deg, 60 px | 19 ms | 114 ms | 10.9 ms |
| directional blur 30 deg, 200 px | 64 ms | 376 ms | 32 ms |
| grain | 1.1 ms | 4.8 ms | 2.0 ms |
| vignette | 1.5 ms | 6.7 ms | 2.0 ms |
| **all ten above in one frame** | 125 ms | 707 ms | 49 ms |

Heavy effects (bloom, rays, directional blur) are 1.4 to 3 times faster than the 10-thread CPU
and 6 to 15 times faster than one thread; cheap per-pixel effects are transfer-bound and
about break-even (glitch is slower on the GPU: one frame round trip for a 2 ms CPU effect).
The GPU pays off when a frame stacks effects, since the round trip is paid once. An
end-to-end render of the 162-frame, all-effects scene above took 10.9 s with `--gpu` against
19.1 s on the CPU. The shared device serialises GPU work, but CPU compositing of other frames
continues while one frame waits on it. Reproduce with
`cargo test --release -p motion-render --features gpu --test gpu_parity -- --ignored --nocapture`
(add `RAYON_NUM_THREADS=1` for single-thread CPU numbers).

### Limits

- The canvas must fit the adapter's storage-buffer limits (`GpuError::TooLarge`, then the CPU
  path); 4K and 8K are fine on desktop GPUs.
- `rays` with a NaN length is skipped (per the "invalid parameters skip" rule); the CPU code
  would blend texel (0, 0), which no timeline output produces.
- Glitch offsets are bounded to 1e9 px in the shader (i32 arithmetic); beyond that the CPU's
  result is meaningless anyway.

## Layer effects (ResolvedLayer fields)

### `glyphs: Option<Vec<GlyphPose>>` (text layers)

- `None`, an empty list, or all poses equal to `GlyphPose::REST` draw the layer exactly as
  before (same combined path, pixel-identical).
- Otherwise glyph `k` is the `k`-th non-whitespace character of the displayed text (after
  `uppercase`, or the Count override) in reading order, across lines. A ligature or cluster
  glyph takes the pose of its first character; later characters keep their own indices.
  Missing poses are `REST`.
- Each posed glyph is drawn on its own: `translate(dx, dy)`, rotation (degrees, positive
  clockwise on screen, like layer rotation) and uniform scale about the **centre of the
  glyph's ink bounds**, opacity = layer opacity x `pose.opacity`. Glyphs still at `REST`
  are drawn together as one path first. Non-finite poses, scale 0 and opacity 0 skip the
  glyph.
- Outlines come from `TextEngine::glyph_outlines` (same layout as `outline`; the layout
  width is the static width for plain text and the frame's `width` for Count overrides),
  cached per `(layer id, text)`.

### `points: Option<Vec<[f32; 2]>>` (polylines)

Replace the layer's own points (box space); `trim`, stroke, fill and closing behave as for
authored points.

### `blur: Option<f32>`

The radius is the circle-of-confusion radius; the Gaussian sigma is `radius / 2` (three box
passes, `blur::gaussian_blur`). Radii below 0.6 px (sigma under 0.3), NaN and negatives are
ignored.

- Leaf layers draw into a transparent canvas-sized offscreen **without** opacity or clip, are
  blurred, then composited with the layer's opacity and clip mask (so the clip edge stays crisp).
- Groups blur their composited children as one image, then apply opacity and clip.
- The blur only touches the bounding box of the non-transparent pixels plus the kernel
  support (pixels outside the pixmap count as transparent), so cost follows the layer, not the
  canvas. Full canvas: about 15 ms (10 threads) / 28 ms (1 thread).
- With `projective`, the layer's opacity and clip stay inside the warp (the clip lives in box
  space) and the blur is applied to the warped result.

### `projective: Option<[f32; 9]>` (leaf layers)

Row-major homography from the layer's box space to canvas px; it replaces `transform` for
drawing and `content_transform` is the box-relative content offset only.

1. The layer renders in box space (opacity, clip, tint, content offset exactly as a plain
   layer) into an offscreen of the box size plus a 16 px transparent margin (boxes over
   4096 px are rendered scaled down).
2. The offscreen is warped onto the canvas through the inverse homography with bilinear
   sampling in premultiplied space, over the projected quad's bounding box. Texels outside the
   offscreen are transparent, so quad edges are antialiased. Strongly minified quads are
   supersampled up to 3 x 3.
3. A homography is defined up to scale: the matrix is oriented so the box centre has
   positive `w`; points whose forward `w` is not positive (behind the camera) are not drawn
   (the horizon is tested per pixel), singular or non-finite matrices draw nothing.

An identity homography reproduces the affine draw (max channel difference <= 2; exact for
integer translations). **Groups ignore `projective`:** their children carry their own resolved
transforms, so a tilted group must put the homography on each child. Cost of a full-canvas
quad: about 15 ms (10 threads) / 70 ms (1 thread).
