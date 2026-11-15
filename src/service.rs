//! Служба защиты жителей: приезжает по тревоге, окружает луноход кольцом и стреляет только
//! по нему, по дороге подбирает паникующих жителей и увозит их.
//!
//! Сколько машин и каких — решает ступень тревоги ([`Alarm::level`]): желаемый состав
//! [`composition`], пополнение раз в [`balance::REFILL_PERIOD`] со стороны ближайшего посёлка.
//! Лишние машины и все машины при нулевой тревоге уезжают прочь и исчезают за кадром.
//!
//! Машины ездят на тех же [`GroundVehicle`] и [`Steering`], что враги, и стреляют теми же
//! сообщениями. Команда — [`Team::Service`]: с врагами друг друга не ранят. Облик — белые
//! округлые корпуса с оранжевой полосой, вращающаяся антенна-тарелка, большие колёса;
//! без мигалок, надписей и полицейской раскраски.

use std::f32::consts::{FRAC_PI_2, TAU};

use bevy::prelude::*;

use crate::alarm::{self, Alarm};
use crate::buildings::House;
use crate::bullet::FireBullet;
use crate::combat::{Collider, Died, Health, Team};
use crate::enemy::Enemy;
use crate::explosion::Explosion;
use crate::frame::{GameSet, RunMessages};
use crate::lava::{self, LavaPools};
use crate::mortar::{self, FireShell};
use crate::obstacles::Obstacle;
use crate::residents::{Resident, ResidentDied};
use crate::rng::GameRng;
use crate::rockets::FireRocket;
use crate::rover::{Rover, RoverMotion};
use crate::run::{RunScoped, WorldRebuilt};
use crate::settlements::Settlements;
use crate::steering::{self, Steering};
use crate::terrain::Terrain;
use crate::vehicle::{self, GroundVehicle};

/// Числа Службы.
pub mod balance {
    /// Больше машин Службы одновременно не бывает.
    pub const MAX_VEHICLES: u8 = 10;
    /// Как часто подъезжает пополнение, с.
    pub const REFILL_PERIOD: f32 = 5.0;
    /// Сколько машин подъезжает за раз.
    pub const PER_REFILL: u8 = 2;
    /// Радиус кольца окружения вокруг лунохода, м.
    pub const RING_RADIUS: f32 = 18.0;
    /// На сколько точка на кольце опережает машину, рад: кольцо медленно вращается.
    pub const RING_LEAD: f32 = 0.15;
    /// Расстояние появления от лунохода: за кадром, м.
    pub const SPAWN_DISTANCE: f32 = 48.0;
    /// Разброс направления появления вокруг направления на посёлок, рад.
    pub const SPAWN_SPREAD: f32 = 0.35;
    /// Уезжающая машина исчезает на таком расстоянии от лунохода, м.
    pub const LEAVE_DISTANCE: f32 = 55.0;
    /// Отставшая машина убирается на таком расстоянии, м.
    pub const DESPAWN_DISTANCE: f32 = 120.0;
    /// Мест для жителей в машине.
    pub const SEATS: u8 = 4;
    /// Машина с местами едет к паникующему жителю ближе этого, м.
    pub const PICKUP_SEEK: f32 = 12.0;
    /// Житель садится в машину ближе этого, м.
    pub const PICKUP_REACH: f32 = 2.5;
    /// Скорость у жителя: медленнее, чем жители отскакивают от машин, м/с.
    pub const PICKUP_SPEED: f32 = 2.5;

    /// Патрульный вездеход: здоровье, скорость, м/с, ступень, с которой приезжает.
    pub const PATROL: (f32, f32, u8) = (25.0, 13.0, 1);
    /// Бронированный транспортёр.
    pub const TRANSPORT: (f32, f32, u8) = (90.0, 9.0, 3);
    /// Установка поддержки.
    pub const SUPPORT: (f32, f32, u8) = (160.0, 6.0, 5);

    /// Пулемёт патрульного: дальность, м, урон пули, пуль в очереди, пауза в очереди и между
    /// очередями, с.
    pub const GUN_RANGE: f32 = 26.0;
    pub const BULLET_DAMAGE: f32 = 4.0;
    pub const BURST: u8 = 3;
    pub const BURST_GAP: f32 = 0.12;
    pub const GUN_RELOAD: f32 = 1.6;
    /// Ракета транспортёра: дальность, м, перезарядка, с.
    pub const ROCKET_RANGE: f32 = 30.0;
    pub const ROCKET_RELOAD: f32 = 4.0;
    /// Залп установки поддержки: дальность, м, снарядов, перезарядка, с, мощность снаряда,
    /// разброс точек падения, м.
    pub const MORTAR_RANGE: f32 = 40.0;
    pub const VOLLEY: u8 = 3;
    pub const VOLLEY_RELOAD: f32 = 6.0;
    pub const SHELL_POWER: f32 = 0.3;
    pub const SHELL_SCATTER: f32 = 4.0;
}

