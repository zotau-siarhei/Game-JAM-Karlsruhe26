# Дизайн 3D-моделей для Meshy

Источник правды — `openspec/changes/visual-style` (art-direction, models, rover-visuals), а также описания из `waves-alarm`, `residents`, `buildings-destruction` и `terrain-rover-camera`. Этот файл только переводит их в промпты для Meshy. Id совпадают с `assets/models/models.json`, бюджеты треугольников взяты из `visual-style/design.md`.

## Общий стиль (из art-direction)
- Яркая комедийная палитра: насыщенно, но не кислотно. Мультяшный характер, крупные детали, округлые формы, слегка преувеличенные пропорции. Не мрачно и без реалистичной жестокости.
- Силуэт узнаваем **сверху**: камера смотрит сверху с небольшим наклоном.
- Цвета сторон:
  - луноход — бирюзовый с белым;
  - враги — фиолетовый с жёлтыми деталями;
  - Служба защиты — белый с оранжевым, без мигалок, сирен, надписей и полицейской раскраски;
  - жители — пастельные, разноцветные;
  - **красный только для опасности**. В моделях красного нет вообще: ни на крышах, ни на вагонах.
- Поверхность планеты: зелёная, разрушенная — коричневая. Модели не должны сливаться с зелёным.

### Суффикс стиля (добавлять к каждому промпту)
```
stylized cartoon game asset, chunky rounded shapes, slightly exaggerated proportions, clean saturated colors, simple hand-painted textures, strong readable silhouette from a top-down camera, single object, centered, no text, no logos, no red color
```
Negative prompt (если поле есть): `realistic, gritty, dark, rusty, blood, text, logo, police, siren, red, thin details, wires`

### Настройки Meshy
- Art style: **Cartoon** / Stylized (если есть выбор). PBR: базовый цвет без металла.
- Target polycount и Remesh — по бюджету из таблицы ниже. Топология: triangle.
- Export: **GLB**. Модель смотрит вперёд по +Z (иначе исправляется `rotation_y` в `models.json`).
- Для согласованности: сначала сделать **одну эталонную модель (луноход)**, затем остальные через Image-to-3D или с тем же суффиксом. Каждый промпт и итог записываются в `SOURCES.md`.

## Модели по приоритету
Кредитов мало, поэтому идём сверху вниз. Модели из «Позже» делаются только если останется время.

### Обязательно (в кадре всегда)
| id | бюджет | промпт |
|---|---|---|
| `rover_body` | 15000 | `cute retro moon rover inspired by a 1970s lunar rover: round tub-shaped body like a bathtub with a hinged solar-panel lid on top, turquoise and white paint, a short camera mast with two big friendly round glass eyes, two springy antennas, a small rotating turret ring on top (empty, weapon attached separately), body only, no wheels or tracks` + суффикс |
| `rover_track` | 1500 (×2) | `one chunky cartoon tank track pod for a small rover: rounded rubber tread loop around four visible road wheels, turquoise side cover with white rim, side view length 2x height` + суффикс |
| `resident` | 800 | `small funny alien villager: capsule-shaped body, big round head with huge cute eyes, two short arms, one antenna with a little ball on top, pastel color, optional tiny scarf` + суффикс |
| `enemy_rammer` | 5000 | `small aggressive alien buggy with a heavy front ram bumper and one stubby gun, purple body with yellow details, big chunky wheels, angry headlights like eyes` + суффикс |
| `tree_1` | 1500 | `alien tree: short thick trunk, crown made of 2-3 round blobs in a strange color (violet, teal or orange), cartoon` + суффикс |
| `boulder` | 1500 | `round lumpy cartoon boulder, grey-violet stone, a few flat facets` + суффикс |
| `crystal` | 1500 | `cluster of tall glowing crystal prisms, cyan, emissive look` + суффикс |
| `house_small` | 4000 | `small cozy alien cottage: pastel walls, saturated colored round-ish roof (blue, yellow or violet, not red), round door and round windows` + суффикс |

### Желательно (волны после первой минуты)
| id | бюджет | промпт |
|---|---|---|
| `enemy_mortar` | 5000 | `squat alien artillery vehicle with a short fat mortar tube pointing up, purple body, yellow stripes, six chunky wheels` + суффикс |
| `enemy_rocket_robot` | 5000 | `two-legged walking alien robot with a shoulder rocket pod, purple armor plates, yellow visor, chunky legs` + суффикс |
| `service_car` | 4000 | `rounded white alien patrol car with orange stripes, big chunky wheels, a rotating dish antenna on the roof, friendly look, no lights, no sirens, no text` + суффикс |
| `house_two_story` | 4000 | `two-story alien house, pastel walls, saturated colored roof (not red), round windows, small balcony` + суффикс |
| `weapon_mg` / `weapon_mortar` / `weapon_rockets` | 1500 | `turret attachment for a cartoon rover: twin machine gun` / `short fat mortar tube` / `small 4-tube rocket launcher box` + `turquoise and white` + суффикс |

