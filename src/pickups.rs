//! Подбираемые предметы: ящики с оружием на парашютах и трофеи, выпавшие из врагов.
//!
//! Предмет подбирается проездом. Ящики падают раз в несколько секунд в кольце вокруг лунохода,
//! чаще у мест разрушений. Содержимое открывается по времени забега. Враг роняет при гибели
//! оружие, которое носит по своему типу (`CarriedWeapon` ставит `enemy.rs`).
//!
//! Золотой ящик улучшения ([`GoldenCrate`]) падает с неба реже обычных, выпадает из танка и всегда
//! из босса ([`SpawnUpgrade`]). Подбор шлёт `upgrades::UpgradePicked`, уровень выбирает `upgrades`.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::arsenal::{Inventory, spec};
use crate::buildings::BuildingDestroyed;
use crate::camera::GameCamera;
use crate::combat::{Died, Health, SurvivalTime};
use crate::enemy::HeavyTank;
use crate::frame::{GameSet, RunMessages};
use crate::models::Models;
use crate::rng::GameRng;
use crate::rover::Rover;
use crate::run::{RunScoped, WorldRebuilt};
use crate::subsidence::TerrainChanged;
use crate::terrain::Terrain;
use crate::upgrades::UpgradePicked;
use crate::weapons::CurrentWeapon;

/// Радиус подбора, м: от центра лунохода до центра ящика, ящик крупный.
const PICKUP_RADIUS: f32 = 2.8;

/// Как часто появляется ящик и сколько их может лежать рядом с луноходом.
const CRATE_INTERVAL: f32 = 12.0;
const MAX_CRATES: usize = 4;

/// Ящики ближе этого к луноходу входят в предел [`MAX_CRATES`], м.
const CRATE_COUNT_RADIUS: f32 = 70.0;

/// Ящик дальше этого от лунохода убирается: игрок уехал, а ящик не держит место нового, м.
const CRATE_FORGET_RADIUS: f32 = 150.0;

/// Кольцо появления ящиков вокруг лунохода, м.
const CRATE_RING: (f32, f32) = (15.0, 40.0);

/// Высота, с которой падает ящик, и скорость спуска на парашюте.
const DROP_HEIGHT: f32 = 22.0;
const PARACHUTE_SPEED: f32 = 4.0;

/// Вес места у разрушения и радиус «у разрушения».
const RUIN_WEIGHT: f32 = 3.0;
const RUIN_RADIUS: f32 = 6.0;

/// Сколько последних мест разрушений помнить.
const RUIN_MEMORY: usize = 30;

/// Золотой ящик с неба: не раньше этой секунды забега, не чаще раза в столько секунд и с таким
/// шансом на каждый падающий ящик.
const GOLDEN_FROM: f32 = 60.0;
const GOLDEN_GAP: f32 = 45.0;
const GOLDEN_CHANCE: f32 = 0.12;

/// Шанс золотого ящика из погибшего тяжёлого танка.
const GOLDEN_FROM_TANK: f32 = 0.4;

/// Искр вокруг золотого ящика и радиус их кружения, м.
const SPARKLES: usize = 4;
const SPARKLE_ORBIT: f32 = 1.8;

/// Размер ящика с оружием, м: заметен рядом с луноходом (3,6 × 3,0 м).
const CRATE_SIZE: f32 = 2.0;
/// Размер золотого ящика, м: чуть крупнее обычного.
const GOLDEN_SIZE: f32 = 2.2;
/// Размер светящегося трофея, м.
const LOOT_SIZE: f32 = 1.2;
/// Парашют над ящиком в долях размера ящика: высота центра купола и его размеры.
const CANOPY_Y: f32 = 1.6;
const CANOPY_SCALE: Vec3 = Vec3::new(1.1, 0.55, 1.1);

/// Трофей врага живёт и мигает последние секунды.
const LOOT_SECONDS: f32 = 20.0;
const LOOT_BLINK: f32 = 5.0;

/// Подключает подбираемые предметы.
pub struct PickupsPlugin;

