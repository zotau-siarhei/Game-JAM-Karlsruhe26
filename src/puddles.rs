//! Лужи в низинах: мелкая вода в лунках рельефа, через которую машины едут с брызгами.
//!
//! Лужа выбирается при первом появлении участка в сцене по хешу сида и координат: примерно
//! в каждом седьмом участке, на пологой низине вне гор, путей и домов. Лужа целиком внутри
//! своего участка: лунка глубиной [`DEPTH`] с неровным краем и ровным дном вдавливается в карту
//! высот этого участка, земля вокруг темнеет, поверх лежит плоский диск воды. Объекты на месте
//! лужи уступают ей: они помечаются разрушенными ещё до появления в сцене. Решение и лужа хранятся
//! в ресурсе [`Puddles`] до конца забега, поэтому после отъезда и возвращения лужа та же.
//!
//! [`Puddles::at`] отвечает, в воде ли точка плана: её спрашивают следы, пыль, гильзы, птицы
//! и трава. Машина с [`TrackMaker`] в воде поднимает брызги и пускает круги ряби, луноход при
//! въезде шлёт [`WaterSplash`] для звука всплеска. Скорость машин вода не меняет.
//!
//! Взрыв рядом с водой поднимает столб брызг. Изменение рельефа (`TerrainChanged`), задевшее
//! лужу, убирает её до конца забега, лава рядом испаряет её с паром.
//!
//! Вода в кратерах: каждое изменение рельефа запоминается кандидатом. Кратер глубиной от 0,8
//! до 6 м, который [`CRATER_QUIET`] секунд не менялся и не у лавы, за [`CRATER_FILL`] секунд
//! набирает воду до трети глубины, не выше порога перелива через вал. Площадь — заливка
//! `lava::flood` от дна, не дальше радиуса чаши плюс [`CRATER_SPREAD`]. Новый взрыв в кратере
//! выплёскивает воду и начинает отсчёт заново. `Puddles::at` отвечает и за эту воду.

use std::collections::{HashMap, HashSet};
use std::f32::consts::TAU;

use bevy::asset::RenderAssetUsages;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy_hanabi::prelude::*;

use crate::camera::Focus;
use crate::craters;
use crate::explosion::Explosion;
use crate::frame::GameSet;
use crate::lava::{self, LavaPools};
use crate::props::{PropStatus, Props};
use crate::railway::TRACK_BAND;
use crate::rover::Rover;
use crate::run::{RunScoped, WorldRebuilt};
use crate::settlements::{Footprint, Settlements, region_of};
use crate::subsidence::TerrainChanged;
use crate::terrain::{self, CHUNK_SIZE, CRATER_EXTENT, ChunkDirty, ChunkMesh, Terrain, chunk_coord_at, chunk_origin};
use crate::tracks::{TrackMaker, mesh_height};

/// Доля участков, где ищется место для лужи: подходит примерно каждое четвёртое место.
const PUDDLE_CHANCE: f32 = 0.6;

/// Сколько мест в участке пробуется, прежде чем отказаться от лужи.
const ATTEMPTS: u32 = 10;

/// Радиус лунки без неровности края: от и до, м. Лужа выходит от 2 до 8 м в поперечнике.
const RADIUS: (f32, f32) = (1.5, 4.0);

/// Неровность края: доли радиуса у гармоник 3 и 5 по углу.
const WOBBLE: (f32, f32) = (0.18, 0.1);

/// Глубина лунки в центре, м.
const DEPTH: f32 = 0.35;

/// Насколько темнеет мокрая земля в лунке: доля яркости, которая уходит.
const WET_DARKEN: f32 = 0.45;

/// До скольких радиусов края темнеет земля вокруг лунки.
const WET_REACH: f32 = 1.2;

/// Диск воды — эта доля края лунки: дальше меш рельефа уже выше воды.
const DISC_SHARE: f32 = 0.92;

/// `Puddles::at` считает водой эту долю края лунки: там вода точно видна.
const WATER_SHARE: f32 = 0.72;

/// Вода ниже самой низкой точки края диска на столько, м: диск не торчит над берегом.
const SHORE_MARGIN: f32 = 0.03;

/// Выше этого — горы, там луж нет, м.
const MAX_HEIGHT: f32 = 8.0;

/// Склон под лужей не круче: нормаль по Y не меньше.
const MIN_UP: f32 = 0.94;

/// Насколько может гулять высота по краю лунки, м: дно выравнивается по самой низкой точке
/// края, и с верхней стороны лунки остаётся бережок такой высоты.
const RIM_FLATNESS: f32 = 0.6;

/// Дно выровнено полностью до этой доли края лунки, дальше плавно сходит к нетронутому краю.
const FLAT_BOTTOM: f32 = 0.6;

/// Низина: середина ниже средней высоты точек на таком расстоянии, м.
const LOWLAND_REACH: f32 = 6.0;

/// Зазор до стартовой точки и домов от края лунки, м; объекты ближе последнего убираются.
const START_CLEAR: f32 = 12.0;
const HOUSE_CLEAR: f32 = 2.0;
const PROP_CLEAR: f32 = 0.8;

/// Отрезков по кругу у диска воды и у кольца ряби.
const DISC_SEGMENTS: usize = 28;
const RING_SEGMENTS: usize = 32;

/// Цвет воды: тёмная синь с отсветом неба, почти непрозрачная.
const WATER_COLOR: Color = Color::srgba(0.16, 0.28, 0.40, 0.85);

/// Брызги машины в воде не чаще этого, с, и не медленнее этой скорости, м/с.
const SPLASH_PERIOD: f32 = 0.2;
const SPLASH_SPEED: f32 = 1.0;

/// Дальше этого от фокуса брызг нет, м.
const SPLASH_RANGE: f32 = 70.0;

/// Звук въезда лунохода в воду не чаще этого, с.
const SOUND_PERIOD: f32 = 1.0;

/// Круг ряби: за сколько растёт и тает, с, начальный и конечный радиус, м, непрозрачность.
const RIPPLE_TIME: f32 = 1.0;
const RIPPLE_RADIUS: (f32, f32) = (0.3, 2.2);
const RIPPLE_ALPHA: f32 = 0.55;

