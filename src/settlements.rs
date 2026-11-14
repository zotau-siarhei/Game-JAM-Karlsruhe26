//! Посёлки: где на бесконечной планете стоят дома и в каком они состоянии.
//!
//! Мир делится на регионы [`REGION_SIZE`]×[`REGION_SIZE`] м. Хеш от сида и координат региона
//! решает, есть ли в нём посёлок, где его центр и какие в нём дома. Регион генерируется при
//! первом обращении и живёт в ресурсе [`Settlements`] до конца забега, поэтому разрушенный
//! дом после отъезда и возвращения остаётся разрушенным. Регион старта получает посёлок всегда.
//!
//! Сущности домов создаёт модуль `buildings`; он же пишет изменения обратно в [`HouseState`].

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, TAU};

use bevy::prelude::*;

use crate::run::RunStarted;
use crate::terrain::Terrain;

/// Сторона региона посёлков, м. Регион `(0, 0)` стоит центром в начале координат.
pub const REGION_SIZE: f32 = 160.0;

/// Доля регионов, в которых есть посёлок.
const SETTLEMENT_CHANCE: f32 = 0.6;

/// Сколько домов в обычном посёлке: от и до включительно.
const HOUSES_PER_SETTLEMENT: (u32, u32) = (4, 9);

/// Сколько домов в посёлке у старта.
const START_HOUSES: (u32, u32) = (5, 8);

/// Меньше домов у старта не бывает: иначе посёлок ищется в другом направлении.
const START_MIN_HOUSES: usize = 3;

/// Расстояние от точки старта до центра стартового посёлка, м.
const START_SETTLEMENT_DISTANCE: f32 = 35.0;

/// Ни один дом не ближе этого к точке старта, м: луноход не появляется в посёлке.
pub const START_CLEARANCE: f32 = 12.0;

/// Наименьший зазор между стенами соседних домов, м: шире любой машины.
pub const MIN_HOUSE_GAP: f32 = 7.0;

/// Наибольший уклон под домом, рад.
const MAX_SLOPE: f32 = 15.0 * std::f32::consts::PI / 180.0;

/// Радиус разброса домов вокруг центра посёлка, м: база и прибавка на дом.
const SPREAD_BASE: f32 = 12.0;
const SPREAD_PER_HOUSE: f32 = 2.2;
const SPREAD_MAX: f32 = 28.0;

/// Отступ центра посёлка от края региона, м: посёлки соседних регионов не сливаются.
const REGION_MARGIN: f32 = 50.0;

/// Сколько раз пробовать поставить дом, прежде чем сдаться.
const PLACEMENT_ATTEMPTS: u32 = 150;

/// Разброс поворота дома от сетки посёлка, рад.
const YAW_JITTER: f32 = 0.18;

/// Число цветов стен и крыш в палитре `buildings`.
pub const WALL_COLORS: usize = 6;
pub const ROOF_COLORS: usize = 5;

/// Подключает хранилище посёлков и его сброс при новом забеге.
pub struct SettlementsPlugin;

impl Plugin for SettlementsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Settlements>()
            .add_systems(Update, reset_settlements);
    }
}

/// Тип дома.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HouseKind {
    /// Маленький домик с двускатной крышей.
    Hut,
    /// Двухэтажный дом.
    TwoStory,
    /// Длинный барак с плоской крышей.
    Barrack,
}

impl HouseKind {
    /// Половина ширины (X) и половина длины (Z) дома в плане, м.
    pub fn half_extents(self) -> Vec2 {
        match self {
            HouseKind::Hut => Vec2::new(2.5, 2.5),
            HouseKind::TwoStory => Vec2::new(3.0, 3.5),
            HouseKind::Barrack => Vec2::new(2.5, 6.0),
        }
    }

    /// Высота стен над землёй, м.
    pub fn wall_height(self) -> f32 {
        match self {
            HouseKind::Hut => 3.0,
            HouseKind::TwoStory => 6.0,
            HouseKind::Barrack => 3.5,
        }
    }

    /// Прочность целого дома.
    pub fn max_health(self) -> f32 {
        match self {
            HouseKind::Barrack => 220.0,
            _ => 150.0,
        }
    }
}

/// Что с домом сейчас.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HouseStatus {
    Intact,
    /// Провалился в кратер: положение и крен дома.
    Sunk(Transform),
    Destroyed,
}