impl Plugin for PickupsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CrateDirector>()
            .add_run_message::<SpawnLoot>()
            .add_run_message::<SpawnCrate>()
            .init_resource::<CrateLook>()
            .add_run_message::<SpawnUpgrade>()
            .add_run_message::<WeaponPicked>()
            .add_systems(Startup, create_pickup_assets)
            .add_systems(
                Update,
                (
                    reset_crates.in_set(GameSet::Reset),
                    (remember_ruins, drop_crates, spawn_loot, spawn_tossed_crates, spawn_upgrades, descend, collect)
                        .chain()
                        .in_set(GameSet::Simulation),
                    // Враг ещё существует: трофей берёт оружие с него.
                    drop_loot.in_set(GameSet::Death),
                    (float_popups, spin_sparkles, split_crate_model).in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Подбираемый предмет.
#[derive(Component, Debug, Clone, Copy)]
pub struct Pickup {
    pub weapon: CurrentWeapon,
    pub amount: u16,
    /// Сколько осталось жить. `None` — живёт до подбора.
    pub expires: Option<f32>,
}

/// Ящик ещё на парашюте.
#[derive(Component)]
struct Parachuting {
    canopy: Entity,
    sway: f32,
}

/// Ящик лежит на земле.
#[derive(Component)]
struct Crate;

/// Ящик летит по дуге: выброшен, например, из гибнущего босса.
#[derive(Component)]
struct Tossed {
    velocity: Vec3,
}

/// Гравитация для выброшенных ящиков, м/с².
const TOSS_GRAVITY: f32 = 20.0;

/// Просьба выбросить ящик с оружием: из `position` со скоростью `velocity`, без парашюта.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpawnCrate {
    pub position: Vec3,
    pub velocity: Vec3,
    pub weapon: CurrentWeapon,
}

/// Золотой ящик улучшения: подбор поднимает уровень оружия, а не даёт заряды.
#[derive(Component)]
pub struct GoldenCrate;

/// Искра, кружащая вокруг золотого ящика.
#[derive(Component)]
struct Sparkle {
    phase: f32,
}

/// Оружие, которое носит враг.
#[derive(Component)]
pub struct CarriedWeapon {
    pub weapon: CurrentWeapon,
}

/// Подобран ящик оружия: какого вида и впервые ли за забег. По нему звучат звон и реплика.
#[derive(Message, Debug, Clone, Copy)]
pub struct WeaponPicked {
    pub weapon: CurrentWeapon,
    /// Этого вида у лунохода не было.
    pub new: bool,
}

/// Всплывающая надпись подбора.
#[derive(Component)]
pub struct Popup {
    world: Vec3,
    elapsed: f32,
}

#[derive(Resource, Default)]
struct CrateDirector {
    timer: f32,
    /// Секунд с последнего золотого ящика с неба.
    since_golden: f32,
    /// Последние места разрушений.
    ruins: Vec<Vec3>,
}

#[derive(Resource)]
struct PickupAssets {
    cube: Handle<Mesh>,
    canopy: Handle<Mesh>,
    crate_material: Handle<StandardMaterial>,
    gold_material: Handle<StandardMaterial>,
    sparkle_material: Handle<StandardMaterial>,
    canopy_material: Handle<StandardMaterial>,
    glow: Vec<Handle<StandardMaterial>>,
    /// Крышка ящика, окрашенная в цвет оружия, по порядку `CurrentWeapon::ALL`.
    lids: Vec<Handle<StandardMaterial>>,
}

fn create_pickup_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let glow = CurrentWeapon::ALL
        .iter()
        .map(|w| {
            let c = spec(*w).color;
            materials.add(StandardMaterial {
                base_color: c,
                emissive: c.to_linear() * 3.0,
                unlit: true,
                ..default()
            })
        })
        .collect();
    // Крышка — краска цвета оружия: не светится, чуть темнее полосы, чтобы полоса читалась.
    let lids = CurrentWeapon::ALL
        .iter()
        .map(|w| {
            let c = spec(*w).color.to_srgba();
            materials.add(StandardMaterial {
                base_color: Color::srgb(c.red * 0.85, c.green * 0.85, c.blue * 0.85),
                perceptual_roughness: 0.55,
                ..default()
            })
        })
        .collect();
    commands.insert_resource(PickupAssets {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        canopy: meshes.add(Sphere::new(1.0).mesh().uv(12, 6)),
        crate_material: materials.add(Color::srgb(0.55, 0.38, 0.2)),
        // Золото светится через свечение камеры: яркость выше единицы.
        gold_material: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.78, 0.2),
            emissive: LinearRgba::rgb(2.2, 1.5, 0.3),
            metallic: 0.9,
            perceptual_roughness: 0.25,
            ..default()
        }),
        sparkle_material: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.95, 0.6),
            emissive: LinearRgba::rgb(6.0, 5.0, 1.5),
            unlit: true,
            ..default()
        }),
        canopy_material: materials.add(Color::srgb(0.95, 0.95, 0.95)),
        glow,
        lids,
    });
}