/// Сколько живёт сущность разового эффекта брызг, столба брызг и пара, с.
const SPLASH_LIFETIME: f32 = 1.2;
const COLUMN_LIFETIME: f32 = 2.0;
const STEAM_LIFETIME: f32 = 4.0;

/// Вода в кратере: затишье до начала наполнения и время подъёма, с.
const CRATER_QUIET: f32 = 20.0;
const CRATER_FILL: f32 = 60.0;

/// Глубина дна кратера ниже исходного рельефа, при которой набирается вода: от и до, м.
const CRATER_DEPTH: (f32, f32) = (0.8, 6.0);

/// Вода не дальше радиуса чаши плюс столько, м.
const CRATER_SPREAD: f32 = 2.0;

/// Уровень ниже порога перелива через вал на столько, м.
const SPILL_MARGIN: f32 = 0.15;

/// Вода видна и считается водой, когда она выше дна на столько, м.
const MIN_WATER: f32 = 0.05;

/// Чаши меньше этого радиуса не запоминаются: шаги «Колосса», м.
const MIN_CRATER: f32 = 1.5;

/// Больше этого радиуса чаша не растёт при слиянии кратеров, м.
const MAX_CRATER: f32 = 14.0;

/// Запоминается не больше стольких кратеров: сверх предела уходит старый без воды.
const MAX_CRATERS: usize = 48;

/// Кратеры и лава у воды проверяются раз в столько секунд.
const CHECK_PERIOD: f32 = 1.0;

/// Плагин луж.
pub struct PuddlesPlugin;

