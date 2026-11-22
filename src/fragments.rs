//! Куски моделей: сетка модели режется на куски с её же текстурой, разрушение разбрасывает их.
//!
//! Когда модель из [`CUT`] загрузилась и её сцена появилась в мире, треугольники модели делятся
//! на куски k-средними по центрам треугольников. Начальные центры — самые далёкие друг от друга
//! треугольники, поэтому куски одни и те же в каждом запуске. Каждому куску добавляется толщина:
//! внутренняя сторона, сдвинутая против нормали, и бока по краям, чтобы в полёте он был плитой,
//! а не пустой скорлупой. Материал общий с моделью, плюс закопчённая копия для разрушенного.
//!
//! Модели режутся по одной в пуле фоновых задач. Результат — ресурс [`ModelPieces`]. Модель с числом кусков 1
//! (колесо, нога босса) не режется, а отлетает целиком.
//!
//! Разбросать куски того, что стоит в мире, — [`Breaker::burst`]: он находит среди потомков
//! сущности меши нарезанных моделей и бросает их куски обломками с тех мест, где они были.

use std::collections::{HashMap, HashSet};

use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::SystemParam;
use bevy::gltf::{Gltf, GltfMesh};
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;

use crate::debris::{DebrisShape, SpawnDebris};
use crate::frame::GameSet;
use crate::models::ModelBank;

/// Модели, которые режутся на куски, и сколько кусков; 1 — модель отлетает целиком.
const CUT: [(&str, usize); 23] = [
    ("house_small", 18),
    ("house_two_story", 22),
    ("house_barrack", 24),
    ("boulder", 8),
    ("crystal", 6),
    ("bush", 5),
    ("enemy_rammer", 9),
    ("enemy_mortar", 9),
    ("enemy_interceptor", 7),
    ("enemy_wheel", 1),
    ("enemy_tank_hull", 10),
    ("enemy_tank_turret", 5),
    ("enemy_laser_turret", 5),
    ("enemy_tank_track", 3),
    ("boss_body", 14),
    ("boss_leg", 1),
    ("service_patrol", 9),
    ("service_transport", 9),
    ("service_support", 9),
    ("train_loco", 12),
    ("train_wagon_tank", 10),
    ("train_wagon_freight", 8),
    ("train_wagon_passenger", 8),
];

/// Как часто ищется материал ещё не нарезанных моделей, кадров: поиск идёт по всем мешам мира,
/// а модель босса, например, в забеге может не появиться вовсе.
const SEARCH_EVERY: u32 = 15;

/// Толщина куска в долях наибольшего размера модели.
const THICKNESS: f32 = 0.035;

/// Кусок меньше этого числа треугольников не делается: мелочь от краёв.
const MIN_TRIANGLES: usize = 6;

/// Проходов k-средних.
const ITERATIONS: usize = 8;

/// Яркость закопчённых кусков: как у дома после половины прочности.
const CHARRED_SHADE: f32 = 0.55;

/// Подключает нарезку моделей.
pub struct FragmentsPlugin;

impl Plugin for FragmentsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ModelPieces>()
            .add_systems(Update, cut_models.in_set(GameSet::Presentation));
    }
}

/// Кусок модели: сетка с началом в центре куска.
#[derive(Clone, Debug)]
pub struct ModelPiece {
    pub mesh: Handle<Mesh>,
    /// Центр куска в осях файла модели.
    pub center: Vec3,
    /// Половина габаритов куска в осях файла модели.
    pub half: Vec3,
}

/// Куски одной модели.
pub struct PieceSet {
    pub pieces: Vec<ModelPiece>,
    /// Материал модели.
    pub material: Handle<StandardMaterial>,
    /// Закопчённая копия материала.
    pub charred: Handle<StandardMaterial>,
    /// Растяжка модели из `models.json`.
    entry: Transform,
    /// Низ и верх модели по высоте в осях файла.
    low: f32,
    high: f32,
}

impl PieceSet {
    /// Кусок на своём месте в системе объекта, где модель стоит под растяжкой `fit`
    /// (как `Transform::from_scale(fit)` над сценой модели). Поворот модели в списке — кратный
    /// прямому углу, поэтому растяжка переносится на масштаб куска точно.
    pub fn local(&self, piece: &ModelPiece, fit: Vec3) -> Transform {
        let e = self.entry;
        let at = e.translation + e.rotation * (e.scale * piece.center);
        Transform {
            translation: fit * at,
            rotation: e.rotation,
            scale: e.scale * (e.rotation.inverse() * fit).abs(),
        }
    }

