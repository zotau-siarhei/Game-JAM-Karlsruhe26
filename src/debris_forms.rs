//! Библиотека форм обломков: глыба, осколок, доска, лист металла, ком земли, крошка.
//!
//! Формы строятся процедурно при запуске, по [`VARIANTS`] варианта каждой, поэтому их застаёт
//! прогрев шейдеров (`src/prewarm.rs`). Каждая форма вписана в куб 1×1×1 с центром в начале
//! координат, как куб обломка: размер `SpawnDebris::size` растягивает её так же, как растягивал
//! куб. Грани плоские, раскладка вершин та же, что у `Cuboid` (позиция, нормаль, UV), — новых
//! конвейеров рендера формы не добавляют.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

/// Вариантов каждой формы.
pub const VARIANTS: usize = 4;

/// Вид формы обломка.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FormKind {
    /// Каменная глыба: неровный многогранник.
    Chunk,
    /// Осколок кристалла: сужающаяся пятигранная призма с острым концом вверх (по `y`).
    Shard,
    /// Доска: брусок вдоль `z` со скошенным торцом.
    Plank,
    /// Лист металла: тонкий лист, согнутый поперёк `x`.
    Sheet,
    /// Ком земли: неровный, с плоским низом.
    Clod,
    /// Крошка: мелкий острый многогранник.
    Crumb,
}

impl FormKind {
    pub const ALL: [FormKind; 6] = [
        FormKind::Chunk,
        FormKind::Shard,
        FormKind::Plank,
        FormKind::Sheet,
        FormKind::Clod,
        FormKind::Crumb,
    ];

    /// Номер вида в [`FormKind::ALL`].
    pub fn index(self) -> usize {
        self as usize
    }

    /// Блестит ли материал: осколки кварца и листы металла гладкие.
    pub fn glossy(self) -> bool {
        matches!(self, FormKind::Shard | FormKind::Sheet)
    }
}

/// Сетка формы `kind` варианта `variant`, вписанная в куб 1×1×1.
pub fn build(kind: FormKind, variant: usize) -> Mesh {
    let mut rng = Hash(0x5EED_0000 + kind.index() as u32 * 97 + variant as u32 * 13);
    let triangles = match kind {
        FormKind::Chunk => blob(&mut rng, 0.75, 1.15, false),
        FormKind::Clod => blob(&mut rng, 0.6, 1.2, true),
        FormKind::Crumb => crumb(&mut rng),
        FormKind::Shard => shard(&mut rng),
        FormKind::Plank => plank(&mut rng),
        FormKind::Sheet => sheet(&mut rng),
    };
    flat_mesh(fit_unit(triangles))
}

/// Простая детерминированная случайность: формы одни и те же в каждом запуске.
struct Hash(u32);

impl Hash {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let mut h = self.0;
        h ^= h >> 15;
        h = h.wrapping_mul(0x2C1B_3C6D);
        h ^= h >> 12;
        (h >> 8) as f32 / (1u32 << 24) as f32
    }

    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next()
    }
}

type Triangle = [Vec3; 3];

/// Треугольники, развёрнутые наружу от точки `inside`.
fn outward(triangles: impl IntoIterator<Item = Triangle>, inside: Vec3) -> Vec<Triangle> {
    triangles
        .into_iter()
        .map(|[a, b, c]| {
            let normal = (b - a).cross(c - a);
            let center = (a + b + c) / 3.0;
            if normal.dot(center - inside) >= 0.0 { [a, b, c] } else { [a, c, b] }
        })
        .collect()
}

