//! Поезда: расписание у ближайшей линии, состав по пути, столкновения, сход с рельсов и остовы.
//!
//! Когда линия ближе 90 м и поездов нет, через 25–40 с у края экрана мигает метка, а за ней
//! въезжает состав: локомотив и 4–7 вагонов трёх видов. Каждый вагон стоит на пути по двум
//! тележкам, поэтому состав изгибается в поворотах; внутри невскрытого туннеля вагон скрыт.
//!
//! Вагон сбивает машины: урон и отброс в сторону от пути. Разрушенное звено впереди или гибель
//! вагона сбрасывает состав с рельсов: вагоны становятся свободными телами с хореографией гармошки,
//! разворотов и переворотов, локомотив и цистерны взрываются, остальные горят. Остановившийся
//! вагон — обугленный остов-препятствие, он хранится до конца забега.

use std::collections::HashMap;

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::buildings::{BuildingDestroyed, DestroyCause};
use crate::camera::GameCamera;
use crate::combat::{Collider, Damage, Died, Health, Team, blast_damage};
use crate::explosion::Explosion;
use crate::models::{ModelShade, Models};
use crate::obstacles::{Obstacle, Shape};
use crate::pickups::{SpawnLoot, crate_weapons};
use crate::rails::TrackDamage;
use crate::railway::{LineId, Lines};
use crate::rover::Rover;
use crate::run::{GameState, RunScoped, RunStarted};
use crate::settlements::Footprint;
use crate::sound_list::Dice;
use crate::terrain::{Terrain, VisibleChunks, chunk_coord_at};
use crate::vehicle::GroundVehicle;
use crate::waves::Threat;
use crate::combat::SurvivalTime;

/// Поезд ставится, если линия ближе этого к луноходу, м.
const SCHEDULE_RANGE: f32 = 90.0;
/// Пауза до поезда, с: от и до.
const SCHEDULE_DELAY: (f32, f32) = (25.0, 40.0);
/// Сколько мигает метка до появления, с.
const WARNING_TIME: f32 = 3.0;
/// На каком расстоянии по пути от лунохода появляется голова, м.
const SPAWN_BEHIND: f32 = 100.0;
/// Поезд убирается, уйдя так далеко от лунохода, м.
const DESPAWN_DISTANCE: f32 = 150.0;
/// Скорость поезда, м/с.
const TRAIN_SPEED: f32 = 18.0;
/// Вагонов за локомотивом: от и до.
const WAGONS: (usize, usize) = (4, 7);
/// Зазор между вагонами, м.
const COUPLING: f32 = 0.8;
/// Ширина вагона, м.
const CAR_WIDTH: f32 = 3.0;
/// Высота пола вагона над путями, м.
const FLOOR: f32 = 0.6;

/// Урон машине от удара вагона и скорость отброса, м/с.
const HIT_DAMAGE: f32 = 60.0;
const KNOCK_SPEED: f32 = 15.0;
/// Урон от кувыркающегося вагона.
const TUMBLE_DAMAGE: f32 = 40.0;

/// Взрыв схода: мощность выше порога замедления, малый радиус.
const DERAIL_BLAST: (f32, f32) = (3.0, 1.55);
/// Взрыв локомотива и цистерны: радиус, мощность.
const LOCO_BLAST: (f32, f32) = (6.0, 1.6);
const TANK_BLAST: (f32, f32) = (7.0, 2.0);
/// Локомотив взрывается через столько после схода, цистерна — не позже, с.
const LOCO_DELAY: f32 = 0.3;
const TANK_DELAY: f32 = 1.2;
/// Сколько горят вагоны, с.
const BURN_TIME: f32 = 6.0;
/// Вес крушения в угрозе.
const WRECK_THREAT: f32 = 6.0;
/// Яркость обугленного остова из модели: текстура почти чёрная.
const CHARRED_SHADE: f32 = 0.1;
/// Модель вагона опущена на столько: колёса на рельсах, а начало вагона — над ними, м.
const MODEL_DROP: f32 = 0.3;

/// Плагин поездов.
pub struct TrainsPlugin;

impl Plugin for TrainsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Schedule>()
            .init_resource::<WreckStore>()
            .add_systems(Startup, create_assets)
            .add_systems(
                Update,
                (
                    reset_trains,
                    run_schedule.run_if(in_state(GameState::Playing)),
                    move_trains,
                    place_cars,
                    hit_vehicles,
                    blast_cars,
                    derail_trains,
                    move_wrecks,
                    tumble_damage,
                    settle_wrecks,
                    stream_wrecks,
                    knock_back,
                    fade_fires,
                )
                    .chain()
                    .after(crate::vehicle::move_ground_vehicles),
            );
    }
}

