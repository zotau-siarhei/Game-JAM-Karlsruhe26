## 1. Урон по своим

- [x] 1.1 `src/combat.rs`: враг → враг полный урон и попадание, `ShotFrom` и `spared_ally`, `AllyProof` в `apply_damage`; тесты; проверка: `cargo test` — сделано: тесты `shot_spares_its_shooter_but_not_other_allies`, `ally_proof_ignores_own_team_damage`, `enemy_bullet_hits_another_enemy`, `bullet_spares_the_enemy_it_was_fired_from`
- [x] 1.2 `src/bullet.rs`, `src/rockets.rs`, `src/laser.rs`: стрелок не задевает себя; `src/boss.rs`: `AllyProof`; `src/audio_events.rs`: без `kill` за убийство своими; проверка: `cargo test`, прогон `--bench` с врагами — сделано: за 120 с `--bench=120 --immortal` враги убили друг друга 14 раз, луноход-автопилот — 6, паник нет; глазами в бою не смотрел
