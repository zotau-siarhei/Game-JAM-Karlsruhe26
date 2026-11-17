# Дизайн 3D-моделей для Meshy

Источник правды — `openspec/changes/visual-style` (art-direction, models, rover-visuals), а также описания из `waves-alarm`, `residents`, `buildings-destruction` и `terrain-rover-camera`. Этот файл только переводит их в промпты для Meshy. Id совпадают с `assets/models/models.json`, бюджеты треугольников взяты из `visual-style/design.md`.

## Общий стиль (из art-direction)
- Яркая комедийная палитра: насыщенно, но не кислотно. Мультяшный характер, крупные детали, округлые формы, слегка преувеличенные пропорции. Не мрачно и без реалистичной жестокости.
- Силуэт узнаваем **сверху**: камера смотрит сверху с небольшим наклоном.
- Цвета сторон:
  - луноход — бирюзовый с белым;
  - враги — тёмный графит с янтарно-жёлтыми метками, реалистичная боевая техника (решение 2026-11-15, раздел «Враги» ниже);
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

### Цвета обломков
Текстуры моделей в игре живут только в видеопамяти, поэтому цвета для обломков (дом, вагон, валун, кварц)
посчитаны заранее: после добавления или замены модели запустить `python3 scripts/model_colors.py` — он
пишет в `models.json` поля `colors` (низ модели) и `top_colors` (верх, крыша). Тест `repository_models_have_colors`
падает, если у записи их нет.

## Модели по приоритету
Кредитов мало, поэтому идём сверху вниз. Модели из «Позже» делаются только если останется время.

### Обязательно (в кадре всегда)
| id | бюджет | промпт |
|---|---|---|
| `rover_body` | 15000 | `cute retro moon rover inspired by a 1970s lunar rover: round tub-shaped body like a bathtub with a hinged solar-panel lid on top, turquoise and white paint, a short camera mast with two big friendly round glass eyes, two springy antennas, a small rotating turret ring on top (empty, weapon attached separately), body only, no wheels or tracks` + суффикс |
| `rover_track` | 1500 (×2) | `one chunky cartoon tank track pod for a small rover: rounded rubber tread loop around four visible road wheels, turquoise side cover with white rim, side view length 2x height` + суффикс |
| `boulder` | 1500 | `round lumpy cartoon boulder, grey-violet stone, a few flat facets` + суффикс |
| `crystal` | 1500 | `cluster of tall glowing crystal prisms, cyan, emissive look` + суффикс |
| `house_small` | 4000 | `small cozy alien cottage: pastel walls, saturated colored round-ish roof (blue, yellow or violet, not red), round door and round windows` + суффикс |

### Желательно (волны после первой минуты)
| id | бюджет | промпт |
|---|---|---|
| `service_car` | 4000 | `rounded white alien patrol car with orange stripes, big chunky wheels, a rotating dish antenna on the roof, friendly look, no lights, no sirens, no text` + суффикс |
| `house_two_story` | 4000 | `two-story alien house, pastel walls, saturated colored roof (not red), round windows, small balcony` + суффикс |
| `weapon_mg` / `weapon_mortar` / `weapon_rockets` | 1500 | `turret attachment for a cartoon rover: twin machine gun` / `short fat mortar tube` / `small 4-tube rocket launcher box` + `turquoise and white` + суффикс |
| `enemy_interceptor` | 5000 | `A long low narrow interceptor dragster, enemy vehicle for a top-down action game, semi-realistic hard-surface 3D game asset: needle-like wedge nose with two forward stun prongs ending in glowing pale-blue electrode balls, small crackling tesla coil on the roof, length four times the width, two jet nozzles at the back, four small wheels set far apart at the ends, dark graphite armor with amber-yellow racing stripes and small amber sensor lights, no guns, no cannons, single vehicle, centered, 3/4 view from above, plain light grey background, no text, no logos, no red` — в стиле врагов из раздела «Враги»; корпус 1,0 × 4,4 м, нос по ходу (в `models.json` подогнать `scale` и `rotation_y`) |

### Позже
| id | бюджет | промпт |
|---|---|---|
| `train_loco` / `train_wagon` | 4000 | `chunky cartoon alien locomotive, pastel blue and yellow (not red)` / `matching cargo wagon` + суффикс |
| `house_barrack` | 4000 | `long low alien barrack, pastel walls, flat saturated roof` + суффикс |

