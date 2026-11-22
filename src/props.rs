//! Объекты на поверхности: деревья, кусты, камни, валуны, кристаллы кварца, заборы, столбы и антенны.
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

use crate::fire::Fire;
use crate::camera::Focus;
use crate::frame::GameSet;
use crate::models::Models;
use crate::obstacles::{Obstacle, Shape};
use crate::rover::Rover;
use crate::run::{RunScoped, WorldRebuilt};
use crate::vehicle::GroundVehicle;
use crate::combat::{Collider, Damage, Died, Health, blast_damage};
use crate::debris::{DebrisShape, SpawnDebris};
use crate::debris_forms::FormKind;
use crate::fragments::{Breaker, Burst};
use crate::explosion::Explosion;
use crate::settlements::{HouseState, REGION_SIZE, Settlements, region_of};
use crate::terrain::{CHUNK_SIZE, Terrain, VisibleChunks, chunk_coord_at, chunk_origin};

/// Ячеек сетки по стороне участка: шаг около 2,5 м.
/// Сколько дымит крона после огня, с.
const CROWN_SMOKE: f32 = 8.0;

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

/// Доля объектов, которая остаётся после выбора вида: дважды убрано по трети, чтобы можно было ехать.
const KEEP_FRACTION: f32 = 4.0 / 9.0;

/// Вариантов цвета и формы у каждого вида.
const VARIANTS: usize = 4;

/// Цвета камней по вариантам.
const STONE_COLORS: [Color; VARIANTS] = [
    Color::srgb(0.50, 0.49, 0.47),
    Color::srgb(0.46, 0.40, 0.33),
    Color::srgb(0.62, 0.57, 0.48),
    Color::srgb(0.30, 0.29, 0.28),
];

/// Цвета кварца по вариантам: молочно-белый, бледно-голубой, сероватый, голубоватый лёд.
/// Кварц не светится: свечение оставлено огню, вспышкам, лазеру, лаве и раскалённому металлу.
const GEM_COLORS: [Color; VARIANTS] = [
    Color::srgb(0.90, 0.92, 0.94),
    Color::srgb(0.76, 0.85, 0.93),
    Color::srgb(0.84, 0.85, 0.83),
    Color::srgb(0.83, 0.91, 0.95),
];

/// Сколько объектов создаётся за кадр, когда в сцену входят новые участки: целыми участками,
/// ближние к фокусу первыми. Без предела на границе участка за кадр появлялось около 1300.
const SPAWN_PER_FRAME: usize = 250;

/// Шероховатость матовых поверхностей и гладких граней кварца.
const MATTE: f32 = 0.8;
const GLOSSY: f32 = 0.25;

/// Подключает объекты поверхности.
pub struct PropsPlugin;

impl Plugin for PropsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Props>()
            .init_resource::<SpawnedProps>()
            .add_systems(Startup, create_prop_assets)
            .add_systems(
                Update,
                (
                    reset_props.in_set(GameSet::Reset),
                    crush_small_props.in_set(GameSet::Simulation),
                    shockwave_props.in_set(GameSet::Damage),
                    (remember_prop_health, break_props).chain().in_set(GameSet::Death),
                    remove_dead_props.in_set(GameSet::Cleanup),
                    // После луж: объекты на месте лужи уступают ей до появления.
                    stream_props.after(crate::puddles::place_puddles).in_set(GameSet::Streaming),
                    (fly_shards, fell_trees, sway_trees, burn_crowns).chain().in_set(GameSet::Presentation),
                ),
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

    /// Радиус в плане и высота при масштабе 1, м: для столкновений и пуль.
    pub fn footprint(self) -> (f32, f32) {
        match self {
            // Мешает только ствол: под крону луноход заезжает.
            PropKind::Tree => (0.3, 6.5),
            PropKind::Boulder => (1.5, 1.6),
            PropKind::Crystal => (0.8, 2.6),
            PropKind::Bush => (0.8, 0.7),
            PropKind::Rock => (0.45, 0.3),
            PropKind::Fence => (1.0, 1.0),
            PropKind::Post | PropKind::Antenna => (0.3, 3.0),
        }
    }

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
    /// Дерево упало: положение лежащего ствола. Пень стоит на месте основания.
    Fallen(Transform),
    Destroyed,
}

