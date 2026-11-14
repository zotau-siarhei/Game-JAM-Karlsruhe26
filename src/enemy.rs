//! Враги. Первый тип — таран-стрелок: появляется за кадром, едет прямо на луноход,
//! стреляет очередями при прямой видимости и таранит вплотную.
//!
//! Второй тип — мортирщик: держит дистанцию и накрывает луноход навесным огнём.
//!
//! Когда и сколько врагов появляется, решает директор волн в `src/waves.rs`: он просит
//! поставить врага сообщением [`SpawnEnemy`].

use bevy::prelude::*;

use crate::bullet::{self, FireBullet};
use crate::combat::{Collider, Damage, Died, Health, Team};
use crate::explosion::Explosion;
use crate::mortar::{self, FireShell};
use crate::rover::{self, Rover, RoverMotion};
use crate::run::{GameState, RunScoped};
use crate::sound_list::Dice;
use crate::terrain::Terrain;
use crate::vehicle::{self, GroundVehicle};

/// Здоровье таран-стрелка.
const ENEMY_HEALTH: f32 = 30.0;

/// Скорость таран-стрелка, м/с. Луноход быстрее: от врага можно уехать.
pub const ENEMY_SPEED: f32 = 9.0;

/// Скорость поворота врага к цели, рад/с.
const ENEMY_TURN_SPEED: f32 = 2.2;

/// Радиус коллайдера врага, м.
const ENEMY_RADIUS: f32 = 1.2;

/// Размер корпуса врага: ширина, высота, длина, м.
const HULL_SIZE: Vec3 = Vec3::new(1.4, 0.7, 2.0);

/// Радиус колеса врага, м.
const WHEEL_RADIUS: f32 = 0.32;

/// Высота центра корпуса врага над поверхностью, м.
const RIDE_HEIGHT: f32 = HULL_SIZE.y * 0.5 + WHEEL_RADIUS * 0.6;

/// Кольцо появления вокруг лунохода, м. Кадр видит около 30 м от центра, враг появляется вне его.
const SPAWN_RING: (f32, f32) = (45.0, 55.0);

/// Враги дальше этого расстояния от лунохода убираются, м.
const DESPAWN_DISTANCE: f32 = 120.0;

/// Урон тарана.
const RAM_DAMAGE: f32 = 12.0;

/// Один враг таранит не чаще раза в столько секунд.
const RAM_INTERVAL: f32 = 0.5;

/// Запас к сумме радиусов, при котором машины считаются столкнувшимися, м.
const RAM_MARGIN: f32 = 0.2;

/// Дистанция, с которой враг стреляет, м.
const FIRE_RANGE: f32 = 28.0;

/// Пуль в очереди.
const BURST_SIZE: u8 = 3;

/// Пауза между пулями очереди, с.
const BURST_GAP: f32 = 0.12;

/// Пауза между очередями, с.
const RELOAD: f32 = 1.5;

/// Разброс пуль по горизонтали в каждую сторону, рад.
const SPREAD: f32 = 4.0 * std::f32::consts::PI / 180.0;

/// Урон пули врага.
const BULLET_DAMAGE: f32 = 4.0;

/// Высота дула над центром корпуса, м.
const MUZZLE_HEIGHT: f32 = 0.5;

/// Взрыв при гибели врага: радиус, м, и мощность. Мощность ниже порога кратера.
const DEATH_BLAST: (f32, f32) = (2.5, 0.25);

/// Цвет корпуса врага.
const HULL_COLOR: Color = Color::srgb(0.72, 0.16, 0.12);

/// Цвет колёс и тарана врага.
const DARK_COLOR: Color = Color::srgb(0.1, 0.1, 0.12);

/// Здоровье мортирщика.
const MORTAR_HEALTH: f32 = 40.0;

/// Скорость мортирщика, м/с.
const MORTAR_SPEED: f32 = 7.0;

/// Ближе этого расстояния до лунохода мортирщик отступает, дальше второго — подъезжает, м.
const MORTAR_BAND: (f32, f32) = (22.0, 34.0);

/// Перезарядка мортиры врага, с.
const MORTAR_RELOAD: f32 = 3.0;

/// Мощность взрыва вражеского снаряда: прямое попадание отнимает около трети здоровья.
const MORTAR_POWER: f32 = 0.35;

/// Разброс точки прицеливания мортирщика, м.
const MORTAR_SCATTER: f32 = 3.0;

