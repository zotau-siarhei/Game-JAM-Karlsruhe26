//! Облик тяжёлых танков: корпус, башня и два гусеничных блока из моделей Meshy, бегущие грунтозацепы.
//!
//! Танк собран из частей, чтобы башня поворачивалась отдельно от корпуса, а гусеницы бежали по
//! ходу машины. Шасси у ракетного и лазерного танков общее: `enemy_tank_hull` и `enemy_tank_track`;
//! наверху у ракетного — пушечная башня `enemy_tank_turret`, у лазерного — лазерная установка
//! `enemy_laser_turret`. Всё в `models.json`; масштаб, поворот и подъём над землёй — там же. Пока модели грузятся, корпус —
//! коробка. Логика танка (`enemy::drive_tanks`) пишет курс башни в [`TankGun`], этот модуль только
//! показывает его.
//!
//! Все размеры — в системе координат танка, которая растянута на `enemy::TANK_SCALE`.
//! Перёд танка — -Z.

use std::f32::consts::{FRAC_PI_2, PI};

use bevy::prelude::*;

use crate::enemy::{HeavyTank, RIDE_HEIGHT, TANK_FIRE_RANGE, TANK_SCALE, TankArm};
use crate::frame::GameSet;
use crate::models::Models;
use crate::rover_look::{track_length, track_point, track_travel};
use crate::vehicle::GroundVehicle;

/// Высота земли под центром танка.
const GROUND_Y: f32 = -RIDE_HEIGHT;

/// Корпус-заглушка, пока модель грузится: размер и цвет.
const HULL_BOX: Vec3 = Vec3::new(1.4, 0.55, 2.0);
const HULL_COLOR: Color = Color::srgb(0.24, 0.25, 0.27);
/// Высота центра корпуса над землёй: днище на середине гусениц.
const HULL_Y: f32 = 0.5;

/// Масштаб модели башни (`models.json`). Начало модели — середина низа башни без ствола: вокруг
/// неё башня и поворачивается.
const TURRET_SCALE: f32 = 1.95;
/// Высота оси поворота башни (низа башни) над землёй: в погоне на крыше корпуса.
const TURRET_Y: f32 = 0.7;
/// Масштаб лазерной установки (`models.json`). Начало модели — середина опорного кольца снизу.
const LASER_SCALE: f32 = 1.3;

/// Модель верха танка и дульный срез: вперёд от оси башни и вверх от её низа, в системе танка.
struct Top {
    model: &'static str,
    muzzle_forward: f32,
    muzzle_up: f32,
}

/// Пушечная башня ракетного танка: дульный срез в модели 0,631 вперёд и 0,116 вверх.
const CANNON: Top = Top {
    model: "enemy_tank_turret",
    muzzle_forward: 0.631 * TURRET_SCALE,
    muzzle_up: 0.116 * TURRET_SCALE,
};
/// Лазерная установка: линза в модели 0,557 вперёд и 0,341 вверх.
const LASER: Top = Top {
    model: "enemy_laser_turret",
    muzzle_forward: 0.557 * LASER_SCALE,
    muzzle_up: 0.341 * LASER_SCALE,
};

/// Запасная лазерная установка, пока нет модели: корпус-коробка, излучатель и светящаяся линза.
const LASER_BODY: Vec3 = Vec3::new(0.85, 0.45, 0.85);

fn top(arm: TankArm) -> &'static Top {
    match arm {
        TankArm::Rockets => &CANNON,
        TankArm::Laser => &LASER,
    }
}

/// Полуширина колеи: центр гусеничного блока от оси танка, блок чуть заходит под корпус.
const TRACK_X: f32 = 0.79;
/// Радиус закругления ленты по наружной поверхности: половина высоты блока.
const TRACK_RADIUS: f32 = 0.26;
/// Половина длины прямого участка ленты: полдлины блока без закругления.
const TRACK_HALF_STRAIGHT: f32 = 0.63;
/// Высота центра гусеничного блока: лента касается земли.
const TRACK_Y: f32 = GROUND_Y + TRACK_RADIUS;

/// Грунтозацепов на одну ленту.
const GROUSERS: usize = 22;
/// Размер грунтозацепа: поперёк ленты (лента в модели шириной 0,18), толщина, вдоль ленты.
const GROUSER_SIZE: Vec3 = Vec3::new(0.17, 0.03, 0.07);
/// Цвет грунтозацепов: светлее ленты модели, чтобы её бег был заметен.
const GROUSER_COLOR: Color = Color::srgb(0.3, 0.3, 0.34);

/// Больше такого сдвига за кадр — это перенос танка, а не езда, м.
const MAX_STEP: f32 = 3.0;

/// Плагин облика тяжёлого танка.
pub struct TankLookPlugin;

impl Plugin for TankLookPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, create_assets).add_systems(
            Update,
            (attach_look, pose_turrets, run_tracks)
                .chain()
                .in_set(GameSet::Presentation),
        );
    }
}

