# Перекраска текстуры человека: кожа, волосы, одежда по классам цветов.
# Скины жителей: /Applications/Blender.app/Contents/MacOS/Blender -b --python scripts/recolor_people.py -- \
#   <id> <текстура.png из glb> <папка> <число скинов> <размер> <glb>
import bpy, sys, numpy as np
args = sys.argv[sys.argv.index('--') + 1:]
model, src, out, n, size, glb = args[0], args[1], args[2], int(args[3]), int(args[4]), args[5]

LUM = np.array([0.3, 0.55, 0.15], dtype=np.float32)
# классы: имя -> (центры в сырых значениях пикселей, вид)
CLASSES = {
    'resident_man': [
        ('skin', [[0.646, 0.457, 0.392], [0.526, 0.346, 0.285]], 'skin'),
        ('bottom', [[0.016, 0.089, 0.169]], 'bottom'),
        ('top', [[0.077, 0.176, 0.293], [0.18, 0.297, 0.419]], 'top'),
        ('hair', [[0.128, 0.091, 0.058], [0.269, 0.143, 0.066]], 'hair'),
        ('keep', [[0.85, 0.82, 0.72]], 'keep'),
        ('void', [[0.0, 0.0, 0.0]], 'void'),
    ],
    'resident_woman': [
        ('skin', [[0.876, 0.62, 0.533], [0.904, 0.675, 0.572], [0.699, 0.448, 0.333]], 'skin'),
        ('top', [[0.753, 0.651, 0.555]], 'top'),
        ('bottom', [[0.536, 0.539, 0.52]], 'bottom'),
        ('hair', [[0.232, 0.123, 0.059]], 'hair'),
        ('keep', [[0.903, 0.903, 0.885]], 'keep'),
        ('void', [[0.0, 0.0, 0.0]], 'void'),
    ],
    'resident_elder': [
        ('skin', [[0.691, 0.479, 0.403]], 'skin'),
        ('top', [[0.11, 0.168, 0.084]], 'top'),
        ('bottom', [[0.263, 0.184, 0.082]], 'bottom'),
        ('keep', [[0.522, 0.459, 0.422], [0.805, 0.787, 0.768], [0.161, 0.137, 0.106], [0.129, 0.078, 0.031]], 'keep'),
        ('void', [[0.0, 0.0, 0.0]], 'void'),
    ],
    'resident_young': [
        ('skin', [[0.55, 0.365, 0.294], [0.477, 0.304, 0.224]], 'skin'),
        ('bottom', [[0.429, 0.57, 0.713]], 'bottom'),
        ('top', [[0.856, 0.589, 0.232], [0.815, 0.541, 0.192]], 'top'),
        ('hair', [[0.227, 0.1, 0.049]], 'hair'),
        ('keep', [[0.786, 0.826, 0.866]], 'keep'),
        ('void', [[0.0, 0.0, 0.0]], 'void'),
    ],
}[model]

SKIN = [[0.93, 0.72, 0.62], [0.80, 0.58, 0.46], [0.68, 0.46, 0.34], [0.52, 0.34, 0.24], [0.38, 0.24, 0.16], [0.27, 0.17, 0.11]]
HAIR = [[0.03, 0.025, 0.02], [0.09, 0.055, 0.03], [0.2, 0.12, 0.06], [0.42, 0.28, 0.14], [0.7, 0.55, 0.32], [0.3, 0.1, 0.04]]
TOPS = [[0.08, 0.12, 0.3], [0.05, 0.05, 0.06], [0.5, 0.5, 0.5], [0.85, 0.85, 0.82], [0.72, 0.6, 0.42], [0.25, 0.32, 0.14],
        [0.35, 0.22, 0.35], [0.8, 0.6, 0.15], [0.1, 0.42, 0.42], [0.55, 0.7, 0.85], [0.6, 0.48, 0.7], [0.85, 0.55, 0.6],
        [0.2, 0.45, 0.25], [0.6, 0.3, 0.15]]
BOTTOMS = [[0.05, 0.1, 0.22], [0.04, 0.04, 0.045], [0.3, 0.3, 0.3], [0.55, 0.46, 0.32], [0.22, 0.25, 0.13],
           [0.3, 0.42, 0.6], [0.28, 0.18, 0.1], [0.12, 0.14, 0.2], [0.7, 0.68, 0.62]]

img = bpy.data.images.load(src)
w, h = img.size
px = np.array(img.pixels[:], dtype=np.float32).reshape(h, w, 4)
step = w // size
px = px.reshape(size, step, size, step, 4).mean((1, 3))
rgb = px[..., :3].reshape(-1, 3)

# мягкая принадлежность пикселя классам
SIGMA = 0.06
dist = []
for _, centers, _ in CLASSES:
    c = np.array(centers, dtype=np.float32)
    dist.append(((rgb[:, None, :] - c[None]) ** 2).sum(2).min(1))
dist = np.stack(dist, 1)
weight = np.exp(-(dist - dist.min(1, keepdims=True)) / (2 * SIGMA ** 2))
weight /= weight.sum(1, keepdims=True)
lum = rgb @ LUM

# Куски развёртки (UV-острова меша): верх и низ одного цвета (у мужчины оба синие) по пикселю
# путаются, поэтому кусок целиком отдаётся тому, чего в нём больше. Острова берутся из меша:
# в текстуре они прилегают вплотную и по пикселям склеиваются.
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=glb)
mesh = next(o for o in bpy.data.objects if o.type == 'MESH').data
uv = mesh.uv_layers.active.data
parent = list(range(len(mesh.polygons)))
def find(a):
    while parent[a] != a:
        parent[a] = parent[parent[a]]
        a = parent[a]
    return a