/// Цвет корпуса мортирщика.
const MORTAR_COLOR: Color = Color::srgb(0.5, 0.2, 0.62);

/// Подключает врагов.
pub struct EnemyPlugin;

impl Plugin for EnemyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Dice>()
            .add_message::<SpawnEnemy>()
            .add_systems(Startup, create_enemy_assets)
            .add_systems(
                Update,
                (
                    spawn_enemies.run_if(in_state(GameState::Playing)),
                    chase_rover,
                    drive_mortarmen,
                    vehicle::move_ground_vehicles,
                    separate_enemies,
                    ram_rover,
                    shoot_rover,
                    fire_mortarmen,
                    remove_far_enemies,
                    remove_dead_enemies,
                )
                    .chain()
                    .after(rover::drive_rover),
            );
    }
}

/// Враг.
#[derive(Component)]
pub struct Enemy;

/// Тип врага.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnemyKind {
    /// Едет на луноход, стреляет очередями и таранит.
    RamShooter,
    /// Держит дистанцию и стреляет из мортиры.
    Mortar,
}

/// Мортирщик: время до следующего выстрела, с.
#[derive(Component)]
struct Mortarman {
    reload: f32,
}

/// Что мортирщик делает на расстоянии `distance` от лунохода.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MortarMove {
    Approach,
    Hold,
    Retreat,
}

fn mortar_move(distance: f32) -> MortarMove {
    if distance < MORTAR_BAND.0 {
        MortarMove::Retreat
    } else if distance > MORTAR_BAND.1 {
        MortarMove::Approach
    } else {
        MortarMove::Hold
    }
}

/// Оружие врага: таран и пулемёт.
#[derive(Component, Debug)]
struct EnemyWeapons {
    /// Сколько секунд до следующего тарана.
    ram_cooldown: f32,
    /// Сколько секунд до следующей очереди.
    reload: f32,
    /// Сколько пуль осталось в текущей очереди.
    burst_left: u8,
    /// Сколько секунд до следующей пули очереди.
    burst_timer: f32,
}

/// Просьба поставить врага.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpawnEnemy {
    pub place: SpawnPlace,
    pub kind: EnemyKind,
}

/// Где появляется враг.
#[derive(Debug, Clone, Copy)]
pub enum SpawnPlace {
    /// За кадром, в направлении `angle` от лунохода, рад.
    Edge { angle: f32 },
    /// В заданной точке: высота берётся с рельефа.
    Point(Vec3),
}

/// Общие меши и материалы врагов: одинаковые враги рисуются одним пакетом.
#[derive(Resource)]
struct EnemyAssets {
    hull: Handle<Mesh>,
    wheel: Handle<Mesh>,
    ram: Handle<Mesh>,
    barrel: Handle<Mesh>,
    hull_material: Handle<StandardMaterial>,
    mortar_material: Handle<StandardMaterial>,
    dark_material: Handle<StandardMaterial>,
}

fn create_enemy_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(EnemyAssets {
        hull: meshes.add(Cuboid::from_size(HULL_SIZE)),
        wheel: meshes.add(Cylinder::new(WHEEL_RADIUS, 0.28)),
        ram: meshes.add(Cuboid::new(HULL_SIZE.x + 0.5, 0.35, 0.35)),
        barrel: meshes.add(Cylinder::new(0.28, 1.5)),
        hull_material: materials.add(HULL_COLOR),
        mortar_material: materials.add(MORTAR_COLOR),
        dark_material: materials.add(DARK_COLOR),
    });
}

/// Точка на окружности радиуса `distance` вокруг `center` по углу `angle`.
pub fn ring_point(center: Vec3, angle: f32, distance: f32) -> Vec3 {
    center + Vec3::new(angle.cos(), 0.0, angle.sin()) * distance
}

/// Ставит запрошенных врагов на кольце вокруг лунохода, вне кадра.
fn spawn_enemies(
    mut commands: Commands,
    mut requests: MessageReader<SpawnEnemy>,
    mut dice: ResMut<Dice>,
    assets: Res<EnemyAssets>,
    terrain: Res<Terrain>,
    rover: Query<(&Transform, &Health), With<Rover>>,
) {
    let Some((rover, health)) = rover.iter().next() else {
        return;
    };
    if health.is_dead() {
        return;
    }
    for request in requests.read() {
        let position = match request.place {
            SpawnPlace::Edge { angle } => {
                let distance = dice.range(SPAWN_RING.0, SPAWN_RING.1);
                ring_point(rover.translation, angle, distance)
            }
            SpawnPlace::Point(point) => point,
        };
        spawn_enemy(&mut commands, &mut dice, &assets, &terrain, position, rover.translation, request.kind);
    }
}

