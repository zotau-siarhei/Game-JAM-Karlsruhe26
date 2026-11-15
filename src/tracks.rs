//! Следы на грунте: колеи колёсных машин, рифлёные полосы гусениц, отпечатки ступней «Колосса»
//! и пыль из-под колёс врагов.
//!
//! Машина с [`TrackMaker`] через каждые [`SEGMENT_STEP`] метров пути кладёт под левой и правой
//! опорой отрезок, соединённый с предыдущим той же опоры: дуги поворота не рвутся. Отрезки лежат
//! в кольцевом буфере [`Tracks`] и рисуются одним мешем-лентой поверх рельефа: углы ленты стоят
//! на тех же треугольниках, что и меш участка, и подняты на [`LIFT`]. Отрезки бледнеют за
//! последние [`FADE`] секунд из [`LIFETIME`], пропадают там, где рельеф изменился
//! (`TerrainChanged`), и не ложатся на лаву, рельсы и пол вскрытого туннеля.
//!
//! «Колосс» шагает: через каждые [`STRIDE`] метров одна из четырёх ступней вдавливает рельеф
//! на [`FOOT_DEPTH`] и оставляет тёмный отпечаток до конца забега.
//!
//! Луж пока нет (`puddles` не сделан), поэтому мокрого следа и пропуска следов в воде нет.

use std::collections::VecDeque;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_hanabi::prelude::*;

use crate::boss::Boss;
use crate::frame::GameSet;
use crate::lava::{self, LavaPools};
use crate::quality::Quality;
use crate::railway::TRACK_FLAT;
use crate::rover::Rover;
use crate::run::WorldRebuilt;
use crate::subsidence::TerrainChanged;
use crate::tank_look::TankGun;
use crate::terrain::{Terrain, VisibleChunks};
use crate::vehicle::GroundVehicle;

/// Через сколько метров пути опоры кладётся новый отрезок, м.
const SEGMENT_STEP: f32 = 0.4;

/// Больше такого сдвига за кадр — перенос машины (рестарт), а не езда: колея начинается заново, м.
const MAX_JUMP: f32 = 3.0;

/// Сколько живёт след колёс и гусениц, с.
const LIFETIME: f32 = 40.0;

/// За сколько последних секунд жизни след бледнеет до нуля, с.
const FADE: f32 = 10.0;

/// Предел отрезков колёс и гусениц: при превышении первыми уходят самые старые.
const MAX_SEGMENTS: usize = 4000;

/// Предел отпечатков ступней: они не бледнеют, но и бесконечно копиться не должны.
const MAX_FOOTPRINTS: usize = 600;

/// Подъём ленты над рельефом, м: против мерцания вместе со сдвигом глубины материала.
const LIFT: f32 = 0.04;

/// Сдвиг глубины материала следов: лента рисуется поверх рельефа.
const DEPTH_BIAS: f32 = 8.0;

/// Пересборка меша не чаще этого, с.
const REBUILD_PERIOD: f32 = 0.1;

/// Непрозрачность свежего следа: гусеницы, колёса, ступни.
const TREADS_ALPHA: f32 = 0.55;
const WHEELS_ALPHA: f32 = 0.6;
const FEET_ALPHA: f32 = 0.7;

/// Цвет следа в линейном RGB: тёмный влажный грунт.
const TRACK_COLOR: [f32; 3] = [0.07, 0.05, 0.03];

/// Длина одного повтора рисунка гусеницы вдоль следа, м: два грунтозацепа.
const TREAD_REPEAT: f32 = 0.85;

/// Длина повтора текстуры колеи колёс, м: рисунка вдоль нет, число только против дрожания UV.
const WHEEL_REPEAT: f32 = 4.0;

/// До какого расстояния от оси путей отрезок не ложится на насыпь с рельсами, м.
const RAIL_SKIP: f32 = 2.6;

/// Шаг «Колосса»: через столько метров пути опускается следующая ступня, м.
const STRIDE: f32 = 2.4;

/// Ступня: радиус вмятины и её глубина, м.
const FOOT_RADIUS: f32 = 1.3;
const FOOT_DEPTH: f32 = 0.3;

/// Отпечаток ступни: ширина и длина, м.
const FOOT_SIZE: Vec2 = Vec2::new(2.0, 3.0);

/// Ступни относительно корпуса в долях полуопоры `GroundVehicle::half_base`: как ноги на модели.
const FOOT_SPREAD: Vec2 = Vec2::new(1.1, 1.0);