seen = {}
for poly in mesh.polygons:
    for li in poly.loop_indices:
        key = (mesh.loops[li].vertex_index, round(uv[li].uv[0], 5), round(uv[li].uv[1], 5))
        if key in seen:
            a, b = find(seen[key]), find(poly.index)
            if a != b:
                parent[a] = b
        else:
            seen[key] = poly.index
big = size * size
label = np.full(big, big, dtype=np.int64)
ys, xs = np.mgrid[0:size, 0:size]
for poly in mesh.polygons:
    isl = find(poly.index)
    pts = np.array([uv[li].uv[:] for li in poly.loop_indices], dtype=np.float64) * size
    for k in range(1, len(pts) - 1):
        a, b, c = pts[0], pts[k], pts[k + 1]
        x0, x1 = int(max(min(a[0], b[0], c[0]) - 1, 0)), int(min(max(a[0], b[0], c[0]) + 2, size))
        y0, y1 = int(max(min(a[1], b[1], c[1]) - 1, 0)), int(min(max(a[1], b[1], c[1]) + 2, size))
        if x0 >= x1 or y0 >= y1:
            continue
        px_ = xs[y0:y1, x0:x1] + 0.5; py_ = ys[y0:y1, x0:x1] + 0.5
        d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1])
        if abs(d) < 1e-12:
            continue
        l1 = ((b[1] - c[1]) * (px_ - c[0]) + (c[0] - b[0]) * (py_ - c[1])) / d
        l2 = ((c[1] - a[1]) * (px_ - c[0]) + (a[0] - c[0]) * (py_ - c[1])) / d
        inside = (l1 >= -0.05) & (l2 >= -0.05) & (1 - l1 - l2 >= -0.05)
        sub = label.reshape(size, size)[y0:y1, x0:x1]
        sub[inside] = isl
print('ISLANDS', len(set(find(i) for i in range(len(parent)))), 'covered', (label < big).mean())
kinds = [k for _, _, k in CLASSES]
# Верх, низ и «оставить как есть» (белая обувь, воротник) делят кусок по большинству: иначе
# светлые места ткани просвечивают белыми полосами по швам.
# Пиксели вне найденных островов, но не фон: каждый решается сам по себе.
stray = (label == big) & (rgb.max(1) > 0.04)
label = np.where(stray, np.arange(big), label)
print('STRAY', stray.mean())
covered = label < big
# Чёрный — только фон между кусками: на самих кусках тёмная ткань не считается фоном.
iv = kinds.index('void')
dist[:, iv] = np.where(covered, np.inf, dist[:, iv])
weight = np.exp(-(dist - dist.min(1, keepdims=True)) / (2 * SIGMA ** 2))
weight /= weight.sum(1, keepdims=True)
group = [i for i, k in enumerate(kinds) if k in ('top', 'bottom', 'keep')]
sums = np.stack([np.bincount(label, weights=weight[:, i], minlength=big + 1) for i in group], 1)
winner = sums.argmax(1)[label]
both = weight[:, group].sum(1)
for j, i in enumerate(group):
    weight[:, i] = np.where(covered, np.where(winner == j, both, 0), weight[:, i])

rng = np.random.default_rng(sum(map(ord, model)))
def deck(palette, kind):
    ref = float((np.array([c for _, cs, k in CLASSES if k == kind for c in cs] or [[0.5, 0.5, 0.5]], dtype=np.float32) @ LUM).mean())
    # Из тёмной ткани светлая выходит пятнами: тёмному источнику только тёмные цвета.
    ok = [c for c in palette if ref > 0.12 or float(np.array(c) @ LUM) < 0.25]
    # Без повторов, пока цвета не кончатся.
    while True:
        for i in rng.permutation(len(ok)):
            yield ok[i]
tops, bottoms = deck(TOPS, 'top'), deck(BOTTOMS, 'bottom')
for v in range(1, n + 1):
    # Тона кожи по кругу, чтобы в наборе были все; к тёмной коже — только тёмные волосы.
    s = (v * 5 + rng.integers(2)) % len(SKIN)
    skin = np.array(SKIN[s], dtype=np.float32)
    hair = np.array(HAIR[rng.integers(3 if s >= 3 else len(HAIR))], dtype=np.float32)
    top = np.array(next(tops), dtype=np.float32)
    bottom = np.array(next(bottoms), dtype=np.float32)
    target = {'skin': skin, 'hair': hair, 'top': top, 'bottom': bottom}
    result = np.zeros_like(rgb)
    for i, (_, centers, kind) in enumerate(CLASSES):
        if kind in ('keep', 'void', 'fixed'):
            col = rgb
        else:
            ref = float((np.array(centers, dtype=np.float32) @ LUM).mean())
            if kind == 'skin':
                # кожа: тон меняется отношением цветов, рисунок лица сохраняется
                col = rgb * (target['skin'] / np.array(centers[0], dtype=np.float32))
            else:
                r = (lum / ref)[:, None]
                t = target[kind][None]
                # Блики ткани не пересвечивают цвет: выше среднего — мягко к белому.
                col = t * np.minimum(r, 1.4)
        result += weight[:, i:i + 1] * col
    result = np.clip(result, 0, 1)
    outpx = np.concatenate([result.reshape(size, size, 3), np.ones((size, size, 1), np.float32)], 2)
    im = bpy.data.images.new(f'{model}_{v}', size, size, alpha=False)
    im.pixels[:] = outpx.ravel()
    im.filepath_raw = f'{out}/{model}_{v}.png'
    im.file_format = 'PNG'
    im.save()
    print('SAVED', im.filepath_raw)
