//! Банк моделей Meshy: список `assets/models/models.json`, облик объекта из модели, затемнение.
//!
//! Список встраивается в игру при сборке. Объект спрашивает облик по id через [`Models::look`]:
//! если id в списке нет, объект строит свой запасной облик из примитивов, как раньше. Масштаб,
//! поворот вокруг вертикали и подъём модели берутся из списка, поэтому подгонка модели под объект —
//! правка `models.json`, а не кода.
//!
//! Все модели грузятся при старте, чтобы первый поезд или босс не ждал диска. Когда модель
//! загрузилась, считаются её треугольники: сверх бюджета из списка — предупреждение в лог.
//!
//! [`ModelShade`] затемняет все меши под сущностью, сохраняя текстуры: закопчённый дом, обугленный
//! остов вагона. Материалы модели лежат на дочерних сущностях сцены, которые появляются не сразу,
//! поэтому затемнение доходит до них, когда сцена разложилась.

use std::collections::HashMap;

use bevy::gltf::Gltf;
use bevy::gltf::GltfMesh;
use bevy::prelude::*;
use serde::Deserialize;

use crate::combat::HitFlash;

/// Список моделей, встроенный при сборке.
const LIST: &str = include_str!("../assets/models/models.json");

/// Плагин банка моделей.
pub struct ModelsPlugin;

impl Plugin for ModelsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Models>()
            .init_resource::<ShadeCache>()
            .add_systems(Update, (check_budgets, apply_shade));
    }
}

/// Запись списка моделей.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelEntry {
    pub id: String,
    pub file: String,
    /// Равномерный масштаб модели.
    pub scale: f32,
    /// Поворот вокруг вертикали, рад: ставит перёд модели туда, куда смотрит объект.
    pub rotation_y: f32,
    /// Подъём модели над началом координат объекта, м.
    pub offset_y: f32,
    /// Бюджет треугольников.
    pub budget: usize,
}

#[derive(Deserialize)]
struct ModelList {
    models: Vec<ModelEntry>,
}

/// Разбирает список моделей.
fn parse_list(text: &str) -> Vec<ModelEntry> {
    match serde_json::from_str::<ModelList>(text) {
        Ok(list) => list.models,
        Err(error) => {
            warn!("models.json не разобран: {error}");
            Vec::new()
        }
    }
}

/// Загруженная модель.
struct Loaded {
    entry: ModelEntry,
    scene: Handle<WorldAsset>,
    gltf: Handle<Gltf>,
    /// Треугольники уже посчитаны.
    checked: bool,
}

/// Банк моделей по id.
#[derive(Resource)]
pub struct Models {
    models: HashMap<String, Loaded>,
}

impl FromWorld for Models {
    fn from_world(world: &mut World) -> Self {
        // В тестах без ассетов банк пуст: объекты строят запасной облик.
        let Some(server) = world.get_resource::<AssetServer>() else {
            return Self {
                models: HashMap::new(),
            };
        };
        let models = parse_list(LIST)
            .into_iter()
            .map(|entry| {
                let path = format!("models/{}", entry.file);
                let scene = server.load(GltfAssetLabel::Scene(0).from_asset(path.clone()));
                let gltf = server.load(path);
                (
                    entry.id.clone(),
                    Loaded {
                        entry,
                        scene,
                        gltf,
                        checked: false,
                    },
                )
            })
            .collect();
        Self { models }
    }
}

impl Models {
    /// Облик из модели `id` в системе объекта, сдвинутый на `at`. `None` — модели нет.
    pub fn look(&self, id: &str, at: Transform) -> Option<(WorldAssetRoot, Transform)> {
        let loaded = self.models.get(id)?;
        Some((
            WorldAssetRoot(loaded.scene.clone()),
            at * model_transform(&loaded.entry),
        ))
    }
}

/// Преобразование модели из списка: подъём, поворот, масштаб.
fn model_transform(entry: &ModelEntry) -> Transform {
    Transform::from_xyz(0.0, entry.offset_y, 0.0)
        .with_rotation(Quat::from_rotation_y(entry.rotation_y))
        .with_scale(Vec3::splat(entry.scale))
}

/// Считает треугольники загруженных моделей и предупреждает о превышении бюджета.
fn check_budgets(
    mut models: ResMut<Models>,
    gltfs: Res<Assets<Gltf>>,
    gltf_meshes: Res<Assets<GltfMesh>>,
    meshes: Res<Assets<Mesh>>,
) {
    for loaded in models.models.values_mut().filter(|m| !m.checked) {
        let Some(gltf) = gltfs.get(&loaded.gltf) else {
            continue;
        };
        let Some(triangles) = count_triangles(gltf, &gltf_meshes, &meshes) else {
            continue;
        };
        loaded.checked = true;
        if triangles > loaded.entry.budget {
            warn!(
                "модель {}: {triangles} треугольников, бюджет {}",
                loaded.entry.id, loaded.entry.budget
            );
        }
    }
}

