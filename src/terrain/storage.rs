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
    fn normal_points_up() {
        let terrain = Terrain::new(4);
        for i in 0..20 {
            let n = terrain.normal_at(i as f32 * 3.3, i as f32 * -2.1);
            assert!(n.y > 0.0 && n.is_normalized());
        }
    }
}