fn glow_of(assets: &PickupAssets, weapon: CurrentWeapon) -> Handle<StandardMaterial> {
    assets.glow[weapon_index(weapon)].clone()
}

fn lid_of(assets: &PickupAssets, weapon: CurrentWeapon) -> Handle<StandardMaterial> {
    assets.lids[weapon_index(weapon)].clone()
}

fn weapon_index(weapon: CurrentWeapon) -> usize {
    CurrentWeapon::ALL.iter().position(|w| *w == weapon).unwrap_or(0)
}

/// Крышка модели `pickup_crate` по высоте в осях файла: верхняя плоскость (0,29–0,31) с кромкой.
/// Кольцо строп выше и остаётся в корпусе.
const LID_BAND: (f32, f32) = (0.24, 0.345);
/// Насколько светлее текстуры краска крышки: текстура ящика тёмная (около 0,1 в линейном цвете),
/// цвет оружия на ней множится, чтобы крышка вышла цвета оружия с рисунком досок.
const LID_PAINT: f32 = 5.0;

/// Модель ящика, разделённая на корпус и крышку: крышка рисуется текстурой модели в цвет оружия.
#[derive(Resource, Default)]
struct CrateLook {
    parts: Option<CrateParts>,
    /// Разделить не удалось: остаётся модель с пластиной краски.
    failed: bool,
}

struct CrateParts {
    body: Handle<Mesh>,
    lid: Handle<Mesh>,
    material: Handle<StandardMaterial>,
    /// Крышки по порядку `CurrentWeapon::ALL`.
    lids: Vec<Handle<StandardMaterial>>,
    /// Растяжка модели из `models.json`.
    entry: Transform,
}

/// Делит модель ящика на корпус и крышку, когда модель загрузилась и её сцена появилась в мире
/// (материал берётся у меша сцены).
#[allow(clippy::too_many_arguments)]
fn split_crate_model(
    mut look: ResMut<CrateLook>,
    bank: Res<crate::models::ModelBank>,
    gltfs: Res<Assets<bevy::gltf::Gltf>>,
    gltf_meshes: Res<Assets<bevy::gltf::GltfMesh>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    shown: Query<(&Mesh3d, &MeshMaterial3d<StandardMaterial>)>,
) {
    if look.parts.is_some() || look.failed {
        return;
    }
    let Some(gltf) = bank.ready_gltf("pickup_crate").and_then(|h| gltfs.get(&h)) else {
        return;
    };
    let Some(entry) = bank.entry_transform("pickup_crate") else {
        return;
    };
    let Some(primitive) = gltf.meshes.first().and_then(|m| gltf_meshes.get(m)).and_then(|m| m.primitives.first()) else {
        return;
    };
    let Some(material) = shown.iter().find(|(m, _)| m.0 == primitive.mesh).map(|(_, m)| m.0.clone()) else {
        return;
    };
    let in_lid = |c: [Vec3; 3]| c.iter().all(|p| (LID_BAND.0..=LID_BAND.1).contains(&p.y));
    let Some((lid, body)) = meshes.get(&primitive.mesh).and_then(|mesh| crate::fragments::split_mesh(mesh, in_lid)) else {
        warn!("Ящик: модель не разделена на корпус и крышку, крышка — пластиной");
        look.failed = true;
        return;
    };
    let source = materials.get(&material).cloned();
    let lids = CurrentWeapon::ALL
        .iter()
        .map(|w| {
            let c = spec(*w).color.to_linear();
            let paint = match source.clone() {
                // Текстура модели под краской: видны доски и уголки.
                Some(mut copy) => {
                    copy.base_color = Color::LinearRgba(LinearRgba::rgb(c.red * LID_PAINT, c.green * LID_PAINT, c.blue * LID_PAINT));
                    copy
                }
                None => StandardMaterial {
                    base_color: spec(*w).color,
                    perceptual_roughness: 0.55,
                    ..default()
                },
            };
            materials.add(paint)
        })
        .collect();
    look.parts = Some(CrateParts {
        body: meshes.add(body),
        lid: meshes.add(lid),
        material,
        lids,
        entry,
    });
}

