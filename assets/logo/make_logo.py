#!/usr/bin/env python3
"""Generate the SimpleRecord logo: assets/logo/logo.svg and support/icon.png (1024px).

Usage: python3 assets/logo/make_logo.py   (needs Pillow)
A red record dot inside a timer ring that is open at the top right.
"""
import math, os
from PIL import Image, ImageDraw, ImageFilter, ImageChops

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
S = 4  # supersampling
N = 1024 * S
C = N // 2

RING_R, RING_W = 270, 36
GAP_CENTER, GAP = -50, 64  # degrees, y down: -50 is upper right
A0 = GAP_CENTER + GAP / 2  # arc runs clockwise from A0 ...
A1 = GAP_CENTER - GAP / 2 + 360  # ... to A1


def pt(a, r=RING_R):
    return 512 + r * math.cos(math.radians(a)), 512 + r * math.sin(math.radians(a))


def lerp(c1, c2, t):
    return tuple(int(c1[i] + (c2[i] - c1[i]) * t) for i in range(3))


def vgrad(size, top, bottom):
    g = Image.linear_gradient("L").resize((size, size))
    return Image.composite(Image.new("RGB", (size, size), bottom), Image.new("RGB", (size, size), top), g)


def squircle_mask(x0, y0, x1, y1, r):
    m = Image.new("L", (N, N), 0)
    ImageDraw.Draw(m).rounded_rectangle([v * S for v in (x0, y0, x1, y1)], r * S, fill=255)
    return m


def build_png():
    img = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    bg = vgrad(N, (0x34, 0x36, 0x4A), (0x14, 0x15, 0x1C)).convert("RGBA")
    img.paste(bg, mask=squircle_mask(100, 100, 924, 924, 185))
    # inner hairline
    hl = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    ImageDraw.Draw(hl).rounded_rectangle([102 * S, 102 * S, 922 * S, 922 * S], 183 * S,
                                         outline=(255, 255, 255, 30), width=4 * S)
    img = Image.alpha_composite(img, hl)
    # glow
    glow = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    ImageDraw.Draw(glow).ellipse([(512 - 170) * S, (512 - 170) * S, (512 + 170) * S, (512 + 170) * S],
                                 fill=(255, 59, 48, 150))
    glow = glow.filter(ImageFilter.GaussianBlur(60 * S))
    img = Image.alpha_composite(img, glow)
    # ring (arc with round caps), light gradient top-left to bottom-right
    ring = Image.new("L", (N, N), 0)
    d = ImageDraw.Draw(ring)
    ro = RING_R + RING_W / 2  # PIL strokes inward from the bbox
    bbox = [(512 - ro) * S, (512 - ro) * S, (512 + ro) * S, (512 + ro) * S]
    d.arc(bbox, A0, A1, fill=255, width=RING_W * S)
    for a in (A0, A1):
        x, y = pt(a)
        r = RING_W / 2
        d.ellipse([(x - r) * S, (y - r) * S, (x + r) * S, (y + r) * S], fill=255)
    diag = Image.linear_gradient("L").rotate(45, resample=Image.BICUBIC).resize((N, N))
    ring_col = Image.composite(Image.new("RGB", (N, N), (0xB9, 0xBC, 0xCD)),
                               Image.new("RGB", (N, N), (255, 255, 255)), diag).convert("RGBA")
    img.paste(ring_col, mask=ring)
    # dot with radial shading
    dot = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    r = 152
    dm = Image.new("L", (N, N), 0)
    ImageDraw.Draw(dm).ellipse([(512 - r) * S, (512 - r) * S, (512 + r) * S, (512 + r) * S], fill=255)
    base = Image.new("RGB", (N, N), (0xE8, 0x2A, 0x1F))
    spot = Image.new("L", (N, N), 0)
    ImageDraw.Draw(spot).ellipse([(470 - 150) * S, (450 - 150) * S, (470 + 150) * S, (450 + 150) * S], fill=255)
    spot = spot.filter(ImageFilter.GaussianBlur(70 * S))
    light = Image.new("RGB", (N, N), (0xFF, 0x7A, 0x6B))
    base = Image.composite(light, base, spot)
    img.paste(base.convert("RGBA"), mask=dm)
    # specular highlight
    sp = Image.new("L", (N, N), 0)
    ImageDraw.Draw(sp).ellipse([(455 - 42) * S, (445 - 28) * S, (455 + 42) * S, (445 + 28) * S], fill=90)
    sp = sp.rotate(30, center=(455 * S, 445 * S), resample=Image.BICUBIC)
    img.paste(Image.new("RGBA", (N, N), (255, 255, 255, 255)), mask=sp)
    return img.resize((1024, 1024), Image.LANCZOS)


def build_svg():
    x0, y0 = pt(A0)
    x1, y1 = pt(A1)
    large = 1 if (A1 - A0) > 180 else 0
    return f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">
  <title>SimpleRecord</title>
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#34364a"/><stop offset="1" stop-color="#14151c"/>
    </linearGradient>
    <linearGradient id="ring" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="#b9bccd"/>
    </linearGradient>
    <radialGradient id="dot" cx="0.38" cy="0.32" r="0.8">
      <stop offset="0" stop-color="#ff7a6b"/><stop offset="0.55" stop-color="#ff3b30"/><stop offset="1" stop-color="#c81d14"/>
    </radialGradient>
    <radialGradient id="glow" cx="0.5" cy="0.5" r="0.5">
      <stop offset="0.5" stop-color="#ff3b30" stop-opacity="0.45"/><stop offset="1" stop-color="#ff3b30" stop-opacity="0"/>
    </radialGradient>
  </defs>
  <rect x="100" y="100" width="824" height="824" rx="185" fill="url(#bg)"/>
  <rect x="102" y="102" width="820" height="820" rx="183" fill="none" stroke="#fff" stroke-opacity="0.12" stroke-width="4"/>
  <circle cx="512" cy="512" r="300" fill="url(#glow)"/>
  <path d="M {x0:.1f} {y0:.1f} A {RING_R} {RING_R} 0 {large} 1 {x1:.1f} {y1:.1f}" fill="none" stroke="url(#ring)" stroke-width="{RING_W}" stroke-linecap="round"/>
  <circle cx="512" cy="512" r="152" fill="url(#dot)"/>
  <ellipse cx="455" cy="445" rx="42" ry="28" fill="#fff" fill-opacity="0.35" transform="rotate(-30 455 445)"/>
</svg>
'''


if __name__ == "__main__":
    open(os.path.join(HERE, "logo.svg"), "w").write(build_svg())
    build_png().save(os.path.join(ROOT, "support", "icon.png"))
    print("wrote logo.svg and support/icon.png")