impl Plugin for PuddlesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Puddles>()
            .add_message::<WaterSplash>()
            .add_systems(Startup, create_assets)
            .add_systems(
                Update,
                (
                    reset_puddles.in_set(GameSet::Reset),
                    // До пересборки мешей этого кадра: лунка попадает в меш сразу, а объекты
                    // на месте лужи убираются до своего появления.
                    place_puddles
                        .after(terrain::stream_chunks)
                        .before(terrain::rebuild_dirty_chunks)
                        .in_set(GameSet::Streaming),
                    // После кратеров этого кадра: задетая вода пропадает по новому рельефу.
                    soak_blasts.after(craters::dig_craters).in_set(GameSet::Damage),
                    fill_craters.in_set(GameSet::Simulation),
                    (wade, animate_ripples, expire_splashes).chain().in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Луноход въехал в воду: для звука всплеска.
#[derive(Message, Debug, Clone, Copy)]
pub struct WaterSplash {
    pub position: Vec3,
}

/// Лужа: лунка с неровным краем и уровень воды в ней.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Puddle {
    /// Середина на плане.
    pub center: Vec2,
    /// Радиус лунки без неровности края, м.
    pub radius: f32,
    /// Высота глади, м.
    pub level: f32,
    /// Высота самой низкой точки края до лунки: по ней выровнено дно, м.
    base: f32,
    /// Сдвиги фаз гармоник края, рад.
    phase: [f32; 2],
}

impl Puddle {
    /// Край лунки в направлении `angle`, м.
    fn edge(&self, angle: f32) -> f32 {
        self.radius
            * (1.0 + WOBBLE.0 * (3.0 * angle + self.phase[0]).sin() + WOBBLE.1 * (5.0 * angle + self.phase[1]).sin())
    }

    /// Дальше этого от середины лунки нет ни в одном направлении, м.
    pub fn reach(&self) -> f32 {
        self.radius * (1.0 + WOBBLE.0 + WOBBLE.1)
    }

    /// Расстояние от середины в долях края лунки в этом направлении.
    fn relative(&self, p: Vec2) -> f32 {
        let d = p - self.center;
        d.length() / self.edge(d.y.atan2(d.x))
    }

    /// Вода ли в точке плана.
    fn contains(&self, p: Vec2) -> bool {
        p.distance_squared(self.center) < self.reach() * self.reach() && self.relative(p) < WATER_SHARE
    }
}

/// Лужи забега.
#[derive(Resource, Default)]
pub struct Puddles {
    /// Участки, для которых лужа уже выбрана: `None` — лужи нет или её уже нет.
    chunks: HashMap<IVec2, Option<Puddle>>,
    /// Кратеры, которые могут набрать воду, по времени появления.
    craters: Vec<Crater>,
    /// Сколько прошло с последней проверки кратеров и лавы, с.
    clock: f32,
}

/// Кратер, который может набрать воду.
struct Crater {
    center: Vec2,
    /// Радиус чаши, м.
    radius: f32,
    /// Сколько прошло с последнего изменения, с.
    quiet: f32,
    /// Уже проверен после затишья: до нового изменения не проверяется.
    checked: bool,
    water: Option<CraterWater>,
}

/// Вода в кратере.
struct CraterWater {
    /// Дно: отсюда идёт заливка.
    origin: IVec2,
    bottom: f32,
    target: f32,
    level: f32,
    age: f32,
    cells: HashSet<IVec2>,
    /// Сущность плоскости воды.
    entity: Option<Entity>,
    /// Ячейки менялись: плоскость пересобирается.
    dirty: bool,
}

impl CraterWater {
    /// Видна ли уже вода.
    fn visible(&self) -> bool {
        self.level > self.bottom + MIN_WATER
    }
}

impl Puddles {
    /// В воде ли точка плана: в луже или в воде кратера.
    pub fn at(&self, p: Vec2) -> bool {
        self.level_at(p).is_some()
    }

    /// Уровень воды в точке плана, если там вода.
    pub fn level_at(&self, p: Vec2) -> Option<f32> {
        // Лужа целиком внутри своего участка: достаточно участка точки.
        if let Some(Some(puddle)) = self.chunks.get(&chunk_coord_at(p.x, p.y))
            && puddle.contains(p)
        {
            return Some(puddle.level);
        }
        let cell = IVec2::new(p.x.round() as i32, p.y.round() as i32);
        self.craters
            .iter()
            .filter_map(|c| c.water.as_ref())
            .find(|w| w.visible() && w.cells.contains(&cell))
            .map(|w| w.level)
    }

    /// Точка на глади воды в пределах `reach` от точки плана: середина лужи или ближняя к взрыву
    /// точка воды в кратере.
    fn water_near(&self, p: Vec2, reach: f32) -> Option<Vec3> {
        let here = chunk_coord_at(p.x, p.y);
        for dz in -1..=1 {
            for dx in -1..=1 {
                if let Some(Some(puddle)) = self.chunks.get(&(here + IVec2::new(dx, dz)))
                    && puddle.center.distance(p) < reach + puddle.reach()
                {
                    return Some(Vec3::new(puddle.center.x, puddle.level, puddle.center.y));
                }
            }
        }
        self.craters.iter().find_map(|crater| {
            let water = crater.water.as_ref().filter(|w| w.visible())?;
            let d = crater.center.distance(p);
            (d < reach + crater.radius).then(|| {
                let at = if d < crater.radius { p } else { crater.center };
                Vec3::new(at.x, water.level, at.y)
            })
        })
    }
}

/// Диск воды лужи: дочерняя сущность меша участка.
#[derive(Component)]
struct PuddleWater {
    chunk: IVec2,
}

/// Плоскость воды в кратере.
#[derive(Component)]
struct CraterSurface;

/// Участок уже проверен на лужу.
#[derive(Component)]
pub struct PuddleChecked;

/// Брызги и рябь машины в воде.
#[derive(Component, Default)]
struct Wader {
    /// Где была в прошлом кадре.
    last: Option<Vec3>,
    /// Была ли в воде в прошлом кадре.
    inside: bool,
    /// До следующих брызг и до следующего звука, с.
    splash: f32,
    sound: f32,
}

/// Круг ряби на воде.
#[derive(Component)]
struct Ripple {
    age: f32,
}

/// Разовый эффект брызг: удаляется через столько секунд.
#[derive(Component)]
struct Fleeting(f32);

/// Общие меши, материал и эффекты воды.
#[derive(Resource)]
pub struct WaterAssets {
    water: Handle<StandardMaterial>,
    ring: Handle<Mesh>,
    splash: Handle<EffectAsset>,
    column: Handle<EffectAsset>,
    steam: Handle<EffectAsset>,
}

/// Хеш сида, участка и номера: число от 0 до 1.
fn hash(seed: u32, coord: IVec2, salt: u32) -> f32 {
    let mut x = seed ^ 0x5D1E_ACE5 ^ (coord.x as u32).wrapping_mul(0x9E37_79B1) ^ (coord.y as u32).wrapping_mul(0x85EB_CA77);
    x ^= salt.wrapping_mul(0xC2B2_AE3D);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

/// Место и форма лужи в участке, если она там есть. Уровня воды ещё нет: его даёт [`dig`].
///
/// Зависит только от сида, нетронутого рельефа и домов. Лужа с мокрым кольцом вокруг целиком
/// внутри участка и не касается его крайних вершин: соседние участки она не меняет.
pub fn plan_puddle(terrain: &Terrain, coord: IVec2, houses: &[Footprint]) -> Option<Puddle> {
    let seed = terrain.seed();
    if hash(seed, coord, 1) >= PUDDLE_CHANCE {
        return None;
    }
    let origin = chunk_origin(coord);
    (0..ATTEMPTS).find_map(|i| {
        let radius = RADIUS.0 + (RADIUS.1 - RADIUS.0) * hash(seed, coord, 10 + i);
        let mut puddle = Puddle {
            center: Vec2::ZERO,
            radius,
            level: 0.0,
            base: 0.0,
            phase: [hash(seed, coord, 20 + i) * TAU, hash(seed, coord, 30 + i) * TAU],
        };
        let reach = puddle.reach();
        let margin = reach * WET_REACH + 1.0;
        let span = CHUNK_SIZE as f32 - 2.0 * margin;
        puddle.center = origin + Vec2::new(margin + hash(seed, coord, 40 + i) * span, margin + hash(seed, coord, 50 + i) * span);
        site_fits(terrain, &mut puddle, houses).then_some(puddle)
    })
}

/// Подходит ли место: пологая нетронутая низина вне гор, путей, старта и домов. Запоминает
/// высоту дна.
fn site_fits(terrain: &Terrain, puddle: &mut Puddle, houses: &[Footprint]) -> bool {
    let c = puddle.center;
    let reach = puddle.reach();
    let h = terrain.height_at(c.x, c.y);
    let (vx, vz) = (c.x.round() as i32, c.y.round() as i32);
    if h >= MAX_HEIGHT
        || (terrain.vertex_height(vx, vz) - terrain.original_height(vx, vz)).abs() > 1e-3
        || terrain.normal_at(c.x, c.y).y < MIN_UP
        || c.length() < START_CLEAR + reach
        || terrain.lines().nearest(c).distance < TRACK_BAND + reach
        || houses.iter().any(|f| f.distance(c) < reach + HOUSE_CLEAR)
    {
        return false;
    }
    let around: f32 = (0..8)
        .map(|i| {
            let p = c + Vec2::from_angle(i as f32 * TAU / 8.0) * LOWLAND_REACH;
            terrain.height_at(p.x, p.y)
        })
        .sum::<f32>()
        / 8.0;
    if h >= around {
        return false;
    }
    let rim: Vec<f32> = (0..16)
        .map(|i| {
            let a = i as f32 * TAU / 16.0;
            let p = c + Vec2::from_angle(a) * puddle.edge(a);
            terrain.height_at(p.x, p.y)
        })
        .collect();
    let low = rim.iter().copied().fold(f32::MAX, f32::min);
    let high = rim.iter().copied().fold(f32::MIN, f32::max);
    puddle.base = low;
    high - low < RIM_FLATNESS
}

/// Вдавливает лунку лужи в рельеф, темнит мокрую землю и ставит уровень воды по самой низкой
/// точке края диска. Дно — чаша глубиной [`DEPTH`] под самой низкой точкой края: на пологом
/// склоне вода всё равно лежит во всю лунку. Возвращает изменённые участки.
fn dig(terrain: &mut Terrain, puddle: &mut Puddle) -> Vec<IVec2> {
    let c = puddle.center;
    let wet = puddle.reach() * WET_REACH;
    let (x0, x1) = ((c.x - wet).floor() as i32, (c.x + wet).ceil() as i32);
    let (z0, z1) = ((c.y - wet).floor() as i32, (c.y + wet).ceil() as i32);
    let mut changed = Vec::new();
    for vz in z0..=z1 {
        for vx in x0..=x1 {
            let x = puddle.relative(Vec2::new(vx as f32, vz as f32));
            if x >= WET_REACH {
                continue;
            }
            let bottom = puddle.base - DEPTH * (1.0 - x * x).max(0.0).powi(2);
            let w = ((1.0 - x) / (1.0 - FLAT_BOTTOM)).clamp(0.0, 1.0);
            let blend = w * w * (3.0 - 2.0 * w);
            let t = ((x - 0.85) / (WET_REACH - 0.85)).clamp(0.0, 1.0);
            let darken = WET_DARKEN * (1.0 - t * t * (3.0 - 2.0 * t));
            for coord in terrain.edit_vertex(vx, vz, |h, color| {
                *h -= blend * (*h - bottom).max(0.0);
                for channel in &mut color[..3] {
                    *channel *= 1.0 - darken;
                }
            }) {
                if !changed.contains(&coord) {
                    changed.push(coord);
                }
            }
        }
    }
    puddle.level = (0..DISC_SEGMENTS)
        .map(|i| {
            let a = i as f32 * TAU / DISC_SEGMENTS as f32;
            let p = c + Vec2::from_angle(a) * puddle.edge(a) * DISC_SHARE;
            mesh_height(terrain, p.x, p.y)
        })
        .fold(f32::MAX, f32::min)
        - SHORE_MARGIN;
    changed
}

/// Дома из регионов вокруг участка: их контуры на плане.
fn nearby_houses(coord: IVec2, terrain: &Terrain, settlements: &mut Settlements) -> Vec<Footprint> {
    let origin = chunk_origin(coord);
    let margin = 20.0;
    let low = region_of(origin - Vec2::splat(margin));
    let high = region_of(origin + Vec2::splat(CHUNK_SIZE as f32 + margin));
    let mut houses = Vec::new();
    for rz in low.y..=high.y {
        for rx in low.x..=high.x {
            let region = settlements.region(IVec2::new(rx, rz), terrain);
            houses.extend(region.houses.iter().map(|h| h.footprint()));
        }
    }
    houses
}

/// Объекты участка на месте лужи уступают ей: помечаются разрушенными и не появляются.
fn clear_props(puddle: &Puddle, coord: IVec2, terrain: &Terrain, settlements: &mut Settlements, props: &mut Props) {
    let clear = puddle.reach() + PROP_CLEAR;
    for prop in props.chunk(coord, terrain, settlements).iter_mut() {
        if prop.transform.translation.xz().distance(puddle.center) < clear {
            prop.status = PropStatus::Destroyed;
        }
    }
}

fn create_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut effects: ResMut<Assets<EffectAsset>>,
) {
    commands.insert_resource(WaterAssets {
        water: materials.add(StandardMaterial {
            base_color: WATER_COLOR,
            alpha_mode: bevy::prelude::AlphaMode::Blend,
            perceptual_roughness: 0.05,
            reflectance: 0.6,
            ..default()
        }),
        ring: meshes.add(ring_mesh()),
        splash: effects.add(splash_effect()),
        column: effects.add(column_effect()),
        steam: effects.add(steam_effect()),
    });
}

/// Брызги из-под колёс: капли летят вверх и вбок и падают обратно.
fn splash_effect() -> EffectAsset {
    let writer = ExprWriter::new();
    let init_pos = SetPositionSphereModifier {
        center: writer.lit(Vec3::Y * 0.1).expr(),
        radius: writer.lit(0.5).expr(),
        dimension: ShapeDimension::Volume,
    };
    let side = writer.rand(VectorType::VEC3F) * writer.lit(Vec3::new(2.0, 0.0, 2.0)) - writer.lit(Vec3::new(1.0, 0.0, 1.0));
    let up = writer.lit(Vec3::Y) * writer.lit(1.5).uniform(writer.lit(2.5));
    let speed = writer.lit(1.8).uniform(writer.lit(3.0));
    let init_vel = SetAttributeModifier::new(Attribute::VELOCITY, ((side + up) * speed).expr());
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, writer.lit(0.45).uniform(writer.lit(0.8)).expr());
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let update_gravity = AccelModifier::new(writer.lit(Vec3::Y * -18.0).expr());

    let mut color = bevy_hanabi::Gradient::new();
    color.add_key(0.0, Vec4::new(0.80, 0.88, 0.95, 0.85));
    color.add_key(0.7, Vec4::new(0.70, 0.80, 0.90, 0.6));
    color.add_key(1.0, Vec4::new(0.70, 0.80, 0.90, 0.0));
    let mut size = bevy_hanabi::Gradient::new();
    size.add_key(0.0, Vec3::splat(0.18));
    size.add_key(1.0, Vec3::splat(0.08));

    EffectAsset::new(64, SpawnerSettings::once(24.0.into()), writer.finish())
        .with_name("puddle_splash")
        .init(init_pos)
        .init(init_vel)
        .init(init_lifetime)
        .init(init_age)
        .update(update_gravity)
        .render(OrientModifier::new(OrientMode::FaceCameraPosition))
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Overwrite,
            mask: ColorBlendMask::RGBA,
        })
        .render(SizeOverLifetimeModifier {
            gradient: size,
            screen_space_size: false,
        })
}

