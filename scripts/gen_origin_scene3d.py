#!/usr/bin/env python3
"""Writes examples/v2/origin.scene3d.json, the 3D plate of examples/v2/origin.program.json.

Run from the repo root: `python3 scripts/gen_origin_scene3d.py`. It prints the
lockup's `rules_from` (where the flattened lattice sits at the cut) and the
scale layers' depths (placed where the camera is on each beat).
"""
import json, math, sys

OUT = sys.argv[1] if len(sys.argv) > 1 else "examples/v2/origin.scene3d.json"

def srgb2lin(c):
    c = c / 255.0
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4

def hexlin(h):
    h = h.lstrip('#')
    return [round(srgb2lin(int(h[i:i + 2], 16)), 5) for i in (0, 2, 4)]

CHAR = "#16171A"
CHAR_SRGB = [round(int(CHAR[i:i + 2], 16) / 255, 4) for i in (1, 3, 5)]
CHAR_LIN = hexlin(CHAR)
CYAN = [0.05, 0.85, 1.0]           # neon rim / ribbon (linear)
FONT_O = "fonts/Poppins-Black.ttf"
FONT_SWISS = "fonts/ArchivoBlack-Regular.ttf"
FONT_MONO = "fonts/IBMPlexMono-Medium.ttf"

S = 4.0                             # O em size (world units)
DEPTH = 1.6
OUT_R = 0.728 * S / 2               # 1.456
CTR_RX, CTR_RY = 0.277 * S / 2, 0.315 * S / 2   # 0.554, 0.63
TUNNEL = 34.0
CURVE_Z = -46.0

T_COND = 1.92      # coordinates arrive, O condenses
T_SHEAR = 2.62     # shear + extrude (beat)
T_FLASH = 4.25     # counter flashes (beat)
T_DIVE = 5.04      # dive hit (downbeat)
T_SHATTER = 8.22   # shatter (downbeat)
T_IMPACT = 13.0    # cut to the lockup (downbeat)

def spring(k, d):
    return {"spring": {"stiffness": k, "damping": d, "mass": 1.0}}

def mat(**kw):
    m = {"base": [0.0, 0.0, 0.0], "roughness": 0.5, "metal": 0.0}
    m.update(kw)
    return m

objects = []
sweeps = []

# --- grids (texture: <= 1.35:1 on the ground) -----------------------------
grid_mat = mat(base=[0, 0, 0], roughness=1.0, lit=0.0, emissive=CHAR_LIN, grid=1.0, grid_pitch=0.25,
               grid_px=1.0, grid_colour=hexlin("#33363C"), grid_fade=7.5)
objects.append({"id": "grid", "mesh": {"kind": "plane", "size": [44.0, 26.0]},
                "material": grid_mat, "position": [0, 0, -2.4], "visible": [0.0, 4.98]})
objects.append({"id": "grid_far", "mesh": {"kind": "plane", "size": [60.0, 36.0]},
                "material": dict(grid_mat, grid_fade=22.0), "position": [0, 0, CURVE_Z - 7.0]})

# --- coordinates (each names where it sits; they fall to the origin) ------
coords = [(-3.4, 1.7), (-1.9, 2.25), (0.4, 2.35), (2.3, 1.95), (3.6, 0.75), (3.1, -1.35),
          (1.1, -2.2), (-1.2, -2.05), (-3.15, -1.05), (-2.6, 0.35), (2.0, 0.15), (-0.5, 1.15)]
