from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw


ROOT = Path(__file__).resolve().parent / "gpt-image"
SOURCE = ROOT / "savelink-icon-blue-tile-approved-v4.png"
BASE = ROOT / "savelink-icon-blue-tile-base.png"
OUTPUT = ROOT / "savelink-icon-blue-tile-png-interlocked-ai-v5-no-cat-mouth.png"
INSPECTION = ROOT / "_inspection-png-interlocked-ai-v5-no-cat-mouth.png"
SMALL_PREVIEW = ROOT / "savelink-icon-blue-tile-png-interlocked-ai-v5-no-cat-mouth-small-preview.png"


CONTROL_POINTS = [
    (300, 536),
    (350, 543),
    (395, 556),
    (440, 565),
    (482, 568),
    (512, 571),
    (540, 576),
    (568, 580),
    (595, 580),
    (620, 574),
    (640, 568),
    (662, 558),
    (682, 545),
    (706, 538),
    (736, 539),
    (768, 548),
    (800, 556),
    (830, 558),
]


def catmull_rom_curve(points: list[tuple[int, int]], samples: int = 24) -> list[tuple[float, float]]:
    padded = [points[0], *points, points[-1]]
    curve: list[tuple[float, float]] = []
    for index in range(1, len(padded) - 2):
        p0 = np.array(padded[index - 1], dtype=np.float64)
        p1 = np.array(padded[index], dtype=np.float64)
        p2 = np.array(padded[index + 1], dtype=np.float64)
        p3 = np.array(padded[index + 2], dtype=np.float64)
        for step in range(samples):
            t = step / samples
            point = 0.5 * (
                2 * p1
                + (-p0 + p2) * t
                + (2 * p0 - 5 * p1 + 4 * p2 - p3) * t * t
                + (-p0 + 3 * p1 - 3 * p2 + p3) * t * t * t
            )
            curve.append((float(point[0]), float(point[1])))
    curve.append(tuple(map(float, points[-1])))
    return curve


def build_foreground_mask(size: tuple[int, int], scale: int = 4) -> Image.Image:
    width, height = size
    curve = catmull_rom_curve(CONTROL_POINTS)
    scaled_curve = [(round(x * scale), round(y * scale)) for x, y in curve]
    polygon = [
        (0, 0),
        (width * scale, 0),
        (width * scale, round(CONTROL_POINTS[-1][1] * scale)),
        *reversed(scaled_curve),
        (0, round(CONTROL_POINTS[0][1] * scale)),
    ]
    mask = Image.new("L", (width * scale, height * scale), 0)
    ImageDraw.Draw(mask).polygon(polygon, fill=255)
    return mask.resize(size, Image.Resampling.LANCZOS)


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
    base = Image.open(BASE).convert("RGB")
    if source.size != base.size:
        raise ValueError(f"source and base dimensions differ: {source.size} != {base.size}")

    foreground_mask = build_foreground_mask(source.size)
    result = Image.composite(base, source, foreground_mask)
    result.save(OUTPUT, optimize=True)
    result.crop((340, 390, 900, 1015)).save(INSPECTION, optimize=True)
    save_small_preview(result)


if __name__ == "__main__":
    main()
