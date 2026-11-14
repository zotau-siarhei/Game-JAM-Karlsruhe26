//! Лазер: импульс бьёт всё на отрезке луча и прорезает в холмах траншею с раскалёнными краями.
//!
//! Луч идёт горизонтально от башни к прицелу. Урон получают чужие машины и всё со здоровьем,
//! что стоит на пути: дома, деревья, валуны. Где холм выше луча, рельеф срезается чуть ниже
//! луча; края прореза светятся оранжевым и за несколько секунд остывают до тёмно-коричневого.

use std::collections::HashSet;

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::aim::AimPoint;
use crate::arsenal::Inventory;
use crate::combat::{Collider, Damage, Health, Team};
use crate::obstacles::Obstacle;
use crate::rover::{self, Rover};
use crate::run::{GameState, RunScoped, RunStarted};
use crate::subsidence::TerrainChanged;
use crate::terrain::{self, ChunkDirty, Terrain, VisibleChunks};
use crate::weapons::CurrentWeapon;

/// Перезарядка, с.
const RELOAD: f32 = 0.6;

/// Длина луча, м.
const BEAM_LENGTH: f32 = 40.0;

/// Урон импульса.
const BEAM_DAMAGE: f32 = 60.0;

/// Высота луча над центром лунохода, м.
const BEAM_HEIGHT: f32 = 0.9;

/// Сколько гаснет луч, с.
const BEAM_SECONDS: f32 = 0.2;

/// Полуширина траншеи, м.
const TRENCH_HALF_WIDTH: f32 = 1.2;

/// На сколько дно траншеи ниже луча, м.
const TRENCH_BELOW_BEAM: f32 = 0.4;

/// За сколько остывают края, с.
const COOL_SECONDS: f32 = 4.0;

/// Как часто пересобирать остывающие участки, с.
const COOL_STEP: f32 = 0.25;

/// Раскалённый край и остывший.
const HOT: [f32; 3] = [1.0, 0.45, 0.05];
const COLD: [f32; 3] = [0.10, 0.06, 0.04];

/// Подключает лазер.
pub struct LaserPlugin;

impl Plugin for LaserPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HotEdges>()
            .add_systems(Startup, create_laser_assets)
            .add_systems(
                Update,
                (
                    reset_hot_edges,
                    fire_laser.run_if(in_state(GameState::Playing)),
                    fade_beams,
                    cool_edges,
                )
                    .chain()
                    .after(rover::drive_rover)
                    .before(terrain::rebuild_dirty_chunks),
            );
    }
}

/// Гаснущий луч.
#[derive(Component)]
struct Beam {
    remaining: f32,
    thickness: f32,
}

/// Раскалённые вершины траншеи: мировые координаты и время с прореза.
#[derive(Resource, Default)]
struct HotEdges {
    vertices: Vec<(IVec2, f32)>,
    since_step: f32,
}

#[derive(Resource)]
struct LaserAssets {
    beam: Handle<Mesh>,
    beam_material: Handle<StandardMaterial>,
}

fn create_laser_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(LaserAssets {
        beam: meshes.add(Cylinder::new(1.0, 1.0)),
        beam_material: materials.add(StandardMaterial {
            base_color: Color::srgb(0.6, 0.95, 1.0),
            emissive: LinearRgba::rgb(4.0, 14.0, 20.0),
            unlit: true,
            ..default()
        }),
    });
}

fn reset_hot_edges(mut started: MessageReader<RunStarted>, mut hot: ResMut<HotEdges>) {
    if started.read().last().is_some() {
        *hot = HotEdges::default();
    }
}

/// Расстояние от точки до отрезка в плане.
fn flat_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

