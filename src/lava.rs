//! Лава под корой: глубокий кратер светится трещинами, пробитая кора извергается и разливает озеро.
//!
//! Глубина точки — исходная высота рельефа минус текущая. Глубже [`CRUST`] — прорыв: извержение
//! с каплями и взрывом, затем озеро. Озеро — горизонтальная поверхность на уровне `level` в связной
//! области ячеек 1×1 м, где рельеф ниже уровня, не дальше 40 м от места прорыва. Плоскость лавы
//! рисуется над всеми ячейками с запасом, а рельеф выше уровня сам закрывает её по глубине.
//!
//! Лава жжёт: луноход теряет здоровье и вязнет, враги и жители сгорают, предметы исчезают,
//! звенья путей рвутся. Все враги, кроме таран-стрелков, объезжают озёра.

use std::collections::{HashSet, VecDeque};

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::NotShadowCaster;
use bevy::math::Affine2;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::combat::{Damage, Health, Team};
use crate::craters;
use crate::enemy::Enemy;
use crate::explosion::Explosion;
use crate::frame::GameSet;
use crate::pickups::Pickup;
use crate::rails::TrackDamage;
use crate::residents::Resident;
use crate::rover::{Rover, RoverMotion};
use crate::run::{RunScoped, WorldRebuilt};
use crate::subsidence::TerrainChanged;
use crate::terrain::{ChunkDirty, Terrain, VisibleChunks};
use crate::waves::Threat;

/// Толщина коры, м: мортира углубляет на 4,9 м за выстрел, прорыв — на седьмом.
pub const CRUST: f32 = 32.0;
/// С какой доли коры на дне появляются трещины и с какой — дым.
const CRACKS_FROM: f32 = 0.6;
const SMOKE_FROM: f32 = 0.75;
/// Наибольшее удаление заливки от места прорыва, м.
const POOL_REACH: i32 = 40;
/// Уровень озера ниже исходной поверхности, м.
const POOL_BELOW: f32 = 4.0;
/// За сколько озеро набирает уровень, с.
const FILL_TIME: f32 = 6.0;
/// Как часто пересчитывается заливка, с.
const REFILL: f32 = 0.5;
/// Подъём уровня от взрыва в лаве, м.
const BLAST_RISE: f32 = 0.6;
/// Урон в секунду: луноху и врагам.
const ROVER_BURN: f32 = 22.0;
const ENEMY_BURN: f32 = 60.0;
/// Урон капли и её радиус.
const DROP_DAMAGE: f32 = 15.0;
const DROP_RADIUS: f32 = 1.5;
/// Вес прорыва в угрозе.
const BREACH_THREAT: f32 = 15.0;
/// Сколько горячих точек дымит одновременно.
const MAX_HOT_SPOTS: usize = 12;

/// Плагин лавы.
pub struct LavaPlugin;

