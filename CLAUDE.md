# Space Guard

Игра для Game Jam Karlsruhe 2026: аркадный 3D-шутер с видом сверху, луноход на холмистой
планете с разрушаемым ландшафтом. Rust, Bevy 0.20.1.

## Где что лежит

- `docs/requirements.md` — исходные требования к игре и порядок приоритетов.
- `openspec/config.yaml` — контекст проекта и правила для артефактов OpenSpec.
- `openspec/specs/` — принятые спецификации.
- `openspec/changes/` — изменения в работе: `proposal.md`, `design.md`, `tasks.md` и дельты спецификаций.
- `assets/voice/`, `assets/sfx/`, `assets/music/` — реплики, звуки и музыка: список JSON и mp3 рядом.
  Формат и соглашение о вариантах — в `assets/voice/README.md`.
- Забег — `src/run.rs`: состояния `GameState::{Loading, Menu, Playing, Paused, GameOver}`, игра открывается
  загрузкой (`src/loading.rs`), потом меню, рестарт по Enter. Пока идёт загрузка, ничего не звучит и мир скрыт
  чёрным занавесом (`src/fade.rs`); звуки списков она ждёт через `loading::Preload`. Начало забега, рестарт
  и выход в меню — только через `RunWorld::start_run` / `open_menu`: они идут через затемнение, мир
  перестраивается под занавесом. Новый экран — корень с `fade::Appear`, чтобы проявлялся плавно. Сущность, которая живёт один забег, получает компонент `RunScoped`.
- Перестройка мира — сообщение `WorldRebuilt { seed }` из `src/run.rs`. Его шлёт `RunWorld::reset`
  при начале забега, рестарте и выходе в меню, а при запуске игры — `run`: рельеф уже новый,
  сущности с `RunScoped` удалены, луноход на старте. Всё, что хранит состояние мира или забега
  (ресурс с картой, списком сущностей, таймером, счётчиком; компонент, оставшийся на луноходе),
  сбрасывается по нему системой в `GameSet::Reset`. `RunStarted` (без полей) приходит следом
  только в начале забега: на него подписывай лишь то, чего не должно быть в меню (приветствие штаба).
- Порядок кадра — наборы `GameSet` из `src/frame.rs`. Каждая система в `Update` стоит в одном из
  них: `.in_set(GameSet::…)`. Наборы идут цепочкой, между ними применяются команды:
  - `Screens` — экраны и переходы забега (`run.rs`); `Reset` — сброс по `WorldRebuilt`. Работают всегда.
  - `Input` — ввод игрока: езда, выбор оружия, спуск. `Simulation` — игровая логика: враги, волны,
    снаряды, таймеры. `Damage` — взрывы и попадания превращаются в `Damage`, кратеры и
    `TerrainChanged`. `ApplyDamage` — одна система `combat::apply_damage`, сюда ничего не ставить.
    `Death` — читатели `Died`. `Cleanup` — удаление погибших и отставших. Эти шесть наборов
    работают только в `GameState::Playing`: своего `run_if(in_state(Playing))` системе не нужно.
  - `Streaming` — подгрузка мира по `VisibleChunks` после `terrain::stream_chunks`.
    `Presentation` — камера, интерфейс, эффекты, звук. Работают всегда: в меню, на паузе
    и на экране конца забега. Что показывается только в забеге, само проверяет `State<GameState>`.
  - Пауза останавливает `Time<Virtual>`: `Res<Time>` в `Update` на паузе даёт нулевой шаг.
    По реальному времени (`Time<Real>`) считай только то, что должно идти и на паузе.
  - Сообщение игрового мира (взрыв, урон, выстрел, появление врага) регистрируй через
    `app.add_run_message::<T>()` из `frame::RunMessages`: непрочитанные сообщения стираются при
    перестройке мира. Сообщения звука и экранов — обычным `add_message`.
- Фокус — ресурс `Focus` из `src/camera.rs`: точка, за которой идут камера и подгрузка мира.
  В забеге это луноход, в меню — точка, летящая над планетой; луноход в меню спрятан и стоит на
  старте. Подгрузка мира читает `Focus` или `VisibleChunks`, а не луноход.
