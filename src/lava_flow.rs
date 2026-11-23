//! Течение лавы по рельефу: у каждой ячейки 1×1 м (вершины сетки рельефа) своя толщина горячей
//! лавы и застывшей корки. Шаг [`flow_step`] — без Bevy: лава перетекает к соседям, у которых
//! поверхность (рельеф + корка + лава) ниже, а за краем ямы понемногу остывает в корку.
//!
//! Перетекание — «виртуальные трубы» между соседними ячейками: поток в трубе набирает скорость от
//! перепада поверхностей и гаснет от трения, а его разгон пропорционален толщине лавы. Глубокий канал
//! поэтому несёт много, тонкая плёнка — мало, и лава уходит вниз по склону за секунды, а не
//! расползается, как при простом выравнивании соседей.
//!
//! Порог текучести как у вязкой жидкости: лава трогается, только если перепад к соседу больше
//! `YIELD_K / толщина`. Глубокое озеро поэтому стоит ровно, а тонкий слой останавливается: на ровном
//! месте — толщиной около `√YIELD_K`, на склоне — тоньше. Отсюда плавный край потока, и вверх лава не
//! течёт: против перепада поток только тормозится.
//!
//! Остывание идёт всегда, а не только у стоящей лавы: за шаг слой за краем ямы теряет в корку
//! `COOL_RATE · dt`. Пока источник подаёт, поток идёт дальше; тонкий край застывает первым, и поток
//! останавливается языком сам. Иначе толстая лужа, медленно ползущая по пологому склону, не застыла бы.

use std::collections::{HashMap, HashSet};

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

/// Шаг течения, с.
pub const FLOW_STEP: f32 = 0.2;
/// Подшаг труб, с: шаг течения делится на подшаги, чтобы глубокое озеро не раскачивалось.
const SUBSTEP: f32 = 0.05;
/// Разгон потока в трубе на метр перепада и метр толщины, м/с².
pub const PIPE_ACCEL: f32 = 4.0;
/// Доля потока, которая остаётся после подшага: трение вязкой лавы.
pub const PIPE_FRICTION: f32 = 0.95;
/// Толщина, от которой разгон дальше не растёт, м: глубокое озеро не плещет.
const PIPE_DEPTH: f32 = 3.0;
/// Порог текучести: перепад, при котором слой толщиной `d` трогается, — `YIELD_K / d`, м².
/// На ровном месте лава останавливается слоем около √0,045 ≈ 0,21 м.
pub const YIELD_K: f32 = 0.045;
/// Порог текучести не больше этого, м: самая тонкая плёнка на крутом склоне всё-таки стекает.
const YIELD_MAX: f32 = 0.5;
/// Скорость остывания лавы за краем ямы, м/с: слой в полметра застывает за 25 с.
pub const COOL_RATE: f32 = 0.02;
/// Горячая лава тоньше этого — уже не лава (не жжёт, не рисуется), м.
pub const MIN_DEPTH: f32 = 0.02;
/// Остаток тоньше этого пропадает, м: иначе за потоком тянулся бы бесконечный след долей миллиметра.
const DUST: f32 = 0.002;

/// Насколько поверхность горячей лавы рисуется над толщиной, м: не мерцает с рельефом.
pub const SURFACE_LIFT: f32 = 0.05;
/// Корка рисуется чуть ниже горячей лавы над ней, м.
pub const CRUST_LIFT: f32 = 0.03;
/// Угол квадрата сетки без лавы уходит под рельеф на столько, м: край потока плавно уходит в землю.
pub const EDGE_SINK: f32 = 0.1;

/// Лава в ячейке.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LavaCell {
    /// Толщина горячей лавы, м.
    pub hot: f32,
    /// Толщина застывшей корки под ней, м.
    pub crust: f32,
    /// Высота рельефа под ячейкой на последнем шаге, м.
    pub ground: f32,
    /// Поток в трубах к соседям `NEIGHBOURS`, м³/с.
    pub out: [f32; 4],
}

impl LavaCell {
    /// Горячая ли лава здесь: жжёт, рисуется, её объезжают.
    pub fn is_hot(&self) -> bool {
        self.hot >= MIN_DEPTH
    }