/// Поворот машины Службы, рад/с.
const TURN_SPEED: f32 = 2.6;

/// Корпус: ширина, высота, длина, м.
const HULL: Vec3 = Vec3::new(1.5, 0.6, 2.4);

/// Радиус колеса: колёса крупнее, чем у врагов, м.
const WHEEL_RADIUS: f32 = 0.45;

/// Высота центра корпуса над поверхностью, м.
const RIDE_HEIGHT: f32 = HULL.y * 0.5 + WHEEL_RADIUS * 0.7;

/// Высота дула над центром корпуса, м.
const MUZZLE_HEIGHT: f32 = 0.9;

/// Разброс пуль в каждую сторону, рад.
const SPREAD: f32 = 4.0 * std::f32::consts::PI / 180.0;

/// Взрыв при гибели машины: радиус, м, и мощность — ниже порога кратера.
const DEATH_BLAST: (f32, f32) = (2.5, 0.25);

/// Скорость вращения антенны-тарелки, рад/с.
const DISH_SPIN: f32 = 2.5;

/// Белый корпус.
const WHITE: Color = Color::srgb(0.93, 0.93, 0.9);
/// Оранжевая полоса.
const ORANGE: Color = Color::srgb(1.0, 0.5, 0.08);
/// Колёса.
const DARK: Color = Color::srgb(0.12, 0.12, 0.13);
/// Тарелка и стойки.
const GREY: Color = Color::srgb(0.62, 0.64, 0.66);
/// Спасённые жители на крыше: тёплый неброский цвет одежды.
const PASSENGER: Color = Color::srgb(0.55, 0.42, 0.3);

/// Подключает Службу защиты жителей.
pub struct ServicePlugin;

