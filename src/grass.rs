//! Трава: пучки травинок на пологих равнинах, собранные в поляны.
//!
//! Пучки участка ставятся по сетке ячеек со сдвигом, где маска полян из шума низкой частоты
//! гуще порога, склон пологий, нет гор, путей, домов и объектов. Цвет берётся с рельефа под
//! пучком, поэтому оттенок травы совпадает с травой на грунте. Один меш на участок, без теней.
//!
//! Состояние пучка хранится до конца забега: машины его приминают по ходу, взрыв и лава выжигают,
//! кратер убирает. Изменённые участки пересобираются не больше [`REBUILDS_PER_FRAME`] за кадр.

use std::collections::{HashMap, HashSet};

use bevy::asset::RenderAssetUsages;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use noise::{NoiseFn, Perlin};

use crate::craters;
use crate::explosion::Explosion;
use crate::frame::GameSet;
use crate::lava::LavaPools;
use crate::obstacles::NearObstacles;
use crate::quality::Quality;
use crate::railway::TRACK_FLAT;
use crate::rover::{ROVER_SCALE, Rover, RoverMotion};
use crate::run::{RunScoped, WorldRebuilt};
use crate::subsidence::TerrainChanged;
use crate::terrain::{self, CHUNK_SIZE, Terrain, VisibleChunks, chunk_coord_at, chunk_origin};
use crate::vehicle::{self, GroundVehicle};

/// Ячеек сетки пучков на сторону участка: ячейка 1,28 м.
const CELLS: usize = 25;
const CELL: f32 = CHUNK_SIZE as f32 / CELLS as f32;
/// Пучок не круче этого: компонента нормали по вертикали.
const MIN_UP: f32 = 0.85;
/// Выше этого — горы, травы нет, м.
const MAX_HEIGHT: f32 = 12.0;
/// Частота маски полян, 1/м, и сдвиг сида шума.
const MASK_FREQUENCY: f64 = 0.035;
const MASK_SEED_OFFSET: u32 = 0x6A5_7155;
/// Маска гуще этого — поляна; до полной густоты набирается на [`MASK_RAMP`].
const MASK_THRESHOLD: f32 = -0.05;
const MASK_RAMP: f32 = 0.35;
/// Травинок в пучке и их высота: от и до, м.
const BLADES: usize = 5;
const BLADE_HEIGHT: (f32, f32) = (0.3, 0.6);
/// Полуширина основания травинки, м.
const BLADE_HALF_WIDTH: f32 = 0.035;
/// Пучок не ближе этого к дому или объекту, м.
const OBSTACLE_CLEAR: f32 = 0.4;
/// Через сколько кадров после появления участка ставится трава: дома и объекты уже в сетке.
const SETTLE_FRAMES: u8 = 8;
/// Участков, пересобираемых за кадр.
const REBUILDS_PER_FRAME: usize = 2;
/// Выгорание от взрыва — в радиусе взрыва, умноженном на это.
const BURN_REACH: f32 = 1.8;
/// Рельеф под пучком опустился глубже — пучок исчез в кратере, м.
const GONE_DROP: f32 = 0.3;
/// Машина приминает траву, когда едет быстрее, м/с.
const CRUSH_SPEED: f32 = 0.5;
/// Как часто трава у лавы выгорает, с.
const LAVA_CHECK: f32 = 1.0;
/// Примятый пучок темнее на столько.
const CRUSHED_SHADE: f32 = 0.65;
/// Цвет выжженной травы, линейный.
const BURNT_COLOR: [f32; 4] = [0.03, 0.025, 0.02, 1.0];

/// Подключает траву.
pub struct GrassPlugin;

impl Plugin for GrassPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Grass>()
            .add_systems(Startup, create_material)
            .add_systems(
                Update,
                (
                    reset_grass.in_set(GameSet::Reset),
                    crush_grass.after(vehicle::move_ground_vehicles).in_set(GameSet::Simulation),
                    (scorch_grass, burn_by_lava).after(craters::dig_craters).in_set(GameSet::Damage),
                    stream_grass.after(terrain::stream_chunks).in_set(GameSet::Streaming),
                ),
            );
    }
}

/// Что стало с пучком.
#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Fresh,
    /// Примят машиной, курс которой — угол в плане.
    Crushed(f32),
    Burnt,
    Gone,
}

