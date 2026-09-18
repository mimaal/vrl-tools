"""Draws the Marketplace icon, as an editable SVG and as the PNG that ships.

Both come out of the constants below, so the two cannot drift apart. Run it
after changing them:

    python scripts/gen-icon.py

The mark is a leading dot followed by three bars: a VRL path, `.field`, which
is the one piece of syntax every VRL program starts with. Two shapes and two
colours, because the icon is read at 16 pixels in a sidebar far more often
than at 128 in a listing.
"""

from pathlib import Path

from PIL import Image, ImageDraw

# Everything is laid out in a 512 box and scaled down, which is also what gives
# the PNG its antialiasing: Pillow's rounded rectangles have hard edges.
BOX = 512
OUT = 128

BACKGROUND = "#121B24"
DOT = "#34E3CF"
BAR = "#2A9D91"

CORNER = 112

DOT_CENTRE = (148, 256)
DOT_RADIUS = 58

BAR_LEFT = 250
BAR_HEIGHT = 52
BAR_TOPS = (144, 230, 316)
BAR_RIGHTS = (432, 392, 352)

ROOT = Path(__file__).resolve().parent.parent
MEDIA = ROOT / "editors" / "vscode" / "media"


def bars():
    for top, right in zip(BAR_TOPS, BAR_RIGHTS):
        yield BAR_LEFT, top, right, top + BAR_HEIGHT


def write_png() -> Path:
    image = Image.new("RGBA", (BOX, BOX), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)

    draw.rounded_rectangle((0, 0, BOX - 1, BOX - 1), radius=CORNER, fill=BACKGROUND)

    cx, cy = DOT_CENTRE
    draw.ellipse(
        (cx - DOT_RADIUS, cy - DOT_RADIUS, cx + DOT_RADIUS, cy + DOT_RADIUS),
        fill=DOT,
    )

    for left, top, right, bottom in bars():
        draw.rounded_rectangle(
            (left, top, right, bottom), radius=BAR_HEIGHT // 2, fill=BAR
        )

    path = MEDIA / "icon.png"
    image.resize((OUT, OUT), Image.LANCZOS).save(path)
    return path


def write_svg() -> Path:
    cx, cy = DOT_CENTRE
    rects = "\n".join(
        f'  <rect x="{left}" y="{top}" width="{right - left}" '
        f'height="{bottom - top}" rx="{BAR_HEIGHT // 2}" fill="{BAR}"/>'
        for left, top, right, bottom in bars()
    )
    svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {BOX} {BOX}" \
width="{BOX}" height="{BOX}">
  <rect width="{BOX}" height="{BOX}" rx="{CORNER}" fill="{BACKGROUND}"/>
  <circle cx="{cx}" cy="{cy}" r="{DOT_RADIUS}" fill="{DOT}"/>
{rects}
</svg>
"""
    path = MEDIA / "icon.svg"
    path.write_text(svg, encoding="utf-8")
    return path


if __name__ == "__main__":
    MEDIA.mkdir(parents=True, exist_ok=True)
    for written in (write_svg(), write_png()):
        print(f"wrote {written.relative_to(ROOT)}")
