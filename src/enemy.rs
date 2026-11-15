//! Враги. Первый тип — таран-стрелок: появляется за кадром, едет прямо на луноход,
//! стреляет очередями при прямой видимости и таранит вплотную.
//!
//! Второй тип — мортирщик: держит дистанцию и накрывает луноход навесным огнём.
//!
//! Третий — робот-ракетчик: прячется в кратере, за домом или холмом, высовывается для выстрела
//! ракетой и прячется снова. Четвёртый — тяжёлый танк: заходит с фланга, секунду показывает
//! линию прицела и бьёт импульсом лазера, который режет рельеф.
//!
//! Когда и сколько врагов появляется, решает директор волн в `src/waves.rs`: он просит
//! поставить врага сообщением [`SpawnEnemy`].

use bevy::prelude::*;

use crate::boss::Boss;
use crate::buildings::{BreakDoor, House};
use crate::settlements::Settlements;
use crate::bullet::FireBullet;
use crate::combat::{Collider, Damage, Died, Health, Team};
use crate::explosion::Explosion;
use crate::frame::{GameSet, RunMessages};
use crate::laser::FireLaser;
use crate::mortar::{self, FireShell};
use crate::obstacles::Obstacle;
use crate::rng::GameRng;
use crate::rockets::FireRocket;
use crate::rover::{Rover, RoverMotion};
use crate::run::RunScoped;
use crate::sound_list::Dice;
use crate::steering::{self, Steering};
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

/// Здоровье ракетчика.
const ROCKETEER_HEALTH: f32 = 50.0;
/// Скорость ракетчика, м/с.
const ROCKETEER_SPEED: f32 = 7.5;
/// Укрытие ищется в этом кольце вокруг лунохода, м.
const COVER_RING: (f32, f32) = (10.0, 25.0);
/// Сколько точек-кандидатов перебирается при поиске укрытия.
const COVER_CANDIDATES: usize = 12;
/// Точка ниже окружающего рельефа на столько — дно кратера, м.
const CRATER_DIP: f32 = 1.5;
/// Сколько ракетчик сидит в укрытии, с: от и до.
const HIDE_TIME: (f32, f32) = (1.5, 3.0);
/// На сколько ракетчик высовывается вбок, м.
const PEEK_SHIFT: f32 = 3.0;
/// Как часто перепроверяется укрытие, с.
const COVER_RECHECK: f32 = 0.5;
/// Высота пускового контейнера над центром корпуса, м.
const LAUNCHER_HEIGHT: f32 = 0.9;
/// Цвет ракетчика.
const ROCKETEER_COLOR: Color = Color::srgb(0.85, 0.55, 0.12);

/// Здоровье тяжёлого танка.
const TANK_HEALTH: f32 = 220.0;
/// Скорость танка, м/с.
const TANK_SPEED: f32 = 6.0;
/// Поворот танка, рад/с: медленный, линия прицела заметно доворачивает.
const TANK_TURN_SPEED: f32 = 1.2;
/// Во сколько раз танк крупнее таран-стрелка.
const TANK_SCALE: f32 = 1.6;
/// Насколько в стороне от линии к луноходу танк заходит, м.
const FLANK_OFFSET: f32 = 30.0;
/// С какого расстояния танк стреляет, м: чуть меньше длины луча.
const TANK_FIRE_RANGE: f32 = 36.0;
/// Сколько видна линия прицела перед выстрелом, с.
const TANK_AIM_TIME: f32 = 1.0;
/// Перезарядка танка, с.
const TANK_RELOAD: f32 = 4.0;
/// Урон импульса танка.
const TANK_LASER_DAMAGE: f32 = 70.0;
/// Цвет танка.
const TANK_COLOR: Color = Color::srgb(0.32, 0.36, 0.3);

/// Подключает врагов.
pub struct EnemyPlugin;

