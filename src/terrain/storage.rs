//! Хранилище участков рельефа на весь забег и запросы высоты и нормали в точке мира.

use std::collections::HashMap;

use bevy::prelude::*;

use super::generator::HeightGenerator;

/// Сторона квадратного участка, м.
pub const CHUNK_SIZE: i32 = 32;

/// Число вершин вдоль стороны участка: шаг сетки 1 м, крайние вершины общие с соседом.
pub const CHUNK_VERTS: usize = CHUNK_SIZE as usize + 1;

/// Цвет травы в низинах.
const GRASS_LOW: Color = Color::srgb(0.13, 0.36, 0.10);

/// Цвет травы на вершинах.
const GRASS_HIGH: Color = Color::srgb(0.42, 0.66, 0.22);

/// Высота, на которой трава самая тёмная, м.
const GRASS_LOW_HEIGHT: f32 = -6.0;

/// Высота, на которой трава самая светлая, м.
const GRASS_HIGH_HEIGHT: f32 = 14.0;

/// Насколько пятна шума меняют яркость травы.
const GRASS_TINT_STRENGTH: f32 = 0.12;

/// Шаг, на котором считается нормаль в точке мира, м.
const NORMAL_STEP: f32 = 1.0;

/// Полуширина вала в долях радиуса чаши: гребень на радиусе чаши, вал сходит на нет к `1 + это`.
/// Вал мортиры (R = 5 м) получается шириной 5 м, и на каждом луче его ловят несколько вершин
/// сетки с шагом 1 м: гребень ровный, а не зубчатый.
const CRATER_RIM_HALF_WIDTH: f32 = 0.5;

/// Насколько пятна шума сдвигают границу коричневой земли, в долях полуширины вала.
const CRATER_EDGE_JITTER: f32 = 0.35;

/// Цвет разрушенной земли в кратере.
const CRATER_BROWN: Color = Color::srgb(0.36, 0.23, 0.12);

/// Насколько пятна шума меняют яркость земли в кратере.
const CRATER_TINT_STRENGTH: f32 = 0.18;

/// Изменение высоты кратером на расстоянии `x` от центра в радиусах чаши.
///
/// Чаша `-depth · (1 - x²)²` внутри `x < 1` плавно, без излома, сходит на нет к краю.
/// Вал `rim · (1 - u²)²`, где `u = (x - 1) / CRATER_RIM_HALF_WIDTH`, — горб с гребнем на краю
/// чаши. Обе части гладкие вместе с наклоном, поэтому на сетке нет ступенек и изломов.
pub fn crater_profile(x: f32, depth: f32, rim: f32) -> f32 {
    let bowl = if x < 1.0 { -depth * (1.0 - x * x).powi(2) } else { 0.0 };
    let u = (x - 1.0) / CRATER_RIM_HALF_WIDTH;
    let bank = if u.abs() < 1.0 { rim * (1.0 - u * u).powi(2) } else { 0.0 };
    bowl + bank
}

/// Состояние одного участка: высоты и цвета вершин сетки `CHUNK_VERTS` × `CHUNK_VERTS`.
///
/// Вершина `(ix, iz)` стоит в точке мира `(origin.x + ix, height, origin.y + iz)`, где
/// `origin = chunk_origin(coord)`. Крайние вершины хранятся в обоих соседних участках,
/// поэтому, меняя рельеф на границе, нужно писать в оба участка.
#[derive(Clone, Debug, PartialEq)]
pub struct ChunkData {
    /// Высоты вершин, м, по строкам вдоль X: индекс `ChunkData::index(ix, iz)`.
    pub heights: Vec<f32>,
    /// Цвета вершин в линейном RGBA, тот же порядок, что у `heights`.
    pub colors: Vec<[f32; 4]>,
}

impl ChunkData {
    /// Индекс вершины `(ix, iz)` в массивах участка.
    pub fn index(ix: usize, iz: usize) -> usize {
        iz * CHUNK_VERTS + ix
    }

    /// Высота вершины `(ix, iz)`.
    pub fn height(&self, ix: usize, iz: usize) -> f32 {
        self.heights[Self::index(ix, iz)]
    }