ping_from, ping_to, ping_t0, ping_dur = -7.5, 7.5, 0.1, 1.05
pdir = [1 / math.sqrt(2), -1 / math.sqrt(2), 0.0]
for i, (x, y) in enumerate(coords):
    txt = f"{x:+.2f}, {y:+.2f}".replace("-", "−")
    s = x * pdir[0] + y * pdir[1]
    t_pass = ping_t0 + (s - ping_from) / (ping_to - ping_from) * ping_dur
    d = math.hypot(x, y)
    # Gravity: a slow creep that accelerates into the origin (farther = longer).
    over = 1.05 + 0.12 * math.sqrt(d)
    at = T_COND - over
    dim = [0.012] * 3
    lit = [0.62, 0.64, 0.68]
    objects.append({
        "id": f"coord{i}",
        "mesh": {"kind": "text", "text": txt, "font": FONT_MONO, "size": 0.2, "depth": 0.004},
        "material": mat(base=[0, 0, 0], roughness=1.0, lit=0.0, emissive=dim),
        "position": [x, y, 0.0],
        "visible": [0.0, T_COND + 0.02],
        "keys": [
            {"at_s": t_pass - 0.06, "over_s": 0.3, "ease": "out",
             "material": mat(base=[0, 0, 0], roughness=1.0, lit=0.0, emissive=lit)},
            {"at_s": at, "over_s": over, "ease": "in",
             "position": [0.0, 0.0, 0.0], "scale": [0.03, 0.03, 0.03]},
        ],
    })
sweeps.append({"only": ["grid", "grid_far"] + [f"coord{i}" for i in range(len(coords))],
               "origin": [0, 0, 0], "dir": pdir, "from": ping_from, "to": ping_to,
               "start_s": ping_t0, "duration_s": ping_dur, "ease": "in_out", "width": 0.55,
               "colour": [0.11, 0.12, 0.13]})

# --- the O: condense, shear + extrude, tilt, straighten --------------------
o_mat = mat(base=[0.02, 0.02, 0.022], roughness=0.28, metal=0.85, brushed=0.5,
            rim=[c * 3.0 for c in CYAN], rim_power=30.0)
block_keys = [
    {"at_s": 0.0, "over_s": 0.0, "scale": [0.02, 0.02, 0.01]},
    {"at_s": T_COND - 0.03, "over_s": 0.6, "ease": spring(190, 11), "scale": [1.0, 1.0, 0.012]},
    # Shear in at peak momentum while the body extrudes backwards.
    {"at_s": T_SHEAR - 0.06, "over_s": 0.26, "ease": "in", "shear": [0.42, 0.42]},
    {"at_s": T_SHEAR, "over_s": 0.55, "ease": "out", "scale": [1.0, 1.0, 1.0]},
    {"at_s": T_SHEAR + 0.24, "over_s": 0.8, "ease": spring(150, 10), "shear": [0.0, 0.0]},
    # Tilts back under the locked gaze.
    {"at_s": 3.1, "over_s": 0.75, "ease": "smooth", "rotation_deg": [-20.0, 16.0, 0.0]},
    # Straightens to face the dive.
    {"at_s": 3.72, "over_s": 0.55, "ease": "smooth", "rotation_deg": [0.0, 0.0, 0.0]},
]
hot = dict(o_mat, emissive=[1.5, 1.52, 1.58])
objects.append({"id": "o", "mesh": {"kind": "glyph", "text": "O", "font": FONT_O, "size": S,
                                     "depth": DEPTH, "bevel": 0.07},
                "material": hot, "visible": [T_COND - 0.03, T_SHATTER],
                # Condenses white-hot from the coordinates' light, cools to black.
                # It stays warm through the flat phase and goes black as the
                # extrusion reveals its form.
                "keys": block_keys + [{"at_s": T_COND + 0.04, "over_s": 0.3, "ease": "out",
                                       "material": dict(o_mat, emissive=[0.075, 0.077, 0.082])},
                                      {"at_s": T_SHEAR - 0.05, "over_s": 0.5, "ease": "smooth",
                                       "material": o_mat}]})

def with_scale(keys, sx, sy):
    out = []
    for k in keys:
        k = dict(k)
        if "scale" in k:
            k["scale"] = [k["scale"][0] * sx, k["scale"][1] * sy, k["scale"][2]]
        out.append(k)
    return out

