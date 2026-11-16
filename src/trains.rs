//! Поезда: расписание у ближайшей линии, состав по пути, столкновения, сход с рельсов и остовы.
//!
//! Когда линия ближе 90 м и поездов нет, через 25–40 с у края экрана мигает метка, а за ней
//! въезжает состав: локомотив и 4–7 вагонов трёх видов. Каждый вагон стоит на пути по двум
//! тележкам, поэтому состав изгибается в поворотах; внутри невскрытого туннеля вагон скрыт.
//!
//! Вагон сбивает машины: урон и отброс в сторону от пути. Разрушенное звено впереди или гибель
//! вагона сбрасывает состав с рельсов: вагоны становятся свободными телами с хореографией гармошки,
//! разворотов и переворотов, локомотив и цистерны взрываются по очереди, все вагоны горят огнём
//! с дымом, а погасший остов ещё дымит.
//! Остановившийся вагон — обугленный остов-препятствие, он хранится до конца забега; остовов не
//! больше [`MAX_WRECKS`], старые уходят первыми. Вагоны внутри невскрытого туннеля при сходе
//! остаются в горе и исчезают.
//!
//! Вагон можно уничтожить чем угодно: у него есть здоровье, коллайдер и команда `Neutral`.
//! Уничтоженный или сошедший грузовой вагон роняет ящики, пассажирский поднимает тревогу, если
//! его уничтожил игрок. Взрывы крушения записываются на того, кто его вызвал: порвал путь или
//! добил вагон.
//!
//! Наружу поезда сообщают [`TrainIncoming`] и [`TrainDerailed`].

use std::collections::HashMap;

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::alarm::Alarm;
use crate::camera::GameCamera;
use crate::combat::{Collider, Damage, Died, Health, ROVER_MAX_HEALTH, Team, blast_damage};
use crate::explosion::Explosion;
use crate::frame::{GameSet, RunMessages};
use crate::debris::SpawnDebris;
use crate::models::{ModelBank, ModelShade, Models};
use crate::obstacles::{Obstacle, Shape};
use crate::pickups::{SpawnLoot, crate_weapons};
use crate::rails::{LINK_LENGTH, TrackDamage};
use crate::residents::Resident;
use crate::railway::{LineId, Lines};
use crate::rng::GameRng;
use crate::rover::{Rover, RoverMotion};
use crate::run::{RunScoped, WorldRebuilt};
use crate::settlements::Footprint;
use crate::sound_list::Dice;
use crate::terrain::{self, Terrain, VisibleChunks, chunk_coord_at};
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

/// Урон машине от удара вагона: пятая часть здоровья лунохода (было 60 при здоровье 300).
const HIT_DAMAGE: f32 = ROVER_MAX_HEALTH * 0.2;
/// Скорость отброса, м/с.
const KNOCK_SPEED: f32 = 15.0;
/// Сошедший вагон сбивает жителя, пока летит быстрее этого, м/с.
const RESIDENT_HIT_SPEED: f32 = 4.0;
/// Радиус жителя для удара вагоном, м.
const RESIDENT_RADIUS: f32 = 0.5;
/// Урон от кувыркающегося вагона: 2/15 здоровья лунохода (было 40 при здоровье 300).
const TUMBLE_DAMAGE: f32 = ROVER_MAX_HEALTH * 2.0 / 15.0;
/// Радиус коллайдера вагона для пуль и лазера: половина ширины и четверть длины, м. Круг не
/// повторяет кузов: середину ловит целиком, концы — частично, рядом с бортом немного лишнего.
fn car_collider(kind: CarKind) -> f32 {
    CAR_WIDTH * 0.5 + kind.size().0 * 0.25
}

/// Взрыв схода: мощность выше порога замедления, малый радиус.
const DERAIL_BLAST: (f32, f32) = (3.0, 1.55);
/// Взрыв локомотива и цистерны: радиус, мощность.
const LOCO_BLAST: (f32, f32) = (6.0, 1.6);
const TANK_BLAST: (f32, f32) = (7.0, 2.0);
/// Локомотив взрывается через столько после схода, цистерна — не позже, с.
const LOCO_DELAY: f32 = 0.3;
const TANK_DELAY: f32 = 1.2;
/// Каждая следующая цистерна состава рвётся позже предыдущей на столько, с.
const TANK_STAGGER: f32 = 0.35;
/// Взрывы вагонов идут не чаще, чем через столько, с: семь цистерн не рвутся в одном кадре.
const BLAST_GAP: f32 = 0.25;
/// Сколько горят вагоны, с.
const BURN_TIME: f32 = 6.0;
/// Сколько вагон дымит после того, как огонь погас, с.
const SMOKE_TIME: f32 = 10.0;
/// Размер огня горящего вагона, м: пламя модуля огня, свет — из его общего пула.
const CAR_FIRE_SIZE: f32 = 2.4;
/// Вес крушения в угрозе: только крушения от игрока.
const WRECK_THREAT: f32 = 6.0;
/// Яркость обугленного остова из модели: текстура почти чёрная.
const CHARRED_SHADE: f32 = 0.1;
/// Модель вагона опущена на столько: колёса на рельсах, а начало вагона — над ними, м.
const MODEL_DROP: f32 = 0.3;
/// Очков тревоги за пассажирский вагон, уничтоженный игроком: как за дом.
const PASSENGER_ALARM: f32 = 6.0;
/// Больше остовов забег не хранит: старые исчезают первыми.
const MAX_WRECKS: usize = 40;
/// Угол кузова ушёл под рельеф глубже этого за кадр — это стена (склон горы у портала), а не
/// земля: вагон в неё упирается, а не взлетает на вершину, м.
const WALL_STEP: f32 = 1.5;
/// Поезд ставится, только если путь свободен от разрывов и остовов на столько вперёд от лунохода
/// по ходу поезда: вся дорога до его ухода, м.
const CLEAR_AHEAD: f32 = 260.0;

