//! Объекты на поверхности: деревья, кусты, камни, валуны, кристаллы, заборы, столбы и антенны.
//!
//! Объекты генерируются для каждого участка рельефа при первом обращении: сетка ячеек со
//! случайным сдвигом точки, а три маски низкочастотного шума решают, что где растёт — рощи,
//! каменистые россыпи, заросли кристаллов. Состояния живут в ресурсе [`Props`] до конца забега,
//! сущности создаются и удаляются вместе с мешем участка.

use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use noise::{NoiseFn, Perlin};

use crate::run::{RunScoped, RunStarted};
use crate::settlements::{self, HouseState, REGION_SIZE, Settlements, region_of};
use crate::terrain::{CHUNK_SIZE, Terrain, VisibleChunks, chunk_coord_at, chunk_origin};

/// Ячеек сетки по стороне участка: шаг около 2,5 м.
const CELLS: i32 = 13;

/// Частота масок, 1/м: роща или россыпь — несколько десятков метров.
const MASK_FREQUENCY: f64 = 1.0 / 55.0;

/// Сдвиги сида для масок леса, камней и кристаллов.
const FOREST_SEED: u32 = 3_000;
const ROCKS_SEED: u32 = 4_000;
const CRYSTAL_SEED: u32 = 5_000;

/// Запас вокруг дома, в котором объектов нет, м.
const HOUSE_MARGIN: f32 = 2.0;

/// Радиус чистого места у точки старта, м.
const START_CLEAR: f32 = 8.0;

/// Крупные объекты не стоят на склонах круче этого: `y` нормали меньше.
const LARGE_MIN_UP: f32 = 0.9;

/// Вариантов цвета и формы у каждого вида.
const VARIANTS: usize = 4;

/// Подключает объекты поверхности.
pub struct PropsPlugin;

impl Plugin for PropsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Props>()
            .init_resource::<SpawnedProps>()
            .add_systems(Startup, create_prop_assets)
            .add_systems(
                Update,
                (reset_props, stream_props)
                    .chain()
                    .after(settlements::reset_settlements),
            );
    }
}

/// Вид объекта.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropKind {
    Tree,
    Bush,
    Rock,
    Boulder,
    Crystal,
    Fence,
    Post,
    Antenna,
}

impl PropKind {
    /// Крупный объект — препятствие. Мелкий машина сносит на ходу.
    pub fn is_large(self) -> bool {
        matches!(self, PropKind::Tree | PropKind::Boulder | PropKind::Crystal)
    }

    // Прочность читает разрушение объектов (задача 5.1).
    #[allow(dead_code)]
    pub fn max_health(self) -> f32 {
        match self {
            PropKind::Tree => 40.0,
            PropKind::Boulder => 120.0,
            PropKind::Crystal => 60.0,
            _ => 10.0,
        }
    }
}

/// Что с объектом сейчас.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PropStatus {
    Intact,
    // Падение, пень и уничтожение появятся с разрушением объектов (задача 5.1).
    #[allow(dead_code)]
    Fallen(Transform),
    #[allow(dead_code)]
    Stump,
    #[allow(dead_code)]
    Destroyed,
}

/// Состояние одного объекта на весь забег.
#[derive(Debug, Clone, PartialEq)]
pub struct PropState {
    pub kind: PropKind,
    /// Основание объекта на поверхности, поворот и масштаб.
    pub transform: Transform,
    pub variant: usize,
    #[allow(dead_code)]
    pub health: f32,
    pub status: PropStatus,
}

/// Объекты забега по участкам рельефа.
#[derive(Resource, Default)]
pub struct Props {
    seed: Option<u32>,
    masks: Option<Masks>,
    chunks: HashMap<IVec2, Vec<PropState>>,
}

impl Props {
    pub fn new(seed: u32) -> Self {
        Self {
            seed: Some(seed),
            masks: Some(Masks::new(seed)),
            chunks: HashMap::new(),
        }
    }

    /// Объекты участка, генерирует их при первом обращении.
    pub fn chunk(
        &mut self,
        coord: IVec2,
        terrain: &Terrain,
        settlements: &mut Settlements,
    ) -> &mut Vec<PropState> {
        let (Some(seed), Some(masks)) = (self.seed, self.masks.as_ref()) else {
            return self.chunks.entry(coord).or_default();
        };
        self.chunks
            .entry(coord)
            .or_insert_with(|| generate_chunk(seed, masks, coord, terrain, settlements))
    }

