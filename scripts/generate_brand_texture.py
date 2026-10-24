#!/usr/bin/env python3
"""Generate one abstract print texture plate for Taste Engine branding."""
from __future__ import annotations

import base64
import json
import os
import re
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "frontend" / "assets" / "brand-final"
OUT.mkdir(parents=True, exist_ok=True)

PROMPT = """Create a square abstract texture plate for the visual identity of Taste Engine, a personal visual reference library that saves images and finds related images by vibe, style, and color. Make a flat editorial screen-print field with coarse comic halftone dots, irregular ink blocks, slight cyan and coral registration offsets, dusty mauve, pale paper, and ink black. Use abstract cropped geometry only. This is texture for the inside of two blank reference cards, not a logo. No text, no letters, no words, no numbers, no people, no faces, no skulls, no animals, no machines, no icons, no arrows, no lightning, no circles, no gradients, no glow, no 3D, no photorealism, no watermark, no border. Keep the shapes bold and sparse so the texture stays readable when reduced."""


def get_key():
    key = os.environ.get("OPENROUTER_API_KEY", "")
    if not key:
        env_file = Path(__file__).resolve().parents[1] / ".env"
        if env_file.exists():
            m = re.search(r"^OPENROUTER_API_KEY=(.+)$", env_file.read_text(encoding="utf-8"), re.MULTILINE)
            if m:
                key = m.group(1).strip().strip("\"'")
    if not key:
        raise RuntimeError("OPENROUTER_API_KEY not found")
    return key


def discover_model(key):
    req = urllib.request.Request("https://openrouter.ai/api/v1/models", headers={"Authorization": "Bearer " + key})
    with urllib.request.urlopen(req, timeout=30) as response:
        data = json.load(response)
    ids = []
    for item in data.get("data", []):
        if "image" in ((item.get("architecture") or {}).get("output_modalities") or []):
            model_id = item.get("id", "")
            if model_id not in {"openrouter/auto", "openrouter/auto-beta"}:
                ids.append(model_id)
    for preferred in ("google/gemini-3.1-flash-image", "google/gemini-2.5-flash-image", "openai/gpt-5-image-mini"):
        if preferred in ids:
            return preferred
    if not ids:
        raise RuntimeError("no image model in catalog")
    return ids[0]


def decode(data):
    item = (data.get("data") or [{}])[0]
    if item.get("b64_json"):
        return base64.b64decode(item["b64_json"])
    if item.get("url"):
        with urllib.request.urlopen(item["url"], timeout=120) as response:
            return response.read()
    raise RuntimeError("image response had no image")


def image_generation(key, model):
    body = json.dumps({"model": model, "prompt": PROMPT, "n": 1}).encode()
    req = urllib.request.Request(
        "https://openrouter.ai/api/v1/images/generations",
        data=body,
        method="POST",
        headers={
            "Authorization": "Bearer " + key,
            "Content-Type": "application/json",
            "HTTP-Referer": "http://localhost:8770",
            "X-Title": "Taste Engine brand texture",
        },
    )
    with urllib.request.urlopen(req, timeout=180) as response:
        return decode(json.load(response))


def chat_fallback(key, model):
    body = json.dumps({
        "model": model,
        "messages": [{"role": "user", "content": PROMPT}],
        "modalities": ["text", "image"],
    }).encode()
    req = urllib.request.Request(
        "https://openrouter.ai/api/v1/chat/completions",
        data=body,
        method="POST",
        headers={"Authorization": "Bearer " + key, "Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=180) as response:
        data = json.load(response)
    content = ((data.get("choices") or [{}])[0].get("message") or {}).get("content") or []
    if isinstance(content, str):
        raise RuntimeError("chat response contained no image part")
    for part in content:
        image = part.get("image_url") or {}
        value = image.get("url", "")
        if value.startswith("data:image"):
            return base64.b64decode(value.split(",", 1)[1])
        if value:
            with urllib.request.urlopen(value, timeout=120) as response:
                return response.read()
    raise RuntimeError("chat response contained no image")


def main():
    key = get_key()
    model = discover_model(key)
    try:
        raw = image_generation(key, model)
    except urllib.error.HTTPError:
        raw = chat_fallback(key, model)
    path = OUT / "taste-engine-texture-source.png"
    path.write_bytes(raw)
    (OUT / "texture-manifest.json").write_text(json.dumps({"model": model, "prompt": PROMPT, "bytes": len(raw)}, indent=2), encoding="utf-8")
    print(f"model={model}")
    print(f"wrote {path} ({len(raw)} bytes)")


if __name__ == "__main__":
    raise SystemExit(main())