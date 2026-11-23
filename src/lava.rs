//! Лава под корой: глубокий кратер светится трещинами, пробитая кора извергается и разливает озеро.
//!
//! Глубина точки — исходная высота рельефа минус текущая. Глубже [`CRUST`] — прорыв: извержение
//! с каплями и взрывом, затем озеро. Озеро — горизонтальная поверхность на уровне `level` в связной
//! области ячеек 1×1 м, где рельеф ниже уровня, не дальше 40 м от места прорыва. Плоскость лавы
//! рисуется над всеми ячейками с запасом, а рельеф выше уровня сам закрывает её по глубине.
//!
//! Лава жжёт тиком [`BURN_TICK`]: луноход теряет здоровье и вязнет, враги и жители сгорают,
//! предметы исчезают, звенья путей рвутся, деревья и дома у края горят. Все враги, кроме
//! таран-стрелков, объезжают озёра. Над озёрами у фокуса камеры лопаются пузыри, поднимается дым,
//! изредка озеро плюётся каплями.

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::NotShadowCaster;
use bevy::math::Affine2;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::buildings::House;
use crate::camera::Focus;
use crate::combat::{Damage, Health, Team};
use crate::craters;
use crate::enemy::Enemy;
use crate::explosion::Explosion;
use crate::frame::{GameSet, RunMessages};
use crate::lava_flow::{self, Cells, FLOW_STEP, LavaCell, Layer, StepResult};
use crate::pickups::Pickup;
use crate::props::{Prop, PropKind};
use crate::rails::TrackDamage;
use crate::residents::Resident;
use crate::rng::GameRng;
use crate::rover::{Rover, RoverMotion};
use crate::run::{RunScoped, WorldRebuilt};
use crate::subsidence::TerrainChanged;
use crate::terrain::{CRATER_EXTENT, Terrain, VisibleChunks};
use crate::waves::Threat;