/// Вид вагона.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarKind {
    Locomotive,
    Passenger,
    Freight,
    Tank,
}

impl CarKind {
    /// Длина и высота кузова, м.
    fn size(self) -> (f32, f32) {
        match self {
            CarKind::Locomotive => (12.0, 3.8),
            CarKind::Passenger => (11.0, 3.4),
            CarKind::Freight => (10.0, 3.2),
            CarKind::Tank => (9.0, 2.9),
        }
    }

    fn health(self) -> f32 {
        match self {
            CarKind::Locomotive => 200.0,
            CarKind::Tank => 80.0,
            _ => 120.0,
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    /// Id модели в `models.json`.
    fn model(self) -> &'static str {
        match self {
            CarKind::Locomotive => "train_loco",
            CarKind::Passenger => "train_wagon_passenger",
            CarKind::Freight => "train_wagon_freight",
            CarKind::Tank => "train_wagon_tank",
        }
    }
}

/// Состав на пути.
#[derive(Component)]
struct Train {
    line: LineId,
    /// Параметр пути головы.
    head: f32,
    /// Направление: 1 — в сторону роста параметра.
    dir: f32,
    /// Вагоны от головы к хвосту.
    cars: Vec<Entity>,
}

/// Вагон на рельсах.
#[derive(Component)]
struct OnTrack {
    train: Entity,
    /// Расстояние от головы до переднего конца вагона по пути, м.
    offset: f32,
}

/// Вагон.
#[derive(Component)]
pub struct Car {
    kind: CarKind,
}

/// Сошедший вагон: свободное тело.
#[derive(Component)]
struct Wreck {
    velocity: Vec3,
    /// Угловая скорость в осях вагона, рад/с.
    spin: Vec3,
    age: f32,
    /// Взорвался ли уже: у локомотива и цистерны.
    exploded: bool,
    /// Пауза до следующего урона от кувыркания, с.
    hit_cooldown: f32,
    still: f32,
    /// Огонь уже зажжён.
    lit: bool,
}

/// Огонь горящего вагона.
#[derive(Component)]
struct Fire {
    remaining: f32,
}

/// Машину отбросило вагоном.
#[derive(Component)]
struct Knockback {
    velocity: Vec3,
    remaining: f32,
}

/// Машина недавно получила удар поезда.
#[derive(Component)]
struct TrainHitCooldown(f32);

/// Остов в хранилище.
#[derive(Clone, Copy)]
struct WreckRecord {
    kind: CarKind,
    transform: Transform,
}

/// Остовы забега по участкам рельефа и те, что сейчас в сцене.
#[derive(Resource, Default)]
struct WreckStore {
    records: HashMap<IVec2, Vec<WreckRecord>>,
    shown: HashMap<IVec2, Vec<Entity>>,
}

impl WreckStore {
    fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

/// Расписание поездов.
#[derive(Resource, Default)]
struct Schedule {
    timer: Option<f32>,
    /// Объявленный поезд: линия, параметр появления, направление, сколько осталось мигать.
    pending: Option<(LineId, f32, f32, f32)>,
}

/// Метка у края экрана перед поездом.
#[derive(Component)]
struct TrainWarning;

#[derive(Resource)]
struct TrainAssets {
    cube: Handle<Mesh>,
    cylinder: Handle<Mesh>,
    bodies: [Handle<StandardMaterial>; 4],
    charred: Handle<StandardMaterial>,
    dark: Handle<StandardMaterial>,
    window: Handle<StandardMaterial>,
}

fn create_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let body = |m: &mut Assets<StandardMaterial>, c: Color| {
        m.add(StandardMaterial {
            base_color: c,
            perceptual_roughness: 0.6,
            ..default()
        })
    };
    commands.insert_resource(TrainAssets {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        cylinder: meshes.add(Cylinder::new(0.5, 1.0)),
        bodies: [
            body(&mut materials, Color::srgb(0.15, 0.32, 0.22)),
            body(&mut materials, Color::srgb(0.75, 0.68, 0.35)),
            body(&mut materials, Color::srgb(0.45, 0.25, 0.15)),
            body(&mut materials, Color::srgb(0.6, 0.62, 0.65)),
        ],
        charred: body(&mut materials, Color::srgb(0.08, 0.07, 0.065)),
        dark: body(&mut materials, Color::srgb(0.1, 0.1, 0.11)),
        window: materials.add(StandardMaterial {
            base_color: Color::srgb(0.9, 0.85, 0.5),
            emissive: LinearRgba::rgb(1.2, 1.0, 0.4),
            ..default()
        }),
    });
}

fn reset_trains(
    mut started: MessageReader<RunStarted>,
    mut schedule: ResMut<Schedule>,
    mut store: ResMut<WreckStore>,
) {
    if started.read().last().is_some() {
        // Сущности прошлого забега удалены через RunScoped.
        *schedule = Schedule::default();
        *store = WreckStore::default();
    }
}

/// Ставит поезд у ближайшей линии: пауза, мигающая метка у края экрана, затем состав.
#[allow(clippy::too_many_arguments)]
fn run_schedule(
    mut commands: Commands,
    time: Res<Time>,
    mut dice: ResMut<Dice>,
    mut schedule: ResMut<Schedule>,
    terrain: Res<Terrain>,
    assets: Res<TrainAssets>,
    models: Res<Models>,
    trains: Query<(), With<Train>>,
    rover: Single<&Transform, With<Rover>>,
    camera: Single<(&Camera, &GlobalTransform), With<GameCamera>>,
    window: Single<&Window>,
    mut warnings: Query<(Entity, &mut Node, &mut Visibility), With<TrainWarning>>,
) {
    let dt = time.delta_secs();
    let p = Vec2::new(rover.translation.x, rover.translation.z);
    if let Some((line, s, dir, left)) = schedule.pending {
        let left = left - dt;
        // Метка у края экрана в сторону, откуда придёт поезд.
        let at = terrain.lines().point(line, s);
        let world = Vec3::new(at.x, terrain.track_level(line, s) + 2.0, at.y);
        let (camera, camera_transform) = *camera;
        let size = Vec2::new(window.width(), window.height());
        let screen = camera
            .world_to_viewport(camera_transform, world)
            .unwrap_or(size * 0.5 + (at - p).normalize_or(Vec2::X) * size.x);
        let center = size * 0.5;
        let d = screen - center;
        let k = ((center.x - 70.0) / d.x.abs().max(1e-3)).min((center.y - 30.0) / d.y.abs().max(1e-3)).min(1.0);
        let edge = center + d * k;
        if warnings.is_empty() {
            commands.spawn((
                TrainWarning,
                RunScoped,
                Text::new("TRAIN!"),
                TextFont {
                    font_size: FontSize::Px(30.0),
                    ..default()
                },
                TextColor(Color::srgb(1.0, 0.85, 0.2)),
                Node {
                    position_type: PositionType::Absolute,
                    ..default()
                },
            ));
        }
        for (_, mut node, mut visibility) in &mut warnings {
            node.left = Val::Px(edge.x - 50.0);
            node.top = Val::Px(edge.y - 15.0);
            *visibility = if (left * 4.0).fract() < 0.6 { Visibility::Inherited } else { Visibility::Hidden };
        }
        if left > 0.0 {
            schedule.pending = Some((line, s, dir, left));
            return;
        }
        schedule.pending = None;
        for (entity, ..) in &warnings {
            commands.entity(entity).despawn();
        }
        spawn_train(&mut commands, &assets, &models, &mut dice, line, s, dir);
        return;
    }
    let near = terrain.lines().nearest(p);
    if !trains.is_empty() || near.distance > SCHEDULE_RANGE {
        return;
    }
    let timer = schedule.timer.get_or_insert_with(|| dice.range(SCHEDULE_DELAY.0, SCHEDULE_DELAY.1));
    *timer -= dt;
    if *timer > 0.0 {
        return;
    }
    schedule.timer = None;
    let dir = if dice.range(0.0, 1.0) < 0.5 { 1.0 } else { -1.0 };
    let s = Lines::param(near.line, p) - dir * SPAWN_BEHIND;
    schedule.pending = Some((near.line, s, dir, WARNING_TIME));
}

/// Создаёт состав: локомотив и вагоны со случайными долями видов.
fn spawn_train(
    commands: &mut Commands,
    assets: &TrainAssets,
    models: &Models,
    dice: &mut Dice,
    line: LineId,
    head: f32,
    dir: f32,
) {
    let wagons = WAGONS.0 + dice.below(WAGONS.1 - WAGONS.0 + 1);
    let weights = [dice.range(0.2, 1.0), dice.range(0.2, 1.0), dice.range(0.2, 1.0)];
    let total: f32 = weights.iter().sum();
    let train = commands.spawn((RunScoped, Transform::default(), Visibility::default())).id();
    let mut cars = Vec::new();
    let mut offset = 0.0;
    for i in 0..=wagons {
        let kind = if i == 0 {
            CarKind::Locomotive
        } else {
            let r = dice.range(0.0, total);
            if r < weights[0] {
                CarKind::Passenger
            } else if r < weights[0] + weights[1] {
                CarKind::Freight
            } else {
                CarKind::Tank
            }
        };
        let car = spawn_car(commands, assets, models, kind, Transform::default());
        commands.entity(car).insert((OnTrack { train, offset }, Health::new(kind.health())));
        cars.push(car);
        offset += kind.size().0 + COUPLING;
    }
    commands.entity(train).insert(Train { line, head, dir, cars });
}

/// Вагон: модель из `models.json`, а без неё — кузов с деталями из кубов. Перёд вагона — +Z.
fn spawn_car(commands: &mut Commands, assets: &TrainAssets, models: &Models, kind: CarKind, transform: Transform) -> Entity {
    let (length, height) = kind.size();
    let body_h = height - FLOOR;
    let mut entity = commands.spawn((
        Car { kind },
        RunScoped,
        transform,
        Visibility::default(),
    ));
    if let Some(look) = models.look(kind.model(), Transform::from_xyz(0.0, -MODEL_DROP, 0.0)) {
        entity.with_child(look);
        return entity.id();
    }
    entity.with_children(|car| {
        let body = match kind {
            // Цистерна — горизонтальный цилиндр на раме.
            CarKind::Tank => (assets.cylinder.clone(), Transform::from_xyz(0.0, FLOOR + body_h * 0.5, 0.0)
                .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))
                .with_scale(Vec3::new(CAR_WIDTH, length - 0.6, body_h))),
            _ => (assets.cube.clone(), Transform::from_xyz(0.0, FLOOR + body_h * 0.5, 0.0)
                .with_scale(Vec3::new(CAR_WIDTH, body_h, length))),
        };
        car.spawn((Mesh3d(body.0), MeshMaterial3d(assets.bodies[kind.index()].clone()), body.1));
        // Рама и тележки.
        car.spawn((
            Mesh3d(assets.cube.clone()),
            MeshMaterial3d(assets.dark.clone()),
            Transform::from_xyz(0.0, FLOOR * 0.5, 0.0).with_scale(Vec3::new(CAR_WIDTH * 0.8, FLOOR, length * 0.9)),
        ));
        match kind {
            CarKind::Locomotive => {
                // Кабина повыше спереди и труба.
                car.spawn((
                    Mesh3d(assets.cube.clone()),
                    MeshMaterial3d(assets.window.clone()),
                    NotShadowCaster,
                    Transform::from_xyz(0.0, height - 0.9, length * 0.5 - 1.5).with_scale(Vec3::new(CAR_WIDTH + 0.05, 0.8, 1.6)),
                ));
                car.spawn((
                    Mesh3d(assets.cylinder.clone()),
                    MeshMaterial3d(assets.dark.clone()),
                    Transform::from_xyz(0.0, height + 0.4, -length * 0.2).with_scale(Vec3::new(0.7, 1.0, 0.7)),
                ));
            }
            CarKind::Passenger => {
                car.spawn((
                    Mesh3d(assets.cube.clone()),
                    MeshMaterial3d(assets.window.clone()),
                    NotShadowCaster,
                    Transform::from_xyz(0.0, FLOOR + body_h * 0.62, 0.0).with_scale(Vec3::new(CAR_WIDTH + 0.05, 0.7, length - 1.5)),
                ));
            }
            _ => {}
        }
    });
    entity.id()
}

