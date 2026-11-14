//! Рулёжка врагов: объезд домов и крупных объектов и выход из застревания.
//!
//! Тот, кто ведёт машину, пишет в [`Steering`] желаемый курс и скорость. Система
//! [`steer_vehicles`] щупает путь тремя лучами — вперёд, влево и вправо по 6 м, — по коллайдерам
//! [`Obstacle`] и отворачивает от препятствия, запоминая сторону объезда, чтобы не метаться.
//! Если машина 2 с почти не сдвинулась, она 1 с отъезжает назад с разворотом вбок.
//!
//! Видимость для стрельбы — [`has_los`]: рельеф, как у пуль, плюс дома и крупные объекты.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use bevy::prelude::*;

use crate::bullet;
use crate::combat::Collider;
use crate::obstacles::{self, Obstacle, Shape};
use crate::terrain::Terrain;
use crate::vehicle::{self, GroundVehicle};

/// Длина лучей, м.
const LOOK: f32 = 6.0;

/// Угол боковых лучей от курса, рад.
const RAY_ANGLE: f32 = 30.0 * PI / 180.0;

/// Шаг проверки луча, м.
const RAY_STEP: f32 = 0.5;

/// Сколько секунд помнится сторона объезда после того, как путь освободился.
const SIDE_MEMORY: f32 = 1.5;

/// Окно проверки застревания, с.
const STUCK_WINDOW: f32 = 2.0;

/// Если за окно машина сдвинулась меньше, она застряла, м.
const STUCK_DISTANCE: f32 = 1.0;

/// Сколько длится отъезд назад, с.
const BACK_OFF_TIME: f32 = 1.0;

/// Скорость отъезда назад от обычной скорости.
const BACK_OFF_SPEED: f32 = 0.6;

/// Машина, которая сама объезжает препятствия.
#[derive(Component, Debug, Clone)]
pub struct Steering {
    /// Куда машина хочет ехать, курс, рад.
    pub desired: f32,
    /// Желаемая скорость, м/с. Ноль — стоит и поворачивается к `desired`.
    pub speed: f32,
    /// Скорость поворота, рад/с.
    pub turn_speed: f32,
    /// Сторона объезда: 1 — влево, -1 — вправо, 0 — не выбрана.
    side: f32,
    side_timer: f32,
    /// Где машина была в начале окна проверки застревания.
    anchor: Vec2,
    window: f32,
    /// Сколько ещё отъезжать назад, с.
    backing: f32,
}

impl Steering {
    pub fn new(heading: f32, turn_speed: f32) -> Self {
        Self {
            desired: heading,
            speed: 0.0,
            turn_speed,
            side: 0.0,
            side_timer: 0.0,
            anchor: Vec2::NAN,
            window: 0.0,
            backing: 0.0,
        }
    }
}

/// Угол `angle`, приведённый к промежутку от -π до π.
fn wrap(angle: f32) -> f32 {
    (angle + PI).rem_euclid(TAU) - PI
}

/// Насколько свободен луч из `origin` по курсу `heading`: расстояние до первой занятой точки
/// или [`LOOK`], если луч свободен.
fn clearance(origin: Vec2, heading: f32, blocked: &impl Fn(Vec2) -> bool) -> f32 {
    let forward = vehicle::forward_of(heading);
    let forward = Vec2::new(forward.x, forward.z);
    let steps = (LOOK / RAY_STEP) as usize;
    (1..=steps)
        .map(|i| i as f32 * RAY_STEP)
        .find(|&d| blocked(origin + forward * d))
        .unwrap_or(LOOK)
}