impl Plugin for EnemyPlugin {
    fn build(&self, app: &mut App) {
        app.add_run_message::<SpawnEnemy>()
            .add_systems(Startup, create_enemy_assets)
            .add_systems(
                Update,
                (
                    (
                        spawn_enemies,
                        chase_rover,
                        drive_mortarmen,
                        drive_rocketeers,
                        drive_tanks,
                        avoid_lava,
                        steering::steer_vehicles,
                        vehicle::move_ground_vehicles,
                        separate_enemies,
                        ram_rover,
                        shoot_rover,
                        fire_mortarmen,
                    )
                        .chain()
                        .in_set(GameSet::Simulation),
                    // После всех читателей `Died`: трофей и звук ещё застают врага.
                    (remove_far_enemies, remove_dead_enemies).chain().in_set(GameSet::Cleanup),
                ),
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
    /// Прячется в укрытиях и стреляет ракетами.
    Rocketeer,
    /// Медленный прочный танк с импульсным лазером, заходит с фланга.
    HeavyTank,
}

/// Что делает ракетчик.
#[derive(Debug, Clone, Copy, PartialEq)]
enum CoverState {
    /// Едет к укрытию.
    Moving,
    /// Сидит в укрытии, остаток времени, с.
    Hiding(f32),
    /// Высунулся к точке, откуда видно луноход.
    Peeking(Vec3),
}

/// Ракетчик: укрытие и что он в нём делает.
#[derive(Component)]
struct Rocketeer {
    cover: Option<Vec3>,
    state: CoverState,
    recheck: f32,
}

/// Что делает танк.
#[derive(Debug, Clone, Copy, PartialEq)]
enum TankState {
    /// Заходит с фланга.
    Flank,
    /// Стоит и целится, остаток времени, с.
    Aiming(f32),
}

/// Тяжёлый танк.
#[derive(Component)]
struct HeavyTank {
    state: TankState,
    reload: f32,
    /// Сторона захода: 1 или -1.
    side: f32,
}

/// Линия прицела танка: тонкий красный луч, виден только во время прицеливания.
#[derive(Component)]
struct AimLine;

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
    /// Из дома: выбивает дверь на стене со стороны лунохода и выезжает сквозь неё.
    House(Entity),
}

/// Общие меши и материалы врагов: одинаковые враги рисуются одним пакетом.
#[derive(Resource)]
struct EnemyAssets {
    hull: Handle<Mesh>,
    wheel: Handle<Mesh>,
    ram: Handle<Mesh>,
    barrel: Handle<Mesh>,
    launcher: Handle<Mesh>,
    aim_line: Handle<Mesh>,
    rocketeer_material: Handle<StandardMaterial>,
    tank_material: Handle<StandardMaterial>,
    aim_material: Handle<StandardMaterial>,
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
        launcher: meshes.add(Cuboid::new(1.0, 0.5, 1.2)),
        // Ось цилиндра — Y; линия поворачивается вдоль -Z при создании.
        aim_line: meshes.add(Cylinder::new(0.04, 1.0)),
        rocketeer_material: materials.add(ROCKETEER_COLOR),
        tank_material: materials.add(TANK_COLOR),
        aim_material: materials.add(StandardMaterial {
            base_color: Color::srgba(1.0, 0.15, 0.1, 0.8),
            emissive: LinearRgba::rgb(12.0, 1.0, 0.5),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            ..default()
        }),
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
    mut dice: ResMut<GameRng>,
    assets: Res<EnemyAssets>,
    terrain: Res<Terrain>,
    rover: Query<(&Transform, &Health), With<Rover>>,
    houses: Query<&House>,
    settlements: Res<Settlements>,
    mut doors: MessageWriter<BreakDoor>,
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
            SpawnPlace::House(entity) => {
                let Some(state) = houses.get(entity).ok().and_then(|h| settlements.house(h.0)) else {
                    continue;
                };
                let (side, out) = state.side_facing(Vec2::new(rover.translation.x, rover.translation.z));
                doors.write(BreakDoor { house: entity, side });
                // Враг встаёт в проёме: стена дома выталкивает его наружу.
                let door = state.wall_center(side) + out * 0.3;
                Vec3::new(door.x, 0.0, door.y)
            }
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

