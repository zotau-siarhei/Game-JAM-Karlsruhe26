# /Applications/Blender.app/Contents/MacOS/Blender -b --python scripts/meshy_fix/textures_png.py -- <вход.glb> <выход.glb> [SIZE] [папка для временных PNG]
# Уменьшает текстуры glb до SIZE и сохраняет их в PNG: Bevy в игре собран без JPEG.
# Картинка сохраняется в PNG-файл, загружается заново и подставляется в узлы материалов.
import bpy, sys, os
a = sys.argv[sys.argv.index('--') + 1:]
src, dst = a[0], a[1]
SIZE = int(a[2]) if len(a) > 2 else 1024
tmp = a[3] if len(a) > 3 else '/tmp'
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=src)
swap = {}
for i, im in enumerate(list(bpy.data.images)):
    w, h = im.size
    if w == 0:
        continue
    if w > SIZE or h > SIZE:
        k = SIZE / max(w, h)
        im.scale(max(1, int(w * k)), max(1, int(h * k)))
    path = os.path.join(tmp, f'img_{i}.png')
    im.filepath_raw = path
    im.file_format = 'PNG'
    im.save()
    new = bpy.data.images.load(path)
    new.colorspace_settings.name = im.colorspace_settings.name
    new.pack()
    swap[im.name] = new
for m in bpy.data.materials:
    if m.node_tree:
        for n in m.node_tree.nodes:
            if n.type == 'TEX_IMAGE' and n.image and n.image.name in swap:
                n.image = swap[n.image.name]
bpy.ops.export_scene.gltf(filepath=dst, export_format='GLB', export_image_format='AUTO', export_animations=True)
