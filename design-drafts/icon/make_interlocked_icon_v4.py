from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter


ROOT = Path(__file__).resolve().parent / "gpt-image"
SOURCE = ROOT / "savelink-icon-blue-tile-approved-v3.png"
OUTPUT = ROOT / "savelink-icon-blue-tile-png-interlocked-ai-v4-blue-surround.png"
INSPECTION = ROOT / "_inspection-png-interlocked-ai-v4-blue-surround.png"
SMALL_PREVIEW = ROOT / "savelink-icon-blue-tile-png-interlocked-ai-v4-small-preview.png"


def blue_by_row(image: np.ndarray) -> np.ndarray:
    height = image.shape[0]
    rows = np.empty((height, 3), dtype=np.float32)
    previous = np.array([34, 108, 224], dtype=np.float32)

    for y in range(height):
        row = image[y]
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
    labels_height = 34
    widths = [size * scale for size in sizes]
    canvas = Image.new(
        "RGB",
        (sum(widths) + gap * (len(sizes) + 1), max(widths) + labels_height + gap * 2),
        "#f4f6f8",
    )
    draw = ImageDraw.Draw(canvas)
    x = gap
    for size, display_width in zip(sizes, widths):
        icon = image.resize((size, size), Image.Resampling.LANCZOS)
        icon = icon.resize((display_width, display_width), Image.Resampling.NEAREST)
        y = gap + max(widths) - display_width
        canvas.paste(icon, (x, y))
        draw.text((x, gap + max(widths) + 8), f"{size}px", fill="#30343b")
        x += display_width + gap
    canvas.save(SMALL_PREVIEW, optimize=True)


def main() -> None:
    source = Image.open(SOURCE).convert("RGB")
    pixels = np.asarray(source).copy()

    yellow_core = (
        (pixels[:, :, 0] > 220)
        & (pixels[:, :, 1] > 135)
        & (pixels[:, :, 1] < 235)
        & (pixels[:, :, 2] < 145)
    )
    core_mask = Image.fromarray((yellow_core * 255).astype(np.uint8), mode="L")

    # A blurred chain mask creates a rounded, antialiased blue surround. The
    # thresholds correspond to roughly 15 px of visible separation at 1024 px.
    blurred = np.asarray(core_mask.filter(ImageFilter.GaussianBlur(11))).astype(np.float32)
    surround_alpha = np.clip((blurred - 14.0) / 12.0, 0.0, 1.0)

    # Only white and yellow/white antialiasing need recoloring. Existing blue is
    # deliberately left untouched, and solid yellow remains the original artwork.
    candidate = (
        (pixels[:, :, 0] > 190)
        & (pixels[:, :, 2] > 90)
        & (
            pixels[:, :, 1].astype(np.int16)
            - pixels[:, :, 2].astype(np.int16)
            < 105
        )
    )
    alpha = surround_alpha * candidate.astype(np.float32)

    rows = blue_by_row(pixels)
    row_colors = np.broadcast_to(rows[:, None, :], pixels.shape)
    yellow_reference = np.array([255, 190, 60], dtype=np.float32)
    white_mix = np.clip((pixels[:, :, 2].astype(np.float32) - 55.0) / 200.0, 0.0, 1.0)
    replacement = (
        yellow_reference[None, None, :] * (1.0 - white_mix[:, :, None])
        + row_colors * white_mix[:, :, None]
    )

    composed = (
        pixels.astype(np.float32) * (1.0 - alpha[:, :, None])
        + replacement * alpha[:, :, None]
    )
    result = Image.fromarray(np.clip(composed, 0, 255).astype(np.uint8), mode="RGB")
    result.save(OUTPUT, optimize=True)
    result.crop((370, 390, 910, 1015)).save(INSPECTION, optimize=True)
    save_small_preview(result)


if __name__ == "__main__":
    main()