impl Plugin for LavaPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LavaPools>()
            .init_resource::<HotSpots>()
            .add_message::<CrustBreached>()
            .add_systems(Startup, create_assets)
            .add_systems(
                Update,
                (
                    reset_lava.in_set(GameSet::Reset),
                    (burn_in_lava, fly_drops).chain().in_set(GameSet::Simulation),
                    // После кратеров этого кадра: кора проверяется по новому рельефу, озёра
                    // сразу пересчитываются.
                    (watch_crust, blast_in_lava, grow_pools)
                        .chain()
                        .after(craters::dig_craters)
                        .in_set(GameSet::Damage),
                    (draw_pools, animate_lava, smoke_hot_spots, fade_smoke)
                        .chain()
                        .in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Кора пробита: на это подписываются угроза, штаб и музыка.
#[derive(Message, Debug, Clone, Copy)]
pub struct CrustBreached {
    #[allow(dead_code)]
    pub position: Vec3,
}

/// Лавовое озеро.
pub struct Pool {
    origin: IVec2,
    level: f32,
    start_level: f32,
    target_level: f32,
    age: f32,
    refill: f32,
    cells: HashSet<IVec2>,
    /// Сущность плоскости лавы и номер версии ячеек, по которой она построена.
    mesh: Option<Entity>,
    dirty: bool,
}

/// Озёра забега.
#[derive(Resource, Default)]
pub struct LavaPools {
    pools: Vec<Pool>,
}

impl LavaPools {
    /// Уровень лавы в точке плана, если она в озере.
    pub fn lava_at(&self, x: f32, z: f32) -> Option<f32> {
        let cell = IVec2::new(x.round() as i32, z.round() as i32);
        self.pools.iter().find(|p| p.cells.contains(&cell)).map(|p| p.level)
    }

    /// Ячейки всех озёр: для мини-карты.
    pub fn cells(&self) -> impl Iterator<Item = &IVec2> {
        self.pools.iter().flat_map(|p| p.cells.iter())
    }
}

/// Дымящие глубокие кратеры.
#[derive(Resource, Default)]
struct HotSpots {
    spots: Vec<Vec3>,
    timer: f32,
}

/// Капля лавы в полёте.
#[derive(Component)]
struct Drop {
    velocity: Vec3,
}

/// Клуб дыма или искра над горячим кратером.
#[derive(Component)]
struct Smoke {
    velocity: Vec3,
    age: f32,
    life: f32,
}

/// Плоскость лавы: слой течения или корок.
#[derive(Component)]
struct LavaSurface {
    crust: bool,
}

#[derive(Resource)]
struct LavaAssets {
    ball: Handle<Mesh>,
    drop: Handle<StandardMaterial>,
    smoke: Handle<StandardMaterial>,
    spark: Handle<StandardMaterial>,
    flow: Handle<StandardMaterial>,
    crust: Handle<StandardMaterial>,
}

/// Шумовая текстура для течения и корок: светлые прожилки на тёмном.
fn noise_image(seed: f32, crust: bool) -> Image {
    const N: usize = 64;
    let mut data = Vec::with_capacity(N * N * 4);
    for y in 0..N {
        for x in 0..N {
            let (u, v) = (x as f32 / N as f32 * std::f32::consts::TAU, y as f32 / N as f32 * std::f32::consts::TAU);
            let n = (u * 2.0 + (v * 3.0 + seed).sin() * 1.5).sin() * (v * 2.0 + (u * 3.0 - seed).cos() * 1.5).cos();
            let n = n * 0.5 + 0.5;
            if crust {
                // Тёмные пятна корок, между ними прозрачно.
                let a = ((n - 0.55) * 5.0).clamp(0.0, 1.0);
                data.extend_from_slice(&[25, 15, 10, (a * 230.0) as u8]);
            } else {
                let k = n.powf(1.5);
                data.extend_from_slice(&[255, (90.0 + 140.0 * k) as u8, (20.0 + 60.0 * k) as u8, 255]);
            }
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: N as u32,
            height: N as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });
    image
}

fn create_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let flow = images.add(noise_image(0.0, false));
    let crust = images.add(noise_image(2.3, true));
    commands.insert_resource(LavaAssets {
        ball: meshes.add(Sphere::new(1.0).mesh().ico(1).expect("капля лавы")),
        drop: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.5, 0.1),
            emissive: LinearRgba::rgb(8.0, 2.5, 0.4),
            unlit: true,
            ..default()
        }),
        smoke: materials.add(StandardMaterial {
            base_color: Color::srgba(0.2, 0.18, 0.17, 0.5),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            ..default()
        }),
        spark: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.7, 0.2),
            emissive: LinearRgba::rgb(10.0, 4.0, 0.8),
            unlit: true,
            ..default()
        }),
        flow: materials.add(StandardMaterial {
            // Ярче единицы: светится через свечение камеры.
            base_color: Color::linear_rgb(4.0, 2.0, 1.2),
            base_color_texture: Some(flow),
            unlit: true,
            ..default()
        }),
        crust: materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(crust),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            ..default()
        }),
    });
}

fn reset_lava(
    mut started: MessageReader<WorldRebuilt>,
    mut pools: ResMut<LavaPools>,
    mut spots: ResMut<HotSpots>,
) {
    if started.read().last().is_some() {
        // Плоскости, капли и дым — сущности забега, удалены через RunScoped.
        *pools = LavaPools::default();
        *spots = HotSpots::default();
    }
}

