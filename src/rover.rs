//! Луноход игрока: танковое управление, посадка на рельеф и наклон по склону.

use bevy::prelude::*;

use crate::run::GameState;

use crate::terrain::Terrain;

/// Высота центра лунохода над поверхностью, м: от неё отсчитываются башня, дуло и облик.
pub const RIDE_HEIGHT: f32 = 0.48;

/// Максимальная скорость вперёд, м/с.
const MAX_FORWARD_SPEED: f32 = 14.0;

/// Максимальная скорость назад, м/с.
const MAX_REVERSE_SPEED: f32 = 7.0;

/// Скорость поворота корпуса, рад/с.
const TURN_SPEED: f32 = 2.8;

/// Время разгона до максимальной скорости и торможения до нуля, с.
const ACCEL_TIME: f32 = 0.3;

/// Предельный наклон корпуса от вертикали, рад: луноход не переворачивается.
const MAX_TILT: f32 = 35.0 * std::f32::consts::PI / 180.0;

/// Скорость, с которой наклон догоняет склон, 1/с.
const TILT_SMOOTHING: f32 = 10.0;

/// Точки касания гусениц в системе корпуса (x вбок, z назад), м: по ним считается наклон.
const WHEEL_POINTS: [Vec2; 4] = [
    Vec2::new(-0.6, -0.9),
    Vec2::new(0.6, -0.9),
    Vec2::new(-0.6, 0.9),
    Vec2::new(0.6, 0.9),
];

/// Луноход, которым управляет игрок.
#[derive(Component, Default)]
pub struct Rover;

/// Состояние движения лунохода.
#[derive(Component, Default, Debug)]
pub struct RoverMotion {
    /// Скорость вдоль корпуса, м/с: больше нуля — вперёд, меньше — назад.
    pub speed: f32,
    /// Курс: поворот вокруг вертикали, рад. Ноль — нос смотрит на -Z (север).
    pub heading: f32,
    /// Сглаженная ось «вверх» корпуса.
    pub up: Vec3,
}

/// Множитель скорости лунохода: меньше единицы — луноход едет медленнее, например при ремонте.
#[derive(Component, Debug, Clone, Copy)]
pub struct SpeedFactor(pub f32);

/// Плагин лунохода.
pub struct RoverPlugin;

impl Plugin for RoverPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_rover)
            // Вне забега луноходом не управляют: в меню его двигает фон, после гибели он стоит.
            .add_systems(Update, drive_rover.run_if(in_state(GameState::Playing)));
    }
}

/// Ставит луноход на поверхность в начале координат. Облик добавляет `rover_look`.
fn spawn_rover(mut commands: Commands, terrain: Res<Terrain>) {
    let up = terrain.normal_at(0.0, 0.0);
    let y = terrain.height_at(0.0, 0.0) + RIDE_HEIGHT;
    commands.spawn((
        Rover,
        RoverMotion {
            up,
            ..default()
        },
        Transform::from_xyz(0.0, y, 0.0).with_rotation(Quat::from_rotation_arc(Vec3::Y, up)),
        Visibility::default(),
    ));
}

/// Ось управления по двум клавишам с запасными стрелками: от -1 до 1.
fn axis(keys: &ButtonInput<KeyCode>, plus: [KeyCode; 2], minus: [KeyCode; 2]) -> f32 {
    let plus = keys.any_pressed(plus) as i32 as f32;
    let minus = keys.any_pressed(minus) as i32 as f32;
    plus - minus
}

/// Танковое управление, посадка на рельеф и наклон по склону.
pub fn drive_rover(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    terrain: Res<Terrain>,
    rover: Single<(&mut Transform, &mut RoverMotion, Option<&SpeedFactor>), With<Rover>>,
) {
    let (mut transform, mut motion, speed_factor) = rover.into_inner();
    let dt = time.delta_secs();

    let throttle = axis(&keys, [KeyCode::KeyW, KeyCode::ArrowUp], [KeyCode::KeyS, KeyCode::ArrowDown]);
    let turn = axis(&keys, [KeyCode::KeyA, KeyCode::ArrowLeft], [KeyCode::KeyD, KeyCode::ArrowRight]);

    // Танковый поворот: A всегда крутит корпус против часовой стрелки, даже на заднем ходу.
    motion.heading += turn * TURN_SPEED * dt;

    let target_speed = if throttle > 0.0 {
        MAX_FORWARD_SPEED
    } else if throttle < 0.0 {
        -MAX_REVERSE_SPEED
    } else {
        0.0
    } * speed_factor.map_or(1.0, |factor| factor.0);
    let accel = MAX_FORWARD_SPEED / ACCEL_TIME * dt;
    motion.speed += (target_speed - motion.speed).clamp(-accel, accel);

    let yaw = Quat::from_rotation_y(motion.heading);
    let forward = yaw * Vec3::NEG_Z;
    let mut pos = transform.translation + forward * motion.speed * dt;

    // Наклон: средняя нормаль под колёсами, ограниченная по углу и сглаженная по времени.
    let mut normal = WHEEL_POINTS
        .iter()
        .map(|p| {
            let w = pos + yaw * Vec3::new(p.x, 0.0, p.y);
            terrain.normal_at(w.x, w.z)
        })
        .sum::<Vec3>()
        .normalize();
    let tilt = normal.angle_between(Vec3::Y);
    if tilt > MAX_TILT {
        normal = Vec3::Y.slerp(normal, MAX_TILT / tilt);
    }
    let blend = 1.0 - (-TILT_SMOOTHING * dt).exp();
    motion.up = motion.up.lerp(normal, blend).normalize_or(Vec3::Y);

    pos.y = terrain.height_at(pos.x, pos.z) + RIDE_HEIGHT;
    transform.translation = pos;
    transform.rotation = Quat::from_rotation_arc(Vec3::Y, motion.up) * yaw;
}