/// Пучок травы.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Tuft {
    position: Vec2,
    /// Высота основания, м: исходная, после кратера — новая.
    y: f32,
    /// Высота травинок, м.
    height: f32,
    /// Поворот веера травинок, рад.
    yaw: f32,
    /// Цвет основания и кончиков, линейный.
    base: [f32; 4],
    tip: [f32; 4],
    state: State,
}

/// Трава одного участка.
struct Patch {
    /// Пучки по ячейкам `CELLS × CELLS`.
    cells: Vec<Option<Tuft>>,
    entity: Option<Entity>,
    dirty: bool,
}

/// Трава забега по участкам.
#[derive(Resource, Default)]
struct Grass {
    patches: HashMap<IVec2, Patch>,
    /// Участки в сцене, которые ждут, пока на них встанут дома и объекты: сколько кадров ещё.
    settling: HashMap<IVec2, u8>,
    lava_timer: f32,
}

impl Grass {
    /// Пучки в квадрате со стороной `2·reach` вокруг `center`: участок и номер ячейки.
    fn around(&self, center: Vec2, reach: f32) -> Vec<(IVec2, usize)> {
        let mut found = Vec::new();
        let (lo, hi) = (center - Vec2::splat(reach), center + Vec2::splat(reach));
        let (c0, c1) = (chunk_coord_at(lo.x, lo.y), chunk_coord_at(hi.x, hi.y));
        for cz in c0.y..=c1.y {
            for cx in c0.x..=c1.x {
                let coord = IVec2::new(cx, cz);
                if !self.patches.contains_key(&coord) {
                    continue;
                }
                let origin = chunk_origin(coord);
                let cell = |v: f32, o: f32| (((v - o) / CELL).floor() as i32).clamp(0, CELLS as i32 - 1) as usize;
                for iz in cell(lo.y, origin.y)..=cell(hi.y, origin.y) {
                    for ix in cell(lo.x, origin.x)..=cell(hi.x, origin.x) {
                        found.push((coord, iz * CELLS + ix));
                    }
                }
            }
        }
        found
    }

    /// Меняет состояние пучков в `cells`, если `change` вернула новое; участок помечается к пересборке.
    fn update(&mut self, cells: &[(IVec2, usize)], mut change: impl FnMut(&Tuft) -> Option<Tuft>) {
        for &(coord, index) in cells {
            let Some(patch) = self.patches.get_mut(&coord) else {
                continue;
            };
            let Some(tuft) = patch.cells[index].as_mut() else {
                continue;
            };
            if let Some(new) = change(tuft)
                && new != *tuft
            {
                *tuft = new;
                patch.dirty = true;
            }
        }
    }
}

#[derive(Resource)]
struct GrassMaterial(Handle<StandardMaterial>);

fn create_material(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    commands.insert_resource(GrassMaterial(materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.9,
        cull_mode: None,
        ..default()
    })));
}

/// Новый мир: трава прошлого удалена через `RunScoped`, состояния забыты.
fn reset_grass(mut rebuilt: MessageReader<WorldRebuilt>, mut grass: ResMut<Grass>) {
    if rebuilt.read().count() > 0 {
        *grass = Grass::default();
    }
}

