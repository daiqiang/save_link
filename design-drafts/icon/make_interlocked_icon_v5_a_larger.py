from pathlib import Path

from PIL import Image, ImageDraw


ROOT = Path(__file__).resolve().parent / "gpt-image"
SOURCE = ROOT / "savelink-icon-blue-tile-approved-v4.png"
BASE = ROOT / "savelink-icon-blue-tile-base.png"
OUTPUT = ROOT / "savelink-icon-blue-tile-v5-a-larger-big-circle.png"
INSPECTION = ROOT / "_inspection-v5-a-larger-big-circle.png"
SMALL_PREVIEW = ROOT / "savelink-icon-blue-tile-v5-a-larger-big-circle-small-preview.png"

# The original A used (500, 420, 160). Moving the center upward while increasing
# the radius keeps the lowest point at y=580 and produces a visibly broader arc.
CIRCLES = [(500, 360, 220), (670, 490, 80)]


def circle_mask(size: tuple[int, int], scale: int = 4) -> Image.Image:
    mask = Image.new("L", (size[0] * scale, size[1] * scale), 0)
    draw = ImageDraw.Draw(mask)
    for center_x, center_y, radius in CIRCLES:
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
    base = Image.open(BASE).convert("RGB")
    if source.size != base.size:
        raise ValueError(f"source and base dimensions differ: {source.size} != {base.size}")

    result = Image.composite(base, source, circle_mask(source.size))
    result.save(OUTPUT, optimize=True)
    result.crop((300, 300, 930, 1015)).save(INSPECTION, optimize=True)
    save_small_preview(result)


if __name__ == "__main__":
    main()
