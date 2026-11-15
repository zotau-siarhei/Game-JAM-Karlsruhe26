//! Волны врагов: угроза растёт со временем и разрушениями, директор по ней собирает волны.
//!
//! Все числа, от которых зависит длина забега, лежат в [`balance`]: настраивать их там.

use std::f32::consts::TAU;

use bevy::prelude::*;

use crate::combat::{Health, Team};
use crate::boss::SpawnBoss;
use crate::buildings::House;
use crate::capsule::DropCapsule;
use crate::settlements::{HouseStatus, Settlements};
use crate::enemy::{Enemy, EnemyKind, SpawnEnemy, SpawnPlace};
use crate::explosion::Explosion;
use crate::frame::GameSet;
use crate::rng::GameRng;
use crate::rover::Rover;
use crate::buildings::{BuildingDestroyed, DestroyCause};
use crate::run::{RunScoped, WorldRebuilt};

/// Кривые баланса: всё, что зависит от угрозы.
pub mod balance {
    /// Вес кратера в разрушениях.
    pub const CRATER_WEIGHT: f32 = 1.0;

    /// Вес дома, снесённого игроком, в разрушениях.
    pub const HOUSE_WEIGHT: f32 = 4.0;

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

    /// С какой угрозы появляются ракетчики.
    pub const ROCKETEER_THREAT: f32 = 1.6;

    /// С какой угрозы появляются тяжёлые танки.
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
}

/// Подключает угрозу, директора волн и объявление волны.
pub struct WavesPlugin;

impl Plugin for WavesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Threat>()
            .init_resource::<Director>()
            .add_message::<WaveStarted>()
            .add_systems(
                Update,
                (
                    reset_waves.in_set(GameSet::Reset),
                    (grow_threat, direct_waves).chain().in_set(GameSet::Simulation),
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
    if threat >= balance::ROCKETEER_THREAT {
        kinds.push((EnemyKind::Rocketeer, 3));
    }
    if threat >= balance::TANK_THREAT {
        kinds.push((EnemyKind::HeavyTank, 5));
    }
    kinds
}

/// Состав волны: случайные покупки из доступных типов, пока хватает бюджета.
/// `roll` выдаёт случайный номер меньше переданного числа.
fn buy_wave(threat: f32, mut roll: impl FnMut(usize) -> usize) -> Vec<EnemyKind> {
    let kinds = available_kinds(threat);
    let mut budget = balance::wave_budget(threat);
    let mut wave = Vec::new();
    loop {
        let affordable: Vec<_> = kinds.iter().filter(|(_, cost)| *cost <= budget).collect();
        if affordable.is_empty() {
            return wave;
        }
        let (kind, cost) = *affordable[roll(affordable.len()) % affordable.len()];
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

/// Угроза растёт со временем, с каждым кратером от оружия игрока и с каждым снесённым им домом.
fn grow_threat(
    time: Res<Time>,
    mut explosions: MessageReader<Explosion>,
    mut houses: MessageReader<BuildingDestroyed>,
    mut threat: ResMut<Threat>,
) {
    threat.add_time(time.delta_secs());
    for explosion in explosions.read() {
        if explosion.source_team == Team::Player {
            threat.add_destruction(balance::CRATER_WEIGHT);
        }
    }
    for house in houses.read() {
        if house.cause == DestroyCause::Player {
            threat.add_destruction(balance::HOUSE_WEIGHT);
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
    enemies: Query<(), With<Enemy>>,
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
            let wave = buy_wave(threat.value(), |limit| dice.below(limit));
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
        assert!(!buy_wave(0.5, |limit| limit - 1).contains(&EnemyKind::Mortar));
        let later = buy_wave(1.0, |limit| limit - 1);
        assert!(later.contains(&EnemyKind::Mortar));
        // Бюджет 7 очков: мортирщик стоит два, таран-стрелок одно.
        let cost: u32 = later.iter().map(|kind| if *kind == EnemyKind::Mortar { 2 } else { 1 }).sum();
        assert_eq!(cost, 7);
    }

    #[test]
    fn first_wave_is_only_ram_shooters() {
        let wave = buy_wave(0.0, |limit| limit - 1);
        assert_eq!(wave, vec![EnemyKind::RamShooter; 4]);
    }

    #[test]
    fn waves_grow_and_lulls_shrink_with_threat() {
        assert!(buy_wave(2.0, |_| 0).len() > buy_wave(0.0, |_| 0).len());
        assert_eq!(balance::lull_seconds(0.0), 14.0);
        assert!(balance::lull_seconds(2.0) < balance::lull_seconds(0.0));
        assert_eq!(balance::lull_seconds(100.0), 6.0);
    }
}