glass = mat(base=[0.92, 0.96, 1.0], roughness=0.06, transmission=1.0, ior=1.45, dispersion=0.1)
# The counter: flashes white, settles into clear glass.
flash_keys = with_scale(block_keys, 0.995, 0.995) + [
    {"at_s": T_FLASH, "over_s": 0.07, "ease": "out",
     "material": mat(base=[0.92, 0.96, 1.0], roughness=0.06, emissive=[7.0, 7.0, 7.2], ior=1.45)},
    {"at_s": T_FLASH + 0.09, "over_s": 0.55, "ease": "out", "material": glass},
]
objects.append({"id": "counter", "mesh": {"kind": "cylinder", "radius": CTR_RX, "length": DEPTH,
                                           "sides": 72, "caps": True},
                "material": mat(base=[0.92, 0.96, 1.0], roughness=0.06, ior=1.45, transmission=0.002),
                "visible": [T_FLASH - 0.02, 4.97],
                "keys": [{"at_s": 0, "over_s": 0, "scale": [1.0, CTR_RY / CTR_RX, 1.0]}] +
                        with_scale(flash_keys, 1.0, CTR_RY / CTR_RX)})
# The tunnel: the counter's glass running deep behind the O.
objects.append({"id": "tunnel", "mesh": {"kind": "cylinder", "radius": CTR_RX * 0.99, "length": DEPTH,
                                          "sides": 96},
                "material": glass, "visible": [T_FLASH + 0.25, T_SHATTER],
                "keys": [{"at_s": 0, "over_s": 0, "scale": [1.0, CTR_RY / CTR_RX, 1.0]},
                         {"at_s": T_FLASH + 0.2, "over_s": 0.6, "ease": "in",
                          "scale": [1.0, CTR_RY / CTR_RX, TUNNEL / DEPTH]}]})
# A faint measuring lattice around the glass (seen through it, dispersed).
objects.append({"mesh": {"kind": "cylinder", "radius": CTR_RX * 1.32, "length": DEPTH, "sides": 120,
                         "ribs": [1.0, 20.0]},
                "material": mat(base=[0, 0, 0], lit=0.0, emissive=[0.05, 0.055, 0.06], wire=1.0, wire_px=0.8),
                "visible": [T_FLASH + 0.25, T_SHATTER],
                "keys": [{"at_s": 0, "over_s": 0, "scale": [1.0, CTR_RY / CTR_RX, 1.0]},
                         {"at_s": T_FLASH + 0.2, "over_s": 0.6, "ease": "in",
                          "scale": [1.0, CTR_RY / CTR_RX, TUNNEL / DEPTH]}]})
# Scale markers: thin rings just outside the glass (seen through it, dispersed).
for k in range(1, int(TUNNEL)):
    z = -1.0 * k - 0.6
    objects.append({"mesh": {"kind": "torus", "radius": CTR_RX * 1.14, "tube": 0.007},
                    "material": mat(base=[0, 0, 0], lit=0.0, emissive=[0.55, 0.58, 0.62] if k % 5 else [1.4, 1.45, 1.5]),
                    "position": [0, 0, z], "visible": [T_FLASH + 0.3, T_SHATTER],
                    "keys": [{"at_s": 0, "over_s": 0, "scale": [1.0, CTR_RY / CTR_RX, 1.0]}]})

# Swiss type layers: a descent through scales toward the origin, each passed
# on a beat (placed where the camera is on that beat), the last one held.
BEAT = 60.0 / 150.8
PASS_T = [T_DIVE + BEAT * k for k in range(1, 6)]
DIVE_A = (4.42, 0.64, -0.8)          # punch: at, over, eye z (ease in)
DIVE_B = (5.0, 2.3, -28.6)           # cruise: linear
DIVE_C = (7.15, 0.85, -TUNNEL + 1.6) # brake: out (quintic) — stops before the last layer

def cube_in(u): return u ** 3
def lin(u): return u
def quint_out(u): return 1 - (1 - u) ** 5
def prog(t, at, over, f):
    u = min(1.0, max(0.0, (t - at) / over))
    return f(u)
def eye_z(t):
    z = 6.5
    z += (DIVE_A[2] - z) * prog(t, DIVE_A[0], DIVE_A[1], cube_in)
    z += (DIVE_B[2] - z) * prog(t, DIVE_B[0], DIVE_B[1], lin)
    z += (DIVE_C[2] - z) * prog(t, DIVE_C[0], DIVE_C[1], quint_out)
    return z