    let (health, material, scale, turn) = match kind {
        EnemyKind::RamShooter => (ENEMY_HEALTH, assets.hull_material.clone(), 1.0, ENEMY_TURN_SPEED),
        EnemyKind::Mortar => (MORTAR_HEALTH, assets.mortar_material.clone(), 1.0, ENEMY_TURN_SPEED),
        EnemyKind::Rocketeer => (ROCKETEER_HEALTH, assets.rocketeer_material.clone(), 1.0, ENEMY_TURN_SPEED),
        EnemyKind::HeavyTank => (TANK_HEALTH, assets.tank_material.clone(), TANK_SCALE, TANK_TURN_SPEED),
    };
    position.y += RIDE_HEIGHT * (scale - 1.0);
    let mut enemy = commands.spawn((
        Enemy,
        RunScoped,
        Health::new(health),
        Team::Enemy,
        Collider {
            radius: ENEMY_RADIUS * scale,
        },
        GroundVehicle::new(heading, RIDE_HEIGHT * scale, Vec2::new(HULL_SIZE.x, HULL_SIZE.z) * 0.5 * scale),
        Steering::new(heading, turn),
        Mesh3d(assets.hull.clone()),
        MeshMaterial3d(material),
        Transform::from_translation(position)
            .with_rotation(Quat::from_rotation_y(heading))
            .with_scale(Vec3::splat(scale)),
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
        EnemyKind::Rocketeer => enemy.insert(Rocketeer {
            cover: None,
            state: CoverState::Moving,
            recheck: 0.0,
        }),
        EnemyKind::HeavyTank => enemy.insert(HeavyTank {
            state: TankState::Flank,
            reload: dice.range(1.0, TANK_RELOAD),
            side: if dice.range(0.0, 1.0) < 0.5 { 1.0 } else { -1.0 },
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
                // Пусковой контейнер на спине.
                EnemyKind::Rocketeer => parent.spawn((
                    Mesh3d(assets.launcher.clone()),
                    MeshMaterial3d(assets.dark_material.clone()),
                    Transform::from_xyz(0.0, HULL_SIZE.y * 0.5 + 0.25, 0.2),
                )),
                // Длинный ствол лазера вперёд и линия прицела, пока скрытая.
                EnemyKind::HeavyTank => {
                    parent.spawn((
                        Mesh3d(assets.barrel.clone()),
                        MeshMaterial3d(assets.dark_material.clone()),
                        Transform::from_xyz(0.0, HULL_SIZE.y * 0.5 + 0.2, -0.9)
                            .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))
                            .with_scale(Vec3::new(0.5, 1.2, 0.5)),
                    ));
                    let length = TANK_FIRE_RANGE / TANK_SCALE;
                    parent.spawn((
                        AimLine,
                        Mesh3d(assets.aim_line.clone()),
                        MeshMaterial3d(assets.aim_material.clone()),
                        bevy::light::NotShadowCaster,
                        Visibility::Hidden,
                        Transform::from_xyz(0.0, HULL_SIZE.y * 0.5 + 0.2, -length * 0.5)
                            .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))
                            .with_scale(Vec3::new(1.0, length, 1.0)),
                    ))
                }
            };
        });
}

/// Направляет врагов на луноход; объезд препятствий — в `steering`. Когда луноход погиб, враги встают.
fn chase_rover(
    rover: Query<(&Transform, &Health), With<Rover>>,
    mut enemies: Query<
        (&Transform, &mut Steering),
        (With<Enemy>, Without<Mortarman>, Without<Rocketeer>, Without<HeavyTank>),
    >,
) {
    let target = rover
        .iter()
        .next()
        .filter(|(_, health)| !health.is_dead())
        .map(|(transform, _)| transform.translation);
    for (transform, mut steering) in &mut enemies {
        let Some(target) = target else {
            steering.speed = 0.0;
            continue;
        };
        steering.desired = vehicle::heading_towards(target - transform.translation);
        steering.speed = ENEMY_SPEED;
    }
}

