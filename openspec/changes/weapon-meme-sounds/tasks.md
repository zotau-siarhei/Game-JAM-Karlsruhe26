# Tasks: weapon-meme-sounds (после базового оружия)

## 1. Ассеты
- [ ] 1.1 ElevenLabs SFX: выстрелы 5 видов оружия, свист мортиры, взрывы трёх мощностей, перезарядка, холостой щелчок
- [ ] 1.2 Мем-звуки (свои): Vine boom, airhorn, hitmarker, oof, bonk, 8-бит бип, womp-womp, ошибка в стиле XP, ритм «tung tung»
- [ ] 1.3 TTS BRAINROT/ARCADE: «Bombardiro Crocodilo!», «Ballerina Cappuccina!», «Six... seven», «Tra-la-le-ro», «Mamma mia»

## 2. WeaponSfxPlugin
- [ ] 2.1 `weapon_sfx.rs`: подписка на Messages оружия, обычные звуки fire/reload/empty/impact, `cargo build` проходит
- [ ] 2.2 Правила редкости: счётчик N-го выстрела, кулдаун 20 с на вид мема, отправка голосовых мемов в очередь VoicePlugin
- [ ] 2.3 Комикс-надписи: UI/billboard-текст с анимацией scale + fade за 0.6–1.0 с
- [ ] 2.4 Juice: отдача, вспышка, тряска камеры по мощности

## 3. Проверка
- [ ] 3.1 30 с непрерывной стрельбы: мемы не надоедают, нет наложений с речью
- [ ] 3.2 WASM: производительность не падает от надписей и звуков (пул сущностей, лимит одновременных звуков ~8)
