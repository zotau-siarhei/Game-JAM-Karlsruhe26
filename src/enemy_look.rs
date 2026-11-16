//! Облик врагов в движении: колёса машин.
//!
//! Корпуса таран-стрелка и мортирщика — модели без колёс, колёса — отдельная модель
//! `enemy_wheel`, по одной сущности на колесо. Колесо крутится на пройденный путь, передние
//! поворачивают по скорости разворота корпуса, как у настоящей машины.

use bevy::prelude::*;

use crate::frame::GameSet;
use crate::models::Models;
use crate::vehicle::GroundVehicle;

/// Модель колеса: диаметр 1 м, ось вдоль X.
const WHEEL_MODEL: &str = "enemy_wheel";
/// Предельный поворот передних колёс, рад.
const MAX_STEER: f32 = 0.5;
/// Сглаживание поворота колёс, 1/с.
const STEER_SMOOTHING: f32 = 8.0;

/// Подключает облик врагов в движении.
pub struct EnemyLookPlugin;

impl Plugin for EnemyLookPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, roll_wheels.in_set(GameSet::Presentation));
    }
}

/// Колёса машины в системе координат корпуса (до масштаба врага): нос смотрит на -Z.
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
    /// Колёсная база в системе корпуса, м.
    wheelbase: f32,
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
    let along = layout.axles.iter().map(|&(z, _)| z);
    let wheelbase = along.clone().fold(f32::MIN, f32::max) - along.fold(f32::MAX, f32::min);
    for &(along, steers) in layout.axles {
        for side in [-1.0, 1.0] {
            let left = side < 0.0;
            parent
                .spawn((
                    Wheel {
                        radius: layout.radius,
                        wheelbase,
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
    vehicles: Query<(&GroundVehicle, &Transform), Without<Wheel>>,
    mut wheels: Query<(&mut Wheel, &mut Transform, &ChildOf)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for (mut wheel, mut transform, child_of) in &mut wheels {
        let Ok((vehicle, body)) = vehicles.get(child_of.parent()) else {
            continue;
        };
        // Радиус колеса задан в системе корпуса, а корпус бывает увеличен.
        let radius = wheel.radius * body.scale.y;
        wheel.roll = (wheel.roll + vehicle.speed * dt / radius) % std::f32::consts::TAU;
        if wheel.steers {
            let yaw = wheel.last_heading.map_or(0.0, |last| {
                let turn = (vehicle.heading - last + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
                    - std::f32::consts::PI;
                turn / dt
            });
            wheel.last_heading = Some(vehicle.heading);
            let target = steer_angle(vehicle.speed, yaw, wheel.wheelbase * body.scale.y);
            wheel.steer += (target - wheel.steer) * (STEER_SMOOTHING * dt).min(1.0);
        }
        transform.rotation = wheel_rotation(wheel.steer, wheel.roll, wheel.left);
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
}