/// Толщина коры, м: мортира углубляет на 4,9 м за выстрел, прорыв — на шестом. Тоньше прежних
/// 32 м: шахта под луноходом до прорыва была такой глубокой, что камера смотрела в колодец.
pub const CRUST: f32 = 27.0;
/// С какой доли коры на дне появляются трещины и с какой — дым.
const CRACKS_FROM: f32 = 0.6;
const SMOKE_FROM: f32 = 0.75;
/// Наибольшее удаление лавы от места прорыва, м: страховка, если поток не остановится сам.
const FLOW_REACH: i32 = 80;
/// Уровень озера ниже исходной поверхности, м.
const POOL_BELOW: f32 = 4.0;
/// За сколько озеро набирает уровень, с: пузыри начинаются с половины.
const FILL_TIME: f32 = 6.0;
/// Сколько лавы подаёт источник, м³/с: шахта прорыва наполняется за несколько секунд.
const SOURCE_RATE: f32 = 250.0;
/// Радиус подачи вокруг места прорыва, ячеек.
const SOURCE_RADIUS: i32 = 2;
/// Сколько лавы из одного прорыва может вылиться за край ямы, м³.
const SPILL_BUDGET: f32 = 400.0;
/// Сетка пересобирается, если толщина где-то сдвинулась за шаг больше этого, м.
const MESH_EPS: f32 = 0.005;
/// Пузыри и плевки — только там, где лава толще этого, м.
const BUBBLE_DEPTH: f32 = 0.3;
/// Подъём уровня от взрыва в лаве, м.
const BLAST_RISE: f32 = 0.6;
/// Взрыв слабее этого уровень не поднимает: гибель врага (0,25), ракета (0,45), вражеская
/// мортира (0,35). Мортира игрока (1,4) и бомба (1,6) поднимают.
const BLAST_RISE_POWER: f32 = 1.0;
/// Потолок уровня ниже исходной поверхности у места прорыва, м: взрывы поднимают озеро
/// не больше чем на `POOL_BELOW - POOL_CEILING_BELOW` (около четырёх выстрелов мортиры).
const POOL_CEILING_BELOW: f32 = 1.5;
/// Взрыв слабее этого не выплёскивает капли.
const SPLASH_POWER: f32 = 0.3;
/// Шаг урона от лавы, с: урон приходит несколько раз в секунду, а не каждый кадр.
const BURN_TICK: f32 = 0.25;
/// Урон в секунду: луноходу, врагам и жителям (жители гибнут почти сразу).
const ROVER_BURN: f32 = 22.0;
const ENEMY_BURN: f32 = 60.0;
const RESIDENT_BURN: f32 = 300.0;
/// Урон в секунду горящему дереву или дому.
const EDGE_BURN: f32 = 8.0;
/// На каком удалении от ячейки лавы загораются деревья и дома, м: от центра дерева и от центра дома.
const TREE_REACH: f32 = 2.0;
const HOUSE_REACH: f32 = 5.0;
/// Сколько горит объект, в который попала капля, с.
const DROP_FIRE: f32 = 4.0;
/// Сколько объектов может гореть одновременно: лишние не загораются.
const MAX_FIRES: usize = 16;
/// Пузыри, дым и плевки — только у озёр ближе этого к фокуса камеры, м.
const BUBBLE_RANGE: f32 = 90.0;
/// Как часто на озере лопается пузырь и поднимается клуб дыма, с.
const BUBBLE_EVERY: f32 = 0.2;
const PUFF_EVERY: f32 = 0.6;
/// Между плевками каплями, с: от и до.
const SPIT_EVERY: (f32, f32) = (3.0, 6.0);
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
            .init_resource::<LavaFires>()
            .add_run_message::<CrustBreached>()
            .add_systems(Startup, create_assets)
            .add_systems(
                Update,
                (
                    reset_lava.in_set(GameSet::Reset),
                    (burn_in_lava, fly_drops, bubble_pools).chain().in_set(GameSet::Simulation),
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

/// Место прорыва: источник лавы и озеро в его яме. Сама лава — общие ячейки [`LavaPools`].
pub struct Pool {
    origin: IVec2,
    /// До какой высоты источник наполняет яму.
    target_level: f32,
    /// Выше этого взрывы уровень не поднимают.
    max_level: f32,
    age: f32,
    /// Сколько лавы вылилось за край ямы, м³.
    spilled: f32,
    /// Свет над ямой.
    light: Option<Entity>,
    /// Сколько до следующего пузыря, клуба дыма и плевка, с.
    bubble: f32,
    puff: f32,
    spit: f32,
}

impl Pool {
    /// Новое озеро в месте прорыва, исходная поверхность там на высоте `surface`.
    fn new(origin: IVec2, surface: f32) -> Self {
        Self {
            origin,
            target_level: surface - POOL_BELOW,
            max_level: surface - POOL_CEILING_BELOW,
            age: 0.0,
            spilled: 0.0,
            light: None,
            bubble: 0.0,
            puff: 0.0,
            spit: SPIT_EVERY.0,
        }
    }

    /// Поднимает уровень от взрыва мощности `power`, не выше потолка. Слабый взрыв не поднимает.
    fn raise(&mut self, power: f32) {
        if power >= BLAST_RISE_POWER {
            self.target_level = (self.target_level + BLAST_RISE).min(self.max_level.max(self.target_level));
        }
    }
}

/// Лава забега: места прорыва и ячейки с лавой ([`lava_flow`]).
#[derive(Resource, Default)]
pub struct LavaPools {
    pools: Vec<Pool>,
    cells: Cells,
    /// Горячие ячейки и их рамка: для быстрых проверок «где лава».
    hot: HashSet<IVec2>,
    bounds: Option<(IVec2, IVec2)>,
    /// Сколько накопилось времени до следующего шага течения, с.
    step_clock: f32,
    /// Сетки надо пересобрать.
    dirty: bool,
    /// Сетка горячей лавы (с дочерней сеткой корок поверх) и сетка застывшей.
    hot_surface: Option<Entity>,
    cooled_surface: Option<Entity>,
    /// Сколько накопилось времени до следующего тика урона, с.
    burn_clock: f32,
    /// Растёт при каждом изменении горячих ячеек, не сбрасывается и при перестройке мира.
    revision: u64,
}

impl LavaPools {
    /// Поверхность у места прорыва, целевой уровень, число горячих ячеек и есть ли сетка у каждого
    /// озера: для отладочных сцен.
    pub fn summary(&self) -> Vec<(f32, f32, usize, bool)> {
        self.pools
            .iter()
            .map(|p| {
                let surface = self.cells.get(&p.origin).map_or(f32::NAN, LavaCell::surface);
                (surface, p.target_level, self.hot.len(), self.hot_surface.is_some())
            })
            .collect()
    }

    /// Замер лавы за краем ямы у каждого озера: для сцены проверки `lava_slope`.
    pub fn overhangs(&self, terrain: &Terrain) -> Vec<Overhang> {
        let origins: Vec<IVec2> = self.pools.iter().map(|p| p.origin).collect();
        self.pools
            .iter()
            .map(|p| {
                let near = self
                    .cells
                    .iter()
                    .filter(|(c, _)| (**c - p.origin).length_squared() <= FLOW_REACH * FLOW_REACH)
                    .map(|(c, cell)| (*c, cell.surface()));
                Overhang::measure(near, &origins, |x, z| terrain.vertex_height(x, z), |x, z| terrain.original_height(x, z))
            })
            .collect()
    }

    /// Шаг течения: источники подают лаву в свои ямы, лава перетекает и застывает.
    fn step(&mut self, height: impl Fn(i32, i32) -> f32 + Copy) -> StepResult {
        let origins: Vec<IVec2> = self.pools.iter().map(|p| p.origin).collect();
        let pit = |c: IVec2| origins.iter().any(|o| (c - *o).as_vec2().length() <= Overhang::RIM);
        let allowed = |c: IVec2| origins.iter().any(|o| (c - *o).length_squared() <= FLOW_REACH * FLOW_REACH);
        for pool in &mut self.pools {
            pool.spilled = self
                .cells
                .iter()
                .filter(|(c, _)| !pit(**c) && (**c - pool.origin).length_squared() <= FLOW_REACH * FLOW_REACH)
                .map(|(_, cell)| cell.hot + cell.crust)
                .sum();
            if pool.spilled >= SPILL_BUDGET {
                continue;
            }
            // Источник льёт в ячейки вокруг места прорыва, где поверхность ниже уровня озера.
            let surface = |c: IVec2| self.cells.get(&c).map_or_else(|| height(c.x, c.y), LavaCell::surface);
            let spots: Vec<(IVec2, f32)> = (-SOURCE_RADIUS..=SOURCE_RADIUS)
                .flat_map(|dz| (-SOURCE_RADIUS..=SOURCE_RADIUS).map(move |dx| IVec2::new(dx, dz)))
                .filter(|o| o.length_squared() <= SOURCE_RADIUS * SOURCE_RADIUS)
                .map(|o| pool.origin + o)
                .map(|c| (c, pool.target_level - surface(c)))
                .filter(|(_, room)| *room > 0.0)
                .collect();
            if spots.is_empty() {
                continue;
            }
            let share = SOURCE_RATE * FLOW_STEP / spots.len() as f32;
            for (c, room) in spots {
                lava_flow::pour(&mut self.cells, c, share.min(room), height);
            }
        }
        let result = lava_flow::flow_step(&mut self.cells, FLOW_STEP, height, allowed, pit);
        if result.reshaped || result.moved > MESH_EPS {
            self.dirty = true;
        }
        if result.reshaped {
            self.refresh_hot();
        }
        result
    }

    /// Пересчитывает набор горячих ячеек и рамку.
    fn refresh_hot(&mut self) {
        let hot: HashSet<IVec2> = self.cells.iter().filter(|(_, c)| c.is_hot()).map(|(k, _)| *k).collect();
        if hot == self.hot {
            return;
        }
        self.bounds = hot.iter().fold(None, |b: Option<(IVec2, IVec2)>, c| match b {
            None => Some((*c, *c)),
            Some((lo, hi)) => Some((lo.min(*c), hi.max(*c))),
        });
        self.hot = hot;
        self.revision += 1;
    }
}

/// Сколько лавы озера лежит за краем ямы прорыва и как высоко она там над землёй. Лава, которая
/// течёт, за краем лежит тонким слоем по рельефу; горизонтальная плита над склоном — толстая
/// и выше исходной поверхности.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Overhang {
    /// Ячеек с лавой всего и за краем ямы (дальше [`Overhang::RIM`] от всех мест прорыва).
    pub cells: usize,
    pub outside: usize,
    /// Ячеек за краем, над которыми лава толще 1 м.
    pub thick: usize,
    /// Наибольшая толщина лавы за краем, м.
    pub max_thickness: f32,
    /// Насколько поверхность лавы за краем выше исходной поверхности склона, м (наибольшее).
    pub above_surface: f32,
    /// Дальняя ячейка от места прорыва, м.
    pub reach: f32,
    /// Ячеек за краем на нетронутой земле (рельеф ячейки и соседей не дальше 0,3 м от исходного: не кратер и не вал)
    /// и наибольшая толщина лавы на ней, м. Плита над склоном — толстая лава как раз на такой земле.
    pub intact: usize,
    pub intact_thickness: f32,
}

impl Overhang {
    /// Край ямы прорыва от её центра, м: кратер мортиры с запасом. Ближе лава не застывает.
    pub const RIM: f32 = 9.0;

    /// Замер по ячейкам с лавой: место и высота поверхности лавы.
    pub fn measure(
        cells: impl IntoIterator<Item = (IVec2, f32)>,
        origins: &[IVec2],
        height: impl Fn(i32, i32) -> f32,
        original: impl Fn(i32, i32) -> f32,
    ) -> Self {
        let distance = |c: IVec2| origins.iter().map(|o| (c - *o).as_vec2().length()).fold(f32::MAX, f32::min);
        let mut o = Overhang {
            cells: 0,
            outside: 0,
            thick: 0,
            max_thickness: 0.0,
            above_surface: f32::MIN,
            reach: 0.0,
            intact: 0,
            intact_thickness: 0.0,
        };
        for (c, surface) in cells {
            let d = distance(c);
            o.cells += 1;
            o.reach = o.reach.max(d);
            if d <= Self::RIM {
                continue;
            }
            let thickness = surface - height(c.x, c.y);
            o.outside += 1;
            o.thick += usize::from(thickness > 1.0);
            o.max_thickness = o.max_thickness.max(thickness);
            o.above_surface = o.above_surface.max(surface - original(c.x, c.y));
            let untouched = |x: i32, z: i32| (height(x, z) - original(x, z)).abs() <= 0.3;
            if [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)].iter().all(|(dx, dz)| untouched(c.x + dx, c.y + dz)) {
                o.intact += 1;
                o.intact_thickness = o.intact_thickness.max(thickness);
            }
        }
        o
    }
}

