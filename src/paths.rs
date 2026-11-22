//! Тропинки в посёлках: натоптанная земля от дверей домов, связанная в одну сеть.
//!
//! Два дома соединены тропинкой, если между ними нет третьего, который ближе к обоим (граф
//! относительных соседей): все дома связаны, есть петли, длинных диагоналей нет. Тропинка
//! начинается у двери (фасад +Z дома; если там закрыто — другая сторона), выходит через калитку
//! забора и идёт по сетке 1 м в обход домов, заборов и путей, предпочитая пологие места и уже
//! натоптанные клетки — так тропинки сливаются. Раскладка зависит только от сида и домов региона.
//! Земля красится в цвета вершин рельефа при появлении участка; трава, деревья, кусты и камни на
//! тропинках не растут. Спокойные жители гуляют по тропинкам: [`VillagePaths::stroll`].

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use bevy::prelude::*;

use crate::frame::GameSet;
use crate::props::{PropKind, PropStatus, Props};
use crate::run::WorldRebuilt;
use crate::settlements::{HouseState, Settlements, region_of};
use crate::sound_list::Dice;
use crate::terrain::{self, CHUNK_SIZE, ChunkMesh, Terrain, VisibleChunks, chunk_origin};

/// Ширина тропинки, м.
const TRAIL_WIDTH: f32 = 1.6;
/// Насколько тропинка обходит дом, м.
const HOUSE_CLEAR: f32 = 1.0;
/// Насколько тропинка обходит забор, м.
const FENCE_CLEAR: f32 = 0.7;
/// Где кончается отвод от двери: за калиткой, м от стены.
const PORCH: f32 = 4.5;
/// Поле сетки вокруг домов, м.
const GRID_MARGIN: f32 = 12.0;
/// Цена подъёма на метр высоты на метр пути: тропинки идут пологими местами.
const SLOPE_COST: f32 = 6.0;
/// Доля цены по уже натоптанной клетке: тропинки сливаются.
const TRODDEN: f32 = 0.45;
/// Сколько от края тропинки до ствола дерева и до куста или камня, м: крона не закрывает тропинку.
const TREE_CLEAR: f32 = 2.5;
const PROP_CLEAR: f32 = 0.6;
/// Цвет натоптанной земли.
const PATH_COLOR: Color = Color::srgb(0.66, 0.55, 0.38);
/// Шаг точек графа прогулки и расстояние, на котором точки разных тропинок связываются, м.
const NODE_STEP: f32 = 2.0;
const JOIN: f32 = 1.5;
/// Прогулка: начало не дальше, м; цель — от и до, м от начала.
const STROLL_REACH: f32 = 15.0;
const STROLL_MIN: f32 = 6.0;
const STROLL_MAX: f32 = 40.0;

pub struct PathsPlugin;

impl Plugin for PathsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VillagePaths>().add_systems(
            Update,
            (
                reset_paths.in_set(GameSet::Reset),
                // После луж, до объектов и пересборки мешей: тропинка попадает в меш сразу, а
                // деревья на ней убираются до своего появления.
                paint_paths
                    .after(terrain::stream_chunks)
                    .after(crate::puddles::place_puddles)
                    .before(terrain::rebuild_dirty_chunks)
                    .in_set(GameSet::Streaming),
            ),
        );
    }
}

/// Одна тропинка: ломаная в плане.
#[derive(Debug, Clone)]
pub struct Trail {
    pub points: Vec<Vec2>,
    /// Рамка ломаной с запасом: быстрый отсев.
    min: Vec2,
    max: Vec2,
}

impl Trail {
    fn new(points: Vec<Vec2>) -> Self {
        let pad = Vec2::splat(TRAIL_WIDTH + TREE_CLEAR);
        let min = points.iter().fold(Vec2::splat(f32::MAX), |m, p| m.min(*p)) - pad;
        let max = points.iter().fold(Vec2::splat(f32::MIN), |m, p| m.max(*p)) + pad;
        Self { points, min, max }
    }

    /// Расстояние от точки до оси тропинки; `None` — точка далеко за рамкой.
    fn distance(&self, p: Vec2) -> Option<f32> {
        if p.cmplt(self.min).any() || p.cmpgt(self.max).any() {
            return None;
        }
        if self.points.len() == 1 {
            return Some(p.distance(self.points[0]));
        }
        self.points
            .windows(2)
            .map(|w| segment_distance(p, w[0], w[1]))
            .min_by(f32::total_cmp)
    }
}

fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Тропинки одного региона и граф прогулки по ним.
#[derive(Debug, Default)]
struct Region {
    trails: Vec<Trail>,
    walk: WalkGraph,
}

/// Тропинки посёлков забега.
#[derive(Resource, Default)]
pub struct VillagePaths {
    regions: HashMap<IVec2, Region>,
    /// Участки, земля которых уже покрашена: краска не ложится дважды.
    painted: HashSet<IVec2>,
}

impl VillagePaths {
    /// Точка на тропинке: тут не растёт трава. Только уже проложенные регионы.
    pub fn at(&self, p: Vec2) -> bool {
        let center = region_of(p);
        (-1..=1).any(|dz| {
            (-1..=1).any(|dx| {
                self.regions
                    .get(&(center + IVec2::new(dx, dz)))
                    .is_some_and(|region| {
                        region
                            .trails
                            .iter()
                            .any(|t| t.distance(p).is_some_and(|d| d < TRAIL_WIDTH * 0.5 + 0.2))
                    })
            })
        })
    }

    /// Маршрут прогулки по тропинкам посёлка дома `home` от точки `from`: от ближайшей точки сети
    /// к случайной в [`STROLL_MIN`]–[`STROLL_MAX`] м. `None` — тропинок рядом нет.
    pub fn stroll(&self, home: Vec2, from: Vec2, dice: &mut Dice) -> Option<Vec<Vec2>> {
        self.regions.get(&region_of(home))?.walk.stroll(from, dice)
    }

    /// Тропинки региона, прокладывает при первом обращении.
    fn region(
        &mut self,
        coord: IVec2,
        terrain: &Terrain,
        settlements: &mut Settlements,
    ) -> &[Trail] {
        &self
            .regions
            .entry(coord)
            .or_insert_with(|| {
                let houses = settlements.region(coord, terrain).houses.clone();
                let trails = plan(&houses, terrain);
                let walk = WalkGraph::new(&trails);
                Region { trails, walk }
            })
            .trails
    }
}

fn reset_paths(mut rebuilt: MessageReader<WorldRebuilt>, mut paths: ResMut<VillagePaths>) {
    if rebuilt.read().last().is_none() {
        return;
    }
    paths.regions.clear();
    paths.painted.clear();
}

/// Красит тропинки на земле новых участков и убирает с них деревья, кусты и камни.
pub fn paint_paths(
    mut commands: Commands,
    mut paths: ResMut<VillagePaths>,
    mut terrain: ResMut<Terrain>,
    mut settlements: ResMut<Settlements>,
    mut props: ResMut<Props>,
    visible: Res<VisibleChunks>,
    chunks: Query<&ChunkMesh, Added<ChunkMesh>>,
) {
    // До объявления мира дома пустые: тропинок бы не было.
    if settlements.seed().is_none() {
        return;
    }
    let color = LinearRgba::from(PATH_COLOR);
    let path_color = [color.red, color.green, color.blue];
    for chunk in &chunks {
        let coord = chunk.0;
        if !paths.painted.insert(coord) {
            continue;
        }
        let origin = chunk_origin(coord);
        let reach = GRID_MARGIN + TRAIL_WIDTH;
        let low = region_of(origin - Vec2::splat(reach));
        let high = region_of(origin + Vec2::splat(CHUNK_SIZE as f32 + reach));
        let mut trails: Vec<Trail> = Vec::new();
        for rz in low.y..=high.y {
            for rx in low.x..=high.x {
                trails.extend_from_slice(paths.region(
                    IVec2::new(rx, rz),
                    &terrain,
                    &mut settlements,
                ));
            }
        }
        let chunk_min = origin - Vec2::splat(TRAIL_WIDTH);
        let chunk_max = origin + Vec2::splat(CHUNK_SIZE as f32 + TRAIL_WIDTH);
        trails.retain(|t| t.min.cmple(chunk_max).all() && t.max.cmpge(chunk_min).all());
        if trails.is_empty() {
            continue;
        }
        for prop in props.chunk(coord, &terrain, &mut settlements).iter_mut() {
            let clear = match prop.kind {
                PropKind::Fence | PropKind::Post | PropKind::Antenna => continue,
                PropKind::Tree => TREE_CLEAR,
                _ => PROP_CLEAR,
            };
            let p = prop.transform.translation.xz();
            if trails
                .iter()
                .any(|t| t.distance(p).is_some_and(|d| d < TRAIL_WIDTH * 0.5 + clear))
            {
                prop.status = PropStatus::Destroyed;
            }
        }
        let mut changed = Vec::new();
        let base = origin.as_ivec2();
        // Вершина на дальней границе — первая у соседнего участка: красит он.
        for iz in 0..CHUNK_SIZE {
            for ix in 0..CHUNK_SIZE {
                let (vx, vz) = (base.x + ix, base.y + iz);
                let p = Vec2::new(vx as f32, vz as f32);
                let cover = trails
                    .iter()
                    .filter_map(|t| t.distance(p).map(coverage))
                    .fold(0.0_f32, f32::max);
                if cover <= 0.0 {
                    continue;
                }
                changed.extend(terrain.edit_vertex(vx, vz, |_, c| {
                    for i in 0..3 {
                        c[i] += (path_color[i] - c[i]) * cover;
                    }
                }));
            }
        }
        if !changed.is_empty() {
            changed.sort_by_key(|c| (c.x, c.y));
            changed.dedup();
            visible.mark_dirty(&mut commands, changed);
        }
    }
}