    /// Высота поверхности: рельеф, корка и горячая лава.
    pub fn surface(&self) -> f32 {
        self.ground + self.crust + self.hot
    }
}

/// Ячейки лавы на плане.
pub type Cells = HashMap<IVec2, LavaCell>;

/// Итог шага.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct StepResult {
    /// Изменился набор горячих ячеек или корки: пересчитать, где лава, и пересобрать сетки.
    pub reshaped: bool,
    /// Наибольшее изменение толщины горячей лавы за шаг, м: если мало — сетку можно не пересобирать.
    pub moved: f32,
}

const NEIGHBOURS: [IVec2; 4] = [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y];

/// Добавляет горячую лаву объёмом `volume` м³ в ячейку `at`.
pub fn pour(cells: &mut Cells, at: IVec2, volume: f32, height: impl Fn(i32, i32) -> f32) {
    let cell = cells.entry(at).or_insert_with(|| LavaCell {
        ground: height(at.x, at.y),
        ..default()
    });
    cell.hot += volume;
}

/// Один шаг течения длиной `dt` секунд. `height` — рельеф, `allowed` — может ли ячейка принять
/// лаву (предел удаления от прорыва), `pit` — ячейка ямы прорыва: такие не застывают.
pub fn flow_step(
    cells: &mut Cells,
    dt: f32,
    height: impl Fn(i32, i32) -> f32,
    allowed: impl Fn(IVec2) -> bool,
    pit: impl Fn(IVec2) -> bool,
) -> StepResult {
    let hot_before: HashSet<IVec2> = cells.iter().filter(|(_, c)| c.is_hot()).map(|(k, _)| *k).collect();
    let crust_before = cells.values().filter(|c| c.crust > 0.0).count();
    // Рельеф под лавой мог измениться (кратер, провал): поверхность считается от текущего.
    for (at, cell) in cells.iter_mut() {
        cell.ground = height(at.x, at.y);
    }
    let start: HashMap<IVec2, f32> = cells.iter().map(|(k, c)| (*k, c.hot)).collect();
    let substeps = (dt / SUBSTEP).round().max(1.0) as usize;
    for _ in 0..substeps {
        substep(cells, &height, &allowed);
    }

    let moved = cells
        .iter()
        .map(|(at, cell)| (cell.hot - start.get(at).copied().unwrap_or(0.0)).abs())
        .fold(0.0f32, f32::max);
    // Лава за краем ямы остывает в корку, плёнка тоньше горячей застывает целиком, крошки пропадают.
    for (&at, cell) in cells.iter_mut() {
        if cell.hot < DUST {
            cell.hot = 0.0;
            cell.out = [0.0; 4];
            continue;
        }
        if pit(at) {
            continue;
        }
        let cooled = if cell.hot - COOL_RATE * dt < MIN_DEPTH { cell.hot } else { COOL_RATE * dt };
        cell.hot -= cooled;
        cell.crust += cooled;
        if cell.hot == 0.0 {
            cell.out = [0.0; 4];
        }
    }
    cells.retain(|_, c| c.hot > 0.0 || c.crust > 0.0);

    let hot_after = cells.iter().filter(|(_, c)| c.is_hot()).count();
    let crust_after = cells.values().filter(|c| c.crust > 0.0).count();
    let reshaped = hot_after != hot_before.len()
        || crust_after != crust_before
        || cells.iter().any(|(k, c)| c.is_hot() && !hot_before.contains(k));
    StepResult { reshaped, moved }
}