/// Состояние одного дома на весь забег.
#[derive(Debug, Clone, PartialEq)]
pub struct HouseState {
    /// Центр дома в плане: `x` и `z` мира.
    pub position: Vec2,
    /// Поворот вокруг вертикали, рад.
    pub yaw: f32,
    pub kind: HouseKind,
    /// Индекс цвета стен в палитре, от 0 до [`WALL_COLORS`].
    pub wall_color: usize,
    /// Индекс цвета крыши в палитре, от 0 до [`ROOF_COLORS`].
    pub roof_color: usize,
    pub health: f32,
    pub status: HouseStatus,
}

impl HouseState {
    /// Прямоугольник дома в плане.
    pub fn footprint(&self) -> Footprint {
        Footprint {
            center: self.position,
            yaw: self.yaw,
            half: self.kind.half_extents(),
        }
    }
}

/// Ориентированный прямоугольник в плане (`x`, `z` мира).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Footprint {
    pub center: Vec2,
    /// Поворот вокруг вертикали, рад, как у `Quat::from_rotation_y`.
    pub yaw: f32,
    /// Половина размера по локальным X и Z.
    pub half: Vec2,
}

impl Footprint {
    /// Точка мира в системе прямоугольника.
    pub fn to_local(&self, p: Vec2) -> Vec2 {
        let d = p - self.center;
        let v = Quat::from_rotation_y(-self.yaw) * Vec3::new(d.x, 0.0, d.y);
        Vec2::new(v.x, v.z)
    }

    /// Точка системы прямоугольника в мире.
    pub fn to_world(&self, local: Vec2) -> Vec2 {
        let v = Quat::from_rotation_y(self.yaw) * Vec3::new(local.x, 0.0, local.y);
        self.center + Vec2::new(v.x, v.z)
    }

    /// Ближайшая к `p` точка прямоугольника. Для точки внутри — она сама.
    pub fn closest_point(&self, p: Vec2) -> Vec2 {
        let local = self.to_local(p).clamp(-self.half, self.half);
        self.to_world(local)
    }

    /// Расстояние от точки до прямоугольника, 0 внутри.
    pub fn distance(&self, p: Vec2) -> f32 {
        (p - self.closest_point(p)).length()
    }

    /// Четыре угла в мире.
    pub fn corners(&self) -> [Vec2; 4] {
        let h = self.half;
        [
            self.to_world(Vec2::new(-h.x, -h.y)),
            self.to_world(Vec2::new(h.x, -h.y)),
            self.to_world(Vec2::new(h.x, h.y)),
            self.to_world(Vec2::new(-h.x, h.y)),
        ]
    }

    /// Зазор между двумя прямоугольниками, 0 при пересечении.
    ///
    /// Ближайшая пара точек двух непересекающихся выпуклых многоугольников всегда включает
    /// вершину одного из них, поэтому хватает расстояний от углов до другого прямоугольника.
    pub fn gap(&self, other: &Footprint) -> f32 {
        if self.distance(other.center) == 0.0 || other.distance(self.center) == 0.0 {
            return 0.0;
        }
        let a = self.corners().map(|c| other.distance(c));
        let b = other.corners().map(|c| self.distance(c));
        a.into_iter().chain(b).fold(f32::MAX, f32::min)
    }
}

/// Дома одного региона.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RegionState {
    pub houses: Vec<HouseState>,
}

/// Адрес дома в хранилище.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HouseId {
    pub region: IVec2,
    pub index: usize,
}

/// Посёлки забега: все регионы, к которым обращались.
#[derive(Resource, Default)]
pub struct Settlements {
    /// Сид текущего забега. `None` — забег ещё не объявлен.
    seed: Option<u32>,
    regions: HashMap<IVec2, RegionState>,
}

impl Settlements {
    /// Пустое хранилище для сида.
    pub fn new(seed: u32) -> Self {
        Self {
            seed: Some(seed),
            regions: HashMap::new(),
        }
    }

    /// Сид забега, если он уже объявлен.
    pub fn seed(&self) -> Option<u32> {
        self.seed
    }

    /// Регион, генерирует его при первом обращении. Без сида регион пустой.
    pub fn region(&mut self, coord: IVec2, terrain: &Terrain) -> &mut RegionState {
        let seed = self.seed;
        self.regions.entry(coord).or_insert_with(|| match seed {
            Some(seed) => generate_region(seed, coord, terrain),
            None => RegionState::default(),
        })
    }

    /// Дом по адресу, если его регион уже сгенерирован.
    pub fn house(&self, id: HouseId) -> Option<&HouseState> {
        self.regions.get(&id.region)?.houses.get(id.index)
    }

    pub fn house_mut(&mut self, id: HouseId) -> Option<&mut HouseState> {
        self.regions.get_mut(&id.region)?.houses.get_mut(id.index)
    }
}

