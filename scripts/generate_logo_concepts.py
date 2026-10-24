#!/usr/bin/env python3
"""Generate six Taste Engine logo concepts with one locked symbol and varied treatments."""
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
OUT = ROOT / "frontend" / "assets" / "logo-concepts"
OUT.mkdir(parents=True, exist_ok=True)

STYLES = {
    "01-dithered-risograph": """Screen-printed risograph treatment. Use coarse Bayer-style dithering, flat spot inks, slight registration drift, sparse paper grain, and no smooth gradients. Limit the palette to ink black, dusty mauve, and one restrained coral accent. Make it feel like a physical experimental poster.""",
    "03-ambient-bloom": """Dark ambient treatment. Keep the symbol hard and simple, then add a restrained soft bloom around its brightest edges. Use near-black, pale lavender, and icy blue. Create spacious negative space and quiet light. Avoid neon cyberpunk excess.""",
    "04-liquid-chrome": """Liquid chrome treatment. Render the symbol as a warped polished mercury surface with sharp white reflections, graphite black, and deep mauve undertones. Make the material feel physical and premium, but keep the silhouette simple. Do not show a literal object.""",
    "06-smoked-glass-refraction": """Smoked glass treatment. Build the symbol from translucent dark glass planes with subtle refraction, internal highlights, and restrained blue-mauve dispersion. Keep the edges legible and the center calm. Make it feel like a material study, not a gadget.""",
    "07-brutalist-poster-geometry": """Brutalist poster treatment. Use only hard-edged flat geometric blocks, sharp cropping, uneven scale, and strong negative space. Use near-black, bone white, and dusty mauve with one small rust accent. No texture, no lighting effects, and no decoration.""",
    "10-engraved-ink-density": """Engraved ink treatment. Use dense intaglio crosshatching, etched contour lines, uneven ink pressure, and a limited ink-black, bone, and muted mauve palette. Make the mark feel found in an old technical print. Do not introduce gothic subjects or literal illustrations.""",
}

COMMON = """Create one square logo concept for Taste Engine, a personal visual bookmarking app. Use one abstract symbol in every concept: a broken circular aperture crossed by one angular notch, with one small offset dot. Keep the symbol centered, iconic, and readable at 24 pixels. Abstract mark only. Do not add a literal skull, face, person, animal, eye, hand, machine, object, letter, word, typography, interface, mockup, frame, watermark, or extra symbols. Use one symbol on a simple background. """


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
    req = urllib.request.Request(
        "https://openrouter.ai/api/v1/models",
        headers={"Authorization": "Bearer " + key},
    )
    with urllib.request.urlopen(req, timeout=30) as response:
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
            "X-Title": "Taste Engine logo concepts",
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
    for name, treatment in STYLES.items():
        prompt = COMMON + treatment + " Output a clean 1:1 square image."
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
    print(f"created={len(manifest['created'])}/{len(STYLES)}")
    return 0 if len(manifest["created"]) == len(STYLES) else 1


if __name__ == "__main__":
    raise SystemExit(main())
