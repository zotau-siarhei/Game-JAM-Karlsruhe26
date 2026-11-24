//! Прогрев конвейеров рендера: всё, что может появиться в кадре, один раз рисуется до игры.
//!
//! Bevy собирает конвейер (пайплайн) для каждой новой пары «раскладка меша + ключ материала»
//! и для каждого эффекта частиц, когда объект впервые попадает в кадр. На macOS и в браузере
//! компиляция синхронная: кадр стоит, пока шейдер собирается, — это фриз при первом враге,
//! взрыве или пламени. Поэтому при запуске (и после смены качества) каждый класс меша ставится
//! с каждым классом материала, плюс сцены всех моделей, эффекты частиц и материалы пламени.
//! Объекты крошечные и стоят под рельефом на луче взгляда камеры: в кадре и в каскадах теней
//! они есть, а видно их не будет.
//!
//! Конвейер, собранный после прогрева, — поздний: он пишется в лог с меткой и временем,
//! а число поздних конвейеров попадает в сводку `--bench` (`src/perf.rs`).
//!
//! `SPACE_GUARD_NO_PREWARM=1` выключает прогрев: так видно, сколько конвейеров собирается в игре.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use bevy::asset::AssetEventSystems;
use bevy::mesh::PrimitiveTopology;
use bevy::pbr::StandardMaterialKey;
use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy::render::render_resource::{PipelineCache, PipelineDescriptor};
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy_hanabi::prelude::{EffectAsset, EffectMaterial, ParticleEffect};

use crate::camera::{Focus, GameCamera};
use crate::fire::FlameMaterial;
use crate::models::ModelBank;
use crate::quality::Quality;
use crate::terrain::Terrain;

/// Масштаб объектов прогрева: размер не влияет на конвейер.
const SCALE: f32 = 0.01;
/// Насколько за фокус по лучу взгляда стоят меши и модели, м: около 3 м под рельефом.
const MESH_DEPTH: f32 = 4.0;
/// То же для эффектов частиц: частицы разлетаются, им нужно глубже.
const EFFECT_DEPTH: f32 = 40.0;
/// Столько кадров подряд без новых конвейеров — прогрев закончен.
const QUIET_FRAMES: u32 = 10;
/// Предел прогрева, с.
const TIMEOUT: f32 = 30.0;
/// Столько кадров прогрев ждёт перед началом (`prewarm.start_frames` профиля платформы: на телефонах 3).
/// На Android кадр ждёт сборки всех конвейеров (`compile_pipelines`), и сборка занимает один кадр
/// в 15 с: без ожидания всё это время был бы чёрный экран, а не экран загрузки. Дробить прогрев по
/// кадрам нельзя: кадры между сборками теряли устройство GPU Adreno 810 (3 запуска из 6).
fn start_frames() -> u32 {
    crate::profile::get().prewarm.start_frames
}

/// Плагин прогрева.
pub struct PrewarmPlugin;

impl Plugin for PrewarmPlugin {
    fn build(&self, app: &mut App) {
        let watch = PipelineWatch::default();
        let disabled = std::env::var("SPACE_GUARD_NO_PREWARM").is_ok_and(|v| !v.is_empty() && v != "0");
        let mut prewarm = Prewarm::default();
        if disabled {
            info!("Прогрев шейдеров выключен (SPACE_GUARD_NO_PREWARM)");
            prewarm.done = true;
            watch.lock().warm = true;
        }
        app.insert_resource(watch.clone())
            .insert_resource(prewarm)
            .init_resource::<Catalog>()
            .add_systems(Startup, add_templates)
            .add_systems(
                PostUpdate,
                (catalog_meshes, catalog_materials).after(AssetEventSystems),
            )
            .add_systems(
                Update,
                (restart_on_quality, cover_recompile, warm, follow_camera)
                    .chain()
                    // Занавес красится в том же кадре, что и встаёт надпись.
                    .before(crate::fade::CurtainMove)
                    .in_set(crate::frame::GameSet::Presentation),
            )
            // Под занавесом повторного прогрева меню и пауза ввод не принимают: игрок не видит строк.
            .configure_sets(Update, crate::menu::MenuInput.run_if(not(covered)));
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.insert_resource(watch).add_systems(
                Render,
                compile_pipelines
                    .after(RenderSystems::Prepare)
                    .before(RenderSystems::Render),
            );
        }
    }
}

