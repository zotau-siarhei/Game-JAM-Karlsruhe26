## Why

Геймпад делали и проверяли на macOS. Игрок собирается играть и на Linux; код платформ лежал
вперемешку, а на Linux не было световой панели.

## What Changes

- Код геймпада разложен по платформам: `src/gamepad/macos.rs` (ввод через GameController),
  `src/haptics/macos.rs` (вибрация и свет через CoreHaptics), `src/haptics/bevy_rumble.rs`
  (вибрация через gilrs: Linux, Windows), `src/haptics/linux_light.rs` (световая панель на Linux).
- Световая панель DualShock 4 и DualSense на Linux — через светодиоды драйвера ядра в
  `/sys/class/leds`; без прав на запись в лог один раз пишется правило udev.
- Модули других платформ собираются в тестах на любой системе.

Вне рамок: браузер, световая панель на Windows.

## Capabilities

### New Capabilities
Нет.

### Modified Capabilities
- `gamepad`: требование о платформах.

## Impact

`src/gamepad.rs`, `src/haptics.rs` и новые подмодули; `src/gc_input.rs` переехал в `src/gamepad/macos.rs`; CLAUDE.md.