/// Узор трещин: 0 — корка, 1 — светящаяся линия.
fn crack(x: f32, z: f32) -> f32 {
    let f = ((x * 0.9 + (z * 0.45).sin() * 2.0).sin() * (z * 0.8 + (x * 0.5).sin() * 2.0).sin()).abs();
    (1.0 - f / 0.18).clamp(0.0, 1.0)
}

/// Глубина вершины от исходной поверхности, м.
fn depth(terrain: &Terrain, vx: i32, vz: i32) -> f32 {
    terrain.original_height(vx, vz) - terrain.vertex_height(vx, vz)
}

/// Следит за глубиной в изменённых местах: трещины на дне, дым, прорыв коры.
#[allow(clippy::too_many_arguments)]
fn watch_crust(
    mut commands: Commands,
    mut changes: MessageReader<TerrainChanged>,
    mut terrain: ResMut<Terrain>,
    visible: Res<VisibleChunks>,
    mut pools: ResMut<LavaPools>,
    mut spots: ResMut<HotSpots>,
    mut threat: ResMut<Threat>,
    assets: Res<LavaAssets>,
    mut explosions: MessageWriter<Explosion>,
    mut breached: MessageWriter<CrustBreached>,
) {
    let areas: Vec<(Vec3, f32)> = changes.read().map(|c| (c.center, c.radius)).collect();
    let mut dirty: HashSet<IVec2> = HashSet::new();
    for (center, radius) in areas {
        let r = radius.ceil() as i32;
        let (cx, cz) = (center.x.round() as i32, center.z.round() as i32);
        let mut deepest = (0.0f32, IVec2::new(cx, cz));
        for vz in cz - r..=cz + r {
            for vx in cx - r..=cx + r {
                let d = depth(&terrain, vx, vz);
                if d > deepest.0 {
                    deepest = (d, IVec2::new(vx, vz));
                }
                if d < CRUST * CRACKS_FROM {
                    continue;
                }
                // Чем глубже, тем ярче трещины: у самой коры линии светятся.
                let k = ((d / CRUST - CRACKS_FROM) / (1.0 - CRACKS_FROM)).clamp(0.0, 1.0);
                let line = crack(vx as f32, vz as f32) * k;
                for c in terrain.edit_vertex(vx, vz, |_, color| {
                    let base = [0.08, 0.06, 0.05];
                    let glow = [3.0, 0.9, 0.15];
                    for i in 0..3 {
                        color[i] = base[i] + (glow[i] - base[i]) * line;
                    }
                }) {
                    dirty.insert(c);
                }
            }
        }
        let (d, cell) = deepest;
        let bottom = Vec3::new(cell.x as f32, terrain.vertex_height(cell.x, cell.y), cell.y as f32);
        if d >= CRUST * SMOKE_FROM && !spots.spots.iter().any(|s| s.distance(bottom) < 4.0) {
            spots.spots.push(bottom);
            if spots.spots.len() > MAX_HOT_SPOTS {
                spots.spots.remove(0);
            }
        }
        let in_pool = pools.pools.iter().any(|p| p.cells.contains(&cell) || p.origin.as_vec2().distance(cell.as_vec2()) < 6.0);
        if d < CRUST || in_pool {
            continue;
        }
        // Прорыв: извержение и новое озеро.
        let target = terrain.original_height(cell.x, cell.y) - POOL_BELOW;
        pools.pools.push(Pool {
            origin: cell,
            level: bottom.y,
            start_level: bottom.y,
            target_level: target,
            age: 0.0,
            refill: 0.0,
            cells: HashSet::from([cell]),
            mesh: None,
            dirty: true,
        });
        spots.spots.retain(|s| s.distance(bottom) > 10.0);
        explosions.write(Explosion {
            position: bottom,
            radius: 4.0,
            power: 1.8,
            source_team: Team::Player,
        });
        breached.write(CrustBreached { position: bottom });
        threat.add_destruction(BREACH_THREAT);
        // Столб лавы и капли по дугам.
        for i in 0..40 {
            let a = i as f32 * 2.399;
            let column = i < 15;
            let (side, up) = if column { (1.5, 28.0 + (i % 5) as f32 * 3.0) } else { (5.0 + (i % 4) as f32 * 2.5, 14.0 + (i % 3) as f32 * 3.0) };
            commands.spawn((
                Drop {
                    velocity: Vec3::new(a.cos() * side, up, a.sin() * side),
                },
                RunScoped,
                NotShadowCaster,
                Mesh3d(assets.ball.clone()),
                MeshMaterial3d(assets.drop.clone()),
                Transform::from_translation(bottom + Vec3::Y).with_scale(Vec3::splat(if column { 0.6 } else { 0.35 })),
            ));
        }
    }
    for coord in dirty {
        for o in [IVec2::ZERO, IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y] {
            if let Some(e) = visible.entity(coord + o) {
                commands.entity(e).try_insert(ChunkDirty);
            }
        }
    }
}