/// Хеш сида, точки сетки и номера: число от 0 до 1.
fn hash(seed: u32, x: i32, z: i32, salt: u32) -> f32 {
    let mut h = seed ^ (x as u32).wrapping_mul(0x9E37_79B1) ^ (z as u32).wrapping_mul(0x85EB_CA77);
    h ^= salt.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^= h >> 16;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// Растёт ли трава в месте с вертикальной нормалью `up`, высотой `height` и расстоянием
/// `track_distance` до оси путей.
fn tuft_allowed(up: f32, height: f32, track_distance: f32) -> bool {
    up > MIN_UP && height < MAX_HEIGHT && track_distance > TRACK_FLAT + 0.5
}

/// Густота поляны по маске от -1 до 1: ноль до порога, дальше плавно до единицы.
fn meadow_density(mask: f32) -> f32 {
    ((mask - MASK_THRESHOLD) / MASK_RAMP).clamp(0.0, 1.0)
}

/// Пучки участка по сиду и нетронутому рельефу, без учёта домов и объектов.
fn generate(terrain: &Terrain, mask: &Perlin, coord: IVec2) -> Vec<Option<Tuft>> {
    let seed = terrain.seed();
    let origin = chunk_origin(coord);
    let mut cells = vec![None; CELLS * CELLS];
    for iz in 0..CELLS {
        for ix in 0..CELLS {
            let (gx, gz) = (coord.x * CELLS as i32 + ix as i32, coord.y * CELLS as i32 + iz as i32);
            let p = origin
                + Vec2::new(
                    (ix as f32 + 0.15 + 0.7 * hash(seed, gx, gz, 1)) * CELL,
                    (iz as f32 + 0.15 + 0.7 * hash(seed, gx, gz, 2)) * CELL,
                );
            let density = meadow_density(mask.get([p.x as f64 * MASK_FREQUENCY, p.y as f64 * MASK_FREQUENCY]) as f32);
            if hash(seed, gx, gz, 3) >= density {
                continue;
            }
            let y = terrain.height_at(p.x, p.y);
            let track = terrain.lines().nearest(p).distance;
            if !tuft_allowed(terrain.normal_at(p.x, p.y).y, y, track) {
                continue;
            }
            let ground = terrain.vertex_color(p.x.round() as i32, p.y.round() as i32).unwrap_or([0.12, 0.3, 0.08, 1.0]);
            let shade = |k: f32| [ground[0] * k, ground[1] * k, ground[2] * k, 1.0];
            let tip = 1.15 + 0.3 * hash(seed, gx, gz, 4);
            cells[iz * CELLS + ix] = Some(Tuft {
                position: p,
                y,
                height: BLADE_HEIGHT.0 + (BLADE_HEIGHT.1 - BLADE_HEIGHT.0) * hash(seed, gx, gz, 5) * (0.5 + 0.5 * density),
                yaw: hash(seed, gx, gz, 6) * std::f32::consts::TAU,
                base: shade(0.7),
                tip: shade(tip),
                state: State::Fresh,
            });
        }
    }
    cells
}

/// Маска полян забега.
fn mask_for(terrain: &Terrain) -> Perlin {
    Perlin::new(terrain.seed().wrapping_add(MASK_SEED_OFFSET))
}

/// Ставит траву на участки в сцене и убирает с выгруженных, пересобирает изменённые меши.
#[allow(clippy::too_many_arguments)]
fn stream_grass(
    mut commands: Commands,
    mut grass: ResMut<Grass>,
    terrain: Res<Terrain>,
    visible: Res<VisibleChunks>,
    obstacles: NearObstacles,
    quality: Res<Quality>,
    material: Res<GrassMaterial>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut known: Local<HashSet<IVec2>>,
) {
    let grass = &mut *grass;
    if quality.is_changed() {
        for patch in grass.patches.values_mut() {
            patch.dirty = true;
        }
    }
    // Выгруженные участки: меш удаляется, состояние остаётся.
    for (coord, patch) in grass.patches.iter_mut() {
        if visible.entity(*coord).is_none()
            && let Some(entity) = patch.entity.take()
        {
            commands.entity(entity).try_despawn();
        }
    }
    known.retain(|coord| visible.entity(*coord).is_some());
    grass.settling.retain(|coord, _| visible.entity(*coord).is_some());
    if visible.is_changed() {
        for patch_coord in visible_coords(&visible) {
            if known.insert(patch_coord) {
                if let Some(patch) = grass.patches.get_mut(&patch_coord) {
                    patch.dirty = true;
                } else {
                    grass.settling.insert(patch_coord, SETTLE_FRAMES);
                }
            }
        }
    }
    // Новые участки: пучки по сиду минус дома и объекты.
    let mut ready = Vec::new();
    for (coord, frames) in grass.settling.iter_mut() {
        *frames = frames.saturating_sub(1);
        if *frames == 0 {
            ready.push(*coord);
        }
    }
    if !ready.is_empty() {
        let mask = mask_for(&terrain);
        for coord in ready {
            grass.settling.remove(&coord);
            let mut cells = generate(&terrain, &mask, coord);
            for cell in cells.iter_mut() {
                let blocked = cell.is_some_and(|tuft| {
                    obstacles
                        .near(tuft.position, OBSTACLE_CLEAR + 1.0)
                        .any(|(_, o)| o.shape.push_out(tuft.position, OBSTACLE_CLEAR).is_some())
                });
                if blocked {
                    *cell = None;
                }
            }
            grass.patches.insert(coord, Patch { cells, entity: None, dirty: true });
        }
    }
    // Пересборка: не больше двух участков за кадр.
    let step = if *quality == Quality::Low { 2 } else { 1 };
    let mut budget = REBUILDS_PER_FRAME;
    for (coord, patch) in grass.patches.iter_mut() {
        if budget == 0 {
            break;
        }
        if !patch.dirty || visible.entity(*coord).is_none() {
            continue;
        }
        budget -= 1;
        patch.dirty = false;
        let mesh = build_mesh(&patch.cells, step);
        match (patch.entity, mesh) {
            (Some(entity), Some(mesh)) => {
                commands.entity(entity).try_insert(Mesh3d(meshes.add(mesh)));
            }
            (None, Some(mesh)) => {
                patch.entity = Some(
                    commands
                        .spawn((
                            RunScoped,
                            Mesh3d(meshes.add(mesh)),
                            MeshMaterial3d(material.0.clone()),
                            NotShadowCaster,
                            Transform::IDENTITY,
                        ))
                        .id(),
                );
            }
            (Some(entity), None) => {
                commands.entity(entity).try_despawn();
                patch.entity = None;
            }
            (None, None) => {}
        }
    }
}

/// Координаты участков в сцене.
fn visible_coords(visible: &VisibleChunks) -> Vec<IVec2> {
    visible.coords().collect()
}

/// Меш травы участка: травинки — треугольники с вершинными цветами. `step` — каждый какой
/// пучок рисовать. `None`, если пучков нет.
fn build_mesh(cells: &[Option<Tuft>], step: usize) -> Option<Mesh> {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    for tuft in cells.iter().flatten().step_by(step) {
        let base = Vec3::new(tuft.position.x, tuft.y - 0.02, tuft.position.y);
        let (height, lean, color_tip, color_base, fan) = match tuft.state {
            State::Gone => continue,
            State::Fresh => (tuft.height, 0.45, tuft.tip, tuft.base, None),
            State::Crushed(heading) => {
                let shade = |c: [f32; 4]| [c[0] * CRUSHED_SHADE, c[1] * CRUSHED_SHADE, c[2] * CRUSHED_SHADE, 1.0];
                (0.07, 0.0, shade(tuft.tip), shade(tuft.base), Some(heading))
            }
            State::Burnt => (tuft.height * 0.3, 0.3, BURNT_COLOR, BURNT_COLOR, None),
        };
        for k in 0..BLADES {
            let spread = k as f32 / BLADES as f32;
            let (out, tip) = match fan {
                // Лежит вдоль хода машины, веером ±0,4 рад.
                Some(heading) => {
                    let dir = vehicle::forward_of(heading + (spread - 0.5) * 0.8);
                    (dir, base + dir * tuft.height * 0.9 + Vec3::Y * height)
                }
                None => {
                    let dir = vehicle::forward_of(tuft.yaw + spread * std::f32::consts::TAU);
                    (dir, base + dir * height * lean + Vec3::Y * height)
                }
            };
            let side = Vec3::new(-out.z, 0.0, out.x) * BLADE_HALF_WIDTH;
            positions.extend([(base - side).to_array(), (base + side).to_array(), tip.to_array()]);
            colors.extend([color_base, color_base, color_tip]);
        }
    }
    if positions.is_empty() {
        return None;
    }
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    let indices = (0..positions.len() as u32).collect();
    Some(
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
            .with_inserted_indices(Indices::U32(indices)),
    )
}

/// Лежит ли точка `p` в прямоугольнике машины: центр `center`, курс `heading`, половины `half`.
fn under_vehicle(p: Vec2, center: Vec2, heading: f32, half: Vec2) -> bool {
    let forward = vehicle::forward_of(heading);
    let forward = Vec2::new(forward.x, forward.z);
    let side = Vec2::new(-forward.y, forward.x);
    let d = p - center;
    d.dot(side).abs() <= half.x && d.dot(forward).abs() <= half.y
}

/// Машины приминают траву под собой по направлению хода.
fn crush_grass(
    mut grass: ResMut<Grass>,
    vehicles: Query<(&Transform, &GroundVehicle)>,
    rover: Query<(&Transform, &RoverMotion), With<Rover>>,
) {
    let rover_half = Vec2::new(0.75, 1.0) * ROVER_SCALE;
    let all = vehicles
        .iter()
        .map(|(t, v)| (t.translation, v.heading, v.speed, v.half_base))
        .chain(rover.iter().map(|(t, m)| (t.translation, m.heading, m.speed, rover_half)));
    for (position, heading, speed, half) in all {
        if speed.abs() < CRUSH_SPEED {
            continue;
        }
        let center = Vec2::new(position.x, position.z);
        let cells = grass.around(center, half.length());
        grass.update(&cells, |tuft| {
            (tuft.state == State::Fresh && under_vehicle(tuft.position, center, heading, half))
                .then_some(Tuft { state: State::Crushed(heading), ..*tuft })
        });
    }
}

/// Взрыв выжигает траву вокруг, кратер её убирает, а на изменённом рельефе пучки переставляются.
fn scorch_grass(
    mut grass: ResMut<Grass>,
    mut explosions: MessageReader<Explosion>,
    mut changes: MessageReader<TerrainChanged>,
    terrain: Res<Terrain>,
) {
    for explosion in explosions.read() {
        let center = Vec2::new(explosion.position.x, explosion.position.z);
        let reach = explosion.radius * BURN_REACH;
        let cells = grass.around(center, reach);
        grass.update(&cells, |tuft| {
            (tuft.state != State::Gone && tuft.position.distance(center) <= reach)
                .then_some(Tuft { state: State::Burnt, ..*tuft })
        });
    }
    for change in changes.read() {
        let center = Vec2::new(change.center.x, change.center.z);
        let cells = grass.around(center, change.radius);
        grass.update(&cells, |tuft| {
            if tuft.state == State::Gone || tuft.position.distance(center) > change.radius {
                return None;
            }
            let y = terrain.height_at(tuft.position.x, tuft.position.y);
            Some(if y < tuft.y - GONE_DROP { Tuft { state: State::Gone, ..*tuft } } else { Tuft { y, ..*tuft } })
        });
    }
}

/// Трава на лаве и у её края выгорает: раз в [`LAVA_CHECK`].
fn burn_by_lava(time: Res<Time>, mut grass: ResMut<Grass>, pools: Res<LavaPools>) {
    grass.lava_timer -= time.delta_secs();
    if grass.lava_timer > 0.0 {
        return;
    }
    grass.lava_timer = LAVA_CHECK;
    for patch in grass.patches.values_mut() {
        for tuft in patch.cells.iter_mut().flatten() {
            if matches!(tuft.state, State::Burnt | State::Gone) {
                continue;
            }
            let p = tuft.position;
            let near = [Vec2::ZERO, Vec2::X, Vec2::NEG_X, Vec2::Y, Vec2::NEG_Y]
                .iter()
                .any(|o| pools.lava_at(p.x + o.x, p.y + o.y).is_some());
            if near {
                tuft.state = State::Burnt;
                patch.dirty = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn тот_же_сид_даёт_те_же_пучки() {
        let a = Terrain::new(11);
        let b = Terrain::new(11);
        for coord in [IVec2::new(2, -3), IVec2::new(-5, 4), IVec2::new(7, 7)] {
            assert_eq!(generate(&a, &mask_for(&a), coord), generate(&b, &mask_for(&b), coord));
        }
        // Где-то трава есть: маска не пустая.
        let total: usize = (-4..4)
            .flat_map(|x| (-4..4).map(move |z| IVec2::new(x, z)))
            .map(|c| generate(&a, &mask_for(&a), c).iter().flatten().count())
            .sum();
        assert!(total > 1000, "{total}");
    }

    #[test]
    fn на_склоне_круче_порога_пучков_нет() {
        assert!(tuft_allowed(0.95, 2.0, 30.0));
        assert!(!tuft_allowed(0.8, 2.0, 30.0));
        // Горы и пути.
        assert!(!tuft_allowed(0.99, MAX_HEIGHT + 1.0, 30.0));
        assert!(!tuft_allowed(0.99, 2.0, TRACK_FLAT));
    }

    #[test]
    fn машина_приминает_только_под_собой() {
        let half = Vec2::new(1.0, 2.0);
        // Курс 0 — нос на -Z.
        assert!(under_vehicle(Vec2::new(0.5, -1.5), Vec2::ZERO, 0.0, half));
        assert!(!under_vehicle(Vec2::new(1.5, 0.0), Vec2::ZERO, 0.0, half));
        assert!(!under_vehicle(Vec2::new(0.0, 2.5), Vec2::ZERO, 0.0, half));
    }

    #[test]
    fn меш_без_исчезнувших_пучков() {
        let tuft = |state| {
            Some(Tuft {
                position: Vec2::ZERO,
                y: 0.0,
                height: 0.4,
                yaw: 0.0,
                base: [0.1; 4],
                tip: [0.2; 4],
                state,
            })
        };
        assert!(build_mesh(&[tuft(State::Gone), None], 1).is_none());
        let mesh = build_mesh(&[tuft(State::Fresh), tuft(State::Crushed(1.0)), tuft(State::Burnt)], 1).unwrap();
        assert_eq!(mesh.count_vertices(), 3 * BLADES * 3);
    }
}