/// Порядок ступней при ходьбе: левая передняя, правая задняя, правая передняя, левая задняя.
const FOOT_ORDER: [(f32, f32); 4] = [(-1.0, -1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, 1.0)];

/// Пыль из-под колёс: частиц в секунду на полной скорости, скорость, с которой пыль начинается,
/// и скорость, на которой её больше всего, м/с.
const DUST_RATE: f32 = 22.0;
const DUST_FROM_SPEED: f32 = 2.0;
const DUST_FULL_SPEED: f32 = 12.0;

/// Дальше этого от лунохода машины не пылят, м.
const DUST_RANGE: f32 = 60.0;

/// Цикл эмиттера пыли, с: как у пыли лунохода.
const DUST_PERIOD: f32 = 0.1;

/// Плагин следов.
pub struct TracksPlugin;

impl Plugin for TracksPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Tracks>()
            .add_systems(Startup, spawn_track_mesh)
            .add_systems(
                Update,
                (
                    reset_tracks.in_set(GameSet::Reset),
                    stomp.in_set(GameSet::Damage),
                    (attach_makers, attach_dust, lay_tracks, cut_and_rebuild, emit_dust)
                        .chain()
                        .in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Чем машина касается земли.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    /// Гусеницы: широкие полосы с поперечной рифлёностью.
    Treads,
    /// Колёса: узкие тёмные колеи.
    Wheels,
    /// Ступни шагающего босса: вмятины с отпечатком.
    Feet,
}

/// Машина оставляет следы.
#[derive(Component, Debug, Clone, Copy)]
pub struct TrackMaker {
    pub kind: TrackKind,
    /// Расстояние между центрами левой и правой опоры, м.
    pub gauge: f32,
    /// Ширина одной полосы, м.
    pub width: f32,
}

/// Где опоры машины касались земли в последний раз.
#[derive(Component, Default)]
struct TrackTrail {
    /// Последняя точка левой и правой опоры; `None` — колея прервана.
    last: [Option<Contact>; 2],
    /// Положение машины в прошлом кадре.
    position: Option<Vec3>,
    /// Путь с прошлого шага ступни, м.
    stride: f32,
    /// Какая ступня следующая.
    foot: usize,
}

/// Точка касания опоры: края полосы на рельефе.
#[derive(Debug, Clone, Copy)]
struct Contact {
    /// Середина полосы на плане.
    center: Vec2,
    left: Vec3,
    right: Vec3,
    normal: Vec3,
    /// Путь вдоль колеи с её начала, м: координата текстуры.
    distance: f32,
}

/// Отрезок колеи между двумя точками касания одной опоры.
#[derive(Debug, Clone, Copy)]
struct Segment {
    from: Contact,
    to: Contact,
    kind: TrackKind,
    /// Время появления по часам игры, с.
    born: f32,
}

/// Отпечаток ступни.
#[derive(Debug, Clone, Copy)]
struct Footprint {
    center: Vec2,
    /// Направление «вперёд» корпуса на плане.
    forward: Vec2,
}

/// Следы забега.
#[derive(Resource, Default)]
pub struct Tracks {
    segments: VecDeque<Segment>,
    footprints: VecDeque<Footprint>,
    /// Отпечатки этого кадра: в буфер они попадают после того, как свой же `TerrainChanged`
    /// срежет старые следы под ступнёй.
    pending_feet: Vec<Footprint>,
    /// Буфер менялся с последней пересборки меша.
    dirty: bool,
    /// Сколько прошло с последней пересборки, с.
    clock: f32,
}

impl Tracks {
    /// Добавляет отрезок; сверх предела уходит самый старый.
    fn push(&mut self, segment: Segment) {
        self.segments.push_back(segment);
        while self.segments.len() > MAX_SEGMENTS {
            self.segments.pop_front();
        }
        self.dirty = true;
    }

    /// Добавляет отпечаток ступни; сверх предела уходит самый старый.
    fn push_foot(&mut self, foot: Footprint) {
        self.footprints.push_back(foot);
        while self.footprints.len() > MAX_FOOTPRINTS {
            self.footprints.pop_front();
        }
        self.dirty = true;
    }

    /// Убирает отрезки старше [`LIFETIME`]. Отрезки лежат по времени появления.
    fn expire(&mut self, now: f32) {
        while self.segments.front().is_some_and(|s| now - s.born > LIFETIME) {
            self.segments.pop_front();
            self.dirty = true;
        }
    }

    /// Убирает отрезки, у которых хоть один конец в круге на плане, и отпечатки с центром в нём.
    fn cut(&mut self, center: Vec2, radius: f32) {
        let r2 = radius * radius;
        let inside = |p: Vec2| p.distance_squared(center) < r2;
        let before = self.segments.len() + self.footprints.len();
        self.segments.retain(|s| !inside(s.from.center) && !inside(s.to.center));
        self.footprints.retain(|f| !inside(f.center));
        if self.segments.len() + self.footprints.len() != before {
            self.dirty = true;
        }
    }

    /// Есть ли бледнеющие отрезки: тогда меш меняется и без новых отрезков.
    fn fading(&self, now: f32) -> bool {
        self.segments.front().is_some_and(|s| now - s.born > LIFETIME - FADE)
    }
}

/// Сущность меша всех следов.
#[derive(Component)]
struct TrackMesh;

/// Эмиттер пыли у задней оси машины врагов или Службы.
#[derive(Component)]
struct WheelDust;

/// Общий эффект пыли из-под колёс.
#[derive(Resource)]
struct DustEffect(Handle<EffectAsset>);

/// Создаёт сущность меша следов, его материал с текстурой и эффект пыли.
fn spawn_track_mesh(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut effects: ResMut<Assets<EffectAsset>>,
) {
    let texture = images.add(track_image());
    let material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(texture),
        alpha_mode: bevy::prelude::AlphaMode::Blend,
        perceptual_roughness: 1.0,
        depth_bias: DEPTH_BIAS,
        ..default()
    });
    commands.spawn((
        TrackMesh,
        Mesh3d(meshes.add(empty_mesh())),
        MeshMaterial3d(material),
        Transform::IDENTITY,
        NotShadowCaster,
        // Границы меша меняются вместе с буфером.
        NoFrustumCulling,
    ));
    commands.insert_resource(DustEffect(effects.add(crate::rover_look::dust_effect())));
}