/// Создаёт врага типа `kind` в точке `position` носом к точке `target`.
fn spawn_enemy(
    commands: &mut Commands,
    dice: &mut Dice,
    assets: &EnemyAssets,
    terrain: &Terrain,
    mut position: Vec3,
    target: Vec3,
    kind: EnemyKind,
) {
    position.y = terrain.height_at(position.x, position.z) + RIDE_HEIGHT;
    let heading = vehicle::heading_towards(target - position);

    let (health, material) = match kind {
        EnemyKind::RamShooter => (ENEMY_HEALTH, assets.hull_material.clone()),
        EnemyKind::Mortar => (MORTAR_HEALTH, assets.mortar_material.clone()),
    };
    let mut enemy = commands.spawn((
        Enemy,
        RunScoped,
        Health::new(health),
        Team::Enemy,
        Collider {
            radius: ENEMY_RADIUS,
        },
        GroundVehicle::new(heading, RIDE_HEIGHT, Vec2::new(HULL_SIZE.x, HULL_SIZE.z) * 0.5),
        Mesh3d(assets.hull.clone()),
        MeshMaterial3d(material),
        Transform::from_translation(position).with_rotation(Quat::from_rotation_y(heading)),
    ));
    match kind {
        EnemyKind::RamShooter => enemy.insert(EnemyWeapons {
            ram_cooldown: 0.0,
            // Разная задержка, чтобы враги не стреляли залпом.
            reload: dice.range(0.3, RELOAD),
            burst_left: 0,
            burst_timer: 0.0,
        }),
        EnemyKind::Mortar => enemy.insert(Mortarman {
            reload: dice.range(1.0, MORTAR_RELOAD),
        }),
    };
    enemy.with_children(|parent| {
            for (side, along) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                parent.spawn((
                    Mesh3d(assets.wheel.clone()),
                    MeshMaterial3d(assets.dark_material.clone()),
                    Transform::from_xyz(
                        side * (HULL_SIZE.x * 0.5 + 0.14),
                        -HULL_SIZE.y * 0.5,
                        along * HULL_SIZE.z * 0.33,
                    )
                    .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
                ));
            }
            match kind {
                // Таран на носу: видно, куда враг едет.
                EnemyKind::RamShooter => parent.spawn((
                    Mesh3d(assets.ram.clone()),
                    MeshMaterial3d(assets.dark_material.clone()),
                    Transform::from_xyz(0.0, -0.1, -HULL_SIZE.z * 0.5 - 0.15),
                )),
                // Ствол мортиры смотрит вверх и вперёд.
                EnemyKind::Mortar => parent.spawn((
                    Mesh3d(assets.barrel.clone()),
                    MeshMaterial3d(assets.dark_material.clone()),
                    Transform::from_xyz(0.0, HULL_SIZE.y * 0.5 + 0.45, -0.2)
                        .with_rotation(Quat::from_rotation_x(-0.5)),
                )),
            };
        });
}

/// Поворачивает врагов к луноходу и ведёт их вперёд. Когда луноход погиб, враги встают.
fn chase_rover(
    time: Res<Time>,
    rover: Query<(&Transform, &Health), With<Rover>>,
    mut enemies: Query<(&Transform, &mut GroundVehicle), (With<Enemy>, Without<Mortarman>)>,
) {
    let target = rover
        .iter()
        .next()
        .filter(|(_, health)| !health.is_dead())
        .map(|(transform, _)| transform.translation);
    let max_turn = ENEMY_TURN_SPEED * time.delta_secs();
    for (transform, mut vehicle) in &mut enemies {
        let Some(target) = target else {
            vehicle.speed = 0.0;
            continue;
        };
        let to_target = target - transform.translation;
        vehicle.heading = vehicle::steer(vehicle.heading, vehicle::heading_towards(to_target), max_turn);
        vehicle.speed = ENEMY_SPEED;
    }
}

