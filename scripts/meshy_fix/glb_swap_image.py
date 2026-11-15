# Заменяет картинку в glb (бинарный кусок пересобирается, смещения bufferView сдвигаются).
# python3 scripts/meshy_fix/glb_swap_image.py <вход.glb> <номер картинки> <новая.png> <выход.glb>
import json, struct, sys
src, idx, png, dst = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4]
d = open(src, 'rb').read()
jl = struct.unpack('<I', d[12:16])[0]
j = json.loads(d[20:20 + jl])
bl = struct.unpack('<I', d[20 + jl:24 + jl])[0]
binc = d[28 + jl:28 + jl + bl]
new_img = open(png, 'rb').read()
target = j['images'][idx]['bufferView']
j['images'][idx]['mimeType'] = 'image/png'
out = bytearray()
for i, bv in enumerate(j['bufferViews']):
    o = bv.get('byteOffset', 0)
    data = new_img if i == target else binc[o:o + bv['byteLength']]
    while len(out) % 4:
        out.append(0)
    bv['byteOffset'] = len(out)
    bv['byteLength'] = len(data)
    out += data
while len(out) % 4:
    out.append(0)
j['buffers'][0]['byteLength'] = len(out)
js = json.dumps(j, separators=(',', ':')).encode()
js += b' ' * (-len(js) % 4)
total = 12 + 8 + len(js) + 8 + len(out)
with open(dst, 'wb') as f:
    f.write(struct.pack('<III', 0x46546C67, 2, total))
    f.write(struct.pack('<II', len(js), 0x4E4F534A)); f.write(js)
    f.write(struct.pack('<II', len(out), 0x004E4942)); f.write(out)
print(dst, total)