- Урон — только сообщением `Damage { target, amount, source_team }` из `src/combat.rs`; здоровье
  напрямую не менять. Гибель приходит сообщением `Died { entity, position, killer }`: место гибели
  и команда убившего. Читатель, которому нужны компоненты погибшего, стоит в `GameSet::Death`;
  удаляй погибшую сущность в `GameSet::Cleanup`, как враги, чтобы её застали все читатели.
  Читателю в `GameSet::Presentation` доступны только поля сообщения. Время забега — ресурс `SurvivalTime`.
- Рельеф меняй и сразу после этого шли `TerrainChanged { center, radius }` из `src/subsidence.rs`:
  так делает `craters::dig_craters`. Читатель, которому нужен рельеф этого кадра, стоит в
  `GameSet::Damage` с `.after(craters::dig_craters)`.
  Исключение — колеи (`src/tracks.rs`): машины проминают рельеф на сантиметры без `TerrainChanged`
  (его читатели рассчитаны на кратеры), трава узнаёт о колее из `GroundPressed`, меши участков
  пересобираются пачкой. Следы и колеи живут до конца забега.
- Случайность игровой логики — ресурс `GameRng` из `src/rng.rs` (`below`, `range`): засевается сидом
  мира при каждом `WorldRebuilt`. `Dice` из `src/sound_list.rs` — только для звука, штаба
  и комикс-надписей.
- Всё, что ездит по рельефу, кроме лунохода, получает `GroundVehicle` из `src/vehicle.rs`: управление
  пишет скорость и курс, посадку и наклон делает общая система. Враги — `src/enemy.rs`.
- Геймпад — `src/gamepad.rs`: кнопки нажимают виртуальные клавиши в `ButtonInput` (в забеге
  R2 — левая кнопка мыши, крест — пробел), поэтому новой команде с клавиатуры достаточно добавить
  строку в `virtual_for`; аналоговое — ресурс `PadInput`, устройство — `InputDevice`, подсказка
  экрана для обоих устройств — компонент `DeviceHint`. На macOS ввод геймпада идёт через
  GameController (`src/gamepad/macos.rs`): gilrs там кнопок DualShock 4 не получает; на Android —
  фильтр очереди ввода (`src/gamepad/android.rs`, патч android-activity при сборке:
  `android/patch_android_activity.py`); на Linux и Windows — gilrs самого Bevy.
