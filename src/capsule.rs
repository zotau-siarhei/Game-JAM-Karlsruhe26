//! Десантные капсулы: враг прибывает с неба. Место падения заранее отмечено кольцом на земле,
//! капсула падает с огненным следом, взрывается с кратером и уроном, затем из неё выезжает враг.
//!
//! Капсулу просит директор волн сообщением [`DropCapsule`].

use std::f32::consts::FRAC_PI_2;

use bevy::prelude::*;

use crate::combat::Team;
use crate::enemy::{self, SpawnEnemy, SpawnPlace};
use crate::explosion::Explosion;
use crate::rover::Rover;
use crate::run::RunScoped;
use crate::sound_list::Dice;
use crate::terrain::Terrain;

/// Сколько секунд отметка видна до начала падения.
const MARK_SECONDS: f32 = 2.0;

/// Сколько секунд капсула падает.
const FALL_SECONDS: f32 = 0.6;

/// С какой высоты капсула падает, м.
const FALL_HEIGHT: f32 = 60.0;

/// Через сколько секунд после удара из капсулы выезжает враг.
const OPEN_SECONDS: f32 = 0.4;

/// Сколько секунд пустая капсула лежит обломком.
const DEBRIS_SECONDS: f32 = 20.0;

/// На каком расстоянии от лунохода падает капсула, м: в кадре, но не на голову.
const LANDING_DISTANCE: (f32, f32) = (10.0, 22.0);

/// Взрыв при ударе: радиус, м, и мощность — половина мортиры.
const IMPACT_BLAST: (f32, f32) = (5.0, 0.5);

/// Размер капсулы: радиус и высота, м.
const CAPSULE_SIZE: (f32, f32) = (0.9, 2.2);

/// Кольцо отметки: внутренний и внешний радиус, м.
const RING: (f32, f32) = (1.7, 2.1);

/// Подключает десантные капсулы.
pub struct CapsulePlugin;

impl Plugin for CapsulePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Dice>()
            .add_message::<DropCapsule>()
            .add_systems(Startup, create_assets)
            .add_systems(Update, (mark_landings, drop_capsules, settle_debris).chain());
    }
}

/// Просьба сбросить капсулу с врагом в направлении `angle` от лунохода, рад.
#[derive(Message, Debug, Clone, Copy)]
pub struct DropCapsule {
    pub angle: f32,
}

/// Капсула от отметки до выезда врага. Сущность стоит в точке падения на земле.
#[derive(Component)]
struct Capsule {
    /// Сколько секунд прошло с появления отметки.
    age: f32,
    landed: bool,
    ring: Entity,
    body: Entity,
    trail: Entity,
}

/// Пустая капсула: лежит на земле и через время исчезает.
#[derive(Component)]
struct Debris {
    remaining: f32,
}

#[derive(Resource)]
struct CapsuleAssets {
    ring: Handle<Mesh>,
    body: Handle<Mesh>,
    trail: Handle<Mesh>,
    ring_material: Handle<StandardMaterial>,
    body_material: Handle<StandardMaterial>,
    trail_material: Handle<StandardMaterial>,
}

fn create_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let glowing = |color: Color, glow: LinearRgba| StandardMaterial {
        base_color: color,
        emissive: glow,
        unlit: true,
        // Кольцо плоское: его видно с обеих сторон.
        double_sided: true,
        cull_mode: None,
        ..default()
    };
    commands.insert_resource(CapsuleAssets {
        ring: meshes.add(Annulus::new(RING.0, RING.1)),
        body: meshes.add(Cylinder::new(CAPSULE_SIZE.0, CAPSULE_SIZE.1)),
        trail: meshes.add(Cuboid::new(0.6, 7.0, 0.6)),
        ring_material: materials.add(glowing(Color::srgb(1.0, 0.2, 0.1), LinearRgba::rgb(6.0, 0.8, 0.4))),
        body_material: materials.add(StandardMaterial {
            base_color: Color::srgb(0.2, 0.2, 0.24),
            perceptual_roughness: 0.6,
            ..default()
        }),
        trail_material: materials.add(glowing(Color::srgb(1.0, 0.6, 0.15), LinearRgba::rgb(8.0, 3.5, 0.6))),
    });
}

/// Высота капсулы над землёй через `falling` секунд после начала падения, м.
fn fall_height(falling: f32) -> f32 {
    FALL_HEIGHT * (1.0 - (falling / FALL_SECONDS).clamp(0.0, 1.0))
}