/// Текстура следов: три столбца по 16 пикселей — гусеница с поперечными полосами, колея
/// без рисунка и эллипс ступни. Рисунок в альфе, цвет даёт цвет вершин.
fn track_image() -> Image {
    const W: usize = 16;
    const H: usize = 16;
    let mut data = Vec::with_capacity(3 * W * H * 4);
    for y in 0..H {
        for column in 0..3 {
            for x in 0..W {
                let t = (x as f32 + 0.5) / W as f32;
                let edge = 1.0 - (2.0 * t - 1.0).abs().powi(4);
                let a = match column {
                    // Грунтозацепы: две тёмные полосы на повтор.
                    0 => edge * if y % 8 < 3 { 1.0 } else { 0.5 },
                    1 => edge,
                    _ => {
                        let dx = 2.0 * t - 1.0;
                        let dy = 2.0 * (y as f32 + 0.5) / H as f32 - 1.0;
                        let r = (dx * dx + dy * dy).sqrt();
                        ((1.0 - r) * 4.0).clamp(0.0, 1.0)
                    }
                };
                data.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
            }
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: (3 * W) as u32,
            height: H as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });
    image
}

/// Пустой меш: один вырожденный треугольник.
fn empty_mesh() -> Mesh {
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 3])
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; 3])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]; 3])
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; 3])
        .with_inserted_indices(Indices::U32(vec![0, 1, 2]))
}

/// Новый забег или меню: следов нет.
fn reset_tracks(mut rebuilt: MessageReader<WorldRebuilt>, mut tracks: ResMut<Tracks>) {
    if rebuilt.read().last().is_some() {
        *tracks = Tracks {
            dirty: true,
            ..default()
        };
    }
}

