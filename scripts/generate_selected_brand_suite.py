#!/usr/bin/env python3
"""Generate selected Taste Engine lightning-between-pages brand assets."""
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
OUT = ROOT / "frontend" / "assets" / "selected-brand"
OUT.mkdir(parents=True, exist_ok=True)

VARIATIONS = {
    "a-straight-bridge": "Three overlapping page panels form a compact triangle. A single thick angular lightning path travels from the left page through the center and into the right page. Keep the path centered and leave large clean gaps around it.",
    "b-split-flash": "Three overlapping page panels form a compact triangle. A single lightning path splits into two short sharp branches between the pages, like one visual reference suddenly becoming two related references. Keep one connected path and no extra symbols.",
    "c-wide-zigzag": "Three page panels run from left to right. A single wide lightning path zigzags across all three panels, with three deliberate bends. Keep the mark balanced and readable as a compact emblem.",
    "d-folded-corner": "Three page panels overlap with clipped corners. A single lightning path enters through one page corner, folds through the middle page, and exits through the last page. Make the page corners and path share one visual language.",
    "e-negative-space-flash": "Three page panels create a strong dark silhouette. The lightning path appears as a clean negative-space cut through the panels, with only small mauve and cyan registration offsets around the cut.",
    "f-single-page-link": "Two large page panels sit apart with a narrow central gap. One bold lightning path bridges the gap and touches both pages. Use only two pages so the mark stays highly readable at tiny sizes.",
}

BASE = """Create a square brand mark for Taste Engine, a personal visual reference library that saves images and finds related images by vibe, style, and color. The mark must show saved pages connected by a lightning-like path. Use exactly three abstract rectangular page panels unless the prompt says two. Each page has a dark near-black border and a simple halftone image field. Use an ink-black base, dusty mauve, muted cyan, and restrained coral. Use bold screen-print geometry, coarse comic halftone dots, slight cyan and coral registration offsets, and tactile riso grain. Keep the mark flat and graphic, not photorealistic. Do not add any text, letters, words, typography, people, faces, skulls, animals, eyes, machines, magnifying glasses, targets, circles, dots, stars, arrows, interface mockups, app tiles, rounded square frames, drop shadows, gradients, smooth 3D rendering, or watermark. The lightning path must connect the pages and must not look like a letter. """

BANNER = """Create a wide 3:1 banner for Taste Engine, a personal visual reference library that saves images and finds related images by vibe, style, and color. Place three abstract saved page panels across the center-left and center-right, connected by one bold lightning path. Leave generous dark negative space around the mark. Use an ink-black and deep blue-black background with dusty mauve, muted cyan, restrained coral, and pale paper highlights. Use bold screen-print geometry, coarse comic halftone dots, slight cyan and coral registration offsets, and tactile riso grain. Keep the banner graphic and editorial. Do not add any text, letters, words, typography, people, faces, skulls, animals, eyes, machines, circles, dots, stars, arrows, interface mockups, gradients, photorealistic 3D, drop shadows, or watermark. """

LOGO = """Create a square final logo for Taste Engine, a personal visual reference library that saves images and finds related images by vibe, style, and color. Show three abstract rectangular saved pages connected by one thick lightning-like path. The path must be the main gesture. Use a near-black background, black page borders, dusty mauve, muted cyan, restrained coral, and pale paper highlights. Use coarse comic halftone dots, slight cyan and coral registration offsets, and tactile riso grain. Make the mark bold, high contrast, and readable at 24 pixels. Keep it flat and graphic, not photorealistic. Do not add any text, letters, words, typography, people, faces, skulls, animals, eyes, machines, magnifying glasses, targets, circles, dots, stars, arrows, interface mockups, rounded square frames, gradients, drop shadows, or watermark. """

FAVICON = """Create a high-contrast square favicon for Taste Engine, a personal visual reference library that saves images and finds related images. Show two abstract black page panels with one thick white lightning path bridging them. Use a solid near-black background with one small dusty mauve registration offset. Use only hard flat shapes with very sparse halftone texture. The mark must remain clear at 16 pixels. Do not add text, letters, words, circles, dots, stars, arrows, people, objects, gradients, shadows, rounded app tiles, or watermark. """


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


def generate(key: str, model: str, prompt: str, aspect: str = "1:1") -> bytes:
    full_prompt = prompt + f" Output a clean {aspect} image."
    payload = json.dumps({"model": model, "prompt": full_prompt, "n": 1}).encode()
    request = urllib.request.Request(
        "https://openrouter.ai/api/v1/images/generations",
        data=payload,
        headers={
            "Authorization": "Bearer " + key,
            "Content-Type": "application/json",
            "HTTP-Referer": "http://localhost:8770",
            "X-Title": "Taste Engine selected brand suite",
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
    jobs = []
    for name, variation in VARIATIONS.items():
        jobs.append((f"variation-{name}", BASE + variation, "1:1"))
    jobs.extend([
        ("final-logo", LOGO, "1:1"),
        ("final-favicon", FAVICON, "1:1"),
        ("final-banner", BANNER, "3:1"),
    ])
    print(f"model={model}")
    for name, prompt, aspect in jobs:
        path = OUT / (name + ".png")
        manifest["prompts"][name] = prompt
        try:
            image = generate(key, model, prompt, aspect)
            if len(image) < 1000:
                raise RuntimeError("image output was too small")
            path.write_bytes(image)
            manifest["created"].append({"name": name, "path": str(path), "bytes": len(image), "aspect": aspect})
            print(f"ok {name} {len(image)} bytes")
        except Exception as error:
            print(f"failed {name}: {error}")
            manifest.setdefault("errors", []).append({"name": name, "error": str(error)})
        time.sleep(1)
    (OUT / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    expected = len(jobs)
    print(f"created={len(manifest['created'])}/{expected}")
    return 0 if len(manifest["created"]) == expected else 1


if __name__ == "__main__":
    raise SystemExit(main())
น给主人留下些什么吧  