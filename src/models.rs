//! Модели из Meshy: список `assets/models/models.json` и облик объектов по идентификатору.
//!
//! Список — файл JSON вида `{"models": [{"id", "file", "scale", "rotation_y", "offset_y",
//! "budget"}]}`, файлы моделей (GLB) лежат рядом со списком. Модель подключается записью в списке,
//! без изменения кода: код просит облик по идентификатору объекта через
//! [`Models::spawn_visual`] и передаёт запасной облик из примитивов.
//!
//! Запасной облик виден, пока модель грузится, и остаётся, если записи или файла нет либо файл
//! не загрузился. Файл читается при первом запросе, а не при старте: модель, которую никто
//! не запросил, не занимает память.
//!
//! [`ModelShade`] затемняет все меши под сущностью, сохраняя текстуры: закопчённый дом, обугленный
//! остов вагона. Части модели появляются не сразу, поэтому затемнение доходит до них, когда
//! сцена разложилась.

use std::collections::HashMap;

use crate::combat::FlashedMesh;

use bevy::asset::io::file::FileAssetReader;
use bevy::asset::{LoadState, RecursiveDependencyLoadState};
use bevy::ecs::system::SystemParam;
use bevy::gltf::{Gltf, GltfMesh};
use bevy::prelude::*;
use serde::Deserialize;

/// Каталог моделей внутри `assets` и имя списка в нём.
const MODELS_DIR: &str = "models";
const MODELS_LIST: &str = "models.json";

/// Подключает список моделей и замену запасного облика загруженной моделью.
pub struct ModelsPlugin;

impl Plugin for ModelsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ModelBank::load(MODELS_DIR, MODELS_LIST))
            .init_resource::<ShadeCache>()
            .add_systems(
                Update,
                (track_loading, swap_fallbacks, apply_shade)
                    .chain()
                    .in_set(crate::frame::GameSet::Presentation),
            );
    }
}

/// Одна запись списка моделей.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ModelEntry {
    /// Идентификатор, по которому код игры просит облик объекта.
    pub id: String,
    /// Имя файла GLB рядом со списком.
    #[serde(default)]
    pub file: String,
    /// Масштаб модели. Игровые размеры и столкновения от него не зависят.
    #[serde(default = "unit_scale")]
    pub scale: f32,
    /// Поворот вокруг вертикали, рад. Объекты игры смотрят на -Z: модели, которая смотрит
    /// на +Z, нужен поворот на π.
    #[serde(default)]
    pub rotation_y: f32,
    /// Высота начала координат модели над низом объекта, м.
    #[serde(default)]
    pub offset_y: f32,
    /// Бюджет треугольников. Ноль — без проверки.
    #[serde(default)]
    pub budget: usize,
}

fn unit_scale() -> f32 {
    1.0
}

impl ModelEntry {
    /// Положение модели относительно низа объекта.
    pub fn transform(&self) -> Transform {
        Transform::from_xyz(0.0, self.offset_y, 0.0)
            .with_rotation(Quat::from_rotation_y(self.rotation_y))
            .with_scale(Vec3::splat(self.scale))
    }
}

/// Содержимое файла списка.
#[derive(Deserialize)]
struct ListFile {
    models: Vec<ModelEntry>,
}

/// Разбирает текст списка.
pub fn parse_list(json: &str) -> Result<Vec<ModelEntry>, serde_json::Error> {
    serde_json::from_str::<ListFile>(json).map(|file| file.models)
}

/// Загруженная или загружаемая модель: весь файл и его сцена.
struct Handles {
    gltf: Handle<Gltf>,
    scene: Handle<WorldAsset>,
}

/// Что сейчас с моделью.
enum ModelState {
    /// Файла рядом со списком нет: остаётся запасной облик.
    Missing,
    /// Файл есть, модель ещё никто не просил.
    Idle,
    Loading(Handles),
    Ready(Handles),
    /// Файл не загрузился: остаётся запасной облик.
    Failed,
}

/// Запись списка и состояние её файла.
struct Model {
    entry: ModelEntry,
    /// Путь к файлу для загрузчика: от каталога `assets`.
    path: String,
    state: ModelState,
}

/// Модели по идентификатору.
#[derive(Resource, Default)]
pub struct ModelBank {
    models: HashMap<String, Model>,
}

