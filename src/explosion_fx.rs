//! Эффекты взрыва: вспышка света, огненный шар и кольцо ударной волны с пылью.
//!
//! На каждое сообщение `Explosion` создаётся одна корневая сущность `ExplosionFx` с дочерними
//! слоями. Каждый слой анимируется по возрасту корня, корень удаляется по таймеру вместе со
//! слоями. Все слои масштабируются по мощности взрыва и получают случайный разброс, поэтому
//! два взрыва в одном месте выглядят по-разному.
//!
//! | Слой | Чем сделан | Время |
//! |---|---|---|
//! | Вспышка | `PointLight` без теней, яркость гаснет по экспоненте | 0–0,15 с |
//! | Огненный шар | несколько неосвещённых сфер, растут, желтеют, краснеют и темнеют; поверх частицы огня | 0–0,7 с |
//! | Ударная волна | кольцо, лежащее на рельефе, быстро расходится и тает; по краю частицы пыли | 0–0,4 с |
//! | Выброс грунта | частицы с гравитацией и крупные комья-обломки | 0–1,5 с |
//! | Искры | яркие короткие частицы с гравитацией | 0–0,6 с |
//! | Горящие осколки | горящие обломки со шлейфом дыма, догорают на земле | 0–5 с |
//! | Дым | крупные полупрозрачные частицы, подъём, расширение, снос ветром | 0,3–4 с |
//!
//! Полный эффект — пять систем частиц, поэтому одновременно их не больше [`MAX_FULL_FX`].
//! Слабый, далёкий или сверх предела взрыв получает упрощённый эффект: огненный шар, огонь
//! и грунт с третью частиц, без пыли, искр, дыма и кольца. Сверх [`MAX_FX`] остаётся только
//! огненный шар, а взрыв дальше [`FX_RANGE`] от фокуса камеры не рисуется вовсе. Свет вспышки
//! удаляется, как только она погасла.

use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::Face;
use bevy_hanabi::prelude::*;

use crate::camera::Focus;
use crate::debris::{DebrisShape, SpawnDebris};
use crate::debris_forms::FormKind;
use crate::explosion::Explosion;
use crate::frame::GameSet;
use crate::run::RunScoped;
use crate::terrain::Terrain;

/// Сколько живёт корень взрыва, с: дольше всех слоёв, включая частицы пыли.
const FX_LIFETIME: f32 = 4.5;
/// Сколько живёт корень упрощённого эффекта, с: дыма нет, дольше всех живёт грунт.
const LIGHT_FX_LIFETIME: f32 = 1.6;

/// Сколько полных эффектов (пять систем частиц) может жить одновременно.
const MAX_FULL_FX: usize = 8;
/// Сколько эффектов любого вида может жить одновременно; сверх него — только огненный шар.
const MAX_FX: usize = 24;
/// Взрыв слабее этого получает упрощённый эффект: гибель врага (0,25), ракета (0,45),
/// вражеская мортира (0,35), капсула (0,5).
const LIGHT_POWER: f32 = 0.8;
/// Дальше этого от фокуса камеры взрыв получает упрощённый эффект, м: у края кадра и за ним.
const FULL_RANGE: f32 = 60.0;
/// Дальше этого от фокуса камеры эффекта нет, м: за пределом подгруженных участков.
const FX_RANGE: f32 = 120.0;
/// Доля частиц огня и грунта у упрощённого эффекта.
const LIGHT_SHARE: f32 = 0.35;

/// Пиковая яркость вспышки при мощности 1, лм.
const FLASH_PEAK: f32 = 3.0e7;
/// Постоянная затухания вспышки, с: за 0,15 с яркость падает примерно в 40 раз.
const FLASH_DECAY: f32 = 0.04;
/// Сколько горит вспышка, с.
const FLASH_TIME: f32 = 0.15;
/// Дальность света вспышки в радиусах взрыва.
const FLASH_RANGE: f32 = 8.0;
/// Высота вспышки над точкой взрыва, м: чтобы свет падал на склоны вокруг, а не только в чашу.
const FLASH_HEIGHT: f32 = 2.0;
/// Цвет вспышки: тёплый белый.
const FLASH_COLOR: Color = Color::srgb(1.0, 0.85, 0.6);

/// Сколько живёт огненный шар, с.
const FIREBALL_TIME: f32 = 0.7;
/// Наибольший радиус огненного шара в радиусах взрыва.
const FIREBALL_SIZE: f32 = 0.55;
/// С какой доли наибольшего радиуса шар начинает расти.
const FIREBALL_START: f32 = 0.2;
/// Насколько шар поднимается за свою жизнь, м.
const FIREBALL_RISE: f32 = 1.5;
/// Сколько сфер в огненном шаре: от и до.
const FIREBALL_BLOBS: (u32, u32) = (3, 5);

/// Насколько ярче цвета ядро шара в момент вспышки и до какой доли жизни шар раскалён.
const FIREBALL_GLOW: f32 = 3.0;
const FIREBALL_HOT: f32 = 0.35;