/// Мортирщик подъезжает на дальность выстрела, встаёт и отступает, если луноход слишком близко.
fn drive_mortarmen(
    time: Res<Time>,
    rover: Query<(&Transform, &Health), With<Rover>>,
    mut enemies: Query<(&Transform, &mut GroundVehicle), With<Mortarman>>,
) {
    let target = rover
        .iter()
        .next()
        .filter(|(_, health)| !health.is_dead())
        .map(|(transform, _)| transform.translation);
    let max_turn = ENEMY_TURN_SPEED * time.delta_secs();
    for (transform, mut vehicle) in &mut enemies {
        let Some(target) = target else {
            vehicle.speed = 0.0;
            continue;
        };
        let to_target = target - transform.translation;
        let (direction, speed) = match mortar_move(to_target.length()) {
            MortarMove::Approach => (to_target, MORTAR_SPEED),
            MortarMove::Hold => (to_target, 0.0),
            MortarMove::Retreat => (-to_target, MORTAR_SPEED),
        };
        vehicle.heading = vehicle::steer(vehicle.heading, vehicle::heading_towards(direction), max_turn);
        vehicle.speed = speed;
    }
}

/// Точка, куда мортирщик целится: туда, где луноход окажется к падению снаряда.
fn lead_target(rover: Vec3, velocity: Vec3) -> Vec3 {
    rover + velocity * mortar::FLIGHT_TIME
}

/// Стоящий мортирщик раз в несколько секунд стреляет с упреждением и разбросом.
fn fire_mortarmen(
    time: Res<Time>,
    mut dice: ResMut<Dice>,
    rover: Query<(&Transform, &RoverMotion, &Health), (With<Rover>, Without<Enemy>)>,
    mut enemies: Query<(&Transform, &GroundVehicle, &mut Mortarman)>,
    mut shots: MessageWriter<FireShell>,
) {
    let Some((rover, motion, health)) = rover.iter().next() else {
        return;
    };
    if health.is_dead() {
        return;
    }
    let velocity = vehicle::forward_of(motion.heading) * motion.speed;
    for (transform, vehicle, mut mortarman) in &mut enemies {
        mortarman.reload -= time.delta_secs();
        if mortarman.reload > 0.0 || vehicle.speed != 0.0 {
            continue;
        }
        mortarman.reload = MORTAR_RELOAD;
        let scatter = Vec3::new(
            dice.range(-MORTAR_SCATTER, MORTAR_SCATTER),
            0.0,
            dice.range(-MORTAR_SCATTER, MORTAR_SCATTER),
        );
        shots.write(FireShell {
            origin: transform.translation + Vec3::Y * 1.4,
            target: lead_target(rover.translation, velocity) + scatter,
            team: Team::Enemy,
            power: MORTAR_POWER,
        });
    }
}

/// На сколько и куда сдвинуть первую машину, чтобы она не пересекалась со второй.
/// `None`, если машины не пересекаются.
fn overlap_push(first: Vec3, second: Vec3, radius_sum: f32) -> Option<Vec3> {
    let mut away = first - second;
    away.y = 0.0;
    let distance = away.length();
    if distance >= radius_sum {
        return None;
    }
    // Машины точно в одной точке: расходятся вдоль X.
    let direction = if distance > 1e-4 { away / distance } else { Vec3::X };
    Some(direction * (radius_sum - distance))
}

/// Расталкивает врагов между собой и не даёт им въехать в луноход.
fn separate_enemies(
    rover: Query<(&Transform, &Collider), (With<Rover>, Without<Enemy>)>,
    mut enemies: Query<(&mut Transform, &Collider), With<Enemy>>,
) {
    let mut pairs = enemies.iter_combinations_mut();
    while let Some([(mut first, first_collider), (mut second, second_collider)]) = pairs.fetch_next() {
        let radius_sum = first_collider.radius + second_collider.radius;
        if let Some(push) = overlap_push(first.translation, second.translation, radius_sum) {
            first.translation += push * 0.5;
            second.translation -= push * 0.5;
        }
    }
    let Some((rover, rover_collider)) = rover.iter().next() else {
        return;
    };
    for (mut enemy, collider) in &mut enemies {
        let radius_sum = collider.radius + rover_collider.radius;
        if let Some(push) = overlap_push(enemy.translation, rover.translation, radius_sum) {
            enemy.translation += push;
        }
    }
}