impl ModelBank {
    /// Читает `assets/<dir>/<file>` и отмечает, у каких записей есть файл модели.
    ///
    /// Ошибки не останавливают игру: о них пишется в лог, объекты получают запасной облик.
    pub fn load(dir: &str, file: &str) -> Self {
        let mut bank = Self::default();
        let dir_path = FileAssetReader::get_base_path().join("assets").join(dir);
        let list_path = dir_path.join(file);

        let json = match std::fs::read_to_string(&list_path) {
            Ok(json) => json,
            Err(error) => {
                warn!("Модели: не прочитан {}: {error}", list_path.display());
                return bank;
            }
        };
        let entries = match parse_list(&json) {
            Ok(entries) => entries,
            Err(error) => {
                warn!("Модели: ошибка в {}: {error}", list_path.display());
                return bank;
            }
        };

        let mut missing = Vec::new();
        for entry in entries {
            let exists = !entry.file.is_empty() && dir_path.join(&entry.file).is_file();
            if !exists {
                missing.push(entry.id.clone());
            }
            let id = entry.id.clone();
            let model = Model {
                path: format!("{dir}/{}", entry.file),
                state: if exists { ModelState::Idle } else { ModelState::Missing },
                entry,
            };
            if bank.models.insert(id.clone(), model).is_some() {
                warn!("Модели: идентификатор «{id}» повторяется, взята последняя запись");
            }
        }

        info!(
            "Модели: записей {}, с файлом {}",
            bank.models.len(),
            bank.models.len() - missing.len().min(bank.models.len())
        );
        if !missing.is_empty() {
            warn!("Модели: нет файлов, будет запасной облик: {}", missing.join(", "));
        }
        bank
    }

    /// Сцена модели, её положение относительно низа объекта и признак «уже загружена».
    /// Первый запрос ставит файл на загрузку. `None` — модели нет, нужен запасной облик.
    fn request(&mut self, asset_server: &AssetServer, id: &str) -> Option<(Handle<WorldAsset>, Transform, bool)> {
        let model = self.models.get_mut(id)?;
        if matches!(model.state, ModelState::Idle) {
            model.state = ModelState::Loading(Handles {
                gltf: asset_server.load(model.path.clone()),
                scene: asset_server.load(GltfAssetLabel::Scene(0).from_asset(model.path.clone())),
            });
        }
        match &model.state {
            ModelState::Loading(handles) => Some((handles.scene.clone(), model.entry.transform(), false)),
            ModelState::Ready(handles) => Some((handles.scene.clone(), model.entry.transform(), true)),
            ModelState::Missing | ModelState::Idle | ModelState::Failed => None,
        }
    }
}

/// Доступ к моделям из системы, которая создаёт облик объектов.
#[derive(SystemParam)]
pub struct Models<'w> {
    bank: ResMut<'w, ModelBank>,
    asset_server: Res<'w, AssetServer>,
}

impl Models<'_> {
    /// Есть ли у `id` файл модели: тогда объект показывается моделью, а не примитивами.
    pub fn has(&self, id: &str) -> bool {
        self.bank
            .models
            .get(id)
            .is_some_and(|model| !matches!(model.state, ModelState::Missing | ModelState::Failed))
    }

    /// Файл glTF модели `id`, если она уже запрошена: из него берутся анимации.
    pub fn gltf(&self, id: &str) -> Option<Handle<Gltf>> {
        match &self.bank.models.get(id)?.state {
            ModelState::Loading(handles) | ModelState::Ready(handles) => Some(handles.gltf.clone()),
            _ => None,
        }
    }

    /// Вешает на объект облик `id`: модель из списка или запасной облик из примитивов.
    ///
    /// `ground` — низ объекта в системе родителя: от него отсчитываются смещение, поворот
    /// и масштаб из списка. `fallback` создаёт запасной облик прямо в родителе. Пока модель
    /// грузится, видны примитивы; когда она загрузится, они убираются. Если модели нет или она
    /// не загрузилась, примитивы остаются.
    pub fn spawn_visual(
        &mut self,
        parent: &mut ChildSpawnerCommands,
        id: &str,
        ground: Transform,
        fallback: impl FnOnce(&mut FallbackSpawner),
    ) {
        let model = self.bank.request(&self.asset_server, id);
        let mut parts = Vec::new();
        if model.as_ref().is_none_or(|(.., ready)| !ready) {
            fallback(&mut FallbackSpawner {
                parent: &mut *parent,
                spawned: &mut parts,
            });
        }
        let Some((scene, transform, ready)) = model else {
            return;
        };
        let mut visual = parent.spawn((WorldAssetRoot(scene), ground * transform));
        if !ready {
            visual.insert(PendingModel {
                id: id.to_owned(),
                fallback: parts,
            });
        }
    }
}