    // Пишет разрушение объектов (задача 5.1).
    #[allow(dead_code)]
    pub fn get_mut(&mut self, coord: IVec2, index: usize) -> Option<&mut PropState> {
        self.chunks.get_mut(&coord)?.get_mut(index)
    }
}

/// Маски групп объектов.
struct Masks {
    forest: Perlin,
    rocks: Perlin,
    crystals: Perlin,
}

impl Masks {
    fn new(seed: u32) -> Self {
        Self {
            forest: Perlin::new(seed.wrapping_add(FOREST_SEED)),
            rocks: Perlin::new(seed.wrapping_add(ROCKS_SEED)),
            crystals: Perlin::new(seed.wrapping_add(CRYSTAL_SEED)),
        }
    }

    /// Значение маски в точке, примерно от 0 до 1.
    fn sample(noise: &Perlin, p: Vec2) -> f32 {
        let v = noise.get([p.x as f64 * MASK_FREQUENCY, p.y as f64 * MASK_FREQUENCY]) as f32;
        (v * 0.5 + 0.5).clamp(0.0, 1.0)
    }
}

/// Генерирует объекты участка. Один сид и один участок дают одни и те же объекты.
fn generate_chunk(
    seed: u32,
    masks: &Masks,
    coord: IVec2,
    terrain: &Terrain,
    settlements: &mut Settlements,
) -> Vec<PropState> {
    let mut rng = Rng::new(seed, coord);
    let origin = chunk_origin(coord);
    let houses = nearby_houses(coord, terrain, settlements);
    let cell = CHUNK_SIZE as f32 / CELLS as f32;
    let mut props = Vec::new();

    for iz in 0..CELLS {
        for ix in 0..CELLS {
            let p = origin
                + Vec2::new(
                    (ix as f32 + rng.range(0.1, 0.9)) * cell,
                    (iz as f32 + rng.range(0.1, 0.9)) * cell,
                );
            let forest = Masks::sample(&masks.forest, p);
            let rocks = Masks::sample(&masks.rocks, p);
            let crystals = Masks::sample(&masks.crystals, p);
            let roll = rng.next_f32();
            let kind = if crystals > 0.72 && roll < 0.55 {
                PropKind::Crystal
            } else if forest > 0.56 {
                match roll {
                    r if r < 0.6 => PropKind::Tree,
                    r if r < 0.85 => PropKind::Bush,
                    _ => continue,
                }
            } else if rocks > 0.58 {
                match roll {
                    r if r < 0.3 => PropKind::Boulder,
                    r if r < 0.75 => PropKind::Rock,
                    _ => continue,
                }
            } else {
                // Между группами — редкие кусты, камни и одинокие деревья, иногда поляны.
                match roll {
                    r if r < 0.22 => PropKind::Bush,
                    r if r < 0.40 => PropKind::Rock,
                    r if r < 0.47 => PropKind::Tree,
                    _ => continue,
                }
            };
            if p.length() < START_CLEAR {
                continue;
            }
            if houses
                .iter()
                .any(|h| h.footprint().distance(p) < HOUSE_MARGIN)
            {
                continue;
            }
            if kind.is_large() && terrain.normal_at(p.x, p.y).y < LARGE_MIN_UP {
                continue;
            }
            props.push(make_prop(&mut rng, kind, p, terrain));
        }
    }

    add_village_props(&mut props, coord, &houses, terrain);
    props
}