impl Plugin for ServicePlugin {
    fn build(&self, app: &mut App) {
        app.add_run_message::<SpawnService>()
            .add_run_message::<ServiceArrived>()
            .init_resource::<ServiceCall>()
            .add_systems(Startup, create_assets)
            .add_systems(
                Update,
                (
                    reset_call.in_set(GameSet::Reset),
                    (call_service, spawn_service, evacuate, drive_service, avoid_lava)
                        .chain()
                        .before(steering::steer_vehicles)
                        .in_set(GameSet::Simulation),
                    (separate_service, shoot_rover)
                        .chain()
                        .after(vehicle::move_ground_vehicles)
                        .in_set(GameSet::Simulation),
                    service_deaths.in_set(GameSet::Death),
                    remove_service.in_set(GameSet::Cleanup),
                    (spin_dishes, show_passengers).in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Вид машины Службы.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    /// Патрульный вездеход: быстрый, слабый, пулемёт.
    Patrol,
    /// Бронированный транспортёр: ракеты.
    Transport,
    /// Установка поддержки: залпы мортиры.
    Support,
}

impl ServiceKind {
    const ALL: [ServiceKind; 3] = [ServiceKind::Patrol, ServiceKind::Transport, ServiceKind::Support];

    fn index(self) -> usize {
        self as usize
    }

    /// Здоровье и скорость, м/с.
    fn stats(self) -> (f32, f32) {
        let (health, speed, _) = match self {
            ServiceKind::Patrol => balance::PATROL,
            ServiceKind::Transport => balance::TRANSPORT,
            ServiceKind::Support => balance::SUPPORT,
        };
        (health, speed)
    }

    /// Во сколько раз машина крупнее патрульной.
    fn scale(self) -> f32 {
        match self {
            ServiceKind::Patrol => 1.0,
            ServiceKind::Transport => 1.25,
            ServiceKind::Support => 1.45,
        }
    }

    /// Перезарядка оружия, с.
    fn reload(self) -> f32 {
        match self {
            ServiceKind::Patrol => balance::GUN_RELOAD,
            ServiceKind::Transport => balance::ROCKET_RELOAD,
            ServiceKind::Support => balance::VOLLEY_RELOAD,
        }
    }
}

/// Желаемый состав Службы на ступени тревоги `level`: сколько патрульных, транспортёров
/// и установок поддержки. Всего `2·ступень`, но не больше [`balance::MAX_VEHICLES`].
pub fn composition(level: u8) -> [u8; 3] {
    let total = (2 * level).min(balance::MAX_VEHICLES);
    let support = if level >= balance::SUPPORT.2 { 2 } else { 0 };
    let transport = if level >= balance::TRANSPORT.2 { level - 1 } else { 0 };
    let transport = transport.min(total - support);
    [total - support - transport, transport, support]
}

/// Какую машину прислать следующей: самую тяжёлую из недостающих. `counts` — сколько машин
/// каждого вида уже на месте и не уезжает.
fn next_kind(level: u8, counts: [u8; 3]) -> Option<ServiceKind> {
    let wanted = composition(level);
    ServiceKind::ALL
        .into_iter()
        .rev()
        .find(|kind| counts[kind.index()] < wanted[kind.index()])
}

/// Машина Службы защиты жителей.
#[derive(Component, Debug)]
pub struct ServiceVehicle {
    kind: ServiceKind,
    /// Уезжает прочь: тревога спала, машина лишняя или полна жителей.
    leaving: bool,
    /// Сколько секунд до следующей очереди, ракеты или залпа.
    reload: f32,
    /// Сколько пуль или снарядов осталось в текущей очереди.
    burst_left: u8,
    burst_timer: f32,
    /// Сколько жителей в машине.
    passengers: u8,
}

/// Антенна-тарелка: вращается.
#[derive(Component)]
struct Dish;

/// Место жителя на крыше: номер с нуля. Видно, когда место занято.
#[derive(Component)]
struct Seat(u8);

/// Просьба поставить машину Службы.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpawnService {
    pub kind: ServiceKind,
    /// Направление от лунохода, рад: машина появляется за кадром.
    pub angle: f32,
}

/// Служба приехала по вызову: до этого на месте не было ни одной её машины.
#[derive(Message, Debug, Clone, Copy)]
pub struct ServiceArrived {
    /// Где появилась первая машина.
    #[allow(dead_code)]
    pub position: Vec3,
}

/// Вызов Службы: время до пополнения.
#[derive(Resource, Default, Debug)]
struct ServiceCall {
    refill: f32,
}

/// Общие меши и материалы машин Службы.
#[derive(Resource)]
struct ServiceAssets {
    hull: Handle<Mesh>,
    cabin: Handle<Mesh>,
    stripe: Handle<Mesh>,
    wheel: Handle<Mesh>,
    mast: Handle<Mesh>,
    dish: Handle<Mesh>,
    pod: Handle<Mesh>,
    tube: Handle<Mesh>,
    passenger: Handle<Mesh>,
    white: Handle<StandardMaterial>,
    orange: Handle<StandardMaterial>,
    dark: Handle<StandardMaterial>,
    grey: Handle<StandardMaterial>,
    person: Handle<StandardMaterial>,
}

fn create_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(ServiceAssets {
        hull: meshes.add(Cuboid::from_size(HULL)),
        // Округлая кабина: капсула вдоль корпуса.
        cabin: meshes.add(Capsule3d::new(0.5, 1.1)),
        stripe: meshes.add(Cuboid::new(HULL.x + 0.04, 0.16, HULL.z + 0.04)),
        wheel: meshes.add(Cylinder::new(WHEEL_RADIUS, 0.32)),
        mast: meshes.add(Cylinder::new(0.05, 0.5)),
        dish: meshes.add(Cylinder::new(0.42, 0.06)),
        pod: meshes.add(Cuboid::new(0.9, 0.35, 0.9)),
        tube: meshes.add(Cylinder::new(0.22, 1.1)),
        passenger: meshes.add(Capsule3d::new(0.12, 0.2)),
        white: materials.add(WHITE),
        orange: materials.add(ORANGE),
        dark: materials.add(DARK),
        grey: materials.add(GREY),
        person: materials.add(PASSENGER),
    });
}

fn reset_call(mut rebuilt: MessageReader<WorldRebuilt>, mut call: ResMut<ServiceCall>) {
    if rebuilt.read().last().is_some() {
        *call = ServiceCall::default();
    }
}

/// Горизонтальное направление по углу `angle`.
fn direction(angle: f32) -> Vec3 {
    Vec3::new(angle.cos(), 0.0, angle.sin())
}

/// Угол точки `p` вокруг `center`, как у [`direction`].
fn angle_around(center: Vec3, p: Vec3) -> f32 {
    (p.z - center.z).atan2(p.x - center.x)
}

/// Вызывает пополнение по ступени тревоги и отправляет лишние машины прочь.
#[allow(clippy::too_many_arguments)]
fn call_service(
    time: Res<Time>,
    alarm: Res<Alarm>,
    mut call: ResMut<ServiceCall>,
    mut dice: ResMut<GameRng>,
    rover: Query<(&Transform, &Health), With<Rover>>,
    houses: Query<&GlobalTransform, With<House>>,
    mut vehicles: Query<(&Transform, &mut ServiceVehicle)>,
    mut requests: MessageWriter<SpawnService>,
) {
    let Some((rover, health)) = rover.iter().next() else {
        return;
    };
    let level = if health.is_dead() { 0 } else { alarm.level() };
    let wanted = composition(level);

    // Лишние машины уезжают: сначала самые дальние.
    let mut counts = [0u8; 3];
    let mut staying: Vec<(f32, Mut<ServiceVehicle>)> = vehicles
        .iter_mut()
        .filter(|(_, v)| !v.leaving)
        .map(|(t, v)| (t.translation.distance(rover.translation), v))
        .collect();
    staying.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, vehicle) in &mut staying {
        let i = vehicle.kind.index();
        if counts[i] < wanted[i] {
            counts[i] += 1;
        } else {
            vehicle.leaving = true;
        }
    }