/// Икосаэдр с вершинами, сдвинутыми по лучу на `low..high`: неровная глыба. `flat_bottom` —
/// нижние вершины прижаты к одной высоте, как у кома, легшего на землю.
fn blob(rng: &mut Hash, low: f32, high: f32, flat_bottom: bool) -> Vec<Triangle> {
    let t = (1.0 + 5.0_f32.sqrt()) * 0.5;
    let mut vertices: Vec<Vec3> = [
        (-1.0, t, 0.0),
        (1.0, t, 0.0),
        (-1.0, -t, 0.0),
        (1.0, -t, 0.0),
        (0.0, -1.0, t),
        (0.0, 1.0, t),
        (0.0, -1.0, -t),
        (0.0, 1.0, -t),
        (t, 0.0, -1.0),
        (t, 0.0, 1.0),
        (-t, 0.0, -1.0),
        (-t, 0.0, 1.0),
    ]
    .into_iter()
    .map(|(x, y, z)| Vec3::new(x, y, z).normalize() * rng.range(low, high))
    .collect();
    if flat_bottom {
        for v in &mut vertices {
            v.y = v.y.max(-0.55);
        }
    }
    const FACES: [[usize; 3]; 20] = [
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    outward(FACES.iter().map(|f| f.map(|i| vertices[i])), Vec3::ZERO)
}

/// Октаэдр с неровными вершинами: острая крошка.
fn crumb(rng: &mut Hash) -> Vec<Triangle> {
    let v: Vec<Vec3> = [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z]
        .into_iter()
        .map(|d| d * rng.range(0.6, 1.2) + Vec3::new(rng.range(-0.2, 0.2), rng.range(-0.2, 0.2), rng.range(-0.2, 0.2)))
        .collect();
    let faces = [[0, 2, 4], [2, 1, 4], [1, 3, 4], [3, 0, 4], [2, 0, 5], [1, 2, 5], [3, 1, 5], [0, 3, 5]];
    let inside = v.iter().copied().sum::<Vec3>() / v.len() as f32;
    outward(faces.iter().map(|f| f.map(|i| v[i])), inside)
}

/// Осколок: пятиугольник основания, кольцо поуже выше и острый конец.
fn shard(rng: &mut Hash) -> Vec<Triangle> {
    let ring = |rng: &mut Hash, y: f32, r: f32, shift: Vec2| -> Vec<Vec3> {
        (0..5)
            .map(|i| {
                let a = i as f32 / 5.0 * std::f32::consts::TAU + rng.range(-0.25, 0.25);
                let k = r * rng.range(0.8, 1.1);
                Vec3::new(a.cos() * k + shift.x, y, a.sin() * k + shift.y)
            })
            .collect()
    };
    let base = ring(rng, 0.0, 1.0, Vec2::ZERO);
    let lean = Vec2::new(rng.range(-0.15, 0.15), rng.range(-0.15, 0.15));
    let height = rng.range(1.8, 2.4);
    let shoulder = ring(rng, height, 0.8, lean);
    let tip = Vec3::new(lean.x * 1.6, rng.range(3.0, 3.6), lean.y * 1.6);
    let mut triangles = Vec::new();
    let bottom = Vec3::new(0.0, 0.0, 0.0);
    for i in 0..5 {
        let j = (i + 1) % 5;
        triangles.push([bottom, base[i], base[j]]);
        triangles.push([base[i], base[j], shoulder[j]]);
        triangles.push([base[i], shoulder[j], shoulder[i]]);
        triangles.push([shoulder[i], shoulder[j], tip]);
    }
    let inside = Vec3::new(lean.x * 0.5, 1.4, lean.y * 0.5);
    outward(triangles, inside)
}

/// Шестигранник по восьми углам: низ (`0..4`) и верх (`4..8`), обход по кругу.
fn hexahedron(c: [Vec3; 8]) -> Vec<Triangle> {
    let quads = [[0, 1, 2, 3], [4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7]];
    let inside = c.iter().copied().sum::<Vec3>() / 8.0;
    outward(
        quads.iter().flat_map(|q| [[c[q[0]], c[q[1]], c[q[2]]], [c[q[0]], c[q[2]], c[q[3]]]]),
        inside,
    )
}

/// Доска вдоль `z`: один торец скошен, другой отломан неровно.
fn plank(rng: &mut Hash) -> Vec<Triangle> {
    let (w, h) = (0.5, 0.15);
    let bevel = rng.range(0.3, 0.8);
    let broken = rng.range(-0.2, 0.2);
    let z0 = -3.0;
    let z1 = 3.0;
    hexahedron([
        Vec3::new(-w, -h, z0),
        Vec3::new(w, -h, z0 + broken),
        Vec3::new(w, -h, z1),
        Vec3::new(-w, -h, z1 - bevel * 0.3),
        Vec3::new(-w, h, z0),
        Vec3::new(w, h, z0 + broken),
        Vec3::new(w, h, z1 - bevel),
        Vec3::new(-w, h, z1 - bevel * 1.3),
    ])
}

/// Лист металла, согнутый поперёк `x`: профиль из шести точек (низ и верх листа) в плоскости
/// `xy`, вытянутый вдоль `z`. Профиль обходится против часовой стрелки, поэтому грани
/// разворачиваются наружу без проверки центра: лист не выпуклый.
fn sheet(rng: &mut Hash) -> Vec<Triangle> {
    let t = 0.08;
    let bend = rng.range(-0.2, 0.3);
    let angle = rng.range(0.35, 0.8);
    let (z0, z1) = (-0.6 + rng.range(-0.1, 0.1), 0.6 + rng.range(-0.1, 0.1));
    let rise = Vec2::new(angle.cos(), angle.sin()) * (1.0 - bend);
    let n2 = Vec2::new(-angle.sin(), angle.cos());
    let p0 = Vec2::new(-1.0, 0.0);
    let p1 = Vec2::new(bend, 0.0);
    let p2 = p1 + rise;
    let q2 = p2 + n2 * t;
    let q1 = p1 + (Vec2::Y + n2).normalize() * t / (angle * 0.5).cos();
    let q0 = p0 + Vec2::Y * t;
    let profile = [p0, p1, p2, q2, q1, q0];
    let at = |p: Vec2, z: f32| Vec3::new(p.x, p.y, z);
    let mut triangles = Vec::new();
    // Торцы: два выпуклых четырёхугольника профиля.
    for quad in [[p0, p1, q1, q0], [p1, p2, q2, q1]] {
        triangles.push([at(quad[0], z1), at(quad[1], z1), at(quad[2], z1)]);
        triangles.push([at(quad[0], z1), at(quad[2], z1), at(quad[3], z1)]);
        triangles.push([at(quad[0], z0), at(quad[2], z0), at(quad[1], z0)]);
        triangles.push([at(quad[0], z0), at(quad[3], z0), at(quad[2], z0)]);
    }
    // Бока по рёбрам профиля.
    for i in 0..profile.len() {
        let (a, b) = (profile[i], profile[(i + 1) % profile.len()]);
        triangles.push([at(a, z0), at(b, z0), at(b, z1)]);
        triangles.push([at(a, z0), at(b, z1), at(a, z1)]);
    }
    triangles
}

/// Вписывает треугольники в куб 1×1×1 с центром в начале координат.
fn fit_unit(triangles: Vec<Triangle>) -> Vec<Triangle> {
    let (lo, hi) = triangles
        .iter()
        .flatten()
        .fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let center = (lo + hi) * 0.5;
    let size = (hi - lo).max(Vec3::splat(1e-4));
    triangles.into_iter().map(|t| t.map(|p| (p - center) / size)).collect()
}

/// Сетка с плоскими гранями: у каждого треугольника свои вершины и нормаль.
fn flat_mesh(triangles: Vec<Triangle>) -> Mesh {
    let mut positions = Vec::with_capacity(triangles.len() * 3);
    let mut normals = Vec::with_capacity(triangles.len() * 3);
    let mut uvs = Vec::with_capacity(triangles.len() * 3);
    for [a, b, c] in triangles {
        let n = (b - a).cross(c - a).normalize_or(Vec3::Y);
        for p in [a, b, c] {
            positions.push(p.to_array());
            normals.push(n.to_array());
            // Планарная развёртка: у материала обломка нет текстуры, UV нужны только раскладке.
            uvs.push([p.x + 0.5, p.z + 0.5]);
        }
    }
    let count = positions.len() as u32;
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32((0..count).collect()))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn key(p: [f32; 3]) -> [i32; 3] {
        p.map(|c| (c * 1e4).round() as i32)
    }

    #[test]
    fn формы_замкнуты_с_нормалями_и_вписаны_в_куб() {
        for kind in FormKind::ALL {
            for variant in 0..VARIANTS {
                let mesh = build(kind, variant);
                let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap().as_float3().unwrap();
                let normals = mesh.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap().as_float3().unwrap();
                assert_eq!(positions.len(), normals.len());
                assert!(normals.iter().all(|n| (Vec3::from(*n).length() - 1.0).abs() < 1e-3), "{kind:?}");
                // Замкнутость: у каждого ребра обхода есть обратное.
                let mut edges: HashMap<([i32; 3], [i32; 3]), i32> = HashMap::new();
                for t in positions.chunks_exact(3) {
                    for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                        *edges.entry((key(a), key(b))).or_default() += 1;
                    }
                }
                for (&(a, b), &n) in &edges {
                    assert_eq!(edges.get(&(b, a)).copied().unwrap_or(0), n, "{kind:?} {variant}: дыра");
                }
                // Габариты — куб 1×1×1 с центром в начале координат.
                let (lo, hi) = positions
                    .iter()
                    .fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(Vec3::from(*p)), hi.max(Vec3::from(*p))));
                assert!((hi - lo - Vec3::ONE).abs().max_element() < 1e-3, "{kind:?}: {lo} {hi}");
                assert!((hi + lo).abs().max_element() < 1e-3);
                // Нормали смотрят наружу: объём со знаком положителен.
                let volume: f32 = positions
                    .chunks_exact(3)
                    .map(|t| Vec3::from(t[0]).dot(Vec3::from(t[1]).cross(Vec3::from(t[2]))))
                    .sum();
                assert!(volume > 0.0, "{kind:?} {variant}: вывернута");
            }
        }
    }

    #[test]
    fn варианты_различаются() {
        let a = build(FormKind::Chunk, 0);
        let b = build(FormKind::Chunk, 1);
        let p = |m: &Mesh| m.attribute(Mesh::ATTRIBUTE_POSITION).unwrap().as_float3().unwrap().to_vec();
        assert_ne!(p(&a), p(&b));
    }
}
