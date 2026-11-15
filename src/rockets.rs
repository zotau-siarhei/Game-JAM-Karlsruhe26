//! Ракеты: обычные летят низко над рельефом к точке прицела, самонаводящиеся уходят залпом
//! веером вверх и доворачивают к врагам у прицела.
//!
//! Ракета взрывается о рельеф, препятствие, чужую машину или в точке прицела и отправляет
//! малый [`Explosion`]: кратер, урон и обломки делают его подписчики.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::aim::AimPoint;
use crate::arsenal::Inventory;
use crate::bullet::terrain_hit;
use crate::combat::{Collider, Health, Team};
use crate::enemy::Enemy;
use crate::explosion::Explosion;
use crate::frame::{GameSet, RunMessages};
use crate::obstacles::{self, Obstacle};
use crate::rover::{self, Rover};
use crate::run::{GameState, RunScoped, WorldRebuilt};
use crate::terrain::Terrain;
use crate::weapons::CurrentWeapon;

/// Перезарядка обычной ракеты, с.
const ROCKET_RELOAD: f32 = 0.25;

/// Скорость обычной ракеты, м/с.
const ROCKET_SPEED: f32 = 45.0;

/// Высота полёта над рельефом, м.
const CRUISE_HEIGHT: f32 = 1.2;

/// Как быстро ракета догоняет высоту полёта, 1/с.
const CRUISE_FOLLOW: f32 = 8.0;

/// Взрыв ракеты: мощность от мортиры и радиус.
const ROCKET_POWER: f32 = 0.45;
const ROCKET_RADIUS: f32 = 2.5;

/// Самонаводящиеся: ракет в залпе, перезарядка, скорость, поворот, взлёт, поиск целей.
const SALVO: usize = 6;
const SALVO_RELOAD: f32 = 1.5;
const HOMING_SPEED: f32 = 32.0;
const HOMING_TURN: f32 = 4.0;
const LAUNCH_SECONDS: f32 = 0.35;
const FAN_ANGLE: f32 = 25.0 * std::f32::consts::PI / 180.0;
const TARGET_RANGE: f32 = 40.0;
const RETARGET_RANGE: f32 = 15.0;

/// Ракета дальше этого от старта взрывается сама, м.
const MAX_RANGE: f32 = 80.0;

/// Как часто ракета оставляет клуб дыма, с.
const PUFF_INTERVAL: f32 = 0.07;

/// Сколько живёт клуб дыма, с: на ракету в полёте приходится около дюжины клубов.
const PUFF_SECONDS: f32 = 0.9;

/// Сколько клубов дыма всех ракет может быть сразу: залп не плодит сотни сущностей.
const MAX_PUFFS: usize = 90;

/// Радиус шара пламени и клуба дыма в масштабе 1, м.
const FLAME_RADIUS: f32 = 0.18;

/// Начальный размер клуба: клубы реже, поэтому крупнее и сливаются в след.
const PUFF_SIZE: f32 = 1.6;

/// Подключает ракеты.
pub struct RocketsPlugin;

impl Plugin for RocketsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Launchers>()
            .add_run_message::<FireRocket>()
            .add_systems(Startup, create_rocket_assets)
            .add_systems(
            Update,
            (
                reset_launchers.in_set(GameSet::Reset),
                (fire_rockets, fire_homing)
                    .chain()
                    .after(rover::drive_rover)
                    .in_set(GameSet::Input),
                (launch_requested, fly_rockets).chain().in_set(GameSet::Simulation),
                (show_locks, fade_puffs).in_set(GameSet::Presentation),
            ),
        );
    }
}

/// Остаток перезарядки пусковых лунохода, с.
#[derive(Resource, Default)]
struct Launchers {
    rockets: f32,
    salvo: f32,
}

/// Мир перестроен: перезарядка прошлого забега не переходит в новый.
fn reset_launchers(mut rebuilt: MessageReader<WorldRebuilt>, mut launchers: ResMut<Launchers>) {
    if rebuilt.read().last().is_some() {
        *launchers = Launchers::default();
    }
}