/// Столб брызг от взрыва: вода летит вверх выше машины и падает обратно. Высоту задаёт `power`.
fn column_effect() -> EffectAsset {
    let writer = ExprWriter::new();
    let power = writer.add_property("power", Value::Scalar(1.0.into()));
    let init_pos = SetPositionSphereModifier {
        center: writer.lit(Vec3::Y * 0.2).expr(),
        radius: writer.lit(1.0).expr(),
        dimension: ShapeDimension::Volume,
    };
    let side = writer.rand(VectorType::VEC3F) * writer.lit(Vec3::new(2.0, 0.0, 2.0)) - writer.lit(Vec3::new(1.0, 0.0, 1.0));
    let up = writer.lit(Vec3::Y) * writer.lit(3.0).uniform(writer.lit(5.0));
    let speed = writer.lit(2.2).uniform(writer.lit(3.6)) * writer.prop(power);
    let init_vel = SetAttributeModifier::new(Attribute::VELOCITY, ((side + up) * speed).expr());
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, writer.lit(0.9).uniform(writer.lit(1.6)).expr());
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let update_gravity = AccelModifier::new(writer.lit(Vec3::Y * -20.0).expr());
    let update_drag = LinearDragModifier::new(writer.lit(0.4).expr());

    let mut color = bevy_hanabi::Gradient::new();
    color.add_key(0.0, Vec4::new(0.85, 0.92, 1.0, 0.9));
    color.add_key(0.7, Vec4::new(0.75, 0.85, 0.95, 0.7));
    color.add_key(1.0, Vec4::new(0.75, 0.85, 0.95, 0.0));
    let mut size = bevy_hanabi::Gradient::new();
    size.add_key(0.0, Vec3::splat(0.35));
    size.add_key(1.0, Vec3::splat(0.18));

    EffectAsset::new(256, SpawnerSettings::once(90.0.into()), writer.finish())
        .with_name("puddle_column")
        .init(init_pos)
        .init(init_vel)
        .init(init_lifetime)
        .init(init_age)
        .update(update_gravity)
        .update(update_drag)
        .render(OrientModifier::new(OrientMode::FaceCameraPosition))
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Overwrite,
            mask: ColorBlendMask::RGBA,
        })
        .render(SizeOverLifetimeModifier {
            gradient: size,
            screen_space_size: false,
        })
}