    call.refill = (call.refill - time.delta_secs()).max(0.0);
    if level == 0 || call.refill > 0.0 {
        return;
    }
    let Some(_) = next_kind(level, counts) else {
        return;
    };
    call.refill = balance::REFILL_PERIOD;

    // Со стороны ближайшего посёлка, а если посёлков рядом нет — откуда угодно.
    let toward = houses
        .iter()
        .map(GlobalTransform::translation)
        .min_by(|a, b| a.distance_squared(rover.translation).total_cmp(&b.distance_squared(rover.translation)))
        .map(|house| angle_around(rover.translation, house))
        .unwrap_or_else(|| dice.range(0.0, TAU));
    for _ in 0..balance::PER_REFILL {
        let Some(kind) = next_kind(level, counts) else {
            break;
        };
        counts[kind.index()] += 1;
        let angle = toward + dice.range(-balance::SPAWN_SPREAD, balance::SPAWN_SPREAD);
        requests.write(SpawnService { kind, angle });
    }
}

/// Ставит запрошенные машины за кадром и сообщает о приезде Службы.
#[allow(clippy::too_many_arguments)]
fn spawn_service(
    mut commands: Commands,
    mut requests: MessageReader<SpawnService>,
    mut dice: ResMut<GameRng>,
    assets: Res<ServiceAssets>,
    terrain: Res<Terrain>,
    rover: Query<&Transform, With<Rover>>,
    vehicles: Query<&ServiceVehicle>,
    mut arrived: MessageWriter<ServiceArrived>,
) {
    let Some(rover) = rover.iter().next() else {
        requests.clear();
        return;
    };
    let mut present = vehicles.iter().any(|v| !v.leaving);
    for request in requests.read() {
        let mut position = rover.translation + direction(request.angle) * balance::SPAWN_DISTANCE;
        position.y = terrain.height_at(position.x, position.z);
        if !present {
            present = true;
            arrived.write(ServiceArrived { position });
        }
        let reload = dice.range(0.5, request.kind.reload());
        spawn_vehicle(&mut commands, &assets, position, rover.translation, request.kind, reload);
    }
}

