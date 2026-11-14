//! Ландшафт: рельеф из шума по сиду, хранилище участков на весь забег, меши и подгрузка
//! участков вокруг лунохода.
//!
//! Как менять рельеф (кратеры): записать новые высоты и цвета в `Terrain::chunk_mut`,
//! затем повесить `ChunkDirty` на сущности мешей изменённых участков и их соседей по краю
//! (`VisibleChunks::entity`). Участки вне сцены помечать не нужно: их меш соберётся
//! из изменённых данных, когда они снова появятся.

mod generator;
mod mesh;
mod storage;

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::prelude::*;

use crate::rover::{self, Rover};

pub use mesh::build_chunk_mesh;
// Часть API — для следующих изменений (кратеры пишут в ChunkData).
#[allow(unused_imports)]
pub use storage::{CHUNK_SIZE, CHUNK_VERTS, ChunkData, Terrain, chunk_coord_at, chunk_origin};

/// Радиус подгрузки в участках: в сцене квадрат (2R+1)×(2R+1) вокруг лунохода.
pub const LOAD_RADIUS: i32 = 3;

/// Участки дальше этого радиуса убираются из сцены. Запас в одно кольцо против мигания на границе.
const UNLOAD_RADIUS: i32 = LOAD_RADIUS + 1;

/// Сид для отладки: `Some(n)` даёт один и тот же рельеф при каждом запуске.
const DEBUG_SEED: Option<u32> = None;

/// Насколько грунт матовый.
const GROUND_ROUGHNESS: f32 = 0.95;

/// Сид забега.
#[derive(Resource, Clone, Copy, Debug)]
pub struct RunSeed(pub u32);

impl RunSeed {
    /// Сид из `DEBUG_SEED`, а без него — из времени запуска.
    pub fn from_clock() -> Self {
        let seed = DEBUG_SEED.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u32)
                .unwrap_or(0)
        });
        Self(seed)
    }
}

/// Сущность меша участка с координатами участка.
#[derive(Component, Clone, Copy, Debug)]
pub struct ChunkMesh(pub IVec2);

/// Маркер: данные участка изменились, меш надо пересобрать в этом кадре.
#[derive(Component, Default)]
pub struct ChunkDirty;

/// Участки, которые сейчас в сцене.
#[derive(Resource, Default)]
pub struct VisibleChunks {
    /// Участок под луноходом на момент последней подгрузки.
    center: Option<IVec2>,
    /// Сущности мешей по координатам участков.
    entities: HashMap<IVec2, Entity>,
}

impl VisibleChunks {
    /// Сущность меша участка, если участок сейчас в сцене.
    pub fn entity(&self, coord: IVec2) -> Option<Entity> {
        self.entities.get(&coord).copied()
    }
}

/// Общий материал всех участков: белый, окраску дают цвета вершин.
#[derive(Resource)]
pub struct TerrainMaterial(pub Handle<StandardMaterial>);

/// Плагин ландшафта.
pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VisibleChunks>()
            .add_systems(PreStartup, (init_terrain, init_material))
            .add_systems(
                Update,
                (stream_chunks, rebuild_dirty_chunks)
                    .chain()
                    .after(rover::drive_rover),
            );
    }
}

/// Создаёт хранилище рельефа для сида забега.
fn init_terrain(mut commands: Commands, seed: Option<Res<RunSeed>>) {
    let seed = seed.map(|s| *s).unwrap_or_else(RunSeed::from_clock);
    let terrain = Terrain::new(seed.0);
    info!("Сид забега: {}", terrain.seed());
    commands.insert_resource(terrain);
}

/// Создаёт общий материал участков.
fn init_material(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    let material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: GROUND_ROUGHNESS,
        ..default()
    });
    commands.insert_resource(TerrainMaterial(material));
}