/// Регион, в который попадает точка плана.
pub fn region_of(p: Vec2) -> IVec2 {
    ((p + Vec2::splat(REGION_SIZE * 0.5)) / REGION_SIZE)
        .floor()
        .as_ivec2()
}

/// Угол региона с наименьшими координатами.
fn region_min(coord: IVec2) -> Vec2 {
    coord.as_vec2() * REGION_SIZE - Vec2::splat(REGION_SIZE * 0.5)
}

/// Генерирует дома региона. Один сид и одни координаты дают одни и те же дома.
pub fn generate_region(seed: u32, coord: IVec2, terrain: &Terrain) -> RegionState {
    let mut rng = Rng::for_region(seed, coord);

    if coord == IVec2::ZERO {
        // Посёлок у старта: если склоны не пустили дома, пробуем другое направление.
        let mut best = RegionState::default();
        let start_angle = rng.range(0.0, TAU);
        for attempt in 0..8 {
            let angle = start_angle + attempt as f32 * TAU / 8.0;
            let center = Vec2::from_angle(angle) * START_SETTLEMENT_DISTANCE;
            let count = rng.range_u32(START_HOUSES.0, START_HOUSES.1);
            let region = place_houses(&mut rng, center, count, true, terrain);
            if region.houses.len() >= START_MIN_HOUSES {
                return region;
            }
            if region.houses.len() > best.houses.len() {
                best = region;
            }
        }
        return best;
    }

    if rng.next_f32() >= SETTLEMENT_CHANCE {
        return RegionState::default();
    }
    let min = region_min(coord) + Vec2::splat(REGION_MARGIN);
    let span = REGION_SIZE - 2.0 * REGION_MARGIN;
    let center = min + Vec2::new(rng.range(0.0, span), rng.range(0.0, span));
    let count = rng.range_u32(HOUSES_PER_SETTLEMENT.0, HOUSES_PER_SETTLEMENT.1);
    place_houses(&mut rng, center, count, false, terrain)
}

/// Ставит до `count` домов вокруг центра случайным перебором мест.
fn place_houses(
    rng: &mut Rng,
    center: Vec2,
    count: u32,
    near_start: bool,
    terrain: &Terrain,
) -> RegionState {
    let spread = (SPREAD_BASE + SPREAD_PER_HOUSE * count as f32).min(SPREAD_MAX);
    // Дома посёлка выровнены по общей сетке, так посёлок выглядит улицей, а не россыпью.
    let grid_yaw = rng.range(0.0, FRAC_PI_2);
    let mut houses: Vec<HouseState> = Vec::new();

    for _ in 0..PLACEMENT_ATTEMPTS {
        if houses.len() >= count as usize {
            break;
        }
        let offset = Vec2::from_angle(rng.range(0.0, TAU)) * spread * rng.next_f32().sqrt();
        let kind = match rng.next_f32() {
            r if r < 0.45 => HouseKind::Hut,
            r if r < 0.8 => HouseKind::TwoStory,
            _ => HouseKind::Barrack,
        };
        let yaw = grid_yaw
            + rng.range_u32(0, 3) as f32 * FRAC_PI_2
            + rng.range(-YAW_JITTER, YAW_JITTER);
        let house = HouseState {
            position: center + offset,
            yaw,
            kind,
            wall_color: rng.range_u32(0, WALL_COLORS as u32 - 1) as usize,
            roof_color: rng.range_u32(0, ROOF_COLORS as u32 - 1) as usize,
            health: kind.max_health(),
            status: HouseStatus::Intact,
        };
        let footprint = house.footprint();
        if near_start && footprint.distance(Vec2::ZERO) < START_CLEARANCE {
            continue;
        }
        if houses
            .iter()
            .any(|other| other.footprint().gap(&footprint) < MIN_HOUSE_GAP)
        {
            continue;
        }
        if !is_flat_enough(&footprint, terrain) {
            continue;
        }
        houses.push(house);
    }
    RegionState { houses }
}

/// Уклон под прямоугольником не круче [`MAX_SLOPE`]: и в каждой точке, и между углами.
fn is_flat_enough(footprint: &Footprint, terrain: &Terrain) -> bool {
    let min_up = MAX_SLOPE.cos();
    let points = footprint.corners();
    let steep = points
        .iter()
        .chain(std::iter::once(&footprint.center))
        .any(|p| terrain.normal_at(p.x, p.y).y < min_up);
    if steep {
        return false;
    }
    let heights = points.map(|p| terrain.height_at(p.x, p.y));
    let low = heights.iter().copied().fold(f32::MAX, f32::min);
    let high = heights.iter().copied().fold(f32::MIN, f32::max);
    let diagonal = footprint.half.length() * 2.0;
    (high - low) <= diagonal * MAX_SLOPE.tan()
}

