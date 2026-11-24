## 1. Копия wgpu-hal

- [x] 1.1 `vendor/wgpu-hal` из 30.0.2, правки #10230, #10124, #10264, `PATCH.md`
- [x] 1.2 `[patch.crates-io]` в `Cargo.toml` и `android/Cargo.toml`

## 2. Чтобы не потерять при обновлении

- [x] 2.1 Тест `tests/vendor_patches.rs` для wgpu-hal и bevy_audio
- [x] 2.2 CLAUDE.md, раздел «Зависимости»

## 3. Проверка

- [x] 3.1 `cargo test`; сборка под Android; эмулятор: меню, забег, `setprop debug.spaceguard.lose` — игра открывается заново без паники о семафоре
