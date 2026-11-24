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
//! Застывшая целиком ячейка выходит из шага ([`Field::cooled`]): для соседей она — рельеф с коркой.

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

/// Ячейки с горячей лавой (и коркой под ней) на плане.
pub type Cells = HashMap<IVec2, LavaCell>;

/// Лава на плане: ячейки, где есть горячая лава, и застывшие целиком ячейки отдельно.
///
/// Застывшая ячейка в шаге течения не участвует: для соседей она просто часть рельефа
/// (рельеф + корка). Так шаг зависит только от горячей лавы, а корка, которая копится до конца
/// забега от всех прорывов, его не замедляет. Горячая лава, натёкшая на корку, забирает ячейку
/// обратно в шаг вместе с коркой.
#[derive(Debug, Default, Clone)]
pub struct Field {
    /// Ячейки с горячей лавой.
    pub cells: Cells,
    /// Застывшие ячейки: толщина корки, м.
    pub cooled: HashMap<IVec2, f32>,
}

/// Итог шага.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct StepResult {
    /// Изменился набор горячих ячеек: пересчитать, где лава жжёт.
    pub reshaped: bool,
    /// Наибольшее изменение толщины горячей лавы за шаг, м: если мало — сетку можно не пересобирать.
    pub moved: f32,
    /// Ячейки, застывшие целиком за этот шаг, и толщина их корки.
    pub frozen: Vec<(IVec2, f32)>,
    /// Застывшие ячейки, на которые натекла горячая лава, и толщина их корки.
    pub covered: Vec<(IVec2, f32)>,
}

impl StepResult {
    /// Изменились застывшие ячейки: пересобрать сетку корки.
    pub fn cooled_changed(&self) -> bool {
        !self.frozen.is_empty() || !self.covered.is_empty()
    }
}

const NEIGHBOURS: [IVec2; 4] = [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y];

impl Field {
    /// Высота под горячей лавой в точке сетки: рельеф и корка застывшей ячейки.
    fn base(&self, at: IVec2, height: &impl Fn(i32, i32) -> f32) -> f32 {
        height(at.x, at.y) + self.cooled.get(&at).copied().unwrap_or(0.0)
    }

    /// Поверхность в точке сетки: у горячей ячейки — с лавой, иначе рельеф и корка.
    pub fn surface_at(&self, at: IVec2, height: &impl Fn(i32, i32) -> f32) -> f32 {
        self.cells.get(&at).map_or_else(|| self.base(at, height), LavaCell::surface)
    }

    /// Ячейка шага: существующая или новая; застывшая отдаёт в неё свою корку.
    fn cell_mut(&mut self, at: IVec2, height: &impl Fn(i32, i32) -> f32, covered: &mut Vec<(IVec2, f32)>) -> &mut LavaCell {
        if !self.cells.contains_key(&at) {
            let crust = self.cooled.remove(&at).unwrap_or(0.0);
            if crust > 0.0 {
                covered.push((at, crust));
            }
            self.cells.insert(
                at,
                LavaCell {
                    crust,
                    ground: height(at.x, at.y),
                    ..default()
                },
            );
        }
        self.cells.get_mut(&at).expect("ячейка только что добавлена")
    }

    /// Добавляет горячую лаву объёмом `volume` м³ в ячейку `at`. Возвращает корку, если ячейка
    /// была застывшей (её объём переходит из застывших в горячие).
    #[cfg(test)]
    pub fn pour(&mut self, at: IVec2, volume: f32, height: impl Fn(i32, i32) -> f32) -> Option<f32> {
        let mut covered = Vec::new();
        self.cell_mut(at, &height, &mut covered).hot += volume;
        covered.first().map(|(_, crust)| *crust)
    }

    /// Толщина корки в точке сетки: у застывшей ячейки и под горячей лавой.
    #[cfg(test)]
    fn crust_at(&self, at: IVec2) -> f32 {
        self.cells.get(&at).map_or_else(|| self.cooled.get(&at).copied().unwrap_or(0.0), |c| c.crust)
    }

