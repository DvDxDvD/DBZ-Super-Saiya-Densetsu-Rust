"""Finishes the pictures of the README gallery: removes the small "F1 debug" label that a UI snapshot
draws in its top right corner, and recompresses the PNGs (the game writes them uncompressed).

    python tools/gallery/finish.py docs/media

Needs Pillow. With `--banner`, also makes banner.png (1600 x 500) from the title and the attack pictures.
"""
import glob
import os
import sys

from PIL import Image

# The label's box by picture width: Classic is 1170 wide, Remastered 1488 wide
LABEL = {1170: (1015, 14, 1170, 50), 1488: (1330, 14, 1488, 50)}


def clean(image):
    box = LABEL.get(image.width)
    if box is None:
        return image
    x0, y0, x1, y1 = box
    pixels = image.load()
    for y in range(y0, min(y1, image.height)):
        left = pixels[x0 - 1, y]
        for x in range(x0, min(x1, image.width)):
            pixels[x, y] = left
    return image


def banner(folder):
    """banner.png (1600 x 500): the title on the left, the attack on the right, each cut from the
    1024 x 836 panel in the middle of its 1488 x 956 Remastered picture."""
    width, height = 1600, 500
    half = width // 2
    out = Image.new("RGB", (width, height))
    cuts = (
        ("cmp-01-title-remastered.png", (0, 50, 1024, 690)),
        ("cmp-09-attack-remastered.png", (112, 40, 912, 540)),
    )
    for at, (name, box) in enumerate(cuts):
        panel = Image.open(os.path.join(folder, name)).convert("RGB").crop((232, 60, 1256, 896))
        out.paste(panel.crop(box).resize((half, height), Image.LANCZOS), (at * half, 0))
    out.save(os.path.join(folder, "banner.png"), optimize=True)


def main():
    folder = sys.argv[1]
    for path in sorted(glob.glob(os.path.join(folder, "cmp-*.png"))):
        image = clean(Image.open(path).convert("RGB"))
        image.save(path, optimize=True)
        print(path, os.path.getsize(path) // 1024, "KB")
    if "--banner" in sys.argv:
        banner(folder)


if __name__ == "__main__":
    main()