/// Заливка: связная область ячеек ниже уровня, не дальше `reach` от начала.
pub fn flood(origin: IVec2, level: f32, reach: i32, height: impl Fn(i32, i32) -> f32) -> HashSet<IVec2> {
    let mut cells = HashSet::new();
    let mut queue = VecDeque::from([origin]);
    cells.insert(origin);
    while let Some(c) = queue.pop_front() {
        for o in [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y] {
            let n = c + o;
            if cells.contains(&n) || (n - origin).length_squared() > reach * reach || height(n.x, n.y) >= level {
                continue;
            }
            cells.insert(n);
            queue.push_back(n);
        }
    }
    cells
}

/// Взрыв в лаве поднимает её уровень и выплёскивает капли.
fn blast_in_lava(
    mut commands: Commands,
    mut explosions: MessageReader<Explosion>,
    mut pools: ResMut<LavaPools>,
    assets: Res<LavaAssets>,
) {
    for e in explosions.read() {
        let cell = IVec2::new(e.position.x.round() as i32, e.position.z.round() as i32);
        let Some(pool) = pools.pools.iter_mut().find(|p| p.cells.contains(&cell) && p.age > 0.5) else {
            continue;
        };
        pool.target_level += BLAST_RISE;
        for i in 0..8 {
            let a = i as f32 * 0.785 + e.position.x;
            commands.spawn((
                Drop {
                    velocity: Vec3::new(a.cos() * 5.0, 10.0 + (i % 3) as f32 * 2.0, a.sin() * 5.0),
                },
                RunScoped,
                NotShadowCaster,
                Mesh3d(assets.ball.clone()),
                MeshMaterial3d(assets.drop.clone()),
                Transform::from_translation(Vec3::new(e.position.x, pool.level + 0.3, e.position.z)).with_scale(Vec3::splat(0.3)),
            ));
        }
    }
}

/// Уровень озёр растёт к целевому, заливка пересчитывается, дотянувшиеся друг до друга озёра сливаются.
fn grow_pools(time: Res<Time>, terrain: Res<Terrain>, mut pools: ResMut<LavaPools>, mut track: ResMut<TrackDamage>) {
    let dt = time.delta_secs();
    for pool in &mut pools.pools {
        pool.age += dt;
        let t = (pool.age / FILL_TIME).clamp(0.0, 1.0);
        let smooth = t * t * (3.0 - 2.0 * t);
        let wanted = pool.start_level + (pool.target_level - pool.start_level) * smooth;
        // После наполнения уровень догоняет подъёмы от взрывов.
        pool.level = if t < 1.0 { wanted } else { pool.level + (pool.target_level - pool.level) * (dt * 2.0).min(1.0) };
        pool.refill -= dt;
        if pool.refill > 0.0 {
            continue;
        }
        pool.refill = REFILL;
        let cells = flood(pool.origin, pool.level, POOL_REACH, |x, z| terrain.vertex_height(x, z));
        if cells != pool.cells {
            pool.cells = cells;
            pool.dirty = true;
        }
    }
    // Слияние: общие ячейки — одно озеро с наибольшим уровнем.
    let mut i = 0;
    while i < pools.pools.len() {
        let mut j = i + 1;
        while j < pools.pools.len() {
            if !pools.pools[i].cells.is_disjoint(&pools.pools[j].cells) {
                let other = pools.pools.remove(j);
                let pool = &mut pools.pools[i];
                pool.cells.extend(other.cells);
                pool.level = pool.level.max(other.level);
                pool.target_level = pool.target_level.max(other.target_level);
                pool.dirty = true;
                // Плоскость слитого озера удаляет draw_pools: её больше нет ни у одного озера.
                continue;
            }
            j += 1;
        }
        i += 1;
    }
    // Звенья путей в лаве рвутся.
    for pool in &pools.pools {
        if !pool.dirty {
            continue;
        }
        for near in terrain.lines().near(pool.origin.as_vec2(), POOL_REACH as f32) {
            let mut s = near.s - POOL_REACH as f32;
            while s < near.s + POOL_REACH as f32 {
                let p = terrain.lines().point(near.line, s);
                if pool.cells.contains(&IVec2::new(p.x.round() as i32, p.y.round() as i32)) {
                    track.break_link((near.line, (s / crate::rails::LINK_LENGTH).floor() as i32));
                }
                s += 1.0;
            }
        }
    }
}

