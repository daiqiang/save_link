from collections import deque
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter


ROOT = Path(__file__).resolve().parent / "gpt-image"
BASE_PATH = ROOT / "savelink-icon-blue-tile-base.png"
V4_PATH = ROOT / "savelink-icon-blue-tile-approved-v4.png"
V6_PATH = ROOT / "savelink-icon-blue-tile-approved-v6.png"

WHOLE_GROUP_OUTPUT = ROOT / "savelink-icon-blue-tile-v7-a-whole-group-up.png"
WHOLE_GROUP_INSPECTION = ROOT / "_inspection-v7-a-whole-group-up.png"
CHAIN_ONLY_OUTPUT = ROOT / "savelink-icon-blue-tile-v7-b-chain-assembly-up.png"
CHAIN_ONLY_INSPECTION = ROOT / "_inspection-v7-b-chain-assembly-up.png"
SMALL_COMPARISON = ROOT / "savelink-icon-blue-tile-v7-variants-small-preview.png"

WHOLE_GROUP_SHIFT_Y = -50
CHAIN_ASSEMBLY_SHIFT_Y = -75
FRONT_CIRCLES = [(500, 360, 220), (670, 490, 80)]
GAP_WIDTH = 12


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


def make_pure_tile(base: Image.Image) -> Image.Image:
    pixels = np.asarray(base).copy()
    white = (
        (pixels[:, :, 0] > 235)
        & (pixels[:, :, 1] > 235)
        & (pixels[:, :, 2] > 235)
    )
    cloud = connected_component(white, (500, 300))
    cloud = np.asarray(
        Image.fromarray((cloud * 255).astype(np.uint8), mode="L").filter(
            ImageFilter.MaxFilter(11)
        )
    ) > 0
    rows = np.broadcast_to(blue_by_row(pixels)[:, None, :], pixels.shape)
    pixels[cloud] = rows[cloud].astype(np.uint8)
    return Image.fromarray(pixels, mode="RGB")


def extract_alpha(
    foreground: Image.Image,
    background: Image.Image,
    bbox: tuple[int, int, int, int] | None = None,
) -> np.ndarray:
    front = np.asarray(foreground).astype(np.int16)
    back = np.asarray(background).astype(np.int16)
    difference = np.max(np.abs(front - back), axis=2).astype(np.float32)
    alpha = np.clip((difference - 2.0) / 24.0, 0.0, 1.0)
    if bbox is not None:
        limited = np.zeros_like(alpha)
        left, top, right, bottom = bbox
        limited[top:bottom, left:right] = alpha[top:bottom, left:right]
        alpha = limited
    return alpha


def visible_subject_alpha(image: Image.Image) -> np.ndarray:
    pixels = np.asarray(image)
    rows = np.broadcast_to(blue_by_row(pixels)[:, None, :], pixels.shape)
    red = pixels[:, :, 0].astype(np.int16)
    green = pixels[:, :, 1].astype(np.int16)
    blue = pixels[:, :, 2].astype(np.int16)

    white = (red > 235) & (green > 235) & (blue > 235)
    cloud = connected_component(white, (500, 300))
    cloud_near = np.asarray(
        Image.fromarray((cloud * 255).astype(np.uint8), mode="L").filter(
            ImageFilter.MaxFilter(9)
        )
    ) > 0
    white_alpha = np.clip(
        (red.astype(np.float32) - rows[:, :, 0])
        / np.maximum(1.0, 255.0 - rows[:, :, 0]),
        0.0,
        1.0,
    )

    yellow = (
        (red > 180)
        & (green > 120)
        & (blue < 180)
        & (red - green > 20)
        & (green - blue > 30)
    )
    yellow_near = np.asarray(
        Image.fromarray((yellow * 255).astype(np.uint8), mode="L").filter(
            ImageFilter.MaxFilter(9)
        )
    ) > 0
    yellow_alpha = np.clip(
        (red.astype(np.float32) - rows[:, :, 0])
        / np.maximum(1.0, 255.0 - rows[:, :, 0]),
        0.0,
        1.0,
    )

    return np.maximum(
        white_alpha * cloud_near.astype(np.float32),
        yellow_alpha * yellow_near.astype(np.float32),
    )


def chain_layer_alpha(chain: Image.Image, base: Image.Image) -> np.ndarray:
    pixels = np.asarray(chain)
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
    yellow_near = np.asarray(
        Image.fromarray((yellow * 255).astype(np.uint8), mode="L").filter(
            ImageFilter.MaxFilter(13)
        )
    ).astype(np.float32) / 255.0

    yellow_like = (
        (red > 140)
        & (green > 95)
        & (red - green > 10)
        & (green - blue > 18)
    )
    difference = extract_alpha(chain, base, bbox=(340, 350, 920, 1015))
    yellow_alpha = difference * yellow_near * yellow_like.astype(np.float32)
    return yellow_alpha


def chain_underlay_alpha(chain: Image.Image) -> np.ndarray:
    pixels = np.asarray(chain)
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
    blurred = np.asarray(
        Image.fromarray((yellow * 255).astype(np.uint8), mode="L").filter(
            ImageFilter.GaussianBlur(10)
        )
    ).astype(np.float32)
    solid = Image.fromarray(((blurred > 5.0) * 255).astype(np.uint8), mode="L")
    draw = ImageDraw.Draw(solid)
    draw.line([(525, 550), (635, 680)], fill=255, width=92)
    draw.ellipse((479, 504, 571, 596), fill=255)
    draw.ellipse((589, 634, 681, 726), fill=255)
    draw.line([(635, 705), (755, 860)], fill=255, width=100)
    draw.ellipse((585, 655, 685, 755), fill=255)
    draw.ellipse((705, 810, 805, 910), fill=255)
    alpha = np.asarray(solid.filter(ImageFilter.GaussianBlur(1.2))).astype(np.float32) / 255.0
    limited = np.zeros_like(alpha)
    limited[350:1015, 340:920] = alpha[350:1015, 340:920]
    return limited


