//! Дома в сцене: сущности создаются, когда весь рельеф под домом загружен, и удаляются
//! вместе с ним. Состояние дома живёт в [`Settlements`], сущность ссылается на него через [`House`].

use std::collections::HashMap;

use bevy::prelude::*;

use crate::combat::{Damage, Died, Health, Team, blast_damage};
use crate::collapse::CollapseHouse;
use crate::subsidence::Collapsing;
use crate::explosion::Explosion;
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
            .add_message::<BreakDoor>()
            .add_systems(Startup, create_house_assets)
            .add_systems(
                Update,
                (forget_spawned_houses, stream_houses)
                    .chain()
                    .after(crate::settlements::reset_settlements),
            )
            .add_message::<BuildingDestroyed>()
            .add_systems(
                Update,
                (
                    break_doors,
                    blast_houses,
                    react_to_hits,
                    destroy_houses,
                    wobble_houses,
                    darken_damaged_houses,
                )
                    .chain()
                    .after(stream_houses),
            );
    }
}

/// Сущность дома и адрес его состояния в [`Settlements`].
#[derive(Component, Debug, Clone, Copy)]
pub struct House(pub HouseId);

/// Просьба выбить дверь дома на стене `side`: из него выезжает враг.
#[derive(Message, Debug, Clone, Copy)]
pub struct BreakDoor {
    pub house: Entity,
    pub side: u8,
}

/// Проём выбитой двери на стене `side` в осях дома.
fn door_hole(half: Vec2, side: u8) -> Transform {
    let (x, z, scale) = match side {
        0 => (0.0, half.y + 0.05, Vec3::new(1.7, 2.1, 0.1)),
        1 => (0.0, -half.y - 0.05, Vec3::new(1.7, 2.1, 0.1)),
        2 => (half.x + 0.05, 0.0, Vec3::new(0.1, 2.1, 1.7)),
        _ => (-half.x - 0.05, 0.0, Vec3::new(0.1, 2.1, 1.7)),
    };
    Transform::from_xyz(x, 1.05, z).with_scale(scale)
}

/// Выбивает дверь: тёмный проём, щепки, вздрагивание дома. Проём запоминается в хранилище.
fn break_doors(
    mut commands: Commands,
    mut requests: MessageReader<BreakDoor>,
    assets: Res<HouseAssets>,
    mut settlements: ResMut<Settlements>,
    houses: Query<(&House, &GlobalTransform)>,
    mut damage: MessageWriter<Damage>,
    mut debris: MessageWriter<crate::debris::SpawnDebris>,
) {
    for request in requests.read() {
        let Ok((house, transform)) = houses.get(request.house) else {
            continue;
        };
        let Some(state) = settlements.house_mut(house.0) else {
            continue;
        };
        let first = state.broken_door.is_none();
        state.broken_door.get_or_insert(request.side);
        let half = state.kind.half_extents();
        let door = state.wall_center(request.side);
        let (_, out) = state.side_facing(door * 2.0 - state.position);
        if first {
            commands
                .entity(request.house)
                .with_child((Mesh3d(assets.cube.clone()), MeshMaterial3d(assets.hole.clone()), door_hole(half, request.side)));
        }
        let base = transform.translation().y;
        let seed = request.house.to_bits() as u32;
        for i in 0..8u32 {
            let k = ((seed.wrapping_mul(2654435761).wrapping_add(i * 977)) % 1000) as f32 / 1000.0;
            let side = Vec2::new(-out.y, out.x) * (k - 0.5) * 3.0;
            let v = out * (5.0 + 4.0 * k) + side;
            debris.write(crate::debris::SpawnDebris {
                position: Vec3::new(door.x, base + 0.8 + k, door.y),
                velocity: Vec3::new(v.x, 4.0 + 3.0 * k, v.y),
                size: Vec3::new(0.12, 0.08, 0.5 + 0.3 * k),
                color: Color::srgb(0.45, 0.3, 0.16),
                burning: false,
            });
        }
        // Удар изнутри: дом вздрагивает.
        damage.write(Damage {
            target: request.house,
            amount: 0.5,
            source_team: Team::Enemy,
        });
    }
}

