#!/usr/bin/env python3
"""Regenerate the skin assets from the source photo.

Usage: python3 tools/make_assets.py
Reads  assets/skin_source.png
Writes assets/skin_base.png       (label text erased)
       assets/hub_sprite_flat.png (flat-fielded, feathered circular hub crop)

Tweak the calibration constants below if you swap in a different photo,
then update the matching constants in src/cassette.rs.
"""
from PIL import Image, ImageDraw, ImageFilter
import numpy as np
import random, statistics, pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent / "assets"

TOP_HUB = (428, 507)
HUB_R = 74
LABEL = (222, 335, 330, 1018)

src = Image.open(ROOT / "skin_source.png").convert("RGB")

# --- hub sprite: circular crop, flat-fielded (remove baked directional light),
# feathered alpha edge ---
cx, cy = TOP_HUB
crop = src.crop((cx - HUB_R, cy - HUB_R, cx + HUB_R, cy + HUB_R)).convert("RGBA")
arr = np.asarray(crop).astype(np.float32)
rgb, alpha = arr[..., :3], arr[..., 3]
lighting = np.asarray(
    Image.fromarray(rgb.astype(np.uint8)).filter(
        ImageFilter.GaussianBlur(HUB_R * 2 * 0.28)
    )
).astype(np.float32) + 1.0
flat = np.clip(rgb / lighting * lighting.mean(axis=(0, 1), keepdims=True), 0, 255)
mask = Image.new("L", crop.size, 0)
ImageDraw.Draw(mask).ellipse([0, 0, crop.size[0], crop.size[1]], fill=255)
mask = mask.filter(ImageFilter.GaussianBlur(1.5))
out = np.dstack([flat, np.asarray(mask)]).astype(np.uint8)
Image.fromarray(out, "RGBA").save(ROOT / "hub_sprite_flat.png")

# --- skin base: erase baked label text with sampled gradient + grain ---
base = src.copy()
x0, y0, x1, y1 = LABEL
def med(region):
    px = list(region.getdata())
    return tuple(int(statistics.median(c[i] for c in px)) for i in range(3))
top_col = med(base.crop((x0 + 8, 345, x1 - 8, 415)))
bot_col = med(base.crop((x0 + 8, 975, x1 - 8, 1015)))
d = ImageDraw.Draw(base)
for y in range(y0, y1):
    t = (y - y0) / (y1 - y0)
    d.line([(x0, y), (x1, y)],
           fill=tuple(int(top_col[i] * (1 - t) + bot_col[i] * t) for i in range(3)))
random.seed(7)
px = base.load()
for _ in range(6000):
    x, y = random.randint(x0, x1 - 1), random.randint(y0, y1 - 1)
    dv = random.randint(-4, 3)
    r0, g0, b0 = px[x, y]
    px[x, y] = tuple(max(0, min(255, v + dv)) for v in (r0, g0, b0))
base.save(ROOT / "skin_base.png")
print("assets regenerated")

# --- pressed-state key patches: backdrop strip above + key slid down by T,
# bottom clipped behind the deck edge. T MUST match PRESS_TRAVEL in cassette.rs.
from PIL import ImageEnhance
T = 6
BUTTONS = {
    "play": (162, 66, 236, 124),
    "ff": (250, 66, 320, 124),
    "rew": (334, 66, 404, 124),
    "stop": (422, 66, 490, 124),
    "funct": (532, 66, 690, 124),
}
for name, (bx0, by0, bx1, by1) in BUTTONS.items():
    patch = src.crop((bx0, by0 - T, bx1, by1))
    w, h = patch.size
    key = ImageEnhance.Brightness(patch.crop((0, T, w, h))).enhance(0.86)
    out = patch.copy()
    out.paste(patch.crop((0, 0, w, T)), (0, T))
    out.paste(key, (0, 2 * T))
    out.save(ROOT / f"btn_{name}_pressed.png")
print("key patches regenerated")