    /// Высота центра куска: 0 — низ модели, 1 — верх.
    pub fn level(&self, piece: &ModelPiece) -> f32 {
        ((piece.center.y - self.low) / (self.high - self.low).max(1e-4)).clamp(0.0, 1.0)
    }
}

/// Нарезанные модели по идентификатору.
#[derive(Resource, Default)]
pub struct ModelPieces {
    sets: HashMap<&'static str, PieceSet>,
    /// Исходная сетка модели → её идентификатор: так находятся куски меша, стоящего в мире.
    by_mesh: HashMap<AssetId<Mesh>, &'static str>,
}

impl ModelPieces {
    /// Куски модели `id`, если она уже нарезана и кусков больше нуля.
    pub fn get(&self, id: &str) -> Option<&PieceSet> {
        self.sets.get(id).filter(|set| !set.pieces.is_empty())
    }

    /// Куски модели, которой принадлежит исходная сетка `mesh`.
    pub fn by_mesh(&self, mesh: AssetId<Mesh>) -> Option<&PieceSet> {
        self.by_mesh.get(&mesh).and_then(|id| self.get(id))
    }

    /// Все нарезанные модели: для отладочной сцены кусков.
    pub fn all(&self) -> impl Iterator<Item = (&'static str, &PieceSet)> {
        self.sets.iter().filter(|(_, set)| !set.pieces.is_empty()).map(|(id, set)| (*id, set))
    }
}

/// Как бросить куски: откуда разлёт, с какой скоростью и сколько их.
#[derive(Clone, Copy, Debug)]
pub struct Burst {
    /// Центр разлёта: куски летят от него в стороны.
    pub from: Vec3,
    /// Скорость в стороны и вверх, м/с.
    pub speed: f32,
    pub lift: f32,
    /// Доля кусков, которая бросается: 1 — все.
    pub share: f32,
    /// Сколько кусков горит.
    pub burning: usize,
    /// Куски в копоти, а не в цвете модели.
    pub charred: bool,
}

/// Часть сущности для осколков: сетка, материал и место в мире.
pub struct Part {
    pub mesh: Handle<Mesh>,
    pub material: Handle<StandardMaterial>,
    pub transform: Transform,
    /// Это кусок нарезанной модели, а не меш целиком.
    pub piece: bool,
}

/// Доступ к кускам моделей из системы, которая разрушает объект.
#[derive(SystemParam)]
pub struct Breaker<'w, 's> {
    pieces: Res<'w, ModelPieces>,
    children: Query<'w, 's, &'static Children>,
    meshes: Query<
        'w,
        's,
        (&'static Mesh3d, &'static GlobalTransform, Option<&'static MeshMaterial3d<StandardMaterial>>),
        Without<crate::models::ShadowProxy>,
    >,
    /// Обломки: система с `Breaker` пишет их только сюда, второй писатель того же сообщения
    /// в системе недопустим.
    pub debris: MessageWriter<'w, SpawnDebris>,
}

impl Breaker<'_, '_> {
    /// Бросает обломками куски нарезанных моделей, которые стоят потомками `root`. Возвращает,
    /// сколько кусков брошено: 0 — модели не нарезаны или показан запасной облик.
    pub fn burst(&mut self, root: Entity, burst: Burst) -> usize {
        let mut thrown = 0;
        let mut k = 0u32;
        for part in self.children.iter_descendants(root) {
            let Ok((mesh, global, _)) = self.meshes.get(part) else {
                continue;
            };
            let Some(set) = self.pieces.by_mesh(mesh.0.id()) else {
                continue;
            };
            let material = if burst.charred { &set.charred } else { &set.material };
            for piece in &set.pieces {
                k += 1;
                // Доля кусков — по золотому сечению: выбранные разбросаны по всей модели.
                if (k as f32 * 0.618_034).fract() >= burst.share {
                    continue;
                }
                let world = global.mul_transform(Transform::from_translation(piece.center)).compute_transform();
                let p = world.translation;
                let jitter = |salt: u32| {
                    let h = (k.wrapping_mul(2_654_435_761) ^ salt.wrapping_mul(0x9E37_79B9)).wrapping_mul(0x85EB_CA6B);
                    (h >> 8) as f32 / (1u32 << 24) as f32
                };
                let out = (p - burst.from).with_y(0.0).normalize_or(Quat::from_rotation_y(k as f32 * 2.4) * Vec3::X);
                let velocity = out * burst.speed * (0.6 + 0.8 * jitter(1)) + Vec3::Y * burst.lift * (0.7 + 0.6 * jitter(2));
                self.debris.write(SpawnDebris {
                    position: p,
                    velocity,
                    size: world.scale,
                    color: Color::WHITE,
                    burning: thrown < burst.burning,
                    shape: DebrisShape::Mesh {
                        mesh: piece.mesh.clone(),
                        material: material.clone(),
                        half: piece.half,
                    },
                    rotation: Some(world.rotation),
                });
                thrown += 1;
            }
        }
        thrown
    }