/// Множитель урона взрыва по дому: дома хрупкие, мортира у стены сносит дом с одного попадания.
const HOUSE_BLAST_MULTIPLIER: f32 = 4.0;

/// Сколько длится вздрагивание при попадании, с.
const WOBBLE_SECONDS: f32 = 0.25;

/// Наибольший наклон при вздрагивании, рад.
const WOBBLE_ANGLE: f32 = 0.04;

/// Кто разрушил дом.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestroyCause {
    Player,
    Enemy,
    Collapse,
}

/// Дом разрушен. Отправляется ровно один раз на дом: на него подпишутся тревога, жители и ящики.
#[derive(Message, Debug, Clone, Copy)]
pub struct BuildingDestroyed {
    // Читают жители и ящики из следующих изменений.
    #[allow(dead_code)]
    pub position: Vec3,
    /// Читают угроза (`src/waves.rs`) и тревога (`src/alarm.rs`).
    pub cause: DestroyCause,
}

/// Чей был последний урон по дому: он и считается разрушителем.
#[derive(Component, Debug, Clone, Copy)]
struct LastHit(Team);

/// Центр последнего взрыва, задевшего дом: стены падают от него.
#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct BlastFrom(pub(crate) Vec3);

/// Дом вздрагивает после попадания.
#[derive(Component, Debug, Clone, Copy)]
struct Wobble {
    /// Положение дома в покое.
    rest: Transform,
    remaining: f32,
    /// Ось наклона в плане.
    axis: Vec3,
}

/// Стена дома: её материал темнеет после половины прочности.
#[derive(Component, Debug, Clone, Copy)]
struct HouseWalls {
    entity: Entity,
    color: usize,
    darkened: bool,
}

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
    /// Закопчённые стены повреждённого дома.
    damaged_walls: Vec<Handle<StandardMaterial>>,
    /// Руины: фундамент и груды обломков.
    ruin: Handle<StandardMaterial>,
    roofs: Vec<Handle<StandardMaterial>>,
    trim: Handle<StandardMaterial>,
    /// Тёмный проём выбитой двери.
    hole: Handle<StandardMaterial>,
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
        damaged_walls: WALL_PALETTE
            .iter()
            .map(|&c| matte(&mut materials, c.mix(&Color::srgb(0.15, 0.12, 0.1), 0.55)))
            .collect(),
        ruin: matte(&mut materials, Color::srgb(0.32, 0.29, 0.27)),
        roofs: ROOF_PALETTE.iter().map(|&c| matte(&mut materials, c)).collect(),
        trim: matte(&mut materials, TRIM_COLOR),
        hole: matte(&mut materials, Color::srgb(0.03, 0.025, 0.02)),
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
                if spawned.0.contains_key(&id) || !loaded(house) {
                    continue;
                }
                let entity = if house.status == HouseStatus::Destroyed {
                    spawn_ruin(&mut commands, &assets, &terrain, house)
                } else {
                    spawn_house(&mut commands, &assets, &terrain, id, house)
                };
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
        cube.clone(),
        assets.trim.clone(),
        Transform::from_xyz(0.0, 0.9, half.y + 0.03).with_scale(Vec3::new(1.0, 1.8, 0.1)),
    ));

    if let Some(side) = house.broken_door {
        parts.push((assets.cube.clone(), assets.hole.clone(), door_hole(half, side)));
    }

    let max = house.kind.max_health();
    let mut entity = commands.spawn((
        House(id),
        RunScoped,
        transform,
        Visibility::default(),
        Obstacle {
            shape: Shape::Rect(footprint),
            top: transform.translation.y + wall_h + TWO_STORY_ROOF_HEIGHT,
        },
        Health {
            current: house.health.min(max),
            max,
        },
    ));
    let mut walls = None;
    entity.with_children(|parent| {
        for (i, (mesh, material, t)) in parts.into_iter().enumerate() {
            let child = parent.spawn((Mesh3d(mesh), MeshMaterial3d(material), t)).id();
            if i == 0 {
                walls = Some(child);
            }
        }
    });
    if let Some(walls) = walls {
        entity.insert(HouseWalls {
            entity: walls,
            color: house.wall_color,
            darkened: false,
        });
    }
    entity.id()
}

