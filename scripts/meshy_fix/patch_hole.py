# Закрывает дырку в плоской стене, у которой нет открытого края (Meshy оставил треугольник без
# грани на фронтоне маленького дома). Стена прощупывается лучами снаружи вдоль оси X: в каждом ряду
# по Z ищется участок между кусками стены, где луч уходит внутрь модели. По этим участкам строится
# заплатка с запасом по бокам и сверху, на 1 мм модели глубже стены — видна только сквозь дырку и
# не спорит со стеной за глубину. Цвет — точка текстуры стены рядом с дыркой (UV попадания луча).
#
# /Applications/Blender.app/Contents/MacOS/Blender -b --python scripts/meshy_fix/patch_hole.py -- <вход.glb> <выход.glb> <знак стороны ±1> <y0> <y1> <z0> <z1>
# Окно поиска y0..y1, z0..z1 — в координатах Blender (Z вверх), как выводит скрипт при ошибке.
import bpy, bmesh, sys
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform

a = sys.argv[sys.argv.index('--') + 1:]
src, dst, side = a[0], a[1], float(a[2])
y0, y1, z0, z1 = map(float, a[3:7])
STEP, MARGIN, BEHIND = 0.002, 0.008, 0.001

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=src)
ob = [o for o in bpy.data.objects if o.type == 'MESH'][0]
me = ob.data
bm = bmesh.new(); bm.from_mesh(me); bm.faces.ensure_lookup_table()
uv_layer = bm.loops.layers.uv.active
bvh = BVHTree.FromBMesh(bm)
outer = max(abs(v.co.x) for v in bm.verts) + 0.3
d = Vector((-side, 0, 0))


def hit(y, z):
    loc, nor, idx, dist = bvh.ray_cast(Vector((side * outer, y, z)), d, 2 * outer)
    return (loc, idx) if idx is not None else (None, None)


# Плоскость стены — самые наружные попадания в окне поиска.
xs = []
rows = []
z = z0
while z <= z1:
    cols = []
    y = y0
    while y <= y1:
        loc, idx = hit(y, z)
        cols.append((y, None if loc is None else loc.x * side, idx, loc))
        y += STEP
    rows.append((z, cols))
    z += STEP
wall_x = max(c[1] for _, cols in rows for c in cols if c[1] is not None)
# Стена — попадание у наружной плоскости (у фронтона бывают ступеньки в пару сантиметров),
# дырка — луч ушёл внутрь модели.
is_wall = lambda c: c[1] is not None and c[1] > wall_x - 0.03
is_hole = lambda c: c[1] is None or c[1] < wall_x - 0.1

strip, sample = [], None  # (z, y левый, y правый, глубина стены)
for z, cols in rows:
    mid = len(cols) // 2
    if not is_hole(cols[mid]):
        continue
    li = mid
    while li > 0 and not is_wall(cols[li]):
        li -= 1
    ri = mid
    while ri < len(cols) - 1 and not is_wall(cols[ri]):
        ri += 1
    if not (is_wall(cols[li]) and is_wall(cols[ri])):
        continue
    depth = min(cols[li][1], cols[ri][1])
    strip.append((z, cols[li][0] + STEP - MARGIN, cols[ri][0] - STEP + MARGIN, depth))
    if sample is None and li >= 3 and is_wall(cols[li - 3]):
        sample = cols[li - 3]
if not strip:
    sys.exit('дырки в окне поиска нет')
print(f'стена x={wall_x * side:.4f}, дырка z {strip[0][0]:.3f}..{strip[-1][0]:.3f}, рядов {len(strip)}')

# Цвет: из точек стены в окне поиска берётся медианный цвет штукатурки, и выбирается точка
# текстуры ближе всего к нему, где карта нормалей ровная — заплатка светится как стена.
import numpy as np


def image_of(f, socket):
    m = ob.material_slots[f.material_index].material
    for link in m.node_tree.links:
        if link.to_socket.name == socket and link.from_node.type == 'TEX_IMAGE':
            return link.from_node.image
        if socket == 'Normal' and link.to_node.type == 'NORMAL_MAP' and link.from_node.type == 'TEX_IMAGE':
            return link.from_node.image
    return None