/// Курс, которым ехать, и сторона объезда.
///
/// `heading` — нынешний курс машины, `desired` — куда она хочет, `side` — запомненная сторона.
fn avoid(origin: Vec2, heading: f32, desired: f32, side: f32, blocked: &impl Fn(Vec2) -> bool) -> (f32, f32) {
    let ahead = clearance(origin, heading, blocked);
    let left = clearance(origin, heading + RAY_ANGLE, blocked);
    let right = clearance(origin, heading - RAY_ANGLE, blocked);
    if ahead < LOOK {
        // Впереди препятствие: отворачиваем тем круче, чем оно ближе.
        let side = if side != 0.0 {
            side
        } else if left >= right {
            1.0
        } else {
            -1.0
        };
        let urgency = 1.0 - ahead / LOOK;
        return (heading + side * (RAY_ANGLE + urgency * FRAC_PI_2), side);
    }
    // Впереди свободно. Если цель сбоку, а луч в её сторону упирается в стену, едем вдоль стены.
    let turn = wrap(desired - heading);
    let toward = if turn > 0.0 { left } else { right };
    if turn.abs() > 0.05 && turn.abs() < FRAC_PI_2 * 1.5 && toward < LOOK {
        return (heading, side);
    }
    (desired, side)
}

/// Центр и радиус описанной окружности формы.
fn bounds(shape: &Shape) -> (Vec2, f32) {
    match *shape {
        Shape::Circle { center, radius } => (center, radius),
        Shape::Rect(rect) => (rect.center, rect.half.length()),
    }
}

/// Ведёт машины с [`Steering`]: объезд, поворот с ограниченной скоростью и выход из застревания.
pub fn steer_vehicles(
    time: Res<Time>,
    obstacles: Query<&Obstacle>,
    mut vehicles: Query<(&Transform, &mut Steering, &mut GroundVehicle, Option<&Collider>)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let mut near: Vec<Shape> = Vec::new();
    for (transform, mut steering, mut vehicle, collider) in &mut vehicles {
        let origin = Vec2::new(transform.translation.x, transform.translation.z);
        let radius = collider.map_or(1.0, |c| c.radius);
        let max_turn = steering.turn_speed * dt;

        if steering.speed == 0.0 {
            // Стоит: только поворачивается, застревание не считается.
            steering.anchor = origin;
            steering.window = 0.0;
            steering.backing = 0.0;
            vehicle.heading = vehicle::steer(vehicle.heading, steering.desired, max_turn);
            vehicle.speed = 0.0;
            continue;
        }

        if steering.backing > 0.0 {
            // Отъезд назад: корма уходит в сторону, обратную объезду.
            steering.backing -= dt;
            let target = vehicle.heading - steering.side * FRAC_PI_2;
            vehicle.heading = vehicle::steer(vehicle.heading, target, max_turn);
            vehicle.speed = -steering.speed.abs() * BACK_OFF_SPEED;
            if steering.backing <= 0.0 {
                steering.anchor = origin;
                steering.window = 0.0;
            }
            continue;
        }

        // Застревание: за окно машина почти не сдвинулась.
        if !steering.anchor.is_finite() {
            steering.anchor = origin;
        }
        steering.window += dt;
        if steering.window >= STUCK_WINDOW {
            let moved = origin.distance(steering.anchor);
            steering.anchor = origin;
            steering.window = 0.0;
            if moved < STUCK_DISTANCE {
                steering.backing = BACK_OFF_TIME;
                // Объезд не помог: пробуем другую сторону.
                steering.side = if steering.side > 0.0 { -1.0 } else { 1.0 };
                steering.side_timer = SIDE_MEMORY + BACK_OFF_TIME;
                continue;
            }
        }

        near.clear();
        near.extend(obstacles.iter().map(|o| o.shape).filter(|shape| {
            let (center, r) = bounds(shape);
            center.distance(origin) <= LOOK + r + radius
        }));
        let (heading, side) = if near.is_empty() {
            (steering.desired, steering.side)
        } else {
            let blocked = |p: Vec2| near.iter().any(|shape| shape.push_out(p, radius * 0.9).is_some());
            avoid(origin, vehicle.heading, steering.desired, steering.side, &blocked)
        };
        if side != 0.0 && (side != steering.side || heading != steering.desired) {
            steering.side_timer = SIDE_MEMORY;
        }
        steering.side = side;
        steering.side_timer -= dt;
        if steering.side_timer <= 0.0 {
            steering.side = 0.0;
        }
        vehicle.heading = vehicle::steer(vehicle.heading, heading, max_turn);
        vehicle.speed = steering.speed;
    }
}

