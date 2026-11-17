//! Враги. Первый тип — таран-стрелок: появляется за кадром, едет прямо на луноход,
//! стреляет очередями при прямой видимости и таранит вплотную.
//!
//! Второй тип — мортирщик: держит дистанцию и накрывает луноход навесным огнём.
//!
//! Третий и четвёртый — тяжёлые танки на общем шасси. Ракетный заходит с фланга, наводит башню
//! и пускает из ствола ракету. Лазерный с лазерной установкой вместо башни заходит так же,
//! секунду показывает линию прицела и бьёт импульсом лазера, который режет рельеф.
//!
//! Пятый — перехватчик: быстрая длинная машина без оружия, приходит, только когда игрок
//! уезжает от волны (`src/interceptor.rs`).
//!
//! Когда и сколько врагов появляется, решает директор волн в `src/waves.rs`: он просит
//! поставить врага сообщением [`SpawnEnemy`].

use bevy::prelude::*;

use crate::boss::Boss;
use crate::buildings::{BreakDoor, House};
use crate::settlements::Settlements;
use crate::bullet::FireBullet;
use crate::combat::{Collider, Damage, Died, Health, Team};
use crate::enemy_look;
use crate::explosion::Explosion;
use crate::frame::{GameSet, RunMessages};
use crate::interceptor::{self, Interceptor};
use crate::laser::FireLaser;
use crate::models::Models;
use crate::mortar::{self, FireShell};
use crate::obstacles::Obstacle;
use crate::pickups::CarriedWeapon;
use crate::rng::GameRng;
use crate::rockets::FireRocket;
use crate::rover::{Rover, RoverMotion};
use crate::run::RunScoped;
use crate::sound_list::Dice;
use crate::steering::{self, Steering};
use crate::tank_look::{self, TankGun};
use crate::terrain::Terrain;
use crate::vehicle::{self, GroundVehicle};
use crate::waves::enemy_damage;
use crate::weapons::CurrentWeapon;

/// Здоровье таран-стрелка.
pub const ENEMY_HEALTH: f32 = 30.0;

/// Скорость таран-стрелка, м/с. Луноход быстрее: от врага можно уехать.
pub const ENEMY_SPEED: f32 = 9.0;

/// Скорость поворота врага к цели, рад/с.
const ENEMY_TURN_SPEED: f32 = 2.2;

/// Радиус коллайдера врага, м.
const ENEMY_RADIUS: f32 = 1.2;

/// Размер корпуса врага: ширина, высота, длина, м.
const HULL_SIZE: Vec3 = Vec3::new(1.4, 0.7, 2.0);

/// Радиус колеса врага, м: задаёт посадку корпуса. Сами колёса машин — `enemy_look`.
const WHEEL_RADIUS: f32 = 0.32;

/// Высота центра корпуса врага над поверхностью, м.
pub(crate) const RIDE_HEIGHT: f32 = HULL_SIZE.y * 0.5 + WHEEL_RADIUS * 0.6;

/// Кольцо появления вокруг лунохода, м. Кадр видит около 30 м от центра, враг появляется вне его.
const SPAWN_RING: (f32, f32) = (45.0, 55.0);

/// Враги дальше этого расстояния от лунохода убираются, м.
const DESPAWN_DISTANCE: f32 = 120.0;

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

/// Высота дула над центром корпуса, м.
const MUZZLE_HEIGHT: f32 = 0.5 * RAMMER_GROWTH;

/// Взрыв при гибели врага: радиус, м, и мощность. Мощность ниже порога кратера.
const DEATH_BLAST: (f32, f32) = (2.5, 0.25);

/// Цвет корпуса врага: тёмный графит, как у моделей.
const HULL_COLOR: Color = Color::srgb(0.24, 0.25, 0.27);

/// Цвет колёс и тарана врага.
const DARK_COLOR: Color = Color::srgb(0.1, 0.1, 0.12);

/// Здоровье мортирщика.
pub const MORTAR_HEALTH: f32 = 40.0;

/// Скорость мортирщика, м/с.
const MORTAR_SPEED: f32 = 7.0;

/// Ближе этого расстояния до лунохода мортирщик отступает, дальше второго — подъезжает, м.
const MORTAR_BAND: (f32, f32) = (22.0, 34.0);