def pixels(im):
    arr = np.empty(im.size[0] * im.size[1] * 4, dtype=np.float32)
    im.pixels.foreach_get(arr)
    return arr.reshape(im.size[1], im.size[0], 4)


def uv_at(fi, loc):
    f = bm.faces[fi]
    tri = [l.vert.co for l in f.loops][:3]
    uvs = [Vector((*l[uv_layer].uv, 0)) for l in f.loops][:3]
    return barycentric_transform(loc, *tri, *uvs).to_2d()


wall_cells = [c for _, cols in rows for c in cols if is_wall(c)]
f = bm.faces[wall_cells[0][2]]
base = pixels(image_of(f, 'Base Color'))
nim = image_of(f, 'Normal')
nrm = pixels(nim) if nim else None
# Средний цвет по окрестности 33×33 текселя: точка у края островка текстуры в уменьшенной
# текстуре и на мипмапах смешается с соседями и посереет — нужна середина ровного участка.
R = 16
cs = np.pad(base[..., :3], ((R + 1, R), (R + 1, R), (0, 0)), mode='edge').cumsum(0).cumsum(1)
local = (cs[2 * R + 1:, 2 * R + 1:] - cs[:-2 * R - 1, 2 * R + 1:] - cs[2 * R + 1:, :-2 * R - 1]
         + cs[:-2 * R - 1, :-2 * R - 1]) / (2 * R + 1) ** 2
cand = []
for c in wall_cells:
    uv = uv_at(c[2], c[3])
    px = (min(base.shape[1] - 1, max(0, int(uv.x % 1 * base.shape[1]))),
          min(base.shape[0] - 1, max(0, int(uv.y % 1 * base.shape[0]))))
    col = base[px[1], px[0], :3]
    flat = 0.0
    if nrm is not None:
        nx = int(uv.x % 1 * nrm.shape[1]) % nrm.shape[1]; ny = int(uv.y % 1 * nrm.shape[0]) % nrm.shape[0]
        flat = abs(nrm[ny, nx, 0] - 0.5) + abs(nrm[ny, nx, 1] - 0.5)
    cand.append((uv, col, flat, local[px[1], px[0]]))
median = np.median(np.array([c[1] for c in cand]), axis=0)
uv, col, flat, _ = min(cand, key=lambda c: float(np.abs(c[1] - median).sum() + 2 * np.abs(c[3] - median).sum()) + 0.5 * c[2])
print(f'цвет стены {tuple(round(float(t), 3) for t in median)}, взят {tuple(round(float(t), 3) for t in col)}, наклон нормали {flat:.3f}')
f = bm.faces[wall_cells[0][2]]

mat = f.material_index
new_faces = []
def face(*vs):
    nf = bm.faces.new(vs); nf.material_index = mat; new_faces.append(nf)
pairs = []
for i, (z, yl, yr, depth) in enumerate(strip):
    zz = z - STEP * 0.5 if i == 0 else z
    px = side * (depth - BEHIND)
    pairs.append((bm.verts.new((px, yl, zz)), bm.verts.new((px, yr, zz))))
for (l0, r0), (l1, r1) in zip(pairs, pairs[1:]):
    face(l0, r0, r1); face(l0, r1, l1)
z, yl, yr, depth = strip[-1]
apex = bm.verts.new((side * (depth - BEHIND), (yl + yr) / 2, z + MARGIN))
face(pairs[-1][0], pairs[-1][1], apex)
for nf in new_faces:
    if nf.normal.x * side < 0:
        nf.normal_flip()
    for l in nf.loops:
        l[uv_layer].uv = uv
verts = new_faces
print(f'заплатка: {len(verts)} граней, UV {tuple(round(t, 3) for t in uv)}')
bm.to_mesh(me); me.update()
# Нормали заново для всей модели: у новых граней сохранённых нормалей нет, и они темнеют.
bpy.context.view_layer.objects.active = ob
if me.has_custom_normals:
    bpy.ops.mesh.customdata_custom_splitnormals_clear()
for p in me.polygons:
    p.use_smooth = True
me.use_auto_smooth = True
me.auto_smooth_angle = 0.5236
bpy.ops.export_scene.gltf(filepath=dst, export_format='GLB', export_image_format='AUTO', export_normals=True)