/// Новый забег: посёлки прошлого выбрасываются, следующие строятся от нового сида.
pub fn reset_settlements(mut started: MessageReader<RunStarted>, mut settlements: ResMut<Settlements>) {
    if let Some(run) = started.read().last() {
        *settlements = Settlements::new(run.seed);
    }
}

/// Простой детерминированный генератор SplitMix64: одинаковый на всех платформах.
struct Rng(u64);

impl Rng {
    fn for_region(seed: u32, coord: IVec2) -> Self {
        let mut state = (seed as u64) << 32 ^ 0x5E77_1E05;
        state ^= (coord.x as u32 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        state ^= (coord.y as u32 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        let mut rng = Self(state);
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

    /// Число от 0 включительно до 1 не включительно.
    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next_f32()
    }

    /// Целое от `low` до `high` включительно.
    fn range_u32(&mut self, low: u32, high: u32) -> u32 {
        low + (self.next_u64() % (high - low + 1) as u64) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEEDS: [u32; 6] = [1, 7, 42, 1234, 99_999, 3_000_000_000];

    /// Регионы вокруг старта, которые проверяют тесты.
    fn sample_regions() -> impl Iterator<Item = IVec2> {
        (-2..=2).flat_map(|z| (-2..=2).map(move |x| IVec2::new(x, z)))
    }

    #[test]
    fn same_seed_gives_same_houses() {
        for seed in SEEDS {
            let a = Terrain::new(seed);
            let b = Terrain::new(seed);
            for coord in sample_regions() {
                assert_eq!(
                    generate_region(seed, coord, &a),
                    generate_region(seed, coord, &b),
                    "сид {seed}, регион {coord}"
                );
            }
        }
    }

    #[test]
    fn different_seeds_give_different_houses() {
        let a = generate_region(1, IVec2::ZERO, &Terrain::new(1));
        let b = generate_region(2, IVec2::ZERO, &Terrain::new(2));
        assert_ne!(a, b);
    }

    #[test]
    fn houses_are_at_least_7_m_apart() {
        for seed in SEEDS {
            let terrain = Terrain::new(seed);
            for coord in sample_regions() {
                let houses = generate_region(seed, coord, &terrain).houses;
                for (i, a) in houses.iter().enumerate() {
                    for b in &houses[i + 1..] {
                        let gap = a.footprint().gap(&b.footprint());
                        assert!(gap >= MIN_HOUSE_GAP, "сид {seed}, регион {coord}: зазор {gap}");
                    }
                }
            }
        }
    }

    #[test]
    fn start_has_settlement_and_free_space() {
        for seed in SEEDS {
            let houses = generate_region(seed, IVec2::ZERO, &Terrain::new(seed)).houses;
            assert!(houses.len() >= START_MIN_HOUSES, "сид {seed}: домов {}", houses.len());
            for house in &houses {
                let d = house.footprint().distance(Vec2::ZERO);
                assert!(d >= START_CLEARANCE, "сид {seed}: дом в {d} м от старта");
            }
        }
    }

    #[test]
    fn settlements_appear_across_the_world() {
        // Регионы 160 м: на минуту езды приходится несколько регионов, и посёлки в них есть.
        let terrain = Terrain::new(5);
        let with_houses = (1..=10)
            .filter(|&x| !generate_region(5, IVec2::new(x, 0), &terrain).houses.is_empty())
            .count();
        assert!(with_houses >= 3, "посёлков в 10 регионах: {with_houses}");
    }

    #[test]
    fn regions_are_centered_on_origin() {
        assert_eq!(region_of(Vec2::ZERO), IVec2::ZERO);
        assert_eq!(region_of(Vec2::new(79.0, -79.0)), IVec2::ZERO);
        assert_eq!(region_of(Vec2::new(81.0, -81.0)), IVec2::new(1, -1));
    }

    #[test]
    fn footprint_distance_and_gap() {
        let a = Footprint {
            center: Vec2::ZERO,
            yaw: FRAC_PI_2,
            half: Vec2::new(1.0, 3.0),
        };
        // Повёрнутый на 90° прямоугольник вытянут по X.
        assert!(a.distance(Vec2::new(2.9, 0.0)) < 1e-4);
        assert!((a.distance(Vec2::new(0.0, 2.0)) - 1.0).abs() < 1e-4);
        let b = Footprint {
            center: Vec2::new(10.0, 0.0),
            yaw: 0.0,
            half: Vec2::new(1.0, 1.0),
        };
        assert!((a.gap(&b) - 6.0).abs() < 1e-4);
        assert_eq!(a.gap(&a), 0.0);
    }
}
