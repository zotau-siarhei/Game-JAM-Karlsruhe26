//! Волны врагов: угроза растёт со временем и разрушениями, директор по ней собирает волны.
//!
//! Все числа, от которых зависит длина забега, лежат в [`balance`]: настраивать их там.

use std::f32::consts::TAU;

use bevy::prelude::*;

use crate::combat::{Died, Health, Team};
use crate::boss::SpawnBoss;
use crate::buildings::House;
use crate::capsule::{Capsule, DropCapsule};
use crate::settlements::{HouseStatus, Settlements};
use crate::enemy::{Enemy, EnemyKind, SpawnEnemy, SpawnPlace};
use crate::interceptor::Interceptor;
use crate::explosion::Explosion;
use crate::frame::{GameSet, RunMessages};
use crate::laser::FireLaser;
use crate::props::Prop;
use crate::rng::GameRng;
use crate::rover::Rover;
use crate::buildings::{BuildingDestroyed, DestroyCause};
use crate::run::{RunScoped, WorldRebuilt};
use crate::terrain::Terrain;

/// Кривые баланса: всё, что зависит от угрозы.
pub mod balance {
    /// Вес кратера в разрушениях.
    pub const CRATER_WEIGHT: f32 = 1.0;

    /// Вес дома, снесённого игроком или провалившегося в кратер, в разрушениях.
    pub const HOUSE_WEIGHT: f32 = 4.0;

    /// Вес лазерной траншеи игрока в разрушениях.
    pub const TRENCH_WEIGHT: f32 = 0.5;

    /// Вес объекта (дерева, валуна, кристалла), разрушенного игроком, в разрушениях.
    pub const OBJECT_WEIGHT: f32 = 0.5;

    /// Сколько разрушений равны одной минуте забега.
    const DESTRUCTION_PER_MINUTE: f32 = 40.0;

    /// Передышка перед первой волной, с.
    pub const FIRST_LULL: f32 = 4.0;

    /// Сколько врагов может быть одновременно: лишние ждут в очереди.
    pub const MAX_ALIVE: usize = 24;

    /// Пауза между появлениями врагов одной волны, с.
    pub const SPAWN_GAP: f32 = 0.35;

    /// Разброс врагов вокруг направления волны, рад.
    pub const DIRECTION_SPREAD: f32 = 25.0 * std::f32::consts::PI / 180.0;

    /// С какой угрозы враги начинают прибывать в десантных капсулах.
    pub const CAPSULE_THREAT: f32 = 1.0;

    /// Доля врагов волны, прибывающих в капсулах.
    pub const CAPSULE_SHARE: f32 = 0.3;

    /// Доля врагов волны, выезжающих из домов, если в секторе волны есть целый дом.
    pub const HOUSE_SHARE: f32 = 0.3;

    /// Дома дальше этого от лунохода врагов не выпускают, м.
    pub const HOUSE_RANGE: f32 = 50.0;

    /// Полуширина сектора волны, в котором ищется дом, рад.
    pub const HOUSE_SECTOR: f32 = 40.0 * std::f32::consts::PI / 180.0;

    /// Прибывает ли враг в капсуле: `roll` — случайное число от 0 до 1.
    pub fn uses_capsule(threat: f32, roll: f32) -> bool {
        threat >= CAPSULE_THREAT && roll < CAPSULE_SHARE
    }

    /// С какой угрозы в волнах появляются мортирщики.
    pub const MORTAR_THREAT: f32 = 0.8;

    /// Каждая какая волна завершается боссом.
    pub const BOSS_EVERY: u32 = 4;

    /// С какой угрозы появляются ракетные танки.
    pub const ROCKET_TANK_THREAT: f32 = 1.6;

    /// С какой угрозы появляются лазерные танки.
    pub const TANK_THREAT: f32 = 2.5;