/// Руины на месте разрушенного дома: плоский фундамент и груды обломков цветов дома.
fn spawn_ruin(
    commands: &mut Commands,
    assets: &HouseAssets,
    terrain: &Terrain,
    house: &HouseState,
) -> Entity {
    let footprint = house.footprint();
    let size = footprint.half * 2.0;
    let base = terrain.height_at(house.position.x, house.position.y);
    let wall = assets.walls[house.wall_color].clone();
    let roof = assets.roofs[house.roof_color].clone();
    // Груды обломков по углам и в центре: позиция, размер, материал.
    let heaps = [
        (Vec3::new(-0.3, 0.4, -0.25), Vec3::new(0.45, 0.8, 0.4), wall.clone()),
        (Vec3::new(0.3, 0.3, 0.3), Vec3::new(0.4, 0.6, 0.35), roof.clone()),
        (Vec3::new(0.25, 0.25, -0.3), Vec3::new(0.3, 0.5, 0.3), wall),
        (Vec3::new(-0.2, 0.2, 0.3), Vec3::new(0.35, 0.4, 0.3), roof),
    ];
    commands
        .spawn((
            RunScoped,
            Transform::from_xyz(house.position.x, base, house.position.y)
                .with_rotation(Quat::from_rotation_y(house.yaw)),
            Visibility::default(),
        ))
        .with_children(|parent| {
            parent.spawn((
                Mesh3d(assets.cube.clone()),
                MeshMaterial3d(assets.ruin.clone()),
                Transform::from_xyz(0.0, -0.3, 0.0).with_scale(Vec3::new(size.x + 0.6, 0.9, size.y + 0.6)),
            ));
            for (i, (offset, scale, material)) in heaps.into_iter().enumerate() {
                parent.spawn((
                    Mesh3d(assets.cube.clone()),
                    MeshMaterial3d(material),
                    Transform::from_translation(offset * Vec3::new(size.x, 1.0, size.y))
                        .with_rotation(Quat::from_euler(EulerRot::XYZ, 0.3 * i as f32, 0.7 * i as f32, 0.2))
                        .with_scale(scale * Vec3::new(size.x, 1.5, size.y)),
                ));
            }
        })
        .id()
}

/// Взрыв ранит дом по расстоянию до ближайшей точки его стены, а не до центра.
fn blast_houses(
    mut commands: Commands,
    mut explosions: MessageReader<Explosion>,
    settlements: Res<Settlements>,
    houses: Query<(Entity, &House, &Health)>,
    mut damage: MessageWriter<Damage>,
) {
    for explosion in explosions.read() {
        let center = Vec2::new(explosion.position.x, explosion.position.z);
        for (entity, house, health) in &houses {
            let Some(state) = settlements.house(house.0) else {
                continue;
            };
            if health.is_dead() {
                continue;
            }
            let distance = state.footprint().distance(center);
            let amount = blast_damage(explosion.power, explosion.radius, distance, false)
                * HOUSE_BLAST_MULTIPLIER;
            if amount > 0.0 {
                commands.entity(entity).try_insert(BlastFrom(explosion.position));
                damage.write(Damage {
                    target: entity,
                    amount,
                    source_team: explosion.source_team,
                });
            }
        }
    }
}

