//! Облик врагов в движении: колёса машин и шаг робота-ракетчика.
//!
//! Корпуса таран-стрелка и мортирщика — модели без колёс, колёса — отдельная модель
//! `enemy_wheel`, по одной сущности на колесо. Колесо крутится на пройденный путь, передние
//! поворачивают по скорости разворота корпуса, как у настоящей машины.
//!
//! Робот-ракетчик — модель Meshy со скелетом Mixamo и клипами стояния, ходьбы и бега на месте,
//! как у жителей (`src/resident_anim.rs`). Клип выбирается по скорости из `GroundVehicle`, темп
//! клипа подгоняется под неё, чтобы ступни не скользили.

use std::time::Duration;

use bevy::gltf::Gltf;
use bevy::prelude::*;

use crate::frame::GameSet;
use crate::models::Models;
use crate::vehicle::GroundVehicle;

/// Модель колеса: диаметр 1 м, ось вдоль X.
const WHEEL_MODEL: &str = "enemy_wheel";
/// Модель робота-ракетчика.
const ROBOT_MODEL: &str = "enemy_rocket_robot";

/// Предельный поворот передних колёс, рад.
const MAX_STEER: f32 = 0.5;
/// Сглаживание поворота колёс, 1/с.
const STEER_SMOOTHING: f32 = 8.0;

/// Клипы робота.
const IDLE_CLIP: &str = "Idle_02";
const WALK_CLIP: &str = "Walking";
const RUN_CLIP: &str = "Running";
/// Скорость тела в клипах ходьбы и бега при темпе 1, м/с: скорость ступни в фазе опоры по скелету
/// модели (1,41 и 5,38 в единицах модели), умноженная на масштаб модели 1,1 из `models.json`.
const CLIP_WALK_SPEED: f32 = 1.55;
const CLIP_RUN_SPEED: f32 = 5.9;
/// Ниже этой скорости робот стоит, выше второй — бежит, м/с.
const STAND_SPEED: f32 = 0.4;
const RUN_FROM: f32 = 3.5;
/// Пределы темпа клипа.
const RATE_LIMITS: (f32, f32) = (0.6, 1.6);
/// Переход между клипами, с.
const BLEND: f32 = 0.2;

/// Подключает облик врагов в движении.
pub struct EnemyLookPlugin;

impl Plugin for EnemyLookPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RobotClips>().add_systems(
            Update,
            (
                roll_wheels,
                (build_robot_graph, attach_robot_players, drive_robots).chain(),
            )
                .in_set(GameSet::Presentation),
        );
    }
}

/// Колёса машины в системе координат корпуса: нос смотрит на -Z.
pub struct WheelLayout {
    /// Радиус колеса, м.
    pub radius: f32,
    /// Расстояние от продольной оси до середины колеса, м.
    pub half_track: f32,
    /// Оси: положение вдоль корпуса, м, и поворачивают ли колёса этой оси.
    pub axles: &'static [(f32, bool)],
}

/// Колёса бронебагги таран-стрелка: арки измерены по модели `enemy_rammer`
/// (масштаб 2,52, оси на -0,24 и 0,30, колея ±0,205, радиус 0,1 в единицах модели).
pub const RAMMER_WHEELS: WheelLayout = WheelLayout {
    radius: 0.252,
    half_track: 0.517,
    axles: &[(-0.605, true), (0.756, false)],
};

/// Колёса шестиколёсного транспортёра мортирщика: модель `enemy_mortar`
/// (масштаб 2,72, оси на -0,30, 0,095 и 0,31, колея ±0,195, радиус 0,085).
pub const MORTAR_WHEELS: WheelLayout = WheelLayout {
    radius: 0.231,
    half_track: 0.53,
    axles: &[(-0.816, true), (0.258, false), (0.843, false)],
};

/// Колесо машины врага: дочерняя сущность корпуса.
#[derive(Component)]
pub struct Wheel {
    radius: f32,
    steers: bool,
    /// Колесо слева: модель развёрнута ступицей наружу.
    left: bool,
    /// Угол качения, рад.
    roll: f32,
    /// Угол поворота, рад.
    steer: f32,
    /// Курс корпуса в прошлом кадре: по нему считается скорость разворота.
    last_heading: Option<f32>,
}