/// Создаёт машину вида `kind` на рельефе в точке `ground` носом к `target`.
fn spawn_vehicle(
    commands: &mut Commands,
    assets: &ServiceAssets,
    ground: Vec3,
    target: Vec3,
    kind: ServiceKind,
    reload: f32,
) {
    let scale = kind.scale();
    let (health, _) = kind.stats();
    let heading = vehicle::heading_towards(target - ground);
    let position = ground + Vec3::Y * RIDE_HEIGHT * scale;
    commands
        .spawn((
            ServiceVehicle {
                kind,
                leaving: false,
                reload,
                burst_left: 0,
                burst_timer: 0.0,
                passengers: 0,
            },
            RunScoped,
            Health::new(health),
            Team::Service,
            Collider {
                radius: 1.3 * scale,
            },
            GroundVehicle::new(heading, RIDE_HEIGHT * scale, Vec2::new(HULL.x, HULL.z) * 0.5 * scale),
            Steering::new(heading, TURN_SPEED),
            Mesh3d(assets.hull.clone()),
            MeshMaterial3d(assets.white.clone()),
            Transform::from_translation(position)
                .with_rotation(Quat::from_rotation_y(heading))
                .with_scale(Vec3::splat(scale)),
        ))
        .with_children(|parent| {
            // Округлая кабина впереди и оранжевая полоса вокруг корпуса.
            parent.spawn((
                Mesh3d(assets.cabin.clone()),
                MeshMaterial3d(assets.white.clone()),
                Transform::from_xyz(0.0, HULL.y * 0.5 + 0.05, -0.25)
                    .with_rotation(Quat::from_rotation_x(FRAC_PI_2))
                    .with_scale(Vec3::new(1.2, 1.0, 0.8)),
            ));
            parent.spawn((
                Mesh3d(assets.stripe.clone()),
                MeshMaterial3d(assets.orange.clone()),
                Transform::from_xyz(0.0, 0.0, 0.0),
            ));
            // Большие колёса.
            for (side, along) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                parent.spawn((
                    Mesh3d(assets.wheel.clone()),
                    MeshMaterial3d(assets.dark.clone()),
                    Transform::from_xyz(side * (HULL.x * 0.5 + 0.16), -HULL.y * 0.5, along * HULL.z * 0.34)
                        .with_rotation(Quat::from_rotation_z(FRAC_PI_2)),
                ));
            }
            // Антенна-тарелка на стойке за кабиной.
            parent.spawn((
                Mesh3d(assets.mast.clone()),
                MeshMaterial3d(assets.grey.clone()),
                Transform::from_xyz(0.0, HULL.y * 0.5 + 0.25, 0.55),
            ));
            parent.spawn((
                Dish,
                Mesh3d(assets.dish.clone()),
                MeshMaterial3d(assets.grey.clone()),
                Transform::from_xyz(0.0, HULL.y * 0.5 + 0.55, 0.55).with_rotation(Quat::from_rotation_x(0.5)),
            ));
            // Оружие по виду: пулемёт патрульного — только ствол-стойка, у транспортёра
            // ракетный контейнер, у установки поддержки — труба мортиры.
            match kind {
                ServiceKind::Patrol => {}
                ServiceKind::Transport => {
                    parent.spawn((
                        Mesh3d(assets.pod.clone()),
                        MeshMaterial3d(assets.orange.clone()),
                        Transform::from_xyz(0.0, HULL.y * 0.5 + 0.18, 0.95),
                    ));
                }
                ServiceKind::Support => {
                    parent.spawn((
                        Mesh3d(assets.tube.clone()),
                        MeshMaterial3d(assets.dark.clone()),
                        Transform::from_xyz(0.0, HULL.y * 0.5 + 0.45, 0.9).with_rotation(Quat::from_rotation_x(-0.4)),
                    ));
                }
            }
            // Места для спасённых на крыше сзади, пока пустые.
            for seat in 0..balance::SEATS {
                let x = (seat as f32 - (balance::SEATS - 1) as f32 * 0.5) * 0.3;
                parent.spawn((
                    Seat(seat),
                    Mesh3d(assets.passenger.clone()),
                    MeshMaterial3d(assets.person.clone()),
                    Visibility::Hidden,
                    Transform::from_xyz(x, HULL.y * 0.5 + 0.22, -0.95),
                ));
            }
        });
}

/// Машина с местами подбирает паникующих жителей рядом: житель исчезает в машине, в посёлок
/// не возвращается и погибшим не считается. Полная машина уезжает.
fn evacuate(
    mut commands: Commands,
    mut settlements: ResMut<Settlements>,
    residents: Query<(Entity, &Transform, &Resident)>,
    mut vehicles: Query<(&Transform, &mut ServiceVehicle)>,
) {
    let mut taken: Vec<Entity> = Vec::new();
    for (transform, mut vehicle) in &mut vehicles {
        if vehicle.passengers >= balance::SEATS {
            continue;
        }
        for (entity, resident_transform, resident) in &residents {
            if vehicle.passengers >= balance::SEATS {
                break;
            }
            if !resident.is_panicking() || taken.contains(&entity) {
                continue;
            }
            let gap = (resident_transform.translation - transform.translation).with_y(0.0).length();
            if gap > balance::PICKUP_REACH {
                continue;
            }
            taken.push(entity);
            vehicle.passengers += 1;
            if let Some(home) = settlements.house_mut(resident.home()) {
                home.residents = home.residents.saturating_sub(1);
            }
            commands.entity(entity).try_despawn();
        }
        if vehicle.passengers >= balance::SEATS {
            vehicle.leaving = true;
        }
    }
}