/// Верх крышки модели `pickup_crate` в масштабе ящика: плоскость на высоте 0,29–0,31 в осях
/// модели при масштабе 1,05; выше неё только кольцо строп. У запасного куба — его верх.
const MODEL_LID_TOP: f32 = 0.325;
/// Крышка — с отступом от края: тёмные стальные уголки модели остаются видны.
const LID_SIZE: f32 = 0.78;

fn reset_crates(mut rebuilt: MessageReader<WorldRebuilt>, mut director: ResMut<CrateDirector>) {
    if rebuilt.read().last().is_some() {
        *director = CrateDirector::default();
    }
}

fn remember_ruins(
    mut director: ResMut<CrateDirector>,
    mut terrain: MessageReader<TerrainChanged>,
    mut houses: MessageReader<BuildingDestroyed>,
) {
    let places: Vec<Vec3> = terrain
        .read()
        .map(|t| t.center)
        .chain(houses.read().map(|h| h.position))
        .collect();
    for p in places {
        director.ruins.push(p);
    }
    let excess = director.ruins.len().saturating_sub(RUIN_MEMORY);
    director.ruins.drain(..excess);
}

/// Виды, которые могут лежать в ящике на этой секунде забега.
pub fn crate_weapons(seconds: f32) -> Vec<CurrentWeapon> {
    let mut list = vec![CurrentWeapon::Mortar, CurrentWeapon::Rockets];
    if seconds >= 60.0 {
        list.extend([CurrentWeapon::HomingRockets, CurrentWeapon::Laser]);
    }
    if seconds >= 120.0 {
        list.push(CurrentWeapon::Airstrike);
    }
    list
}

/// Ящик на парашюте раз в [`CRATE_INTERVAL`], если ящиков меньше предела.
#[allow(clippy::too_many_arguments)]
fn drop_crates(
    mut commands: Commands,
    mut models: Models,
    time: Res<Time>,
    survival: Res<SurvivalTime>,
    inventory: Res<Inventory>,
    terrain: Res<Terrain>,
    assets: Res<PickupAssets>,
    look: Res<CrateLook>,
    mut director: ResMut<CrateDirector>,
    mut dice: ResMut<GameRng>,
    crates: Query<(Entity, &Transform), With<Crate>>,
    rover: Single<&Transform, (With<Rover>, Without<Crate>)>,
) {
    // Далёкие ящики убираются, в предел входят только ближние.
    let mut near = 0;
    for (entity, transform) in &crates {
        let distance = transform.translation.with_y(0.0).distance(rover.translation.with_y(0.0));
        if distance > CRATE_FORGET_RADIUS {
            commands.entity(entity).despawn();
        } else if distance <= CRATE_COUNT_RADIUS {
            near += 1;
        }
    }
    director.timer += time.delta_secs();
    director.since_golden += time.delta_secs();
    if director.timer < CRATE_INTERVAL || near >= MAX_CRATES {
        return;
    }
    director.timer = 0.0;

    // Место: из десяти кандидатов, у разрушений вес больше.
    let candidates: Vec<(Vec3, f32)> = (0..10)
        .map(|_| {
            let angle = dice.range(0.0, std::f32::consts::TAU);
            let distance = dice.range(CRATE_RING.0, CRATE_RING.1);
            let p = rover.translation + Vec3::new(angle.cos(), 0.0, angle.sin()) * distance;
            let near_ruin = director.ruins.iter().any(|r| r.with_y(0.0).distance(p.with_y(0.0)) < RUIN_RADIUS);
            (p, if near_ruin { RUIN_WEIGHT } else { 1.0 })
        })
        .collect();
    let total: f32 = candidates.iter().map(|c| c.1).sum();
    let mut roll = dice.range(0.0, total);
    let place = candidates
        .iter()
        .find(|c| {
            roll -= c.1;
            roll <= 0.0
        })
        .unwrap_or(&candidates[0])
        .0;

    let ground = terrain.height_at(place.x, place.z);
    if golden_due(survival.0, director.since_golden, dice.range(0.0, 1.0)) {
        director.since_golden = 0.0;
        spawn_golden(&mut commands, &mut models, &assets, Vec3::new(place.x, ground + DROP_HEIGHT, place.z), true);
        return;
    }

    // Содержимое: новые для лунохода виды выпадают чаще.
    let kinds = crate_weapons(survival.0);
    let weights: Vec<f32> = kinds.iter().map(|w| if inventory.owns(*w) { 1.0 } else { 3.0 }).collect();
    let mut roll = dice.range(0.0, weights.iter().sum());
    let weapon = kinds
        .iter()
        .zip(&weights)
        .find(|(_, w)| {
            roll -= **w;
            roll <= 0.0
        })
        .map_or(kinds[0], |(k, _)| *k);

    let canopy = commands
        .spawn((
            Mesh3d(assets.canopy.clone()),
            MeshMaterial3d(assets.canopy_material.clone()),
            NotShadowCaster,
            Transform::from_xyz(0.0, CANOPY_Y, 0.0).with_scale(CANOPY_SCALE),
        ))
        .id();
    let at = Vec3::new(place.x, ground + DROP_HEIGHT, place.z);
    let id = spawn_crate(&mut commands, &mut models, &assets, &look, at, weapon);
    commands.entity(id).insert(Parachuting { canopy, sway: 0.0 }).add_child(canopy);
}