/// Ставит колёса машины. `ground_y` — высота земли в системе корпуса; `fallback` — сетка
/// диаметром 1 м с осью вдоль Y и материал на время загрузки модели колеса.
pub fn spawn_wheels(
    parent: &mut ChildSpawnerCommands,
    models: &mut Models,
    layout: &WheelLayout,
    ground_y: f32,
    fallback: (Handle<Mesh>, Handle<StandardMaterial>),
) {
    for &(along, steers) in layout.axles {
        for side in [-1.0, 1.0] {
            let left = side < 0.0;
            parent
                .spawn((
                    Wheel {
                        radius: layout.radius,
                        steers,
                        left,
                        roll: 0.0,
                        steer: 0.0,
                        last_heading: None,
                    },
                    Transform::from_xyz(side * layout.half_track, ground_y + layout.radius, along)
                        .with_rotation(wheel_rotation(0.0, 0.0, left))
                        .with_scale(Vec3::splat(layout.radius * 2.0)),
                    Visibility::default(),
                ))
                .with_children(|wheel| {
                    let (mesh, material) = fallback.clone();
                    models.spawn_visual(wheel, WHEEL_MODEL, Transform::IDENTITY, |spare| {
                        spare.spawn((
                            Mesh3d(mesh),
                            MeshMaterial3d(material),
                            Transform::from_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
                        ));
                    });
                });
        }
    }
}

/// Поворот колеса: руль вокруг вертикали, качение вокруг оси, левые развёрнуты ступицей наружу.
fn wheel_rotation(steer: f32, roll: f32, left: bool) -> Quat {
    let outward = if left {
        Quat::from_rotation_y(std::f32::consts::PI)
    } else {
        Quat::IDENTITY
    };
    // Вперёд — это -Z: верх колеса уходит вперёд при повороте вокруг X на отрицательный угол.
    Quat::from_rotation_y(steer) * Quat::from_rotation_x(-roll) * outward
}

/// Угол поворота передних колёс по скорости и скорости разворота: велосипедная модель.
fn steer_angle(speed: f32, yaw_rate: f32, wheelbase: f32) -> f32 {
    if speed.abs() < 0.5 {
        return 0.0;
    }
    (wheelbase * yaw_rate / speed).atan().clamp(-MAX_STEER, MAX_STEER)
}

/// Крутит колёса на пройденный путь и поворачивает передние.
fn roll_wheels(
    time: Res<Time>,
    vehicles: Query<&GroundVehicle>,
    mut wheels: Query<(&mut Wheel, &mut Transform, &ChildOf)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for (mut wheel, mut transform, child_of) in &mut wheels {
        let Ok(vehicle) = vehicles.get(child_of.parent()) else {
            continue;
        };
        wheel.roll = (wheel.roll + vehicle.speed * dt / wheel.radius) % std::f32::consts::TAU;
        if wheel.steers {
            let yaw = wheel.last_heading.map_or(0.0, |last| {
                let turn = (vehicle.heading - last + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
                    - std::f32::consts::PI;
                turn / dt
            });
            wheel.last_heading = Some(vehicle.heading);
            // База — расстояние от передней оси до задней, у обеих машин около 1,4–1,7 м.
            let target = steer_angle(vehicle.speed, yaw, 1.5);
            wheel.steer += (target - wheel.steer) * (STEER_SMOOTHING * dt).min(1.0);
        }
        transform.rotation = wheel_rotation(wheel.steer, wheel.roll, wheel.left);
    }
}

/// Робот, которого анимирует скелет. Ставится на врага-ракетчика.
#[derive(Component, Default)]
pub struct Walker {
    /// Сущность с `AnimationPlayer` в сцене модели, когда она загружена.
    player: Option<Entity>,
}

/// Граф анимаций робота и узлы клипов.
#[derive(Resource, Default)]
struct RobotClips(Option<RobotGraph>);

struct RobotGraph {
    graph: Handle<AnimationGraph>,
    idle: AnimationNodeIndex,
    walk: AnimationNodeIndex,
    run: AnimationNodeIndex,
}

/// Что делает робот.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Gait {
    Stand,
    Walk,
    Run,
}

/// Длительность клипов робота, с.
fn clip_length(gait: Gait) -> f32 {
    match gait {
        Gait::Stand => 1.867,
        Gait::Walk => 1.033,
        Gait::Run => 0.633,
    }
}

/// Клип и его темп по скорости робота.
fn gait(speed: f32) -> (Gait, f32) {
    let speed = speed.abs();
    let rate = |natural: f32| (speed / natural).clamp(RATE_LIMITS.0, RATE_LIMITS.1);
    if speed < STAND_SPEED {
        (Gait::Stand, 1.0)
    } else if speed > RUN_FROM {
        (Gait::Run, rate(CLIP_RUN_SPEED))
    } else {
        (Gait::Walk, rate(CLIP_WALK_SPEED))
    }
}