/// Ставит отметку падения и готовит капсулу над ней.
fn mark_landings(
    mut commands: Commands,
    mut requests: MessageReader<DropCapsule>,
    mut dice: ResMut<Dice>,
    assets: Res<CapsuleAssets>,
    terrain: Res<Terrain>,
    rover: Query<&Transform, With<Rover>>,
) {
    let Some(rover) = rover.iter().next() else {
        return;
    };
    for request in requests.read() {
        let distance = dice.range(LANDING_DISTANCE.0, LANDING_DISTANCE.1);
        let mut target = enemy::ring_point(rover.translation, request.angle, distance);
        target.y = terrain.height_at(target.x, target.z);

        let ring = commands
            .spawn((
                Mesh3d(assets.ring.clone()),
                MeshMaterial3d(assets.ring_material.clone()),
                // Кольцо лежит плашмя чуть над землёй, чтобы не тонуть в склоне.
                Transform::from_xyz(0.0, 0.4, 0.0).with_rotation(Quat::from_rotation_x(-FRAC_PI_2)),
            ))
            .id();
        let trail = commands
            .spawn((
                Mesh3d(assets.trail.clone()),
                MeshMaterial3d(assets.trail_material.clone()),
                Transform::from_xyz(0.0, CAPSULE_SIZE.1 * 0.5 + 3.5, 0.0),
            ))
            .id();
        let body = commands
            .spawn((
                Mesh3d(assets.body.clone()),
                MeshMaterial3d(assets.body_material.clone()),
                Transform::from_xyz(0.0, FALL_HEIGHT, 0.0),
                Visibility::Hidden,
            ))
            .add_child(trail)
            .id();
        commands
            .spawn((
                Capsule {
                    age: 0.0,
                    landed: false,
                    ring,
                    body,
                    trail,
                },
                RunScoped,
                Transform::from_translation(target),
                Visibility::default(),
            ))
            .add_children(&[ring, body]);
    }
}

/// Ведёт капсулу: отметка пульсирует, капсула падает, взрывается и выпускает врага.
fn drop_capsules(
    mut commands: Commands,
    time: Res<Time>,
    mut capsules: Query<(Entity, &mut Capsule, &Transform)>,
    mut parts: Query<(&mut Transform, &mut Visibility), Without<Capsule>>,
    mut explosions: MessageWriter<Explosion>,
    mut spawn: MessageWriter<SpawnEnemy>,
) {
    for (entity, mut capsule, transform) in &mut capsules {
        capsule.age += time.delta_secs();
        let falling = capsule.age - MARK_SECONDS;

        if !capsule.landed {
            if let Ok((mut ring, _)) = parts.get_mut(capsule.ring) {
                ring.scale = Vec3::splat(1.0 + 0.15 * (capsule.age * 10.0).sin());
            }
            if falling >= 0.0
                && let Ok((mut body, mut visibility)) = parts.get_mut(capsule.body)
            {
                body.translation.y = fall_height(falling) + CAPSULE_SIZE.1 * 0.5;
                *visibility = Visibility::Inherited;
            }
            if falling >= FALL_SECONDS {
                capsule.landed = true;
                commands.entity(capsule.ring).try_despawn();
                commands.entity(capsule.trail).try_despawn();
                explosions.write(Explosion {
                    position: transform.translation,
                    radius: IMPACT_BLAST.0,
                    power: IMPACT_BLAST.1,
                    source_team: Team::Enemy,
                });
            }
        } else if falling >= FALL_SECONDS + OPEN_SECONDS {
            spawn.write(SpawnEnemy {
                place: SpawnPlace::Point(transform.translation),
            });
            commands
                .entity(entity)
                .try_remove::<Capsule>()
                .try_insert(Debris {
                    remaining: DEBRIS_SECONDS,
                });
        }
    }
}

/// Пустая капсула лежит на дне своего кратера и через время исчезает.
fn settle_debris(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut debris: Query<(Entity, &mut Debris, &mut Transform)>,
) {
    for (entity, mut debris, mut transform) in &mut debris {
        debris.remaining -= time.delta_secs();
        if debris.remaining <= 0.0 {
            commands.entity(entity).try_despawn();
            continue;
        }
        let ground = terrain.height_at(transform.translation.x, transform.translation.z);
        // Наполовину в земле: видно, что капсула врезалась.
        let wanted = ground - CAPSULE_SIZE.1 * 0.3;
        if transform.translation.y != wanted {
            transform.translation.y = wanted;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capsule_falls_from_the_sky_to_the_ground() {
        assert_eq!(fall_height(0.0), FALL_HEIGHT);
        assert!((fall_height(FALL_SECONDS * 0.5) - FALL_HEIGHT * 0.5).abs() < 1e-3);
        assert_eq!(fall_height(FALL_SECONDS), 0.0);
        assert_eq!(fall_height(FALL_SECONDS * 3.0), 0.0);
    }
}