/// Выброшенные ящики: летят по дуге и ложатся, где упали.
fn spawn_tossed_crates(
    mut commands: Commands,
    mut models: Models,
    assets: Res<PickupAssets>,
    look: Res<CrateLook>,
    mut requests: MessageReader<SpawnCrate>,
) {
    for request in requests.read() {
        let id = spawn_crate(&mut commands, &mut models, &assets, &look, request.position, request.weapon);
        commands.entity(id).insert(Tossed {
            velocity: request.velocity,
        });
    }
}

/// Ящик с оружием: модель с крышкой в цвет оружия. Пока модель не разделена на корпус и
/// крышку — модель или куб с пластиной краски и светящейся полосой сверху.
fn spawn_crate(
    commands: &mut Commands,
    models: &mut Models,
    assets: &PickupAssets,
    look: &CrateLook,
    at: Vec3,
    weapon: CurrentWeapon,
) -> Entity {
    if let Some(parts) = &look.parts {
        let place = Transform::from_xyz(0.0, -0.5, 0.0) * parts.entry;
        return commands
            .spawn((
                Pickup {
                    weapon,
                    amount: spec(weapon).per_crate,
                    expires: None,
                },
                Crate,
                RunScoped,
                Transform::from_translation(at).with_scale(Vec3::splat(CRATE_SIZE)),
                Visibility::default(),
            ))
            .with_children(|c| {
                c.spawn((Mesh3d(parts.body.clone()), MeshMaterial3d(parts.material.clone()), place));
                c.spawn((Mesh3d(parts.lid.clone()), MeshMaterial3d(parts.lids[weapon_index(weapon)].clone()), place));
            })
            .id();
    }
    commands
        .spawn((
            Pickup {
                weapon,
                amount: spec(weapon).per_crate,
                expires: None,
            },
            Crate,
            RunScoped,
            Transform::from_translation(at).with_scale(Vec3::splat(CRATE_SIZE)),
            Visibility::default(),
        ))
        .with_children(|c| {
            // Ящик — единичный куб в масштабе ящика; модель стоит на его дне.
            let lid_top = if models.has("pickup_crate") { MODEL_LID_TOP } else { 0.5 };
            models.spawn_visual(c, "pickup_crate", Transform::from_xyz(0.0, -0.5, 0.0), |spare| {
                spare.spawn((Mesh3d(assets.cube.clone()), MeshMaterial3d(assets.crate_material.clone())));
            });
            // Крышка в цвет оружия и светящаяся полоса того же цвета поверх неё.
            c.spawn((
                Mesh3d(assets.cube.clone()),
                MeshMaterial3d(lid_of(&assets, weapon)),
                Transform::from_xyz(0.0, lid_top + 0.02, 0.0).with_scale(Vec3::new(LID_SIZE, 0.05, LID_SIZE)),
            ));
            c.spawn((
                Mesh3d(assets.cube.clone()),
                MeshMaterial3d(glow_of(&assets, weapon)),
                Transform::from_xyz(0.0, lid_top + 0.05, 0.0).with_scale(Vec3::new(LID_SIZE * 0.95, 0.03, 0.14)),
            ));
        })
        .id()
}

