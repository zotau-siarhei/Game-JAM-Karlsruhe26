//! Обломки: летят по баллистике, кувыркаются, отскакивают от рельефа и засыпают.
//!
//! Обломок создаётся сообщением [`SpawnDebris`]. Уснувший обломок записывается в хранилище
//! [`DebrisStore`] по участку рельефа и восстанавливается, когда участок снова загружен,
//! поэтому обломки лежат на месте после отъезда и возвращения. Коллайдеров у обломков нет:
//! машины проезжают сквозь них. Уснувшие обломки оседают по любому [`TerrainChanged`].

use std::collections::HashMap;

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy_hanabi::prelude::*;

use crate::debris_forms::{self, FormKind};
use crate::camera::Focus;
use crate::fire::Fire;
use crate::explosion::Explosion;
use crate::frame::{GameSet, RunMessages};
use crate::run::{RunScoped, WorldRebuilt};
use crate::subsidence::TerrainChanged;
use crate::terrain::{self, Terrain, VisibleChunks, chunk_coord_at};

/// Гравитация, м/с².
const GRAVITY: f32 = 20.0;

/// Упругость отскока.
const BOUNCINESS: f32 = 0.3;

/// Сколько раз обломок отскакивает, прежде чем лечь.
const MAX_BOUNCES: u32 = 2;

/// Доля горизонтальной скорости, которая остаётся после удара о землю.
const GROUND_FRICTION: f32 = 0.45;

/// Медленнее этого обломок на земле засыпает, м/с.
const SLEEP_SPEED: f32 = 0.8;

/// Обломки меньше этого не отбрасывают тени, м.
const SHADOW_SIZE: f32 = 0.8;

/// Сколько горящих обломков может быть одновременно.
const MAX_BURNING: usize = 12;

/// Обломок крупнее этого оставляет в полёте шлейф пыли, м.
const TRAIL_SIZE: f32 = 1.4;
/// Сколько шлейфов может быть одновременно: каждый — свой эффект частиц.
const MAX_TRAILS: usize = 6;
/// Обломок крупнее этого поднимает облачко пыли при первом касании земли, м.
const PUFF_SIZE: f32 = 0.9;
/// Сколько облачков может быть одновременно.
const MAX_PUFFS: usize = 10;
/// Сколько живёт облачко, с.
const PUFF_SECONDS: f32 = 1.4;

/// Сколько уснувших обломков лежит в забеге. Сверх этого убираются самые мелкие в самом дальнем
/// от фокуса участке.
const MAX_RESTING: usize = 1500;
/// На телефоне лежит меньше обломков и тень бросают только крупные: каждый обломок — ещё одна
/// сетка в проходах кадра и теней, а к концу долгого забега их набираются тысячи.
const MOBILE_MAX_RESTING: usize = 600;
const MOBILE_SHADOW_SIZE: f32 = 1.2;

/// Обломок меньше этого тени не отбрасывает, м.
fn shadow_size() -> f32 {
    static SIZE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *SIZE.get_or_init(|| if crate::platform::is_mobile() { MOBILE_SHADOW_SIZE } else { SHADOW_SIZE })
}

/// Сколько уснувших обломков может лежать в забеге.
fn max_resting() -> usize {
    static COUNT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *COUNT.get_or_init(|| if crate::platform::is_mobile() { MOBILE_MAX_RESTING } else { MAX_RESTING })
}

/// Сколько обломков может лететь одновременно. Сверх этого мелкие (меньше [`SHADOW_SIZE`])
/// не появляются: в авиаудар по посёлку мелочь не видна за крупными кусками.
const MAX_FLYING: usize = 400;

/// Подключает обломки.
pub struct DebrisPlugin;