    /// Части потомков `root` для осколков: у нарезанной модели — её куски на своих местах,
    /// у остальных мешей — меш целиком.
    pub fn parts(&self, root: Entity) -> Vec<Part> {
        let mut parts = Vec::new();
        for entity in self.children.iter_descendants(root) {
            let Ok((mesh, global, material)) = self.meshes.get(entity) else {
                continue;
            };
            if let Some(set) = self.pieces.by_mesh(mesh.0.id()) {
                for piece in &set.pieces {
                    parts.push(Part {
                        mesh: piece.mesh.clone(),
                        material: set.material.clone(),
                        transform: global.mul_transform(Transform::from_translation(piece.center)).compute_transform(),
                        piece: true,
                    });
                }
            } else if let Some(material) = material {
                parts.push(Part {
                    mesh: mesh.0.clone(),
                    material: material.0.clone(),
                    transform: global.compute_transform(),
                    piece: false,
                });
            }
        }
        parts
    }
}

/// Цвет листов брони погибшей машины: графит в копоти.
const ARMOR_COLOR: Color = Color::srgb(0.16, 0.16, 0.15);

/// Гибель машины (враг, танк, босс, Служба): куски корпуса и колёса в копоти разлетаются
/// с её мест, два горят, плюс листы брони из библиотеки. Обломки — независимо от мощности
/// взрыва гибели. `scale` — во сколько раз машина крупнее бронебагги.
pub fn wreck_vehicle(breaker: &mut Breaker, entity: Entity, transform: &Transform, scale: f32) {
    let center = transform.translation;
    let thrown = breaker.burst(
        entity,
        Burst {
            from: center - Vec3::Y * 0.8 * scale,
            speed: 4.0 * scale.sqrt(),
            lift: 6.0 * scale.sqrt(),
            share: 1.0,
            burning: 2,
            charred: true,
        },
    );
    // Без нарезанной модели (запасной облик) — больше листов.
    let sheets = if thrown > 0 { 3 } else { 7 };
    let seed = (center.x * 3.7 + center.z * 1.3).abs();
    for i in 0..sheets {
        let a = seed + i as f32 * 2.399;
        let out = Vec3::new(a.cos(), 0.0, a.sin());
        let k = (a * 7.1).sin().abs();
        breaker.debris.write(SpawnDebris {
            position: center + out * 0.5 * scale + Vec3::Y * 0.6 * scale,
            velocity: out * (3.0 + 4.0 * k) * scale.sqrt() + Vec3::Y * (6.0 + 3.0 * k),
            size: Vec3::new(0.9 + 0.5 * k, 0.25, 0.6 + 0.3 * k) * scale.sqrt(),
            color: ARMOR_COLOR,
            burning: thrown == 0 && i < 2,
            shape: DebrisShape::Form(if i % 3 == 2 { crate::debris_forms::FormKind::Chunk } else { crate::debris_forms::FormKind::Sheet }),
            rotation: None,
        });
    }
}

/// Нарезка модели, которая идёт в пуле фоновых задач.
struct PendingCut {
    id: &'static str,
    mesh: AssetId<Mesh>,
    material: Handle<StandardMaterial>,
    entry: Transform,
    started: bevy::platform::time::Instant,
    task: bevy::tasks::Task<Option<Cut>>,
}

/// Режет загруженные модели в пуле фоновых задач, по одной за раз: k-means и выдавливание кусков
/// на телефоне занимали кадр целиком. Материал берётся у меша сцены модели в мире: glTF хранит свой
/// материал, а сцена рисуется готовым `StandardMaterial`.
#[allow(clippy::too_many_arguments)]
fn cut_models(
    mut pieces: ResMut<ModelPieces>,
    bank: Res<ModelBank>,
    gltfs: Res<Assets<Gltf>>,
    gltf_meshes: Res<Assets<GltfMesh>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    shown: Query<(&Mesh3d, &MeshMaterial3d<StandardMaterial>)>,
    mut frame: Local<u32>,
    mut pending: Local<Option<PendingCut>>,
) {
    if let Some(cut) = pending.as_mut() {
        let Some(result) = bevy::tasks::futures::check_ready(&mut cut.task) else {
            return;
        };
        let PendingCut { id, mesh, material, entry, started, .. } = pending.take().expect("нарезка идёт");
        let Some(cut) = result else {
            warn!("Куски: «{id}» не нарезана — данных сетки нет в памяти игры");
            pieces.sets.insert(id, PieceSet {
                pieces: Vec::new(),
                material: material.clone(),
                charred: material,
                entry,
                low: 0.0,
                high: 1.0,
            });
            return;
        };
        let charred = materials
            .get(&material)
            .cloned()
            .map(|mut copy| {
                let c = copy.base_color.to_linear();
                let k = CHARRED_SHADE;
                copy.base_color = Color::LinearRgba(LinearRgba::new(c.red * k, c.green * k, c.blue * k, c.alpha));
                copy.emissive = copy.emissive * k;
                materials.add(copy)
            })
            .unwrap_or_else(|| material.clone());
        let list = cut
            .pieces
            .into_iter()
            .map(|(mesh, center, half)| ModelPiece {
                mesh: meshes.add(mesh),
                center,
                half,
            })
            .collect::<Vec<_>>();
        let ms = started.elapsed().as_secs_f32() * 1000.0;
        info!("Куски: «{id}» нарезана на {} кусков за {ms:.0} мс (в фоне)", list.len());
        pieces.by_mesh.insert(mesh, id);
        pieces.sets.insert(id, PieceSet {
            pieces: list,
            material,
            charred,
            entry,
            low: cut.low,
            high: cut.high,
        });
        // Следующая модель — в следующем кадре, без паузы поиска.
        *frame = SEARCH_EVERY - 1;
        return;
    }
    *frame += 1;
    if pieces.sets.len() == CUT.len() || *frame % SEARCH_EVERY != 0 {
        return;
    }
    for (id, count) in CUT {
        if pieces.sets.contains_key(id) {
            continue;
        }
        let Some(gltf) = bank.ready_gltf(id).and_then(|h| gltfs.get(&h)) else {
            continue;
        };
        let Some(entry) = bank.entry_transform(id) else {
            continue;
        };
        // Режется самая крупная сетка файла: у вагонов первые сетки — колёса, кузов последний.
        let Some(primitive) = gltf
            .meshes
            .iter()
            .filter_map(|m| gltf_meshes.get(m))
            .flat_map(|m| m.primitives.iter())
            .max_by_key(|p| meshes.get(&p.mesh).map_or(0, |mesh| mesh.count_vertices()))
        else {
            continue;
        };
        let Some(material) = shown.iter().find(|(m, _)| m.0 == primitive.mesh).map(|(_, m)| m.0.clone()) else {
            // Сцена модели ещё не появилась в мире.
            continue;
        };
        // Копия сетки уходит в фоновую задачу; без данных в памяти игры резать нечего.
        let source = meshes.get(&primitive.mesh).map(|mesh| mesh.clone());
        let task = bevy::tasks::AsyncComputeTaskPool::get()
            .spawn(async move { source.and_then(|mesh| cut_mesh(&mesh, count)) });
        *pending = Some(PendingCut {
            id,
            mesh: primitive.mesh.id(),
            material,
            entry,
            started: bevy::platform::time::Instant::now(),
            task,
        });
        // Одна модель за раз.
        return;
    }
}

/// Нарезка: куски (сетка, центр, половина габаритов) и высота модели.
struct Cut {
    pieces: Vec<(Mesh, Vec3, Vec3)>,
    low: f32,
    high: f32,
}

/// Режет сетку на `count` кусков. `None` — данных сетки нет или в ней нет треугольников.
fn cut_mesh(mesh: &Mesh, count: usize) -> Option<Cut> {
    let positions = mesh.try_attribute(Mesh::ATTRIBUTE_POSITION).ok()?.as_float3()?;
    let normals = mesh.try_attribute(Mesh::ATTRIBUTE_NORMAL).ok().and_then(|v| v.as_float3());
    let uvs = match mesh.try_attribute(Mesh::ATTRIBUTE_UV_0).ok() {
        Some(VertexAttributeValues::Float32x2(uvs)) => Some(uvs.as_slice()),
        _ => None,
    };
    let indices: Vec<usize> = match mesh.try_indices_option().ok()? {
        Some(indices) => indices.iter().collect(),
        None => (0..positions.len()).collect(),
    };
    let triangles: Vec<[usize; 3]> = indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
    if triangles.is_empty() {
        return None;
    }
    let source = Source { positions, normals, uvs };
    let centers: Vec<Vec3> = triangles
        .iter()
        .map(|t| (source.p(t[0]) + source.p(t[1]) + source.p(t[2])) / 3.0)
        .collect();
    let (low, high) = positions
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p[1]), hi.max(p[1])));
    let (min, max) = bounds(positions.iter().map(|p| Vec3::from(*p)));
    let thickness = (max - min).max_element() * THICKNESS;

    if count <= 1 {
        // Модель целиком: без толщины, она и так замкнута.
        let mut out = Builder::default();
        for t in &triangles {
            let [pa, pb, pc] = t.map(|i| source.p(i));
            let face = (pb - pa).cross(pc - pa).normalize_or(Vec3::Y);
            for &i in t {
                out.vertex(source.p(i), source.n(i, face), source.uv(i));
            }
        }
        return Some(Cut {
            pieces: vec![finish(out)],
            low,
            high,
        });
    }
    let labels = cluster(&centers, count.min(triangles.len()));
    let mut groups: Vec<Vec<[usize; 3]>> = vec![Vec::new(); count];
    for (triangle, label) in triangles.iter().zip(labels) {
        groups[label].push(*triangle);
    }
    let pieces = groups
        .iter()
        .filter(|group| group.len() >= MIN_TRIANGLES)
        .map(|group| build_piece(&source, group, thickness))
        .collect();
    Some(Cut { pieces, low, high })
}

