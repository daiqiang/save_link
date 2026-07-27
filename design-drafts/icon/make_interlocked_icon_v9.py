from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

import make_interlocked_icon_v7_variants as v7


ROOT = Path(__file__).resolve().parent / "gpt-image"
SOURCE = ROOT / "savelink-icon-blue-tile-approved-v8.png"
OUTPUT = ROOT / "savelink-icon-blue-tile-v9-centered.png"
INSPECTION = ROOT / "_inspection-v9-centered.png"
SMALL_PREVIEW = ROOT / "savelink-icon-blue-tile-v9-centered-small-preview.png"

SHIFT_X = -18


def shift_x(array: np.ndarray, amount: int) -> np.ndarray:
    shifted = np.zeros_like(array)
    if amount < 0:
        distance = -amount
        shifted[:, :-distance] = array[:, distance:]
    elif amount > 0:
        shifted[:, amount:] = array[:, :-amount]
    else:
        shifted[:] = array
    return shifted


def composite_shifted_subject(
    background: Image.Image,
    subject: Image.Image,
    alpha: np.ndarray,
) -> Image.Image:
    back = np.asarray(background).astype(np.float32)
    front = shift_x(np.asarray(subject).astype(np.float32), SHIFT_X)
    shifted_alpha = shift_x(alpha, SHIFT_X)
    composed = back * (1.0 - shifted_alpha[:, :, None]) + front * shifted_alpha[:, :, None]
    return Image.fromarray(np.clip(composed, 0, 255).astype(np.uint8), mode="RGB")


def save_inspection(source: Image.Image, result: Image.Image) -> None:
    size = 512
    gap = 24
    canvas = Image.new("RGB", (size * 2 + gap * 3, size + gap * 2), "#f4f6f8")
    for index, image in enumerate((source, result)):
        preview = image.resize((size, size), Image.Resampling.LANCZOS)
        x = gap + index * (size + gap)
        canvas.paste(preview, (x, gap))
        draw = ImageDraw.Draw(canvas)
        center_x = x + size // 2
        draw.line((center_x, gap, center_x, gap + size), fill="#e5484d", width=1)
    canvas.save(INSPECTION, optimize=True)


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
    base = Image.open(v7.BASE_PATH).convert("RGB")
    source = Image.open(SOURCE).convert("RGB")
    pure_tile = v7.make_pure_tile(base)
    subject_alpha = v7.visible_subject_alpha(source)
    result = composite_shifted_subject(pure_tile, source, subject_alpha)

    result.save(OUTPUT, optimize=True)
    save_inspection(source, result)
    save_small_preview(result)


if __name__ == "__main__":
    main()
