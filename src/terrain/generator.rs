//! Генератор рельефа: высота и оттенок зелени в точке мира по сиду забега.

use noise::{Fbm, MultiFractal, NoiseFn, Perlin};

/// Частота холмов, 1/м: один холм примерно на 45 м.
const HILLS_FREQUENCY: f64 = 1.0 / 45.0;

/// Число октав фрактального шума холмов.
const HILLS_OCTAVES: usize = 4;

/// Затухание амплитуды от октавы к октаве у холмов.
const HILLS_PERSISTENCE: f64 = 0.45;

/// Амплитуда холмов, м.
const HILLS_AMPLITUDE: f32 = 7.0;

// Ограничение уклона путей рассчитано на перепад холмов не больше `PROFILE_RANGE`.
const _: () = assert!(2.0 * HILLS_AMPLITUDE <= crate::railway::PROFILE_RANGE);

/// Частота крупной составляющей для гор, 1/м.
const MOUNTAINS_FREQUENCY: f64 = 1.0 / 260.0;

/// Высота гор над равниной, м.
const MOUNTAINS_HEIGHT: f32 = 22.0;

/// Частота пятен оттенка травы, 1/м.
const TINT_FREQUENCY: f64 = 1.0 / 7.0;

/// Сдвиг сида для гор, чтобы горы не повторяли холмы.
const MOUNTAINS_SEED_OFFSET: u32 = 1_000;

/// Сдвиг сида для пятен оттенка.
const TINT_SEED_OFFSET: u32 = 2_000;

/// Шумы, из которых складывается рельеф одного забега.
pub struct HeightGenerator {
    /// Холмы: фрактальный шум Перлина.
    hills: Fbm<Perlin>,
    /// Горы: низкочастотный шум.
    mountains: Fbm<Perlin>,
    /// Пятна оттенка травы.
    tint: Perlin,
    /// Железная дорога: насыпи и выемки вдоль путей.
    lines: crate::railway::Lines,
}

impl HeightGenerator {
    /// Создаёт генератор для сида забега.
    pub fn new(seed: u32) -> Self {
        Self {
            hills: Fbm::<Perlin>::new(seed)
                .set_octaves(HILLS_OCTAVES)
                .set_frequency(HILLS_FREQUENCY)
                .set_persistence(HILLS_PERSISTENCE),
            mountains: Fbm::<Perlin>::new(seed.wrapping_add(MOUNTAINS_SEED_OFFSET))
                .set_octaves(2)
                .set_frequency(MOUNTAINS_FREQUENCY),
            tint: Perlin::new(seed.wrapping_add(TINT_SEED_OFFSET)),
            lines: crate::railway::Lines::new(seed),
        }
    }

    /// Высота нетронутой поверхности в точке мира, м.
    ///
    /// У железной дороги рельеф выровнен до уровня путей: насыпи и выемки, под горами туннели.
    pub fn height(&self, x: f32, z: f32) -> f32 {
        self.lines
            .corrected_height(x, z, |x, z| self.raw_height(x, z), |x, z| self.hills_height(x, z))
    }

    /// Нетронутая поверхность без путей, м.
    pub fn raw_height(&self, x: f32, z: f32) -> f32 {
        let p = [x as f64, z as f64];
        // Горы: только положительная часть шума, возведённая в квадрат, чтобы между горами была равнина.
        let m = (self.mountains.get(p) as f32).max(0.0);
        self.hills_height(x, z) + m * m * MOUNTAINS_HEIGHT * 2.0
    }

    /// Холмистая часть рельефа без гор, м: по ней строится уровень путей.
    pub fn hills_height(&self, x: f32, z: f32) -> f32 {
        self.hills.get([x as f64, z as f64]) as f32 * HILLS_AMPLITUDE
    }

    /// Линии железной дороги забега.
    pub fn lines(&self) -> &crate::railway::Lines {
        &self.lines
    }

    /// Оттенок травы в точке мира, примерно от -1 до 1.
    pub fn tint(&self, x: f32, z: f32) -> f32 {
        self.tint
            .get([x as f64 * TINT_FREQUENCY, z as f64 * TINT_FREQUENCY]) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Точки для сравнения рельефа.
    fn sample_points() -> impl Iterator<Item = (f32, f32)> {
        (0..20).map(|i| (i as f32 * 13.7 - 120.0, i as f32 * -7.3 + 55.5))
    }

    #[test]
    fn same_seed_gives_same_heights() {
        let a = HeightGenerator::new(42);
        let b = HeightGenerator::new(42);
        for (x, z) in sample_points() {
            assert_eq!(a.height(x, z), b.height(x, z));
        }
    }

    #[test]
    fn different_seeds_give_different_heights() {
        let a = HeightGenerator::new(42);
        let b = HeightGenerator::new(43);
        let diff: f32 = sample_points()
            .map(|(x, z)| (a.height(x, z) - b.height(x, z)).abs())
            .sum();
        assert!(diff > 5.0, "рельеф почти совпал: суммарная разница {diff}");
    }

    #[test]
    fn terrain_is_hilly() {
        let g = HeightGenerator::new(7);
        let heights: Vec<f32> = (-50..50)
            .flat_map(|x| (-50..50).map(move |z| (x as f32, z as f32)))
            .map(|(x, z)| g.height(x, z))
            .collect();
        let min = heights.iter().copied().fold(f32::MAX, f32::min);
        let max = heights.iter().copied().fold(f32::MIN, f32::max);
        assert!(max - min > 3.0, "перепад высот всего {} м", max - min);
    }
}