/// Объект на поверхности в точке `p` со случайным поворотом, масштабом и вариантом.
fn make_prop(rng: &mut Rng, kind: PropKind, p: Vec2, terrain: &Terrain) -> PropState {
    let scale = match kind {
        PropKind::Tree => rng.range(0.8, 1.35),
        PropKind::Boulder => rng.range(0.8, 1.4),
        PropKind::Crystal => rng.range(0.7, 1.3),
        _ => rng.range(0.75, 1.25),
    };
    let up = terrain.normal_at(p.x, p.y);
    // Деревья и кристаллы растут вертикально, камни и кусты ложатся по склону.
    let tilt = match kind {
        PropKind::Tree | PropKind::Crystal | PropKind::Post | PropKind::Antenna | PropKind::Fence => {
            Quat::IDENTITY
        }
        _ => Quat::from_rotation_arc(Vec3::Y, up),
    };
    PropState {
        kind,
        transform: Transform::from_xyz(p.x, terrain.height_at(p.x, p.y), p.y)
            .with_rotation(tilt * Quat::from_rotation_y(rng.range(0.0, TAU)))
            .with_scale(Vec3::splat(scale)),
        variant: rng.range_u32(0, VARIANTS as u32 - 1) as usize,
        health: kind.max_health(),
        status: PropStatus::Intact,
    }
}

/// Дома, которые могут задевать участок: из регионов вокруг него.
fn nearby_houses(coord: IVec2, terrain: &Terrain, settlements: &mut Settlements) -> Vec<HouseState> {
    let origin = chunk_origin(coord);
    let size = CHUNK_SIZE as f32;
    let margin = 20.0;
    let low = region_of(origin - Vec2::splat(margin));
    let high = region_of(origin + Vec2::splat(size + margin));
    debug_assert!(high.x - low.x <= (size + 2.0 * margin) as i32 / REGION_SIZE as i32 + 1);
    let mut houses = Vec::new();
    for rz in low.y..=high.y {
        for rx in low.x..=high.x {
            let region = settlements.region(IVec2::new(rx, rz), terrain);
            houses.extend(region.houses.iter().cloned());
        }
    }
    houses
}

/// Заборы перед домами, столбы по их концам и антенны во дворах.
///
/// Решение о заборе зависит только от дома, поэтому каждый участок кладёт к себе ровно те
/// секции, которые попали в него, и забор на границе участков не двоится.
fn add_village_props(
    props: &mut Vec<PropState>,
    coord: IVec2,
    houses: &[HouseState],
    terrain: &Terrain,
) {
    for house in houses {
        let footprint = house.footprint();
        let mut rng = Rng::from_position(house.position);
        let half = footprint.half;
        if rng.next_f32() < 0.7 {
            // Забор вдоль фасада с дверью, секции по 2 м.
            let z = half.y + 3.0;
            let x0 = -half.x - 1.5;
            let sections = ((2.0 * half.x + 3.0) / 2.0).round() as i32;
            let section = (2.0 * half.x + 3.0) / sections as f32;
            for i in 0..sections {
                // Калитка напротив двери.
                if sections > 2 && i == sections / 2 {
                    continue;
                }
                let local = Vec2::new(x0 + (i as f32 + 0.5) * section, z);
                let p = footprint.to_world(local);
                if chunk_coord_at(p.x, p.y) != coord {
                    continue;
                }
                props.push(PropState {
                    kind: PropKind::Fence,
                    transform: Transform::from_xyz(p.x, terrain.height_at(p.x, p.y), p.y)
                        .with_rotation(Quat::from_rotation_y(house.yaw))
                        .with_scale(Vec3::new(section / 2.0, 1.0, 1.0)),
                    variant: house.wall_color % VARIANTS,
                    health: PropKind::Fence.max_health(),
                    status: PropStatus::Intact,
                });
            }
            for x in [x0, -x0] {
                let p = footprint.to_world(Vec2::new(x, z));
                if chunk_coord_at(p.x, p.y) == coord {
                    props.push(make_prop(&mut rng, PropKind::Post, p, terrain));
                }
            }
        }
        if rng.next_f32() < 0.35 {
            let p = footprint.to_world(Vec2::new(half.x + 1.5, -half.y + 1.0));
            if chunk_coord_at(p.x, p.y) == coord {
                props.push(make_prop(&mut rng, PropKind::Antenna, p, terrain));
            }
        }
    }
}

/// Новый забег: объекты строятся заново от нового сида.
fn reset_props(
    mut started: MessageReader<RunStarted>,
    mut props: ResMut<Props>,
    mut spawned: ResMut<SpawnedProps>,
) {
    if let Some(run) = started.read().last() {
        *props = Props::new(run.seed);
        // Сущности прошлого забега удалены через RunScoped.
        spawned.0.clear();
    }
}

