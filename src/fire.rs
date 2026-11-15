//! Огонь: один модуль, которым горит всё.
//!
//! Носитель получает компонент [`Fire`] с размером и силой. Модуль ставит рядом пламя — одну
//! или две вертикальные карточки с шейдером `assets/shaders/flame.wesl`, повёрнутые к камере
//! вокруг вертикали, — искры и дым на `bevy_hanabi`. Пламя идёт за позицией носителя, но не
//! наследует его вращение и масштаб: на кувыркающемся обломке оно стоит ровно. Свет получают
//! только несколько ближайших к камере огней из общего пула. Дальние огни без искр и дыма.
//!
//! Огонь с `lifetime` гаснет к концу жизни и снимается; если задан `smoke_after`, место дымит
//! ещё столько секунд.

use bevy::asset::RenderAssetUsages;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;
use bevy_hanabi::prelude::*;

use crate::camera::GameCamera;
use crate::frame::GameSet;
use crate::quality::Quality;
use crate::run::{RunScoped, WorldRebuilt};

/// Шейдер пламени.
const FLAME_SHADER: &str = "shaders/flame.wesl";
/// Ступеней силы пламени: по материалу на ступень, огни одной силы рисуются одним материалом.
const STRENGTH_STEPS: usize = 8;
/// С какого размера пламя из двух карточек: вторая повёрнута, чтобы сверху не было видно ребра.
const TWIN_SIZE: f32 = 1.2;
/// Доля жизни огня в конце, за которую он гаснет.
const FADE_SHARE: f32 = 0.3;
/// Дальше этого от камеры огонь без искр и дыма, м.
const FAR: f32 = 60.0;
/// Период выпуска частиц постоянных эффектов, с.
const EMIT_PERIOD: f32 = 0.1;
/// Искр и клубов дыма в секунду на метр размера огня при полной силе.
const SPARK_RATE: f32 = 14.0;
const SMOKE_RATE: f32 = 6.0;
/// Свет огня: сила на метр размера, лм, и дальность на метр размера, м.
const LIGHT_PER_SIZE: f32 = 60_000.0;
const LIGHT_RANGE: f32 = 7.0;
/// Цвет света огня.
const LIGHT_COLOR: Color = Color::srgb(1.0, 0.55, 0.2);

/// Подключает огонь.
pub struct FirePlugin;

impl Plugin for FirePlugin {
    fn build(&self, app: &mut App) {
        // Без сервера ресурсов (тест порядка кадра) материал не регистрируется.
        if app.world().contains_resource::<AssetServer>() {
            app.add_plugins(MaterialPlugin::<FlameMaterial>::default());
        }
        app.add_systems(Startup, (create_fire_assets, spawn_light_pool))
            .add_systems(
                Update,
                (
                    reset_lights.in_set(GameSet::Reset),
                    age_fires.in_set(GameSet::Simulation),
                    (light_fires, follow_fires, end_smoke)
                        .chain()
                        .in_set(GameSet::Presentation),
                ),
            )
            // Пламя ставится и убирается сразу после команд кадра, где огонь появился.
            .add_systems(PostUpdate, (ignite_fires, put_out_fires, raise_plumes).chain());
    }
}

/// Огонь на носителе.
#[derive(Component, Debug, Clone)]
pub struct Fire {
    /// Размер огня: высота пламени примерно вдвое больше, м.
    pub size: f32,
    /// Сила от 0 до 1: низкий рваный огонь или высокий плотный.
    pub intensity: f32,
    /// Сколько огонь горит, с; `None` — пока его не снимут.
    pub lifetime: Option<f32>,
    /// Сколько секунд место дымит после огня.
    pub smoke_after: f32,
    age: f32,
}

impl Fire {
    /// Огонь размера `size` и силы `intensity`, горит, пока его не снимут.
    pub fn new(size: f32, intensity: f32) -> Self {
        Self {
            size,
            intensity: intensity.clamp(0.0, 1.0),
            lifetime: None,
            smoke_after: 0.0,
            age: 0.0,
        }
    }

