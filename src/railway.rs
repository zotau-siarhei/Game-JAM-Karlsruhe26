//! Железная дорога: бесконечные линии, вычисляемые функцией от координаты, и профиль путей.
//!
//! Мир пересекают параллельные линии вдоль X (`z ≈ k·L`), без пересечений. Каждая линия
//! плавно отклоняется от своей оси по шуму, поэтому точку и касательную пути можно вычислить для
//! любого куска мира без хранения линии. Уровень путей — рельеф холмов вдоль линии, сглаженный
//! скользящим средним и ограниченный по уклону; горы в профиль не входят, под ними путь уходит
//! в туннель.
//!
//! Профиль считается по узлам с шагом [`KNOT_STEP`]: сглаженный уровень в узле, затем
//! ограничение уклона между узлами, затем гладкая кривая через узлы. Уровни узлов
//! запоминаются, поэтому повторный запрос уровня шум не считает.

use std::collections::HashMap;
use std::sync::Mutex;

use bevy::prelude::*;
use noise::{NoiseFn, Perlin};

/// Шаг сетки линий, м.
pub const LINE_SPACING: f32 = 260.0;

/// Наибольшее отклонение линии от оси, м, и длина волны изгиба: крупные петли и мелкие извивы,
/// как будто путь обходит рощи. Радиус поворота не меньше ~20 м, чтобы прошёл поезд.
const BEND_AMPLITUDE: f32 = 40.0;
const BEND_WAVELENGTH: f32 = 300.0;
const WIGGLE_AMPLITUDE: f32 = 10.0;
const WIGGLE_WAVELENGTH: f32 = 90.0;

/// Сдвиг линии от узла сетки в обе стороны, м.
const OFFSET_RANGE: f32 = 40.0;

/// Ширина треугольного окна сглаживания профиля и шаг выборок рельефа в нём, м. Шаг заметно
/// мельче холма (45 м): редкие выборки не усредняют холмы, а пропускают их в профиль.
const PROFILE_WINDOW: f32 = 240.0;
const PROFILE_SAMPLE_STEP: f32 = 5.0;

/// Шаг узлов профиля, м.
const KNOT_STEP: f32 = 20.0;

/// Наибольший уклон профиля. Предел — 3 %; запас оставлен на погрешность счёта.
pub const MAX_SLOPE: f32 = 0.028;

/// Наибольший перепад сглаженного уровня вдоль линии, м: холмы не выше и не ниже 7 м.
/// От него зависит, на сколько узлов в обе стороны смотрит ограничение уклона.
pub const PROFILE_RANGE: f32 = 14.0;

/// На сколько узлов в каждую сторону смотрит ограничение уклона: дальше узел повлиять не может,
/// потому что предельный уклон на таком расстоянии набирает весь перепад [`PROFILE_RANGE`].
const LIMIT_REACH: i32 = (PROFILE_RANGE / (MAX_SLOPE * KNOT_STEP)) as i32 + 1;

/// Полоса, где рельеф равен уровню путей, и полоса перехода к рельефу, м.
pub const TRACK_FLAT: f32 = 5.0;
pub const TRACK_BAND: f32 = 14.0;

/// Насыпь под путями: высота над уровнем путей, полуширина плоского верха и где кончается
/// откос, м. Рельсы и вагоны стоят на её верхе.
pub const EMBANKMENT_HEIGHT: f32 = 0.8;
const EMBANKMENT_TOP: f32 = 2.0;
const EMBANKMENT_FOOT: f32 = 4.5;

/// Высота насыпи над уровнем путей на расстоянии `distance` от оси: плоский верх, плавный откос.
pub fn embankment(distance: f32) -> f32 {
    let t = ((distance - EMBANKMENT_TOP) / (EMBANKMENT_FOOT - EMBANKMENT_TOP)).clamp(0.0, 1.0);
    EMBANKMENT_HEIGHT * (1.0 - t * t * (3.0 - 2.0 * t))
}

/// На сколько гора выше путей, чтобы под ней был туннель, м.
pub const TUNNEL_DEPTH: f32 = 9.0;

/// Туннель не короче этого, м: гребень уже прорезается выемкой.
pub const TUNNEL_MIN_LENGTH: f32 = 30.0;

/// Шаг выборок вдоль пути, по которым меряется длина туннеля, м.
const TUNNEL_STEP: f32 = 2.0;

/// Сдвиг сида для изгибов линий.
const BEND_SEED: u32 = 6_000;

