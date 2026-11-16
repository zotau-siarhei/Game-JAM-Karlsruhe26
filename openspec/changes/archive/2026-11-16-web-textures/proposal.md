## Why

Браузерная версия на Android (Chrome 157, Nothing A059, Snapdragon SM7635, Adreno 8xx) не
запускается: загрузка обрывается с «GPU device lost: A valid external Instance reference no longer
exists». Отладка по USB показала падение процесса видеокарты Chrome (SIGSEGV) внутри
`vkCreateGraphicsPipelines` — в компиляторе шейдеров Adreno (`libllvm-qgl.so`) на первом же
PBR-конвейере. Отдельная страница с одним шейдером и перебор его частей сузили причину до одного
вызова: `textureSampleCompareLevel` по `texture_depth_cube_array` — выборка тени точечного
источника. Без него шейдер компилируется за ~0,3 с.

Заодно сделано то, что сначала считалось причиной, — память: почти у всех 33 моделей текстуры
2048×2048, около 560 МБ видеопамяти и столько же копией в памяти игры. Для телефона это лишнее
и без падения.

## What Changes

- Страница браузерной версии перед компиляцией заменяет выборку тени точечного источника
  (`textureSampleCompareLevel` по `point_shadow_textures`) на 1.0 — во всех браузерах. У всех
  точечных источников в игре тени выключены (`shadow_maps_enabled: false`), выборка и так давала 1.0.
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

- `web/index.html`: замена выборки тени точечного источника в шейдерах.
- `scripts/web_textures.py` (новый), `scripts/build-web.sh`, `.github/workflows/web.yml`.
- `src/models.rs`: загрузка моделей с `texture_in_gpu_only`.
- Сборке браузерной версии нужен Pillow: `python3-pil` на Ubuntu; на macOS без него скрипт
  берёт Pillow через `uv`.