/// Состояние конвейеров из рендер-мира для основного.
#[derive(Default)]
pub struct WatchState {
    /// Всего конвейеров в кэше.
    total: usize,
    /// Ждут компиляции (шейдер ещё не загружен).
    waiting: usize,
    /// Новых конвейеров в последнем кадре рендера.
    fresh: usize,
    /// Прогрев закончен: новые конвейеры — поздние.
    warm: bool,
    /// Поздних конвейеров и время их компиляции, мс.
    late: usize,
    late_ms: f32,
}

/// Общий для основного и рендер-мира ресурс состояния конвейеров.
#[derive(Resource, Clone, Default)]
pub struct PipelineWatch(Arc<Mutex<WatchState>>);

impl PipelineWatch {
    fn lock(&self) -> std::sync::MutexGuard<'_, WatchState> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Поздних конвейеров после прогрева и сколько миллисекунд кадров ушло на их компиляцию.
    pub fn late(&self) -> (usize, f32) {
        let state = self.lock();
        (state.late, state.late_ms)
    }
}

/// Рендер-мир: компилирует очередь конвейеров сам (позже это сделал бы Bevy в
/// `RenderSystems::Render`), меряет время и замечает новые конвейеры.
///
/// Где профиль велит (`prewarm.wait_pipelines`, Android), конвейеры собираются параллельно в пуле
/// асинхронных задач, но кадр ждёт, пока все
/// начатые не соберутся: отрисовка вперемешку со сборкой вешала GPU Adreno 810 (DeviceLost),
/// а по одному в рендер-потоке прогрев шёл 15 с. Одно ожидание — не дольше `prewarm.wait_limit`:
/// пока кадр стоит, ввод не обрабатывается, и Android после 5 с показал бы «не отвечает» (ANR).
fn compile_pipelines(mut cache: ResMut<PipelineCache>, watch: Res<PipelineWatch>) {
    let start = Instant::now();
    cache.process_queue();
    mark_compiling(&cache, "compiling", |p| matches!(p.state, bevy::render::render_resource::CachedPipelineState::Creating(_)));
    let prewarm = &crate::profile::get().prewarm;
    if prewarm.wait_pipelines {
        use bevy::render::render_resource::CachedPipelineState;
        let creating = |cache: &PipelineCache| {
            cache.pipelines().filter(|p| matches!(p.state, CachedPipelineState::Creating(_))).count()
        };
        while creating(&cache) > 0 && start.elapsed().as_secs_f32() < prewarm.wait_limit {
            std::thread::sleep(std::time::Duration::from_millis(1));
            cache.process_queue();
        }
        let waited = start.elapsed().as_secs_f32();
        if waited > 0.25 {
            let left = creating(&cache);
            info!("Конвейеры: кадр ждал сборки {:.0} мс, собирается ещё {left}", waited * 1000.0);
        }
    }
    let ms = start.elapsed().as_secs_f32() * 1000.0;
    let total = cache.pipelines().count();
    let mut state = watch.lock();
    let fresh = total.saturating_sub(state.total);
    if fresh > 0 && state.warm {
        let all: Vec<&PipelineDescriptor> = cache.pipelines().map(|p| &p.descriptor).collect();
        let (old, new) = all.split_at(state.total.min(all.len()));
        let labels: Vec<String> = new.iter().map(|d| format!("{} [{}]", label(d), difference(d, old))).collect();
        warn!("Поздний конвейер: {} за {ms:.1} мс", labels.join(", "));
        state.late += fresh;
        state.late_ms += ms;
    }
    state.fresh = fresh;
    state.total = total;
    state.waiting = cache.waiting_pipelines().count();
}

/// Отметка следа (`trail.rs`): сколько конвейеров в очереди или сборке и метки первых из них. Если
/// драйвер упадёт на сборке, при следующем запуске видно, на каких.
fn mark_compiling(
    cache: &PipelineCache,
    what: &str,
    filter: impl Fn(&&bevy::render::render_resource::CachedPipeline) -> bool,
) {
    let pending: Vec<_> = cache.pipelines().filter(filter).collect();
    crate::trail::mark_pipelines((!pending.is_empty()).then(|| {
        let labels: Vec<String> = pending.iter().take(3).map(|p| label(&p.descriptor)).collect();
        format!("pipelines {what}: {} ({})", pending.len(), labels.join(", "))
    }));
}