/// Перезарядка мортиры врага, с.
const MORTAR_RELOAD: f32 = 3.0;

/// Разброс точки прицеливания мортирщика, м.
const MORTAR_SCATTER: f32 = 3.0;

/// Цвет корпуса мортирщика.
const MORTAR_COLOR: Color = Color::srgb(0.2, 0.21, 0.23);

/// Во сколько раз враги крупнее прежнего: все, кроме босса, больше лунохода (enemy-size-medkit).
/// Размеры дула и пусковой растут вместе с обликом.
const GROWTH: f32 = 1.5;
/// Таран-стрелок узкий, поэтому растёт сильнее: и длиннее, почти как луноход с гусеницами в ширину и вдвое длиннее.
const RAMMER_GROWTH: f32 = 1.8;
/// Танк и так велик: растёт меньше, чтобы босс оставался самым крупным.
const TANK_GROWTH: f32 = 1.3;

/// Здоровье лазерного танка.
pub const TANK_HEALTH: f32 = 220.0;
/// Здоровье ракетного танка: он приходит раньше лазерного и стоит в волне дешевле.
pub const ROCKET_TANK_HEALTH: f32 = 160.0;
/// Скорость танка, м/с.
const TANK_SPEED: f32 = 6.0;
/// Поворот танка, рад/с: медленный, линия прицела заметно доворачивает.
const TANK_TURN_SPEED: f32 = 1.2;
/// Масштаб врагов относительно базового корпуса `HULL_SIZE`: корпус, модель, колёса, коллайдер
/// и посадка растягиваются вместе. Подобран к луноходу (около 3,6 × 3,0 м, высота 2,5 м)
/// и жителям (рост 1,7 м): бронебагги 6,1 × 3,1 м, транспортёр мортиры 5,8 м,
/// тяжёлые танки 5,7 м длиной — все крупнее лунохода, босс крупнее всех.
const RAMMER_SCALE: f32 = 1.4 * RAMMER_GROWTH;
const MORTAR_SCALE: f32 = 1.5 * GROWTH;
pub(crate) const TANK_SCALE: f32 = 2.1 * TANK_GROWTH;
/// Насколько в стороне от линии к луноходу танк заходит, м.
const FLANK_OFFSET: f32 = 30.0;
/// На каком расстоянии от лунохода точка фланга, м: в дальности выстрела.
const FLANK_DISTANCE: f32 = TANK_FIRE_RANGE * 0.7;
/// Насколько курс корпуса может отличаться от направления на луноход, чтобы прицеливание шло, рад.
const TANK_AIM_TOLERANCE: f32 = 0.08;
/// С какого расстояния танк стреляет, м: чуть меньше длины луча.
pub(crate) const TANK_FIRE_RANGE: f32 = 36.0;
/// Сколько видна линия прицела перед выстрелом, с.
const TANK_AIM_TIME: f32 = 1.0;
/// Перезарядка танка, с; на тяжёлой короче (`Difficulty::tank_reload`).
const TANK_RELOAD: f32 = 4.0;
/// Ракетный танк наводит башню без линии прицела: столько стоит перед выстрелом, с.
const ROCKET_AIM_TIME: f32 = 0.5;
/// Насколько башня ракетного танка может не смотреть на луноход при выстреле, рад.
const ROCKET_AIM_TOLERANCE: f32 = 0.12;
// Перезарядка ракетного танка — `Difficulty::tank_reload`: 3,5 с, на тяжёлой 2 с.
/// Скорость поворота башни танка, рад/с.
const TANK_TURRET_TURN_SPEED: f32 = 1.5;