    /// Генерирует нетронутый участок из шума.
    fn generate(coord: IVec2, generator: &HeightGenerator) -> Self {
        let origin = chunk_origin(coord);
        let low = GRASS_LOW.to_linear();
        let high = GRASS_HIGH.to_linear();
        let mut heights = Vec::with_capacity(CHUNK_VERTS * CHUNK_VERTS);
        let mut colors = Vec::with_capacity(CHUNK_VERTS * CHUNK_VERTS);
        for iz in 0..CHUNK_VERTS {
            for ix in 0..CHUNK_VERTS {
                let x = origin.x + ix as f32;
                let z = origin.y + iz as f32;
                let h = generator.height(x, z);
                let t = ((h - GRASS_LOW_HEIGHT) / (GRASS_HIGH_HEIGHT - GRASS_LOW_HEIGHT))
                    .clamp(0.0, 1.0);
                let brightness = 1.0 + generator.tint(x, z) * GRASS_TINT_STRENGTH;
                let c = low.mix(&high, t) * brightness;
                heights.push(h);
                colors.push([c.red, c.green, c.blue, 1.0]);
            }
        }
        Self { heights, colors }
    }
}

/// Мировые координаты угла участка: `(x, z)` вершины `(0, 0)`.
pub fn chunk_origin(coord: IVec2) -> Vec2 {
    (coord * CHUNK_SIZE).as_vec2()
}

/// Участок, в который попадает точка мира.
pub fn chunk_coord_at(x: f32, z: f32) -> IVec2 {
    IVec2::new(
        (x.floor() as i32).div_euclid(CHUNK_SIZE),
        (z.floor() as i32).div_euclid(CHUNK_SIZE),
    )
}

/// Участок и локальная вершина для вершины мировой сетки `(vx, vz)`.
fn split_vertex(vx: i32, vz: i32) -> (IVec2, usize, usize) {
    let coord = IVec2::new(vx.div_euclid(CHUNK_SIZE), vz.div_euclid(CHUNK_SIZE));
    (
        coord,
        vx.rem_euclid(CHUNK_SIZE) as usize,
        vz.rem_euclid(CHUNK_SIZE) as usize,
    )
}

/// Все участки, где хранится вершина мировой сетки `(vx, vz)`, с локальными индексами в них.
///
/// Вершина на границе участков хранится и в соседе как его крайняя (индекс `CHUNK_SIZE`):
/// всего от одного до четырёх участков.
fn vertex_owners(vx: i32, vz: i32) -> impl Iterator<Item = (IVec2, usize, usize)> {
    let (coord, ix, iz) = split_vertex(vx, vz);
    let last = CHUNK_SIZE as usize;
    let xs = [(0, ix), (-1, last)];
    let zs = [(0, iz), (-1, last)];
    let x_count = if ix == 0 { 2 } else { 1 };
    let z_count = if iz == 0 { 2 } else { 1 };
    zs.into_iter().take(z_count).flat_map(move |(dz, lz)| {
        xs.into_iter()
            .take(x_count)
            .map(move |(dx, lx)| (coord + IVec2::new(dx, dz), lx, lz))
    })
}

/// Рельеф забега: все участки, к которым обращались, живут до конца забега.
///
/// Единственный источник высоты поверхности для лунохода, врагов, снарядов и объектов.
/// Чтобы изменить рельеф, пишите в `chunk_mut` и вешайте `ChunkDirty` на сущность меша
/// участка (и соседей по краю), см. `VisibleChunks`.
#[derive(Resource)]
pub struct Terrain {
    /// Сид забега.
    seed: u32,
    /// Шумы нетронутого рельефа.
    generator: HeightGenerator,
    /// Сгенерированные участки по координатам.
    chunks: HashMap<IVec2, ChunkData>,
}

impl Terrain {
    /// Пустое хранилище нового забега.
    pub fn new(seed: u32) -> Self {
        Self {
            seed,
            generator: HeightGenerator::new(seed),
            chunks: HashMap::new(),
        }
    }

    /// Сид забега.
    pub fn seed(&self) -> u32 {
        self.seed
    }

    /// Линии железной дороги забега.
    pub fn lines(&self) -> &crate::railway::Lines {
        self.generator.lines()
    }

