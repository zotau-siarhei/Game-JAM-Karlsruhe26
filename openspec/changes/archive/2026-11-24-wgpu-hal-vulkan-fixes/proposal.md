## Why

На POCO X4 Pro 5G (Adreno 619) игра вылетает при сборке шейдеров и в забеге: видеокарта теряется, а при
закрытии wgpu-hal 30.0.2 паникует о занятом семафоре цепочки кадров. В trunk wgpu это исправлено (#10230),
как и ещё две ошибки Vulkan, бьющие по телефонам: пул дескрипторов без размеров, на котором падают драйверы
старше Vulkan 1.3.215 (#10124), и `unreachable!()` на `ERROR_FRAGMENTED_POOL` у Mali (#10264). В выпуски
30.x эти исправления не попали.

## What Changes

- Своя копия wgpu-hal 30.0.2 в `vendor/wgpu-hal` с тремя исправлениями, подключена `[patch.crates-io]` в
  `Cargo.toml` и `android/Cargo.toml`; `PATCH.md` описывает правки и порядок обновления.
- Тест `tests/vendor_patches.rs`: падает, если wgpu-hal или bevy_audio в `Cargo.lock` не из своей копии
  (обновление молча отключило бы заплатку).
- CLAUDE.md: при обновлении Bevy или wgpu пересматривать копии.

## Capabilities

### New Capabilities

### Modified Capabilities
- `android-app-integration`: Vulkan на телефонах со старыми драйверами Adreno и Mali.

## Impact

`vendor/wgpu-hal`, `Cargo.toml`, `android/Cargo.toml`, `Cargo.lock`, `tests/vendor_patches.rs`, `CLAUDE.md`.
На macOS и в браузере код Vulkan не собирается — поведение там не меняется.
