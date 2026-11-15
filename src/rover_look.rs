//! Облик лунохода: корпус и два гусеничных блока из моделей Meshy и бегущие грунтозацепы.
//!
//! Модели берутся из списка `assets/models/models.json` по идентификаторам `rover_body`
//! и `rover_track`: файл, масштаб, поворот и высота заданы там. Корпус в модели смотрит на +Z,
//! а луноход едет на -Z, поэтому в списке у корпуса поворот на π. Гусеничный блок в модели лежит
//! вдоль X катками к -Z, поэтому каждый блок повёрнут здесь катками наружу. Пока модели грузятся,
//! и если их нет, луноход показан запасным обликом из примитивов.
//!
//! Гусеница в модели неподвижна, поэтому поверх неё по контуру «стадион» бегут грунтозацепы.
//! Путь каждого борта считается по фактическому движению лунохода: при езде вперёд верхняя
//! ветвь бежит вперёд, нижняя назад, а на повороте на месте борта бегут в разные стороны.
//!
//! Подвеска: каждый гусеничный блок садится на рельеф под своими концами — поднимается,
//! опускается и наклоняется в пределах хода. Из-под задних концов гусениц летит пыль, тем
//! больше, чем быстрее едет луноход. С потерей здоровья корпус покрывается копотью, а ниже
//! половины здоровья луноход дымит, тем гуще, чем меньше здоровья осталось. Частиц — по доле
//! ступени качества.

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, PI};

use bevy::prelude::*;
use bevy_hanabi::prelude::*;

use crate::combat::Health;
use crate::models::{FallbackSpawner, Models};
use crate::quality::Quality;
use crate::rover::{MAX_FORWARD_SPEED, RIDE_HEIGHT, ROVER_SCALE, Rover, RoverMotion};
use crate::terrain::Terrain;

/// Идентификаторы моделей корпуса и гусеничного блока в списке моделей.
const BODY_MODEL: &str = "rover_body";
const TRACK_MODEL: &str = "rover_track";

/// Полуширина колеи: центр гусеничного блока от оси лунохода, м.
pub const TRACK_X: f32 = 0.72;

/// Радиус закругления гусеницы по наружной поверхности ленты, м. Размеры ленты заданы игрой:
/// модель блока подгоняется к ним масштабом в списке моделей.
const TRACK_RADIUS: f32 = 0.344;

/// Половина длины прямого участка гусеницы, м.
const TRACK_HALF_STRAIGHT: f32 = 0.416;

/// Высота центра гусеничного блока над центром лунохода, м: лента касается земли.
const TRACK_Y: f32 = -RIDE_HEIGHT + TRACK_RADIUS;

/// Грунтозацепов на одну гусеницу.
const GROUSERS: usize = 18;

/// Размер грунтозацепа: поперёк ленты, толщина, вдоль ленты, м.
const GROUSER_SIZE: Vec3 = Vec3::new(0.28, 0.035, 0.07);

/// Цвет грунтозацепов: тёмная сталь, чуть светлее резины, чтобы бег ленты был заметен.
const GROUSER_COLOR: Color = Color::srgb(0.36, 0.38, 0.42);

/// Больше такого сдвига за кадр — это перенос лунохода (рестарт), а не езда, м.
const MAX_STEP: f32 = 3.0;

/// Запасной корпус: короб между гусеницами, палуба над ними и бампер на носу.
/// Размеры по X, Y, Z и высота центра над землёй, м.
const SPARE_HULL_SIZE: Vec3 = Vec3::new(1.0, 0.42, 1.5);
const SPARE_HULL_HEIGHT: f32 = 0.52;
const SPARE_DECK_SIZE: Vec3 = Vec3::new(1.3, 0.1, 1.36);
const SPARE_DECK_HEIGHT: f32 = 0.78;
const SPARE_BUMPER_SIZE: Vec3 = Vec3::new(0.7, 0.14, 0.08);

/// Толщина запасного гусеничного блока поперёк хода, м.
const SPARE_TRACK_WIDTH: f32 = 0.3;

/// Цвета запасного облика: бирюзовый корпус, белая палуба, тёмная резина.
const SPARE_HULL_COLOR: Color = Color::srgb(0.10, 0.62, 0.62);
const SPARE_DECK_COLOR: Color = Color::srgb(0.90, 0.90, 0.86);
const SPARE_RUBBER_COLOR: Color = Color::srgb(0.12, 0.12, 0.13);

