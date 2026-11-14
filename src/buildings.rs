//! Дома в сцене: сущности создаются, когда весь рельеф под домом загружен, и удаляются
//! вместе с ним. Состояние дома живёт в [`Settlements`], сущность ссылается на него через [`House`].

use std::collections::HashMap;

use bevy::prelude::*;

use crate::obstacles::{Obstacle, Shape};
use crate::rover::Rover;
use crate::run::{RunScoped, RunStarted};
use crate::settlements::{
    HouseId, HouseKind, HouseState, HouseStatus, ROOF_COLORS, Settlements,
    WALL_COLORS, region_of,
};
use crate::terrain::{CHUNK_SIZE, LOAD_RADIUS, Terrain, VisibleChunks, chunk_coord_at};

/// На сколько стены уходят ниже самой низкой точки рельефа под домом, м.
const FOUNDATION_DEPTH: f32 = 1.5;

/// Свес крыши за стены, м.
const ROOF_OVERHANG: f32 = 0.4;

/// Высота двускатной крыши, м: у домика и у двухэтажного дома.
const HUT_ROOF_HEIGHT: f32 = 2.0;
const TWO_STORY_ROOF_HEIGHT: f32 = 2.6;

/// Толщина плоской крыши барака, м.
const FLAT_ROOF_THICKNESS: f32 = 0.4;

/// Пастельные стены.
const WALL_PALETTE: [Color; WALL_COLORS] = [
    Color::srgb(0.98, 0.93, 0.62),
    Color::srgb(0.66, 0.93, 0.80),
    Color::srgb(0.80, 0.72, 0.98),
    Color::srgb(0.65, 0.84, 0.99),
    Color::srgb(0.99, 0.82, 0.62),
    Color::srgb(0.95, 0.95, 0.90),
];

/// Насыщенные крыши. Красного нет: он оставлен для опасностей.
const ROOF_PALETTE: [Color; ROOF_COLORS] = [
    Color::srgb(0.12, 0.35, 0.85),
    Color::srgb(0.05, 0.60, 0.58),
    Color::srgb(0.52, 0.18, 0.75),
    Color::srgb(0.95, 0.55, 0.05),
    Color::srgb(0.20, 0.62, 0.18),
];

/// Цвет двери и трубы.
const TRIM_COLOR: Color = Color::srgb(0.25, 0.18, 0.14);

/// Подключает появление домов в сцене.
pub struct BuildingsPlugin;

impl Plugin for BuildingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpawnedHouses>()
            .add_systems(Startup, create_house_assets)
            .add_systems(
                Update,
                (forget_spawned_houses, stream_houses)
                    .chain()
                    .after(crate::settlements::reset_settlements),
            );
    }
}

/// Сущность дома и адрес его состояния в [`Settlements`].
// Адрес читают урон и провал дома (задачи 3.2 и 6.1).
#[allow(dead_code)]
#[derive(Component, Debug, Clone, Copy)]
pub struct House(pub HouseId);

/// Дома, у которых сейчас есть сущность.
#[derive(Resource, Default)]
struct SpawnedHouses(HashMap<HouseId, Entity>);

/// Общие меши и материалы домов: одинаковые части рисуются одним вызовом.
#[derive(Resource)]
struct HouseAssets {
    /// Куб со стороной 1, растягивается масштабом.
    cube: Handle<Mesh>,
    /// Треугольная призма: основание 1 по X на высоте 0, конёк на высоте 1, длина 1 по Z.
    prism: Handle<Mesh>,
    walls: Vec<Handle<StandardMaterial>>,
    roofs: Vec<Handle<StandardMaterial>>,
    trim: Handle<StandardMaterial>,
}

fn create_house_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let triangle = Triangle2d::new(
        Vec2::new(-0.5, 0.0),
        Vec2::new(0.5, 0.0),
        Vec2::new(0.0, 1.0),
    );
    let matte = |materials: &mut Assets<StandardMaterial>, color: Color| {
        materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: 0.85,
            ..default()
        })
    };
    commands.insert_resource(HouseAssets {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        prism: meshes.add(Extrusion::new(triangle, 1.0)),
        walls: WALL_PALETTE.iter().map(|&c| matte(&mut materials, c)).collect(),
        roofs: ROOF_PALETTE.iter().map(|&c| matte(&mut materials, c)).collect(),
        trim: matte(&mut materials, TRIM_COLOR),
    });
}

/// Новый забег: старые сущности удалены через [`RunScoped`], список забывается.
fn forget_spawned_houses(mut started: MessageReader<RunStarted>, mut spawned: ResMut<SpawnedHouses>) {
    if started.read().last().is_some() {
        spawned.0.clear();
    }
}