/// Горящие деревья и дома: огонь над целью и сколько ему ещё гореть.
#[derive(Resource, Default)]
struct LavaFires {
    fires: HashMap<Entity, Fire>,
}

struct Fire {
    flame: Entity,
    left: f32,
}

impl LavaFires {
    /// Поджигает цель или продлевает её огонь. Сверх предела новые цели не загораются.
    fn ignite(&mut self, commands: &mut Commands, target: Entity, at: Vec3, size: f32, time: f32) {
        if let Some(fire) = self.fires.get_mut(&target) {
            fire.left = fire.left.max(time);
            return;
        }
        if self.fires.len() >= MAX_FIRES {
            return;
        }
        // Пламя модуля огня стоит низом чуть ниже прежнего центра шара.
        let flame = commands
            .spawn((
                crate::fire::Fire::new(size * 1.4, 1.0).then_smoke(6.0),
                RunScoped,
                Transform::from_translation(at - Vec3::Y * size * 0.6),
            ))
            .id();
        self.fires.insert(target, Fire { flame, left: time });
    }
}

impl LavaPools {
    /// Номер версии горячих ячеек: изменился — изменилось, где лава жжёт.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Задевает ли рамка горячей лавы (с запасом в ячейку) прямоугольник плана `lo..hi`.
    pub fn touches(&self, lo: Vec2, hi: Vec2) -> bool {
        self.bounds.is_some_and(|(a, b)| {
            a.x as f32 - 1.0 <= hi.x && b.x as f32 + 1.0 >= lo.x && a.y as f32 - 1.0 <= hi.y && b.y as f32 + 1.0 >= lo.y
        })
    }

    pub fn is_empty(&self) -> bool {
        self.pools.is_empty()
    }

    /// Высота поверхности горячей лавы в точке плана, если она там есть.
    pub fn lava_at(&self, x: f32, z: f32) -> Option<f32> {
        let cell = IVec2::new(x.round() as i32, z.round() as i32);
        if !self.hot.contains(&cell) {
            return None;
        }
        self.cells.get(&cell).map(LavaCell::surface)
    }

    /// Есть ли горячая лава в `reach` метрах от точки плана.
    pub fn near(&self, p: Vec2, reach: f32) -> bool {
        let Some((lo, hi)) = self.bounds else {
            return false;
        };
        if p.x < lo.x as f32 - reach || p.x > hi.x as f32 + reach || p.y < lo.y as f32 - reach || p.y > hi.y as f32 + reach {
            return false;
        }
        let r = reach.ceil() as i32;
        let c = IVec2::new(p.x.round() as i32, p.y.round() as i32);
        (-r..=r).any(|dz| {
            (-r..=r).any(|dx| {
                let o = IVec2::new(dx, dz);
                o.as_vec2().length() <= reach && self.hot.contains(&(c + o))
            })
        })
    }