/// Строит меш плоскости по ячейкам с запасом в одну ячейку.
fn surface_mesh(cells: &HashSet<IVec2>, level: f32) -> Mesh {
    let mut covered: HashSet<IVec2> = HashSet::new();
    for c in cells {
        for dz in -1..=1 {
            for dx in -1..=1 {
                covered.insert(*c + IVec2::new(dx, dz));
            }
        }
    }
    let mut positions = Vec::with_capacity(covered.len() * 4);
    let mut uvs = Vec::with_capacity(covered.len() * 4);
    let mut indices = Vec::with_capacity(covered.len() * 6);
    for c in &covered {
        let base = positions.len() as u32;
        for (dx, dz) in [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
            let (x, z) = (c.x as f32 + dx, c.y as f32 + dz);
            positions.push([x, level, z]);
            uvs.push([x / 8.0, z / 8.0]);
        }
        indices.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
    }
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices))
}

/// Пересобирает плоскости озёр при изменении ячеек и держит их на уровне.
fn draw_pools(
    mut commands: Commands,
    mut pools: ResMut<LavaPools>,
    assets: Res<LavaAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut roots: Query<&mut Transform, With<LavaSurface>>,
    surfaces: Query<(Entity, &LavaSurface)>,
) {
    let mut alive: Vec<Entity> = Vec::new();
    for pool in &mut pools.pools {
        if pool.dirty {
            pool.dirty = false;
            if let Some(old) = pool.mesh.take() {
                commands.entity(old).try_despawn();
            }
            let flat = surface_mesh(&pool.cells, 0.0);
            let mesh = meshes.add(flat);
            let root = commands
                .spawn((
                    LavaSurface { crust: false },
                    RunScoped,
                    NotShadowCaster,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(assets.flow.clone()),
                    Transform::from_xyz(0.0, pool.level, 0.0),
                    Visibility::default(),
                ))
                .with_children(|p| {
                    p.spawn((
                        LavaSurface { crust: true },
                        NotShadowCaster,
                        Mesh3d(mesh),
                        MeshMaterial3d(assets.crust.clone()),
                        Transform::from_xyz(0.0, 0.04, 0.0),
                    ));
                    let c = pool.origin.as_vec2();
                    p.spawn((
                        PointLight {
                            color: Color::srgb(1.0, 0.45, 0.1),
                            intensity: 3_000_000.0,
                            range: 40.0,
                            shadow_maps_enabled: false,
                            ..default()
                        },
                        Transform::from_xyz(c.x, 3.0, c.y),
                    ));
                })
                .id();
            pool.mesh = Some(root);
        }
        if let Some(root) = pool.mesh {
            alive.push(root);
            if let Ok(mut t) = roots.get_mut(root) {
                t.translation.y = pool.level;
            }
        }
    }
    // Плоскости слившихся озёр.
    for (entity, surface) in &surfaces {
        if !surface.crust && !alive.contains(&entity) {
            commands.entity(entity).despawn();
        }
    }
}

