//! Птицы: стайки на ровных низинах, которые взлетают от шума боя и садятся в другом месте.
//!
//! Место стаи выбирается по хешу сида и координат участка: примерно в каждом третьем участке,
//! на пологой низине вне гор, путей, лавы и домов. Травы пока нет (`grass` не сделан), поэтому
//! «поляна» здесь — ровное место ниже [`MEADOW_MAX_HEIGHT`]. Стая — корень с 5–12 птицами из общих
//! мешей: ромб-тело и два крыла. Птицы отбрасывают тень, поэтому сверху их видно и в полёте.
//!
//! Испуг: взрыв ближе [`SCARE_BLAST`], очередь ближе [`SCARE_SHOT`] или машина ближе
//! [`SCARE_VEHICLE`]. Стая взлетает вся сразу, птицы разлетаются от источника и набирают высоту,
//! через [`SCATTER_TIME`] собираются и кружат вокруг летящего по дуге центра. Через 10–20 с стая
//! садится на ровное место дальше [`MIN_LANDING`] от места взлёта, а если такого нет среди
//! загруженных участков или луноход далеко, пропадает. Место взлёта остаётся пустым до конца
//! забега. У птиц нет коллайдера и здоровья: пули, снаряды и машины их не замечают.

use std::collections::{HashMap, HashSet};
use std::f32::consts::{PI, TAU};

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::buildings::House;
use crate::bullet::FireBullet;
use crate::camera::Focus;
use crate::explosion::Explosion;
use crate::frame::GameSet;
use crate::lava::{self, LavaPools};
use crate::puddles::{self, Puddles};
use crate::railway::TRACK_BAND;
use crate::rover::Rover;
use crate::run::{RunScoped, WorldRebuilt};
use crate::terrain::{CHUNK_SIZE, LOAD_RADIUS, Terrain, VisibleChunks, chunk_coord_at, chunk_origin};
use crate::vehicle::{GroundVehicle, heading_towards};

/// Доля участков со стаей.
const FLOCK_CHANCE: f32 = 1.0 / 3.0;

/// Птиц в стае.
const FLOCK_SIZE: (u32, u32) = (5, 12);

/// Радиус, в котором сидят птицы стаи, м.
const FLOCK_RADIUS: f32 = 2.5;

/// Выше этого — горы, там птиц нет, м.
const MEADOW_MAX_HEIGHT: f32 = 8.0;

/// Склон поляны не круче: нормаль по Y не меньше.
const MEADOW_MIN_UP: f32 = 0.96;

/// Ближе этого к старту стай нет: луноход стоит там в начале забега, м.
const START_CLEAR: f32 = 25.0;

/// Ближе этого к дому стай нет, м.
const HOUSE_CLEAR: f32 = 8.0;

/// Испуг: взрыв, выстрел и машина ближе этого, м.
const SCARE_BLAST: f32 = 40.0;
const SCARE_SHOT: f32 = 25.0;
const SCARE_VEHICLE: f32 = 8.0;

/// Сколько птицы разлетаются, прежде чем собраться в стаю, с.
const SCATTER_TIME: f32 = 3.0;

/// Сколько стая кружит до посадки, с.
const FLIGHT_TIME: (f32, f32) = (10.0, 20.0);

/// Высота полёта, м.
const ALTITUDE: (f32, f32) = (12.0, 18.0);

/// Скорость центра кружащей стаи и садящейся стаи, м/с.
const CIRCLE_SPEED: f32 = 6.0;
const LANDING_SPEED: f32 = 8.0;

/// Новое место посадки не ближе этого к месту взлёта, м.
const MIN_LANDING: f32 = 30.0;

/// Летящая стая дальше этого от лунохода пропадает, м.
const MAX_DISTANCE: f32 = 80.0;

/// Частота взмахов крыльев, Гц.
const FLAP_RATE: f32 = 8.0;

/// Размер птицы: множитель общих мешей. Чуть крупнее жизни: камера далеко.
const BIRD_SCALE: f32 = 1.3;

/// Плагин птиц.
pub struct BirdsPlugin;