/// Просьба выпустить обычную ракету: так стреляют враги с ракетами.
#[derive(Message, Debug, Clone, Copy)]
pub struct FireRocket {
    pub origin: Vec3,
    pub target: Vec3,
    pub team: Team,
}

fn launch_requested(mut commands: Commands, mut requests: MessageReader<FireRocket>, assets: Res<RocketAssets>) {
    for r in requests.read() {
        let flat = (r.target - r.origin).with_y(0.0).normalize_or(Vec3::NEG_Z);
        spawn_rocket(
            &mut commands,
            &assets,
            r.origin,
            Rocket {
                velocity: flat * ROCKET_SPEED,
                team: r.team,
                target: r.target,
                travelled: 0.0,
                puff: 0.0,
                homing: None,
            },
        );
    }
}

/// Ракета в полёте.
#[derive(Component)]
pub struct Rocket {
    velocity: Vec3,
    team: Team,
    /// Точка прицела: в ней ракета взрывается.
    target: Vec3,
    travelled: f32,
    puff: f32,
    homing: Option<Homing>,
}

/// Наведение самонаводящейся ракеты.
struct Homing {
    /// Сколько ещё длится взлёт веером.
    launch: f32,
    entity: Option<Entity>,
}

/// Клуб дыма следа: вытянут вдоль пути, чтобы редкие клубы сливались в сплошной след.
#[derive(Component)]
struct Puff {
    remaining: f32,
    /// Длина клуба вдоль пути в масштабе меша.
    stretch: f32,
}

/// Метка захвата над врагом.
#[derive(Component)]
struct LockMarker(usize);

#[derive(Resource)]
struct RocketAssets {
    body: Handle<Mesh>,
    body_material: Handle<StandardMaterial>,
    flame: Handle<Mesh>,
    flame_material: Handle<StandardMaterial>,
    puff_material: Handle<StandardMaterial>,
    ring: Handle<Mesh>,
    lock_material: Handle<StandardMaterial>,
}

fn create_rocket_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(RocketAssets {
        body: meshes.add(Capsule3d::new(0.12, 0.6)),
        body_material: materials.add(Color::srgb(0.95, 0.85, 0.25)),
        flame: meshes.add(Sphere::new(FLAME_RADIUS)),
        flame_material: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.6, 0.15),
            emissive: LinearRgba::rgb(10.0, 4.0, 0.5),
            unlit: true,
            ..default()
        }),
        puff_material: materials.add(StandardMaterial {
            base_color: Color::srgb(0.75, 0.73, 0.7),
            unlit: true,
            ..default()
        }),
        ring: meshes.add(Torus::new(1.0, 1.2)),
        lock_material: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.25, 0.2),
            emissive: LinearRgba::rgb(3.0, 0.6, 0.4),
            unlit: true,
            ..default()
        }),
    });
}

fn spawn_rocket(commands: &mut Commands, assets: &RocketAssets, origin: Vec3, rocket: Rocket) {
    let dir = rocket.velocity.normalize_or(Vec3::NEG_Z);
    commands
        .spawn((
            rocket,
            RunScoped,
            NotShadowCaster,
            Mesh3d(assets.body.clone()),
            MeshMaterial3d(assets.body_material.clone()),
            Transform::from_translation(origin).with_rotation(Quat::from_rotation_arc(Vec3::Y, dir)),
        ))
        .with_children(|r| {
            r.spawn((
                Mesh3d(assets.flame.clone()),
                MeshMaterial3d(assets.flame_material.clone()),
                Transform::from_xyz(0.0, -0.5, 0.0),
            ));
        });
}

/// Дуло: над башней лунохода.
fn muzzle(rover: &Transform) -> Vec3 {
    rover.translation + Vec3::Y * 0.9
}