Тяжёлый танк (`enemy_tank_hull`, `_turret`, `_track`) и босс (`boss_body`, `boss_leg`) переделаны 2026-11-15 в стиле новых врагов — см. раздел «Враги» ниже.

### Сделано 2026-11-15: поезда, дома, оружие (первый босс заменён, см. «Враги»)
Все через Image (2 варианта, взят первый) → Image to 3D, Smart Topology с бюджетом из таблицы. Бюджеты
соблюдены, упрощать не пришлось. Подключены через `Models::spawn_visual` (`src/models.rs`), примитивы — запасной облик. `scale` и `offset_y` в `models.json`
подобраны под размеры из кода (`trains.rs`, `settlements.rs`, `boss.rs`) и проверены на снимках из игры.
У оружия масштаб 0,6, потому что корень лунохода сам увеличен вдвое. У вагонов перёд — +Z (как в `trains.rs`),
поэтому `rotation_y` у них +π/2, а не −π/2.

| id | треуг. | как сделана | куда смотрит в файле |
|---|---|---|---|
| `train_loco` | 3957 | `Chunky cartoon alien diesel locomotive ... pastel blue body with bright yellow stripes and a yellow rounded cab at the front` | кабина на −X, `rotation_y` ставит её на +Z, как в `trains.rs` |
| `train_wagon_passenger` / `_freight` / `_tank` | ~3980 | картинка локомотива в Image References: `A passenger railway car` / `A freight boxcar railway wagon` / `A tank car railway wagon matching the locomotive in the reference image` | вдоль X, симметричны |
| `house_two_story` | 3711 | `Two-story cozy alien house: pastel mint walls, saturated violet rounded roof, ... small balcony` | дверь на +Z |
| `house_barrack` | 3823 | `Long low one-story alien barrack: ... three times longer than wide, pastel peach walls, flat saturated teal roof` | длинный по X, повёрнут вдоль Z |
| `weapon_mg` / `weapon_mortar` / `weapon_rockets` / `weapon_laser` | ~1500 | `Weapon attachment for the turret of a cute cartoon moon rover: twin machine gun` / `short fat mortar tube` / `rocket launcher box` / `sci-fi laser emitter` + `turquoise and white housing ... flat round mounting plate` | ствол на −X, у ракет на +X; мортира смотрит вверх |

`service_car` не делали: машины Службы защиты в коде пока нет.

### Деревья и кусты (2026-11-15)
В одном стиле с людьми, без мультяшного суффикса. Image (Nano Banana Pro, 3:4, 2 картинки) →
Image to 3D: Smart Topology, Poly Count 3000 (куст 1500), Pose None. Настройки Image to 3D на странице
картинок каждый раз сбрасываются на High Detail — выставлять заново. Общий хвост промпта:
`semi-realistic 3D game asset, natural earthy colors, whole tree visible from the base to the top, centered,
3/4 view from slightly above, plain light grey background, no ground, no grass, no shadow, no text`.

| id | треуг. | промпт (начало) | высота в игре |
|---|---|---|---|
| `tree_oak` | 2846 | `A single realistic deciduous broadleaf tree (oak): thick brown trunk with visible roots flare, a full rounded crown of natural green leaves made of several clumps` | 6 м |
| `tree_spruce` | 2326 | `A single realistic Norway spruce tree: conical shape with layered drooping dark green branches` | 6 м |
| `tree_poplar` | 2548 | `A single realistic Lombardy poplar tree: straight brown-grey trunk, tall narrow columnar crown` | 7 м |
| `tree_pine` | 2669 | `A single realistic Scots pine tree: tall straight orange-brown trunk bare in the lower half, a dense flat-topped dark green needle crown` | 6,5 м |
| `bush` | 1175 | `A single realistic leafy green shrub bush: wide rounded dense bush` | 1,1 м |

Кроны сделаны пластинами листвы, изнанка пластин светлая — сверху почти не видна. Второй куст вышел
стопкой белых пластин, его не взяли. Модели в файле по центру, высота ≈ 0,95: `scale` = высота / 0,952,
`offset_y` = 0,476 × `scale`.

