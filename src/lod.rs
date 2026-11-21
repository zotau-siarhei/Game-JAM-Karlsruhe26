//! Облегчение моделей для телефона (профиль платформы, раздел `models`): мобильной видеокарте не по силам
//! тысячи деревьев по 2,5 тысячи треугольников и текстуры 2048 без mip-уровней.
//!
//! - Сетки моделей с полем `mobile_detail` в `models.json` упрощает meshoptimizer: схлопывает
//!   рёбра, следя за ошибкой формы, текстурных координат и нормалей; через швы текстуры —
//!   тоже (`Permissive`), иначе сетка Meshy, где у каждой точки три-четыре копии вершины, почти
//!   не упрощается. Вершины остаются прежними, меняется только список треугольников.
//! - Текстуры моделей уменьшаются до `models.texture_max` профиля и получают mip-уровни: без них каждый
//!   пиксель далёкой модели читает случайное место большой текстуры, кэш видеокарты не работает.
//!   Уровни усредняются в линейном цвете.
//!
//! - Модели с полем `mobile_shadow` отбрасывают тень грубым силуэтом ([`shadow_proxy`]): проход
//!   теней рисует каждое дерево заново, а для тени мелкие детали листвы не нужны. Силуэт лежит на слое
//!   [`SHADOW_LAYER`], который видит только солнце, а сама модель тени не отбрасывает
//!   (`models::attach_shadow_proxies`).
//!
//! Упрощение сеток — только в нативной сборке: meshoptimizer написан на C++.

use bevy::asset::AssetEventSystems;
#[cfg(not(target_arch = "wasm32"))]
use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;
use bevy::render::render_resource::{TextureDimension, TextureFormat};

/// Слой отрисовки силуэтов для тени: его видит солнце (`main.rs`), но не камера.
pub const SHADOW_LAYER: usize = 1;


pub struct LodPlugin;

impl Plugin for LodPlugin {
    fn build(&self, app: &mut App) {
        if crate::profile::get().models.texture_max.is_some() {
            app.add_systems(PostUpdate, shrink_model_textures.after(AssetEventSystems));
        }
    }
}

/// Уменьшает только что загруженные текстуры моделей до передачи в рендер: данные текстур моделей
/// живут только в рендер-мире, поэтому их надо успеть изменить в кадре загрузки.
fn shrink_model_textures(
    mut events: MessageReader<AssetEvent<Image>>,
    mut images: ResMut<Assets<Image>>,
    asset_server: Res<AssetServer>,
) {
    let Some(max_side) = crate::profile::get().models.texture_max else {
        return;
    };
    for event in events.read() {
        let AssetEvent::Added { id } = event else {
            continue;
        };
        let from_model = asset_server
            .get_path(*id)
            .is_some_and(|path| path.path().extension().is_some_and(|ext| ext == "glb"));
        if !from_model {
            continue;
        }
        if let Some(mut image) = images.get_mut(*id) {
            let before = image.texture_descriptor.size;
            if shrink_texture(&mut image, max_side) {
                let after = &image.texture_descriptor;
                info!(
                    "Текстура модели для телефона: {}×{} → {}×{}, уровней {}",
                    before.width, before.height, after.size.width, after.size.height, after.mip_level_count
                );
            }
        }
    }
}