/// Обычные ракеты очередью, пока зажата кнопка.
#[allow(clippy::too_many_arguments)]
fn fire_rockets(
    mut commands: Commands,
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    weapon: Res<CurrentWeapon>,
    aim: Res<AimPoint>,
    assets: Res<RocketAssets>,
    mut inventory: ResMut<Inventory>,
    mut launchers: ResMut<Launchers>,
    rover: Single<(&Transform, &Health), With<Rover>>,
) {
    let cooldown = &mut launchers.rockets;
    *cooldown = (*cooldown - time.delta_secs()).max(0.0);
    let (rover, health) = *rover;
    let trigger = mouse.pressed(MouseButton::Left)
        && *weapon == CurrentWeapon::Rockets
        && inventory.usable(CurrentWeapon::Rockets)
        && !health.is_dead();
    if !trigger || *cooldown > 0.0 {
        return;
    }
    *cooldown = ROCKET_RELOAD;
    inventory.spend(CurrentWeapon::Rockets);
    let origin = muzzle(rover);
    let flat = (aim.position - origin).with_y(0.0).normalize_or(Vec3::NEG_Z);
    spawn_rocket(
        &mut commands,
        &assets,
        origin,
        Rocket {
            velocity: flat * ROCKET_SPEED,
            team: Team::Player,
            target: aim.position,
            travelled: 0.0,
            puff: 0.0,
            homing: None,
        },
    );
}

/// До [`SALVO`] ближайших к прицелу врагов в пределах [`TARGET_RANGE`] от лунохода; если врагов
/// меньше, цели повторяются по кругу.
fn pick_targets(rover: Vec3, aim: Vec3, enemies: impl Iterator<Item = (Entity, Vec3)>) -> Vec<Entity> {
    let mut near: Vec<(f32, Entity)> = enemies
        .filter(|(_, p)| p.distance(rover) <= TARGET_RANGE)
        .map(|(e, p)| (p.distance_squared(aim), e))
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    near.truncate(SALVO);
    if near.is_empty() {
        return Vec::new();
    }
    (0..SALVO).map(|i| near[i % near.len()].1).collect()
}

/// Залп самонаводящихся ракет по нажатию.
#[allow(clippy::too_many_arguments)]
fn fire_homing(
    mut commands: Commands,
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    weapon: Res<CurrentWeapon>,
    aim: Res<AimPoint>,
    assets: Res<RocketAssets>,
    mut inventory: ResMut<Inventory>,
    mut launchers: ResMut<Launchers>,
    rover: Single<(&Transform, &Health), With<Rover>>,
    enemies: Query<(Entity, &GlobalTransform, &Health), With<Enemy>>,
) {
    let cooldown = &mut launchers.salvo;
    *cooldown = (*cooldown - time.delta_secs()).max(0.0);
    let (rover, health) = *rover;
    let trigger = mouse.pressed(MouseButton::Left)
        && *weapon == CurrentWeapon::HomingRockets
        && inventory.usable(CurrentWeapon::HomingRockets)
        && !health.is_dead();
    if !trigger || *cooldown > 0.0 {
        return;
    }
    *cooldown = SALVO_RELOAD;
    inventory.spend(CurrentWeapon::HomingRockets);
    let targets = pick_targets(
        rover.translation,
        aim.position,
        enemies
            .iter()
            .filter(|(_, _, h)| !h.is_dead())
            .map(|(e, t, _)| (e, t.translation())),
    );
    let origin = muzzle(rover);
    let forward = (aim.position - origin).with_y(0.0).normalize_or(Vec3::NEG_Z);
    for i in 0..SALVO {
        let spread = (i as f32 / (SALVO - 1) as f32 - 0.5) * 2.0 * FAN_ANGLE;
        let side = Quat::from_rotation_y(spread) * forward;
        let velocity = (Vec3::Y * 1.6 + side * 0.6).normalize() * HOMING_SPEED;
        spawn_rocket(
            &mut commands,
            &assets,
            origin,
            Rocket {
                velocity,
                team: Team::Player,
                target: aim.position,
                travelled: 0.0,
                puff: i as f32 * 0.005,
                homing: Some(Homing {
                    launch: LAUNCH_SECONDS,
                    entity: targets.get(i).copied(),
                }),
            },
        );
    }
}

