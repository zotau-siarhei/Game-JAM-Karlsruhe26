//! Наземная машина: едет по рельефу, стоит на поверхности и наклоняется по склону.
//!
//! Тот, кто управляет машиной, пишет в [`GroundVehicle`] желаемую скорость и курс, а система
//! [`move_ground_vehicles`] двигает машину и сажает её на рельеф. Так враги съезжают в кратеры
//! и выезжают из них, не застревая и не переворачиваясь.

use bevy::prelude::*;

use crate::terrain::Terrain;

/// Предельный наклон корпуса от вертикали, рад.
const MAX_TILT: f32 = 35.0 * std::f32::consts::PI / 180.0;

/// Скорость, с которой наклон догоняет склон, 1/с.
const TILT_SMOOTHING: f32 = 10.0;

/// Машина, которая едет по рельефу.
#[derive(Component, Debug, Clone)]
pub struct GroundVehicle {
    /// Скорость вдоль корпуса, м/с.
    pub speed: f32,
    /// Курс: поворот вокруг вертикали, рад. Ноль — нос смотрит на -Z.
    pub heading: f32,
    /// Сглаженная ось «вверх» корпуса.
    pub up: Vec3,
    /// Высота центра корпуса над поверхностью, м.
    pub ride_height: f32,
    /// Половина ширины и половина длины опоры, м: по этим точкам считается наклон.
    pub half_base: Vec2,
}

impl GroundVehicle {
    pub fn new(heading: f32, ride_height: f32, half_base: Vec2) -> Self {
        Self {
            speed: 0.0,
            heading,
            up: Vec3::Y,
            ride_height,
            half_base,
        }
    }

    /// Направление «вперёд» по курсу.
    pub fn forward(&self) -> Vec3 {
        forward_of(self.heading)
    }
}

/// Направление «вперёд» для курса: ноль — на -Z, положительный курс поворачивает влево.
pub fn forward_of(heading: f32) -> Vec3 {
    Quat::from_rotation_y(heading) * Vec3::NEG_Z
}

/// Курс, при котором машина смотрит вдоль горизонтального направления `direction`.
pub fn heading_towards(direction: Vec3) -> f32 {
    (-direction.x).atan2(-direction.z)
}

/// Поворачивает курс к цели не больше чем на `max_turn` радиан, по кратчайшей дуге.
pub fn steer(heading: f32, target: f32, max_turn: f32) -> f32 {
    let diff = (target - heading + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    heading + diff.clamp(-max_turn, max_turn)
}

/// Двигает машины по курсу и сажает их на рельеф с наклоном по склону.
pub fn move_ground_vehicles(
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut vehicles: Query<(&mut Transform, &mut GroundVehicle)>,
) {
    let dt = time.delta_secs();
    for (mut transform, mut vehicle) in &mut vehicles {
        let yaw = Quat::from_rotation_y(vehicle.heading);
        let mut position = transform.translation + vehicle.forward() * vehicle.speed * dt;

        // Средняя нормаль под четырьмя углами опоры, ограниченная по углу и сглаженная.
        let base = vehicle.half_base;
        let corners = [
            Vec3::new(-base.x, 0.0, -base.y),
            Vec3::new(base.x, 0.0, -base.y),
            Vec3::new(-base.x, 0.0, base.y),
            Vec3::new(base.x, 0.0, base.y),
        ];
        let mut normal = corners
            .iter()
            .map(|corner| {
                let point = position + yaw * *corner;
                terrain.normal_at(point.x, point.z)
            })
            .sum::<Vec3>()
            .normalize_or(Vec3::Y);
        let tilt = normal.angle_between(Vec3::Y);
        if tilt > MAX_TILT {
            normal = Vec3::Y.slerp(normal, MAX_TILT / tilt);
        }
        let blend = 1.0 - (-TILT_SMOOTHING * dt).exp();
        vehicle.up = vehicle.up.lerp(normal, blend).normalize_or(Vec3::Y);

        position.y = terrain.height_at(position.x, position.z) + vehicle.ride_height;
        transform.translation = position;
        transform.rotation = Quat::from_rotation_arc(Vec3::Y, vehicle.up) * yaw;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, PI};

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn heading_points_at_direction() {
        for direction in [Vec3::NEG_Z, Vec3::X, Vec3::Z, Vec3::NEG_X, Vec3::new(1.0, 0.0, -1.0).normalize()] {
            let forward = forward_of(heading_towards(direction));
            assert!(forward.distance(direction) < 1e-4, "{direction} -> {forward}");
        }
    }

    #[test]
    fn steering_is_limited_and_takes_the_short_way() {
        assert!(close(steer(0.0, 1.0, 0.25), 0.25));
        assert!(close(steer(0.0, 0.1, 0.25), 0.1));
        // Цель чуть «за спиной» слева: поворот идёт через PI, а не почти полный круг вправо.
        let turned = steer(PI - 0.1, -PI + 0.1, 0.5);
        assert!(close(turned, PI + 0.1));
        assert!(close(steer(0.0, -FRAC_PI_2, 0.3), -0.3));
    }

    #[test]
    fn vehicle_sits_on_terrain_and_moves_forward() {
        let mut app = App::new();
        app.insert_resource(Terrain::new(7))
            .init_resource::<Time>()
            .add_systems(Update, move_ground_vehicles);
        let mut vehicle = GroundVehicle::new(0.0, 0.5, Vec2::new(0.6, 0.9));
        vehicle.speed = 10.0;
        let entity = app.world_mut().spawn((Transform::from_xyz(3.0, 100.0, 5.0), vehicle)).id();

        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(100));
        app.update();

        let position = app.world().get::<Transform>(entity).unwrap().translation;
        // Курс ноль — движение на -Z.
        assert!(close(position.x, 3.0));
        assert!(close(position.z, 4.0));
        let ground = app.world().resource::<Terrain>().height_at(position.x, position.z);
        assert!(close(position.y, ground + 0.5));
        let up = app.world().get::<Transform>(entity).unwrap().up();
        assert!(up.angle_between(Vec3::Y) <= MAX_TILT + 1e-3);
    }
}
