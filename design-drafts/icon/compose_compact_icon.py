from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw


ROOT = Path(__file__).resolve().parent
SOURCE = ROOT / "gpt-image" / "savelink-cloud-chain-milestone-v1-highlight-parallel-v2.png"
OUTPUT = ROOT / "gpt-image" / "savelink-cloud-chain-compact-v2-large-inset-chain.png"
PREVIEW = ROOT / "gpt-image" / "savelink-cloud-chain-compact-v2-small-preview.png"


def extract_layer(
    source: Image.Image,
    box: tuple[int, int, int, int],
    color: str,
) -> Image.Image:
    crop = np.asarray(source.crop(box).convert("RGB"), dtype=np.int16)
    red, _, blue = np.moveaxis(crop, 2, 0)

    # Separate the established blue cloud and orange chain by chroma. This
    # prevents the two source crops from carrying pixels of the other shape.
    if color == "blue":
        chroma = blue - red
        alpha = np.clip((chroma - 4) * 16, 0, 255).astype(np.uint8)
        dark = np.mean(crop, axis=2) < 160
        foreground = np.empty_like(crop, dtype=np.uint8)
        foreground[dark] = (0, 22, 70)
        foreground[~dark] = (216, 226, 245)
    elif color == "orange":
        chroma = red - blue
        alpha = np.clip((chroma - 24) * 4, 0, 255).astype(np.uint8)
        foreground = np.empty_like(crop, dtype=np.uint8)
        foreground[:] = (254, 114, 9)
    else:
        raise ValueError(f"unsupported layer color: {color}")

    rgba = np.dstack((foreground, alpha))
    return Image.fromarray(rgba, "RGBA")


def contain(image: Image.Image, width: int) -> Image.Image:
    height = round(image.height * width / image.width)
    return image.resize((width, height), Image.Resampling.LANCZOS)


def nonwhite_alpha(image: Image.Image) -> Image.Image:
    rgb = np.asarray(image.convert("RGB"), dtype=np.int16)
    distance = np.max(np.abs(rgb - 255), axis=2)
    alpha = np.clip((distance - 2) * 20, 0, 255).astype(np.uint8)
    rgba = np.dstack((rgb.astype(np.uint8), alpha))
    return Image.fromarray(rgba, "RGBA")


def main() -> None:
    source = Image.open(SOURCE).convert("RGB")

    cloud = contain(extract_layer(source, (190, 90, 883, 553), "blue"), 850)
    chain = contain(extract_layer(source, (255, 520, 621, 916), "orange"), 390)

    canvas = Image.new("RGBA", (1024, 1024), (255, 255, 255, 255))
    canvas.alpha_composite(cloud, ((1024 - cloud.width) // 2, 82))

    # Most of the enlarged chain sits inside the cloud, leaving only the lower
    # link outside. Both motifs remain legible when the icon is downscaled.
    canvas.alpha_composite(chain, ((1024 - chain.width) // 2, 365))
    canvas.convert("RGB").save(OUTPUT, quality=100)

    icon = nonwhite_alpha(canvas)
    taskbar = Image.new("RGBA", (760, 220), (232, 241, 246, 255))
    draw = ImageDraw.Draw(taskbar)
    sizes = (64, 40, 32, 24, 16)
    x = 35
    for size in sizes:
        small = icon.resize((size, size), Image.Resampling.LANCZOS)
        y = 78 + (64 - size) // 2
        taskbar.alpha_composite(small, (x + (64 - size) // 2, y))
        draw.text((x + 20, 164), f"{size}px", fill=(42, 55, 70, 255))
        x += 140
    taskbar.convert("RGB").save(PREVIEW, quality=100)


if __name__ == "__main__":
    main()