    /// Уровень путей на линии в параметре `s`.
    pub fn track_level(&self, line: crate::railway::LineId, s: f32) -> f32 {
        self.generator
            .lines()
            .level(line, s, |x, z| self.generator.hills_height(x, z))
    }

    /// Нетронутая высота без путей: по ней видно, где над путями гора.
    pub fn raw_height(&self, x: f32, z: f32) -> f32 {
        self.generator.raw_height(x, z)
    }

    /// Данные участка, генерирует его при первом обращении.
    pub fn chunk(&mut self, coord: IVec2) -> &ChunkData {
        self.chunk_mut(coord)
    }

    /// Изменяемые данные участка, генерирует его при первом обращении.
    pub fn chunk_mut(&mut self, coord: IVec2) -> &mut ChunkData {
        let generator = &self.generator;
        self.chunks
            .entry(coord)
            .or_insert_with(|| ChunkData::generate(coord, generator))
    }

    /// Данные участка, если он уже сгенерирован.
    pub fn get_chunk(&self, coord: IVec2) -> Option<&ChunkData> {
        self.chunks.get(&coord)
    }

    /// Высота вершины мировой сетки `(vx, vz)` (шаг 1 м).
    ///
    /// Не сгенерированный участок считается нетронутым и берётся прямо из шума:
    /// это ровно те высоты, которые он получит при генерации.
    pub fn vertex_height(&self, vx: i32, vz: i32) -> f32 {
        let (coord, ix, iz) = split_vertex(vx, vz);
        match self.chunks.get(&coord) {
            Some(chunk) => chunk.height(ix, iz),
            None => self.generator.height(vx as f32, vz as f32),
        }
    }

    /// Высота поверхности в точке мира: билинейная интерполяция четырёх соседних вершин.
    pub fn height_at(&self, x: f32, z: f32) -> f32 {
        let (fx, fz) = (x.floor(), z.floor());
        let (tx, tz) = (x - fx, z - fz);
        let (vx, vz) = (fx as i32, fz as i32);
        // Вершины vx..=vx+1 всегда лежат в одном участке: локальный индекс vx не больше 31.
        let (coord, ix, iz) = split_vertex(vx, vz);
        let [h00, h10, h01, h11] = match self.chunks.get(&coord) {
            Some(c) => [
                c.height(ix, iz),
                c.height(ix + 1, iz),
                c.height(ix, iz + 1),
                c.height(ix + 1, iz + 1),
            ],
            None => {
                let g = &self.generator;
                let (x0, z0) = (fx, fz);
                [
                    g.height(x0, z0),
                    g.height(x0 + 1.0, z0),
                    g.height(x0, z0 + 1.0),
                    g.height(x0 + 1.0, z0 + 1.0),
                ]
            }
        };
        let near = h00 + (h10 - h00) * tx;
        let far = h01 + (h11 - h01) * tx;
        near + (far - near) * tz
    }