### Жители-люди (2026-11-15)
Решение пользователя: жители — как настоящие люди (`residents`, раздел 7). Мультяшный `resident` удалён.
Image: Pose = A-Pose, 3:4, промпт `realistic adult man / woman / elderly man / young woman, casual everyday clothes in earthy colors,
full body, A-pose, plain light grey background`. Image to 3D: Smart Topology 6000, Pose A-Pose. Animate → Rig → Humanoid,
рост 1,7 м, метки суставов ставятся сами (метку «Groin» не трогать: это тазобедренный сустав). Анимации из библиотеки
(бесплатно): Idle 1, Walking, Running, Talk with Hands Open. Скачивание: glb, Mixamo, All Added, Single file.

| id | треуг. | клипы | куда смотрит |
|---|---|---|---|
| `resident_man`, `resident_woman`, `resident_elder`, `resident_young` | ~6000 | `Idle_02`, `Walking`, `Running`, `Talk_with_Hands_Open` | перёд +Z, `rotation_y` = π; ступни на y = 0, рост в метрах |

Скины: `scripts/recolor_people.py` (Blender) перекрашивает текстуру по классам цветов (кожа, волосы, верх, низ),
верх и низ делят UV-острова меша по большинству; результат — `assets/models/skins/<id>_<1..7>.png`, 512 px.

### Враги (2026-11-15)
Решение пользователя: фиолетовые мультяшные машинка и робот не подходят — враги в одном стиле с людьми
(`visual-style` 7.3). Без мультяшного суффикса. Машины сделаны по частям: сначала картинка целой машины,
потом по ней как образцу (Image References) корпус без колёс и одно колесо. Колесо общее для обеих машин.

| id | треуг. | промпт |
|---|---|---|
| — (образец) | — | `A compact armored combat buggy, enemy vehicle for a top-down action game, semi-realistic hard-surface 3D game asset: low angular dark graphite armor plates, a heavy steel ram plow on the front bumper, a small machine gun turret on the roof pointing forward, four big off-road wheels, amber-yellow warning markings and small amber sensor lights, rugged military look, single vehicle, centered, 3/4 view from above, plain light grey background, no text, no logos, no red` |
| `enemy_rammer` | 3888 | `Only the body of the same armored buggy as in the reference image, as a separate game part: NO wheels at all, empty wheel arches, the body hovers with nothing under it, keep the front ram plow and the roof machine gun turret pointing forward, same dark graphite armor and amber-yellow markings, semi-realistic hard-surface 3D game asset, 3/4 view from above, plain light grey background, no text, no red` |
| — (образец) | — | `A six-wheeled armored mortar carrier, enemy vehicle for a top-down action game, semi-realistic hard-surface 3D game asset: boxy dark graphite armored body, a short thick mortar tube on a turntable at the rear pointing up at 45 degrees, six big off-road wheels, amber-yellow warning stripes and small amber sensor lights, rugged military look, single vehicle, centered, 3/4 view from above, plain light grey background, no text, no logos, no red` |
| `enemy_mortar` | 3594 | `Only the body of the same six-wheeled armored mortar carrier as in the reference image, as a separate game part: NO wheels at all, empty wheel arches, the body hovers with nothing under it, keep the mortar tube on its turntable at the rear, same dark graphite armor and amber-yellow markings, semi-realistic hard-surface 3D game asset, 3/4 view from above, plain light grey background, no text, no red` |
| `enemy_wheel` | 1476 | `ONE single wheel of the same armored vehicle as in the reference image, as a separate game part: chunky black off-road tire with deep tread and a dark graphite steel rim with amber-yellow bolts, only one wheel, no vehicle, no axle, side 3/4 view, semi-realistic 3D game asset, centered, plain light grey background, no text` |
| ~~`enemy_rocket_robot`~~ (убран 2026-11-17, `laser-tank`) | 5934 | `A humanoid combat robot soldier, enemy for a top-down action game, semi-realistic sci-fi hard-surface 3D game asset: human proportions, dark graphite armor plates over a grey mechanical frame, amber-yellow warning markings, a single glowing amber visor slit on the head, a boxy four-tube rocket launcher pod mounted on its right shoulder, sturdy mechanical legs with feet, full body, standing in A-pose, front view, plain light grey background, no text, no logos, no red` |

