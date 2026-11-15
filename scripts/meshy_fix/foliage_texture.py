# Убирает белёсые пятна с листвы: Meshy запёк в текстуру светлый фон картинки-образца, и на
# кончиках веток проступает белое. Бледные малонасыщенные пиксели перекрашиваются в цвет
# соседней листвы, рисунок (светлота) сохраняется.
# /Applications/Blender.app/Contents/MacOS/Blender -b --python scripts/meshy_fix/foliage_texture.py -- <вход.png> <выход.png> [маска.png]
import bpy, sys
import numpy as np

a = sys.argv[sys.argv.index('--') + 1:]
src, dst = a[0], a[1]
mask_out = a[2] if len(a) > 2 else None

im = bpy.data.images.load(src)
w, h = im.size
px = np.array(im.pixels[:], dtype=np.float32).reshape(h, w, 4)
rgb = px[..., :3]

mx = rgb.max(-1)
mn = rgb.min(-1)
sat = (mx - mn) / np.maximum(mx, 1e-4)


def smooth(x, a, b):
    t = np.clip((x - a) / (b - a), 0.0, 1.0)
    return t * t * (3 - 2 * t)


# Насколько пиксель «белёсый»: светлый и почти без цвета. Серая кора темнее — её не трогает.
pale = smooth(mx, 0.38, 0.52) * (1.0 - smooth(sat, 0.16, 0.30))
hard = pale > 0.5
# Источники цвета: живая листва и кора, без чёрного фона атласа.
# Вес — по насыщенности: цвет берётся у сочной листвы, а не у полубледной каймы пятна.
src_w = ((pale < 0.05) & (mx > 0.05)).astype(np.float32) * (0.05 + smooth(sat, 0.28, 0.5))


def blur(x):
    p = np.pad(x, ((1, 1), (1, 1), (0, 0)), mode='edge')
    return sum(p[i:i + x.shape[0], j:j + x.shape[1]] for i in range(3) for j in range(3)) / 9.0


def push_pull(color, weight):
    """Заливает пустые места цветом ближайших источников: пирамида вниз и вверх."""
    levels = [(color * weight[..., None], weight)]
    c, wt = levels[0]
    while min(c.shape[0], c.shape[1]) > 1:
        hh, ww = c.shape[0] // 2 * 2, c.shape[1] // 2 * 2
        c = c[:hh, :ww].reshape(hh // 2, 2, ww // 2, 2, 3).sum((1, 3))
        wt = wt[:hh, :ww].reshape(hh // 2, 2, ww // 2, 2).sum((1, 3))
        levels.append((c, wt))
    fill = levels[-1][0] / np.maximum(levels[-1][1], 1e-6)[..., None]
    for c, wt in reversed(levels[:-1]):
        up = blur(np.repeat(np.repeat(fill, 2, 0), 2, 1))
        up = np.pad(up, ((0, c.shape[0] - up.shape[0]), (0, c.shape[1] - up.shape[1]), (0, 0)), mode='edge')
        own = c / np.maximum(wt, 1e-6)[..., None]
        k = np.clip(wt * 4, 0, 1)[..., None]
        fill = own * k + up * (1 - k)
    return fill


fill = push_pull(rgb, src_w)
# Рисунок листвы: светлота пятна относительно его средней — немного вариаций, без плоской заливки.
lum = rgb @ np.array([0.3, 0.55, 0.15], dtype=np.float32)
mean = lum[hard].mean() if hard.any() else 1.0
detail = np.clip(0.82 + 0.3 * (lum / mean - 1.0), 0.65, 0.98)[..., None]
# Заливка чуть насыщеннее: усреднение по пирамиде гасит цвет.
grey = fill.mean(-1, keepdims=True)
fill = grey + (fill - grey) * 1.3
new = np.clip(fill * detail, 0, 1)
k = pale[..., None]
px[..., :3] = rgb * (1 - k) + new * k

print(f'{src}: белёсых пикселей {hard.mean() * 100:.1f}% атласа')
im.pixels[:] = px.ravel()
im.filepath_raw = dst
im.file_format = 'PNG'
im.save()

if mask_out:
    m = bpy.data.images.new('mask', w, h)
    mp = np.zeros((h, w, 4), np.float32)
    mp[..., 0] = mp[..., 1] = mp[..., 2] = pale
    mp[..., 3] = 1
    m.pixels[:] = mp.ravel()
    m.filepath_raw = mask_out
    m.file_format = 'PNG'
    m.save()