/// Пар над испарившейся водой: белые клубы поднимаются, расплываются и тают.
fn steam_effect() -> EffectAsset {
    let writer = ExprWriter::new();
    let init_pos = SetPositionSphereModifier {
        center: writer.lit(Vec3::Y * 0.3).expr(),
        radius: writer.lit(1.5).expr(),
        dimension: ShapeDimension::Volume,
    };
    let init_vel = SetVelocitySphereModifier {
        center: writer.lit(Vec3::Y * -2.0).expr(),
        speed: writer.lit(0.6).uniform(writer.lit(1.6)).expr(),
    };
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, writer.lit(2.0).uniform(writer.lit(3.5)).expr());
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let update_rise = AccelModifier::new(writer.lit(Vec3::new(0.4, 1.2, -0.2)).expr());
    let update_drag = LinearDragModifier::new(writer.lit(1.0).expr());
    let round = RoundModifier {
        roundness: writer.lit(1.0).expr(),
    };

    let mut color = bevy_hanabi::Gradient::new();
    color.add_key(0.0, Vec4::new(0.95, 0.95, 0.95, 0.0));
    color.add_key(0.15, Vec4::new(0.95, 0.95, 0.95, 0.55));
    color.add_key(1.0, Vec4::new(0.9, 0.9, 0.9, 0.0));
    let mut size = bevy_hanabi::Gradient::new();
    size.add_key(0.0, Vec3::splat(0.6));
    size.add_key(1.0, Vec3::splat(2.8));

    EffectAsset::new(64, SpawnerSettings::once(40.0.into()), writer.finish())
        .with_name("puddle_steam")
        .init(init_pos)
        .init(init_vel)
        .init(init_lifetime)
        .init(init_age)
        .update(update_rise)
        .update(update_drag)
        .render(OrientModifier::new(OrientMode::FaceCameraPosition))
        .render(round)
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Overwrite,
            mask: ColorBlendMask::RGBA,
        })
        .render(SizeOverLifetimeModifier {
            gradient: size,
            screen_space_size: false,
        })
}

/// Плоское кольцо радиуса 1 шириной 0,12: круг ряби, размер задаёт масштаб.
fn ring_mesh() -> Mesh {
    let mut positions = Vec::with_capacity((RING_SEGMENTS + 1) * 2);
    for i in 0..=RING_SEGMENTS {
        let d = Vec2::from_angle(i as f32 * TAU / RING_SEGMENTS as f32);
        for r in [0.88, 1.0] {
            positions.push([d.x * r, 0.0, d.y * r]);
        }
    }
    let mut indices = Vec::with_capacity(RING_SEGMENTS * 6);
    for i in 0..RING_SEGMENTS as u32 {
        let (a, b, c, d) = (2 * i, 2 * i + 1, 2 * i + 2, 2 * i + 3);
        indices.extend_from_slice(&[a, c, b, b, c, d]);
    }
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_indices(Indices::U32(indices))
}

/// Диск воды лужи с неровным краем, середина в начале координат.
fn disc_mesh(puddle: &Puddle) -> Mesh {
    let mut positions = vec![[0.0f32; 3]];
    for i in 0..DISC_SEGMENTS {
        let a = i as f32 * TAU / DISC_SEGMENTS as f32;
        let p = Vec2::from_angle(a) * puddle.edge(a) * DISC_SHARE;
        positions.push([p.x, 0.0, p.y]);
    }
    let n = DISC_SEGMENTS as u32;
    // Обход против часовой стрелки при взгляде сверху: лицевая сторона смотрит вверх.
    let indices: Vec<u32> = (0..n).flat_map(|i| [0, 1 + (i + 1) % n, 1 + i]).collect();
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_indices(Indices::U32(indices))
}

/// Новый мир: лужи выбираются заново. Диски прошлого мира удалены вместе с мешами участков.
fn reset_puddles(mut rebuilt: MessageReader<WorldRebuilt>, mut puddles: ResMut<Puddles>) {
    if rebuilt.read().last().is_some() {
        *puddles = Puddles::default();
    }
}

/// Выбирает лужи новых участков сцены, вдавливает лунки и ставит диски воды.
#[allow(clippy::too_many_arguments)]
pub fn place_puddles(
    mut commands: Commands,
    mut puddles: ResMut<Puddles>,
    mut terrain: ResMut<Terrain>,
    mut settlements: ResMut<Settlements>,
    mut props: ResMut<Props>,
    mut meshes: ResMut<Assets<Mesh>>,
    assets: Res<WaterAssets>,
    chunks: Query<(Entity, &ChunkMesh), Without<PuddleChecked>>,
) {
    // До объявления мира дома и объекты пустые: лужа встала бы на место будущего дома.
    if settlements.seed().is_none() {
        return;
    }
    for (entity, chunk) in &chunks {
        let coord = chunk.0;
        commands.entity(entity).try_insert(PuddleChecked);
        let puddle = match puddles.chunks.get(&coord) {
            Some(decided) => *decided,
            None => {
                let houses = nearby_houses(coord, &terrain, &mut settlements);
                let mut puddle = plan_puddle(&terrain, coord, &houses);
                if let Some(puddle) = puddle.as_mut() {
                    clear_props(puddle, coord, &terrain, &mut settlements, &mut props);
                    dig(&mut terrain, puddle);
                    commands.entity(entity).try_insert(ChunkDirty);
                }
                puddles.chunks.insert(coord, puddle);
                puddle
            }
        };
        let Some(puddle) = puddle else {
            continue;
        };
        let local = puddle.center - chunk_origin(coord);
        commands.spawn((
            PuddleWater { chunk: coord },
            Mesh3d(meshes.add(disc_mesh(&puddle))),
            MeshMaterial3d(assets.water.clone()),
            Transform::from_xyz(local.x, puddle.level, local.y),
            NotShadowCaster,
            ChildOf(entity),
        ));
    }
}

