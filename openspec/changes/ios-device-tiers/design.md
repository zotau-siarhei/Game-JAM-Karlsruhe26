## Context

Профили платформ (`platform-profiles`) дают iOS два профиля: `ios-phone` и `ios-tablet`. Исследование
(источники — в конце) показало, чем различаются устройства под iOS 16+:

| Чип | Семейство Metal | Память | Примеры |
|---|---|---|---|
| A9, A10 | Apple3 | 2–3 ГБ | iPad 5–7, iPad Pro 2017 |
| A11 | Apple4 | 2–3 ГБ | iPhone 8, X |
| A12, A13 | Apple5, Apple6 | 3–4 ГБ | iPhone XR, 11, SE 2; iPad 8, 9 |
| A14–A16 | Apple7, Apple8 | 4–6 ГБ | iPhone 12–15, iPad 10, Air 4 |
| A17 Pro+, M1+ | Apple9+, Apple7+ (Mac2) | 8–16 ГБ | iPhone 15 Pro+, 16, 17; iPad Pro, Air на M |

Объём памяти iPhone Apple не публикует; предел процесса (jetsam) — около половины памяти: 3 ГБ →
~1,5 ГБ, 4 ГБ → ~2,1 ГБ. Что зависит от семейства в wgpu 30 (проверено в исходниках `wgpu-hal`):
MSAA 8 — только Apple9+ (1 и 4 — везде); BC-сжатие — Apple9+ и iPad на M; bindless Bevy (argument
buffers Tier 2) — Apple6+, ниже Bevy сам переходит на обычные группы привязок; на Apple3 текстур на
шейдер 23 вместо 96. Глубина Depth24 на iOS нет (Bevy берёт Depth32Float — работает везде).

Что советует Apple для игр: проверять возможности, а не модель (`supportsFamily`,
`os_proc_available_memory`, `recommendedMaxWorkingSetSize`); масштабировать качество по уровням; на
TBDR беречь полосу (memoryless-цели, MSAA с memoryless, разрешение кадра, MetalFX); реагировать на
`thermalState` и Low Power Mode; не слать команды GPU в фоне; держать стабильные 60 кадров, а не
рваные 120; Game Mode (iOS 18+) — по ключу Info.plist.

## Goals / Non-Goals

**Goals:**
- Не ставиться на устройства, где игра заведомо не пойдёт.
- Три уровня устройств iOS по возможностям; числа профиля под каждый.
- Ни одно значение профиля или переопределения не роняет игру на устройстве без поддержки.
- Системные жесты не мешают сенсорному управлению.

**Non-Goals:**
- Патчи Bevy и wgpu: memoryless-цели (`TRANSIENT_ATTACHMENT` Bevy не ставит), MetalFX, HDR-цель
  Rg11b10, темп кадров через `CADisplayLink`. Каждое — отдельное изменение после замеров.
- ASTC-текстуры (KTX2 вместо PNG в glTF) — отдельное изменение: меняет сборку ассетов.
- Entitlement `increased-memory-limit` и Sustained Execution — нужна платная программа разработчика.
- 120 Гц (`CADisableMinimumFrameDurationOnPhone`): тяжёлой 3D-сцене Apple советует стабильные 60.

## Decisions

### 1. Минимальное устройство — A12, ключом, а не версией iOS
`iphone-ipad-minimum-performance-a12` в `UIRequiredDeviceCapabilities` (iOS 14+) отсекает A9–A11 и
оставляет iOS 16. Альтернатива — минимальная iOS 18 (тоже A12+) — отсекла бы ещё и A12+ устройства на
старой системе. Требование можно ослабить в обновлении, но не ужесточить — решаем до выкладки.

### 2. Уровень по возможностям, а не по таблице моделей
Профиль собирается в начале `main`, до адаптера wgpu, поэтому `src/ios.rs` спрашивает Metal сам:
`MTLCreateSystemDefaultDevice().supportsFamily(...)`, `NSProcessInfo.physicalMemory`,
`os_proc_available_memory()`. Семейства накопительные: новое устройство отвечает «да» на Apple9, значит
неизвестная модель попадает в сильные. Правила:

| Уровень | Условие |
|---|---|
| слабый | семейство ниже Apple7 **или** до предела памяти < 1,8 ГБ |
| сильный | iPhone: Apple9+ и память ≥ 8 ГБ; iPad: Apple7+ и память ≥ 8 ГБ (M-чипы) |
| средний | остальное |

Таблица моделей по `utsname.machine` не знает новых устройств и требует обновлений — отвергнута.

### 3. Профили уровней — наследники профилей класса
`ios-phone` и `ios-tablet` остаются средним уровнем; слабый и сильный наследуют от них и меняют только
отличия. Числа (до замеров — оценка по полосе и памяти):

| | phone-low | phone | phone-high | tablet-low | tablet | tablet-high |
|---|---|---|---|---|---|---|
| плотность low/med/high | 1 / 1,25 / 1,5 | 1,25 / 1,5 / 2 | 1,5 / 2 / 2,5 | 1 / 1,25 / 1,5 | 1 / 1,5 / 2 | 1,5 / 2 / 2 |
| ступень по умолчанию | low | low | medium | low | medium | medium |
| MSAA на high | нет | 4 | 4 | нет | 4 | 4 |
| тени med / high | 1024×1 / 1024×1 | 1024×1 / 2048×2 | как компьютер | 1024×1 / 1024×1 | как phone | как компьютер |
| обломков | 300 | 600 | 800 | 300 | 600 | 1000 |
| огни / вспышки | 1-2-3 / 2 | 2-3-4 / 3 | 3-4-6 / 4 | 1-2-3 / 2 | 2-3-4 / 3 | 4-6-8 / 6 |
| текстуры моделей | 512 | 1024 | 1024 | 512 | 1024 | 1024 |
| предел кадров | — | — | — | 60 | 60 | 60 |

iPhone без ключа `CADisableMinimumFrameDurationOnPhone` и так не больше 60 Гц.

### 4. Пределы по возможностям адаптера — в `quality.rs`
MSAA из профиля уменьшается до наибольшего числа выборок, которое адаптер поддерживает для
`Rgba16Float` (`RenderAdapter::get_texture_format_features`), с предупреждением. Это общее для всех
платформ (Android и браузер тоже не везде умеют 8).

### 5. Окно и Info.plist
Поля `Window` Bevy на iOS: `prefers_home_indicator_hidden = true`,
`preferred_screen_edges_deferring_system_gestures = ScreenEdge::Bottom` — без патчей. Info.plist:
`LSApplicationCategoryType = public.app-category.action-games`, `GCSupportsGameMode`, `LSSupportsGameMode`.
`UIApplicationSupportsIndirectInputEvents` не ставим: без него мышь и трекпад iPad приходят касаниями,
и управление касаниями работает с ними как есть.

### 6. Память и нагрев
Опрос `os_proc_available_memory` раз в 10 с уже есть; ниже 300 МБ — флаг «мало памяти»: предел лежащих
обломков вдвое меньше, в лог. Нагрев «critical» — предел кадров 30 поверх профиля
(общий ограничитель `profile::limit_frames`), «serious» и Low Power Mode — как сейчас, кадр ×0,8.
Фон: Bevy не рисует в состоянии `Suspended`; проверить на iPhone, что в логе нет ошибок Metal
`notPermitted` после «Домой».

## Risks / Trade-offs

- Числа уровней — оценка, не замер → правятся по замерам на iPhone 15 Pro Max (сильный) и iPad 9
  (слабый); средних устройств у нас нет — их числа остаются нынешними `ios-phone`/`ios-tablet`.
- Ключ A12 отсекает iPhone X и iPad 7 → это ~1–2 % устройств, и игра на них всё равно не шла бы.
- `os_proc_available_memory` при запуске зависит от фоновых приложений → порог 1,8 ГБ с запасом; уровень
  выбирается один раз за запуск.

## Источники

Metal Feature Set Tables; WWDC20 10621 «Support performance-intensive apps and games», 10602, 10632;
WWDC22 10106; WWDC24 10085; WWDC25 209; Tech Talk 111372 «Bring your high-end game to iPhone 15 Pro»;
документация `os_proc_available_memory`, `recommendedMaxWorkingSetSize`, `UIRequiredDeviceCapabilities`,
«Preparing your Metal app to run in the background», ProMotion; исходники wgpu-hal 30.0.2
(`metal/adapter.rs`) и Bevy 0.20.1.