- Корпуса смотрят носом на −X (`rotation_y` = −π/2). Колёсные арки измерены по профилю модели, оси, колея и
  радиус колёс записаны в `src/enemy_look.rs` (`RAMMER_WHEELS`, `MORTAR_WHEELS`) до масштаба врага (`RAMMER_SCALE`, `MORTAR_SCALE` в `src/enemy.rs`): при замене модели их надо
  перемерить.
- Колесо: ось вдоль Z, диаметр 0,95, ступица на +Z; в `models.json` масштаб до диаметра 1 м и ось по X, размер
  колеса задаёт `enemy_look`.
- Робот: Image с Pose = A-Pose → Image to 3D (Smart Topology 6000, Pose A-Pose) → Rig → Humanoid, рост 2 м → клипы
  `Idle_02`, `Walking`, `Running`, как у жителей. Перёд +Z, ступни на y = 0, в игре 2,6 м (модель 2,2 м × `ROBOT_SCALE`).

Тяжёлый танк по частям (башня поворачивается, ленты бегут — `src/tank_look.rs`). Образец целого:
`A heavy armored battle tank with a big laser cannon, enemy vehicle for a top-down action game, semi-realistic hard-surface 3D game asset: wide low dark graphite armored hull, a large angular turret with one long thick energy laser cannon pointing straight forward and aligned with the hull, a glowing amber lens at the muzzle, two wide tracks with dark steel treads, amber-yellow warning markings and small amber sensor lights, rugged military look, single vehicle, centered, 3/4 view from above, plain light grey background, no text, no logos, no red`.

| id | треуг. | промпт (с образцом в Image References, кроме гусеницы) |
|---|---|---|
| `enemy_tank_hull` | 3215 | `Only the hull of the same heavy tank as in the reference image, as a separate game part: NO turret and NO tracks at all, flat empty turret ring on top, the hull hovers with nothing under it and no wheels, same dark graphite armor and amber-yellow markings, ...` |
| `enemy_tank_turret` | 2332 | `Only the turret of the same heavy tank as in the reference image, as a separate game part: NO hull and NO tracks, the turret alone with its one long thick laser cannon pointing straight forward, the cannon is centered and aligned with the turret, glowing amber lens at the muzzle, ...` |
| `enemy_tank_track` | 1674 | без образца: `ONE single tank track unit of a heavy military tank, as a separate game part: a long low loop of dark steel track links with grousers wrapped around five road wheels, a drive sprocket and an idler, a dark graphite armored side skirt over the top with small amber-yellow markings, only one track, no tank hull, no turret, side 3/4 view from above, ...` |

Босс — шагающая крепость на четырёх ногах. Образец: `A giant four-legged armored assault walker, boss enemy for a top-down action game, semi-realistic sci-fi hard-surface 3D game asset: a massive wide box-shaped dark graphite armored body like a mobile fortress, four thick heavy mechanical legs with knee joints and big flat feet, a battery of short mortar tubes and two rocket launcher pods on its back, one large glowing amber sensor eye at the front, amber-yellow hazard stripes and small amber lights, rugged military look, ...`.

| id | треуг. | промпт (с образцом) |
|---|---|---|
| `boss_body` | 5909 | `Only the body of the same giant walker as in the reference image, as a separate game part: NO legs at all, just round hip joint sockets at the four lower corners, the body hovers with nothing under it, keep the amber sensor eye at the front and the rocket launcher pods on its back, ...` |
| `boss_leg` | 1158 | `ONE single leg of the same giant walker as in the reference image, as a separate game part: one heavy mechanical leg standing straight up vertically, a round hip joint at the top, a knee joint, a thick lower leg and a big flat foot at the bottom, only one leg, no body, ..., side view` |

- Все части смотрят носом на −X (`rotation_y` = −π/2), гусеница лежит вдоль X (`rotation_y` = π/2).
- Башня и нога доработаны скриптом Blender: у башни начало координат перенесено в середину низа башни без
  ствола (вокруг него она поворачивается), дульный срез — 0,631 вперёд и 0,116 вверх в единицах модели
  (`tank_look`). Нога в Meshy вышла парой, вторая отрезана; начало ноги — бедро, нога висит вниз.
