//! Препятствия: дома, валуны, деревья и кристаллы останавливают машины и пули.
//!
//! Сущность-препятствие получает [`Obstacle`] с формой в плане и высотой верха. Машины
//! (луноход и всё с `GroundVehicle`) после движения выталкиваются из препятствий по нормали:
//! касательная составляющая движения остаётся, поэтому машина скользит вдоль стены.
//! Пули проверяют путь функцией [`segment_hit`].

use bevy::prelude::*;

use crate::combat::{Collider, Damage, Health, Team};
use crate::enemy::Enemy;
use crate::rover::{self, Rover, RoverMotion};
use std::f32::consts::{PI, TAU};
use crate::settlements::Footprint;
use crate::vehicle::{self, GroundVehicle};

/// Радиус машины без коллайдера, м.
const DEFAULT_VEHICLE_RADIUS: f32 = 1.0;

/// Зазор после выталкивания, м: машина не остаётся на границе из-за погрешности.
const SKIN: f32 = 1e-3;

/// Шаг проверки пути пули, м.
const STEP: f32 = 0.25;

/// Подключает выталкивание машин из препятствий.
pub struct ObstaclesPlugin;

impl Plugin for ObstaclesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (ram_obstacles, push_vehicles_out)
                .chain()
                .after(rover::drive_rover)
                .after(vehicle::move_ground_vehicles),
        );
    }
}

/// Форма препятствия в плане (`x`, `z` мира).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Circle { center: Vec2, radius: f32 },
    Rect(Footprint),
}

impl Shape {
    /// Куда и насколько сдвинуть круг с центром `p` и радиусом `radius`, чтобы он не пересекал форму.
    pub fn push_out(&self, p: Vec2, radius: f32) -> Option<Vec2> {
        match *self {
            Shape::Circle { center, radius: r } => {
                let d = p - center;
                let len = d.length();
                let need = r + radius;
                if len >= need {
                    return None;
                }
                let dir = if len > 1e-4 { d / len } else { Vec2::X };
                Some(dir * (need - len + SKIN))
            }
            Shape::Rect(rect) => {
                let local = rect.to_local(p);
                let h = rect.half;
                let inside = local.x.abs() < h.x && local.y.abs() < h.y;
                let local_push = if inside {
                    // Центр внутри: наружу через ближайшую сторону.
                    let to_x = h.x - local.x.abs();
                    let to_z = h.y - local.y.abs();
                    if to_x < to_z {
                        Vec2::new((to_x + radius + SKIN) * local.x.signum_or_one(), 0.0)
                    } else {
                        Vec2::new(0.0, (to_z + radius + SKIN) * local.y.signum_or_one())
                    }
                } else {
                    let closest = local.clamp(-h, h);
                    let d = local - closest;
                    let len = d.length();
                    if len >= radius {
                        return None;
                    }
                    d / len.max(1e-4) * (radius - len + SKIN)
                };
                let world = Quat::from_rotation_y(rect.yaw) * Vec3::new(local_push.x, 0.0, local_push.y);
                Some(Vec2::new(world.x, world.z))
            }
        }
    }

    /// Лежит ли точка плана внутри формы.
    pub fn contains(&self, p: Vec2) -> bool {
        match *self {
            Shape::Circle { center, radius } => p.distance_squared(center) <= radius * radius,
            Shape::Rect(rect) => rect.distance(p) == 0.0,
        }
    }

    /// Радиус описанной окружности и её центр: для быстрого отсева.
    fn bounds(&self) -> (Vec2, f32) {
        match *self {
            Shape::Circle { center, radius } => (center, radius),
            Shape::Rect(rect) => (rect.center, rect.half.length()),
        }
    }
}

trait SignumOrOne {
    fn signum_or_one(self) -> f32;
}

impl SignumOrOne for f32 {
    fn signum_or_one(self) -> f32 {
        if self < 0.0 { -1.0 } else { 1.0 }
    }
}

/// Препятствие для машин и пуль.
#[derive(Component, Debug, Clone, Copy)]
pub struct Obstacle {
    pub shape: Shape,
    /// Высота верха препятствия в мире: пуля выше пролетает.
    pub top: f32,
}

/// Первая точка отрезка, которая попадает в препятствие, и само препятствие.
pub fn segment_hit<'a>(
    from: Vec3,
    to: Vec3,
    obstacles: impl IntoIterator<Item = (Entity, &'a Obstacle)>,
) -> Option<(Entity, Vec3)> {
    let length = from.distance(to);
    let mid = (from + to) * 0.5;
    let mid = Vec2::new(mid.x, mid.z);
    let steps = (length / STEP).ceil().max(1.0) as usize;
    let mut best: Option<(usize, Entity)> = None;
    for (entity, obstacle) in obstacles {
        let (center, r) = obstacle.shape.bounds();
        if center.distance(mid) > r + length * 0.5 {
            continue;
        }
        let limit = best.map_or(steps, |(s, _)| s);
        let hit = (0..=limit).find(|&s| {
            let p = from.lerp(to, s as f32 / steps as f32);
            p.y <= obstacle.top && obstacle.shape.contains(Vec2::new(p.x, p.z))
        });
        if let Some(s) = hit {
            best = Some((s, entity));
        }
    }
    best.map(|(s, entity)| (entity, from.lerp(to, s as f32 / steps as f32)))
}