/// Ящики спускаются на парашюте или летят по дуге, все предметы стоят на рельефе и не
/// вращаются, трофеи мигают перед исчезновением.
fn descend(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut items: Query<(
        Entity,
        &mut Transform,
        &mut Pickup,
        Option<&mut Parachuting>,
        Option<&mut Tossed>,
        &mut Visibility,
    )>,
) {
    let dt = time.delta_secs();
    for (entity, mut transform, mut pickup, chute, tossed, mut visibility) in &mut items {
        let p = transform.translation;
        let ground = terrain.height_at(p.x, p.z) + transform.scale.y * 0.5;
        if let Some(mut tossed) = tossed {
            tossed.velocity.y -= TOSS_GRAVITY * dt;
            transform.translation += tossed.velocity * dt;
            let p = transform.translation;
            let ground = terrain.height_at(p.x, p.z) + transform.scale.y * 0.5;
            if transform.translation.y <= ground && tossed.velocity.y <= 0.0 {
                transform.translation.y = ground;
                commands.entity(entity).try_remove::<Tossed>();
            }
            continue;
        }
        if let Some(mut chute) = chute {
            chute.sway += dt;
            transform.translation.y -= PARACHUTE_SPEED * dt;
            transform.rotation = Quat::from_rotation_z((chute.sway * 2.0).sin() * 0.15);
            if transform.translation.y <= ground {
                commands.entity(chute.canopy).try_despawn();
                commands.entity(entity).try_remove::<Parachuting>();
                transform.rotation = Quat::IDENTITY;
            }
            continue;
        }
        transform.translation.y = ground;
        if let Some(left) = pickup.expires.as_mut() {
            *left -= dt;
            if *left <= 0.0 {
                commands.entity(entity).despawn();
                continue;
            }
            let blink = *left < LOOT_BLINK && (*left * 8.0).sin() < 0.0;
            *visibility = if blink { Visibility::Hidden } else { Visibility::Inherited };
        }
    }
}

/// Подбор проездом.
fn collect(
    mut commands: Commands,
    mut inventory: ResMut<Inventory>,
    items: Query<(Entity, &Transform, &Pickup, Has<GoldenCrate>), (Without<Parachuting>, Without<Tossed>)>,
    rover: Single<(&Transform, &Health), With<Rover>>,
    mut upgrades: MessageWriter<UpgradePicked>,
    mut picked: MessageWriter<WeaponPicked>,
) {
    let (rover, health) = *rover;
    if health.is_dead() {
        return;
    }
    for (entity, transform, pickup, golden) in &items {
        if transform.translation.with_y(0.0).distance(rover.translation.with_y(0.0)) > PICKUP_RADIUS {
            continue;
        }
        if golden {
            // Надпись, звук и реплику даёт `upgrades::WeaponUpgraded`.
            commands.entity(entity).despawn();
            upgrades.write(UpgradePicked {
                position: rover.translation,
            });
            continue;
        }
        let new = !inventory.owns(pickup.weapon);
        let added = inventory.add(pickup.weapon, pickup.amount);
        if !new && added == 0 {
            // Боезапас полный: предмет остаётся лежать до следующего раза.
            continue;
        }
        commands.entity(entity).despawn();
        picked.write(WeaponPicked {
            weapon: pickup.weapon,
            new,
        });
        let label = if new {
            format!("NEW: {}", pickup.weapon.name())
        } else {
            format!("+{} {}", added, pickup.weapon.name())
        };
        commands.spawn((
            Popup {
                world: rover.translation + Vec3::Y * 2.0,
                elapsed: 0.0,
            },
            RunScoped,
            Text::new(label),
            TextFont {
                font_size: FontSize::Px(20.0),
                ..default()
            },
            TextColor(spec(pickup.weapon).color),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
        ));
    }
}

