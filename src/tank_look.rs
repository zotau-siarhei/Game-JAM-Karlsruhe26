//! Облик тяжёлого танка: корпус, башня и два гусеничных блока из моделей Meshy, бегущие грунтозацепы.
//!
//! Танк собран из частей, чтобы башня поворачивалась отдельно от корпуса, а гусеницы бежали по
//! ходу машины. Модели — `assets/models/enemy_tank_hull.glb`, `enemy_tank_turret.glb` и
//! `enemy_tank_track.glb`. Логика танка (`enemy::drive_tanks`) пишет курс башни в [`TankGun`],
//! этот модуль только показывает его.
//!
//! Все размеры — в системе координат танка, которая растянута на `enemy::TANK_SCALE`.
//! Перёд танка — -Z.

use std::f32::consts::{FRAC_PI_2, PI};

use bevy::prelude::*;

use crate::enemy::{RIDE_HEIGHT, TANK_FIRE_RANGE, TANK_SCALE};
use crate::rover_look::{track_length, track_point, track_travel};
use crate::vehicle::GroundVehicle;

/// Модель корпуса.
const HULL_MODEL: &str = "models/enemy_tank_hull.glb";
/// Модель башни с пушкой.
const TURRET_MODEL: &str = "models/enemy_tank_turret.glb";
/// Модель гусеничного блока.
const TRACK_MODEL: &str = "models/enemy_tank_track.glb";

/// Высота земли под центром танка.
const GROUND_Y: f32 = -RIDE_HEIGHT;

/// Масштаб модели корпуса: в модели 0,95 × 0,36 × 0,83, длина вдоль X.
const HULL_SCALE: f32 = 1.6;
/// Поворот модели корпуса: её перёд (низкий скошенный конец) смотрит на -X, а нужно на -Z, рад.
const HULL_YAW: f32 = -FRAC_PI_2;
/// Высота центра модели корпуса над землёй: днище на середине гусениц.
const HULL_Y: f32 = 0.5;

/// Масштаб модели башни: в модели купол диаметром 0,96 и высотой 0,56.
const TURRET_SCALE: f32 = 0.95;
/// Поворот модели башни: ствол в модели смотрит на -X, а нужно на -Z, рад.
const TURRET_YAW: f32 = -FRAC_PI_2;
/// Высота оси поворота башни (низа купола) над землёй: на крыше корпуса.
const TURRET_Y: f32 = 0.77;
/// Сдвиг центра модели башни от оси поворота: модель стоит низом на оси.
const TURRET_MODEL_OFFSET: Vec3 = Vec3::new(0.0, 0.282 * TURRET_SCALE, 0.0);
/// Дульный срез от оси башни вперёд.
const MUZZLE_FORWARD: f32 = 0.478 * TURRET_SCALE;
/// Высота оси ствола над осью башни.
const MUZZLE_UP: f32 = 0.4 * TURRET_SCALE;

/// Масштаб модели гусеничного блока: в модели 0,94 × 0,28 × 0,24, длина вдоль X.
/// Модель упрощена в Blender с 25 тыс. до 2 тыс. треугольников.
const TRACK_SCALE: f32 = 1.9;
/// Поворот модели гусеничного блока, чтобы она легла вдоль Z, рад.
const TRACK_YAW: f32 = FRAC_PI_2;
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
/// Размер грунтозацепа: поперёк ленты (лента в модели шириной 0,30), толщина, вдоль ленты.
const GROUSER_SIZE: Vec3 = Vec3::new(0.28, 0.03, 0.07);
/// Цвет грунтозацепов: тёмная сталь, чтобы бег ленты был заметен на жёлтых зацепах модели.
const GROUSER_COLOR: Color = Color::srgb(0.3, 0.3, 0.34);

/// Больше такого сдвига за кадр — это перенос танка, а не езда, м.
const MAX_STEP: f32 = 3.0;

/// Плагин облика тяжёлого танка.
pub struct TankLookPlugin;

impl Plugin for TankLookPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, create_assets)
            .add_systems(Update, (attach_look, pose_turrets, run_tracks).chain());
    }
}

/// Пушка танка: курс башни относительно корпуса, рад (ноль — по носу, плюс — влево),
/// и идёт ли прицеливание (видна линия прицела).
#[derive(Component, Default)]
pub struct TankGun {
    pub yaw: f32,
    pub aiming: bool,
}

/// Направление ствола в мире и точка дульного среза.
pub fn muzzle(transform: &Transform, vehicle: &GroundVehicle, gun: &TankGun) -> (Vec3, Vec3) {
    let direction = crate::vehicle::forward_of(vehicle.heading + gun.yaw);
    let origin = transform.translation
        + Vec3::Y * (TURRET_Y + GROUND_Y + MUZZLE_UP) * TANK_SCALE
        + direction * MUZZLE_FORWARD * TANK_SCALE;
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
    hull: Handle<WorldAsset>,
    turret: Handle<WorldAsset>,
    track: Handle<WorldAsset>,
    grouser: Handle<Mesh>,
    grouser_material: Handle<StandardMaterial>,
    aim_line: Handle<Mesh>,
    aim_material: Handle<StandardMaterial>,
}

fn create_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(TankLookAssets {
        hull: asset_server.load(GltfAssetLabel::Scene(0).from_asset(HULL_MODEL)),
        turret: asset_server.load(GltfAssetLabel::Scene(0).from_asset(TURRET_MODEL)),
        track: asset_server.load(GltfAssetLabel::Scene(0).from_asset(TRACK_MODEL)),
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
    });
}

/// Ставит на новый танк корпус, башню с линией прицела, гусеничные блоки и грунтозацепы.
fn attach_look(
    mut commands: Commands,
    tanks: Query<Entity, Added<TankGun>>,
    assets: Res<TankLookAssets>,
) {
    for tank in &tanks {
        commands
            .entity(tank)
            .insert(TrackDrive::default())
            .with_children(|parent| {
                parent.spawn((
                    WorldAssetRoot(assets.hull.clone()),
                    Transform::from_xyz(0.0, GROUND_Y + HULL_Y, 0.0)
                        .with_rotation(Quat::from_rotation_y(HULL_YAW))
                        .with_scale(Vec3::splat(HULL_SCALE)),
                ));
                parent
                    .spawn((
                        Turret,
                        Transform::from_xyz(0.0, GROUND_Y + TURRET_Y, 0.0),
                        Visibility::default(),
                    ))
                    .with_children(|turret| {
                        turret.spawn((
                            WorldAssetRoot(assets.turret.clone()),
                            Transform::from_translation(TURRET_MODEL_OFFSET)
                                .with_rotation(Quat::from_rotation_y(TURRET_YAW))
                                .with_scale(Vec3::splat(TURRET_SCALE)),
                        ));
                        let length = TANK_FIRE_RANGE / TANK_SCALE;
                        turret.spawn((
                            AimLine,
                            Mesh3d(assets.aim_line.clone()),
                            MeshMaterial3d(assets.aim_material.clone()),
                            bevy::light::NotShadowCaster,
                            Visibility::Hidden,
                            Transform::from_xyz(0.0, MUZZLE_UP, -MUZZLE_FORWARD - length * 0.5)
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
                            let yaw = TRACK_YAW + if side > 0.0 { PI } else { 0.0 };
                            frame.spawn((
                                WorldAssetRoot(assets.track.clone()),
                                Transform::from_rotation(Quat::from_rotation_y(yaw))
                                    .with_scale(Vec3::splat(TRACK_SCALE)),
                            ));
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