/// Срезает рельеф вдоль отрезка ниже высоты луча с плавным краем. Возвращает изменённые участки
/// и вершины, которые срезаны.
///
/// Высоты только опускаются: на ровном месте ниже луча рельеф не меняется.
pub fn cut_trench(terrain: &mut Terrain, from: Vec2, to: Vec2, beam_y: f32) -> (Vec<IVec2>, Vec<IVec2>) {
    let floor = beam_y - TRENCH_BELOW_BEAM;
    let reach = TRENCH_HALF_WIDTH + 1.0;
    let (x0, x1) = (from.x.min(to.x) - reach, from.x.max(to.x) + reach);
    let (z0, z1) = (from.y.min(to.y) - reach, from.y.max(to.y) + reach);
    let mut chunks = Vec::new();
    let mut cut = Vec::new();
    for vz in z0.floor() as i32..=z1.ceil() as i32 {
        for vx in x0.floor() as i32..=x1.ceil() as i32 {
            let d = flat_distance(Vec2::new(vx as f32, vz as f32), from, to);
            if d > reach {
                continue;
            }
            // Внутри полуширины — дно, дальше плавный подъём к нетронутому.
            let edge = ((d - TRENCH_HALF_WIDTH) / (reach - TRENCH_HALF_WIDTH)).clamp(0.0, 1.0);
            let limit = floor + edge * edge * 3.0;
            let current = terrain.vertex_height(vx, vz);
            if current <= limit {
                continue;
            }
            for c in terrain.edit_vertex(vx, vz, |h, color| {
                *h = h.min(limit);
                color[0] = HOT[0];
                color[1] = HOT[1];
                color[2] = HOT[2];
            }) {
                if !chunks.contains(&c) {
                    chunks.push(c);
                }
            }
            cut.push(IVec2::new(vx, vz));
        }
    }
    (chunks, cut)
}

/// Помечает на пересборку участки и их соседей по краю.
fn mark_dirty(commands: &mut Commands, visible: &VisibleChunks, chunks: impl IntoIterator<Item = IVec2>) {
    let mut dirty = HashSet::new();
    for c in chunks {
        for o in [IVec2::ZERO, IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y] {
            dirty.insert(c + o);
        }
    }
    for c in dirty {
        if let Some(e) = visible.entity(c) {
            commands.entity(e).try_insert(ChunkDirty);
        }
    }
}

/// Импульс лазера.
#[allow(clippy::too_many_arguments)]
fn fire_laser(
    mut commands: Commands,
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    weapon: Res<CurrentWeapon>,
    aim: Res<AimPoint>,
    assets: Res<LaserAssets>,
    mut inventory: ResMut<Inventory>,
    mut terrain: ResMut<Terrain>,
    visible: Res<VisibleChunks>,
    mut hot: ResMut<HotEdges>,
    mut cooldown: Local<f32>,
    rover: Single<(&Transform, &Health), With<Rover>>,
    targets: Query<(Entity, &GlobalTransform, &Collider, &Team, &Health), Without<Rover>>,
    obstacles: Query<(Entity, &Obstacle), With<Health>>,
    mut damage: MessageWriter<Damage>,
    mut changed: MessageWriter<TerrainChanged>,
) {
    *cooldown = (*cooldown - time.delta_secs()).max(0.0);
    let (rover, health) = *rover;
    let trigger = mouse.pressed(MouseButton::Left)
        && *weapon == CurrentWeapon::Laser
        && inventory.usable(CurrentWeapon::Laser)
        && !health.is_dead();
    if !trigger || *cooldown > 0.0 {
        return;
    }
    *cooldown = RELOAD;
    inventory.spend(CurrentWeapon::Laser);

    let start = rover.translation + Vec3::Y * BEAM_HEIGHT;
    let dir = (aim.position - start).with_y(0.0).normalize_or(Vec3::NEG_Z);
    let end = start + dir * BEAM_LENGTH;
    let (a, b) = (Vec2::new(start.x, start.z), Vec2::new(end.x, end.z));

    // Урон всем на отрезке, кроме лунохода.
    for (entity, transform, collider, team, target_health) in &targets {
        if *team == Team::Player || target_health.is_dead() {
            continue;
        }
        let p = transform.translation();
        if flat_distance(Vec2::new(p.x, p.z), a, b) <= collider.radius {
            damage.write(Damage {
                target: entity,
                amount: BEAM_DAMAGE,
                source_team: Team::Player,
            });
        }
    }
    let steps = (BEAM_LENGTH / 0.5) as usize;
    for (entity, obstacle) in &obstacles {
        let hit = (0..=steps).any(|s| {
            let p = start.lerp(end, s as f32 / steps as f32);
            p.y <= obstacle.top && obstacle.shape.contains(Vec2::new(p.x, p.z))
        });
        if hit {
            damage.write(Damage {
                target: entity,
                amount: BEAM_DAMAGE,
                source_team: Team::Player,
            });
        }
    }

    // Траншея.
    let (chunks, cut) = cut_trench(&mut terrain, a, b, start.y);
    if !cut.is_empty() {
        mark_dirty(&mut commands, &visible, chunks);
        hot.vertices.extend(cut.into_iter().map(|v| (v, 0.0)));
        let mid = (start + end) * 0.5;
        changed.write(TerrainChanged {
            center: mid,
            radius: BEAM_LENGTH * 0.5 + TRENCH_HALF_WIDTH,
        });
    }

    // Луч.
    commands.spawn((
        Beam {
            remaining: BEAM_SECONDS,
            thickness: 0.18,
        },
        RunScoped,
        NotShadowCaster,
        Mesh3d(assets.beam.clone()),
        MeshMaterial3d(assets.beam_material.clone()),
        Transform::from_translation((start + end) * 0.5)
            .with_rotation(Quat::from_rotation_arc(Vec3::Y, dir))
            .with_scale(Vec3::new(0.18, BEAM_LENGTH, 0.18)),
        PointLight {
            color: Color::srgb(0.6, 0.9, 1.0),
            intensity: 2_000_000.0,
            range: 25.0,
            shadow_maps_enabled: false,
            ..default()
        },
    ));
}

