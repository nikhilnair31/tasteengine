#!/usr/bin/env python3
"""Generate Taste Engine brand assets with an OpenRouter image model."""
from __future__ import annotations

import base64
import json
import os
import re
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "frontend" / "assets"
OUT.mkdir(parents=True, exist_ok=True)

PROMPTS = {
    "taste-engine-logo": """Create a square app icon for a personal visual bookmark vault called Taste Engine.
Make a distinctive abstract symbol that suggests a visual taste sensor, a lens, and a small archive mark.
Use one imperfect concentric eye-like ring intersected by a sharp folded diamond notch, with one small plum dot.
Flat graphic design, deep ink-black background #0d0b11, dusty mauve #b79ad3, dark plum #4b315d, pale lilac #eee6f5.
The mark must work at 16 pixels and must have clear negative space.
No words, no letters, no gradients, no glow, no drop shadow, no 3D, no photorealism, no watermark.
Centered symbol, crisp edges, high contrast, square composition.""",
    "taste-engine-placeholder": """Create a square abstract UI texture for a personal visual bookmark vault called Taste Engine.
Show a quiet field of flat ink-black and dark plum with a single large mauve aperture shape and a tiny pale lilac indexing dot.
The image must feel like an editorial print registration mark and a visual memory fragment.
Flat graphic design, restrained geometry, dusty mauve #b79ad3, dark plum #4b315d, pale lilac #eee6f5, ink #0d0b11.
No words, no letters, no gradients, no glow, no drop shadow, no 3D, no photorealism, no watermark.
High contrast, simple forms, centered subject, square composition, readable when reduced to a 120 pixel card.""",
}


def get_key() -> str:
    key = os.environ.get("OPENROUTER_API_KEY", "")
    if key:
        return key
    env_file = Path(__file__).resolve().parents[1] / ".env"
    if env_file.exists():
        match = re.search(r"^OPENROUTER_API_KEY=(.+)$", env_file.read_text(encoding="utf-8"), re.MULTILINE)
        if match:
            return match.group(1).strip().strip('"\'')
    raise RuntimeError("OPENROUTER_API_KEY not found")


def request_image(key: str, model: str, prompt: str) -> bytes:
    payload = json.dumps({"model": model, "prompt": prompt, "n": 1}).encode()
    req = urllib.request.Request(
        "https://openrouter.ai/api/v1/images/generations",
        data=payload,
        headers={
            "Authorization": "Bearer " + key,
            "Content-Type": "application/json",
            "HTTP-Referer": "http://localhost:8770",
            "X-Title": "Taste Engine asset generation",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=120) as response:
            data = json.load(response)
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"image endpoint returned {exc.code}: {detail[:500]}") from exc
    item = (data.get("data") or [{}])[0]
    if item.get("b64_json"):
        return base64.b64decode(item["b64_json"])
    if item.get("url"):
        with urllib.request.urlopen(item["url"], timeout=120) as response:
            return response.read()
    raise RuntimeError("image response had no URL or b64_json")


def main() -> int:
    model = os.environ.get("TASTE_ENGINE_IMAGE_MODEL", "google/gemini-3.1-flash-image")
    key = get_key()
    print(f"model={model}")
    for name, prompt in PROMPTS.items():
        path = OUT / f"{name}.png"
        try:
            data = request_image(key, model, prompt)
            path.write_bytes(data)
            print(f"wrote {path} ({len(data)} bytes)")
        except Exception as exc:
            print(f"failed {name}: {exc}", file=sys.stderr)
            return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
