//! Перехватчики: не дают просто уехать от боя.
//!
//! [`Flight`] следит за бегством: луноход за окно сместился далеко, обычных врагов рядом нет,
//! а дальше есть преследователи. Несколько секунд бегства подряд — перехват: с флангов впереди
//! по ходу выезжает звено перехватчиков. Они идут на точку упреждения и таранят луноход: удар
//! сбрасывает скорость и на время режет её потолок ([`Impact`]), чтобы волна догнала.
//! Когда перехват кончился, перехватчики держатся сбоку и через время уезжают.
//!
//! Все числа — в [`waves::balance`](crate::waves::balance).

use std::collections::VecDeque;

use bevy::prelude::*;

use crate::combat::{Collider, Damage, Health, Team};
use crate::enemy::{Enemy, EnemyKind, INTERCEPTOR_SPEED, SpawnEnemy, SpawnPlace};
use crate::frame::{GameSet, RunMessages};
use crate::rng::GameRng;
use crate::rover::{Impact, Rover, RoverMotion};
use crate::run::WorldRebuilt;
use crate::steering::Steering;
use crate::vehicle::{self, GroundVehicle};
use crate::waves::{Threat, balance, enemy_damage};

/// Половина длины отрезка корпуса дрона для удара, м: дрон — крест около 4,8 × 4,8 м, удар носом
/// с электродами, поэтому корпус — короткая толстая капсула вдоль курса.
const HULL_HALF_LENGTH: f32 = 1.0;
/// Радиус капсулы корпуса вокруг отрезка, м.
const HULL_RADIUS: f32 = 1.5;
/// Концы электродов на носу дрона в системе перехватчика (до его масштаба): из модели
/// `enemy_interceptor` (x 0,345, z ±0,05, y −0,04; нос +X при масштабе 3,4, поворот +90°, подъём 1,08).
const PRONG_TIPS: [Vec3; 2] = [Vec3::new(-0.17, 0.944, -1.17), Vec3::new(0.17, 0.944, -1.17)];
/// Кольца винтов в системе перехватчика (до его масштаба): центры (±0,335 в модели) и радиус,
/// высота над лопастями.
const ROTORS: [Vec2; 4] = [Vec2::new(1.14, 1.14), Vec2::new(-1.14, 1.14), Vec2::new(1.14, -1.14), Vec2::new(-1.14, -1.14)];
const ROTOR_RADIUS: f32 = 0.45;
const ROTOR_HEIGHT: f32 = 1.13;
/// Скорость винтов, рад/с.
const ROTOR_SPIN: f32 = 21.0;
/// Сколько горит молния от электродов к луноходу при ударе, с, и как часто она меняет излом.
const ZAP_SECONDS: f32 = 0.45;
const ZAP_REDRAW: f32 = 0.04;
/// Отрезков в молнии удара и наибольший излом поперёк, м.
const ZAP_SEGMENTS: usize = 6;
const ZAP_JITTER: f32 = 0.35;
/// Молния бьёт, только если электроды не дальше этого от центра лунохода, м: дрон вплотную.
const ZAP_REACH: f32 = 3.5;
/// Запас к касанию, при котором удар засчитывается, м.
const RAM_MARGIN: f32 = 0.2;

/// Подключает слежение за бегством, звено перехвата и его удаление.
///
/// Езду и удар перехватчиков ставит цепочка врагов в `enemy.rs`: [`drive_interceptors`] перед
/// объездом препятствий, [`ram_by_interceptors`] сразу после движения машин.
pub struct InterceptorPlugin;

