"""Export the CLIP text and vision towers of sentence-transformers/clip-ViT-B-32
to ONNX for the Taste Engine backend. Run once; the two .onnx files land in
backend/models/ next to the tokenizer files.

  python3 -m venv .venv && . .venv/bin/activate
  pip install torch transformers sentence-transformers onnxruntime numpy
  python3 backend/scripts/export_clip.py

The tokenizer files (vocab.json, merges.txt) are already in the repo.
"""
import glob
import os

import torch
from transformers import CLIPModel

MODEL = "sentence-transformers/clip-ViT-B-32"
OUT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "models"))
os.makedirs(OUT, exist_ok=True)

# Downloads from the Hugging Face hub on first run, then uses the local cache.
SNAP = glob.glob(os.path.expanduser(
    "~/.cache/huggingface/hub/models--sentence-transformers--clip-ViT-B-32/snapshots/*/0_CLIPModel"))
model = CLIPModel.from_pretrained(SNAP[0] if SNAP else MODEL)
model.eval()

# ---- text tower: CLIPTextModelWithProjection -> text_embeds (512-d) ----
text_model = model.text_model
proj = model.text_projection

class TextTower(torch.nn.Module):
    def __init__(self):
        super().__init__()
        self.text_model = text_model
        self.text_projection = proj

    def forward(self, input_ids):
        out = self.text_model(input_ids=input_ids)
        # sentence-transformers CLIP uses CLIPTextModelWithProjection semantics:
        # pooled output (EOS token embedding) -> projection
        pooled = out.pooler_output if hasattr(out, "pooler_output") else out[1]
        return self.text_projection(pooled)

tower = TextTower()
dummy = torch.ones(1, 77, dtype=torch.long)
path = os.path.join(OUT, "clip_text.onnx")
torch.onnx.export(
    tower, (dummy,), path,
    input_names=["input_ids"], output_names=["text_embeds"],
    dynamic_axes={"input_ids": {0: "batch", 1: "seq"}, "text_embeds": {0: "batch"}},
    opset_version=17, dynamo=False,
)
print("wrote", path)

# ---- vision tower: CLIPVisionModelWithProjection -> image_embeds (512-d) ----
vision_model = model.vision_model
vision_proj = model.visual_projection

class VisionTower(torch.nn.Module):
    def __init__(self):
        super().__init__()
        self.vision_model = vision_model
        self.visual_projection = vision_proj

    def forward(self, pixel_values):
        out = self.vision_model(pixel_values=pixel_values)
        pooled = out.pooler_output if hasattr(out, "pooler_output") else out[1]
        return self.visual_projection(pooled)

vtower = VisionTower()
dummy_px = torch.ones(1, 3, 224, 224)
vpath = os.path.join(OUT, "clip_vision.onnx")
torch.onnx.export(
    vtower, (dummy_px,), vpath,
    input_names=["pixel_values"], output_names=["image_embeds"],
    dynamic_axes={"pixel_values": {0: "batch"}, "image_embeds": {0: "batch"}},
    opset_version=17, dynamo=False,
)
print("wrote", vpath)

# quick numeric check vs sentence_transformers
from sentence_transformers import SentenceTransformer
st = SentenceTransformer(MODEL, device="cpu", cache_folder=None)
texts = ["hello world", "a cozy desk setup with warm light", "__rs_parity_test__ parity test item alpha"]
ref = st.encode(texts, convert_to_numpy=True)
import numpy as np
import onnxruntime as ort
sess = ort.InferenceSession(path, providers=["CPUExecutionProvider"])
tok = st.tokenizer
enc = tok(texts, padding="max_length", max_length=77, truncation=True, return_tensors="np")
ids = enc["input_ids"].astype(np.int64)
got = sess.run(None, {"input_ids": ids})[0]
for t, r, g in zip(texts, ref, got):
    cos = float(np.dot(r, g) / (np.linalg.norm(r) * np.linalg.norm(g)))
    print(f"cos={cos:.7f} maxabs={float(np.max(np.abs(r - g))):.2e} :: {t[:40]}")