/// Кольца захвата над целями, пока в забеге выбраны самонаводящиеся ракеты. В меню, на паузе
/// и на экране конца забега колец нет.
#[allow(clippy::too_many_arguments)]
fn show_locks(
    mut commands: Commands,
    state: Res<State<GameState>>,
    weapon: Res<CurrentWeapon>,
    aim: Res<AimPoint>,
    assets: Res<RocketAssets>,
    rover: Single<&Transform, With<Rover>>,
    enemies: Query<(Entity, &GlobalTransform, &Health), With<Enemy>>,
    mut markers: Query<(&LockMarker, &mut Transform, &mut Visibility), Without<Rover>>,
    mut spawned: Local<bool>,
) {
    if !*spawned {
        *spawned = true;
        for i in 0..SALVO {
            commands.spawn((
                LockMarker(i),
                NotShadowCaster,
                Mesh3d(assets.ring.clone()),
                MeshMaterial3d(assets.lock_material.clone()),
                Transform::from_scale(Vec3::new(1.0, 0.3, 1.0)),
                Visibility::Hidden,
            ));
        }
        return;
    }
    let targets = if *state.get() == GameState::Playing && *weapon == CurrentWeapon::HomingRockets {
        let mut t = pick_targets(
            rover.translation,
            aim.position,
            enemies
                .iter()
                .filter(|(_, _, h)| !h.is_dead())
                .map(|(e, t, _)| (e, t.translation())),
        );
        t.dedup();
        t.sort();
        t.dedup();
        t
    } else {
        Vec::new()
    };
    for (marker, mut transform, mut visibility) in &mut markers {
        let target = targets.get(marker.0).and_then(|e| enemies.get(*e).ok());
        match target {
            Some((_, t, _)) => {
                transform.translation = t.translation() + Vec3::Y * 2.2;
                *visibility = Visibility::Inherited;
            }
            None => *visibility = Visibility::Hidden,
        }
    }
}