/// Брызги, рябь и всплеск: машина в воде на ходу раз в [`SPLASH_PERIOD`] поднимает брызги.
#[allow(clippy::too_many_arguments)]
fn wade(
    mut commands: Commands,
    time: Res<Time>,
    focus: Res<Focus>,
    puddles: Res<Puddles>,
    assets: Res<WaterAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut makers: Query<(Entity, &GlobalTransform, Option<&mut Wader>, Has<Rover>), With<TrackMaker>>,
    mut splashes: MessageWriter<WaterSplash>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for (entity, transform, wader, rover) in &mut makers {
        let Some(mut wader) = wader else {
            commands.entity(entity).try_insert(Wader::default());
            continue;
        };
        let position = transform.translation();
        let speed = wader.last.map_or(0.0, |last| last.xz().distance(position.xz()) / dt);
        wader.last = Some(position);
        wader.splash -= dt;
        wader.sound -= dt;
        if position.distance(focus.0) > SPLASH_RANGE {
            wader.inside = false;
            continue;
        }
        let Some(level) = puddles.level_at(position.xz()) else {
            wader.inside = false;
            continue;
        };
        let at = Vec3::new(position.x, level, position.z);
        if rover && !wader.inside && wader.sound <= 0.0 {
            wader.sound = SOUND_PERIOD;
            splashes.write(WaterSplash { position: at });
        }
        wader.inside = true;
        if speed < SPLASH_SPEED || wader.splash > 0.0 {
            continue;
        }
        wader.splash = SPLASH_PERIOD;
        spawn_splash(&mut commands, &assets, &mut materials, at);
    }
}

/// Брызги и круг ряби на глади в точке `at`.
fn spawn_splash(commands: &mut Commands, assets: &WaterAssets, materials: &mut Assets<StandardMaterial>, at: Vec3) {
    commands.spawn((
        ParticleEffect::new(assets.splash.clone()),
        Transform::from_translation(at),
        Fleeting(SPLASH_LIFETIME),
        RunScoped,
    ));
    commands.spawn((
        Ripple { age: 0.0 },
        Mesh3d(assets.ring.clone()),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgba(0.85, 0.92, 1.0, RIPPLE_ALPHA),
            alpha_mode: bevy::prelude::AlphaMode::Blend,
            unlit: true,
            ..default()
        })),
        Transform::from_translation(at + Vec3::Y * 0.015).with_scale(Vec3::splat(RIPPLE_RADIUS.0)),
        NotShadowCaster,
        RunScoped,
    ));
}

/// Круги ряби растут и тают.
fn animate_ripples(
    mut commands: Commands,
    time: Res<Time>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut ripples: Query<(Entity, &mut Ripple, &mut Transform, &MeshMaterial3d<StandardMaterial>)>,
) {
    let dt = time.delta_secs();
    for (entity, mut ripple, mut transform, material) in &mut ripples {
        ripple.age += dt;
        let t = ripple.age / RIPPLE_TIME;
        if t >= 1.0 {
            commands.entity(entity).try_despawn();
            continue;
        }
        let grow = 1.0 - (1.0 - t) * (1.0 - t);
        transform.scale = Vec3::splat(RIPPLE_RADIUS.0 + (RIPPLE_RADIUS.1 - RIPPLE_RADIUS.0) * grow);
        if let Some(mut material) = materials.get_mut(&material.0) {
            material.base_color.set_alpha(RIPPLE_ALPHA * (1.0 - t));
        }
    }
}

/// Удаляет отыгравшие разовые эффекты брызг.
fn expire_splashes(mut commands: Commands, time: Res<Time>, mut fx: Query<(Entity, &mut Fleeting)>) {
    for (entity, mut left) in &mut fx {
        left.0 -= time.delta_secs();
        if left.0 <= 0.0 {
            commands.entity(entity).try_despawn();
        }
    }
}

/// Разовый эффект воды в точке `at`: столб брызг мощности `power` или пар.
fn spawn_burst(commands: &mut Commands, effect: &Handle<EffectAsset>, at: Vec3, power: f32, life: f32) {
    let properties = EffectProperties::default().with_properties([("power".to_string(), Value::Scalar(power.into()))]);
    commands.spawn((
        ParticleEffect::new(effect.clone()),
        properties,
        Transform::from_translation(at),
        Fleeting(life),
        RunScoped,
    ));
}

/// Удаляет диски воды лужи участка.
fn remove_discs(commands: &mut Commands, discs: &Query<(Entity, &PuddleWater)>, coord: IVec2) {
    for (entity, disc) in discs {
        if disc.chunk == coord {
            commands.entity(entity).try_despawn();
        }
    }
}

