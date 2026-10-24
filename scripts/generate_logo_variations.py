#!/usr/bin/env python3
"""Generate Taste Engine logo variations with app-relevant shapes."""
from __future__ import annotations

import base64
import json
import os
import re
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "frontend" / "assets" / "logo-variations"
OUT.mkdir(parents=True, exist_ok=True)

STYLES = {
    "dithered-comic-print": """Use angular comic-book halftone and risograph print behavior. Use coarse dot fields, hard ink shapes, slight cyan and coral registration offsets, dusty mauve, and a near-black base. Keep the mark flat and graphic. Make it feel like an experimental printed poster, not a character illustration and not a smooth vector logo.""",
    "stylized-chrome-foil": """Use stylized chrome foil and reflective ink, not photorealistic 3D. Give the mark hard graphic edges with controlled silver reflections, graphite, deep mauve, and one pale highlight. Keep the material treatment on a flat emblem. No studio product shot, perspective floor, rounded app tile, or soft plastic.""",
    "smoked-glass-backed": """Use smoked translucent glass with bright etched edges, but place every glass part over a solid near-black matte backing shape. Use deep blue-black, mauve, and small icy highlights. Keep the mark legible and graphic. The transparency must support the shape, not erase it. No gray studio background, floor, shadow, or floating product render.""",
    "brutalist-reference-system": """Use hard-edged editorial geometry with asymmetrical cropping, flat blocks, a strict grid, and one selected accent tile. Use near-black, bone white, dusty mauve, and one rust accent. Make the mark feel like a visual catalog system. No sword, lightning bolt, target, circle, dot, or generic abstract logo.""",
}

SHAPES = {
    "reference-grid": """Shape: a compact 2-by-2 contact sheet of four unequal rectangular image tiles. Pull one tile partly out of the grid and connect it with one offset corner. The mark must communicate saved visual references. Use rectangles and clipped corners only. Do not use circles or dots.""",
    "reference-stack": """Shape: three overlapping rectangular reference cards with clipped corners. Pull the front card forward with one strong offset edge and one small tab. The mark must communicate a saved collection. Use rectangles and tabs only. Do not use circles or dots.""",
    "similarity-path": """Shape: three angled rectangular image panes connected by one thick continuous angular path. The path must pass through the panes like a visual similarity trail. The mark must communicate finding related references. Use rectangles, sharp bends, and one continuous path only. Do not use circles or dots.""",
}

COMMON = """Create one square logo concept for Taste Engine, a personal visual reference library that saves images and finds related images by vibe, style, and color. Create one isolated abstract brand mark centered on a simple background. The mark must work at 24 pixels and must remain recognizable in a browser header and an app icon. Do not add text, letters, words, typography, people, faces, skulls, animals, eyes, hands, machines, literal objects, interface mockups, borders, watermarks, or extra symbols. Do not use the old broken circular aperture and dot symbol. """


def get_key() -> str:
    key = os.environ.get("OPENROUTER_API_KEY", "")
    if not key:
        env_file = Path(__file__).resolve().parents[1] / ".env"
        if env_file.exists():
            match = re.search(r"^OPENROUTER_API_KEY=(.+)$", env_file.read_text(encoding="utf-8"), re.MULTILINE)
            if match:
                key = match.group(1).strip().strip('"\'')
    if not key:
        raise RuntimeError("OPENROUTER_API_KEY not found")
    return key


def discover_model(key: str) -> str:
    request = urllib.request.Request(
        "https://openrouter.ai/api/v1/models",
        headers={"Authorization": "Bearer " + key},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        data = json.load(response)
    available = []
    for item in data.get("data", []):
        architecture = item.get("architecture") or {}
        if "image" in (architecture.get("output_modalities") or []):
            model_id = item.get("id", "")
            if model_id not in {"openrouter/auto", "openrouter/auto-beta"}:
                available.append(model_id)
    for preferred in (
        "google/gemini-3.1-flash-image",
        "google/gemini-2.5-flash-image",
        "openai/gpt-5-image-mini",
    ):
        if preferred in available:
            return preferred
    if not available:
        raise RuntimeError("no image-output model in catalog")
    return available[0]


def decode_image(data: dict) -> bytes:
    item = (data.get("data") or [{}])[0]
    if item.get("b64_json"):
        return base64.b64decode(item["b64_json"])
    if item.get("url"):
        with urllib.request.urlopen(item["url"], timeout=120) as response:
            return response.read()
    raise RuntimeError("image response had no URL or b64_json")


def generate(key: str, model: str, prompt: str) -> bytes:
    payload = json.dumps({"model": model, "prompt": prompt, "n": 1}).encode()
    request = urllib.request.Request(
        "https://openrouter.ai/api/v1/images/generations",
        data=payload,
        headers={
            "Authorization": "Bearer " + key,
            "Content-Type": "application/json",
            "HTTP-Referer": "http://localhost:8770",
            "X-Title": "Taste Engine logo variations",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=180) as response:
            return decode_image(json.load(response))
    except urllib.error.HTTPError as first_error:
        detail = first_error.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"image endpoint returned {first_error.code}: {detail[:300]}") from first_error


def main() -> int:
    key = get_key()
    model = discover_model(key)
    manifest = {"model": model, "created": [], "prompts": {}}
    print(f"model={model}")
    for style_name, style in STYLES.items():
        for shape_name, shape in SHAPES.items():
            name = f"{style_name}__{shape_name}"
            prompt = COMMON + style + " " + shape + " Output a clean 1:1 square image."
            path = OUT / (name + ".png")
            manifest["prompts"][name] = prompt
            try:
                image = generate(key, model, prompt)
                if len(image) < 1000:
                    raise RuntimeError("image output was too small")
                path.write_bytes(image)
                manifest["created"].append({"name": name, "path": str(path), "bytes": len(image)})
                print(f"ok {name} {len(image)} bytes")
            except Exception as error:
                print(f"failed {name}: {error}")
                manifest.setdefault("errors", []).append({"name": name, "error": str(error)})
            time.sleep(1)
    (OUT / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    expected = len(STYLES) * len(SHAPES)
    print(f"created={len(manifest['created'])}/{expected}")
    return 0 if len(manifest["created"]) == expected else 1


if __name__ == "__main__":
    raise SystemExit(main())