impl Plugin for InterceptorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Flight>()
            .init_resource::<Gadflies>()
            .add_run_message::<InterceptorHit>()
            .add_run_message::<InterceptStarted>()
            .add_systems(Startup, create_zap_material)
            .add_systems(Update, add_core.in_set(GameSet::Presentation))
            .add_systems(
                Update,
                (
                    (reset_flight, reset_gadflies).in_set(GameSet::Reset),
                    (track_flight, dispatch_interceptors, dispatch_gadflies, enlist_gadflies, pick_dasher)
                        .chain()
                        .in_set(GameSet::Simulation),
                    remove_departed.in_set(GameSet::Cleanup),
                    (spin_rotors, spawn_zaps, draw_zaps, glow_cores).in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Перехватчик `drone` ударил луноход в точке `position`; разряд идёт с концов электродов `prongs`.
#[derive(Message, Debug, Clone, Copy)]
pub struct InterceptorHit {
    pub position: Vec3,
    pub prongs: [Vec3; 2],
    pub drone: Entity,
}

/// Винт дрона: крутится вокруг своей оси.
#[derive(Component)]
pub struct Rotor;

/// Молния удара от электрода дрона `drone` (номер `prong`) к корпусу лунохода: концы идут
/// за ними, пока молния горит.
#[derive(Component)]
struct Zap {
    drone: Entity,
    prong: usize,
    from: Vec3,
    to: Vec3,
    left: f32,
    redraw: f32,
    seed: u32,
    segments: Vec<Entity>,
}

/// Начался перехват: игрок уезжает от волны.
#[derive(Message, Debug, Clone, Copy)]
pub struct InterceptStarted;

/// Бегство и перехват текущего забега.
#[derive(Resource, Debug, Default)]
pub struct Flight {
    /// Позиции лунохода раз в [`balance::FLIGHT_SAMPLE`] за последние [`balance::FLIGHT_WINDOW`].
    samples: VecDeque<Vec2>,
    sample_timer: f32,
    /// Сколько секунд бегства накоплено: растёт в кадре бегства, вдвое быстрее падает вне его.
    gauge: f32,
    /// Идёт ли перехват.
    pub active: bool,
    /// Перехват начался в этом кадре.
    started: bool,
    /// Куда луноход сместился за окно.
    travel: Vec2,
    /// Сколько до следующего подкрепления, с.
    reinforce: f32,
}

impl Flight {
    /// Сколько позиций держит окно.
    fn window_len() -> usize {
        (balance::FLIGHT_WINDOW / balance::FLIGHT_SAMPLE).round() as usize + 1
    }

    /// Шаг слежения: луноход в `position`, `near_enemy` — обычный враг ближе
    /// [`balance::CONTACT_RADIUS`], `pursuers` — живой обычный враг дальше него.
    /// Возвращает, начался ли перехват в этом шаге.
    fn step(&mut self, dt: f32, position: Vec2, near_enemy: bool, pursuers: bool) -> bool {
        self.sample_timer -= dt;
        if self.sample_timer <= 0.0 || self.samples.is_empty() {
            self.sample_timer += balance::FLIGHT_SAMPLE;
            self.samples.push_back(position);
            while self.samples.len() > Self::window_len() {
                self.samples.pop_front();
            }
        }
        self.travel = position - self.samples.front().copied().unwrap_or(position);
        let displacement = self.travel.length();
        if self.active {
            if flight_ends(displacement, near_enemy, pursuers) {
                self.active = false;
                self.gauge = 0.0;
            }
            return false;
        }
        if flight_frame(displacement, near_enemy, pursuers) {
            self.gauge += dt;
        } else {
            self.gauge = (self.gauge - 2.0 * dt).max(0.0);
        }
        if self.gauge >= balance::ESCAPE_TRIGGER {
            self.active = true;
            self.gauge = 0.0;
            return true;
        }
        false
    }
}

/// Кадр бегства: луноход далеко сместился за окно, обычных врагов рядом нет, а дальше есть.
fn flight_frame(displacement: f32, near_enemy: bool, pursuers: bool) -> bool {
    displacement >= balance::FLIGHT_DISTANCE && !near_enemy && pursuers
}

/// Конец перехвата: враг подъехал, луноход почти стоит или кружит, или преследователей нет.
/// Порог смещения ниже порога начала: замедленный ударами луноход перехват не обрывает.
fn flight_ends(displacement: f32, near_enemy: bool, pursuers: bool) -> bool {
    near_enemy || !pursuers || displacement < balance::CALM_DISTANCE
}

/// Новый мир: окно и перехват прошлого забега забыты.
fn reset_flight(mut rebuilt: MessageReader<WorldRebuilt>, mut flight: ResMut<Flight>) {
    if rebuilt.read().count() > 0 {
        *flight = Flight::default();
    }
}

/// Следит за бегством лунохода от обычных врагов.
fn track_flight(
    time: Res<Time>,
    mut flight: ResMut<Flight>,
    rover: Query<(&Transform, &Health), With<Rover>>,
    enemies: Query<&Transform, (With<Enemy>, Without<Interceptor>)>,
    mut started: MessageWriter<InterceptStarted>,
) {
    flight.started = false;
    let Some((rover, health)) = rover.iter().next() else {
        return;
    };
    if health.is_dead() {
        flight.active = false;
        return;
    }
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let position = Vec2::new(rover.translation.x, rover.translation.z);
    let (mut near_enemy, mut pursuers) = (false, false);
    for transform in &enemies {
        let distance = Vec2::new(transform.translation.x, transform.translation.z).distance(position);
        if distance < balance::CONTACT_RADIUS {
            near_enemy = true;
        } else {
            pursuers = true;
        }
    }
    if flight.step(dt, position, near_enemy, pursuers) {
        flight.started = true;
        started.write(InterceptStarted);
    }
}

/// Угол появления перехватчика вокруг лунохода: от направления движения `travel` на `offset`
/// в сторону `side` (1 или -1).
fn flank_angle(travel: Vec2, offset: f32, side: f32) -> f32 {
    travel.y.atan2(travel.x) + side * offset
}

/// Ставит звено при начале перехвата и по одному подкреплению, пока он идёт.
fn dispatch_interceptors(
    time: Res<Time>,
    mut flight: ResMut<Flight>,
    threat: Res<Threat>,
    mut rng: ResMut<GameRng>,
    interceptors: Query<(), With<Interceptor>>,
    mut spawns: MessageWriter<SpawnEnemy>,
) {
    if !flight.active {
        return;
    }
    let pack = balance::pack_size(threat.value());
    let alive = interceptors.iter().count();
    let count = if flight.started {
        flight.reinforce = balance::REINFORCE_GAP;
        pack.saturating_sub(alive)
    } else {
        flight.reinforce -= time.delta_secs();
        if flight.reinforce > 0.0 || alive >= pack {
            0
        } else {
            flight.reinforce = balance::REINFORCE_GAP;
            1
        }
    };
    for _ in 0..count {
        let side = if rng.range(0.0, 1.0) < 0.5 { 1.0 } else { -1.0 };
        let offset = rng.range(balance::FLANK_ANGLE.0, balance::FLANK_ANGLE.1);
        spawns.write(SpawnEnemy {
            place: SpawnPlace::Edge {
                angle: flank_angle(flight.travel, offset, side),
            },
            kind: EnemyKind::Interceptor,
        });
    }
}

/// Овод: дрон, который в бою кружит вокруг лунохода и по очереди с напарником бросается на него.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub enum Gadfly {
    /// Кружит на [`balance::GADFLY_RING`] м, угол на круге, рад.
    Circle(f32),
    /// Тормозит, ядро разгорается: осталось секунд.
    Charge(f32),
    /// Бросается в точку `target`, где был луноход в начале рывка; осталось секунд.
    Dash { target: Vec3, left: f32 },
    /// Отходит после рывка, осталось секунд.
    Retreat(f32),
}

/// Оводы забега: сколько заказано и ещё не появилось, и когда следующий рывок.
#[derive(Resource, Debug, Default)]
struct Gadflies {
    pending: usize,
    next_dash: f32,
}

fn reset_gadflies(mut rebuilt: MessageReader<WorldRebuilt>, mut gadflies: ResMut<Gadflies>) {
    if rebuilt.read().count() > 0 {
        *gadflies = Gadflies::default();
    }
}

/// На старте волны с угрозы [`balance::GADFLY_THREAT`] добирает оводов до [`balance::GADFLY_COUNT`].
fn dispatch_gadflies(
    mut started: MessageReader<crate::waves::WaveStarted>,
    threat: Res<Threat>,
    mut gadflies: ResMut<Gadflies>,
    mut rng: ResMut<GameRng>,
    alive: Query<(), With<Gadfly>>,
    mut spawns: MessageWriter<SpawnEnemy>,
) {
    if started.read().count() == 0 {
        return;
    }
    let have = alive.iter().count() + gadflies.pending;
    let missing = balance::gadflies_wanted(threat.value()).saturating_sub(have);
    for _ in 0..missing {
        spawns.write(SpawnEnemy {
            place: SpawnPlace::Edge {
                angle: rng.range(0.0, std::f32::consts::TAU),
            },
            kind: EnemyKind::Interceptor,
        });
    }
    gadflies.pending += missing;
    if missing > 0 && gadflies.next_dash <= 0.0 {
        gadflies.next_dash = rng.range(balance::GADFLY_GAP.0, balance::GADFLY_GAP.1);
    }
}

/// Заказанные оводы — первые появившиеся перехватчики без роли.
fn enlist_gadflies(
    mut commands: Commands,
    mut gadflies: ResMut<Gadflies>,
    fresh: Query<Entity, (Added<Interceptor>, Without<Gadfly>)>,
) {
    for (i, entity) in fresh.iter().enumerate() {
        if gadflies.pending == 0 {
            break;
        }
        gadflies.pending -= 1;
        commands.entity(entity).try_insert(Gadfly::Circle(i as f32 * std::f32::consts::PI));
    }
}

/// Раз в несколько секунд один из кружащих оводов начинает разгон к рывку.
fn pick_dasher(
    time: Res<Time>,
    mut gadflies: ResMut<Gadflies>,
    mut rng: ResMut<GameRng>,
    mut drones: Query<&mut Gadfly>,
) {
    if drones.is_empty() {
        return;
    }
    gadflies.next_dash -= time.delta_secs();
    if gadflies.next_dash > 0.0 {
        return;
    }
    gadflies.next_dash = rng.range(balance::GADFLY_GAP.0, balance::GADFLY_GAP.1);
    let busy = drones.iter().any(|g| !matches!(g, Gadfly::Circle(_)));
    if busy {
        return;
    }
    let count = drones.iter().count();
    let chosen = rng.below(count);
    if let Some(mut gadfly) = drones.iter_mut().nth(chosen) {
        *gadfly = Gadfly::Charge(balance::GADFLY_CHARGE);
    }
}

/// Следующее состояние овода за шаг `dt`: `rover` — где луноход сейчас, `near` — овод у цели рывка.
pub fn gadfly_step(state: Gadfly, dt: f32, rover: Vec3, near: bool) -> Gadfly {
    match state {
        Gadfly::Circle(angle) => Gadfly::Circle(angle + balance::GADFLY_ORBIT * dt),
        Gadfly::Charge(left) if left - dt <= 0.0 => Gadfly::Dash {
            target: rover,
            left: balance::GADFLY_DASH_TIME,
        },
        Gadfly::Charge(left) => Gadfly::Charge(left - dt),
        Gadfly::Dash { left, .. } if near || left - dt <= 0.0 => Gadfly::Retreat(balance::GADFLY_RETREAT),
        Gadfly::Dash { target, left } => Gadfly::Dash { target, left: left - dt },
        Gadfly::Retreat(left) if left - dt <= 0.0 => Gadfly::Circle(0.0),
        Gadfly::Retreat(left) => Gadfly::Retreat(left - dt),
    }
}

/// Ведёт оводов: круг, разгон на месте лицом к луноходу, рывок по прямой без упреждения, отход.
pub fn drive_gadflies(
    time: Res<Time>,
    rover: Query<(&Transform, &Health), With<Rover>>,
    mut drones: Query<(&Transform, &GroundVehicle, &mut Steering, &mut Gadfly), Without<Rover>>,
) {
    let dt = time.delta_secs();
    let rover = rover.iter().next().filter(|(_, health)| !health.is_dead());
    for (transform, vehicle, mut steering, mut gadfly) in &mut drones {
        let Some((rover, _)) = rover else {
            steering.speed = 0.0;
            continue;
        };
        let me = transform.translation;
        let target = rover.translation;
        let near = matches!(*gadfly, Gadfly::Dash { target: t, .. } if (t - me).with_y(0.0).length() < 1.5);
        let mut next = gadfly_step(*gadfly, dt, target, near);
        // Отход кончился: на круг с того угла, где овод сейчас.
        if matches!(*gadfly, Gadfly::Retreat(_)) && matches!(next, Gadfly::Circle(_)) {
            let from = (me - target).with_y(0.0);
            next = Gadfly::Circle(from.z.atan2(from.x));
        }
        *gadfly = next;
        let (desired, speed) = match *gadfly {
            Gadfly::Circle(angle) => {
                let spot = target + Vec3::new(angle.cos(), 0.0, angle.sin()) * balance::GADFLY_RING;
                let to = (spot - me).with_y(0.0);
                (vehicle::heading_towards(to), (to.length() * 1.5).clamp(4.0, INTERCEPTOR_SPEED))
            }
            Gadfly::Charge(_) => (vehicle::heading_towards(target - me), 0.0),
            Gadfly::Dash { target: aim, .. } => (vehicle::heading_towards(aim - me), balance::GADFLY_DASH_SPEED),
            Gadfly::Retreat(_) => (vehicle::heading_towards(me - target), INTERCEPTOR_SPEED * 0.8),
        };
        steering.desired = desired;
        steering.speed = speed;
        let _ = vehicle;
    }
}

/// Светящееся ядро дрона: разгорается, пока овод разгоняется к рывку.
#[derive(Component)]
struct CoreGlow;

/// Ядро дрона в системе перехватчика (до его масштаба): над центром модели.
const CORE_HEIGHT: f32 = 1.37;

/// Ставит каждому новому дрону спрятанное ядро-вспышку.
fn add_core(
    mut commands: Commands,
    assets: Res<ZapAssets>,
    fresh: Query<Entity, Added<Interceptor>>,
) {
    for entity in &fresh {
        commands.entity(entity).with_children(|drone| {
            drone.spawn((
                CoreGlow,
                Mesh3d(assets.core.clone()),
                MeshMaterial3d(assets.material.clone()),
                bevy::light::NotShadowCaster,
                Transform::from_xyz(0.0, CORE_HEIGHT - crate::enemy::RIDE_HEIGHT, 0.0).with_scale(Vec3::ZERO),
            ));
        });
    }
}

/// Ядро растёт и мерцает, пока идёт разгон; в остальное время спрятано.
fn glow_cores(
    time: Res<Time>,
    drones: Query<(&Gadfly, &Children)>,
    mut cores: Query<&mut Transform, With<CoreGlow>>,
) {
    for (gadfly, children) in &drones {
        let size = match *gadfly {
            Gadfly::Charge(left) => {
                let k = 1.0 - left / balance::GADFLY_CHARGE;
                (0.15 + 0.35 * k) * (1.0 + 0.15 * (time.elapsed_secs() * 40.0).sin())
            }
            Gadfly::Dash { .. } => 0.4,
            _ => 0.0,
        };
        for child in children {
            if let Ok(mut core) = cores.get_mut(*child) {
                core.scale = Vec3::splat(size);
            }
        }
    }
}

/// Что делает перехватчик.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
enum Behaviour {
    /// Идёт наперерез, на точку упреждения.
    #[default]
    Attack,
    /// После удара едет мимо, остаток времени, с; затем снова заходит.
    Recoil(f32),
    /// Перехват кончился: держится сбоку, не бьёт.
    Shadow,
    /// Уезжает и исчезает.
    Leave,
}

/// Перехватчик.
#[derive(Component, Debug, Default)]
pub struct Interceptor {
    behaviour: Behaviour,
    /// Сколько секунд без перехвата.
    idle: f32,
}

/// Точка, где окажется луноход в `rover` со скоростью `velocity`, пока к нему едет перехватчик
/// из `me`. Упреждение не больше [`balance::LEAD_MAX`].
fn lead_point(rover: Vec3, velocity: Vec3, me: Vec3) -> Vec3 {
    let to = (rover - me).with_y(0.0);
    let distance = to.length();
    if distance < 1e-3 {
        return rover;
    }
    let closing = INTERCEPTOR_SPEED - velocity.dot(to / distance);
    let time = if closing > 0.5 { (distance / closing).min(balance::LEAD_MAX) } else { balance::LEAD_MAX };
    rover + velocity * time
}

/// Ведёт перехватчики: атака, отход после удара, сопровождение сбоку, отъезд.
pub fn drive_interceptors(
    time: Res<Time>,
    flight: Res<Flight>,
    rover: Query<(&Transform, &RoverMotion, &Health), With<Rover>>,
    mut interceptors: Query<(&Transform, &GroundVehicle, &mut Steering, &mut Interceptor), (Without<Rover>, Without<Gadfly>)>,
) {
    let dt = time.delta_secs();
    let rover = rover.iter().next().filter(|(_, _, health)| !health.is_dead());
    for (transform, vehicle, mut steering, mut interceptor) in &mut interceptors {
        let Some((rover, motion, _)) = rover else {
            steering.speed = 0.0;
            continue;
        };
        let me = transform.translation;
        let target = rover.translation;
        interceptor.behaviour = match interceptor.behaviour {
            Behaviour::Recoil(left) if left - dt <= 0.0 => Behaviour::Attack,
            Behaviour::Recoil(left) => Behaviour::Recoil(left - dt),
            _ if flight.active => Behaviour::Attack,
            Behaviour::Attack => Behaviour::Shadow,
            other => other,
        };
        if flight.active {
            interceptor.idle = 0.0;
        } else {
            interceptor.idle += dt;
            if interceptor.behaviour == Behaviour::Shadow && interceptor.idle > balance::LINGER_TIME {
                interceptor.behaviour = Behaviour::Leave;
            }
        }
        let velocity = vehicle::forward_of(motion.heading) * motion.speed;
        let (desired, speed) = match interceptor.behaviour {
            Behaviour::Attack => (vehicle::heading_towards(lead_point(target, velocity, me) - me), INTERCEPTOR_SPEED),
            Behaviour::Recoil(_) => (vehicle.heading, INTERCEPTOR_SPEED),
            Behaviour::Shadow => {
                // Сбоку от лунохода с той стороны, где перехватчик уже есть.
                let forward = vehicle::forward_of(motion.heading);
                let right = Vec3::new(-forward.z, 0.0, forward.x);
                let side = if (me - target).dot(right) >= 0.0 { 1.0 } else { -1.0 };
                let spot = target + right * side * balance::SHADOW_DISTANCE;
                let to_spot = (spot - me).with_y(0.0);
                if to_spot.length() < 3.0 {
                    (motion.heading, motion.speed.abs())
                } else {
                    let limit = motion.speed.abs().max(6.0);
                    (vehicle::heading_towards(to_spot), to_spot.length().min(limit))
                }
            }
            Behaviour::Leave => (vehicle::heading_towards(me - target), INTERCEPTOR_SPEED),
        };
        steering.desired = desired;
        steering.speed = speed;
    }
}

/// Касание лунохода в `rover` радиуса `rover_radius` корпусом перехватчика с центром `center`
/// и курсом `forward`: корпус — отрезок с радиусом [`HULL_RADIUS`]. Возвращает направление
/// от лунохода к корпусу и глубину вхождения (меньше нуля — касание в запасе, без вхождения).
fn segment_contact(center: Vec2, forward: Vec2, rover: Vec2, rover_radius: f32) -> Option<(Vec2, f32)> {
    let along = (rover - center).dot(forward).clamp(-HULL_HALF_LENGTH, HULL_HALF_LENGTH);
    let closest = center + forward * along;
    let away = closest - rover;
    let distance = away.length();
    let reach = rover_radius + HULL_RADIUS;
    if distance >= reach + RAM_MARGIN {
        return None;
    }
    let normal = if distance > 1e-4 { away / distance } else { Vec2::new(-forward.y, forward.x) };
    Some((normal, reach - distance))
}

/// Удар перехватчика: носом или бортом, лёгшим поперёк пути. Луноход теряет скорость и
/// на время упирается в потолок, перехватчик выталкивается и уходит на новый заход.
#[allow(clippy::type_complexity)]
pub fn ram_by_interceptors(
    mut rover: Query<(Entity, &Transform, &Collider, &Health, &mut RoverMotion, &mut Impact), With<Rover>>,
    mut interceptors: Query<
        (Entity, &mut Transform, &GroundVehicle, &mut Interceptor, Option<&mut Gadfly>),
        Without<Rover>,
    >,
    mut damage: MessageWriter<Damage>,
    mut hits: MessageWriter<InterceptorHit>,
) {
    let Some((entity, rover, collider, health, mut motion, mut impact)) = rover.iter_mut().next() else {
        return;
    };
    if health.is_dead() {
        return;
    }
    let center = Vec2::new(rover.translation.x, rover.translation.z);
    for (drone, mut transform, vehicle, mut interceptor, gadfly) in &mut interceptors {
        let me = Vec2::new(transform.translation.x, transform.translation.z);
        let forward = vehicle.forward();
        let forward = Vec2::new(forward.x, forward.z).normalize_or(Vec2::NEG_Y);
        let Some((normal, depth)) = segment_contact(me, forward, center, collider.radius) else {
            continue;
        };
        if depth > 0.0 {
            transform.translation += Vec3::new(normal.x, 0.0, normal.y) * depth;
        }
        // Овод бьёт только в рывке, потом отходит; перехватчик — в атаке, потом отскакивает.
        if let Some(mut gadfly) = gadfly {
            if !matches!(*gadfly, Gadfly::Dash { .. }) {
                continue;
            }
            *gadfly = Gadfly::Retreat(balance::GADFLY_RETREAT);
        } else if interceptor.behaviour != Behaviour::Attack {
            continue;
        } else {
            interceptor.behaviour = Behaviour::Recoil(balance::RAM_COOLDOWN);
        }
        motion.speed *= balance::HIT_SPEED_KEEP;
        impact.hit();
        damage.write(Damage {
            target: entity,
            amount: enemy_damage::INTERCEPTOR_RAM,
            source_team: Team::Enemy,
        });
        let at = center + normal * collider.radius;
        hits.write(InterceptorHit {
            position: Vec3::new(at.x, rover.translation.y, at.y),
            prongs: prong_tips(&transform),
            drone,
        });
    }
}

/// Концы электродов дрона в мире.
pub fn prong_tips(transform: &Transform) -> [Vec3; 2] {
    PRONG_TIPS.map(|tip| transform.transform_point(tip - Vec3::Y * crate::enemy::RIDE_HEIGHT))
}

/// Ставит дрону четыре винта над кольцами модели: две тёмные лопасти крестом.
pub fn spawn_rotors(parent: &mut ChildSpawnerCommands, mesh: Handle<Mesh>, material: Handle<StandardMaterial>) {
    for center in ROTORS {
        parent
            .spawn((
                Rotor,
                Transform::from_xyz(center.x, ROTOR_HEIGHT - crate::enemy::RIDE_HEIGHT, center.y),
                Visibility::default(),
            ))
            .with_children(|rotor| {
                for turn in [0.0, std::f32::consts::FRAC_PI_2] {
                    rotor.spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        bevy::light::NotShadowCaster,
                        Transform::from_rotation(Quat::from_rotation_y(turn))
                            .with_scale(Vec3::new(ROTOR_RADIUS * 2.0, 0.02, 0.07)),
                    ));
                }
            });
    }
}