/// Двигает составы по пути и убирает ушедшие далеко.
fn move_trains(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    rover: Single<&Transform, With<Rover>>,
    mut trains: Query<(Entity, &mut Train)>,
) {
    let rover_p = Vec2::new(rover.translation.x, rover.translation.z);
    for (entity, mut train) in &mut trains {
        train.head += train.dir * TRAIN_SPEED * time.delta_secs();
        let rover_s = Lines::param(train.line, rover_p);
        // Ушёл вперёд за луноход дальше предела — хвост тоже далеко.
        let ahead = (train.head - rover_s) * train.dir;
        let tail = ahead - 90.0;
        let far = tail > DESPAWN_DISTANCE || terrain.lines().point(train.line, train.head).distance(rover_p) > DESPAWN_DISTANCE + 200.0;
        if far || train.cars.is_empty() {
            for &car in &train.cars {
                commands.entity(car).try_despawn();
            }
            commands.entity(entity).despawn();
        }
    }
}

/// Ставит вагоны на путь по двум тележкам и прячет их внутри невскрытого туннеля.
fn place_cars(
    terrain: Res<Terrain>,
    damage: Res<TrackDamage>,
    trains: Query<&Train>,
    mut cars: Query<(&OnTrack, &Car, &mut Transform, &mut Visibility)>,
) {
    for (on, car, mut transform, mut visibility) in &mut cars {
        let Ok(train) = trains.get(on.train) else {
            continue;
        };
        let (length, _) = car.kind.size();
        let front = train.head - train.dir * on.offset;
        let back = front - train.dir * length;
        let bogie = |s: f32| {
            let p = terrain.lines().point(train.line, s);
            Vec3::new(p.x, terrain.track_level(train.line, s) + 0.3, p.y)
        };
        let f = bogie(front - train.dir * 1.5);
        let b = bogie(back + train.dir * 1.5);
        let forward = (f - b).normalize_or(Vec3::Z);
        transform.translation = (f + b) * 0.5;
        transform.rotation = Quat::from_rotation_arc(Vec3::Z, forward);
        let mid = (front + back) * 0.5;
        *visibility = if damage.in_tunnel(&terrain, train.line, mid) { Visibility::Hidden } else { Visibility::Inherited };
    }
}

