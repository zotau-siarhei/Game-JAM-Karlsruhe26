//! Подбираемые предметы: ящики с оружием на парашютах и трофеи, выпавшие из врагов.
//!
//! Предмет подбирается проездом. Ящики падают раз в несколько секунд в кольце вокруг лунохода,
//! чаще у мест разрушений. Содержимое открывается по времени забега. Враг роняет при гибели
//! оружие, которое носит по своему типу (`CarriedWeapon` ставит `enemy.rs`).

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::arsenal::{Inventory, spec};
use crate::buildings::BuildingDestroyed;
use crate::camera::GameCamera;
use crate::combat::{Died, Health, SurvivalTime};
use crate::frame::{GameSet, RunMessages};
use crate::rng::GameRng;
use crate::rover::Rover;
use crate::run::{RunScoped, WorldRebuilt};
use crate::subsidence::TerrainChanged;
use crate::terrain::Terrain;
use crate::weapons::CurrentWeapon;

/// Радиус подбора, м.
const PICKUP_RADIUS: f32 = 2.0;

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

/// Трофей врага живёт и мигает последние секунды.
const LOOT_SECONDS: f32 = 20.0;
const LOOT_BLINK: f32 = 5.0;

/// Подключает подбираемые предметы.
pub struct PickupsPlugin;