/// Создаёт дома, под которыми загружен весь рельеф, и убирает те, под которыми он выгружен.
fn stream_houses(
    mut commands: Commands,
    mut settlements: ResMut<Settlements>,
    mut spawned: ResMut<SpawnedHouses>,
    terrain: Res<Terrain>,
    visible: Res<VisibleChunks>,
    assets: Res<HouseAssets>,
    rover: Single<&Transform, With<Rover>>,
) {
    if settlements.seed().is_none() || !visible.is_changed() {
        return;
    }
    let loaded = |house: &HouseState| {
        house
            .footprint()
            .corners()
            .iter()
            .all(|c| visible.entity(chunk_coord_at(c.x, c.y)).is_some())
    };

    // Убрать дома, рельеф под которыми выгружен.
    spawned.0.retain(|id, entity| {
        let keep = settlements
            .house(*id)
            .is_some_and(|house| loaded(house));
        if !keep {
            commands.entity(*entity).try_despawn();
        }
        keep
    });

    // Создать дома в регионах, задевающих загруженную область.
    let reach = ((LOAD_RADIUS + 2) * CHUNK_SIZE) as f32;
    let center = Vec2::new(rover.translation.x, rover.translation.z);
    let low = region_of(center - Vec2::splat(reach));
    let high = region_of(center + Vec2::splat(reach));
    for rz in low.y..=high.y {
        for rx in low.x..=high.x {
            let region = IVec2::new(rx, rz);
            let houses = settlements.region(region, &terrain).houses.clone();
            for (index, house) in houses.iter().enumerate() {
                let id = HouseId { region, index };
                if spawned.0.contains_key(&id)
                    || house.status == HouseStatus::Destroyed
                    || !loaded(house)
                {
                    continue;
                }
                let entity = spawn_house(&mut commands, &assets, &terrain, id, house);
                spawned.0.insert(id, entity);
            }
        }
    }
}

/// Создаёт сущность дома: стены, уходящие в землю, крыша, дверь.
fn spawn_house(
    commands: &mut Commands,
    assets: &HouseAssets,
    terrain: &Terrain,
    id: HouseId,
    house: &HouseState,
) -> Entity {
    let footprint = house.footprint();
    let mut samples: Vec<Vec2> = footprint.corners().to_vec();
    samples.push(footprint.center);
    let heights: Vec<f32> = samples.iter().map(|p| terrain.height_at(p.x, p.y)).collect();
    let base = heights.iter().sum::<f32>() / heights.len() as f32;
    let lowest = heights.iter().copied().fold(f32::MAX, f32::min);
    let depth = base - lowest + FOUNDATION_DEPTH;

    let transform = match house.status {
        HouseStatus::Sunk(t) => t,
        _ => Transform::from_xyz(house.position.x, base, house.position.y)
            .with_rotation(Quat::from_rotation_y(house.yaw)),
    };

    let half = house.kind.half_extents();
    let size = half * 2.0;
    let wall_h = house.kind.wall_height();
    let wall = assets.walls[house.wall_color].clone();
    let roof = assets.roofs[house.roof_color].clone();
    let cube = assets.cube.clone();

    let mut parts: Vec<(Handle<Mesh>, Handle<StandardMaterial>, Transform)> = vec![(
        cube.clone(),
        wall,
        Transform::from_xyz(0.0, (wall_h - depth) * 0.5, 0.0)
            .with_scale(Vec3::new(size.x, wall_h + depth, size.y)),
    )];
    let roof_size = size + Vec2::splat(ROOF_OVERHANG * 2.0);
    match house.kind {
        HouseKind::Hut | HouseKind::TwoStory => {
            let roof_h = if house.kind == HouseKind::Hut {
                HUT_ROOF_HEIGHT
            } else {
                TWO_STORY_ROOF_HEIGHT
            };
            parts.push((
                assets.prism.clone(),
                roof,
                Transform::from_xyz(0.0, wall_h, 0.0)
                    .with_scale(Vec3::new(roof_size.x, roof_h, roof_size.y)),
            ));
            // Труба на скате.
            parts.push((
                cube.clone(),
                assets.trim.clone(),
                Transform::from_xyz(half.x * 0.45, wall_h + roof_h * 0.6, -half.y * 0.4)
                    .with_scale(Vec3::new(0.6, roof_h * 0.9, 0.6)),
            ));
        }
        HouseKind::Barrack => {
            parts.push((
                cube.clone(),
                roof.clone(),
                Transform::from_xyz(0.0, wall_h + FLAT_ROOF_THICKNESS * 0.5, 0.0).with_scale(
                    Vec3::new(roof_size.x, FLAT_ROOF_THICKNESS, roof_size.y),
                ),
            ));
            // Короба вентиляции на плоской крыше.
            for z in [-0.5, 0.5] {
                parts.push((
                    cube.clone(),
                    roof.clone(),
                    Transform::from_xyz(0.0, wall_h + FLAT_ROOF_THICKNESS + 0.3, z * half.y)
                        .with_scale(Vec3::new(1.2, 0.6, 1.2)),
                ));
            }
        }
    }
    // Дверь на торце.
    parts.push((
        cube,
        assets.trim.clone(),
        Transform::from_xyz(0.0, 0.9, half.y + 0.03).with_scale(Vec3::new(1.0, 1.8, 0.1)),
    ));

    commands
        .spawn((
            House(id),
            RunScoped,
            transform,
            Visibility::default(),
            Obstacle {
                shape: Shape::Rect(footprint),
                top: transform.translation.y + wall_h + TWO_STORY_ROOF_HEIGHT,
            },
        ))
        .with_children(|parent| {
            for (mesh, material, t) in parts {
                parent.spawn((Mesh3d(mesh), MeshMaterial3d(material), t));
            }
        })
        .id()
}