/// Подшаг труб: потоки разгоняются по старым поверхностям, не отдают больше, чем есть, потом
/// объёмы переносятся разом.
fn substep(cells: &mut Cells, height: &impl Fn(i32, i32) -> f32, allowed: &impl Fn(IVec2) -> bool) {
    let surface = |cells: &Cells, at: IVec2| cells.get(&at).map_or_else(|| height(at.x, at.y), LavaCell::surface);
    let mut flows: Vec<(IVec2, [f32; 4])> = Vec::with_capacity(cells.len());
    for (&at, cell) in cells.iter() {
        if cell.hot <= 0.0 {
            continue;
        }
        let here = cell.surface();
        let threshold = (YIELD_K / cell.hot).min(YIELD_MAX);
        let pipe = cell.hot.min(PIPE_DEPTH);
        let mut out = [0.0f32; 4];
        for (i, o) in NEIGHBOURS.iter().enumerate() {
            let n = at + *o;
            if !allowed(n) {
                continue;
            }
            let drop = here - surface(cells, n);
            // Порог тормозит в обе стороны: ниже него лава стоит.
            let push = if drop > threshold { drop - threshold } else { drop.min(0.0) };
            out[i] = (cell.out[i] * PIPE_FRICTION + SUBSTEP * PIPE_ACCEL * pipe * push).max(0.0);
        }
        let total = out.iter().sum::<f32>() * SUBSTEP;
        // Отдать больше, чем есть, нельзя.
        if total > cell.hot {
            let k = cell.hot / total;
            out.iter_mut().for_each(|f| *f *= k);
        }
        flows.push((at, out));
    }
    let mut delta: HashMap<IVec2, f32> = HashMap::new();
    for (at, out) in &flows {
        for (i, o) in NEIGHBOURS.iter().enumerate() {
            if out[i] > 0.0 {
                let v = out[i] * SUBSTEP;
                *delta.entry(*at + *o).or_default() += v;
                *delta.entry(*at).or_default() -= v;
            }
        }
    }
    for (at, out) in flows {
        if let Some(cell) = cells.get_mut(&at) {
            cell.out = out;
        }
    }
    for (at, d) in delta {
        let cell = cells.entry(at).or_insert_with(|| LavaCell {
            ground: height(at.x, at.y),
            ..default()
        });
        cell.hot = (cell.hot + d).max(0.0);
    }
}

/// Слой сетки: горячая лава или застывшая корка.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Hot,
    Crust,
}

/// Высота слоя в ячейке, если слой там есть.
fn layer_height(cell: &LavaCell, layer: Layer) -> Option<f32> {
    match layer {
        Layer::Hot if cell.is_hot() => Some(cell.surface() + SURFACE_LIFT),
        Layer::Crust if cell.crust > 0.0 => Some(cell.ground + cell.crust + CRUST_LIFT),
        _ => None,
    }
}