impl Plugin for DebrisPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebrisStore>()
            .add_run_message::<SpawnDebris>()
            .add_systems(Startup, create_debris_assets)
            .add_systems(
                Update,
                (
                    reset_debris.in_set(GameSet::Reset),
                    stream_debris.after(terrain::stream_chunks).in_set(GameSet::Streaming),
                    // Обломки ни на что не влияют: они долетают и догорают и вне забега.
                    (wake_debris, spawn_debris, fly_debris, burn_out, fade_puffs).chain().in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Просьба создать летящий обломок.
#[derive(Message, Debug, Clone, Default)]
pub struct SpawnDebris {
    pub position: Vec3,
    pub velocity: Vec3,
    /// Размеры куска, м; у куска модели — масштаб его сетки.
    pub size: Vec3,
    pub color: Color,
    /// Обломок горит несколько секунд.
    pub burning: bool,
    /// Форма: куб или кусок модели.
    pub shape: DebrisShape,
    /// Начальный поворот; без него — случайный.
    pub rotation: Option<Quat>,
}

/// Форма обломка.
#[derive(Debug, Clone, Default)]
pub enum DebrisShape {
    /// Куб цвета `color`.
    #[default]
    Cube,
    /// Форма из библиотеки (`src/debris_forms.rs`) цвета `color`, вписанная в куб 1×1×1, как
    /// куб: `size` растягивает её так же. Вариант выбирается при появлении, в хранилище обломок
    /// лежит уже готовой сеткой с материалом.
    Form(FormKind),
    /// Кусок модели (`src/fragments.rs`) со своим материалом.
    Mesh {
        mesh: Handle<Mesh>,
        material: Handle<StandardMaterial>,
        /// Половина габаритов сетки до масштаба.
        half: Vec3,
    },
}

impl DebrisShape {
    /// Половина габаритов сетки до масштаба.
    fn half(&self) -> Vec3 {
        match self {
            DebrisShape::Cube | DebrisShape::Form(_) => Vec3::splat(0.5),
            DebrisShape::Mesh { half, .. } => *half,
        }
    }
}

/// Наибольший размер обломка, м.
fn extent(shape: &DebrisShape, scale: Vec3) -> f32 {
    (shape.half() * 2.0 * scale).max_element()
}

/// Насколько обломок с поворотом уходит вниз от центра, м.
fn half_height(shape: &DebrisShape, transform: &Transform) -> f32 {
    let half = shape.half() * transform.scale;
    let m = Mat3::from_quat(transform.rotation);
    m.x_axis.y.abs() * half.x + m.y_axis.y.abs() * half.y + m.z_axis.y.abs() * half.z
}

/// Уснувший обломок в хранилище.
#[derive(Debug, Clone)]
struct DebrisPiece {
    transform: Transform,
    color: Color,
    shape: DebrisShape,
}

/// Уснувшие обломки забега по участкам рельефа.
#[derive(Resource, Default)]
struct DebrisStore {
    chunks: HashMap<IVec2, Vec<DebrisPiece>>,
    /// Сколько обломков во всех участках.
    count: usize,
    /// Участки, обломки которых сейчас в сцене.
    shown: HashMap<IVec2, Vec<Entity>>,
}

/// Общий меш, формы библиотеки и материалы по цвету.
#[derive(Resource)]
struct DebrisAssets {
    cube: Handle<Mesh>,
    /// Сетки форм по [`FormKind::index`] и варианту.
    forms: Vec<Vec<Handle<Mesh>>>,
    /// Материалы по цвету и гладкости.
    materials: HashMap<([u8; 3], bool), Handle<StandardMaterial>>,
    /// Пыль: шлейф за крупным обломком и облачко при падении.
    trail: Handle<EffectAsset>,
    puff: Handle<EffectAsset>,
}

impl DebrisAssets {
    /// Сетка и материал обломка.
    fn look(
        &mut self,
        shape: &DebrisShape,
        color: Color,
        materials: &mut Assets<StandardMaterial>,
    ) -> (Mesh3d, MeshMaterial3d<StandardMaterial>) {
        match shape {
            DebrisShape::Cube => (Mesh3d(self.cube.clone()), MeshMaterial3d(self.material(color, false, materials))),
            DebrisShape::Form(kind) => (
                Mesh3d(self.forms[kind.index()][0].clone()),
                MeshMaterial3d(self.material(color, kind.glossy(), materials)),
            ),
            DebrisShape::Mesh { mesh, material, .. } => (Mesh3d(mesh.clone()), MeshMaterial3d(material.clone())),
        }
    }

    fn material(&mut self, color: Color, glossy: bool, materials: &mut Assets<StandardMaterial>) -> Handle<StandardMaterial> {
        let srgb = color.to_srgba();
        let key = ([srgb.red, srgb.green, srgb.blue].map(|c| (c * 255.0) as u8), glossy);
        self.materials
            .entry(key)
            .or_insert_with(|| {
                materials.add(StandardMaterial {
                    base_color: color,
                    // Кварц и металл блестят, камень и земля матовые.
                    perceptual_roughness: if glossy { 0.3 } else { 0.9 },
                    metallic: if glossy { 0.1 } else { 0.0 },
                    ..default()
                })
            })
            .clone()
    }

    /// Форма библиотеки становится готовой сеткой с материалом: так обломок и хранится.
    fn resolve(
        &mut self,
        shape: &DebrisShape,
        color: Color,
        variant: usize,
        materials: &mut Assets<StandardMaterial>,
    ) -> DebrisShape {
        match shape {
            DebrisShape::Form(kind) => DebrisShape::Mesh {
                mesh: self.forms[kind.index()][variant % debris_forms::VARIANTS].clone(),
                material: self.material(color, kind.glossy(), materials),
                half: Vec3::splat(0.5),
            },
            other => other.clone(),
        }
    }
}

/// Летящий обломок.
#[derive(Component)]
struct Flying {
    velocity: Vec3,
    spin: Vec3,
    bounces: u32,
    color: Color,
    shape: DebrisShape,
    /// Шлейф пыли: дочерний эффект частиц.
    trail: Option<Entity>,
}

/// Шлейф пыли за обломком.
#[derive(Component)]
struct DustTrail;

/// Облачко пыли от упавшего обломка: сколько ещё, с.
#[derive(Component)]
struct DustPuff {
    left: f32,
}

/// Обломок горит: сколько ещё, с. Пламя ставит модуль огня по [`Fire`].
#[derive(Component)]
struct Burning {
    remaining: f32,
}

fn create_debris_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut effects: ResMut<Assets<EffectAsset>>,
) {
    // Формы строятся при запуске: их застаёт прогрев шейдеров.
    let forms = FormKind::ALL
        .iter()
        .map(|&kind| (0..debris_forms::VARIANTS).map(|v| meshes.add(debris_forms::build(kind, v))).collect())
        .collect();
    commands.insert_resource(DebrisAssets {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        forms,
        materials: HashMap::new(),
        trail: effects.add(dust_effect(true)),
        puff: effects.add(dust_effect(false)),
    });
}

/// Пыль обломка: `trail` — шлейф, частицы идут всё время полёта; иначе облачко у земли разом.
fn dust_effect(trail: bool) -> EffectAsset {
    let writer = ExprWriter::new();
    let init_pos = SetPositionSphereModifier {
        center: writer.lit(Vec3::ZERO).expr(),
        radius: writer.lit(if trail { 0.3 } else { 0.6 }).expr(),
        dimension: ShapeDimension::Volume,
    };
    let init_vel = if trail {
        SetVelocitySphereModifier {
            center: writer.lit(Vec3::ZERO).expr(),
            speed: writer.lit(0.2).uniform(writer.lit(0.8)).expr(),
        }
    } else {
        // Облачко расходится в стороны у самой земли.
        SetVelocitySphereModifier {
            center: writer.lit(Vec3::Y * 0.6).expr(),
            speed: writer.lit(2.0).uniform(writer.lit(4.0)).expr(),
        }
    };
    let (life_low, life_high) = if trail { (0.5, 0.9) } else { (0.6, 1.2) };
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, writer.lit(life_low).uniform(writer.lit(life_high)).expr());
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let update_rise = AccelModifier::new(writer.lit(Vec3::Y * 0.8).expr());
    let update_drag = LinearDragModifier::new(writer.lit(3.0).expr());
    let round = RoundModifier {
        roundness: writer.lit(1.0).expr(),
    };
    let mut color = bevy_hanabi::Gradient::new();
    color.add_key(0.0, Vec4::new(0.66, 0.6, 0.5, 0.0));
    color.add_key(0.15, Vec4::new(0.66, 0.6, 0.5, if trail { 0.35 } else { 0.55 }));
    color.add_key(1.0, Vec4::new(0.58, 0.54, 0.46, 0.0));
    let mut size = bevy_hanabi::Gradient::new();
    size.add_key(0.0, Vec3::splat(if trail { 0.3 } else { 0.5 }));
    size.add_key(1.0, Vec3::splat(if trail { 1.1 } else { 2.0 }));
    let spawner = if trail { SpawnerSettings::rate(18.0.into()) } else { SpawnerSettings::once(14.0.into()) };
    EffectAsset::new(64, spawner, writer.finish())
        .with_name(if trail { "debris_trail" } else { "debris_puff" })
        .init(init_pos)
        .init(init_vel)
        .init(init_lifetime)
        .init(init_age)
        .update(update_rise)
        .update(update_drag)
        .render(OrientModifier::new(OrientMode::FaceCameraPosition))
        .render(round)
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Overwrite,
            mask: ColorBlendMask::RGBA,
        })
        .render(SizeOverLifetimeModifier {
            gradient: size,
            screen_space_size: false,
        })
}