/// Вешает [`TrackMaker`] на луноход, машины врагов и Службы и на босса.
fn attach_makers(
    mut commands: Commands,
    rovers: Query<Entity, Added<Rover>>,
    vehicles: Query<(Entity, &GroundVehicle, Has<Boss>, Has<TankGun>), Added<GroundVehicle>>,
) {
    for rover in &rovers {
        let scale = crate::rover::ROVER_SCALE;
        commands.entity(rover).try_insert((
            TrackMaker {
                kind: TrackKind::Treads,
                gauge: 2.0 * crate::rover_look::TRACK_X * scale,
                width: 0.3 * scale,
            },
            TrackTrail::default(),
        ));
    }
    for (entity, vehicle, boss, tank) in &vehicles {
        let base = vehicle.half_base;
        let maker = if boss {
            TrackMaker {
                kind: TrackKind::Feet,
                gauge: 2.0 * base.x * FOOT_SPREAD.x,
                width: FOOT_SIZE.x,
            }
        } else if tank {
            TrackMaker {
                kind: TrackKind::Treads,
                gauge: 2.0 * base.x * 0.8,
                width: 0.45 * base.x,
            }
        } else {
            TrackMaker {
                kind: TrackKind::Wheels,
                gauge: 2.0 * base.x * 0.85,
                width: 0.3,
            }
        };
        commands.entity(entity).try_insert((maker, TrackTrail::default()));
    }
}

/// Высота поверхности меша рельефа в точке: на том же треугольнике клетки, что у меша
/// участка (`terrain::mesh`), а не билинейная, иначе лента местами уходит под землю.
fn mesh_height(terrain: &Terrain, x: f32, z: f32) -> f32 {
    let (fx, fz) = (x.floor(), z.floor());
    let (tx, tz) = (x - fx, z - fz);
    let (vx, vz) = (fx as i32, fz as i32);
    let ha = terrain.vertex_height(vx, vz);
    let hb = terrain.vertex_height(vx + 1, vz);
    let hc = terrain.vertex_height(vx, vz + 1);
    let hd = terrain.vertex_height(vx + 1, vz + 1);
    if (hb - hc).abs() <= (ha - hd).abs() {
        // Диагональ b–c: треугольники a-c-b и b-c-d.
        if tx + tz <= 1.0 {
            ha + (hb - ha) * tx + (hc - ha) * tz
        } else {
            hd + (hc - hd) * (1.0 - tx) + (hb - hd) * (1.0 - tz)
        }
    } else if tx >= tz {
        // Диагональ a–d: треугольники a-d-b (справа) и a-c-d.
        ha + (hb - ha) * tx + (hd - hb) * tz
    } else {
        ha + (hc - ha) * tz + (hd - hc) * tx
    }
}

/// Нельзя класть след: лава, насыпь с рельсами или пол вскрытого туннеля.
fn forbidden(terrain: &Terrain, pools: &LavaPools, p: Vec2) -> bool {
    if lava::is_lava(pools, Vec3::new(p.x, 0.0, p.y)) {
        return true;
    }
    let near = terrain.lines().nearest(p);
    if near.distance >= TRACK_FLAT {
        return false;
    }
    let level = terrain.track_level(near.line, near.s);
    let h = terrain.height_at(p.x, p.y);
    // Рельсы лежат на верху насыпи; рядом с ней колея идёт как обычно.
    if near.distance < RAIL_SKIP && (h - level).abs() < 1.0 {
        return true;
    }
    // Пол окна туннеля срезан до уровня путей во всю ширину ровной полосы.
    terrain.is_tunnel(near.line, near.s) && h < level + 1.0
}

/// Точка касания опоры с центром `center` и поперечным направлением `right`.
fn contact(terrain: &Terrain, center: Vec2, right: Vec2, width: f32, distance: f32) -> Contact {
    let edge = |s: f32| {
        let p = center + right * s * width * 0.5;
        Vec3::new(p.x, mesh_height(terrain, p.x, p.y) + LIFT, p.y)
    };
    Contact {
        center,
        left: edge(-1.0),
        right: edge(1.0),
        normal: terrain.normal_at(center.x, center.y),
        distance,
    }
}

/// Кладёт отрезки колей под опорами едущих машин.
fn lay_tracks(
    time: Res<Time>,
    terrain: Res<Terrain>,
    pools: Res<LavaPools>,
    mut tracks: ResMut<Tracks>,
    mut makers: Query<(&GlobalTransform, &TrackMaker, &mut TrackTrail)>,
) {
    let now = time.elapsed_secs();
    for (transform, maker, mut trail) in &mut makers {
        if maker.kind == TrackKind::Feet {
            continue;
        }
        let position = transform.translation();
        if trail.position.is_some_and(|p| p.xz().distance(position.xz()) > MAX_JUMP) {
            trail.last = [None, None];
        }
        trail.position = Some(position);
        let right = (transform.rotation() * Vec3::X).xz().normalize_or(Vec2::X);
        for (side, sign) in [(0, -1.0f32), (1, 1.0)] {
            let center = position.xz() + right * sign * maker.gauge * 0.5;
            let last = trail.last[side];
            let distance = match last {
                Some(last) => {
                    let step = last.center.distance(center);
                    if step < SEGMENT_STEP {
                        continue;
                    }
                    last.distance + step
                }
                None => 0.0,
            };
            if forbidden(&terrain, &pools, center) {
                trail.last[side] = None;
                continue;
            }
            let here = contact(&terrain, center, right, maker.width, distance);
            if let Some(from) = last {
                tracks.push(Segment {
                    from,
                    to: here,
                    kind: maker.kind,
                    born: now,
                });
            }
            trail.last[side] = Some(here);
        }
    }
}