/// Ход подвески гусеничного блока вверх и вниз, м.
const SUSPENSION_TRAVEL: f32 = 0.35;

/// Наибольший наклон гусеничного блока вдоль хода относительно корпуса, рад.
const SUSPENSION_PITCH: f32 = 0.35;

/// Скорость, с которой блок догоняет рельеф под собой, 1/с.
const SUSPENSION_SMOOTHING: f32 = 18.0;

/// Пыль: частиц в секунду с каждой гусеницы на полной скорости.
const DUST_RATE: f32 = 28.0;

/// Дым: частиц в секунду при нулевом здоровье.
const SMOKE_RATE: f32 = 14.0;

/// Ниже этой доли здоровья луноход дымит.
const SMOKE_BELOW: f32 = 0.5;

/// Копоть при нулевом здоровье: доля цвета копоти в цвете материалов.
const SOOT_MAX: f32 = 0.7;

/// Цвет копоти.
const SOOT_COLOR: Color = Color::srgb(0.07, 0.06, 0.05);

/// Копоть перекрашивает материалы, только когда её доля изменилась хотя бы на столько.
const SOOT_STEP: f32 = 0.02;

/// Цикл эмиттеров пыли и дыма, с: новая густота вступает в силу не позже чем через него.
const EMIT_PERIOD: f32 = 0.1;

/// Плагин облика лунохода.
pub struct RoverLookPlugin;

impl Plugin for RoverLookPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (attach_look, run_tracks, fit_suspension, emit_dust_and_smoke, apply_soot)
                .chain()
                .in_set(crate::frame::GameSet::Presentation),
        );
    }
}

/// Эмиттер пыли у заднего конца гусеницы.
#[derive(Component)]
struct DustEmitter;

/// Эмиттер дыма над корпусом.
#[derive(Component)]
struct SmokeEmitter;

/// Копоть на материалах лунохода: исходные цвета и последняя нанесённая доля.
#[derive(Default)]
struct Soot {
    base: HashMap<AssetId<StandardMaterial>, Color>,
    applied: f32,
}

/// Борт гусеницы: -1 левый, 1 правый.
#[derive(Component, Clone, Copy)]
struct TrackSide(f32);

/// Грунтозацеп: номер на ленте.
#[derive(Component)]
struct Grouser(usize);

/// Пройденный путь гусениц по бортам и прошлое положение лунохода.
#[derive(Component, Default)]
struct TrackDrive {
    /// Путь левой и правой ленты, м.
    travel: [f32; 2],
    /// Положение и направление «вперёд» в прошлом кадре.
    last: Option<(Vec3, Vec3)>,
}

/// Длина ленты по контуру, м.
pub fn track_length(half_straight: f32, radius: f32) -> f32 {
    4.0 * half_straight + 2.0 * PI * radius
}

/// Точка ленты на расстоянии `u` по контуру: координаты (z, y) от центра блока и наружная нормаль.
///
/// Отсчёт идёт от передней точки нижней ветви назад, затем вверх по заднему закруглению,
/// вперёд по верхней ветви и вниз по переднему закруглению. Перёд — это -Z.
pub fn track_point(u: f32, half_straight: f32, radius: f32) -> (Vec2, Vec2) {
    let (a, r) = (half_straight, radius);
    let u = u.rem_euclid(track_length(a, r));
    let arc = PI * r;
    if u < 2.0 * a {
        (Vec2::new(-a + u, -r), Vec2::new(0.0, -1.0))
    } else if u < 2.0 * a + arc {
        let t = (u - 2.0 * a) / r;
        let n = Vec2::new(t.sin(), -t.cos());
        (Vec2::new(a, 0.0) + n * r, n)
    } else if u < 4.0 * a + arc {
        (Vec2::new(a - (u - 2.0 * a - arc), r), Vec2::new(0.0, 1.0))
    } else {
        let t = (u - 4.0 * a - arc) / r;
        let n = Vec2::new(-t.sin(), t.cos());
        (Vec2::new(-a, 0.0) + n * r, n)
    }
}

