# Footage & loops (0.11)

Video never enters the deterministic core. Clips and loops are extracted
outside the engine into **sprite sequences** (frame directories); the Timeline
picks a frame, the renderer draws it like a still. Same inputs → same frames.

## Sprite sequences (`scene.rs`)
```json
{ "id": "asset.loop.rocket", "type": "sprite_sequence",
  "path": "library/clay_props_3d/loops/rocket",
  "sprite": { "frame_count": 47, "fps": 12, "mode": "loop", "pattern": "frame_%04d.png" } }
```
- Image layers reference a sprite like a still; optional `playback {start,
  in_frame, out_frame}` (scene-local seconds / 0-based frames).
- `sprite_frame(spec, playback, t_local) = floor((t − start) × fps)` from
  `in_frame`; `loop` wraps within [in, out], `once` holds `out`; before `start`
  → `in_frame`. Pure (`ResolvedLayer.sprite_frame`).
- Renderer: frames decoded lazily into a shared cache keyed (asset, frame);
  treatments apply per frame; a missing frame is an error with its path.
  Validation checks the spec, playback ranges and (with a base dir) that the
  first and last frames exist.

## Screen inserts
`LayerKind::Image.insert = {asset, screen_box:[x,y,w,h], fit, playback}` — an
image or sprite drawn inside the host image's screen hole (fractions of the
host's layer box), clipped to it, under the host's transform and opacity; the
host is drawn on top (its keyed screen is transparent). Catalog `screen_box`
values are fractions of the delivered PNG and are mapped through the host's
Contain fit.

## Living library assets (`compiler/loops.rs`, `--art`)
Families may ship `<family>/loops/catalog.json`:
`{loops:[{id, host, role: hero_loop|screen_insert, path, frame_count, fps,
pattern, width, height, ssim_first_last, source}]}`.
With art direction on, every delivered library image whose family has a
`hero_loop` for it becomes that sprite sequence, and every placed host with a
`screen_box` plays the family's `screen_insert` loop. Without `--art`: nothing
changes.

| family | loop | role | seam SSIM |
|---|---|---|---|
| clay_props_3d | piggy_bank, clock, rocket | hero_loop | 0.973 · 0.950 · 0.927 |
| classical_greyscale | hourglass | hero_loop | 0.948 |
| halftone_retro_objects | tv_toaster_ad | screen_insert (retro_tv) | 0.941 |

## Making loops
OpenArt image→video, cheapest acceptable model (PixVerse V6, 540p, 4 s, 50
credits) with **start frame = end frame** for seamless loops; prompts lock the
camera and keep a flat magenta ground for keyed objects. 210 credits for five.

```
scripts/extract_sprite.py in.mp4 out_dir --key --fps 12 --short 384 --palette   # keyed hero loop
scripts/extract_sprite.py in.mp4 out_dir --fps 12 --short 360 --palette          # full-frame insert / clip
scripts/build_loops.sh                                                            # package all into assets/library/*/loops
```
`extract_sprite.py` keys every frame (same thresholds as `key_cutout.py`) and
crops all frames to one union box so the subject never jumps; drops a final
frame that repeats the first; stores 256-colour PNGs; records the seam SSIM
(last → first, before palette encoding; ≥ 0.9 required).

## Footage plan (operator file)
`<name>.footage.json` (`beat` is 1-based, as in warnings) = `{version, clips:[{beat, role, path, ...}]}`
(`motion_core::footage`, `CompileOptions.footage`, `compile --footage <file>`).
`interstitial` and `screen_insert` stay frozen/unused; `plate` is implemented (0.24).

### Video plates (`role: plate`)
```json
{"version":"0.1","clips":[{"beat":2,"role":"plate","path":"clips/a.mp4","in_s":0.5,"out_s":4.0}]}
```
- `path` = MP4/MOV/WEBM, relative to the plan file. `in_s`/`out_s` optional;
  `frame_count`/`fps` are probed with ffprobe by the CLI when absent.
- The beat scene gets, below its stage group (z 1-2, above the paper): a
  full-bleed `video` asset image layer (cover, desaturate 0.6 + paper tint 0.35,
  slow 1.0 -> 1.06 push, fades in/out with the beat; muted) and a paper-colour
  scrim (opacity 0.55, fixed: frames are not probed at plan time) for text contrast.
  A clip shorter than the beat holds its last frame. Unplaceable clips give a
  `plate_skipped` compile warning. 9:16 and 16:9 (cover crop).
- Asset `type: "video"` carries a `sprite` spec (frame_count, fps = 30, mode) and
  is drawn by image layers like a sprite sequence (`playback`, `sprite_frame`).
  Validation: allowed extensions, spec present, file exists (with a base dir).
- Renderer (`motion-render/src/video.rs`): an `ffmpeg` child writes raw RGBA to a
  pipe (no temp files, no PNGs). A miss decodes the aligned 32-frame window at the
  layer's size; frames live in a byte-bounded LRU, `MOTION_VIDEO_CACHE_MB`
  (default 256). Treatments apply per frame like sprites. Needs `ffmpeg`/`ffprobe` on PATH.

```
motion-engine compile story.intent.json --style s.json --footage story.footage.json --output out.motion.json
```