/// Делит сетку на две по треугольникам: `pick` получает углы треугольника и решает, идёт ли он
/// в первую сетку. Атрибуты — позиции, нормали и UV, сетки без индексов. `None` — данных сетки
/// нет в памяти игры.
pub fn split_mesh(mesh: &Mesh, pick: impl Fn([Vec3; 3]) -> bool) -> Option<(Mesh, Mesh)> {
    let positions = mesh.try_attribute(Mesh::ATTRIBUTE_POSITION).ok()?.as_float3()?;
    let normals = mesh.try_attribute(Mesh::ATTRIBUTE_NORMAL).ok().and_then(|v| v.as_float3());
    let uvs = match mesh.try_attribute(Mesh::ATTRIBUTE_UV_0).ok() {
        Some(VertexAttributeValues::Float32x2(uvs)) => Some(uvs.as_slice()),
        _ => None,
    };
    let indices: Vec<usize> = match mesh.try_indices_option().ok()? {
        Some(indices) => indices.iter().collect(),
        None => (0..positions.len()).collect(),
    };
    let source = Source { positions, normals, uvs };
    let mut parts = [Builder::default(), Builder::default()];
    for t in indices.chunks_exact(3) {
        let corners = [source.p(t[0]), source.p(t[1]), source.p(t[2])];
        let face = (corners[1] - corners[0]).cross(corners[2] - corners[0]).normalize_or(Vec3::Y);
        let part = &mut parts[usize::from(!pick(corners))];
        for &i in t {
            part.vertex(source.p(i), source.n(i, face), source.uv(i));
        }
    }
    let [first, second] = parts.map(|part| {
        let count = part.positions.len() as u32;
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, part.positions.iter().map(|p| p.to_array()).collect::<Vec<_>>())
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, part.normals.iter().map(|n| n.to_array()).collect::<Vec<_>>())
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, part.uvs)
            .with_inserted_indices(Indices::U32((0..count).collect()))
    });
    Some((first, second))
}