/// Семейства линий, которые есть в мире: только параллельные линии вдоль X, без пересечений.
const FAMILIES: [Family; 1] = [Family::AlongX];

/// Семейство линий.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    /// Вдоль оси X: параметр пути — `x`.
    AlongX,
    /// Вдоль оси Z: параметр пути — `z`. Сейчас таких линий нет, чтобы пути не пересекались.
    #[allow(dead_code)]
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
    /// Запомненные уровни узлов профиля: сглаженные и ограниченные по уклону.
    profile: Mutex<Profile>,
}

/// Уровни узлов профиля по линии и номеру узла.
#[derive(Default)]
struct Profile {
    smoothed: HashMap<(LineId, i32), f32>,
    limited: HashMap<(LineId, i32), f32>,
    /// Гора над путями глубже [`TUNNEL_DEPTH`] в выборках через [`TUNNEL_STEP`].
    deep: HashMap<(LineId, i32), bool>,
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
            profile: Mutex::default(),
        }
    }

    /// Поперечная координата оси линии на параметре `s`: `z` для линий вдоль X, `x` для линий вдоль Z.
    pub fn cross(&self, line: LineId, s: f32) -> f32 {
        let fam = match line.family {
            Family::AlongX => 0,
            Family::AlongZ => 1,
        };
        let offset = (hash(self.seed, line.index, fam) * 2.0 - 1.0) * OFFSET_RANGE;
        let row = line.index as f64 * 7.31 + fam as f64 * 101.7;
        let bend = self.bend.get([(s / BEND_WAVELENGTH) as f64, row]) as f32;
        let wiggle = self.bend.get([(s / WIGGLE_WAVELENGTH) as f64 + 500.0, row + 33.3]) as f32;
        line.index as f32 * LINE_SPACING + offset + bend * BEND_AMPLITUDE + wiggle * WIGGLE_AMPLITUDE
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

    // Нужно поездам и крестам на пересечениях (задачи 2.1 и 4.1).
    #[allow(dead_code)]
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
        for family in FAMILIES {
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
        for family in FAMILIES {
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

    /// Уровень путей на параметре `s`: высота холмов вдоль оси, сглаженная и ограниченная
    /// по уклону не круче [`MAX_SLOPE`].
    ///
    /// Кривая между узлами — равномерный квадратичный B-сплайн: его наклон в любой точке лежит
    /// между наклонами двух соседних отрезков между узлами, поэтому предел уклона узлов
    /// выполняется и между ними, а на узлах нет изломов.
    ///
    /// `hills` — холмистая часть рельефа; на всю жизнь линий она должна быть одной и той же
    /// функцией, потому что уровни узлов запоминаются.
    pub fn level(&self, line: LineId, s: f32, hills: impl Fn(f32, f32) -> f32) -> f32 {
        // Отрезок сплайна с опорой на три узла идёт от середины до середины между ними.
        let x = s / KNOT_STEP + 0.5;
        let knot = x.floor();
        let t = x - knot;
        let knot = knot as i32;
        let before = self.limited_knot(line, knot - 1, &hills);
        let here = self.limited_knot(line, knot, &hills);
        let after = self.limited_knot(line, knot + 1, &hills);
        0.5 * (1.0 - t) * (1.0 - t) * before + (0.5 + t * (1.0 - t)) * here + 0.5 * t * t * after
    }

    /// Сглаженный уровень в узле: холмы вдоль оси под треугольным окном.
    fn smoothed_knot(&self, line: LineId, knot: i32, hills: &impl Fn(f32, f32) -> f32) -> f32 {
        if let Some(level) = self.profile.lock().unwrap().smoothed.get(&(line, knot)) {
            return *level;
        }
        let s = knot as f32 * KNOT_STEP;
        let half = (PROFILE_WINDOW * 0.5 / PROFILE_SAMPLE_STEP) as i32;
        let (mut sum, mut weights) = (0.0, 0.0);
        for i in -half..=half {
            let w = 1.0 - i.abs() as f32 / (half + 1) as f32;
            let p = self.point(line, s + i as f32 * PROFILE_SAMPLE_STEP);
            sum += hills(p.x, p.y) * w;
            weights += w;
        }
        let level = sum / weights;
        self.profile.lock().unwrap().smoothed.insert((line, knot), level);
        level
    }

    /// Уровень узла с ограничением уклона: середина между двумя огибающими сглаженного профиля.
    ///
    /// Нижняя огибающая — самая высокая кривая не выше профиля, у которой уклон между узлами
    /// не больше предела; верхняя — самая низкая не ниже профиля. У обеих и у их середины уклон
    /// ограничен. Где профиль и так пологий, огибающие с ним совпадают; крутой склон срезается
    /// поровну сверху и снизу: полувыемка и полунасыпь. Значение узла зависит только от
    /// соседей, а не от того, с какой стороны к нему подошли.
    fn limited_knot(&self, line: LineId, knot: i32, hills: &impl Fn(f32, f32) -> f32) -> f32 {
        if let Some(level) = self.profile.lock().unwrap().limited.get(&(line, knot)) {
            return *level;
        }
        let rise = MAX_SLOPE * KNOT_STEP;
        let (mut lower, mut upper) = (f32::MAX, f32::MIN);
        for other in knot - LIMIT_REACH..=knot + LIMIT_REACH {
            let level = self.smoothed_knot(line, other, hills);
            let reach = rise * (other - knot).abs() as f32;
            lower = lower.min(level + reach);
            upper = upper.max(level - reach);
        }
        let level = 0.5 * (lower + upper);
        self.profile.lock().unwrap().limited.insert((line, knot), level);
        level
    }

    /// Гора над осью на параметре `s` выше уровня путей больше чем на [`TUNNEL_DEPTH`].
    fn is_deep(&self, line: LineId, s: f32, raw: &impl Fn(f32, f32) -> f32, hills: &impl Fn(f32, f32) -> f32) -> bool {
        let axis = self.point(line, s);
        raw(axis.x, axis.y) > self.level(line, s, hills) + TUNNEL_DEPTH
    }

    /// [`Self::is_deep`] в выборке номер `i` через [`TUNNEL_STEP`], с запоминанием.
    fn deep_sample(&self, line: LineId, i: i32, raw: &impl Fn(f32, f32) -> f32, hills: &impl Fn(f32, f32) -> f32) -> bool {
        if let Some(deep) = self.profile.lock().unwrap().deep.get(&(line, i)) {
            return *deep;
        }
        let deep = self.is_deep(line, i as f32 * TUNNEL_STEP, raw, hills);
        self.profile.lock().unwrap().deep.insert((line, i), deep);
        deep
    }

    /// Проходит ли путь на параметре `s` в туннеле: гора над ним выше уровня путей больше чем на
    /// [`TUNNEL_DEPTH`], и так подряд не меньше [`TUNNEL_MIN_LENGTH`]. Короткий гребень — выемка.
    ///
    /// `raw` — исходный рельеф, `hills` — его холмистая часть без гор; как и у [`Self::level`],
    /// это одни и те же функции на всю жизнь линий: выборки запоминаются.
    pub fn is_tunnel(&self, line: LineId, s: f32, raw: impl Fn(f32, f32) -> f32, hills: impl Fn(f32, f32) -> f32) -> bool {
        if !self.is_deep(line, s, &raw, &hills) {
            return false;
        }
        let deep = |i: i32| self.deep_sample(line, i, &raw, &hills);
        // Ближайшая глубокая выборка рядом с `s`; от неё — глубокие подряд в обе стороны.
        let i = (s / TUNNEL_STEP).floor() as i32;
        let start = if deep(i) {
            i
        } else if deep(i + 1) {
            i + 1
        } else {
            return false;
        };
        let need = (TUNNEL_MIN_LENGTH / TUNNEL_STEP).ceil() as i32;
        let (mut lo, mut hi) = (start, start);
        while hi - lo < need && deep(lo - 1) {
            lo -= 1;
        }
        while hi - lo < need && deep(hi + 1) {
            hi += 1;
        }
        hi - lo >= need
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
        if self.is_tunnel(near.line, near.s, &raw, &hills) {
            return h;
        }
        let t = ((near.distance - TRACK_FLAT) / (TRACK_BAND - TRACK_FLAT)).clamp(0.0, 1.0);
        let k = t * t * (3.0 - 2.0 * t);
        level + (h - level) * k + embankment(near.distance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;


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
    fn profile_slope_is_at_most_3_percent() {
        // Уровень по настоящим холмам рельефа.
        for seed in [8, 21, 99] {
            let terrain = crate::terrain::Terrain::new(seed);
            let line = LineId {
                family: Family::AlongX,
                index: 0,
            };
            let mut worst: f32 = 0.0;
            let mut s = -600.0;
            while s < 600.0 {
                let a = terrain.track_level(line, s);
                let b = terrain.track_level(line, s + 4.0);
                let run = terrain.lines().point(line, s).distance(terrain.lines().point(line, s + 4.0));
                worst = worst.max((b - a).abs() / run);
                s += 4.0;
            }
            assert!(worst <= 0.03, "сид {seed}: уклон {worst}");
        }
    }

    #[test]
    fn profile_slope_is_limited_even_on_a_cliff() {
        // Обрыв во весь перепад холмов: сглаживание его не спасает, уклон держит ограничение.
        let lines = Lines::new(3);
        let line = LineId {
            family: Family::AlongX,
            index: 0,
        };
        let cliff = |x: f32, _z: f32| if x > 0.0 { 7.0 } else { -7.0 };
        let mut worst: f32 = 0.0;
        let mut s = -700.0;
        while s < 700.0 {
            let rise = lines.level(line, s + 1.0, cliff) - lines.level(line, s, cliff);
            worst = worst.max(rise.abs());
            s += 1.0;
        }
        assert!(worst <= 0.03, "уклон {worst}");
        // Вдали от обрыва профиль лежит на рельефе, у обрыва — посередине.
        assert!((lines.level(line, -650.0, cliff) + 7.0).abs() < 0.01);
        assert!((lines.level(line, 650.0, cliff) - 7.0).abs() < 0.01);
        assert!(lines.level(line, 0.0, cliff).abs() < 0.5);
    }

    #[test]
    fn short_ridge_is_cut_and_long_mountain_is_a_tunnel() {
        // Холмов нет, уровень путей 0; поперёк линии гребень высотой 15 м. Выборки запоминаются,
        // поэтому у каждого рельефа свои линии.
        let lines = Lines::new(3);
        let line = LineId {
            family: Family::AlongX,
            index: 0,
        };
        let hills = |_x: f32, _z: f32| 0.0;
        let ridge = |width: f32| move |x: f32, _z: f32| if (x - 100.0).abs() < width * 0.5 { 15.0 } else { 0.0 };
        // Гребень 20 м: выемка, рельеф над осью срезан до уровня путей, на нём насыпь.
        let narrow = ridge(20.0);
        assert!(!lines.is_tunnel(line, 100.0, narrow, hills));
        let axis = lines.point(line, 100.0);
        assert!((lines.corrected_height(axis.x, axis.y, narrow, hills) - EMBANKMENT_HEIGHT).abs() < 0.01);
        // Гора 60 м: туннель, гора над осью остаётся.
        let lines = Lines::new(3);
        let wide = ridge(60.0);
        for s in [75.0, 100.0, 125.0] {
            assert!(lines.is_tunnel(line, s, wide, hills), "s = {s}");
        }
        assert!(!lines.is_tunnel(line, 140.0, wide, hills));
        assert!((lines.corrected_height(axis.x, axis.y, wide, hills) - 15.0).abs() < 0.01);
    }

    #[test]
    fn track_runs_on_an_embankment() {
        // Ровная земля: у оси насыпь, за откосом земля на уровне путей.
        let lines = Lines::new(4);
        let line = LineId {
            family: Family::AlongX,
            index: 0,
        };
        let flat = |_x: f32, _z: f32| 0.0;
        let at = |d: f32| {
            let p = lines.point(line, 50.0) + Vec2::Y * d;
            lines.corrected_height(p.x, p.y, flat, flat)
        };
        assert!((at(0.0) - EMBANKMENT_HEIGHT).abs() < 0.05);
        assert!((at(1.5) - EMBANKMENT_HEIGHT).abs() < 0.05);
        assert!(at(3.2) < EMBANKMENT_HEIGHT && at(3.2) > 0.0, "откос");
        assert!(at(6.0).abs() < 0.01);
    }

    #[test]
    fn gentle_profile_is_not_changed_by_the_slope_limit() {
        // Уклон 1 % вдвое положе предела: уровень идёт по рельефу.
        let lines = Lines::new(3);
        let line = LineId {
            family: Family::AlongX,
            index: 0,
        };
        let ramp = |x: f32, _z: f32| x * 0.01;
        for s in [-333.0, -10.0, 0.0, 47.5, 410.0] {
            assert!((lines.level(line, s, ramp) - s * 0.01).abs() < 0.01, "s = {s}");
        }
    }
}
