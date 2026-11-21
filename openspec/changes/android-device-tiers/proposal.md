## Why

У iOS теперь уровни устройств по возможностям (`ios-device-tiers`), а Android по-прежнему получает один
профиль `android` для всех телефонов. Числа в нём подобраны замерами на одном телефоне (A059, Adreno 810,
8 ГБ). Телефону на Adreno 610 и 4 ГБ они тяжелы, флагману на Adreno 830 — малы. Ещё по рекомендациям
Google для игр у нас нет: реакции на нагрев (ADPF Thermal), опроса памяти (сигналы `onTrimMemory`
нативная активность не получает, а `onLowMemory` с Android 14 не приходит), предела частоты на экранах
90–120 Гц (`ANativeWindow_setFrameRate`), Game Mode. Пакет для Google Play не просит у bundletool
выравнивания библиотек по 16 КБ — требование Play для target SDK 35+ (APK уже выровнен).

## What Changes

- Уровень устройства Android при запуске по возможностям: модель GPU из адаптера Vulkan (wgpu),
  объём памяти (`/proc/meminfo`), признак слабого устройства (`ro.config.low_ram`). Профили
  `android-low`, `android` (средний — нынешние числа), `android-high`.
- Нагрев: запас до троттлинга (`AThermal_getThermalHeadroom`, Android 12+) — кадр на шаг меньше, а у
  самого предела — не больше 30 кадров; общие флаги с iOS.
- Частота: экран просится на частоту предела кадров (`ANativeWindow_setFrameRate`, Android 11+);
  профилю `android` — предел 60.
- Память: опрос `ActivityManager.MemoryInfo` через JNI — при `lowMemory` обломков вдвое меньше, как на iOS.
- Game Mode: `game_mode_config.xml` в манифесте; режим «экономия батареи» — 30 кадров и кадр на шаг меньше.
- AAB: `PAGE_ALIGNMENT_16K` в `BundleConfig.json`, проверка `bundletool dump config`.

## Capabilities

### New Capabilities
- `android-device-tiers`: уровни устройств Android по GPU и памяти, профили уровней, реакция на нагрев,
  память, частоту экрана и Game Mode, выравнивание пакета для Google Play.

### Modified Capabilities
<!-- нет: профиль android остаётся средним уровнем с прежними числами, кроме предела 60 кадров -->

## Impact

- `src/android.rs` (уровень, нагрев, память, частота, Game Mode), `src/profile.rs` (выбор профиля),
  `assets/platform/profiles.json`, `android/AndroidManifest.xml`, `android/res/xml/game_mode_config.xml`,
  `scripts/build-aab.sh`.
- Без патчей и без Java: функции NDK новее minSdk 29 берутся через `dlsym`, остальное — JNI (уже есть).
- Вне изменения: Swappy и `VK_GOOGLE_display_timing` (патч wgpu), `TRANSIENT_ATTACHMENT` для MSAA и
  глубины (патч Bevy), Performance Hint API (нужны id потоков Bevy и отчёт каждый кадр — после замеров),
  ASTC и раздача текстур по формату в Play (меняет сборку ассетов), Android Performance Tuner и
  Memory Advice API (нужен Java-код).