/// Сколько пробегают ленты левого и правого борта, когда луноход сместился на `step` вперёд
/// и повернулся на `turn` радиан влево. Полуширина колеи — `half_gauge`.
pub fn track_travel(step: f32, turn: f32, half_gauge: f32) -> [f32; 2] {
    [step - turn * half_gauge, step + turn * half_gauge]
}

/// Меши и материалы запасного облика лунохода.
struct SpareLook {
    hull: Handle<Mesh>,
    deck: Handle<Mesh>,
    bumper: Handle<Mesh>,
    /// Прямой участок гусеничного блока.
    track_box: Handle<Mesh>,
    /// Закругление блока: цилиндр с осью поперёк хода.
    track_end: Handle<Mesh>,
    hull_material: Handle<StandardMaterial>,
    deck_material: Handle<StandardMaterial>,
    rubber_material: Handle<StandardMaterial>,
}

impl SpareLook {
    fn new(meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) -> Self {
        let mut matte = |color: Color| {
            materials.add(StandardMaterial {
                base_color: color,
                perceptual_roughness: 0.7,
                ..default()
            })
        };
        Self {
            hull: meshes.add(Cuboid::from_size(SPARE_HULL_SIZE)),
            deck: meshes.add(Cuboid::from_size(SPARE_DECK_SIZE)),
            bumper: meshes.add(Cuboid::from_size(SPARE_BUMPER_SIZE)),
            track_box: meshes.add(Cuboid::new(
                SPARE_TRACK_WIDTH,
                TRACK_RADIUS * 2.0,
                TRACK_HALF_STRAIGHT * 2.0,
            )),
            track_end: meshes.add(Cylinder::new(TRACK_RADIUS, SPARE_TRACK_WIDTH)),
            hull_material: matte(SPARE_HULL_COLOR),
            deck_material: matte(SPARE_DECK_COLOR),
            rubber_material: matte(SPARE_RUBBER_COLOR),
        }
    }

    /// Корпус в системе лунохода: центр лунохода на [`RIDE_HEIGHT`] выше земли, нос на -Z.
    fn body(&self, parts: &mut FallbackSpawner) {
        parts.spawn((
            Mesh3d(self.hull.clone()),
            MeshMaterial3d(self.hull_material.clone()),
            Transform::from_xyz(0.0, SPARE_HULL_HEIGHT - RIDE_HEIGHT, 0.0),
        ));
        parts.spawn((
            Mesh3d(self.deck.clone()),
            MeshMaterial3d(self.deck_material.clone()),
            Transform::from_xyz(0.0, SPARE_DECK_HEIGHT - RIDE_HEIGHT, 0.0),
        ));
        parts.spawn((
            Mesh3d(self.bumper.clone()),
            MeshMaterial3d(self.rubber_material.clone()),
            Transform::from_xyz(0.0, SPARE_HULL_HEIGHT - RIDE_HEIGHT, -SPARE_HULL_SIZE.z * 0.5),
        ));
    }

    /// Гусеничный блок в системе своей рамы: контур «стадион», по которому бегут грунтозацепы.
    fn track(&self, parts: &mut FallbackSpawner) {
        parts.spawn((
            Mesh3d(self.track_box.clone()),
            MeshMaterial3d(self.rubber_material.clone()),
            Transform::default(),
        ));
        for end in [-1.0f32, 1.0] {
            parts.spawn((
                Mesh3d(self.track_end.clone()),
                MeshMaterial3d(self.rubber_material.clone()),
                // Ось цилиндра — Y: кладём её поперёк хода, вдоль X.
                Transform::from_xyz(0.0, 0.0, end * TRACK_HALF_STRAIGHT)
                    .with_rotation(Quat::from_rotation_z(FRAC_PI_2)),
            ));
        }
    }
}

