## Why

Браузерная версия на Android (Chrome, Adreno 8xx) не запускается: на ~89 МБ из 151 загрузка
обрывается с «GPU device lost: A valid external Instance reference no longer exists». Так
Chrome сообщает, что процесс видеокарты убит, — чаще всего по памяти. Почти у всех 33 моделей
текстуры 2048×2048: в несжатом виде это около 560 МБ видеопамяти, и ещё столько же держит копия
картинок в памяти игры. Уменьшение разрешения холста (0ab6541) не помогло: оно экономит десятки
мегабайт, а не сотни.

## What Changes

- Браузерная сборка уменьшает текстуры внутри моделей до 1024 пикселей по большей стороне
  (`scripts/web_textures.py` из `scripts/build-web.sh`): видеопамяти и загрузки вчетверо меньше,
  модели в браузере весят 59 МБ вместо 120. Исходники в `assets/` не меняются, натив их не трогает.
- Текстуры моделей после передачи в видеокарту больше не держатся в памяти игры
  (`GltfLoaderSettings::load_materials = RENDER_WORLD`) — и в браузере, и в нативной версии.
- CI ставит Pillow (`python3-pil`) для уменьшения текстур.

## Capabilities

### New Capabilities
- `web-memory`: браузерная версия помещается в память телефона.

### Modified Capabilities
<!-- нет -->

## Impact

- `scripts/web_textures.py` (новый), `scripts/build-web.sh`, `.github/workflows/web.yml`.
- `src/models.rs`: загрузка моделей с `texture_in_gpu_only`.
- Сборке браузерной версии нужен Pillow: `python3-pil` на Ubuntu; на macOS без него скрипт
  берёт Pillow через `uv`.