fn reset_debris(mut rebuilt: MessageReader<WorldRebuilt>, mut store: ResMut<DebrisStore>) {
    if rebuilt.read().last().is_some() {
        // Сущности прошлого забега удалены через RunScoped.
        *store = DebrisStore::default();
    }
}

/// Создаёт летящие обломки по сообщениям.
fn spawn_debris(
    mut commands: Commands,
    mut requests: MessageReader<SpawnDebris>,
    mut assets: ResMut<DebrisAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    burning: Query<(), With<Burning>>,
    flying: Query<(), With<Flying>>,
    trails: Query<(), With<DustTrail>>,
) {
    let mut burning_count = burning.iter().count();
    let mut flying_count = flying.iter().count();
    let mut trail_count = trails.iter().count();
    for (i, request) in requests.read().enumerate() {
        if flying_count >= MAX_FLYING && extent(&request.shape, request.size) < shadow_size() {
            continue;
        }
        flying_count += 1;
        // Вариант формы — от места появления: соседние обломки разные.
        let p = request.position;
        let variant = (p.x * 7.3 + p.z * 3.1 + p.y * 1.7 + i as f32).abs() as usize;
        let shape = assets.resolve(&request.shape, request.color, variant, &mut materials);
        let (mesh, material) = assets.look(&shape, request.color, &mut materials);
        let k = i as f32;
        // Крупный кусок кувыркается медленнее.
        let size = extent(&shape, request.size);
        let spin_rate = 8.0 / size.max(1.0).sqrt();
        // Шлейф пыли за крупным быстрым куском; убирается, когда кусок ляжет.
        let trail = (size >= TRAIL_SIZE && trail_count < MAX_TRAILS && request.velocity.length() > 4.0).then(|| {
            trail_count += 1;
            commands.spawn((DustTrail, ParticleEffect::new(assets.trail.clone()), Transform::default())).id()
        });
        let mut entity = commands.spawn((
            Flying {
                velocity: request.velocity,
                spin: Vec3::new((k * 1.3).sin(), (k * 2.1).cos(), (k * 0.7).sin()) * spin_rate,
                bounces: 0,
                color: request.color,
                shape,
                trail,
            },
            RunScoped,
            mesh,
            material,
            Transform::from_translation(request.position)
                .with_rotation(request.rotation.unwrap_or(Quat::from_euler(EulerRot::XYZ, k, k * 0.5, k * 0.3)))
                .with_scale(request.size),
        ));
        if size < shadow_size() {
            entity.insert(NotShadowCaster);
        }
        let parent = entity.id();
        if let Some(trail) = trail {
            commands.entity(trail).insert(ChildOf(parent));
        }
        if request.burning && burning_count < MAX_BURNING {
            burning_count += 1;
            // Пламя стоит вертикально над обломком и не кувыркается вместе с ним.
            let remaining = 3.0 + (k * 0.37).fract() * 3.0;
            let size = (size * 1.2).clamp(0.5, 1.6);
            commands
                .entity(parent)
                .insert((Burning { remaining }, Fire::new(size, 0.85).for_seconds(remaining).then_smoke(3.0)));
        }
    }
}

