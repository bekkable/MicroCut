"""Draws the MicroCut icon: a film strip whose middle part is held by two range brackets.

Original artwork, licensed like MicroCut (MIT OR Apache-2.0). Writes `icon.png` (1024 px, macOS bundle and
window) and `icon.ico` (16-256 px, Windows exe) next to this script. Needs Pillow: `python make_icon.py`.
"""

from pathlib import Path

from PIL import Image, ImageDraw

SIZE = 1024
SCALE = 4  # drawn larger, then scaled down: smooth edges
HERE = Path(__file__).resolve().parent

TOP = (52, 59, 74)
BOTTOM = (29, 33, 43)
STRIP = (18, 21, 27)
HOLE = (61, 69, 86)
FRAME = (74, 82, 99)
FRAME_SELECTED = (214, 222, 235)
BRACKET = (255, 86, 64)  # the timeline's cursor colour


def s(value):
    return round(value * SCALE)


def box(x0, y0, x1, y1):
    return [s(x0), s(y0), s(x1), s(y1)]


def draw():
    size = s(SIZE)
    gradient = Image.new("RGB", (1, size))
    for y in range(size):
        t = y / (size - 1)
        gradient.putpixel((0, y), tuple(round(a + (b - a) * t) for a, b in zip(TOP, BOTTOM)))
    gradient = gradient.resize((size, size))

    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).rounded_rectangle(box(64, 64, 960, 960), radius=s(200), fill=255)
    image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    image.paste(gradient, (0, 0), mask)
    d = ImageDraw.Draw(image)

    # Film strip, cut off by the icon's edges so it seems to run on. Frames sit on a grid centred on the
    # icon; the three in the middle are selected.
    left, right, top, bottom = 120, 904, 352, 672
    centre, frame_w, pitch = SIZE // 2, 116, 152
    d.rectangle(box(left, top, right, bottom), fill=STRIP)

    hole_w, hole_h = 34, 38
    for k in range(-6, 6):
        # Between the frame centres, so the brackets fall between holes too.
        x = centre + (k + 0.5) * pitch / 2 - hole_w / 2
        if x > left + 6 and x + hole_w < right - 6:
            for y in (top + 20, bottom - 20 - hole_h):
                d.rounded_rectangle(box(x, y, x + hole_w, y + hole_h), radius=s(8), fill=HOLE)

    frame_top, frame_bottom = top + 78, bottom - 78
    for k in range(-3, 4):
        x0 = max(centre + k * pitch - frame_w / 2, left + 8)
        x1 = min(centre + k * pitch + frame_w / 2, right - 8)
        if x1 - x0 > 20:
            fill = FRAME_SELECTED if abs(k) <= 1 else FRAME
            d.rounded_rectangle(box(x0, frame_top, x1, frame_bottom), radius=s(10), fill=fill)

    # Range brackets "[ ]" in the gaps around the selected frames, reaching past the strip.
    bar, tab, reach = 32, 60, 56
    y0, y1 = top - reach, bottom + reach
    for side in (-1, 1):
        gap_centre = centre + side * 1.5 * pitch
        x0 = gap_centre - bar / 2
        d.rounded_rectangle(box(x0, y0, x0 + bar, y1), radius=s(8), fill=BRACKET)
        # The tabs point inwards, towards the selection.
        tx0, tx1 = (x0, x0 + bar + tab) if side < 0 else (x0 - tab, x0 + bar)
        for y in (y0, y1 - bar):
            d.rounded_rectangle(box(tx0, y, tx1, y + bar), radius=s(8), fill=BRACKET)

    return image.resize((SIZE, SIZE), Image.LANCZOS)


def main():
    icon = draw()
    icon.save(HERE / "icon.png", optimize=True)
    icon.save(HERE / "icon.ico", sizes=[(n, n) for n in (16, 20, 24, 32, 40, 48, 64, 128, 256)])
    print("wrote", HERE / "icon.png", "and", HERE / "icon.ico")


if __name__ == "__main__":
    main()