/// Вершины исходной сетки.
struct Source<'a> {
    positions: &'a [[f32; 3]],
    normals: Option<&'a [[f32; 3]]>,
    uvs: Option<&'a [[f32; 2]]>,
}

impl Source<'_> {
    fn p(&self, i: usize) -> Vec3 {
        Vec3::from(self.positions[i])
    }

    fn uv(&self, i: usize) -> [f32; 2] {
        self.uvs.map_or([0.0, 0.0], |uvs| uvs[i])
    }

    /// Нормаль вершины, а без нормалей в файле — нормаль треугольника.
    fn n(&self, i: usize, face: Vec3) -> Vec3 {
        self.normals.map_or(face, |normals| Vec3::from(normals[i]).normalize_or(face))
    }
}

fn bounds(points: impl Iterator<Item = Vec3>) -> (Vec3, Vec3) {
    points.fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(p), hi.max(p)))
}

/// K-средних по точкам с начальными центрами по самым далёким точкам: без случайности,
/// одни и те же метки в каждом запуске.
fn cluster(points: &[Vec3], count: usize) -> Vec<usize> {
    let count = count.max(1);
    let mut seeds = vec![points[0]];
    let mut nearest: Vec<f32> = points.iter().map(|p| p.distance_squared(points[0])).collect();
    while seeds.len() < count {
        let (far, _) = nearest
            .iter()
            .enumerate()
            .fold((0, -1.0), |best, (i, &d)| if d > best.1 { (i, d) } else { best });
        let seed = points[far];
        seeds.push(seed);
        for (d, p) in nearest.iter_mut().zip(points) {
            *d = d.min(p.distance_squared(seed));
        }
    }
    let mut labels = vec![0; points.len()];
    for _ in 0..ITERATIONS {
        for (label, p) in labels.iter_mut().zip(points) {
            *label = seeds
                .iter()
                .enumerate()
                .map(|(k, s)| (k, p.distance_squared(*s)))
                .fold((0, f32::MAX), |best, (k, d)| if d < best.1 { (k, d) } else { best })
                .0;
        }
        let mut sums = vec![(Vec3::ZERO, 0usize); count];
        for (label, p) in labels.iter().zip(points) {
            sums[*label].0 += *p;
            sums[*label].1 += 1;
        }
        for (seed, (sum, n)) in seeds.iter_mut().zip(sums) {
            if n > 0 {
                *seed = sum / n as f32;
            }
        }
    }
    labels
}