/// Цвет огненного шара по доле его жизни: от жёлто-белого через оранжевый к тёмному дыму.
const FIREBALL_COLORS: [(f32, [f32; 4]); 5] = [
    (0.0, [1.0, 0.97, 0.85, 1.0]),
    (0.15, [1.0, 0.78, 0.32, 1.0]),
    (0.35, [0.95, 0.42, 0.08, 0.95]),
    (0.65, [0.32, 0.16, 0.08, 0.75]),
    (1.0, [0.12, 0.11, 0.1, 0.0]),
];

/// Сколько расходится ударная волна, с.
const SHOCK_TIME: f32 = 0.4;
/// До какого радиуса доходит волна, в радиусах взрыва.
const SHOCK_REACH: f32 = 2.4;
/// Ширина кольца в начале и в конце, м.
const SHOCK_WIDTH: (f32, f32) = (0.5, 1.6);
/// Непрозрачность кольца в начале.
const SHOCK_ALPHA: f32 = 0.7;
/// Подъём кольца над рельефом, м: чтобы не мерцало с поверхностью.
const SHOCK_LIFT: f32 = 0.08;
/// Число сегментов кольца.
const SHOCK_SEGMENTS: usize = 72;
/// Цвет кольца: светлая пыль.
const SHOCK_COLOR: [f32; 3] = [0.9, 0.85, 0.75];

/// Частиц огня на взрыв мощности 1.
const FIRE_PARTICLES: f32 = 60.0;
/// Частиц пыли ударной волны на взрыв мощности 1.
const DUST_PARTICLES: f32 = 90.0;
/// Частиц выброшенного грунта на взрыв мощности 1.
const EJECTA_PARTICLES: f32 = 150.0;
/// Искр на взрыв мощности 1.
const SPARK_PARTICLES: f32 = 70.0;
/// Клубов дыма на взрыв мощности 1.
const SMOKE_PARTICLES: f32 = 40.0;
/// Комьев грунта на взрыв: от и до.
const CLODS: (u32, u32) = (6, 10);
/// Горящих осколков на взрыв: от и до.
const BURNING_SHARDS: (u32, u32) = (4, 6);
/// Взрыв слабее этого комьев и осколков не разбрасывает: пули и мелочь не засоряют поле.
const DEBRIS_MIN_POWER: f32 = 0.3;
/// Цвет комьев грунта.
const CLOD_COLOR: Color = Color::srgb(0.36, 0.25, 0.16);
/// Цвет горящих осколков: обугленный.
const SHARD_COLOR: Color = Color::srgb(0.14, 0.11, 0.09);

/// Плагин эффектов взрыва.
pub struct ExplosionFxPlugin;

impl Plugin for ExplosionFxPlugin {
    fn build(&self, app: &mut App) {
        // `HanabiPlugin` подключает `main`: ему нужен рендер.
        app.add_systems(Startup, create_assets)
            .add_systems(
                Update,
                (
                    spawn_explosions,
                    age_explosions,
                    animate_flash,
                    animate_fireball,
                    animate_shockwave,
                )
                    .chain()
                    .in_set(GameSet::Presentation),
            );
    }
}

/// Насколько подробно рисуется взрыв.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FxTier {
    /// Все слои: вспышка, шар, кольцо, пять систем частиц.
    Full,
    /// Вспышка, шар и две системы частиц с третью частиц.
    Light,
    /// Только огненный шар.
    Minimal,
    /// Ничего: взрыв далеко за кадром.
    Hidden,
}

/// Подробность эффекта по мощности, расстоянию до фокуса камеры и числу живых эффектов.
pub fn fx_tier(power: f32, distance: f32, full_alive: usize, alive: usize) -> FxTier {
    if distance > FX_RANGE {
        FxTier::Hidden
    } else if alive >= MAX_FX {
        FxTier::Minimal
    } else if power < LIGHT_POWER || distance > FULL_RANGE || full_alive >= MAX_FULL_FX {
        FxTier::Light
    } else {
        FxTier::Full
    }
}

/// Корень взрыва: возраст и параметры, по которым анимируются слои.
#[derive(Component)]
struct ExplosionFx {
    /// Сколько прошло с взрыва, с.
    age: f32,
    /// Сколько живёт корень, с.
    life: f32,
    /// Полный ли эффект: для предела одновременных.
    full: bool,
    /// Центр взрыва.
    center: Vec3,
    /// Мощность взрыва.
    power: f32,
}

/// Слой «вспышка».
#[derive(Component)]
struct FlashLayer;

/// Одна сфера огненного шара.
#[derive(Component)]
struct FireballBlob {
    /// Смещение от центра шара при наибольшем радиусе.
    offset: Vec3,
    /// Масштаб сферы по осям при наибольшем радиусе.
    size: Vec3,
    /// Задержка появления, с: сферы раздуваются не одновременно.
    delay: f32,
}

/// Слой «ударная волна»: кольцо, которое каждый кадр кладётся на рельеф.
#[derive(Component)]
struct ShockwaveLayer {
    /// Наибольший радиус волны, м.
    reach: f32,
}