/// Течение: текстура лавы и корок сдвигается в разные стороны, свет мерцает.
fn animate_lava(
    time: Res<Time>,
    assets: Res<LavaAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    pools: Res<LavaPools>,
    mut lights: Query<&mut PointLight, With<ChildOf>>,
    surfaces: Query<&Children, With<LavaSurface>>,
) {
    if pools.pools.is_empty() {
        return;
    }
    let t = time.elapsed_secs();
    if let Some(mut m) = materials.get_mut(&assets.flow) {
        m.uv_transform = Affine2::from_translation(Vec2::new(t * 0.05, t * 0.02));
    }
    if let Some(mut m) = materials.get_mut(&assets.crust) {
        m.uv_transform = Affine2::from_translation(Vec2::new(-t * 0.015, t * 0.01));
    }
    for children in &surfaces {
        for child in children.iter() {
            if let Ok(mut light) = lights.get_mut(child) {
                light.intensity = 3_000_000.0 * (0.8 + 0.2 * (t * 7.0).sin() * (t * 3.1).cos());
            }
        }
    }
}

/// Лава жжёт: луноход теряет здоровье и вязнет, враги и жители сгорают, предметы исчезают.
#[allow(clippy::type_complexity)]
fn burn_in_lava(
    mut commands: Commands,
    time: Res<Time>,
    pools: Res<LavaPools>,
    terrain: Res<Terrain>,
    mut rover: Query<(Entity, &Transform, &mut RoverMotion, &Health), With<Rover>>,
    enemies: Query<(Entity, &Transform), With<Enemy>>,
    residents: Query<(Entity, &Transform), With<Resident>>,
    pickups: Query<(Entity, &Transform), With<Pickup>>,
    mut damage: MessageWriter<Damage>,
) {
    if pools.pools.is_empty() {
        return;
    }
    let dt = time.delta_secs();
    let in_lava = |t: &Transform| {
        pools
            .lava_at(t.translation.x, t.translation.z)
            .is_some_and(|level| terrain.height_at(t.translation.x, t.translation.z) < level)
    };
    for (entity, transform, mut motion, health) in &mut rover {
        if !health.is_dead() && in_lava(transform) {
            damage.write(Damage {
                target: entity,
                amount: ROVER_BURN * dt,
                source_team: Team::Neutral,
            });
            motion.speed *= (1.0 - 1.5 * dt).max(0.0);
        }
    }
    for (entity, transform) in &enemies {
        if in_lava(transform) {
            damage.write(Damage {
                target: entity,
                amount: ENEMY_BURN * dt,
                source_team: Team::Neutral,
            });
        }
    }
    for (entity, transform) in &residents {
        if in_lava(transform) {
            damage.write(Damage {
                target: entity,
                amount: 5.0,
                source_team: Team::Neutral,
            });
        }
    }
    for (entity, transform) in &pickups {
        if in_lava(transform) {
            commands.entity(entity).despawn();
        }
    }
}

/// Капли летят по дугам и жгут всё со здоровьем, куда упали.
fn fly_drops(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut drops: Query<(Entity, &mut Transform, &mut Drop)>,
    targets: Query<(Entity, &GlobalTransform, &Health), Without<Drop>>,
    mut damage: MessageWriter<Damage>,
) {
    let dt = time.delta_secs();
    for (entity, mut transform, mut drop) in &mut drops {
        drop.velocity.y -= 25.0 * dt;
        transform.translation += drop.velocity * dt;
        let ground = terrain.height_at(transform.translation.x, transform.translation.z);
        if transform.translation.y > ground || drop.velocity.y > 0.0 {
            continue;
        }
        for (target, t, health) in &targets {
            if !health.is_dead() && t.translation().distance(transform.translation) < DROP_RADIUS {
                damage.write(Damage {
                    target,
                    amount: DROP_DAMAGE,
                    source_team: Team::Neutral,
                });
            }
        }
        commands.entity(entity).despawn();
    }
}

