## Context

`src/profile.rs` выбирает профиль в начале `main`, до создания адаптера wgpu. На iOS уровень берётся у Metal
напрямую (`ios::device`). На Android профиль один — `android`, его числа сняты замерами на A059 (Adreno 810,
8 ГБ, экран 120 Гц). Что уже есть по требованиям Google: target SDK 36, только arm64, `vulkan.version`
1.1 required, `appCategory="game"`, библиотека с LOAD-сегментами по 16 КБ (`max-page-size=16384`,
проверено `llvm-readelf`), APK — `zipalign -P 16` (проверено). Bevy сам отключает GPU-предобработку на
Adreno ≤ 730 (кроме 720) и Mali с драйвером < r48.

Рекомендации Google (источники — в конце): уровень устройства по памяти, SoC и GPU (так делит устройства
Play device targeting); ADPF Thermal — запас `getThermalHeadroom` (0 — холодный, 1,0 — уровень SEVERE)
раз в 10 с: от 0,85 лёгкий троттлинг, от 0,95 умеренный, от 1,0 — снижать нагрузку сразу; статус
(`getCurrentThermalStatus`) может навсегда остаться NONE. Частота: игре на экране 90/120 Гц —
`setFrameRate(60)`, иначе рывки. Память: `onLowMemory` с API 34 не приходит, `onTrimMemory` — только при
уходе в фон, значит, в игре — опрос `ActivityManager.getMemoryInfo()`. Game Mode (Android 12+):
`game_mode_config.xml` + режим из `GameManager.getGameMode()` при каждом возврате.

## Goals / Non-Goals

**Goals:** уровни Android по возможностям; реакция на нагрев, память, Game Mode; частота экрана по пределу
кадров; AAB по 16 КБ. Всё — из Rust, без Java и без патчей.

**Non-Goals:** Swappy (`VK_GOOGLE_display_timing` — только через `as_hal`, патч wgpu); `TRANSIENT_ATTACHMENT`
для MSAA и глубины (Bevy его не ставит); Performance Hint API (API 33+, нужны id потоков Bevy и отчёт
каждый кадр — отдельно, после замеров); ASTC и TCFT в Play; Android Performance Tuner, Memory Advice API,
настоящий `onTrimMemory` (нужен Java-код).

## Decisions

### 1. Уровень — по GPU адаптера Vulkan и памяти
До `App` создаётся временный `wgpu::Instance` с бэкендом Vulkan, `enumerate_adapters` даёт имя GPU
(«Adreno (TM) 810», «Mali-G715», «Xclipse 940», «PowerVR …»); номер Adreno разбирает
`bevy_render::get_adreno_model`. Память — `MemTotal` из `/proc/meminfo`, слабое устройство —
`ro.config.low_ram`. Свойство `ro.soc.model` пишется в лог, но в правило не входит: имена SoC без таблицы
ничего не говорят. Номер Adreno не растёт с мощностью (810 слабее 740), поэтому флагманы — по второй
цифре:

| Уровень | Условие |
|---|---|
| слабый | память < 5 ГБ, или `ro.config.low_ram`, или Adreno 6xx и ниже, Mali-G31…G78 (Bifrost и ранний Valhall), PowerVR |
| сильный | память ≥ 7 ГБ и (Adreno 7x0/8x0 со второй цифрой ≥ 3, Immortalis, Xclipse 9xx) |
| средний | остальное, в том числе неизвестный GPU |

Неизвестный GPU — средний, а не сильный (в отличие от iOS, где семейства Metal накопительные): имена
Android-GPU порядка не задают. Альтернатива — device targeting в Play — работает только для раздачи
ассетов и не даёт уровень коду в игре.

### 2. Профили
`android` остаётся средним с прежними числами плюс `frame_limit` 60. `android-low` и `android-high` —
наследники:

| | android-low | android | android-high |
|---|---|---|---|
| плотность low/med/high | 1 / 1,25 / 1,5 | 1,25 / 1,5 / 2 | 1,5 / 2 / 2,5 |
| тени med / high | 1024×1 / 1024×1 | 1024×1 / 2048×2 | 1024×2 / 2048×3 |
| обломков | 300 | 600 | 900 |
| огни / вспышки | 1-2-3 / 2 | 2-3-4 / 3 | 3-4-6 / 4 |
| текстуры моделей | 512 | 1024 | 1024 |
| bloom | нет на всех ступенях | low нет | как сейчас |
| предел кадров | 30 | 60 | 60 |

MSAA на Android по-прежнему нет (Adreno; без transient-вложений оно дорого).

### 3. Нагрев, память, Game Mode — общие флаги профиля
Флаги `profile::throttled` (кадр ×0,8), `overheated` (≤ 30 кадров) и `low_memory` (обломков вдвое) уже
есть для iOS. Android ставит их так: headroom ≥ 0,95 → throttled, ≥ 1,0 → overheated, назад — с запасом
0,1; `MemoryInfo.lowMemory` → low_memory; Game Mode BATTERY → throttled и overheated. Флаги объединяются
(если хоть один источник требует).

### 4. Функции NDK новее minSdk — через `dlsym`
minSdk 29, а `AThermal_*` — API 30/31, `ANativeWindow_setFrameRate` — API 30. `dlopen("libandroid.so")` +
`dlsym`; нет функции — нет реакции, без ошибки. Поднимать minSdk ради этого незачем.

### 5. Частота экрана
После каждого `AppLifecycle` `WillResume` (окно пересоздаётся) — `ANativeWindow_setFrameRate(окно, предел,
COMPATIBILITY_DEFAULT)` по `frame_limit`; при `overheated` — 30. Ограничитель кадров `profile::limit_frames`
работает и дальше: система может не послушаться.

### 6. AAB
`"uncompressNativeLibraries": { "enabled": true, "alignment": "PAGE_ALIGNMENT_16K" }` в `BundleConfig.json`;
`build-aab.sh` проверяет `bundletool dump config` и падает, если выравнивания нет.

## Risks / Trade-offs

- Правило уровней по именам GPU неполное → неизвестное — средний уровень; правило правится по логу.
- Предел 60 на A059 меняет нынешнее поведение (было до 120 Гц на FIFO) → замер: FPS не ниже прежнего на
  medium, телефон холоднее.
- Временный `wgpu::Instance` до `App` — лишние ~50 мс запуска → замер времени до меню.
- Слабого и сильного Android-телефона у нас нет: их профили — оценка, проверка — эмулятор и
  переопределение `profile`.

## Источники

developer.android.com: games/optimize/adpf, adpf/thermal, gamemode/gamemode-api, gamemode-interventions,
media/optimize/performance/frame-rate, topic/performance/memory-management, ComponentCallbacks2,
google/play/device-targeting, guide/practices/page-sizes, google/play/requirements/target-sdk,
games/optimize/textures, games/sdk/frame-pacing; исходники Bevy 0.20.1 (`gpu_preprocessing.rs`,
`get_adreno_model`) и wgpu-hal 30.0.2 (`vulkan/adapter.rs`).