/// Ставит на новый луноход корпус, гусеничные блоки и грунтозацепы.
fn attach_look(
    mut commands: Commands,
    rovers: Query<Entity, Added<Rover>>,
    mut models: Models,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut effects: ResMut<Assets<EffectAsset>>,
) {
    for rover in &rovers {
        let dust = effects.add(dust_effect());
        let smoke = effects.add(smoke_effect());
        let spare = SpareLook::new(&mut meshes, &mut materials);
        let grouser_mesh = meshes.add(Cuboid::from_size(GROUSER_SIZE));
        let grouser_material = materials.add(StandardMaterial {
            base_color: GROUSER_COLOR,
            metallic: 0.3,
            perceptual_roughness: 0.6,
            ..default()
        });

        commands
            .entity(rover)
            .insert(TrackDrive::default())
            .with_children(|parent| {
                // Низ лунохода — на высоту посадки ниже его центра.
                let ground = Transform::from_xyz(0.0, -RIDE_HEIGHT, 0.0);
                models.spawn_visual(parent, BODY_MODEL, ground, |parts| spare.body(parts));
                for side in [-1.0f32, 1.0] {
                    parent
                        .spawn((
                            TrackSide(side),
                            Transform::from_xyz(side * TRACK_X, TRACK_Y, 0.0),
                            Visibility::default(),
                        ))
                        .with_children(|frame| {
                            // Низ блока — на радиус ленты ниже центра рамы. Катки модели смотрят
                            // на -Z: поворот выводит их наружу.
                            let ground = Transform::from_xyz(0.0, -TRACK_RADIUS, 0.0)
                                .with_rotation(Quat::from_rotation_y(-side * FRAC_PI_2));
                            models.spawn_visual(frame, TRACK_MODEL, ground, |parts| spare.track(parts));
                            for i in 0..GROUSERS {
                                frame.spawn((
                                    Grouser(i),
                                    Mesh3d(grouser_mesh.clone()),
                                    MeshMaterial3d(grouser_material.clone()),
                                    Transform::default(),
                                ));
                            }
                        });
                    // Пыль у заднего конца гусеницы, у самой земли.
                    parent.spawn((
                        DustEmitter,
                        emitter(&dust),
                        Transform::from_xyz(side * TRACK_X, -RIDE_HEIGHT + 0.1, TRACK_HALF_STRAIGHT + TRACK_RADIUS),
                    ));
                }
                parent.spawn((SmokeEmitter, emitter(&smoke), Transform::from_xyz(0.0, 0.5, 0.2)));
            });
    }
}

/// Эмиттер постоянного эффекта: густоту задаёт `emit_dust_and_smoke`, сначала он молчит.
fn emitter(effect: &Handle<EffectAsset>) -> impl Bundle {
    let settings = SpawnerSettings::new(0.0.into(), EMIT_PERIOD.into(), EMIT_PERIOD.into(), 0);
    (ParticleEffect::new(effect.clone()), EffectSpawner::new(&settings))
}

/// Пыль из-под гусениц: светлые клубы, поднимаются и расходятся.
pub(crate) fn dust_effect() -> EffectAsset {
    let writer = ExprWriter::new();
    let init_pos = SetPositionSphereModifier {
        center: writer.lit(Vec3::ZERO).expr(),
        radius: writer.lit(0.2).expr(),
        dimension: ShapeDimension::Volume,
    };
    let init_vel = SetVelocitySphereModifier {
        center: writer.lit(Vec3::Y * -0.5).expr(),
        speed: writer.lit(0.6).uniform(writer.lit(1.6)).expr(),
    };
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, writer.lit(0.7).uniform(writer.lit(1.3)).expr());
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let update_rise = AccelModifier::new(writer.lit(Vec3::Y * 0.6).expr());
    let update_drag = LinearDragModifier::new(writer.lit(2.0).expr());
    let round = RoundModifier {
        roundness: writer.lit(1.0).expr(),
    };

    let mut color = bevy_hanabi::Gradient::new();
    color.add_key(0.0, Vec4::new(0.62, 0.55, 0.43, 0.0));
    color.add_key(0.15, Vec4::new(0.62, 0.55, 0.43, 0.45));
    color.add_key(1.0, Vec4::new(0.55, 0.5, 0.42, 0.0));
    let mut size = bevy_hanabi::Gradient::new();
    size.add_key(0.0, Vec3::splat(0.35));
    size.add_key(1.0, Vec3::splat(1.6));

    EffectAsset::new(64, SpawnerSettings::rate(0.0.into()), writer.finish())
        .with_name("rover_dust")
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

