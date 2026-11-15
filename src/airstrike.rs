//! Авиаудар: отметка на земле по направлению от лунохода к прицелу, через две секунды над ней
//! пролетает самолёт и кладёт серию бомб с мощными взрывами и большими кратерами.
//!
//! Длина серии и число самолётов — по уровню (`upgrades::airstrike`): на третьем уровне второй
//! самолёт бомбит поперёк первого, линии кратеров пересекаются в точке прицела.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::aim::AimPoint;
use crate::arsenal::Inventory;
use crate::combat::{Health, Team};
use crate::explosion::Explosion;
use crate::frame::GameSet;
use crate::rover::{self, Rover};
use crate::run::RunScoped;
use crate::settlements::Footprint;
use crate::terrain::Terrain;
use crate::upgrades::{self, WeaponLevels};
use crate::weapons::CurrentWeapon;

/// Ширина отметки, м. Длина — по числу бомб: [`mark_length`].
const MARK_WIDTH: f32 = 6.0;

/// Второй самолёт заходит позже первого, с.
const SECOND_PLANE_DELAY: f32 = 0.4;

/// Через сколько после вызова появляется самолёт, с.
const DELAY: f32 = 2.0;

/// Высота и скорость самолёта.
const PLANE_HEIGHT: f32 = 25.0;
const PLANE_SPEED: f32 = 60.0;

/// Запас захода сверх упреждения первой бомбы, м: самолёт появляется до точки её сброса.
const APPROACH_MARGIN: f32 = 15.0;

/// Бомбы: число на первом уровне, шаг, мощность и радиус взрыва.
pub const BOMBS: usize = 6;
const BOMB_STEP: f32 = 5.0;
const BOMB_POWER: f32 = 1.6;
const BOMB_RADIUS: f32 = 7.0;

/// Гравитация для бомб, м/с².
const GRAVITY: f32 = 20.0;

/// Сегментов отметки на длинной стороне.
const MARK_SEGMENTS: usize = 8;

/// Время падения бомбы с высоты пролёта, с.
fn fall_time() -> f32 {
    (2.0 * PLANE_HEIGHT / GRAVITY).sqrt()
}

/// Смещение точки падения бомбы `index` из `bombs` от центра отметки вдоль пролёта, м.
fn bomb_offset(index: usize, bombs: usize) -> f32 {
    -(bombs as f32 - 1.0) * BOMB_STEP * 0.5 + index as f32 * BOMB_STEP
}

/// Длина отметки вдоль пролёта при `bombs` бомбах, м: шесть бомб — 30 м.
fn mark_length(bombs: usize) -> f32 {
    bombs as f32 * BOMB_STEP
}

/// С какого расстояния до центра самолёт заходит и где исчезает, м.
///
/// Бомба сбрасывается за `PLANE_SPEED · fall_time` до своей точки, поэтому заход дальше
/// упреждения первой бомбы: иначе в первом же кадре падают сразу несколько бомб в одну точку.
fn approach(bombs: usize) -> f32 {
    PLANE_SPEED * fall_time() - bomb_offset(0, bombs) + APPROACH_MARGIN
}

/// Подключает авиаудар.
pub struct AirstrikePlugin;

impl Plugin for AirstrikePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, create_airstrike_assets).add_systems(
            Update,
            (
                call_airstrike.after(rover::drive_rover).in_set(GameSet::Input),
                (run_strikes, pulse_marks, drop_bombs).chain().in_set(GameSet::Simulation),
            ),
        );
    }
}

/// Вызванный авиаудар. Живёт от вызова до ухода самолёта и падения последней бомбы.
#[derive(Component)]
pub struct Strike {
    /// Центр отметки на земле.
    pub center: Vec3,
    /// Направление пролёта в плане.
    pub dir: Vec3,
    /// Бомб в серии.
    bombs: usize,
    elapsed: f32,
    /// Самолёт уже появился.
    plane_spawned: bool,
    plane: Option<Entity>,
    shadow: Option<Entity>,
    dropped: usize,
    /// Сколько бомб уже взорвалось: отметка держится до последней.
    landed: usize,
    marks: Vec<Entity>,
}

/// Сегмент отметки на земле.
#[derive(Component)]
pub struct Mark {
    pub strike: Entity,
    /// Высота сегмента без пульсации: у рамки тонкая полоса, у сигнального дыма столб.
    base_height: f32,
}

/// Падающая бомба.
#[derive(Component)]
struct Bomb {
    velocity: Vec3,
    strike: Entity,
}