fn label(descriptor: &PipelineDescriptor) -> String {
    let label = match descriptor {
        PipelineDescriptor::RenderPipelineDescriptor(d) => d.label.as_deref(),
        PipelineDescriptor::ComputePipelineDescriptor(d) => d.label.as_deref(),
        PipelineDescriptor::MeshPipelineDescriptor(d) => d.label.as_deref(),
    };
    label.unwrap_or("без метки").to_owned()
}

/// Чем конвейер отличается от ближайшего прогретого с той же меткой: шейдер-дефы и вершины.
/// По этой строке видно, какого сочетания прогрев не покрыл.
fn difference(descriptor: &PipelineDescriptor, warmed: &[&PipelineDescriptor]) -> String {
    let features = |d: &PipelineDescriptor| -> HashSet<String> {
        let mut set = HashSet::new();
        if let PipelineDescriptor::RenderPipelineDescriptor(d) = d {
            set.extend(d.vertex.shader_defs.iter().map(|def| format!("{def:?}")));
            if let Some(fragment) = &d.fragment {
                set.extend(fragment.shader_defs.iter().map(|def| format!("{def:?}")));
                set.extend(fragment.targets.iter().map(|t| format!("target {:?}", t.as_ref().map(|t| (t.format, t.blend.is_some())))));
            }
            for buffer in &d.vertex.buffers {
                set.extend(buffer.attributes.iter().map(|a| format!("attribute {} {:?}", a.shader_location, a.format)));
            }
            set.insert(format!("primitive {:?} cull {:?}", d.primitive.topology, d.primitive.cull_mode));
            set.insert(format!("depth {:?}", d.depth_stencil.as_ref().map(|s| (s.format, s.depth_write_enabled, s.depth_compare))));
            set.insert(format!("msaa {}", d.multisample.count));
        }
        set
    };
    let mine = features(descriptor);
    let name = label(descriptor);
    let closest = warmed
        .iter()
        .filter(|d| label(d) == name)
        .map(|d| features(d))
        .min_by_key(|other| mine.symmetric_difference(other).count());
    let Some(other) = closest else {
        return "нет прогретого с этой меткой".to_owned();
    };
    let mut added: Vec<&String> = mine.difference(&other).collect();
    let mut missing: Vec<&String> = other.difference(&mine).collect();
    added.sort();
    missing.sort();
    let mut attributes: Vec<&String> = mine.iter().filter(|f| f.starts_with("attribute")).collect();
    attributes.sort();
    format!("есть {added:?}, нет {missing:?}; вершины {attributes:?}")
}

/// Класс меша для конвейера: топология, атрибуты вершин и морф-цели.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct LayoutClass {
    topology: PrimitiveTopology,
    attributes: Vec<&'static str>,
    morph: bool,
}

impl LayoutClass {
    /// Класс меша. `None` — данные меша уже отданы видеокарте.
    fn of(mesh: &Mesh) -> Option<Self> {
        let mut attributes: Vec<&'static str> = mesh.try_attributes().ok()?.map(|(a, _)| a.name).collect();
        attributes.sort_unstable();
        Some(Self {
            topology: mesh.primitive_topology(),
            attributes,
            morph: mesh.has_morph_targets(),
        })
    }

    fn has_uv(&self) -> bool {
        self.attributes.contains(&Mesh::ATTRIBUTE_UV_0.name)
    }

    /// Меш со скелетом или морф-целями рисуется только со своими компонентами (`SkinnedMesh`,
    /// `MorphWeights`), иначе группа привязок не совпадёт с конвейером. Такие меши есть только
    /// в моделях glTF, а модели прогреваются своими сценами.
    fn standalone(&self) -> bool {
        !self.morph
            && !self.attributes.contains(&Mesh::ATTRIBUTE_JOINT_INDEX.name)
            && !self.attributes.contains(&Mesh::ATTRIBUTE_JOINT_WEIGHT.name)
    }

