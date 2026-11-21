в работе: Sergei Zotov

## 1. Профили и ресурс

- [x] 1.1 `assets/platform/profiles.json` с профилями base, desktop, macos, linux, windows, web, web-mobile, mobile, android, ios-phone, ios-tablet и числами из таблицы Context в design.md; `src/profile.rs`: разбор, глубокое слияние по `parent`, значения со ступенями `[low, medium, high]`, ресурс `PlatformProfile`, `ProfilePlugin` первым в `main.rs`; проверка: `cargo test` — каждый профиль собирается, все поля заданы
- [ ] 1.2 Определение платформы и класса устройства: iOS — `UIDevice.userInterfaceIdiom` через `objc2` в `src/ios.rs`, браузер — `window.spaceGuardMobile`, Android — телефон; `SPACE_GUARD_MOBILE=1` на компьютере выбирает профиль mobile; проверка: `cargo test`, имя профиля в логе на Mac и в симуляторе iPhone
- [x] 1.3 Переопределения: путь ключа → `SPACE_GUARD_<ПУТЬ>` (компьютер, iOS), `debug.spaceguard.<путь>` (Android), `?<путь>=` (браузер); синонимы старых ручек (`density`, `shadows`, `detail`, `quality`, `off`, `SPACE_GUARD_OFF`, `SPACE_GUARD_DETAIL`, `?quality=`); строка в лог со всеми действующими; проверка: `cargo test` на разбор ключей, запуск с `SPACE_GUARD_RENDER_DENSITY=1`
- [x] 1.4 Тест равенства: профили macos, linux, windows, web, web-mobile, android дают ровно сегодняшние числа (таблица Context); проверка: `cargo test`

## 2. Перенос модулей на профиль

- [x] 2.1 `render_scale.rs` и `quality.rs`: плотность кадра, MSAA на high, карта и каскады теней, bloom — из профиля; убраны `cfg!(target_os = "android")` и чтение свойств Android в этих модулях; проверка: `cargo test`, снимок `SPACE_GUARD_SHOT` на Mac до и после совпадает, APK на эмуляторе — ступени как раньше
- [x] 2.2 `debris.rs`, `explosion_fx.rs`, `fire.rs`: лежащие обломки, размер обломка с тенью, вспышки, пул огней — из профиля вместо `is_mobile()`; проверка: `cargo test`, `cargo run -- --bench=60` на Mac — числа в сводке прежние
- [x] 2.3 `lod.rs`, `models.rs`: множитель упрощения моделей, тени-силуэты, предел текстур — из профиля; `SPACE_GUARD_DETAIL` — синоним; проверка: `cargo test`, снимок с `SPACE_GUARD_MOBILE=1` на Mac совпадает с прежним
- [x] 2.4 `prewarm.rs`, `main.rs`: кадров до старта, ожидание конвейеров, синхронная сборка, пулы потоков — из профиля; Android-значения прежние; проверка: `cargo build`, APK на эмуляторе загружается до меню без зависания
- [x] 2.5 `platform.rs`, `settings.rs`, `touch.rs`, `gamepad.rs`, `menu.rs`: ступень по умолчанию, касание по умолчанию, скрытие курсора, пункт «Выход» — из профиля; `platform::quality_hint` и `is_mobile` остаются только там, где речь не о железе; проверка: `cargo test`, `cargo run -- --touch`, меню в браузерной сборке без «Выход»
- [x] 2.6 F3 и сводка `--bench`: имя профиля, класс устройства, ступень, переопределения; проверка: `cargo run`, F3

## 3. iOS

- [ ] 3.1 Профили ios-phone и ios-tablet по таблице решения 6 в design.md; проверка: `cargo test`, лог в симуляторе iPhone и iPad — плотность 1,5 на medium
- [ ] 3.2 `scripts/build-ios.sh`: текстуры моделей до 1024 при сборке (`scripts/web_textures.py`, как у Android), ключ `--env КЛЮЧ=значение` для запуска на телефоне; проверка: размер `.app` меньше, игра грузится на iPhone
- [ ] 3.3 Пауза звука в фоне по `AppLifecycle` общим кодом для Android и iOS; ограничитель кадров `frame_limit` (iPad ProMotion — 60); проверка: iPhone — свернуть и вернуться во время забега; симулятор iPad — 60 кадров в F3
- [ ] 3.4 Замер на iPhone 15 Pro Max: 5 минут забега на low, medium, high — FPS из F3, нагрев, память (`os_proc_available_memory` в лог раз в 10 с); числа профиля iPhone поправить по замеру; проверка: `scripts/build-ios.sh --release --run`
- [ ] 3.5 Слой нагрева: при `ProcessInfo.thermalState` ≥ serious или в режиме энергосбережения плотность кадра на шаг ниже, в лог; проверка: iPhone, долгий забег на high до нагрева

## 4. Документация и проверка

- [ ] 4.1 `CLAUDE.md`: раздел о профилях — где файл, порядок слоёв, имена переопределений по платформам; старые записи о `debug.spaceguard.*` сведены к новой схеме; проверка: `openspec validate --all`
- [ ] 4.2 Проверка на всех платформах: Mac (`cargo run`), браузер (`scripts/build-web.sh --serve`, телефон через `adb reverse`), Android (APK на телефоне), iPhone — в каждой имя профиля в логе и прежняя производительность; проверка: ручной прогон