/// Докуда от взрыва в долях радиуса подпрыгивают уснувшие обломки.
const WAKE_REACH: f32 = 1.8;

/// Насколько обломок может разойтись с новым рельефом, прежде чем его поднять, м.
const SETTLE_GAP: f32 = 0.05;

/// Забирает из хранилища уснувшие обломки в круге, для которых `take` вернул `true`, и убирает
/// их сущности из сцены. Возвращает забранные обломки.
fn take_pieces(
    store: &mut DebrisStore,
    commands: &mut Commands,
    center: Vec3,
    reach: f32,
    mut take: impl FnMut(&DebrisPiece) -> bool,
) -> Vec<DebrisPiece> {
    let mut taken = Vec::new();
    let low = chunk_coord_at(center.x - reach, center.z - reach);
    let high = chunk_coord_at(center.x + reach, center.z + reach);
    for cz in low.y..=high.y {
        for cx in low.x..=high.x {
            let chunk = IVec2::new(cx, cz);
            let Some(pieces) = store.chunks.get_mut(&chunk) else {
                continue;
            };
            let mut shown = store.shown.get_mut(&chunk);
            let mut i = 0;
            while i < pieces.len() {
                let p = pieces[i].transform.translation;
                if (p - center).with_y(0.0).length() > reach || !take(&pieces[i]) {
                    i += 1;
                    continue;
                }
                taken.push(pieces.remove(i));
                store.count = store.count.saturating_sub(1);
                // Списки обломков участка и их сущностей идут в одном порядке.
                if let Some(entities) = shown.as_mut()
                    && i < entities.len()
                {
                    commands.entity(entities.remove(i)).try_despawn();
                }
            }
        }
    }
    taken
}

