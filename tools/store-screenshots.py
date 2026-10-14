#!/usr/bin/env python3
"""Microsoft Store screenshots (docs/store/screenshots/*.png, 1920 x 1080).

Rustshot's own preview renders, never a capture of a desktop:

    RUSTSHOT_PREVIEW_DIR=<dir> cargo test --bin rustshot -- --include-ignored \
        store_screenshot preview_settings_pngs
    python tools/store-screenshots.py <dir>

The overlay renders are already 1920 x 1080 (a sample desktop drawn by
src/editor/store_tests.rs); the History, Edit and Settings windows are put
on the same neutral wallpaper with rounded corners and a soft shadow.
Needs Pillow.
"""
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

W, H = 1920, 1080
OUT = Path(__file__).resolve().parent.parent / "docs" / "store" / "screenshots"

# (output, source, how): "full" is copied as it is; "window" is composited.
SHOTS = [
    ("01-overlay-dark.png", "store-overlay-dark.png", "full"),
    ("02-overlay-light.png", "store-overlay-light.png", "full"),
    ("03-text-mode.png", "store-text-mode.png", "full"),
    ("04-quick-redact.png", "store-redacted.png", "full"),
    ("05-recording-setup.png", "store-video-panel.png", "full"),
    ("06-history.png", "store-history-dark.png", "window"),
    ("07-edit-canvas.png", "store-edit-presentation-light.png", "window"),
    ("08-settings.png", "settings-shortcuts-global-dark-150.png", "window"),
]


def wallpaper():
    """The overlay renders' wallpaper: a slate gradient, top to bottom."""
    img = Image.new("RGB", (W, H))
    px = ImageDraw.Draw(img)
    for y in range(H):
        t = y / H
        px.line([(0, y), (W, y)], fill=(int(64 + 34 * t), int(72 + 36 * t), int(88 + 40 * t)))
    return img


def window(src):
    """`src` centred on the wallpaper: 8 px corners, a 1 px edge, a shadow."""
    win = Image.open(src).convert("RGB")
    w, h = win.size
    assert w <= W and h <= H, f"{src} is {w} x {h}"
    bg = wallpaper().convert("RGBA")
    x, y = (W - w) // 2, (H - h) // 2
    mask = Image.new("L", (w, h), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, w - 1, h - 1], radius=8, fill=255)
    shadow = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle([x, y + 12, x + w, y + h + 12], radius=12, fill=(0, 0, 0, 110))
    bg = Image.alpha_composite(bg, shadow.filter(ImageFilter.GaussianBlur(28)))
    bg.paste(win, (x, y), mask)
    edge = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    ImageDraw.Draw(edge).rounded_rectangle([x, y, x + w - 1, y + h - 1], radius=8, outline=(255, 255, 255, 40), width=1)
    return Image.alpha_composite(bg, edge).convert("RGB")


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    src = Path(sys.argv[1])
    OUT.mkdir(parents=True, exist_ok=True)
    for name, file, how in SHOTS:
        img = Image.open(src / file).convert("RGB") if how == "full" else window(src / file)
        assert img.size == (W, H), f"{file}: {img.size}"
        img.save(OUT / name, optimize=True)
        print(OUT / name)


if __name__ == "__main__":
    main()
