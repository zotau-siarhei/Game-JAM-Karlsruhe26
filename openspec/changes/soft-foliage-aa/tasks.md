## 1. Мягкие кроны

- [ ] 1.1 Поле `soft_crown` записи модели в `src/models.rs`, смягчение нормалей мешей после упрощения и материалы без переворота нормали (`Material{i}/std`); `soft_crown: 0.7` у `tree_oak`, `tree_spruce`, `tree_poplar`, `tree_pine`, `bush`. Проверка: `cargo test` (тест смягчения: нормаль верхушки смотрит вверх, у ствола не меняется); снимок `SPACE_GUARD_SCENE` с деревьями — кроны без тёмных пятен — **в работе: Sergei Zotov**

## 2. FXAA

- [ ] 2.1 `render.fxaa` по ступеням в `src/profile.rs` и `assets/platform/profiles.json`, тест профилей. Проверка: `cargo test profile` — **в работе: Sergei Zotov**
- [ ] 2.2 `quality::apply_quality`: FXAA на игровой камере, когда ступень разрешает и MSAA нет; `off fxaa`. Проверка: снимки Low и Medium — края без лесенки; High на Mac — MSAA без FXAA — **в работе: Sergei Zotov**
- [ ] 2.3 Прогрев: после запуска и смен ступени Low → Medium → High в логе нет «Поздний конвейер» с FXAA и «Материал нового класса после прогрева». Проверка: лог запуска — **в работе: Sergei Zotov**
- [ ] 2.4 Замер `--bench` на Mac: Medium до и после. Проверка: итог в этот файл — **в работе: Sergei Zotov**