/// Плагин поездов.
pub struct TrainsPlugin;

impl Plugin for TrainsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Schedule>()
            .init_resource::<WreckStore>()
            .add_run_message::<TrainIncoming>()
            .add_run_message::<TrainDerailed>()
            .add_systems(Startup, create_assets)
            .add_systems(
                Update,
                (
                    reset_trains.in_set(GameSet::Reset),
                    (
                        run_schedule,
                        move_trains,
                        place_cars,
                        hit_vehicles,
                        run_over_residents,
                        move_wrecks,
                        tumble_damage,
                        settle_wrecks,
                        knock_back,
                    )
                        .chain()
                        .after(crate::vehicle::move_ground_vehicles)
                        .in_set(GameSet::Simulation),
                    blast_cars.in_set(GameSet::Damage),
                    derail_trains.in_set(GameSet::Death),
                    stream_wrecks.after(terrain::stream_chunks).in_set(GameSet::Streaming),
                    burn_fires.in_set(GameSet::Presentation),
                    (find_wheels, roll_wheels, turn_wheels).chain().in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Объявлен поезд: у края экрана мигает метка. `position` — где появится голова.
#[derive(Message, Debug, Clone, Copy)]
pub struct TrainIncoming {
    pub position: Vec3,
}

/// Поезд сошёл с рельсов. `position` — первый видимый сошедший вагон, `cause` — кто вызвал сход.
#[derive(Message, Debug, Clone, Copy)]
pub struct TrainDerailed {
    pub position: Vec3,
    pub cause: Team,
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
    /// Груз уже выпал, тревога уже поднята: вагон уничтожен один раз.
    spent: bool,
}

/// Сошедший вагон: свободное тело.
#[derive(Component)]
struct Wreck {
    velocity: Vec3,
    /// Угловая скорость в осях вагона, рад/с.
    spin: Vec3,
    age: f32,
    /// Когда взорваться, с от схода; у вагонов, которые не взрываются, — бесконечность.
    due: f32,
    /// Взорвался ли уже: у локомотива и цистерны.
    exploded: bool,
    /// Кто вызвал сход: на него записываются взрывы вагона.
    cause: Team,
    /// Пауза до следующего урона от кувыркания, с.
    hit_cooldown: f32,
    still: f32,
    /// Огонь уже зажжён.
    lit: bool,
}

/// Носитель огня горящего вагона: пламя, свет и дым после огня ставит модуль огня.
#[derive(Component)]
struct CarFire {
    /// Сколько горит, с.
    age: f32,
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
    /// Порядковый номер: чем меньше, тем старше.
    serial: u64,
    kind: CarKind,
    transform: Transform,
}

/// Остовы забега по участкам рельефа и те, что сейчас в сцене: номер записи и сущность.
#[derive(Resource, Default)]
struct WreckStore {
    records: HashMap<IVec2, Vec<WreckRecord>>,
    shown: HashMap<IVec2, Vec<(u64, Entity)>>,
    next: u64,
}

impl WreckStore {
    fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    fn len(&self) -> usize {
        self.records.values().map(Vec::len).sum()
    }

    /// Записывает остов, показанный сущностью `entity`. Сверх [`MAX_WRECKS`] самый старый
    /// забывается; его сущность, если она в сцене, возвращается для удаления.
    fn push(&mut self, coord: IVec2, kind: CarKind, transform: Transform, entity: Entity) -> Option<Entity> {
        let serial = self.next;
        self.next += 1;
        self.records.entry(coord).or_default().push(WreckRecord { serial, kind, transform });
        self.shown.entry(coord).or_default().push((serial, entity));
        if self.len() <= MAX_WRECKS {
            return None;
        }
        let (&oldest_coord, oldest) = self
            .records
            .iter()
            .filter_map(|(c, list)| list.iter().map(|r| r.serial).min().map(|s| (c, s)))
            .min_by_key(|&(_, s)| s)?;
        let list = self.records.get_mut(&oldest_coord)?;
        list.retain(|r| r.serial != oldest);
        if list.is_empty() {
            self.records.remove(&oldest_coord);
        }
        let shown = self.shown.get_mut(&oldest_coord)?;
        let at = shown.iter().position(|&(s, _)| s == oldest)?;
        Some(shown.remove(at).1)
    }

    /// Лежит ли остов на оси линии между параметрами `from` и `to`.
    fn on_line(&self, terrain: &Terrain, line: LineId, from: f32, to: f32) -> bool {
        let (lo, hi) = (from.min(to), from.max(to));
        self.records.values().flatten().any(|r| {
            let p = Vec2::new(r.transform.translation.x, r.transform.translation.z);
            let s = Lines::param(line, p);
            (lo..=hi).contains(&s) && terrain.lines().point(line, s).distance(p) < CAR_WIDTH + 2.0
        })
    }
}

/// Свободен ли путь между параметрами `from` и `to`: нет разрывов и остовов на оси.
fn line_clear(terrain: &Terrain, damage: &TrackDamage, store: &WreckStore, line: LineId, from: f32, to: f32) -> bool {
    let (lo, hi) = (from.min(to), from.max(to));
    let mut s = lo;
    while s <= hi + LINK_LENGTH {
        if damage.is_broken(line, s) {
            return false;
        }
        s += LINK_LENGTH;
    }
    !store.on_line(terrain, line, lo, hi)
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

/// Цвета кузова запасного облика по [`CarKind::index`].
const BODY_COLORS: [Color; 4] = [
    Color::srgb(0.15, 0.32, 0.22),
    Color::srgb(0.75, 0.68, 0.35),
    Color::srgb(0.45, 0.25, 0.15),
    Color::srgb(0.6, 0.62, 0.65),
];

/// Обломки кузова: листы и куски в цветах модели вагона (без модели — в цвете запасного облика)
/// разлетаются из его объёма. `burning` — сколько из них горит.
fn burst_car(
    debris: &mut MessageWriter<SpawnDebris>,
    bank: &ModelBank,
    kind: CarKind,
    transform: &Transform,
    count: u32,
    speed: f32,
    burning: u32,
) {
    let colors = bank.colors(kind.model());
    let (length, height) = kind.size();
    let seed = (transform.translation.x * 13.7 + transform.translation.z * 7.3).abs() as u32;
    for i in 0..count {
        let h = seed.wrapping_add(i).wrapping_mul(2654435761);
        let k = (h % 1000) as f32 / 1000.0;
        let m = ((h / 1000) % 1000) as f32 / 1000.0;
        // Точка вдоль кузова и сторона, в которую летит кусок.
        let local = Vec3::new((k - 0.5) * length * 0.8, height * (0.3 + 0.6 * m), 0.0);
        let side = if i % 2 == 0 { 1.0 } else { -1.0 };
        let out = transform.rotation * Vec3::new((k - 0.5) * 0.6, 0.0, side).normalize();
        let color = colors
            .as_ref()
            .map_or(BODY_COLORS[kind.index()], |c| c.any(i as usize + seed as usize));
        debris.write(SpawnDebris {
            position: transform.translation + transform.rotation * local,
            velocity: out * speed * (0.6 + 0.6 * m) + Vec3::Y * speed * (0.8 + 0.6 * k),
            // Листы обшивки: тонкие и широкие, через один — кусок потолще.
            size: if i % 3 == 0 {
                Vec3::new(0.6 + 0.4 * k, 0.4 + 0.3 * m, 0.5)
            } else {
                Vec3::new(1.0 + 0.8 * k, 0.08, 0.6 + 0.5 * m)
            },
            color,
            burning: i < burning,
        });
    }
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
        bodies: BODY_COLORS.map(|c| body(&mut materials, c)),
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
    mut commands: Commands,
    mut rebuilt: MessageReader<WorldRebuilt>,
    mut schedule: ResMut<Schedule>,
    mut store: ResMut<WreckStore>,
    knocked: Query<Entity, Or<(With<Knockback>, With<TrainHitCooldown>)>>,
) {
    if rebuilt.read().last().is_some() {
        // Сущности прошлого забега удалены через RunScoped.
        *schedule = Schedule::default();
        *store = WreckStore::default();
        // Луноход переживает забег: отброс от поезда не должен перейти в новый.
        for entity in &knocked {
            commands.entity(entity).try_remove::<(Knockback, TrainHitCooldown)>();
        }
    }
}

/// Ставит поезд у ближайшей линии: пауза, мигающая метка у края экрана, затем состав.
///
/// Поезд идёт только по свободному пути: если вперёд и назад путь порван или на нём остовы,
/// поезда на этой линии нет, пока луноход не уедет к другой. Путь, порванный после объявления
/// поезда, сбрасывает его с рельсов — так и задумано.
#[allow(clippy::too_many_arguments)]
fn run_schedule(
    mut commands: Commands,
    time: Res<Time>,
    mut dice: ResMut<GameRng>,
    mut schedule: ResMut<Schedule>,
    terrain: Res<Terrain>,
    damage: Res<TrackDamage>,
    store: Res<WreckStore>,
    assets: Res<TrainAssets>,
    mut models: Models,
    mut incoming: MessageWriter<TrainIncoming>,
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
        spawn_train(&mut commands, &assets, &mut models, &mut dice, line, s, dir);
        return;
    }
    let near = terrain.lines().nearest(p);
    if !trains.is_empty() || near.distance > SCHEDULE_RANGE {
        return;
    }
    let timer = schedule.timer.get_or_insert_with(|| {
        // Отладочная сцена поезда (`src/debug_shot.rs`): состав выходит сразу.
        if std::env::var("SPACE_GUARD_SCENE").is_ok_and(|s| s == "train") {
            1.0
        } else {
            dice.range(SCHEDULE_DELAY.0, SCHEDULE_DELAY.1)
        }
    });
    *timer -= dt;
    if *timer > 0.0 {
        return;
    }
    schedule.timer = None;
    let first = if dice.range(0.0, 1.0) < 0.5 { 1.0 } else { -1.0 };
    let rover_s = Lines::param(near.line, p);
    let Some(dir) = [first, -first]
        .into_iter()
        .find(|&dir| line_clear(&terrain, &damage, &store, near.line, rover_s - dir * SPAWN_BEHIND, rover_s + dir * CLEAR_AHEAD))
    else {
        // Путь порван в обе стороны: следующая попытка — после новой паузы.
        return;
    };
    let s = rover_s - dir * SPAWN_BEHIND;
    schedule.pending = Some((near.line, s, dir, WARNING_TIME));
    let at = terrain.lines().point(near.line, s);
    incoming.write(TrainIncoming {
        position: Vec3::new(at.x, terrain.track_level(near.line, s), at.y),
    });
}

/// Создаёт состав: локомотив и вагоны со случайными долями видов.
fn spawn_train(
    commands: &mut Commands,
    assets: &TrainAssets,
    models: &mut Models,
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
        commands.entity(car).insert((
            OnTrack { train, offset },
            Health::new(kind.health()),
            Team::Neutral,
            Collider {
                radius: car_collider(kind),
            },
        ));
        cars.push(car);
        offset += kind.size().0 + COUPLING;
    }
    commands.entity(train).insert(Train { line, head, dir, cars });
}

/// Вагон: модель из `models.json`, запасной облик — кузов с деталями из кубов. Перёд вагона — +Z.
fn spawn_car(commands: &mut Commands, assets: &TrainAssets, models: &mut Models, kind: CarKind, transform: Transform) -> Entity {
    let (length, height) = kind.size();
    let body_h = height - FLOOR;
    let mut entity = commands.spawn((
        Car { kind, spent: false },
        WheelRoll::default(),
        RunScoped,
        transform,
        Visibility::default(),
    ));
    entity.with_children(|parent| {
        models.spawn_visual(parent, kind.model(), Transform::from_xyz(0.0, -MODEL_DROP, 0.0), |car| {
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

/// Концы вагона по пути: параметры переднего и заднего конца.
fn car_span(train: &Train, on: &OnTrack, kind: CarKind) -> (f32, f32) {
    let front = train.head - train.dir * on.offset;
    (front, front - train.dir * kind.size().0)
}

/// Ставит вагоны на путь по двум тележкам и прячет их, когда они целиком в невскрытом туннеле.
/// Вагон, въехавший в туннель наполовину, виден: часть в горе закрывает сам склон.
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
        let (front, back) = car_span(train, on, car.kind);
        let bogie = |s: f32| {
            let p = terrain.lines().point(train.line, s);
            Vec3::new(p.x, terrain.track_level(train.line, s) + 0.3, p.y)
        };
        let f = bogie(front - train.dir * 1.5);
        let b = bogie(back + train.dir * 1.5);
        let forward = (f - b).normalize_or(Vec3::Z);
        transform.translation = (f + b) * 0.5;
        transform.rotation = Quat::from_rotation_arc(Vec3::Z, forward);
        let hidden = damage.in_tunnel(&terrain, train.line, front) && damage.in_tunnel(&terrain, train.line, back);
        *visibility = if hidden { Visibility::Hidden } else { Visibility::Inherited };
    }
}

/// Колесо вагона: узел модели с именем `wheel_<n>_r<радиус × 10000>`, его выделяет из модели
/// `scripts/meshy_fix/split_wheels.py`. Ось колеса — ось Z узла.
#[derive(Component)]
struct CarWheel {
    car: Entity,
    /// Поворот узла в модели.
    base: Quat,
    /// Радиус в единицах модели.
    radius: f32,
}

/// Путь вагона по рельсам, м: по нему поворачиваются колёса. Сошедший вагон его не копит,
/// и колёса у остова стоят.
#[derive(Component, Default)]
struct WheelRoll {
    last: Option<Vec3>,
    distance: f32,
}

/// Скачок вагона больше этого за кадр — перестановка, а не езда, м.
const WHEEL_JUMP: f32 = 20.0;

/// Радиус колеса в единицах модели по имени узла `wheel_<n>_r<радиус × 10000>`.
fn wheel_radius(name: &str) -> Option<f32> {
    let (_, r) = name.strip_prefix("wheel_")?.rsplit_once("_r")?;
    let r = r.parse::<f32>().ok()? / 10_000.0;
    (r > 0.0).then_some(r)
}

/// Находит колёса в загруженной модели вагона.
fn find_wheels(
    mut commands: Commands,
    named: Query<(Entity, &Name, &Transform), Added<Name>>,
    parents: Query<&ChildOf>,
    cars: Query<(), With<Car>>,
) {
    for (entity, name, transform) in &named {
        let Some(radius) = wheel_radius(name.as_str()) else {
            continue;
        };
        let Some(car) = parents.iter_ancestors(entity).find(|&e| cars.contains(e)) else {
            continue;
        };
        commands.entity(entity).insert(CarWheel { car, base: transform.rotation, radius });
    }
}

/// Копит путь вагонов на рельсах: вперёд — плюс, назад — минус.
fn roll_wheels(mut cars: Query<(&Transform, &mut WheelRoll), With<OnTrack>>) {
    for (transform, mut roll) in &mut cars {
        let p = transform.translation;
        if let Some(last) = roll.last {
            let step = p - last;
            if step.length() < WHEEL_JUMP {
                roll.distance += step.dot(transform.rotation * Vec3::Z);
            }
        }
        roll.last = Some(p);
    }
}

/// Поворачивает колёса на пройденный путь: колесо катится по рельсу без проскальзывания.
fn turn_wheels(
    cars: Query<(&WheelRoll, &GlobalTransform)>,
    mut wheels: Query<(&CarWheel, &mut Transform, &GlobalTransform)>,
) {
    for (wheel, mut transform, global) in &mut wheels {
        let Ok((roll, car)) = cars.get(wheel.car) else {
            continue;
        };
        let (scale, rotation, _) = global.to_scale_rotation_translation();
        // Катящееся колесо вращается вокруг «вверх × вперёд»: знак — с какой стороны ось узла.
        let side = car.up().cross(*car.back());
        let angle = side.dot(rotation * Vec3::Z) * roll.distance / (wheel.radius * scale.y);
        transform.rotation = wheel.base * Quat::from_rotation_z(angle);
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
///
/// Вагон — стена для машин: каждый кадр машина выталкивается из кузова на свою сторону пути,
/// поэтому состав нельзя проехать насквозь. Удар — урон, отброс от пути и остановка лунохода.
#[allow(clippy::type_complexity)]
fn hit_vehicles(
    mut commands: Commands,
    cars: Query<(&Transform, &Car, &Visibility), With<OnTrack>>,
    mut vehicles: Query<
        (Entity, &mut Transform, &Collider, Option<&TrainHitCooldown>, Option<&mut RoverMotion>),
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
        let roof = transform.translation.y + car.kind.size().1 + 1.0;
        for (entity, mut vehicle, collider, cooldown, motion) in &mut vehicles {
            let p = Vec2::new(vehicle.translation.x, vehicle.translation.z);
            // Машина над вагоном — на горе над туннелем: вагон под ней её не задевает.
            if footprint.distance(p) > collider.radius || vehicle.translation.y > roof {
                continue;
            }
            // Сбоку от кузова — на свою сторону; спереди и сзади — куда ближе.
            if let Some(push) = shape.push_out(p, collider.radius) {
                vehicle.translation.x += push.x;
                vehicle.translation.z += push.y;
            }
            if cooldown.is_some() {
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
            // Луноход останавливается: удар гасит его разгон.
            if let Some(mut motion) = motion {
                motion.speed = 0.0;
            }
            damage.write(Damage {
                target: entity,
                amount: HIT_DAMAGE,
                source_team: Team::Neutral,
            });
        }
        for (entity, obstacle, health) in &obstacles {
            if health.is_dead() {
                continue;
            }
            let (center, r) = obstacle.shape.bounds();
            if footprint.distance(center) <= r * 0.5 || shape.contains(center) {
                commands.entity(entity).try_remove::<Obstacle>();
                damage.write(Damage {
                    target: entity,
                    amount: health.current + 1.0,
                    source_team: Team::Neutral,
                });
            }
        }
    }
}

/// Взрывы ранят вагоны вдвое: общий урон взрыва по команде вагона считает `combat`, здесь —
/// столько же сверху, чтобы прочность вагонов против взрывов осталась прежней.
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
            let amount = blast_damage(e.power, e.radius, transform.translation().distance(e.position), false);
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

/// Вагон уничтожен — погиб или сошёл с рельсов: грузовой роняет ящики, пассажирский поднимает
/// тревогу, если его уничтожил игрок. Срабатывает один раз на вагон.
#[allow(clippy::too_many_arguments)]
fn car_destroyed(
    car: &mut Car,
    at: Vec3,
    by: Team,
    survival: f32,
    dice: &mut Dice,
    alarm: &mut Alarm,
    loot: &mut MessageWriter<SpawnLoot>,
) {
    if car.spent {
        return;
    }
    car.spent = true;
    match car.kind {
        CarKind::Freight => {
            let weapons = crate_weapons(survival);
            for k in 0..1 + dice.below(2) {
                let weapon = weapons[dice.below(weapons.len())];
                loot.write(SpawnLoot {
                    position: at + Vec3::new(2.0 + k as f32 * 1.5, 1.0, 1.5),
                    weapon,
                });
            }
        }
        CarKind::Passenger if by == Team::Player => alarm.add(PASSENGER_ALARM),
        _ => {}
    }
}

/// Сход с рельсов: разрушенное звено перед головой — весь состав; гибель вагона — он и все позади.
///
/// Сход записывается на того, кто его вызвал: на порвавшего путь или на добившего вагон. Вагоны,
/// которые в этот момент целиком в невскрытом туннеле, остаются в горе: они исчезают, а не
/// выходят на её поверхность.
#[allow(clippy::too_many_arguments)]
fn derail_trains(
    mut commands: Commands,
    terrain: Res<Terrain>,
    damage: Res<TrackDamage>,
    survival: Res<SurvivalTime>,
    mut dice: ResMut<GameRng>,
    mut threat: ResMut<Threat>,
    mut alarm: ResMut<Alarm>,
    mut died: MessageReader<Died>,
    mut trains: Query<(Entity, &mut Train)>,
    mut cars: Query<(&mut Car, &Transform, &GlobalTransform, Option<&OnTrack>)>,
    mut explosions: MessageWriter<Explosion>,
    mut loot: MessageWriter<SpawnLoot>,
    mut derailed: MessageWriter<TrainDerailed>,
    mut debris: MessageWriter<SpawnDebris>,
    bank: Res<ModelBank>,
) {
    let mut derails: Vec<(Entity, usize, Team)> = Vec::new();
    for death in died.read() {
        let Ok((mut car, _, global, on)) = cars.get_mut(death.entity) else {
            continue;
        };
        let on_train = on.map(|on| on.train);
        car_destroyed(&mut car, global.translation(), death.killer, survival.0, &mut dice, &mut alarm, &mut loot);
        if let Some(train_entity) = on_train
            && let Ok((_, train)) = trains.get(train_entity)
            && let Some(index) = train.cars.iter().position(|&c| c == death.entity)
        {
            derails.push((train_entity, index, death.killer));
        }
    }
    for (entity, train) in &trains {
        let ahead = train.head + train.dir * TRAIN_SPEED * 0.2;
        let blocked = damage.is_broken(train.line, ahead) && !damage.in_tunnel(&terrain, train.line, ahead);
        if blocked {
            let cause = damage.broken_by(train.line, ahead).unwrap_or(Team::Neutral);
            derails.push((entity, 0, cause));
        }
    }
    for (entity, from, cause) in derails {
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
        let mut tanks = 0;
        for (j, car_entity) in fallen.into_iter().enumerate() {
            let Ok((car, transform, _, Some(on))) = cars.get(car_entity) else {
                continue;
            };
            // Вагон целиком в туннеле: остаётся в горе.
            let (front, back) = car_span(&train, on, car.kind);
            if damage.in_tunnel(&terrain, train.line, front) && damage.in_tunnel(&terrain, train.line, back) {
                commands.entity(car_entity).try_despawn();
                continue;
            }
            if first {
                first = false;
                explosions.write(Explosion {
                    position: transform.translation,
                    radius: DERAIL_BLAST.0,
                    power: DERAIL_BLAST.1,
                    source_team: cause,
                });
                derailed.write(TrainDerailed {
                    position: transform.translation,
                    cause,
                });
            }
            // При сходе с кузова срывает обшивку.
            burst_car(&mut debris, &bank, car.kind, transform, 4, 6.0, 0);
            // Цистерны состава рвутся по очереди, а не в одном кадре.
            let due = match car.kind {
                CarKind::Locomotive => LOCO_DELAY,
                CarKind::Tank => {
                    tanks += 1;
                    TANK_DELAY + (tanks - 1) as f32 * TANK_STAGGER
                }
                _ => f32::MAX,
            };
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
                        due,
                        exploded: !matches!(car.kind, CarKind::Locomotive | CarKind::Tank),
                        cause,
                        hit_cooldown: 0.0,
                        still: 0.0,
                        lit: false,
                    },
                    Visibility::Inherited,
                ));
        }
        // В угрозу идут только крушения, которые устроил игрок.
        if cause == Team::Player && !first {
            threat.add_destruction(WRECK_THREAT);
        }
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
///
/// Угол, ушедший под рельеф глубже [`WALL_STEP`], упёрся в стену — склон горы у портала или
/// обрыв: вагон не взбирается на неё, а отскакивает назад. Вагон, целиком ушедший в гору, исчезает.
#[allow(clippy::too_many_arguments)]
fn move_wrecks(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut wrecks: Query<(Entity, &Car, &mut Transform, &mut Wreck)>,
    mut explosions: MessageWriter<Explosion>,
    mut debris: MessageWriter<SpawnDebris>,
    bank: Res<ModelBank>,
    mut blast_wait: Local<f32>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    *blast_wait -= dt;
    let depths = |transform: &Transform, kind: CarKind| {
        corners(transform, kind).map(|c| terrain.height_at(c.x, c.z) - c.y)
    };
    for (entity, car, mut transform, mut wreck) in &mut wrecks {
        wreck.age += dt;
        wreck.velocity.y -= 25.0 * dt;
        let before = transform.translation;
        transform.translation += wreck.velocity * dt;
        let spin = wreck.spin;
        transform.rotation = (transform.rotation * Quat::from_scaled_axis(spin * dt)).normalize();
        // Стена: шаг по плану отменяется, скорость вдоль плана гасится с отскоком.
        if depths(&transform, car.kind).iter().any(|&d| d > WALL_STEP) {
            transform.translation.x = before.x;
            transform.translation.z = before.z;
            wreck.velocity.x *= -0.2;
            wreck.velocity.z *= -0.2;
        }
        let depths = depths(&transform, car.kind);
        if depths.iter().all(|&d| d > WALL_STEP) {
            // Вагон целиком в горе.
            commands.entity(entity).try_despawn();
            continue;
        }
        // Контакт: самый низкий угол кузова не уходит под рельеф; углы в стене не в счёт.
        let lift = depths.iter().copied().filter(|&d| d <= WALL_STEP).fold(f32::MIN, f32::max);
        if lift > 0.0 {
            transform.translation.y += lift;
            let impact = -wreck.velocity.y;
            if impact > 0.0 {
                wreck.velocity.y = impact * 0.15;
            }
            let friction = (1.0 - 2.2 * dt).max(0.0);
            wreck.velocity.x *= friction;
            wreck.velocity.z *= friction;
            wreck.spin *= (1.0 - 2.5 * dt).max(0.0);
            // Цистерна рвётся при сильном ударе о землю — в свою очередь, не раньше.
            if car.kind == CarKind::Tank && impact > 6.0 && !wreck.exploded {
                wreck.due = wreck.due.min(wreck.age);
            }
        }
        // Взрывы вагонов разнесены по времени: не чаще одного за BLAST_GAP.
        if !wreck.exploded && wreck.age >= wreck.due && *blast_wait <= 0.0 {
            wreck.exploded = true;
            *blast_wait = BLAST_GAP;
            let (radius, power) = if car.kind == CarKind::Tank { TANK_BLAST } else { LOCO_BLAST };
            explosions.write(Explosion {
                position: transform.translation,
                radius,
                power,
                source_team: wreck.cause,
            });
            burst_car(&mut debris, &bank, car.kind, &transform, 10, 9.0, 3);
        }
        // Вагон загорается сразу после схода: свет, огонь и дым из кузова.
        if !wreck.lit {
            wreck.lit = true;
            let height = car.kind.size().1;
            commands.spawn((
                CarFire { age: 0.0 },
                crate::fire::Fire::new(CAR_FIRE_SIZE, 1.0).for_seconds(BURN_TIME).then_smoke(SMOKE_TIME),
                Transform::from_xyz(0.0, height * 0.6, 0.0),
                ChildOf(entity),
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

/// Вагон на ходу и кувыркающийся вагон сбивают жителей насмерть: от поезда не отскочишь.
#[allow(clippy::type_complexity)]
fn run_over_residents(
    cars: Query<(&Transform, &Car, &Visibility, Option<&Wreck>), Or<(With<OnTrack>, With<Wreck>)>>,
    residents: Query<(Entity, &Transform, &Health), With<Resident>>,
    mut damage: MessageWriter<Damage>,
) {
    for (transform, car, visibility, wreck) in &cars {
        if *visibility == Visibility::Hidden || wreck.is_some_and(|w| w.velocity.length() < RESIDENT_HIT_SPEED) {
            continue;
        }
        let footprint = car_footprint(transform, car.kind);
        let roof = transform.translation.y + car.kind.size().1 + 1.0;
        for (entity, resident, health) in &residents {
            let p = Vec2::new(resident.translation.x, resident.translation.z);
            // Житель на горе над туннелем вагон не задевает.
            if health.is_dead() || footprint.distance(p) > RESIDENT_RADIUS || resident.translation.y > roof {
                continue;
            }
            damage.write(Damage {
                target: entity,
                amount: health.current + 1.0,
                source_team: Team::Neutral,
            });
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
                    source_team: Team::Neutral,
                });
            }
        }
    }
}

/// Остановившийся вагон становится обугленным остовом-препятствием и записывается в хранилище.
/// Сошедший вагон уничтожен: груз и тревога — как при гибели, если их ещё не было.
#[allow(clippy::too_many_arguments)]
fn settle_wrecks(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<TrainAssets>,
    survival: Res<SurvivalTime>,
    mut dice: ResMut<GameRng>,
    mut alarm: ResMut<Alarm>,
    mut store: ResMut<WreckStore>,
    mut wrecks: Query<(Entity, &mut Car, &Transform, &mut Wreck, &Children)>,
    mut materials: Query<&mut MeshMaterial3d<StandardMaterial>>,
    mut loot: MessageWriter<SpawnLoot>,
) {
    for (entity, mut car, transform, mut wreck, children) in &mut wrecks {
        if wreck.velocity.length() < 0.6 && wreck.spin.length() < 0.4 {
            wreck.still += time.delta_secs();
        } else {
            wreck.still = 0.0;
        }
        if wreck.still < 0.5 || wreck.age < 1.5 || !wreck.exploded {
            continue;
        }
        car_destroyed(&mut car, transform.translation, wreck.cause, survival.0, &mut dice, &mut alarm, &mut loot);
        let coord = chunk_coord_at(transform.translation.x, transform.translation.z);
        if let Some(oldest) = store.push(coord, car.kind, *transform, entity) {
            commands.entity(oldest).try_despawn();
        }
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
            .remove::<(Wreck, Health, Team, Collider)>()
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
    mut models: Models,
    mut store: ResMut<WreckStore>,
) {
    if !visible.is_changed() || store.is_empty() {
        return;
    }
    let store = &mut *store;
    store.shown.retain(|&coord, entities| {
        let keep = visible.entity(coord).is_some();
        if !keep {
            for &(_, e) in entities.iter() {
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
            let e = spawn_car(&mut commands, &assets, &mut models, record.kind, record.transform);
            let top = corners(&record.transform, record.kind).iter().map(|c| c.y).fold(f32::MIN, f32::max);
            commands.entity(e).insert((
                ModelShade(CHARRED_SHADE),
                Car {
                    kind: record.kind,
                    spent: true,
                },
                Obstacle {
                    shape: Shape::Rect(car_footprint(&record.transform, record.kind)),
                    top,
                },
            ));
            entities.push((record.serial, e));
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
            commands.entity(entity).try_remove::<Knockback>();
        }
    }
    for (entity, mut cooldown) in &mut cooldowns {
        cooldown.0 -= dt;
        if cooldown.0 <= 0.0 {
            commands.entity(entity).try_remove::<TrainHitCooldown>();
        }
    }
}

/// Вагон горит [`BURN_TIME`], затем носитель огня удаляется: остов дымит [`SMOKE_TIME`]
/// дымом модуля огня.
fn burn_fires(mut commands: Commands, time: Res<Time>, mut fires: Query<(Entity, &mut CarFire)>) {
    for (entity, mut fire) in &mut fires {
        fire.age += time.delta_secs();
        if fire.age >= BURN_TIME + 0.2 {
            commands.entity(entity).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn радиус_колеса_из_имени_узла() {
        assert_eq!(super::wheel_radius("wheel_3_r468"), Some(0.0468));
        assert_eq!(super::wheel_radius("wheel_0_r0"), None);
        assert_eq!(super::wheel_radius("geometry_0"), None);
        assert_eq!(super::wheel_radius("wheel_rim"), None);
    }

    use super::*;

    #[test]
    fn после_сброса_остовов_нет() {
        let mut app = App::new();
        app.add_message::<WorldRebuilt>()
            .init_resource::<Schedule>()
            .init_resource::<WreckStore>()
            .add_systems(Update, reset_trains);
        app.world_mut().resource_mut::<WreckStore>().records.insert(
            IVec2::ZERO,
            vec![WreckRecord {
                serial: 0,
                kind: CarKind::Tank,
                transform: Transform::default(),
            }],
        );
        app.world_mut().write_message(WorldRebuilt { seed: 1 });
        app.update();
        assert!(app.world().resource::<WreckStore>().is_empty());
    }

    #[test]
    fn остовов_не_больше_предела_и_старые_уходят_первыми() {
        let mut store = WreckStore::default();
        let mut evicted = Vec::new();
        for i in 0..MAX_WRECKS + 3 {
            let coord = IVec2::new(i as i32 % 3, 0);
            let entity = Entity::from_raw_u32(i as u32 + 1).unwrap();
            if let Some(old) = store.push(coord, CarKind::Freight, Transform::default(), entity) {
                evicted.push(old);
            }
        }
        assert_eq!(store.len(), MAX_WRECKS);
        let first: Vec<Entity> = (1..=3).map(|i| Entity::from_raw_u32(i).unwrap()).collect();
        assert_eq!(evicted, first);
        assert!(store.records.values().flatten().all(|r| r.serial >= 3));
    }

    #[test]
    fn удар_поезда_и_кувыркание_держат_прежнюю_долю_здоровья() {
        assert!((HIT_DAMAGE / ROVER_MAX_HEALTH - 60.0 / 300.0).abs() < 1e-6);
        assert!((TUMBLE_DAMAGE / ROVER_MAX_HEALTH - 40.0 / 300.0).abs() < 1e-6);
    }
}