/// Общие ресурсы эффектов: создаются один раз при старте.
#[derive(Resource)]
struct ExplosionFxAssets {
    sphere: Handle<Mesh>,
    fire: Handle<EffectAsset>,
    dust: Handle<EffectAsset>,
    ejecta: Handle<EffectAsset>,
    sparks: Handle<EffectAsset>,
    smoke: Handle<EffectAsset>,
    /// Огонь и грунт упрощённого эффекта.
    light_fire: Handle<EffectAsset>,
    light_ejecta: Handle<EffectAsset>,
}

/// Яркость вспышки в момент `age`, лм.
pub fn flash_intensity(age: f32, power: f32) -> f32 {
    if age >= FLASH_TIME {
        return 0.0;
    }
    FLASH_PEAK * power * (-age / FLASH_DECAY).exp()
}

/// Радиус огненного шара по доле жизни `t` в долях наибольшего: быстро растёт и замедляется.
pub fn fireball_growth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    FIREBALL_START + (1.0 - FIREBALL_START) * (1.0 - (1.0 - t).powi(3))
}

/// Цвет огненного шара по доле жизни `t`, линейно между ключами `FIREBALL_COLORS`.
pub fn fireball_color(t: f32) -> [f32; 4] {
    let t = t.clamp(0.0, 1.0);
    let keys = &FIREBALL_COLORS;
    let i = keys
        .windows(2)
        .position(|w| t <= w[1].0)
        .unwrap_or(keys.len() - 2);
    let (t0, a) = keys[i];
    let (t1, b) = keys[i + 1];
    let k = (t - t0) / (t1 - t0);
    std::array::from_fn(|c| a[c] + (b[c] - a[c]) * k)
}

/// Во сколько раз ядро огненного шара ярче своего цвета: в начале жизни выше единицы, чтобы
/// светилось через свечение камеры, к остыванию — ровно цвет.
pub fn fireball_glow(t: f32) -> f32 {
    1.0 + FIREBALL_GLOW * (1.0 - t / FIREBALL_HOT).max(0.0)
}

/// Радиус, ширина и непрозрачность кольца ударной волны по доле жизни `t`.
pub fn shockwave_shape(t: f32, reach: f32) -> (f32, f32, f32) {
    let t = t.clamp(0.0, 1.0);
    let radius = reach * (1.0 - (1.0 - t).powi(2));
    let width = SHOCK_WIDTH.0 + (SHOCK_WIDTH.1 - SHOCK_WIDTH.0) * t;
    let alpha = SHOCK_ALPHA * (1.0 - t).powf(1.5);
    (radius, width, alpha)
}

/// Простой генератор случайных чисел для разброса: xorshift от сида взрыва.
struct Rng(u32);

impl Rng {
    fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }

    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// Случайное число от `min` до `max`.
    fn range(&mut self, min: f32, max: f32) -> f32 {
        let unit = (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32;
        min + (max - min) * unit
    }
}

/// Сид взрыва из его позиции и времени: разный у двух взрывов в одной точке.
fn explosion_seed(position: Vec3, elapsed: f32) -> u32 {
    let mut h = 0x9e37_79b9u32;
    for bits in [
        position.x.to_bits(),
        position.y.to_bits(),
        position.z.to_bits(),
        elapsed.to_bits(),
    ] {
        h = (h ^ bits).wrapping_mul(0x85eb_ca6b);
        h ^= h >> 13;
    }
    h
}

/// Создаёт сферу огненного шара и шаблоны частиц.
fn create_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut effects: ResMut<Assets<EffectAsset>>,
) {
    commands.insert_resource(ExplosionFxAssets {
        sphere: meshes.add(
            Sphere::new(1.0)
                .mesh()
                .ico(3)
                .expect("сфера огненного шара"),
        ),
        fire: effects.add(fire_effect(FIRE_PARTICLES)),
        dust: effects.add(dust_effect()),
        ejecta: effects.add(ejecta_effect(EJECTA_PARTICLES)),
        sparks: effects.add(spark_effect()),
        smoke: effects.add(smoke_effect()),
        light_fire: effects.add(fire_effect(FIRE_PARTICLES * LIGHT_SHARE)),
        light_ejecta: effects.add(ejecta_effect(EJECTA_PARTICLES * LIGHT_SHARE)),
    });
}