/// Добавляет участки вокруг лунохода и убирает дальние. Данные участков остаются в хранилище.
fn stream_chunks(
    mut commands: Commands,
    mut terrain: ResMut<Terrain>,
    mut visible: ResMut<VisibleChunks>,
    mut meshes: ResMut<Assets<Mesh>>,
    material: Res<TerrainMaterial>,
    rover: Single<&Transform, With<Rover>>,
) {
    let center = chunk_coord_at(rover.translation.x, rover.translation.z);
    if visible.center == Some(center) {
        return;
    }
    visible.center = Some(center);

    visible.entities.retain(|&coord, &mut entity| {
        let far = (coord - center).abs().max_element() > UNLOAD_RADIUS;
        if far {
            commands.entity(entity).despawn();
        }
        !far
    });

    for dz in -LOAD_RADIUS..=LOAD_RADIUS {
        for dx in -LOAD_RADIUS..=LOAD_RADIUS {
            let coord = center + IVec2::new(dx, dz);
            if visible.entities.contains_key(&coord) {
                continue;
            }
            terrain.chunk(coord);
            let origin = chunk_origin(coord);
            let entity = commands
                .spawn((
                    ChunkMesh(coord),
                    Mesh3d(meshes.add(build_chunk_mesh(&terrain, coord))),
                    MeshMaterial3d(material.0.clone()),
                    Transform::from_xyz(origin.x, 0.0, origin.y),
                ))
                .id();
            visible.entities.insert(coord, entity);
        }
    }
}

/// Пересобирает меши участков, помеченных `ChunkDirty`.
pub fn rebuild_dirty_chunks(
    mut commands: Commands,
    mut terrain: ResMut<Terrain>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut dirty: Query<(Entity, &ChunkMesh, &mut Mesh3d), With<ChunkDirty>>,
) {
    for (entity, chunk, mut mesh) in &mut dirty {
        terrain.chunk(chunk.0);
        mesh.0 = meshes.add(build_chunk_mesh(&terrain, chunk.0));
        commands.entity(entity).remove::<ChunkDirty>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Приложение без окна и рендера: только подгрузка участков вокруг лунохода.
    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .insert_resource(RunSeed(17))
            .init_resource::<VisibleChunks>()
            .add_systems(PreStartup, (init_terrain, init_material))
            .add_systems(Update, (stream_chunks, rebuild_dirty_chunks).chain());
        app.world_mut()
            .spawn((Rover, Transform::from_xyz(5.0, 0.0, 5.0)));
        app
    }

    /// Ставит луноход в точку мира.
    fn move_rover(app: &mut App, x: f32, z: f32) {
        let world = app.world_mut();
        let mut q = world.query_filtered::<&mut Transform, With<Rover>>();
        let mut t = q.single_mut(world).unwrap();
        t.translation.x = x;
        t.translation.z = z;
    }

    /// Число сущностей мешей участков в мире.
    fn chunk_entities(app: &mut App) -> usize {
        let world = app.world_mut();
        world.query::<&ChunkMesh>().iter(world).count()
    }

    #[test]
    fn streaming_keeps_scene_bounded_and_storage_alive() {
        let mut app = headless_app();
        app.update();
        let side = (2 * LOAD_RADIUS + 1) as usize;
        assert_eq!(chunk_entities(&mut app), side * side);
        let start = app.world().resource::<Terrain>().get_chunk(IVec2::ZERO).cloned();
        assert!(start.is_some());

        // Долгая езда в одну сторону: сцена не растёт.
        let max_side = (2 * UNLOAD_RADIUS + 1) as usize;
        for step in 1..=60 {
            move_rover(&mut app, 5.0 + step as f32 * 10.0, 5.0);
            app.update();
            let n = chunk_entities(&mut app);
            assert!(n <= max_side * max_side, "в сцене {n} участков");
        }
        assert_eq!(app.world().resource::<VisibleChunks>().entity(IVec2::ZERO), None);

        // Возврат к старту: данные участка прежние, он снова в сцене.
        move_rover(&mut app, 5.0, 5.0);
        app.update();
        let terrain = app.world().resource::<Terrain>();
        assert_eq!(terrain.get_chunk(IVec2::ZERO).cloned(), start);
        assert!(app.world().resource::<VisibleChunks>().entity(IVec2::ZERO).is_some());
    }

    #[test]
    fn dirty_chunk_is_rebuilt() {
        let mut app = headless_app();
        app.update();
        let entity = app.world().resource::<VisibleChunks>().entity(IVec2::ZERO).unwrap();
        let before = app.world().get::<Mesh3d>(entity).unwrap().0.clone();
        app.world_mut().resource_mut::<Terrain>().chunk_mut(IVec2::ZERO).heights[0] -= 5.0;
        app.world_mut().entity_mut(entity).insert(ChunkDirty);
        app.update();
        assert_ne!(app.world().get::<Mesh3d>(entity).unwrap().0, before);
        assert!(app.world().get::<ChunkDirty>(entity).is_none());
    }
}