/// Попадание: дом вздрагивает, запоминает стрелявшего, прочность пишется в хранилище.
fn react_to_hits(
    mut commands: Commands,
    mut damages: MessageReader<Damage>,
    mut settlements: ResMut<Settlements>,
    houses: Query<(&House, &Transform, &Health, Option<&Wobble>)>,
) {
    for hit in damages.read() {
        let Ok((house, transform, health, wobble)) = houses.get(hit.target) else {
            continue;
        };
        if let Some(state) = settlements.house_mut(house.0) {
            // Сообщение урона применяется в этом же кадре: пишем прочность с запасом на него.
            state.health = (health.current - hit.amount).max(0.0);
        }
        let rest = wobble.map_or(*transform, |w| w.rest);
        let angle = ((hit.target.to_bits() % 1024) as f32 * 1.7 + health.current).sin() * std::f32::consts::PI;
        commands.entity(hit.target).try_insert((
            LastHit(hit.source_team),
            Wobble {
                rest,
                remaining: WOBBLE_SECONDS,
                axis: Vec3::new(angle.cos(), 0.0, angle.sin()),
            },
        ));
    }
}

/// Дом с кончившейся прочностью сменяется руинами. Событие разрушения — один раз на дом.
fn destroy_houses(
    mut commands: Commands,
    mut died: MessageReader<Died>,
    mut settlements: ResMut<Settlements>,
    mut spawned: ResMut<SpawnedHouses>,
    mut destroyed: MessageWriter<BuildingDestroyed>,
    assets: Res<HouseAssets>,
    terrain: Res<Terrain>,
    houses: Query<(
        &House,
        &Transform,
        Option<&LastHit>,
        Option<&Wobble>,
        Option<&BlastFrom>,
        Has<Collapsing>,
    )>,
    mut collapse: MessageWriter<CollapseHouse>,
) {
    for death in died.read() {
        let Ok((house, transform, last_hit, wobble, blast, collapsing)) = houses.get(death.entity) else {
            continue;
        };
        let Some(state) = settlements.house_mut(house.0) else {
            continue;
        };
        if !matches!(state.status, HouseStatus::Intact | HouseStatus::Sunk(_)) {
            continue;
        }
        state.status = HouseStatus::Destroyed;
        state.health = 0.0;
        let state = state.clone();
        commands.entity(death.entity).try_despawn();
        let ruin = spawn_ruin(&mut commands, &assets, &terrain, &state);
        spawned.0.insert(house.0, ruin);
        let cause = match last_hit.map(|h| h.0) {
            _ if collapsing => DestroyCause::Collapse,
            Some(Team::Enemy) => DestroyCause::Enemy,
            _ => DestroyCause::Player,
        };
        let position = wobble.map_or(transform.translation, |w| w.rest.translation);
        destroyed.write(BuildingDestroyed { position, cause });
        collapse.write(CollapseHouse {
            house: state.clone(),
            base: position,
            wall_color: WALL_PALETTE[state.wall_color],
            roof_color: ROOF_PALETTE[state.roof_color],
            wall_material: assets.walls[state.wall_color].clone(),
            roof_material: assets.roofs[state.roof_color].clone(),
            blast: blast.map(|b| b.0),
        });
    }
}

/// Короткое затухающее покачивание дома после попадания.
fn wobble_houses(
    mut commands: Commands,
    time: Res<Time>,
    mut houses: Query<(Entity, &mut Transform, &mut Wobble)>,
) {
    for (entity, mut transform, mut wobble) in &mut houses {
        wobble.remaining -= time.delta_secs();
        if wobble.remaining <= 0.0 {
            *transform = wobble.rest;
            commands.entity(entity).remove::<Wobble>();
            continue;
        }
        let t = wobble.remaining / WOBBLE_SECONDS;
        let angle = WOBBLE_ANGLE * t * (t * 40.0).sin();
        transform.rotation = Quat::from_axis_angle(wobble.axis, angle) * wobble.rest.rotation;
    }
}

/// После половины прочности стены дома темнеют, как закопчённые.
fn darken_damaged_houses(
    assets: Res<HouseAssets>,
    mut houses: Query<(&Health, &mut HouseWalls), Changed<Health>>,
    mut materials: Query<&mut MeshMaterial3d<StandardMaterial>>,
) {
    for (health, mut walls) in &mut houses {
        if walls.darkened || health.fraction() > 0.5 {
            continue;
        }
        if let Ok(mut material) = materials.get_mut(walls.entity) {
            material.0 = assets.damaged_walls[walls.color].clone();
            walls.darkened = true;
        }
    }
}
