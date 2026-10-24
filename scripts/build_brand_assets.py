#!/usr/bin/env python3
"""Build clean multi-size Taste Engine assets from a selected geometry."""
from __future__ import annotations

import json
import math
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "frontend" / "assets"
FINAL = ASSETS / "brand-final"
FINAL.mkdir(parents=True, exist_ok=True)

BG = (12, 12, 15, 255)
PAPER = (246, 241, 232, 255)
INK = (7, 7, 9, 255)
MAUVE = (169, 143, 196, 255)
CYAN = (76, 202, 205, 255)
CORAL = (235, 112, 104, 255)


def poly(cx, cy, w, h, skew, tilt):
    pts = [(-w / 2, -h / 2), (w / 2, -h / 2), (w / 2, h / 2), (-w / 2, h / 2)]
    c, s = math.cos(tilt), math.sin(tilt)
    out = []
    for x, y in pts:
        x += skew * y / h
        out.append((cx + x * c - y * s, cy + x * s + y * c))
    return out


def lightning(x1, y1, x2, y2, bend, width):
    dx, dy = x2 - x1, y2 - y1
    nx, ny = -dy, dx
    mag = max(1.0, math.hypot(nx, ny))
    nx, ny = nx / mag, ny / mag
    mx, my = (x1 + x2) / 2, (y1 + y2) / 2
    kx, ky = mx + nx * bend, my + ny * bend
    ux, uy = dx / max(1.0, math.hypot(dx, dy)), dy / max(1.0, math.hypot(dx, dy))
    return [(x1, y1), (mx - ux * 44, my - uy * 44),
            (kx - nx * width, ky - ny * width), (x2, y2),
            (kx + nx * width, ky + ny * width), (mx + ux * 44, my + uy * 44)]


def draw_core(size, favicon=False):
    im = Image.new("RGBA", (size, size), BG)
    d = ImageDraw.Draw(im)
    if favicon:
        # Two page blocks and a large central connector. No texture at this size.
        page_w, page_h = int(size * .31), int(size * .56)
        left = (int(size * .29), int(size * .50))
        right = (int(size * .71), int(size * .50))
        d.polygon(poly(left[0] + 4, left[1] + 4, page_w, page_h, 0, -0.06), fill=MAUVE)
        d.polygon(poly(right[0] - 4, right[1] - 4, page_w, page_h, 0, 0.06), fill=CYAN)
        d.polygon(poly(*left, page_w, page_h, 0, -0.06), fill=PAPER)
        d.polygon(poly(*right, page_w, page_h, 0, 0.06), fill=PAPER)
        d.polygon(lightning(size * .42, size * .50, size * .58, size * .50, 0, size * .08), fill=INK)
        d.polygon(lightning(size * .42, size * .49, size * .58, size * .49, 0, size * .035), fill=PAPER)
        return im

    # Large logo: two saved reference pages and one semantic connection.
    w, h = int(size * .27), int(size * .52)
    y = size * .50
    lx, rx = size * .32, size * .68
    left = poly(lx, y, w, h, 0, -0.06)
    right = poly(rx, y, w, h, 0, 0.06)
    d.polygon([(x + size * .009, yy + size * .009) for x, yy in left], fill=MAUVE)
    d.polygon([(x - size * .009, yy - size * .009) for x, yy in right], fill=CYAN)
    d.polygon(left, fill=PAPER)
    d.polygon(right, fill=PAPER)
    for page, tint in ((left, MAUVE), (right, CORAL)):
        cx = sum(x for x, _ in page) / 4
        cy = sum(yy for _, yy in page) / 4
        inner = poly(cx, cy, w - size * .035, h - size * .035, 0, -0.06 if page is left else 0.06)
        d.polygon(inner, fill=tint)
        # Halftone texture stays sparse and regular.
        step = max(12, int(size * .018))
        for yy in range(int(cy - h * .18), int(cy + h * .20), step):
            for xx in range(int(cx - w * .28), int(cx + w * .28), step):
                if ((xx // step + yy // step) % 3) == 0:
                    r = max(2, int(size * .005))
                    d.ellipse((xx, yy, xx + r, yy + r), fill=INK)
    bolt = lightning(lx + w * .34, y, rx - w * .34, y, 0, size * .035)
    d.polygon(bolt, fill=INK)
    bolt_inner = lightning(lx + w * .34, y - size * .002, rx - w * .34, y - size * .002, 0, size * .014)
    d.polygon(bolt_inner, fill=PAPER)
    return im


def make_placeholder(size=512):
    im = Image.new("RGBA", (size, size), BG)
    d = ImageDraw.Draw(im)
    # Calm repeatable field for missing media, not a second logo.
    for i in range(-size, size * 2, 44):
        d.line((i, 0, i - size, size), fill=(169, 143, 196, 45), width=2)
    for y in range(34, size, 34):
        for x in range(34, size, 34):
            if (x // 34 + y // 34) % 3 == 0:
                d.ellipse((x - 3, y - 3, x + 3, y + 3), fill=(246, 241, 232, 90))
    d.polygon(lightning(size * .28, size * .63, size * .72, size * .37, 0, size * .06), fill=(235, 112, 104, 180))
    return im


def save_sizes(source, stem, sizes):
    for size in sizes:
        out = source.resize((size, size), Image.Resampling.LANCZOS)
        out.save(FINAL / f"{stem}-{size}.png", optimize=True)


def main():
    logo = draw_core(1024, favicon=False)
    favicon = draw_core(1024, favicon=True)
    logo.save(FINAL / "taste-engine-logo.png", optimize=True)
    favicon.save(FINAL / "taste-engine-favicon.png", optimize=True)
    save_sizes(favicon, "taste-engine-favicon", [16, 32, 48, 128, 192, 512])
    save_sizes(logo, "taste-engine-logo", [256, 512])

    banner = Image.new("RGBA", (1800, 600), BG)
    bd = ImageDraw.Draw(banner)
    # Repeat the chosen system across a wide editorial field.
    mark = draw_core(500, favicon=False)
    banner.alpha_composite(mark, (650, 50))
    for x in range(0, 1800, 60):
        bd.line((x, 0, x - 180, 600), fill=(169, 143, 196, 20), width=1)
    banner.save(FINAL / "taste-engine-banner.png", optimize=True)
    make_placeholder().save(FINAL / "taste-engine-placeholder.png", optimize=True)

    meta = {
        "meaning": "two saved visual references connected by a semantic link",
        "source": "procedural geometry selected from 500 scored variants, with Taste Engine reference-driven palette and print treatment",
        "files": sorted(p.name for p in FINAL.iterdir() if p.is_file()),
    }
    (FINAL / "manifest.json").write_text(json.dumps(meta, indent=2), encoding="utf-8")
    print(json.dumps(meta, indent=2))


if __name__ == "__main__":
    main()
