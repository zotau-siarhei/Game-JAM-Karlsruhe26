## Why

Игра есть для настольных систем, браузера и Android. На iPhone браузерная версия идёт через WebKit:
страница переписывает шейдеры, полного экрана нет, память вкладки ограничена. Нужна нативная сборка под
iOS: приложение, которое ставится на iPhone и в симулятор напрямую, с тем же сенсорным управлением.

## What Changes

- `scripts/build-ios.sh` собирает игру под iOS (телефон или симулятор) и упаковывает приложение `.app`
  без проекта Xcode: исполняемый файл, `Info.plist`, ассеты, иконка; подписывает и ставит.
- На iOS рекорды и настройки лежат в `Library/Application Support` песочницы приложения, качество
  по умолчанию низкое, игра альбомная и на весь экран.
- Сенсорные кнопки обходят вырез, Dynamic Island и полоску «домой»: отступы безопасной области из UIKit.
- Геймпад и его вибрация на iOS идут через GameController, как на macOS.

## Capabilities

### New Capabilities
- `ios-build`: сборка и запуск игры как приложения iOS.

### Modified Capabilities

## Impact

- Новые файлы: `ios/Info.plist`, `scripts/build-ios.sh`, `src/ios.rs`.
- Правки: `Cargo.toml` (зависимости для iOS), `src/main.rs`, `src/platform.rs`, `src/aim.rs`,
  `src/gamepad.rs`, `src/haptics.rs`, `CLAUDE.md`.
- Для сборки нужны Xcode и Rust-таргеты `aarch64-apple-ios`, `aarch64-apple-ios-sim`; для телефона —
  сертификат Apple Development и профиль подготовки.
