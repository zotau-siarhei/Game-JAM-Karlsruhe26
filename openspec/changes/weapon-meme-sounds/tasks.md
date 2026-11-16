# Tasks: weapon-meme-sounds (после базового оружия)

## 1. Ассеты
- [x] 1.1 ElevenLabs SFX: выстрелы 5 видов оружия, свист мортиры, взрывы трёх мощностей, перезарядка, холостой щелчок — сделано: перезарядка `reload` (2 варианта) и холостой щелчок `dry_fire` сгенерированы в веб-интерфейсе ElevenLabs
- [x] 1.2 Мем-звуки (свои): Vine boom, airhorn, hitmarker, oof, bonk, 8-бит бип, womp-womp, ошибка в стиле XP, ритм «tung tung» — сделано: свои мем-звуки `meme_boom`, `meme_airhorn`, `meme_hitmarker`, `meme_oof`, `meme_bonk`, `meme_beep`, `meme_womp`, `meme_error`, `meme_tung` (ElevenLabs, оригинальные, не сэмплы мемов); звучат под надписи `comic::ComicShown` (BOMBARDIRO!/KA-BOOM!, TUNG TUNG TUNG, WOMP WOMP, 6-7), на сериях убийств 3/5/10, на гибели лунохода, на выборе пустого оружия и на таране дома или валуна
- [x] 1.3 TTS BRAINROT/ARCADE: «Bombardiro Crocodilo!», «Ballerina Cappuccina!», «Six... seven», «Tra-la-le-ro», «Mamma mia» — сделано: `airstrike_fly`, `rockets_fire_meme`, `mg_reload`, `mortar_reload`, `rockets_reload` в `assets/voice/lines.json`, файлы озвучены

## 2. WeaponSfxPlugin
- [x] 2.1 `weapon_sfx.rs`: подписка на Messages оружия, обычные звуки fire/reload/empty/impact, `cargo build` проходит — сделано: перезарядки в игре нет, поэтому `reload` и «Six... seven.» (`mg_reload`) звучат в `src/weapon_sfx.rs` после очереди пулемёта от 40 пуль и паузы 0,4 с, а также при пополнении боезапаса (`mortar_reload`, `rockets_reload`, `ammo_crate`); `dry_fire` — на последнем заряде (`arsenal::AmmoRanOut`)
- [x] 2.2 Правила редкости: счётчик N-го выстрела, кулдаун 20 с на вид мема, отправка голосовых мемов в очередь VoicePlugin — сделано: `src/weapon_sfx.rs`, каждый 8-й снаряд мортиры и 12-я ракета; мем не чаще раза в 10 с, один и тот же — раз в 30 с (не 20), через отбор штаба `src/hq.rs`, а не очередь
- [x] 2.3 Комикс-надписи: UI/billboard-текст с анимацией scale + fade за 0.6–1.0 с — сделано: `src/comic.rs`, надписи UI у точки мира с выскакиванием и таянием за 0,8 с: BOMBARDIRO!/KA-BOOM! на взрывах игрока мощнее 1,3 (35 %), TUNG TUNG TUNG/6-7 на каждой 25-й пуле, WOMP WOMP на каждом 6-м импульсе лазера, BRUH на пустом оружии; одна надпись не чаще раза в 8 с
- [ ] 2.4 Juice: отдача, вспышка, тряска камеры по мощности — частично: тряска камеры по мощности — `src/impact.rs` (mortar-craters 4.3) — нет отдачи, вспышка у ствола только у мортиры — **в работе: Sergei Zotov**

## 2a. Исправления по архитектурному ревью

- [x] 2a.1 Н11: модуль `src/weapon_sfx.rs` (`WeaponSfxPlugin`): мем-поводы `mortar_fire_meme` на каждый 8-й снаряд мортиры игрока и `rockets_fire_meme` на каждую 12-ю ракету, через отбор штаба; повод `ammo_empty` — когда боезапас кончился (`arsenal::AmmoRanOut`). Звуков механизма перезарядки и холостого щелчка в `assets/sfx` нет — под них кода нет, задачи 1.1 и 2.1 открыты
- [ ] 2a.2 Проверить в игре: мем-реплики редки и не перебивают речь

## 3. Проверка
- [ ] 3.1 30 с непрерывной стрельбы: мемы не надоедают, нет наложений с речью
- [ ] 3.2 Нативная сборка (`cargo run`, целевое железо — MacBook Air M2): производительность не падает от надписей и звуков (пул сущностей, лимит одновременных звуков ~8)
- [ ] 3.3 Проверить в игре надписи 2.3: выскакивают у точки события, тают за 0,8 с, не чаще раза в 8 с; проверка: `cargo run`
