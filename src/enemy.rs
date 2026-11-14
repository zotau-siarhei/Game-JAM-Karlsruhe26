//! Враги. Первый тип — таран-стрелок: появляется за кадром и едет прямо на луноход.

use std::f32::consts::TAU;

use bevy::prelude::*;

use crate::combat::{Collider, Died, Health, Team};
use crate::rover::{self, Rover};
use crate::sound_list::Dice;
use crate::terrain::Terrain;
use crate::vehicle::{self, GroundVehicle};

/// Здоровье таран-стрелка.
const ENEMY_HEALTH: f32 = 30.0;

/// Скорость таран-стрелка, м/с. Луноход быстрее: от врага можно уехать.
pub const ENEMY_SPEED: f32 = 9.0;

/// Скорость поворота врага к цели, рад/с.
const ENEMY_TURN_SPEED: f32 = 2.2;

/// Радиус коллайдера врага, м.
const ENEMY_RADIUS: f32 = 1.2;

/// Размер корпуса врага: ширина, высота, длина, м.
const HULL_SIZE: Vec3 = Vec3::new(1.4, 0.7, 2.0);

/// Радиус колеса врага, м.
const WHEEL_RADIUS: f32 = 0.32;

/// Высота центра корпуса врага над поверхностью, м.
const RIDE_HEIGHT: f32 = HULL_SIZE.y * 0.5 + WHEEL_RADIUS * 0.6;

/// Как часто появляется новый враг, с.
const SPAWN_INTERVAL: f32 = 4.0;

/// Сколько врагов может быть одновременно.
pub const MAX_ENEMIES: usize = 8;

/// Кольцо появления вокруг лунохода, м. Кадр видит около 30 м от центра, враг появляется вне его.
pub const SPAWN_RING: (f32, f32) = (45.0, 55.0);

/// Враги дальше этого расстояния от лунохода убираются, м.
const DESPAWN_DISTANCE: f32 = 120.0;

/// Цвет корпуса врага.
const HULL_COLOR: Color = Color::srgb(0.72, 0.16, 0.12);

/// Цвет колёс и тарана врага.
const DARK_COLOR: Color = Color::srgb(0.1, 0.1, 0.12);

/// Подключает врагов.
pub struct EnemyPlugin;

impl Plugin for EnemyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Dice>()
            .init_resource::<SpawnTimer>()
            .add_message::<EnemySpawned>()
            .add_systems(Startup, create_enemy_assets)
            .add_systems(
                Update,
                (
                    spawn_enemies,
                    chase_rover,
                    vehicle::move_ground_vehicles,
                    separate_enemies,
                    remove_far_enemies,
                    remove_dead_enemies,
                )
                    .chain()
                    .after(rover::drive_rover),
            );
    }
}

/// Враг.
#[derive(Component)]
pub struct Enemy;

/// Появился новый враг.
#[derive(Message, Debug, Clone, Copy)]
pub struct EnemySpawned {
    // Сущность пригодится мини-карте и отметкам опасности.
    #[allow(dead_code)]
    pub entity: Entity,
}

/// Общие меши и материалы врагов: одинаковые враги рисуются одним пакетом.
#[derive(Resource)]
struct EnemyAssets {
    hull: Handle<Mesh>,
    wheel: Handle<Mesh>,
    ram: Handle<Mesh>,
    hull_material: Handle<StandardMaterial>,
    dark_material: Handle<StandardMaterial>,
}

/// Время до появления следующего врага.
#[derive(Resource)]
struct SpawnTimer(f32);

impl Default for SpawnTimer {
    fn default() -> Self {
        Self(SPAWN_INTERVAL)
    }
}

fn create_enemy_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(EnemyAssets {
        hull: meshes.add(Cuboid::from_size(HULL_SIZE)),
        wheel: meshes.add(Cylinder::new(WHEEL_RADIUS, 0.28)),
        ram: meshes.add(Cuboid::new(HULL_SIZE.x + 0.5, 0.35, 0.35)),
        hull_material: materials.add(HULL_COLOR),
        dark_material: materials.add(DARK_COLOR),
    });
}