/// Видно ли `to` из `from`: отрезок не уходит под рельеф и не задевает дома и крупные объекты.
pub fn has_los<'a>(
    terrain: &Terrain,
    obstacles: impl IntoIterator<Item = (Entity, &'a Obstacle)>,
    from: Vec3,
    to: Vec3,
) -> bool {
    bullet::has_line_of_sight(terrain, from, to) && obstacles::segment_hit(from, to, obstacles).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settlements::Footprint;

    /// Дом 10 × 6 м с центром в `center`.
    fn house(center: Vec2) -> Shape {
        Shape::Rect(Footprint {
            center,
            yaw: 0.0,
            half: Vec2::new(5.0, 3.0),
        })
    }

    fn blocked_by(shapes: Vec<Shape>, radius: f32) -> impl Fn(Vec2) -> bool {
        move |p| shapes.iter().any(|s| s.push_out(p, radius).is_some())
    }

    #[test]
    fn free_road_keeps_desired_heading() {
        let blocked = blocked_by(vec![], 1.0);
        let (heading, side) = avoid(Vec2::ZERO, 0.0, 0.4, 0.0, &blocked);
        assert_eq!(heading, 0.4);
        assert_eq!(side, 0.0);
    }

    #[test]
    fn wall_ahead_turns_away_and_remembers_side() {
        // Курс 0 — на -Z, дом в 4 м впереди.
        let blocked = blocked_by(vec![house(Vec2::new(0.0, -7.0))], 1.0);
        let (heading, side) = avoid(Vec2::ZERO, 0.0, 0.0, 0.0, &blocked);
        assert!(side != 0.0);
        assert!(heading.abs() > RAY_ANGLE, "курс {heading} не отвернул");
        // Запомненная сторона сохраняется, даже если другая свободнее.
        let (heading, kept) = avoid(Vec2::ZERO, 0.0, 0.0, -1.0, &blocked);
        assert_eq!(kept, -1.0);
        assert!(heading < 0.0);
    }

    #[test]
    fn obstacle_off_to_the_side_prefers_the_free_side() {
        // Дом впереди и чуть левее: объезд вправо (отрицательный курс).
        let blocked = blocked_by(vec![house(Vec2::new(-5.0, -7.0))], 1.0);
        let (heading, side) = avoid(Vec2::ZERO, 0.0, 0.0, 0.0, &blocked);
        assert_eq!(side, -1.0);
        assert!(heading < 0.0);
    }

    #[test]
    fn slides_along_wall_while_target_is_behind_it() {
        // Едет вдоль стены на -Z, стена слева (в -X), цель за стеной левее: курс не меняется.
        let wall = Shape::Rect(Footprint {
            center: Vec2::new(-4.0, -5.0),
            yaw: 0.0,
            half: Vec2::new(2.0, 8.0),
        });
        let blocked = blocked_by(vec![wall], 1.0);
        let (heading, _) = avoid(Vec2::ZERO, 0.0, 0.6, 1.0, &blocked);
        assert_eq!(heading, 0.0);
    }

    /// Прогон машины к цели за домом простым интегрированием, без рельефа.
    #[test]
    fn vehicle_drives_around_a_house_to_its_target() {
        let shapes = vec![house(Vec2::new(0.0, -15.0))];
        let radius = 1.2;
        let blocked = blocked_by(shapes.clone(), radius * 0.9);
        let target = Vec2::new(0.0, -30.0);
        let mut p = Vec2::ZERO;
        let mut heading = 0.0;
        let mut side = 0.0;
        let dt = 1.0 / 60.0;
        let mut reached = false;
        for _ in 0..(60 * 15) {
            let to = target - p;
            if to.length() < 2.0 {
                reached = true;
                break;
            }
            let desired = vehicle::heading_towards(Vec3::new(to.x, 0.0, to.y));
            let (h, s) = avoid(p, heading, desired, side, &blocked);
            side = s;
            heading = vehicle::steer(heading, h, 2.2 * dt);
            let f = vehicle::forward_of(heading);
            p += Vec2::new(f.x, f.z) * 9.0 * dt;
            // Как в obstacles.rs: машину выталкивает из стены.
            for shape in &shapes {
                if let Some(push) = shape.push_out(p, radius) {
                    p += push;
                }
            }
        }
        assert!(reached, "машина не доехала до цели, стоит в {p}");
    }
}
