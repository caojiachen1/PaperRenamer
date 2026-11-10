"""Generate monochrome PaperRenamer icons (PNG sizes + multi-size ICO)."""
from PIL import Image, ImageDraw
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"
OUT.mkdir(parents=True, exist_ok=True)


def render(size: int) -> Image.Image:
    s = size / 256.0
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle(
        [int(8 * s), int(8 * s), int(248 * s), int(248 * s)],
        radius=int(48 * s),
        fill=(26, 26, 26, 255),
    )
    # document sheet
    d.polygon(
        [(78 * s, 52 * s), (150 * s, 52 * s), (182 * s, 84 * s),
         (182 * s, 204 * s), (78 * s, 204 * s)],
        fill=(255, 255, 255, 255),
    )
    # folded corner
    d.polygon(
        [(150 * s, 52 * s), (150 * s, 84 * s), (182 * s, 84 * s)],
        fill=(176, 176, 176, 255),
    )
    # title lines
    d.rounded_rectangle([96 * s, 108 * s, 164 * s, 118 * s], radius=int(5 * s), fill=(26, 26, 26, 255))
    d.rounded_rectangle([96 * s, 130 * s, 150 * s, 137 * s], radius=int(3 * s), fill=(110, 110, 110, 255))
    d.rounded_rectangle([96 * s, 148 * s, 158 * s, 155 * s], radius=int(3 * s), fill=(110, 110, 110, 255))
    d.rounded_rectangle([96 * s, 166 * s, 140 * s, 173 * s], radius=int(3 * s), fill=(110, 110, 110, 255))
    return img


for name, sz in [("32x32.png", 32), ("128x128.png", 128), ("128x128@2x.png", 256), ("icon.png", 256)]:
    render(sz).save(OUT / name)

ico_sizes = [16, 24, 32, 48, 64, 128, 256]
render(256).save(OUT / "icon.ico", sizes=[(s, s) for s in ico_sizes])
print("icons written to", OUT)