/// Здоровье перехватчика: три пули пулемёта.
pub const INTERCEPTOR_HEALTH: f32 = 15.0;
/// Скорость перехватчика, м/с: быстрее лунохода.
pub const INTERCEPTOR_SPEED: f32 = 20.0;
/// Поворот перехватчика, рад/с: радиус разворота 12,5 м, вдвое больше, чем у лунохода.
const INTERCEPTOR_TURN_SPEED: f32 = 1.6;
/// Корпус перехватчика: ширина, высота, длина, м.
const INTERCEPTOR_HULL: Vec3 = Vec3::new(1.0, 0.45, 4.4);
/// Круговой коллайдер перехватчика для пуль и взрывов, м: покрывает середину корпуса.
const INTERCEPTOR_RADIUS: f32 = 1.7;
/// Цвет полос на корпусе перехватчика.
const STRIPE_COLOR: Color = Color::srgb(0.95, 0.78, 0.15);

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
                        drive_tanks,
                        interceptor::drive_interceptors,
                        interceptor::drive_gadflies,
                        avoid_lava,
                        steering::steer_vehicles,
                        vehicle::move_ground_vehicles,
                        // До расталкивания: оно держит круг, а удар считается по длинному корпусу.
                        interceptor::ram_by_interceptors,
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
    /// Тяжёлый танк: заходит с фланга и стреляет из ствола башни ракетами.
    HeavyTank,
    /// Тяжёлый танк с лазерной установкой вместо пушечной башни: бьёт импульсом лазера по курсу корпуса.
    LaserTank,
    /// Быстрая машина без оружия: таранит убегающий луноход и замедляет его.
    Interceptor,
}

/// Что делает танк.
#[derive(Debug, Clone, Copy, PartialEq)]
enum TankState {
    /// Заходит с фланга.
    Flank,
    /// Стоит и целится, остаток времени, с.
    Aiming(f32),
}

/// Чем стреляет танк.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TankArm {
    /// Ракеты из ствола башни.
    Rockets,
    /// Лазерная установка: импульс по курсу корпуса.
    Laser,
}

/// Тяжёлый танк, ракетный или лазерный: шасси и заход с фланга общие.
#[derive(Component)]
pub struct HeavyTank {
    pub arm: TankArm,
    state: TankState,
    reload: f32,
    /// Сторона захода: 1 или -1.
    side: f32,
    /// Точка фланга: угол вокруг лунохода, рад. Выбирается при начале захода и держится до него,
    /// иначе точка уезжает вместе с танком и он кружит, не доходя до неё.
    flank: Option<f32>,
}

impl HeavyTank {
    /// Танк стоит и целится: видна линия прицела.
    pub fn aiming(&self) -> bool {
        matches!(self.state, TankState::Aiming(_))
    }
}

/// Угол точки фланга вокруг лунохода в `rover` для танка в `me`, заходящего со стороны `side`:
/// на [`FLANK_OFFSET`] вбок от линии «луноход — танк» на расстоянии [`FLANK_DISTANCE`].
fn flank_angle(me: Vec3, rover: Vec3, side: f32) -> f32 {
    let bearing = (me.z - rover.z).atan2(me.x - rover.x);
    bearing + side * FLANK_OFFSET.atan2(FLANK_DISTANCE)
}

/// Угол между курсами, по кратчайшей дуге, рад.
fn heading_error(heading: f32, target: f32) -> f32 {
    ((target - heading + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI).abs()
}

/// Мортирщик: время до следующего выстрела, с.
#[derive(Component)]
pub(crate) struct Mortarman {
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
    interceptor_hull: Handle<Mesh>,
    interceptor_nose: Handle<Mesh>,
    nozzle: Handle<Mesh>,
    stripe: Handle<Mesh>,
    /// Единичный брусок: лопасти винтов дрона.
    blade: Handle<Mesh>,
    prong: Handle<Mesh>,
    electrode: Handle<Mesh>,
    shock_material: Handle<StandardMaterial>,
    stripe_material: Handle<StandardMaterial>,
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
        // Запасное колесо диаметром 1 м: `enemy_look` растягивает его под размер колеса машины.
        wheel: meshes.add(Cylinder::new(0.5, 0.45)),
        ram: meshes.add(Cuboid::new(HULL_SIZE.x + 0.5, 0.35, 0.35)),
        barrel: meshes.add(Cylinder::new(0.28, 1.5)),
        interceptor_hull: meshes.add(Cuboid::from_size(INTERCEPTOR_HULL)),
        interceptor_nose: meshes.add(Cone::new(0.5, 1.2)),
        nozzle: meshes.add(Cylinder::new(0.18, 0.4)),
        stripe: meshes.add(Cuboid::new(0.12, 0.02, INTERCEPTOR_HULL.z * 0.9)),
        blade: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        prong: meshes.add(Cylinder::new(0.05, 1.0)),
        electrode: meshes.add(Sphere::new(0.12)),
        shock_material: materials.add(StandardMaterial {
            base_color: crate::shock::ARC_COLOR,
            emissive: crate::shock::ARC_GLOW,
            unlit: true,
            ..default()
        }),
        stripe_material: materials.add(STRIPE_COLOR),
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
    mut models: Models,
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
        spawn_enemy(&mut commands, &mut dice, &assets, &mut models, &terrain, position, rover.translation, request.kind);
    }
}