/// Луч тоньшает и гаснет.
fn fade_beams(mut commands: Commands, time: Res<Time>, mut beams: Query<(Entity, &mut Transform, &mut Beam, &mut PointLight)>) {
    for (entity, mut transform, mut beam, mut light) in &mut beams {
        beam.remaining -= time.delta_secs();
        if beam.remaining <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let t = beam.remaining / BEAM_SECONDS;
        transform.scale.x = beam.thickness * t;
        transform.scale.z = beam.thickness * t;
        light.intensity = 2_000_000.0 * t;
    }
}

/// Края траншеи остывают: цвет вершин смешивается к тёмному, участки пересобираются раз в четверть секунды.
fn cool_edges(
    mut commands: Commands,
    time: Res<Time>,
    mut hot: ResMut<HotEdges>,
    mut terrain: ResMut<Terrain>,
    visible: Res<VisibleChunks>,
) {
    if hot.vertices.is_empty() {
        return;
    }
    let dt = time.delta_secs();
    hot.since_step += dt;
    for v in &mut hot.vertices {
        v.1 += dt;
    }
    if hot.since_step < COOL_STEP {
        return;
    }
    hot.since_step = 0.0;
    let mut chunks = Vec::new();
    for &(v, age) in &hot.vertices {
        let t = (age / COOL_SECONDS).min(1.0);
        let k = t * t * (3.0 - 2.0 * t);
        let color = [0, 1, 2].map(|i| HOT[i] + (COLD[i] - HOT[i]) * k);
        for c in terrain.edit_vertex(v.x, v.y, |_, c| {
            c[0] = color[0];
            c[1] = color[1];
            c[2] = color[2];
        }) {
            if !chunks.contains(&c) {
                chunks.push(c);
            }
        }
    }
    hot.vertices.retain(|v| v.1 < COOL_SECONDS);
    mark_dirty(&mut commands, &visible, chunks);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trench_never_raises_and_keeps_low_ground() {
        let mut terrain = Terrain::new(3);
        let (from, to) = (Vec2::new(0.0, 0.0), Vec2::new(30.0, 0.0));
        let before: Vec<f32> = (-4..=34).flat_map(|x| (-4..=4).map(move |z| (x, z))).map(|(x, z)| terrain.vertex_height(x, z)).collect();
        let lowest = before.iter().copied().fold(f32::MAX, f32::min);
        // Луч выше всего рельефа: ничего не меняется.
        let (_, cut) = cut_trench(&mut terrain, from, to, lowest + 100.0);
        assert!(cut.is_empty());
        // Луч посередине: высоты только опускаются.
        let mid = before.iter().sum::<f32>() / before.len() as f32;
        cut_trench(&mut terrain, from, to, mid);
        let after: Vec<f32> = (-4..=34).flat_map(|x| (-4..=4).map(move |z| (x, z))).map(|(x, z)| terrain.vertex_height(x, z)).collect();
        for (b, a) in before.iter().zip(&after) {
            assert!(a <= b);
            if *b <= mid - TRENCH_BELOW_BEAM {
                assert_eq!(a, b);
            }
        }
    }
}