/// Треугольники модели. `None` — меши ещё грузятся.
fn count_triangles(
    gltf: &Gltf,
    gltf_meshes: &Assets<GltfMesh>,
    meshes: &Assets<Mesh>,
) -> Option<usize> {
    let mut triangles = 0;
    for handle in &gltf.meshes {
        for primitive in &gltf_meshes.get(handle)?.primitives {
            let mesh = meshes.get(&primitive.mesh)?;
            triangles += mesh
                .indices()
                .map_or(mesh.count_vertices(), |indices| indices.len())
                / 3;
        }
    }
    Some(triangles)
}

/// Затемнение всех мешей под сущностью: 1 — как есть, 0 — чёрный. Текстуры остаются.
#[derive(Component, Clone, Copy)]
pub struct ModelShade(pub f32);

/// Затемнённые копии материалов и их исходники.
#[derive(Resource, Default)]
struct ShadeCache {
    /// (исходный материал, ступень затемнения) → копия.
    shaded: HashMap<(AssetId<StandardMaterial>, u32), Handle<StandardMaterial>>,
    /// Копия → исходный материал: повторное затемнение считается от исходника.
    original: HashMap<AssetId<StandardMaterial>, Handle<StandardMaterial>>,
}

/// Ступень затемнения для ключа кэша: сотые доли.
fn shade_step(shade: f32) -> u32 {
    (shade.clamp(0.0, 1.0) * 100.0).round() as u32
}

/// Подменяет материалы мешей под [`ModelShade`] затемнёнными копиями.
fn apply_shade(
    mut cache: ResMut<ShadeCache>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    roots: Query<(Entity, &ModelShade)>,
    children: Query<&Children>,
    mut parts: Query<&mut MeshMaterial3d<StandardMaterial>, Without<HitFlash>>,
) {
    for (root, shade) in &roots {
        let step = shade_step(shade.0);
        for part in children.iter_descendants(root) {
            let Ok(mut material) = parts.get_mut(part) else {
                continue;
            };
            let original = cache
                .original
                .get(&material.0.id())
                .cloned()
                .unwrap_or_else(|| material.0.clone());
            let key = (original.id(), step);
            if let Some(shaded) = cache.shaded.get(&key) {
                if material.0 != *shaded {
                    material.0 = shaded.clone();
                }
                continue;
            }
            let Some(mut copy) = materials.get(&original).cloned() else {
                continue;
            };
            let k = step as f32 / 100.0;
            let c = copy.base_color.to_linear();
            copy.base_color =
                Color::LinearRgba(LinearRgba::new(c.red * k, c.green * k, c.blue * k, c.alpha));
            copy.emissive = copy.emissive * k;
            let shaded = materials.add(copy);
            cache.original.insert(shaded.id(), original);
            cache.shaded.insert(key, shaded.clone());
            material.0 = shaded;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn список_моделей_разбирается() {
        let list = parse_list(LIST);
        assert!(list.len() >= 20);
        for id in [
            "train_loco",
            "train_wagon_passenger",
            "train_wagon_freight",
            "train_wagon_tank",
            "house_small",
            "house_two_story",
            "house_barrack",
            "boss_body",
            "boss_leg",
            "weapon_mg",
            "weapon_mortar",
            "weapon_rockets",
            "weapon_laser",
        ] {
            assert!(list.iter().any(|m| m.id == id), "нет модели {id}");
        }
    }

    #[test]
    fn файлы_моделей_на_месте() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/models");
        for entry in parse_list(LIST) {
            assert!(dir.join(&entry.file).exists(), "нет файла {}", entry.file);
        }
    }

    #[test]
    fn без_ассетов_банк_пуст_и_облика_нет() {
        let mut world = World::new();
        let models = Models::from_world(&mut world);
        assert!(models.look("train_loco", Transform::IDENTITY).is_none());
    }

    #[test]
    fn затемнение_считается_от_исходника() {
        let mut app = App::new();
        app.init_resource::<Assets<StandardMaterial>>()
            .init_resource::<ShadeCache>()
            .add_systems(Update, apply_shade);
        let original = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color: Color::LinearRgba(LinearRgba::rgb(1.0, 0.5, 0.2)),
                ..default()
            });
        let root = app.world_mut().spawn(ModelShade(0.5)).id();
        let part = app
            .world_mut()
            .spawn((MeshMaterial3d(original.clone()), ChildOf(root)))
            .id();
        let color = |app: &App| {
            let handle = &app
                .world()
                .get::<MeshMaterial3d<StandardMaterial>>(part)
                .unwrap()
                .0;
            app.world()
                .resource::<Assets<StandardMaterial>>()
                .get(handle)
                .unwrap()
                .base_color
                .to_linear()
        };
        app.update();
        assert!((color(&app).red - 0.5).abs() < 1e-4);
        // Сильнее затемнение — снова от исходного цвета, а не поверх прошлого.
        app.world_mut().entity_mut(root).insert(ModelShade(0.2));
        app.update();
        assert!((color(&app).red - 0.2).abs() < 1e-4);
        app.update();
        assert!((color(&app).red - 0.2).abs() < 1e-4);
    }
}