layers = [("1 m", "1 : 1"), ("10 cm", "1 : 10"), ("1 cm", "1 : 100"), ("1 mm", "1 : 1 000"),
          ("100 \u00b5m", "1 : 10 000"), ("10 \u00b5m", "1 : 100 000")]
LAYER_Z = [eye_z(t) for t in PASS_T] + [DIVE_C[2] - 1.0]
print("layer z:", [round(z, 2) for z in LAYER_Z])
for i, (big, ratio) in enumerate(layers):
    z = LAYER_Z[i]
    w_em = {3: 0.4, 4: 0.3, 5: 0.25, 6: 0.21}.get(len(big), 0.24)
    side = -1 if i % 2 else 1
    lx = 0.1 * side if i < 5 else 0.0
    white = mat(base=[0, 0, 0], roughness=1.0, lit=0.0, emissive=[0.92, 0.93, 0.95])
    cyan_t = mat(base=[0, 0, 0], roughness=1.0, lit=0.0, emissive=[0.6, 0.85, 0.95])
    vis = [T_FLASH + 0.3, T_SHATTER]
    # Passed layers dim just before the camera goes through (no flash).
    def dimmed(m):
        if i == 5:
            return []
        dark = dict(m, emissive=[c * 0.12 for c in m["emissive"]])
        return [{"at_s": PASS_T[i] - 0.13, "over_s": 0.12, "ease": "in", "material": dark}]
    parts = [
        ({"kind": "text", "text": big, "font": FONT_SWISS, "size": w_em, "depth": 0.01}, white, [lx, 0.07, z]),
        ({"kind": "text", "text": ratio, "font": FONT_MONO, "size": 0.06, "depth": 0.004}, cyan_t, [lx, -0.13, z]),
        ({"kind": "box", "size": [0.5, 0.008, 0.004]}, white, [lx, -0.22, z]),
    ] + [({"kind": "box", "size": [0.008, 0.05 if tx else 0.03, 0.004]}, white, [lx + tx, -0.205, z])
         for tx in (-0.25, 0.0, 0.25)]
    for mesh, m, pos in parts:
        objects.append({"mesh": mesh, "material": m, "position": pos, "visible": vis,
                        "keys": dimmed(m)})

# --- the stream, the ribbon, the lattice -----------------------------------
CS = 1.2
curve = [[x * CS, y * CS, z] for x, y, z in
         [[-4.6, -2.45, 0.6], [-2.7, -2.15, 0.2], [-1.0, -0.75, -0.6], [0.9, 0.75, 0.5],
          [2.7, 2.15, -0.2], [4.6, 2.45, 0.3]]]
cpos = [0.0, 0.0, CURVE_Z]
T_LEAD = T_SHATTER + 0.06 + 1.35   # lead bead arrives
stream = {
    "kind": "stream", "curve": {"points": curve}, "count": 46, "radius": 0.17, "seed": 7,
    "start_s": T_SHATTER + 0.04, "travel_s": 1.35, "stagger_s": 0.022, "spacing": 0.0175,
    "source": {"centre": [0.0, 0.0, -TUNNEL + 0.4 - CURVE_Z], "axis": [0, 0, 1],
               "radius": CTR_RX * 1.02, "length": 0.9},
    "gather_s": 0.55, "stretch": 0.16, "jitter": 0.35,
    "fade": {"start_s": T_LEAD + 0.05, "duration_s": 0.7, "ease": "smooth"},
}
MAGENTA = [0.95, 0.12, 0.75]
ferro = mat(base=[0.05, 0.05, 0.055], roughness=0.1, metal=1.0, iridescence=0.9,
            film=[CYAN, MAGENTA],
            rim=[c * 1.2 for c in CYAN], rim_power=10.0)
objects.append({"id": "stream", "mesh": stream, "material": ferro, "position": cpos,
                "visible": [T_SHATTER, T_LEAD + 1.0]})