/// Шаги «Колосса»: через каждые [`STRIDE`] метров ступня вдавливает рельеф.
fn stomp(
    mut commands: Commands,
    mut terrain: ResMut<Terrain>,
    pools: Res<LavaPools>,
    visible: Res<VisibleChunks>,
    mut tracks: ResMut<Tracks>,
    mut makers: Query<(&GlobalTransform, &TrackMaker, &GroundVehicle, &mut TrackTrail)>,
    mut changes: MessageWriter<TerrainChanged>,
) {
    for (transform, maker, vehicle, mut trail) in &mut makers {
        if maker.kind != TrackKind::Feet {
            continue;
        }
        let position = transform.translation();
        let step = trail.position.map_or(0.0, |p| p.xz().distance(position.xz()));
        trail.position = Some(position);
        if step > MAX_JUMP {
            continue;
        }
        trail.stride += step;
        if trail.stride < STRIDE {
            continue;
        }
        trail.stride = 0.0;
        let (sx, sz) = FOOT_ORDER[trail.foot % FOOT_ORDER.len()];
        trail.foot = trail.foot.wrapping_add(1);
        let yaw = Quat::from_rotation_y(vehicle.heading);
        let offset = Vec3::new(sx * maker.gauge * 0.5, 0.0, sz * vehicle.half_base.y * FOOT_SPREAD.y);
        let foot = (position + yaw * offset).xz();
        if forbidden(&terrain, &pools, foot) {
            continue;
        }
        let center = Vec3::new(foot.x, terrain.height_at(foot.x, foot.y), foot.y);
        let changed = terrain.apply_crater(center, FOOT_RADIUS, FOOT_DEPTH, 0.0);
        visible.mark_dirty(&mut commands, changed);
        changes.write(TerrainChanged {
            center,
            radius: FOOT_RADIUS,
        });
        tracks.pending_feet.push(Footprint {
            center: foot,
            forward: vehicle.forward().xz().normalize_or(Vec2::NEG_Y),
        });
    }
}

/// Срезает следы по изменениям рельефа, убирает старые и пересобирает меш.
fn cut_and_rebuild(
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut changes: MessageReader<TerrainChanged>,
    mut tracks: ResMut<Tracks>,
    mut meshes: ResMut<Assets<Mesh>>,
    mesh: Query<&Mesh3d, With<TrackMesh>>,
) {
    for change in changes.read() {
        tracks.cut(change.center.xz(), change.radius);
    }
    for foot in std::mem::take(&mut tracks.pending_feet) {
        tracks.push_foot(foot);
    }
    let now = time.elapsed_secs();
    tracks.expire(now);
    tracks.clock += time.delta_secs();
    if tracks.clock < REBUILD_PERIOD || !(tracks.dirty || tracks.fading(now)) {
        return;
    }
    tracks.clock = 0.0;
    tracks.dirty = false;
    let Some(handle) = mesh.iter().next() else {
        return;
    };
    if let Some(mut mesh) = meshes.get_mut(&handle.0) {
        *mesh = build_mesh(&tracks, &terrain, now);
    }
}

/// Столбец текстуры вида следа: пределы U с отступом в полтора пикселя от соседних столбцов.
fn column(kind: TrackKind) -> (f32, f32) {
    let i = match kind {
        TrackKind::Treads => 0.0,
        TrackKind::Wheels => 1.0,
        TrackKind::Feet => 2.0,
    };
    let pad = 1.5 / 48.0;
    (i / 3.0 + pad, (i + 1.0) / 3.0 - pad)
}

