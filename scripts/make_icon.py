#!/usr/bin/env python3
"""
Generate Lumen.app icon.

Design:
  - Dark navy rounded-square background, slightly lighter toward center
  - Slim orange border ring -- definition at all sizes without dominating
  - Gauge track arc (visible warm gray), full sweep with gap at bottom
  - Bright lit orange arc from start to indicator
  - Hero "homing" dot -- large glowing orange ball at the indicator edge
  - Thin needle from pivot to indicator
  - Small dark pivot hub at center
"""

import math
import os
import shutil
import subprocess
from pathlib import Path

try:
    from PIL import Image, ImageDraw, ImageFilter
except ImportError:
    raise SystemExit("pip install pillow")

SIZES = [16, 32, 64, 128, 256, 512, 1024]

# Gauge geometry -- all as fractions of icon size
# PIL angles: 0°=East, increasing clockwise in screen space
# 135° = lower-left (7:30 o'clock), 405° = lower-right (4:30), 270° sweep with gap at bottom
ARC_START_DEG  = 135   # lower-left
ARC_END_DEG    = 405   # lower-right (= 45° + 360, gives a 270° clockwise sweep through top)
INDICATOR_FRAC = 0.65  # 65% -> upper-right (~1:30 o'clock), active/energetic reading

# Unified orange palette -- all gauge elements share this base hue
ORANGE_BASE   = (255, 122, 25)   # border + lit arc base
ORANGE_BRIGHT = (255, 168, 55)   # dot (lighter, higher luminance)


def rounded_rect_mask(size: int, radius_frac: float = 0.22) -> Image.Image:
    mask = Image.new("L", (size, size), 0)
    d = ImageDraw.Draw(mask)
    r = int(size * radius_frac)
    d.rounded_rectangle([0, 0, size - 1, size - 1], radius=r, fill=255)
    return mask


def composite(base: Image.Image, layer: Image.Image) -> Image.Image:
    return Image.alpha_composite(base, layer)


