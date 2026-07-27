from pathlib import Path

from PIL import Image

import make_interlocked_icon_v7_variants as v7


ROOT = Path(__file__).resolve().parent / "gpt-image"
OUTPUT = ROOT / "savelink-icon-blue-tile-v8-uniform-blue-gap.png"
INSPECTION = ROOT / "_inspection-v8-uniform-blue-gap.png"
SMALL_PREVIEW = ROOT / "savelink-icon-blue-tile-v8-uniform-blue-gap-small-preview.png"

# Match the existing left diagonal gap in V7 instead of narrowing it to 12 px.
TARGET_GAP_WIDTH = 20


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
    chain = Image.open(v7.V4_PATH).convert("RGB")

    underlay_alpha = v7.chain_underlay_alpha(chain)
    background = v7.paint_blue_underlay(base, underlay_alpha, v7.CHAIN_ASSEMBLY_SHIFT_Y)
    chain_alpha = v7.chain_layer_alpha(chain, base)
    shifted_chain = v7.composite_layer(
        background,
        chain,
        chain_alpha,
        v7.CHAIN_ASSEMBLY_SHIFT_Y,
    )
    circles = [
        (center_x, center_y + v7.CHAIN_ASSEMBLY_SHIFT_Y, radius)
        for center_x, center_y, radius in v7.FRONT_CIRCLES
    ]

    original_gap_width = v7.GAP_WIDTH
    try:
        v7.GAP_WIDTH = TARGET_GAP_WIDTH
        result = v7.apply_cloud_front_and_gap(shifted_chain, base, circles)
    finally:
        v7.GAP_WIDTH = original_gap_width

    result.save(OUTPUT, optimize=True)
    result.crop((300, 300, 930, 1015)).save(INSPECTION, optimize=True)
    save_small_preview(result)


if __name__ == "__main__":
    main()
