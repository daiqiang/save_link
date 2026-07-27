from pathlib import Path

from PIL import Image, ImageDraw


ROOT = Path(__file__).resolve().parent / "gpt-image"
SOURCE = ROOT / "savelink-icon-blue-tile-approved-v4.png"
BASE = ROOT / "savelink-icon-blue-tile-base.png"

VARIANTS = {
    "big-small": {
        "circles": [(500, 420, 160), (670, 490, 80)],
        "output": ROOT / "savelink-icon-blue-tile-v5-big-small-circles.png",
        "inspection": ROOT / "_inspection-v5-big-small-circles.png",
    },
    "big-two-small": {
        "circles": [(520, 400, 170), (400, 495, 72), (685, 490, 80)],
        "output": ROOT / "savelink-icon-blue-tile-v5-big-two-small-circles.png",
        "inspection": ROOT / "_inspection-v5-big-two-small-circles.png",
    },
}

COMPARISON = ROOT / "savelink-icon-blue-tile-v5-circle-variants-comparison.png"
SMALL_COMPARISON = ROOT / "savelink-icon-blue-tile-v5-circle-variants-small-preview.png"


def circle_mask(size: tuple[int, int], circles: list[tuple[int, int, int]], scale: int = 4) -> Image.Image:
    mask = Image.new("L", (size[0] * scale, size[1] * scale), 0)
    draw = ImageDraw.Draw(mask)
    for center_x, center_y, radius in circles:
        draw.ellipse(
            (
                (center_x - radius) * scale,
                (center_y - radius) * scale,
                (center_x + radius) * scale,
                (center_y + radius) * scale,
            ),
            fill=255,
        )
    return mask.resize(size, Image.Resampling.LANCZOS)


def make_comparison(images: list[Image.Image]) -> None:
    display_size = 512
    gap = 24
    canvas = Image.new("RGB", (display_size * 2 + gap * 3, display_size + gap * 2), "#f4f6f8")
    for index, image in enumerate(images):
        preview = image.resize((display_size, display_size), Image.Resampling.LANCZOS)
        canvas.paste(preview, (gap + index * (display_size + gap), gap))
    canvas.save(COMPARISON, optimize=True)

    sizes = [64, 40, 32, 24, 16]
    scale = 4
    row_height = 64 * scale + 20
    row_width = sum(size * scale for size in sizes) + gap * (len(sizes) + 1)
    small = Image.new("RGB", (row_width, row_height * 2 + gap), "#f4f6f8")
    for row, image in enumerate(images):
        x = gap
        for size in sizes:
            icon = image.resize((size, size), Image.Resampling.LANCZOS)
            icon = icon.resize((size * scale, size * scale), Image.Resampling.NEAREST)
            y = row * (row_height + gap) + (64 - size) * scale
            small.paste(icon, (x, y))
            x += size * scale + gap
    small.save(SMALL_COMPARISON, optimize=True)


def main() -> None:
    source = Image.open(SOURCE).convert("RGB")
    base = Image.open(BASE).convert("RGB")
    if source.size != base.size:
        raise ValueError(f"source and base dimensions differ: {source.size} != {base.size}")

    results = []
    for variant in VARIANTS.values():
        mask = circle_mask(source.size, variant["circles"])
        result = Image.composite(base, source, mask)
        result.save(variant["output"], optimize=True)
        result.crop((330, 340, 930, 1015)).save(variant["inspection"], optimize=True)
        results.append(result)

    make_comparison(results)


if __name__ == "__main__":
    main()
