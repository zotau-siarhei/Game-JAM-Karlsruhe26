//! Луноход игрока: танковое управление, посадка на рельеф и наклон по склону.

use bevy::prelude::*;

use crate::frame::GameSet;
use crate::run::WorldRebuilt;
use crate::waves::balance;
use crate::terrain::Terrain;

/// Высота центра лунохода над поверхностью, м: от неё отсчитываются башня, дуло и облик.
pub const RIDE_HEIGHT: f32 = 0.48;

/// Во сколько раз луноход больше своей модели: масштаб корня, облик и башня растут вместе с ним.
pub const ROVER_SCALE: f32 = 2.0;

/// Максимальная скорость вперёд, м/с.
pub const MAX_FORWARD_SPEED: f32 = 14.0;

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

/// Замедление от удара перехватчика: пока `left > 0`, потолок скорости режется до
/// [`balance::IMPACT_FACTOR`]. Отдельно от [`SpeedFactor`]: его снимает конец ремонта.
#[derive(Component, Default, Debug, Clone, Copy)]
pub struct Impact {
    /// Сколько ещё действует ограничение, с.
    pub left: f32,
}

impl Impact {
    /// Удар: ограничение продлевается, но копится не дольше [`balance::IMPACT_MAX`].
    pub fn hit(&mut self) {
        self.left = (self.left + balance::IMPACT_TIME).min(balance::IMPACT_MAX);
    }

    /// Множитель потолка скорости от удара.
    pub fn factor(&self) -> f32 {
        if self.left > 0.0 { balance::IMPACT_FACTOR } else { 1.0 }
    }
}

/// Множитель потолка скорости: ремонт и удар складываются.
fn speed_cap(repair: Option<&SpeedFactor>, impact: Option<&Impact>) -> f32 {
    repair.map_or(1.0, |factor| factor.0) * impact.map_or(1.0, Impact::factor)
}

/// Плагин лунохода.
pub struct RoverPlugin;

impl Plugin for RoverPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_rover)
            // Вне забега луноходом не управляют: в меню он спрятан и стоит на старте.
            .add_systems(Update, (reset_impact.in_set(GameSet::Reset), drive_rover.in_set(GameSet::Input)));
    }
}

/// Ставит луноход на поверхность в начале координат. Облик добавляет `rover_look`.
fn spawn_rover(mut commands: Commands, terrain: Res<Terrain>) {
    let up = terrain.normal_at(0.0, 0.0);
    let y = terrain.height_at(0.0, 0.0) + RIDE_HEIGHT * ROVER_SCALE;
    commands.spawn((
        Rover,
        RoverMotion {
            up,
            ..default()
        },
        Impact::default(),
        Transform::from_xyz(0.0, y, 0.0)
            .with_rotation(Quat::from_rotation_arc(Vec3::Y, up))
            .with_scale(Vec3::splat(ROVER_SCALE)),
        Visibility::default(),
    ));
}

/// Новый мир: замедление от удара прошлого забега снято.
fn reset_impact(mut rebuilt: MessageReader<WorldRebuilt>, mut rover: Query<&mut Impact, With<Rover>>) {
    if rebuilt.read().count() == 0 {
        return;
    }
    for mut impact in &mut rover {
        impact.left = 0.0;
    }
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
    rover: Single<(&mut Transform, &mut RoverMotion, Option<&SpeedFactor>, Option<&mut Impact>), With<Rover>>,
) {
    let (mut transform, mut motion, speed_factor, mut impact) = rover.into_inner();
    let dt = time.delta_secs();
    let cap = speed_cap(speed_factor, impact.as_deref());
    if let Some(impact) = impact.as_mut() {
        impact.left = (impact.left - dt).max(0.0);
    }

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
    } * cap;
    let accel = MAX_FORWARD_SPEED / ACCEL_TIME * dt;
    motion.speed += (target_speed - motion.speed).clamp(-accel, accel);

    let yaw = Quat::from_rotation_y(motion.heading);
    let forward = yaw * Vec3::NEG_Z;
    let mut pos = transform.translation + forward * motion.speed * dt;

    // Наклон: средняя нормаль под колёсами, ограниченная по углу и сглаженная по времени.
    let mut normal = WHEEL_POINTS
        .iter()
        .map(|p| {
            let w = pos + yaw * Vec3::new(p.x, 0.0, p.y) * ROVER_SCALE;
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

    pos.y = terrain.height_at(pos.x, pos.z) + RIDE_HEIGHT * ROVER_SCALE;
    transform.translation = pos;
    transform.rotation = Quat::from_rotation_arc(Vec3::Y, motion.up) * yaw;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn удар_режет_потолок_до_0_55() {
        let mut impact = Impact::default();
        assert_eq!(speed_cap(None, Some(&impact)), 1.0);
        impact.hit();
        assert!((speed_cap(None, Some(&impact)) - 0.55).abs() < 1e-6);
    }

    #[test]
    fn повторные_удары_продлевают_но_не_опускают_ниже_и_не_дольше_3_с() {
        let mut impact = Impact::default();
        for _ in 0..5 {
            impact.hit();
        }
        assert!((impact.left - balance::IMPACT_MAX).abs() < 1e-6);
        assert!((impact.factor() - balance::IMPACT_FACTOR).abs() < 1e-6);
    }

    #[test]
    fn ремонт_и_удар_складываются_конец_ремонта_не_снимает_удар() {
        let mut impact = Impact::default();
        impact.hit();
        let repair = SpeedFactor(0.5);
        assert!((speed_cap(Some(&repair), Some(&impact)) - 0.5 * 0.55).abs() < 1e-6);
        // Ремонт кончился — компонент снят, удар ещё действует.
        assert!((speed_cap(None, Some(&impact)) - 0.55).abs() < 1e-6);
    }
}
