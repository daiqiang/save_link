from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw


ROOT = Path(__file__).resolve().parent / "gpt-image"
SOURCE = ROOT / "savelink-icon-blue-tile-approved-v5.png"
OUTPUT = ROOT / "savelink-icon-blue-tile-v6-blue-cloud-chain-gap.png"
INSPECTION = ROOT / "_inspection-v6-blue-cloud-chain-gap.png"
SMALL_PREVIEW = ROOT / "savelink-icon-blue-tile-v6-blue-cloud-chain-gap-small-preview.png"

CIRCLES = [(500, 360, 220), (670, 490, 80)]
GAP_WIDTH = 12


def circle_mask(
    size: tuple[int, int],
    radius_expansion: int = 0,
    scale: int = 4,
) -> Image.Image:
    mask = Image.new("L", (size[0] * scale, size[1] * scale), 0)
    draw = ImageDraw.Draw(mask)
    for center_x, center_y, radius in CIRCLES:
        expanded = radius + radius_expansion
        draw.ellipse(
            (
                (center_x - expanded) * scale,
                (center_y - expanded) * scale,
                (center_x + expanded) * scale,
                (center_y + expanded) * scale,
            ),
            fill=255,
        )
    return mask.resize(size, Image.Resampling.LANCZOS)


def blue_by_row(image: np.ndarray) -> np.ndarray:
    rows = np.empty((image.shape[0], 3), dtype=np.float32)
    previous = np.array([34, 108, 224], dtype=np.float32)
    for y, row in enumerate(image):
        blue = row[
            (row[:, 2] > 150)
            & (row[:, 0] < 100)
            & (row[:, 1] > 70)
            & (row[:, 1] < 180)
        ]
        if len(blue):
            previous = np.median(blue, axis=0)
        rows[y] = previous
    return rows


def save_small_preview(image: Image.Image) -> None:
    sizes = [64, 40, 32, 24, 16]
    scale = 4
    gap = 24
    widths = [size * scale for size in sizes]
    canvas = Image.new("RGB", (sum(widths) + gap * 6, max(widths) + gap * 2), "#f4f6f8")
    x = gap
    for size, display_width in zip(sizes, widths):
        icon = image.resize((size, size), Image.Resampling.LANCZOS)
        icon = icon.resize((display_width, display_width), Image.Resampling.NEAREST)
        canvas.paste(icon, (x, gap + max(widths) - display_width))
        x += display_width + gap
    canvas.save(SMALL_PREVIEW, optimize=True)


def main() -> None:
    source = Image.open(SOURCE).convert("RGB")
    pixels = np.asarray(source).copy()

    cloud = np.asarray(circle_mask(source.size), dtype=np.float32) / 255.0
    expanded = np.asarray(
        circle_mask(source.size, radius_expansion=GAP_WIDTH),
        dtype=np.float32,
    ) / 255.0
    outside_cloud_ring = np.clip(expanded - cloud, 0.0, 1.0)

    red = pixels[:, :, 0].astype(np.int16)
    green = pixels[:, :, 1].astype(np.int16)
    blue = pixels[:, :, 2].astype(np.int16)
    yellow = (
        (red > 180)
        & (green > 120)
        & (blue < 180)
        & (red - green > 20)
        & (green - blue > 30)
    )
    gap_alpha = outside_cloud_ring * yellow.astype(np.float32)

    row_colors = np.broadcast_to(blue_by_row(pixels)[:, None, :], pixels.shape)
    composed = (
        pixels.astype(np.float32) * (1.0 - gap_alpha[:, :, None])
        + row_colors * gap_alpha[:, :, None]
    )
    result = Image.fromarray(np.clip(composed, 0, 255).astype(np.uint8), mode="RGB")
    result.save(OUTPUT, optimize=True)
    result.crop((300, 300, 930, 1015)).save(INSPECTION, optimize=True)
    save_small_preview(result)


if __name__ == "__main__":
    main()