    /// Вырывает кратер с центром в точке мира `(center.x, center.z)` и возвращает участки,
    /// данные которых изменились.
    ///
    /// К высоте каждой вершины прибавляется гладкий профиль `crater_profile`: чаша глубиной `depth`
    /// и вал высотой `rim` с гребнем на радиусе `R`. Профиль прибавляется к текущим высотам,
    /// поэтому повторный взрыв углубляет кратер без предела.
    /// Цвет вершин смешивается с коричневым: внутри чаши полностью, на валу с плавным переходом
    /// к исходному цвету, граница слегка неровная по шуму. Общие вершины на границах пишутся
    /// во все участки, где они хранятся.
    pub fn apply_crater(&mut self, center: Vec3, radius: f32, depth: f32, rim: f32) -> Vec<IVec2> {
        let mut changed = Vec::new();
        if radius <= 0.0 {
            return changed;
        }
        let outer = radius * (1.0 + CRATER_RIM_HALF_WIDTH);
        let (x0, x1) = ((center.x - outer).floor() as i32, (center.x + outer).ceil() as i32);
        let (z0, z1) = ((center.z - outer).floor() as i32, (center.z + outer).ceil() as i32);
        let brown = CRATER_BROWN.to_linear();
        for vz in z0..=z1 {
            for vx in x0..=x1 {
                let d = Vec2::new(vx as f32 - center.x, vz as f32 - center.z).length();
                if d >= outer {
                    continue;
                }
                let delta = crater_profile(d / radius, depth, rim);
                let tint = self.generator.tint(vx as f32, vz as f32);
                // Коричневый до гребня вала, дальше плавно уходит в исходный цвет к краю вала.
                let t = (d / radius - 1.0) / CRATER_RIM_HALF_WIDTH + tint * CRATER_EDGE_JITTER;
                let t = t.clamp(0.0, 1.0);
                let scorch = 1.0 - t * t * (3.0 - 2.0 * t);
                let variation = 1.0 + tint * CRATER_TINT_STRENGTH;
                let target = brown * variation;
                for (coord, ix, iz) in vertex_owners(vx, vz) {
                    let chunk = self.chunk_mut(coord);
                    let i = ChunkData::index(ix, iz);
                    chunk.heights[i] += delta;
                    let c = &mut chunk.colors[i];
                    // Смешение только в сторону коричневого: повторный взрыв зелень не вернёт.
                    c[0] += (target.red - c[0]) * scorch;
                    c[1] += (target.green - c[1]) * scorch;
                    c[2] += (target.blue - c[2]) * scorch;
                    if !changed.contains(&coord) {
                        changed.push(coord);
                    }
                }
            }
        }
        changed
    }

    /// Меняет высоту и цвет вершины мировой сетки `(vx, vz)` во всех участках, где она хранится,
    /// и возвращает эти участки. `edit` получает высоту и линейный цвет и вызывается для каждой
    /// копии вершины с одинаковыми исходными значениями, поэтому копии не расходятся.
    pub fn edit_vertex(&mut self, vx: i32, vz: i32, mut edit: impl FnMut(&mut f32, &mut [f32; 4])) -> Vec<IVec2> {
        let mut changed = Vec::new();
        for (coord, ix, iz) in vertex_owners(vx, vz) {
            let chunk = self.chunk_mut(coord);
            let i = ChunkData::index(ix, iz);
            edit(&mut chunk.heights[i], &mut chunk.colors[i]);
            changed.push(coord);
        }
        changed
    }