/// Прямоугольник вагона в плане.
fn car_footprint(transform: &Transform, kind: CarKind) -> Footprint {
    let forward = transform.rotation * Vec3::Z;
    Footprint {
        center: Vec2::new(transform.translation.x, transform.translation.z),
        yaw: forward.x.atan2(forward.z),
        half: Vec2::new(CAR_WIDTH * 0.5, kind.size().0 * 0.5),
    }
}

/// Вагоны на ходу сбивают машины и сносят мелкие объекты на пути.
#[allow(clippy::type_complexity)]
fn hit_vehicles(
    mut commands: Commands,
    cars: Query<(&Transform, &Car, &Visibility), With<OnTrack>>,
    vehicles: Query<
        (Entity, &Transform, &Collider, Option<&TrainHitCooldown>),
        (Or<(With<Rover>, With<GroundVehicle>)>, Without<Car>),
    >,
    obstacles: Query<(Entity, &Obstacle, &Health), Without<Car>>,
    mut damage: MessageWriter<Damage>,
) {
    for (transform, car, visibility) in &cars {
        if *visibility == Visibility::Hidden {
            continue;
        }
        let footprint = car_footprint(transform, car.kind);
        let shape = Shape::Rect(footprint);
        for (entity, vehicle, collider, cooldown) in &vehicles {
            let p = Vec2::new(vehicle.translation.x, vehicle.translation.z);
            if cooldown.is_some() || footprint.distance(p) > collider.radius {
                continue;
            }
            let local = footprint.to_local(p);
            let side = footprint.to_world(Vec2::new(local.x.signum(), 0.0)) - footprint.center;
            let away = side.normalize_or(Vec2::X);
            commands.entity(entity).try_insert((
                Knockback {
                    velocity: Vec3::new(away.x, 0.0, away.y) * KNOCK_SPEED,
                    remaining: 0.6,
                },
                TrainHitCooldown(1.0),
            ));
            damage.write(Damage {
                target: entity,
                amount: HIT_DAMAGE,
                source_team: Team::Enemy,
            });
        }
        for (entity, obstacle, health) in &obstacles {
            if health.is_dead() {
                continue;
            }
            let (center, r) = obstacle.shape.bounds();
            if footprint.distance(center) <= r * 0.5 || shape.contains(center) {
                commands.entity(entity).remove::<Obstacle>();
                damage.write(Damage {
                    target: entity,
                    amount: health.current + 1.0,
                    source_team: Team::Player,
                });
            }
        }
    }
}

