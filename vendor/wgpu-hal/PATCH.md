# Копия wgpu-hal 30.0.2

Это wgpu-hal 30.0.2 с crates.io. В него перенесены три исправления Vulkan из trunk wgpu, которые не попали
в выпуски 30.x. Подключается через `[patch.crates-io]` в `Cargo.toml` и `android/Cargo.toml`.

- [#10230](https://github.com/gfx-rs/wgpu/pull/10230), `src/vulkan/swapchain/native.rs`.
  Если удалить поверхность, когда взятый кадр ещё жив, `release_resources` больше не паникует
  («Trying to destroy a SwapchainAcquireSemaphore that is still in use by a SurfaceTexture»), а пропускает
  занятый семафор. Такой кадр остаётся после потери видеокарты: `Surface::discard` в wgpu-core видит
  потерянное устройство и выходит раньше. Этот вылет был на POCO X4 Pro 5G (Adreno 619). Задача wgpu —
  #9277.
- [#10124](https://github.com/gfx-rs/wgpu/pull/10124), `src/vulkan/descriptor.rs`, `create_descriptor_pool`.
  Пул дескрипторов для раскладки без дескрипторов (пустая группа привязок) получает одну запись сэмплера.
  До Vulkan 1.3.215 `poolSizeCount == 0` запрещён, и старые драйверы (Adreno 6xx на MIUI) на нём падают.
- [#10264](https://github.com/gfx-rs/wgpu/pull/10264), `src/vulkan/descriptor.rs`, `alloc` и `free`.
  `ERROR_FRAGMENTED_POOL` и `ERROR_OUT_OF_POOL_MEMORY` больше не попадают в `unreachable!()`. Пул
  сбрасывается или помечается фрагментированным, и выделение повторяется в другом пуле. Mali отказывает
  так даже в пуле одинаковых наборов.

Как обновить: когда в Cargo.lock появится wgpu-hal, в котором есть все три исправления (31+), копию удалить
вместе со строкой `wgpu-hal` в `[patch.crates-io]`. При обновлении на другой 30.x — снова скопировать крейт
из `~/.cargo/registry` и перенести эти три правки.

Тест `tests/vendor_patches.rs` падает, если в `Cargo.lock` wgpu-hal взят не из этой копии. Так обновление
wgpu не пройдёт мимо этих правок.