- Шарниры бёдер босса в модели — ±0,2 по обеим осям на 0,385 ниже центра, глаз — на переднем торце на высоте 0,06:
  отсюда `MODEL_HIPS` и `MODEL_CORE` в `src/boss.rs` при масштабе 8,5.

### Ящики, перехватчик, лазерная установка (2026-11-17, `laser-tank`)
Картинка nano-banana-2 → Image to 3D meshy-t2.5 Smart Topology, текстура meshy-n1. Одобрены Sergei Zotov 17.11
и записаны в `models.json`. Суффикс у всех:
`semi-realistic hard-surface 3D game asset, single object, centered, 3/4 view from above, plain light grey background, no text, no logos, no red`.

| id | треуг. | промпт (без суффикса) |
|---|---|---|
| `enemy_laser_turret` | 2410 | `Only the turret of a heavy battle tank, as a separate game part, a sci-fi laser weapon system: a squat wide angular armored rotating turret with a flat round bottom, NO hull and NO tracks, on top a big laser projector instead of a cannon: a short thick emitter housing with ring-shaped cooling fins and heavy coils, ending in a large round glowing amber-orange crystal lens at the front, two cylindrical power capacitors with glowing amber bands on both sides, a small sensor dish at the back, dark graphite armor plates with amber-yellow warning markings and small amber sensor lights, rugged military look, the emitter points straight forward and is centered` |
| `enemy_interceptor` | 5108 | промпт из раздела «Желательно» |
| `pickup_crate` | 1437 | `A sturdy military supply drop crate, pickup item for a top-down action game: cube-shaped crate with thick olive-tan wooden plank panels, chunky dark steel corner brackets and edge bands, a steel parachute harness ring on top, a recessed glowing white light strip across the lid` |
| `pickup_golden` | 1570 | `A precious golden upgrade crate, rare pickup item for a top-down action game: cube-shaped crate made of polished shiny gold metal plates, thick rounded gold corner brackets, glowing warm yellow seams along the edges, a raised five-pointed star emblem on the lid and on each side, a gold parachute harness ring on top` |
| `pickup_medkit` | 1516 | `A repair and first aid kit case for a moon rover, pickup item for a top-down action game: chunky cube-shaped white hard-shell case with rounded edges, a big bold turquoise plus-shaped cross on the lid and on every side, dark grey latches and a carry handle on top, small turquoise light` |

- Лазерная установка и перехватчик смотрят носом на −X (`rotation_y` = −π/2). У установки начало координат
  перенесено в середину опорного кольца снизу (узел GLB сдвинут на −0,078 по X и +0,283 по Y): линза в
  0,557 вперёд и 0,341 вверх, масштаб 1,3 (`LASER` в `src/tank_look.rs`).
- Ящики и аптечка — кубы около 0,8–0,95 в единицах модели, центр в середине. Ящики в игре — единичный куб в
  масштабе `CRATE_SIZE` / `GOLDEN_SIZE` (`src/pickups.rs`), модель ставится на его дно: в `models.json`
  `scale` ≈ 1,05, `offset_y` ≈ 0,5. Аптечка — `MEDKIT_SIZE` 1,6 м (`src/repair.rs`): `scale` ≈ 1,7,
  `offset_y` ≈ 0,65.

### Служба защиты жителей и щебень (2026-11-17)
Машины Службы — как ящики: nano-banana-2 → meshy-t2.5 Smart Topology 4000, текстура meshy-n1. Белые с оранжевой
полосой, без мигалок и надписей; на крыше пустая круглая площадка, тарелку антенны крутит код отдельной деталью.
Кабина на −X (`rotation_y` = −π/2). Подключены по id (`ServiceKind::model` в `src/service.rs`); `scale` подобран
по длине корпуса `HULL`, `offset_y` ставит колёса на землю.