/// Взрывы ранят вагоны: у вагонов нет команды, поэтому урон считается здесь, как у домов.
fn blast_cars(
    mut explosions: MessageReader<Explosion>,
    cars: Query<(Entity, &GlobalTransform, &Health), With<Car>>,
    mut damage: MessageWriter<Damage>,
) {
    for e in explosions.read() {
        for (entity, transform, health) in &cars {
            if health.is_dead() {
                continue;
            }
            let amount = blast_damage(e.power, e.radius, transform.translation().distance(e.position), false) * 2.0;
            if amount > 0.0 {
                damage.write(Damage {
                    target: entity,
                    amount,
                    source_team: e.source_team,
                });
            }
        }
    }
}

/// Сход с рельсов: разрушенное звено перед головой — весь состав; гибель вагона — он и все позади.
#[allow(clippy::too_many_arguments)]
fn derail_trains(
    mut commands: Commands,
    terrain: Res<Terrain>,
    damage: Res<TrackDamage>,
    survival: Res<SurvivalTime>,
    mut dice: ResMut<Dice>,
    mut threat: ResMut<Threat>,
    mut died: MessageReader<Died>,
    mut trains: Query<(Entity, &mut Train)>,
    cars: Query<(&OnTrack, &Car, &Transform)>,
    dead_cars: Query<(&Car, &GlobalTransform)>,
    mut explosions: MessageWriter<Explosion>,
    mut loot: MessageWriter<SpawnLoot>,
    mut destroyed: MessageWriter<BuildingDestroyed>,
) {
    let mut derails: Vec<(Entity, usize)> = Vec::new();
    for death in died.read() {
        // Погибший вагон: груз и тревога.
        if let Ok((car, transform)) = dead_cars.get(death.entity) {
            let at = transform.translation();
            match car.kind {
                CarKind::Freight => {
                    let weapons = crate_weapons(survival.0);
                    for k in 0..1 + dice.below(2) {
                        let weapon = weapons[dice.below(weapons.len())];
                        loot.write(SpawnLoot {
                            position: at + Vec3::new(2.0 + k as f32 * 1.5, 1.0, 1.5),
                            weapon,
                        });
                    }
                }
                CarKind::Passenger => {
                    destroyed.write(BuildingDestroyed {
                        position: at,
                        cause: DestroyCause::Player,
                    });
                }
                _ => {}
            }
        }
        if let Ok((on, ..)) = cars.get(death.entity)
            && let Ok((_, train)) = trains.get(on.train)
            && let Some(index) = train.cars.iter().position(|&c| c == death.entity)
        {
            derails.push((on.train, index));
        }
    }
    for (entity, train) in &trains {
        let ahead = train.head + train.dir * TRAIN_SPEED * 0.2;
        let blocked = damage.is_broken(train.line, ahead) && !damage.in_tunnel(&terrain, train.line, ahead);
        if blocked {
            derails.push((entity, 0));
        }
    }
    for (entity, from) in derails {
        let Ok((_, mut train)) = trains.get_mut(entity) else {
            continue;
        };
        if from >= train.cars.len() {
            continue;
        }
        let fallen: Vec<Entity> = train.cars.split_off(from);
        let tangent = terrain.lines().tangent(train.line, train.head) * train.dir;
        let forward = Vec3::new(tangent.x, 0.0, tangent.y);
        let side = forward.cross(Vec3::Y);
        let mut first = true;
        for (j, car_entity) in fallen.into_iter().enumerate() {
            let Ok((_, car, transform)) = cars.get(car_entity) else {
                continue;
            };
            if first {
                first = false;
                explosions.write(Explosion {
                    position: transform.translation,
                    radius: DERAIL_BLAST.0,
                    power: DERAIL_BLAST.1,
                    source_team: Team::Player,
                });
            }
            // Хореография: гармошка — бок чередуется, разворот, у каждого второго — переворот.
            let sign = if j % 2 == 0 { 1.0 } else { -1.0 };
            let speed = TRAIN_SPEED * (1.0 - 0.04 * j as f32).max(0.5);
            let velocity = forward * speed + side * sign * dice.range(2.0, 5.0) + Vec3::Y * dice.range(1.5, 4.0);
            let roll = if j % 2 == 1 { sign * dice.range(1.8, 3.4) } else { sign * dice.range(0.0, 0.6) };
            let spin = Vec3::new(dice.range(-0.4, 0.4), sign * dice.range(1.0, 3.0), roll);
            commands
                .entity(car_entity)
                .remove::<OnTrack>()
                .insert((
                    Wreck {
                        velocity,
                        spin,
                        age: 0.0,
                        exploded: !matches!(car.kind, CarKind::Locomotive | CarKind::Tank),
                        hit_cooldown: 0.0,
                        still: 0.0,
                        lit: false,
                    },
                    Visibility::Inherited,
                ));
        }
        threat.add_destruction(WRECK_THREAT);
    }
}