#[derive(Resource)]
struct AirstrikeAssets {
    cube: Handle<Mesh>,
    safe: Handle<StandardMaterial>,
    danger: Handle<StandardMaterial>,
    plane: Handle<StandardMaterial>,
    shadow: Handle<StandardMaterial>,
    bomb: Handle<StandardMaterial>,
    smoke: Handle<StandardMaterial>,
    sphere: Handle<Mesh>,
}

fn create_airstrike_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let glow = |materials: &mut Assets<StandardMaterial>, color: Color, e: LinearRgba| {
        materials.add(StandardMaterial {
            base_color: color,
            emissive: e,
            unlit: true,
            ..default()
        })
    };
    commands.insert_resource(AirstrikeAssets {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        sphere: meshes.add(Sphere::new(0.5).mesh().ico(1).unwrap()),
        safe: glow(&mut materials, Color::srgb(1.0, 0.85, 0.2), LinearRgba::rgb(3.0, 2.4, 0.3)),
        danger: glow(&mut materials, Color::srgb(1.0, 0.15, 0.1), LinearRgba::rgb(4.0, 0.4, 0.2)),
        plane: materials.add(Color::srgb(0.55, 0.6, 0.68)),
        shadow: materials.add(StandardMaterial {
            base_color: Color::srgba(0.0, 0.0, 0.0, 0.45),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..default()
        }),
        bomb: materials.add(Color::srgb(0.15, 0.16, 0.18)),
        smoke: glow(&mut materials, Color::srgb(1.0, 0.3, 0.6), LinearRgba::rgb(2.0, 0.5, 1.2)),
    });
}

/// Прямоугольник отметки в плане.
fn footprint(center: Vec3, dir: Vec3, bombs: usize) -> Footprint {
    Footprint {
        center: Vec2::new(center.x, center.z),
        // Локальная ось Z — вдоль пролёта.
        yaw: (dir.x).atan2(dir.z),
        half: Vec2::new(MARK_WIDTH * 0.5, mark_length(bombs) * 0.5),
    }
}

/// Вызов по нажатию: отметка и сигнальный дым.
#[allow(clippy::too_many_arguments)]
fn call_airstrike(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    weapon: Res<CurrentWeapon>,
    aim: Res<AimPoint>,
    assets: Res<AirstrikeAssets>,
    terrain: Res<Terrain>,
    levels: Res<WeaponLevels>,
    mut inventory: ResMut<Inventory>,
    rover: Single<(&Transform, &Health), With<Rover>>,
) {
    let (rover, health) = *rover;
    if !mouse.just_pressed(MouseButton::Left)
        || *weapon != CurrentWeapon::Airstrike
        || !inventory.usable(CurrentWeapon::Airstrike)
        || health.is_dead()
    {
        return;
    }
    inventory.spend(CurrentWeapon::Airstrike);
    let level = upgrades::airstrike(levels.level(CurrentWeapon::Airstrike));
    let dir = (aim.position - rover.translation).with_y(0.0).normalize_or(Vec3::NEG_Z);
    for plane in 0..level.planes {
        // Второй самолёт — поперёк первого и чуть позже.
        let dir = Quat::from_rotation_y(plane as f32 * std::f32::consts::FRAC_PI_2) * dir;
        let delay = plane as f32 * SECOND_PLANE_DELAY;
        spawn_strike(&mut commands, &assets, &terrain, aim.position, dir, level.bombs, delay);
    }
}