| id | треуг. | промпт (без общего суффикса из раздела выше) |
|---|---|---|
| `service_patrol` | 4046 | `A fast four-wheeled civil protection patrol all-terrain vehicle for a top-down action game: rounded friendly glossy white body with a bold orange stripe along the sides, big chunky off-road wheels with dark tires, a rounded cab at the front with dark tinted windows, a small twin machine gun on a low roof mount pointing forward, a flat round empty mount plate on the roof behind the cab, no lights, no sirens, no light bar, no police markings` |
| `service_transport` | 4113 | `A six-wheeled civil protection armored rescue transporter ...: long rounded friendly glossy white armored body with a bold orange stripe along the sides, a rounded cab at the front, a passenger compartment with a flat open roof area with bench seats at the back, a small orange boxy four-tube rocket launcher pod on the roof, a flat round empty mount plate on the roof, six big chunky wheels, ...` |
| `service_support` | 4034 | `A heavy six-wheeled civil protection support vehicle ...: wide rounded friendly glossy white truck body with a bold orange stripe, a rounded cab at the front, on the rear platform a short thick dark grey mortar tube on a turntable pointing up and back at 45 degrees, a flat round empty mount plate on the roof, six big chunky off-road wheels, ...` |

Щебень для насыпи (просьба сессии, которая делала насыпь, `railway::ballast_share`): `ballast_1` … `ballast_5`,
text-to-3d meshy-t2.5 Smart Topology 250, без текстуры, промпт `a single crushed railway ballast stone, <форма>,
sharp broken flat facets, low-poly game asset` (формы: почти куб, плоский клин, вытянутый осколок, пирамида с
плоским низом, неровный ком). Скрипт `flat_stone.py` (в scratchpad сессии) развалил вершины на плоские грани и
задал матовый материал sRGB ≈ (0,46; 0,43; 0,39) ± 0,04. 256–264 треугольника, центр габарита в нуле, наибольший
размер ≈ 0,96 — масштаб 0,15–0,35 даёт нужные метры.

Все модели этого раздела одобрены Sergei Zotov 17.11. Машины Службы — в `models.json`; щебень (`ballast_*`) ждёт
кода насыпи и лежит в `/home/user/space-guard-models-pending/`. Страница с моделями —
https://claude.ai/code/artifact/b4f12db9-9374-47a5-b089-e5152581f280 (каталог моделей `YUSgKihaVcM9EwQaE5S3Sv`
аккаунту этой сессии недоступен).

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

### Трофейный боекомплект (2026-11-17)
Боекомплект погибшего врага (`pickups::spawn_loot_item`): мортирщик оставляет снаряды, ракетчики — кассету
ракет (и для самонаводящихся), лазерный танк — энергоячейку; от пулемётчиков трофея нет. Конвейер как у
машин Службы: nano-banana-2 → meshy-t2.5 Smart Topology 1500, текстура meshy-n1 2K, 63 кредита за три.
Цвет полос — цвет оружия в интерфейсе. Meshy кладёт длинную сторону (~0,95) по X, центр в нуле: `offset_y` —
половина высоты в масштабе. Длина в игре 1,1–1,25 м, чтобы трофей читался сверху. Одобрены Ivan Tsarev и
Sergei Zotov 17.11. Общий суффикс промптов: `, semi-realistic hard-surface 3D game asset, single object,
centered, 3/4 view from above, plain light grey background, no text, no logos, no red`.

| id | треуг. | промпт (без суффикса) |
|---|---|---|
| `loot_mortar` | 1486 | `A compact open ammunition rack with five mortar bombs, ammo pickup item dropped on the ground for a top-down action game: a low sturdy dark graphite steel frame box with open top and open sides, holding five short thick fat mortar bombs standing upright side by side, each bomb with a rounded nose, a bright orange band around the body and a finned tail, chunky steel carry handles on both ends, bold orange stripe along the frame, rugged military look` |
| `loot_rockets` | 1461 | `A rocket ammunition cassette, ammo pickup item dropped on the ground for a top-down action game: a compact rectangular dark graphite steel launch container with an open top, holding four short thick missiles lying side by side in a row, each missile with a pointed amber-yellow nose cone and small tail fins, bold amber-yellow stripes on the container lid edges and sides, chunky steel latches and a carry handle, rugged military look` |
| `loot_laser` | 1482 | `A heavy laser energy cell, ammo pickup item dropped on the ground for a top-down action game: a massive thick cylindrical battery lying on its side, held in a sturdy dark graphite steel cradle frame with clamps and a carry handle on top, glowing bright cyan translucent energy windows and glowing cyan rings around the cylinder, chunky metal end caps with connector terminals, dark graphite metal body, rugged sci-fi military look` |