/// Над глубокими кратерами поднимается дым и изредка летят искры.
fn smoke_hot_spots(mut commands: Commands, time: Res<Time>, mut spots: ResMut<HotSpots>, assets: Res<LavaAssets>) {
    spots.timer -= time.delta_secs();
    if spots.timer > 0.0 || spots.spots.is_empty() {
        return;
    }
    spots.timer = 0.4;
    let t = time.elapsed_secs();
    for (i, spot) in spots.spots.iter().enumerate() {
        let a = t * 3.7 + i as f32 * 1.3;
        commands.spawn((
            Smoke {
                velocity: Vec3::new(a.cos() * 0.4 + 0.6, 2.5, a.sin() * 0.4),
                age: 0.0,
                life: 2.5,
            },
            RunScoped,
            NotShadowCaster,
            Mesh3d(assets.ball.clone()),
            MeshMaterial3d(assets.smoke.clone()),
            Transform::from_translation(*spot + Vec3::Y * 0.5).with_scale(Vec3::splat(0.6)),
        ));
        if (a * 10.0).sin() > 0.6 {
            commands.spawn((
                Smoke {
                    velocity: Vec3::new(a.sin() * 2.0, 7.0, a.cos() * 2.0),
                    age: 0.0,
                    life: 0.8,
                },
                RunScoped,
                NotShadowCaster,
                Mesh3d(assets.ball.clone()),
                MeshMaterial3d(assets.spark.clone()),
                Transform::from_translation(*spot + Vec3::Y * 0.3).with_scale(Vec3::splat(0.08)),
            ));
        }
    }
}

fn fade_smoke(mut commands: Commands, time: Res<Time>, mut smoke: Query<(Entity, &mut Transform, &mut Smoke)>) {
    let dt = time.delta_secs();
    for (entity, mut t, mut s) in &mut smoke {
        s.age += dt;
        if s.age > s.life {
            commands.entity(entity).despawn();
            continue;
        }
        t.translation += s.velocity * dt;
        s.velocity *= 1.0 - 0.5 * dt;
        if s.life > 1.0 {
            t.scale *= 1.0 + 0.7 * dt;
        }
    }
}

/// Лежит ли точка в лаве: для объезда врагами и выбора укрытий.
pub fn is_lava(pools: &LavaPools, p: Vec3) -> bool {
    pools.lava_at(p.x, p.z).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn после_семи_выстрелов_мортиры_глубина_больше_коры() {
        let mut terrain = Terrain::new(7);
        let p = Vec3::new(40.0, 0.0, 40.0);
        // Как в craters.rs: радиус 7, глубина 0,5·R·мощность 1,4, вал 0,12·R·1,4.
        for _ in 0..6 {
            terrain.apply_crater(p, 7.0, 4.9, 1.176);
        }
        assert!(depth(&terrain, 40, 40) < CRUST, "шесть выстрелов кору не пробивают");
        terrain.apply_crater(p, 7.0, 4.9, 1.176);
        assert!(depth(&terrain, 40, 40) > CRUST);
    }

    #[test]
    fn заливка_не_выходит_за_предел() {
        let cells = flood(IVec2::ZERO, 10.0, 5, |_, _| 0.0);
        assert!(cells.iter().all(|c| c.length_squared() <= 25));
        assert!(cells.len() > 50);
    }

    #[test]
    fn заливка_не_попадает_в_несвязную_низину() {
        // Низина справа отделена стеной выше уровня.
        let cells = flood(IVec2::ZERO, 5.0, 30, |x, _| if x == 4 { 10.0 } else { 0.0 });
        assert!(cells.iter().all(|c| c.x < 4));
    }

    #[test]
    fn взрыв_поднимает_уровень() {
        let mut app = App::new();
        app.add_message::<Explosion>()
            .insert_resource(LavaPools {
                pools: vec![Pool {
                    origin: IVec2::ZERO,
                    level: 1.0,
                    start_level: 1.0,
                    target_level: 1.0,
                    age: 10.0,
                    refill: 0.0,
                    cells: HashSet::from([IVec2::ZERO]),
                    mesh: None,
                    dirty: false,
                }],
            })
            .insert_resource(LavaAssets {
                ball: Handle::default(),
                drop: Handle::default(),
                smoke: Handle::default(),
                spark: Handle::default(),
                flow: Handle::default(),
                crust: Handle::default(),
            })
            .add_systems(Update, blast_in_lava);
        app.world_mut().write_message(Explosion {
            position: Vec3::ZERO,
            radius: 5.0,
            power: 1.0,
            source_team: Team::Player,
        });
        app.update();
        assert!(app.world().resource::<LavaPools>().pools[0].target_level > 1.0);
    }
}