/// Оружие, которое носит враг типа `kind`: им он стреляет, его и роняет. Перехватчик без оружия.
fn carried_weapon(kind: EnemyKind) -> Option<CurrentWeapon> {
    match kind {
        EnemyKind::RamShooter => Some(CurrentWeapon::MachineGun),
        EnemyKind::Mortar => Some(CurrentWeapon::Mortar),
        EnemyKind::HeavyTank => Some(CurrentWeapon::Rockets),
        EnemyKind::LaserTank => Some(CurrentWeapon::Laser),
        EnemyKind::Interceptor => None,
    }
}

/// Модель врага в `models.json`. Танк собирает `tank_look` из трёх моделей.
fn enemy_model(kind: EnemyKind) -> Option<&'static str> {
    match kind {
        EnemyKind::RamShooter => Some("enemy_rammer"),
        EnemyKind::Mortar => Some("enemy_mortar"),
        EnemyKind::HeavyTank | EnemyKind::LaserTank => None,
        EnemyKind::Interceptor => Some("enemy_interceptor"),
    }
}

/// Создаёт врага типа `kind` в точке `position` носом к точке `target`.
#[allow(clippy::too_many_arguments)]
fn spawn_enemy(
    commands: &mut Commands,
    dice: &mut Dice,
    assets: &EnemyAssets,
    models: &mut Models,
    terrain: &Terrain,
    mut position: Vec3,
    target: Vec3,
    kind: EnemyKind,
) {
    position.y = terrain.height_at(position.x, position.z) + RIDE_HEIGHT;
    let heading = vehicle::heading_towards(target - position);

    let (health, material, scale, turn) = match kind {
        EnemyKind::RamShooter => (ENEMY_HEALTH, assets.hull_material.clone(), RAMMER_SCALE, ENEMY_TURN_SPEED),
        EnemyKind::Mortar => (MORTAR_HEALTH, assets.mortar_material.clone(), MORTAR_SCALE, ENEMY_TURN_SPEED),
        EnemyKind::HeavyTank => (ROCKET_TANK_HEALTH, assets.dark_material.clone(), TANK_SCALE, TANK_TURN_SPEED),
        EnemyKind::LaserTank => (TANK_HEALTH, assets.dark_material.clone(), TANK_SCALE, TANK_TURN_SPEED),
        EnemyKind::Interceptor => (INTERCEPTOR_HEALTH, assets.hull_material.clone(), GROWTH, INTERCEPTOR_TURN_SPEED),
    };
    let (radius, half_base) = match kind {
        EnemyKind::Interceptor => (INTERCEPTOR_RADIUS * scale, Vec2::new(1.6, 1.6) * scale),
        _ => (ENEMY_RADIUS * scale, Vec2::new(HULL_SIZE.x, HULL_SIZE.z) * 0.5 * scale),
    };
    position.y += RIDE_HEIGHT * (scale - 1.0);
    let mut enemy = commands.spawn((
        Enemy,
        RunScoped,
        Health::new(health),
        Team::Enemy,
        Collider { radius },
        GroundVehicle::new(heading, RIDE_HEIGHT * scale, half_base),
        Steering::new(heading, turn),
        Transform::from_translation(position)
            .with_rotation(Quat::from_rotation_y(heading))
            .with_scale(Vec3::splat(scale)),
        Visibility::default(),
    ));
    // Враг носит своё оружие и роняет его при гибели (`pickups::drop_loot`).
    if let Some(weapon) = carried_weapon(kind) {
        enemy.insert(CarriedWeapon { weapon });
    }
    // Корпус, башню и гусеницы танка ставит `tank_look`.
    if matches!(kind, EnemyKind::HeavyTank | EnemyKind::LaserTank) {
        enemy.insert(TankGun::default());
    }
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
        EnemyKind::HeavyTank | EnemyKind::LaserTank => enemy.insert(HeavyTank {
            arm: if kind == EnemyKind::LaserTank { TankArm::Laser } else { TankArm::Rockets },
            state: TankState::Flank,
            reload: dice.range(1.0, TANK_RELOAD),
            side: if dice.range(0.0, 1.0) < 0.5 { 1.0 } else { -1.0 },
            flank: None,
        }),
        EnemyKind::Interceptor => enemy.insert(Interceptor::default()),
    };
    let Some(model) = enemy_model(kind) else {
        return;
    };
    // Модель врага стоит низом на земле; пока она грузится или без неё — корпус из примитивов.
    // Колёса машин — отдельные сущности, они крутятся и поворачивают.
    enemy.with_children(|parent| {
        // Винты дрона крутятся поверх неподвижных лопастей модели.
        if kind == EnemyKind::Interceptor {
            interceptor::spawn_rotors(parent, assets.blade.clone(), assets.dark_material.clone());
        }
        models.spawn_visual(parent, model, Transform::from_xyz(0.0, -RIDE_HEIGHT, 0.0), |spare| {
            if kind == EnemyKind::Interceptor {
                spawn_interceptor_spare(spare, assets);
                return;
            }
            spare.spawn((Mesh3d(assets.hull.clone()), MeshMaterial3d(material)));
            match kind {
                // Таран на носу: видно, куда враг едет.
                EnemyKind::RamShooter => {
                    spare.spawn((
                        Mesh3d(assets.ram.clone()),
                        MeshMaterial3d(assets.dark_material.clone()),
                        Transform::from_xyz(0.0, -0.1, -HULL_SIZE.z * 0.5 - 0.15),
                    ));
                }
                // Ствол мортиры смотрит вверх и вперёд.
                EnemyKind::Mortar => {
                    spare.spawn((
                        Mesh3d(assets.barrel.clone()),
                        MeshMaterial3d(assets.dark_material.clone()),
                        Transform::from_xyz(0.0, HULL_SIZE.y * 0.5 + 0.45, -0.2)
                            .with_rotation(Quat::from_rotation_x(-0.5)),
                    ));
                }
                // У танков облик собирает `tank_look`, у перехватчика — свой запасной облик выше.
                EnemyKind::HeavyTank | EnemyKind::LaserTank | EnemyKind::Interceptor => {}
            }
        });
        let wheels = match kind {
            EnemyKind::RamShooter => Some(&enemy_look::RAMMER_WHEELS),
            EnemyKind::Mortar => Some(&enemy_look::MORTAR_WHEELS),
            EnemyKind::HeavyTank | EnemyKind::LaserTank | EnemyKind::Interceptor => None,
        };
        if let Some(layout) = wheels {
            let fallback = (assets.wheel.clone(), assets.dark_material.clone());
            enemy_look::spawn_wheels(parent, models, layout, -RIDE_HEIGHT, fallback);
        }
    });
}