impl Plugin for BirdsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Birds>()
            .add_message::<BirdsTookOff>()
            .add_systems(Startup, create_assets)
            .add_systems(
                Update,
                (
                    reset_birds.in_set(GameSet::Reset),
                    stream_flocks.after(puddles::place_puddles).in_set(GameSet::Streaming),
                    (scare_flocks, fly_flocks, animate_birds).chain().in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Стая взлетела: для звука взлёта.
#[derive(Message, Debug, Clone, Copy)]
pub struct BirdsTookOff {
    pub position: Vec3,
}

/// Стаи забега.
#[derive(Resource, Default)]
struct Birds {
    /// Участки, которые уже проверены: стая на земле там или `None`, если стаи нет.
    checked: HashMap<IVec2, Option<Entity>>,
    /// Участки, откуда стая улетела: там до конца забега пусто.
    flown: HashSet<IVec2>,
}

/// Общие меши и материал птиц.
#[derive(Resource)]
struct BirdAssets {
    body: Handle<Mesh>,
    wing: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

/// Стая: корень, птицы — дочерние сущности.
#[derive(Component)]
struct Flock {
    /// Участок, на котором стая сидит.
    home: IVec2,
    state: FlockState,
    /// Откуда взлетела.
    takeoff: Vec3,
    /// Курс центра в полёте, рад.
    heading: f32,
    /// Генератор разброса стаи.
    rng: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum FlockState {
    Sitting,
    Flying { age: f32, duration: f32 },
    Landing { target: Vec3, chunk: IVec2 },
}

/// Птица стаи.
#[derive(Component)]
struct Bird {
    /// Сдвиг фазы взмахов и кружения, рад.
    phase: f32,
    /// Где сидит относительно центра стаи на плане.
    seat: Vec2,
    /// Скорость в полёте относительно центра стаи, м/с.
    velocity: Vec3,
    /// Своя высота полёта, м.
    altitude: f32,
    /// Радиус кружения вокруг центра, м.
    orbit: f32,
    /// До следующего прыжка на земле и с начала прыжка, с.
    hop_in: f32,
    hop_age: f32,
    /// Курс на земле, рад.
    yaw: f32,
}

/// Крыло: `-1` — левое, `1` — правое.
#[derive(Component)]
struct Wing(f32);

/// Хеш сида, участка и номера: число от 0 до 1.
fn hash(seed: u32, coord: IVec2, salt: u32) -> f32 {
    let mut x = seed ^ (coord.x as u32).wrapping_mul(0x9E37_79B1) ^ (coord.y as u32).wrapping_mul(0x85EB_CA77);
    x ^= salt.wrapping_mul(0xC2B2_AE3D);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

/// Следующее число генератора стаи от 0 до 1.
fn next(rng: &mut u32) -> f32 {
    let mut x = *rng;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *rng = x;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

/// Ровное место для стаи в участке, если оно есть: три попытки по хешу.
fn meadow_point(terrain: &Terrain, coord: IVec2) -> Option<Vec2> {
    let origin = chunk_origin(coord);
    let margin = 4.0;
    let span = CHUNK_SIZE as f32 - 2.0 * margin;
    (0..3).find_map(|i| {
        let p = origin
            + Vec2::new(
                margin + hash(terrain.seed(), coord, 10 + i) * span,
                margin + hash(terrain.seed(), coord, 20 + i) * span,
            );
        let ok = p.length() > START_CLEAR
            && terrain.height_at(p.x, p.y) < MEADOW_MAX_HEIGHT
            && terrain.normal_at(p.x, p.y).y > MEADOW_MIN_UP
            && terrain.lines().nearest(p).distance > TRACK_BAND;
        ok.then_some(p)
    })
}

/// Место и число птиц стаи участка. Зависит только от сида и рельефа.
pub fn flock_site(terrain: &Terrain, coord: IVec2) -> Option<(Vec2, u32)> {
    if hash(terrain.seed(), coord, 1) >= FLOCK_CHANCE {
        return None;
    }
    let p = meadow_point(terrain, coord)?;
    let span = (FLOCK_SIZE.1 - FLOCK_SIZE.0 + 1) as f32;
    let count = FLOCK_SIZE.0 + (hash(terrain.seed(), coord, 2) * span) as u32;
    Some((p, count.min(FLOCK_SIZE.1)))
}

fn create_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Тело — вытянутый ромб: нос на -Z, хвост на +Z.
    let body = {
        let v = [
            [0.0, 0.0, -0.17],
            [0.0, 0.0, 0.15],
            [-0.06, 0.0, 0.0],
            [0.06, 0.0, 0.0],
            [0.0, 0.06, 0.0],
            [0.0, -0.04, 0.0],
        ];
        let faces: [u32; 24] = [0, 4, 2, 0, 3, 4, 1, 2, 4, 1, 4, 3, 0, 2, 5, 0, 5, 3, 1, 5, 2, 1, 3, 5];
        flat_mesh(&v, &faces)
    };
    // Крыло — треугольник вправо от плеча; левое крыло — то же с отражением.
    let wing = flat_mesh(&[[0.0, 0.0, -0.07], [0.0, 0.0, 0.07], [0.34, 0.0, 0.05]], &[0, 1, 2]);
    commands.insert_resource(BirdAssets {
        body: meshes.add(body),
        wing: meshes.add(wing),
        material: materials.add(StandardMaterial {
            base_color: Color::srgb(0.12, 0.11, 0.12),
            perceptual_roughness: 0.9,
            cull_mode: None,
            double_sided: true,
            ..default()
        }),
    });
}

/// Меш из треугольников с плоскими нормалями.
fn flat_mesh(points: &[[f32; 3]], faces: &[u32]) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    for tri in faces.chunks(3) {
        let [a, b, c] = [0, 1, 2].map(|i| Vec3::from_array(points[tri[i] as usize]));
        let n = (b - a).cross(c - a).normalize_or(Vec3::Y);
        for p in [a, b, c] {
            positions.push(p.to_array());
            normals.push(n.to_array());
        }
    }
    let indices = (0..positions.len() as u32).collect();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_indices(Indices::U32(indices))
}

/// Новый мир: стай прошлого мира нет, все места снова свободны.
fn reset_birds(mut rebuilt: MessageReader<WorldRebuilt>, mut birds: ResMut<Birds>) {
    if rebuilt.read().last().is_some() {
        *birds = Birds::default();
    }
}

/// Сажает стаи на загруженные участки и убирает сидящие стаи с выгруженных.
#[allow(clippy::too_many_arguments)]
fn stream_flocks(
    mut commands: Commands,
    mut birds: ResMut<Birds>,
    terrain: Res<Terrain>,
    pools: Res<LavaPools>,
    puddles: Res<Puddles>,
    visible: Res<VisibleChunks>,
    focus: Res<Focus>,
    houses: Query<&GlobalTransform, With<House>>,
    assets: Res<BirdAssets>,
) {
    let birds = &mut *birds;
    birds.checked.retain(|&coord, flock| {
        let keep = visible.entity(coord).is_some();
        if !keep && let Some(entity) = flock {
            commands.entity(*entity).try_despawn();
        }
        keep
    });
    let center = chunk_coord_at(focus.0.x, focus.0.z);
    for dz in -LOAD_RADIUS..=LOAD_RADIUS {
        for dx in -LOAD_RADIUS..=LOAD_RADIUS {
            let coord = center + IVec2::new(dx, dz);
            if visible.entity(coord).is_none() || birds.checked.contains_key(&coord) {
                continue;
            }
            let site = if birds.flown.contains(&coord) {
                None
            } else {
                flock_site(&terrain, coord).filter(|(p, _)| {
                    let at = Vec3::new(p.x, 0.0, p.y);
                    !lava::is_lava(&pools, at)
                        && !puddles.at(*p)
                        && houses.iter().all(|h| h.translation().xz().distance(*p) > HOUSE_CLEAR)
                })
            };
            let flock = site.map(|(p, count)| {
                let rng = (hash(terrain.seed(), coord, 3) * u32::MAX as f32) as u32 | 1;
                spawn_flock(&mut commands, &assets, &terrain, coord, p, count, rng)
            });
            birds.checked.insert(coord, flock);
        }
    }
}

/// Стая из `count` птиц, сидящая в точке `p`.
fn spawn_flock(
    commands: &mut Commands,
    assets: &BirdAssets,
    terrain: &Terrain,
    coord: IVec2,
    p: Vec2,
    count: u32,
    mut rng: u32,
) -> Entity {
    let root = Vec3::new(p.x, terrain.height_at(p.x, p.y), p.y);
    commands
        .spawn((
            Flock {
                home: coord,
                state: FlockState::Sitting,
                takeoff: root,
                heading: 0.0,
                rng,
            },
            RunScoped,
            Transform::from_translation(root),
            Visibility::default(),
        ))
        .with_children(|parent| {
            for _ in 0..count {
                let angle = next(&mut rng) * TAU;
                let seat = Vec2::from_angle(angle) * FLOCK_RADIUS * next(&mut rng).sqrt();
                let yaw = next(&mut rng) * TAU;
                parent
                    .spawn((
                        Bird {
                            phase: next(&mut rng) * TAU,
                            seat,
                            velocity: Vec3::ZERO,
                            altitude: ALTITUDE.0 + next(&mut rng) * (ALTITUDE.1 - ALTITUDE.0),
                            orbit: 2.0 + next(&mut rng) * 4.0,
                            hop_in: 1.0 + next(&mut rng) * 2.0,
                            hop_age: 1.0,
                            yaw,
                        },
                        Mesh3d(assets.body.clone()),
                        MeshMaterial3d(assets.material.clone()),
                        Transform::from_xyz(seat.x, 0.0, seat.y)
                            .with_rotation(Quat::from_rotation_y(yaw))
                            .with_scale(Vec3::splat(BIRD_SCALE)),
                    ))
                    .with_children(|bird| {
                        for side in [-1.0f32, 1.0] {
                            bird.spawn((
                                Wing(side),
                                Mesh3d(assets.wing.clone()),
                                MeshMaterial3d(assets.material.clone()),
                                Transform::from_xyz(side * 0.03, 0.03, -0.02).with_scale(Vec3::new(side, 1.0, 1.0)),
                            ));
                        }
                    });
            }
        })
        .id()
}

/// Поднимает сидящие стаи от взрывов, очередей и машин рядом.
#[allow(clippy::too_many_arguments)]
fn scare_flocks(
    mut birds: ResMut<Birds>,
    mut explosions: MessageReader<Explosion>,
    mut shots: MessageReader<FireBullet>,
    vehicles: Query<&GlobalTransform, Or<(With<GroundVehicle>, With<Rover>)>>,
    mut flocks: Query<(&Transform, &mut Flock, &Children)>,
    mut flock_birds: Query<&mut Bird>,
    mut took_off: MessageWriter<BirdsTookOff>,
) {
    let mut sources: Vec<(Vec2, f32)> = explosions.read().map(|e| (e.position.xz(), SCARE_BLAST)).collect();
    sources.extend(shots.read().map(|s| (s.origin.xz(), SCARE_SHOT)));
    sources.extend(vehicles.iter().map(|v| (v.translation().xz(), SCARE_VEHICLE)));
    for (transform, mut flock, children) in &mut flocks {
        if flock.state != FlockState::Sitting {
            continue;
        }
        let here = transform.translation.xz();
        let Some(&(from, _)) = sources.iter().find(|(p, reach)| p.distance_squared(here) < reach * reach) else {
            continue;
        };
        let away = (here - from).normalize_or(Vec2::X);
        let duration = FLIGHT_TIME.0 + next(&mut flock.rng) * (FLIGHT_TIME.1 - FLIGHT_TIME.0);
        flock.state = FlockState::Flying { age: 0.0, duration };
        flock.takeoff = transform.translation;
        flock.heading = heading_towards(Vec3::new(away.x, 0.0, away.y));
        birds.flown.insert(flock.home);
        birds.checked.insert(flock.home, None);
        for child in children.iter() {
            let Ok(mut bird) = flock_birds.get_mut(child) else {
                continue;
            };
            let spread = Vec2::from_angle((next(&mut flock.rng) - 0.5) * 1.6).rotate(away);
            let speed = 4.0 + next(&mut flock.rng) * 3.0;
            bird.velocity = Vec3::new(spread.x * speed, 4.0 + next(&mut flock.rng) * 2.0, spread.y * speed);
        }
        took_off.write(BirdsTookOff {
            position: transform.translation,
        });
    }
}

/// Полёт стаи: разлёт, кружение, выбор места и посадка.
#[allow(clippy::too_many_arguments)]
fn fly_flocks(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    pools: Res<LavaPools>,
    visible: Res<VisibleChunks>,
    focus: Res<Focus>,
    mut birds: ResMut<Birds>,
    rover: Query<&GlobalTransform, With<Rover>>,
    mut flocks: Query<(Entity, &mut Transform, &mut Flock)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let rover = rover.iter().next().map(GlobalTransform::translation);
    for (entity, mut transform, mut flock) in &mut flocks {
        match flock.state {
            FlockState::Sitting => {}
            FlockState::Flying { age, duration } => {
                let age = age + dt;
                if age > SCATTER_TIME {
                    // Центр летит по дуге.
                    flock.heading += 0.35 * dt;
                    let forward = Quat::from_rotation_y(flock.heading) * Vec3::NEG_Z;
                    transform.translation += forward * CIRCLE_SPEED * dt;
                }
                let far = rover.is_some_and(|r| r.xz().distance(transform.translation.xz()) > MAX_DISTANCE);
                if far {
                    commands.entity(entity).try_despawn();
                    continue;
                }
                if age < duration {
                    flock.state = FlockState::Flying { age, duration };
                    continue;
                }
                // Ближайшее свободное ровное место среди загруженных участков дальше 30 м от взлёта.
                let center = chunk_coord_at(focus.0.x, focus.0.z);
                let here = transform.translation.xz();
                let target = (-LOAD_RADIUS..=LOAD_RADIUS)
                    .flat_map(|dz| (-LOAD_RADIUS..=LOAD_RADIUS).map(move |dx| center + IVec2::new(dx, dz)))
                    .filter(|c| {
                        visible.entity(*c).is_some()
                            && !birds.flown.contains(c)
                            && !matches!(birds.checked.get(c), Some(Some(_)))
                    })
                    .filter_map(|c| meadow_point(&terrain, c).map(|p| (c, p)))
                    .filter(|(_, p)| {
                        p.distance(flock.takeoff.xz()) > MIN_LANDING && !lava::is_lava(&pools, Vec3::new(p.x, 0.0, p.y))
                    })
                    .min_by(|a, b| a.1.distance_squared(here).total_cmp(&b.1.distance_squared(here)));
                match target {
                    Some((chunk, p)) => {
                        let target = Vec3::new(p.x, terrain.height_at(p.x, p.y), p.y);
                        // Место занято этой стаей: другая туда не сядет.
                        birds.checked.insert(chunk, Some(entity));
                        flock.state = FlockState::Landing { target, chunk };
                    }
                    None => {
                        commands.entity(entity).try_despawn();
                    }
                }
            }
            FlockState::Landing { target, chunk } => {
                let to = target - transform.translation;
                let step = LANDING_SPEED * dt;
                if to.xz().length() <= step {
                    transform.translation = target;
                    flock.home = chunk;
                    flock.state = FlockState::Sitting;
                } else {
                    let dir = to.xz().normalize();
                    transform.translation += Vec3::new(dir.x, 0.0, dir.y) * step;
                    transform.translation.y += (target.y - transform.translation.y) * (2.0 * dt).min(1.0);
                    flock.heading = heading_towards(Vec3::new(dir.x, 0.0, dir.y));
                }
            }
        }
    }
}

/// Птицы: на земле поворачиваются и подпрыгивают, в полёте машут крыльями и кружат.
fn animate_birds(
    time: Res<Time>,
    terrain: Res<Terrain>,
    flocks: Query<(&Transform, &Flock, &Children), (Without<Bird>, Without<Wing>)>,
    mut flock_birds: Query<(&mut Transform, &mut Bird, &Children), Without<Wing>>,
    mut wings: Query<(&mut Transform, &Wing), (Without<Bird>, Without<Flock>)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let t = time.elapsed_secs();
    for (root, flock, children) in &flocks {
        for child in children.iter() {
            let Ok((mut transform, mut bird, bird_children)) = flock_birds.get_mut(child) else {
                continue;
            };
            let flying = match flock.state {
                FlockState::Sitting => {
                    // Птица садится на своё место и изредка подпрыгивает с поворотом.
                    bird.hop_in -= dt;
                    bird.hop_age += dt;
                    if bird.hop_in <= 0.0 {
                        bird.hop_in = 1.0 + (bird.phase * 7.3 + t).sin().abs() * 2.0;
                        bird.hop_age = 0.0;
                        bird.yaw += (t * 3.1 + bird.phase).sin() * 1.5;
                    }
                    let hop = if bird.hop_age < 0.3 { (bird.hop_age / 0.3 * PI).sin() * 0.15 } else { 0.0 };
                    let world = root.translation + Vec3::new(bird.seat.x, 0.0, bird.seat.y);
                    let ground = terrain.height_at(world.x, world.z) - root.translation.y;
                    let wanted = Vec3::new(bird.seat.x, ground + hop, bird.seat.y);
                    // Мягко, чтобы севшая стая не дёргалась к местам.
                    transform.translation = transform.translation.lerp(wanted, (6.0 * dt).min(1.0));
                    transform.rotation = Quat::from_rotation_y(bird.yaw);
                    false
                }
                FlockState::Flying { age, .. } if age < SCATTER_TIME => {
                    // Разлёт: набор высоты от источника испуга.
                    bird.velocity.y += (bird.altitude - transform.translation.y).clamp(-4.0, 4.0) * dt;
                    let v = bird.velocity;
                    transform.translation += v * dt;
                    face(&mut transform, v);
                    true
                }
                FlockState::Flying { age, .. } => {
                    // Кружение: каждая птица на своей окружности вокруг центра.
                    let a = age * 0.9 + bird.phase;
                    let target = Vec3::new(a.cos() * bird.orbit, bird.altitude, a.sin() * bird.orbit);
                    let v = (target - transform.translation) * 1.5;
                    bird.velocity = v;
                    transform.translation += v * dt;
                    let forward = Quat::from_rotation_y(flock.heading) * Vec3::NEG_Z * CIRCLE_SPEED;
                    face(&mut transform, v + forward);
                    true
                }
                FlockState::Landing { target, .. } => {
                    // Снижение: высота падает вместе с расстоянием до места.
                    let left = (target - root.translation).xz().length();
                    let height = (left * 0.5).min(bird.altitude);
                    let wanted = Vec3::new(bird.seat.x, height, bird.seat.y);
                    let v = (wanted - transform.translation) * 2.0;
                    transform.translation += v * dt;
                    let forward = Quat::from_rotation_y(flock.heading) * Vec3::NEG_Z * LANDING_SPEED;
                    face(&mut transform, forward);
                    true
                }
            };
            for wing in bird_children.iter() {
                let Ok((mut wing_transform, side)) = wings.get_mut(wing) else {
                    continue;
                };
                wing_transform.rotation = if flying {
                    Quat::from_rotation_z(side.0 * (t * FLAP_RATE * TAU + bird.phase).sin() * 0.9)
                } else {
                    // Сложены вдоль тела назад.
                    Quat::from_rotation_y(-side.0 * 1.3)
                };
            }
        }
    }
}

/// Поворачивает птицу клювом по горизонтальной скорости.
fn face(transform: &mut Transform, velocity: Vec3) {
    if velocity.xz().length_squared() > 0.01 {
        transform.rotation = Quat::from_rotation_y(heading_towards(velocity.with_y(0.0)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn тот_же_сид_даёт_те_же_стаи() {
        let a = Terrain::new(42);
        let b = Terrain::new(42);
        let mut found = 0;
        for z in -6..6 {
            for x in -6..6 {
                let coord = IVec2::new(x, z);
                let (sa, sb) = (flock_site(&a, coord), flock_site(&b, coord));
                assert_eq!(sa, sb);
                found += usize::from(sa.is_some());
            }
        }
        assert!(found > 0, "на 144 участках нет ни одной стаи");
        // Другой сид — другие места.
        let c = Terrain::new(43);
        let differs = (-6..6).any(|x| flock_site(&a, IVec2::new(x, 0)) != flock_site(&c, IVec2::new(x, 0)));
        assert!(differs);
    }

    #[test]
    fn стая_в_пределах_числа_птиц_и_участка() {
        let terrain = Terrain::new(7);
        for z in -5..5 {
            for x in -5..5 {
                let coord = IVec2::new(x, z);
                if let Some((p, count)) = flock_site(&terrain, coord) {
                    assert!((FLOCK_SIZE.0..=FLOCK_SIZE.1).contains(&count));
                    assert_eq!(chunk_coord_at(p.x, p.y), coord);
                }
            }
        }
    }
}
