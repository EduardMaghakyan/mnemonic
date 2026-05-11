#!/usr/bin/env python3
"""Generate two synthetic test PNGs for the image+audio spike.

Run: python3 make-test-image.py
Outputs alongside this script:
  test-stacktrace.png   — terminal-style Rust panic, exercises OCR
  test-mockup.png       — simple UI mockup, exercises captioning
"""

from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).parent

# --- 1. Stack-trace style image (forces text extraction) ---
W, H = 900, 360
img = Image.new("RGB", (W, H), (24, 26, 32))
draw = ImageDraw.Draw(img)

# Try a monospace font; fall back to default if not available.
font = None
for path in [
    "/System/Library/Fonts/Menlo.ttc",
    "/System/Library/Fonts/Monaco.ttf",
    "/Library/Fonts/Andale Mono.ttf",
]:
    if Path(path).exists():
        try:
            font = ImageFont.truetype(path, 18)
            break
        except OSError:
            continue
if font is None:
    font = ImageFont.load_default()

lines = [
    "$ cargo run --release",
    "    Finished release [optimized] target(s) in 0.42s",
    "     Running `target/release/mnemonic-app`",
    "",
    "thread 'main' panicked at 'index out of bounds: the len is 0 but the index is 0',",
    "  src/merge.rs:42:18",
    "note: run with `RUST_BACKTRACE=1` for a backtrace",
    "",
    "$ ",
]
y = 24
for line in lines:
    color = (170, 200, 230) if line.startswith("$") else (235, 235, 235)
    if "panicked" in line or "out of bounds" in line:
        color = (240, 120, 110)
    draw.text((24, y), line, fill=color, font=font)
    y += 28

img.save(HERE / "test-stacktrace.png", optimize=True)
print(f"wrote {HERE / 'test-stacktrace.png'} ({(HERE / 'test-stacktrace.png').stat().st_size} bytes)")

# --- 2. UI-mockup style image (forces captioning, no OCR) ---
W, H = 480, 540
img = Image.new("RGB", (W, H), (245, 245, 248))
draw = ImageDraw.Draw(img)
draw.rectangle([60, 80, W - 60, 130], outline=(180, 180, 190), width=2)
draw.rectangle([60, 160, W - 60, 210], outline=(180, 180, 190), width=2)
draw.rectangle([60, 250, W - 60, 310], fill=(80, 100, 200))
img.save(HERE / "test-mockup.png", optimize=True)
print(f"wrote {HERE / 'test-mockup.png'} ({(HERE / 'test-mockup.png').stat().st_size} bytes)")