    /// Нормаль поверхности в точке мира по разности высот вокруг неё.
    pub fn normal_at(&self, x: f32, z: f32) -> Vec3 {
        let dx = self.height_at(x + NORMAL_STEP, z) - self.height_at(x - NORMAL_STEP, z);
        let dz = self.height_at(x, z + NORMAL_STEP) - self.height_at(x, z - NORMAL_STEP);
        Vec3::new(-dx, 2.0 * NORMAL_STEP, -dz).normalize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn height_between_vertices_lies_between_vertex_heights() {
        let mut terrain = Terrain::new(11);
        terrain.chunk(IVec2::ZERO);
        terrain.chunk(IVec2::new(-1, -1));
        for &(x, z) in &[(3.25, 7.5), (0.9, 0.1), (31.5, 31.5), (-0.5, -12.75), (-31.9, -0.3)] {
            let (vx, vz) = (f32::floor(x) as i32, f32::floor(z) as i32);
            let corners = [
                terrain.vertex_height(vx, vz),
                terrain.vertex_height(vx + 1, vz),
                terrain.vertex_height(vx, vz + 1),
                terrain.vertex_height(vx + 1, vz + 1),
            ];
            let min = corners.iter().copied().fold(f32::MAX, f32::min);
            let max = corners.iter().copied().fold(f32::MIN, f32::max);
            let h = terrain.height_at(x, z);
            assert!(h >= min - 1e-4 && h <= max + 1e-4, "{h} вне [{min}, {max}] в ({x}, {z})");
        }
    }

    #[test]
    fn height_at_vertex_equals_vertex_height() {
        let mut terrain = Terrain::new(5);
        terrain.chunk(IVec2::new(1, -2));
        let h = terrain.height_at(40.0, -50.0);
        assert_eq!(h, terrain.chunk(IVec2::new(1, -2)).height(8, 14));
    }

    #[test]
    fn repeated_chunk_request_returns_same_data() {
        let mut terrain = Terrain::new(3);
        let first = terrain.chunk(IVec2::new(2, -1)).clone();
        terrain.chunk(IVec2::new(5, 5));
        assert_eq!(&first, terrain.chunk(IVec2::new(2, -1)));
        assert_eq!(terrain.chunks.len(), 2);
    }

    #[test]
    fn changed_chunk_keeps_changes() {
        let mut terrain = Terrain::new(3);
        terrain.chunk_mut(IVec2::ZERO).heights[ChunkData::index(4, 4)] = -100.0;
        assert_eq!(terrain.vertex_height(4, 4), -100.0);
        assert_eq!(terrain.height_at(4.0, 4.0), -100.0);
    }

    #[test]
    fn height_without_chunk_matches_generated_chunk() {
        let mut terrain = Terrain::new(9);
        let before = terrain.height_at(-20.3, 47.8);
        terrain.chunk(chunk_coord_at(-20.3, 47.8));
        assert_eq!(before, terrain.height_at(-20.3, 47.8));
    }

    #[test]
    fn shared_edge_vertices_match() {
        let mut terrain = Terrain::new(1);
        let left = terrain.chunk(IVec2::ZERO).clone();
        let right = terrain.chunk(IVec2::X).clone();
        for iz in 0..CHUNK_VERTS {
            assert_eq!(left.height(CHUNK_VERTS - 1, iz), right.height(0, iz));
        }
    }

    #[test]
    fn repeated_explosion_deepens_center() {
        let mut terrain = Terrain::new(7);
        let center = Vec3::new(10.0, 0.0, 10.0);
        let mut previous = terrain.height_at(center.x, center.z);
        for _ in 0..10 {
            terrain.apply_crater(center, 5.0, 2.5, 0.6);
            let h = terrain.height_at(center.x, center.z);
            assert!(h < previous - 2.0, "центр не углубился: {h} после {previous}");
            previous = h;
        }
    }

    #[test]
    fn crater_heights_match_on_shared_border() {
        let mut terrain = Terrain::new(2);
        // Центр на углу четырёх участков: кратер задевает все четыре.
        let changed = terrain.apply_crater(Vec3::new(32.0, 0.0, 32.0), 5.0, 2.5, 0.6);
        assert_eq!(changed.len(), 4);
        let n = CHUNK_VERTS - 1;
        let pairs = [
            (IVec2::new(0, 0), IVec2::new(1, 0)),
            (IVec2::new(0, 1), IVec2::new(1, 1)),
        ];
        for (left, right) in pairs {
            let (l, r) = (terrain.chunk(left).clone(), terrain.chunk(right).clone());
            for iz in 0..CHUNK_VERTS {
                assert_eq!(l.height(n, iz), r.height(0, iz));
                assert_eq!(l.colors[ChunkData::index(n, iz)], r.colors[ChunkData::index(0, iz)]);
            }
        }
        let (bottom, top) = (terrain.chunk(IVec2::new(1, 0)).clone(), terrain.chunk(IVec2::ONE).clone());
        for ix in 0..CHUNK_VERTS {
            assert_eq!(bottom.height(ix, n), top.height(ix, 0));
        }
        // Угловая вершина одна на четыре участка и опущена на полную глубину.
        let pristine = HeightGenerator::new(2).height(32.0, 32.0);
        assert!((terrain.vertex_height(32, 32) - (pristine - 2.5)).abs() < 1e-4);
    }

    #[test]
    fn crater_has_rim_and_brown_bowl() {
        let mut terrain = Terrain::new(4);
        let before = terrain.clone_heights_around(50, 50);
        terrain.apply_crater(Vec3::new(50.0, 0.0, 50.0), 5.0, 2.5, 0.6);
        let after = terrain.clone_heights_around(50, 50);
        // Вал: на расстоянии 6 м от центра земля поднялась.
        assert!(after[1] > before[1] + 0.3);
        // Дальше вала рельеф не тронут.
        assert_eq!(after[2], before[2]);
        let brown = CRATER_BROWN.to_linear();
        let (coord, ix, iz) = split_vertex(50, 50);
        let c = terrain.chunk(coord).colors[ChunkData::index(ix, iz)];
        assert!(c[0] > c[1] && (c[0] - brown.red).abs() < 0.1, "центр не коричневый: {c:?}");
    }

    #[test]
    fn crater_profile_is_smooth() {
        let (depth, rim) = (2.5, 0.6);
        let outer = 1.0 + CRATER_RIM_HALF_WIDTH;
        assert!((crater_profile(0.0, depth, rim) + depth).abs() < 1e-6, "центр на полной глубине");
        assert_eq!(crater_profile(outer, depth, rim), 0.0, "за валом рельеф не тронут");
        // Без скачков высоты и наклона: на стыках чаши, гребня и края вала разности соседних
        // точек почти равны.
        let h = 1e-3;
        for x in [1.0, outer] {
            let left = crater_profile(x - h, depth, rim) - crater_profile(x - 2.0 * h, depth, rim);
            let right = crater_profile(x + 2.0 * h, depth, rim) - crater_profile(x + h, depth, rim);
            assert!((left - right).abs() < 1e-3, "излом профиля на x = {x}: {left} и {right}");
        }
    }

    #[test]
    fn mortar_rim_is_wide_enough_for_the_grid() {
        // Гребень, где вал выше половины своей высоты, шире двух шагов сетки:
        // на каждом луче его ловят несколько вершин, и он не рассыпается на зубья.
        let (radius, depth, rim) = (5.0, 0.0, 0.6);
        let crest: Vec<f32> = (0..1000)
            .map(|i| i as f32 * 0.01)
            .filter(|d| crater_profile(d / radius, depth, rim) > rim * 0.5)
            .collect();
        let width = crest.last().unwrap() - crest.first().unwrap();
        assert!(width > 2.0, "гребень вала {width} м");
    }

    #[test]
    fn crater_rim_crest_is_even_around_the_circle() {
        // Самая высокая вершина вала в каждом из 24 секторов поднимается почти одинаково при любом
        // сдвиге центра относительно сетки. Узкий вал прежнего профиля здесь гулял на 1,25 м:
        // в одних секторах вершина попадала на гребень, в других мимо, и край шёл гребёнкой.
        for (seed, center) in [(9, Vec2::new(40.37, 40.81)), (3, Vec2::new(40.0, 40.0)), (5, Vec2::new(40.5, 40.5))] {
            let mut terrain = Terrain::new(seed);
            let ring = |x: i32, z: i32| Vec2::new(x as f32, z as f32) - center;
            let vertices: Vec<(i32, i32)> = (28..54)
                .flat_map(|z| (28..54).map(move |x| (x, z)))
                .filter(|&(x, z)| (4.0..8.0).contains(&ring(x, z).length()))
                .collect();
            let before: Vec<f32> = vertices.iter().map(|&(x, z)| terrain.vertex_height(x, z)).collect();
            for _ in 0..3 {
                terrain.apply_crater(center.extend(0.0).xzy(), 5.0, 2.5, 0.6);
            }
            let mut crest = [f32::MIN; 24];
            for (&(x, z), h) in vertices.iter().zip(before) {
                let angle = ring(x, z).to_angle() + std::f32::consts::PI;
                let sector = ((angle / std::f32::consts::TAU * 24.0) as usize).min(23);
                crest[sector] = crest[sector].max(terrain.vertex_height(x, z) - h);
            }
            let spread = crest.iter().cloned().fold(f32::MIN, f32::max) - crest.iter().cloned().fold(f32::MAX, f32::min);
            // Вал после трёх взрывов 1,8 м, гребень по кругу ровный с точностью до 0,5 м.
            assert!(spread < 0.5, "гребень зубчатый: по секторам разброс {spread} м");
        }
    }

    impl Terrain {
        /// Высоты в центре, в 6 и в 8 м от него по X.
        fn clone_heights_around(&self, vx: i32, vz: i32) -> [f32; 3] {
            [0, 6, 8].map(|d| self.vertex_height(vx + d, vz))
        }
    }

    #[test]
    fn normal_points_up() {
        let terrain = Terrain::new(4);
        for i in 0..20 {
            let n = terrain.normal_at(i as f32 * 3.3, i as f32 * -2.1);
            assert!(n.y > 0.0 && n.is_normalized());
        }
    }
}