    /// Горячие ячейки: для мини-карты.
    pub fn cells(&self) -> impl Iterator<Item = &IVec2> {
        self.hot.iter()
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

/// Клуб дыма, искра или пузырь: летит, растёт со скоростью `grow` в секунду и исчезает.
#[derive(Component)]
struct Smoke {
    velocity: Vec3,
    age: f32,
    life: f32,
    grow: f32,
}

#[derive(Resource)]
struct LavaAssets {
    ball: Handle<Mesh>,
    drop: Handle<StandardMaterial>,
    smoke: Handle<StandardMaterial>,
    spark: Handle<StandardMaterial>,
    flow: Handle<StandardMaterial>,
    crust: Handle<StandardMaterial>,
    /// Застывшая лава.
    cooled: Handle<StandardMaterial>,
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
            // Огонь ярче единицы: светится через свечение камеры (`unlit` выводит только `base_color`).
            base_color: Color::linear_rgb(3.2, 1.0, 0.16),
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
            // Огонь ярче единицы: светится через свечение камеры (`unlit` выводит только `base_color`).
            base_color: Color::linear_rgb(4.0, 1.6, 0.32),
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
        // Застывшая лава: тёмный базальт, освещается как рельеф. Создаётся при старте, чтобы его
        // застал прогрев шейдеров.
        cooled: materials.add(StandardMaterial {
            base_color: Color::srgb(0.11, 0.09, 0.08),
            perceptual_roughness: 0.95,
            ..default()
        }),
    });
}

fn reset_lava(
    mut started: MessageReader<WorldRebuilt>,
    mut pools: ResMut<LavaPools>,
    mut spots: ResMut<HotSpots>,
    mut fires: ResMut<LavaFires>,
) {
    if started.read().last().is_some() {
        // Плоскости, капли, дым и огонь — сущности забега, удалены через RunScoped.
        let revision = pools.revision + 1;
        *pools = LavaPools { revision, ..default() };
        *spots = HotSpots::default();
        *fires = LavaFires::default();
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

/// Кто пробил кору в точке: команда ближайшего взрыва этого кадра, в зону кратера которого она
/// попала. Без взрыва (траншея, вскрытый туннель, провал) — ничья: такой прорыв не растит угрозу.
pub fn breacher(blasts: &[Explosion], at: Vec3) -> Team {
    blasts
        .iter()
        .filter(|e| e.position.with_y(0.0).distance(at.with_y(0.0)) <= e.radius * CRATER_EXTENT)
        .min_by(|a, b| a.position.distance_squared(at).total_cmp(&b.position.distance_squared(at)))
        .map_or(Team::Neutral, |e| e.source_team)
}

/// Следит за глубиной в изменённых местах: трещины на дне, дым, прорыв коры.
#[allow(clippy::too_many_arguments)]
fn watch_crust(
    mut commands: Commands,
    mut changes: MessageReader<TerrainChanged>,
    mut blasts: MessageReader<Explosion>,
    mut terrain: ResMut<Terrain>,
    visible: Res<VisibleChunks>,
    mut pools: ResMut<LavaPools>,
    mut spots: ResMut<HotSpots>,
    mut threat: ResMut<Threat>,
    assets: Res<LavaAssets>,
    mut breached: MessageWriter<CrustBreached>,
) {
    let areas: Vec<(Vec3, f32)> = changes.read().map(|c| (c.center, c.radius)).collect();
    let blasts: Vec<Explosion> = blasts.read().copied().collect();
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
        let in_pool =
            pools.hot.contains(&cell) || pools.pools.iter().any(|p| p.origin.as_vec2().distance(cell.as_vec2()) < 6.0);
        if d < CRUST || in_pool {
            continue;
        }
        // Прорыв: извержение и новое озеро.
        info!("Лава: прорыв коры на глубине {d:.1} м");
        pools.pools.push(Pool::new(cell, terrain.original_height(cell.x, cell.y)));
        spots.spots.retain(|s| s.distance(bottom) > 10.0);
        // Взрыв прорыва — от того, кто пробил кору: вражеский снаряд угрозу не растит.
        let team = breacher(&blasts, bottom);
        // Через команды: эта система сама читает взрывы, писать их ей нельзя.
        commands.write_message(Explosion {
            position: bottom,
            radius: 4.0,
            power: 1.8,
            source_team: team,
        });
        breached.write(CrustBreached { position: bottom });
        if team == Team::Player {
            threat.add_destruction(BREACH_THREAT);
        }
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
    visible.mark_dirty(&mut commands, dirty);
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


/// Взрыв в лаве поднимает уровень ближнего озера и выплёскивает капли.
fn blast_in_lava(
    mut commands: Commands,
    mut explosions: MessageReader<Explosion>,
    mut pools: ResMut<LavaPools>,
    assets: Res<LavaAssets>,
) {
    for e in explosions.read() {
        if e.power < SPLASH_POWER {
            continue;
        }
        let Some(surface) = pools.lava_at(e.position.x, e.position.z) else {
            continue;
        };
        let cell = IVec2::new(e.position.x.round() as i32, e.position.z.round() as i32);
        let Some(pool) = pools
            .pools
            .iter_mut()
            .filter(|p| p.age > 0.5)
            .min_by_key(|p| (p.origin - cell).length_squared())
        else {
            continue;
        };
        pool.raise(e.power);
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
                Transform::from_translation(Vec3::new(e.position.x, surface + 0.3, e.position.z)).with_scale(Vec3::splat(0.3)),
            ));
        }
    }
}