def draw_icon(size: int) -> Image.Image:
    S = size
    cx = S / 2
    cy = S * 0.56   # pivot sits in lower half; arc sweeps up through the top

    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))

    # ── Background ─────────────────────────────────────────────────────
    bg = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    bg_d = ImageDraw.Draw(bg)
    rr = int(S * 0.22)
    bg_d.rounded_rectangle(
        [0, 0, S - 1, S - 1],
        radius=rr,
        fill=(14, 13, 26, 255),        # dark navy-purple
    )
    img = composite(img, bg)

    # Subtle warm inner glow centered on the gauge pivot
    glow_layer = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    glow_r = int(S * 0.46)
    glow_d = ImageDraw.Draw(glow_layer)
    glow_d.ellipse([cx - glow_r, cy - glow_r, cx + glow_r, cy + glow_r],
                   fill=(38, 35, 68, 70))
    glow_layer = glow_layer.filter(ImageFilter.GaussianBlur(S * 0.18))
    img = composite(img, glow_layer)

    # ── Orange border ring ──────────────────────────────────────────────
    # Slim inset stroke -- gives definition without eating the interior
    border_inset = max(1, int(S * 0.030))
    border_w     = max(1, int(S * 0.022))   # thinner than before
    inner_r      = int((S - 2 * border_inset) * 0.20)

    border_layer = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    border_d = ImageDraw.Draw(border_layer)
    border_d.rounded_rectangle(
        [border_inset, border_inset, S - 1 - border_inset, S - 1 - border_inset],
        radius=inner_r,
        outline=(*ORANGE_BASE, 235),
        width=border_w,
    )
    img = composite(img, border_layer)

    # Soft border glow -- inward, so it warms the interior edge
    border_glow = border_layer.filter(ImageFilter.GaussianBlur(S * 0.024))
    img = composite(img, border_glow)

    # ── Gauge geometry ──────────────────────────────────────────────────
    arc_r   = S * 0.305         # fits 270° sweep within border at cy=0.56
    arc_box = [cx - arc_r, cy - arc_r, cx + arc_r, cy + arc_r]
    arc_w   = max(2, int(S * 0.060))

    sweep       = ARC_END_DEG - ARC_START_DEG           # degrees in sweep
    ind_deg     = ARC_START_DEG + sweep * INDICATOR_FRAC # ~256°
    ind_rad     = math.radians(ind_deg)
    dot_x       = cx + arc_r * math.cos(ind_rad)
    dot_y       = cy + arc_r * math.sin(ind_rad)

    # ── Track arc (warm gray, full sweep) ──────────────────────────────
    track = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    track_d = ImageDraw.Draw(track)
    track_d.arc(arc_box, start=ARC_START_DEG, end=ARC_END_DEG,
                fill=(75, 68, 98, 205), width=arc_w)
    img = composite(img, track)

    # ── Lit arc (orange, same hue as border) ───────────────────────────
    # Glow layer behind the solid arc
    lit_glow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    lit_glow_d = ImageDraw.Draw(lit_glow)
    lit_glow_d.arc(arc_box, start=ARC_START_DEG, end=int(ind_deg),
                   fill=(*ORANGE_BASE, 155), width=arc_w * 2)
    lit_glow = lit_glow.filter(ImageFilter.GaussianBlur(S * 0.038))
    img = composite(img, lit_glow)

    lit = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    lit_d = ImageDraw.Draw(lit)
    lit_d.arc(arc_box, start=ARC_START_DEG, end=int(ind_deg),
              fill=(*ORANGE_BASE, 255), width=arc_w)
    img = composite(img, lit)

    # ── Hero "homing" dot ──────────────────────────────────────────────
    # Layer 1: wide bloom -- same orange family as border
    bloom_r = S * 0.15
    bloom = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    bloom_d = ImageDraw.Draw(bloom)
    bloom_d.ellipse(
        [dot_x - bloom_r, dot_y - bloom_r, dot_x + bloom_r, dot_y + bloom_r],
        fill=(*ORANGE_BASE, 145),
    )
    bloom = bloom.filter(ImageFilter.GaussianBlur(S * 0.088))
    img = composite(img, bloom)

    # Layer 2: tight halo
    halo_r = S * 0.078
    halo = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    halo_d = ImageDraw.Draw(halo)
    halo_d.ellipse(
        [dot_x - halo_r, dot_y - halo_r, dot_x + halo_r, dot_y + halo_r],
        fill=(*ORANGE_BRIGHT, 205),
    )
    halo = halo.filter(ImageFilter.GaussianBlur(S * 0.028))
    img = composite(img, halo)

    # Layer 3: solid dot
    dot_r = S * 0.058
    dot_layer = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    dot_d = ImageDraw.Draw(dot_layer)
    dot_d.ellipse(
        [dot_x - dot_r, dot_y - dot_r, dot_x + dot_r, dot_y + dot_r],
        fill=(*ORANGE_BRIGHT, 255),
    )
    # Specular highlight -- keeps the glassy look
    spec_r = dot_r * 0.44
    dot_d.ellipse(
        [dot_x - spec_r, dot_y - dot_r * 1.1,
         dot_x + spec_r, dot_y - dot_r * 0.12],
        fill=(255, 248, 195, 215),
    )
    img = composite(img, dot_layer)

    # ── Needle (thin, from pivot to just inside the dot) ───────────────
    # Skip at very small sizes -- too thin to render cleanly
    if S >= 64:
        needle_layer = Image.new("RGBA", (S, S), (0, 0, 0, 0))
        needle_d = ImageDraw.Draw(needle_layer)
        # Stop needle just short of the dot center so it doesn't poke through
        reach = arc_r * 0.82
        nx = cx + reach * math.cos(ind_rad)
        ny = cy + reach * math.sin(ind_rad)
        nw = max(1, int(S * 0.016))
        needle_d.line([(cx, cy), (nx, ny)], fill=(*ORANGE_BASE, 190), width=nw)
        img = composite(img, needle_layer)

    # ── Pivot hub ──────────────────────────────────────────────────────
    if S >= 32:
        hub_r = max(2, int(S * 0.032))
        hub = Image.new("RGBA", (S, S), (0, 0, 0, 0))
        hub_d = ImageDraw.Draw(hub)
        hub_d.ellipse(
            [cx - hub_r, cy - hub_r, cx + hub_r, cy + hub_r],
            fill=(35, 30, 50, 220),
        )
        # Orange ring on pivot -- same hue as everything else
        hub_d.ellipse(
            [cx - hub_r, cy - hub_r, cx + hub_r, cy + hub_r],
            outline=(*ORANGE_BASE, 170),
            width=max(1, int(S * 0.012)),
        )
        img = composite(img, hub)

    # ── Apply rounded-rect mask ────────────────────────────────────────
    mask = rounded_rect_mask(S)
    img.putalpha(mask)
    return img


def build_iconset(out_dir: Path):
    out_dir.mkdir(parents=True, exist_ok=True)
    mapping = {
        16:   ("icon_16x16.png",),
        32:   ("icon_16x16@2x.png", "icon_32x32.png"),
        64:   ("icon_32x32@2x.png",),
        128:  ("icon_128x128.png",),
        256:  ("icon_128x128@2x.png", "icon_256x256.png"),
        512:  ("icon_256x256@2x.png", "icon_512x512.png"),
        1024: ("icon_512x512@2x.png",),
    }
    for size, names in mapping.items():
        img = draw_icon(size)
        for name in names:
            img.save(out_dir / name, "PNG")
            print(f"  {name}")


def main():
    repo        = Path(__file__).parent.parent
    iconset_dir = repo / "Lumen" / "Resources" / "AppIcon.iconset"
    icns_path   = repo / "Lumen" / "Resources" / "AppIcon.icns"

    print("Generating icon sizes...")
    build_iconset(iconset_dir)

    print("Converting to .icns...")
    subprocess.run(
        ["iconutil", "-c", "icns", str(iconset_dir), "-o", str(icns_path)],
        check=True,
    )
    shutil.rmtree(iconset_dir)
    print(f"Written: {icns_path}")


if __name__ == "__main__":
    main()