/// Надпись подбора плывёт вверх и тает за секунду.
fn float_popups(
    mut commands: Commands,
    time: Res<Time>,
    camera: Single<(&Camera, &GlobalTransform), With<GameCamera>>,
    mut popups: Query<(Entity, &mut Popup, &mut Node, &mut TextColor)>,
) {
    let (camera, camera_transform) = *camera;
    for (entity, mut popup, mut node, mut color) in &mut popups {
        popup.elapsed += time.delta_secs();
        if popup.elapsed >= 1.0 {
            commands.entity(entity).despawn();
            continue;
        }
        if let Ok(screen) = camera.world_to_viewport(camera_transform, popup.world) {
            node.left = Val::Px(screen.x - 60.0);
            node.top = Val::Px(screen.y - 40.0 - popup.elapsed * 50.0);
        }
        color.0.set_alpha(1.0 - popup.elapsed);
    }
}

/// Погибший враг с оружием роняет светящийся трофей.
fn drop_loot(
    mut commands: Commands,
    mut models: Models,
    mut died: MessageReader<Died>,
    assets: Res<PickupAssets>,
    mut dice: ResMut<GameRng>,
    carriers: Query<(&GlobalTransform, &CarriedWeapon, Has<HeavyTank>)>,
) {
    for death in died.read() {
        let Ok((transform, carried, tank)) = carriers.get(death.entity) else {
            continue;
        };
        if tank && dice.range(0.0, 1.0) < GOLDEN_FROM_TANK {
            // Рядом с трофеем, чтобы не слиплись.
            spawn_golden(&mut commands, &mut models, &assets, transform.translation() + Vec3::new(2.5, 0.0, 0.0), false);
        }
        if carried.weapon == CurrentWeapon::MachineGun {
            continue;
        }
        spawn_loot_item(&mut commands, &mut models, &assets, transform.translation(), carried.weapon);
    }
}

/// Просьба положить светящийся трофей с оружием: так босс оставляет ящики после гибели.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpawnLoot {
    pub position: Vec3,
    pub weapon: CurrentWeapon,
}

fn spawn_loot(
    mut commands: Commands,
    mut models: Models,
    mut requests: MessageReader<SpawnLoot>,
    assets: Res<PickupAssets>,
) {
    for r in requests.read() {
        spawn_loot_item(&mut commands, &mut models, &assets, r.position, r.weapon);
    }
}

/// Модель трофея по оружию: снаряды мортиры, кассета ракет (и самонаводящихся), ячейка лазера.
fn loot_model(weapon: CurrentWeapon) -> Option<&'static str> {
    match weapon {
        CurrentWeapon::Mortar => Some("loot_mortar"),
        CurrentWeapon::Rockets | CurrentWeapon::HomingRockets => Some("loot_rockets"),
        CurrentWeapon::Laser => Some("loot_laser"),
        _ => None,
    }
}

/// Трофей — боекомплект погибшего врага: модель своего оружия, без модели — светящийся куб
/// цвета оружия. Лежит [`LOOT_SECONDS`], перед исчезновением мигает.
fn spawn_loot_item(commands: &mut Commands, models: &mut Models, assets: &PickupAssets, at: Vec3, weapon: CurrentWeapon) {
    let pickup = Pickup {
        weapon,
        amount: spec(weapon).per_crate,
        expires: Some(LOOT_SECONDS),
    };
    let cube = (Mesh3d(assets.cube.clone()), MeshMaterial3d(glow_of(assets, weapon)), NotShadowCaster);
    let Some(id) = loot_model(weapon) else {
        commands.spawn((pickup, RunScoped, cube, Transform::from_translation(at).with_scale(Vec3::splat(LOOT_SIZE)), Visibility::default()));
        return;
    };
    // Единичный объём: предмет стоит центром на полметра над землёй, модель — на его дне.
    commands
        .spawn((pickup, RunScoped, Transform::from_translation(at), Visibility::default()))
        .with_children(|c| {
            models.spawn_visual(c, id, Transform::from_xyz(0.0, -0.5, 0.0), |spare| {
                spare.spawn((cube.0, cube.1, cube.2, Transform::from_scale(Vec3::splat(LOOT_SIZE))));
            });
        });
}

/// Просьба положить золотой ящик улучшения: так босс оставляет его после гибели.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpawnUpgrade {
    pub position: Vec3,
}