/// Столкнулись ли машины: расстояние по горизонтали не больше суммы радиусов с запасом.
fn is_ramming(enemy: Vec3, rover: Vec3, radius_sum: f32) -> bool {
    Vec2::new(enemy.x - rover.x, enemy.z - rover.z).length() <= radius_sum + RAM_MARGIN
}

/// Таран: враг, упёршийся в луноход, наносит урон не чаще раза в [`RAM_INTERVAL`].
fn ram_rover(
    time: Res<Time>,
    rover: Query<(Entity, &Transform, &Collider, &Health), (With<Rover>, Without<Enemy>)>,
    mut enemies: Query<(&Transform, &Collider, &mut EnemyWeapons), With<Enemy>>,
    mut damage: MessageWriter<Damage>,
) {
    let rover = rover.iter().next().filter(|(_, _, _, health)| !health.is_dead());
    for (transform, collider, mut weapons) in &mut enemies {
        weapons.ram_cooldown = (weapons.ram_cooldown - time.delta_secs()).max(0.0);
        let Some((target, rover_transform, rover_collider, _)) = rover else {
            continue;
        };
        let radius_sum = collider.radius + rover_collider.radius;
        if weapons.ram_cooldown <= 0.0
            && is_ramming(transform.translation, rover_transform.translation, radius_sum)
        {
            weapons.ram_cooldown = RAM_INTERVAL;
            damage.write(Damage {
                target,
                amount: RAM_DAMAGE,
                source_team: Team::Enemy,
            });
        }
    }
}

/// Что делает пулемёт врага в этом кадре.
#[derive(Debug, PartialEq)]
enum GunAction {
    /// Ждёт.
    Wait,
    /// Выпускает пулю.
    Fire,
}

impl EnemyWeapons {
    /// Ведёт перезарядку и очередь. `can_start` — цель в дальности и видна.
    fn tick_gun(&mut self, dt: f32, can_start: bool) -> GunAction {
        self.reload = (self.reload - dt).max(0.0);
        if self.burst_left == 0 {
            if !can_start || self.reload > 0.0 {
                return GunAction::Wait;
            }
            self.burst_left = BURST_SIZE;
            self.burst_timer = 0.0;
            self.reload = RELOAD;
        }
        self.burst_timer -= dt;
        if self.burst_timer > 0.0 {
            return GunAction::Wait;
        }
        self.burst_left -= 1;
        self.burst_timer = BURST_GAP;
        GunAction::Fire
    }
}

/// Стрельба: на средней дистанции и только когда между врагом и луноходом нет рельефа.
fn shoot_rover(
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut dice: ResMut<Dice>,
    rover: Query<(&Transform, &Health), (With<Rover>, Without<Enemy>)>,
    mut enemies: Query<(&Transform, &mut EnemyWeapons), With<Enemy>>,
    mut bullets: MessageWriter<FireBullet>,
) {
    let Some(target) = rover
        .iter()
        .next()
        .filter(|(_, health)| !health.is_dead())
        .map(|(transform, _)| transform.translation)
    else {
        return;
    };
    let dt = time.delta_secs();
    for (transform, mut weapons) in &mut enemies {
        let muzzle = transform.translation + Vec3::Y * MUZZLE_HEIGHT;
        let can_start = muzzle.distance(target) <= FIRE_RANGE
            && bullet::has_line_of_sight(&terrain, muzzle, target);
        if weapons.tick_gun(dt, can_start) == GunAction::Fire {
            let spread = Quat::from_rotation_y(dice.range(-SPREAD, SPREAD));
            bullets.write(FireBullet {
                origin: muzzle,
                direction: spread * (target - muzzle),
                team: Team::Enemy,
                damage: BULLET_DAMAGE,
            });
        }
    }
}

/// Убирает врагов, от которых луноход уехал далеко.
fn remove_far_enemies(
    mut commands: Commands,
    rover: Query<&Transform, With<Rover>>,
    enemies: Query<(Entity, &Transform), With<Enemy>>,
) {
    let Some(rover) = rover.iter().next() else {
        return;
    };
    for (entity, transform) in &enemies {
        if transform.translation.distance(rover.translation) > DESPAWN_DISTANCE {
            commands.entity(entity).despawn();
        }
    }
}