impl Plugin for PickupsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CrateDirector>()
            .add_run_message::<SpawnLoot>()
            .add_systems(Startup, create_pickup_assets)
            .add_systems(
                Update,
                (
                    reset_crates.in_set(GameSet::Reset),
                    (remember_ruins, drop_crates, spawn_loot, descend, collect)
                        .chain()
                        .in_set(GameSet::Simulation),
                    // Враг ещё существует: трофей берёт оружие с него.
                    drop_loot.in_set(GameSet::Death),
                    float_popups.in_set(GameSet::Presentation),
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

/// Оружие, которое носит враг.
#[derive(Component)]
pub struct CarriedWeapon {
    pub weapon: CurrentWeapon,
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
    /// Последние места разрушений.
    ruins: Vec<Vec3>,
}

#[derive(Resource)]
struct PickupAssets {
    cube: Handle<Mesh>,
    canopy: Handle<Mesh>,
    crate_material: Handle<StandardMaterial>,
    canopy_material: Handle<StandardMaterial>,
    glow: Vec<Handle<StandardMaterial>>,
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
    commands.insert_resource(PickupAssets {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        canopy: meshes.add(Sphere::new(1.0).mesh().uv(12, 6)),
        crate_material: materials.add(Color::srgb(0.55, 0.38, 0.2)),
        canopy_material: materials.add(Color::srgb(0.95, 0.95, 0.95)),
        glow,
    });
}

fn glow_of(assets: &PickupAssets, weapon: CurrentWeapon) -> Handle<StandardMaterial> {
    let i = CurrentWeapon::ALL.iter().position(|w| *w == weapon).unwrap_or(0);
    assets.glow[i].clone()
}

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
    time: Res<Time>,
    survival: Res<SurvivalTime>,
    inventory: Res<Inventory>,
    terrain: Res<Terrain>,
    assets: Res<PickupAssets>,
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

    let ground = terrain.height_at(place.x, place.z);
    let canopy = commands
        .spawn((
            Mesh3d(assets.canopy.clone()),
            MeshMaterial3d(assets.canopy_material.clone()),
            NotShadowCaster,
            Transform::from_xyz(0.0, 2.2, 0.0).with_scale(Vec3::new(1.6, 0.8, 1.6)),
        ))
        .id();
    commands
        .spawn((
            Pickup {
                weapon,
                amount: spec(weapon).per_crate,
                expires: None,
            },
            Crate,
            Parachuting { canopy, sway: 0.0 },
            RunScoped,
            Mesh3d(assets.cube.clone()),
            MeshMaterial3d(assets.crate_material.clone()),
            Transform::from_xyz(place.x, ground + DROP_HEIGHT, place.z).with_scale(Vec3::splat(1.1)),
        ))
        .add_child(canopy)
        .with_children(|c| {
            // Полоса цвета оружия на крышке.
            c.spawn((
                Mesh3d(assets.cube.clone()),
                MeshMaterial3d(glow_of(&assets, weapon)),
                Transform::from_xyz(0.0, 0.5, 0.0).with_scale(Vec3::new(1.05, 0.08, 0.3)),
            ));
        });
}

/// Ящики спускаются на парашюте, все предметы стоят на рельефе, трофеи вращаются и мигают.
fn descend(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut items: Query<(Entity, &mut Transform, &mut Pickup, Option<&mut Parachuting>, &mut Visibility)>,
) {
    let dt = time.delta_secs();
    for (entity, mut transform, mut pickup, chute, mut visibility) in &mut items {
        let p = transform.translation;
        let ground = terrain.height_at(p.x, p.z) + transform.scale.y * 0.5;
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
            transform.rotate_y(dt * 2.0);
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
    items: Query<(Entity, &Transform, &Pickup), Without<Parachuting>>,
    rover: Single<(&Transform, &Health), With<Rover>>,
) {
    let (rover, health) = *rover;
    if health.is_dead() {
        return;
    }
    for (entity, transform, pickup) in &items {
        if transform.translation.with_y(0.0).distance(rover.translation.with_y(0.0)) > PICKUP_RADIUS {
            continue;
        }
        let new = !inventory.owns(pickup.weapon);
        let added = inventory.add(pickup.weapon, pickup.amount);
        if !new && added == 0 {
            // Боезапас полный: предмет остаётся лежать до следующего раза.
            continue;
        }
        commands.entity(entity).despawn();
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
    mut died: MessageReader<Died>,
    assets: Res<PickupAssets>,
    carriers: Query<(&GlobalTransform, &CarriedWeapon)>,
) {
    for death in died.read() {
        let Ok((transform, carried)) = carriers.get(death.entity) else {
            continue;
        };
        if carried.weapon == CurrentWeapon::MachineGun {
            continue;
        }
        commands.spawn((
            Pickup {
                weapon: carried.weapon,
                amount: spec(carried.weapon).per_crate,
                expires: Some(LOOT_SECONDS),
            },
            RunScoped,
            NotShadowCaster,
            Mesh3d(assets.cube.clone()),
            MeshMaterial3d(glow_of(&assets, carried.weapon)),
            Transform::from_translation(transform.translation()).with_scale(Vec3::new(0.7, 0.7, 0.7)),
            Visibility::default(),
        ));
    }
}

/// Просьба положить светящийся трофей с оружием: так босс оставляет ящики после гибели.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpawnLoot {
    pub position: Vec3,
    pub weapon: CurrentWeapon,
}

fn spawn_loot(mut commands: Commands, mut requests: MessageReader<SpawnLoot>, assets: Res<PickupAssets>) {
    for r in requests.read() {
        commands.spawn((
            Pickup {
                weapon: r.weapon,
                amount: spec(r.weapon).per_crate,
                expires: None,
            },
            RunScoped,
            NotShadowCaster,
            Mesh3d(assets.cube.clone()),
            MeshMaterial3d(glow_of(&assets, r.weapon)),
            Transform::from_translation(r.position),
            Visibility::default(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_minute_crates_hold_only_mortar_and_rockets() {
        assert_eq!(crate_weapons(0.0), vec![CurrentWeapon::Mortar, CurrentWeapon::Rockets]);
        assert_eq!(crate_weapons(59.9), vec![CurrentWeapon::Mortar, CurrentWeapon::Rockets]);
        assert!(crate_weapons(61.0).contains(&CurrentWeapon::Laser));
        assert!(!crate_weapons(61.0).contains(&CurrentWeapon::Airstrike));
        assert!(crate_weapons(121.0).contains(&CurrentWeapon::Airstrike));
    }
}