    /// Огонь горит `seconds` секунд и гаснет к концу.
    pub fn for_seconds(mut self, seconds: f32) -> Self {
        self.lifetime = Some(seconds);
        self
    }

    /// После огня место дымит `seconds` секунд.
    pub fn then_smoke(mut self, seconds: f32) -> Self {
        self.smoke_after = seconds;
        self
    }

    /// Сила с угасанием к концу жизни: последние [`FADE_SHARE`] жизни падает до нуля.
    pub fn strength(&self) -> f32 {
        let fade = match self.lifetime {
            Some(life) if life > 0.0 => ((life - self.age) / (life * FADE_SHARE)).clamp(0.0, 1.0),
            Some(_) => 0.0,
            None => 1.0,
        };
        self.intensity * fade
    }

    /// Огонь догорел.
    pub fn is_out(&self) -> bool {
        self.lifetime.is_some_and(|life| self.age >= life)
    }
}

/// Материал пламени: сила в `params.x`.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct FlameMaterial {
    #[uniform(0)]
    params: Vec4,
}

impl Material for FlameMaterial {
    fn fragment_shader() -> ShaderRef {
        FLAME_SHADER.into()
    }

    fn alpha_mode(&self) -> bevy::prelude::AlphaMode {
        bevy::prelude::AlphaMode::Blend
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }
}

#[derive(Resource)]
struct FireAssets {
    card: Handle<Mesh>,
    /// Материалы по ступеням силы, от слабой к полной.
    flames: Vec<Handle<FlameMaterial>>,
    sparks: Handle<EffectAsset>,
    smoke: Handle<EffectAsset>,
}

impl FireAssets {
    fn flame(&self, strength: f32) -> Handle<FlameMaterial> {
        let step = (strength.clamp(0.0, 1.0) * (STRENGTH_STEPS - 1) as f32).round() as usize;
        self.flames[step].clone()
    }
}

/// Пламя огня: отдельная сущность рядом с носителем.
#[derive(Component)]
struct Flame {
    carrier: Entity,
    size: f32,
    step: usize,
}

/// Карточка пламени; `twist` — поворот от направления на камеру.
#[derive(Component)]
struct FlameCard;

#[derive(Component)]
struct Sparks;

#[derive(Component)]
struct FireSmoke;

/// Ссылка носителя на своё пламя.
#[derive(Component)]
struct FlameOf;

/// Дым на месте погасшего огня: сколько ещё дымит, с.
#[derive(Component)]
struct Afterglow {
    left: f32,
}

/// Столб дыма без огня: дымит, пока сущность есть. Например, над горячим кратером лавы.
#[derive(Component)]
pub struct SmokePlume;

/// Источник света из пула.
#[derive(Component)]
struct PooledLight(usize);

fn create_fire_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<FlameMaterial>>,
    mut effects: ResMut<Assets<EffectAsset>>,
) {
    // Карточка 1 × 1 низом в начале координат.
    let card = Mesh::from(Rectangle::new(1.0, 1.0)).translated_by(Vec3::Y * 0.5);
    let flames = (0..STRENGTH_STEPS)
        .map(|i| {
            let strength = i as f32 / (STRENGTH_STEPS - 1) as f32;
            materials.add(FlameMaterial {
                params: Vec4::new(strength, 0.0, 0.0, 0.0),
            })
        })
        .collect();
    commands.insert_resource(FireAssets {
        card: meshes.add(card),
        flames,
        sparks: effects.add(sparks_effect()),
        smoke: effects.add(smoke_effect()),
    });
}