/// Будит уснувшие обломки. Взрыв подбрасывает их от центра, и они падают в новый кратер.
/// Любое другое изменение рельефа (траншея лазера, вскрытый туннель, провал) отпускает обломки,
/// разошедшиеся с новой поверхностью: висящие падают, утонувшие встают на неё.
fn wake_debris(
    mut commands: Commands,
    mut explosions: MessageReader<Explosion>,
    mut changes: MessageReader<TerrainChanged>,
    terrain: Res<Terrain>,
    mut store: ResMut<DebrisStore>,
    mut requests: MessageWriter<SpawnDebris>,
) {
    let store = &mut *store;
    for explosion in explosions.read() {
        let reach = explosion.radius * WAKE_REACH;
        let c = explosion.position;
        for piece in take_pieces(store, &mut commands, c, reach, |_| true) {
            let p = piece.transform.translation;
            let offset = (p - c).with_y(0.0);
            let k = 1.0 - offset.length() / reach;
            requests.write(SpawnDebris {
                position: p + Vec3::Y * 0.2,
                velocity: offset.normalize_or(Vec3::X) * 6.0 * k + Vec3::Y * (2.0 + 7.0 * k),
                size: piece.transform.scale,
                color: piece.color,
                burning: false,
                rotation: Some(piece.transform.rotation),
                shape: piece.shape,
            });
        }
    }
    // Обломки у взрывов уже подброшены выше, здесь остаются только разошедшиеся с рельефом.
    for change in changes.read() {
        let off_ground = |piece: &DebrisPiece| {
            let p = piece.transform.translation;
            let ground = terrain.height_at(p.x, p.z) + half_height(&piece.shape, &piece.transform);
            (p.y - ground).abs() > SETTLE_GAP
        };
        for piece in take_pieces(store, &mut commands, change.center, change.radius, off_ground) {
            let p = piece.transform.translation;
            // Утонувший встаёт на поверхность, висящий падает с места.
            let ground = terrain.height_at(p.x, p.z) + half_height(&piece.shape, &piece.transform);
            requests.write(SpawnDebris {
                position: p.with_y(p.y.max(ground)),
                velocity: Vec3::ZERO,
                size: piece.transform.scale,
                color: piece.color,
                burning: false,
                rotation: Some(piece.transform.rotation),
                shape: piece.shape,
            });
        }
    }
}