/// Нижние углы кузова в мире.
fn corners(transform: &Transform, kind: CarKind) -> [Vec3; 8] {
    let (length, height) = kind.size();
    let (hx, hz) = (CAR_WIDTH * 0.5, length * 0.5);
    let mut out = [Vec3::ZERO; 8];
    let mut i = 0;
    for y in [0.0, height] {
        for x in [-hx, hx] {
            for z in [-hz, hz] {
                out[i] = transform.transform_point(Vec3::new(x, y, z));
                i += 1;
            }
        }
    }
    out
}

/// Свободные вагоны: гравитация, трение о рельеф по углам кузова, расталкивание, взрывы.
#[allow(clippy::too_many_arguments)]
fn move_wrecks(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut wrecks: Query<(Entity, &Car, &mut Transform, &mut Wreck)>,
    mut explosions: MessageWriter<Explosion>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for (entity, car, mut transform, mut wreck) in &mut wrecks {
        wreck.age += dt;
        wreck.velocity.y -= 25.0 * dt;
        transform.translation += wreck.velocity * dt;
        let spin = wreck.spin;
        transform.rotation = (transform.rotation * Quat::from_scaled_axis(spin * dt)).normalize();
        // Контакт: самый низкий угол кузова не уходит под рельеф.
        let lowest = corners(&transform, car.kind)
            .iter()
            .map(|c| c.y - terrain.height_at(c.x, c.z))
            .fold(f32::MAX, f32::min);
        if lowest < 0.0 {
            transform.translation.y -= lowest;
            let impact = -wreck.velocity.y;
            if impact > 0.0 {
                wreck.velocity.y = impact * 0.15;
            }
            let friction = (1.0 - 2.2 * dt).max(0.0);
            wreck.velocity.x *= friction;
            wreck.velocity.z *= friction;
            wreck.spin *= (1.0 - 2.5 * dt).max(0.0);
            // Цистерна рвётся при сильном ударе о землю.
            if car.kind == CarKind::Tank && impact > 6.0 && !wreck.exploded {
                wreck.age = wreck.age.max(TANK_DELAY);
            }
        }
        let due = match car.kind {
            CarKind::Locomotive => LOCO_DELAY,
            CarKind::Tank => TANK_DELAY,
            _ => f32::MAX,
        };
        if !wreck.exploded && wreck.age >= due {
            wreck.exploded = true;
            let (radius, power) = if car.kind == CarKind::Tank { TANK_BLAST } else { LOCO_BLAST };
            explosions.write(Explosion {
                position: transform.translation,
                radius,
                power,
                source_team: Team::Player,
            });
        }
        // Вагон загорается сразу после схода.
        if !wreck.lit {
            wreck.lit = true;
            commands.entity(entity).with_child((
                Fire { remaining: BURN_TIME },
                PointLight {
                    color: Color::srgb(1.0, 0.5, 0.15),
                    intensity: 600_000.0,
                    range: 14.0,
                    shadow_maps_enabled: false,
                    ..default()
                },
                Transform::from_xyz(0.0, car.kind.size().1 + 0.5, 0.0),
            ));
        }
    }
    // Расталкивание вагонов и передача скорости налетающего.
    let mut pairs = wrecks.iter_combinations_mut();
    while let Some([(_, a_car, mut a, mut aw), (_, b_car, mut b, mut bw)]) = pairs.fetch_next() {
        let r = (a_car.kind.size().0 + b_car.kind.size().0) * 0.25 + CAR_WIDTH * 0.5;
        let mut d = a.translation - b.translation;
        d.y = 0.0;
        let dist = d.length();
        if dist >= r || dist < 1e-3 {
            continue;
        }
        let n = d / dist;
        let push = n * (r - dist) * 0.5;
        a.translation += push;
        b.translation -= push;
        let rel = (aw.velocity - bw.velocity).dot(n);
        if rel < 0.0 {
            let impulse = n * rel * 0.6;
            aw.velocity -= impulse;
            bw.velocity += impulse;
        }
    }
}

