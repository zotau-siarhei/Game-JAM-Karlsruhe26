//! Следы на грунте: колеи колёсных машин, рифлёные полосы гусениц, отпечатки ступней «Колосса»
//! и пыль из-под колёс врагов.
//!
//! Машина с [`TrackMaker`] через каждые [`SEGMENT_STEP`] метров пути кладёт под левой и правой
//! опорой отрезок, соединённый с предыдущим той же опоры: дуги поворота не рвутся. Опора при этом
//! продавливает рельеф: вершины под ней опускаются на глубину колеи с плавным краем и буреют,
//! трава в полосе пропадает ([`GroundPressed`]). Ресурс [`Tracks`] помнит, на сколько продавлена
//! каждая вершина, поэтому повторный проезд колею не углубляет. Изменённые участки рельефа
//! пересобираются пачкой раз в [`FLUSH_PERIOD`]: просадка в сантиметры не шлёт `TerrainChanged`,
//! его читатели рассчитаны на кратеры.
//!
//! Следы живут до конца забега и хранятся по участкам рельефа: у участка в сцене свой меш-лента,
//! он пересобирается, только когда на участке что-то изменилось. Углы ленты ставятся при сборке
//! на те же треугольники, что и меш участка, и подняты на [`LIFT`]: после продавливания лента
//! ложится на дно колеи. Следы пропадают там, где рельеф изменился (`TerrainChanged`), и не
//! ложатся на лаву, щебень насыпи, пол вскрытого туннеля и воду.
//!
//! «Колосс» шагает: через каждые [`STRIDE`] метров одна из четырёх ступней вдавливает рельеф
//! на [`FOOT_DEPTH`] и оставляет тёмный отпечаток.
//!
//! Мокрый след: опора, вышедшая из воды (`Puddles::at`), ещё [`WET_LENGTH`] метров кладёт
//! тёмные и плотные отрезки, к концу обычные. В воде пыли нет.

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_hanabi::prelude::*;

use crate::boss::Boss;
use crate::frame::{GameSet, RunMessages};
use crate::lava::{self, LavaPools};
use crate::puddles::Puddles;
use crate::quality::Quality;
use crate::railway::{BALLAST_FULL, BALLAST_EDGE, TRACK_FLAT};
use crate::rover::Rover;
use crate::run::WorldRebuilt;
use crate::subsidence::TerrainChanged;
use crate::tank_look::TankGun;
use crate::terrain::{Terrain, VisibleChunks, chunk_coord_at};
use crate::vehicle::GroundVehicle;

/// Через сколько метров пути опоры кладётся новый отрезок, м.
const SEGMENT_STEP: f32 = 0.4;

/// Больше такого сдвига за кадр — перенос машины (рестарт), а не езда: колея начинается заново, м.
const MAX_JUMP: f32 = 3.0;

/// Предел отрезков колёс и гусениц на участке рельефа: при превышении первыми уходят самые старые.
const MAX_SEGMENTS: usize = 1500;

/// Предел отпечатков ступней на участке.
const MAX_FOOTPRINTS: usize = 120;

/// Глубина колеи гусениц и колёс, м.
const TREADS_DEPTH: f32 = 0.25;
const WHEELS_DEPTH: f32 = 0.15;

/// На сколько за край опоры спадает просадка до нуля, м: сетка рельефа — 1 м.
const PRESS_EDGE: f32 = 0.7;

/// Меньше такой добавки вершина не опускается: колея уже продавлена, м.
const PRESS_EPSILON: f32 = 0.01;

/// Насколько вершина на полной глубине колеи смешивается с цветом голого грунта.
const SOIL_MIX: f32 = 0.75;

/// Цвет голого грунта в колее, sRGB.
const SOIL: Color = Color::srgb(0.3, 0.21, 0.12);

/// Трава пропадает в полосе шире опоры на столько с каждой стороны, м.
const GRASS_CLEAR: f32 = 0.3;

/// Вершина продавлена глубже этого — трава на ней при появлении участка не растёт, м.
const GRASS_RUT_DEPTH: f32 = 0.05;

/// Пачка: изменённые участки рельефа и меши следов пересобираются не чаще этого, с.
const FLUSH_PERIOD: f32 = 0.15;

/// Подъём ленты над рельефом, м: против мерцания вместе со сдвигом глубины материала.
const LIFT: f32 = 0.04;