/// Дым повреждённого лунохода: тёмные клубы, поднимаются и сносятся ветром.
fn smoke_effect() -> EffectAsset {
    let writer = ExprWriter::new();
    let init_pos = SetPositionSphereModifier {
        center: writer.lit(Vec3::ZERO).expr(),
        radius: writer.lit(0.3).expr(),
        dimension: ShapeDimension::Volume,
    };
    let init_vel = SetVelocitySphereModifier {
        center: writer.lit(Vec3::Y * -1.0).expr(),
        speed: writer.lit(0.8).uniform(writer.lit(1.8)).expr(),
    };
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, writer.lit(1.8).uniform(writer.lit(2.8)).expr());
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    // Ветер в ту же сторону, что у дыма взрывов.
    let update_wind = AccelModifier::new(writer.lit(Vec3::new(0.8, 1.6, -0.4)).expr());
    let update_drag = LinearDragModifier::new(writer.lit(1.0).expr());
    let round = RoundModifier {
        roundness: writer.lit(1.0).expr(),
    };

    let mut color = bevy_hanabi::Gradient::new();
    color.add_key(0.0, Vec4::new(0.12, 0.11, 0.1, 0.0));
    color.add_key(0.1, Vec4::new(0.12, 0.11, 0.1, 0.6));
    color.add_key(0.6, Vec4::new(0.28, 0.27, 0.26, 0.35));
    color.add_key(1.0, Vec4::new(0.4, 0.4, 0.4, 0.0));
    let mut size = bevy_hanabi::Gradient::new();
    size.add_key(0.0, Vec3::splat(0.5));
    size.add_key(1.0, Vec3::splat(2.6));

    EffectAsset::new(64, SpawnerSettings::rate(0.0.into()), writer.finish())
        .with_name("rover_smoke")
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

/// Густота пыли, частиц в секунду с гусеницы: по доле от полной скорости.
pub fn dust_rate(speed: f32) -> f32 {
    DUST_RATE * (speed.abs() / MAX_FORWARD_SPEED).min(1.0)
}

/// Густота дыма, частиц в секунду: ноль выше [`SMOKE_BELOW`] здоровья, растёт к нулю здоровья.
pub fn smoke_rate(health: f32) -> f32 {
    SMOKE_RATE * ((SMOKE_BELOW - health) / SMOKE_BELOW).clamp(0.0, 1.0)
}

/// Доля копоти по доле здоровья: целый луноход чистый.
pub fn soot_share(health: f32) -> f32 {
    SOOT_MAX * (1.0 - health.clamp(0.0, 1.0))
}

/// Сдвиг и наклон гусеничного блока по высотам земли под его передним и задним концом,
/// отсчитанным от земли, на которой стоит корпус, м. Расстояние между концами — `length`.
pub fn suspension_pose(front: f32, back: f32, length: f32) -> (f32, f32) {
    let lift = ((front + back) * 0.5).clamp(-SUSPENSION_TRAVEL, SUSPENSION_TRAVEL);
    let pitch = (front - back).atan2(length).clamp(-SUSPENSION_PITCH, SUSPENSION_PITCH);
    (lift, pitch)
}

/// Подвеска: гусеничный блок поднимается, опускается и наклоняется по рельефу под концами.
fn fit_suspension(
    time: Res<Time>,
    terrain: Res<Terrain>,
    rover: Query<&Transform, With<Rover>>,
    mut frames: Query<(&TrackSide, &mut Transform), Without<Rover>>,
) {
    let Some(rover) = rover.iter().next() else {
        return;
    };
    let to_local = rover.compute_affine().inverse();
    let reach = TRACK_HALF_STRAIGHT + TRACK_RADIUS * 0.5;
    let blend = 1.0 - (-SUSPENSION_SMOOTHING * time.delta_secs()).exp();
    for (side, mut frame) in &mut frames {
        // Высота земли под концом блока над землёй корпуса, в метрах мира.
        let ground = |z: f32| {
            let world = rover.transform_point(Vec3::new(side.0 * TRACK_X, -RIDE_HEIGHT, z));
            let below = Vec3::new(world.x, terrain.height_at(world.x, world.z), world.z);
            (to_local.transform_point3(below).y + RIDE_HEIGHT) * ROVER_SCALE
        };
        let (lift, pitch) = suspension_pose(ground(-reach), ground(reach), 2.0 * reach * ROVER_SCALE);
        let wanted = Transform::from_xyz(side.0 * TRACK_X, TRACK_Y + lift / ROVER_SCALE, 0.0)
            .with_rotation(Quat::from_rotation_x(pitch));
        frame.translation = frame.translation.lerp(wanted.translation, blend);
        frame.rotation = frame.rotation.slerp(wanted.rotation, blend);
    }
}

