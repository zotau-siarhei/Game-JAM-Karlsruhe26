# Переподвешивает кости в glb к новому родителю, сохраняя их положение в покое.
# Meshy оставил корни пальцев робота без родителя: ладонь уходит, пальцы висят на месте.
# Blender -b --python reparent.py -- <вход.glb> <выход.glb> <кость>=<новый родитель> ...
# (Blender — ради mathutils; сам glb правится как JSON, сетка, скин и клипы не трогаются.)
import json, struct, sys
from mathutils import Matrix, Quaternion, Vector
a = sys.argv[sys.argv.index('--') + 1:]
src, dst, pairs = a[0], a[1], [p.split('=') for p in a[2:]]
d = open(src, 'rb').read()
jl = struct.unpack('<I', d[12:16])[0]
j = json.loads(d[20:20 + jl])
rest = d[20 + jl:]
nodes = j['nodes']
by_name = {n.get('name'): i for i, n in enumerate(nodes)}
parent = {}
for i, n in enumerate(nodes):
    for c in n.get('children', []):
        parent[c] = i

def local(n):
    if 'matrix' in n:
        m = n['matrix']; return Matrix([m[0:4], m[4:8], m[8:12], m[12:16]]).transposed()
    t = Vector(n.get('translation', [0, 0, 0]))
    r = n.get('rotation', [0, 0, 0, 1]); q = Quaternion((r[3], r[0], r[1], r[2]))
    s = n.get('scale', [1, 1, 1])
    return Matrix.LocRotScale(t, q, Vector(s))

def world(i):
    m = local(nodes[i])
    while i in parent:
        i = parent[i]; m = local(nodes[i]) @ m
    return m

for bone, new_parent in pairs:
    b, p = by_name[bone], by_name[new_parent]
    wb, wp = world(b), world(p)
    old = parent.get(b)
    if old is not None:
        nodes[old]['children'].remove(b)
    else:
        for sc in j['scenes']:
            if b in sc['nodes']: sc['nodes'].remove(b)
    nodes[p].setdefault('children', []).append(b)
    parent[b] = p
    m = wp.inverted() @ wb
    t, q, s = m.decompose()
    n = nodes[b]; n.pop('matrix', None)
    n['translation'] = list(t); n['rotation'] = [q.x, q.y, q.z, q.w]; n['scale'] = list(s)
    err = (world(b) - wb)
    print(f'{bone}: было под {nodes[old]["name"] if old is not None else "корнем"}, теперь под {new_parent}; '
          f'сдвиг в покое {max(abs(x) for row in err for x in row):.2e}')

js = json.dumps(j, separators=(',', ':')).encode()
js += b' ' * (-len(js) % 4)
with open(dst, 'wb') as f:
    f.write(struct.pack('<III', 0x46546C67, 2, 12 + 8 + len(js) + len(rest)))
    f.write(struct.pack('<II', len(js), 0x4E4F534A)); f.write(js); f.write(rest)