/// Сущность объекта и адрес его состояния в [`Props`].
// Адрес читают столкновения и разрушение (задачи 2.3 и 5.1).
#[allow(dead_code)]
#[derive(Component, Debug, Clone, Copy)]
pub struct Prop {
    pub chunk: IVec2,
    pub index: usize,
}

/// Сущности объектов по участкам, которые сейчас в сцене.
#[derive(Resource, Default)]
struct SpawnedProps(HashMap<IVec2, Vec<Entity>>);

/// Часть облика объекта: меш, материал и положение относительно основания.
struct Part {
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
    transform: Transform,
}

/// Общие меши и материалы: одинаковые объекты рисуются одним вызовом.
#[derive(Resource)]
struct PropAssets {
    /// Облик по виду и варианту.
    looks: HashMap<(PropKind, usize), Vec<Part>>,
}

fn create_prop_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut mat = |color: Color, emissive: LinearRgba| {
        materials.add(StandardMaterial {
            base_color: color,
            emissive,
            perceptual_roughness: 0.8,
            ..default()
        })
    };
    let none = LinearRgba::BLACK;

    let sphere = meshes.add(Sphere::new(1.0).mesh().ico(2).unwrap());
    let rock = meshes.add(Sphere::new(1.0).mesh().ico(1).unwrap());
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let cylinder = meshes.add(Cylinder::new(1.0, 1.0));
    let crystal = meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(5));
    let cone = meshes.add(Cone::new(1.0, 1.0));

    let trunk = mat(Color::srgb(0.42, 0.28, 0.20), none);
    let wood = mat(Color::srgb(0.62, 0.46, 0.30), none);
    let metal = mat(Color::srgb(0.75, 0.78, 0.82), none);
    // Странные инопланетные кроны: бирюзовая, сиреневая, лимонная, мятная.
    let crowns = [
        Color::srgb(0.10, 0.75, 0.70),
        Color::srgb(0.70, 0.35, 0.90),
        Color::srgb(0.85, 0.90, 0.20),
        Color::srgb(0.35, 0.85, 0.40),
    ];
    let bushes = [
        Color::srgb(0.20, 0.60, 0.35),
        Color::srgb(0.45, 0.70, 0.20),
        Color::srgb(0.25, 0.55, 0.60),
        Color::srgb(0.55, 0.40, 0.70),
    ];
    let stones = [
        Color::srgb(0.55, 0.52, 0.50),
        Color::srgb(0.48, 0.45, 0.52),
        Color::srgb(0.62, 0.57, 0.48),
        Color::srgb(0.42, 0.44, 0.46),
    ];
    let gems = [
        Color::srgb(0.30, 0.85, 1.00),
        Color::srgb(0.85, 0.40, 1.00),
        Color::srgb(0.40, 1.00, 0.70),
        Color::srgb(1.00, 0.85, 0.30),
    ];

    let part = |mesh: &Handle<Mesh>, material: &Handle<StandardMaterial>, t: Transform| Part {
        mesh: mesh.clone(),
        material: material.clone(),
        transform: t,
    };
    let mut looks = HashMap::new();
    for v in 0..VARIANTS {
        let crown = mat(crowns[v], none);
        let bush = mat(bushes[v], none);
        let stone = mat(stones[v], none);
        let gem_color = gems[v].to_linear();
        let gem = mat(gems[v], gem_color * 1.5);
        let fv = v as f32;

        // Дерево: ствол и крона из двух-трёх шаров.
        let trunk_h = 2.2 + fv * 0.3;
        let mut tree = vec![part(
            &cylinder,
            &trunk,
            Transform::from_xyz(0.0, trunk_h * 0.5, 0.0).with_scale(Vec3::new(0.25, trunk_h, 0.25)),
        )];
        let blobs: &[(Vec3, f32)] = match v {
            0 => &[(Vec3::new(0.0, 0.0, 0.0), 1.4)],
            1 => &[(Vec3::new(-0.5, -0.1, 0.0), 1.0), (Vec3::new(0.5, 0.3, 0.2), 1.1)],
            2 => &[
                (Vec3::new(0.0, 0.0, 0.0), 1.1),
                (Vec3::new(0.0, 1.0, 0.0), 0.8),
                (Vec3::new(0.0, 1.7, 0.0), 0.5),
            ],
            _ => &[
                (Vec3::new(-0.6, 0.0, -0.3), 0.9),
                (Vec3::new(0.6, 0.1, -0.2), 0.9),
                (Vec3::new(0.0, 0.6, 0.5), 1.0),
            ],
        };
        for &(offset, r) in blobs {
            tree.push(part(
                &sphere,
                &crown,
                Transform::from_translation(Vec3::new(0.0, trunk_h + 0.6, 0.0) + offset)
                    .with_scale(Vec3::new(r, r * 0.85, r)),
            ));
        }
        looks.insert((PropKind::Tree, v), tree);

        looks.insert(
            (PropKind::Bush, v),
            vec![
                part(
                    &sphere,
                    &bush,
                    Transform::from_xyz(0.0, 0.35, 0.0).with_scale(Vec3::new(0.9, 0.5, 0.8)),
                ),
                part(
                    &sphere,
                    &bush,
                    Transform::from_xyz(0.4, 0.3, 0.3).with_scale(Vec3::new(0.5, 0.4, 0.5)),
                ),
            ],
        );
        looks.insert(
            (PropKind::Rock, v),
            vec![part(
                &rock,
                &stone,
                Transform::from_xyz(0.0, 0.15, 0.0)
                    .with_scale(Vec3::new(0.45 + fv * 0.05, 0.3, 0.35)),
            )],
        );
        looks.insert(
            (PropKind::Boulder, v),
            vec![
                part(
                    &rock,
                    &stone,
                    Transform::from_xyz(0.0, 0.6, 0.0)
                        .with_rotation(Quat::from_rotation_y(fv))
                        .with_scale(Vec3::new(1.6, 1.2 + fv * 0.1, 1.3)),
                ),
                part(
                    &rock,
                    &stone,
                    Transform::from_xyz(0.9, 0.3, 0.6).with_scale(Vec3::new(0.7, 0.5, 0.6)),
                ),
            ],
        );
        // Кристалл: несколько вытянутых светящихся призм под разными углами.
        let mut shards = Vec::new();
        for (i, (h, lean)) in [(2.6, 0.0), (1.7, 0.45), (1.3, -0.5), (1.0, 0.7)]
            .iter()
            .enumerate()
            .take(2 + v % 3)
        {
            let around = Quat::from_rotation_y(i as f32 * 2.1);
            shards.push(part(
                &crystal,
                &gem,
                Transform::from_rotation(around * Quat::from_rotation_z(*lean))
                    * Transform::from_xyz(0.0, h * 0.5, 0.0).with_scale(Vec3::new(0.35, *h, 0.35)),
            ));
        }
        looks.insert((PropKind::Crystal, v), shards);

        // Секция забора 2 м вдоль локального X: две перекладины и штакетник.
        let mut fence = Vec::new();
        for y in [0.35, 0.8] {
            fence.push(part(
                &cube,
                &wood,
                Transform::from_xyz(0.0, y, 0.0).with_scale(Vec3::new(2.0, 0.1, 0.06)),
            ));
        }
        for x in [-0.75, -0.25, 0.25, 0.75] {
            fence.push(part(
                &cube,
                &wood,
                Transform::from_xyz(x, 0.5, 0.0).with_scale(Vec3::new(0.12, 1.0, 0.05)),
            ));
        }
        looks.insert((PropKind::Fence, v), fence);
        looks.insert(
            (PropKind::Post, v),
            vec![part(
                &cube,
                &wood,
                Transform::from_xyz(0.0, 0.65, 0.0).with_scale(Vec3::new(0.2, 1.3, 0.2)),
            )],
        );
        // Антенна: мачта и тарелка, смотрящая в небо.
        looks.insert(
            (PropKind::Antenna, v),
            vec![
                part(
                    &cylinder,
                    &metal,
                    Transform::from_xyz(0.0, 1.6, 0.0).with_scale(Vec3::new(0.08, 3.2, 0.08)),
                ),
                part(
                    &cone,
                    &metal,
                    Transform::from_xyz(0.0, 3.2, 0.0)
                        .with_rotation(Quat::from_rotation_x(PI + 0.5))
                        .with_scale(Vec3::new(0.7, 0.3, 0.7)),
                ),
            ],
        );
    }
    commands.insert_resource(PropAssets { looks });
}