glow = mat(base=[0.0, 0.0, 0.0], roughness=0.3, emissive=[c * 2.2 for c in CYAN])
solid = mat(base=[0.78, 0.8, 0.82], roughness=0.32, metal=0.7, emissive=[0.03, 0.035, 0.04],
            wire_px=1.4, wire_glow=[0.35, 0.37, 0.4])
T_FREEZE = T_LEAD + 0.62
tube = {"kind": "tube", "curve": {"points": curve}, "radius": 0.035, "sides": 18, "segments": 260,
        "ribs": [44.0, 9.0],
        "draw": {"start_s": T_LEAD - 0.12, "duration_s": 0.2, "ease": "out"},
        "slack": {"amount": 0.7, "dir": [0.0, -1.0, 0.35], "start_s": T_LEAD - 0.05,
                  "duration_s": 1.1, "spring": {"stiffness": 260.0, "damping": 8.0, "mass": 1.0}},
        "thicken": {"radius": 0.3, "start_s": T_FREEZE, "duration_s": 0.4, "ease": spring(220, 18)}}
T_SCAN = T_FREEZE + 0.55
T_EXPAND = 12.0
objects.append({"id": "ribbon", "mesh": tube, "material": glow, "position": cpos,
                "visible": [T_LEAD - 0.12, T_IMPACT],
                "keys": [
                    {"at_s": T_FREEZE, "over_s": 0.4, "ease": "out", "material": solid},
                    {"at_s": T_EXPAND, "over_s": 0.55, "ease": "out", "scale": [1.22, 1.22, 1.22]},
                    {"at_s": 12.42, "over_s": 0.56, "ease": "in", "scale": [1.3, 1.3, 0.001]},
                ]})
# The scanline runs back down the ribbon (top right → bottom left), stripping it.
sdir = [-0.894, -0.447, 0.0]
sweeps.append({"only": ["ribbon"], "origin": cpos, "dir": sdir, "from": -8.0, "to": 8.0,
               "start_s": T_SCAN - 0.1, "duration_s": 1.3, "ease": "in_out", "width": 0.05,
               "colour": [c * 4.0 for c in CYAN], "strip": True})

# --- camera ------------------------------------------------------------------
TR = [OUT_R * 0.7071, OUT_R * 0.7071, 0.0]          # the O's top-right extremity
EYE_END, EYE_PUSH, FOV_END = -TUNNEL + 1.0, -TUNNEL - 0.6, 30.0
cam_keys = [
    {"at_s": 0.0, "orbit": [0.0, 0.0, 9.2], "target": [0, 0, 0], "fov_deg": 35.0, "aperture_px": 0.0},
    {"at_s": 0.0, "over_s": 2.6, "ease": "smooth", "orbit": [0.0, 0.0, 8.3]},
    # Barrel roll along the extrusion edge, gaze locking to the top-right.
    {"at_s": T_SHEAR + 0.2, "over_s": 1.0, "ease": "in_out", "orbit": [40.0, 16.0, 7.0],
     "target": [TR[0] * 0.55, TR[1] * 0.55, -0.4], "roll_deg": 360.0},
    {"at_s": T_SHEAR + 0.95, "over_s": 0.6, "ease": "smooth", "orbit": [32.0, 9.0, 7.2]},
    # Square up on the counter, then dive down its axis.
    {"at_s": 3.66, "over_s": 0.6, "ease": "smooth", "target": [0.0, 0.0, -6.0],
     "orbit": [0.0, 0.0, 12.5]},
    # Punch into the core on the downbeat, cruise past a layer per beat,
    # brake and land on the last layer.
    {"at_s": DIVE_A[0], "over_s": DIVE_A[1], "ease": "in", "eye": [0.0, 0.0, DIVE_A[2]],
     "target": [0.0, 0.0, -20.0]},
    {"at_s": DIVE_B[0], "over_s": DIVE_B[1], "ease": "linear", "eye": [0.0, 0.0, DIVE_B[2]],
     "target": [0.0, 0.0, -TUNNEL - 12.0]},
    {"at_s": DIVE_C[0], "over_s": DIVE_C[1], "ease": "out", "eye": [0.0, 0.0, DIVE_C[2]]},
    {"at_s": T_DIVE - 0.32, "over_s": 0.45, "ease": "out", "fov_deg": 104.0, "aperture_px": 5.0},
    {"at_s": 5.2, "over_s": 0.6, "ease": "smooth", "fov_deg": 66.0, "aperture_px": 9.0},
    {"at_s": 7.1, "over_s": 0.8, "ease": "smooth", "fov_deg": 62.0, "aperture_px": 7.0},
    # Out of the glass: pull the lens back in and frame the stream.
    {"at_s": T_SHATTER + 0.05, "over_s": 0.95, "ease": "smooth", "fov_deg": FOV_END, "aperture_px": 0.0},
    {"at_s": T_SHATTER + 0.05, "over_s": 1.5, "ease": "out", "eye": [0.0, 0.0, EYE_END],
     "target": [0.0, 0.0, CURVE_Z]},
    # Ease in towards the lattice as it expands into the fold.
    {"at_s": T_EXPAND, "over_s": 1.0, "ease": "in", "eye": [0.0, 0.0, EYE_PUSH]},
]
camera = {"target": [0, 0, 0], "fov_deg": 35.0, "keys": cam_keys,
          "focus": [{"at_s": 0.0, "distance": 8.3}, {"at_s": 4.6, "distance": 2.4, "over_s": 0.5},
                    {"at_s": 7.1, "distance": 1.0, "over_s": 0.8},
                    {"at_s": T_SHATTER, "distance": 11.0, "over_s": 0.6}],
          "aperture_px": 0.0}