fn spawn_upgrades(
    mut commands: Commands,
    mut models: Models,
    mut requests: MessageReader<SpawnUpgrade>,
    assets: Res<PickupAssets>,
) {
    for r in requests.read() {
        spawn_golden(&mut commands, &mut models, &assets, r.position, false);
    }
}

/// Пора ли сбросить с неба золотой ящик: после [`GOLDEN_FROM`] секунд забега, не чаще раза в
/// [`GOLDEN_GAP`] и с шансом [`GOLDEN_CHANCE`]; `roll` — случайное число от 0 до 1.
fn golden_due(survival: f32, since_golden: f32, roll: f32) -> bool {
    survival >= GOLDEN_FROM && since_golden >= GOLDEN_GAP && roll < GOLDEN_CHANCE
}

/// Золотой ящик с кружащими искрами. С парашютом — падает с неба, без — лежит на месте.
fn spawn_golden(commands: &mut Commands, models: &mut Models, assets: &PickupAssets, position: Vec3, parachute: bool) {
    let id = commands
        .spawn((
            Pickup {
                weapon: CurrentWeapon::MachineGun,
                amount: 0,
                expires: None,
            },
            GoldenCrate,
            Crate,
            RunScoped,
            Transform::from_translation(position).with_scale(Vec3::splat(GOLDEN_SIZE)),
            Visibility::default(),
        ))
        .with_children(|c| {
            models.spawn_visual(c, "pickup_golden", Transform::from_xyz(0.0, -0.5, 0.0), |spare| {
                spare.spawn((Mesh3d(assets.cube.clone()), MeshMaterial3d(assets.gold_material.clone())));
            });
            for i in 0..SPARKLES {
                c.spawn((
                    Sparkle {
                        phase: i as f32 / SPARKLES as f32 * std::f32::consts::TAU,
                    },
                    NotShadowCaster,
                    Mesh3d(assets.cube.clone()),
                    MeshMaterial3d(assets.sparkle_material.clone()),
                    Transform::from_scale(Vec3::splat(0.1)),
                ));
            }
        })
        .id();
    if parachute {
        let canopy = commands
            .spawn((
                Mesh3d(assets.canopy.clone()),
                MeshMaterial3d(assets.canopy_material.clone()),
                NotShadowCaster,
                Transform::from_xyz(0.0, CANOPY_Y, 0.0).with_scale(CANOPY_SCALE),
            ))
            .id();
        commands.entity(id).add_child(canopy).insert(Parachuting { canopy, sway: 0.0 });
    }
}

/// Искры кружат вокруг золотого ящика на разной высоте и мерцают размером.
fn spin_sparkles(time: Res<Time>, mut sparkles: Query<(&Sparkle, &mut Transform)>) {
    let t = time.elapsed_secs();
    // Искры — потомки масштабированного ящика: орбита в долях его размера.
    let orbit = SPARKLE_ORBIT / GOLDEN_SIZE;
    for (sparkle, mut transform) in &mut sparkles {
        let a = sparkle.phase + t * 2.2;
        transform.translation = Vec3::new(a.cos() * orbit, 0.3 + (a * 1.7).sin() * 0.35, a.sin() * orbit);
        transform.scale = Vec3::splat(0.07 + 0.06 * (t * 9.0 + sparkle.phase * 3.0).sin().abs());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_crate_falls_only_after_a_minute_and_not_too_often() {
        assert!(!golden_due(59.0, 100.0, 0.0));
        assert!(!golden_due(120.0, 44.0, 0.0));
        assert!(golden_due(120.0, 46.0, 0.05));
        assert!(!golden_due(120.0, 46.0, 0.5));
    }

    #[test]
    fn first_minute_crates_hold_only_mortar_and_rockets() {
        assert_eq!(crate_weapons(0.0), vec![CurrentWeapon::Mortar, CurrentWeapon::Rockets]);
        assert_eq!(crate_weapons(59.9), vec![CurrentWeapon::Mortar, CurrentWeapon::Rockets]);
        assert!(crate_weapons(61.0).contains(&CurrentWeapon::Laser));
        assert!(!crate_weapons(61.0).contains(&CurrentWeapon::Airstrike));
        assert!(crate_weapons(121.0).contains(&CurrentWeapon::Airstrike));
    }
}