/// Мортирщик подъезжает на дальность выстрела, встаёт и отступает, если луноход слишком близко.
fn drive_mortarmen(
    rover: Query<(&Transform, &Health), With<Rover>>,
    mut enemies: Query<(&Transform, &mut Steering), With<Mortarman>>,
) {
    let target = rover
        .iter()
        .next()
        .filter(|(_, health)| !health.is_dead())
        .map(|(transform, _)| transform.translation);
    for (transform, mut steering) in &mut enemies {
        let Some(target) = target else {
            steering.speed = 0.0;
            continue;
        };
        let to_target = target - transform.translation;
        let (direction, speed) = match mortar_move(to_target.length()) {
            MortarMove::Approach => (to_target, MORTAR_SPEED),
            MortarMove::Hold => (to_target, 0.0),
            MortarMove::Retreat => (-to_target, MORTAR_SPEED),
        };
        steering.desired = vehicle::heading_towards(direction);
        steering.speed = speed;
    }
}

/// Скрыта ли точка `spot` от лунохода: рельеф, дома или валуны между ними.
fn hidden_from<'a>(
    terrain: &Terrain,
    obstacles: impl IntoIterator<Item = (Entity, &'a Obstacle)>,
    spot: Vec3,
    rover: Vec3,
) -> bool {
    !steering::has_los(terrain, obstacles, spot + Vec3::Y * 0.8, rover + Vec3::Y * 0.8)
}

/// Насколько точка ниже окружающего рельефа в 4 м вокруг, м.
fn dip_at(terrain: &Terrain, x: f32, z: f32) -> f32 {
    let around = [(4.0, 0.0), (-4.0, 0.0), (0.0, 4.0), (0.0, -4.0)]
        .iter()
        .map(|(dx, dz)| terrain.height_at(x + dx, z + dz))
        .sum::<f32>()
        / 4.0;
    around - terrain.height_at(x, z)
}

/// Ищет укрытие: из кандидатов в кольце вокруг лунохода со стороны ракетчика выбирает скрытую точку,
/// предпочитая дно кратера и близкие к ракетчику. `None`, если скрытых точек нет.
fn find_cover(
    terrain: &Terrain,
    pools: &crate::lava::LavaPools,
    obstacles: &Query<(Entity, &Obstacle)>,
    dice: &mut Dice,
    me: Vec3,
    rover: Vec3,
) -> Option<Vec3> {
    let base = (me.z - rover.z).atan2(me.x - rover.x);
    let mut best: Option<(f32, Vec3)> = None;
    for _ in 0..COVER_CANDIDATES {
        let angle = base + dice.range(-1.4, 1.4);
        let mut p = ring_point(rover, angle, dice.range(COVER_RING.0, COVER_RING.1));
        p.y = terrain.height_at(p.x, p.z);
        if !hidden_from(terrain, obstacles.iter(), p, rover) || crate::lava::is_lava(pools, p) {
            continue;
        }
        let mut score = -p.distance(me) * 0.1;
        if dip_at(terrain, p.x, p.z) > CRATER_DIP {
            score += 5.0;
        }
        if best.is_none_or(|(b, _)| score > b) {
            best = Some((score, p));
        }
    }
    best.map(|(_, p)| p)
}