fn spin_rotors(time: Res<Time>, mut rotors: Query<&mut Transform, With<Rotor>>) {
    for mut transform in &mut rotors {
        transform.rotate_y(ROTOR_SPIN * time.delta_secs());
    }
}

/// Материал молнии удара: ярче единицы, чтобы светилась через свечение камеры (у `unlit`
/// работает только `base_color`). Создаётся при старте, чтобы его застал прогрев шейдеров.
#[derive(Resource)]
struct ZapAssets {
    material: Handle<StandardMaterial>,
    /// Шар ядра-вспышки овода перед рывком.
    core: Handle<Mesh>,
}

fn create_zap_material(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    commands.insert_resource(ZapAssets {
        material: materials.add(StandardMaterial {
            base_color: Color::linear_rgb(3.0, 5.5, 14.0),
            unlit: true,
            ..default()
        }),
        core: meshes.add(Sphere::new(1.0).mesh().ico(2).expect("ядро дрона")),
    });
}

/// Удар: с каждого электрода к корпусу лунохода бьёт своя молния.
fn spawn_zaps(
    mut commands: Commands,
    mut hits: MessageReader<InterceptorHit>,
    assets: Res<crate::shock::ShockAssets>,
    material: Res<ZapAssets>,
    rover: Query<&Transform, With<Rover>>,
) {
    let Some(rover) = rover.iter().next() else {
        return;
    };
    for (n, hit) in hits.read().enumerate() {
        for (i, from) in hit.prongs.into_iter().enumerate() {
            let reach = from.with_y(0.0).distance(rover.translation.with_y(0.0));
            if reach > ZAP_REACH {
                continue;
            }
            let to = zap_target(rover, from);
            let segments = (0..ZAP_SEGMENTS)
                .map(|_| {
                    commands
                        .spawn((
                            crate::run::RunScoped,
                            Mesh3d(assets.segment.clone()),
                            MeshMaterial3d(material.material.clone()),
                            bevy::light::NotShadowCaster,
                            Transform::default(),
                        ))
                        .id()
                })
                .collect();
            commands.spawn((
                crate::run::RunScoped,
                Zap {
                    drone: hit.drone,
                    prong: i,
                    from,
                    to,
                    left: ZAP_SECONDS,
                    redraw: 0.0,
                    seed: (n * 2 + i) as u32 * 977 + (from.x.abs() * 100.0) as u32,
                    segments,
                },
            ));
        }
    }
}