/// Уменьшает текстуру RGBA8 до стороны не больше `max_side` и строит mip-уровни до 1×1.
/// `false` — формат не тот, уровни уже есть или данных нет.
pub fn shrink_texture(image: &mut Image, max_side: u32) -> bool {
    let descriptor = &image.texture_descriptor;
    let srgb = match descriptor.format {
        TextureFormat::Rgba8UnormSrgb => true,
        TextureFormat::Rgba8Unorm => false,
        _ => return false,
    };
    if descriptor.mip_level_count != 1
        || descriptor.dimension != TextureDimension::D2
        || descriptor.size.depth_or_array_layers != 1
    {
        return false;
    }
    let (mut width, mut height) = (descriptor.size.width, descriptor.size.height);
    let Some(mut level) = image.data.take() else {
        return false;
    };
    if level.len() != (width * height * 4) as usize {
        image.data = Some(level);
        return false;
    }
    let tables = ColorTables::new(srgb);
    while width.max(height) > max_side {
        (level, width, height) = half(&level, width, height, &tables);
    }
    let size = (width, height);
    let mut data = level.clone();
    let mut levels = 1;
    while width > 1 || height > 1 {
        (level, width, height) = half(&level, width, height, &tables);
        data.extend_from_slice(&level);
        levels += 1;
    }
    image.texture_descriptor.size.width = size.0;
    image.texture_descriptor.size.height = size.1;
    image.texture_descriptor.mip_level_count = levels;
    image.data = Some(data);
    true
}

/// Перевод канала цвета в линейный и обратно: у sRGB по таблицам, у линейной текстуры как есть.
struct ColorTables {
    to_linear: [f32; 256],
    /// Линейное значение, квантованное в 4096 ступеней, → байт.
    from_linear: Vec<u8>,
}

impl ColorTables {
    fn new(srgb: bool) -> Self {
        let decode = |v: f32| if srgb { srgb_to_linear(v) } else { v };
        let encode = |v: f32| if srgb { linear_to_srgb(v) } else { v };
        let mut to_linear = [0.0; 256];
        for (i, value) in to_linear.iter_mut().enumerate() {
            *value = decode(i as f32 / 255.0);
        }
        let from_linear = (0..4096).map(|i| (encode(i as f32 / 4095.0) * 255.0).round() as u8).collect();
        Self { to_linear, from_linear }
    }

    fn encode(&self, linear: f32) -> u8 {
        self.from_linear[(linear.clamp(0.0, 1.0) * 4095.0).round() as usize]
    }
}

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// Уровень вдвое меньше: каждый пиксель — среднее квадрата 2×2 (у нечётной стороны — края).
/// Цвет усредняется в линейном пространстве, прозрачность — как есть.
fn half(level: &[u8], width: u32, height: u32, tables: &ColorTables) -> (Vec<u8>, u32, u32) {
    let (w, h) = ((width / 2).max(1), (height / 2).max(1));
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let rows = [(2 * y).min(height - 1), (2 * y + 1).min(height - 1)];
        for x in 0..w {
            let columns = [(2 * x).min(width - 1), (2 * x + 1).min(width - 1)];
            let mut sum = [0.0f32; 4];
            for row in rows {
                for column in columns {
                    let i = ((row * width + column) * 4) as usize;
                    for c in 0..3 {
                        sum[c] += tables.to_linear[level[i + c] as usize];
                    }
                    sum[3] += f32::from(level[i + 3]) / 255.0;
                }
            }
            for value in &sum[..3] {
                out.push(tables.encode(value / 4.0));
            }
            out.push((sum[3] / 4.0 * 255.0).round() as u8);
        }
    }
    (out, w, h)
}

/// Вес ошибки текстурных координат относительно ошибки формы.
#[cfg(not(target_arch = "wasm32"))]
const UV_WEIGHT: f32 = 1.0;
/// Вес ошибки нормалей.
#[cfg(not(target_arch = "wasm32"))]
const NORMAL_WEIGHT: f32 = 0.5;
/// Допустимая ошибка формы относительно размера модели.
#[cfg(not(target_arch = "wasm32"))]
const MAX_ERROR: f32 = 0.05;

