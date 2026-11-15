# Убирает мерцание плоскостей домов Meshy (Smart Topology). Две причины:
# 1. Нормали усреднены через несвязанные куски: большой треугольник на скате крыши или стене
#    светится не так, как соседние, и вспыхивает при повороте камеры (блестящая крыша — особенно).
# 2. Дубли поверхности: куски граней лежат друг на друге почти без зазора, камера не различает,
#    какой ближе, и они мерцают.
# Грань удаляется, только если параллельная грань ближе GAP закрывает её целиком (15 точек на
# треугольнике): той же стороной — нижняя, навстречу — смотрящая внутрь модели. Частично
# закрытые не трогаются — дыр не бывает. Затем нормали заново с разрывом по 30°: дома угловатые.
#
# /Applications/Blender.app/Contents/MacOS/Blender -b --python scripts/meshy_fix/stacked_faces.py -- <вход.glb> <выход.glb> [GAP доля размера]
import bpy, bmesh, sys, math
from mathutils.bvhtree import BVHTree

a = sys.argv[sys.argv.index('--') + 1:]
src, dst = a[0], a[1]
GAP = float(a[2]) if len(a) > 2 else 0.0005  # 0,5 мм на модель высотой 1 — около 3 мм в игре
ORDER = 4

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=src)


def samples(f):
    v = [l.vert.co for l in f.loops]
    c = f.calc_center_median()
    return [((v[0] * i + v[1] * j + v[2] * (ORDER - i - j)) / ORDER).lerp(c, 0.04)
            for i in range(ORDER + 1) for j in range(ORDER + 1 - i)]


for ob in [o for o in bpy.data.objects if o.type == 'MESH']:
    me = ob.data
    bm = bmesh.new(); bm.from_mesh(me)
    bmesh.ops.triangulate(bm, faces=bm.faces[:])
    bm.faces.ensure_lookup_table(); bm.verts.ensure_lookup_table()
    faces = bm.faces
    size = max(ob.dimensions); gap = size * GAP; flat = size * 1e-5
    centre = sum((v.co for v in bm.verts), bm.verts[0].co * 0) / len(bm.verts)
    bvh = BVHTree.FromBMesh(bm)
    outward = [f.normal.dot((f.calc_center_median() - centre).normalized()) for f in faces]
    doomed, kept = set(), set()
    for f in faces:
        if f.normal.length < 0.5 or f.index in kept:
            continue
        n = f.normal
        winners = set()
        for p in samples(f):
            win = None
            for loc, nor, idx, d in bvh.find_nearest_range(p, gap):
                if idx == f.index or idx in doomed:
                    continue
                g = faces[idx]
                dot = n.dot(g.normal)
                if abs(dot) < 0.95 or (p - loc).cross(n).length > flat:
                    continue  # не параллельна или точка не проецируется внутрь g
                if (dot > 0 and (loc - p).dot(n) > -flat) or (dot < 0 and outward[idx] > outward[f.index]):
                    win = idx; break
            if win is None:
                break
            winners.add(win)
        else:
            doomed.add(f.index); kept |= winners
    start = len(faces)
    bmesh.ops.delete(bm, geom=[faces[i] for i in doomed], context='FACES_ONLY')
    bmesh.ops.delete(bm, geom=[v for v in bm.verts if not v.link_faces], context='VERTS')
    bm.to_mesh(me); me.update()
    bpy.context.view_layer.objects.active = ob
    if me.has_custom_normals:
        bpy.ops.mesh.customdata_custom_splitnormals_clear()
    for p in me.polygons:
        p.use_smooth = True
    me.use_auto_smooth = True
    me.auto_smooth_angle = math.radians(30)
    print(f'{ob.name}: удалено закрытых дублей {len(doomed)} из {start} граней')

bpy.ops.export_scene.gltf(filepath=dst, export_format='GLB', export_image_format='AUTO', export_normals=True)