/// Состояние одного объекта на весь забег.
#[derive(Debug, Clone, PartialEq)]
pub struct PropState {
    pub kind: PropKind,
    /// Основание объекта на поверхности, поворот и масштаб.
    pub transform: Transform,
    pub variant: usize,
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
            // Как в природе: больше всего деревьев, кусты, немного камней и редкие кристаллы.
            let kind = if crystals > 0.8 && roll < 0.15 {
                PropKind::Crystal
            } else if forest > 0.5 {
                match roll {
                    r if r < 0.7 => PropKind::Tree,
                    r if r < 0.9 => PropKind::Bush,
                    _ => continue,
                }
            } else if rocks > 0.7 {
                match roll {
                    r if r < 0.12 => PropKind::Boulder,
                    r if r < 0.4 => PropKind::Rock,
                    _ => continue,
                }
            } else {
                // Между группами — одинокие деревья, кусты и редкие камни, иногда поляны.
                match roll {
                    r if r < 0.18 => PropKind::Tree,
                    r if r < 0.32 => PropKind::Bush,
                    r if r < 0.37 => PropKind::Rock,
                    _ => continue,
                }
            };
            if rng.next_f32() >= KEEP_FRACTION {
                continue;
            }
            if p.length() < START_CLEAR {
                continue;
            }
            if terrain.lines().nearest(p).distance < crate::railway::TRACK_BAND {
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

/// Мир перестроен: объекты строятся заново от нового сида.
fn reset_props(
    mut rebuilt: MessageReader<WorldRebuilt>,
    mut props: ResMut<Props>,
    mut spawned: ResMut<SpawnedProps>,
) {
    if let Some(world) = rebuilt.read().last() {
        *props = Props::new(world.seed);
        // Сущности прошлого забега удалены через RunScoped.
        spawned.0.clear();
    }
}

/// Сущность объекта и адрес его состояния в [`Props`].
#[derive(Component, Debug, Clone, Copy)]
pub struct Prop {
    pub chunk: IVec2,
    pub index: usize,
    pub kind: PropKind,
}

/// Сущности объектов по участкам, которые сейчас в сцене.
#[derive(Resource, Default)]
struct SpawnedProps(HashMap<IVec2, Vec<Entity>>);

impl SpawnedProps {
    /// Сущности объектов из участков, которые задевает круг `radius` вокруг `center`: вместо обхода
    /// всех объектов мира. Удалённые сущности тоже попадаются — их отсеивает запрос.
    fn near(&self, center: Vec3, radius: f32) -> impl Iterator<Item = Entity> + '_ {
        let low = chunk_coord_at(center.x - radius, center.z - radius);
        let high = chunk_coord_at(center.x + radius, center.z + radius);
        (low.x..=high.x)
            .flat_map(move |x| (low.y..=high.y).map(move |z| IVec2::new(x, z)))
            .filter_map(|coord| self.0.get(&coord))
            .flatten()
            .copied()
    }
}

/// Запас к радиусу машины при поиске мелочи под колёсами: больше радиуса любого `Crushable`.
const CRUSH_SEARCH_MARGIN: f32 = 6.0;

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
    /// Пень упавшего дерева.
    stump: Part,
}

