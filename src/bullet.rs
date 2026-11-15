//! Пули: летят по прямой, упираются в рельеф и ранят чужую команду.
//!
//! Чтобы выстрелить, отправь сообщение [`FireBullet`]:
//!
//! ```ignore
//! bullets.write(FireBullet { origin, direction, team: Team::Player, damage: 6.0 });
//! ```

use bevy::prelude::*;

use crate::combat::{Collider, Damage, Health, Team};
use crate::frame::GameSet;
use crate::obstacles::{self, Obstacle};
use crate::run::RunScoped;
use crate::terrain::Terrain;

/// Скорость пули, м/с.
pub const BULLET_SPEED: f32 = 60.0;

/// Дальность полёта пули, м.
pub const BULLET_RANGE: f32 = 45.0;

/// Шаг проверки пути пули и прямой видимости, м.
const STEP: f32 = 0.5;

/// Насколько коллайдер выше своего радиуса по вертикали: пуля над холмом не задевает машину под ним.
const HIT_HEIGHT: f32 = 1.5;

/// Подключает пули.
pub struct BulletPlugin;

impl Plugin for BulletPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<FireBullet>()
            .add_systems(Startup, create_bullet_assets)
            .add_systems(
                Update,
                (
                    (spawn_bullets, move_bullets).chain().in_set(GameSet::Simulation),
                    fade_ground_flashes.in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Просьба выпустить пулю.
#[derive(Message, Debug, Clone, Copy)]
pub struct FireBullet {
    /// Откуда: дуло.
    pub origin: Vec3,
    /// Куда. Длина не важна.
    pub direction: Vec3,
    /// Чья пуля: свою команду она не ранит.
    pub team: Team,
    /// Урон при попадании.
    pub damage: f32,
}

/// Летящая пуля.
#[derive(Component)]
struct Bullet {
    velocity: Vec3,
    /// Сколько ещё пролетит, м.
    remaining: f32,
    team: Team,
    damage: f32,
}

/// Сколько секунд видна вспышка там, где пуля ушла в землю.
const GROUND_FLASH_SECONDS: f32 = 0.12;

/// Вспышка на земле в месте попадания пули: сжимается и исчезает.
#[derive(Component)]
struct GroundFlash {
    remaining: f32,
}

/// Общий меш и материалы пуль: все пули рисуются одним пакетом на команду.
#[derive(Resource)]
struct BulletAssets {
    mesh: Handle<Mesh>,
    flash: Handle<Mesh>,
    player: Handle<StandardMaterial>,
    enemy: Handle<StandardMaterial>,
}

fn create_bullet_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let glowing = |color: Color, glow: LinearRgba| StandardMaterial {
        base_color: color,
        emissive: glow,
        unlit: true,
        ..default()
    };
    commands.insert_resource(BulletAssets {
        // Вытянута вдоль -Z: так пуля читается как трассер.
        mesh: meshes.add(Cuboid::new(0.12, 0.12, 0.9)),
        flash: meshes.add(Sphere::new(0.35)),
        player: materials.add(glowing(Color::srgb(1.0, 0.9, 0.4), LinearRgba::rgb(6.0, 5.0, 1.5))),
        enemy: materials.add(glowing(Color::srgb(1.0, 0.35, 0.2), LinearRgba::rgb(6.0, 1.2, 0.6))),
    });
}

fn spawn_bullets(
    mut commands: Commands,
    mut shots: MessageReader<FireBullet>,
    assets: Res<BulletAssets>,
) {
    for shot in shots.read() {
        let Some(direction) = shot.direction.try_normalize() else {
            continue;
        };
        let material = match shot.team {
            Team::Player => assets.player.clone(),
            Team::Enemy | Team::Neutral => assets.enemy.clone(),
        };
        commands.spawn((
            RunScoped,
            Bullet {
                velocity: direction * BULLET_SPEED,
                remaining: BULLET_RANGE,
                team: shot.team,
                damage: shot.damage,
            },
            Mesh3d(assets.mesh.clone()),
            MeshMaterial3d(material),
            Transform::from_translation(shot.origin).looking_to(direction, Vec3::Y),
        ));
    }
}

/// Где отрезок от `from` до `to` уходит под рельеф. `None` — путь свободен.
pub fn terrain_hit(terrain: &Terrain, from: Vec3, to: Vec3) -> Option<Vec3> {
    let length = from.distance(to);
    let steps = (length / STEP).ceil().max(1.0) as usize;
    (1..=steps)
        .map(|step| from.lerp(to, step as f32 / steps as f32))
        .find(|point| point.y < terrain.height_at(point.x, point.z))
}

/// Видно ли точку `to` из точки `from`: нет ли между ними рельефа.
pub fn has_line_of_sight(terrain: &Terrain, from: Vec3, to: Vec3) -> bool {
    terrain_hit(terrain, from, to).is_none()
}

/// Задевает ли отрезок от `from` до `to` коллайдер с центром `center` и радиусом `radius`.
fn segment_hits(from: Vec3, to: Vec3, center: Vec3, radius: f32) -> bool {
    let along = to - from;
    let length_squared = along.length_squared();
    let nearest = if length_squared < 1e-8 {
        from
    } else {
        from + along * ((center - from).dot(along) / length_squared).clamp(0.0, 1.0)
    };
    let offset = nearest - center;
    Vec2::new(offset.x, offset.z).length() <= radius && offset.y.abs() <= radius + HIT_HEIGHT
}

/// Двигает пули. Пуля исчезает, когда попала в чужую машину, ушла в рельеф или пролетела дальность.
fn move_bullets(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut bullets: Query<(Entity, &mut Transform, &mut Bullet)>,
    targets: Query<(Entity, &GlobalTransform, &Collider, &Team, &Health)>,
    mut damage: MessageWriter<Damage>,
    assets: Option<Res<BulletAssets>>,
    obstacles: Query<(Entity, &Obstacle)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for (entity, mut transform, mut bullet) in &mut bullets {
        let from = transform.translation;
        let step = (bullet.velocity * dt).clamp_length_max(bullet.remaining);
        let mut to = from + step;
        let mut stop = false;
        let mut into_ground = false;

        // Рельеф на пути укорачивает отрезок: за холмом пуля уже никого не заденет.
        if let Some(ground) = terrain_hit(&terrain, from, to) {
            to = ground;
            stop = true;
            into_ground = true;
        }
        // Дома, валуны и деревья тоже останавливают пулю.
        let mut wall_hit = None;
        if let Some((wall, point)) = obstacles::segment_hit(from, to, &obstacles) {
            to = point;
            stop = true;
            into_ground = false;
            wall_hit = Some(wall);
        }

        let hit = targets
            .iter()
            .filter(|(_, _, _, team, health)| **team != bullet.team && !health.is_dead())
            .filter(|(_, target, collider, _, _)| segment_hits(from, to, target.translation(), collider.radius))
            .min_by(|left, right| {
                let left = left.1.translation().distance_squared(from);
                let right = right.1.translation().distance_squared(from);
                left.total_cmp(&right)
            });
        if let Some((target, _, _, _, _)) = hit {
            damage.write(Damage {
                target,
                amount: bullet.damage,
                source_team: bullet.team,
            });
            stop = true;
            into_ground = false;
        } else if let Some(wall) = wall_hit {
            // Препятствие со здоровьем (дом) получает урон, валун пулю просто глотает.
            damage.write(Damage {
                target: wall,
                amount: bullet.damage,
                source_team: bullet.team,
            });
        }

        // Пуля ушла в землю: короткая вспышка цвета пули показывает, куда она попала.
        if let (true, Some(assets)) = (into_ground, assets.as_ref()) {
            let material = match bullet.team {
                Team::Player => assets.player.clone(),
                Team::Enemy | Team::Neutral => assets.enemy.clone(),
            };
            commands.spawn((
                RunScoped,
                GroundFlash {
                    remaining: GROUND_FLASH_SECONDS,
                },
                Mesh3d(assets.flash.clone()),
                MeshMaterial3d(material),
                Transform::from_translation(to),
            ));
        }

        bullet.remaining -= step.length();
        if stop || bullet.remaining <= 0.0 {
            commands.entity(entity).despawn();
        } else {
            transform.translation = to;
        }
    }
}

/// Вспышка на земле сжимается и удаляется.
fn fade_ground_flashes(
    mut commands: Commands,
    time: Res<Time>,
    mut flashes: Query<(Entity, &mut GroundFlash, &mut Transform)>,
) {
    for (entity, mut flash, mut transform) in &mut flashes {
        flash.remaining -= time.delta_secs();
        if flash.remaining <= 0.0 {
            commands.entity(entity).try_despawn();
        } else {
            transform.scale = Vec3::splat(flash.remaining / GROUND_FLASH_SECONDS);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_above_terrain_is_free_and_path_into_ground_is_blocked() {
        let terrain = Terrain::new(3);
        let ground = |x: f32, z: f32| terrain.height_at(x, z);
        // Высоко над рельефом: холмы ниже 100 м.
        let from = Vec3::new(0.0, ground(0.0, 0.0) + 200.0, 0.0);
        let to = Vec3::new(30.0, ground(30.0, 0.0) + 200.0, 0.0);
        assert!(has_line_of_sight(&terrain, from, to));

        let buried = Vec3::new(30.0, ground(30.0, 0.0) - 5.0, 0.0);
        let hit = terrain_hit(&terrain, from, buried).unwrap();
        assert!(!has_line_of_sight(&terrain, from, buried));
        // Точка попадания — у поверхности, не дальше шага под ней.
        assert!(hit.y < ground(hit.x, hit.z));
        assert!(hit.distance(buried) < from.distance(buried));
    }

    #[test]
    fn segment_hits_collider_only_when_it_passes_through() {
        let center = Vec3::new(10.0, 0.0, 0.0);
        assert!(segment_hits(Vec3::ZERO, Vec3::new(20.0, 0.0, 0.0), center, 1.2));
        assert!(segment_hits(Vec3::ZERO, Vec3::new(20.0, 0.0, 1.0), center, 1.2));
        // Мимо по горизонтали.
        assert!(!segment_hits(Vec3::ZERO, Vec3::new(20.0, 0.0, 6.0), center, 1.2));
        // Отрезок кончается раньше цели.
        assert!(!segment_hits(Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0), center, 1.2));
        // Высоко над целью.
        assert!(!segment_hits(Vec3::new(0.0, 8.0, 0.0), Vec3::new(20.0, 8.0, 0.0), center, 1.2));
    }

    fn app_with_bullets() -> (App, Entity) {
        let mut app = App::new();
        let terrain = Terrain::new(5);
        // Цель стоит на поверхности в 10 м от начала координат, стрелок целится ей в центр.
        let position = Vec3::new(10.0, terrain.height_at(10.0, 0.0) + 1.0, 0.0);
        app.insert_resource(terrain)
            .init_resource::<Time>()
            .add_message::<Damage>()
            .init_resource::<Taken>()
            .add_systems(Update, (move_bullets, collect_damage).chain());
        let target = app
            .world_mut()
            .spawn((
                GlobalTransform::from_translation(position),
                Collider { radius: 1.2 },
                Team::Enemy,
                Health::new(30.0),
            ))
            .id();
        (app, target)
    }

    #[derive(Resource, Default)]
    struct Taken(Vec<(Entity, f32)>);

    fn collect_damage(mut damage: MessageReader<Damage>, mut taken: ResMut<Taken>) {
        taken.0.extend(damage.read().map(|hit| (hit.target, hit.amount)));
    }

    fn fly(app: &mut App, bullet: Bullet, origin: Vec3) -> Entity {
        let entity = app.world_mut().spawn((Transform::from_translation(origin), bullet)).id();
        for _ in 0..60 {
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(std::time::Duration::from_millis(16));
            app.update();
        }
        entity
    }

    #[test]
    fn bullet_damages_other_team_once_and_disappears() {
        let (mut app, target) = app_with_bullets();
        let aim = app.world().get::<GlobalTransform>(target).unwrap().translation();
        let origin = Vec3::new(aim.x - 6.0, aim.y + 0.5, aim.z);
        let bullet = Bullet {
            velocity: (aim - origin).normalize() * BULLET_SPEED,
            remaining: BULLET_RANGE,
            team: Team::Player,
            damage: 6.0,
        };
        let entity = fly(&mut app, bullet, origin);
        assert_eq!(app.world().resource::<Taken>().0, vec![(target, 6.0)]);
        assert!(app.world().get_entity(entity).is_err());
    }

    #[test]
    fn bullet_does_not_hurt_its_own_team() {
        let (mut app, target) = app_with_bullets();
        let aim = app.world().get::<GlobalTransform>(target).unwrap().translation();
        let origin = Vec3::new(aim.x - 6.0, aim.y + 0.5, aim.z);
        let bullet = Bullet {
            velocity: (aim - origin).normalize() * BULLET_SPEED,
            remaining: BULLET_RANGE,
            team: Team::Enemy,
            damage: 6.0,
        };
        let entity = fly(&mut app, bullet, origin);
        assert!(app.world().resource::<Taken>().0.is_empty());
        // Пуля пролетела дальность или ушла в рельеф и исчезла.
        assert!(app.world().get_entity(entity).is_err());
    }
}