    /// Можно ли рисовать меш этого класса материалом этого класса. Точки и линии несовместимы
    /// со сдвигом глубины: такой конвейер wgpu отвергает, и Bevy закрывает игру.
    fn fits(&self, material: &MaterialClass) -> bool {
        if !self.standalone() {
            return false;
        }
        let triangles = matches!(self.topology, PrimitiveTopology::TriangleList | PrimitiveTopology::TriangleStrip);
        triangles || material.key & StandardMaterialKey::DEPTH_BIAS.bits() == 0
    }
}

/// Класс материала для конвейера: ключ `StandardMaterial` и режим прозрачности.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct MaterialClass {
    key: u64,
    alpha: &'static str,
}

impl MaterialClass {
    fn of(material: &StandardMaterial) -> Self {
        let alpha = match material.alpha_mode {
            AlphaMode::Opaque => "opaque",
            AlphaMode::Mask(_) => "mask",
            AlphaMode::Blend => "blend",
            AlphaMode::Premultiplied => "premultiplied",
            AlphaMode::AlphaToCoverage => "coverage",
            AlphaMode::Add => "add",
            AlphaMode::Multiply => "multiply",
        };
        Self {
            key: StandardMaterialKey::from(material).bits(),
            alpha,
        }
    }
}

/// Представители классов: по одному мешу и материалу на класс. Меш с данными только на
/// видеокарте классифицируется в кадре создания, пока данные ещё в основном мире.
#[derive(Resource, Default)]
struct Catalog {
    meshes: HashMap<LayoutClass, Handle<Mesh>>,
    materials: HashMap<MaterialClass, Handle<StandardMaterial>>,
}

fn catalog_meshes(
    mut events: MessageReader<AssetEvent<Mesh>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut catalog: ResMut<Catalog>,
) {
    for event in events.read() {
        let (AssetEvent::Added { id } | AssetEvent::Modified { id }) = *event else {
            continue;
        };
        let Some(class) = meshes.get(id).and_then(LayoutClass::of) else {
            continue;
        };
        if !catalog.meshes.contains_key(&class) {
            if let Some(handle) = meshes.get_strong_handle(id) {
                catalog.meshes.insert(class, handle);
            }
        }
    }
}

fn catalog_materials(
    mut events: MessageReader<AssetEvent<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut catalog: ResMut<Catalog>,
    prewarm: Res<Prewarm>,
) {
    for event in events.read() {
        let (AssetEvent::Added { id } | AssetEvent::Modified { id }) = *event else {
            continue;
        };
        let Some(class) = materials.get(id).map(MaterialClass::of) else {
            continue;
        };
        if !catalog.materials.contains_key(&class) {
            if prewarm.done {
                // Его конвейеры соберутся в игре: такой материал стоит создавать при старте.
                warn!("Материал нового класса после прогрева: {class:?}, цвет {:?}", materials.get(id).map(|m| m.base_color));
            }
            if let Some(handle) = materials.get_strong_handle(id) {
                catalog.materials.insert(class, handle);
            }
        }
    }
}

/// Материалы-образцы: классы, которые код создаёт посреди боя (вспышки, лучи, гаснущие
/// осколки), а при старте их может не быть. Без текстур, с отсечением граней и без, во всех
/// режимах прозрачности. Хранятся в каталоге и прогреваются вместе с остальными.
fn add_templates(mut materials: ResMut<Assets<StandardMaterial>>, mut templates: Local<Vec<Handle<StandardMaterial>>>) {
    for alpha_mode in [AlphaMode::Opaque, AlphaMode::Mask(0.5), AlphaMode::Blend, AlphaMode::Add] {
        for cull_mode in [Some(bevy::render::render_resource::Face::Back), None] {
            for unlit in [false, true] {
                templates.push(materials.add(StandardMaterial {
                    alpha_mode,
                    cull_mode,
                    double_sided: cull_mode.is_none(),
                    unlit,
                    ..default()
                }));
            }
        }
    }
}

/// Что уже стоит в прогреве.
#[derive(PartialEq, Eq, Hash)]
enum Warmed {
    Pair(LayoutClass, MaterialClass),
    Flame(LayoutClass),
    Effect(AssetId<EffectAsset>),
    Scene(AssetId<WorldAsset>),
}