/// Ракетчик: ищет укрытие, прячется, высовывается, стреляет ракетой и прячется снова.
#[allow(clippy::too_many_arguments)]
fn drive_rocketeers(
    time: Res<Time>,
    terrain: Res<Terrain>,
    pools: Res<crate::lava::LavaPools>,
    obstacles: Query<(Entity, &Obstacle)>,
    mut dice: ResMut<GameRng>,
    rover: Query<(&Transform, &Health), With<Rover>>,
    mut enemies: Query<(&Transform, &mut Steering, &mut Rocketeer), Without<Rover>>,
    mut rockets: MessageWriter<FireRocket>,
) {
    let target = rover
        .iter()
        .next()
        .filter(|(_, health)| !health.is_dead())
        .map(|(transform, _)| transform.translation);
    let dt = time.delta_secs();
    for (transform, mut steering, mut robot) in &mut enemies {
        let Some(target) = target else {
            steering.speed = 0.0;
            continue;
        };
        let me = transform.translation;
        robot.recheck -= dt;
        // Укрытие перепроверяется не каждый кадр: видимость дорогая.
        if robot.recheck <= 0.0 && !matches!(robot.state, CoverState::Peeking(_)) {
            robot.recheck = COVER_RECHECK;
            let lost = robot
                .cover
                .is_none_or(|c| !hidden_from(&terrain, obstacles.iter(), c, target) || c.distance(target) > COVER_RING.1 + 10.0);
            if lost {
                robot.cover = find_cover(&terrain, &pools, &obstacles, &mut dice, me, target);
                robot.state = CoverState::Moving;
            }
        }
        // Укрытия нет — подъезжает на дальность выстрела и стреляет оттуда.
        let cover = robot.cover.unwrap_or_else(|| ring_point(target, (me.z - target.z).atan2(me.x - target.x), 20.0));
        match robot.state {
            CoverState::Moving => {
                let to = (cover - me).with_y(0.0);
                if to.length() < 1.5 {
                    robot.state = CoverState::Hiding(dice.range(HIDE_TIME.0, HIDE_TIME.1));
                    steering.speed = 0.0;
                } else {
                    steering.desired = vehicle::heading_towards(to);
                    steering.speed = ROCKETEER_SPEED;
                }
            }
            CoverState::Hiding(left) => {
                steering.speed = 0.0;
                steering.desired = vehicle::heading_towards(target - me);
                let left = left - dt;
                robot.state = if left > 0.0 {
                    CoverState::Hiding(left)
                } else {
                    // Высовывается вбок от линии на луноход.
                    let side = (target - me).with_y(0.0).normalize_or(Vec3::X).cross(Vec3::Y);
                    let sign = if dice.range(0.0, 1.0) < 0.5 { 1.0 } else { -1.0 };
                    CoverState::Peeking(cover + side * sign * PEEK_SHIFT)
                };
            }
            CoverState::Peeking(spot) => {
                let muzzle = me + Vec3::Y * LAUNCHER_HEIGHT;
                let to = (spot - me).with_y(0.0);
                let sees = steering::has_los(&terrain, obstacles.iter(), muzzle, target + Vec3::Y * 0.5);
                if sees || to.length() < 1.0 {
                    if sees || robot.cover.is_none() {
                        rockets.write(FireRocket {
                            origin: muzzle,
                            target,
                            team: Team::Enemy,
                        });
                    }
                    robot.state = CoverState::Moving;
                    steering.speed = 0.0;
                } else {
                    steering.desired = vehicle::heading_towards(to);
                    steering.speed = ROCKETEER_SPEED;
                }
            }
        }
    }
}