/// Упрощает сетку до доли `detail` треугольников (0 < detail < 1). Возвращает треугольники
/// до и после; `None` — сетка без позиций или треугольников либо уже отдана видеокарте.
#[cfg(not(target_arch = "wasm32"))]
pub fn simplify(mesh: &mut Mesh, detail: f32) -> Option<(usize, usize)> {
    use meshopt::{SimplifyOptions, SimplifyVertex, VertexDataAdapter};

    let Ok(Some(VertexAttributeValues::Float32x3(positions))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION)
    else {
        return None;
    };
    let uvs = match mesh.try_attribute_option(Mesh::ATTRIBUTE_UV_0) {
        Ok(Some(VertexAttributeValues::Float32x2(uvs))) if uvs.len() == positions.len() => Some(uvs),
        _ => None,
    };
    let normals = match mesh.try_attribute_option(Mesh::ATTRIBUTE_NORMAL) {
        Ok(Some(VertexAttributeValues::Float32x3(normals))) if normals.len() == positions.len() => Some(normals),
        _ => None,
    };
    let indices: Vec<u32> = match mesh.try_indices_option().ok()?? {
        Indices::U16(list) => list.iter().map(|&i| u32::from(i)).collect(),
        Indices::U32(list) => list.clone(),
    };
    if indices.len() < 3 || indices.len() % 3 != 0 {
        return None;
    }
    // Признаки вершины для ошибки упрощения: UV и нормаль подряд.
    const STRIDE: usize = 5;
    let mut attributes = Vec::with_capacity(positions.len() * STRIDE);
    for i in 0..positions.len() {
        attributes.extend_from_slice(&uvs.map_or([0.0; 2], |uvs| uvs[i]));
        attributes.extend_from_slice(&normals.map_or([0.0; 3], |normals| normals[i]));
    }
    let weights = [UV_WEIGHT, UV_WEIGHT, NORMAL_WEIGHT, NORMAL_WEIGHT, NORMAL_WEIGHT];
    let vertices = VertexDataAdapter::new(meshopt::typed_to_bytes(positions), 12, 0).ok()?;
    let locks = vec![SimplifyVertex::None; positions.len()];
    let target = ((indices.len() as f32 * detail) as usize / 3 * 3).max(3);
    let result = meshopt::simplify_with_attributes_and_locks(
        &indices,
        &vertices,
        &attributes,
        &weights,
        STRIDE * size_of::<f32>(),
        &locks,
        target,
        MAX_ERROR,
        SimplifyOptions::Permissive,
        None,
    );
    if result.is_empty() {
        return None;
    }
    let before = indices.len() / 3;
    let after = result.len() / 3;
    mesh.try_insert_indices(Indices::U32(result)).ok()?;
    Some((before, after))
}

#[cfg(target_arch = "wasm32")]
pub fn simplify(_mesh: &mut Mesh, _detail: f32) -> Option<(usize, usize)> {
    None
}

/// Допустимая ошибка силуэта для тени относительно размера модели.
#[cfg(not(target_arch = "wasm32"))]
const SHADOW_ERROR: f32 = 0.2;