/// Запасной облик перехватчика: длинный низкий корпус с жёлтыми полосами, клин на носу,
/// два сопла сзади и четыре маленьких колеса, разнесённых по длине. Центр корпуса — в начале
/// координат родителя, низ колёс — на земле.
fn spawn_interceptor_spare(spare: &mut crate::models::FallbackSpawner, assets: &EnemyAssets) {
    let hull = INTERCEPTOR_HULL;
    let dark = || MeshMaterial3d(assets.dark_material.clone());
    spare.spawn((Mesh3d(assets.interceptor_hull.clone()), MeshMaterial3d(assets.hull_material.clone())));
    for x in [-0.25, 0.25] {
        spare.spawn((
            Mesh3d(assets.stripe.clone()),
            MeshMaterial3d(assets.stripe_material.clone()),
            Transform::from_xyz(x, hull.y * 0.5 + 0.01, 0.0),
        ));
    }
    // Конус носом вперёд (-Z), сплющен до высоты корпуса.
    spare.spawn((
        Mesh3d(assets.interceptor_nose.clone()),
        MeshMaterial3d(assets.hull_material.clone()),
        Transform::from_xyz(0.0, 0.0, -hull.z * 0.5 - 0.55)
            .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
            .with_scale(Vec3::new(1.0, 1.0, hull.y / hull.x)),
    ));
    // Электроды шокера: два штыря по бокам носа со светящимися шарами на концах.
    for x in [-0.32, 0.32] {
        spare.spawn((
            Mesh3d(assets.prong.clone()),
            dark(),
            Transform::from_xyz(x, 0.05, -hull.z * 0.5 - 0.5).with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
        ));
        spare.spawn((
            Mesh3d(assets.electrode.clone()),
            MeshMaterial3d(assets.shock_material.clone()),
            Transform::from_xyz(x, 0.05, -hull.z * 0.5 - 1.0),
        ));
    }
    for x in [-0.25, 0.25] {
        spare.spawn((
            Mesh3d(assets.nozzle.clone()),
            dark(),
            Transform::from_xyz(x, 0.05, hull.z * 0.5 + 0.15).with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
        ));
    }
    let wheel_y = -RIDE_HEIGHT + 0.25;
    for (x, z) in [(-0.55, -1.6), (0.55, -1.6), (-0.55, 1.6), (0.55, 1.6)] {
        spare.spawn((
            Mesh3d(assets.wheel.clone()),
            dark(),
            Transform::from_xyz(x, wheel_y, z)
                .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2))
                .with_scale(Vec3::new(0.5, 0.45, 0.5)),
        ));
    }
}

