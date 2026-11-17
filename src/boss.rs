//! Босс «Колосс»: шагающая крепость, которая приходит каждой четвёртой волной.
//!
//! Предупреждение на экране, через 2 с босс падает с неба и оставляет большой кратер. Затем по кругу
//! три атаки, каждая с подготовкой в секунду: залп мортиры по отметкам вокруг лунохода, веер ракет
//! и таран, сносящий дома и объекты. Гибель — серия взрывов по корпусу, огромный взрыв
//! с замедлением, кратер и ящики с оружием. Пока босс жив, вверху экрана его полоса здоровья.
//!
//! Директор волн просит босса сообщением [`SpawnBoss`].

use std::f32::consts::TAU;

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::combat::{Collider, Damage, Died, Health, Team};
use crate::enemy::Enemy;
use crate::explosion::Explosion;
use crate::frame::{GameSet, RunMessages};
use crate::models::Models;
use crate::mortar::FireShell;
use crate::obstacles::Obstacle;
use crate::pickups::{SpawnCrate, SpawnUpgrade};
use crate::rng::GameRng;
use crate::rockets::FireRocket;
use crate::rover::Rover;
use crate::run::RunScoped;
use crate::terrain::Terrain;
use crate::vehicle::{self, GroundVehicle};
use crate::waves::{Threat, enemy_damage};
use crate::weapons::CurrentWeapon;

/// Сколько висит предупреждение до высадки, с.
const WARNING_TIME: f32 = 2.0;
/// Сколько босс падает с неба, с.
const FALL_TIME: f32 = 0.9;
/// С какой высоты он падает, м.
const FALL_HEIGHT: f32 = 60.0;
/// На каком расстоянии от лунохода высаживается, м.
const LANDING_DISTANCE: f32 = 28.0;
/// Взрыв высадки: радиус, мощность. Мощность как у бомбы авиаудара.
const LANDING_BLAST: (f32, f32) = (6.0, 1.6);

/// Размер корпуса: ширина, высота, длина, м.
const BODY: Vec3 = Vec3::new(5.0, 3.0, 7.0);
/// Высота центра корпуса над землёй, м: под корпусом ноги.
const RIDE_HEIGHT: f32 = 3.4;
/// Ядро на модели туловища: высота и вынос вперёд — янтарный глаз на переднем торце.
const MODEL_CORE: (f32, f32) = (3.4, 3.35);
/// Бёдра модели: шарниры под туловищем, куда крепятся ноги, м: поперёк, высота, вдоль.
/// Модель ноги начинается от бедра и высотой до земли.
const MODEL_HIPS: Vec3 = Vec3::new(1.7, -0.4, 1.7);
/// Радиус коллайдера, м.
const RADIUS: f32 = 4.2;
/// Здоровье при нулевой угрозе; растёт на 40 % за единицу угрозы.
const BASE_HEALTH: f32 = 1500.0;

/// Скорость шага, м/с, и поворот, рад/с.
const WALK_SPEED: f32 = 4.0;
const TURN_SPEED: f32 = 0.9;
/// Босс держится на этом расстоянии от лунохода, м.
const KEEP_DISTANCE: f32 = 24.0;
/// Сколько он идёт между атаками, с.
const WALK_TIME: f32 = 3.0;
/// Подготовка каждой атаки, с.
const PREPARE_TIME: f32 = 1.0;

/// Залп мортиры: снарядов, разброс отметок вокруг лунохода, мощность снаряда.
const SALVO_SHELLS: usize = 8;
const SALVO_SPREAD: (f32, f32) = (2.0, 11.0);
/// Веер ракет: число ракет и полуширина веера, рад.
const FAN_ROCKETS: usize = 10;
const FAN_HALF_ANGLE: f32 = 0.7;
/// Таран: скорость и длительность. Урон — `waves::enemy_damage::BOSS_RAM`.
const RAM_SPEED: f32 = 18.0;
const RAM_TIME: f32 = 3.0;
/// Запас к сумме радиусов босса и лунохода, при котором таран засчитывается, м. Расталкивание
/// (`enemy::separate_enemies`) держит босса ровно на сумме радиусов, удар должен быть дальше неё.
const RAM_REACH: f32 = 1.5;

/// Сколько длится гибель до большого взрыва, с, и число малых взрывов по корпусу.
const DYING_TIME: f32 = 1.5;
const DYING_BLASTS: usize = 6;
/// Большой взрыв гибели: радиус, мощность. Мощность выше порога замедления времени.
const DEATH_BLAST: (f32, f32) = (7.0, 2.2);
/// Ящиков с оружием, вываливающихся из корпуса при гибели.
const LOOT_CRATES: usize = 12;
/// Скорость вылета ящиков: вбок и вверх, м/с.
const LOOT_TOSS: ((f32, f32), (f32, f32)) = ((4.0, 11.0), (7.0, 13.0));

