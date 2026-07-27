from pathlib import Path
from collections import deque

import numpy as np
from PIL import Image, ImageDraw, ImageFilter


ROOT = Path(__file__).resolve().parent / "gpt-image"
SOURCE = ROOT / "savelink-icon-blue-tile-png-interlocked-ai-v2-background-aware-gaps.png"
OUTPUT = ROOT / "savelink-icon-blue-tile-png-interlocked-ai-v3-blue-negative-space.png"
INSPECTION = ROOT / "_inspection-png-interlocked-ai-v3-blue-negative-space.png"
SMALL_PREVIEW = ROOT / "savelink-icon-blue-tile-png-interlocked-ai-v3-small-preview.png"


def connected_component(mask: np.ndarray, seed: tuple[int, int]) -> np.ndarray:
    height, width = mask.shape
    result = np.zeros_like(mask)
    queue = deque([seed])
    result[seed[1], seed[0]] = True
    while queue:
        x, y = queue.popleft()
        for nx, ny in ((x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)):
            if (
                0 <= nx < width
                and 0 <= ny < height
                and mask[ny, nx]
                and not result[ny, nx]
            ):
                result[ny, nx] = True
                queue.append((nx, ny))
    return result


def blue_by_row(image: np.ndarray) -> np.ndarray:
    """Return the tile's representative blue for each row."""
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

    white_component = connected_component(
        (pixels[:, :, 0] > 235)
        & (pixels[:, :, 1] > 235)
        & (pixels[:, :, 2] > 235),
        (530, 550),
    )
    component_y, component_x = np.where(white_component)
    print(
        "upper-hole white component:",
        len(component_x),
        (
            int(component_x.min()),
            int(component_y.min()),
            int(component_x.max()),
            int(component_y.max()),
        ),
    )
    # The upper link's hole is a closed white component in the source. Expanding
    # that exact component by a few pixels captures only its antialiased inner edge.
    edit_mask = np.asarray(
        Image.fromarray((white_component * 255).astype(np.uint8), mode="L").filter(
            ImageFilter.MaxFilter(9)
        )
    ) > 0

    # Replace white and light antialiasing in the selected negative space.
    # Blue-channel intensity estimates how much white is mixed into each edge pixel.
    light = (
        (pixels[:, :, 0] > 215)
        & (pixels[:, :, 1] > 175)
        & (pixels[:, :, 2] > 105)
    )
    target = edit_mask & light
    rows = blue_by_row(pixels)
    row_colors = np.broadcast_to(rows[:, None, :], pixels.shape)

    yellow_reference = np.array([255, 190, 60], dtype=np.float32)
    white_mix = np.clip((pixels[:, :, 2].astype(np.float32) - 55.0) / 200.0, 0.0, 1.0)
    replacement = (
        yellow_reference[None, None, :] * (1.0 - white_mix[:, :, None])
        + row_colors * white_mix[:, :, None]
    )
    pixels[target] = np.clip(replacement[target], 0, 255).astype(np.uint8)

    result = Image.fromarray(pixels, mode="RGB")
    result.save(OUTPUT, optimize=True)
    result.crop((380, 400, 900, 1010)).save(INSPECTION, optimize=True)
    save_small_preview(result)


if __name__ == "__main__":
    main()