/// Ход прогрева. Загрузка (`src/loading.rs`) ждёт его конца.
#[derive(Resource, Default)]
pub struct Prewarm {
    done: bool,
    warmed: HashSet<Warmed>,
    /// Корни объектов прогрева: меши и модели, эффекты частиц.
    roots: Option<(Entity, Entity)>,
    quiet: u32,
    started: Option<f32>,
    /// Конвейеров в кэше на начало прогрева.
    pipelines_before: usize,
    /// Кадров до начала повторного прогрева: сначала на экран выходит занавес с надписью.
    hold: u8,
    /// Ступень качества сейчас и ступени, уже прогретые за этот запуск: их конвейеры в кэше,
    /// и повторный прогрев проходит без занавеса (`quiet_restart`).
    quality: Option<Quality>,
    warmed_qualities: Vec<Quality>,
    quiet_restart: bool,
}

impl Prewarm {
    /// Прогрев закончен.
    pub fn done(&self) -> bool {
        self.done
    }
}

/// Повторный прогрев идёт под занавесом (`cover_recompile`): ввод экранов в это время не нужен.
pub fn covered(prewarm: Res<Prewarm>) -> bool {
    crate::profile::get().prewarm.wait_pipelines && !prewarm.done
}

/// Смена качества: другой ключ вида — прогрев заново. Объекты недоделанного прогрева убираются,
/// иначе новый поставил бы их ещё раз.
fn restart_on_quality(
    mut commands: Commands,
    quality: Res<Quality>,
    mut prewarm: ResMut<Prewarm>,
    watch: Res<PipelineWatch>,
) {
    prewarm.quality = Some(*quality);
    // Прогрев ещё не начат: начнётся уже с новым качеством.
    if !quality.is_changed() || quality.is_added() || prewarm.started.is_none() {
        return;
    }
    info!("Прогрев шейдеров заново: качество {:?}", *quality);
    // Ступень уже была прогрета: собирать нечего, занавес только мелькнул бы.
    prewarm.quiet_restart = prewarm.warmed_qualities.contains(&*quality);
    if let Some((meshes, effects)) = prewarm.roots.take() {
        commands.entity(meshes).despawn();
        commands.entity(effects).despawn();
    }
    // Кадр ждёт сборки только там, где `wait_pipelines`: туда же и занавес; два кадра — чтобы кадр
    // с занавесом успел выйти на экран раньше, чем ожидание сборки остановит следующий.
    prewarm.hold = if crate::profile::get().prewarm.wait_pipelines && !prewarm.quiet_restart { 2 } else { 0 };
    prewarm.done = false;
    prewarm.warmed.clear();
    prewarm.quiet = 0;
    prewarm.started = None;
    watch.lock().warm = false;
}

/// Непрозрачность занавеса на время повторного прогрева.
const RECOMPILE_DIM: f32 = 0.7;

/// Надпись поверх занавеса на время повторного прогрева.
#[derive(Component)]
struct RecompileNotice;