/// Сетка слоя по рельефу: квадрат между четырьмя вершинами рисуется, если слой есть хотя бы в одном
/// углу. Угол со слоем стоит на его поверхности, угол без слоя — под рельефом на [`EDGE_SINK`], но не
/// выше средней поверхности углов со слоем: у края озера в яме гладь продолжается под стенку
/// (её закрывает рельеф), а край потока на склоне уходит в землю. `None` — слоя нет нигде.
pub fn layer_mesh(cells: &Cells, layer: Layer, height: impl Fn(i32, i32) -> f32) -> Option<Mesh> {
    let mut squares: HashSet<IVec2> = HashSet::new();
    for (at, cell) in cells {
        if layer_height(cell, layer).is_some() {
            for dz in -1..=0 {
                for dx in -1..=0 {
                    squares.insert(*at + IVec2::new(dx, dz));
                }
            }
        }
    }
    if squares.is_empty() {
        return None;
    }
    let mut positions = Vec::with_capacity(squares.len() * 4);
    let mut uvs = Vec::with_capacity(squares.len() * 4);
    let mut indices = Vec::with_capacity(squares.len() * 6);
    const CORNERS: [IVec2; 4] = [IVec2::new(0, 0), IVec2::new(1, 0), IVec2::new(1, 1), IVec2::new(0, 1)];
    for square in &squares {
        let tops: [Option<f32>; 4] = CORNERS.map(|o| cells.get(&(*square + o)).and_then(|c| layer_height(c, layer)));
        let (sum, count) = tops.iter().flatten().fold((0.0, 0), |(s, n), h| (s + h, n + 1));
        if count == 0 {
            continue;
        }
        let mean = sum / count as f32;
        let base = positions.len() as u32;
        for (o, top) in CORNERS.iter().zip(tops) {
            let v = *square + *o;
            // Под слоем — рельеф и корка: край горячей лавы уходит под верх корки, а не под землю.
            let base = cells.get(&v).map_or_else(|| height(v.x, v.y), |c| c.ground + c.crust);
            let y = top.unwrap_or_else(|| (base - EDGE_SINK).min(mean));
            positions.push([v.x as f32, y, v.y as f32]);
            uvs.push([v.x as f32 / 8.0, v.y as f32 / 8.0]);
        }
        indices.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
    }
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    Some(
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
            .with_inserted_indices(Indices::U32(indices)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(cells: &Cells) -> f32 {
        cells.values().map(|c| c.hot + c.crust).sum()
    }

    /// Шаги, пока лава не перестанет двигаться (или `limit` шагов). Застывания нет.
    fn settle(cells: &mut Cells, height: impl Fn(i32, i32) -> f32 + Copy, allowed: impl Fn(IVec2) -> bool + Copy, limit: usize) -> usize {
        for i in 0..limit {
            let r = flow_step(cells, FLOW_STEP, height, allowed, |_| true);
            if r.moved < 1e-4 && !r.reshaped {
                return i;
            }
        }
        limit
    }

    #[test]
    fn объём_сохраняется() {
        let height = |x: i32, z: i32| -0.3 * x as f32 + 0.05 * z as f32;
        let mut cells = Cells::new();
        pour(&mut cells, IVec2::ZERO, 20.0, height);
        settle(&mut cells, height, |_| true, 400);
        let v = volume(&cells);
        // Пропадают только крошки тоньше DUST.
        assert!((v - 20.0).abs() < 0.2, "объём {v}");
    }

    #[test]
    fn стекает_по_склону_и_не_течёт_вверх() {
        // Плоскость падает к +x.
        let height = |x: i32, _: i32| -0.5 * x as f32;
        let mut cells = Cells::new();
        // Источник подаёт понемногу, как место прорыва: разом налитый столб растёкся бы во все стороны.
        for i in 0..100 {
            if i < 20 {
                pour(&mut cells, IVec2::ZERO, 0.5, height);
            }
            flow_step(&mut cells, FLOW_STEP, height, |_| true, |_| true);
        }
        let hot: Vec<IVec2> = cells.iter().filter(|(_, c)| c.hot > DUST).map(|(k, _)| *k).collect();
        assert!(hot.iter().any(|c| c.x > 10), "лава ушла вниз по склону");
        assert!(hot.iter().all(|c| c.x >= -1), "вверх по склону лава не идёт: {:?}", hot.iter().min_by_key(|c| c.x));
        let (sum, n) = cells.iter().fold((0.0, 0.0), |(s, n), (k, c)| (s + k.x as f32 * c.hot, n + c.hot));
        assert!(sum / n > 5.0, "центр масс внизу склона");
    }

    #[test]
    fn на_ровном_месте_останавливается_слоем() {
        let height = |_: i32, _: i32| 0.0;
        let mut cells = Cells::new();
        pour(&mut cells, IVec2::ZERO, 12.0, height);
        let steps = settle(&mut cells, height, |_| true, 2000);
        assert!(steps < 2000, "лава остановилась");
        let reach = cells.keys().map(|c| c.as_vec2().length()).fold(0.0, f32::max);
        assert!(reach < 10.0, "растеклась не бесконечно: {reach}");
        let max = cells.values().map(|c| c.hot).fold(0.0, f32::max);
        assert!(max < 1.0, "слой, а не столб: {max}");
        // Край — слой порядка √YIELD_K, а не плёнка и не стена.
        let edge = cells
            .iter()
            .filter(|(k, _)| NEIGHBOURS.iter().any(|o| !cells.contains_key(&(**k + *o))))
            .map(|(_, c)| c.hot)
            .fold(f32::MAX, f32::min);
        assert!(edge > 0.05 && edge < 0.4, "толщина края {edge}");
    }

    #[test]
    fn глубокое_озеро_ровное() {
        // Чаша 7×7 м глубиной 10 м.
        let height = |x: i32, z: i32| if x.abs() <= 3 && z.abs() <= 3 { -10.0 } else { 0.0 };
        let mut cells = Cells::new();
        pour(&mut cells, IVec2::ZERO, 49.0 * 5.0, height);
        settle(&mut cells, height, |_| true, 2000);
        let tops: Vec<f32> = cells.values().map(LavaCell::surface).collect();
        let (lo, hi) = tops.iter().fold((f32::MAX, f32::MIN), |(a, b), t| (a.min(*t), b.max(*t)));
        assert!(hi - lo < 0.1, "гладь ровная: {lo}..{hi}");
        assert!((hi + 5.0).abs() < 0.1, "уровень на половине глубины: {hi}");
    }

    #[test]
    fn не_выходит_за_предел() {
        let height = |_: i32, _: i32| 0.0;
        let mut cells = Cells::new();
        pour(&mut cells, IVec2::ZERO, 30.0, height);
        settle(&mut cells, height, |c| c.length_squared() <= 9, 2000);
        assert!(cells.keys().all(|c| c.length_squared() <= 9));
    }

    #[test]
    fn лава_за_краем_ямы_застывает_а_в_яме_нет() {
        // Яма 3×3 глубиной 2 м; налито больше, чем она вмещает, — излишек растекается вокруг.
        let pit = |c: IVec2| c.x.abs() <= 1 && c.y.abs() <= 1;
        let height = |x: i32, z: i32| if pit(IVec2::new(x, z)) { -2.0 } else { 0.0 };
        let mut cells = Cells::new();
        pour(&mut cells, IVec2::ZERO, 9.0 * 2.0 + 6.0, height);
        let mut reshaped = false;
        for _ in 0..(60.0 / FLOW_STEP) as usize {
            reshaped |= flow_step(&mut cells, FLOW_STEP, height, |_| true, pit).reshaped;
        }
        assert!(reshaped);
        assert!(cells.iter().filter(|(k, _)| pit(**k)).all(|(_, c)| c.is_hot()), "яма не застывает");
        let outside: Vec<_> = cells.iter().filter(|(k, _)| !pit(**k)).collect();
        assert!(!outside.is_empty(), "лава вылилась");
        assert!(outside.iter().all(|(_, c)| c.hot == 0.0 && c.crust > 0.0), "вокруг корка");
    }

    #[test]
    fn горячая_лава_течёт_поверх_корки() {
        let height = |x: i32, _: i32| -0.5 * x as f32;
        let mut cells = Cells::new();
        // Корка толщиной 1 м на склоне у места излива.
        for x in 1..=3 {
            cells.insert(IVec2::new(x, 0), LavaCell { crust: 1.0, ground: height(x, 0), ..default() });
        }
        pour(&mut cells, IVec2::ZERO, 5.0, height);
        for _ in 0..60 {
            flow_step(&mut cells, FLOW_STEP, height, |_| true, |_| true);
        }
        assert!(cells.iter().any(|(k, c)| k.x > 3 && c.hot > DUST), "лава перетекла через корку");
        assert!(cells.iter().filter(|(k, _)| (1..=3).contains(&k.x) && k.y == 0).all(|(_, c)| c.crust >= 1.0), "корка на месте");
    }

    #[test]
    fn сетка_лежит_на_поверхности_и_уходит_под_землю_по_краю() {
        let height = |x: i32, _: i32| 0.2 * x as f32;
        let mut cells = Cells::new();
        cells.insert(IVec2::ZERO, LavaCell { hot: 0.3, ground: 0.0, ..default() });
        let mesh = layer_mesh(&cells, Layer::Hot, height).expect("сетка");
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
            panic!("позиции");
        };
        for p in positions {
            let (x, z) = (p[0] as i32, p[2] as i32);
            if (x, z) == (0, 0) {
                assert!((p[1] - (0.3 + SURFACE_LIFT)).abs() < 1e-5, "вершина в лаве на её поверхности");
            } else {
                assert!(p[1] <= height(x, z) - EDGE_SINK + 1e-5, "вершина без лавы под рельефом: {p:?}");
            }
        }
        assert!(layer_mesh(&cells, Layer::Crust, height).is_none(), "корки нет");
    }
}
