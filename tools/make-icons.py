#!/usr/bin/env python3
"""Fortax Mail's own icon set: python3 tools/make-icons.py

A royal-blue rounded square, a white envelope, and the Fortax drop in its flap — drawn here, not taken
from anywhere. Writes the PNG, SVG and ICO files the build scripts and packaging expect.
"""
import math, os, struct
from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOP, BOTTOM = (78, 117, 242), (35, 64, 184)          # #4E75F2 → #2340B8, the Fortax royal
S = 1024


def bezier(p0, p1, p2, p3, n=48):
    return [tuple((1 - t) ** 3 * p0[i] + 3 * (1 - t) ** 2 * t * p1[i] + 3 * (1 - t) * t ** 2 * p2[i] + t ** 3 * p3[i] for i in range(2))
            for t in (k / n for k in range(n + 1))]


def drop_points(cx, cy, size):
    """The hiSAI drop, centred on (cx, cy) at `size` across."""
    s = size / 100
    pts = (bezier((50, 11), (63, 11), (87, 45), (87, 62)) + bezier((87, 62), (87, 80), (70, 91), (50, 91))
           + bezier((50, 91), (30, 91), (13, 80), (13, 62)) + bezier((13, 62), (13, 45), (37, 11), (50, 11)))
    return [(cx - size / 2 + x * s, cy - size / 2 + y * s) for x, y in pts]


def artwork(size=S, background=True):
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    k = size / 1024
    if background:
        grad = Image.new("RGBA", (size, size))
        px = grad.load()
        for y in range(size):
            f = y / size
            c = tuple(int(TOP[i] + (BOTTOM[i] - TOP[i]) * f) for i in range(3)) + (255,)
            for x in range(size):
                px[x, y] = c
        mask = Image.new("L", (size, size), 0)
        ImageDraw.Draw(mask).rounded_rectangle([0, 0, size - 1, size - 1], radius=int(236 * k), fill=255)
        img.paste(grad, (0, 0), mask)

    d = ImageDraw.Draw(img)
    white = (255, 255, 255, 255)
    # envelope body
    x0, y0, x1, y1 = 176 * k, 268 * k, 848 * k, 756 * k
    d.rounded_rectangle([x0, y0, x1, y1], radius=int(64 * k), fill=white)
    # the fold, cut out of the body so the shape reads as an envelope at any size
    mid = (x0 + x1) / 2
    fold = [(x0 + 30 * k, y0 + 62 * k), (mid, y0 + 300 * k), (x1 - 30 * k, y0 + 62 * k)]
    d.line(fold, fill=(BOTTOM if background else TOP) + (255,), width=int(34 * k), joint="curve")
    # the drop, sitting in the flap
    pts = drop_points(mid, y0 + 108 * k, 168 * k)
    d.polygon(pts, fill=(TOP if background else BOTTOM) + (255,))
    eye = 17 * k
    for dx in (-30 * k, 30 * k):
        d.rounded_rectangle([mid + dx - eye / 2, y0 + 92 * k, mid + dx + eye / 2, y0 + 92 * k + 30 * k], radius=eye / 2, fill=white)
    return img


def ico(img, path, sizes=(16, 24, 32, 48, 64, 128, 256)):
    img.save(path, sizes=[(s, s) for s in sizes])


def main():
    icons = os.path.join(ROOT, "resources", "app-icon")
    os.makedirs(icons, exist_ok=True)
    masked = artwork(S, True)
    plain = artwork(S, False)
    masked.save(os.path.join(icons, "fortax-mail-masked.png"))
    masked.resize((512, 512), Image.LANCZOS).save(os.path.join(icons, "fortax-mail-masked-512.png"))
    plain.save(os.path.join(icons, "fortax-mail-nonmasked.png"))
    ico(masked, os.path.join(icons, "fortax-mail.ico"))
    ico(masked, os.path.join(ROOT, "platform", "windows", "fortax-mail.ico"))

    svg = svg_icon(True)
    open(os.path.join(icons, "fortax-mail-masked.svg"), "w").write(svg)
    open(os.path.join(icons, "fortax-mail-nonmasked.svg"), "w").write(svg_icon(False))
    brand = os.path.join(ROOT, "ui", "icons", "brand")
    os.makedirs(brand, exist_ok=True)
    open(os.path.join(brand, "fortax-mail.svg"), "w").write(svg)
    print("icons written to", icons)


def svg_icon(background=True):
    body = "M50 11C63 11 87 45 87 62C87 80 70 91 50 91C30 91 13 80 13 62C13 45 37 11 50 11Z"
    bg = ('<rect x="0" y="0" width="1024" height="1024" rx="236" fill="url(#g)"/>' if background else '')
    line = "#2340B8" if background else "#4E75F2"
    dropfill = "#4E75F2" if background else "#2340B8"
    return f'''<svg width="1024" height="1024" viewBox="0 0 1024 1024" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <linearGradient id="g" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#4E75F2"/><stop offset="1" stop-color="#2340B8"/>
    </linearGradient>
  </defs>
  {bg}
  <rect x="176" y="268" width="672" height="488" rx="64" fill="#fff"/>
  <path d="M206 330 L512 568 L818 330" fill="none" stroke="{line}" stroke-width="34" stroke-linecap="round" stroke-linejoin="round"/>
  <g transform="translate(428 292) scale(1.68)">
    <path d="{body}" fill="{dropfill}"/>
    <rect x="35.5" y="44" width="9" height="16" rx="4.5" fill="#fff"/>
    <rect x="55.5" y="44" width="9" height="16" rx="4.5" fill="#fff"/>
  </g>
</svg>
'''


if __name__ == "__main__":
    main()