/// Удар: отметка на земле по контуру, сигнальный дым и сам [`Strike`].
fn spawn_strike(
    commands: &mut Commands,
    assets: &AirstrikeAssets,
    terrain: &Terrain,
    center: Vec3,
    dir: Vec3,
    bombs: usize,
    delay: f32,
) {
    let strike = commands.spawn(RunScoped).id();
    let rect = footprint(center, dir, bombs);

    // Отметка: сегменты по контуру прямоугольника, каждый на рельефе.
    let mut marks = Vec::new();
    let h = rect.half;
    let mut edge = |a: Vec2, b: Vec2, count: usize, commands: &mut Commands| {
        for i in 0..count {
            let p0 = a.lerp(b, i as f32 / count as f32);
            let p1 = a.lerp(b, (i as f32 + 0.7) / count as f32);
            let (w0, w1) = (rect.to_world(p0), rect.to_world(p1));
            let mid = (w0 + w1) * 0.5;
            let along = (w1 - w0).extend(0.0);
            let y = terrain.height_at(mid.x, mid.y) + 0.15;
            marks.push(
                commands
                    .spawn((
                        Mark {
                            strike,
                            base_height: 0.15,
                        },
                        RunScoped,
                        NotShadowCaster,
                        Mesh3d(assets.cube.clone()),
                        MeshMaterial3d(assets.safe.clone()),
                        Transform::from_xyz(mid.x, y, mid.y)
                            .with_rotation(Quat::from_rotation_y((-along.y).atan2(along.x)))
                            .with_scale(Vec3::new((w1 - w0).length(), 0.15, 0.35)),
                    ))
                    .id(),
            );
        }
    };
    let segments = MARK_SEGMENTS * bombs / BOMBS;
    edge(Vec2::new(-h.x, -h.y), Vec2::new(-h.x, h.y), segments, commands);
    edge(Vec2::new(h.x, -h.y), Vec2::new(h.x, h.y), segments, commands);
    edge(Vec2::new(-h.x, -h.y), Vec2::new(h.x, -h.y), 2, commands);
    edge(Vec2::new(-h.x, h.y), Vec2::new(h.x, h.y), 2, commands);
    // Сигнальный дым — светящийся столб в центре.
    marks.push(
        commands
            .spawn((
                Mark {
                    strike,
                    base_height: 3.0,
                },
                RunScoped,
                NotShadowCaster,
                Mesh3d(assets.sphere.clone()),
                MeshMaterial3d(assets.smoke.clone()),
                Transform::from_xyz(center.x, center.y + 1.5, center.z).with_scale(Vec3::new(0.8, 3.0, 0.8)),
            ))
            .id(),
    );
    commands.entity(strike).insert(Strike {
        center,
        dir,
        bombs,
        elapsed: -delay,
        plane_spawned: false,
        plane: None,
        shadow: None,
        dropped: 0,
        landed: 0,
        marks,
    });
}

/// Отметка пульсирует и краснеет, если луноход внутри неё.
fn pulse_marks(
    time: Res<Time>,
    assets: Res<AirstrikeAssets>,
    strikes: Query<&Strike>,
    rover: Single<&Transform, (With<Rover>, Without<Mark>)>,
    mut marks: Query<(&Mark, &mut Transform, &mut MeshMaterial3d<StandardMaterial>), Without<Rover>>,
) {
    let pulse = 1.0 + (time.elapsed_secs() * 10.0).sin() * 0.25;
    for (mark, mut transform, mut material) in &mut marks {
        let Ok(strike) = strikes.get(mark.strike) else {
            continue;
        };
        let inside = footprint(strike.center, strike.dir, strike.bombs).distance(Vec2::new(rover.translation.x, rover.translation.z)) < 1.5;
        let wanted = if inside { &assets.danger } else { &assets.safe };
        if material.0 != *wanted {
            material.0 = wanted.clone();
        }
        transform.scale.y = mark.base_height * pulse;
    }
}