/// Баллистика, отскоки и засыпание. Уснувший обломок пишется в хранилище.
fn fly_debris(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut store: ResMut<DebrisStore>,
    mut flying: Query<(Entity, &mut Transform, &mut Flying)>,
    assets: Res<DebrisAssets>,
    puffs: Query<(), With<DustPuff>>,
    focus: Option<Res<Focus>>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let mut puff_count = puffs.iter().count();
    for (entity, mut transform, mut piece) in &mut flying {
        piece.velocity.y -= GRAVITY * dt;
        transform.translation += piece.velocity * dt;
        let spin = piece.spin * dt;
        transform.rotate(Quat::from_euler(EulerRot::XYZ, spin.x, spin.y, spin.z));

        let ground =
            terrain.height_at(transform.translation.x, transform.translation.z) + half_height(&piece.shape, &transform);
        if transform.translation.y > ground {
            continue;
        }
        transform.translation.y = ground;
        if piece.velocity.y < 0.0 {
            // Крупный кусок при первом касании поднимает облачко пыли.
            if piece.bounces == 0 && puff_count < MAX_PUFFS && extent(&piece.shape, transform.scale) >= PUFF_SIZE {
                puff_count += 1;
                let at = transform.translation.with_y(ground - half_height(&piece.shape, &transform));
                commands.spawn((
                    DustPuff { left: PUFF_SECONDS },
                    RunScoped,
                    ParticleEffect::new(assets.puff.clone()),
                    Transform::from_translation(at),
                ));
            }
            piece.velocity.y = -piece.velocity.y * BOUNCINESS;
            piece.velocity.x *= GROUND_FRICTION;
            piece.velocity.z *= GROUND_FRICTION;
            piece.spin *= 0.5;
            piece.bounces += 1;
        }
        if piece.bounces > MAX_BOUNCES || piece.velocity.length() < SLEEP_SPEED {
            if let Some(trail) = piece.trail.take() {
                commands.entity(trail).try_despawn();
            }
            // Ложится плашмя по рельефу и засыпает.
            let up = terrain.normal_at(transform.translation.x, transform.translation.z);
            let (_, yaw, _) = transform.rotation.to_euler(EulerRot::YXZ);
            // Куб лежит как летел, кусок модели — плашмя, самой тонкой стороной вниз.
            let lay = match piece.shape {
                DebrisShape::Cube | DebrisShape::Form(_) => Quat::IDENTITY,
                DebrisShape::Mesh { half, .. } => {
                    let size = half * transform.scale;
                    let thin = if size.x <= size.y && size.x <= size.z {
                        Vec3::X
                    } else if size.z <= size.y {
                        Vec3::Z
                    } else {
                        Vec3::Y
                    };
                    Quat::from_rotation_arc(thin, Vec3::Y)
                }
            };
            transform.rotation = Quat::from_rotation_arc(Vec3::Y, up) * Quat::from_rotation_y(yaw) * lay;
            transform.translation.y = terrain.height_at(transform.translation.x, transform.translation.z)
                + half_height(&piece.shape, &transform);
            let chunk = chunk_coord_at(transform.translation.x, transform.translation.z);
            let stored = store.chunks.entry(chunk).or_default();
            let had_pieces = !stored.is_empty();
            stored.push(DebrisPiece {
                transform: *transform,
                color: piece.color,
                shape: piece.shape.clone(),
            });
            store.count += 1;
            // Списки обломков участка и их сущностей идут в одном порядке.
            if let Some(shown) = store.shown.get_mut(&chunk) {
                shown.push(entity);
                commands.entity(entity).try_remove::<Flying>();
            } else if !had_pieces {
                store.shown.insert(chunk, vec![entity]);
                commands.entity(entity).try_remove::<Flying>();
            } else {
                // Остальные обломки участка не в сцене: этот появится вместе с ними.
                commands.entity(entity).despawn();
            }
        }
    }
    let focus = focus.map_or(Vec3::ZERO, |f| f.0);
    limit_resting(&mut store, &mut commands, focus);
}

/// Держит число лежащих обломков не больше [`max_resting`]: убирает самый мелкий обломок
/// в самом дальнем от фокуса участке, пока лишние не кончатся.
fn limit_resting(store: &mut DebrisStore, commands: &mut Commands, focus: Vec3) {
    while store.count > max_resting() {
        let far = store
            .chunks
            .iter()
            .filter(|(_, pieces)| !pieces.is_empty())
            .map(|(&chunk, pieces)| (chunk, pieces[0].transform.translation.with_y(0.0).distance_squared(focus.with_y(0.0))))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(chunk, _)| chunk);
        let Some(chunk) = far else {
            store.count = 0;
            return;
        };
        let pieces = store.chunks.get_mut(&chunk).expect("участок найден выше");
        let smallest = pieces
            .iter()
            .enumerate()
            .min_by(|a, b| extent(&a.1.shape, a.1.transform.scale).total_cmp(&extent(&b.1.shape, b.1.transform.scale)))
            .map(|(i, _)| i)
            .expect("участок не пустой");
        pieces.remove(smallest);
        store.count -= 1;
        // Списки обломков участка и их сущностей идут в одном порядке.
        if let Some(entities) = store.shown.get_mut(&chunk)
            && smallest < entities.len()
        {
            commands.entity(entities.remove(smallest)).try_despawn();
        }
    }
}

