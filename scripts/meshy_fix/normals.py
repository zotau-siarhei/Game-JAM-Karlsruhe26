# Пересчитывает нормали листвы: Meshy усреднил их через тонкие листовые пластины, смотрящие
# в разные стороны, и с двусторонним материалом грани мигают светом и тенью при повороте.
# Нормали считаются заново с разрывом по углу: каждая грань светится по своей стороне.
# /Applications/Blender.app/Contents/MacOS/Blender -b --python scripts/meshy_fix/normals.py -- <вход.glb> <выход.glb> [угол, градусы]
import bpy, sys, math
a = sys.argv[sys.argv.index('--') + 1:]
src, dst = a[0], a[1]
angle = math.radians(float(a[2]) if len(a) > 2 else 30.0)
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=src)
for ob in [o for o in bpy.data.objects if o.type == 'MESH']:
    me = ob.data
    bpy.context.view_layer.objects.active = ob
    if me.has_custom_normals:
        bpy.ops.mesh.customdata_custom_splitnormals_clear()
    for p in me.polygons:
        p.use_smooth = True
    me.use_auto_smooth = True
    me.auto_smooth_angle = angle
    me.calc_normals_split()
    mixed = sum(any(p.normal.dot(me.loops[li].normal) < 0 for li in p.loop_indices) for p in me.polygons)
    print(f'{ob.name}: граней {len(me.polygons)}, с нормалью против грани осталось {mixed}')
bpy.ops.export_scene.gltf(filepath=dst, export_format='GLB', export_image_format='AUTO', export_normals=True)
