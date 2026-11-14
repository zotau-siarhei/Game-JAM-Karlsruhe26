//! Авиаудар: отметка на земле по направлению от лунохода к прицелу, через две секунды над ней
//! пролетает самолёт и кладёт серию бомб с мощными взрывами и большими кратерами.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::aim::AimPoint;
use crate::arsenal::Inventory;
use crate::combat::{Health, Team};
use crate::explosion::Explosion;
use crate::rover::{self, Rover};
use crate::run::{GameState, RunScoped};
use crate::settlements::Footprint;
use crate::terrain::Terrain;
use crate::weapons::CurrentWeapon;

/// Размер отметки: длина вдоль пролёта и ширина, м.
const MARK_LENGTH: f32 = 30.0;
const MARK_WIDTH: f32 = 6.0;

/// Через сколько после вызова появляется самолёт, с.
const DELAY: f32 = 2.0;

/// Высота и скорость самолёта.
const PLANE_HEIGHT: f32 = 25.0;
const PLANE_SPEED: f32 = 60.0;

/// С какого расстояния до центра самолёт заходит и где исчезает, м.
const APPROACH: f32 = 90.0;

/// Бомбы: число, шаг, мощность и радиус взрыва.
const BOMBS: usize = 6;
const BOMB_STEP: f32 = 5.0;
const BOMB_POWER: f32 = 1.6;
const BOMB_RADIUS: f32 = 7.0;

/// Гравитация для бомб, м/с².
const GRAVITY: f32 = 20.0;

/// Сегментов отметки на длинной стороне.
const MARK_SEGMENTS: usize = 8;

/// Подключает авиаудар.
pub struct AirstrikePlugin;

impl Plugin for AirstrikePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, create_airstrike_assets).add_systems(
            Update,
            (
                call_airstrike.run_if(in_state(GameState::Playing)),
                run_strikes,
                pulse_marks,
                drop_bombs,
            )
                .chain()
                .after(rover::drive_rover),
        );
    }
}

/// Вызванный авиаудар.
#[derive(Component)]
pub struct Strike {
    center: Vec3,
    dir: Vec3,
    elapsed: f32,
    plane: Option<Entity>,
    shadow: Option<Entity>,
    dropped: usize,
    marks: Vec<Entity>,
}

/// Сегмент отметки на земле.
#[derive(Component)]
struct Mark {
    strike: Entity,
}

/// Падающая бомба.
#[derive(Component)]
struct Bomb {
    velocity: Vec3,
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
fn footprint(center: Vec3, dir: Vec3) -> Footprint {
    Footprint {
        center: Vec2::new(center.x, center.z),
        // Локальная ось Z — вдоль пролёта.
        yaw: (dir.x).atan2(dir.z),
        half: Vec2::new(MARK_WIDTH * 0.5, MARK_LENGTH * 0.5),
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
    let dir = (aim.position - rover.translation).with_y(0.0).normalize_or(Vec3::NEG_Z);
    let center = aim.position;
    let strike = commands.spawn(RunScoped).id();
    let rect = footprint(center, dir);

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
                        Mark { strike },
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
    edge(Vec2::new(-h.x, -h.y), Vec2::new(-h.x, h.y), MARK_SEGMENTS, &mut commands);
    edge(Vec2::new(h.x, -h.y), Vec2::new(h.x, h.y), MARK_SEGMENTS, &mut commands);
    edge(Vec2::new(-h.x, -h.y), Vec2::new(h.x, -h.y), 2, &mut commands);
    edge(Vec2::new(-h.x, h.y), Vec2::new(h.x, h.y), 2, &mut commands);
    // Сигнальный дым — светящийся столб в центре.
    marks.push(
        commands
            .spawn((
                Mark { strike },
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
        elapsed: 0.0,
        plane: None,
        shadow: None,
        dropped: 0,
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
        let inside = footprint(strike.center, strike.dir).distance(Vec2::new(rover.translation.x, rover.translation.z)) < 1.5;
        let wanted = if inside { &assets.danger } else { &assets.safe };
        if material.0 != *wanted {
            material.0 = wanted.clone();
        }
        transform.scale.y = 0.15 * pulse;
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
        strike.elapsed += dt;
        if strike.elapsed < DELAY {
            continue;
        }
        let flown = (strike.elapsed - DELAY) * PLANE_SPEED - APPROACH;
        let ground = strike.center + strike.dir * flown;
        let plane_pos = Vec3::new(ground.x, strike.center.y + PLANE_HEIGHT, ground.z);
        let rotation = Quat::from_rotation_arc(Vec3::NEG_Z, strike.dir);
        if strike.plane.is_none() {
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
            for &mark in &strike.marks {
                commands.entity(mark).try_despawn();
            }
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
        let fall_time = (2.0 * PLANE_HEIGHT / GRAVITY).sqrt();
        while strike.dropped < BOMBS {
            let along = -(BOMBS as f32 - 1.0) * BOMB_STEP * 0.5 + strike.dropped as f32 * BOMB_STEP;
            if flown + PLANE_SPEED * fall_time < along {
                break;
            }
            strike.dropped += 1;
            commands.spawn((
                Bomb {
                    velocity: strike.dir * PLANE_SPEED,
                },
                RunScoped,
                Mesh3d(assets.sphere.clone()),
                MeshMaterial3d(assets.bomb.clone()),
                Transform::from_translation(plane_pos - Vec3::Y).with_scale(Vec3::new(0.7, 0.7, 1.4)),
            ));
        }
        if flown > APPROACH {
            for e in [strike.plane, strike.shadow].into_iter().flatten() {
                commands.entity(e).try_despawn();
            }
            commands.entity(entity).despawn();
        }
    }
}

/// Бомбы падают по баллистике и взрываются о землю.
fn drop_bombs(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut bombs: Query<(Entity, &mut Transform, &mut Bomb)>,
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
            explosions.write(Explosion {
                position: Vec3::new(p.x, ground, p.z),
                radius: BOMB_RADIUS,
                power: BOMB_POWER,
                source_team: Team::Player,
            });
        }
    }
}