/// Танк заходит с фланга, на дальности выстрела встаёт, секунду доворачивает с видимой линией
/// прицела и бьёт импульсом лазера по направлению ствола.
#[allow(clippy::too_many_arguments)]
fn drive_tanks(
    time: Res<Time>,
    terrain: Res<Terrain>,
    obstacles: Query<(Entity, &Obstacle)>,
    rover: Query<(&Transform, &Health), With<Rover>>,
    mut enemies: Query<(&Transform, &GroundVehicle, &mut Steering, &mut HeavyTank, &Children), Without<Rover>>,
    mut lines: Query<&mut Visibility, With<AimLine>>,
    mut lasers: MessageWriter<FireLaser>,
) {
    let target = rover
        .iter()
        .next()
        .filter(|(_, health)| !health.is_dead())
        .map(|(transform, _)| transform.translation);
    let dt = time.delta_secs();
    for (transform, vehicle, mut steering, mut tank, children) in &mut enemies {
        tank.reload -= dt;
        let mut show_line = false;
        let Some(target) = target else {
            steering.speed = 0.0;
            tank.state = TankState::Flank;
            continue;
        };
        let me = transform.translation;
        let to_rover = (target - me).with_y(0.0);
        match tank.state {
            TankState::Flank => {
                // Точка сбоку от линии «луноход — танк»: танк не едет в лоб.
                let side = to_rover.normalize_or(Vec3::X).cross(Vec3::Y) * tank.side;
                let goal = target - to_rover.normalize_or(Vec3::X) * (TANK_FIRE_RANGE * 0.7) + side * FLANK_OFFSET;
                let to_goal = (goal - me).with_y(0.0);
                let in_range = to_rover.length() < TANK_FIRE_RANGE;
                let muzzle = me + Vec3::Y * 1.2;
                if in_range
                    && tank.reload <= 0.0
                    && steering::has_los(&terrain, obstacles.iter(), muzzle, target + Vec3::Y * 0.5)
                {
                    tank.state = TankState::Aiming(TANK_AIM_TIME);
                } else if to_goal.length() < 3.0 {
                    // Дошёл до точки фланга: заходит с другой стороны.
                    tank.side = -tank.side;
                } else {
                    steering.desired = vehicle::heading_towards(to_goal);
                    steering.speed = TANK_SPEED;
                }
            }
            TankState::Aiming(left) => {
                show_line = true;
                steering.speed = 0.0;
                steering.desired = vehicle::heading_towards(to_rover);
                let left = left - dt;
                if left > 0.0 {
                    tank.state = TankState::Aiming(left);
                } else {
                    lasers.write(FireLaser {
                        origin: me + Vec3::Y * (HULL_SIZE.y * 0.5 + 0.2) * TANK_SCALE,
                        direction: vehicle.forward(),
                        team: Team::Enemy,
                        damage: TANK_LASER_DAMAGE,
                    });
                    tank.reload = TANK_RELOAD;
                    tank.state = TankState::Flank;
                    show_line = false;
                }
            }
        }
        for child in children.iter() {
            if let Ok(mut visibility) = lines.get_mut(child) {
                *visibility = if show_line { Visibility::Visible } else { Visibility::Hidden };
            }
        }
    }
}

/// Все, кроме таран-стрелков, объезжают лаву: если впереди по курсу озеро, курс уводится вбок.
fn avoid_lava(
    pools: Res<crate::lava::LavaPools>,
    mut enemies: Query<(&Transform, &mut Steering), (With<Enemy>, Without<EnemyWeapons>)>,
) {
    for (transform, mut steering) in &mut enemies {
        if steering.speed == 0.0 {
            continue;
        }
        for turn in [0.0, 0.8, -0.8, 1.6, -1.6] {
            let heading = steering.desired + turn;
            let ahead = transform.translation + vehicle::forward_of(heading) * 5.0;
            if !crate::lava::is_lava(&pools, ahead) {
                steering.desired = heading;
                break;
            }
        }
    }
}

/// Точка, куда мортирщик целится: туда, где луноход окажется к падению снаряда.
fn lead_target(rover: Vec3, velocity: Vec3) -> Vec3 {
    rover + velocity * mortar::FLIGHT_TIME
}

/// Стоящий мортирщик раз в несколько секунд стреляет с упреждением и разбросом.
fn fire_mortarmen(
    time: Res<Time>,
    mut dice: ResMut<GameRng>,
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

/// Стрельба: на средней дистанции и только когда между врагом и луноходом нет рельефа,
/// домов и крупных объектов.
fn shoot_rover(
    time: Res<Time>,
    terrain: Res<Terrain>,
    obstacles: Query<(Entity, &Obstacle)>,
    mut dice: ResMut<GameRng>,
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
            && steering::has_los(&terrain, &obstacles, muzzle, target);
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
    enemies: Query<(Entity, &Transform), (With<Enemy>, Without<Boss>)>,
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
    enemies: Query<&Transform, (With<Enemy>, Without<Boss>)>,
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