/// Создаёт части запасного облика и запоминает их, чтобы убрать, когда модель загрузится.
pub struct FallbackSpawner<'a, 'w> {
    parent: &'a mut ChildSpawnerCommands<'w>,
    spawned: &'a mut Vec<Entity>,
}

impl FallbackSpawner<'_, '_> {
    /// Создаёт часть запасного облика как дочернюю сущность объекта.
    pub fn spawn(&mut self, bundle: impl Bundle) -> EntityCommands<'_> {
        let part = self.parent.spawn(bundle);
        self.spawned.push(part.id());
        part
    }
}

/// Модель ещё грузится: рядом с ней стоит запасной облик.
#[derive(Component)]
struct PendingModel {
    id: String,
    /// Части запасного облика: убираются, когда модель загрузилась.
    fallback: Vec<Entity>,
}

/// Следит за загрузкой моделей: отмечает готовые и не загрузившиеся, сверяет бюджет треугольников.
fn track_loading(
    asset_server: Res<AssetServer>,
    mut bank: ResMut<ModelBank>,
    gltfs: Res<Assets<Gltf>>,
    gltf_meshes: Res<Assets<GltfMesh>>,
    meshes: Res<Assets<Mesh>>,
) {
    if !bank.models.values().any(|model| matches!(model.state, ModelState::Loading(_))) {
        return;
    }
    for model in bank.models.values_mut() {
        let ModelState::Loading(handles) = &model.state else {
            continue;
        };
        let id = &model.entry.id;
        let failure = match (
            asset_server.load_state(&handles.gltf),
            asset_server.recursive_dependency_load_state(&handles.gltf),
        ) {
            (LoadState::Failed(error), _) | (_, RecursiveDependencyLoadState::Failed(error)) => Some(error),
            _ => None,
        };
        if let Some(error) = failure {
            warn!("Модели: «{id}» не загружена, остаётся запасной облик: {error}");
            model.state = ModelState::Failed;
            continue;
        }
        if !asset_server.is_loaded_with_dependencies(&handles.gltf) {
            continue;
        }
        let triangles = gltfs
            .get(&handles.gltf)
            .map_or(0, |gltf| triangle_count(gltf, &gltf_meshes, &meshes));
        let budget = model.entry.budget;
        if budget > 0 && triangles > budget {
            warn!("Модели: «{id}» тяжелее бюджета: треугольников {triangles}, бюджет {budget}");
        }
        if let ModelState::Loading(handles) = std::mem::replace(&mut model.state, ModelState::Failed) {
            model.state = ModelState::Ready(handles);
        }
    }
}

/// Треугольников во всех мешах файла модели.
fn triangle_count(gltf: &Gltf, gltf_meshes: &Assets<GltfMesh>, meshes: &Assets<Mesh>) -> usize {
    gltf.meshes
        .iter()
        .filter_map(|mesh| gltf_meshes.get(mesh))
        .flat_map(|mesh| mesh.primitives.iter())
        .filter_map(|primitive| meshes.get(&primitive.mesh))
        .map(mesh_triangles)
        .sum()
}

/// Треугольников в меше из списка треугольников. Данные меша, уже отданные видеокарте, не считаются.
fn mesh_triangles(mesh: &Mesh) -> usize {
    match mesh.try_indices_option() {
        Ok(Some(indices)) => indices.len() / 3,
        Ok(None) => mesh
            .try_attribute(Mesh::ATTRIBUTE_POSITION)
            .map_or(0, |positions| positions.len() / 3),
        Err(_) => 0,
    }
}