/// Шаг течения раз в [`FLOW_STEP`]: источники наполняют ямы, лава течёт по рельефу и застывает.
/// Рельеф под лавой берётся текущий, поэтому новый кратер под лавой заполнится сам. Звенья путей
/// в горячей лаве рвутся.
fn grow_pools(time: Res<Time>, terrain: Res<Terrain>, mut pools: ResMut<LavaPools>, mut track: ResMut<TrackDamage>) {
    if pools.pools.is_empty() {
        return;
    }
    let dt = time.delta_secs();
    for pool in &mut pools.pools {
        pool.age += dt;
    }
    // Долгий кадр не нагоняется пачкой шагов: лава просто течёт медленнее.
    pools.step_clock = (pools.step_clock + dt).min(FLOW_STEP * 2.0);
    if pools.step_clock < FLOW_STEP {
        return;
    }
    pools.step_clock -= FLOW_STEP;
    let result = pools.step(|x, z| terrain.vertex_height(x, z));
    if !result.reshaped {
        return;
    }
    for pool in &pools.pools {
        for near in terrain.lines().near(pool.origin.as_vec2(), FLOW_REACH as f32) {
            let mut s = near.s - FLOW_REACH as f32;
            while s < near.s + FLOW_REACH as f32 {
                let p = terrain.lines().point(near.line, s);
                if pools.hot.contains(&IVec2::new(p.x.round() as i32, p.y.round() as i32)) {
                    track.break_link((near.line, (s / crate::rails::LINK_LENGTH).floor() as i32));
                }
                s += 1.0;
            }
        }
    }
}

/// Строит меш плоскости по ячейкам с запасом в одну ячейку.
pub fn surface_mesh(cells: &HashSet<IVec2>, level: f32) -> Mesh {
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

/// Свет над ямой прорыва.
#[derive(Component)]
struct LavaGlow;

/// Пересобирает сетки лавы после шага, где она сдвинулась, и ставит свет над новыми ямами.
fn draw_pools(
    mut commands: Commands,
    mut pools: ResMut<LavaPools>,
    terrain: Res<Terrain>,
    assets: Res<LavaAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let pools = &mut *pools;
    for pool in &mut pools.pools {
        if pool.light.is_none() {
            pool.light = Some(
                commands
                    .spawn((
                        LavaGlow,
                        RunScoped,
                        PointLight {
                            color: Color::srgb(1.0, 0.45, 0.1),
                            intensity: 3_000_000.0,
                            range: 40.0,
                            shadow_maps_enabled: false,
                            ..default()
                        },
                        Transform::from_xyz(pool.origin.x as f32, pool.target_level + 3.0, pool.origin.y as f32),
                    ))
                    .id(),
            );
        }
    }
    if !pools.dirty {
        return;
    }
    pools.dirty = false;
    let height = |x: i32, z: i32| terrain.vertex_height(x, z);
    // Горячая лава: течение и корки поверх, высоты уже в вершинах.
    if let Some(old) = pools.hot_surface.take() {
        commands.entity(old).try_despawn();
    }
    if let Some(mesh) = lava_flow::layer_mesh(&pools.cells, Layer::Hot, height) {
        let mesh = meshes.add(mesh);
        let root = commands
            .spawn((
                RunScoped,
                NotShadowCaster,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(assets.flow.clone()),
                Transform::default(),
                Visibility::default(),
            ))
            .with_children(|p| {
                p.spawn((
                    NotShadowCaster,
                    Mesh3d(mesh),
                    MeshMaterial3d(assets.crust.clone()),
                    Transform::from_xyz(0.0, 0.04, 0.0),
                ));
            })
            .id();
        pools.hot_surface = Some(root);
    }
    // Застывшая лава.
    if let Some(old) = pools.cooled_surface.take() {
        commands.entity(old).try_despawn();
    }
    if let Some(mesh) = lava_flow::layer_mesh(&pools.cells, Layer::Crust, height) {
        pools.cooled_surface = Some(
            commands
                .spawn((
                    RunScoped,
                    Mesh3d(meshes.add(mesh)),
                    MeshMaterial3d(assets.cooled.clone()),
                    Transform::default(),
                    Visibility::default(),
                ))
                .id(),
        );
    }
}

/// Течение: текстура лавы и корок сдвигается в разные стороны, свет мерцает.
fn animate_lava(
    time: Res<Time>,
    assets: Res<LavaAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    pools: Res<LavaPools>,
    mut lights: Query<&mut PointLight, With<LavaGlow>>,
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
    for mut light in &mut lights {
        light.intensity = 3_000_000.0 * (0.8 + 0.2 * (t * 7.0).sin() * (t * 3.1).cos());
    }
}

/// Лава жжёт: луноход вязнет каждый кадр, а урон приходит тиком [`BURN_TICK`] с тем же уроном
/// в секунду. Враги, машины Службы и жители сгорают, предметы исчезают, деревья и дома у края загораются.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn burn_in_lava(
    mut commands: Commands,
    time: Res<Time>,
    mut pools: ResMut<LavaPools>,
    mut fires: ResMut<LavaFires>,
    terrain: Res<Terrain>,
    mut rover: Query<(Entity, &Transform, &mut RoverMotion, &Health), With<Rover>>,
    // Машины Службы горят в лаве так же, как враги.
    enemies: Query<(Entity, &Transform), Or<(With<Enemy>, With<crate::service::ServiceVehicle>)>>,
    residents: Query<(Entity, &Transform), With<Resident>>,
    pickups: Query<(Entity, &Transform), With<Pickup>>,
    flammable: Query<(Entity, &GlobalTransform, &Health, Option<&Prop>), Or<(With<Prop>, With<House>)>>,
    mut damage: MessageWriter<Damage>,
) {
    let dt = time.delta_secs();
    if pools.pools.is_empty() {
        pools.burn_clock = 0.0;
    } else {
        pools.burn_clock += dt;
    }
    let tick = pools.burn_clock >= BURN_TICK;
    let step = if tick { std::mem::take(&mut pools.burn_clock) } else { 0.0 };
    let pools = &*pools;
    let in_lava = |t: &Transform| {
        pools
            .lava_at(t.translation.x, t.translation.z)
            .is_some_and(|level| terrain.height_at(t.translation.x, t.translation.z) < level)
    };
    let mut burn = |target: Entity, per_second: f32| {
        damage.write(Damage {
            target,
            amount: per_second * step,
            source_team: Team::Neutral,
        });
    };
    for (entity, transform, mut motion, health) in &mut rover {
        if !health.is_dead() && in_lava(transform) {
            motion.speed *= (1.0 - 1.5 * dt).max(0.0);
            if tick {
                burn(entity, ROVER_BURN);
            }
        }
    }
    if tick {
        for (entity, transform) in &enemies {
            if in_lava(transform) {
                burn(entity, ENEMY_BURN);
            }
        }
        for (entity, transform) in &residents {
            if in_lava(transform) {
                burn(entity, RESIDENT_BURN);
            }
        }
        for (entity, transform) in &pickups {
            if in_lava(transform) {
                commands.entity(entity).despawn();
            }
        }
        // Деревья и дома у края загораются; огонь держится, пока лава рядом.
        for (entity, transform, health, prop) in &flammable {
            let p = transform.translation();
            let reach = match prop {
                Some(prop) if prop.kind == PropKind::Tree => TREE_REACH,
                Some(_) => continue,
                None => HOUSE_REACH,
            };
            if !health.is_dead() && pools.near(Vec2::new(p.x, p.z), reach) {
                let (lift, size) = if prop.is_some() { (2.0, 0.9) } else { (3.0, 1.8) };
                fires.ignite(&mut commands, entity, p + Vec3::Y * lift, size, BURN_TICK * 2.0);
            }
        }
    }
    // Горящие получают урон и догорают; сгоревшие и пропавшие гаснут.
    fires.fires.retain(|&target, fire| {
        fire.left -= dt;
        let alive = flammable.get(target).is_ok_and(|(_, _, health, _)| !health.is_dead());
        if fire.left <= 0.0 || !alive {
            commands.entity(fire.flame).try_despawn();
            return false;
        }
        if tick {
            damage.write(Damage {
                target,
                amount: EDGE_BURN * step,
                source_team: Team::Neutral,
            });
        }
        true
    });
}