    /// Один шаг течения длиной `dt` секунд. `pours` — что источники вливают в этот шаг (ячейка,
    /// объём м³): заливка входит в шаг после снимка, поэтому новая горячая ячейка и подъём озера
    /// видны в итоге. `height` — рельеф, `allowed` — может ли ячейка принять лаву (предел удаления от
    /// прорыва), `pit` — ячейка ямы прорыва: такие не застывают.
    pub fn step(
        &mut self,
        dt: f32,
        pours: &[(IVec2, f32)],
        height: impl Fn(i32, i32) -> f32,
        allowed: impl Fn(IVec2) -> bool,
        pit: impl Fn(IVec2) -> bool,
    ) -> StepResult {
        let hot_before: HashSet<IVec2> = self.cells.iter().filter(|(_, c)| c.is_hot()).map(|(k, _)| *k).collect();
        // Рельеф под лавой мог измениться (кратер, провал): поверхность считается от текущего, а
        // сдвиг земли под лавой входит в `moved` — сетка ляжет на новый рельеф.
        let mut ground_moved = 0.0f32;
        for (at, cell) in self.cells.iter_mut() {
            let ground = height(at.x, at.y);
            ground_moved = ground_moved.max((ground - cell.ground).abs());
            cell.ground = ground;
        }
        let start: HashMap<IVec2, f32> = self.cells.iter().map(|(k, c)| (*k, c.hot)).collect();
        let mut covered = Vec::new();
        for (at, volume) in pours {
            self.cell_mut(*at, &height, &mut covered).hot += volume;
        }
        let substeps = (dt / SUBSTEP).round().max(1.0) as usize;
        for _ in 0..substeps {
            self.substep(&height, &allowed, &mut covered);
        }
        let moved = self
            .cells
            .iter()
            .map(|(at, cell)| (cell.hot - start.get(at).copied().unwrap_or(0.0)).abs())
            .fold(ground_moved, f32::max);

        // Лава за краем ямы остывает в корку, плёнка тоньше горячей застывает целиком. Крошки
        // пропадают, а на корке за краем ямы уходят в неё: иначе ячейка корки у кромки озера каждый
        // шаг принимала бы крошку, теряла её и снова застывала с той же высотой — без конца. На голой
        // земле крошка корки не даёт: не будет ореола корки в доли миллиметра вокруг потока.
        for (&at, cell) in self.cells.iter_mut() {
            if cell.hot < DUST {
                if !pit(at) && cell.crust > 0.0 {
                    cell.crust += cell.hot;
                }
                cell.hot = 0.0;
                continue;
            }
            if pit(at) {
                continue;
            }
            let cooled = if cell.hot - COOL_RATE * dt < MIN_DEPTH { cell.hot } else { COOL_RATE * dt };
            cell.hot -= cooled;
            cell.crust += cooled;
        }
        // Ячейки без горячей лавы уходят из шага: с коркой — в застывшие, пустые — совсем.
        let mut frozen = Vec::new();
        self.cells.retain(|at, cell| {
            if cell.hot > 0.0 {
                return true;
            }
            if cell.crust > 0.0 {
                frozen.push((*at, cell.crust));
            }
            false
        });
        for (at, crust) in &frozen {
            self.cooled.insert(*at, *crust);
        }
        // Ячейка, которую в этом шаге и закрыли, и снова заморозили почти той же коркой, не меняет
        // сетку корки: такая пара не считается изменением.
        let was: HashMap<IVec2, f32> = covered.iter().copied().collect();
        let same = |at: &IVec2, crust: f32| was.get(at).is_some_and(|old| (crust - old).abs() < CRUST_EPS);
        let now: HashMap<IVec2, f32> = frozen.iter().copied().collect();
        frozen.retain(|(at, crust)| !same(at, *crust));
        covered.retain(|(at, old)| !now.get(at).is_some_and(|crust| (crust - old).abs() < CRUST_EPS));

        let reshaped = self.cells.iter().filter(|(_, c)| c.is_hot()).count() != hot_before.len()
            || self.cells.iter().any(|(k, c)| c.is_hot() && !hot_before.contains(k));
        StepResult {
            reshaped,
            moved,
            frozen,
            covered,
        }
    }