/// Создаёт объекты участков, которые появились в сцене, и убирает объекты выгруженных.
fn stream_props(
    mut commands: Commands,
    mut props: ResMut<Props>,
    mut spawned: ResMut<SpawnedProps>,
    mut settlements: ResMut<Settlements>,
    terrain: Res<Terrain>,
    visible: Res<VisibleChunks>,
    chunks: Query<&crate::terrain::ChunkMesh>,
    assets: Res<PropAssets>,
) {
    if props.seed.is_none() || settlements.seed().is_none() || !visible.is_changed() {
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
        let states = props.chunk(coord, &terrain, &mut settlements);
        let mut entities = Vec::with_capacity(states.len());
        for (index, state) in states.iter().enumerate() {
            let transform = match state.status {
                PropStatus::Intact => state.transform,
                PropStatus::Fallen(t) => t,
                PropStatus::Stump | PropStatus::Destroyed => continue,
            };
            let Some(parts) = assets.looks.get(&(state.kind, state.variant)) else {
                continue;
            };
            let small = !state.kind.is_large();
            let entity = commands
                .spawn((Prop { chunk: coord, index }, RunScoped, transform, Visibility::default()))
                .with_children(|parent| {
                    for p in parts {
                        let mut child = parent.spawn((
                            Mesh3d(p.mesh.clone()),
                            MeshMaterial3d(p.material.clone()),
                            p.transform,
                        ));
                        if small {
                            child.insert(NotShadowCaster);
                        }
                    }
                })
                .id();
            entities.push(entity);
        }
        spawned.0.insert(coord, entities);
    }
}