/// Мягкий круглый клуб для дыма: прозрачность спадает к краю, 64 × 64.
fn soft_puff() -> Image {
    const SIZE: u32 = 64;
    let mut data = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let d = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) / SIZE as f32 * 2.0 - Vec2::ONE;
            let a = (1.0 - d.length()).clamp(0.0, 1.0);
            let a = a * a * (3.0 - 2.0 * a);
            data.extend([255, 255, 255, (a * 255.0) as u8]);
        }
    }
    Image::new(
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Искры: мелкие раскалённые угольки летят вверх, кружатся и гаснут.
fn sparks_effect() -> EffectAsset {
    let writer = ExprWriter::new();
    let init_pos = SetPositionSphereModifier {
        center: writer.lit(Vec3::Y * 0.4).expr(),
        radius: writer.lit(0.5).expr(),
        dimension: ShapeDimension::Volume,
    };
    let init_vel = SetVelocitySphereModifier {
        center: writer.lit(Vec3::Y * -3.0).expr(),
        speed: writer.lit(1.5).uniform(writer.lit(4.0)).expr(),
    };
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, writer.lit(0.6).uniform(writer.lit(1.4)).expr());
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    // Тяга вверх и случайный снос: угольки вьются, а не летят по прямой.
    let lift = writer.lit(Vec3::Y * 2.5) + writer.rand(VectorType::VEC3F) * writer.lit(6.0) - writer.lit(Vec3::splat(3.0));
    let update_lift = AccelModifier::new(lift.expr());
    let update_drag = LinearDragModifier::new(writer.lit(1.2).expr());
    let mut color = bevy_hanabi::Gradient::new();
    color.add_key(0.0, Vec4::new(6.0, 3.6, 1.2, 1.0));
    color.add_key(0.5, Vec4::new(3.0, 1.0, 0.2, 1.0));
    color.add_key(1.0, Vec4::new(0.6, 0.1, 0.02, 0.0));
    let mut size = bevy_hanabi::Gradient::new();
    size.add_key(0.0, Vec3::splat(0.09));
    size.add_key(1.0, Vec3::splat(0.03));
    EffectAsset::new(256, SpawnerSettings::rate(0.0.into()), writer.finish())
        .with_name("fire_sparks")
        .with_alpha_mode(bevy_hanabi::AlphaMode::Add)
        .init(init_pos)
        .init(init_vel)
        .init(init_lifetime)
        .init(init_age)
        .update(update_lift)
        .update(update_drag)
        .render(OrientModifier::new(OrientMode::FaceCameraPosition))
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

/// Дым: мягкие клубы поднимаются, растут, сносятся ветром и тают.
fn smoke_effect() -> EffectAsset {
    let writer = ExprWriter::new();
    let init_pos = SetPositionSphereModifier {
        center: writer.lit(Vec3::Y * 1.2).expr(),
        radius: writer.lit(0.6).expr(),
        dimension: ShapeDimension::Volume,
    };
    let init_vel = SetVelocitySphereModifier {
        center: writer.lit(Vec3::Y * -1.0).expr(),
        speed: writer.lit(0.8).uniform(writer.lit(1.8)).expr(),
    };
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, writer.lit(2.5).uniform(writer.lit(4.0)).expr());
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    // Ветер в ту же сторону, что у дыма взрывов и вагонов.
    let update_wind = AccelModifier::new(writer.lit(Vec3::new(0.9, 1.4, -0.45)).expr());
    let update_drag = LinearDragModifier::new(writer.lit(1.0).expr());
    let mut module = writer.finish();
    module.add_texture_slot("puff", SlotDimension::D2);
    let mut color = bevy_hanabi::Gradient::new();
    color.add_key(0.0, Vec4::new(0.12, 0.11, 0.1, 0.0));
    color.add_key(0.12, Vec4::new(0.14, 0.13, 0.12, 0.45));
    color.add_key(0.6, Vec4::new(0.3, 0.29, 0.28, 0.25));
    color.add_key(1.0, Vec4::new(0.45, 0.45, 0.45, 0.0));
    let mut size = bevy_hanabi::Gradient::new();
    size.add_key(0.0, Vec3::splat(0.8));
    size.add_key(1.0, Vec3::splat(4.0));
    EffectAsset::new(128, SpawnerSettings::rate(0.0.into()), module)
        .with_name("fire_smoke")
        .init(init_pos)
        .init(init_vel)
        .init(init_lifetime)
        .init(init_age)
        .update(update_wind)
        .update(update_drag)
        // Белый клуб с мягкой прозрачностью: цвет и прозрачность по жизни умножаются на него.
        .render(ParticleTextureModifier {
            texture_slot: 0,
            sample_mapping: ImageSampleMapping::Modulate,
        })
        .render(OrientModifier::new(OrientMode::FaceCameraPosition))
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

/// Эмиттер постоянного эффекта: густоту задаёт `follow_fires`, сначала он молчит.
fn emitter(effect: &Handle<EffectAsset>) -> impl Bundle {
    let settings = SpawnerSettings::new(0.0.into(), EMIT_PERIOD.into(), EMIT_PERIOD.into(), 0);
    (ParticleEffect::new(effect.clone()), EffectSpawner::new(&settings))
}

/// Ставит пламя новым огням.
fn ignite_fires(
    mut commands: Commands,
    assets: Option<Res<FireAssets>>,
    puff: Option<Res<PuffTexture>>,
    fires: Query<(Entity, &Fire, &GlobalTransform), Without<FlameOf>>,
) {
    let Some(assets) = assets else {
        return;
    };
    for (carrier, fire, transform) in &fires {
        let strength = fire.strength();
        commands
            .spawn((
                RunScoped,
                Flame {
                    carrier,
                    size: fire.size,
                    step: usize::MAX,
                },
                Transform::from_translation(transform.translation()),
                Visibility::default(),
            ))
            .with_children(|root| {
                let cards = if fire.size >= TWIN_SIZE { 2 } else { 1 };
                for k in 0..cards {
                    let (width, height) = (fire.size * 1.3 * (1.0 - 0.25 * k as f32), fire.size * 2.0);
                    root.spawn((
                        FlameCard,
                        Mesh3d(assets.card.clone()),
                        MeshMaterial3d(assets.flame(strength)),
                        NotShadowCaster,
                        Transform::from_rotation(Quat::from_rotation_y(k as f32 * 1.1))
                            .with_scale(Vec3::new(width, height * (1.0 - 0.2 * k as f32), 1.0)),
                    ));
                }
                root.spawn((Sparks, emitter(&assets.sparks), Transform::default()));
                let mut smoke = root.spawn((FireSmoke, emitter(&assets.smoke), Transform::from_xyz(0.0, fire.size, 0.0)));
                if let Some(puff) = puff.as_deref() {
                    smoke.insert(EffectMaterial {
                        images: vec![puff.0.clone()],
                    });
                }
            });
        commands.entity(carrier).try_insert(FlameOf);
    }
}

/// Мягкая текстура клуба дыма.
#[derive(Resource)]
struct PuffTexture(Handle<Image>);

/// Огонь снят или носитель удалён: пламя гаснет.
fn put_out_fires(mut commands: Commands, flames: Query<(Entity, &Flame)>, fires: Query<(), With<Fire>>) {
    for (entity, flame) in &flames {
        if fires.contains(flame.carrier) {
            continue;
        }
        commands.entity(entity).try_despawn();
        if let Ok(mut carrier) = commands.get_entity(flame.carrier) {
            carrier.try_remove::<FlameOf>();
        }
    }
}

/// Огонь стареет; догоревший снимается, а его место дымит `smoke_after`.
fn age_fires(
    mut commands: Commands,
    time: Res<Time>,
    assets: Option<Res<FireAssets>>,
    puff: Option<Res<PuffTexture>>,
    mut fires: Query<(Entity, &mut Fire, &GlobalTransform)>,
) {
    let dt = time.delta_secs();
    for (entity, mut fire, transform) in &mut fires {
        fire.age += dt;
        if !fire.is_out() {
            continue;
        }
        commands.entity(entity).try_remove::<Fire>();
        if fire.smoke_after > 0.0
            && let Some(assets) = assets.as_deref()
        {
            let mut smoke = commands.spawn((
                RunScoped,
                FireSmoke,
                Afterglow { left: fire.smoke_after },
                emitter(&assets.smoke),
                Transform::from_translation(transform.translation() + Vec3::Y * fire.size),
            ));
            if let Some(puff) = puff.as_deref() {
                smoke.insert(EffectMaterial {
                    images: vec![puff.0.clone()],
                });
            }
        }
    }
}

/// Новым столбам дыма — эмиттер.
fn raise_plumes(
    mut commands: Commands,
    assets: Option<Res<FireAssets>>,
    puff: Option<Res<PuffTexture>>,
    plumes: Query<Entity, (With<SmokePlume>, Without<EffectSpawner>)>,
) {
    let Some(assets) = assets else {
        return;
    };
    for entity in &plumes {
        let mut plume = commands.entity(entity);
        plume.try_insert((FireSmoke, emitter(&assets.smoke)));
        if let Some(puff) = puff.as_deref() {
            plume.try_insert(EffectMaterial {
                images: vec![puff.0.clone()],
            });
        }
    }
}

/// Дым после огня тает и удаляется.
fn end_smoke(
    mut commands: Commands,
    time: Res<Time>,
    quality: Res<Quality>,
    mut smokes: Query<(Entity, &mut Afterglow, &mut EffectSpawner), Without<SmokePlume>>,
    mut plumes: Query<&mut EffectSpawner, With<SmokePlume>>,
) {
    for mut spawner in &mut plumes {
        set_rate(&mut spawner, SMOKE_RATE * quality.particle_share());
    }
    for (entity, mut glow, mut spawner) in &mut smokes {
        glow.left -= time.delta_secs();
        let rate = if glow.left > 0.0 { SMOKE_RATE * 1.5 * quality.particle_share() } else { 0.0 };
        set_rate(&mut spawner, rate);
        // Последние клубы живут до 4 с.
        if glow.left < -4.0 {
            commands.entity(entity).try_despawn();
        }
    }
}

/// Задаёт эмиттеру число частиц в секунду.
fn set_rate(spawner: &mut EffectSpawner, rate: f32) {
    let count = (rate * EMIT_PERIOD).max(0.0);
    if spawner.settings.count() != count.into() {
        spawner.settings.set_count(count.into());
    }
}

/// Пламя идёт за носителем, поворачивается к камере, меняет силу; у дальних огней нет частиц.
#[allow(clippy::type_complexity)]
fn follow_fires(
    camera: Query<&GlobalTransform, With<GameCamera>>,
    quality: Res<Quality>,
    assets: Option<Res<FireAssets>>,
    fires: Query<(&Fire, &GlobalTransform), Without<Flame>>,
    mut flames: Query<(&mut Flame, &mut Transform, &Children)>,
    mut cards: Query<&mut MeshMaterial3d<FlameMaterial>, With<FlameCard>>,
    mut spawners: Query<(&mut EffectSpawner, Has<Sparks>), Or<(With<Sparks>, With<FireSmoke>)>>,
) {
    let Some(assets) = assets else {
        return;
    };
    let eye = camera.iter().next().map(GlobalTransform::translation);
    let share = quality.particle_share();
    for (mut flame, mut transform, children) in &mut flames {
        let Ok((fire, carrier)) = fires.get(flame.carrier) else {
            continue;
        };
        transform.translation = carrier.translation();
        let far = eye.is_some_and(|e| e.distance(transform.translation) > FAR);
        if let Some(eye) = eye {
            let to = eye - transform.translation;
            transform.rotation = Quat::from_rotation_y(to.x.atan2(to.z));
        }
        let strength = fire.strength();
        let step = (strength * (STRENGTH_STEPS - 1) as f32).round() as usize;
        let changed = step != flame.step;
        flame.step = step;
        flame.size = fire.size;
        for child in children.iter() {
            if changed && let Ok(mut material) = cards.get_mut(child) {
                material.0 = assets.flame(strength);
            }
            if let Ok((mut spawner, sparks)) = spawners.get_mut(child) {
                let base = if sparks { SPARK_RATE } else { SMOKE_RATE };
                let rate = if far { 0.0 } else { base * fire.size * strength * share };
                set_rate(&mut spawner, rate);
            }
        }
    }
}

/// Пул света: столько источников, сколько разрешает ступень качества.
fn lights_for(quality: Quality) -> usize {
    match quality {
        Quality::Low => 4,
        Quality::Medium => 6,
        Quality::High => 8,
    }
}

/// Источники света пула: создаются один раз и переходят от огня к огню.
fn spawn_light_pool(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.insert_resource(PuffTexture(images.add(soft_puff())));
    for i in 0..lights_for(Quality::High) {
        commands.spawn((
            PooledLight(i),
            PointLight {
                color: LIGHT_COLOR,
                intensity: 0.0,
                range: 10.0,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::default(),
            Visibility::Hidden,
        ));
    }
}

/// Новый мир: свет пула гаснет до первых огней.
fn reset_lights(mut rebuilt: MessageReader<WorldRebuilt>, mut lights: Query<&mut Visibility, With<PooledLight>>) {
    if rebuilt.read().count() == 0 {
        return;
    }
    for mut visibility in &mut lights {
        *visibility = Visibility::Hidden;
    }
}

/// Номера `count` огней, ближайших к `eye`, по возрастанию расстояния.
fn nearest(positions: &[Vec3], eye: Vec3, count: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..positions.len()).collect();
    order.sort_by(|&a, &b| positions[a].distance_squared(eye).total_cmp(&positions[b].distance_squared(eye)));
    order.truncate(count);
    order
}

/// Ближайшие к камере огни получают мерцающий свет из пула, остальные — без света.
fn light_fires(
    time: Res<Time<Real>>,
    quality: Res<Quality>,
    camera: Query<&GlobalTransform, With<GameCamera>>,
    fires: Query<(&Fire, &GlobalTransform)>,
    mut lights: Query<(&PooledLight, &mut PointLight, &mut Transform, &mut Visibility)>,
) {
    let Some(eye) = camera.iter().next().map(GlobalTransform::translation) else {
        return;
    };
    let fires: Vec<(f32, Vec3)> = fires
        .iter()
        .filter(|(fire, _)| fire.strength() > 0.05)
        .map(|(fire, t)| (fire.size * fire.strength(), t.translation()))
        .collect();
    let positions: Vec<Vec3> = fires.iter().map(|f| f.1).collect();
    let chosen = nearest(&positions, eye, lights_for(*quality));
    let t = time.elapsed_secs();
    for (pooled, mut light, mut transform, mut visibility) in &mut lights {
        let Some(&index) = chosen.get(pooled.0) else {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
            }
            continue;
        };
        let (power, position) = fires[index];
        // Мерцание: два синуса с несоизмеримыми частотами и фазой от места огня.
        let phase = position.x * 0.7 + position.z * 1.3;
        let flicker = 0.8 + 0.12 * (t * 9.0 + phase).sin() + 0.08 * (t * 23.0 + phase * 2.0).sin();
        light.intensity = LIGHT_PER_SIZE * power * flicker;
        light.range = LIGHT_RANGE * power.max(0.5) + 4.0;
        transform.translation = position + Vec3::Y * (0.6 + power * 0.5);
        if *visibility != Visibility::Inherited {
            *visibility = Visibility::Inherited;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn огонь_гаснет_к_концу_жизни() {
        let mut fire = Fire::new(1.0, 0.8).for_seconds(10.0);
        assert!((fire.strength() - 0.8).abs() < 1e-6);
        fire.age = 6.9;
        assert!((fire.strength() - 0.8).abs() < 1e-6, "до угасания сила полная");
        fire.age = 8.5;
        assert!(fire.strength() < 0.8 && fire.strength() > 0.0);
        fire.age = 10.0;
        assert!(fire.is_out());
        assert_eq!(fire.strength(), 0.0);
        // Без срока огонь не гаснет.
        let endless = Fire::new(1.0, 1.0);
        assert!(!endless.is_out());
        assert_eq!(endless.strength(), 1.0);
    }

    #[test]
    fn свет_получают_ближайшие_огни() {
        let positions = [Vec3::new(50.0, 0.0, 0.0), Vec3::new(5.0, 0.0, 0.0), Vec3::new(20.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)];
        assert_eq!(nearest(&positions, Vec3::ZERO, 2), vec![3, 1]);
        assert_eq!(nearest(&positions, Vec3::ZERO, 10).len(), 4);
        assert!(lights_for(Quality::Low) < lights_for(Quality::High));
    }
}