/// Точка на окружности радиуса `distance` вокруг `center` по углу `angle`.
fn ring_point(center: Vec3, angle: f32, distance: f32) -> Vec3 {
    center + Vec3::new(angle.cos(), 0.0, angle.sin()) * distance
}

/// Раз в несколько секунд ставит врага в случайной точке кольца вокруг лунохода, вне кадра.
fn spawn_enemies(
    mut commands: Commands,
    time: Res<Time>,
    mut timer: ResMut<SpawnTimer>,
    mut dice: ResMut<Dice>,
    assets: Res<EnemyAssets>,
    terrain: Res<Terrain>,
    rover: Query<(&Transform, &Health), With<Rover>>,
    enemies: Query<(), With<Enemy>>,
    mut spawned: MessageWriter<EnemySpawned>,
) {
    let Some((rover, health)) = rover.iter().next() else {
        return;
    };
    if health.is_dead() {
        return;
    }
    timer.0 -= time.delta_secs();
    if timer.0 > 0.0 {
        return;
    }
    timer.0 = SPAWN_INTERVAL;
    if enemies.iter().count() >= MAX_ENEMIES {
        return;
    }

    let angle = dice.range(0.0, TAU);
    let distance = dice.range(SPAWN_RING.0, SPAWN_RING.1);
    let mut position = ring_point(rover.translation, angle, distance);
    position.y = terrain.height_at(position.x, position.z) + RIDE_HEIGHT;
    let heading = vehicle::heading_towards(rover.translation - position);

    let entity = commands
        .spawn((
            Enemy,
            Health::new(ENEMY_HEALTH),
            Team::Enemy,
            Collider {
                radius: ENEMY_RADIUS,
            },
            GroundVehicle::new(heading, RIDE_HEIGHT, Vec2::new(HULL_SIZE.x, HULL_SIZE.z) * 0.5),
            Mesh3d(assets.hull.clone()),
            MeshMaterial3d(assets.hull_material.clone()),
            Transform::from_translation(position).with_rotation(Quat::from_rotation_y(heading)),
        ))
        .with_children(|parent| {
            for (side, along) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                parent.spawn((
                    Mesh3d(assets.wheel.clone()),
                    MeshMaterial3d(assets.dark_material.clone()),
                    Transform::from_xyz(
                        side * (HULL_SIZE.x * 0.5 + 0.14),
                        -HULL_SIZE.y * 0.5,
                        along * HULL_SIZE.z * 0.33,
                    )
                    .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
                ));
            }
            // Таран на носу: видно, куда враг едет.
            parent.spawn((
                Mesh3d(assets.ram.clone()),
                MeshMaterial3d(assets.dark_material.clone()),
                Transform::from_xyz(0.0, -0.1, -HULL_SIZE.z * 0.5 - 0.15),
            ));
        })
        .id();
    spawned.write(EnemySpawned { entity });
}

/// Поворачивает врагов к луноходу и ведёт их вперёд. Когда луноход погиб, враги встают.
fn chase_rover(
    time: Res<Time>,
    rover: Query<(&Transform, &Health), With<Rover>>,
    mut enemies: Query<(&Transform, &mut GroundVehicle), With<Enemy>>,
) {
    let target = rover
        .iter()
        .next()
        .filter(|(_, health)| !health.is_dead())
        .map(|(transform, _)| transform.translation);
    let max_turn = ENEMY_TURN_SPEED * time.delta_secs();
    for (transform, mut vehicle) in &mut enemies {
        let Some(target) = target else {
            vehicle.speed = 0.0;
            continue;
        };
        let to_target = target - transform.translation;
        vehicle.heading = vehicle::steer(vehicle.heading, vehicle::heading_towards(to_target), max_turn);
        vehicle.speed = ENEMY_SPEED;
    }
}