/// Кувыркающийся вагон ранит машины, через которые проходит.
#[allow(clippy::type_complexity)]
fn tumble_damage(
    time: Res<Time>,
    mut wrecks: Query<(&Transform, &Car, &mut Wreck)>,
    vehicles: Query<(Entity, &Transform, &Collider), (Or<(With<Rover>, With<GroundVehicle>)>, Without<Car>)>,
    mut damage: MessageWriter<Damage>,
) {
    for (transform, car, mut wreck) in &mut wrecks {
        wreck.hit_cooldown -= time.delta_secs();
        if wreck.hit_cooldown > 0.0 || wreck.velocity.length() < 4.0 {
            continue;
        }
        let footprint = car_footprint(transform, car.kind);
        for (entity, vehicle, collider) in &vehicles {
            let p = Vec2::new(vehicle.translation.x, vehicle.translation.z);
            if footprint.distance(p) <= collider.radius {
                wreck.hit_cooldown = 0.5;
                damage.write(Damage {
                    target: entity,
                    amount: TUMBLE_DAMAGE,
                    source_team: Team::Enemy,
                });
            }
        }
    }
}

/// Остановившийся вагон становится обугленным остовом-препятствием и записывается в хранилище.
#[allow(clippy::too_many_arguments)]
fn settle_wrecks(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<TrainAssets>,
    mut store: ResMut<WreckStore>,
    mut wrecks: Query<(Entity, &Car, &Transform, &mut Wreck, &Children)>,
    mut materials: Query<&mut MeshMaterial3d<StandardMaterial>>,
) {
    for (entity, car, transform, mut wreck, children) in &mut wrecks {
        if wreck.velocity.length() < 0.6 && wreck.spin.length() < 0.4 {
            wreck.still += time.delta_secs();
        } else {
            wreck.still = 0.0;
        }
        if wreck.still < 0.5 || wreck.age < 1.5 || !wreck.exploded {
            continue;
        }
        let coord = chunk_coord_at(transform.translation.x, transform.translation.z);
        store.records.entry(coord).or_default().push(WreckRecord {
            kind: car.kind,
            transform: *transform,
        });
        store.shown.entry(coord).or_default().push(entity);
        for child in children.iter() {
            if let Ok(mut material) = materials.get_mut(child)
                && material.0 != assets.window
            {
                material.0 = assets.charred.clone();
            }
        }
        let top = corners(transform, car.kind).iter().map(|c| c.y).fold(f32::MIN, f32::max);
        commands
            .entity(entity)
            .remove::<(Wreck, Health)>()
            .insert(ModelShade(CHARRED_SHADE))
            .insert(Obstacle {
                shape: Shape::Rect(car_footprint(transform, car.kind)),
                top,
            });
    }
}