/// Ключ вершины для поиска краёв: координаты с точностью до десятой доли миллиметра модели.
fn key(p: Vec3) -> [i32; 3] {
    (p * 1e4).round().as_ivec3().to_array()
}

/// Вершины собираемой сетки куска.
#[derive(Default)]
struct Builder {
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    uvs: Vec<[f32; 2]>,
}

impl Builder {
    fn vertex(&mut self, p: Vec3, n: Vec3, uv: [f32; 2]) {
        self.positions.push(p);
        self.normals.push(n);
        self.uvs.push(uv);
    }

    /// Треугольник с нормалью `hint` у всех вершин: обход разворачивается под неё.
    fn flat(&mut self, corners: [(Vec3, [f32; 2]); 3], hint: Vec3) {
        let [a, b, c] = corners;
        let face = (b.0 - a.0).cross(c.0 - a.0);
        let ordered = if face.dot(hint) >= 0.0 { [a, b, c] } else { [a, c, b] };
        for (p, uv) in ordered {
            self.vertex(p, hint, uv);
        }
    }
}

/// Сетка куска: лицевая сторона из исходных треугольников, обратная сдвинута против нормали
/// на `thickness`, по краям — бока. Начало сетки — центр габаритов куска.
fn build_piece(source: &Source, triangles: &[[usize; 3]], thickness: f32) -> (Mesh, Vec3, Vec3) {
    let mut out = Builder::default();

    // Рёбра куска по обходу: край — ребро, обратного которому в куске нет.
    let edges: HashSet<([i32; 3], [i32; 3])> = triangles
        .iter()
        .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
        .map(|(a, b)| (key(source.p(a)), key(source.p(b))))
        .collect();

    for t in triangles {
        let [pa, pb, pc] = t.map(|i| source.p(i));
        let face = (pb - pa).cross(pc - pa).normalize_or(Vec3::Y);
        let inner = |i: usize| source.p(i) - source.n(i, face) * thickness;
        // Лицевая сторона: исходные нормали и обход.
        for &i in t {
            out.vertex(source.p(i), source.n(i, face), source.uv(i));
        }
        // Обратная сторона.
        out.flat([(inner(t[0]), source.uv(t[0])), (inner(t[1]), source.uv(t[1])), (inner(t[2]), source.uv(t[2]))], -face);
        // Бока по краям куска.
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            if edges.contains(&(key(source.p(b)), key(source.p(a)))) {
                continue;
            }
            let outward = (source.p(b) - source.p(a)).cross(face).normalize_or(face);
            let (ua, ub) = (source.uv(a), source.uv(b));
            out.flat([(source.p(a), ua), (source.p(b), ub), (inner(b), ub)], outward);
            out.flat([(source.p(a), ua), (inner(b), ub), (inner(a), ua)], outward);
        }
    }

    finish(out)
}

