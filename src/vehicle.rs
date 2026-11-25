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

/// Просвет между туловищем на ногах и землёй под его углами, м.
const HULL_CLEARANCE: f32 = 0.1;

/// Глубже этого ступня в землю не уходит, м: на изломе рельефа другие ноги висят.
const FOOT_SINK: f32 = 0.4;

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

/// Опора шагающей машины: высоту корпуса держат четыре ступни, а не земля под центром.
///
/// Без неё воронка точно под корпусом роняла машину на всю глубину чаши: у ступней и у края
/// корпуса земля оставалась выше, и машина уходила в грунт.
#[derive(Component, Debug, Clone, Copy)]
pub struct LegStance {
    /// Половина расстояния между ступнями поперёк и вдоль, м.
    pub feet: Vec2,
    /// Половина ширины и длины туловища, м.
    pub hull: Vec2,
    /// Низ туловища относительно центра машины, м.
    pub hull_bottom: f32,
}

impl LegStance {
    /// Углы прямоугольника `half` на высоте `y` в системе машины.
    fn corners(half: Vec2, y: f32) -> [Vec3; 4] {
        [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)].map(|(x, z)| Vec3::new(x * half.x, y, z * half.y))
    }

    /// Нормаль плоскости, на которой стоят ступни.
    fn normal(&self, ground: impl Fn(f32, f32) -> f32, position: Vec3, yaw: Quat) -> Vec3 {
        let [a, b, c, d] = Self::corners(self.feet, 0.0).map(|local| {
            let p = position + yaw * local;
            ground(p.x, p.z)
        });
        let right = (b + d - a - c) / (4.0 * self.feet.x);
        let back = (c + d - a - b) / (4.0 * self.feet.y);
        yaw * Vec3::new(-right, 1.0, -back).normalize()
    }

    /// Высота центра машины: ступни в среднем на земле, ни одна не глубже [`FOOT_SINK`], туловище
    /// не ниже земли под своими углами.
    fn height(&self, ground: impl Fn(f32, f32) -> f32, position: Vec3, rotation: Quat, ride_height: f32) -> f32 {
        let support = |local: Vec3| {
            let offset = rotation * local;
            ground(position.x + offset.x, position.z + offset.z) - offset.y
        };
        let highest = |heights: [f32; 4]| heights.into_iter().fold(f32::MIN, f32::max);
        let feet = Self::corners(self.feet, -ride_height).map(support);
        let hull = Self::corners(self.hull, self.hull_bottom).map(support);
        (feet.iter().sum::<f32>() / 4.0)
            .max(highest(feet) - FOOT_SINK)
            .max(highest(hull) + HULL_CLEARANCE)
    }

    /// Насколько ступни и низ туловища выше земли при таком положении машины, м: наименьшее по
    /// четырём углам, меньше нуля — ушли в грунт.
    pub fn gaps(&self, ground: impl Fn(f32, f32) -> f32, position: Vec3, rotation: Quat, ride_height: f32) -> (f32, f32) {
        let gap = |local: Vec3| {
            let p = position + rotation * local;
            p.y - ground(p.x, p.z)
        };
        let lowest = |corners: [Vec3; 4]| corners.map(gap).into_iter().fold(f32::MAX, f32::min);
        (
            lowest(Self::corners(self.feet, -ride_height)),
            lowest(Self::corners(self.hull, self.hull_bottom)),
        )
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
    mut vehicles: Query<(&mut Transform, &mut GroundVehicle, Option<&LegStance>)>,
) {
    let dt = time.delta_secs();
    for (mut transform, mut vehicle, stance) in &mut vehicles {
        let yaw = Quat::from_rotation_y(vehicle.heading);
        let mut position = transform.translation + vehicle.forward() * vehicle.speed * dt;

        let ground = |x: f32, z: f32| terrain.height_at(x, z);
        // Средняя нормаль под четырьмя углами опоры, ограниченная по углу и сглаженная. У шагающей
        // машины — плоскость ступней: наклон склона между ними ей не важен.
        let base = vehicle.half_base;
        let corners = [
            Vec3::new(-base.x, 0.0, -base.y),
            Vec3::new(base.x, 0.0, -base.y),
            Vec3::new(-base.x, 0.0, base.y),
            Vec3::new(base.x, 0.0, base.y),
        ];
        let mut normal = match stance {
            Some(stance) => stance.normal(ground, position, yaw),
            None => corners
                .iter()
                .map(|corner| {
                    let point = position + yaw * *corner;
                    terrain.normal_at(point.x, point.z)
                })
                .sum::<Vec3>()
                .normalize_or(Vec3::Y),
        };
        let tilt = normal.angle_between(Vec3::Y);
        if tilt > MAX_TILT {
            normal = Vec3::Y.slerp(normal, MAX_TILT / tilt);
        }
        let blend = 1.0 - (-TILT_SMOOTHING * dt).exp();
        vehicle.up = vehicle.up.lerp(normal, blend).normalize_or(Vec3::Y);

        let rotation = Quat::from_rotation_arc(Vec3::Y, vehicle.up) * yaw;
        position.y = match stance {
            Some(stance) => stance.height(ground, position, rotation, vehicle.ride_height),
            None => ground(position.x, position.z) + vehicle.ride_height,
        };
        transform.translation = position;
        transform.rotation = rotation;
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

    /// Воронка под центром: машина на ногах остаётся на ступнях, туловище не уходит в края.
    #[test]
    fn машина_на_ногах_не_проваливается_в_воронку() {
        let stance = LegStance {
            feet: Vec2::splat(1.7),
            hull: Vec2::new(2.5, 3.3),
            hull_bottom: -0.4,
        };
        let ride = 3.4;
        for (radius, depth) in [(4.0f32, 1.7f32), (4.0, 13.6), (7.0, 4.9), (7.0, 14.7)] {
            let pit = |x: f32, z: f32| {
                let k = (x * x + z * z) / (radius * radius);
                if k < 1.0 { -depth * (1.0 - k) * (1.0 - k) } else { 0.0 }
            };
            let y = stance.height(pit, Vec3::ZERO, Quat::IDENTITY, ride);
            let (feet, hull) = stance.gaps(pit, Vec3::Y * y, Quat::IDENTITY, ride);
            assert!(feet > -FOOT_SINK - 1e-3, "ступни в земле на {feet} м: воронка {radius}×{depth}");
            assert!(hull > HULL_CLEARANCE - 1e-3, "туловище в земле на {hull} м: воронка {radius}×{depth}");
        }
        // На ровном месте — обычная высота.
        assert!(close(stance.height(|_, _| 5.0, Vec3::ZERO, Quat::IDENTITY, ride), 5.0 + ride));
        // На склоне вдоль курса с наклоном по нему все ступни на земле.
        let tilt = Quat::from_rotation_x(0.3);
        let slope = |_: f32, z: f32| -z * 0.3f32.tan();
        let y = stance.height(slope, Vec3::ZERO, tilt, ride);
        let (feet, _) = stance.gaps(slope, Vec3::Y * y, tilt, ride);
        assert!(feet.abs() < 1e-3, "{feet}");
        assert!(stance.normal(slope, Vec3::ZERO, Quat::IDENTITY).distance(tilt * Vec3::Y) < 1e-3);
        // Под одним боком вырыта пропасть: другой бок в землю не уходит.
        let cliff = |x: f32, _: f32| if x > 0.0 { -10.0 } else { 0.0 };
        let normal = stance.normal(cliff, Vec3::ZERO, Quat::IDENTITY);
        let tilt = Quat::from_rotation_arc(Vec3::Y, Vec3::Y.slerp(normal, MAX_TILT / normal.angle_between(Vec3::Y)));
        let y = stance.height(cliff, Vec3::ZERO, tilt, ride);
        let (feet, hull) = stance.gaps(cliff, Vec3::Y * y, tilt, ride);
        assert!(feet > -FOOT_SINK - 1e-3 && hull > 0.0, "{feet} {hull}");
    }
}
