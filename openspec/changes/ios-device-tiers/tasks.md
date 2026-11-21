## 1. Установка и окно

- [ ] 1.1 `ios/Info.plist`: `iphone-ipad-minimum-performance-a12` в `UIRequiredDeviceCapabilities`, `LSApplicationCategoryType`, `GCSupportsGameMode`, `LSSupportsGameMode`; проверка: `scripts/build-ios.sh --sim --run`, установка на iPad 9 проходит
- [ ] 1.2 `src/main.rs`: на iOS `prefers_home_indicator_hidden`, `preferred_screen_edges_deferring_system_gestures = Bottom`; проверка: iPhone — свайп снизу во время забега не сворачивает игру с первого раза

## 2. Уровень устройства

- [ ] 2.1 `src/ios.rs`: `device_tier()` — семейство GPU (`MTLCreateSystemDefaultDevice`, `supportsFamily`), `physicalMemory`, `os_proc_available_memory`; строка в лог с уровнем и данными; проверка: `cargo check --target aarch64-apple-ios`, лог в симуляторе
- [ ] 2.2 `src/profile.rs`: выбор `ios-phone-low|ios-phone|ios-phone-high` и `ios-tablet-low|ios-tablet|ios-tablet-high` по уровню; профили в `assets/platform/profiles.json` по таблице решения 3; проверка: `cargo test` (каждый профиль собирается, низкий легче среднего, средний легче высокого)
- [ ] 2.3 Предел MSAA по возможностям адаптера в `src/quality.rs`; проверка: `cargo test`, Mac с `SPACE_GUARD_RENDER_MSAA_HIGH=16` — предупреждение, а не паника

## 3. Память и нагрев

- [ ] 3.1 Мало памяти (< 300 МБ до предела) — предел обломков вдвое, лог; проверка: `cargo test` на правило, лог на iPad 9 в долгом забеге
- [ ] 3.2 Нагрев «critical» — предел 30 кадров; проверка: `cargo test` на правило

## 4. Проверка на устройствах

- [ ] 4.1 iPad 9: уровень «слабый» в логе, забег 5 минут на low и medium — FPS из F3, память в логе; числа `ios-tablet-low` по замеру
- [ ] 4.2 iPhone 15 Pro Max: уровень «сильный», забег 5 минут на medium и high; «Домой» во время забега — в логе нет ошибок Metal; числа `ios-phone-high` по замеру
- [ ] 4.3 `CLAUDE.md`: уровни устройств iOS и ключи Info.plist; `openspec validate --all`