scene = {
    "width": 1920, "height": 1080, "fps": 30, "duration_s": T_IMPACT,
    "sky": [CHAR_SRGB, CHAR_SRGB], "ambient": 0.32,
    "lights": [
        {"dir": [-0.55, 0.75, 0.55], "colour": [1.0, 0.97, 0.92], "intensity": 3.0},
        {"dir": [0.85, 0.25, -0.45], "colour": [0.45, 0.85, 1.0], "intensity": 2.2},
        {"dir": [0.1, -0.5, 1.0], "colour": [1.0, 0.3, 0.8], "intensity": 0.3},
    ],
    "objects": objects, "camera": camera, "sweeps": sweeps,
    "ao": 0.8, "ao_radius": 0.3, "glass_px": 28.0, "fringe_px": 1.6,
    "softboxes": [
        {"dir": [0.0, 1.0, 0.3], "radius_deg": 22.0, "colour": [1.3, 1.3, 1.32]},
        {"dir": [-0.9, 0.25, 0.45], "radius_deg": 12.0, "colour": [0.8, 0.78, 0.74]},
        {"dir": [-0.35, 0.3, 1.0], "radius_deg": 14.0, "colour": [0.16, 0.16, 0.17]},
        {"dir": [0.6, 0.35, -0.8], "radius_deg": 9.0, "colour": [c * 0.9 for c in CYAN]},
    ],
}
with open(OUT, "w") as f:
    json.dump(scene, f, indent=1)
print("wrote", OUT, len(objects), "objects")

# Where the flattened lattice sits on screen at the cut (for the lockup's rules_from).
t = T_IMPACT - 0.03
p_in = ((t - T_EXPAND) / 1.0) ** 3
eye_z = EYE_END + (EYE_PUSH - EYE_END) * min(1.0, p_in)
k = ((t - 12.42) / 0.56) ** 3
sc = 1.22 + (1.3 - 1.22) * min(1.0, k)
d = -CURVE_Z + eye_z
th = math.tan(math.radians(FOV_END / 2))
def proj(x, y):
    return [round(0.5 + x * sc / (d * th * 16 / 9) / 2, 3), round(0.5 - y * sc / (d * th) / 2, 3)]
a, b = proj(*curve[0][:2]), proj(*curve[-1][:2])
print("rules_from:", json.dumps([a, [round(a[0] + (b[0] - a[0]) * 0.4, 3), a[1]],
                                  [round(a[0] + (b[0] - a[0]) * 0.6, 3), b[1]], b]))
