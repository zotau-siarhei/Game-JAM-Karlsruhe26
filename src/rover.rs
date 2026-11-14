//! Луноход игрока: танковое управление, посадка на рельеф и наклон по склону.

use bevy::prelude::*;

use crate::terrain::Terrain;

/// Цвет корпуса лунохода.
const ROVER_COLOR: Color = Color::srgb(0.75, 0.78, 0.82);

/// Цвет колёс и мачты.
const WHEEL_COLOR: Color = Color::srgb(0.12, 0.12, 0.14);

/// Размер корпуса: ширина, высота, длина, м.
const HULL_SIZE: Vec3 = Vec3::new(1.2, 0.6, 1.8);

/// Радиус колеса, м.
const WHEEL_RADIUS: f32 = 0.3;

/// Ширина колеса, м.
const WHEEL_WIDTH: f32 = 0.25;

/// Насколько корпус приподнят над поверхностью: колёса под ним, м.
const RIDE_HEIGHT: f32 = HULL_SIZE.y * 0.5 + WHEEL_RADIUS * 0.6;

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

/// Точки касания колёс в системе корпуса (x вбок, z назад), м.
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

/// Плагин лунохода.
pub struct RoverPlugin;

impl Plugin for RoverPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_rover)
            .add_systems(Update, drive_rover);
    }
}

/// Ставит луноход на поверхность в начале координат.
fn spawn_rover(
    mut commands: Commands,
    terrain: Res<Terrain>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let hull = meshes.add(Cuboid::from_size(HULL_SIZE));
    let wheel = meshes.add(Cylinder::new(WHEEL_RADIUS, WHEEL_WIDTH));
    let mast = meshes.add(Cuboid::new(0.15, 0.6, 0.15));
    let body_material = materials.add(ROVER_COLOR);
    let wheel_material = materials.add(WHEEL_COLOR);

    let up = terrain.normal_at(0.0, 0.0);
    let y = terrain.height_at(0.0, 0.0) + RIDE_HEIGHT;
    let mut rover = commands.spawn((
        Rover,
        RoverMotion {
            up,
            ..default()
        },
        Mesh3d(hull),
        MeshMaterial3d(body_material),
        Transform::from_xyz(0.0, y, 0.0).with_rotation(Quat::from_rotation_arc(Vec3::Y, up)),
    ));
    rover.with_children(|parent| {
        for p in WHEEL_POINTS {
            let side = p.x.signum() * (HULL_SIZE.x + WHEEL_WIDTH) * 0.5;
            parent.spawn((
                Mesh3d(wheel.clone()),
                MeshMaterial3d(wheel_material.clone()),
                Transform::from_xyz(side, -HULL_SIZE.y * 0.5, p.y * 0.8)
                    .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
            ));
        }
        // Мачта у носа показывает, куда смотрит луноход.
        parent.spawn((
            Mesh3d(mast),
            MeshMaterial3d(wheel_material),
            Transform::from_xyz(0.0, HULL_SIZE.y * 0.5 + 0.3, -HULL_SIZE.z * 0.35),
        ));
    });
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
    rover: Single<(&mut Transform, &mut RoverMotion), With<Rover>>,
) {
    let (mut transform, mut motion) = rover.into_inner();
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
    };
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