/// Строит граф анимаций робота, когда его модель загружена.
fn build_robot_graph(
    mut clips: ResMut<RobotClips>,
    models: Models,
    walkers: Query<(), With<Walker>>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    if clips.0.is_some() || walkers.is_empty() {
        return;
    }
    let Some(gltf) = models.gltf(ROBOT_MODEL).and_then(|h| gltfs.get(&h)) else {
        return;
    };
    let clip = |name: &str| gltf.named_animations.get(name).cloned();
    let (Some(idle), Some(walk), Some(run)) = (clip(IDLE_CLIP), clip(WALK_CLIP), clip(RUN_CLIP)) else {
        warn_once!("Робот: в модели «{ROBOT_MODEL}» нет клипов {IDLE_CLIP}, {WALK_CLIP}, {RUN_CLIP}");
        return;
    };
    let (graph, nodes) = AnimationGraph::from_clips([idle, walk, run]);
    clips.0 = Some(RobotGraph {
        graph: graphs.add(graph),
        idle: nodes[0],
        walk: nodes[1],
        run: nodes[2],
    });
}

/// Вешает граф на `AnimationPlayer` в сцене робота и запускает стояние со своей фазой.
fn attach_robot_players(
    mut commands: Commands,
    clips: Res<RobotClips>,
    players: Query<Entity, (With<AnimationPlayer>, Without<AnimationGraphHandle>)>,
    parents: Query<&ChildOf>,
    mut walkers: Query<&mut Walker>,
) {
    let Some(clips) = &clips.0 else {
        return;
    };
    for player in &players {
        let Some(robot) = parents.iter_ancestors(player).find(|&e| walkers.contains(e)) else {
            continue;
        };
        let Ok(mut walker) = walkers.get_mut(robot) else {
            continue;
        };
        let mut transitions = AnimationTransitions::new();
        let mut animation = AnimationPlayer::default();
        transitions
            .play(&mut animation, clips.idle, Duration::ZERO)
            .repeat()
            .seek_to(phase(robot) * clip_length(Gait::Stand));
        commands
            .entity(player)
            .insert((AnimationGraphHandle(clips.graph.clone()), transitions, animation));
        walker.player = Some(player);
    }
}

/// Своя фаза клипа от 0 до 1: роботы не шагают в ногу.
fn phase(robot: Entity) -> f32 {
    (robot.index().index() as f32 * 0.618_034).fract()
}

/// Выбирает клип по скорости робота и подгоняет темп.
fn drive_robots(
    clips: Res<RobotClips>,
    walkers: Query<(Entity, &Walker, &GroundVehicle)>,
    mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
) {
    let Some(clips) = &clips.0 else {
        return;
    };
    for (robot, walker, vehicle) in &walkers {
        let Some(Ok((mut player, mut transitions))) = walker.player.map(|p| players.get_mut(p)) else {
            continue;
        };
        let (gait, rate) = gait(vehicle.speed);
        let node = match gait {
            Gait::Stand => clips.idle,
            Gait::Walk => clips.walk,
            Gait::Run => clips.run,
        };
        if transitions.get_main_animation() != Some(node) {
            transitions
                .play(&mut player, node, Duration::from_secs_f32(BLEND))
                .repeat()
                .seek_to(phase(robot) * clip_length(gait));
        }
        if let Some(active) = player.animation_mut(node) {
            active.set_speed(rate);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn передние_колёса_поворачивают_в_сторону_разворота() {
        // Разворот налево (курс растёт) — колёса налево, назад — наоборот.
        assert!(steer_angle(8.0, 1.0, 1.5) > 0.0);
        assert!(steer_angle(-3.0, 1.0, 1.5) < 0.0);
        assert_eq!(steer_angle(0.1, 2.0, 1.5), 0.0);
        assert_eq!(steer_angle(1.0, 50.0, 1.5), MAX_STEER);
    }

    #[test]
    fn верх_колеса_уходит_вперёд() {
        let top = wheel_rotation(0.0, 0.3, false) * Vec3::Y;
        assert!(top.z < 0.0);
        let top = wheel_rotation(0.0, 0.3, true) * Vec3::Y;
        assert!(top.z < 0.0);
    }

    #[test]
    fn робот_стоит_идёт_бежит() {
        assert_eq!(gait(0.0).0, Gait::Stand);
        assert_eq!(gait(2.0).0, Gait::Walk);
        assert_eq!(gait(7.5).0, Gait::Run);
        assert!((gait(7.5).1 * CLIP_RUN_SPEED - 7.5).abs() < 1e-4);
    }
}