/// Грубый силуэт сетки для прохода теней: доля `detail` треугольников. Вершины берутся из исходной сетки со всеми её признаками, поэтому
/// раскладка та же и конвейер тени тот же, что у модели: прогреву ничего нового не нужно.
#[cfg(not(target_arch = "wasm32"))]
pub fn shadow_proxy(mesh: &Mesh, detail: f32) -> Option<Mesh> {
    use bevy::asset::RenderAssetUsages;
    use meshopt::VertexDataAdapter;

    let Ok(Some(VertexAttributeValues::Float32x3(positions))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION)
    else {
        return None;
    };
    let indices: Vec<u32> = match mesh.try_indices_option().ok()?? {
        Indices::U16(list) => list.iter().map(|&i| u32::from(i)).collect(),
        Indices::U32(list) => list.clone(),
    };
    if indices.len() < 3 || indices.len() % 3 != 0 {
        return None;
    }
    let vertices = VertexDataAdapter::new(meshopt::typed_to_bytes(positions), 12, 0).ok()?;
    let target = ((indices.len() as f32 * detail) as usize / 3 * 3).max(3);
    // Тени не нужны швы текстуры: вершины свариваются по положению, и сетка упрощается
    // как сплошная, сохраняя силуэт.
    let remap = meshopt::generate_position_remap(&vertices);
    let welded: Vec<u32> = indices.iter().map(|&i| remap[i as usize]).collect();
    let options = meshopt::SimplifyOptions::Permissive | meshopt::SimplifyOptions::Prune;
    let picked = meshopt::simplify(&welded, &vertices, target, SHADOW_ERROR, options, None);
    if picked.is_empty() {
        return None;
    }
    // Только вершины, на которые ссылаются оставшиеся треугольники.
    let mut remap = vec![u32::MAX; positions.len()];
    let mut order = Vec::new();
    let mut compact = Vec::with_capacity(picked.len());
    for &old in &picked {
        let slot = &mut remap[old as usize];
        if *slot == u32::MAX {
            *slot = order.len() as u32;
            order.push(old as usize);
        }
        compact.push(*slot);
    }
    let mut proxy = Mesh::new(mesh.primitive_topology(), RenderAssetUsages::default());
    for (attribute, values) in mesh.try_attributes().ok()? {
        let values = match values {
            VertexAttributeValues::Float32x2(v) => VertexAttributeValues::Float32x2(order.iter().map(|&i| v[i]).collect()),
            VertexAttributeValues::Float32x3(v) => VertexAttributeValues::Float32x3(order.iter().map(|&i| v[i]).collect()),
            VertexAttributeValues::Float32x4(v) => VertexAttributeValues::Float32x4(order.iter().map(|&i| v[i]).collect()),
            // Других признаков у моделей с силуэтом нет; с ними раскладка не совпала бы.
            _ => return None,
        };
        proxy.insert_attribute(attribute.clone(), values);
    }
    proxy.insert_indices(Indices::U32(compact));
    Some(proxy)
}

#[cfg(target_arch = "wasm32")]
pub fn shadow_proxy(_mesh: &Mesh, _detail: f32) -> Option<Mesh> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;

    #[test]
    fn шар_упрощается_до_доли_треугольников() {
        let mut mesh = Sphere::new(1.0).mesh().ico(20).unwrap();
        mesh.asset_usage = RenderAssetUsages::all();
        let (before, after) = simplify(&mut mesh, 0.25).unwrap();
        assert!(before > 1000);
        assert!(after <= before / 3, "{before} → {after}");
        assert!(after > 0);
    }

    #[test]
    fn силуэт_грубее_и_с_той_же_раскладкой() {
        let mut mesh = Sphere::new(1.0).mesh().ico(20).unwrap();
        mesh.asset_usage = RenderAssetUsages::all();
        let before = mesh.indices().unwrap().len() / 3;
        let proxy = shadow_proxy(&mesh, 0.05).unwrap();
        let after = proxy.indices().unwrap().len() / 3;
        assert!(after * 10 < before, "{before} → {after}");
        let ids = |m: &Mesh| m.attributes().map(|(a, values)| (a.id, values.len() > 0)).collect::<Vec<_>>();
        assert_eq!(ids(&proxy), ids(&mesh));
        // Вершин не больше, чем нужно оставшимся треугольникам.
        assert!(proxy.count_vertices() <= after * 3);
    }

    #[test]
    fn текстура_уменьшается_и_получает_уровни() {
        let mut image = Image::new_fill(
            bevy::render::render_resource::Extent3d { width: 64, height: 32, depth_or_array_layers: 1 },
            TextureDimension::D2,
            &[200, 100, 50, 255],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::all(),
        );
        assert!(shrink_texture(&mut image, 16));
        let descriptor = &image.texture_descriptor;
        assert_eq!((descriptor.size.width, descriptor.size.height), (16, 8));
        // 16×8, 8×4, 4×2, 2×1, 1×1.
        assert_eq!(descriptor.mip_level_count, 5);
        let data = image.data.as_ref().unwrap();
        assert_eq!(data.len(), (16 * 8 + 8 * 4 + 4 * 2 + 2 + 1) * 4);
        // Однотонная текстура остаётся того же цвета на всех уровнях.
        assert!(data.chunks(4).all(|p| p == [200, 100, 50, 255]));
    }
}