    /// Угроза: минуты забега плюс разрушения.
    pub fn threat(seconds: f32, destruction: f32) -> f32 {
        seconds / 60.0 + destruction / DESTRUCTION_PER_MINUTE
    }

    /// Передышка между волнами, с.
    pub fn lull_seconds(threat: f32) -> f32 {
        (14.0 - 1.5 * threat).max(6.0)
    }

    /// Бюджет волны в очках.
    pub fn wave_budget(threat: f32) -> u32 {
        (4.0 + 3.0 * threat).floor() as u32
    }

    /// Насколько быстрее растёт вес дорогого типа за единицу угрозы сверх [`TANK_THREAT`]
    /// и за каждое очко его стоимости сверх одного.
    pub const WEIGHT_GROWTH: f32 = 0.25;

    /// Вес типа стоимостью `cost` при покупке волны. До [`TANK_THREAT`] все типы равновероятны,
    /// дальше дорогие типы выпадают всё чаще: состав волн тяжелеет и после третьей минуты.
    pub fn kind_weight(cost: u32, threat: f32) -> f32 {
        1.0 + WEIGHT_GROWTH * cost.saturating_sub(1) as f32 * (threat - TANK_THREAT).max(0.0)
    }

    // Перехват (`src/interceptor.rs`): перехватчики приходят, когда игрок уезжает от волны.

    /// За какое окно считается смещение лунохода, с. Круг диаметром меньше
    /// [`FLIGHT_DISTANCE`] за любое окно бегством не считается.
    pub const FLIGHT_WINDOW: f32 = 6.0;

    /// Как часто запоминается позиция лунохода в окне, с.
    pub const FLIGHT_SAMPLE: f32 = 0.25;

    /// Смещение за окно, с которого игрок отрывается от погони, м: 9 м/с таран-стрелка × окно.
    pub const FLIGHT_DISTANCE: f32 = 54.0;

    /// Смещение за окно, ниже которого луноход стоит или кружит и перехват кончается, м.
    pub const CALM_DISTANCE: f32 = 18.0;

    /// Обычный враг ближе этого — бой, а не бегство, м: край кадра.
    pub const CONTACT_RADIUS: f32 = 30.0;

    /// Сколько секунд бегства подряд нужно, чтобы начался перехват.
    pub const ESCAPE_TRIGGER: f32 = 3.0;

    /// Пауза между подкреплениями звена во время перехвата, с.
    pub const REINFORCE_GAP: f32 = 6.0;

    /// Сколько перехватчики держатся рядом после конца перехвата, прежде чем уехать, с.
    pub const LINGER_TIME: f32 = 15.0;

    /// На каком расстоянии сбоку от лунохода держится перехватчик вне перехвата, м.
    pub const SHADOW_DISTANCE: f32 = 20.0;

    /// Уехавший перехватчик дальше этого от лунохода удаляется, м.
    pub const LEAVE_DISTANCE: f32 = 70.0;

    /// Сколько перехватчик после удара едет мимо, прежде чем зайти снова, с.
    pub const RAM_COOLDOWN: f32 = 1.5;

    /// Наибольшее упреждение курса перехвата, с.
    pub const LEAD_MAX: f32 = 1.5;

    /// Угол появления звена от направления движения лунохода, рад: от и до, сторона случайная.
    pub const FLANK_ANGLE: (f32, f32) = (50.0 * std::f32::consts::PI / 180.0, 100.0 * std::f32::consts::PI / 180.0);

    /// Потолок скорости лунохода после удара — доля полной скорости.
    pub const IMPACT_FACTOR: f32 = 0.55;

    /// Какая доля скорости лунохода остаётся сразу после удара.
    pub const HIT_SPEED_KEEP: f32 = 0.3;

    /// На сколько удар продлевает ограничение скорости, с, и дольше какого оно не копится.
    pub const IMPACT_TIME: f32 = 1.5;
    pub const IMPACT_MAX: f32 = 3.0;