/// Пушка танка: курс башни относительно корпуса, рад (ноль — по носу, плюс — влево),
/// и идёт ли прицеливание (видна линия прицела).
#[derive(Component, Default)]
pub struct TankGun {
    pub yaw: f32,
    pub aiming: bool,
}

/// Направление ствола в мире и точка дульного среза танка с оружием `arm`.
pub fn muzzle(transform: &Transform, vehicle: &GroundVehicle, gun: &TankGun, arm: TankArm) -> (Vec3, Vec3) {
    let top = top(arm);
    let direction = crate::vehicle::forward_of(vehicle.heading + gun.yaw);
    let origin = transform.translation
        + Vec3::Y * (TURRET_Y + GROUND_Y + top.muzzle_up) * TANK_SCALE
        + direction * top.muzzle_forward * TANK_SCALE;
    (origin, direction)
}

/// Узел башни: поворачивается вокруг вертикали.
#[derive(Component)]
struct Turret;

/// Линия прицела: тонкий красный луч из ствола.
#[derive(Component)]
struct AimLine;

/// Борт гусеницы: 0 левый, 1 правый.
#[derive(Component)]
struct TrackSide(usize);

/// Грунтозацеп: номер на ленте.
#[derive(Component)]
struct Grouser(usize);

/// Пройденный путь лент по бортам и прошлое положение танка.
#[derive(Component, Default)]
struct TrackDrive {
    travel: [f32; 2],
    last: Option<(Vec3, f32)>,
}

#[derive(Resource)]
struct TankLookAssets {
    hull: Handle<Mesh>,
    hull_material: Handle<StandardMaterial>,
    grouser: Handle<Mesh>,
    grouser_material: Handle<StandardMaterial>,
    aim_line: Handle<Mesh>,
    aim_material: Handle<StandardMaterial>,
    laser_body: Handle<Mesh>,
    laser_emitter: Handle<Mesh>,
    laser_lens: Handle<Mesh>,
    lens_material: Handle<StandardMaterial>,
}

fn create_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(TankLookAssets {
        hull: meshes.add(Cuboid::from_size(HULL_BOX)),
        hull_material: materials.add(HULL_COLOR),
        grouser: meshes.add(Cuboid::from_size(GROUSER_SIZE)),
        grouser_material: materials.add(StandardMaterial {
            base_color: GROUSER_COLOR,
            metallic: 0.3,
            perceptual_roughness: 0.6,
            ..default()
        }),
        // Ось цилиндра — Y; линия поворачивается вдоль -Z при создании.
        aim_line: meshes.add(Cylinder::new(0.04, 1.0)),
        aim_material: materials.add(StandardMaterial {
            base_color: Color::srgba(1.0, 0.15, 0.1, 0.8),
            emissive: LinearRgba::rgb(12.0, 1.0, 0.5),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            ..default()
        }),
        laser_body: meshes.add(Cuboid::from_size(LASER_BODY)),
        laser_emitter: meshes.add(Cylinder::new(0.13, 1.0)),
        laser_lens: meshes.add(Sphere::new(0.15)),
        lens_material: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.6, 0.2),
            emissive: LinearRgba::rgb(4.0, 1.8, 0.4),
            ..default()
        }),
    });
}

/// Ставит на новый танк корпус, башню с линией прицела, гусеничные блоки и грунтозацепы.
fn attach_look(
    mut commands: Commands,
    tanks: Query<(Entity, &HeavyTank), Added<TankGun>>,
    assets: Res<TankLookAssets>,
    mut models: Models,
) {
    for (tank, heavy) in &tanks {
        let arm = heavy.arm;
        let top = top(arm);
        commands
            .entity(tank)
            .insert(TrackDrive::default())
            .with_children(|parent| {
                models.spawn_visual(
                    parent,
                    "enemy_tank_hull",
                    Transform::from_xyz(0.0, GROUND_Y, 0.0),
                    |spare| {
                        spare.spawn((
                            Mesh3d(assets.hull.clone()),
                            MeshMaterial3d(assets.hull_material.clone()),
                            Transform::from_xyz(0.0, GROUND_Y + HULL_Y, 0.0),
                        ));
                    },
                );
                parent
                    .spawn((
                        Turret,
                        Transform::from_xyz(0.0, GROUND_Y + TURRET_Y, 0.0),
                        Visibility::default(),
                    ))
                    .with_children(|turret| {
                        models.spawn_visual(turret, top.model, Transform::IDENTITY, |spare| {
                            if arm == TankArm::Laser {
                                spawn_laser_spare(spare, &assets, top);
                            }
                        });
                        let length = TANK_FIRE_RANGE / TANK_SCALE;
                        turret.spawn((
                            AimLine,
                            Mesh3d(assets.aim_line.clone()),
                            MeshMaterial3d(assets.aim_material.clone()),
                            bevy::light::NotShadowCaster,
                            Visibility::Hidden,
                            Transform::from_xyz(0.0, top.muzzle_up, -top.muzzle_forward - length * 0.5)
                                .with_rotation(Quat::from_rotation_x(FRAC_PI_2))
                                .with_scale(Vec3::new(1.0, length, 1.0)),
                        ));
                    });
                for (index, side) in [-1.0f32, 1.0].into_iter().enumerate() {
                    parent
                        .spawn((
                            TrackSide(index),
                            Transform::from_xyz(side * TRACK_X, TRACK_Y, 0.0),
                            Visibility::default(),
                        ))
                        .with_children(|frame| {
                            // Блоки зеркальны: правый развёрнут на пол-оборота, катками наружу.
                            let yaw = if side > 0.0 { PI } else { 0.0 };
                            let ground = Transform::from_rotation(Quat::from_rotation_y(yaw));
                            models.spawn_visual(frame, "enemy_tank_track", ground, |_| {});
                            for i in 0..GROUSERS {
                                frame.spawn((
                                    Grouser(i),
                                    Mesh3d(assets.grouser.clone()),
                                    MeshMaterial3d(assets.grouser_material.clone()),
                                    Transform::default(),
                                ));
                            }
                        });
                }
            });
    }
}

