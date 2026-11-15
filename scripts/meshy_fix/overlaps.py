# Убирает мерцание листвы: из пар почти совпадающих параллельных граней (между ними меньше
# EPS от размера модели) удаляется внутренняя — ближе к центру. Внешняя закрывает то же место.
# /Applications/Blender.app/Contents/MacOS/Blender -b --python scripts/meshy_fix/overlaps.py -- <вход.glb> <выход.glb> [доля EPS]
import bpy, bmesh, sys
from mathutils.bvhtree import BVHTree
a = sys.argv[sys.argv.index('--') + 1:]
src, dst = a[0], a[1]
EPS = float(a[2]) if len(a) > 2 else 0.0015
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=src)
for ob in [o for o in bpy.data.objects if o.type == 'MESH']:
    me = ob.data
    bm = bmesh.new(); bm.from_mesh(me); bm.faces.ensure_lookup_table(); bm.verts.ensure_lookup_table()
    bvh = BVHTree.FromBMesh(bm)
    size = max(ob.dimensions)
    eps = size * EPS
    center = sum((v.co for v in bm.verts), bm.verts[0].co * 0) / len(bm.verts)
    verts_of = [set(v.index for v in f.verts) for f in bm.faces]
    doomed = set()
    for f in bm.faces:
        if f.index in doomed:
            continue
        c = f.calc_center_median()
        # Центр и точки на полпути к вершинам: ловит и частичное наложение.
        samples = [c] + [c.lerp(v.co, 0.6) for v in f.verts]
        for p in samples:
            hit = None
            for (loc, nor, idx, dist) in bvh.find_nearest_range(p, eps):
                if idx == f.index or idx in doomed or verts_of[f.index] & verts_of[idx]:
                    continue
                if abs(f.normal.dot(bm.faces[idx].normal)) < 0.9:
                    continue
                hit = idx
                break
            if hit is not None:
                g = bm.faces[hit]
                inner = f if (c - center).length < (g.calc_center_median() - center).length else g
                doomed.add(inner.index)
                break
    before = len(bm.faces)
    bmesh.ops.delete(bm, geom=[bm.faces[i] for i in doomed], context='FACES_ONLY')
    loose = [v for v in bm.verts if not v.link_faces]
    bmesh.ops.delete(bm, geom=loose, context='VERTS')
    bm.to_mesh(me); me.update()
    print(f'{ob.name}: удалено {len(doomed)} граней из {before} ({len(doomed)/before*100:.1f}%)')
bpy.ops.export_scene.gltf(filepath=dst, export_format='GLB', export_image_format='AUTO')