/// Длина ноги от бедра до земли, м.
const LEG_LENGTH: f32 = RIDE_HEIGHT + MODEL_HIPS.y;
/// Размах шага: угол ноги от вертикали в крайнем положении, рад — в ходьбе и на таране.
const STRIDE_ANGLE: (f32, f32) = (0.28, 0.42);
/// На сколько поднимается бедро ноги в переносе, м.
const STEP_LIFT: f32 = 0.45;
/// Поворот на месте тоже перебирает ногами: путь ступни за радиан поворота, м.
const TURN_REACH: f32 = 2.5;
/// Покачивание туловища в шаге: подъём, м, и крен, рад; наклон вперёд на таране, рад.
const BODY_BOB: f32 = 0.12;
const BODY_ROLL: f32 = 0.04;
const RAM_LEAN: f32 = 0.1;
/// Поджатые ноги в падении: угол к центру корпуса, рад, подъём бедра, м; время разгибания, с.
const FOLD_ANGLE: f32 = 0.7;
const FOLD_LIFT: f32 = 0.7;
const UNFOLD_TIME: f32 = 0.35;
/// При гибели ноги подгибаются наполовину.
const DYING_FOLD: f32 = 0.5;

const BODY_COLOR: Color = Color::srgb(0.3, 0.12, 0.14);
const LEG_COLOR: Color = Color::srgb(0.12, 0.12, 0.14);

/// Плагин босса.
pub struct BossPlugin;