/// Облачка пыли от упавших обломков исчезают, когда частицы осели.
fn fade_puffs(mut commands: Commands, time: Res<Time>, mut puffs: Query<(Entity, &mut DustPuff)>) {
    for (entity, mut puff) in &mut puffs {
        puff.left -= time.delta_secs();
        if puff.left <= 0.0 {
            commands.entity(entity).despawn();
        }
    }
}

/// Огонь на обломках гаснет.
fn burn_out(mut commands: Commands, time: Res<Time>, mut burning: Query<(Entity, &mut Burning)>) {
    for (entity, mut fire) in &mut burning {
        fire.remaining -= time.delta_secs();
        if fire.remaining <= 0.0 {
            commands.entity(entity).try_remove::<(Burning, Fire)>();
        }
    }
}

/// Уснувшие обломки появляются и убираются вместе с участками рельефа.
fn stream_debris(
    mut commands: Commands,
    visible: Res<VisibleChunks>,
    mut store: ResMut<DebrisStore>,
    mut assets: ResMut<DebrisAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !visible.is_changed() {
        return;
    }
    let store = &mut *store;
    store.shown.retain(|&chunk, entities| {
        let keep = visible.entity(chunk).is_some();
        if !keep {
            for &e in entities.iter() {
                commands.entity(e).try_despawn();
            }
        }
        keep
    });
    for (&chunk, pieces) in &store.chunks {
        if store.shown.contains_key(&chunk) || visible.entity(chunk).is_none() {
            continue;
        }
        let entities = pieces
            .iter()
            .map(|piece| {
                let (mesh, material) = assets.look(&piece.shape, piece.color, &mut materials);
                let mut e = commands.spawn((RunScoped, mesh, material, piece.transform));
                if extent(&piece.shape, piece.transform.scale) < shadow_size() {
                    e.insert(NotShadowCaster);
                }
                e.id()
            })
            .collect();
        store.shown.insert(chunk, entities);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(at: Vec3, size: f32) -> DebrisPiece {
        DebrisPiece {
            transform: Transform::from_translation(at).with_scale(Vec3::splat(size)),
            color: Color::WHITE,
            shape: DebrisShape::Cube,
        }
    }

    #[test]
    fn лишние_обломки_уходят_мелкие_и_дальние() {
        let mut world = World::new();
        let mut store = DebrisStore::default();
        let near = chunk_coord_at(0.0, 0.0);
        let far = chunk_coord_at(500.0, 0.0);
        store.chunks.insert(near, (0..MAX_RESTING).map(|_| piece(Vec3::ZERO, 0.1)).collect());
        store.chunks.insert(far, vec![piece(Vec3::new(500.0, 0.0, 0.0), 2.0), piece(Vec3::new(500.0, 0.0, 0.0), 0.5)]);
        store.count = MAX_RESTING + 2;
        let mut commands = world.commands();
        limit_resting(&mut store, &mut commands, Vec3::ZERO);
        assert_eq!(store.count, MAX_RESTING);
        // Ушли оба дальних, ближние мелкие остались все.
        assert_eq!(store.chunks[&near].len(), MAX_RESTING);
        assert!(store.chunks[&far].is_empty());

        store.chunks.get_mut(&far).unwrap().extend([piece(Vec3::new(500.0, 0.0, 0.0), 2.0), piece(Vec3::new(500.0, 0.0, 0.0), 0.5)]);
        store.count += 1;
        let mut commands = world.commands();
        limit_resting(&mut store, &mut commands, Vec3::ZERO);
        // Из дальнего участка ушёл самый мелкий.
        assert_eq!(store.chunks[&far].len(), 1);
        assert_eq!(store.chunks[&far][0].transform.scale.x, 2.0);
    }
}
