## 1. Перехватчик как тип врага

- [ ] 1.1 `EnemyKind::Interceptor` в `src/enemy.rs`: здоровье 15, скорость 20 м/с, поворот 1,6 рад/с, корпус 1,0 × 4,4 м, `GroundVehicle` с опорой 0,5 × 2,2, круговой `Collider` 1,3 м, компонент `Interceptor`; `carried_weapon` возвращает `Option`, у перехватчика оружия нет; запасной облик из примитивов (длинный корпус, клин на носу, два сопла, четыре колеса); `chase_rover` получает `Without<Interceptor>`; проверка: `cargo test` с тестами `interceptor_is_faster_than_rover` и «перехватчик ничего не роняет», `cargo build`
- [ ] 1.2 Директор волн не считает перехватчиков в `MAX_ALIVE`, `buy_wave` их не покупает; числа перехвата (`FLIGHT_WINDOW`, `FLIGHT_DISTANCE`, `CALM_DISTANCE`, `CONTACT_RADIUS`, `ESCAPE_TRIGGER`, `REINFORCE_GAP`, `LINGER_TIME`, `SHADOW_DISTANCE`, `RAM_COOLDOWN`, `IMPACT_FACTOR`, `HIT_SPEED_KEEP`, `VOICE_GAP`, `pack_size`) — в `waves::balance`, `INTERCEPTOR_RAM` — в `waves::enemy_damage`; проверка: `cargo test` с тестом «волна не содержит перехватчиков» и тестом `pack_size` (2 в начале, не больше 4)

## 2. Бегство

- [ ] 2.1 Модуль `src/interceptor.rs` с `InterceptorPlugin`: ресурс `Flight` (окно позиций по 0,25 с за 4 с, накопитель, `active`), сброс по `WorldRebuilt` в `GameSet::Reset`, система `track_flight` в `GameSet::Simulation`; чистая функция `flight_frame(displacement, near_enemy, pursuers)` и правило конца перехвата; проверка: `cargo test` с тестами «прямая на полном ходу от отставших — бегство», «стоит — нет», «круг радиусом 25 м — нет», «враг в 30 м — нет», «врагов нет — нет», «перехват не мигает при скорости 7 м/с», «конец при подходе обычного врага»
- [ ] 2.2 `dispatch_interceptors`: при начале перехвата звено `pack_size(threat)` через `SpawnEnemy { place: Edge { angle } }` с флангов впереди по ходу (50°–100° от курса), подкрепление по одному раз в 6 с; проверка: `cargo test` с тестом угла появления (точка впереди-сбоку от направления движения), `cargo run`: оторвавшись от волны по прямой, через ~3 с видишь перехватчиков с боков впереди

## 3. Атака и замедление

- [ ] 3.1 Компонент `rover::Impact { left }`: ставится луноходу при появлении, `drive_rover` тикает его и умножает целевую скорость на `IMPACT_FACTOR` вместе с `SpeedFactor`; сброс по `WorldRebuilt`; проверка: `cargo test` с тестами «удар режет потолок до 0,55», «повторные удары продлевают, но не опускают ниже и не дольше 3 с», «ремонт и удар складываются, конец ремонта не снимает удар»
- [ ] 3.2 `drive_interceptors` перед `steering::steer_vehicles`: состояния Attack (курс на точку упреждения `lead_point`), Recoil, Shadow, Leave (удаление дальше 70 м); проверка: `cargo test` с тестом `lead_point` (точка впереди едущего лунохода, у стоящего — сам луноход, упреждение не больше 1,5 с), `cargo run`: перехватчики идут наперерез, а не в хвост
- [ ] 3.3 `ram_by_interceptors` после `vehicle::move_ground_vehicles`: касание отрезка корпуса (`segment_contact`) — `Damage` на `INTERCEPTOR_RAM`, сброс скорости лунохода × 0,3, `Impact`, выталкивание перехватчика, Recoil на 1,5 с, сообщение `InterceptorHit { position }`; проверка: `cargo test` с тестами «удар носом», «въезд лунохода в борт — удар», «рядом, но не касается — нет», `cargo run`: после удара луноход ощутимо тормозит, перехватчик уходит на разворот

## 4. Отклик игроку

- [ ] 4.1 Мини-карта: `Mark::Interceptor` — вытянутая узкая метка цвета врагов, в `update_markers` сразу после босса, обычные враги без перехватчиков; проверка: `cargo run`, метки перехватчиков на мини-карте отличаются от обычных врагов
- [ ] 4.2 Звук и реплика в `src/audio_events.rs`: `PlaySound::at("interceptor_hit", position)` по `InterceptorHit`, `VoiceCue::new("interceptors")` при начале перехвата не чаще раза в 30 с; записи в `assets/sfx/sounds.json` и `assets/voice/lines.json`, генерация через `scripts/gen_audio.py` и `scripts/gen_voice.py` (если ключа ElevenLabs нет — на время звучат готовые `rover_hit` и реплика без файла пропускается); проверка: `cargo test` (разбор списков), `cargo run`: удар слышно, штаб предупреждает о перехватчиках
- [ ] 4.3 Промпт модели `enemy_interceptor` в `assets/models/MESHY_DESIGN.md` (раздел «Желательно», бюджет 5000); запись в `models.json` и `enemy_model` — когда появится `enemy_interceptor.glb`, с подгонкой `scale` и `rotation_y`; проверка: промпт в файле; с моделью — `cargo run`, перехватчик вдвое длиннее таран-стрелка, нос по ходу движения

## 5. Проверка целиком

- [ ] 5.1 Ручной прогон: уехать от первой волны по прямой, покружить на месте при врагах за 40 м, драться на ходу рядом с врагами, встать и отстреливаться при перехватчиках рядом; проверка: `cargo run` — перехват только в первом случае, волна догоняет замедленный луноход, перехватчики сбиваются пулемётом, в остальных случаях не таранят; при надобности подстроить числа в `waves::balance`, `cargo build --release` идёт плавно на MacBook Air M2