    /// Подшаг труб: потоки разгоняются по старым поверхностям, не отдают больше, чем есть, потом
    /// объёмы переносятся разом.
    fn substep(&mut self, height: &impl Fn(i32, i32) -> f32, allowed: &impl Fn(IVec2) -> bool, covered: &mut Vec<(IVec2, f32)>) {
        let mut flows: Vec<(IVec2, [f32; 4])> = Vec::with_capacity(self.cells.len());
        for (&at, cell) in self.cells.iter() {
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
                let drop = here - self.surface_at(n, height);
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
            if let Some(cell) = self.cells.get_mut(&at) {
                cell.out = out;
            }
        }
        for (at, d) in delta {
            let cell = self.cell_mut(at, height, covered);
            cell.hot = (cell.hot + d).max(0.0);
        }
    }

    /// Сетка горячей лавы. Край уходит под верх того, что под ней: корки или рельефа.
    pub fn hot_mesh(&self, height: impl Fn(i32, i32) -> f32) -> Option<Mesh> {
        let mut squares: HashSet<IVec2> = HashSet::new();
        for (at, _) in self.cells.iter().filter(|(_, c)| c.is_hot()) {
            squares.extend(squares_at(*at));
        }
        layer_mesh(
            squares,
            |at| self.hot_top(at).map(|top| top + SURFACE_LIFT),
            |at| self.hot_base(at, &height),
        )
    }

    /// Поверхность горячей лавы в вершине, если она там есть.
    fn hot_top(&self, at: IVec2) -> Option<f32> {
        self.cells.get(&at).filter(|c| c.is_hot()).map(LavaCell::surface)
    }

    /// Что под горячей лавой в вершине: корка или рельеф.
    fn hot_base(&self, at: IVec2, height: &impl Fn(i32, i32) -> f32) -> f32 {
        self.cells.get(&at).map_or_else(|| self.base(at, height), |c| c.ground + c.crust)
    }

    /// Высота поверхности горячей лавы над тем, что под ней (корка или рельеф), в точке плана —
    /// как у её сетки (без подъёма над рельефом): поверхность и основание между вершинами по четырём
    /// углам квадрата, сухой угол уходит под основание. Больше нуля — горячая лава там видна и жжёт;
    /// у стенки ямы выше глади и у края корки — меньше нуля. `None` — ни в одном углу квадрата горячей
    /// лавы нет.
    pub fn hot_depth_at(&self, x: f32, z: f32, height: impl Fn(i32, i32) -> f32) -> Option<f32> {
        let square = IVec2::new(x.floor() as i32, z.floor() as i32);
        let base = |at: IVec2| self.hot_base(at, &height);
        let tops = square_corners(square, |at| self.hot_top(at), base)?;
        let bases = CORNERS.map(|o| base(square + o));
        let (fx, fz) = (x - square.x as f32, z - square.y as f32);
        let lerp = |[c00, c10, c11, c01]: [f32; 4]| {
            c00 * (1.0 - fx) * (1.0 - fz) + c10 * fx * (1.0 - fz) + c11 * fx * fz + c01 * (1.0 - fx) * fz
        };
        Some(lerp(tops) - lerp(bases))
    }

    /// Рельеф изменился: земля под горячей лавой в круге сразу берётся новая (шаг течения сделал бы
    /// это только через до 0,2 с), чтобы сетка и жжение в этом кадре шли по новому рельефу.
    pub fn refresh_ground(&mut self, center: IVec2, radius: i32, height: impl Fn(i32, i32) -> f32) {
        for (at, cell) in self.cells.iter_mut() {
            if (*at - center).length_squared() <= radius * radius {
                cell.ground = height(at.x, at.y);
            }
        }
    }

