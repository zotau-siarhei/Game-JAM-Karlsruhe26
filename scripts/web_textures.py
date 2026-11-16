"""Уменьшает текстуры внутри моделей glb для браузерной сборки.

    python3 -I scripts/web_textures.py <папка assets> <кэш> [предел, по умолчанию 1024]

Меняет файлы на месте: каждая картинка больше предела по стороне сжимается до него (Lanczos),
остальное в файле не трогается. Нужно телефону: текстура 2048×2048 занимает 16 МБ видеопамяти,
четыре десятка таких роняют WebGPU на Android (DeviceLost). Готовые файлы кладутся в кэш по
хэшу исходника, повторная сборка их не пересчитывает. Нужен Pillow.
"""
import hashlib
import io
import json
import os
import shutil
import struct
import sys

from PIL import Image

GLB_MAGIC = 0x46546C67
JSON_CHUNK = 0x4E4F534A
BIN_CHUNK = 0x004E4942


def align(data: bytes, pad: bytes) -> bytes:
    return data + pad * (-len(data) % 4)


def shrink_image(data: bytes, limit: int) -> bytes | None:
    """PNG или JPEG, уменьшенный до `limit` по большей стороне; `None` — уменьшать не нужно."""
    image = Image.open(io.BytesIO(data))
    if max(image.size) <= limit:
        return None
    fmt = image.format
    scale = limit / max(image.size)
    size = (max(1, round(image.width * scale)), max(1, round(image.height * scale)))
    image = image.resize(size, Image.Resampling.LANCZOS)
    out = io.BytesIO()
    if fmt == "JPEG":
        image.save(out, "JPEG", quality=90)
    else:
        image.save(out, "PNG", compress_level=6)
    return out.getvalue()


def shrink_glb(raw: bytes, limit: int) -> bytes | None:
    """Модель с уменьшенными текстурами; `None` — в ней нечего уменьшать."""
    magic, version, _ = struct.unpack_from("<III", raw, 0)
    if magic != GLB_MAGIC or version != 2:
        return None
    json_len, kind = struct.unpack_from("<II", raw, 12)
    assert kind == JSON_CHUNK
    doc = json.loads(raw[20:20 + json_len])
    rest = 20 + json_len
    if rest >= len(raw):
        return None
    bin_len, kind = struct.unpack_from("<II", raw, rest)
    assert kind == BIN_CHUNK
    binary = raw[rest + 8:rest + 8 + bin_len]

    views = doc.get("bufferViews", [])
    replaced = {}
    for image in doc.get("images", []):
        index = image.get("bufferView")
        if index is None:
            continue
        view = views[index]
        start = view.get("byteOffset", 0)
        smaller = shrink_image(binary[start:start + view["byteLength"]], limit)
        if smaller is not None:
            replaced[index] = smaller
    if not replaced:
        return None

    # Буфер собирается заново в прежнем порядке видов, каждый с границы 4 байт.
    order = sorted(range(len(views)), key=lambda i: views[i].get("byteOffset", 0))
    new_binary = b""
    for index in order:
        view = views[index]
        assert view.get("buffer", 0) == 0, "в glb один буфер"
        start = view.get("byteOffset", 0)
        data = replaced.get(index, binary[start:start + view["byteLength"]])
        new_binary = align(new_binary, b"\0")
        view["byteOffset"] = len(new_binary)
        view["byteLength"] = len(data)
        new_binary += data
    new_binary = align(new_binary, b"\0")
    doc["buffers"][0]["byteLength"] = len(new_binary)

    text = align(json.dumps(doc, separators=(",", ":"), ensure_ascii=False).encode(), b" ")
    total = 12 + 8 + len(text) + 8 + len(new_binary)
    return (struct.pack("<III", GLB_MAGIC, 2, total)
            + struct.pack("<II", len(text), JSON_CHUNK) + text
            + struct.pack("<II", len(new_binary), BIN_CHUNK) + new_binary)


def main() -> None:
    assets, cache = sys.argv[1], sys.argv[2]
    limit = int(sys.argv[3]) if len(sys.argv) > 3 else 1024
    os.makedirs(cache, exist_ok=True)
    before = after = 0
    for directory, _, names in os.walk(assets):
        for name in sorted(names):
            if not name.endswith(".glb"):
                continue
            path = os.path.join(directory, name)
            with open(path, "rb") as handle:
                raw = handle.read()
            key = hashlib.sha256(raw + str(limit).encode()).hexdigest()[:24]
            cached = os.path.join(cache, key + ".glb")
            if not os.path.exists(cached):
                smaller = shrink_glb(raw, limit) or raw
                with open(cached + ".tmp", "wb") as handle:
                    handle.write(smaller)
                os.replace(cached + ".tmp", cached)
            # Клон APFS заменяется копией: исходник в репозитории не меняется.
            os.remove(path)
            shutil.copyfile(cached, path)
            before += len(raw)
            after += os.path.getsize(path)
    print(f"текстуры моделей до {limit}px: {before / 1e6:.1f} МБ → {after / 1e6:.1f} МБ")


if __name__ == "__main__":
    main()