/// Сдвиг глубины материала следов: лента рисуется поверх рельефа.
const DEPTH_BIAS: f32 = 8.0;

/// Непрозрачность следа: гусеницы, колёса, ступни.
const TREADS_ALPHA: f32 = 0.7;
const WHEELS_ALPHA: f32 = 0.7;
const FEET_ALPHA: f32 = 0.7;

/// Цвет следа в линейном RGB: тёмный влажный грунт.
const TRACK_COLOR: [f32; 3] = [0.07, 0.05, 0.03];

/// Сколько метров за опорой тянется мокрый след после воды, м.
const WET_LENGTH: f32 = 6.0;

/// Мокрый след: насколько темнее цвет и какая непрозрачность у самого мокрого отрезка.
const WET_DARKEN: f32 = 0.6;
const WET_ALPHA: f32 = 0.85;

/// Длина одного повтора рисунка гусеницы вдоль следа, м: два грунтозацепа.
const TREAD_REPEAT: f32 = 0.85;

/// Длина повтора текстуры колеи колёс, м: рисунка вдоль нет, число только против дрожания UV.
const WHEEL_REPEAT: f32 = 4.0;

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
            .add_run_message::<GroundPressed>()
            .add_systems(Startup, create_assets)
            .add_systems(
                Update,
                (
                    reset_tracks.in_set(GameSet::Reset),
                    (lay_tracks, stomp, flush).chain().in_set(GameSet::Damage),
                    (attach_makers, attach_dust, cut_and_rebuild, emit_dust)
                        .chain()
                        .in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Опора машины продавила грунт: трава в круге пропадает.
#[derive(Message, Debug, Clone, Copy)]
pub struct GroundPressed {
    pub center: Vec2,
    pub radius: f32,
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

impl TrackKind {
    /// Глубина колеи, м: ступни вдавливают рельеф сами, в [`stomp`].
    fn depth(self) -> f32 {
        match self {
            TrackKind::Treads => TREADS_DEPTH,
            TrackKind::Wheels => WHEELS_DEPTH,
            TrackKind::Feet => 0.0,
        }
    }
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
    /// Сколько метров мокрого следа ещё осталось у левой и правой опоры.
    wet: [f32; 2],
}

/// Точка касания опоры на плане: высоты краёв полосы берутся из рельефа при сборке меша.
#[derive(Debug, Clone, Copy)]
struct Contact {
    /// Середина полосы.
    center: Vec2,
    /// Поперечное направление, вправо по ходу.
    right: Vec2,
    /// Полуширина полосы, м.
    half: f32,
    /// Путь вдоль колеи с её начала, м: координата текстуры.
    distance: f32,
}

/// Отрезок колеи между двумя точками касания одной опоры.
#[derive(Debug, Clone, Copy)]
struct Segment {
    from: Contact,
    to: Contact,
    kind: TrackKind,
    /// Насколько отрезок мокрый: 0 — сухой, 1 — только из воды.
    wet: f32,
}

/// Отпечаток ступни.
#[derive(Debug, Clone, Copy)]
struct Footprint {
    center: Vec2,
    /// Направление «вперёд» корпуса на плане.
    forward: Vec2,
}

/// Следы одного участка рельефа.
#[derive(Default)]
struct ChunkTracks {
    segments: VecDeque<Segment>,
    footprints: VecDeque<Footprint>,
    /// Сущность меша, пока участок в сцене.
    entity: Option<Entity>,
    /// Меш надо пересобрать.
    dirty: bool,
}

/// Следы и колеи забега.
#[derive(Resource, Default)]
pub struct Tracks {
    /// Следы по участкам: отрезок лежит на участке своего начала.
    chunks: HashMap<IVec2, ChunkTracks>,
    /// Отпечатки этого кадра: в буфер они попадают после того, как свой же `TerrainChanged`
    /// срежет старые следы под ступнёй.
    pending_feet: Vec<Footprint>,
    /// На сколько продавлена вершина мировой сетки, м.
    pressed: HashMap<IVec2, f32>,
    /// Участки рельефа, изменённые колеёй с последней пачки.
    terrain_dirty: HashSet<IVec2>,
    /// Сколько прошло с последней пачки, с.
    clock: f32,
    /// Пачка ушла в этом кадре: пора пересобрать меши следов.
    flushed: bool,
}

impl Tracks {
    /// Следы участка, создаёт пустые при первом обращении.
    fn chunk(&mut self, coord: IVec2) -> &mut ChunkTracks {
        self.chunks.entry(coord).or_default()
    }

    /// Добавляет отрезок на участок его начала; сверх предела уходит самый старый.
    fn push(&mut self, segment: Segment) {
        let chunk = self.chunk(chunk_coord_at(segment.from.center.x, segment.from.center.y));
        chunk.segments.push_back(segment);
        while chunk.segments.len() > MAX_SEGMENTS {
            chunk.segments.pop_front();
        }
        chunk.dirty = true;
    }

    /// Добавляет отпечаток ступни; сверх предела на участке уходит самый старый.
    fn push_foot(&mut self, foot: Footprint) {
        let chunk = self.chunk(chunk_coord_at(foot.center.x, foot.center.y));
        chunk.footprints.push_back(foot);
        while chunk.footprints.len() > MAX_FOOTPRINTS {
            chunk.footprints.pop_front();
        }
        chunk.dirty = true;
    }

    /// Убирает отрезки, у которых хоть один конец в круге на плане, и отпечатки с центром в нём.
    /// Участки, задетые кругом, пересобираются: рельеф под лентой мог измениться.
    fn cut(&mut self, center: Vec2, radius: f32) {
        let r2 = radius * radius;
        let inside = |p: Vec2| p.distance_squared(center) < r2;
        // Отрезок лежит на участке начала, а конец может быть на соседнем: запас в шаг отрезка.
        let reach = radius + SEGMENT_STEP;
        let (low, high) = (
            chunk_coord_at(center.x - reach, center.y - reach),
            chunk_coord_at(center.x + reach, center.y + reach),
        );
        for cz in low.y..=high.y {
            for cx in low.x..=high.x {
                let Some(chunk) = self.chunks.get_mut(&IVec2::new(cx, cz)) else {
                    continue;
                };
                chunk.segments.retain(|s| !inside(s.from.center) && !inside(s.to.center));
                chunk.footprints.retain(|f| !inside(f.center));
                chunk.dirty = true;
            }
        }
    }

    /// Продавлен ли грунт в точке плана настолько, что трава там не растёт.
    pub fn rut_at(&self, p: Vec2) -> bool {
        let v = IVec2::new(p.x.round() as i32, p.y.round() as i32);
        self.pressed.get(&v).is_some_and(|&d| d > GRASS_RUT_DEPTH)
    }

    /// Продавливает рельеф под опорой с центром `center` и полушириной `half` на глубину `depth`
    /// с плавным спадом за край. Вершина опускается только на то, чего ей не хватает до нужной
    /// глубины, и буреет пропорционально. Изменённые участки копятся до пачки.
    fn press(&mut self, terrain: &mut Terrain, center: Vec2, half: f32, depth: f32) {
        let outer = half + PRESS_EDGE;
        let soil = SOIL.to_linear();
        let soil = [soil.red, soil.green, soil.blue];
        let (x0, x1) = ((center.x - outer).floor() as i32, (center.x + outer).ceil() as i32);
        let (z0, z1) = ((center.y - outer).floor() as i32, (center.y + outer).ceil() as i32);
        for vz in z0..=z1 {
            for vx in x0..=x1 {
                let d = Vec2::new(vx as f32, vz as f32).distance(center);
                if d >= outer {
                    continue;
                }
                let t = ((d - half) / PRESS_EDGE).clamp(0.0, 1.0);
                let want = depth * (1.0 - t * t * (3.0 - 2.0 * t));
                let done = self.pressed.entry(IVec2::new(vx, vz)).or_insert(0.0);
                let delta = want - *done;
                if delta < PRESS_EPSILON {
                    continue;
                }
                *done = want;
                let mix = SOIL_MIX * delta / depth;
                let changed = terrain.edit_vertex(vx, vz, |h, c| {
                    *h -= delta;
                    for (channel, target) in c[..3].iter_mut().zip(soil) {
                        *channel += (target - *channel) * mix;
                    }
                });
                self.terrain_dirty.extend(changed);
            }
        }
    }
}

/// Общие ресурсы следов: материал ленты и образец меша. Образец (раскладка с цветом вершин)
/// создаётся при старте, чтобы прогрев (`src/prewarm.rs`) собрал его конвейер с материалом
/// ленты: иначе первый след в забеге стоил кадра компиляции (на телефоне — четверть секунды).
#[derive(Resource)]
#[allow(dead_code)]
struct TrackMaterial(Handle<StandardMaterial>, Handle<Mesh>);

/// Меш следов участка.
#[derive(Component)]
struct TrackMesh;

/// Эмиттер пыли у задней оси машины врагов или Службы.
#[derive(Component)]
struct WheelDust;

/// Общий эффект пыли из-под колёс.
#[derive(Resource)]
struct DustEffect(Handle<EffectAsset>);

/// Создаёт материал следов с текстурой и эффект пыли.
fn create_assets(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut effects: ResMut<Assets<EffectAsset>>,
    mut meshes: ResMut<Assets<Mesh>>,
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
    commands.insert_resource(TrackMaterial(material, meshes.add(empty_mesh())));
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

/// Новый забег или меню: следов и колей нет, меши участков удаляются.
fn reset_tracks(mut commands: Commands, mut rebuilt: MessageReader<WorldRebuilt>, mut tracks: ResMut<Tracks>) {
    if rebuilt.read().last().is_some() {
        for chunk in tracks.chunks.values() {
            if let Some(entity) = chunk.entity {
                commands.entity(entity).try_despawn();
            }
        }
        *tracks = Tracks::default();
    }
}

/// Вешает [`TrackMaker`] на луноход, машины врагов и Службы и на босса. Дрон-перехватчик парит
/// и следов не оставляет.
fn attach_makers(
    mut commands: Commands,
    rovers: Query<Entity, Added<Rover>>,
    vehicles: Query<
        (Entity, &GroundVehicle, Has<Boss>, Has<TankGun>),
        (Added<GroundVehicle>, Without<crate::interceptor::Interceptor>),
    >,
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
pub fn mesh_height(terrain: &Terrain, x: f32, z: f32) -> f32 {
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

/// Нельзя класть след: лава, щебень насыпи или пол вскрытого туннеля.
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
    // На щебне насыпи колеи нет: обрыв посередине перехода щебня в траву. Насыпь, срытая
    // кратером, щебень потеряла, по ней колея идёт как обычно.
    if near.distance < (BALLAST_FULL + BALLAST_EDGE) * 0.5 && (h - level).abs() < 1.0 {
        return true;
    }
    // Пол окна туннеля срезан до уровня путей во всю ширину ровной полосы.
    terrain.is_tunnel(near.line, near.s) && h < level + 1.0
}

/// Кладёт отрезки колей под опорами едущих машин и продавливает под ними грунт.
fn lay_tracks(
    mut terrain: ResMut<Terrain>,
    pools: Res<LavaPools>,
    puddles: Res<Puddles>,
    mut tracks: ResMut<Tracks>,
    mut makers: Query<(&GlobalTransform, &TrackMaker, &mut TrackTrail)>,
    mut pressed: MessageWriter<GroundPressed>,
) {
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
        let half = maker.width * 0.5;
        for (side, sign) in [(0, -1.0f32), (1, 1.0)] {
            let center = position.xz() + right * sign * maker.gauge * 0.5;
            let last = trail.last[side];
            let (distance, step) = match last {
                Some(last) => {
                    let step = last.center.distance(center);
                    if step < SEGMENT_STEP {
                        continue;
                    }
                    (last.distance + step, step)
                }
                None => (0.0, 0.0),
            };
            // В воде следа нет, а на выходе опора мокрая.
            if puddles.at(center) {
                trail.wet[side] = WET_LENGTH;
                trail.last[side] = None;
                continue;
            }
            if forbidden(&terrain, &pools, center) {
                trail.last[side] = None;
                continue;
            }
            tracks.press(&mut terrain, center, half, maker.kind.depth());
            pressed.write(GroundPressed {
                center,
                radius: half + GRASS_CLEAR,
            });
            let here = Contact {
                center,
                right,
                half,
                distance,
            };
            let wet = trail.wet[side] / WET_LENGTH;
            trail.wet[side] = (trail.wet[side] - step).max(0.0);
            if let Some(from) = last {
                tracks.push(Segment {
                    from,
                    to: here,
                    kind: maker.kind,
                    wet,
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

/// Раз в [`FLUSH_PERIOD`] помечает на пересборку участки рельефа, продавленные колеёй,
/// и разрешает в этом кадре пересобрать меши следов: лента и рельеф меняются вместе.
fn flush(mut commands: Commands, time: Res<Time>, visible: Res<VisibleChunks>, mut tracks: ResMut<Tracks>) {
    tracks.clock += time.delta_secs();
    if tracks.clock < FLUSH_PERIOD {
        return;
    }
    tracks.clock = 0.0;
    tracks.flushed = true;
    let changed: Vec<IVec2> = tracks.terrain_dirty.drain().collect();
    for &coord in &changed {
        for offset in [IVec2::ZERO, IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y] {
            if let Some(chunk) = tracks.chunks.get_mut(&(coord + offset)) {
                chunk.dirty = true;
            }
        }
    }
    visible.mark_dirty(&mut commands, changed);
}

/// Срезает следы по изменениям рельефа, ставит и убирает меши следов вслед за участками в сцене
/// и пересобирает изменённые.
#[allow(clippy::too_many_arguments)]
fn cut_and_rebuild(
    mut commands: Commands,
    terrain: Res<Terrain>,
    visible: Res<VisibleChunks>,
    material: Res<TrackMaterial>,
    mut changes: MessageReader<TerrainChanged>,
    mut tracks: ResMut<Tracks>,
    mut meshes: ResMut<Assets<Mesh>>,
    handles: Query<&Mesh3d, With<TrackMesh>>,
) {
    // Кратер и ступня меняют рельеф сразу, без пачки: ленту надо переложить в этом же кадре.
    let mut urgent = false;
    for change in changes.read() {
        tracks.cut(change.center.xz(), change.radius);
        urgent = true;
    }
    for foot in std::mem::take(&mut tracks.pending_feet) {
        tracks.push_foot(foot);
        urgent = true;
    }
    let tracks = &mut *tracks;
    let rebuild = std::mem::take(&mut tracks.flushed) || urgent;
    for (coord, chunk) in tracks.chunks.iter_mut() {
        match (visible.entity(*coord).is_some(), chunk.entity) {
            (false, Some(entity)) => {
                commands.entity(entity).try_despawn();
                chunk.entity = None;
            }
            (true, None) => {
                let mesh = meshes.add(build_mesh(chunk, &terrain));
                let entity = commands
                    .spawn((
                        TrackMesh,
                        Mesh3d(mesh),
                        MeshMaterial3d(material.0.clone()),
                        Transform::IDENTITY,
                        NotShadowCaster,
                        // Отрезки на краю выходят за участок: границы считать не стоит.
                        NoFrustumCulling,
                    ))
                    .id();
                chunk.entity = Some(entity);
                chunk.dirty = false;
            }
            (true, Some(entity)) if rebuild && chunk.dirty => {
                chunk.dirty = false;
                if let Ok(handle) = handles.get(entity)
                    && let Some(mut mesh) = meshes.get_mut(&handle.0)
                {
                    *mesh = build_mesh(chunk, &terrain);
                }
            }
            _ => {}
        }
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

/// Меш следов участка: квад на отрезок, сетка 4×4 на отпечаток ступни. Высоты — с рельефа сейчас.
fn build_mesh(chunk: &ChunkTracks, terrain: &Terrain) -> Mesh {
    let count = chunk.segments.len() * 4 + chunk.footprints.len() * 16;
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(count);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(count);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(count);
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity(count);
    let mut indices: Vec<u32> = Vec::with_capacity(chunk.segments.len() * 12 + chunk.footprints.len() * 108);
    let [r, g, b] = TRACK_COLOR;
    let ground = |p: Vec2| [p.x, mesh_height(terrain, p.x, p.y) + LIFT, p.y];

    for segment in &chunk.segments {
        let (alpha, repeat) = match segment.kind {
            TrackKind::Treads => (TREADS_ALPHA, TREAD_REPEAT),
            _ => (WHEELS_ALPHA, WHEEL_REPEAT),
        };
        let dark = 1.0 - WET_DARKEN * segment.wet;
        let alpha = alpha + (WET_ALPHA - alpha) * segment.wet;
        let color = [r * dark, g * dark, b * dark, alpha];
        let (u0, u1) = column(segment.kind);
        let base = positions.len() as u32;
        for c in [segment.from, segment.to] {
            let v = c.distance / repeat;
            positions.push(ground(c.center - c.right * c.half));
            positions.push(ground(c.center + c.right * c.half));
            normals.extend([terrain.normal_at(c.center.x, c.center.y).to_array(); 2]);
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
    for foot in &chunk.footprints {
        let forward = foot.forward;
        let right = Vec2::new(-forward.y, forward.x);
        let base = positions.len() as u32;
        for j in 0..4 {
            for i in 0..4 {
                let (s, t) = (i as f32 / 3.0, j as f32 / 3.0);
                let p = foot.center + right * (s - 0.5) * FOOT_SIZE.x + forward * (t - 0.5) * FOOT_SIZE.y;
                positions.push(ground(p));
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
    // Дрон-перехватчик парит: пыли из-под колёс у него нет.
    vehicles: Query<
        (Entity, &GroundVehicle, &Transform),
        (Added<GroundVehicle>, Without<Boss>, Without<crate::interceptor::Interceptor>),
    >,
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

/// Густота пыли машин: по скорости, ноль на лаве, в воде, вдали от лунохода и на ступени «Низкая».
fn emit_dust(
    quality: Res<Quality>,
    pools: Res<LavaPools>,
    puddles: Res<Puddles>,
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
        let wet = puddles.at(position.xz());
        let rate = if *quality == Quality::Low || !near || wet || lava::is_lava(&pools, position) {
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
            center: Vec2::new(x, 0.5),
            right: Vec2::Y,
            half: 0.3,
            distance: x,
        }
    }

    fn segment(x: f32) -> Segment {
        Segment {
            from: point(x),
            to: point(x + SEGMENT_STEP),
            kind: TrackKind::Wheels,
            wet: 0.0,
        }
    }

    fn segments(tracks: &Tracks) -> impl Iterator<Item = &Segment> {
        tracks.chunks.values().flat_map(|c| c.segments.iter())
    }

    #[test]
    fn буфер_участка_не_растёт_больше_предела_и_следы_не_гаснут() {
        let mut tracks = Tracks::default();
        // Все отрезки в одном участке: начало у x = 10.
        for _ in 0..MAX_SEGMENTS * 2 {
            tracks.push(segment(10.0));
        }
        assert_eq!(tracks.chunks.len(), 1);
        assert_eq!(segments(&tracks).count(), MAX_SEGMENTS);
        // Отрезки на другом участке не вытесняют чужие.
        tracks.push(segment(-10.0));
        assert_eq!(segments(&tracks).count(), MAX_SEGMENTS + 1);
    }

    #[test]
    fn отрезок_в_круге_изменения_удаляется_вне_круга_остаётся() {
        let mut tracks = Tracks::default();
        // Колея через границу участков x = 0.
        for i in 0..100 {
            tracks.push(segment(-20.0 + i as f32 * SEGMENT_STEP));
        }
        tracks.push_foot(Footprint {
            center: Vec2::new(0.0, 0.5),
            forward: Vec2::NEG_Y,
        });
        let c = Vec2::new(0.0, 0.5);
        tracks.cut(c, 3.0);
        assert!(tracks.chunks.values().all(|ch| ch.footprints.is_empty()));
        for s in segments(&tracks) {
            for p in [s.from.center, s.to.center] {
                assert!(p.distance(c) >= 3.0, "конец {p} в круге");
            }
        }
        // Вне круга остались отрезки по обе стороны кратера.
        assert!(segments(&tracks).any(|s| s.to.center.x < -3.0));
        assert!(segments(&tracks).any(|s| s.from.center.x > 3.0));
    }

    #[test]
    fn повторный_проезд_не_углубляет_колею() {
        let mut terrain = Terrain::new(7);
        let mut tracks = Tracks::default();
        let center = Vec2::new(5.3, 7.6);
        let before = terrain.vertex_height(5, 8);
        let far = terrain.vertex_height(9, 8);
        tracks.press(&mut terrain, center, 0.6, TREADS_DEPTH);
        let once = terrain.vertex_height(5, 8);
        assert!(before - once > 0.5 * TREADS_DEPTH, "вершина под опорой продавлена: {before} → {once}");
        assert!(!tracks.terrain_dirty.is_empty());
        for _ in 0..10 {
            tracks.press(&mut terrain, center, 0.6, TREADS_DEPTH);
        }
        assert!((terrain.vertex_height(5, 8) - once).abs() < 1e-5);
        // Вдали от опоры рельеф не тронут.
        assert_eq!(terrain.vertex_height(9, 8), far);
        assert!(tracks.rut_at(Vec2::new(5.2, 7.9)));
        assert!(!tracks.rut_at(Vec2::new(9.0, 8.0)));
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