    /// Сетка застывших ячеек в квадрате плана `tile` (стороной [`CRUST_TILE`] м): корка копится до
    /// конца забега, и после шага пересобираются только квадраты, где что-то застыло или натекло.
    /// Корка под горячей лавой закрыта ею и не рисуется.
    pub fn crust_mesh(&self, tile: IVec2, height: impl Fn(i32, i32) -> f32) -> Option<Mesh> {
        let lo = tile * CRUST_TILE;
        let squares = (0..CRUST_TILE).flat_map(move |dz| (0..CRUST_TILE).map(move |dx| lo + IVec2::new(dx, dz)));
        layer_mesh(
            squares,
            |at| self.cooled.get(&at).map(|crust| height(at.x, at.y) + crust + CRUST_LIFT),
            |at| height(at.x, at.y),
        )
    }
}

/// Изменение корки меньше этого, м, сетку корки не пересобирает.
const CRUST_EPS: f32 = 0.005;

/// Сторона квадрата плана для сетки корки, м.
pub const CRUST_TILE: i32 = 32;

/// Квадраты сетки (по левому нижнему углу), у которых точка `at` — один из углов.
fn squares_at(at: IVec2) -> impl Iterator<Item = IVec2> {
    (-1..=0).flat_map(move |dz| (-1..=0).map(move |dx| at + IVec2::new(dx, dz)))
}

/// Квадраты корки, сетку которых меняет изменение в точке `at`.
pub fn crust_tiles_at(at: IVec2) -> impl Iterator<Item = IVec2> {
    squares_at(at).map(|s| s.div_euclid(IVec2::splat(CRUST_TILE)))
}

/// Углы квадрата плана по порядку обхода.
const CORNERS: [IVec2; 4] = [IVec2::new(0, 0), IVec2::new(1, 0), IVec2::new(1, 1), IVec2::new(0, 1)];

/// Высоты углов квадрата `square` (по [`CORNERS`]) для слоя: угол со слоем — на его поверхности
/// (`top`), угол без слоя — под тем, что под слоем (`base`), на [`EDGE_SINK`], но не выше средней
/// поверхности углов со слоем. `None` — слоя нет ни в одном углу.
fn square_corners(square: IVec2, top: impl Fn(IVec2) -> Option<f32>, base: impl Fn(IVec2) -> f32) -> Option<[f32; 4]> {
    let tops: [Option<f32>; 4] = CORNERS.map(|o| top(square + o));
    let (sum, count) = tops.iter().flatten().fold((0.0, 0), |(s, n), h| (s + h, n + 1));
    if count == 0 {
        return None;
    }
    let mean = sum / count as f32;
    let mut heights = [0.0; 4];
    for (i, (o, top)) in CORNERS.iter().zip(tops).enumerate() {
        heights[i] = top.unwrap_or_else(|| (base(square + *o) - EDGE_SINK).min(mean));
    }
    Some(heights)
}

