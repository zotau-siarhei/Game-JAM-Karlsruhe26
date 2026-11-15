# Выносит колёса модели поезда в отдельные узлы, чтобы игра их крутила (visual-style 6.4).
# Колесо у Meshy — отдельный связный кусок сетки: круглый сбоку, тонкий поперёк, у самого низа.
# Каждое колесо становится узлом `wheel_<n>_r<радиус × 10000>` с началом в центре колеса;
# форма и положение модели не меняются. Клип `Animation` — оборот колёс за секунду, для
# просмотра в каталоге; игра его не запускает, колёса крутит `trains::turn_wheels`.
# /Applications/Blender.app/Contents/MacOS/Blender -b --python scripts/meshy_fix/split_wheels.py -- <вход.glb> <выход.glb>
import bpy, bmesh, sys, math
from mathutils import Vector

a = sys.argv[sys.argv.index('--') + 1:]
src, dst = a[0], a[1]
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=src)
car = [o for o in bpy.data.objects if o.type == 'MESH'][0]
me = car.data
bm = bmesh.new(); bm.from_mesh(me)
bm.verts.ensure_lookup_table()
# Связные куски по рёбрам; вершины на швах UV Meshy разделены, поэтому сшиваем по положению.
key = lambda co: (round(co.x, 5), round(co.y, 5), round(co.z, 5))
groups = {}
for v in bm.verts:
    groups.setdefault(key(v.co), []).append(v)
parent = {}
def find(x):
    while parent.get(x, x) != x:
        x = parent[x]
    return x
def union(a, b):
    ra, rb = find(a), find(b)
    if ra != rb:
        parent[ra] = rb
for e in bm.edges:
    union(key(e.verts[0].co), key(e.verts[1].co))
comps = {}
for f in bm.faces:
    comps.setdefault(find(key(f.verts[0].co)), []).append(f)
zmin = min(v.co.z for v in bm.verts); zmax = max(v.co.z for v in bm.verts)
wheels = []
for faces in comps.values():
    vs = {v for f in faces for v in f.verts}
    lo = Vector((min(v.co.x for v in vs), min(v.co.y for v in vs), min(v.co.z for v in vs)))
    hi = Vector((max(v.co.x for v in vs), max(v.co.y for v in vs), max(v.co.z for v in vs)))
    ext = hi - lo
    round_side = 0.75 < ext.x / max(ext.z, 1e-6) < 1.33
    thin = ext.y < 0.6 * max(ext.x, ext.z)
    low = lo.z < zmin + 0.25 * (zmax - zmin)
    if round_side and thin and low and ext.z > 0.03:
        wheels.append((faces, (lo + hi) * 0.5, (ext.x + ext.z) * 0.25))
if not wheels:
    sys.exit('колёс нет')
# Колёса — самые крупные круглые куски; мелкие на той же оси и с той же стороны (ступицы)
# крутятся вместе с колесом, остальные (пружины и рама тележки) остаются в кузове.
rmax = max(r for _, _, r in wheels)
primary = [[list(f), c, r] for f, c, r in wheels if r >= 0.8 * rmax]
for faces, c, r in wheels:
    if r >= 0.8 * rmax:
        continue
    for p in primary:
        pc, pr = p[1], p[2]
        if abs(c.x - pc.x) < pr * 0.5 and abs(c.z - pc.z) < pr * 0.5 and c.y * pc.y > 0:
            p[0].extend(faces)
            break
wheels = [(f, c, r) for f, c, r in primary]
print(f'колёс найдено: {len(wheels)}')

# Каждое колесо — в свой объект: копия сетки без чужих граней.
wheel_faces = {f.index for faces, _, _ in wheels for f in faces}
objs = []
for i, (faces, centre, radius) in enumerate(sorted(wheels, key=lambda w: (w[1].x, w[1].y))):
    keep = {f.index for f in faces}
    wm = me.copy()
    wb = bmesh.new(); wb.from_mesh(wm); wb.faces.ensure_lookup_table()
    bmesh.ops.delete(wb, geom=[f for f in wb.faces if f.index not in keep], context='FACES')
    bmesh.ops.translate(wb, verts=wb.verts, vec=-centre)
    wb.to_mesh(wm); wb.free()
    ob = bpy.data.objects.new(f'wheel_{i}_r{round(radius * 10000)}', wm)
    bpy.context.collection.objects.link(ob)
    ob.location = car.matrix_world @ centre
    ob.parent = car
    ob.matrix_parent_inverse = car.matrix_world.inverted()
    ob.location = centre
    objs.append(ob)
bmesh.ops.delete(bm, geom=[f for f in bm.faces if f.index in wheel_faces], context='FACES')
bm.to_mesh(me); bm.free()

# Клип оборота колёс вокруг поперечной оси (Y Blender) — только для каталога.
scene = bpy.context.scene
scene.frame_start, scene.frame_end = 0, 24
scene.render.fps = 24
for ob in objs:
    ob.rotation_mode = 'XYZ'
    ob.rotation_euler = (0, 0, 0); ob.keyframe_insert('rotation_euler', frame=0)
    ob.rotation_euler = (0, -2 * math.pi, 0); ob.keyframe_insert('rotation_euler', frame=24)
    for fc in ob.animation_data.action.fcurves:
        for k in fc.keyframe_points:
            k.interpolation = 'LINEAR'
    ob.animation_data.action.name = 'Roll'
for im in bpy.data.images:
    im.file_format = 'PNG'
bpy.ops.export_scene.gltf(filepath=dst, export_format='GLB', export_image_format='AUTO',
                          export_animations=True, export_nla_strips=False, export_force_sampling=True)
print('готово', dst)
