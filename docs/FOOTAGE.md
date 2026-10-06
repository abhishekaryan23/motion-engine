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

## Footage plan (operator file, frozen; compiler use not implemented)
`<name>.footage.json` = `{version, clips:[{beat, role: interstitial |
screen_insert, path, frame_count, fps, in_frame, out_frame, mode}]}`
(`motion_core::footage`, `CompileOptions.footage`). Planned: interstitial
footage beats (≤ 6 s, muted) that return to a persistent anchor scene (a
numbered list carried as one SharedElement), and explicit per-beat screen
inserts. Not started in 0.11.