/// Ведёт машины: к паникующему жителю, если есть место, иначе к своей точке на кольце вокруг
/// лунохода; уезжающие — прочь от лунохода. Объезд препятствий — в `steering`.
fn drive_service(
    rover: Query<(&Transform, &Health), With<Rover>>,
    residents: Query<(&Transform, &Resident), Without<ServiceVehicle>>,
    mut vehicles: Query<(Entity, &Transform, &ServiceVehicle, &mut Steering)>,
) {
    let Some(rover) = rover
        .iter()
        .next()
        .filter(|(_, health)| !health.is_dead())
        .map(|(transform, _)| transform.translation)
    else {
        for (_, _, _, mut steering) in &mut vehicles {
            steering.speed = 0.0;
        }
        return;
    };

    // Точки кольца: машины по порядку угла вокруг лунохода, ровно по кругу, кольцо повёрнуто
    // так, чтобы машинам ехать меньше, и чуть впереди каждой — так кольцо медленно вращается.
    let mut ring: Vec<(Entity, f32)> = Vec::new();
    for (entity, transform, vehicle, mut steering) in &mut vehicles {
        let (_, speed) = vehicle.kind.stats();
        let position = transform.translation;
        if vehicle.leaving {
            steering.desired = vehicle::heading_towards(position - rover);
            steering.speed = speed;
            continue;
        }
        let seeking = (vehicle.passengers < balance::SEATS)
            .then(|| {
                residents
                    .iter()
                    .filter(|(_, r)| r.is_panicking())
                    .map(|(t, _)| t.translation)
                    .filter(|p| p.distance(position) < balance::PICKUP_SEEK)
                    .min_by(|a, b| a.distance_squared(position).total_cmp(&b.distance_squared(position)))
            })
            .flatten();
        if let Some(resident) = seeking {
            steering.desired = vehicle::heading_towards(resident - position);
            steering.speed = if resident.distance(position) < 6.0 {
                balance::PICKUP_SPEED
            } else {
                speed
            };
            continue;
        }
        ring.push((entity, angle_around(rover, position)));
    }
    let targets = ring_targets(&mut ring);
    for (entity, target_angle) in targets {
        let Ok((_, transform, vehicle, mut steering)) = vehicles.get_mut(entity) else {
            continue;
        };
        let (_, speed) = vehicle.kind.stats();
        let point = rover + direction(target_angle) * balance::RING_RADIUS;
        let to = (point - transform.translation).with_y(0.0);
        let distance = to.length();
        if distance < 1.0 {
            steering.desired = vehicle::heading_towards(rover - transform.translation);
            steering.speed = 0.0;
        } else {
            steering.desired = vehicle::heading_towards(to);
            steering.speed = (distance * 1.2).clamp(3.0, speed);
        }
    }
}

/// Углы точек кольца для машин с нынешними углами `ring` вокруг лунохода.
///
/// Машины расходятся ровно по кругу в том же порядке, в каком стоят; поворот кольца — средний
/// по машинам, чтобы никому не ехать через центр, плюс опережение [`balance::RING_LEAD`].
fn ring_targets(ring: &mut [(Entity, f32)]) -> Vec<(Entity, f32)> {
    ring.sort_by(|a, b| a.1.total_cmp(&b.1));
    let n = ring.len() as f32;
    let (sin, cos) = ring.iter().enumerate().fold((0.0, 0.0), |(s, c), (i, (_, angle))| {
        let offset = angle - TAU * i as f32 / n;
        (s + offset.sin(), c + offset.cos())
    });
    let offset = sin.atan2(cos) + balance::RING_LEAD;
    ring.iter()
        .enumerate()
        .map(|(i, (entity, _))| (*entity, offset + TAU * i as f32 / n))
        .collect()
}

/// Машины Службы объезжают лаву: если впереди по курсу озеро, курс уводится вбок.
fn avoid_lava(pools: Res<LavaPools>, mut vehicles: Query<(&Transform, &mut Steering), With<ServiceVehicle>>) {
    for (transform, mut steering) in &mut vehicles {
        if steering.speed == 0.0 {
            continue;
        }
        for turn in [0.0, 0.8, -0.8, 1.6, -1.6] {
            let heading = steering.desired + turn;
            let ahead = transform.translation + vehicle::forward_of(heading) * 5.0;
            if !lava::is_lava(&pools, ahead) {
                steering.desired = heading;
                break;
            }
        }
    }
}

/// На сколько сдвинуть машину в точке `first` от машины в `second`, чтобы они не пересекались.
fn overlap_push(first: Vec3, second: Vec3, radius_sum: f32) -> Option<Vec3> {
    let mut away = first - second;
    away.y = 0.0;
    let distance = away.length();
    if distance >= radius_sum {
        return None;
    }
    let direction = if distance > 1e-4 { away / distance } else { Vec3::X };
    Some(direction * (radius_sum - distance))
}

/// Расталкивает машины Службы между собой и отодвигает их от врагов и лунохода.
fn separate_service(
    others: Query<(&Transform, &Collider), (Or<(With<Rover>, With<Enemy>)>, Without<ServiceVehicle>)>,
    mut vehicles: Query<(&mut Transform, &Collider), With<ServiceVehicle>>,
) {
    let mut pairs = vehicles.iter_combinations_mut();
    while let Some([(mut first, a), (mut second, b)]) = pairs.fetch_next() {
        if let Some(push) = overlap_push(first.translation, second.translation, a.radius + b.radius) {
            first.translation += push * 0.5;
            second.translation -= push * 0.5;
        }
    }
    for (mut transform, collider) in &mut vehicles {
        for (other, other_collider) in &others {
            if let Some(push) = overlap_push(transform.translation, other.translation, collider.radius + other_collider.radius) {
                transform.translation += push;
            }
        }
    }
}