/// Дно кратера, его высота и уровень, до которого поднимется вода. `None` — воды не будет:
/// кратер мельче 0,8 м или глубже 6 м ниже исходного рельефа, или вал не держит воду.
pub fn crater_water(terrain: &Terrain, center: Vec2, radius: f32) -> Option<(IVec2, f32, f32)> {
    let r = radius.min(MAX_CRATER);
    let (x0, x1) = ((center.x - r).floor() as i32, (center.x + r).ceil() as i32);
    let (z0, z1) = ((center.y - r).floor() as i32, (center.y + r).ceil() as i32);
    let mut bottom: Option<(IVec2, f32)> = None;
    for vz in z0..=z1 {
        for vx in x0..=x1 {
            if Vec2::new(vx as f32, vz as f32).distance(center) > r {
                continue;
            }
            let h = terrain.vertex_height(vx, vz);
            if bottom.is_none_or(|(_, b)| h < b) {
                bottom = Some((IVec2::new(vx, vz), h));
            }
        }
    }
    let (origin, h) = bottom?;
    let depth = terrain.original_height(origin.x, origin.y) - h;
    if !(CRATER_DEPTH.0..=CRATER_DEPTH.1).contains(&depth) {
        return None;
    }
    // Порог перелива: на каждом луче самая высокая точка вала, из них самая низкая.
    let spill = (0..24)
        .map(|i| {
            let dir = Vec2::from_angle(i as f32 * TAU / 24.0);
            (0..=10)
                .map(|j| {
                    let q = center + dir * r * (0.6 + j as f32 * 0.1);
                    terrain.height_at(q.x, q.y)
                })
                .fold(f32::MIN, f32::max)
        })
        .fold(f32::MAX, f32::min);
    let target = (h + depth / 3.0).min(spill - SPILL_MARGIN);
    (target > h + 2.0 * MIN_WATER).then_some((origin, h, target))
}

/// Ячейки воды кратера на уровне `level`: заливка от дна, не дальше радиуса чаши плюс
/// [`CRATER_SPREAD`] от середины.
pub fn water_cells(terrain: &Terrain, origin: IVec2, center: Vec2, radius: f32, level: f32) -> HashSet<IVec2> {
    let reach = radius.min(MAX_CRATER) + CRATER_SPREAD;
    let from_origin = (reach + origin.as_vec2().distance(center)).ceil() as i32;
    let mut cells = lava::flood(origin, level, from_origin, |x, z| terrain.vertex_height(x, z));
    cells.retain(|c| c.as_vec2().distance(center) <= reach);
    cells
}

/// Взрыв у воды поднимает столб брызг. Изменение рельефа убирает задетые лужи до конца забега,
/// осушает воду в задетом кратере и запоминает кратер: он может набрать воду заново.
fn soak_blasts(
    mut commands: Commands,
    mut puddles: ResMut<Puddles>,
    assets: Res<WaterAssets>,
    mut explosions: MessageReader<Explosion>,
    mut changes: MessageReader<TerrainChanged>,
    discs: Query<(Entity, &PuddleWater)>,
) {
    let puddles = &mut *puddles;
    for e in explosions.read() {
        if let Some(at) = puddles.water_near(e.position.xz(), e.radius) {
            spawn_burst(&mut commands, &assets.column, at, e.power.clamp(0.5, 2.0).sqrt(), COLUMN_LIFETIME);
        }
    }
    for change in changes.read() {
        let c = change.center.xz();
        let r = change.radius;
        let reach = r + RADIUS.1 * (1.0 + WOBBLE.0 + WOBBLE.1);
        let (low, high) = (chunk_coord_at(c.x - reach, c.y - reach), chunk_coord_at(c.x + reach, c.y + reach));
        for cz in low.y..=high.y {
            for cx in low.x..=high.x {
                let coord = IVec2::new(cx, cz);
                if let Some(slot) = puddles.chunks.get_mut(&coord)
                    && slot.is_some_and(|p| p.center.distance(c) < r + p.reach() * WATER_SHARE)
                {
                    *slot = None;
                    remove_discs(&mut commands, &discs, coord);
                }
            }
        }
        let bowl = r / CRATER_EXTENT;
        if let Some(crater) = puddles.craters.iter_mut().find(|k| k.center.distance(c) < k.radius + bowl) {
            crater.radius = crater.radius.max(crater.center.distance(c) + bowl).min(MAX_CRATER);
            crater.quiet = 0.0;
            crater.checked = false;
            if let Some(entity) = crater.water.take().and_then(|w| w.entity) {
                commands.entity(entity).try_despawn();
            }
        } else if bowl >= MIN_CRATER {
            if puddles.craters.len() >= MAX_CRATERS {
                let old = puddles.craters.iter().position(|k| k.water.is_none()).unwrap_or(0);
                if let Some(entity) = puddles.craters.remove(old).water.and_then(|w| w.entity) {
                    commands.entity(entity).try_despawn();
                }
            }
            puddles.craters.push(Crater {
                center: c,
                radius: bowl.min(MAX_CRATER),
                quiet: 0.0,
                checked: false,
                water: None,
            });
        }
    }
}