/// Остовы появляются и исчезают вместе с участками рельефа.
fn stream_wrecks(
    mut commands: Commands,
    visible: Res<VisibleChunks>,
    assets: Res<TrainAssets>,
    models: Res<Models>,
    mut store: ResMut<WreckStore>,
) {
    if !visible.is_changed() || store.is_empty() {
        return;
    }
    let store = &mut *store;
    store.shown.retain(|&coord, entities| {
        let keep = visible.entity(coord).is_some();
        if !keep {
            for &e in entities.iter() {
                commands.entity(e).try_despawn();
            }
        }
        keep
    });
    for (&coord, records) in &store.records {
        if store.shown.contains_key(&coord) || visible.entity(coord).is_none() {
            continue;
        }
        let mut entities = Vec::new();
        for record in records {
            let e = spawn_car(&mut commands, &assets, &models, record.kind, record.transform);
            let top = corners(&record.transform, record.kind).iter().map(|c| c.y).fold(f32::MIN, f32::max);
            commands.entity(e).insert(ModelShade(CHARRED_SHADE)).insert(Obstacle {
                shape: Shape::Rect(car_footprint(&record.transform, record.kind)),
                top,
            });
            entities.push(e);
        }
        store.shown.insert(coord, entities);
    }
}

/// Отброшенная поездом машина летит вбок и тормозит; управление снова её через полсекунды.
fn knock_back(
    mut commands: Commands,
    time: Res<Time>,
    mut knocked: Query<(Entity, &mut Transform, &mut Knockback)>,
    mut cooldowns: Query<(Entity, &mut TrainHitCooldown)>,
) {
    let dt = time.delta_secs();
    for (entity, mut transform, mut knock) in &mut knocked {
        transform.translation += knock.velocity * dt;
        knock.velocity *= (1.0 - 3.0 * dt).max(0.0);
        knock.remaining -= dt;
        if knock.remaining <= 0.0 {
            commands.entity(entity).remove::<Knockback>();
        }
    }
    for (entity, mut cooldown) in &mut cooldowns {
        cooldown.0 -= dt;
        if cooldown.0 <= 0.0 {
            commands.entity(entity).remove::<TrainHitCooldown>();
        }
    }
}

/// Огонь вагона мерцает и гаснет.
fn fade_fires(mut commands: Commands, time: Res<Time>, mut fires: Query<(Entity, &mut Fire, &mut PointLight)>) {
    for (entity, mut fire, mut light) in &mut fires {
        fire.remaining -= time.delta_secs();
        if fire.remaining <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let flicker = 0.75 + 0.25 * (time.elapsed_secs() * 23.0 + entity.to_bits() as f32).sin();
        light.intensity = 600_000.0 * flicker * (fire.remaining / BURN_TIME).min(1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn после_сброса_остовов_нет() {
        let mut app = App::new();
        app.add_message::<RunStarted>()
            .init_resource::<Schedule>()
            .init_resource::<WreckStore>()
            .add_systems(Update, reset_trains);
        app.world_mut().resource_mut::<WreckStore>().records.insert(
            IVec2::ZERO,
            vec![WreckRecord {
                kind: CarKind::Tank,
                transform: Transform::default(),
            }],
        );
        app.world_mut().write_message(RunStarted { seed: 1 });
        app.update();
        assert!(app.world().resource::<WreckStore>().is_empty());
    }
}