    /// Реплика штаба о перехватчиках не чаще раза в столько секунд.
    pub const VOICE_GAP: f32 = 30.0;

    /// Размер звена перехвата при угрозе `threat`: 2 в начале, не больше 4.
    pub fn pack_size(threat: f32) -> usize {
        (2 + (threat.max(0.0) / 2.0).floor() as usize).min(4)
    }
}

/// Урон врагов: все удары по луноходу в одном месте, долями его здоровья.
///
/// Пропорции взяты из истории: пуля и таран таран-стрелка настраивались, пока у лунохода было
/// 100–300 здоровья, и последние игровые пропорции — при 300. Лазер танка и таран босса появились,
/// когда здоровье уже было 1500, их доли оставлены как есть. Баланс в игре не замерен.
pub mod enemy_damage {
    use crate::combat::ROVER_MAX_HEALTH;

    /// Пуля таран-стрелка: 4 из 300.
    pub const BULLET: f32 = ROVER_MAX_HEALTH * (4.0 / 300.0);

    /// Таран таран-стрелка, не чаще раза в 0,5 с: 12 из 300.
    pub const RAM: f32 = ROVER_MAX_HEALTH * (12.0 / 300.0);

    /// Импульс лазера тяжёлого танка: 70 из 1500.
    pub const TANK_LASER: f32 = ROVER_MAX_HEALTH * (70.0 / 1500.0);

    /// Таран босса, один удар за атаку: 90 из 1500.
    pub const BOSS_RAM: f32 = ROVER_MAX_HEALTH * (90.0 / 1500.0);

    /// Удар перехватчика: 1,5 % здоровья, меньше тарана таран-стрелка. Главное в ударе — замедление.
    pub const INTERCEPTOR_RAM: f32 = ROVER_MAX_HEALTH * 0.015;

    /// Мощность снаряда мортирщика. Прямое попадание отнимает `combat::EXPLOSION_DAMAGE` × мощность
    /// (35), но мощность задаёт и размер кратера, поэтому долей здоровья её не поднять: для этого
    /// урон снаряда надо отделить от мощности взрыва в `mortar.rs` и `combat.rs`.
    pub const MORTAR_POWER: f32 = 0.35;

    /// Мощность снаряда залпа босса: см. [`MORTAR_POWER`].
    pub const BOSS_SALVO_POWER: f32 = 0.6;

    // Ракеты ракетного танка и веера босса бьют как ракеты игрока: мощность задана в `rockets.rs`.
}

/// Подключает угрозу, директора волн и объявление волны.
pub struct WavesPlugin;

