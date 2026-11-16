## Why

На iPhone 15 и iPad Air M3 (Safari) игра иногда не запускается: «CANNOT START — RangeError:
Length out of range of buffer» сразу после загрузки модуля (Sergei Zotov, 2026-11-17).
Воспроизведено в Safari на Mac: WebKit иногда отдаёт из `getMappedRange` пустой ArrayBuffer для
буфера, созданного с `mappedAtCreation` (буфер кластеров света 260 байт, получено 0 байт),
и wgpu падает в `create_buffer_init`. Сбой плавающий, видеопамять ни при чём.

## What Changes

- Страница `web/index.html` перехватывает `createBuffer`, `getMappedRange` и `unmap`: если отображённая
  память короче запрошенной, игра получает свою память нужного размера, а при `unmap` она пишется
  в буфер через `queue.writeBuffer`. Буферам, созданным отображёнными без MAP_READ/MAP_WRITE,
  для этого добавляется COPY_DST.
- Пока WebKit отдаёт память правильно, всё идёт как раньше.

## Capabilities

### New Capabilities
<!-- нет -->

### Modified Capabilities
- `web-memory`: запуск в Safari не падает на пустой отображённой памяти буфера.

## Impact

- `web/index.html`, `CLAUDE.md`.