/// Убирает запасной облик у загруженных моделей и сцену у не загрузившихся.
fn swap_fallbacks(
    mut commands: Commands,
    bank: Res<ModelBank>,
    pending: Query<(Entity, &PendingModel, Has<Children>)>,
) {
    for (entity, model, spawned) in &pending {
        match bank.models.get(&model.id).map(|model| &model.state) {
            // Примитивы уходят, когда сцена модели уже стоит на месте: без кадра с пустотой.
            Some(ModelState::Ready(_)) if spawned => {
                for &part in &model.fallback {
                    commands.entity(part).try_despawn();
                }
                commands.entity(entity).try_remove::<PendingModel>();
            }
            Some(ModelState::Failed) | None => commands.entity(entity).try_despawn(),
            _ => {}
        }
    }
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

/// Подменяет материалы мешей под [`ModelShade`] затемнёнными копиями. Меш во вспышке попадания
/// не трогается: вспышка вернёт ему материал, и затемнение ляжет в следующем кадре.
fn apply_shade(
    mut cache: ResMut<ShadeCache>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    roots: Query<(Entity, &ModelShade)>,
    children: Query<&Children>,
    mut parts: Query<&mut MeshMaterial3d<StandardMaterial>, Without<FlashedMesh>>,
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
            copy.base_color = Color::LinearRgba(LinearRgba::new(c.red * k, c.green * k, c.blue * k, c.alpha));
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
    fn shade_is_counted_from_the_original() {
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
        let part = app.world_mut().spawn((MeshMaterial3d(original.clone()), ChildOf(root))).id();
        let color = |app: &App| {
            let handle = &app.world().get::<MeshMaterial3d<StandardMaterial>>(part).unwrap().0;
            app.world().resource::<Assets<StandardMaterial>>().get(handle).unwrap().base_color.to_linear()
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

    #[test]
    fn list_is_parsed() {
        let json = r#"{ "models": [
            { "id": "rover_body", "file": "rover_body.glb", "scale": 0.8, "rotation_y": 3.14159,
              "offset_y": 0.688, "budget": 15000 }
        ] }"#;
        let entries = parse_list(json).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "rover_body");
        assert_eq!(entries[0].file, "rover_body.glb");
        assert_eq!(entries[0].scale, 0.8);
        assert_eq!(entries[0].budget, 15000);
    }

    #[test]
    fn missing_fields_get_defaults() {
        let entries = parse_list(r#"{ "models": [ { "id": "tree_1" } ] }"#).unwrap();
        assert!(entries[0].file.is_empty());
        assert_eq!(entries[0].scale, 1.0);
        assert_eq!(entries[0].rotation_y, 0.0);
        assert_eq!(entries[0].offset_y, 0.0);
        assert_eq!(entries[0].budget, 0);
    }

    #[test]
    fn broken_list_is_an_error() {
        assert!(parse_list(r#"{ "models": [ { "file": "без id.glb" } ] }"#).is_err());
        assert!(parse_list("не json").is_err());
    }

    #[test]
    fn entry_transform_lifts_turns_and_scales() {
        let entries = parse_list(
            r#"{ "models": [ { "id": "a", "scale": 2.0, "rotation_y": 3.14159265, "offset_y": 0.5 } ] }"#,
        )
        .unwrap();
        let transform = entries[0].transform();
        assert_eq!(transform.translation, Vec3::new(0.0, 0.5, 0.0));
        assert_eq!(transform.scale, Vec3::splat(2.0));
        // Поворот на π разворачивает модель носом с +Z на -Z.
        assert!((transform.rotation * Vec3::Z - Vec3::NEG_Z).length() < 1e-4);
    }

    /// Список из репозитория читается игрой, идентификаторы не повторяются, файлы лежат рядом.
    #[test]
    fn repository_list_is_valid() {
        let entries = parse_list(include_str!("../assets/models/models.json")).unwrap();
        let mut ids: Vec<_> = entries.iter().map(|entry| entry.id.as_str()).collect();
        ids.sort_unstable();
        let total = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), total, "идентификаторы повторяются");

        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join(MODELS_DIR);
        let missing: Vec<_> = entries
            .iter()
            .filter(|entry| entry.file.is_empty() || !dir.join(&entry.file).is_file())
            .map(|entry| entry.id.as_str())
            .collect();
        assert!(missing.is_empty(), "нет файлов моделей у записей {missing:?}");
    }
}