impl Plugin for WavesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Threat>()
            .init_resource::<Director>()
            .add_run_message::<WaveStarted>()
            .add_systems(
                Update,
                (
                    reset_waves.in_set(GameSet::Reset),
                    (grow_threat, direct_waves).chain().in_set(GameSet::Simulation),
                    count_broken_objects.in_set(GameSet::Death),
                    (announce_wave, fade_banners).chain().in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Угроза текущего забега. Только растёт.
#[derive(Resource, Default, Debug)]
pub struct Threat {
    seconds: f32,
    destruction: f32,
}

impl Threat {
    /// Значение угрозы: около единицы за минуту спокойного забега.
    pub fn value(&self) -> f32 {
        balance::threat(self.seconds, self.destruction)
    }

    fn add_time(&mut self, seconds: f32) {
        self.seconds += seconds.max(0.0);
    }

    /// Добавляет разрушение с весом из [`balance`].
    pub fn add_destruction(&mut self, weight: f32) {
        self.destruction += weight.max(0.0);
    }
}

/// Типы, доступные при угрозе `threat`, и их стоимость в очках бюджета.
fn available_kinds(threat: f32) -> Vec<(EnemyKind, u32)> {
    let mut kinds = vec![(EnemyKind::RamShooter, 1)];
    if threat >= balance::MORTAR_THREAT {
        kinds.push((EnemyKind::Mortar, 2));
    }
    if threat >= balance::ROCKET_TANK_THREAT {
        kinds.push((EnemyKind::HeavyTank, 4));
    }
    if threat >= balance::TANK_THREAT {
        kinds.push((EnemyKind::LaserTank, 5));
    }
    kinds
}

/// Состав волны: случайные покупки из доступных типов с весами [`balance::kind_weight`],
/// пока хватает бюджета. `roll` выдаёт случайное число от 0 до 1.
fn buy_wave(threat: f32, mut roll: impl FnMut() -> f32) -> Vec<EnemyKind> {
    let kinds = available_kinds(threat);
    let mut budget = balance::wave_budget(threat);
    let mut wave = Vec::new();
    loop {
        let affordable: Vec<_> = kinds.iter().filter(|(_, cost)| *cost <= budget).collect();
        let Some(&&last) = affordable.last() else {
            return wave;
        };
        let total: f32 = affordable.iter().map(|(_, cost)| balance::kind_weight(*cost, threat)).sum();
        let mut pick = roll().clamp(0.0, 1.0) * total;
        let (kind, cost) = affordable
            .iter()
            .copied()
            .find(|(_, cost)| {
                pick -= balance::kind_weight(*cost, threat);
                pick < 0.0
            })
            .copied()
            .unwrap_or(last);
        budget -= cost;
        wave.push(kind);
    }
}

/// Началась волна с номером `number`, считая с единицы.
#[derive(Message, Debug, Clone, Copy)]
pub struct WaveStarted {
    pub number: u32,
}

/// Директор волн: передышка, затем враги волны появляются по очереди.
#[derive(Resource, Debug)]
struct Director {
    /// Номер последней начатой волны.
    wave: u32,
    phase: Phase,
}

#[derive(Debug)]
enum Phase {
    /// Сколько секунд осталось до следующей волны.
    Lull(f32),
    /// Направления врагов, которые ещё не появились, и время до следующего.
    Spawning { queue: Vec<(f32, EnemyKind)>, timer: f32 },
}

impl Default for Director {
    fn default() -> Self {
        Self {
            wave: 0,
            phase: Phase::Lull(balance::FIRST_LULL),
        }
    }
}

/// Объявление волны на экране: гаснет и исчезает.
#[derive(Component)]
struct WaveBanner {
    remaining: f32,
}

/// Сколько секунд объявление волны видно.
const BANNER_SECONDS: f32 = 1.2;

/// Мир перестроен: угроза с нуля, первая волна после начальной передышки.
fn reset_waves(mut started: MessageReader<WorldRebuilt>, mut threat: ResMut<Threat>, mut director: ResMut<Director>) {
    if started.read().count() > 0 {
        *threat = Threat::default();
        *director = Director::default();
    }
}

/// Длина луча лазера, м: как в `laser.rs`.
const LASER_REACH: f32 = 40.0;

/// Лазер режет рельеф на 0,4 м ниже луча (`laser.rs`); с небольшим запасом, м.
const TRENCH_PROBE: f32 = 0.45;

/// Прорезает ли луч из `origin` по `direction` траншею: рельеф на пути выше дна траншеи.
/// Верно и до прореза, и сразу после него: дно траншеи как раз на этой высоте.
fn beam_cuts(height: impl Fn(f32, f32) -> f32, origin: Vec3, direction: Vec3) -> bool {
    let dir = direction.with_y(0.0).normalize_or(Vec3::NEG_Z);
    let floor = origin.y - TRENCH_PROBE;
    (1..=(LASER_REACH / 0.5) as usize).any(|i| {
        let p = origin + dir * (i as f32 * 0.5);
        height(p.x, p.z) > floor
    })
}

/// Угроза растёт со временем и с разрушениями (требование 6.3): каждый кратер от оружия игрока,
/// каждая лазерная траншея игрока, каждый дом, снесённый игроком или провалившийся в кратер.
fn grow_threat(
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut explosions: MessageReader<Explosion>,
    mut lasers: MessageReader<FireLaser>,
    mut houses: MessageReader<BuildingDestroyed>,
    mut threat: ResMut<Threat>,
) {
    threat.add_time(time.delta_secs());
    for explosion in explosions.read() {
        if explosion.source_team == Team::Player {
            threat.add_destruction(balance::CRATER_WEIGHT);
        }
    }
    for shot in lasers.read() {
        if shot.team == Team::Player && beam_cuts(|x, z| terrain.height_at(x, z), shot.origin, shot.direction) {
            threat.add_destruction(balance::TRENCH_WEIGHT);
        }
    }
    for house in houses.read() {
        if matches!(house.cause, DestroyCause::Player | DestroyCause::Collapse) {
            threat.add_destruction(balance::HOUSE_WEIGHT);
        }
    }
}

/// Объект (дерево, валун, кристалл), разрушенный игроком, добавляет угрозы. Стоит в
/// `GameSet::Death`: погибший объект ещё на месте и узнаётся по компоненту.
fn count_broken_objects(mut died: MessageReader<Died>, props: Query<(), With<Prop>>, mut threat: ResMut<Threat>) {
    for death in died.read() {
        if death.killer == Team::Player && props.contains(death.entity) {
            threat.add_destruction(balance::OBJECT_WEIGHT);
        }
    }
}

/// Ведёт волны: после передышки покупает состав, раздаёт направления и ставит врагов по одному.
fn direct_waves(
    time: Res<Time>,
    threat: Res<Threat>,
    mut director: ResMut<Director>,
    mut dice: ResMut<GameRng>,
    rover: Query<&Health, With<Rover>>,
    // Капсула в полёте — враг, который вот-вот появится: тоже в пределе одновременных.
    // Перехватчики не входят в предел: у них свой — размер звена.
    enemies: Query<(), (Or<(With<Enemy>, With<Capsule>)>, Without<Interceptor>)>,
    mut started: MessageWriter<WaveStarted>,
    mut spawn: MessageWriter<SpawnEnemy>,
    mut capsules: MessageWriter<DropCapsule>,
    mut bosses: MessageWriter<SpawnBoss>,
    rover_place: Query<&Transform, With<Rover>>,
    houses: Query<(Entity, &House, &Transform), Without<Rover>>,
    settlements: Res<Settlements>,
) {
    if !rover.iter().any(|health| !health.is_dead()) {
        return;
    }
    let dt = time.delta_secs();
    let director = &mut *director;
    match &mut director.phase {
        Phase::Lull(remaining) => {
            *remaining -= dt;
            if *remaining > 0.0 {
                return;
            }
            let wave = buy_wave(threat.value(), || dice.range(0.0, 1.0));
            // Волна приходит с одной-трёх сторон.
            let directions: Vec<f32> = (0..1 + dice.below(3)).map(|_| dice.range(0.0, TAU)).collect();
            let queue = wave
                .iter()
                .map(|kind| {
                    let direction = directions[dice.below(directions.len())];
                    let spread = dice.range(-balance::DIRECTION_SPREAD, balance::DIRECTION_SPREAD);
                    (direction + spread, *kind)
                })
                .collect();
            director.wave += 1;
            director.phase = Phase::Spawning { queue, timer: 0.0 };
            started.write(WaveStarted { number: director.wave });
        }
        Phase::Spawning { queue, timer } => {
            *timer -= dt;
            if *timer <= 0.0 && enemies.iter().count() < balance::MAX_ALIVE {
                if let Some((angle, kind)) = queue.pop() {
                    let house = rover_place.iter().next().and_then(|rover| {
                        (dice.range(0.0, 1.0) < balance::HOUSE_SHARE)
                            .then(|| house_in_sector(rover.translation, angle, &houses, &settlements))
                            .flatten()
                    });
                    if let Some(house) = house {
                        spawn.write(SpawnEnemy {
                            place: SpawnPlace::House(house),
                            kind,
                        });
                    } else if balance::uses_capsule(threat.value(), dice.range(0.0, 1.0)) {
                        capsules.write(DropCapsule { angle, kind });
                    } else {
                        spawn.write(SpawnEnemy {
                            place: SpawnPlace::Edge { angle },
                            kind,
                        });
                    }
                    *timer = balance::SPAWN_GAP;
                }
            }
            if queue.is_empty() {
                // Каждая четвёртая волна завершается боссом.
                if director.wave % balance::BOSS_EVERY == 0 {
                    bosses.write(SpawnBoss);
                }
                director.phase = Phase::Lull(balance::lull_seconds(threat.value()));
            }
        }
    }
}

/// Ближайший целый дом в секторе волны вокруг направления `angle` от лунохода.
fn house_in_sector(
    rover: Vec3,
    angle: f32,
    houses: &Query<(Entity, &House, &Transform), Without<Rover>>,
    settlements: &Settlements,
) -> Option<Entity> {
    houses
        .iter()
        .filter(|(_, house, _)| {
            settlements
                .house(house.0)
                .is_some_and(|s| s.status == HouseStatus::Intact && s.health > 0.0)
        })
        .filter_map(|(entity, _, transform)| {
            let d = (transform.translation - rover).with_y(0.0);
            let distance = d.length();
            let diff = (d.z.atan2(d.x) - angle + std::f32::consts::PI).rem_euclid(TAU) - std::f32::consts::PI;
            (distance < balance::HOUSE_RANGE && distance > 8.0 && diff.abs() < balance::HOUSE_SECTOR)
                .then_some((entity, distance))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(entity, _)| entity)
}

/// Показывает номер начавшейся волны крупным текстом.
fn announce_wave(mut commands: Commands, mut started: MessageReader<WaveStarted>) {
    for wave in started.read() {
        commands
            .spawn((
                WaveBanner {
                    remaining: BANNER_SECONDS,
                },
                RunScoped,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    top: Val::Percent(22.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
            ))
            .with_children(|banner| {
                banner.spawn((
                    Text::new(format!("WAVE {}", wave.number)),
                    TextFont {
                        font_size: FontSize::Px(72.0),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ));
            });
    }
}

/// Объявление волны гаснет и удаляется.
fn fade_banners(
    mut commands: Commands,
    time: Res<Time>,
    mut banners: Query<(Entity, &mut WaveBanner, &Children)>,
    mut colors: Query<&mut TextColor>,
) {
    for (entity, mut banner, children) in &mut banners {
        banner.remaining -= time.delta_secs();
        if banner.remaining <= 0.0 {
            commands.entity(entity).try_despawn();
            continue;
        }
        let alpha = (banner.remaining / BANNER_SECONDS).clamp(0.0, 1.0);
        for child in children.iter() {
            if let Ok(mut color) = colors.get_mut(child) {
                color.0 = Color::srgba(1.0, 1.0, 1.0, alpha);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threat_never_goes_down() {
        let mut threat = Threat::default();
        threat.add_time(30.0);
        threat.add_destruction(2.0);
        let before = threat.value();
        threat.add_time(-10.0);
        threat.add_destruction(-5.0);
        assert_eq!(threat.value(), before);
        threat.add_time(1.0);
        assert!(threat.value() > before);
    }

    #[test]
    fn destroyed_house_raises_threat_more_than_a_second() {
        let mut by_time = Threat::default();
        by_time.add_time(1.0);
        let mut by_house = Threat::default();
        by_house.add_destruction(balance::HOUSE_WEIGHT);
        assert!(by_house.value() > by_time.value());
    }

    #[test]
    fn capsules_come_only_after_the_threat_grows() {
        assert!(!balance::uses_capsule(0.5, 0.0));
        assert!(balance::uses_capsule(1.0, 0.1));
        assert!(!balance::uses_capsule(3.0, 0.9));
    }

    #[test]
    fn mortarmen_join_after_the_threat_grows() {
        assert!(!buy_wave(0.5, || 0.999).contains(&EnemyKind::Mortar));
        let later = buy_wave(1.0, || 0.999);
        assert!(later.contains(&EnemyKind::Mortar));
        // Бюджет 7 очков: мортирщик стоит два, таран-стрелок одно.
        let cost: u32 = later.iter().map(|kind| if *kind == EnemyKind::Mortar { 2 } else { 1 }).sum();
        assert_eq!(cost, 7);
    }

    #[test]
    fn first_wave_is_only_ram_shooters() {
        let wave = buy_wave(0.0, || 0.999);
        assert_eq!(wave, vec![EnemyKind::RamShooter; 4]);
    }

    #[test]
    fn waves_grow_and_lulls_shrink_with_threat() {
        assert!(buy_wave(2.0, || 0.0).len() > buy_wave(0.0, || 0.0).len());
        assert_eq!(balance::lull_seconds(0.0), 14.0);
        assert!(balance::lull_seconds(2.0) < balance::lull_seconds(0.0));
        assert_eq!(balance::lull_seconds(100.0), 6.0);
    }

    #[test]
    fn expensive_kinds_grow_heavier_after_all_kinds_join() {
        // До танков все типы равновероятны.
        assert_eq!(balance::kind_weight(5, 2.0), 1.0);
        assert_eq!(balance::kind_weight(1, 6.0), 1.0);
        assert!(balance::kind_weight(5, 4.0) > balance::kind_weight(3, 4.0));
        assert!(balance::kind_weight(5, 6.0) > balance::kind_weight(5, 4.0));
        // Средний бросок на пятой минуте покупает больше танков, чем на третьей.
        let mut n = 0;
        let mut even = || {
            n += 1;
            (n as f32 * 0.618).fract()
        };
        let tanks = |wave: Vec<EnemyKind>| wave.iter().filter(|k| **k == EnemyKind::LaserTank).count() as f32 / wave.len() as f32;
        let third = tanks(buy_wave(3.0, &mut even));
        let fifth = tanks(buy_wave(5.0, &mut even));
        assert!(fifth > third, "{third} {fifth}");
    }

    #[test]
    fn laser_trench_counts_only_when_the_beam_cuts_a_hill() {
        let ground = |x: f32, _z: f32| if x > 10.0 { 5.0 } else { 0.0 };
        let origin = Vec3::new(0.0, 1.5, 0.0);
        assert!(beam_cuts(ground, origin, Vec3::X));
        assert!(!beam_cuts(ground, origin, Vec3::NEG_X));
        // Траншея уже прорезана на 0,4 м ниже луча: тот же выстрел всё ещё считается.
        let cut = |x: f32, _z: f32| if x > 10.0 { 1.1 } else { 0.0 };
        assert!(beam_cuts(cut, origin, Vec3::X));
    }

    #[test]
    fn волна_не_содержит_перехватчиков() {
        let mut n = 0u32;
        let mut even = || {
            n += 1;
            (n as f32 * 0.618).fract()
        };
        for threat in [0.0, 1.0, 2.0, 3.0, 5.0, 8.0] {
            assert!(!buy_wave(threat, &mut even).contains(&EnemyKind::Interceptor), "{threat}");
        }
    }

    #[test]
    fn pack_size_starts_at_two_and_stops_at_four() {
        assert_eq!(balance::pack_size(0.0), 2);
        assert_eq!(balance::pack_size(2.0), 3);
        assert_eq!(balance::pack_size(4.5), 4);
        assert_eq!(balance::pack_size(20.0), 4);
    }
}