/// Выталкивает луноход и машины из препятствий.
/// Луноход сбивает препятствия со здоровьем на своём пути: деревья падают от него,
/// камни и кристаллы разлетаются, дома рушатся. Враги не сбиваются.
///
/// Сбитое препятствие сразу перестаёт быть стеной, но немного сносит луноход и чуть
/// поворачивает его от себя: крупное — сильнее.
fn ram_obstacles(
    mut commands: Commands,
    rover: Single<(&mut Transform, &mut RoverMotion, Option<&Collider>), With<Rover>>,
    obstacles: Query<(Entity, &Obstacle, &Health), Without<Enemy>>,
    mut damages: MessageWriter<Damage>,
) {
    let (mut transform, mut motion, collider) = rover.into_inner();
    let radius = collider.map_or(DEFAULT_VEHICLE_RADIUS, |c| c.radius);
    let mut p = Vec2::new(transform.translation.x, transform.translation.z);
    for (entity, obstacle, health) in &obstacles {
        if health.is_dead() {
            continue;
        }
        let (center, r) = obstacle.shape.bounds();
        if center.distance_squared(p) > (r + radius).powi(2) {
            continue;
        }
        let Some(push) = obstacle.shape.push_out(p, radius) else {
            continue;
        };
        // Доля выталкивания и поворот от препятствия растут с его размером.
        let weight = (0.1 + 0.06 * r).min(0.5);
        p += push * weight;
        if let Some(away) = push.try_normalize() {
            let wanted = (-away.x).atan2(-away.y);
            let diff = (wanted - motion.heading + PI).rem_euclid(TAU) - PI;
            motion.heading += diff.clamp(-1.0, 1.0) * weight * 0.4;
        }
        motion.speed *= 1.0 - weight * 0.5;
        commands
            .entity(entity)
            .remove::<Obstacle>()
            .try_insert((
                crate::props::PropBlastFrom(transform.translation),
                crate::buildings::BlastFrom(transform.translation),
            ));
        damages.write(Damage {
            target: entity,
            amount: health.current + 1.0,
            source_team: Team::Player,
        });
    }
    transform.translation.x = p.x;
    transform.translation.z = p.y;
}

fn push_vehicles_out(
    obstacles: Query<&Obstacle>,
    mut vehicles: Query<
        (&mut Transform, Option<&Collider>),
        Or<(With<Rover>, With<GroundVehicle>)>,
    >,
) {
    for (mut transform, collider) in &mut vehicles {
        let radius = collider.map_or(DEFAULT_VEHICLE_RADIUS, |c| c.radius);
        let mut p = Vec2::new(transform.translation.x, transform.translation.z);
        // Два прохода: машина между двумя близкими препятствиями.
        for _ in 0..2 {
            for obstacle in &obstacles {
                let (center, r) = obstacle.shape.bounds();
                if center.distance_squared(p) > (r + radius).powi(2) {
                    continue;
                }
                if let Some(push) = obstacle.shape.push_out(p, radius) {
                    p += push;
                }
            }
        }
        transform.translation.x = p.x;
        transform.translation.z = p.y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Shape {
        Shape::Rect(Footprint {
            center: Vec2::new(10.0, 0.0),
            yaw: 0.3,
            half: Vec2::new(2.5, 6.0),
        })
    }

    #[test]
    fn circle_crossing_rectangle_is_pushed_outside() {
        let shape = rect();
        let Shape::Rect(r) = shape else { unreachable!() };
        for local in [
            Vec2::new(3.0, 0.0),
            Vec2::new(-2.0, 1.0),
            Vec2::new(0.5, 5.5),
            Vec2::new(2.8, 6.3),
            Vec2::new(0.0, 0.0),
        ] {
            let p = r.to_world(local);
            let push = shape.push_out(p, 1.2).expect("круг пересекает прямоугольник");
            let moved = p + push;
            assert!(r.distance(moved) >= 1.2 - 1e-3, "из {local} вытолкнут на {}", r.distance(moved));
            assert!(shape.push_out(moved, 1.2).is_none());
        }
        assert!(shape.push_out(r.to_world(Vec2::new(5.0, 0.0)), 1.2).is_none());
    }

    #[test]
    fn push_keeps_tangential_motion() {
        // Машина у стены вдоль Z: выталкивание меняет только X.
        let shape = Shape::Rect(Footprint {
            center: Vec2::ZERO,
            yaw: 0.0,
            half: Vec2::new(2.0, 5.0),
        });
        let push = shape.push_out(Vec2::new(2.5, 1.0), 1.0).unwrap();
        assert!(push.y.abs() < 1e-5 && push.x > 0.0);
    }

    #[test]
    fn segment_stops_at_obstacle_below_its_top() {
        let obstacle = Obstacle {
            shape: Shape::Circle {
                center: Vec2::new(5.0, 0.0),
                radius: 1.0,
            },
            top: 3.0,
        };
        let e = Entity::PLACEHOLDER;
        let low = segment_hit(Vec3::new(0.0, 1.0, 0.0), Vec3::new(10.0, 1.0, 0.0), [(e, &obstacle)]);
        let (_, point) = low.expect("пуля на высоте 1 м попадает");
        assert!((point.x - 4.0).abs() < 0.3);
        let high = segment_hit(Vec3::new(0.0, 4.0, 0.0), Vec3::new(10.0, 4.0, 0.0), [(e, &obstacle)]);
        assert!(high.is_none());
    }
}