/// Наполняет старые кратеры водой, держит плоскости воды на уровне и испаряет воду у лавы.
#[allow(clippy::too_many_arguments)]
fn fill_craters(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    pools: Res<LavaPools>,
    mut puddles: ResMut<Puddles>,
    assets: Res<WaterAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut surfaces: Query<&mut Transform, With<CraterSurface>>,
    discs: Query<(Entity, &PuddleWater)>,
) {
    let dt = time.delta_secs();
    let puddles = &mut *puddles;
    puddles.clock += dt;
    let check = puddles.clock >= CHECK_PERIOD;
    if check {
        puddles.clock = 0.0;
    }
    for crater in &mut puddles.craters {
        crater.quiet += dt;
        let reach = crater.radius + CRATER_SPREAD;
        if check && crater.water.is_none() && !crater.checked && crater.quiet >= CRATER_QUIET {
            crater.checked = true;
            if !pools.near(crater.center, reach)
                && let Some((origin, bottom, target)) = crater_water(&terrain, crater.center, crater.radius)
            {
                crater.water = Some(CraterWater {
                    origin,
                    bottom,
                    target,
                    level: bottom,
                    age: 0.0,
                    cells: HashSet::from([origin]),
                    entity: None,
                    dirty: true,
                });
            }
        }
        let Some(water) = crater.water.as_mut() else {
            continue;
        };
        // Лава дошла до воды: вода уходит паром.
        if check && pools.near(crater.center, reach) {
            if water.visible() {
                let c = crater.center;
                spawn_burst(&mut commands, &assets.steam, Vec3::new(c.x, water.level, c.y), 1.0, STEAM_LIFETIME);
            }
            if let Some(entity) = water.entity {
                commands.entity(entity).try_despawn();
            }
            crater.water = None;
            continue;
        }
        let rising = water.age < CRATER_FILL;
        water.age += dt;
        let t = (water.age / CRATER_FILL).min(1.0);
        water.level = water.bottom + (water.target - water.bottom) * t;
        if rising && (check || water.age >= CRATER_FILL) {
            let cells = water_cells(&terrain, water.origin, crater.center, crater.radius, water.level);
            if cells != water.cells {
                water.cells = cells;
                water.dirty = true;
            }
        }
        if water.dirty {
            water.dirty = false;
            if let Some(entity) = water.entity.take() {
                commands.entity(entity).try_despawn();
            }
            water.entity = Some(
                commands
                    .spawn((
                        CraterSurface,
                        Mesh3d(meshes.add(lava::surface_mesh(&water.cells, 0.0))),
                        MeshMaterial3d(assets.water.clone()),
                        Transform::from_xyz(0.0, water.level, 0.0),
                        NotShadowCaster,
                        RunScoped,
                    ))
                    .id(),
            );
        } else if let Some(entity) = water.entity
            && let Ok(mut transform) = surfaces.get_mut(entity)
        {
            transform.translation.y = water.level;
        }
    }
    // Лужи у лавы испаряются.
    if check {
        for (coord, slot) in puddles.chunks.iter_mut() {
            if let Some(p) = slot
                && pools.near(p.center, p.reach())
            {
                spawn_burst(&mut commands, &assets.steam, Vec3::new(p.center.x, p.level, p.center.y), 1.0, STEAM_LIFETIME);
                *slot = None;
                remove_discs(&mut commands, &discs, *coord);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Лужа участка по тем же правилам, что в игре.
    fn plan(seed: u32, coord: IVec2) -> Option<Puddle> {
        let terrain = Terrain::new(seed);
        let mut settlements = Settlements::new(seed);
        let houses = nearby_houses(coord, &terrain, &mut settlements);
        plan_puddle(&terrain, coord, &houses)
    }

    fn sample_chunks() -> impl Iterator<Item = IVec2> {
        (-4..=4).flat_map(|z| (-4..=4).map(move |x| IVec2::new(x, z)))
    }

    #[test]
    fn лужа_целиком_внутри_участка() {
        let mut found = 0;
        for seed in [3, 9, 77] {
            for coord in sample_chunks() {
                let Some(puddle) = plan(seed, coord) else {
                    continue;
                };
                found += 1;
                let low = chunk_origin(coord) + Vec2::ONE;
                let high = chunk_origin(coord) + Vec2::splat(CHUNK_SIZE as f32 - 1.0);
                let wet = puddle.reach() * WET_REACH;
                let (a, b) = (puddle.center - Vec2::splat(wet), puddle.center + Vec2::splat(wet));
                assert!(a.cmpge(low).all() && b.cmple(high).all(), "сид {seed}, участок {coord}: лужа {puddle:?}");
                // Лунка с мокрым кольцом не меняет крайние вершины: соседние участки не тронуты.
                let mut terrain = Terrain::new(seed);
                let mut dug = puddle;
                let changed = dig(&mut terrain, &mut dug);
                assert_eq!(changed, vec![coord]);
                assert!(dug.level > terrain.height_at(puddle.center.x, puddle.center.y), "в лунке нет воды");
            }
        }
        assert!(found > 15, "луж почти нет: {found}");
        eprintln!("луж: {found} на 243 участка");
    }

    #[test]
    fn тот_же_сид_даёт_те_же_лужи() {
        let a: Vec<_> = sample_chunks().map(|c| plan(9, c)).collect();
        let b: Vec<_> = sample_chunks().map(|c| plan(9, c)).collect();
        assert_eq!(a, b);
        let other: Vec<_> = sample_chunks().map(|c| plan(10, c)).collect();
        assert_ne!(a, other);
    }

    /// Точка вдали от путей и старта, где кратер мортиры точно держит воду.
    fn crater_site(seed: u32) -> (Terrain, Vec2) {
        let terrain = Terrain::new(seed);
        let p = (0..40)
            .map(|i| Vec2::new(60.0 + i as f32 * 17.0, -45.0 - i as f32 * 11.0))
            .find(|p| terrain.normal_at(p.x, p.y).y > 0.97 && terrain.lines().nearest(*p).distance > TRACK_BAND)
            .expect("нет ровного места");
        (terrain, p)
    }

    #[test]
    fn кратер_глубже_6_м_не_набирает_воду() {
        for seed in [3, 7, 11] {
            let (mut terrain, p) = crater_site(seed);
            let center = Vec3::new(p.x, 0.0, p.y);
            // Мортира: радиус 5 м, чаша 2,5 м, вал 0,6 м.
            terrain.apply_crater(center, 5.0, 2.5, 0.6);
            assert!(crater_water(&terrain, p, 5.0).is_some(), "сид {seed}: кратер 2,5 м без воды");
            terrain.apply_crater(center, 5.0, 2.5, 0.6);
            terrain.apply_crater(center, 5.0, 2.5, 0.6);
            assert!(crater_water(&terrain, p, 5.0).is_none(), "сид {seed}: в воронке 7,5 м вода");
        }
    }

    #[test]
    fn вода_не_выходит_за_радиус_кратера_плюс_2_м() {
        for seed in [3, 7, 11] {
            let (mut terrain, p) = crater_site(seed);
            terrain.apply_crater(Vec3::new(p.x, 0.0, p.y), 5.0, 2.5, 0.6);
            let (origin, bottom, target) = crater_water(&terrain, p, 5.0).unwrap();
            assert!(target <= bottom + 2.5 / 3.0 + 0.05, "выше трети глубины");
            let cells = water_cells(&terrain, origin, p, 5.0, target);
            assert!(cells.len() > 5, "сид {seed}: воды почти нет");
            for c in &cells {
                assert!(c.as_vec2().distance(p) <= 7.0, "сид {seed}: вода в {c}");
                assert!(terrain.vertex_height(c.x, c.y) < target, "сид {seed}: ячейка {c} над водой");
            }
            // Даже уровень выше вала не выводит воду за предел.
            for c in water_cells(&terrain, origin, p, 5.0, target + 10.0) {
                assert!(c.as_vec2().distance(p) <= 7.0, "сид {seed}: перелив в {c}");
            }
        }
    }
}