/// Готовая сетка куска с началом в центре его габаритов.
fn finish(out: Builder) -> (Mesh, Vec3, Vec3) {
    let Builder { positions, normals, uvs } = out;
    let (min, max) = bounds(positions.iter().copied());
    let center = (min + max) * 0.5;
    let half = ((max - min) * 0.5).max(Vec3::splat(0.01));
    let positions: Vec<[f32; 3]> = positions.into_iter().map(|p| (p - center).to_array()).collect();
    let normals: Vec<[f32; 3]> = normals.into_iter().map(|n| n.to_array()).collect();
    let count = positions.len() as u32;
    let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32((0..count).collect()));
    (mesh, center, half)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Плоская сетка 8×8 клеток в плоскости XZ.
    fn grid() -> Mesh {
        let n = 9;
        let positions: Vec<[f32; 3]> = (0..n * n).map(|i| [(i % n) as f32, 0.0, (i / n) as f32]).collect();
        let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
        let uvs = vec![[0.0, 0.0]; positions.len()];
        let mut indices = Vec::new();
        for z in 0..n - 1 {
            for x in 0..n - 1 {
                let a = (z * n + x) as u32;
                indices.extend_from_slice(&[a, a + n as u32, a + 1, a + 1, a + n as u32, a + n as u32 + 1]);
            }
        }
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
            .with_inserted_indices(Indices::U32(indices))
    }

    #[test]
    fn куски_одинаковы_между_запусками_и_покрывают_сетку() {
        let mesh = grid();
        let first = cut_mesh(&mesh, 4).expect("нарезка");
        let second = cut_mesh(&mesh, 4).expect("нарезка");
        assert_eq!(first.pieces.len(), 4);
        let centers = |cut: &Cut| cut.pieces.iter().map(|p| p.1).collect::<Vec<_>>();
        assert_eq!(centers(&first), centers(&second));
        // Лицевых треугольников в кусках не меньше, чем в сетке: у каждого куска ещё обратная
        // сторона и бока.
        let triangles: usize = first.pieces.iter().map(|p| p.0.count_vertices() / 3).sum();
        assert!(triangles >= 2 * 128, "треугольников {triangles}");
    }

    #[test]
    fn кусок_толщиной_вниз_от_лица() {
        let cut = cut_mesh(&grid(), 2).expect("нарезка");
        for (_, center, half) in &cut.pieces {
            // Сетка 8 × 8, толщина 3,5 % от 8.
            assert!((half.y * 2.0 - 8.0 * THICKNESS).abs() < 1e-3);
            assert!(center.y < 0.0);
        }
    }

    #[test]
    fn модель_из_одного_куска_целиком_без_толщины() {
        let cut = cut_mesh(&grid(), 1).expect("нарезка");
        let (mesh, center, half) = &cut.pieces[0];
        assert_eq!(mesh.count_vertices(), 128 * 3);
        assert!((half.x - 4.0).abs() < 1e-3);
        assert!(center.y.abs() < 1e-3);
    }
}