/// Частицы огня: разлетаются из центра, тормозят, поднимаются и темнеют.
fn fire_effect(particles: f32) -> EffectAsset {
    let writer = ExprWriter::new();
    let power = writer.add_property("power", Value::Scalar(1.0.into()));

    let init_pos = SetPositionSphereModifier {
        center: writer.lit(Vec3::ZERO).expr(),
        radius: (writer.lit(0.8) * writer.prop(power)).expr(),
        dimension: ShapeDimension::Volume,
    };
    let init_vel = SetVelocitySphereModifier {
        center: writer.lit(Vec3::ZERO).expr(),
        speed: (writer.lit(5.0).uniform(writer.lit(12.0)) * writer.prop(power)).expr(),
    };
    let init_lifetime = SetAttributeModifier::new(
        Attribute::LIFETIME,
        writer.lit(0.35).uniform(writer.lit(0.7)).expr(),
    );
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let update_drag = LinearDragModifier::new(writer.lit(5.0).expr());
    let update_rise = AccelModifier::new(writer.lit(Vec3::Y * 4.0).expr());
    // Без текстуры частица — квадрат, скругляем до круга.
    let round = RoundModifier {
        roundness: writer.lit(1.0).expr(),
    };

    let mut color = bevy_hanabi::Gradient::new();
    color.add_key(0.0, Vec4::new(1.0, 0.95, 0.7, 1.0));
    color.add_key(0.3, Vec4::new(1.0, 0.55, 0.12, 0.9));
    color.add_key(0.7, Vec4::new(0.6, 0.18, 0.04, 0.5));
    color.add_key(1.0, Vec4::new(0.1, 0.05, 0.02, 0.0));

    let mut size = bevy_hanabi::Gradient::new();
    size.add_key(0.0, Vec3::splat(0.6));
    size.add_key(0.4, Vec3::splat(1.5));
    size.add_key(1.0, Vec3::splat(0.5));

    EffectAsset::new(
        256,
        SpawnerSettings::once(particles.into()),
        writer.finish(),
    )
    .with_name("explosion_fire")
    .with_alpha_mode(bevy_hanabi::AlphaMode::Add)
    .init(init_pos)
    .init(init_vel)
    .init(init_lifetime)
    .init(init_age)
    .update(update_drag)
    .update(update_rise)
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

/// Частицы пыли ударной волны: разлетаются по кругу у земли, тормозят и оседают.
fn dust_effect() -> EffectAsset {
    let writer = ExprWriter::new();
    let power = writer.add_property("power", Value::Scalar(1.0.into()));

    let init_pos = SetPositionCircleModifier {
        center: writer.lit(Vec3::Y * 0.3).expr(),
        axis: writer.lit(Vec3::Y).expr(),
        radius: (writer.lit(1.0) * writer.prop(power)).expr(),
        dimension: ShapeDimension::Volume,
    };
    let init_vel = SetVelocityCircleModifier {
        center: writer.lit(Vec3::ZERO).expr(),
        axis: writer.lit(Vec3::Y).expr(),
        speed: (writer.lit(16.0).uniform(writer.lit(28.0)) * writer.prop(power)).expr(),
    };
    let init_lifetime = SetAttributeModifier::new(
        Attribute::LIFETIME,
        writer.lit(0.6).uniform(writer.lit(1.1)).expr(),
    );
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let update_drag = LinearDragModifier::new(writer.lit(4.0).expr());
    let update_rise = AccelModifier::new(writer.lit(Vec3::Y * 1.5).expr());
    let round = RoundModifier {
        roundness: writer.lit(1.0).expr(),
    };

    let mut color = bevy_hanabi::Gradient::new();
    color.add_key(0.0, Vec4::new(0.75, 0.66, 0.52, 0.0));
    color.add_key(0.1, Vec4::new(0.75, 0.66, 0.52, 0.6));
    color.add_key(0.6, Vec4::new(0.6, 0.53, 0.43, 0.35));
    color.add_key(1.0, Vec4::new(0.5, 0.45, 0.38, 0.0));

    let mut size = bevy_hanabi::Gradient::new();
    size.add_key(0.0, Vec3::splat(0.6));
    size.add_key(1.0, Vec3::splat(2.4));

    EffectAsset::new(
        256,
        SpawnerSettings::once(DUST_PARTICLES.into()),
        writer.finish(),
    )
    .with_name("explosion_dust")
    .init(init_pos)
    .init(init_vel)
    .init(init_lifetime)
    .init(init_age)
    .update(update_drag)
    .update(update_rise)
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

/// Выброшенный грунт: летит вверх конусом, падает по гравитации, бурый.
fn ejecta_effect(particles: f32) -> EffectAsset {
    let writer = ExprWriter::new();
    let power = writer.add_property("power", Value::Scalar(1.0.into()));

    let init_pos = SetPositionSphereModifier {
        center: writer.lit(Vec3::Y * 0.3).expr(),
        radius: (writer.lit(0.8) * writer.prop(power)).expr(),
        dimension: ShapeDimension::Volume,
    };
    // Скорость вверх и вбок: случайный вектор в верхней полусфере, вытянутый вверх.
    let side = writer.rand(VectorType::VEC3F) * writer.lit(Vec3::new(2.0, 0.0, 2.0))
        - writer.lit(Vec3::new(1.0, 0.0, 1.0));
    let up = writer.lit(Vec3::Y) * writer.lit(1.2).uniform(writer.lit(2.2));
    let speed = writer.lit(7.0).uniform(writer.lit(13.0)) * writer.prop(power);
    let init_vel = SetAttributeModifier::new(Attribute::VELOCITY, ((side + up) * speed).expr());
    let init_lifetime = SetAttributeModifier::new(
        Attribute::LIFETIME,
        writer.lit(0.9).uniform(writer.lit(1.5)).expr(),
    );
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let update_gravity = AccelModifier::new(writer.lit(Vec3::Y * -30.0).expr());
    let update_drag = LinearDragModifier::new(writer.lit(0.6).expr());

    let mut color = bevy_hanabi::Gradient::new();
    color.add_key(0.0, Vec4::new(0.42, 0.3, 0.19, 1.0));
    color.add_key(0.8, Vec4::new(0.36, 0.26, 0.17, 0.9));
    color.add_key(1.0, Vec4::new(0.36, 0.26, 0.17, 0.0));

    let mut size = bevy_hanabi::Gradient::new();
    size.add_key(0.0, Vec3::splat(0.35));
    size.add_key(1.0, Vec3::splat(0.2));

    EffectAsset::new(
        512,
        SpawnerSettings::once(particles.into()),
        writer.finish(),
    )
    .with_name("explosion_ejecta")
    .init(init_pos)
    .init(init_vel)
    .init(init_lifetime)
    .init(init_age)
    .update(update_gravity)
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

/// Искры: быстрые, яркие, короткие, вытянуты по скорости.
fn spark_effect() -> EffectAsset {
    let writer = ExprWriter::new();
    let power = writer.add_property("power", Value::Scalar(1.0.into()));

    let init_pos = SetPositionSphereModifier {
        center: writer.lit(Vec3::Y * 0.5).expr(),
        radius: writer.lit(0.4).expr(),
        dimension: ShapeDimension::Volume,
    };
    let init_vel = SetVelocitySphereModifier {
        center: writer.lit(Vec3::Y * -0.6).expr(),
        speed: (writer.lit(14.0).uniform(writer.lit(30.0)) * writer.prop(power)).expr(),
    };
    let init_lifetime = SetAttributeModifier::new(
        Attribute::LIFETIME,
        writer.lit(0.25).uniform(writer.lit(0.6)).expr(),
    );
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let update_gravity = AccelModifier::new(writer.lit(Vec3::Y * -25.0).expr());
    let update_drag = LinearDragModifier::new(writer.lit(2.0).expr());

    let mut color = bevy_hanabi::Gradient::new();
    color.add_key(0.0, Vec4::new(4.0, 3.2, 1.6, 1.0));
    color.add_key(0.6, Vec4::new(2.5, 1.0, 0.3, 1.0));
    color.add_key(1.0, Vec4::new(1.0, 0.25, 0.05, 0.0));

    EffectAsset::new(
        256,
        SpawnerSettings::once(SPARK_PARTICLES.into()),
        writer.finish(),
    )
    .with_name("explosion_sparks")
    .with_alpha_mode(bevy_hanabi::AlphaMode::Add)
    .init(init_pos)
    .init(init_vel)
    .init(init_lifetime)
    .init(init_age)
    .update(update_gravity)
    .update(update_drag)
    .render(OrientModifier::new(OrientMode::AlongVelocity))
    .render(SetSizeModifier {
        size: Vec3::new(0.6, 0.08, 1.0).into(),
    })
    .render(ColorOverLifetimeModifier {
        gradient: color,
        blend: ColorBlendMode::Overwrite,
        mask: ColorBlendMask::RGBA,
    })
}

/// Дым: клубы поднимаются, расширяются, сносятся ветром и тают за несколько секунд.
fn smoke_effect() -> EffectAsset {
    let writer = ExprWriter::new();
    let power = writer.add_property("power", Value::Scalar(1.0.into()));

    let init_pos = SetPositionSphereModifier {
        center: writer.lit(Vec3::Y * 1.0).expr(),
        radius: (writer.lit(1.5) * writer.prop(power)).expr(),
        dimension: ShapeDimension::Volume,
    };
    let init_vel = SetVelocitySphereModifier {
        center: writer.lit(Vec3::Y * -1.0).expr(),
        speed: (writer.lit(1.5).uniform(writer.lit(4.0)) * writer.prop(power)).expr(),
    };
    let init_lifetime = SetAttributeModifier::new(
        Attribute::LIFETIME,
        writer.lit(2.5).uniform(writer.lit(4.0)).expr(),
    );
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    // Подъём и ветер в одну сторону: дым сносит, а не стоит столбом.
    let update_wind = AccelModifier::new(writer.lit(Vec3::new(1.2, 1.0, -0.5)).expr());
    let update_drag = LinearDragModifier::new(writer.lit(1.2).expr());
    let round = RoundModifier {
        roundness: writer.lit(1.0).expr(),
    };

    let mut color = bevy_hanabi::Gradient::new();
    color.add_key(0.0, Vec4::new(0.2, 0.18, 0.16, 0.0));
    color.add_key(0.12, Vec4::new(0.22, 0.2, 0.18, 0.55));
    color.add_key(0.5, Vec4::new(0.35, 0.33, 0.31, 0.35));
    color.add_key(1.0, Vec4::new(0.5, 0.48, 0.46, 0.0));

    let mut size = bevy_hanabi::Gradient::new();
    size.add_key(0.0, Vec3::splat(1.2));
    size.add_key(1.0, Vec3::splat(5.0));

    EffectAsset::new(
        128,
        SpawnerSettings::once(SMOKE_PARTICLES.into()),
        writer.finish(),
    )
    .with_name("explosion_smoke")
    .init(init_pos)
    .init(init_vel)
    .init(init_lifetime)
    .init(init_age)
    .update(update_wind)
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

/// Неосвещённый полупрозрачный материал, видимый с обеих сторон.
fn unlit_material(color: Color) -> StandardMaterial {
    StandardMaterial {
        base_color: color,
        unlit: true,
        alpha_mode: bevy::prelude::AlphaMode::Blend,
        cull_mode: None::<Face>,
        ..default()
    }
}

/// Создаёт корень взрыва со слоями на каждое сообщение `Explosion`: подробность по [`fx_tier`].
#[allow(clippy::too_many_arguments)]
fn spawn_explosions(
    mut commands: Commands,
    time: Res<Time>,
    focus: Res<Focus>,
    assets: Res<ExplosionFxAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut explosions: MessageReader<Explosion>,
    mut debris: MessageWriter<SpawnDebris>,
    roots: Query<&ExplosionFx>,
    flashes: Query<(), With<FlashLayer>>,
) {
    let mut alive = roots.iter().count();
    let mut full_alive = roots.iter().filter(|fx| fx.full).count();
    let mut flashes = flashes.iter().count();
    // Сколько вспышек света может гореть одновременно — из профиля платформы: на телефоне меньше,
    // точечный свет — расчёт для каждого пикселя рядом.
    let max_flashes = crate::profile::get().lights.flashes;
    for (index, e) in explosions.read().enumerate() {
        let seed = explosion_seed(e.position, time.elapsed_secs() + index as f32);
        let mut rng = Rng::new(seed);
        let power = e.power.max(0.05);
        let scale = power.sqrt();

        // Комья и осколки — часть мира, а не картинки: ложатся и вдали от камеры.
        throw_debris(&mut debris, &mut rng, e.position, power, scale);

        let tier = fx_tier(power, e.position.distance(focus.0), full_alive, alive);
        if tier == FxTier::Hidden {
            continue;
        }
        let full = tier == FxTier::Full;
        alive += 1;
        full_alive += usize::from(full);
        let root = commands
            .spawn((
                ExplosionFx {
                    age: 0.0,
                    life: if full { FX_LIFETIME } else { LIGHT_FX_LIFETIME },
                    full,
                    center: e.position,
                    power,
                },
                Transform::from_translation(e.position),
                Visibility::default(),
                RunScoped,
            ))
            .id();

        // Вспышка.
        if tier != FxTier::Minimal && flashes < max_flashes {
            flashes += 1;
            commands.spawn((
                FlashLayer,
                PointLight {
                    color: FLASH_COLOR,
                    intensity: flash_intensity(0.0, power),
                    range: e.radius * FLASH_RANGE * scale,
                    shadow_maps_enabled: false,
                    ..default()
                },
                Transform::from_xyz(0.0, FLASH_HEIGHT * scale, 0.0),
                ChildOf(root),
            ));
        }

        // Огненный шар из нескольких сфер с общим материалом.
        let fireball = materials.add(unlit_material(Color::srgba(1.0, 0.97, 0.85, 1.0)));
        let max_radius = e.radius * FIREBALL_SIZE * scale;
        let blobs = rng.range(FIREBALL_BLOBS.0 as f32, FIREBALL_BLOBS.1 as f32 + 0.999) as u32;
        for i in 0..blobs {
            let (offset, size, delay) = if i == 0 {
                // Главная сфера в центре, чуть приплюснутая.
                let r = max_radius * rng.range(0.85, 1.1);
                (Vec3::ZERO, Vec3::new(r, r * rng.range(0.75, 0.95), r), 0.0)
            } else {
                let angle = rng.range(0.0, std::f32::consts::TAU);
                let dist = max_radius * rng.range(0.4, 0.75);
                let offset = Vec3::new(
                    angle.cos() * dist,
                    max_radius * rng.range(0.0, 0.5),
                    angle.sin() * dist,
                );
                let r = max_radius * rng.range(0.45, 0.7);
                (offset, Vec3::splat(r), rng.range(0.0, 0.08))
            };
            commands.spawn((
                FireballBlob {
                    offset,
                    size,
                    delay,
                },
                Mesh3d(assets.sphere.clone()),
                MeshMaterial3d(fireball.clone()),
                Transform::from_scale(Vec3::splat(0.001)),
                bevy::light::NotShadowCaster,
                ChildOf(root),
            ));
        }

        // Огонь и пыль частицами, у каждого взрыва свой сид разброса.
        let properties = EffectProperties::default()
            .with_properties([("power".to_string(), Value::Scalar(scale.into()))]);
        let effects: &[(&Handle<EffectAsset>, u32)] = match tier {
            FxTier::Full => &[
                (&assets.fire, 1u32),
                (&assets.dust, 2),
                (&assets.ejecta, 3),
                (&assets.sparks, 4),
                (&assets.smoke, 5),
            ],
            FxTier::Light => &[(&assets.light_fire, 1), (&assets.light_ejecta, 3)],
            FxTier::Minimal | FxTier::Hidden => &[],
        };
        for &(effect, salt) in effects {
            commands.spawn((
                ParticleEffect {
                    prng_seed: Some(seed.wrapping_add(salt)),
                    ..ParticleEffect::new(effect.clone())
                },
                properties.clone(),
                Transform::default(),
                ChildOf(root),
            ));
        }

        if !full {
            continue;
        }
        // Кольцо ударной волны. Меш свой у каждого взрыва: вершины кладутся на рельеф.
        commands.spawn((
            ShockwaveLayer {
                reach: e.radius * SHOCK_REACH * scale * rng.range(0.9, 1.1),
            },
            Mesh3d(meshes.add(ring_mesh())),
            MeshMaterial3d(materials.add(unlit_material(Color::srgba(
                SHOCK_COLOR[0],
                SHOCK_COLOR[1],
                SHOCK_COLOR[2],
                SHOCK_ALPHA,
            )))),
            Transform::default(),
            bevy::light::NotShadowCaster,
            // Вершины меняются каждый кадр, а границы меша нет.
            NoFrustumCulling,
            ChildOf(root),
        ));
    }
}

/// Комья грунта и горящие осколки: настоящие обломки, ложатся на рельеф.
fn throw_debris(debris: &mut MessageWriter<SpawnDebris>, rng: &mut Rng, center: Vec3, power: f32, scale: f32) {
    if power >= DEBRIS_MIN_POWER {
        let throw = |rng: &mut Rng, up: (f32, f32), side: (f32, f32)| {
            let angle = rng.range(0.0, std::f32::consts::TAU);
            let side = rng.range(side.0, side.1) * scale;
            Vec3::new(angle.cos() * side, rng.range(up.0, up.1) * scale, angle.sin() * side)
        };
        let clods = rng.range(CLODS.0 as f32, CLODS.1 as f32 + 0.999) as u32;
        for _ in 0..clods {
            let s = rng.range(0.25, 0.55) * scale;
            debris.write(SpawnDebris {
                position: center + Vec3::Y * 0.5,
                velocity: throw(rng, (8.0, 15.0), (3.0, 9.0)),
                size: Vec3::new(s, s * rng.range(0.6, 0.9), s * rng.range(0.7, 1.0)),
                color: CLOD_COLOR,
                burning: false,
                shape: DebrisShape::Form(FormKind::Clod),
                rotation: None,
            });
        }
        let shards = rng.range(BURNING_SHARDS.0 as f32, BURNING_SHARDS.1 as f32 + 0.999) as u32;
        for _ in 0..shards {
            let s = rng.range(0.15, 0.3) * scale;
            debris.write(SpawnDebris {
                position: center + Vec3::Y * 0.8,
                velocity: throw(rng, (10.0, 18.0), (4.0, 11.0)),
                size: Vec3::new(s, s * 0.5, s * 1.4),
                color: SHARD_COLOR,
                burning: true,
                shape: DebrisShape::Form(FormKind::Sheet),
                rotation: None,
            });
        }
    }
}

/// Пустое кольцо нужной топологии: вершины заполняет `animate_shockwave`.
fn ring_mesh() -> Mesh {
    let vertices = (SHOCK_SEGMENTS + 1) * 2;
    let mut indices = Vec::with_capacity(SHOCK_SEGMENTS * 6);
    for i in 0..SHOCK_SEGMENTS as u32 {
        let (inner, outer) = (i * 2, i * 2 + 1);
        let (next_inner, next_outer) = (inner + 2, outer + 2);
        indices.extend_from_slice(&[inner, outer, next_outer, inner, next_outer, next_inner]);
    }
    Mesh::new(PrimitiveTopology::TriangleList, default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; vertices])
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; vertices])
        .with_inserted_indices(Indices::U32(indices))
}

/// Старит взрывы и удаляет отыгравшие вместе со слоями.
fn age_explosions(
    mut commands: Commands,
    time: Res<Time>,
    mut roots: Query<(Entity, &mut ExplosionFx)>,
) {
    for (entity, mut fx) in &mut roots {
        fx.age += time.delta_secs();
        if fx.age >= fx.life {
            commands.entity(entity).despawn();
        }
    }
}

/// Гасит вспышку по экспоненте и удаляет погасший источник света, не дожидаясь корня.
fn animate_flash(
    mut commands: Commands,
    roots: Query<&ExplosionFx>,
    mut lights: Query<(Entity, &ChildOf, &mut PointLight), With<FlashLayer>>,
) {
    for (entity, parent, mut light) in &mut lights {
        if let Ok(fx) = roots.get(parent.parent()) {
            if fx.age >= FLASH_TIME {
                commands.entity(entity).despawn();
            } else {
                light.intensity = flash_intensity(fx.age, fx.power);
            }
        }
    }
}

/// Раздувает сферы огненного шара, поднимает его и меняет цвет к тёмному дыму.
fn animate_fireball(
    roots: Query<&ExplosionFx>,
    mut blobs: Query<(
        &ChildOf,
        &FireballBlob,
        &MeshMaterial3d<StandardMaterial>,
        &mut Transform,
        &mut Visibility,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (parent, blob, material, mut transform, mut visibility) in &mut blobs {
        let Ok(fx) = roots.get(parent.parent()) else {
            continue;
        };
        let t = ((fx.age - blob.delay) / FIREBALL_TIME).max(0.0);
        if t >= 1.0 {
            *visibility = Visibility::Hidden;
            continue;
        }
        let growth = fireball_growth(t);
        transform.translation = blob.offset * growth + Vec3::Y * FIREBALL_RISE * t;
        transform.scale = blob.size * growth;
        // У всех сфер одного взрыва материал общий, цвет по главной сфере.
        if blob.delay == 0.0
            && let Some(mut material) = materials.get_mut(&material.0)
        {
            let [r, g, b, a] = fireball_color(t);
            let color = Color::srgba(r, g, b, a).to_linear();
            let glow = fireball_glow(t);
            material.base_color = Color::linear_rgba(color.red * glow, color.green * glow, color.blue * glow, color.alpha);
        }
    }
}

/// Расширяет кольцо ударной волны, кладёт его на рельеф и растворяет.
fn animate_shockwave(
    terrain: Res<Terrain>,
    roots: Query<&ExplosionFx>,
    mut rings: Query<(
        &ChildOf,
        &ShockwaveLayer,
        &Mesh3d,
        &MeshMaterial3d<StandardMaterial>,
        &mut Visibility,
    )>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (parent, ring, mesh, material, mut visibility) in &mut rings {
        let Ok(fx) = roots.get(parent.parent()) else {
            continue;
        };
        let t = fx.age / SHOCK_TIME;
        if t >= 1.0 {
            *visibility = Visibility::Hidden;
            continue;
        }
        let (radius, width, alpha) = shockwave_shape(t, ring.reach);
        let inner = (radius - width * 0.5).max(0.0);
        let outer = radius + width * 0.5;

        if let Some(mut mesh) = meshes.get_mut(&mesh.0) {
            let center = fx.center;
            let positions: Vec<[f32; 3]> = (0..=SHOCK_SEGMENTS)
                .flat_map(|i| {
                    let dir =
                        Vec2::from_angle(i as f32 / SHOCK_SEGMENTS as f32 * std::f32::consts::TAU);
                    [inner, outer].map(|r| {
                        let p = dir * r;
                        let y = terrain.height_at(center.x + p.x, center.z + p.y) - center.y;
                        [p.x, y + SHOCK_LIFT, p.y]
                    })
                })
                .collect();
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        }
        if let Some(mut material) = materials.get_mut(&material.0) {
            material.base_color =
                Color::srgba(SHOCK_COLOR[0], SHOCK_COLOR[1], SHOCK_COLOR[2], alpha);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flash_fades_out_quickly() {
        let peak = flash_intensity(0.0, 1.0);
        assert!(flash_intensity(0.05, 1.0) < peak * 0.4);
        assert!(flash_intensity(0.14, 1.0) < peak * 0.05);
        assert_eq!(flash_intensity(FLASH_TIME, 1.0), 0.0);
        assert!(flash_intensity(0.0, 2.0) > peak);
    }

    #[test]
    fn fireball_grows_and_darkens() {
        let mut previous_size = 0.0;
        let mut previous_brightness = f32::MAX;
        for step in 0..=10 {
            let t = step as f32 / 10.0;
            let size = fireball_growth(t);
            let [r, g, b, _] = fireball_color(t);
            let brightness = r + g + b;
            assert!(size >= previous_size, "шар не должен сжиматься: t = {t}");
            assert!(
                brightness <= previous_brightness,
                "шар не должен светлеть: t = {t}"
            );
            previous_size = size;
            previous_brightness = brightness;
        }
        assert_eq!(fireball_color(0.0), FIREBALL_COLORS[0].1);
        assert_eq!(
            fireball_color(1.0)[3],
            0.0,
            "к концу шар полностью растворяется"
        );
        assert!((fireball_growth(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn shockwave_expands_and_fades() {
        let (r0, _, a0) = shockwave_shape(0.0, 12.0);
        let (r_mid, _, a_mid) = shockwave_shape(0.5, 12.0);
        let (r1, _, a1) = shockwave_shape(1.0, 12.0);
        assert_eq!(r0, 0.0);
        assert!(r_mid > 12.0 * 0.5, "волна быстро уходит вперёд");
        assert!((r1 - 12.0).abs() < 1e-5);
        assert!(a0 > a_mid && a_mid > a1);
        assert_eq!(a1, 0.0);
    }

    #[test]
    fn effects_are_limited_and_simplified() {
        assert_eq!(fx_tier(1.4, 10.0, 0, 0), FxTier::Full);
        assert_eq!(fx_tier(0.25, 10.0, 0, 0), FxTier::Light, "гибель врага — упрощённый");
        assert_eq!(fx_tier(1.4, FULL_RANGE + 1.0, 0, 0), FxTier::Light, "у края кадра — упрощённый");
        assert_eq!(fx_tier(1.4, 10.0, MAX_FULL_FX, MAX_FULL_FX), FxTier::Light);
        assert_eq!(fx_tier(1.4, 10.0, 0, MAX_FX), FxTier::Minimal);
        assert_eq!(fx_tier(1.4, FX_RANGE + 1.0, 0, 0), FxTier::Hidden);
    }

    #[test]
    fn explosions_in_same_place_get_different_seeds() {
        let p = Vec3::new(3.0, 1.0, -7.0);
        assert_ne!(explosion_seed(p, 10.0), explosion_seed(p, 11.0));
        let mut a = Rng::new(explosion_seed(p, 10.0));
        let mut b = Rng::new(explosion_seed(p, 11.0));
        assert_ne!(a.range(0.0, 1.0), b.range(0.0, 1.0));
    }
}