/// На сколько и куда сдвинуть первую машину, чтобы она не пересекалась со второй.
/// `None`, если машины не пересекаются.
fn overlap_push(first: Vec3, second: Vec3, radius_sum: f32) -> Option<Vec3> {
    let mut away = first - second;
    away.y = 0.0;
    let distance = away.length();
    if distance >= radius_sum {
        return None;
    }
    // Машины точно в одной точке: расходятся вдоль X.
    let direction = if distance > 1e-4 { away / distance } else { Vec3::X };
    Some(direction * (radius_sum - distance))
}

/// Расталкивает врагов между собой и не даёт им въехать в луноход.
fn separate_enemies(
    rover: Query<(&Transform, &Collider), (With<Rover>, Without<Enemy>)>,
    mut enemies: Query<(&mut Transform, &Collider), With<Enemy>>,
) {
    let mut pairs = enemies.iter_combinations_mut();
    while let Some([(mut first, first_collider), (mut second, second_collider)]) = pairs.fetch_next() {
        let radius_sum = first_collider.radius + second_collider.radius;
        if let Some(push) = overlap_push(first.translation, second.translation, radius_sum) {
            first.translation += push * 0.5;
            second.translation -= push * 0.5;
        }
    }
    let Some((rover, rover_collider)) = rover.iter().next() else {
        return;
    };
    for (mut enemy, collider) in &mut enemies {
        let radius_sum = collider.radius + rover_collider.radius;
        if let Some(push) = overlap_push(enemy.translation, rover.translation, radius_sum) {
            enemy.translation += push;
        }
    }
}

/// Убирает врагов, от которых луноход уехал далеко.
fn remove_far_enemies(
    mut commands: Commands,
    rover: Query<&Transform, With<Rover>>,
    enemies: Query<(Entity, &Transform), With<Enemy>>,
) {
    let Some(rover) = rover.iter().next() else {
        return;
    };
    for (entity, transform) in &enemies {
        if transform.translation.distance(rover.translation) > DESPAWN_DISTANCE {
            commands.entity(entity).despawn();
        }
    }
}

/// Убирает погибших врагов.
fn remove_dead_enemies(
    mut commands: Commands,
    mut died: MessageReader<Died>,
    enemies: Query<(), With<Enemy>>,
) {
    for message in died.read() {
        if enemies.contains(message.entity) {
            commands.entity(message.entity).try_despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enemy_is_slower_than_rover() {
        // Скорость лунохода вперёд — 14 м/с.
        assert!(ENEMY_SPEED < 14.0);
    }

    #[test]
    fn spawn_point_is_outside_the_frame() {
        let center = Vec3::new(100.0, 3.0, -40.0);
        for step in 0..16 {
            let angle = step as f32 / 16.0 * TAU;
            for distance in [SPAWN_RING.0, SPAWN_RING.1] {
                let point = ring_point(center, angle, distance);
                let flat = Vec2::new(point.x - center.x, point.z - center.z).length();
                assert!((flat - distance).abs() < 1e-3);
                // Кадр видит около 30 м от лунохода.
                assert!(flat > 30.0);
            }
        }
    }

    #[test]
    fn overlapping_vehicles_are_pushed_apart() {
        let push = overlap_push(Vec3::new(1.0, 5.0, 0.0), Vec3::ZERO, 2.4).unwrap();
        assert!((push - Vec3::new(1.4, 0.0, 0.0)).length() < 1e-4);
        assert!(overlap_push(Vec3::new(3.0, 0.0, 0.0), Vec3::ZERO, 2.4).is_none());
        // Машины в одной точке всё равно расходятся.
        let push = overlap_push(Vec3::ZERO, Vec3::ZERO, 2.4).unwrap();
        assert!((push.length() - 2.4).abs() < 1e-4);
    }
}