/// Самолёт заходит, пролетает над отметкой и сбрасывает бомбы над её точками.
fn run_strikes(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    assets: Res<AirstrikeAssets>,
    mut strikes: Query<(Entity, &mut Strike)>,
    mut transforms: Query<&mut Transform>,
) {
    let dt = time.delta_secs();
    for (entity, mut strike) in &mut strikes {
        let approach = approach(strike.bombs);
        strike.elapsed += dt;
        if strike.elapsed < DELAY {
            continue;
        }
        let flown = (strike.elapsed - DELAY) * PLANE_SPEED - approach;
        let ground = strike.center + strike.dir * flown;
        let plane_pos = Vec3::new(ground.x, strike.center.y + PLANE_HEIGHT, ground.z);
        let rotation = Quat::from_rotation_arc(Vec3::NEG_Z, strike.dir);
        if !strike.plane_spawned {
            strike.plane_spawned = true;
            let plane = commands
                .spawn((RunScoped, Transform::from_translation(plane_pos).with_rotation(rotation), Visibility::default()))
                .with_children(|p| {
                    // Фюзеляж, крылья, хвост.
                    for (offset, scale) in [
                        (Vec3::ZERO, Vec3::new(1.4, 1.2, 9.0)),
                        (Vec3::new(0.0, 0.0, 0.5), Vec3::new(12.0, 0.3, 2.4)),
                        (Vec3::new(0.0, 1.0, 4.0), Vec3::new(0.3, 2.2, 1.6)),
                        (Vec3::new(0.0, 0.2, 4.0), Vec3::new(4.5, 0.25, 1.2)),
                    ] {
                        p.spawn((
                            Mesh3d(assets.cube.clone()),
                            MeshMaterial3d(assets.plane.clone()),
                            Transform::from_translation(offset).with_scale(scale),
                        ));
                    }
                })
                .id();
            let shadow = commands
                .spawn((
                    RunScoped,
                    NotShadowCaster,
                    Mesh3d(assets.cube.clone()),
                    MeshMaterial3d(assets.shadow.clone()),
                    Transform::default(),
                ))
                .id();
            strike.plane = Some(plane);
            strike.shadow = Some(shadow);
        }
        if let Some(mut t) = strike.plane.and_then(|p| transforms.get_mut(p).ok()) {
            t.translation = plane_pos;
            t.rotation = rotation;
        }
        if let Some(mut t) = strike.shadow.and_then(|p| transforms.get_mut(p).ok()) {
            let y = terrain.height_at(plane_pos.x, plane_pos.z) + 0.1;
            *t = Transform::from_xyz(plane_pos.x, y, plane_pos.z)
                .with_rotation(rotation)
                .with_scale(Vec3::new(10.0, 0.05, 8.0));
        }
        // Бомба сбрасывается над своей точкой отметки с упреждением на падение.
        let lead = PLANE_SPEED * fall_time();
        while strike.dropped < strike.bombs {
            if flown + lead < bomb_offset(strike.dropped, strike.bombs) {
                break;
            }
            strike.dropped += 1;
            commands.spawn((
                Bomb {
                    velocity: strike.dir * PLANE_SPEED,
                    strike: entity,
                },
                RunScoped,
                Mesh3d(assets.sphere.clone()),
                MeshMaterial3d(assets.bomb.clone()),
                Transform::from_translation(plane_pos - Vec3::Y).with_scale(Vec3::new(0.7, 0.7, 1.4)),
            ));
        }
        if flown > approach {
            for e in [strike.plane.take(), strike.shadow.take()].into_iter().flatten() {
                commands.entity(e).try_despawn();
            }
        }
        // Отметка и сам удар живут, пока не упала последняя бомба.
        if strike.landed >= strike.bombs {
            for mark in strike.marks.drain(..) {
                commands.entity(mark).try_despawn();
            }
            if strike.plane.is_none() {
                commands.entity(entity).despawn();
            }
        }
    }
}

/// Бомбы падают по баллистике и взрываются о землю.
fn drop_bombs(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut bombs: Query<(Entity, &mut Transform, &mut Bomb)>,
    mut strikes: Query<&mut Strike>,
    mut explosions: MessageWriter<Explosion>,
) {
    let dt = time.delta_secs();
    for (entity, mut transform, mut bomb) in &mut bombs {
        bomb.velocity.y -= GRAVITY * dt;
        transform.translation += bomb.velocity * dt;
        transform.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, bomb.velocity.normalize_or(Vec3::NEG_Y));
        let p = transform.translation;
        let ground = terrain.height_at(p.x, p.z);
        if p.y <= ground {
            commands.entity(entity).despawn();
            if let Ok(mut strike) = strikes.get_mut(bomb.strike) {
                strike.landed += 1;
            }
            explosions.write(Explosion {
                position: Vec3::new(p.x, ground, p.z),
                radius: BOMB_RADIUS,
                power: BOMB_POWER,
                source_team: Team::Player,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Самолёт появляется раньше точки сброса первой бомбы, и в первом кадре не падает ни одна.
    #[test]
    fn plane_appears_before_the_first_drop_point() {
        let lead = PLANE_SPEED * fall_time();
        for bombs in [BOMBS, upgrades::airstrike(2).bombs, upgrades::airstrike(3).bombs] {
            let flown_at_start = -approach(bombs);
            assert!(flown_at_start + lead < bomb_offset(0, bombs));
            // Бомбы сбрасываются по очереди с шагом по пролёту, а не все сразу.
            assert!(bomb_offset(1, bombs) - bomb_offset(0, bombs) > 0.0);
            // Серия симметрична относительно точки прицела: линии крест-накрест пересекаются в ней.
            assert!((bomb_offset(0, bombs) + bomb_offset(bombs - 1, bombs)).abs() < 1e-4);
        }
        assert!(approach(BOMBS) >= 110.0, "{}", approach(BOMBS));
        assert_eq!(mark_length(BOMBS), 30.0);
    }
}