fn create_prop_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut mat = |color: Color, roughness: f32| {
        materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: roughness,
            ..default()
        })
    };

    let sphere = meshes.add(Sphere::new(1.0).mesh().ico(2).unwrap());
    let rock = meshes.add(Sphere::new(1.0).mesh().ico(1).unwrap());
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let cylinder = meshes.add(Cylinder::new(1.0, 1.0));
    let crystal = meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(5));
    let cone = meshes.add(Cone::new(1.0, 1.0));

    let trunk = mat(Color::srgb(0.42, 0.28, 0.20), MATTE);
    let wood = mat(Color::srgb(0.62, 0.46, 0.30), MATTE);
    let metal = mat(Color::srgb(0.75, 0.78, 0.82), MATTE);
    // Зелёные кроны: лиственное дерево, ель, тополь, сосна. Хвоя темнее листвы.
    let crowns = [
        Color::srgb(0.24, 0.50, 0.18),
        Color::srgb(0.12, 0.34, 0.18),
        Color::srgb(0.36, 0.56, 0.20),
        Color::srgb(0.16, 0.38, 0.20),
    ];
    let bushes = [
        Color::srgb(0.22, 0.45, 0.16),
        Color::srgb(0.30, 0.50, 0.18),
        Color::srgb(0.18, 0.38, 0.17),
        Color::srgb(0.34, 0.44, 0.20),
    ];
    // Камни естественных цветов: серый, бурый, светлый песчаник, тёмный базальт.
    let stones = STONE_COLORS;
    let gems = GEM_COLORS;

    let part = |mesh: &Handle<Mesh>, material: &Handle<StandardMaterial>, t: Transform| Part {
        mesh: mesh.clone(),
        material: material.clone(),
        transform: t,
    };
    let mut looks = HashMap::new();
    for v in 0..VARIANTS {
        let crown = mat(crowns[v], MATTE);
        let bush = mat(bushes[v], MATTE);
        let stone = mat(stones[v], MATTE);
        // Кварц без эмиссии: грани блестят на солнце за счёт гладкой поверхности.
        let gem = mat(gems[v], GLOSSY);
        let fv = v as f32;

        // Дерево: высокий ствол и крона выше лунохода, разная форма у каждого варианта.
        let trunk_h = [3.4, 2.8, 3.0, 4.0][v];
        let mut tree = vec![part(
            &cylinder,
            &trunk,
            Transform::from_xyz(0.0, trunk_h * 0.5, 0.0).with_scale(Vec3::new(0.22, trunk_h, 0.22)),
        )];
        // Части кроны: меш, смещение от верха ствола и масштаб.
        let crown_parts: Vec<(&Handle<Mesh>, Vec3, Vec3)> = match v {
            // Лиственное: пышная крона из нескольких шаров.
            0 => vec![
                (&sphere, Vec3::new(0.0, 0.8, 0.0), Vec3::new(1.5, 1.2, 1.5)),
                (&sphere, Vec3::new(-0.9, 0.4, 0.3), Vec3::new(1.0, 0.8, 1.0)),
                (&sphere, Vec3::new(0.8, 0.5, -0.4), Vec3::new(1.1, 0.9, 1.0)),
                (&sphere, Vec3::new(0.2, 1.6, 0.2), Vec3::new(0.9, 0.7, 0.9)),
            ],
            // Ель: ярусы конусов, сужающиеся кверху.
            1 => vec![
                (&cone, Vec3::new(0.0, 0.6, 0.0), Vec3::new(1.6, 1.6, 1.6)),
                (&cone, Vec3::new(0.0, 1.6, 0.0), Vec3::new(1.2, 1.4, 1.2)),
                (&cone, Vec3::new(0.0, 2.5, 0.0), Vec3::new(0.8, 1.2, 0.8)),
            ],
            // Тополь: узкая высокая крона.
            2 => vec![
                (&sphere, Vec3::new(0.0, 1.8, 0.0), Vec3::new(0.8, 2.4, 0.8)),
                (&sphere, Vec3::new(0.2, 0.8, 0.1), Vec3::new(0.7, 1.2, 0.7)),
            ],
            // Сосна: голый высокий ствол и плотная шапка хвои наверху.
            _ => vec![
                (&sphere, Vec3::new(0.0, 0.5, 0.0), Vec3::new(1.5, 0.9, 1.4)),
                (&sphere, Vec3::new(0.4, 1.2, -0.2), Vec3::new(1.0, 0.7, 1.0)),
                (&sphere, Vec3::new(-0.8, 0.1, 0.3), Vec3::new(0.8, 0.5, 0.8)),
            ],
        };
        for (mesh, offset, scale) in crown_parts {
            tree.push(part(
                mesh,
                &crown,
                Transform::from_translation(Vec3::new(0.0, trunk_h, 0.0) + offset).with_scale(scale),
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
        // Кварц: несколько вытянутых призм под разными углами.
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
    let stump = part(
        &cylinder,
        &trunk,
        Transform::from_xyz(0.0, 0.25, 0.0).with_scale(Vec3::new(0.3, 0.5, 0.3)),
    );
    commands.insert_resource(PropAssets { looks, stump });
}

/// Модель объекта из `models.json`: четыре дерева по вариантам (лиственное, ель, тополь, сосна),
/// куст, валун, кварц. Камни помельче, заборы и столбы — из примитивов.
fn prop_model(kind: PropKind, variant: usize) -> Option<&'static str> {
    match kind {
        PropKind::Tree => Some(["tree_oak", "tree_spruce", "tree_poplar", "tree_pine"][variant % 4]),
        PropKind::Bush => Some("bush"),
        PropKind::Boulder => Some("boulder"),
        PropKind::Crystal => Some("crystal"),
        _ => None,
    }
}

/// Ставит облик объекта детьми `parent`: модель, пока она грузится или без неё — примитивы.
fn spawn_prop_look(
    parent: &mut ChildSpawnerCommands,
    models: &mut Models,
    parts: &[Part],
    model: Option<&str>,
    small: bool,
) {
    let bundle = |p: &Part| (Mesh3d(p.mesh.clone()), MeshMaterial3d(p.material.clone()), p.transform);
    match model {
        Some(id) => models.spawn_visual(parent, id, Transform::IDENTITY, |spare| {
            for p in parts {
                spare.spawn(bundle(p));
            }
        }),
        None => {
            for p in parts {
                let mut child = parent.spawn(bundle(p));
                if small {
                    child.insert(NotShadowCaster);
                }
            }
        }
    }
}

/// Создаёт объекты участков, которые появились в сцене, и убирает объекты выгруженных.
///
/// Новые участки создаются не все сразу: за кадр не больше [`SPAWN_PER_FRAME`] объектов,
/// ближние к фокусу первыми.
#[allow(clippy::too_many_arguments)]
fn stream_props(
    mut commands: Commands,
    mut props: ResMut<Props>,
    mut spawned: ResMut<SpawnedProps>,
    mut settlements: ResMut<Settlements>,
    terrain: Res<Terrain>,
    visible: Res<VisibleChunks>,
    focus: Res<Focus>,
    chunks: Query<&crate::terrain::ChunkMesh>,
    assets: Res<PropAssets>,
    mut models: Models,
) {
    if props.seed.is_none() || settlements.seed().is_none() {
        return;
    }
    if visible.is_changed() {
        spawned.0.retain(|&coord, entities| {
            let keep = visible.entity(coord).is_some();
            if !keep {
                for &e in entities.iter() {
                    commands.entity(e).try_despawn();
                }
            }
            keep
        });
    }
    let mut pending: Vec<IVec2> = chunks
        .iter()
        .map(|chunk| chunk.0)
        .filter(|&coord| !spawned.0.contains_key(&coord) && visible.entity(coord).is_some())
        .collect();
    if pending.is_empty() {
        return;
    }
    let here = chunk_coord_at(focus.0.x, focus.0.z);
    pending.sort_by_key(|coord| (*coord - here).length_squared());
    let mut budget = SPAWN_PER_FRAME;
    for coord in pending {
        if budget == 0 {
            break;
        }
        let states = props.chunk(coord, &terrain, &mut settlements);
        budget = budget.saturating_sub(states.len().max(1));
        let mut entities = Vec::with_capacity(states.len());
        for (index, state) in states.iter().enumerate() {
            let transform = match state.status {
                PropStatus::Intact => state.transform,
                PropStatus::Fallen(t) => {
                    entities.push(spawn_stump(&mut commands, &assets, coord, index, state.transform));
                    // Лежащее дерево — без столкновений и здоровья.
                    let entity = commands
                        .spawn((
                            Remains {
                                chunk: coord,
                                index,
                                stump: false,
                            },
                            RunScoped,
                            t,
                            Visibility::default(),
                        ))
                        .with_children(|parent| {
                            let parts = assets.looks.get(&(state.kind, state.variant)).map_or(&[][..], |p| p);
                            let model = prop_model(state.kind, state.variant);
                            spawn_prop_look(parent, &mut models, parts, model, false);
                        })
                        .id();
                    entities.push(entity);
                    continue;
                }
                PropStatus::Destroyed => continue,
            };
            let Some(parts) = assets.looks.get(&(state.kind, state.variant)) else {
                continue;
            };
            let small = !state.kind.is_large();
            let mut entity = commands.spawn((
                Prop {
                    chunk: coord,
                    index,
                    kind: state.kind,
                },
                RunScoped,
                transform,
                Visibility::default(),
            ));
            let base = Vec2::new(transform.translation.x, transform.translation.z);
            let (radius, height) = state.kind.footprint();
            let scale = transform.scale.max_element();
            if small {
                // Кусты, камни, заборы и столбы — мелочь: без тени.
                entity.insert((
                    Crushable {
                        radius: radius * scale,
                    },
                    crate::models::NoShadow,
                ));
            } else {
                entity.insert(Health {
                    current: state.health.min(state.kind.max_health()),
                    max: state.kind.max_health(),
                });
                entity.insert(Obstacle {
                    shape: Shape::Circle {
                        center: base,
                        radius: radius * scale,
                    },
                    top: transform.translation.y + height * scale,
                });
            }
            let entity = entity
                .with_children(|parent| {
                    let model = prop_model(state.kind, state.variant);
                    spawn_prop_look(parent, &mut models, parts, model, small);
                })
                .id();
            entities.push(entity);
        }
        spawned.0.insert(coord, entities);
    }
}

/// Мелкий объект: машина сносит его на ходу.
#[derive(Component, Debug, Clone, Copy)]
pub struct Crushable {
    pub radius: f32,
}

/// Осколок снесённого объекта: летит, кувыркается и исчезает.
#[derive(Component)]
struct Shard {
    velocity: Vec3,
    spin: Vec3,
    remaining: f32,
}

/// Сколько живёт осколок, с.
const SHARD_LIFETIME: f32 = 1.0;

/// Гравитация для осколков, м/с².
const SHARD_GRAVITY: f32 = 20.0;

/// Машина, наехавшая на мелкий объект, сносит его: объект исчезает навсегда, летят осколки.
/// Мелочь под машинами: ищется в участках вокруг каждой машины, а не перебором всех объектов мира.
fn crush_small_props(
    mut commands: Commands,
    mut props: ResMut<Props>,
    spawned: Res<SpawnedProps>,
    vehicles: Query<(&Transform, Option<&Collider>), Or<(With<Rover>, With<GroundVehicle>)>>,
    crushable: Query<(&Prop, &Crushable, &Transform)>,
    breaker: Breaker,
    mut crushed: Local<Vec<Entity>>,
) {
    crushed.clear();
    for (vehicle, collider) in &vehicles {
        let reach = collider.map_or(1.0, |c| c.radius);
        for entity in spawned.near(vehicle.translation, reach + CRUSH_SEARCH_MARGIN) {
            let Ok((prop, small, transform)) = crushable.get(entity) else {
                continue;
            };
            let p = transform.translation;
            let r = reach + small.radius;
            if Vec2::new(vehicle.translation.x - p.x, vehicle.translation.z - p.z).length_squared() >= r * r
                || crushed.contains(&entity)
            {
                continue;
            }
            crushed.push(entity);
            if let Some(state) = props.get_mut(prop.chunk, prop.index) {
                state.status = PropStatus::Destroyed;
            }
            commands.entity(entity).despawn();

            let away = (p - vehicle.translation).with_y(0.0).normalize_or(Vec3::X);
            shatter(&mut commands, transform, &breaker, entity, away);
        }
    }
}

/// Осколки летят в сторону `away` и вверх. Берутся меши всех потомков объекта, в том числе
/// сцены модели: у нарезанной модели (куст) — её куски со своих мест, у примитивов (забор,
/// столб, антенна) — каждая деталь, уменьшенная, со своим масштабом.
fn shatter(commands: &mut Commands, transform: &Transform, breaker: &Breaker, root: Entity, away: Vec3) {
    let p = transform.translation;
    let mut rng = Rng::from_position(Vec2::new(p.x, p.z));
    for (i, part) in breaker.parts(root).into_iter().enumerate() {
        let copies = if part.piece { 1 } else { 2 };
        for _ in 0..copies {
            let velocity = away * rng.range(3.0, 7.0)
                + Vec3::new(rng.range(-2.5, 2.5), rng.range(4.0, 8.0), rng.range(-2.5, 2.5));
            let (at, scale) = if part.piece {
                (part.transform.translation, part.transform.scale * rng.range(0.8, 1.0))
            } else {
                (p + Vec3::Y * 0.4, part.transform.scale * rng.range(0.2, 0.35))
            };
            commands.spawn((
                Shard {
                    velocity,
                    spin: Vec3::new(rng.range(-9.0, 9.0), rng.range(-9.0, 9.0), rng.range(-9.0, 9.0)),
                    remaining: SHARD_LIFETIME + i as f32 * 0.05,
                },
                RunScoped,
                Mesh3d(part.mesh.clone()),
                MeshMaterial3d(part.material.clone()),
                NotShadowCaster,
                Transform::from_translation(at)
                    .with_rotation(part.transform.rotation)
                    .with_scale(scale),
            ));
        }
    }
}

/// Двигает осколки по баллистике и уменьшает их к концу жизни.
fn fly_shards(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut shards: Query<(Entity, &mut Transform, &mut Shard)>,
) {
    let dt = time.delta_secs();
    for (entity, mut transform, mut shard) in &mut shards {
        shard.remaining -= dt;
        if shard.remaining <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        shard.velocity.y -= SHARD_GRAVITY * dt;
        transform.translation += shard.velocity * dt;
        let ground = terrain.height_at(transform.translation.x, transform.translation.z);
        if transform.translation.y < ground {
            transform.translation.y = ground;
            shard.velocity *= Vec3::new(0.5, -0.3, 0.5);
        }
        let spin = shard.spin * dt;
        transform.rotate(Quat::from_euler(EulerRot::XYZ, spin.x, spin.y, spin.z));
        if shard.remaining < 0.3 {
            transform.scale *= 1.0 - dt * 3.0;
        }
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

/// Множитель урона взрыва по крупным объектам.
const PROP_BLAST_MULTIPLIER: f32 = 2.0;

/// За радиусом взрыва до этой доли радиуса деревья качаются.
const SWAY_REACH: f32 = 1.8;

/// Сколько падает дерево, с.
const FALL_SECONDS: f32 = 0.8;

/// Сколько качается дерево, с.
const SWAY_SECONDS: f32 = 1.0;

/// Центр взрыва, который задел объект: дерево падает от него.
#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct PropBlastFrom(pub(crate) Vec3);

/// Дерево падает: поворот вокруг основания.
#[derive(Component)]
struct TreeFall {
    rest: Quat,
    axis: Vec3,
    elapsed: f32,
}

/// Дерево качается от дальнего взрыва: затухающая пружина наклона.
#[derive(Component)]
struct Sway {
    rest: Quat,
    axis: Vec3,
    elapsed: f32,
    amplitude: f32,
}

/// Огонь на кроне упавшего дерева.
#[derive(Component)]
struct CrownFire {
    remaining: f32,
}

/// Остатки упавшего дерева: пень или лежащий ствол. Адрес — состояние дерева в [`Props`]:
/// пень стоит на его основании, ствол лежит в `PropStatus::Fallen`.
#[derive(Component, Debug, Clone, Copy)]
pub struct Remains {
    pub chunk: IVec2,
    pub index: usize,
    pub stump: bool,
}

/// Пень на месте упавшего дерева. Мелкий, без тени.
fn spawn_stump(commands: &mut Commands, assets: &PropAssets, chunk: IVec2, index: usize, base: Transform) -> Entity {
    commands
        .spawn((
            Remains {
                chunk,
                index,
                stump: true,
            },
            RunScoped,
            NotShadowCaster,
            Mesh3d(assets.stump.mesh.clone()),
            MeshMaterial3d(assets.stump.material.clone()),
            Transform::from_translation(base.translation)
                .with_scale(base.scale)
                * assets.stump.transform,
        ))
        .id()
}

/// Ударная волна: мелочь в радиусе разлетается, деревья ломаются, крупным — урон;
/// чуть дальше деревья качаются.
#[allow(clippy::too_many_arguments)]
fn shockwave_props(
    mut commands: Commands,
    mut explosions: MessageReader<Explosion>,
    mut props: ResMut<Props>,
    spawned: Res<SpawnedProps>,
    objects: Query<(&Prop, &Transform, Option<&Health>, Option<&Sway>), Without<TreeFall>>,
    breaker: Breaker,
    mut damage: MessageWriter<Damage>,
    mut shattered: Local<Vec<Entity>>,
) {
    // Два взрыва в одном кадре не разбивают одну мелочь дважды: команды удаления применятся позже.
    shattered.clear();
    for explosion in explosions.read() {
        let center = explosion.position;
        let r = explosion.radius;
        for entity in spawned.near(center, r * SWAY_REACH) {
            let Ok((prop, transform, health, sway)) = objects.get(entity) else {
                continue;
            };
            let offset = (transform.translation - center).with_y(0.0);
            let d = offset.length();
            if d >= r * SWAY_REACH {
                continue;
            }
            let away = offset.normalize_or(Vec3::X);
            if d >= r {
                // Качается только стоящее дерево; лежащий ствол здоровья не имеет.
                if prop.kind == PropKind::Tree && health.is_some() {
                    commands.entity(entity).try_insert(Sway {
                        // Новое качание от положения покоя, а не от наклона прошлого.
                        rest: sway.map_or(transform.rotation, |s| s.rest),
                        axis: Vec3::Y.cross(away).normalize_or(Vec3::X),
                        elapsed: 0.0,
                        amplitude: 0.35 * (1.0 - (d - r) / (r * (SWAY_REACH - 1.0))),
                    });
                }
                continue;
            }
            if !prop.kind.is_large() {
                if shattered.contains(&entity) {
                    continue;
                }
                shattered.push(entity);
                if let Some(state) = props.get_mut(prop.chunk, prop.index) {
                    state.status = PropStatus::Destroyed;
                }
                shatter(&mut commands, transform, &breaker, entity, away);
                commands.entity(entity).despawn();
                continue;
            }
            let Some(health) = health else {
                continue;
            };
            let amount = if prop.kind == PropKind::Tree {
                // Дерево в радиусе ломается всегда.
                health.current + 1.0
            } else {
                blast_damage(explosion.power, r, d, false) * PROP_BLAST_MULTIPLIER
            };
            commands.entity(entity).try_insert(PropBlastFrom(center));
            damage.write(Damage {
                target: entity,
                amount,
                source_team: explosion.source_team,
            });
        }
    }
}

/// Разрушение по материалу: дерево падает и горит, валун и кристалл разлетаются.
#[allow(clippy::too_many_arguments)]
fn break_props(
    mut commands: Commands,
    mut died: MessageReader<Died>,
    mut props: ResMut<Props>,
    mut spawned: ResMut<SpawnedProps>,
    assets: Res<PropAssets>,
    objects: Query<(&Prop, &Transform, Option<&PropBlastFrom>, Option<&Sway>)>,
    mut breaker: Breaker,
    bank: Res<crate::models::ModelBank>,
) {
    for death in died.read() {
        let Ok((prop, transform, blast, sway)) = objects.get(death.entity) else {
            continue;
        };
        let Some(state) = props.get_mut(prop.chunk, prop.index) else {
            continue;
        };
        if state.status != PropStatus::Intact {
            continue;
        }
        let p = transform.translation;
        let away = blast
            .map(|b| (p - b.0).with_y(0.0).normalize_or(Vec3::X))
            .unwrap_or_else(|| Quat::from_rotation_y(p.x * 7.3 + p.z * 3.1) * Vec3::X);
        match prop.kind {
            PropKind::Tree => {
                let axis = Vec3::Y.cross(away).normalize_or(Vec3::X);
                // Качавшееся дерево падает от положения покоя: качание снимается, иначе оно
                // вернуло бы ствол в вертикаль.
                let rest = sway.map_or(transform.rotation, |s| s.rest);
                // Высота — с основания в хранилище: взрыв, сваливший дерево, в этом же кадре уже
                // опустил его в кратер (`subsidence::sink_props`), а сущность ещё оседает сверху.
                // Со старой высотой следующий кратер опустил бы ствол лишний раз, под землю.
                let fallen = Transform {
                    translation: state.transform.translation,
                    rotation: Quat::from_axis_angle(axis, std::f32::consts::FRAC_PI_2) * rest,
                    ..*transform
                };
                state.status = PropStatus::Fallen(fallen);
                let flame_height = 3.0 * transform.scale.y;
                let burn = 4.0 + (p.x * 1.7).sin().abs() * 2.0;
                // Только «мягкие» команды: дерево могло исчезнуть в этом же кадре, и это не паника.
                // Огонь — на кроне, ребёнком падающего дерева; пламя модуля стоит вертикально.
                commands.spawn((
                    CrownFire { remaining: burn + 0.2 },
                    Fire::new(1.4 * transform.scale.y, 0.9).for_seconds(burn).then_smoke(CROWN_SMOKE),
                    ChildOf(death.entity),
                    Transform::from_xyz(0.0, flame_height, 0.0),
                ));
                commands
                    .entity(death.entity)
                    .try_remove::<(Obstacle, Health, Sway)>()
                    .try_insert((
                        TreeFall {
                            rest,
                            axis,
                            elapsed: 0.0,
                        },
                        Remains {
                            chunk: prop.chunk,
                            index: prop.index,
                            stump: false,
                        },
                    ));
                let stump = spawn_stump(&mut commands, &assets, prop.chunk, prop.index, state.transform);
                spawned.0.entry(prop.chunk).or_default().push(stump);
            }
            PropKind::Boulder | PropKind::Crystal => {
                state.status = PropStatus::Destroyed;
                let boulder = prop.kind == PropKind::Boulder;
                let scale = transform.scale.max_element();
                // Куски своей модели со своих мест, разлёт от стороны удара.
                let pieces = breaker.burst(
                    death.entity,
                    Burst {
                        from: p - away * scale - Vec3::Y * 0.5 * scale,
                        speed: 4.0,
                        lift: 5.0,
                        share: 1.0,
                        burning: 0,
                        charred: false,
                    },
                );
                // Плюс мелочь из библиотеки: каменные глыбы и крошка у валуна, вытянутые блестящие
                // осколки у кварца. Без нарезанной модели мелочи больше.
                let (form, color, size) = if boulder {
                    (FormKind::Chunk, STONE_COLORS[state.variant], Vec3::new(0.45, 0.35, 0.4))
                } else {
                    (FormKind::Shard, GEM_COLORS[state.variant], Vec3::new(0.18, 0.5, 0.18))
                };
                let count = if pieces > 0 { 4 } else { 7 } + prop.index % 3;
                let colors = prop_model(prop.kind, state.variant).and_then(|id| bank.colors(id));
                for i in 0..count {
                    let color = colors.as_ref().map_or(color, |c| c.any(i + prop.index));
                    let a = i as f32 / count as f32 * std::f32::consts::TAU + p.x;
                    let dir = (Quat::from_rotation_y(a) * Vec3::X + away).normalize_or(Vec3::X);
                    let crumb = boulder && i % 3 == 2;
                    breaker.debris.write(SpawnDebris {
                        position: p + Vec3::Y * (0.5 + 0.2 * i as f32) * scale,
                        velocity: dir * (3.0 + i as f32 * 0.6) + Vec3::Y * (5.0 + (i % 3) as f32 * 2.0),
                        size: if crumb { size * 0.4 } else { size } * scale * (0.7 + (i % 2) as f32 * 0.5),
                        color,
                        burning: false,
                        shape: DebrisShape::Form(if crumb { FormKind::Crumb } else { form }),
                        rotation: None,
                    });
                }
            }
            _ => {
                state.status = PropStatus::Destroyed;
            }
        }
    }
}

/// Прочность раненого объекта пишется в хранилище: валун, выгруженный и загруженный снова,
/// помнит её. Пишется уже применённое значение, урон не вычитается дважды.
fn remember_prop_health(mut props: ResMut<Props>, objects: Query<(&Prop, &Health), Changed<Health>>) {
    for (prop, health) in &objects {
        if let Some(state) = props.get_mut(prop.chunk, prop.index) {
            state.health = health.current;
        }
    }
}

/// Разбитые валуны и кристаллы уходят из сцены в уборке: их застают все читатели `Died`.
/// Упавшее дерево здоровья уже лишено и остаётся лежать.
fn remove_dead_props(mut commands: Commands, objects: Query<(Entity, &Health), With<Prop>>) {
    for (entity, health) in &objects {
        if health.is_dead() {
            commands.entity(entity).try_despawn();
        }
    }
}

/// Дерево падает с ускорением и коротко подпрыгивает, коснувшись земли.
fn fell_trees(mut commands: Commands, time: Res<Time>, mut trees: Query<(Entity, &mut Transform, &mut TreeFall)>) {
    let half_pi = std::f32::consts::FRAC_PI_2;
    for (entity, mut transform, mut fall) in &mut trees {
        fall.elapsed += time.delta_secs();
        let t = fall.elapsed / FALL_SECONDS;
        let angle = if t < 1.0 {
            half_pi * t * t
        } else {
            let b = (t - 1.0) / 0.25;
            if b >= 1.0 {
                transform.rotation = Quat::from_axis_angle(fall.axis, half_pi) * fall.rest;
                commands.entity(entity).try_remove::<TreeFall>();
                continue;
            }
            half_pi - 0.12 * (b * std::f32::consts::PI).sin()
        };
        transform.rotation = Quat::from_axis_angle(fall.axis, angle) * fall.rest;
    }
}

/// Затухающее качание деревьев от дальнего взрыва.
fn sway_trees(mut commands: Commands, time: Res<Time>, mut trees: Query<(Entity, &mut Transform, &mut Sway), Without<TreeFall>>) {
    for (entity, mut transform, mut sway) in &mut trees {
        sway.elapsed += time.delta_secs();
        if sway.elapsed >= SWAY_SECONDS {
            transform.rotation = sway.rest;
            commands.entity(entity).try_remove::<Sway>();
            continue;
        }
        let t = sway.elapsed;
        let angle = sway.amplitude * (-4.0 * t).exp() * (t * 14.0).cos();
        transform.rotation = Quat::from_axis_angle(sway.axis, angle) * sway.rest;
    }
}

/// Носитель огня кроны удаляется, когда огонь догорел: дым после него ставит модуль огня.
fn burn_crowns(mut commands: Commands, time: Res<Time>, mut fires: Query<(Entity, &mut CrownFire)>) {
    for (entity, mut fire) in &mut fires {
        fire.remaining -= time.delta_secs();
        if fire.remaining <= 0.0 {
            commands.entity(entity).despawn();
        }
    }
}
