#!/usr/bin/env python3
"""Слои адаптивной иконки Android из android/icon/source.png, результат — android/res/mipmap-*.

Однотонный фон исходника становится прозрачным, рисунок вписывается в безопасный круг 66 dp
холста 108 dp, поэтому маска лаунчера его не обрезает. Нужен Pillow.
"""
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "android/icon/source.png"
RES = ROOT / "android/res"
# Цвет фона исходника; он же @color/ic_launcher_background.
BACKGROUND = (0x12, 0x1A, 0x32)
FLOOD_THRESHOLD = 24
CANVAS_DP = 108
SAFE_RADIUS_DP = 33
DENSITIES = {"mdpi": 1, "hdpi": 1.5, "xhdpi": 2, "xxhdpi": 3, "xxxhdpi": 4}
# Части рисунка для монохромного слоя: цвет исходника и номер части.
HOLE, TREAD, BODY, CREAM, GOLD = 0, 1, 2, 3, 4
PARTS = [
    ((0x12, 0x1A, 0x32), HOLE),
    ((0x3F, 0x45, 0x50), HOLE),  # катки
    ((0x17, 0x3F, 0x4B), HOLE),  # борт гусеницы
    ((0x2F, 0x62, 0xB0), HOLE),  # солнечная панель
    ((0x4A, 0x86, 0xE0), HOLE),
    ((0x25, 0x4F, 0x94), HOLE),
    ((0x15, 0x16, 0x1A), TREAD),
    ((0x2B, 0x2E, 0x35), TREAD),
    ((0x2A, 0xAE, 0xB0), BODY),
    ((0x1E, 0x8F, 0x95), BODY),
    ((0xF3, 0xEA, 0xD8), CREAM),
    ((0xCD, 0xBF, 0xA3), CREAM),
    ((0xD0, 0xA1, 0x38), GOLD),
    ((0xB8, 0x89, 0x2C), GOLD),
]


def artwork_mask(image):
    """Всё, что не соединено с краем картинки цветом фона."""
    filled = image.copy()
    marker = (255, 0, 255)
    w, h = image.size
    for seed in ((0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)):
        if filled.getpixel(seed) != marker:
            ImageDraw.floodfill(filled, seed, marker, thresh=FLOOD_THRESHOLD)
    r, g, b = filled.split()
    outside = ImageChops.multiply(
        ImageChops.multiply(r.point(lambda v: 255 * (v == 255)), g.point(lambda v: 255 * (v == 0))),
        b.point(lambda v: 255 * (v == 255)),
    )
    return ImageChops.invert(outside).filter(ImageFilter.GaussianBlur(0.8))


def enclosing_circle(mask):
    """Приближённый наименьший круг вокруг рисунка: центр и радиус в пикселях исходника."""
    step = 4
    small = mask.resize((mask.width // step, mask.height // step), Image.BILINEAR)
    pixels = small.load()
    points = [
        (x * step, y * step)
        for y in range(small.height)
        for x in range(small.width)
        if pixels[x, y] > 128
    ]
    cx, cy = points[0]
    for i in range(1, 300):
        fx, fy = max(points, key=lambda p: (p[0] - cx) ** 2 + (p[1] - cy) ** 2)
        cx += (fx - cx) / (i + 1)
        cy += (fy - cy) / (i + 1)
    radius = max(((x - cx) ** 2 + (y - cy) ** 2) ** 0.5 for x, y in points) + step
    return cx, cy, radius


def monochrome(image, mask):
    """Одноцветный рисунок, который система красит в цвет темы.

    Пиксели делятся на части по ближайшему цвету PARTS: тёмные части становятся дырами,
    между остальными прорезается линия.
    """
    flat = [channel for color, _ in PARTS for channel in color]
    palette = Image.new("P", (1, 1))
    palette.putpalette(flat + [0] * (768 - len(flat)))
    indices = image.quantize(palette=palette, dither=Image.Dither.NONE)
    lut = [part for _, part in PARTS] + [HOLE] * (256 - len(PARTS))
    parts = Image.frombytes("L", image.size, indices.tobytes()).point(lut).filter(ImageFilter.MedianFilter(9))
    solid = parts.point(lambda part: 255 * (part != HOLE))
    edges = parts.filter(ImageFilter.FIND_EDGES).point(lambda v: 255 * (v > 0))
    lines = ImageChops.multiply(edges, solid.filter(ImageFilter.MinFilter(5))).filter(ImageFilter.MaxFilter(11))
    alpha = ImageChops.multiply(ImageChops.multiply(mask, solid), ImageChops.invert(lines))
    layer = Image.new("RGBA", image.size, (255, 255, 255, 0))
    layer.putalpha(alpha.filter(ImageFilter.GaussianBlur(1)))
    return layer


def place(layer, center, radius, size):
    """Рисунок на прозрачном холсте слоя: центр круга — в центре, радиус — безопасный."""
    scale = size * SAFE_RADIUS_DP / CANVAS_DP / radius
    scaled = layer.resize((round(layer.width * scale), round(layer.height * scale)), Image.LANCZOS)
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    canvas.paste(scaled, (round(size / 2 - center[0] * scale), round(size / 2 - center[1] * scale)))
    return canvas


def main():
    image = Image.open(SOURCE).convert("RGB")
    mask = artwork_mask(image)
    cx, cy, radius = enclosing_circle(mask)
    foreground = image.convert("RGBA")
    foreground.putalpha(mask)
    mono = monochrome(image, mask)
    for density, factor in DENSITIES.items():
        size = round(CANVAS_DP * factor)
        folder = RES / f"mipmap-{density}"
        folder.mkdir(parents=True, exist_ok=True)
        place(foreground, (cx, cy), radius, size).save(folder / "ic_launcher_foreground.png", optimize=True)
        place(mono, (cx, cy), radius, size).save(folder / "ic_launcher_monochrome.png", optimize=True)


if __name__ == "__main__":
    main()
