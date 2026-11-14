//! Железная дорога: бесконечные линии, вычисляемые функцией от координаты, и профиль путей.
//!
//! Мир пересекают два семейства линий: вдоль X (`z ≈ k·L`) и вдоль Z (`x ≈ k·L`). Каждая линия
//! плавно отклоняется от своей оси по шуму, поэтому точку и касательную пути можно вычислить для
//! любого куска мира без хранения линии. Уровень путей — рельеф холмов вдоль линии, сглаженный
//! скользящим средним; горы в профиль не входят, под ними путь уходит в туннель.

use bevy::prelude::*;
use noise::{NoiseFn, Perlin};

/// Шаг сетки линий, м.
pub const LINE_SPACING: f32 = 260.0;

/// Наибольшее отклонение линии от оси, м, и длина волны изгиба.
const BEND_AMPLITUDE: f32 = 40.0;
const BEND_WAVELENGTH: f32 = 300.0;

/// Сдвиг линии от узла сетки в обе стороны, м.
const OFFSET_RANGE: f32 = 40.0;

/// Ширина треугольного окна сглаживания профиля и число точек в нём.
const PROFILE_WINDOW: f32 = 240.0;
const PROFILE_SAMPLES: usize = 17;

/// Полоса, где рельеф равен уровню путей, и полоса перехода к рельефу, м.
pub const TRACK_FLAT: f32 = 5.0;
pub const TRACK_BAND: f32 = 14.0;

/// На сколько гора выше путей, чтобы под ней был туннель, м.
pub const TUNNEL_DEPTH: f32 = 9.0;

/// Сдвиг сида для изгибов линий.
const BEND_SEED: u32 = 6_000;

/// Семейство линий.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    /// Вдоль оси X: параметр пути — `x`.
    AlongX,
    /// Вдоль оси Z: параметр пути — `z`.
    AlongZ,
}

/// Номер линии.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineId {
    pub family: Family,
    pub index: i32,
}

/// Линии забега.
pub struct Lines {
    seed: u32,
    bend: Perlin,
}

/// Ближайшая линия к точке.
#[derive(Debug, Clone, Copy)]
pub struct Nearest {
    pub line: LineId,
    /// Параметр пути напротив точки.
    pub s: f32,
    /// Расстояние до оси поперёк, м.
    pub distance: f32,
}