impl Plugin for BossPlugin {
    fn build(&self, app: &mut App) {
        app.add_run_message::<SpawnBoss>()
            .add_systems(Startup, create_assets)
            .add_systems(
                Update,
                (
                    (start_warning, run_warning, fall, fight, crush_obstacles, run_dying)
                        .chain()
                        .before(vehicle::move_ground_vehicles)
                        .in_set(GameSet::Simulation),
                    start_dying.in_set(GameSet::Death),
                    (animate_gait, animate_core, update_health_bar)
                        .chain()
                        .in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Просьба привести босса.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpawnBoss;

/// Босс. Числится врагом, но гибель и уборку ведёт сам.
#[derive(Component)]
pub struct Boss {
    phase: BossPhase,
    /// Какая атака следующая: 0 — залп, 1 — веер, 2 — таран.
    next_attack: usize,
    /// Направление тарана, курс.
    ram_heading: f32,
    /// Таран уже ударил луноход.
    ram_hit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum BossPhase {
    /// Падает с неба: прошло секунд.
    Falling(f32),
    /// Идёт к луноходу: осталось секунд до атаки.
    Walking(f32),
    /// Готовит атаку: осталось секунд.
    Preparing(f32),
    /// Таранит: осталось секунд.
    Ramming(f32),
    /// Гибнет: прошло секунд и сколько малых взрывов уже было.
    Dying(f32, usize),
}

/// Предупреждение перед высадкой.
#[derive(Component)]
struct BossWarning {
    remaining: f32,
    landing: Vec3,
}

/// Светящееся ядро: разгорается во время подготовки атаки.
#[derive(Component)]
struct BossCore;

/// Ореол ядра: прозрачная оболочка, пульсирует чуть иначе, чем само ядро.
#[derive(Component)]
struct BossHalo {
    /// Размер оболочки относительно ядра.
    size: f32,
    /// Сдвиг фазы пульса, рад.
    offset: f32,
}

/// Свет ядра: янтарные отблески на корпусе и земле, разгораются с ядром.
#[derive(Component)]
struct BossCoreLight;

/// Яркость света ядра: в покое, на подготовке атаки (пик) и на таране.
const CORE_LIGHT: (f32, f32, f32) = (600_000.0, 6_000_000.0, 3_000_000.0);

/// Походка босса: фаза шага по пройденному пути, вес шага и поджатость ног.
#[derive(Component)]
struct BossGait {
    /// Фаза цикла шага: целая часть — число циклов, дробная — где сейчас нога.
    phase: f32,
    /// Положение по горизонтали и курс в прошлом кадре.
    last: Option<(Vec2, f32)>,
    /// Насколько босс шагает: 0 — стоит, 1 — идёт.
    walking: f32,
    /// Размах шага, рад: растёт на таране.
    stride_angle: f32,
    /// Поджатость ног: 1 — в падении, 0 — стоит на ногах.
    fold: f32,
    /// Наклон туловища вперёд, рад.
    lean: f32,
}

/// Шарнир ноги на бедре: поворачивается вперёд-назад и поднимается в шаге.
#[derive(Component)]
struct BossLeg {
    /// Бедро в системе босса.
    hip: Vec3,
    /// Сдвиг фазы: диагональные пары ног идут в противофазе.
    offset: f32,
    /// Передняя нога: при поджатии ступня уходит назад, к центру корпуса.
    front: bool,
}

/// Туловище с ядром: покачивается в шаге.
#[derive(Component)]
struct BossTorso;

/// Полоса здоровья вверху экрана и её заполнение.
#[derive(Component)]
struct HealthBar;
#[derive(Component)]
struct HealthFill;

#[derive(Resource)]
struct BossAssets {
    body: Handle<Mesh>,
    leg: Handle<Mesh>,
    core: Handle<Mesh>,
    body_material: Handle<StandardMaterial>,
    leg_material: Handle<StandardMaterial>,
    core_material: Handle<StandardMaterial>,
    /// Оболочки ореола: ближняя ярче, дальняя бледнее.
    halo_materials: [Handle<StandardMaterial>; 2],
}

fn create_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(BossAssets {
        body: meshes.add(Cuboid::from_size(BODY)),
        leg: meshes.add(Cuboid::new(0.9, RIDE_HEIGHT + 0.4, 0.9)),
        core: meshes.add(Sphere::new(0.9).mesh().ico(3).expect("ядро босса")),
        body_material: materials.add(BODY_COLOR),
        leg_material: materials.add(LEG_COLOR),
        // У `unlit` Bevy берёт только `base_color`, без `emissive`: яркость выше 1 даёт свечение (bloom).
        core_material: materials.add(StandardMaterial {
            base_color: Color::linear_rgb(8.0, 2.4, 0.5),
            unlit: true,
            ..default()
        }),
        halo_materials: [0.35, 0.15].map(|alpha| {
            materials.add(StandardMaterial {
                base_color: Color::linear_rgba(1.6, 0.5, 0.08, alpha),
                alpha_mode: AlphaMode::Add,
                unlit: true,
                ..default()
            })
        }),
    });
}

/// Здоровье босса при угрозе `threat`.
pub fn boss_health(threat: f32) -> f32 {
    BASE_HEALTH * (1.0 + 0.4 * threat.max(0.0))
}

/// Показывает предупреждение и запоминает место высадки. Пока прежний босс жив или летит,
/// новый не приходит: босс на поле всегда один.
fn start_warning(
    mut commands: Commands,
    mut requests: MessageReader<SpawnBoss>,
    mut dice: ResMut<GameRng>,
    rover: Single<&Transform, With<Rover>>,
    present: Query<(), Or<(With<Boss>, With<BossWarning>)>>,
) {
    let mut busy = !present.is_empty();
    for _ in requests.read() {
        if busy {
            continue;
        }
        busy = true;
        let angle = dice.range(0.0, TAU);
        let landing = rover.translation + Vec3::new(angle.cos(), 0.0, angle.sin()) * LANDING_DISTANCE;
        commands
            .spawn((
                BossWarning {
                    remaining: WARNING_TIME,
                    landing,
                },
                RunScoped,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    top: Val::Percent(32.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
            ))
            .with_children(|banner| {
                banner.spawn((
                    Text::new("BOSS INCOMING"),
                    TextFont {
                        font_size: FontSize::Px(64.0),
                        ..default()
                    },
                    TextColor(Color::srgb(1.0, 0.3, 0.2)),
                ));
            });
    }
}

/// Мигает предупреждением, затем создаёт падающего босса и полосу здоровья.
fn run_warning(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<BossAssets>,
    threat: Res<Threat>,
    terrain: Res<Terrain>,
    mut models: Models,
    mut warnings: Query<(Entity, &mut BossWarning, &mut Visibility)>,
) {
    for (entity, mut warning, mut visibility) in &mut warnings {
        warning.remaining -= time.delta_secs();
        *visibility = if (warning.remaining * 6.0).fract() < 0.6 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if warning.remaining > 0.0 {
            continue;
        }
        commands.entity(entity).despawn();
        let mut landing = warning.landing;
        landing.y = terrain.height_at(landing.x, landing.z) + RIDE_HEIGHT + FALL_HEIGHT;
        spawn_boss(&mut commands, &assets, &mut models, landing, boss_health(threat.value()));
        spawn_health_bar(&mut commands);
    }
}

/// Ставит босса. Туловище и ноги — модели из `models.json`, пока они грузятся или без них — кубы.
fn spawn_boss(
    commands: &mut Commands,
    assets: &BossAssets,
    models: &mut Models,
    position: Vec3,
    health: f32,
) {
    // Ядро: у модели — на переднем торце, у кубов — сверху спереди.
    let core = if models.has("boss_body") {
        Transform::from_xyz(0.0, MODEL_CORE.0, -MODEL_CORE.1)
    } else {
        Transform::from_xyz(0.0, BODY.y * 0.5 + 0.3, -BODY.z * 0.3)
    };
    commands
        .spawn((
            Boss {
                phase: BossPhase::Falling(0.0),
                next_attack: 0,
                ram_heading: 0.0,
                ram_hit: false,
            },
            Enemy,
            RunScoped,
            Health::new(health),
            Team::Enemy,
            // Свои босса не ранят: ни его удар приземления, ни выстрелы отряда.
            crate::combat::AllyProof,
            Collider { radius: RADIUS },
            BossGait {
                phase: 0.0,
                last: None,
                walking: 0.0,
                stride_angle: STRIDE_ANGLE.0,
                fold: 1.0,
                lean: 0.0,
            },
            Transform::from_translation(position),
            Visibility::default(),
        ))
        .with_children(|parent| {
            parent
                .spawn((BossTorso, Transform::IDENTITY, Visibility::default()))
                .with_children(|torso| {
                    models.spawn_visual(torso, "boss_body", Transform::IDENTITY, |body| {
                        body.spawn((
                            Mesh3d(assets.body.clone()),
                            MeshMaterial3d(assets.body_material.clone()),
                        ));
                    });
                    torso
                        .spawn((
                            BossCore,
                            NotShadowCaster,
                            Mesh3d(assets.core.clone()),
                            MeshMaterial3d(assets.core_material.clone()),
                            core,
                        ))
                        .with_children(|core| {
                            for ((size, offset), material) in [(1.35, 0.0), (1.9, 1.3)].into_iter().zip(&assets.halo_materials) {
                                core.spawn((
                                    BossHalo { size, offset },
                                    NotShadowCaster,
                                    Mesh3d(assets.core.clone()),
                                    MeshMaterial3d(material.clone()),
                                    Transform::from_scale(Vec3::splat(size)),
                                ));
                            }
                            // Тени точечным источникам не включать: на Adreno шейдер тени падает (web/index.html).
                            core.spawn((
                                BossCoreLight,
                                PointLight {
                                    color: Color::srgb(1.0, 0.45, 0.1),
                                    intensity: CORE_LIGHT.0,
                                    range: 22.0,
                                    shadow_maps_enabled: false,
                                    ..default()
                                },
                                Transform::from_xyz(0.0, 0.0, -1.5),
                            ));
                        });
                });
            // Диагональные пары: передняя левая с задней правой, передняя правая с задней левой.
            for (x, z, offset) in [(-1.0, -1.0, 0.0), (1.0, -1.0, 0.5), (-1.0, 1.0, 0.5), (1.0, 1.0, 0.0)] {
                let hip = Vec3::new(x * MODEL_HIPS.x, MODEL_HIPS.y, z * MODEL_HIPS.z);
                let leg = Vec3::new(x * BODY.x * 0.45, -RIDE_HEIGHT * 0.5, z * BODY.z * 0.4);
                parent
                    .spawn((
                        BossLeg {
                            hip,
                            offset,
                            front: z < 0.0,
                        },
                        Transform::from_translation(hip),
                        Visibility::default(),
                    ))
                    .with_children(|pivot| {
                        models.spawn_visual(pivot, "boss_leg", Transform::IDENTITY, |spare| {
                            spare.spawn((
                                Mesh3d(assets.leg.clone()),
                                MeshMaterial3d(assets.leg_material.clone()),
                                Transform::from_translation(leg - hip),
                            ));
                        });
                    });
            }
        });
}

fn spawn_health_bar(commands: &mut Commands) {
    commands
        .spawn((
            HealthBar,
            RunScoped,
            Node {
                position_type: PositionType::Absolute,
                // Под временем забега (`hud.rs`: отступ 16 px, плашка около 40 px).
                top: Val::Px(64.0),
                left: Val::Percent(25.0),
                width: Val::Percent(50.0),
                height: Val::Px(18.0),
                border: UiRect::all(Val::Px(2.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            BorderColor::all(Color::srgb(1.0, 0.85, 0.8)),
        ))
        .with_children(|bar| {
            bar.spawn((
                HealthFill,
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.85, 0.12, 0.1)),
            ));
        });
}

/// Падение с неба: на земле — взрыв высадки с кратером, затем бой.
fn fall(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    rover: Single<&Transform, (With<Rover>, Without<Boss>)>,
    mut bosses: Query<(Entity, &mut Transform, &mut Boss)>,
    mut explosions: MessageWriter<Explosion>,
) {
    for (entity, mut transform, mut boss) in &mut bosses {
        let BossPhase::Falling(t) = boss.phase else {
            continue;
        };
        let t = t + time.delta_secs();
        let ground = terrain.height_at(transform.translation.x, transform.translation.z) + RIDE_HEIGHT;
        let k = (t / FALL_TIME).min(1.0);
        transform.translation.y = ground + FALL_HEIGHT * (1.0 - k * k);
        if k < 1.0 {
            boss.phase = BossPhase::Falling(t);
            continue;
        }
        let feet = transform.translation - Vec3::Y * RIDE_HEIGHT;
        explosions.write(Explosion {
            position: feet,
            radius: LANDING_BLAST.0,
            power: LANDING_BLAST.1,
            source_team: Team::Enemy,
        });
        let heading = vehicle::heading_towards(rover.translation - transform.translation);
        transform.rotation = Quat::from_rotation_y(heading);
        commands.entity(entity).try_insert(GroundVehicle::new(
            heading,
            RIDE_HEIGHT,
            Vec2::new(BODY.x, BODY.z) * 0.4,
        ));
        boss.phase = BossPhase::Walking(WALK_TIME);
    }
}

/// Достаёт ли таран луноход радиуса `rover_radius` на расстоянии `distance` по горизонтали.
fn ram_hits(distance: f32, rover_radius: f32) -> bool {
    distance < RADIUS + rover_radius + RAM_REACH
}

/// Цикл боя: шаг к луноходу, подготовка, атака.
#[allow(clippy::too_many_arguments)]
fn fight(
    time: Res<Time>,
    mut dice: ResMut<GameRng>,
    rover: Single<(Entity, &Transform, &Health, &Collider), (With<Rover>, Without<Boss>)>,
    mut bosses: Query<(&Transform, &mut GroundVehicle, &mut Boss)>,
    mut shells: MessageWriter<FireShell>,
    mut rockets: MessageWriter<FireRocket>,
    mut damage: MessageWriter<Damage>,
) {
    let (rover_entity, rover, rover_health, rover_collider) = *rover;
    let dt = time.delta_secs();
    for (transform, mut vehicle, mut boss) in &mut bosses {
        let me = transform.translation;
        let to_rover = (rover.translation - me).with_y(0.0);
        let face = vehicle::heading_towards(to_rover);
        match boss.phase {
            BossPhase::Walking(left) => {
                vehicle.heading = vehicle::steer(vehicle.heading, face, TURN_SPEED * dt);
                vehicle.speed = if to_rover.length() > KEEP_DISTANCE { WALK_SPEED } else { 0.0 };
                let left = left - dt;
                boss.phase = if left > 0.0 || rover_health.is_dead() {
                    BossPhase::Walking(left.max(0.0))
                } else {
                    BossPhase::Preparing(PREPARE_TIME)
                };
            }
            BossPhase::Preparing(left) => {
                vehicle.speed = 0.0;
                vehicle.heading = vehicle::steer(vehicle.heading, face, TURN_SPEED * 2.0 * dt);
                let left = left - dt;
                if left > 0.0 {
                    boss.phase = BossPhase::Preparing(left);
                    continue;
                }
                let attack = boss.next_attack;
                boss.next_attack = (attack + 1) % 3;
                boss.phase = BossPhase::Walking(WALK_TIME);
                let muzzle = me + Vec3::Y * (BODY.y * 0.5 + 1.0);
                match attack {
                    0 => {
                        // Залп: отметки падения появляются вокруг лунохода, снаряды бьют по ним.
                        for _ in 0..SALVO_SHELLS {
                            let angle = dice.range(0.0, TAU);
                            let distance = dice.range(SALVO_SPREAD.0, SALVO_SPREAD.1);
                            shells.write(FireShell {
                                origin: muzzle,
                                target: rover.translation + Vec3::new(angle.cos(), 0.0, angle.sin()) * distance,
                                team: Team::Enemy,
                                power: enemy_damage::BOSS_SALVO_POWER,
                                radius: crate::mortar::EXPLOSION_RADIUS,
                                cluster: 0,
                            });
                        }
                    }
                    1 => {
                        // Веер ракет по сторонам от лунохода.
                        let distance = to_rover.length().max(8.0);
                        for i in 0..FAN_ROCKETS {
                            let k = i as f32 / (FAN_ROCKETS - 1) as f32 * 2.0 - 1.0;
                            let dir = Quat::from_rotation_y(k * FAN_HALF_ANGLE) * to_rover.normalize_or(Vec3::X);
                            rockets.write(FireRocket {
                                origin: muzzle + dir * (BODY.z * 0.5),
                                target: me + dir * (distance + 6.0),
                                team: Team::Enemy,
                            });
                        }
                    }
                    _ => {
                        boss.ram_heading = face;
                        boss.ram_hit = false;
                        boss.phase = BossPhase::Ramming(RAM_TIME);
                    }
                }
            }
            BossPhase::Ramming(left) => {
                vehicle.heading = vehicle::steer(vehicle.heading, boss.ram_heading, TURN_SPEED * 2.0 * dt);
                vehicle.speed = RAM_SPEED;
                if !boss.ram_hit && !rover_health.is_dead() && ram_hits(to_rover.length(), rover_collider.radius) {
                    boss.ram_hit = true;
                    damage.write(Damage {
                        target: rover_entity,
                        amount: enemy_damage::BOSS_RAM,
                        source_team: Team::Enemy,
                    });
                }
                let left = left - dt;
                boss.phase = if left > 0.0 {
                    BossPhase::Ramming(left)
                } else {
                    vehicle.speed = 0.0;
                    BossPhase::Walking(WALK_TIME)
                };
            }
            BossPhase::Falling(_) | BossPhase::Dying(..) => {}
        }
    }
}

/// Босс сносит дома и объекты, на которые наступает: на таране — шире.
fn crush_obstacles(
    mut commands: Commands,
    bosses: Query<(&Transform, &Boss)>,
    obstacles: Query<(Entity, &Obstacle, &Health), Without<Boss>>,
    mut damage: MessageWriter<Damage>,
) {
    for (transform, boss) in &bosses {
        if matches!(boss.phase, BossPhase::Falling(_) | BossPhase::Dying(..)) {
            continue;
        }
        let reach = if matches!(boss.phase, BossPhase::Ramming(_)) { RADIUS + 2.0 } else { RADIUS };
        let p = Vec2::new(transform.translation.x, transform.translation.z);
        for (entity, obstacle, health) in &obstacles {
            if health.is_dead() {
                continue;
            }
            let (center, r) = obstacle.shape.bounds();
            if center.distance(p) > r + reach {
                continue;
            }
            commands
                .entity(entity)
                .try_remove::<Obstacle>()
                .try_insert((
                    crate::props::PropBlastFrom(transform.translation),
                    crate::buildings::BlastFrom(transform.translation),
                ));
            damage.write(Damage {
                target: entity,
                amount: health.current + 1.0,
                source_team: Team::Enemy,
            });
        }
    }
}

/// Здоровье кончилось: босс замирает, начинается серия взрывов.
fn start_dying(
    mut died: MessageReader<Died>,
    mut bosses: Query<(&mut Boss, Option<&mut GroundVehicle>)>,
) {
    for death in died.read() {
        if let Ok((mut boss, vehicle)) = bosses.get_mut(death.entity) {
            boss.phase = BossPhase::Dying(0.0, 0);
            if let Some(mut vehicle) = vehicle {
                vehicle.speed = 0.0;
            }
        }
    }
}

/// Взрывы по корпусу, затем огромный взрыв, кратер, ящики и уборка.
fn run_dying(
    mut commands: Commands,
    time: Res<Time>,
    mut dice: ResMut<GameRng>,
    mut bosses: Query<(Entity, &Transform, &mut Boss)>,
    mut explosions: MessageWriter<Explosion>,
    mut crates: MessageWriter<SpawnCrate>,
    mut upgrade: MessageWriter<SpawnUpgrade>,
    mut breaker: crate::fragments::Breaker,
) {
    for (entity, transform, mut boss) in &mut bosses {
        let BossPhase::Dying(t, blasts) = boss.phase else {
            continue;
        };
        let t = t + time.delta_secs();
        let due = ((t / DYING_TIME) * DYING_BLASTS as f32).floor() as usize;
        let mut done = blasts;
        while done < due.min(DYING_BLASTS) {
            let offset = Vec3::new(
                dice.range(-0.5, 0.5) * BODY.x,
                dice.range(0.0, 0.5) * BODY.y,
                dice.range(-0.5, 0.5) * BODY.z,
            );
            explosions.write(Explosion {
                position: transform.translation + transform.rotation * offset,
                radius: 3.0,
                power: 0.4,
                source_team: Team::Enemy,
            });
            done += 1;
        }
        if t < DYING_TIME {
            boss.phase = BossPhase::Dying(t, done);
            continue;
        }
        let feet = transform.translation - Vec3::Y * RIDE_HEIGHT;
        explosions.write(Explosion {
            position: feet,
            radius: DEATH_BLAST.0,
            power: DEATH_BLAST.1,
            source_team: Team::Enemy,
        });
        // Ящики всех видов оружия, кроме пулемёта, вываливаются из корпуса во все стороны.
        let weapons: Vec<CurrentWeapon> =
            CurrentWeapon::ALL.iter().copied().filter(|w| *w != CurrentWeapon::MachineGun).collect();
        let body = transform.translation + Vec3::Y * BODY.y * 0.5;
        for i in 0..LOOT_CRATES {
            let angle = i as f32 / LOOT_CRATES as f32 * TAU + dice.range(-0.25, 0.25);
            let out = dice.range(LOOT_TOSS.0.0, LOOT_TOSS.0.1);
            let up = dice.range(LOOT_TOSS.1.0, LOOT_TOSS.1.1);
            crates.write(SpawnCrate {
                position: body + Vec3::new(angle.cos(), 0.0, angle.sin()) * 1.5,
                velocity: Vec3::new(angle.cos() * out, up, angle.sin() * out),
                weapon: weapons[i % weapons.len()],
            });
        }
        // Корпус разлетается кусками, ноги отваливаются целиком.
        crate::fragments::wreck_vehicle(&mut breaker, entity, transform, 2.0);
        // Золотой ящик улучшения — в центре, на месте гибели.
        upgrade.write(SpawnUpgrade {
            position: feet + Vec3::Y,
        });
        commands.entity(entity).despawn();
    }
}

/// Поза ноги в цикле шага с длиной шага `stride`, м: угол от вертикали (плюс — ступня впереди)
/// и доля подъёма бедра. Первая половина цикла — опора: ступня идёт назад ровно с ходом корпуса
/// и стоит на месте относительно земли. Вторая — перенос: нога поднимается и выносится вперёд.
fn leg_pose(phase: f32, stride: f32) -> (f32, f32) {
    let p = phase.rem_euclid(1.0);
    let half = stride * 0.5;
    let (x, lift) = if p < 0.5 {
        (half - stride * p * 2.0, 0.0)
    } else {
        let s = (p - 0.5) * 2.0;
        let eased = s * s * (3.0 - 2.0 * s);
        (-half + stride * eased, (s * std::f32::consts::PI).sin())
    };
    ((x / LEG_LENGTH).clamp(-1.0, 1.0).asin(), lift)
}

/// Длина шага при размахе `angle`, м.
fn stride_length(angle: f32) -> f32 {
    2.0 * LEG_LENGTH * angle.sin()
}

/// На сколько продвигается фаза шага за путь `distance`, м: цикл — опора и перенос, за цикл
/// корпус проходит два шага.
fn phase_advance(distance: f32, angle: f32) -> f32 {
    distance / (2.0 * stride_length(angle))
}

/// Ноги переставляются по диагоналям в такт пути, корпус покачивается; в падении ноги поджаты.
fn animate_gait(
    time: Res<Time>,
    mut bosses: Query<(&Boss, &Transform, &mut BossGait)>,
    mut legs: Query<(&BossLeg, &ChildOf, &mut Transform), (Without<Boss>, Without<BossTorso>)>,
    mut torsos: Query<(&ChildOf, &mut Transform), (With<BossTorso>, Without<Boss>, Without<BossLeg>)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for (boss, transform, mut gait) in &mut bosses {
        let here = transform.translation.xz();
        let heading = transform.rotation.to_euler(EulerRot::YXZ).0;
        let falling = matches!(boss.phase, BossPhase::Falling(_));
        let (moved, turned) = match gait.last {
            Some((last, last_heading)) if !falling => {
                let turn = (heading - last_heading + std::f32::consts::PI).rem_euclid(TAU) - std::f32::consts::PI;
                ((here - last).length(), turn.abs())
            }
            _ => (0.0, 0.0),
        };
        gait.last = Some((here, heading));
        let distance = moved + turned * TURN_REACH;
        let ramming = matches!(boss.phase, BossPhase::Ramming(_));
        let target_angle = if ramming { STRIDE_ANGLE.1 } else { STRIDE_ANGLE.0 };
        gait.stride_angle += (target_angle - gait.stride_angle) * (dt * 4.0).min(1.0);
        let advance = phase_advance(distance, gait.stride_angle);
        gait.phase += advance;
        // Шагает, пока движется; остановился — ноги плавно встают ровно.
        let target_walk = if distance / dt > 0.5 { 1.0 } else { 0.0 };
        gait.walking += (target_walk - gait.walking) * (dt * 5.0).min(1.0);
        let target_fold = match boss.phase {
            BossPhase::Falling(_) => 1.0,
            BossPhase::Dying(..) => DYING_FOLD,
            _ => 0.0,
        };
        let step = dt / UNFOLD_TIME;
        gait.fold = if gait.fold < target_fold {
            (gait.fold + step).min(target_fold)
        } else {
            (gait.fold - step).max(target_fold)
        };
        let target_lean = if ramming { -RAM_LEAN } else { 0.0 };
        gait.lean += (target_lean - gait.lean) * (dt * 3.0).min(1.0);
    }

    let stride_of = |gait: &BossGait| stride_length(gait.stride_angle);
    for (leg, child_of, mut leg_transform) in &mut legs {
        let Ok((_, _, gait)) = bosses.get(child_of.parent()) else {
            continue;
        };
        let (angle, lift) = leg_pose(gait.phase + leg.offset, stride_of(gait));
        let walk_angle = angle * gait.walking;
        let walk_lift = lift * STEP_LIFT * gait.walking;
        let fold_angle = if leg.front { -FOLD_ANGLE } else { FOLD_ANGLE };
        let angle = walk_angle + (fold_angle - walk_angle) * gait.fold;
        let lift = walk_lift + (FOLD_LIFT - walk_lift) * gait.fold;
        leg_transform.translation = leg.hip + Vec3::Y * lift;
        leg_transform.rotation = Quat::from_rotation_x(angle);
    }
    for (child_of, mut torso) in &mut torsos {
        let Ok((_, _, gait)) = bosses.get(child_of.parent()) else {
            continue;
        };
        let p = gait.phase * TAU;
        // Корпус оседает на каждую диагональную пару — дважды за цикл, и перекатывается с боку на бок.
        torso.translation.y = BODY_BOB * (2.0 * p).cos() * gait.walking - FOLD_LIFT * 0.3 * gait.fold;
        torso.rotation = Quat::from_rotation_x(gait.lean) * Quat::from_rotation_z(BODY_ROLL * p.sin() * gait.walking);
    }
}

/// Ядро разгорается и пульсирует во время подготовки атаки, ореол дышит, свет ядра разгорается с ним.
fn animate_core(
    time: Res<Time>,
    bosses: Query<&Boss>,
    torsos: Query<&ChildOf, With<BossTorso>>,
    mut cores: Query<(&ChildOf, &mut Transform, &Children), With<BossCore>>,
    mut halos: Query<(&BossHalo, &mut Transform), Without<BossCore>>,
    mut lights: Query<&mut PointLight, With<BossCoreLight>>,
) {
    let t = time.elapsed_secs();
    for (child_of, mut core, children) in &mut cores {
        let Ok(torso) = torsos.get(child_of.parent()) else {
            continue;
        };
        let Ok(boss) = bosses.get(torso.parent()) else {
            continue;
        };
        // Насколько разгорелось ядро: 0 — покой, 1 — пик подготовки.
        let (scale, glow, light) = match boss.phase {
            BossPhase::Preparing(left) => {
                let charge = 1.0 - left / PREPARE_TIME;
                let flicker = 0.2 * (t * 30.0).sin();
                (1.0 + charge * 1.2 + flicker, 1.0 + charge * 0.6, CORE_LIGHT.0.lerp(CORE_LIGHT.1, charge) * (1.0 + flicker))
            }
            BossPhase::Ramming(_) => (1.6, 1.2, CORE_LIGHT.2),
            // При гибели ядро мигает, как сбоящая лампа.
            BossPhase::Dying(..) => (1.0, 1.0, CORE_LIGHT.0 * (0.5 + 0.5 * (t * 23.0).sin() * (t * 7.0).sin()).max(0.0)),
            _ => (1.0, 1.0, CORE_LIGHT.0 * (1.0 + 0.15 * (t * 2.5).sin())),
        };
        core.scale = Vec3::splat(scale);
        for child in children {
            if let Ok((halo, mut transform)) = halos.get_mut(*child) {
                let breath = 1.0 + 0.08 * (t * 3.0 + halo.offset).sin();
                transform.scale = Vec3::splat(halo.size * glow * breath);
            }
            if let Ok(mut point) = lights.get_mut(*child) {
                point.intensity = light;
            }
        }
    }
}

/// Полоса здоровья следует за здоровьем босса и исчезает с ним.
fn update_health_bar(
    mut commands: Commands,
    bosses: Query<&Health, With<Boss>>,
    bars: Query<Entity, With<HealthBar>>,
    mut fills: Query<&mut Node, With<HealthFill>>,
) {
    let Some(health) = bosses.iter().next() else {
        for bar in &bars {
            commands.entity(bar).despawn();
        }
        return;
    };
    for mut fill in &mut fills {
        fill.width = Val::Percent(health.fraction() * 100.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn таран_достаёт_дальше_расталкивания() {
        // Расталкивание держит босса на сумме радиусов, с луноходом это около 6,6 м.
        let rover_radius = crate::combat::ROVER_COLLIDER_RADIUS * crate::rover::ROVER_SCALE;
        let pushed_apart = RADIUS + rover_radius;
        assert!(ram_hits(pushed_apart, rover_radius));
        assert!(ram_hits(pushed_apart + 1.0, rover_radius));
        assert!(!ram_hits(pushed_apart + 5.0, rover_radius));
    }

    #[test]
    fn ступня_в_опоре_стоит_на_месте() {
        // Корпус идёт вперёд (к -Z) на путь d; ступня — бедро плюс вынос ноги L·sin(угол) вперёд.
        let angle = STRIDE_ANGLE.0;
        let stride = stride_length(angle);
        let foot = |d: f32| {
            let (a, lift) = leg_pose(phase_advance(d, angle), stride);
            assert_eq!(lift, 0.0, "в опоре нога не поднята");
            -d - LEG_LENGTH * a.sin()
        };
        let start = foot(0.0);
        for i in 1..20 {
            let d = stride * i as f32 / 20.0;
            assert!((foot(d) - start).abs() < 1e-3, "ступня уехала на пути {d}");
        }
    }

    #[test]
    fn диагональные_пары_идут_в_противофазе() {
        let stride = stride_length(STRIDE_ANGLE.0);
        for i in 0..40 {
            let phase = i as f32 / 40.0 + 0.0125;
            let (_, lift_a) = leg_pose(phase, stride);
            let (_, lift_b) = leg_pose(phase + 0.5, stride);
            // Одна пара всегда стоит на земле.
            assert!(lift_a == 0.0 || lift_b == 0.0, "обе пары в воздухе на фазе {phase}");
        }
        // В переносе нога выносится вперёд и поднимается.
        let (a_start, _) = leg_pose(0.5, stride);
        let (a_end, _) = leg_pose(0.999, stride);
        let (_, lift_mid) = leg_pose(0.75, stride);
        assert!(a_start < 0.0 && a_end > 0.0 && lift_mid > 0.9);
    }

    #[test]
    fn здоровье_босса_растёт_с_угрозой() {
        assert!(boss_health(3.0) > boss_health(0.0));
        assert!(boss_health(0.0) > 1000.0);
    }
}