/// Насколько вершина на расстоянии `d` от оси покрыта тропинкой: мягкий край.
fn coverage(d: f32) -> f32 {
    let inner = TRAIL_WIDTH * 0.5 - 0.5;
    let outer = TRAIL_WIDTH * 0.5 + 1.0;
    let t = 1.0 - ((d - inner) / (outer - inner)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Порог дома: середина стены, точка у двери снаружи и точка за калиткой. Сторона с дверью (+Z),
/// если там свободно, иначе первая свободная из остальных.
fn porch(house: &HouseState, grid: &Grid) -> Option<(Vec2, Vec2, Vec2)> {
    (0..4).find_map(|side| {
        let wall = house.wall_center(side);
        let normal = (wall - house.position).normalize_or_zero();
        let door = wall + normal * (HOUSE_CLEAR + 0.3);
        let out = wall + normal * PORCH;
        (grid.free(door) && grid.free(out)).then_some((wall, door, out))
    })
}

/// Тропинки посёлка: отводы от дверей и тропинки между соседними домами.
pub fn plan(houses: &[HouseState], terrain: &Terrain) -> Vec<Trail> {
    if houses.is_empty() {
        return Vec::new();
    }
    let mut grid = Grid::new(houses, terrain);
    let mut trails = Vec::new();
    let mut nodes = Vec::new();
    for house in houses {
        let Some((wall, door, out)) = porch(house, &grid) else {
            continue;
        };
        if let Some(route) = grid.route(door, out) {
            let mut points = vec![wall];
            points.extend(route);
            trails.push(Trail::new(points));
            nodes.push(out);
        }
    }
    let mut edges = neighbour_graph(&nodes);
    // Короткие первыми: длинные потом сливаются с ними.
    edges.sort_by(|a, b| {
        nodes[a.0]
            .distance(nodes[a.1])
            .total_cmp(&nodes[b.0].distance(nodes[b.1]))
    });
    for (a, b) in edges {
        if let Some(route) = grid.route(nodes[a], nodes[b]) {
            trails.push(Trail::new(route));
        }
    }
    trails
}

/// Граф относительных соседей: ребро, если нет третьей точки ближе к обоим концам.
fn neighbour_graph(nodes: &[Vec2]) -> Vec<(usize, usize)> {
    let mut edges = Vec::new();
    for a in 0..nodes.len() {
        for b in a + 1..nodes.len() {
            let d = nodes[a].distance(nodes[b]);
            let blocked = (0..nodes.len()).any(|c| {
                c != a && c != b && nodes[a].distance(nodes[c]).max(nodes[b].distance(nodes[c])) < d
            });
            if !blocked {
                edges.push((a, b));
            }
        }
    }
    edges
}

/// Точки ломаной не реже `step`.
fn densify(points: &[Vec2], step: f32) -> Vec<Vec2> {
    let mut out = Vec::new();
    for w in points.windows(2) {
        let n = (w[0].distance(w[1]) / step).ceil().max(1.0) as usize;
        for i in 0..n {
            out.push(w[0].lerp(w[1], i as f32 / n as f32));
        }
    }
    out.extend(points.last());
    out
}

/// Граф прогулки: точки тропинок через [`NODE_STEP`], связи вдоль тропинок и на развилках.
#[derive(Debug, Default)]
struct WalkGraph {
    nodes: Vec<Vec2>,
    links: Vec<Vec<usize>>,
}

impl WalkGraph {
    fn new(trails: &[Trail]) -> Self {
        let mut graph = Self::default();
        let mut owner = Vec::new();
        for (t, trail) in trails.iter().enumerate() {
            let start = graph.nodes.len();
            for p in densify(&trail.points, NODE_STEP) {
                graph.nodes.push(p);
                graph.links.push(Vec::new());
                owner.push(t);
            }
            for i in start + 1..graph.nodes.len() {
                graph.links[i - 1].push(i);
                graph.links[i].push(i - 1);
            }
        }
        for a in 0..graph.nodes.len() {
            for b in a + 1..graph.nodes.len() {
                if owner[a] != owner[b] && graph.nodes[a].distance(graph.nodes[b]) < JOIN {
                    graph.links[a].push(b);
                    graph.links[b].push(a);
                }
            }
        }
        graph
    }

    fn stroll(&self, from: Vec2, dice: &mut Dice) -> Option<Vec<Vec2>> {
        let start = (0..self.nodes.len())
            .min_by(|a, b| {
                self.nodes[*a]
                    .distance(from)
                    .total_cmp(&self.nodes[*b].distance(from))
            })
            .filter(|i| self.nodes[*i].distance(from) < STROLL_REACH)?;
        // Кратчайшие пути от начала: длина и откуда пришли.
        let mut cost = vec![f32::MAX; self.nodes.len()];
        let mut came = vec![usize::MAX; self.nodes.len()];
        let mut open = BinaryHeap::new();
        cost[start] = 0.0;
        open.push((Reverse(0u32), start));
        while let Some((Reverse(c), i)) = open.pop() {
            if c as f32 / 100.0 > cost[i] + 0.01 {
                continue;
            }
            for &j in &self.links[i] {
                let next = cost[i] + self.nodes[i].distance(self.nodes[j]);
                if next < cost[j] {
                    cost[j] = next;
                    came[j] = i;
                    open.push((Reverse((next * 100.0) as u32), j));
                }
            }
        }
        let goals: Vec<usize> = (0..self.nodes.len())
            .filter(|i| (STROLL_MIN..STROLL_MAX).contains(&cost[*i]))
            .collect();
        if goals.is_empty() {
            return None;
        }
        let mut i = goals[dice.below(goals.len())];
        let mut route = vec![self.nodes[i]];
        while i != start {
            i = came[i];
            route.push(self.nodes[i]);
        }
        route.reverse();
        Some(route)
    }
}

/// Сетка 1 м вокруг посёлка: занятые клетки, высоты и натоптанность.
struct Grid {
    min: Vec2,
    size: IVec2,
    blocked: Vec<bool>,
    height: Vec<f32>,
    trodden: Vec<bool>,
}

impl Grid {
    fn new(houses: &[HouseState], terrain: &Terrain) -> Self {
        let mut min = Vec2::splat(f32::MAX);
        let mut max = Vec2::splat(f32::MIN);
        for house in houses {
            for c in house.footprint().corners() {
                min = min.min(c);
                max = max.max(c);
            }
        }
        let min = (min - Vec2::splat(GRID_MARGIN)).floor();
        let max = (max + Vec2::splat(GRID_MARGIN)).ceil();
        let size = (max - min).as_ivec2() + IVec2::ONE;
        let count = (size.x * size.y) as usize;
        let mut grid = Self {
            min,
            size,
            blocked: vec![false; count],
            height: vec![0.0; count],
            trodden: vec![false; count],
        };
        let fences: Vec<(Vec2, Vec2)> = houses.iter().flat_map(crate::props::front_fence).collect();
        for i in 0..count {
            let p = grid.point(i);
            grid.height[i] = terrain.height_at(p.x, p.y);
            let house = houses
                .iter()
                .any(|h| h.footprint().distance(p) < HOUSE_CLEAR);
            let fence = fences
                .iter()
                .any(|(a, b)| segment_distance(p, *a, *b) < FENCE_CLEAR);
            let rail = terrain.lines().nearest(p).distance < crate::railway::TRACK_BAND;
            grid.blocked[i] = house || fence || rail;
        }
        grid
    }

    fn point(&self, i: usize) -> Vec2 {
        let i = i as i32;
        self.min + Vec2::new((i % self.size.x) as f32, (i / self.size.x) as f32)
    }

    fn cell(&self, p: Vec2) -> Option<usize> {
        let c = (p - self.min).round().as_ivec2();
        (c.x >= 0 && c.y >= 0 && c.x < self.size.x && c.y < self.size.y)
            .then(|| (c.y * self.size.x + c.x) as usize)
    }

    fn free(&self, p: Vec2) -> bool {
        self.cell(p).is_some_and(|i| !self.blocked[i])
    }

    /// Путь A* по сетке, сглаженный; клетки пути становятся натоптанными.
    fn route(&mut self, from: Vec2, to: Vec2) -> Option<Vec<Vec2>> {
        let start = self.cell(from)?;
        let goal = self.cell(to)?;
        let target = self.point(goal);
        let heuristic = |p: Vec2| target.distance(p) * TRODDEN;
        let count = self.blocked.len();
        let mut cost = vec![f32::MAX; count];
        let mut came = vec![usize::MAX; count];
        let mut open = BinaryHeap::new();
        cost[start] = 0.0;
        open.push((Reverse((heuristic(from) * 100.0) as u32), start));
        let mut reached = false;
        while let Some((_, i)) = open.pop() {
            if i == goal {
                reached = true;
                break;
            }
            let c = IVec2::new(i as i32 % self.size.x, i as i32 / self.size.x);
            for (dx, dz) in [
                (-1, 0),
                (1, 0),
                (0, -1),
                (0, 1),
                (-1, -1),
                (1, -1),
                (-1, 1),
                (1, 1),
            ] {
                let n = c + IVec2::new(dx, dz);
                if n.x < 0 || n.y < 0 || n.x >= self.size.x || n.y >= self.size.y {
                    continue;
                }
                let j = (n.y * self.size.x + n.x) as usize;
                if self.blocked[j] && j != goal {
                    continue;
                }
                let step = if dx != 0 && dz != 0 {
                    std::f32::consts::SQRT_2
                } else {
                    1.0
                };
                let climb = (self.height[j] - self.height[i]).abs() / step;
                let mut w = step * (1.0 + SLOPE_COST * climb);
                if self.trodden[j] {
                    w *= TRODDEN;
                }
                let next = cost[i] + w;
                if next < cost[j] {
                    cost[j] = next;
                    came[j] = i;
                    let f = next + heuristic(self.point(j));
                    open.push((Reverse((f * 100.0) as u32), j));
                }
            }
        }
        if !reached {
            return None;
        }
        let mut i = goal;
        let mut cells = vec![i];
        while i != start {
            i = came[i];
            cells.push(i);
        }
        cells.reverse();
        // Натоптанность — полоса шириной с тропинку.
        let r = (TRAIL_WIDTH * 0.5).ceil() as i32;
        for &cell in &cells {
            let c = IVec2::new(cell as i32 % self.size.x, cell as i32 / self.size.x);
            for dz in -r..=r {
                for dx in -r..=r {
                    let n = c + IVec2::new(dx, dz);
                    if n.x >= 0 && n.y >= 0 && n.x < self.size.x && n.y < self.size.y {
                        self.trodden[(n.y * self.size.x + n.x) as usize] = true;
                    }
                }
            }
        }
        let mut points: Vec<Vec2> = cells.iter().map(|c| self.point(*c)).collect();
        points[0] = from;
        *points.last_mut().unwrap() = to;
        points = self.pull_string(&points);
        Some(chaikin(&points, 2))
    }

    /// Выпрямляет ступеньки сетки: точка пропускается, если до следующей видно по свободным клеткам.
    fn pull_string(&self, points: &[Vec2]) -> Vec<Vec2> {
        if points.len() <= 2 {
            return points.to_vec();
        }
        let mut out = vec![points[0]];
        let mut anchor = 0;
        for i in 2..points.len() {
            if !self.clear_line(points[anchor], points[i]) {
                out.push(points[i - 1]);
                anchor = i - 1;
            }
        }
        out.push(*points.last().unwrap());
        out
    }

    /// Отрезок проходит по свободным и не слишком крутым клеткам.
    fn clear_line(&self, a: Vec2, b: Vec2) -> bool {
        let n = (a.distance(b) * 2.0).ceil().max(1.0) as usize;
        let ha = self.cell(a).map_or(0.0, |i| self.height[i]);
        (1..n).all(|k| {
            let p = a.lerp(b, k as f32 / n as f32);
            self.cell(p)
                .is_some_and(|i| !self.blocked[i] && (self.height[i] - ha).abs() < 1.5)
        })
    }
}

/// Сглаживание Чайкина: углы срезаются, концы остаются.
fn chaikin(points: &[Vec2], iterations: usize) -> Vec<Vec2> {
    let mut points = points.to_vec();
    for _ in 0..iterations {
        if points.len() < 3 {
            return points;
        }
        let mut out = vec![points[0]];
        for w in points.windows(2) {
            out.push(w[0].lerp(w[1], 0.25));
            out.push(w[0].lerp(w[1], 0.75));
        }
        out.push(*points.last().unwrap());
        points = out;
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Связен ли граф на `n` вершинах.
    fn connected(n: usize, edges: &[(usize, usize)]) -> bool {
        let mut seen = vec![false; n];
        let mut stack = vec![0];
        while let Some(i) = stack.pop() {
            if std::mem::replace(&mut seen[i], true) {
                continue;
            }
            for (a, b) in edges {
                if *a == i {
                    stack.push(*b);
                }
                if *b == i {
                    stack.push(*a);
                }
            }
        }
        seen.into_iter().all(|s| s)
    }

    #[test]
    fn соседи_без_третьего_между() {
        // Три точки на прямой: крайние не соединены напрямую.
        let nodes = [Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(20.0, 0.0)];
        let edges = neighbour_graph(&nodes);
        assert!(edges.contains(&(0, 1)) && edges.contains(&(1, 2)) && !edges.contains(&(0, 2)));
    }

    #[test]
    fn сеть_соседей_связна() {
        let mut dice = Dice::seeded(7);
        for _ in 0..50 {
            let n = 4 + dice.below(6);
            let nodes: Vec<Vec2> = (0..n)
                .map(|_| Vec2::new(dice.range(0.0, 50.0), dice.range(0.0, 50.0)))
                .collect();
            assert!(connected(n, &neighbour_graph(&nodes)));
        }
    }

    #[test]
    fn край_тропинки_мягкий() {
        assert_eq!(coverage(0.0), 1.0);
        assert_eq!(coverage(5.0), 0.0);
        let mid = coverage(TRAIL_WIDTH * 0.5);
        assert!(mid > 0.0 && mid < 1.0);
    }

    #[test]
    fn прогулка_идёт_по_тропинкам() {
        // Две тропинки крестом: прогулка от конца одной доходит до другой через развилку.
        let trails = [
            Trail::new(vec![Vec2::new(-20.0, 0.0), Vec2::new(20.0, 0.0)]),
            Trail::new(vec![Vec2::new(0.0, -20.0), Vec2::new(0.0, 20.0)]),
        ];
        let graph = WalkGraph::new(&trails);
        let mut dice = Dice::seeded(3);
        for _ in 0..20 {
            let route = graph
                .stroll(Vec2::new(-21.0, 1.0), &mut dice)
                .expect("маршрут");
            for w in route.windows(2) {
                assert!(w[0].distance(w[1]) <= NODE_STEP + 0.01);
            }
            for p in &route {
                assert!(
                    p.x.abs() < 0.01 || p.y.abs() < 0.01,
                    "точка {p} не на тропинке"
                );
            }
        }
        assert!(graph.stroll(Vec2::new(100.0, 100.0), &mut dice).is_none());
    }
}
