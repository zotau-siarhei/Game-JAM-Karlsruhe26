#!/usr/bin/env python3
"""Палитра моделей для обломков: пишет в `assets/models/models.json` поля `colors` и `top_colors`.

Текстуры моделей в игре живут только в видеопамяти, поэтому цвета для обломков считаются заранее.
Цвет каждого треугольника берётся из текстуры в центре его UV, вес — площадь треугольника.
`colors` — главные цвета нижних 70 % высоты модели (стены дома, кузов вагона), `top_colors` —
верхних 30 % (крыша). Цвета — до четырёх, по убыванию площади, шестнадцатеричные sRGB.

Запуск из корня репозитория (нужен Pillow): `python3 scripts/model_colors.py`
"""

import io
import json
import math
import re
import struct
import sys
from pathlib import Path

from PIL import Image

MODELS = Path(__file__).resolve().parent.parent / "assets" / "models"
LIST = MODELS / "models.json"

# Доля высоты снизу, которая считается «телом», остальное — «верх».
BODY_SHARE = 0.7
# Сколько цветов ищется и какая наименьшая доля площади у цвета, попавшего в список.
CLUSTERS = 6
MIN_SHARE = 0.08
MAX_COLORS = 4

COMPONENTS = {5120: "b", 5121: "B", 5122: "h", 5123: "H", 5125: "I", 5126: "f"}
WIDTH = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}


def read_glb(path):
    data = path.read_bytes()
    length = struct.unpack_from("<I", data, 12)[0]
    doc = json.loads(data[20 : 20 + length])
    offset = 20 + length
    bin_length = struct.unpack_from("<I", data, offset)[0]
    return doc, data[offset + 8 : offset + 8 + bin_length]


def accessor(doc, blob, index):
    acc = doc["accessors"][index]
    view = doc["bufferViews"][acc["bufferView"]]
    fmt = COMPONENTS[acc["componentType"]]
    width = WIDTH[acc["type"]]
    size = struct.calcsize("<" + fmt)
    stride = view.get("byteStride") or size * width
    start = view.get("byteOffset", 0) + acc.get("byteOffset", 0)
    out = []
    for i in range(acc["count"]):
        values = struct.unpack_from("<" + fmt * width, blob, start + i * stride)
        if acc.get("normalized") and fmt != "f":
            scale = float((1 << (size * 8 - (1 if fmt.islower() else 0))) - 1)
            values = tuple(v / scale for v in values)
        out.append(values)
    return out


def texture(doc, blob, material):
    pbr = material.get("pbrMetallicRoughness", {})
    factor = pbr.get("baseColorFactor", [1, 1, 1, 1])[:3]
    info = pbr.get("baseColorTexture")
    if info is None:
        return None, factor
    image = doc["images"][doc["textures"][info["index"]]["source"]]
    view = doc["bufferViews"][image["bufferView"]]
    start = view.get("byteOffset", 0)
    picture = Image.open(io.BytesIO(blob[start : start + view["byteLength"]])).convert("RGB")
    return picture, factor


def samples(path):
    """Цвет, площадь и высота центра каждого треугольника модели."""
    doc, blob = read_glb(path)
    out = []
    for mesh in doc["meshes"]:
        for prim in mesh["primitives"]:
            attrs = prim["attributes"]
            positions = accessor(doc, blob, attrs["POSITION"])
            uvs = accessor(doc, blob, attrs["TEXCOORD_0"]) if "TEXCOORD_0" in attrs else None
            if "indices" in prim:
                flat = [i[0] for i in accessor(doc, blob, prim["indices"])]
            else:
                flat = list(range(len(positions)))
            material = doc["materials"][prim["material"]] if "material" in prim else {}
            picture, factor = texture(doc, blob, material)
            for t in range(0, len(flat) - 2, 3):
                a, b, c = (positions[flat[t + k]] for k in range(3))
                ab = [b[i] - a[i] for i in range(3)]
                ac = [c[i] - a[i] for i in range(3)]
                cross = (ab[1] * ac[2] - ab[2] * ac[1], ab[2] * ac[0] - ab[0] * ac[2], ab[0] * ac[1] - ab[1] * ac[0])
                area = 0.5 * math.sqrt(sum(v * v for v in cross))
                if area <= 0:
                    continue
                y = (a[1] + b[1] + c[1]) / 3
                if picture is not None and uvs is not None:
                    u = sum(uvs[flat[t + k]][0] for k in range(3)) / 3 % 1.0
                    v = sum(uvs[flat[t + k]][1] for k in range(3)) / 3 % 1.0
                    w, h = picture.size
                    rgb = picture.getpixel((min(int(u * w), w - 1), min(int(v * h), h - 1)))
                    color = tuple(rgb[i] / 255 * factor[i] for i in range(3))
                else:
                    color = tuple(factor)
                out.append((color, area, y))
    return out


def palette(points):
    """Главные цвета по площади: взвешенные k-средних с начальными центрами по самым частым корзинам."""
    if not points:
        return []
    bins = {}
    for color, weight in points:
        key = tuple(min(int(c * 16), 15) for c in color)
        total, acc = bins.get(key, (0.0, (0.0, 0.0, 0.0)))
        bins[key] = (total + weight, tuple(acc[i] + color[i] * weight for i in range(3)))
    items = [(tuple(acc[i] / total for i in range(3)), total) for total, acc in bins.values()]
    items.sort(key=lambda item: -item[1])
    centers = []
    for color, _ in items:
        if all(sum((color[i] - c[i]) ** 2 for i in range(3)) > 0.02 for c in centers):
            centers.append(color)
        if len(centers) == CLUSTERS:
            break
    for _ in range(12):
        sums = [[0.0, 0.0, 0.0, 0.0] for _ in centers]
        for color, weight in items:
            k = min(range(len(centers)), key=lambda j: sum((color[i] - centers[j][i]) ** 2 for i in range(3)))
            for i in range(3):
                sums[k][i] += color[i] * weight
            sums[k][3] += weight
        centers = [tuple(s[i] / s[3] for i in range(3)) if s[3] > 0 else centers[j] for j, s in enumerate(sums)]
    total = sum(s[3] for s in sums)
    ranked = sorted(zip(centers, (s[3] / total for s in sums)), key=lambda item: -item[1])
    chosen = [color for color, share in ranked if share >= MIN_SHARE][:MAX_COLORS]
    return ["#%02x%02x%02x" % tuple(round(min(max(c, 0.0), 1.0) * 255) for c in color) for color in chosen]


def main():
    doc = json.loads(LIST.read_text())
    for entry in doc["models"]:
        path = MODELS / entry["file"]
        if not path.is_file():
            print(f"{entry['id']}: нет файла", file=sys.stderr)
            continue
        points = samples(path)
        low = min(p[2] for p in points)
        high = max(p[2] for p in points)
        cut = low + (high - low) * BODY_SHARE
        entry["colors"] = palette([(c, a) for c, a, y in points if y <= cut])
        entry["top_colors"] = palette([(c, a) for c, a, y in points if y > cut])
        print(f"{entry['id']}: {entry['colors']} / {entry['top_colors']}")
    text = json.dumps(doc, ensure_ascii=False, indent=2)
    # Списки цветов — в одну строку, как остальные поля записи.
    text = re.sub(r"\[\s*((?:\"#[0-9a-f]{6}\",?\s*)*)\]", lambda m: "[" + ", ".join(re.findall(r'"#[0-9a-f]{6}"', m.group(1))) + "]", text)
    LIST.write_text(text + "\n")


if __name__ == "__main__":
    main()