/// Где кадр ждёт сборки конвейеров (`prewarm.wait_pipelines`, Android и iOS), повторный прогрев после
/// смены качества идёт под полупрозрачным занавесом с надписью: иначе меню или пауза замирали бы на время
/// сборки, и игрок не понял бы почему. Меню за занавесом видно. Ступень, уже прогретая за этот запуск,
/// занавеса не получает: её конвейеры в кэше. Первый прогрев закрыт экраном загрузки.
fn cover_recompile(
    mut commands: Commands,
    prewarm: Res<Prewarm>,
    state: Res<State<crate::run::GameState>>,
    mut curtain: ResMut<crate::fade::Curtain>,
    notice: Query<Entity, With<RecompileNotice>>,
) {
    if !crate::profile::get().prewarm.wait_pipelines || *state.get() == crate::run::GameState::Loading {
        return;
    }
    let covered = !notice.is_empty();
    if !prewarm.done && !covered && !prewarm.quiet_restart {
        // Сразу, а не плавно: кадры прогрева долгие, и меню успело бы замереть на виду.
        curtain.dim(RECOMPILE_DIM);
        commands.spawn((
            RecompileNotice,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            GlobalZIndex(crate::fade::ABOVE_CURTAIN_Z),
            children![(
                Text::new("COMPILING SHADERS"),
                TextFont { font_size: FontSize::Px(28.0), ..default() },
                TextColor(Color::srgb(0.8, 0.8, 0.8)),
            )],
        ));
    } else if prewarm.done && covered {
        curtain.open(0.3);
        for entity in &notice {
            commands.entity(entity).despawn();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn warm(
    mut commands: Commands,
    mut prewarm: ResMut<Prewarm>,
    catalog: Res<Catalog>,
    mut bank: ResMut<ModelBank>,
    asset_server: Res<AssetServer>,
    mut effects: ResMut<Assets<EffectAsset>>,
    mut flames: ResMut<Assets<FlameMaterial>>,
    watch: Res<PipelineWatch>,
    time: Res<Time<Real>>,
    mut waited: Local<u32>,
) {
    if prewarm.done {
        return;
    }
    if prewarm.hold > 0 {
        prewarm.hold -= 1;
        return;
    }
    if *waited < start_frames() {
        *waited += 1;
        return;
    }
    let now = time.elapsed_secs();
    let prewarm = &mut *prewarm;
    if prewarm.started.is_none() {
        prewarm.started = Some(now);
        prewarm.pipelines_before = watch.lock().total;
        bank.request_all(&asset_server);
    }
    let (mesh_root, effect_root) = *prewarm.roots.get_or_insert_with(|| {
        let root = |commands: &mut Commands, name: &str| {
            commands
                .spawn((Name::new(name.to_owned()), Transform::default(), Visibility::default()))
                .id()
        };
        (root(&mut commands, "Прогрев: меши"), root(&mut commands, "Прогрев: частицы"))
    });

    let small = Transform::from_scale(Vec3::splat(SCALE));
    let mut added = 0;
    let mut add = |prewarm: &mut Prewarm, what: Warmed| -> bool {
        let new = prewarm.warmed.insert(what);
        added += usize::from(new);
        new
    };

    let flame_id = flames.ids().next();
    let flame = flame_id.and_then(|id| flames.get_strong_handle(id));
    for (layout, mesh) in &catalog.meshes {
        for (class, material) in &catalog.materials {
            if layout.fits(class) && add(prewarm, Warmed::Pair(layout.clone(), *class)) {
                commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), small, ChildOf(mesh_root)));
            }
        }
        if layout.has_uv() && layout.standalone() {
            if let Some(flame) = &flame {
                if add(prewarm, Warmed::Flame(layout.clone())) {
                    commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(flame.clone()), small, ChildOf(mesh_root)));
                }
            }
        }
    }
    let effect_ids: Vec<AssetId<EffectAsset>> = effects.ids().collect();
    for id in effect_ids {
        if !add(prewarm, Warmed::Effect(id)) {
            continue;
        }
        let Some(handle) = effects.get_strong_handle(id) else {
            continue;
        };
        // Конвейер зависит от раскладки слотов текстур, а не от картинок: годится любая.
        let slots = effects.get(id).map_or(0, |effect| effect.texture_layout().layout.len());
        let mut entity = commands.spawn((ParticleEffect::new(handle), Transform::default(), ChildOf(effect_root)));
        if slots > 0 {
            entity.insert(EffectMaterial {
                images: vec![Handle::<Image>::default(); slots],
            });
        }
    }
    for scene in bank.ready_scenes() {
        if add(prewarm, Warmed::Scene(scene.id())) {
            commands.spawn((WorldAssetRoot(scene.clone()), small, ChildOf(mesh_root)));
        }
    }

    let state = watch.lock();
    let busy = added > 0 || state.fresh > 0 || state.waiting > 0 || bank.loading() > 0;
    prewarm.quiet = if busy { 0 } else { prewarm.quiet + 1 };
    let started = prewarm.started.unwrap_or(now);
    let timed_out = now - started > TIMEOUT;
    if prewarm.quiet < QUIET_FRAMES && !timed_out {
        return;
    }
    if timed_out {
        warn!(
            "Прогрев шейдеров не закончился за {TIMEOUT} с: ждут {} конвейеров, грузится моделей {}",
            state.waiting,
            bank.loading()
        );
    }
    info!(
        "Прогрев шейдеров: {} конвейеров за {:.1} с ({} классов мешей, {} классов материалов, {} объектов)",
        state.total.saturating_sub(prewarm.pipelines_before),
        now - started,
        catalog.meshes.len(),
        catalog.materials.len(),
        prewarm.warmed.len()
    );
    drop(state);
    watch.lock().warm = true;
    prewarm.done = true;
    prewarm.quiet_restart = false;
    if let Some(quality) = prewarm.quality
        && !prewarm.warmed_qualities.contains(&quality)
    {
        prewarm.warmed_qualities.push(quality);
    }
    if let Some((meshes, effects)) = prewarm.roots.take() {
        commands.entity(meshes).despawn();
        commands.entity(effects).despawn();
    }
}