- Касания — `src/touch.rs` (схема экрана: https://claude.ai/artifact/GctBfPbonNnJSF1BT2s9kg): устройство
  `InputDevice::Touch`, `device.stick()` — геймпад или касания. Левый большой палец — плавающий стик езды,
  правый — стик прицела (оттяжка за мёртвую зону стреляет, у мортиры длина оттяжки — дальность), короткое
  касание справа — выстрел по ближайшему видимому врагу. Касание пишет в `PadInput` и нажимает виртуальные
  клавиши, как геймпад; узел интерфейса с компонентом `touch::TapAction` (`Key`, `Mouse`, `Step`) — кнопка
  для пальца. Вне забега короткое касание — ресурс `TouchTap`, свайп — `TouchSwipe`; строку меню под
  пальцем ищет `touch::nearest_row`. Размеры — в dp (`touch::layout`, `touch::dp_to_ui`), отступы выреза —
  `platform::safe_area`. На компьютере — `cargo run -- --touch`: левая кнопка мыши изображает палец.
  Android «назад» — Esc (фильтр очереди ввода).
- Вибрация и световая панель — `src/haptics.rs`: событие толкает микшер `ResMut<Rumble>`
  (`kick(сильный, слабый)`), отправку в геймпад делает бэкенд. На macOS это CoreHaptics через
  GameController (`src/haptics/macos.rs`), и только обычный `CHHapticPatternPlayer`: продвинутый
  у геймпадов PlayStation не работает. На Linux и Windows — `GamepadRumbleRequest` через gilrs
  (`src/haptics/bevy_rumble.rs`), на Linux световая панель — `/sys/class/leds`
  (`src/haptics/linux_light.rs`; без правила udev писать туда может только root).
- Мини-карта — `src/minimap.rs`: новой сущности с меткой добавь запрос в `update_markers`.
- Меню — `src/menu.rs` (главное меню и подэкраны: сложность, настройки, рекорды, авторы). Настройки
  (качество, громкости, субтитры, вибрация, выбранная сложность) — ресурс `Settings` из `src/settings.rs`,
  файл `settings.json`; громкость звука — `Settings::gain(группа)`. Сложность — `src/difficulty.rs`:
  ресурс `RunDifficulty` ставится при начале забега, по нему урон по луноходу, рост угрозы, восстановление
  и помощь прицела стиком. Рекорды — таблица на каждую сложность.
- Режим наведения авиаудара — ресурс `airstrike::Targeting`: пока `active()`, он владеет скоростью
  времени (`impact` не трогает её), камера и курсор стика идут по реальному времени, Esc не ставит паузу.
- После гибели фокус камеры облетает места разрушений (`src/showcase.rs`), а не стоит на луноходе.
- Куски моделей для обломков — `src/fragments.rs` (`ModelPieces`, `split_mesh`); модель режется, если она
  в `CUT`. Разбросать куски того, что стоит в мире, — `fragments::Breaker` (`burst`, `parts`), гибель
  машины — `fragments::wreck_vehicle`. Формы обломков без модели (глыба, осколок, доска, лист, ком,
  крошка) — `DebrisShape::Form(FormKind)` из `src/debris_forms.rs`, растягиваются `size`, как куб.
- Волны — `src/waves.rs`: угроза `Threat`, директор волн, все числа баланса в `waves::balance`.
  Врага ставит сообщение `SpawnEnemy { place }` из `src/enemy.rs`, десантную капсулу — `DropCapsule { angle }`
  из `src/capsule.rs`, начало волны — `WaveStarted { number }`.
- Тревога — ресурс `Alarm` из `src/alarm.rs` (ступени 0–5, `alarm.level()`): растёт от `BuildingDestroyed`,
  `ResidentDied`, гибели машин Службы и пассажирских вагонов, управляет только Службой защиты жителей,
  волны от неё не зависят. Очки добавляет `Alarm::add`.
- Служба защиты жителей — `src/service.rs`, команда `Team::Service`: с врагами взаимного урона нет.
  Кто кого ранит и в кого попадает пуля — `Team::damage_share` и `Team::hostile_to` в `src/combat.rs`.
- Враги ведут машину через `Steering` из `src/steering.rs` (желаемый курс и скорость): объезд домов и
  выход из застревания делает общая система; видимость для стрельбы — `steering::has_los`.
- Выбранное оружие — ресурс `CurrentWeapon` из `src/weapons.rs`. Каждое оружие перед выстрелом по
  левой кнопке мыши проверяет, что выбрано именно оно.
- Снаряд мортиры — сообщением `FireShell { origin, target, team, power }` из `src/mortar.rs`, и у игрока, и у врагов.
- Лечение — только сообщением `Heal { target, amount }` из `src/combat.rs`.
- Пули — сообщением `FireBullet { origin, direction, team, damage }` из `src/bullet.rs`; прямая
  видимость над рельефом — `bullet::has_line_of_sight`.
- Озвучка игровых событий собрана в `src/audio_events.rs`: новое событие озвучивается там.
- Звук из кода — только сообщениями и ресурсами, файлы напрямую не грузить:
  - повод для реплики: `VoiceCue::new("house_destroyed")` (`src/hq.rs`), тон штаба — ресурс `HqTone`;
  - звуковой эффект: `PlaySound::at("explosion", position)` или `PlaySound::new("ui_click")` (`src/sound.rs`);
  - громкость звука с позицией убывает с расстоянием до лунохода;
  - музыка: `ResMut<MusicState>`, поле `track` (`src/music.rs`).

## Работа через OpenSpec

Требования превращаются в код через изменения OpenSpec, а не напрямую.

- Перед тем как писать код, посмотри `openspec list`. Если задача относится к активному
  изменению, прочитай его `proposal.md`, `design.md` и `tasks.md` и иди по задачам.
- Выполненную задачу отмечай в `tasks.md` как `- [x]`.
- Новая механика — новое изменение: `/opsx:propose`. Реализация — `/opsx:apply`.
  Принятое изменение переносится в спецификации через `/opsx:archive`.
- После правок в `openspec/` запускай `openspec validate --all`.

## Сборка и проверка

```sh
cargo run                                   # запуск
cargo run --features bevy/dynamic_linking   # быстрая пересборка при разработке
cargo run -- --immortal                     # проверка волн: луноход не теряет здоровья
cargo run -- --bench=180                    # замер: забег сам, без vsync, сводка в лог (src/perf.rs)
cargo check                                 # проверка компиляции
cargo test                                  # тесты
```

- Отладочный снимок без рук: `SPACE_GUARD_AUTOSTART=1 SPACE_GUARD_SCENE=fire SPACE_GUARD_SEED=3
  SPACE_GUARD_SHOT=/путь/кадр.png CARGO_MANIFEST_DIR=$PWD ./target/debug/space_guard` — забег
  начинается сам, перед луноходом встают огни, через 8 с кадр сохраняется и игра закрывается
  (`src/debug_shot.rs`).
- Проверка сценой без рук: `SPACE_GUARD_AUTOSTART=1 SPACE_GUARD_CHECK=ruins|ruins_all|trees|alarm
  SPACE_GUARD_SHOT_DIR=/папка CARGO_MANIFEST_DIR=$PWD ./target/release/space_guard --immortal` — итог
  в лог строками «Проверка», снимки в папку, игра закрывается сама (`src/debug_checks.rs`).
- Прогрев шейдеров — `src/prewarm.rs`: при запуске и смене качества каждый класс меша рисуется с каждым
  классом материала, плюс сцены всех моделей и эффекты частиц, под землёй под камерой. Иначе Bevy
  компилирует конвейер при первом появлении объекта, а на macOS и в браузере кадр при этом стоит.
  Конвейер, собранный после прогрева, пишется в лог («Поздний конвейер»), их число есть в сводке
  `--bench`. Материал создавай при старте (в ресурсе `*Assets`), а не посреди боя: материал нового
  класса после прогрева тоже пишется в лог. `SPACE_GUARD_NO_PREWARM=1` выключает прогрев для сравнения.
- F3 в игре — оверлей FPS и времени кадра по участкам основного и рендер-потока (`src/perf.rs`).
- Сборка нативная: сейчас macOS на Apple Silicon, позже Linux. Есть и браузерная (WASM, WebGPU):
  `scripts/build-web.sh --serve` собирает в `target/web` и открывает на http://localhost:8080,
  страница — `web/index.html`, игра на всё окно; на ней же экран загрузки: считает запросы к
  `assets/` по манифесту `asset-sizes.json` из скрипта сборки, нажатие «старт» будит звук браузера.
  Скрипт уменьшает текстуры внутри моделей до 1024 пикселей (`scripts/web_textures.py`, нужен Pillow:
  `python3-pil` на Ubuntu, на macOS берётся через `uv`): телефону меньше видеопамяти и загрузки.
  Исходники в `assets/` не меняются. Страница заменяет в шейдерах выборку тени точечного источника
  (`textureSampleCompareLevel` по массиву кубических карт) на 1.0: на Adreno (Android) компилятор
  шейдеров на ней падает и Chrome теряет устройство GPU. Поэтому точечным источникам тени не включать.
  Телефон по USB: `adb reverse tcp:8080 tcp:8080`, консоль Chrome — `adb forward tcp:9222
  localabstract:chrome_devtools_remote` и CDP.
  Для Safari и iOS (WebKit) страница переписывает шейдеры перед компиляцией: PBR Bevy 0.20 кладёт
  текстуру в `let`, WebKit такое отклоняет. Ещё WebKit иногда отдаёт пустую память из `getMappedRange` буфера,
  созданного отображённым («Length out of range of buffer» при запуске): страница подставляет свою и пишет её
  в буфер при `unmap`. Паника или ошибка рендера видна прямо на экране загрузки. Нужны `rustup target add wasm32-unknown-unknown`
  и `wasm-bindgen-cli` той же версии, что `wasm-bindgen` в `Cargo.lock`. В браузере нет файлов
  и `std::time`: списки ассетов, часы и проверка файлов — через `src/platform.rs`, рекорды и
  настройки живут только до перезагрузки страницы. `std::time::Instant` не использовать:
  в WASM он падает, бери `bevy::platform::time::Instant`.
  Полный экран — кнопка в углу страницы; на телефоне — сам по касанию «начать». На iPhone полного
  экрана для страниц нет: игра ставится на экран «Домой» по `web/manifest.webmanifest`.
  На телефоне страница рисует не плотнее 1,5 пикселя на точку и просит низкое качество
  (`platform::quality_hint`); ступень можно задать и в адресе: `?quality=low|medium|high`.
- Сборка под Android: `scripts/build-android.sh` (`--abi=x86_64|all`, `--install`, `--run`) собирает
  `target/android/space-guard.apk` без Gradle. Пакет `android/` компилирует `src/main.rs` библиотекой для
  `NativeActivity`, поэтому новую зависимость добавляй и в `android/Cargo.toml`. Нужны Android SDK, NDK и
  таргеты Rust (список — в начале скрипта). Особенности платформы — `src/android.rs` (выборка тени для
  Adreno) и `src/platform.rs`: ассеты лежат в APK, поэтому новый JSON-список надо вшить в
  `platform::read_list`. Потеря видеокарты (DeviceLost) или паника открывают игру заново (`android::restart_once`,
  перезапуск в первую минуту после перезапуска не повторяется); проверка — `setprop debug.spaceguard.lose <секунд>`. Конвейеры на Android собираются параллельно в пуле асинхронных задач, но кадр ждёт
  их готовности (`prewarm::compile_pipelines`): отрисовка вперемешку со сборкой вешала GPU Adreno 810. Управление — касания (`src/touch.rs`), геймпад или клавиатура; вибрации нет.
  Иконка — ресурсы `android/res`; слои пересобирает `scripts/android-icon.py` из `android/icon/source.png` (нужен Pillow).
  Пакет для Google Play — `scripts/build-aab.sh` (`--install` ставит его на телефон через bundletool):
  `target/android/space-guard.aab`, скачивание ~120 МБ. Ключ загрузки — переменные `SPACE_GUARD_UPLOAD_KEYSTORE`,
  `SPACE_GUARD_UPLOAD_ALIAS`, `SPACE_GUARD_UPLOAD_PASSWORD` (ключ и пароль в репозиторий не класть), без них —
  отладочный ключ. Текстуры моделей в APK и AAB уменьшаются до 1024 при сборке (`scripts/web_textures.py`).
  Play требует страниц 16 КБ: библиотека линкуется с `max-page-size=16384`, APK выравнивается `zipalign -P 16`,
  а `BundleConfig.json` пакета объявляет `PAGE_ALIGNMENT_16K` — скрипт сверяет это `bundletool dump config`.
  Эмулятор (Android Studio, AVD `Medium_Phone_API_36.0`, arm64): `~/Library/Android/sdk/emulator/emulator -avd
  Medium_Phone_API_36.0 -gpu host -feature Vulkan -no-snapshot-save` — без `-gpu host` рисует программно и
  медленно. Сборка `--abi=arm64-v8a`, затем `adb install -r`; касания — `adb shell input tap|swipe|motionevent`,
  снимок — `adb exec-out screencap -p`. С target SDK 36 окно идёт под вырез камеры, а «назад» система закрывала
  бы сама: в манифесте `enableOnBackInvokedCallback="false"`.
  Скорость на телефоне (A059, Adreno 810): кадр рисуется не плотнее 1,25/1,5/2 пикселя на точку по ступени
  качества (`src/render_scale.rs`: в рендер-мире уменьшаются цепочка кадров окна и цели камер, растягивает
  система); на medium один каскад теней. Все эти числа — в профиле платформы (см. «Профили платформ» ниже).
  Главная нагрузка — модели: на телефоне (`models.simplify` профиля, на компьютере `SPACE_GUARD_MOBILE=1`)
  сетки моделей с полем `mobile_detail` в `models.json` упрощает
  meshoptimizer (`src/lod.rs`, крейт `meshopt` на C++, не в WASM; для Android C++ собирается без исключений),
  текстуры моделей уменьшаются до 1024 и получают mip-уровни; новой модели сразу ставь `mobile_detail`. К концу
  долгого забега кадр упирается в видеокарту (20+ врагов, тысячи обломков): на телефоне вдвое меньше точечных
  источников (огонь, вспышки взрывов), лежит до 600 обломков, тень бросают обломки от 1,2 м. Модели с полем `mobile_shadow` (деревья) отбрасывают
  тень грубым силуэтом (`models::ShadowProxy`) на слое `lod::SHADOW_LAYER`, который видит только солнце; обломки и
  снятие теней силуэты обходят. Сцену модели запрашивай только после загрузки
  её glTF (`models.rs`): запрос `x.glb` и `x.glb#Scene0` в одном кадре грузит файл дважды. Размеры кадра в
  рендер-мире меняй после набора `RenderSystems::ExtractCommands`: в нём применяются команды передачи кадра.
  Замеры без пересборки — свойства `adb shell setprop debug.spaceguard.<имя>`, читаются при запуске: любой
  путь профиля (`render.density 1`) и прежние имена — `perf 1` (сводка `--bench` в logcat: FPS, потоки, проходы
  на видеокарте, треугольники теней по моделям, пакеты отрисовки), `quality low|medium|high`,
  `density <пикселей на точку>` (0 — экран как есть), `novsync 1`, `shadows <каскадов>,<метров>`,
  `detail <множитель mobile_detail>`, `compute <потоков пула>`, `sync 1`,
  `off shadows,bloom,hdr,grass,particles,model:<id>` (`model:tree*` — по началу имени). Загрузку видеокарты в процентах показывает `/sys/class/kgsl/kgsl-3d0/gpubusy`.
  APK профилируется simpleperf (`profileable` в манифесте); имена функций в отчёте — сборка
  `SPACE_GUARD_SYMBOLS=1 scripts/build-android.sh` и `simpleperf report --symfs` с этим APK по пути с телефона.
- Сборка под iOS: `scripts/build-ios.sh` (`--sim`, `--release`, `--install`, `--run`, `--env КЛЮЧ=значение`,
  `--small-textures`) собирает `target/ios/<sim|device>/SpaceGuard.app` без проекта Xcode. Нужны Xcode и
  `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`; для телефона — сертификат Apple Development и
  профиль подготовки с этим устройством (создаётся в Xcode, скрипт находит его сам). Особенности — `src/ios.rs`:
  размер окна и безопасная область из UIKit, класс устройства (iPhone/iPad), слой нагрева, лог ещё и в файл
  песочницы `Library/Application Support/space_guard.log` (с телефона: `xcrun devicectl device copy from
  --domain-type appDataContainer --domain-identifier io.github.zotausiarhei.spaceguard --source …`).
  Версия SDK в исполняемом файле — 18 (`vtool`): iOS 27 закрывает приложения без сцен UIKit, а winit 0.30
  их не умеет. В симуляторе нет частиц (`bevy_hanabi` нужна непрямая отрисовка). meshoptimizer требует
  libc++ (`ios.rs`) и `IPHONEOS_DEPLOYMENT_TARGET=16.0` (скрипт). Бесплатная команда Apple: если с устройства
  удалить последнее её приложение, доверие разработчику пропадает и его надо дать заново в настройках.
- Профили платформ — `src/profile.rs`, значения в `assets/platform/profiles.json` (вшит в сборку): плотность
  кадра, тени, MSAA, bloom, предел кадров, обломки, огни и вспышки, упрощение моделей, размер текстур,
  прогрев, пулы потоков, ступень и ввод по умолчанию, «Выход» в меню, звук в фоне. Профиль наследует
  `parent` и перечисляет только отличия; значение по ступеням — `[low, medium, high]`. Дерево: `base` →
  `desktop` (`macos`, `linux`, `windows`), `web` (`web-desktop`, `web-mobile`), `mobile` (`android` с
  `android-low|high`, `ios-phone`, `ios-tablet` и их `-low|-high`). Код берёт числа только из `profile::get()`, а не из `cfg!(target_os)` или
  «телефон ли»; условием платформы остаются лишь обходы драйверов и систем (Adreno, WebKit,
  NativeActivity, симулятор iOS). Переопределение любого значения без пересборки — одно имя, свой транспорт:
  `SPACE_GUARD_RENDER_DENSITY=1` (компьютер; iOS — `build-ios.sh --env`), `adb shell setprop
  debug.spaceguard.render.density 1` (Android), `?render.density=1` (браузер); профиль целиком —
  `profile` (`SPACE_GUARD_PROFILE=ios-phone` показывает облик iPhone на Mac). Профиль и переопределения
  видны в логе при запуске, в F3 и в сводке `--bench`. Тест `profile::tests` сверяет профили компьютера,
  браузера и Android с прежними числами: меняешь число — меняй и тест.
  iOS выбирает профиль ещё и по уровню устройства (`ios::device`, `profile::device_tier`): по семейству GPU
  Metal (`supportsFamily`), объёму памяти и запасу до предела процесса (`os_proc_available_memory`), а не по
  модели — `ios-phone-low|ios-phone|ios-phone-high`, `ios-tablet-low|ios-tablet|ios-tablet-high` (слабые —
  A12/A13 или меньше 1,8 ГБ до предела, сильные — A17 Pro+ с 8 ГБ, iPad на M). Симулятор сообщает GPU
  Apple2 и получает слабый уровень. Уровень и данные — строка «iOS: GPU…» в логе. Значение, которое
  видеокарта не умеет (MSAA больше поддерживаемого), уменьшается с предупреждением, а не роняет игру.
  На iOS при остатке памяти меньше 300 МБ обломков вдвое меньше, при нагреве «critical» — не больше
  30 кадров. Info.plist (`ios/Info.plist`) требует A12+ (`iphone-ipad-minimum-performance-a12`: ослабить
  можно, ужесточить в обновлении нельзя), заявляет категорию игры и Game Mode; полоска «Домой» скрыта,
  свайп снизу сворачивает игру только со второго раза (поля `Window` в `main.rs`).
  Android выбирает уровень так же (`android::device`, `profile::android_tier`): имя GPU у временного
  экземпляра Vulkan до запуска Bevy, `MemTotal`, `ro.config.low_ram` → `android-low|android|android-high`
  (слабые — меньше 5 ГБ, Adreno 6xx, Mali-G31…G78, PowerVR; сильные — от 8 ГБ и Adreno 730/740/750/830/840,
  Immortalis, Xclipse 9xx; неизвестный GPU — средний: номер Adreno не растёт с мощностью). Эмулятор
  (1,9 ГБ) получает слабый уровень. Строка «Android: GPU…» в logcat. У `android` предел 60 кадров, и экран
  просится на эту частоту (`ANativeWindow_setFrameRate`). Раз в 10 с — запас до нагрева ADPF
  (`AThermal_getThermalHeadroom`; от 0,95 кадр реже, от 1,0 не больше 30 кадров, назад на 0,1 ниже) и
  статус нагрева, а также `ActivityManager.getMemoryInfo` (`onLowMemory` с Android 14 не приходит) —
  обломков вдвое меньше. Game Mode (`android/res/xml/game_mode_config.xml`) читается при запуске и
  возврате: «экономия батареи» — 30 кадров. Функции NDK новее minSdk 29 — через `dlsym`. Проверка на
  эмуляторе: `adb shell cmd thermalservice override-status 4` (сброс — `reset`), `adb shell cmd game mode
  battery io.github.zotausiarhei.spaceguard`. При возврате Bevy 0.20 шлёт `AppLifecycle::Running`, при
  уходе — `Suspended`: `WillResume` и `WillSuspend` сообщением не приходят.
- Браузерная версия выкладывается на GitHub Pages: https://zotau-siarhei.github.io/Game-JAM-Karlsruhe26/
  Workflow `.github/workflows/web.yml` раз в 4 часа собирает main (`--release`), если после последней
  выкладки были коммиты. Сборка не чаще раза в 4 часа при любом запуске, ручной (Actions → web → Run
  workflow) внутри этого окна ничего не собирает: серверы GitHub не нагружаем, вручную не запускать.
  Исключение — прямая просьба Sergei Zotov: `gh workflow run web.yml -f force=true` собирает сразу.
- Целевое железо — MacBook Air M2, игра должна идти на нём плавно.
- На Linux линкует mold (`.cargo/config.toml`), его надо поставить: `pacman -S mold`.
- Кэш компиляции — sccache через `scripts/rustc-wrapper.sh` (`.cargo/config.toml`): ставится
  `brew install sccache` / `pacman -S sccache`, без него сборка идёт обычным rustc. Кэшируются
  зависимости, общие для всех worktree; крейт игры собирается инкрементально мимо кэша.
  Статистика — `sccache --show-stats`, выключить на одну сборку — `RUSTC_WRAPPER= cargo build`.
- Каждое изменение оставляет игру собираемой и запускаемой.
- Новый worktree перед первой сборкой засевай готовой сборкой соседа: `scripts/seed-target.sh`.
  Скрипт клонирует `target` с самой свежей полной сборкой из соседнего worktree или основной копии
  (APFS `cp -c`: мгновенно и без лишнего места на диске). Зависимости одинаковые, поэтому cargo
  пересобирает только крейт игры: около минуты вместо 10–20 минут с нуля.
- Не указывай `CARGO_TARGET_DIR` на чужой `target` и не делай `cargo clean` в общей копии и чужих
  worktree: это ломает сборку другим сессиям. У каждого worktree свой `target`.

## Правила кода

- Новые механики оформляются как Bevy-плагины в отдельных модулях.
- Играбельность важнее архитектуры: времени несколько часов, без лишних абстракций.
- Комментарии, документы, артефакты OpenSpec и сообщения коммитов — на русском. Структурные
  заголовки OpenSpec и ключевые слова SHALL/MUST — на английском.
- API Bevy 0.20 сверяй с https://docs.rs/bevy/0.20.1, а не по памяти: между версиями оно заметно меняется.
- 3D-модели: новую или изменённую модель сначала показывай в HTML-каталоге моделей
  (https://claude.ai/artifact/YUSgKihaVcM9EwQaE5S3Sv, у изменённой — переключатель «Стало / Было»).
  В `assets/models` и в игру она попадает только после того, как Sergei Zotov её одобрил.

## Зависимости

- `bevy_hanabi` — GPU-частицы: взрывы, дым, искры.
- `bevy_audio` — своя копия 0.20.1 в `vendor/bevy_audio` (`[patch.crates-io]`, и в `android/Cargo.toml`): вывод звука переоткрывается
  при пропаже или смене устройства (наушники отключились и вернулись), см. `vendor/bevy_audio/PATCH.md`.
- `bevy_trauma_shake` — тряска камеры.
- `noise` — шум для ландшафта.
- `serde`, `serde_json` — чтение списка реплик штаба.

Игра в сеть не ходит. Звуки, музыку и реплики через ElevenLabs генерируют скрипты
`scripts/gen_audio.py` и `scripts/gen_voice.py` (Python 3; эффекты на реплики накладывает `ffmpeg`).
Ключ API — в переменной окружения `ELEVENLABS_API_KEY`, в репозиторий не класть.