/// Капли летят по дугам и жгут всё со здоровьем, куда упали; дерево или дом поджигают.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn fly_drops(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut fires: ResMut<LavaFires>,
    mut drops: Query<(Entity, &mut Transform, &mut Drop)>,
    targets: Query<(Entity, &GlobalTransform, &Health, Option<&Prop>, Has<House>), Without<Drop>>,
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
        for (target, t, health, prop, house) in &targets {
            // Дом велик, его точка — в центре: по нему капля попадает в круге на плане.
            let hit = if house {
                t.translation().xz().distance(transform.translation.xz()) < HOUSE_REACH
            } else {
                t.translation().distance(transform.translation) < DROP_RADIUS
            };
            if health.is_dead() || !hit {
                continue;
            }
            damage.write(Damage {
                target,
                amount: DROP_DAMAGE,
                source_team: Team::Neutral,
            });
            let tree = prop.is_some_and(|p| p.kind == PropKind::Tree);
            if tree || house {
                let (lift, size) = if tree { (2.0, 0.9) } else { (3.0, 1.8) };
                fires.ignite(&mut commands, target, t.translation() + Vec3::Y * lift, size, DROP_FIRE);
            }
        }
        commands.entity(entity).despawn();
    }
}

/// Над глубокими кратерами поднимается дым и изредка летят искры.
fn smoke_hot_spots(
    mut commands: Commands,
    time: Res<Time>,
    mut spots: ResMut<HotSpots>,
    assets: Res<LavaAssets>,
    mut plumes: Local<Vec<(Vec3, Entity)>>,
) {
    // Столб мягкого дыма модуля огня над каждой горячей точкой; ушедшие точки гаснут.
    plumes.retain(|(p, e)| {
        let keep = spots.spots.contains(p);
        if !keep {
            commands.entity(*e).try_despawn();
        }
        keep
    });
    for spot in &spots.spots {
        if !plumes.iter().any(|(p, _)| p == spot) {
            let e = commands
                .spawn((crate::fire::SmokePlume, RunScoped, Transform::from_translation(*spot + Vec3::Y * 0.3)))
                .id();
            plumes.push((*spot, e));
        }
    }
    spots.timer -= time.delta_secs();
    if spots.timer > 0.0 || spots.spots.is_empty() {
        return;
    }
    spots.timer = 0.4;
    let t = time.elapsed_secs();
    for (i, spot) in spots.spots.iter().enumerate() {
        let a = t * 3.7 + i as f32 * 1.3;
        if (a * 10.0).sin() > 0.6 {
            commands.spawn((
                Smoke {
                    velocity: Vec3::new(a.sin() * 2.0, 7.0, a.cos() * 2.0),
                    age: 0.0,
                    life: 0.8,
                    grow: 0.0,
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
        t.scale *= 1.0 + s.grow * dt;
    }
}

/// Случайная горячая ячейка ямы прорыва толще [`BUBBLE_DEPTH`]: несколько попыток. `None`, если не попали.
fn random_cell(cells: &Cells, origin: IVec2, rng: &mut GameRng) -> Option<(IVec2, f32)> {
    let r = Overhang::RIM as i32;
    (0..6).find_map(|_| {
        let c = origin + IVec2::new(rng.below((2 * r + 1) as usize) as i32 - r, rng.below((2 * r + 1) as usize) as i32 - r);
        cells.get(&c).filter(|cell| cell.is_hot() && cell.hot > BUBBLE_DEPTH).map(|cell| (c, cell.surface()))
    })
}

/// Озёра у фокуса камеры бурлят: лопаются пузыри, поднимается дым, изредка летят капли.
/// Сущностей на озеро немного: около трёх пузырей и пяти клубов дыма одновременно.
fn bubble_pools(
    mut commands: Commands,
    time: Res<Time>,
    focus: Res<Focus>,
    mut pools: ResMut<LavaPools>,
    mut rng: ResMut<GameRng>,
    assets: Res<LavaAssets>,
) {
    let dt = time.delta_secs();
    let focus = Vec2::new(focus.0.x, focus.0.z);
    let LavaPools { pools, cells, .. } = &mut *pools;
    for pool in pools.iter_mut() {
        if pool.age < FILL_TIME * 0.5 || pool.origin.as_vec2().distance(focus) > BUBBLE_RANGE + Overhang::RIM {
            continue;
        }
        pool.bubble -= dt;
        pool.puff -= dt;
        pool.spit -= dt;
        if pool.bubble <= 0.0 {
            pool.bubble = BUBBLE_EVERY;
            if let Some((c, surface)) = random_cell(cells, pool.origin, &mut rng) {
                commands.spawn((
                    Smoke {
                        velocity: Vec3::Y * 0.3,
                        age: 0.0,
                        life: rng.range(0.35, 0.6),
                        grow: 2.5,
                    },
                    RunScoped,
                    NotShadowCaster,
                    Mesh3d(assets.ball.clone()),
                    MeshMaterial3d(assets.spark.clone()),
                    Transform::from_xyz(c.x as f32, surface + 0.05, c.y as f32).with_scale(Vec3::new(0.25, 0.12, 0.25)),
                ));
            }
        }
        if pool.puff <= 0.0 {
            pool.puff = PUFF_EVERY;
            if let Some((c, surface)) = random_cell(cells, pool.origin, &mut rng) {
                commands.spawn((
                    Smoke {
                        velocity: Vec3::new(0.5, rng.range(1.2, 2.0), -0.2),
                        age: 0.0,
                        life: 3.0,
                        grow: 0.5,
                    },
                    RunScoped,
                    NotShadowCaster,
                    Mesh3d(assets.ball.clone()),
                    MeshMaterial3d(assets.smoke.clone()),
                    Transform::from_xyz(c.x as f32, surface + 0.6, c.y as f32).with_scale(Vec3::splat(0.8)),
                ));
            }
        }
        if pool.spit <= 0.0 {
            pool.spit = rng.range(SPIT_EVERY.0, SPIT_EVERY.1);
            if let Some((c, surface)) = random_cell(cells, pool.origin, &mut rng) {
                for i in 0..3 {
                    let a = rng.range(0.0, std::f32::consts::TAU);
                    let side = rng.range(1.5, 4.0);
                    commands.spawn((
                        Drop {
                            velocity: Vec3::new(a.cos() * side, 9.0 + i as f32 * 1.5, a.sin() * side),
                        },
                        RunScoped,
                        NotShadowCaster,
                        Mesh3d(assets.ball.clone()),
                        MeshMaterial3d(assets.drop.clone()),
                        Transform::from_xyz(c.x as f32, surface + 0.3, c.y as f32).with_scale(Vec3::splat(0.25)),
                    ));
                }
            }
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
    fn после_шести_выстрелов_мортиры_глубина_больше_коры() {
        let mut terrain = Terrain::new(7);
        let p = Vec3::new(40.0, 0.0, 40.0);
        // Как в craters.rs: радиус 7, глубина 0,5·R·мощность 1,4, вал 0,12·R·1,4.
        for _ in 0..5 {
            terrain.apply_crater(p, 7.0, 4.9, 1.176);
        }
        assert!(depth(&terrain, 40, 40) < CRUST, "пять выстрелов кору не пробивают");
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

    /// Наполненное озеро: целевой уровень 1 м, исходная поверхность 5 м.
    fn filled_pool() -> Pool {
        let mut pool = Pool::new(IVec2::ZERO, 1.0 + POOL_BELOW);
        pool.age = 10.0;
        pool
    }

    /// Лава с горячими ячейками `hot` толщиной 1 м на ровном рельефе и озером `filled_pool`.
    fn lava_with(hot: &[IVec2]) -> LavaPools {
        let mut pools = LavaPools {
            pools: vec![filled_pool()],
            ..default()
        };
        for c in hot {
            pools.cells.insert(*c, LavaCell { hot: 1.0, ..default() });
        }
        pools.refresh_hot();
        pools
    }

    #[test]
    fn слабый_взрыв_уровень_не_поднимает_и_есть_потолок() {
        let mut pool = filled_pool();
        let start = pool.target_level;
        pool.raise(0.25);
        assert_eq!(pool.target_level, start, "гибель врага уровень не поднимает");
        for _ in 0..20 {
            pool.raise(1.4);
        }
        assert!(pool.target_level <= pool.max_level + 1e-5);
        assert!(pool.target_level > start);
        assert!(pool.max_level < 1.0 + POOL_BELOW, "озеро не поднимается до исходной поверхности");
    }

    #[test]
    fn прорыв_от_вражеского_снаряда_не_от_игрока() {
        let at = Vec3::new(10.0, -30.0, 10.0);
        let blast = |team, x| Explosion {
            position: Vec3::new(x, -28.0, 10.0),
            radius: 5.0,
            power: 1.4,
            source_team: team,
        };
        assert_eq!(breacher(&[blast(Team::Enemy, 10.0)], at), Team::Enemy);
        assert_eq!(breacher(&[blast(Team::Player, 40.0), blast(Team::Enemy, 11.0)], at), Team::Enemy);
        assert_eq!(breacher(&[blast(Team::Player, 9.0)], at), Team::Player);
        assert_eq!(breacher(&[], at), Team::Neutral, "траншея или туннель — ничей прорыв");
    }

    #[test]
    fn лава_рядом_по_ячейкам() {
        let pools = lava_with(&[IVec2::ZERO, IVec2::X]);
        assert!(pools.near(Vec2::new(3.0, 0.0), 2.0));
        assert!(!pools.near(Vec2::new(4.0, 0.0), 2.0));
        assert!(!pools.near(Vec2::new(50.0, 0.0), 2.0));
    }

    #[test]
    fn застывшая_лава_не_жжёт() {
        let mut pools = lava_with(&[IVec2::ZERO]);
        pools.cells.insert(IVec2::X, LavaCell { crust: 0.5, ..default() });
        pools.refresh_hot();
        assert!(is_lava(&pools, Vec3::ZERO), "горячая");
        assert!(!is_lava(&pools, Vec3::X), "корка");
        assert_eq!(pools.lava_at(0.0, 0.0), Some(1.0), "высота поверхности горячей лавы");
    }

    /// Шесть выстрелов мортиры в точку `at` рельефа с сидом `seed` (как в craters.rs: радиус 7,
    /// мощность 1,4) и взрыв прорыва на дне. Возвращает рельеф и место прорыва — самую глубокую вершину.
    fn breach(seed: u32, at: IVec2) -> (Terrain, IVec2) {
        let mut terrain = Terrain::new(seed);
        let p = Vec3::new(at.x as f32, 0.0, at.y as f32);
        for _ in 0..6 {
            terrain.apply_crater(p, 7.0, 4.9, 1.176);
        }
        // Дно — самая глубокая вершина, как в watch_crust.
        let mut deepest = (0.0f32, at);
        for vz in at.y - 8..=at.y + 8 {
            for vx in at.x - 8..=at.x + 8 {
                let d = depth(&terrain, vx, vz);
                if d > deepest.0 {
                    deepest = (d, IVec2::new(vx, vz));
                }
            }
        }
        assert!(deepest.0 > CRUST, "шесть выстрелов пробивают кору");
        terrain.apply_crater(p, 4.0, 3.6, 0.864);
        (terrain, deepest.1)
    }

    /// Шахта прорыва на склоне горы (сид 3, точка (20, −110), уклон около 0,73) со стенкой,
    /// прорубленной вниз по склону двумя выстрелами: та же расстановка, что в сцене проверки `lava_slope`.
    pub(crate) fn pit_on_slope() -> (Terrain, IVec2) {
        let at = IVec2::new(20, -110);
        let (mut terrain, cell) = breach(3, at);
        let h = |dx: i32, dz: i32| terrain.original_height(at.x + dx, at.y + dz);
        let down = -Vec2::new(h(5, 0) - h(-5, 0), h(0, 5) - h(0, -5)).normalize();
        let p = Vec3::new(at.x as f32, 0.0, at.y as f32);
        for _ in 0..2 {
            terrain.apply_crater(p + Vec3::new(down.x, 0.0, down.y) * 7.0, 7.0, 4.9, 1.176);
        }
        (terrain, cell)
    }

    /// Лава из прорыва в `cell` за `seconds` секунд игры.
    fn simulate(terrain: &Terrain, cell: IVec2, seconds: f32) -> LavaPools {
        let mut pools = LavaPools::default();
        pools.pools.push(Pool::new(cell, terrain.original_height(cell.x, cell.y)));
        for _ in 0..(seconds / FLOW_STEP) as usize {
            pools.step(|x, z| terrain.vertex_height(x, z));
        }
        pools
    }

    #[test]
    fn лава_из_ямы_на_склоне_не_висит_плитой() {
        let (terrain, cell) = pit_on_slope();
        let pools = simulate(&terrain, cell, 60.0);
        let o = pools.overhangs(&terrain)[0];
        println!("прорыв {cell}; {o:?}, вылилось {:.0} м³", pools.pools[0].spilled);
        assert!(o.outside > 0, "лава выходит из ямы через прорубленную стенку: {o:?}");
        assert!(o.reach > Overhang::RIM + 10.0, "и стекает вниз по склону: {o:?}");
        // На нетронутом склоне поток лежит слоем по рельефу (в пологих впадинах — до полутора метров);
        // горизонтальная плита стояла над ним на 5,5 м. Кратер у пролома лава заполняет — его не считаем.
        assert!(o.intact > 0 && o.intact_thickness <= 2.0, "лава на склоне слоем, а не плитой: {o:?}");
    }

    #[test]
    fn шахта_без_пролома_держит_лаву_в_яме() {
        let (terrain, cell) = breach(3, IVec2::ZERO);
        let pools = simulate(&terrain, cell, 30.0);
        let o = pools.overhangs(&terrain)[0];
        assert_eq!(o.outside, 0, "лава в яме: {o:?}");
        let (surface, target, ..) = pools.summary()[0];
        assert!((surface - target).abs() < 0.3, "яма наполнена до уровня: {surface} при цели {target}");
    }

    #[test]
    fn взрыв_поднимает_уровень() {
        let mut app = App::new();
        app.add_message::<Explosion>()
            .insert_resource(lava_with(&[IVec2::ZERO]))
            .insert_resource(LavaAssets {
                ball: Handle::default(),
                drop: Handle::default(),
                smoke: Handle::default(),
                spark: Handle::default(),
                flow: Handle::default(),
                crust: Handle::default(),
                cooled: Handle::default(),
            })
            .add_systems(Update, blast_in_lava);
        app.world_mut().write_message(Explosion {
            position: Vec3::ZERO,
            radius: 5.0,
            power: 1.4,
            source_team: Team::Player,
        });
        let before = app.world().resource::<LavaPools>().pools[0].target_level;
        app.update();
        assert!(app.world().resource::<LavaPools>().pools[0].target_level > before);
    }
}