/// Густота пыли по скорости и дыма по здоровью с долей частиц ступени качества. На лаве
/// и на ступени «Низкая» пыли нет.
fn emit_dust_and_smoke(
    quality: Res<Quality>,
    pools: Res<crate::lava::LavaPools>,
    rover: Query<(&RoverMotion, &Health, &Transform), With<Rover>>,
    mut dust: Query<&mut EffectSpawner, (With<DustEmitter>, Without<SmokeEmitter>)>,
    mut smoke: Query<&mut EffectSpawner, (With<SmokeEmitter>, Without<DustEmitter>)>,
) {
    let Some((motion, health, transform)) = rover.iter().next() else {
        return;
    };
    let share = quality.particle_share();
    let dusty = *quality != Quality::Low && !crate::lava::is_lava(&pools, transform.translation);
    let set = |spawner: &mut EffectSpawner, rate: f32| {
        let count = rate * share * EMIT_PERIOD;
        if spawner.settings.count() != count.into() {
            spawner.settings.set_count(count.into());
        }
    };
    for mut spawner in &mut dust {
        set(&mut spawner, if dusty { dust_rate(motion.speed) } else { 0.0 });
    }
    for mut spawner in &mut smoke {
        set(&mut spawner, smoke_rate(health.fraction()));
    }
}

/// Копоть: материалы корпуса и гусениц темнеют по доле потерянного здоровья. Материалы
/// модели появляются, когда она загрузится, поэтому новые подхватываются по ходу.
fn apply_soot(
    rover: Query<(Entity, &Health), With<Rover>>,
    children: Query<&Children>,
    meshes: Query<&MeshMaterial3d<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut soot: Local<Soot>,
) {
    let Some((rover, health)) = rover.iter().next() else {
        return;
    };
    let mut found = false;
    for entity in children.iter_descendants(rover) {
        let Ok(material) = meshes.get(entity) else {
            continue;
        };
        let id = material.id();
        if !soot.base.contains_key(&id)
            && let Some(loaded) = materials.get(id)
        {
            soot.base.insert(id, loaded.base_color);
            found = true;
        }
    }
    let share = soot_share(health.fraction());
    if !found && (share - soot.applied).abs() < SOOT_STEP && (share > 0.0 || soot.applied == 0.0) {
        return;
    }
    soot.applied = share;
    for (id, base) in &soot.base {
        if let Some(mut material) = materials.get_mut(*id) {
            material.base_color = base.mix(&SOOT_COLOR, share);
        }
    }
}

