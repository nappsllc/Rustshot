#!/usr/bin/env python3
"""Microsoft Store screenshots, 1920 x 1080, one set per UI language.

Rustshot's own preview renders, never a capture of a desktop:

    RUSTSHOT_PREVIEW_DIR=<dir> RUSTSHOT_FIT_LANGS=all \\
        cargo test --bin rustshot -- --include-ignored store_screenshot
    python tools/store-screenshots.py <dir> [--out <folder>] [--docs]

The tests write <dir>/<tag>/*.png for English and every lang/<tag>.txt;
this writes dist/store/screenshots/<store code>/01-....png .. 08-....png
(or under --out) and captions.txt (from docs/store/screenshot-captions.json:
file name, a tab, the caption) per language, in the Store codes msix.ps1
declares (en-us, pt-br, zh-Hans ...). --docs also refreshes the English
set in docs/store/screenshots.

The overlay renders are already 1920 x 1080 (a sample desktop drawn by
src/editor/store_tests.rs); the History, Edit and Settings windows are put
on the same neutral wallpaper with rounded corners and a soft shadow.
Needs Pillow.
"""
import json
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

W, H = 1920, 1080
ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "dist" / "store" / "screenshots"
DOCS = ROOT / "docs" / "store" / "screenshots"
CAPTIONS = ROOT / "docs" / "store" / "screenshot-captions.json"
MAX_BYTES = 2_000_000

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

# lang/<tag>.txt -> the Store's language code (packaging/windows/msix.ps1).
STORE_CODE = {"en": "en-us", "pt-BR": "pt-br", "pt-PT": "pt-pt"}


def store_code(tag):
    return STORE_CODE.get(tag, tag if tag in ("sr-Latn", "zh-Hans", "zh-Hant") else tag.lower())


def wallpaper():
    """The overlay renders' wallpaper: a slate gradient, top to bottom."""
    img = Image.new("RGB", (W, H))
    px = ImageDraw.Draw(img)
    for y in range(H):
        t = y / H
        px.line([(0, y), (W, y)], fill=(int(64 + 34 * t), int(72 + 36 * t), int(88 + 40 * t)))
    return img


WALL = None


def window(src):
    """`src` centred on the wallpaper: 8 px corners, a 1 px edge, a shadow."""
    global WALL
    if WALL is None:
        WALL = wallpaper().convert("RGBA")
    win = Image.open(src).convert("RGB")
    w, h = win.size
    assert w <= W and h <= H, f"{src} is {w} x {h}"
    x, y = (W - w) // 2, (H - h) // 2
    mask = Image.new("L", (w, h), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, w - 1, h - 1], radius=8, fill=255)
    shadow = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle([x, y + 12, x + w, y + h + 12], radius=12, fill=(0, 0, 0, 110))
    bg = Image.alpha_composite(WALL, shadow.filter(ImageFilter.GaussianBlur(28)))
    bg.paste(win, (x, y), mask)
    edge = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    ImageDraw.Draw(edge).rounded_rectangle([x, y, x + w - 1, y + h - 1], radius=8, outline=(255, 255, 255, 40), width=1)
    return Image.alpha_composite(bg, edge).convert("RGB")


def main():
    global OUT
    args = sys.argv[1:]
    docs = "--docs" in args
    args = [a for a in args if a != "--docs"]
    if "--out" in args:
        i = args.index("--out")
        OUT = Path(args[i + 1])
        del args[i:i + 2]
    if len(args) != 1:
        sys.exit(__doc__)
    src = Path(args[0])
    captions = json.loads(CAPTIONS.read_text(encoding="utf-8"))
    tags = sorted(p.name for p in src.iterdir() if p.is_dir())
    assert "en" in tags, f"no {src / 'en'}"
    total = 0
    for tag in tags:
        code = store_code(tag)
        caps = captions.get(code)
        assert caps and len(caps) == len(SHOTS), f"{CAPTIONS.name}: {code} needs {len(SHOTS)} captions"
        out = OUT / code
        out.mkdir(parents=True, exist_ok=True)
        for (name, file, how), cap in zip(SHOTS, caps):
            assert 0 < len(cap) <= 200, f"{code} {name}: caption is {len(cap)} characters"
            img = Image.open(src / tag / file).convert("RGB") if how == "full" else window(src / tag / file)
            assert img.size == (W, H), f"{tag}/{file}: {img.size}"
            img.save(out / name, optimize=True)
            size = (out / name).stat().st_size
            assert size < MAX_BYTES, f"{out / name}: {size} bytes"
            total += size
            if tag == "en" and docs:
                img.save(DOCS / name, optimize=True)
        (out / "captions.txt").write_text(
            "".join(f"{name}\t{cap}\n" for (name, _, _), cap in zip(SHOTS, caps)), encoding="utf-8")
        print(out)
    print(f"{len(tags)} languages, {len(tags) * len(SHOTS)} screenshots, {total / 1e6:.1f} MB")


if __name__ == "__main__":
    main()
