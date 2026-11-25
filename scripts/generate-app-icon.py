# /// script
# requires-python = ">=3.11"
# dependencies = ["resvg-py==0.5.0", "pillow==13.0.0"]
# ///
"""Render the editable SVG: uv run scripts/generate-app-icon.py."""
from io import BytesIO
from pathlib import Path
from PIL import Image
import resvg_py

root = Path(__file__).resolve().parent.parent
assets = root / "assets"
svg = (assets / "PowerDock.svg").read_text(encoding="utf-8")
sizes = [(size, size) for size in (16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 128, 256)]
images = []
for size, _ in sizes:
    # Rasterize each DPI size from the vector instead of shrinking the 256px frame.
    image = Image.open(BytesIO(resvg_py.svg_to_bytes(
        svg_string=svg, width=size, height=size,
    ))).convert("RGBA")
    images.append(image)
images[-1].save(assets / "PowerDock.png")
images[-1].save(assets / "PowerDock.ico", format="ICO", sizes=sizes, append_images=images[:-1])
with Image.open(assets / "PowerDock.ico") as icon:
    assert icon.ico.sizes() == set(sizes)
    for frame in images:
        assert icon.ico.getimage(frame.size).tobytes() == frame.tobytes()
        assert frame.getpixel((0, 0))[3] == 0
print(f"Generated PowerDock.ico with {len(sizes)} native sizes.")