/// Детерминированный генератор SplitMix64.
struct Rng(u64);

impl Rng {
    fn new(seed: u32, coord: IVec2) -> Self {
        let mut state = (seed as u64) << 32 ^ 0x9409_5EED;
        state ^= (coord.x as u32 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        state ^= (coord.y as u32 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        let mut rng = Self(state);
        rng.next_u64();
        rng
    }

    /// Генератор, зависящий только от точки: одинаковый для всех участков.
    fn from_position(p: Vec2) -> Self {
        let mut rng = Self((p.x.to_bits() as u64) << 32 | p.y.to_bits() as u64);
        rng.next_u64();
        rng
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next_f32()
    }

    fn range_u32(&mut self, low: u32, high: u32) -> u32 {
        low + (self.next_u64() % (high - low + 1) as u64) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generate(seed: u32, coord: IVec2) -> (Vec<PropState>, Vec<HouseState>) {
        let terrain = Terrain::new(seed);
        let mut settlements = Settlements::new(seed);
        let props = generate_chunk(seed, &Masks::new(seed), coord, &terrain, &mut settlements);
        let houses = nearby_houses(coord, &terrain, &mut settlements);
        (props, houses)
    }

    fn sample_chunks() -> impl Iterator<Item = IVec2> {
        (-3..=3).flat_map(|z| (-3..=3).map(move |x| IVec2::new(x, z)))
    }

    #[test]
    fn same_seed_gives_same_props() {
        for coord in sample_chunks() {
            assert_eq!(generate(9, coord).0, generate(9, coord).0, "участок {coord}");
        }
    }

    #[test]
    fn no_props_inside_houses_or_at_start() {
        for seed in [3, 9, 77] {
            for coord in sample_chunks() {
                let (props, houses) = generate(seed, coord);
                for prop in &props {
                    let p = Vec2::new(prop.transform.translation.x, prop.transform.translation.z);
                    assert!(p.length() >= START_CLEAR, "сид {seed}: объект у старта в {p}");
                    for house in &houses {
                        assert!(
                            house.footprint().distance(p) > 0.5,
                            "сид {seed}: {:?} внутри дома в {p}",
                            prop.kind
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn chunk_has_at_least_60_props() {
        for seed in [3, 9, 77] {
            for coord in sample_chunks() {
                let (props, houses) = generate(seed, coord);
                // Посёлок занимает место объектов: проверяем участки без домов поблизости.
                if !houses.is_empty() || coord == IVec2::ZERO {
                    continue;
                }
                assert!(props.len() >= 60, "сид {seed}, участок {coord}: {}", props.len());
            }
        }
    }
}