/// Сетка слоя по рельефу из квадратов `squares` (по левому нижнему углу): квадрат рисуется, если
/// слой есть хотя бы в одном углу. Угол со слоем (`top`) стоит на его поверхности, угол без слоя — под тем, что под слоем
/// (`base`), на [`EDGE_SINK`], но не выше средней поверхности углов со слоем: у края озера в яме гладь
/// продолжается под стенку (её закрывает рельеф), а край потока на склоне уходит в землю.
/// `None` — слоя нет нигде.
fn layer_mesh(
    squares: impl IntoIterator<Item = IVec2>,
    top: impl Fn(IVec2) -> Option<f32>,
    base: impl Fn(IVec2) -> f32,
) -> Option<Mesh> {
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    for square in squares {
        let Some(heights) = square_corners(square, &top, &base) else {
            continue;
        };
        let first = positions.len() as u32;
        for (o, y) in CORNERS.iter().zip(heights) {
            let v = square + *o;
            positions.push([v.x as f32, y, v.y as f32]);
            uvs.push([v.x as f32 / 8.0, v.y as f32 / 8.0]);
        }
        indices.extend_from_slice(&[first, first + 2, first + 1, first, first + 3, first + 2]);
    }
    if positions.is_empty() {
        return None;
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

    fn volume(field: &Field) -> f32 {
        field.cells.values().map(|c| c.hot + c.crust).sum::<f32>() + field.cooled.values().sum::<f32>()
    }

    /// Шаги, пока лава не перестанет двигаться (или `limit` шагов). Застывания нет.
    fn settle(field: &mut Field, height: impl Fn(i32, i32) -> f32 + Copy, allowed: impl Fn(IVec2) -> bool + Copy, limit: usize) -> usize {
        for i in 0..limit {
            let r = field.step(FLOW_STEP, &[], height, allowed, |_| true);
            if r.moved < 1e-4 && !r.reshaped {
                return i;
            }
        }
        limit
    }

    #[test]
    fn объём_сохраняется() {
        let height = |x: i32, z: i32| -0.3 * x as f32 + 0.05 * z as f32;
        let mut field = Field::default();
        field.pour(IVec2::ZERO, 20.0, height);
        settle(&mut field, height, |_| true, 400);
        let v = volume(&field);
        // Пропадают только крошки тоньше DUST.
        assert!((v - 20.0).abs() < 0.2, "объём {v}");
    }

    #[test]
    fn стекает_по_склону_и_не_течёт_вверх() {
        // Плоскость падает к +x.
        let height = |x: i32, _: i32| -0.5 * x as f32;
        let mut field = Field::default();
        // Источник подаёт понемногу, как место прорыва: разом налитый столб растёкся бы во все стороны.
        for i in 0..100 {
            if i < 20 {
                field.pour(IVec2::ZERO, 0.5, height);
            }
            field.step(FLOW_STEP, &[], height, |_| true, |_| true);
        }
        let hot: Vec<IVec2> = field.cells.iter().filter(|(_, c)| c.hot > DUST).map(|(k, _)| *k).collect();
        assert!(hot.iter().any(|c| c.x > 10), "лава ушла вниз по склону");
        assert!(hot.iter().all(|c| c.x >= -1), "вверх по склону лава не идёт: {:?}", hot.iter().min_by_key(|c| c.x));
        let (sum, n) = field.cells.iter().fold((0.0, 0.0), |(s, n), (k, c)| (s + k.x as f32 * c.hot, n + c.hot));
        assert!(sum / n > 5.0, "центр масс внизу склона");
    }

    #[test]
    fn на_ровном_месте_останавливается_слоем() {
        let height = |_: i32, _: i32| 0.0;
        let mut field = Field::default();
        field.pour(IVec2::ZERO, 12.0, height);
        let steps = settle(&mut field, height, |_| true, 2000);
        assert!(steps < 2000, "лава остановилась");
        let reach = field.cells.keys().map(|c| c.as_vec2().length()).fold(0.0, f32::max);
        assert!(reach < 10.0, "растеклась не бесконечно: {reach}");
        let max = field.cells.values().map(|c| c.hot).fold(0.0, f32::max);
        assert!(max < 1.0, "слой, а не столб: {max}");
        // Край — слой порядка √YIELD_K, а не плёнка и не стена.
        let edge = field
            .cells
            .iter()
            .filter(|(k, _)| NEIGHBOURS.iter().any(|o| !field.cells.contains_key(&(**k + *o))))
            .map(|(_, c)| c.hot)
            .fold(f32::MAX, f32::min);
        assert!(edge > 0.05 && edge < 0.4, "толщина края {edge}");
    }

    #[test]
    fn глубокое_озеро_ровное() {
        // Чаша 7×7 м глубиной 10 м.
        let height = |x: i32, z: i32| if x.abs() <= 3 && z.abs() <= 3 { -10.0 } else { 0.0 };
        let mut field = Field::default();
        field.pour(IVec2::ZERO, 49.0 * 5.0, height);
        settle(&mut field, height, |_| true, 2000);
        let tops: Vec<f32> = field.cells.values().map(LavaCell::surface).collect();
        let (lo, hi) = tops.iter().fold((f32::MAX, f32::MIN), |(a, b), t| (a.min(*t), b.max(*t)));
        assert!(hi - lo < 0.1, "гладь ровная: {lo}..{hi}");
        assert!((hi + 5.0).abs() < 0.1, "уровень на половине глубины: {hi}");
    }

    #[test]
    fn не_выходит_за_предел() {
        let height = |_: i32, _: i32| 0.0;
        let mut field = Field::default();
        field.pour(IVec2::ZERO, 30.0, height);
        settle(&mut field, height, |c| c.length_squared() <= 9, 2000);
        assert!(field.cells.keys().all(|c| c.length_squared() <= 9));
    }

    #[test]
    fn лава_за_краем_ямы_застывает_а_в_яме_нет() {
        // Яма 3×3 глубиной 2 м; налито больше, чем она вмещает, — излишек растекается вокруг.
        let pit = |c: IVec2| c.x.abs() <= 1 && c.y.abs() <= 1;
        let height = |x: i32, z: i32| if pit(IVec2::new(x, z)) { -2.0 } else { 0.0 };
        let mut field = Field::default();
        field.pour(IVec2::ZERO, 9.0 * 2.0 + 6.0, height);
        let mut reshaped = false;
        for _ in 0..(60.0 / FLOW_STEP) as usize {
            reshaped |= field.step(FLOW_STEP, &[], height, |_| true, pit).reshaped;
        }
        assert!(reshaped);
        assert!(field.cells.iter().filter(|(k, _)| pit(**k)).all(|(_, c)| c.is_hot()), "яма не застывает");
        // Застывшие ячейки ушли из шага: в нём осталась только яма.
        assert!(field.cells.keys().all(|k| pit(*k)), "за краем горячей лавы нет");
        assert!(!field.cooled.is_empty(), "вокруг корка");
        assert!(field.cooled.keys().all(|k| !pit(*k)));
    }

    #[test]
    fn горячая_лава_течёт_поверх_корки() {
        let height = |x: i32, _: i32| -0.5 * x as f32;
        let mut field = Field::default();
        // Корка толщиной 1 м на склоне у места излива.
        for x in 1..=3 {
            field.cells.insert(IVec2::new(x, 0), LavaCell { crust: 1.0, ground: height(x, 0), ..default() });
        }
        field.pour(IVec2::ZERO, 5.0, height);
        for _ in 0..60 {
            field.step(FLOW_STEP, &[], height, |_| true, |_| true);
        }
        assert!(field.cells.iter().any(|(k, c)| k.x > 3 && c.hot > DUST), "лава перетекла через корку");
        assert!((1..=3).all(|x| field.crust_at(IVec2::new(x, 0)) >= 1.0), "корка на месте");
    }

    #[test]
    fn поверхность_горячей_лавы_между_вершинами_как_у_сетки() {
        // Склон 0,73 м/м, сплошной поток толщиной 0,2 м.
        let height = |x: i32, _: i32| -0.73 * x as f32;
        let ground = |x: f32| -0.73 * x;
        let mut field = Field::default();
        for x in 5..15 {
            for z in -2..=2 {
                field.cells.insert(IVec2::new(x, z), LavaCell { hot: 0.2, ground: height(x, z), ..default() });
            }
        }
        let _ = ground;
        for x in [9.6, 9.9, 10.1, 10.4] {
            let d = field.hot_depth_at(x, 0.3, height).expect("лава");
            assert!(d > 0.15, "в x = {x} лава над землёй на {d}");
        }
        // Чаша: сухая стенка выше глади — лавы там нет.
        let bowl = |x: i32, _: i32| if x <= 0 { -2.0 } else { 0.0 };
        let mut lake = Field::default();
        lake.cells.insert(IVec2::ZERO, LavaCell { hot: 1.0, ground: -2.0, ..default() });
        assert!(lake.hot_depth_at(0.2, 0.0, bowl).expect("угол с лавой") > 0.0, "у горячей вершины жжёт");
        assert!(lake.hot_depth_at(0.8, 0.0, bowl).expect("угол с лавой") < 0.0, "на стенке выше глади не жжёт");
        assert!(lake.hot_depth_at(5.5, 0.0, bowl).is_none());
        // Край потока у застывшей корки 0,3 м: у застывших вершин лава ушла под корку и не жжёт.
        let flat = |_: i32, _: i32| 0.0;
        let mut edge = Field::default();
        edge.cells.insert(IVec2::ZERO, LavaCell { hot: 0.1, crust: 0.3, ground: 0.0, ..default() });
        for at in [IVec2::X, IVec2::ONE, IVec2::Y] {
            edge.cooled.insert(at, 0.3);
        }
        assert!(edge.hot_depth_at(0.1, 0.1, flat).expect("лава") > 0.0, "у горячей вершины жжёт");
        assert!(edge.hot_depth_at(0.95, 0.95, flat).expect("угол") < 0.0, "на корке не жжёт");
    }

    #[test]
    fn заливка_источника_видна_в_итоге_шага() {
        let height = |_: i32, _: i32| 0.0;
        let mut field = Field::default();
        let r = field.step(FLOW_STEP, &[(IVec2::ZERO, 1.0)], height, |_| true, |_| true);
        assert!(r.reshaped, "новая горячая ячейка от заливки");
        assert!(r.moved > 0.0, "заливка меняет толщину");
    }

    #[test]
    fn корка_у_кромки_озера_не_пересобирается_без_конца() {
        // Чаша 3×3 глубиной 1 м: яма не остывает, вокруг ровно. Источник держит уровень у кромки.
        let pit = |c: IVec2| c.x.abs() <= 1 && c.y.abs() <= 1;
        let height = move |x: i32, z: i32| if pit(IVec2::new(x, z)) { -1.0 } else { 0.0 };
        let mut field = Field::default();
        let mut busy = 0;
        for i in 0..1500 {
            let room = (0.05 - field.surface_at(IVec2::ZERO, &height)).max(0.0);
            let r = field.step(FLOW_STEP, &[(IVec2::ZERO, room)], height, |c| c.length_squared() <= 100, pit);
            if i >= 1000 && r.cooled_changed() {
                busy += 1;
            }
        }
        assert!(!field.cooled.is_empty(), "вокруг застыла корка");
        assert!(busy < 25, "корка давно стоит, а меняется в {busy} шагах из 500");
    }

    #[test]
    fn застывшие_ячейки_не_входят_в_шаг() {
        let height = |_: i32, _: i32| 0.0;
        let mut field = Field::default();
        // Большое застывшее поле в стороне и немного горячей лавы.
        for x in 100..200 {
            for z in 0..100 {
                field.cooled.insert(IVec2::new(x, z), 0.5);
            }
        }
        field.pour(IVec2::ZERO, 2.0, height);
        let r = field.step(FLOW_STEP, &[], height, |_| true, |_| true);
        assert!(field.cells.len() < 20, "в шаге только горячие: {}", field.cells.len());
        assert!(!r.cooled_changed(), "застывшие не тронуты");
        assert_eq!(field.cooled.len(), 10_000);
    }

    #[test]
    fn сетка_лежит_на_поверхности_и_уходит_под_землю_по_краю() {
        let height = |x: i32, _: i32| 0.2 * x as f32;
        let mut field = Field::default();
        field.cells.insert(IVec2::ZERO, LavaCell { hot: 0.3, ground: 0.0, ..default() });
        let mesh = field.hot_mesh(height).expect("сетка");
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
        assert!(field.crust_mesh(IVec2::ZERO, height).is_none(), "корки нет");
        assert!(field.crust_mesh(IVec2::splat(-1), height).is_none());
    }
}
