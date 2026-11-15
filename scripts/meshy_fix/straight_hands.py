# Выпрямляет кисти в анимациях Meshy: ключи поворота кисти развёрнуты на 150–180° от позы покоя
# поперёк руки, и ладонь загибается к предплечью. Ключ заменяется на: поза покоя × (первый ключ
# клипа⁻¹ × ключ) с ослаблением DAMP — кисть продолжает предплечье, лёгкое движение запястья
# остаётся. Меняются только числа ключей в бинарном куске glb, остальное не трогается.
# python3 scripts/meshy_fix/straight_hands.py <вход.glb> <выход.glb> [DAMP] [узел ...]
# По умолчанию узлы mixamorig:LeftHand и mixamorig:RightHand, DAMP 0.5.
import json, struct, sys, math

src, dst = sys.argv[1], sys.argv[2]
DAMP = float(sys.argv[3]) if len(sys.argv) > 3 else 0.5
NODES = sys.argv[4:] or ['mixamorig:LeftHand', 'mixamorig:RightHand']


def mul(a, b):  # кватернионы (x, y, z, w)
    ax, ay, az, aw = a; bx, by, bz, bw = b
    return (aw*bx + ax*bw + ay*bz - az*by, aw*by - ax*bz + ay*bw + az*bx,
            aw*bz + ax*by - ay*bx + az*bw, aw*bw - ax*bx - ay*by - az*bz)


def inv(q):
    return (-q[0], -q[1], -q[2], q[3])


def damp(q, k):  # тот же поворот на долю k угла
    if q[3] < 0:
        q = tuple(-c for c in q)
    w = max(-1.0, min(1.0, q[3])); ang = 2 * math.acos(w); s = math.sin(ang / 2)
    if s < 1e-6:
        return (0.0, 0.0, 0.0, 1.0)
    ax = [c / s for c in q[:3]]; h = ang * k / 2
    return (ax[0]*math.sin(h), ax[1]*math.sin(h), ax[2]*math.sin(h), math.cos(h))


d = bytearray(open(src, 'rb').read())
jl = struct.unpack_from('<I', d, 12)[0]
j = json.loads(d[20:20 + jl])
bo = 20 + jl + 8  # начало бинарного куска
names = [n.get('name') for n in j['nodes']]
done = set()
for a in j['animations']:
    for ch in a['channels']:
        node = ch['target']['node']
        if names[node] not in NODES or ch['target']['path'] != 'rotation':
            continue
        acc_i = a['samplers'][ch['sampler']]['output']
        if acc_i in done:
            continue
        done.add(acc_i)
        acc = j['accessors'][acc_i]; bv = j['bufferViews'][acc['bufferView']]
        assert acc['componentType'] == 5126 and acc['type'] == 'VEC4'
        off = bo + bv.get('byteOffset', 0) + acc.get('byteOffset', 0)
        stride = bv.get('byteStride', 16)
        keys = [struct.unpack_from('<4f', d, off + k * stride) for k in range(acc['count'])]
        rest = tuple(j['nodes'][node].get('rotation', [0, 0, 0, 1]))
        ref_inv = inv(keys[0])
        for k, q in enumerate(keys):
            n = mul(rest, damp(mul(ref_inv, q), DAMP))
            l = math.sqrt(sum(c * c for c in n))
            struct.pack_into('<4f', d, off + k * stride, *(c / l for c in n))
        print(f'{a["name"]}: {names[node]} — {acc["count"]} ключей выпрямлено')
open(dst, 'wb').write(d)
