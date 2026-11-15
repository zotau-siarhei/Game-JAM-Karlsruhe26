# Закрывает щель по середине протектора колеса (Meshy оставил шов открытым, в двух местах
# он расходится в дыру): края шва сшиваются мостом, новым граням достаются UV соседей.
# /Applications/Blender.app/Contents/MacOS/Blender -b --python scripts/meshy_fix/wheel_seam.py -- <вход.glb> <выход.glb>
import bpy, bmesh, sys, math
a = sys.argv[sys.argv.index('--') + 1:]
src, dst = a[0], a[1]
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=src)
ob = [o for o in bpy.data.objects if o.type == 'MESH'][0]
bpy.context.view_layer.objects.active = ob
ob.select_set(True)
me = ob.data
if me.has_custom_normals:
    bpy.ops.mesh.customdata_custom_splitnormals_clear()
bm = bmesh.new(); bm.from_mesh(me)
bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-5)
bm.faces.ensure_lookup_table()
# Щель по всему протектору: обе её кромки сшиваются целиком, вместе с двумя дырами.
boundary = [e for e in bm.edges if e.is_boundary]
res = bmesh.ops.bridge_loops(bm, edges=boundary)
new_set = set(res['faces'])
for f in bm.faces:
    f.select = f in new_set
uv = bm.loops.layers.uv.active
new_faces = [f for f in bm.faces if f.select]
# Цвет заплатки — ровный цвет соседней резины: UV всех углов новой грани в центр UV
# старой грани за её ребром. Иначе углы попадают в разные острова атласа и текстура
# растягивается полосами.
for f in new_faces:
    olds = [g for e in f.edges for g in e.link_faces if not g.select]
    if not olds:
        continue
    src = olds[0]
    c = sum((l[uv].uv for l in src.loops), src.loops[0][uv].uv * 0) / len(src.loops)
    for loop in f.loops:
        loop[uv].uv = c.copy()
bound = sum(1 for e in bm.edges if e.is_boundary)
print(f'новых граней {len(new_faces)}, граничных рёбер осталось {bound}')
bm.to_mesh(me); bm.free()
for p in me.polygons:
    p.use_smooth = True
me.use_auto_smooth = True
me.auto_smooth_angle = math.radians(40)
bpy.ops.export_scene.gltf(filepath=dst, export_format='GLB', export_image_format='AUTO')