/// Считает путь лент по движению лунохода и раскладывает грунтозацепы по контуру.
fn run_tracks(
    mut rover: Query<(&Transform, &mut TrackDrive), With<Rover>>,
    sides: Query<&TrackSide>,
    mut grousers: Query<(&ChildOf, &Grouser, &mut Transform), Without<Rover>>,
) {
    let Some((transform, mut drive)) = rover.iter_mut().next() else {
        return;
    };
    let position = transform.translation;
    let forward = transform.forward().as_vec3();
    if let Some((last_position, last_forward)) = drive.last {
        let step = (position - last_position).dot(forward);
        let flat = |v: Vec3| Vec2::new(v.x, v.z).normalize_or_zero();
        // Поворот влево — против часовой стрелки сверху: курс растёт.
        let turn = -flat(last_forward).angle_to(flat(forward));
        if step.abs() < MAX_STEP {
            let travel = track_travel(step, turn, TRACK_X);
            drive.travel[0] += travel[0];
            drive.travel[1] += travel[1];
        }
    }
    drive.last = Some((position, forward));

    let length = track_length(TRACK_HALF_STRAIGHT, TRACK_RADIUS);
    let spacing = length / GROUSERS as f32;
    for (parent, grouser, mut transform) in &mut grousers {
        let Ok(side) = sides.get(parent.parent()) else {
            continue;
        };
        let travel = drive.travel[if side.0 < 0.0 { 0 } else { 1 }];
        let u = grouser.0 as f32 * spacing + travel;
        let (point, normal) = track_point(u, TRACK_HALF_STRAIGHT, TRACK_RADIUS);
        let normal = Vec3::new(0.0, normal.y, normal.x);
        transform.translation = Vec3::new(0.0, point.y, point.x) + normal * GROUSER_SIZE.y * 0.5;
        transform.rotation = Quat::from_rotation_arc(Vec3::Y, normal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: f32 = 0.4;
    const R: f32 = 0.3;

    fn close(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-3
    }

    #[test]
    fn track_contour_is_closed_and_continuous() {
        let length = track_length(A, R);
        // Начало и конец контура совпадают.
        assert!(close(track_point(0.0, A, R).0, track_point(length, A, R).0));
        // Стыки участков без разрывов: точки по обе стороны стыка рядом.
        for joint in [2.0 * A, 2.0 * A + PI * R, 4.0 * A + PI * R] {
            let before = track_point(joint - 1e-4, A, R).0;
            let after = track_point(joint + 1e-4, A, R).0;
            assert!(close(before, after), "разрыв на стыке {joint}");
        }
    }

    #[test]
    fn bottom_run_lies_on_the_ground_and_top_run_on_top() {
        let (bottom, normal) = track_point(A, A, R);
        assert!(close(bottom, Vec2::new(0.0, -R)));
        assert!(close(normal, Vec2::new(0.0, -1.0)));
        let (top, normal) = track_point(3.0 * A + PI * R, A, R);
        assert!(close(top, Vec2::new(0.0, R)));
        assert!(close(normal, Vec2::new(0.0, 1.0)));
        // Заднее закругление на +Z: середина дуги — самая задняя точка.
        let (back, _) = track_point(2.0 * A + PI * R * 0.5, A, R);
        assert!(close(back, Vec2::new(A + R, 0.0)));
    }

    #[test]
    fn driving_forward_moves_top_run_forward() {
        // Путь растёт: нижняя ветвь уходит назад (+Z), верхняя — вперёд (-Z).
        let bottom = track_point(A, A, R).0.x;
        let bottom_later = track_point(A + 0.1, A, R).0.x;
        assert!(bottom_later > bottom);
        let top = track_point(3.0 * A + PI * R, A, R).0.x;
        let top_later = track_point(3.0 * A + PI * R + 0.1, A, R).0.x;
        assert!(top_later < top);
    }

    #[test]
    fn dust_grows_with_speed_and_smoke_with_damage() {
        assert_eq!(dust_rate(0.0), 0.0);
        assert!(dust_rate(-3.0) > 0.0);
        assert_eq!(dust_rate(MAX_FORWARD_SPEED * 2.0), DUST_RATE);
        assert_eq!(smoke_rate(1.0), 0.0);
        assert_eq!(smoke_rate(SMOKE_BELOW), 0.0);
        assert!(smoke_rate(0.1) > smoke_rate(0.3));
        assert_eq!(smoke_rate(0.0), SMOKE_RATE);
        assert_eq!(soot_share(1.0), 0.0);
        assert!(soot_share(0.2) > soot_share(0.6));
    }

    #[test]
    fn suspension_follows_the_ground_within_its_travel() {
        let (lift, pitch) = suspension_pose(0.0, 0.0, 2.0);
        assert_eq!((lift, pitch), (0.0, 0.0));
        // Бугор под передним концом: блок приподнят и задран носом вверх.
        let (lift, pitch) = suspension_pose(0.2, 0.0, 2.0);
        assert!(lift > 0.0 && pitch > 0.0);
        // Глубокая яма: ход ограничен.
        let (lift, pitch) = suspension_pose(-5.0, -5.0, 2.0);
        assert_eq!(lift, -SUSPENSION_TRAVEL);
        assert_eq!(pitch, 0.0);
    }

    #[test]
    fn turning_in_place_runs_tracks_in_opposite_directions() {
        let [left, right] = track_travel(0.0, 0.5, TRACK_X);
        assert!(left < 0.0 && right > 0.0);
        assert!((left + right).abs() < 1e-6);
        let [left, right] = track_travel(1.0, 0.0, TRACK_X);
        assert_eq!(left, right);
    }
}