/// Стрельба по луноходу: пулемёт и ракеты — при прямой видимости, мортира — навесом.
/// Уезжающие машины не стреляют.
#[allow(clippy::too_many_arguments)]
fn shoot_rover(
    time: Res<Time>,
    terrain: Res<Terrain>,
    obstacles: Query<(Entity, &Obstacle)>,
    mut dice: ResMut<GameRng>,
    rover: Query<(&Transform, &RoverMotion, &Health), With<Rover>>,
    mut vehicles: Query<(&Transform, &mut ServiceVehicle), Without<Rover>>,
    mut bullets: MessageWriter<FireBullet>,
    mut rockets: MessageWriter<FireRocket>,
    mut shells: MessageWriter<FireShell>,
) {
    let Some((rover, motion, health)) = rover.iter().next() else {
        return;
    };
    if health.is_dead() {
        return;
    }
    let target = rover.translation;
    let velocity = vehicle::forward_of(motion.heading) * motion.speed;
    let dt = time.delta_secs();
    for (transform, mut vehicle) in &mut vehicles {
        vehicle.reload = (vehicle.reload - dt).max(0.0);
        if vehicle.leaving {
            vehicle.burst_left = 0;
            continue;
        }
        let muzzle = transform.translation + Vec3::Y * MUZZLE_HEIGHT * vehicle.kind.scale();
        let distance = muzzle.distance(target);
        match vehicle.kind {
            ServiceKind::Patrol | ServiceKind::Support => {
                let (range, needs_sight) = if vehicle.kind == ServiceKind::Patrol {
                    (balance::GUN_RANGE, true)
                } else {
                    (balance::MORTAR_RANGE, false)
                };
                if vehicle.burst_left == 0 {
                    let ready = vehicle.reload <= 0.0
                        && distance <= range
                        && (!needs_sight || steering::has_los(&terrain, &obstacles, muzzle, target));
                    if !ready {
                        continue;
                    }
                    vehicle.burst_left = if needs_sight { balance::BURST } else { balance::VOLLEY };
                    vehicle.burst_timer = 0.0;
                    vehicle.reload = vehicle.kind.reload();
                }
                vehicle.burst_timer -= dt;
                if vehicle.burst_timer > 0.0 {
                    continue;
                }
                vehicle.burst_left -= 1;
                if needs_sight {
                    vehicle.burst_timer = balance::BURST_GAP;
                    let spread = Quat::from_rotation_y(dice.range(-SPREAD, SPREAD));
                    bullets.write(FireBullet {
                        origin: muzzle,
                        direction: spread * (target - muzzle),
                        team: Team::Service,
                        damage: balance::BULLET_DAMAGE,
                        heavy: false,
                    });
                } else {
                    vehicle.burst_timer = 0.35;
                    let scatter = Vec3::new(
                        dice.range(-balance::SHELL_SCATTER, balance::SHELL_SCATTER),
                        0.0,
                        dice.range(-balance::SHELL_SCATTER, balance::SHELL_SCATTER),
                    );
                    shells.write(FireShell {
                        origin: muzzle,
                        target: target + velocity * mortar::FLIGHT_TIME + scatter,
                        team: Team::Service,
                        power: balance::SHELL_POWER,
                        radius: mortar::EXPLOSION_RADIUS,
                        cluster: 0,
                    });
                }
            }
            ServiceKind::Transport => {
                if vehicle.reload > 0.0
                    || distance > balance::ROCKET_RANGE
                    || !steering::has_los(&terrain, &obstacles, muzzle, target)
                {
                    continue;
                }
                vehicle.reload = balance::ROCKET_RELOAD;
                rockets.write(FireRocket {
                    origin: muzzle,
                    target,
                    team: Team::Service,
                });
            }
        }
    }
}

/// Гибель машины Службы: тревога растёт за машину, жители в ней погибают вместе с ней.
fn service_deaths(
    mut died: MessageReader<Died>,
    vehicles: Query<(&Transform, &ServiceVehicle)>,
    mut alarm: ResMut<Alarm>,
    mut deaths: MessageWriter<ResidentDied>,
    mut explosions: MessageWriter<Explosion>,
) {
    for message in died.read() {
        let Ok((transform, vehicle)) = vehicles.get(message.entity) else {
            continue;
        };
        alarm.add(alarm::balance::SERVICE_VEHICLE_POINTS);
        for _ in 0..vehicle.passengers {
            deaths.write(ResidentDied {
                position: transform.translation,
            });
        }
        explosions.write(Explosion {
            position: transform.translation,
            radius: DEATH_BLAST.0,
            power: DEATH_BLAST.1,
            source_team: Team::Service,
        });
    }
}