/// Куда в корпус лунохода бьёт молния с электрода в `from`: в палубу со стороны удара, концы
/// разведены по сторонам, как электроды.
fn zap_target(rover: &Transform, from: Vec3) -> Vec3 {
    let side = (from - rover.translation).with_y(0.0).normalize_or(Vec3::X);
    let right = Vec3::new(-side.z, 0.0, side.x);
    let own = (from - rover.translation).dot(right).signum();
    rover.translation + side * 0.35 + right * own * 0.45 + Vec3::Y * 0.7
}

/// Псевдослучайное число 0..1 по номеру: облик молнии.
fn zap_noise(seed: u32) -> f32 {
    let mut x = seed.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 13;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

/// Молния удара ломается заново каждые [`ZAP_REDRAW`] с и гаснет за [`ZAP_SECONDS`].
fn draw_zaps(
    mut commands: Commands,
    time: Res<Time>,
    mut zaps: Query<(Entity, &mut Zap)>,
    mut segments: Query<&mut Transform, (Without<Rover>, Without<Interceptor>)>,
    rover: Query<&Transform, With<Rover>>,
    drones: Query<&Transform, (With<Interceptor>, Without<Rover>)>,
) {
    for (entity, mut zap) in &mut zaps {
        zap.left -= time.delta_secs();
        if zap.left <= 0.0 {
            for segment in zap.segments.drain(..) {
                commands.entity(segment).try_despawn();
            }
            commands.entity(entity).despawn();
            continue;
        }
        zap.redraw -= time.delta_secs();
        if zap.redraw > 0.0 {
            continue;
        }
        zap.redraw = ZAP_REDRAW;
        zap.seed = zap.seed.wrapping_add(1);
        // Концы — за электродом и корпусом: дрон отскакивает, луноход едет.
        if let Ok(drone) = drones.get(zap.drone) {
            zap.from = prong_tips(drone)[zap.prong];
        }
        if let Ok(rover) = rover.single() {
            zap.to = zap_target(rover, zap.from);
        }
        let axis = zap.to - zap.from;
        let across = axis.cross(Vec3::Y).normalize_or(Vec3::X);
        let points: Vec<Vec3> = (0..=ZAP_SEGMENTS)
            .map(|k| {
                let t = k as f32 / ZAP_SEGMENTS as f32;
                let mut p = zap.from.lerp(zap.to, t);
                if k > 0 && k < ZAP_SEGMENTS {
                    let s = zap.seed.wrapping_mul(131).wrapping_add(k as u32 * 17);
                    p += across * (zap_noise(s) - 0.5) * 2.0 * ZAP_JITTER
                        + Vec3::Y * (zap_noise(s + 5) - 0.5) * ZAP_JITTER;
                }
                p
            })
            .collect();
        // Тоньше к концу жизни.
        let thickness = 0.16 * (zap.left / ZAP_SECONDS).max(0.4);
        for (k, segment) in zap.segments.iter().enumerate() {
            let Ok(mut transform) = segments.get_mut(*segment) else {
                continue;
            };
            let (a, b) = (points[k], points[k + 1]);
            let d = b - a;
            *transform = Transform::from_translation((a + b) * 0.5)
                .with_rotation(Quat::from_rotation_arc(Vec3::Y, d.normalize_or(Vec3::Y)))
                .with_scale(Vec3::new(thickness, d.length(), thickness));
        }
    }
}

/// Уехавшие перехватчики исчезают за [`balance::LEAVE_DISTANCE`].
fn remove_departed(
    mut commands: Commands,
    rover: Query<&Transform, With<Rover>>,
    interceptors: Query<(Entity, &Transform, &Interceptor)>,
) {
    let Some(rover) = rover.iter().next() else {
        return;
    };
    for (entity, transform, interceptor) in &interceptors {
        if interceptor.behaviour == Behaviour::Leave
            && transform.translation.distance(rover.translation) > balance::LEAVE_DISTANCE
        {
            commands.entity(entity).try_despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{enemy, rover};

    const DT: f32 = 0.05;

    /// Гонит слежение `seconds` секунд по пути `path(t)`; возвращает, шёл ли перехват в конце
    /// и начинался ли хоть раз.
    fn drive(flight: &mut Flight, from: f32, seconds: f32, path: impl Fn(f32) -> Vec2, near: bool, pursuers: bool) -> (bool, bool) {
        let mut ever = false;
        let steps = (seconds / DT) as usize;
        for i in 0..steps {
            let t = from + i as f32 * DT;
            ever |= flight.step(DT, path(t), near, pursuers);
        }
        (flight.active, ever)
    }

    fn straight(speed: f32) -> impl Fn(f32) -> Vec2 {
        move |t| Vec2::new(speed * t, 0.0)
    }

    #[test]
    fn прямая_на_полном_ходу_от_отставших_бегство() {
        let mut flight = Flight::default();
        let (active, ever) = drive(&mut flight, 0.0, 12.0, straight(rover::MAX_FORWARD_SPEED), false, true);
        assert!(active && ever);
    }

    #[test]
    fn стоит_нет() {
        let mut flight = Flight::default();
        assert_eq!(drive(&mut flight, 0.0, 20.0, |_| Vec2::new(5.0, 5.0), false, true), (false, false));
    }

    #[test]
    fn круг_радиусом_25_м_нет() {
        let mut flight = Flight::default();
        let radius = 25.0;
        let omega = rover::MAX_FORWARD_SPEED / radius;
        let circle = |t: f32| Vec2::new((omega * t).cos(), (omega * t).sin()) * radius;
        // Даже без врагов рядом: смещение за окно не дотягивает до бегства.
        assert_eq!(drive(&mut flight, 0.0, 30.0, circle, false, true), (false, false));
    }

    #[test]
    fn враг_в_30_м_нет() {
        let mut flight = Flight::default();
        assert_eq!(drive(&mut flight, 0.0, 20.0, straight(14.0), true, true), (false, false));
    }

    #[test]
    fn врагов_нет_нет() {
        let mut flight = Flight::default();
        assert_eq!(drive(&mut flight, 0.0, 20.0, straight(14.0), false, false), (false, false));
    }

    #[test]
    fn перехват_не_мигает_при_скорости_7_мс() {
        let mut flight = Flight::default();
        drive(&mut flight, 0.0, 12.0, straight(14.0), false, true);
        assert!(flight.active);
        // Удары замедлили луноход до 7 м/с: перехват держится.
        let base = 14.0 * 12.0;
        let slow = move |t: f32| Vec2::new(base + 7.0 * (t - 12.0), 0.0);
        let mut steps = 0;
        for i in 0..(10.0 / DT) as usize {
            let t = 12.0 + i as f32 * DT;
            flight.step(DT, slow(t), false, true);
            steps += usize::from(flight.active);
        }
        assert_eq!(steps, (10.0 / DT) as usize);
    }

    #[test]
    fn конец_при_подходе_обычного_врага() {
        let mut flight = Flight::default();
        drive(&mut flight, 0.0, 12.0, straight(14.0), false, true);
        assert!(flight.active);
        flight.step(DT, Vec2::new(14.0 * 12.0, 0.0), true, true);
        assert!(!flight.active);
    }

    #[test]
    fn звено_появляется_впереди_сбоку() {
        let travel = Vec2::new(3.0, -4.0);
        let dir = travel.normalize();
        for side in [1.0, -1.0] {
            for offset in [balance::FLANK_ANGLE.0, balance::FLANK_ANGLE.1] {
                let p = enemy::ring_point(Vec3::ZERO, flank_angle(travel, offset, side), 50.0);
                let p = Vec2::new(p.x, p.z);
                let ahead = p.dot(dir);
                let aside = p.perp_dot(dir).abs();
                // Сбоку, а не позади: самое большее чуть за траверзом.
                assert!(aside > 50.0 * 0.75, "{aside}");
                assert!(ahead > -50.0 * 0.2, "{ahead}");
            }
        }
    }

    #[test]
    fn lead_point_ahead_of_a_moving_rover() {
        let me = Vec3::new(-30.0, 0.0, 0.0);
        let rover = Vec3::ZERO;
        // Стоящий луноход — цель он сам.
        assert_eq!(lead_point(rover, Vec3::ZERO, me), rover);
        // Едет поперёк: точка впереди по его ходу, упреждение не больше предела.
        let velocity = Vec3::new(0.0, 0.0, -14.0);
        let lead = lead_point(rover, velocity, me);
        assert!(lead.z < -5.0, "{lead}");
        assert!(lead.distance(rover) <= 14.0 * balance::LEAD_MAX + 1e-3);
        // Далеко: упреждение упирается в предел.
        let far = lead_point(rover, velocity, Vec3::new(-500.0, 0.0, 0.0));
        assert!((far.distance(rover) - 14.0 * balance::LEAD_MAX).abs() < 1e-3);
    }

    #[test]
    fn удар_носом() {
        // Перехватчик едет на -Z, луноход перед носом.
        let rover_radius = 2.4;
        let contact = segment_contact(Vec2::new(0.0, 3.8), Vec2::NEG_Y, Vec2::ZERO, rover_radius);
        assert!(contact.is_some());
    }

    #[test]
    fn въезд_лунохода_в_борт_удар() {
        // Перехватчик лёг поперёк пути (курс вдоль X), луноход упёрся в борт у хвоста.
        let contact = segment_contact(Vec2::new(0.0, -2.8), Vec2::X, Vec2::new(-1.5, 0.0), 2.4);
        let (normal, depth) = contact.unwrap();
        assert!(depth > 0.0);
        // Выталкивание — от лунохода к корпусу.
        assert!(normal.y < -0.9, "{normal}");
    }

    #[test]
    fn рядом_но_не_касается_нет() {
        assert!(segment_contact(Vec2::new(0.0, -4.5), Vec2::X, Vec2::ZERO, 2.4).is_none());
        // Вдоль корпуса за хвостом.
        assert!(segment_contact(Vec2::new(6.0, 0.0), Vec2::X, Vec2::ZERO, 2.4).is_none());
    }

    #[test]
    fn рывок_идёт_в_точку_начала_рывка() {
        let start = Vec3::new(3.0, 0.0, 4.0);
        let state = gadfly_step(Gadfly::Charge(0.05), 0.1, start, false);
        let Gadfly::Dash { target, .. } = state else {
            panic!("после разгона — рывок, а не {state:?}");
        };
        assert_eq!(target, start);
        // Луноход уехал — цель рывка прежняя.
        let later = gadfly_step(state, 0.1, Vec3::new(20.0, 0.0, 0.0), false);
        assert!(matches!(later, Gadfly::Dash { target, .. } if target == start));
        // У цели — отход.
        assert!(matches!(gadfly_step(later, 0.1, Vec3::ZERO, true), Gadfly::Retreat(_)));
    }

    #[test]
    fn оводы_с_угрозы_1_2() {
        assert_eq!(balance::gadflies_wanted(1.0), 0);
        assert_eq!(balance::gadflies_wanted(1.5), 2);
    }
}