/// Меш всех следов: квад на отрезок, сетка 4×4 на отпечаток ступни.
fn build_mesh(tracks: &Tracks, terrain: &Terrain, now: f32) -> Mesh {
    let count = tracks.segments.len() * 4 + tracks.footprints.len() * 16;
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(count);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(count);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(count);
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity(count);
    let mut indices: Vec<u32> = Vec::with_capacity(tracks.segments.len() * 6 + tracks.footprints.len() * 54);
    let [r, g, b] = TRACK_COLOR;

    for segment in &tracks.segments {
        let age = now - segment.born;
        let fade = ((LIFETIME - age) / FADE).clamp(0.0, 1.0);
        let (alpha, repeat) = match segment.kind {
            TrackKind::Treads => (TREADS_ALPHA, TREAD_REPEAT),
            _ => (WHEELS_ALPHA, WHEEL_REPEAT),
        };
        let color = [r, g, b, alpha * fade];
        let (u0, u1) = column(segment.kind);
        let base = positions.len() as u32;
        for c in [segment.from, segment.to] {
            let v = c.distance / repeat;
            positions.push(c.left.to_array());
            positions.push(c.right.to_array());
            normals.extend([c.normal.to_array(); 2]);
            uvs.push([u0, v]);
            uvs.push([u1, v]);
            colors.extend([color; 2]);
        }
        // Обе стороны квада: машина могла ехать в любую сторону, обход не угадать.
        indices.extend_from_slice(&[base, base + 2, base + 1, base + 1, base + 2, base + 3]);
        indices.extend_from_slice(&[base, base + 1, base + 2, base + 1, base + 3, base + 2]);
    }

    let (u0, u1) = column(TrackKind::Feet);
    let color = [r, g, b, FEET_ALPHA];
    for foot in &tracks.footprints {
        let forward = foot.forward;
        let right = Vec2::new(-forward.y, forward.x);
        let base = positions.len() as u32;
        for j in 0..4 {
            for i in 0..4 {
                let (s, t) = (i as f32 / 3.0, j as f32 / 3.0);
                let p = foot.center + right * (s - 0.5) * FOOT_SIZE.x + forward * (t - 0.5) * FOOT_SIZE.y;
                positions.push([p.x, mesh_height(terrain, p.x, p.y) + LIFT, p.y]);
                normals.push(terrain.normal_at(p.x, p.y).to_array());
                uvs.push([u0 + (u1 - u0) * s, t]);
                colors.push(color);
            }
        }
        for j in 0..3u32 {
            for i in 0..3u32 {
                let a = base + j * 4 + i;
                let quad = [a, a + 1, a + 4, a + 5];
                indices.extend_from_slice(&[quad[0], quad[2], quad[1], quad[1], quad[2], quad[3]]);
                indices.extend_from_slice(&[quad[0], quad[1], quad[2], quad[1], quad[3], quad[2]]);
            }
        }
    }

    if positions.is_empty() {
        return empty_mesh();
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
        .with_inserted_indices(Indices::U32(indices))
}

/// Ставит эмиттер пыли у задней оси колёсных и гусеничных машин, кроме босса.
fn attach_dust(
    mut commands: Commands,
    effect: Res<DustEffect>,
    vehicles: Query<(Entity, &GroundVehicle, &Transform), (Added<GroundVehicle>, Without<Boss>)>,
) {
    for (entity, vehicle, transform) in &vehicles {
        // Эмиттер — дочерняя сущность: его сдвиг в единицах корпуса до масштаба.
        let scale = transform.scale.x.max(0.01);
        let local = Vec3::new(0.0, -vehicle.ride_height / scale + 0.1, vehicle.half_base.y / scale);
        let settings = SpawnerSettings::new(0.0.into(), DUST_PERIOD.into(), DUST_PERIOD.into(), 0);
        let dust = commands
            .spawn((
                WheelDust,
                ParticleEffect::new(effect.0.clone()),
                EffectSpawner::new(&settings),
                Transform::from_translation(local),
            ))
            .id();
        commands.entity(entity).add_child(dust);
    }
}

/// Густота пыли из-под колёс по скорости, частиц в секунду.
pub fn wheel_dust_rate(speed: f32) -> f32 {
    DUST_RATE * ((speed.abs() - DUST_FROM_SPEED) / (DUST_FULL_SPEED - DUST_FROM_SPEED)).clamp(0.0, 1.0)
}

/// Густота пыли машин: по скорости, ноль на лаве, вдали от лунохода и на ступени «Низкая».
fn emit_dust(
    quality: Res<Quality>,
    pools: Res<LavaPools>,
    rover: Query<&GlobalTransform, With<Rover>>,
    vehicles: Query<(&GroundVehicle, &GlobalTransform)>,
    mut dust: Query<(&ChildOf, &mut EffectSpawner), With<WheelDust>>,
) {
    let rover = rover.iter().next().map(GlobalTransform::translation);
    for (parent, mut spawner) in &mut dust {
        let Ok((vehicle, transform)) = vehicles.get(parent.parent()) else {
            continue;
        };
        let position = transform.translation();
        let near = rover.is_some_and(|r| r.distance(position) < DUST_RANGE);
        let rate = if *quality == Quality::Low || !near || lava::is_lava(&pools, position) {
            0.0
        } else {
            wheel_dust_rate(vehicle.speed) * quality.particle_share()
        };
        let count = rate * DUST_PERIOD;
        if spawner.settings.count() != count.into() {
            spawner.settings.set_count(count.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f32) -> Contact {
        Contact {
            center: Vec2::new(x, 0.0),
            left: Vec3::new(x, 0.0, -0.3),
            right: Vec3::new(x, 0.0, 0.3),
            normal: Vec3::Y,
            distance: x,
        }
    }

    fn segment(x: f32, born: f32) -> Segment {
        Segment {
            from: point(x),
            to: point(x + SEGMENT_STEP),
            kind: TrackKind::Wheels,
            born,
        }
    }

    #[test]
    fn отрезки_старше_40_с_удаляются_и_буфер_не_растёт_больше_предела() {
        let mut tracks = Tracks::default();
        for i in 0..MAX_SEGMENTS * 2 {
            tracks.push(segment(i as f32 * SEGMENT_STEP, i as f32 * 0.01));
        }
        assert_eq!(tracks.segments.len(), MAX_SEGMENTS);
        // Самые старые ушли первыми: остались последние.
        assert!(tracks.segments.front().unwrap().born > (MAX_SEGMENTS as f32 - 1.0) * 0.01);

        let mut tracks = Tracks::default();
        tracks.push(segment(0.0, 0.0));
        tracks.push(segment(1.0, 20.0));
        tracks.expire(39.0);
        assert_eq!(tracks.segments.len(), 2);
        assert!(tracks.fading(39.0), "последние 10 с жизни след бледнеет");
        tracks.expire(41.0);
        assert_eq!(tracks.segments.len(), 1);
        tracks.expire(61.0);
        assert!(tracks.segments.is_empty());
    }

    #[test]
    fn отрезок_в_круге_изменения_удаляется_вне_круга_остаётся() {
        let mut tracks = Tracks::default();
        for i in 0..50 {
            tracks.push(segment(i as f32 * SEGMENT_STEP, 0.0));
        }
        tracks.push_foot(Footprint {
            center: Vec2::new(10.0, 0.0),
            forward: Vec2::NEG_Y,
        });
        tracks.cut(Vec2::new(10.0, 0.0), 3.0);
        assert!(tracks.footprints.is_empty());
        assert!(!tracks.segments.is_empty());
        for s in &tracks.segments {
            for p in [s.from.center, s.to.center] {
                assert!(p.distance(Vec2::new(10.0, 0.0)) >= 3.0, "конец {p} в круге");
            }
        }
        // Вне круга остались отрезки по обе стороны кратера.
        assert!(tracks.segments.iter().any(|s| s.to.center.x < 7.0));
        assert!(tracks.segments.iter().any(|s| s.from.center.x > 13.0));
    }

    #[test]
    fn высота_ленты_совпадает_с_вершинами_рельефа() {
        let terrain = Terrain::new(7);
        for (vx, vz) in [(3, 4), (-10, 7), (40, -33)] {
            let h = mesh_height(&terrain, vx as f32, vz as f32);
            assert!((h - terrain.vertex_height(vx, vz)).abs() < 1e-4);
        }
    }

    #[test]
    fn пыль_только_на_ходу() {
        assert_eq!(wheel_dust_rate(0.0), 0.0);
        assert_eq!(wheel_dust_rate(DUST_FROM_SPEED), 0.0);
        assert!(wheel_dust_rate(6.0) > 0.0);
        assert_eq!(wheel_dust_rate(100.0), DUST_RATE);
    }
}
