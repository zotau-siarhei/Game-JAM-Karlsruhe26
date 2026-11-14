//! Звенья путей: два рельса, шпалы и подсыпка вдоль линий железной дороги.
//!
//! Звенья по 4 м создаются и удаляются вместе с участками рельефа. Звено лежит на текущей
//! поверхности и наклоняется по ней. В туннеле звеньев нет: там путь скрыт горой.

use std::collections::HashMap;

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::railway::{Family, Lines, TUNNEL_DEPTH};
use crate::run::{RunScoped, RunStarted};
use crate::terrain::{CHUNK_SIZE, ChunkMesh, Terrain, VisibleChunks, chunk_coord_at, chunk_origin};

/// Длина звена, м.
pub const LINK_LENGTH: f32 = 4.0;

/// Половина ширины колеи, м.
const GAUGE_HALF: f32 = 0.72;

/// Шпал на звено.
const SLEEPERS: usize = 6;

/// Подключает звенья путей.
pub struct RailsPlugin;

impl Plugin for RailsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpawnedLinks>()
            .add_systems(Startup, create_rail_assets)
            .add_systems(Update, (forget_links, stream_links).chain());
    }
}

/// Звенья по участкам рельефа, которые сейчас в сцене.
#[derive(Resource, Default)]
struct SpawnedLinks(HashMap<IVec2, Vec<Entity>>);

#[derive(Resource)]
struct RailAssets {
    cube: Handle<Mesh>,
    rail: Handle<StandardMaterial>,
    sleeper: Handle<StandardMaterial>,
    ballast: Handle<StandardMaterial>,
}

fn create_rail_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(RailAssets {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        rail: materials.add(StandardMaterial {
            base_color: Color::srgb(0.62, 0.64, 0.68),
            metallic: 0.8,
            perceptual_roughness: 0.35,
            ..default()
        }),
        sleeper: materials.add(Color::srgb(0.36, 0.25, 0.16)),
        ballast: materials.add(StandardMaterial {
            base_color: Color::srgb(0.42, 0.4, 0.38),
            perceptual_roughness: 1.0,
            ..default()
        }),
    });
}

fn forget_links(mut started: MessageReader<RunStarted>, mut spawned: ResMut<SpawnedLinks>) {
    if started.read().last().is_some() {
        // Сущности прошлого забега удалены через RunScoped.
        spawned.0.clear();
    }
}

/// Концы звеньев, которые начинаются в участке: по линиям, проходящим рядом с ним.
fn links_in_chunk(lines: &Lines, coord: IVec2) -> Vec<(Vec2, Vec2)> {
    let origin = chunk_origin(coord);
    let size = CHUNK_SIZE as f32;
    let center = origin + Vec2::splat(size * 0.5);
    let mut links = Vec::new();
    for near in lines.near(center, size) {
        let line = near.line;
        let (from, to) = match line.family {
            Family::AlongX => (origin.x, origin.x + size),
            Family::AlongZ => (origin.y, origin.y + size),
        };
        let first = (from / LINK_LENGTH).floor() as i32;
        let last = (to / LINK_LENGTH).ceil() as i32;
        for i in first..=last {
            let s0 = i as f32 * LINK_LENGTH;
            let a = lines.point(line, s0);
            if chunk_coord_at(a.x, a.y) != coord {
                continue;
            }
            links.push((a, lines.point(line, s0 + LINK_LENGTH)));
        }
    }
    links
}

/// Создаёт звенья участков, появившихся в сцене, и убирает звенья выгруженных.
fn stream_links(
    mut commands: Commands,
    mut spawned: ResMut<SpawnedLinks>,
    terrain: Res<Terrain>,
    visible: Res<VisibleChunks>,
    chunks: Query<&ChunkMesh>,
    assets: Res<RailAssets>,
) {
    if !visible.is_changed() {
        return;
    }
    spawned.0.retain(|&coord, entities| {
        let keep = visible.entity(coord).is_some();
        if !keep {
            for &e in entities.iter() {
                commands.entity(e).try_despawn();
            }
        }
        keep
    });
    for chunk in &chunks {
        let coord = chunk.0;
        if spawned.0.contains_key(&coord) || visible.entity(coord).is_none() {
            continue;
        }
        let mut entities = Vec::new();
        for (a, b) in links_in_chunk(terrain.lines(), coord) {
            let mid = (a + b) * 0.5;
            // В туннеле путь скрыт горой.
            let near = terrain.lines().nearest(mid);
            if terrain.raw_height(mid.x, mid.y) > terrain.track_level(near.line, near.s) + TUNNEL_DEPTH {
                continue;
            }
            let ha = terrain.height_at(a.x, a.y);
            let hb = terrain.height_at(b.x, b.y);
            let start = Vec3::new(a.x, ha, a.y);
            let end = Vec3::new(b.x, hb, b.y);
            let along = (end - start).normalize_or(Vec3::Z);
            let rotation = Quat::from_rotation_arc(Vec3::Z, along);
            let length = start.distance(end);
            let entity = commands
                .spawn((
                    RunScoped,
                    Transform::from_translation((start + end) * 0.5).with_rotation(rotation),
                    Visibility::default(),
                ))
                .with_children(|link| {
                    link.spawn((
                        Mesh3d(assets.cube.clone()),
                        MeshMaterial3d(assets.ballast.clone()),
                        NotShadowCaster,
                        Transform::from_xyz(0.0, -0.02, 0.0).with_scale(Vec3::new(3.0, 0.12, length + 0.05)),
                    ));
                    for i in 0..SLEEPERS {
                        let z = (i as f32 + 0.5) / SLEEPERS as f32 * length - length * 0.5;
                        link.spawn((
                            Mesh3d(assets.cube.clone()),
                            MeshMaterial3d(assets.sleeper.clone()),
                            NotShadowCaster,
                            Transform::from_xyz(0.0, 0.08, z).with_scale(Vec3::new(2.3, 0.1, 0.28)),
                        ));
                    }
                    for x in [-GAUGE_HALF, GAUGE_HALF] {
                        link.spawn((
                            Mesh3d(assets.cube.clone()),
                            MeshMaterial3d(assets.rail.clone()),
                            NotShadowCaster,
                            Transform::from_xyz(x, 0.2, 0.0).with_scale(Vec3::new(0.1, 0.14, length + 0.04)),
                        ));
                    }
                })
                .id();
            entities.push(entity);
        }
        spawned.0.insert(coord, entities);
    }
}