### Позже
| id | бюджет | промпт |
|---|---|---|
| `enemy_heavy_tank` | 5000 | `heavy slow alien tank with a big laser cannon, purple armor with yellow glowing details, wide tracks` + суффикс |
| `boss_colossus` | 15000 | `giant walking alien fortress on four mechanical legs, purple armor, yellow glowing core, mortar and rocket pods on its back` + суффикс |
| `train_loco` / `train_wagon` | 4000 | `chunky cartoon alien locomotive, pastel blue and yellow (not red)` / `matching cargo wagon` + суффикс |
| `house_barrack` | 4000 | `long low alien barrack, pastel walls, flat saturated roof` + суффикс |

Тяжёлый танк сделан по частям, чтобы башня поворачивалась, а гусеницы бежали (`src/tank_look.rs`).
Сначала картинка целого танка, потом по ней как образцу (Image References) — корпус и башня:
`Only the hull of the same alien tank as in the reference image, as a separate game part: NO turret and NO tracks...`,
`Only the turret ... one big laser cannon pointing straight forward, the cannon is centered and aligned with the turret`.
Гусеницу с образцом целого танка модель упорно рисует парой, поэтому её промпт — без образца:
`A single isolated tank track unit, one piece only... Only one object in the picture`. Лимит полигонов к ней
не применился (25 тыс.), её упростили в Blender модификатором Decimate до 2 тыс.

### Сделано 2026-11-15: поезда, дома, босс, оружие
Все через Image (2 варианта, взят первый) → Image to 3D, Smart Topology с бюджетом из таблицы. Бюджеты
соблюдены, упрощать не пришлось. Подключены через `src/models.rs`: `scale` и `offset_y` в `models.json`
подобраны под размеры из кода (`trains.rs`, `settlements.rs`, `boss.rs`) и проверены на снимках из игры.
У оружия масштаб 0,6, потому что корень лунохода сам увеличен вдвое. У вагонов перёд — +Z (как в `trains.rs`),
поэтому `rotation_y` у них +π/2, а не −π/2.

| id | треуг. | как сделана | куда смотрит в файле |
|---|---|---|---|
| `train_loco` | 3957 | `Chunky cartoon alien diesel locomotive ... pastel blue body with bright yellow stripes and a yellow rounded cab at the front` | кабина на −X, `rotation_y` ставит её на +Z, как в `trains.rs` |
| `train_wagon_passenger` / `_freight` / `_tank` | ~3980 | картинка локомотива в Image References: `A passenger railway car` / `A freight boxcar railway wagon` / `A tank car railway wagon matching the locomotive in the reference image` | вдоль X, симметричны |
| `house_two_story` | 3711 | `Two-story cozy alien house: pastel mint walls, saturated violet rounded roof, ... small balcony` | дверь на +Z |
| `house_barrack` | 3823 | `Long low one-story alien barrack: ... three times longer than wide, pastel peach walls, flat saturated teal roof` | длинный по X, повёрнут вдоль Z |
| `boss_body` | 7944 | сначала картинка целого босса (`Giant walking alien fortress boss ... four thick mechanical legs`), по ней `Only the body of the same boss robot ... NO legs at all` | ядро на +Z; низ туловища на высоте низа прежнего куба |
| `boss_leg` | 1572 | по картинке босса: `ONE single mechanical leg ... only one leg, not a pair` | стоит по Y, центр в начале координат |
| `weapon_mg` / `weapon_mortar` / `weapon_rockets` / `weapon_laser` | ~1500 | `Weapon attachment for the turret of a cute cartoon moon rover: twin machine gun` / `short fat mortar tube` / `rocket launcher box` / `sci-fi laser emitter` + `turquoise and white housing ... flat round mounting plate` | ствол на −X, у ракет на +X; мортира смотрит вверх |

`service_car` не делали: машины Службы защиты в коде пока нет.

## Первая планета (по гиту)
В гите нет отдельной «планеты с названием». Первая (и пока единственная) планета описана так:
- холмистая поверхность из шума, **зелёная** с оттенками по высоте, разрушенные места **коричневые** (`terrain-rover-camera`);
- инопланетные деревья с кроной из 2–3 шаров странного цвета, валуны, кусты, светящиеся кристаллы, рощи и россыпи (`buildings-destruction/props`);
- посёлки с пастельными домами и около 10 жителей в каждом, железная дорога с поездами и туннелями;
- рассеянный свет фиолетово-голубого неба (`visual-style`), под корой лава (`lava`).

Планеты из моего прошлого `GAMEDOC.md` (оранжевая GLOOP-7, ледяная FROSTBITE-3) в гите не описаны, поэтому здесь не используются.

## Решения по луноходу
1. **Гусеницы вместо колёс — решено.** `rover-visuals` и `design.md` в `visual-style` обновлены: два гусеничных блока по 4 катка, грунтозацепы — отдельные меши, глаза на мачте.
2. **Глаза** — добавлено требование «Глаза» в `rover-visuals`: смотрят на прицел, моргают, щурятся при попадании.