/// Погибший враг взрывается и исчезает.
fn remove_dead_enemies(
    mut commands: Commands,
    mut died: MessageReader<Died>,
    enemies: Query<&Transform, With<Enemy>>,
    mut explosions: MessageWriter<Explosion>,
) {
    for message in died.read() {
        if let Ok(transform) = enemies.get(message.entity) {
            explosions.write(Explosion {
                position: transform.translation,
                radius: DEATH_BLAST.0,
                power: DEATH_BLAST.1,
                source_team: Team::Enemy,
            });
            commands.entity(message.entity).try_despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use super::*;

    #[test]
    fn enemy_is_slower_than_rover() {
        // Скорость лунохода вперёд — 14 м/с.
        assert!(ENEMY_SPEED < 14.0);
    }

    #[test]
    fn spawn_point_is_outside_the_frame() {
        let center = Vec3::new(100.0, 3.0, -40.0);
        for step in 0..16 {
            let angle = step as f32 / 16.0 * TAU;
            for distance in [SPAWN_RING.0, SPAWN_RING.1] {
                let point = ring_point(center, angle, distance);
                let flat = Vec2::new(point.x - center.x, point.z - center.z).length();
                assert!((flat - distance).abs() < 1e-3);
                // Кадр видит около 30 м от лунохода.
                assert!(flat > 30.0);
            }
        }
    }

    #[test]
    fn ram_needs_contact() {
        let rover = Vec3::new(0.0, 4.0, 0.0);
        assert!(is_ramming(Vec3::new(2.4, 0.0, 0.0), rover, 2.4));
        assert!(is_ramming(Vec3::new(0.0, 9.0, 2.5), rover, 2.4));
        assert!(!is_ramming(Vec3::new(3.0, 4.0, 0.0), rover, 2.4));
    }

    fn weapons(reload: f32) -> EnemyWeapons {
        EnemyWeapons {
            ram_cooldown: 0.0,
            reload,
            burst_left: 0,
            burst_timer: 0.0,
        }
    }

    #[test]
    fn gun_fires_bursts_with_reload_between_them() {
        let mut gun = weapons(0.0);
        let dt = 0.02;
        let mut shots = Vec::new();
        for frame in 0..200 {
            if gun.tick_gun(dt, true) == GunAction::Fire {
                shots.push(frame as f32 * dt);
            }
        }
        // За 4 секунды: очереди в 0, 1,5 и 3 с по три пули.
        assert_eq!(shots.len(), 9);
        assert!(shots[2] - shots[0] < 0.4);
        assert!(shots[3] - shots[0] >= RELOAD - 0.05);
    }

    #[test]
    fn gun_does_not_fire_without_sight_but_finishes_a_burst() {
        let mut gun = weapons(0.0);
        for _ in 0..100 {
            assert_eq!(gun.tick_gun(0.02, false), GunAction::Wait);
        }
        // Очередь началась, цель скрылась: оставшиеся пули всё равно вылетают.
        assert_eq!(gun.tick_gun(0.02, true), GunAction::Fire);
        let rest = (0..50).filter(|_| gun.tick_gun(0.02, false) == GunAction::Fire).count();
        assert_eq!(rest, usize::from(BURST_SIZE) - 1);
    }

    #[test]
    fn overlapping_vehicles_are_pushed_apart() {
        let push = overlap_push(Vec3::new(1.0, 5.0, 0.0), Vec3::ZERO, 2.4).unwrap();
        assert!((push - Vec3::new(1.4, 0.0, 0.0)).length() < 1e-4);
        assert!(overlap_push(Vec3::new(3.0, 0.0, 0.0), Vec3::ZERO, 2.4).is_none());
        // Машины в одной точке всё равно расходятся.
        let push = overlap_push(Vec3::ZERO, Vec3::ZERO, 2.4).unwrap();
        assert!((push.length() - 2.4).abs() < 1e-4);
    }

    #[test]
    fn mortarman_keeps_its_distance() {
        assert_eq!(mortar_move(50.0), MortarMove::Approach);
        assert_eq!(mortar_move(28.0), MortarMove::Hold);
        assert_eq!(mortar_move(10.0), MortarMove::Retreat);
    }

    #[test]
    fn mortarman_leads_a_moving_rover() {
        let rover = Vec3::new(5.0, 0.0, 5.0);
        assert_eq!(lead_target(rover, Vec3::ZERO), rover);
        let ahead = lead_target(rover, Vec3::new(10.0, 0.0, 0.0));
        assert!((ahead.x - (5.0 + 10.0 * mortar::FLIGHT_TIME)).abs() < 1e-4);
        assert_eq!(ahead.z, 5.0);
    }
}