/// Убирает погибшие машины, уехавшие за кадр и отставшие от лунохода.
fn remove_service(
    mut commands: Commands,
    mut died: MessageReader<Died>,
    rover: Query<&Transform, With<Rover>>,
    vehicles: Query<(Entity, &Transform, &ServiceVehicle)>,
) {
    for message in died.read() {
        if vehicles.contains(message.entity) {
            commands.entity(message.entity).try_despawn();
        }
    }
    let Some(rover) = rover.iter().next() else {
        return;
    };
    for (entity, transform, vehicle) in &vehicles {
        let distance = transform.translation.distance(rover.translation);
        if distance > balance::DESPAWN_DISTANCE || (vehicle.leaving && distance > balance::LEAVE_DISTANCE) {
            commands.entity(entity).try_despawn();
        }
    }
}

/// Тарелки антенн вращаются.
fn spin_dishes(time: Res<Time>, mut dishes: Query<&mut Transform, With<Dish>>) {
    let turn = Quat::from_rotation_y(DISH_SPIN * time.delta_secs());
    for mut transform in &mut dishes {
        transform.rotation = turn * transform.rotation;
    }
}

/// На крыше видно, сколько жителей в машине.
fn show_passengers(
    vehicles: Query<(&ServiceVehicle, &Children), Changed<ServiceVehicle>>,
    mut seats: Query<(&Seat, &mut Visibility)>,
) {
    for (vehicle, children) in &vehicles {
        for child in children.iter() {
            let Ok((seat, mut visibility)) = seats.get_mut(child) else {
                continue;
            };
            let wanted = if seat.0 < vehicle.passengers {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            if *visibility != wanted {
                *visibility = wanted;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composition_grows_with_alarm_and_gets_heavier() {
        assert_eq!(composition(0), [0, 0, 0]);
        // Первая ступень: две лёгкие машины.
        assert_eq!(composition(1), [2, 0, 0]);
        assert_eq!(composition(2), [4, 0, 0]);
        // Транспортёры с третьей ступени, установки поддержки — на пятой.
        assert!(composition(3)[1] > 0 && composition(3)[2] == 0);
        assert!(composition(5)[2] > 0);
        for level in 0..=alarm::balance::LEVELS {
            let total: u8 = composition(level).iter().sum();
            assert_eq!(total, (2 * level).min(balance::MAX_VEHICLES));
        }
        let heavy = |level: u8| composition(level)[1] + composition(level)[2];
        assert!(heavy(5) > heavy(4) && heavy(4) > heavy(3));
    }

    #[test]
    fn heaviest_missing_vehicle_comes_first() {
        assert_eq!(next_kind(0, [0, 0, 0]), None);
        assert_eq!(next_kind(1, [0, 0, 0]), Some(ServiceKind::Patrol));
        assert_eq!(next_kind(1, [2, 0, 0]), None);
        // Тревога выросла, пока лёгкие уже на месте: подъезжает подкрепление тяжелее.
        assert_eq!(next_kind(3, [4, 0, 0]), Some(ServiceKind::Transport));
        assert_eq!(next_kind(5, [4, 4, 0]), Some(ServiceKind::Support));
    }

    #[test]
    fn ring_spreads_vehicles_evenly_without_crossing() {
        let e = |i: u32| Entity::from_raw_u32(i).unwrap();
        // Три машины приехали кучей с одной стороны.
        let mut ring = vec![(e(1), 0.1), (e(2), -0.1), (e(3), 0.0)];
        let targets = ring_targets(&mut ring);
        assert_eq!(targets.len(), 3);
        // Порядок по углу сохраняется, точки ровно через треть круга.
        let order: Vec<Entity> = targets.iter().map(|t| t.0).collect();
        assert_eq!(order, vec![e(2), e(3), e(1)]);
        for pair in targets.windows(2) {
            assert!((pair[1].1 - pair[0].1 - TAU / 3.0).abs() < 1e-4);
        }
        // Средняя машина получает точку рядом со своим углом, с опережением.
        let middle = targets[1].1;
        let wrapped = (middle + std::f32::consts::PI).rem_euclid(TAU) - std::f32::consts::PI;
        assert!((wrapped - balance::RING_LEAD).abs() < 0.2, "{wrapped}");
    }

    #[test]
    fn lone_vehicle_heads_to_the_ring_on_its_own_side() {
        let mut ring = vec![(Entity::from_raw_u32(7).unwrap(), 1.0)];
        let targets = ring_targets(&mut ring);
        assert!((targets[0].1 - (1.0 + balance::RING_LEAD)).abs() < 1e-4);
    }
}