/// Лазерная установка из примитивов: коробка на кольце, излучатель вперёд (-Z) и линза на его конце.
fn spawn_laser_spare(spare: &mut crate::models::FallbackSpawner, assets: &TankLookAssets, top: &Top) {
    spare.spawn((
        Mesh3d(assets.laser_body.clone()),
        MeshMaterial3d(assets.hull_material.clone()),
        Transform::from_xyz(0.0, LASER_BODY.y * 0.5, 0.1),
    ));
    let length = top.muzzle_forward - 0.2;
    spare.spawn((
        Mesh3d(assets.laser_emitter.clone()),
        MeshMaterial3d(assets.grouser_material.clone()),
        Transform::from_xyz(0.0, top.muzzle_up, -0.2 - length * 0.5)
            .with_rotation(Quat::from_rotation_x(FRAC_PI_2))
            .with_scale(Vec3::new(1.0, length, 1.0)),
    ));
    spare.spawn((
        Mesh3d(assets.laser_lens.clone()),
        MeshMaterial3d(assets.lens_material.clone()),
        Transform::from_xyz(0.0, top.muzzle_up, -top.muzzle_forward),
    ));
}

/// Поворачивает башню по курсу пушки и показывает линию прицела.
fn pose_turrets(
    tanks: Query<(&TankGun, &Children)>,
    mut turrets: Query<(&mut Transform, &Children), With<Turret>>,
    mut lines: Query<&mut Visibility, With<AimLine>>,
) {
    for (gun, children) in &tanks {
        for child in children.iter() {
            let Ok((mut transform, parts)) = turrets.get_mut(child) else {
                continue;
            };
            transform.rotation = Quat::from_rotation_y(gun.yaw);
            for part in parts.iter() {
                if let Ok(mut visibility) = lines.get_mut(part) {
                    *visibility = if gun.aiming {
                        Visibility::Visible
                    } else {
                        Visibility::Hidden
                    };
                }
            }
        }
    }
}

/// Считает путь лент по движению танка и раскладывает грунтозацепы по контуру.
fn run_tracks(
    mut tanks: Query<(&Transform, &GroundVehicle, &mut TrackDrive, &Children)>,
    sides: Query<(&TrackSide, &Children)>,
    mut grousers: Query<(&Grouser, &mut Transform), Without<TrackDrive>>,
) {
    let length = track_length(TRACK_HALF_STRAIGHT, TRACK_RADIUS);
    let spacing = length / GROUSERS as f32;
    for (transform, vehicle, mut drive, children) in &mut tanks {
        let position = transform.translation;
        if let Some((last_position, last_heading)) = drive.last {
            let step = (position - last_position).dot(vehicle.forward());
            let turn = (vehicle.heading - last_heading + PI).rem_euclid(2.0 * PI) - PI;
            if step.abs() < MAX_STEP {
                // Путь лент — в метрах мира, контур — в системе танка, растянутой на `TANK_SCALE`.
                let travel = track_travel(step, turn, TRACK_X * TANK_SCALE);
                drive.travel[0] += travel[0] / TANK_SCALE;
                drive.travel[1] += travel[1] / TANK_SCALE;
            }
        }
        drive.last = Some((position, vehicle.heading));

        for child in children.iter() {
            let Ok((side, parts)) = sides.get(child) else {
                continue;
            };
            let travel = drive.travel[side.0];
            for part in parts.iter() {
                let Ok((grouser, mut transform)) = grousers.get_mut(part) else {
                    continue;
                };
                let u = grouser.0 as f32 * spacing + travel;
                let (point, normal) = track_point(u, TRACK_HALF_STRAIGHT, TRACK_RADIUS);
                let normal = Vec3::new(0.0, normal.y, normal.x);
                transform.translation =
                    Vec3::new(0.0, point.y, point.x) + normal * GROUSER_SIZE.y * 0.5;
                transform.rotation = Quat::from_rotation_arc(Vec3::Y, normal);
            }
        }
    }
}
