## Why

Игра идёт на шести платформах: macOS, Linux, Windows, браузер, Android, а теперь и iOS. Ограничения под
слабое железо разбросаны по коду условиями `target_os = "android"` и `is_mobile()`. Поэтому iPhone не
получает главных облегчений, сделанных для Android: кадр рисуется в полном разрешении 3×, на средней
ступени — два каскада теней, на высокой — MSAA 4×. В браузере часть облегчений не работает вовсе (нет
meshoptimizer). Подстроить одну платформу, не задев другую, нельзя: числа зашиты в разных модулях.
Нужен один профиль ограничений на каждую платформу, который настраивается отдельно, как Device Profiles
в Unreal или Quality Settings в Unity.

## What Changes

- Профиль платформы — ресурс `PlatformProfile` со всеми ограничениями и облегчениями: плотность
  кадра, тени, MSAA, bloom, предел частоты кадров, обломки, точечные источники, упрощение моделей,
  размер текстур, прогрев конвейеров, пулы потоков, ступень по умолчанию, ввод по умолчанию.
- Значения — в одном файле профилей, вшитом в игру, с наследованием: `base` → `desktop` (`macos`,
  `linux`, `windows`) / `web` (`web-desktop`, `web-mobile`) / `mobile` (`android`, `ios-phone`,
  `ios-tablet`). Внутри профиля — значения по ступеням качества low/medium/high.
- Порядок слоёв: база → платформа → класс устройства → ступень качества → настройки игрока →
  отладочные переопределения. Переопределения едины для всех платформ: переменные `SPACE_GUARD_*`,
  свойства `debug.spaceguard.*` на Android, параметры адреса в браузере, окружение запуска на iOS.
- Модули (`quality`, `render_scale`, `lod`, `debris`, `explosion_fx`, `fire`, `prewarm`, `touch`,
  `gamepad`, `main`) читают профиль вместо `cfg!(target_os = "android")` и `is_mobile()` там, где
  речь о железе, а не об особенностях системы. Обходы драйверов и ОС (Adreno, NativeActivity, WebKit)
  остаются условиями платформы.
- Профиль iOS: плотность кадра как у Android, тени и MSAA под плитковые GPU Apple, предел 60 кадров,
  касания — ввод по умолчанию, пауза звука в фоне, текстуры 1024 при сборке, нет пункта «Выход».
  iPad — отдельный класс устройства.
- Оверлей F3 и сводка `--bench` показывают активный профиль и переопределения.
- Поведение на macOS, Linux, Windows, в браузере и на Android после переноса не меняется: их
  профили повторяют сегодняшние числа.

## Capabilities

### New Capabilities
- `platform-profiles`: профили ограничений и облегчений по платформам и классам устройств, порядок
  слоёв, отладочные переопределения и значения по умолчанию для каждой платформы.

### Modified Capabilities

## Impact

- Новое: `src/profile.rs` (плагин и ресурс `PlatformProfile`), `assets/platform/profiles.json` (вшит
  через `include_str!`, поэтому одинаково доступен в APK, браузере и `.app`).
- Правки: `src/platform.rs`, `src/quality.rs`, `src/render_scale.rs`, `src/lod.rs`, `src/models.rs`,
  `src/debris.rs`, `src/explosion_fx.rs`, `src/fire.rs`, `src/prewarm.rs`, `src/perf.rs`,
  `src/touch.rs`, `src/gamepad.rs`, `src/menu.rs`, `src/main.rs`, `src/ios.rs`, `src/android.rs`,
  `scripts/build-ios.sh`, `ios/Info.plist`, `CLAUDE.md`.
- Зависит от изменения `ios-build` (ветка `ios-native`): профиль iOS включается вместе со сборкой iOS.
- Новых зависимостей нет: на iOS нужные вызовы UIKit и Foundation идут через уже подключённый `objc2`.