fn hash(seed: u32, a: i32, b: u32) -> f32 {
    let mut z = (seed as u64) << 32 ^ (a as u32 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b as u64;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
}

impl Lines {
    pub fn new(seed: u32) -> Self {
        Self {
            seed,
            bend: Perlin::new(seed.wrapping_add(BEND_SEED)),
        }
    }

    /// Поперечная координата оси линии на параметре `s`: `z` для линий вдоль X, `x` для линий вдоль Z.
    pub fn cross(&self, line: LineId, s: f32) -> f32 {
        let fam = match line.family {
            Family::AlongX => 0,
            Family::AlongZ => 1,
        };
        let offset = (hash(self.seed, line.index, fam) * 2.0 - 1.0) * OFFSET_RANGE;
        let bend = self.bend.get([
            (s / BEND_WAVELENGTH) as f64,
            line.index as f64 * 7.31 + fam as f64 * 101.7,
        ]) as f32;
        line.index as f32 * LINE_SPACING + offset + bend * BEND_AMPLITUDE
    }

    /// Точка оси в плане на параметре `s`.
    pub fn point(&self, line: LineId, s: f32) -> Vec2 {
        let c = self.cross(line, s);
        match line.family {
            Family::AlongX => Vec2::new(s, c),
            Family::AlongZ => Vec2::new(c, s),
        }
    }

    /// Единичная касательная в сторону роста `s`.
    pub fn tangent(&self, line: LineId, s: f32) -> Vec2 {
        (self.point(line, s + 0.5) - self.point(line, s - 0.5)).normalize_or(Vec2::X)
    }

    /// Параметр пути для точки плана: координата вдоль оси семейства.
    pub fn param(line: LineId, p: Vec2) -> f32 {
        match line.family {
            Family::AlongX => p.x,
            Family::AlongZ => p.y,
        }
    }

    /// Ближайшая линия к точке плана.
    pub fn nearest(&self, p: Vec2) -> Nearest {
        let mut best: Option<Nearest> = None;
        for family in [Family::AlongX, Family::AlongZ] {
            let (s, across) = match family {
                Family::AlongX => (p.x, p.y),
                Family::AlongZ => (p.y, p.x),
            };
            let k = (across / LINE_SPACING).round() as i32;
            for index in k - 1..=k + 1 {
                let line = LineId { family, index };
                // Поперечное расстояние, поправленное на наклон оси.
                let t = self.tangent(line, s);
                let along_axis = match family {
                    Family::AlongX => t.x,
                    Family::AlongZ => t.y,
                };
                let distance = (across - self.cross(line, s)).abs() * along_axis.abs();
                if best.is_none_or(|b| distance < b.distance) {
                    best = Some(Nearest { line, s, distance });
                }
            }
        }
        best.expect("линий всегда шесть кандидатов")
    }

    /// Все линии, ось которых ближе `radius` к точке.
    pub fn near(&self, p: Vec2, radius: f32) -> Vec<Nearest> {
        let mut found = Vec::new();
        for family in [Family::AlongX, Family::AlongZ] {
            let (s, across) = match family {
                Family::AlongX => (p.x, p.y),
                Family::AlongZ => (p.y, p.x),
            };
            let k = (across / LINE_SPACING).round() as i32;
            for index in k - 1..=k + 1 {
                let line = LineId { family, index };
                let t = self.tangent(line, s);
                let along_axis = match family {
                    Family::AlongX => t.x,
                    Family::AlongZ => t.y,
                };
                let distance = (across - self.cross(line, s)).abs() * along_axis.abs();
                if distance < radius {
                    found.push(Nearest { line, s, distance });
                }
            }
        }
        found
    }

    /// Пересечение линии вдоль X номер `a` с линией вдоль Z номер `b`.
    pub fn intersection(&self, a: i32, b: i32) -> Vec2 {
        let la = LineId {
            family: Family::AlongX,
            index: a,
        };
        let lb = LineId {
            family: Family::AlongZ,
            index: b,
        };
        // Простая итерация: изгибы пологие, поэтому она быстро сходится.
        let mut x = b as f32 * LINE_SPACING;
        let mut z = a as f32 * LINE_SPACING;
        for _ in 0..30 {
            z = self.cross(la, x);
            x = self.cross(lb, z);
        }
        Vec2::new(x, z)
    }

    /// Уровень путей на параметре `s`: высота холмов вдоль оси, сглаженная треугольным окном.
    ///
    /// Треугольное окно гасит короткие холмы намного сильнее прямоугольного, поэтому уклон
    /// профиля остаётся пологим без последовательного ограничения.
    pub fn level(&self, line: LineId, s: f32, hills: impl Fn(f32, f32) -> f32) -> f32 {
        let step = PROFILE_WINDOW / (PROFILE_SAMPLES - 1) as f32;
        let (mut sum, mut weights) = (0.0, 0.0);
        for i in 0..PROFILE_SAMPLES {
            let u = i as f32 / (PROFILE_SAMPLES - 1) as f32 * 2.0 - 1.0;
            let w = 1.0 - u.abs() + 1e-3;
            let q = s - PROFILE_WINDOW * 0.5 + i as f32 * step;
            let p = self.point(line, q);
            sum += hills(p.x, p.y) * w;
            weights += w;
        }
        sum / weights
    }

    /// Высота рельефа с поправкой путей: насыпи и выемки в полосе у оси, туннели под горами.
    ///
    /// `raw` — исходный рельеф, `hills` — его холмистая часть без гор.
    pub fn corrected_height(&self, x: f32, z: f32, raw: impl Fn(f32, f32) -> f32, hills: impl Fn(f32, f32) -> f32) -> f32 {
        let h = raw(x, z);
        let near = self.nearest(Vec2::new(x, z));
        if near.distance >= TRACK_BAND {
            return h;
        }
        let level = self.level(near.line, near.s, &hills);
        let axis = self.point(near.line, near.s);
        if raw(axis.x, axis.y) > level + TUNNEL_DEPTH {
            return h;
        }
        let t = ((near.distance - TRACK_FLAT) / (TRACK_BAND - TRACK_FLAT)).clamp(0.0, 1.0);
        let k = t * t * (3.0 - 2.0 * t);
        level + (h - level) * k
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hills(x: f32, z: f32) -> f32 {
        // Холмы рельефа: основная октава около 45 м, следующие короче.
        (x * 0.14).sin() * 4.0 + (z * 0.1).cos() * 3.0 + (x * 0.28 + z * 0.2).sin() * 1.5
    }

    #[test]
    fn same_seed_gives_same_lines() {
        let (a, b) = (Lines::new(5), Lines::new(5));
        let line = LineId {
            family: Family::AlongZ,
            index: 2,
        };
        for s in [-500.0, 0.0, 123.4, 999.0] {
            assert_eq!(a.point(line, s), b.point(line, s));
        }
        assert_ne!(Lines::new(6).point(line, 0.0), a.point(line, 0.0));
    }

    #[test]
    fn a_line_is_always_closer_than_260_m() {
        let lines = Lines::new(11);
        for i in 0..400 {
            let p = Vec2::new((i as f32 * 37.3).sin() * 3000.0, (i as f32 * 11.9).cos() * 3000.0);
            assert!(lines.nearest(p).distance < LINE_SPACING, "{p}");
        }
    }

    #[test]
    fn intersection_lies_on_both_lines() {
        let lines = Lines::new(3);
        for (a, b) in [(0, 0), (1, -2), (-3, 4)] {
            let p = lines.intersection(a, b);
            let za = lines.cross(LineId { family: Family::AlongX, index: a }, p.x);
            let xb = lines.cross(LineId { family: Family::AlongZ, index: b }, p.y);
            assert!((za - p.y).abs() < 0.05 && (xb - p.x).abs() < 0.05, "{a},{b}: {p}");
        }
    }

    #[test]
    fn profile_slope_is_at_most_3_percent() {
        let lines = Lines::new(8);
        let line = LineId {
            family: Family::AlongX,
            index: 0,
        };
        let mut worst: f32 = 0.0;
        let mut s = -400.0;
        while s < 400.0 {
            let a = lines.level(line, s, hills);
            let b = lines.level(line, s + 4.0, hills);
            let run = lines.point(line, s).distance(lines.point(line, s + 4.0));
            worst = worst.max((b - a).abs() / run);
            s += 4.0;
        }
        assert!(worst <= 0.03, "уклон {worst}");
    }
}