def paint_blue_underlay(
    background: Image.Image,
    alpha: np.ndarray,
    shift_y: int,
) -> Image.Image:
    pixels = np.asarray(background).astype(np.float32)
    shifted_alpha = shift_array(alpha, shift_y)
    rows = np.broadcast_to(blue_by_row(np.asarray(background))[:, None, :], pixels.shape)
    composed = (
        pixels * (1.0 - shifted_alpha[:, :, None])
        + rows * shifted_alpha[:, :, None]
    )
    return Image.fromarray(np.clip(composed, 0, 255).astype(np.uint8), mode="RGB")


def shift_array(array: np.ndarray, shift_y: int) -> np.ndarray:
    shifted = np.zeros_like(array)
    if shift_y < 0:
        amount = -shift_y
        shifted[:-amount] = array[amount:]
    elif shift_y > 0:
        shifted[shift_y:] = array[:-shift_y]
    else:
        shifted[:] = array
    return shifted


def composite_layer(
    background: Image.Image,
    foreground: Image.Image,
    alpha: np.ndarray,
    shift_y: int,
) -> Image.Image:
    back = np.asarray(background).astype(np.float32)
    front = shift_array(np.asarray(foreground).astype(np.float32), shift_y)
    shifted_alpha = shift_array(alpha, shift_y)
    composed = back * (1.0 - shifted_alpha[:, :, None]) + front * shifted_alpha[:, :, None]
    return Image.fromarray(np.clip(composed, 0, 255).astype(np.uint8), mode="RGB")


def circle_mask(
    size: tuple[int, int],
    circles: list[tuple[int, int, int]],
    radius_expansion: int = 0,
    scale: int = 4,
) -> Image.Image:
    mask = Image.new("L", (size[0] * scale, size[1] * scale), 0)
    draw = ImageDraw.Draw(mask)
    for center_x, center_y, radius in circles:
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


def apply_cloud_front_and_gap(
    image: Image.Image,
    base: Image.Image,
    circles: list[tuple[int, int, int]],
) -> Image.Image:
    cloud_mask = circle_mask(image.size, circles)
    clouded = Image.composite(base, image, cloud_mask)
    pixels = np.asarray(clouded).copy()

    cloud = np.asarray(cloud_mask, dtype=np.float32) / 255.0
    expanded = np.asarray(
        circle_mask(image.size, circles, radius_expansion=GAP_WIDTH),
        dtype=np.float32,
    ) / 255.0
    ring = np.clip(expanded - cloud, 0.0, 1.0)

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
    gap_alpha = ring * yellow.astype(np.float32)
    row_colors = np.broadcast_to(blue_by_row(pixels)[:, None, :], pixels.shape)
    composed = (
        pixels.astype(np.float32) * (1.0 - gap_alpha[:, :, None])
        + row_colors * gap_alpha[:, :, None]
    )
    return Image.fromarray(np.clip(composed, 0, 255).astype(np.uint8), mode="RGB")


def save_small_comparison(images: list[Image.Image]) -> None:
    sizes = [64, 40, 32, 24, 16]
    scale = 4
    gap = 24
    widths = [size * scale for size in sizes]
    row_height = max(widths) + gap
    canvas = Image.new(
        "RGB",
        (sum(widths) + gap * 6, row_height * len(images) + gap),
        "#f4f6f8",
    )
    for row, image in enumerate(images):
        x = gap
        for size, display_width in zip(sizes, widths):
            icon = image.resize((size, size), Image.Resampling.LANCZOS)
            icon = icon.resize((display_width, display_width), Image.Resampling.NEAREST)
            y = row * row_height + gap + max(widths) - display_width
            canvas.paste(icon, (x, y))
            x += display_width + gap
    canvas.save(SMALL_COMPARISON, optimize=True)


def main() -> None:
    base = Image.open(BASE_PATH).convert("RGB")
    v4 = Image.open(V4_PATH).convert("RGB")
    v6 = Image.open(V6_PATH).convert("RGB")
    pure_tile = make_pure_tile(base)

    whole_alpha = visible_subject_alpha(v6)
    whole_group = composite_layer(pure_tile, v6, whole_alpha, WHOLE_GROUP_SHIFT_Y)
    whole_group.save(WHOLE_GROUP_OUTPUT, optimize=True)
    whole_group.crop((0, 50, 1024, 1024)).save(WHOLE_GROUP_INSPECTION, optimize=True)

    underlay_alpha = chain_underlay_alpha(v4)
    chain_background = paint_blue_underlay(base, underlay_alpha, CHAIN_ASSEMBLY_SHIFT_Y)
    chain_alpha = chain_layer_alpha(v4, base)
    shifted_chain = composite_layer(
        chain_background,
        v4,
        chain_alpha,
        CHAIN_ASSEMBLY_SHIFT_Y,
    )
    shifted_circles = [
        (center_x, center_y + CHAIN_ASSEMBLY_SHIFT_Y, radius)
        for center_x, center_y, radius in FRONT_CIRCLES
    ]
    chain_assembly = apply_cloud_front_and_gap(shifted_chain, base, shifted_circles)
    chain_assembly.save(CHAIN_ONLY_OUTPUT, optimize=True)
    chain_assembly.crop((0, 50, 1024, 1024)).save(CHAIN_ONLY_INSPECTION, optimize=True)

    save_small_comparison([whole_group, chain_assembly])


if __name__ == "__main__":
    main()