/// Полёт, наведение, след и подрыв.
#[allow(clippy::too_many_arguments)]
fn fly_rockets(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    assets: Res<RocketAssets>,
    mut rockets: Query<(Entity, &mut Transform, &mut Rocket)>,
    targets: Query<(Entity, &GlobalTransform, &Collider, &Team, &Health)>,
    enemies: Query<(Entity, &GlobalTransform, &Health), With<Enemy>>,
    obstacles: Query<(Entity, &Obstacle)>,
    puffs: Query<(), With<Puff>>,
    mut explosions: MessageWriter<Explosion>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let mut puff_count = puffs.iter().count();
    for (entity, mut transform, mut rocket) in &mut rockets {
        let from = transform.translation;
        let mut velocity = rocket.velocity;
        let mut explode_at_target = false;
        let last_target = rocket.target;
        let mut new_target = last_target;
        match rocket.homing.as_mut() {
            None => {
                // Низко над рельефом: высота догоняет рельеф под ракетой.
                let ahead = from + velocity * dt;
                let cruise = terrain.height_at(ahead.x, ahead.z) + CRUISE_HEIGHT;
                velocity.y = (cruise - from.y) * CRUISE_FOLLOW;
                let flat_to_target = (last_target - from).with_y(0.0).length();
                explode_at_target = flat_to_target <= ROCKET_SPEED * dt;
            }
            Some(homing) => {
                if homing.launch > 0.0 {
                    homing.launch -= dt;
                } else {
                    // Погибшая цель — ближайший живой враг рядом с ней, иначе последняя точка.
                    let alive = homing
                        .entity
                        .and_then(|e| enemies.get(e).ok())
                        .filter(|(_, _, h)| !h.is_dead());
                    let goal = match alive {
                        Some((_, t, _)) => t.translation(),
                        None => {
                            let around = last_target;
                            let replacement = enemies
                                .iter()
                                .filter(|(_, t, h)| !h.is_dead() && t.translation().distance(around) < RETARGET_RANGE)
                                .min_by(|a, b| {
                                    a.1.translation()
                                        .distance_squared(around)
                                        .total_cmp(&b.1.translation().distance_squared(around))
                                });
                            homing.entity = replacement.map(|r| r.0);
                            replacement.map_or(around, |r| r.1.translation())
                        }
                    };
                    new_target = goal;
                    let desired = (goal + Vec3::Y * 0.5 - from).normalize_or(velocity.normalize_or(Vec3::Y));
                    let current = velocity.normalize_or(desired);
                    let angle = current.angle_between(desired);
                    let max = HOMING_TURN * dt;
                    let dir = if angle <= max || angle < 1e-4 {
                        desired
                    } else {
                        current.slerp(desired, max / angle).normalize()
                    };
                    velocity = dir * HOMING_SPEED;
                }
            }
        }
        rocket.velocity = velocity;
        rocket.target = new_target;
        let mut to = from + velocity * dt;
        let mut hit = explode_at_target;
        if let Some(ground) = terrain_hit(&terrain, from, to) {
            to = ground;
            hit = true;
        }
        if let Some((_, wall)) = obstacles::segment_hit(from, to, &obstacles) {
            to = wall;
            hit = true;
        }
        let team = rocket.team;
        let struck = targets.iter().any(|(_, t, c, tm, h)| {
            *tm != team && !h.is_dead() && {
                let p = t.translation();
                let along = to - from;
                let k = ((p - from).dot(along) / along.length_squared().max(1e-6)).clamp(0.0, 1.0);
                (from + along * k).distance(p) <= c.radius + 0.3
            }
        });
        rocket.travelled += from.distance(to);
        if hit || struck || rocket.travelled > MAX_RANGE {
            commands.entity(entity).despawn();
            explosions.write(Explosion {
                position: to,
                radius: ROCKET_RADIUS,
                power: ROCKET_POWER,
                source_team: team,
            });
            continue;
        }
        transform.translation = to;
        transform.rotation = Quat::from_rotation_arc(Vec3::Y, velocity.normalize_or(Vec3::Y));

        rocket.puff -= dt;
        if rocket.puff <= 0.0 && puff_count < MAX_PUFFS {
            rocket.puff = PUFF_INTERVAL;
            puff_count += 1;
            let dir = velocity.normalize_or(Vec3::Y);
            let length = velocity.length() * PUFF_INTERVAL;
            let stretch = length / (2.0 * FLAME_RADIUS);
            commands.spawn((
                Puff {
                    remaining: PUFF_SECONDS,
                    stretch,
                },
                RunScoped,
                NotShadowCaster,
                Mesh3d(assets.flame.clone()),
                MeshMaterial3d(assets.puff_material.clone()),
                Transform::from_translation(from - dir * length * 0.5)
                    .with_rotation(Quat::from_rotation_arc(Vec3::Y, dir))
                    .with_scale(Vec3::new(PUFF_SIZE, stretch.max(PUFF_SIZE), PUFF_SIZE)),
            ));
        }
    }
}

/// Дым следа расползается и тает.
fn fade_puffs(mut commands: Commands, time: Res<Time>, mut puffs: Query<(Entity, &mut Transform, &mut Puff)>) {
    let dt = time.delta_secs();
    for (entity, mut transform, mut puff) in &mut puffs {
        puff.remaining -= dt;
        if puff.remaining <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let t = puff.remaining / PUFF_SECONDS;
        let width = (PUFF_SIZE + (1.0 - t) * 2.4) * t.sqrt();
        transform.scale = Vec3::new(width, (puff.stretch * t.sqrt()).max(width), width);
        transform.translation.y += dt * 0.6;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_enemy_takes_the_whole_salvo() {
        let e = Entity::PLACEHOLDER;
        let targets = pick_targets(Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0), [(e, Vec3::new(6.0, 0.0, 0.0))].into_iter());
        assert_eq!(targets, vec![e; SALVO]);
        assert!(pick_targets(Vec3::ZERO, Vec3::ZERO, std::iter::empty()).is_empty());
    }
}