/// Направляет врагов на луноход; объезд препятствий — в `steering`. Когда луноход погиб, враги встают.
///
/// Вплотную к луноходу таран-стрелок не жмёт газ, а стоит носом к нему и таранит: иначе
/// `steering` принял бы упор в луноход за застревание и отъехал назад.
fn chase_rover(
    rover: Query<(&Transform, &Collider, &Health), With<Rover>>,
    mut enemies: Query<
        (&Transform, &Collider, &mut Steering),
        (With<Enemy>, Without<Mortarman>, Without<HeavyTank>, Without<Interceptor>),
    >,
) {
    let target = rover
        .iter()
        .next()
        .filter(|(_, _, health)| !health.is_dead())
        .map(|(transform, collider, _)| (transform.translation, collider.radius));
    for (transform, collider, mut steering) in &mut enemies {
        let Some((target, rover_radius)) = target else {
            steering.speed = 0.0;
            continue;
        };
        steering.desired = vehicle::heading_towards(target - transform.translation);
        let touching = is_ramming(transform.translation, target, collider.radius + rover_radius);
        steering.speed = if touching { 0.0 } else { ENEMY_SPEED };
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

/// Танк заходит с фланга и на дальности выстрела встаёт. Лазерный секунду доворачивает корпус
/// с видимой линией прицела и бьёт импульсом лазера по курсу; ракетный наводит башню и пускает
/// ракету из ствола в луноход.
#[allow(clippy::too_many_arguments)]
fn drive_tanks(
    time: Res<Time>,
    difficulty: Res<crate::difficulty::RunDifficulty>,
    terrain: Res<Terrain>,
    obstacles: Query<(Entity, &Obstacle)>,
    rover: Query<(&Transform, &Health), With<Rover>>,
    mut enemies: Query<(&Transform, &GroundVehicle, &mut Steering, &mut HeavyTank, &mut TankGun), Without<Rover>>,
    mut lasers: MessageWriter<FireLaser>,
    mut rockets: MessageWriter<FireRocket>,
) {
    let target = rover
        .iter()
        .next()
        .filter(|(_, health)| !health.is_dead())
        .map(|(transform, _)| transform.translation);
    let dt = time.delta_secs();
    for (transform, vehicle, mut steering, mut tank, mut gun) in &mut enemies {
        tank.reload -= dt;
        let mut show_line = false;
        let Some(target) = target else {
            steering.speed = 0.0;
            tank.state = TankState::Flank;
            gun.aiming = false;
            continue;
        };
        let me = transform.translation;
        let to_rover = (target - me).with_y(0.0);
        // Башня следит за луноходом по кратчайшей дуге; курс башни — от носа корпуса. Когда корпус
        // довернул на луноход, башня смотрит по носу, и луч идёт из её ствола.
        let wanted = vehicle::heading_towards(to_rover) - vehicle.heading;
        gun.yaw = vehicle::steer(gun.yaw, wanted, TANK_TURRET_TURN_SPEED * dt);
        gun.yaw = (gun.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        match tank.state {
            TankState::Flank => {
                // Точка сбоку от линии «луноход — танк»: танк не едет в лоб.
                let side = tank.side;
                let angle = *tank.flank.get_or_insert_with(|| flank_angle(me, target, side));
                let goal = ring_point(target, angle, FLANK_DISTANCE);
                let to_goal = (goal - me).with_y(0.0);
                let in_range = to_rover.length() < TANK_FIRE_RANGE;
                let muzzle = me + Vec3::Y * 1.2;
                if in_range
                    && tank.reload <= 0.0
                    && steering::has_los(&terrain, obstacles.iter(), muzzle, target + Vec3::Y * 0.5)
                {
                    tank.state = TankState::Aiming(match tank.arm {
                        TankArm::Laser => TANK_AIM_TIME,
                        TankArm::Rockets => ROCKET_AIM_TIME,
                    });
                } else if to_goal.length() < 3.0 {
                    // Дошёл до точки фланга: заходит с другой стороны.
                    tank.side = -tank.side;
                    tank.flank = None;
                } else {
                    steering.desired = vehicle::heading_towards(to_goal);
                    steering.speed = TANK_SPEED;
                }
            }
            TankState::Aiming(left) if tank.arm == TankArm::Rockets => {
                // Ракета летит в луноход, а не по курсу: корпус стоит, наводится одна башня.
                steering.speed = 0.0;
                steering.desired = vehicle.heading;
                let aimed = heading_error(gun.yaw, wanted) < ROCKET_AIM_TOLERANCE;
                let left = if aimed { left - dt } else { left };
                if to_rover.length() > TANK_FIRE_RANGE * 1.2 {
                    tank.state = TankState::Flank;
                } else if left > 0.0 || !aimed {
                    tank.state = TankState::Aiming(left.max(0.0));
                } else {
                    let (origin, _) = tank_look::muzzle(transform, vehicle, &gun, tank.arm);
                    rockets.write(FireRocket {
                        origin,
                        target,
                        team: Team::Enemy,
                    });
                    tank.reload = difficulty.0.tank_reload().1;
                    tank.state = TankState::Flank;
                    tank.side = -tank.side;
                    tank.flank = None;
                }
            }
            TankState::Aiming(left) => {
                show_line = true;
                steering.speed = 0.0;
                let face = vehicle::heading_towards(to_rover);
                steering.desired = face;
                // Секунда прицеливания идёт, только пока корпус и линия прицела смотрят на луноход:
                // луч бьёт по курсу корпуса, медленный танк сначала доворачивает.
                let aimed = heading_error(vehicle.heading, face) < TANK_AIM_TOLERANCE;
                let left = if aimed { left - dt } else { left };
                if to_rover.length() > TANK_FIRE_RANGE * 1.2 {
                    // Луноход ушёл из дальности: снова заход.
                    tank.state = TankState::Flank;
                    show_line = false;
                } else if left > 0.0 || !aimed {
                    tank.state = TankState::Aiming(left.max(0.0));
                } else {
                    let (origin, _) = tank_look::muzzle(transform, vehicle, &gun, tank.arm);
                    lasers.write(FireLaser {
                        origin,
                        direction: vehicle.forward(),
                        team: Team::Enemy,
                        damage: enemy_damage::TANK_LASER,
                        ..FireLaser::BASE
                    });
                    tank.reload = difficulty.0.tank_reload().0;
                    tank.state = TankState::Flank;
                    // После выстрела — новый заход с другой стороны.
                    tank.side = -tank.side;
                    tank.flank = None;
                    show_line = false;
                }
            }
        }
        gun.aiming = show_line;
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
            origin: transform.translation + Vec3::Y * 1.4 * GROWTH,
            target: lead_target(rover.translation, velocity) + scatter,
            team: Team::Enemy,
            power: enemy_damage::MORTAR_POWER,
            radius: crate::mortar::EXPLOSION_RADIUS,
            cluster: 0,
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
    mut enemies: Query<(&mut Transform, &Collider, Has<Interceptor>), With<Enemy>>,
) {
    let mut pairs = enemies.iter_combinations_mut();
    while let Some([(mut first, first_collider, _), (mut second, second_collider, _)]) = pairs.fetch_next() {
        let radius_sum = first_collider.radius + second_collider.radius;
        if let Some(push) = overlap_push(first.translation, second.translation, radius_sum) {
            first.translation += push * 0.5;
            second.translation -= push * 0.5;
        }
    }
    let Some((rover, rover_collider)) = rover.iter().next() else {
        return;
    };
    // Перехватчик выталкивается из лунохода по своему длинному корпусу в `interceptor`.
    for (mut enemy, collider, _) in enemies.iter_mut().filter(|(_, _, interceptor)| !interceptor) {
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
                amount: enemy_damage::RAM,
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
                damage: enemy_damage::BULLET,
                heavy: false,
            });
        }
    }
}

/// Убирает врагов, от которых луноход уехал далеко, и босса тоже: иначе убегающий игрок копил бы
/// боссов. Полоса здоровья босса исчезает вместе с ним.
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

/// Погибший враг взрывается, разлетается кусками корпуса и колёсами и исчезает.
fn remove_dead_enemies(
    mut commands: Commands,
    mut died: MessageReader<Died>,
    enemies: Query<&Transform, (With<Enemy>, Without<Boss>)>,
    mut explosions: MessageWriter<Explosion>,
    mut breaker: crate::fragments::Breaker,
) {
    for message in died.read() {
        if let Ok(transform) = enemies.get(message.entity) {
            crate::fragments::wreck_vehicle(&mut breaker, message.entity, transform, 1.0);
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
    fn enemy_drops_the_weapon_it_fights_with() {
        assert_eq!(carried_weapon(EnemyKind::Mortar), Some(CurrentWeapon::Mortar));
        assert_eq!(carried_weapon(EnemyKind::HeavyTank), Some(CurrentWeapon::Rockets));
        assert_eq!(carried_weapon(EnemyKind::LaserTank), Some(CurrentWeapon::Laser));
        assert_eq!(carried_weapon(EnemyKind::RamShooter), Some(CurrentWeapon::MachineGun));
    }

    #[test]
    fn перехватчик_ничего_не_роняет() {
        assert_eq!(carried_weapon(EnemyKind::Interceptor), None);
    }

    #[test]
    fn interceptor_is_faster_than_rover() {
        assert!(INTERCEPTOR_SPEED > crate::rover::MAX_FORWARD_SPEED);
        // И сбивается быстрее таран-стрелка.
        assert!(INTERCEPTOR_HEALTH < ENEMY_HEALTH);
    }

    #[test]
    fn flank_point_is_reachable_and_in_range() {
        let rover = Vec3::new(10.0, 0.0, -5.0);
        let me = rover + Vec3::new(50.0, 0.0, 0.0);
        for side in [1.0, -1.0] {
            let goal = ring_point(rover, flank_angle(me, rover, side), FLANK_DISTANCE);
            // С точки фланга танк достаёт луноход.
            assert!(goal.distance(rover) < TANK_FIRE_RANGE);
            // Точка сбоку от линии «луноход — танк», а не на ней.
            let off_line = (goal - rover).cross(me - rover).length() / (me - rover).length();
            assert!(off_line > FLANK_OFFSET * 0.5, "{off_line}");
        }
    }

    #[test]
    fn tank_aims_only_when_the_hull_faces_the_rover() {
        assert!(heading_error(0.0, 0.05) < TANK_AIM_TOLERANCE);
        // Через ±π: 3,12 и -3,12 рад расходятся на 0,043 рад, а не на 6,24.
        assert!(heading_error(3.12, -3.12) < TANK_AIM_TOLERANCE);
        assert!(heading_error(0.0, 0.3) > TANK_AIM_TOLERANCE);
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