/// Объекты прогрева стоят на луче взгляда камеры за фокусом, под рельефом.
fn follow_camera(
    prewarm: Res<Prewarm>,
    focus: Res<Focus>,
    terrain: Option<Res<Terrain>>,
    camera: Query<&Transform, (With<GameCamera>, Without<ChildOf>)>,
    mut roots: Query<&mut Transform, Without<GameCamera>>,
) {
    let (Some((meshes, effects)), Ok(eye)) = (prewarm.roots, camera.single()) else {
        return;
    };
    let ground = terrain.map_or(focus.0.y, |t| t.height_at(focus.0.x, focus.0.z));
    let target = Vec3::new(focus.0.x, ground, focus.0.z);
    let forward = (target - eye.translation).normalize_or(Vec3::NEG_Y);
    for (root, depth) in [(meshes, MESH_DEPTH), (effects, EFFECT_DEPTH)] {
        if let Ok(mut transform) = roots.get_mut(root) {
            transform.translation = target + forward * depth;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;

    #[test]
    fn меши_одной_раскладки_в_одном_классе() {
        let cube = Mesh::from(Cuboid::new(1.0, 2.0, 3.0));
        let sphere = Mesh::from(Sphere::new(1.0));
        assert_eq!(LayoutClass::of(&cube), LayoutClass::of(&sphere));
        let bare = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 3]);
        assert_ne!(LayoutClass::of(&cube), LayoutClass::of(&bare));
        assert!(LayoutClass::of(&cube).unwrap().has_uv());
        assert!(!LayoutClass::of(&bare).unwrap().has_uv());
    }

    #[test]
    fn класс_материала_по_ключу_и_прозрачности() {
        let red = StandardMaterial::from(Color::srgb(1.0, 0.0, 0.0));
        let blue = StandardMaterial::from(Color::srgb(0.0, 0.0, 1.0));
        assert_eq!(MaterialClass::of(&red), MaterialClass::of(&blue));
        let glass = StandardMaterial {
            alpha_mode: AlphaMode::Blend,
            ..red.clone()
        };
        assert_ne!(MaterialClass::of(&red), MaterialClass::of(&glass));
        let masked = |cutoff| StandardMaterial {
            alpha_mode: AlphaMode::Mask(cutoff),
            ..default()
        };
        assert_eq!(MaterialClass::of(&masked(0.3)), MaterialClass::of(&masked(0.7)));
    }

    #[test]
    fn точки_не_рисуются_материалом_со_сдвигом_глубины() {
        let points = LayoutClass {
            topology: PrimitiveTopology::PointList,
            attributes: vec![Mesh::ATTRIBUTE_POSITION.name],
            morph: false,
        };
        let plain = MaterialClass::of(&StandardMaterial::default());
        let biased = MaterialClass::of(&StandardMaterial {
            depth_bias: 5.0,
            ..default()
        });
        assert!(points.fits(&plain));
        assert!(!points.fits(&biased));
        let triangles = LayoutClass::of(&Mesh::from(Cuboid::default())).unwrap();
        assert!(triangles.fits(&biased));
        let skinned = LayoutClass {
            attributes: vec![Mesh::ATTRIBUTE_POSITION.name, Mesh::ATTRIBUTE_JOINT_INDEX.name],
            ..triangles
        };
        assert!(!skinned.fits(&plain));
    }
}
