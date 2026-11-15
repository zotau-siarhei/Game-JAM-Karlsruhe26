//! Лазер: импульс бьёт всё на отрезке луча и прорезает в холмах траншею с раскалёнными краями.
//!
//! Луч идёт горизонтально от башни к прицелу. Урон получают чужие машины и всё со здоровьем,
//! что стоит на пути: дома, деревья, валуны. Где холм выше луча, рельеф срезается чуть ниже
//! луча; где луч идёт низко над землёй, на ней остаётся выжженная полоса. Края прореза и полоса
//! светятся оранжевым и за несколько секунд остывают до тёмно-коричневого, вдоль них летят
//! искры и поднимается дым. Цвет вершин хранится в рельефе, поэтому полоса сохраняется.

use std::collections::HashSet;

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::aim::AimPoint;
use crate::arsenal::Inventory;
use crate::combat::{Collider, Damage, Health, Team};
use crate::frame::{GameSet, RunMessages};
use crate::obstacles::Obstacle;
use crate::rover::{self, Rover};
use crate::run::{RunScoped, WorldRebuilt};
use crate::subsidence::TerrainChanged;
use crate::terrain::{ChunkDirty, Terrain, VisibleChunks};
use crate::weapons::CurrentWeapon;

/// Перезарядка, с.
const RELOAD: f32 = 0.6;

/// Длина луча, м.
const BEAM_LENGTH: f32 = 40.0;

/// Урон импульса.
const BEAM_DAMAGE: f32 = 60.0;

/// Высота луча над центром лунохода, м.
const BEAM_HEIGHT: f32 = 0.9;

/// Сколько гаснет луч, с.
const BEAM_SECONDS: f32 = 0.2;

/// Полуширина траншеи, м.
const TRENCH_HALF_WIDTH: f32 = 1.2;

/// На сколько дно траншеи ниже луча, м.
const TRENCH_BELOW_BEAM: f32 = 0.4;

/// За сколько остывают края, с.
const COOL_SECONDS: f32 = 4.0;

/// Как часто пересобирать остывающие участки, с.
const COOL_STEP: f32 = 0.25;

/// Полуширина выжженной полосы на ровном месте, м.
const SCORCH_HALF_WIDTH: f32 = 0.8;

/// Насколько ниже луча земля ещё выжигается, м: луч высоко над низиной её не трогает.
const SCORCH_DEPTH: f32 = 3.0;

/// Искр и клубов дыма вдоль прореза на один импульс.
const EMBERS_PER_SHOT: usize = 14;
const SMOKE_PER_SHOT: usize = 6;

/// Сколько живут искра и клуб дыма, с.
const EMBER_SECONDS: f32 = 0.6;
const SMOKE_SECONDS: f32 = 2.5;

/// Сколько искр и клубов дыма лазера может быть сразу.
const MAX_FX: usize = 80;

/// Раскалённый край и остывший.
const HOT: [f32; 3] = [1.0, 0.45, 0.05];
const COLD: [f32; 3] = [0.10, 0.06, 0.04];

/// Подключает лазер.
pub struct LaserPlugin;

impl Plugin for LaserPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HotEdges>()
            .init_resource::<LaserReload>()
            .add_run_message::<FireLaser>()
            .add_systems(Startup, create_laser_assets)
            .add_systems(
                Update,
                (
                    reset_hot_edges.in_set(GameSet::Reset),
                    fire_laser.after(rover::drive_rover).in_set(GameSet::Input),
                    (apply_laser_shots, cool_edges).chain().in_set(GameSet::Simulation),
                    (fade_beams, animate_fx).in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Гаснущий луч. Появление луча — импульс лазера: по нему звучит выстрел.
#[derive(Component)]
pub struct Beam {
    remaining: f32,
    thickness: f32,
}

/// Раскалённые вершины траншеи: мировые координаты и время с прореза.
#[derive(Resource, Default)]
struct HotEdges {
    vertices: Vec<(IVec2, f32)>,
    since_step: f32,
}

/// Остаток перезарядки лазера лунохода, с.
#[derive(Resource, Default)]
struct LaserReload(f32);

/// Искра или клуб дыма над прорезом.
#[derive(Component)]
struct LaserFx {
    velocity: Vec3,
    remaining: f32,
    /// Дым растёт и всплывает, искра падает по баллистике.
    smoke: bool,
}

#[derive(Resource)]
struct LaserAssets {
    beam: Handle<Mesh>,
    beam_material: Handle<StandardMaterial>,
    enemy_beam_material: Handle<StandardMaterial>,
    ember: Handle<Mesh>,
    ember_material: Handle<StandardMaterial>,
    smoke: Handle<Mesh>,
    smoke_material: Handle<StandardMaterial>,
}

fn create_laser_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(LaserAssets {
        beam: meshes.add(Cylinder::new(1.0, 1.0)),
        beam_material: materials.add(StandardMaterial {
            base_color: Color::srgb(0.6, 0.95, 1.0),
            emissive: LinearRgba::rgb(4.0, 14.0, 20.0),
            unlit: true,
            ..default()
        }),
        enemy_beam_material: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.4, 0.3),
            emissive: LinearRgba::rgb(20.0, 4.0, 2.0),
            unlit: true,
            ..default()
        }),
        ember: meshes.add(Cuboid::new(0.1, 0.1, 0.1)),
        ember_material: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.7, 0.3),
            emissive: LinearRgba::rgb(10.0, 4.0, 0.8),
            unlit: true,
            ..default()
        }),
        smoke: meshes.add(Sphere::new(0.5).mesh().ico(1).unwrap()),
        smoke_material: materials.add(StandardMaterial {
            base_color: Color::srgba(0.25, 0.23, 0.22, 0.5),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..default()
        }),
    });
}

/// Мир перестроен: остывание и перезарядка прошлого забега не переходят в новый.
fn reset_hot_edges(
    mut started: MessageReader<WorldRebuilt>,
    mut hot: ResMut<HotEdges>,
    mut reload: ResMut<LaserReload>,
) {
    if started.read().last().is_some() {
        *hot = HotEdges::default();
        *reload = LaserReload::default();
    }
}

/// Расстояние от точки до отрезка в плане.
fn flat_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

/// Что сделал прорез.
#[derive(Default)]
pub struct TrenchCut {
    /// Участки с изменёнными вершинами.
    pub chunks: Vec<IVec2>,
    /// Срезанные вершины: рельеф стал ниже.
    pub cut: Vec<IVec2>,
    /// Выжженные вершины на ровном месте под лучом: высота та же, цвет раскалённый.
    pub scorched: Vec<IVec2>,
}

/// Срезает рельеф вдоль отрезка ниже высоты луча с плавным краем, а землю, над которой луч идёт
/// низко, выжигает узкой полосой.
///
/// Высоты только опускаются: на ровном месте ниже луча рельеф не меняется, только темнеет.
pub fn cut_trench(terrain: &mut Terrain, from: Vec2, to: Vec2, beam_y: f32) -> TrenchCut {
    let floor = beam_y - TRENCH_BELOW_BEAM;
    let reach = TRENCH_HALF_WIDTH + 1.0;
    let (x0, x1) = (from.x.min(to.x) - reach, from.x.max(to.x) + reach);
    let (z0, z1) = (from.y.min(to.y) - reach, from.y.max(to.y) + reach);
    let mut result = TrenchCut::default();
    let paint = |terrain: &mut Terrain, chunks: &mut Vec<IVec2>, vx: i32, vz: i32, limit: Option<f32>| {
        for c in terrain.edit_vertex(vx, vz, |h, color| {
            if let Some(limit) = limit {
                *h = h.min(limit);
            }
            color[0] = HOT[0];
            color[1] = HOT[1];
            color[2] = HOT[2];
        }) {
            if !chunks.contains(&c) {
                chunks.push(c);
            }
        }
    };
    for vz in z0.floor() as i32..=z1.ceil() as i32 {
        for vx in x0.floor() as i32..=x1.ceil() as i32 {
            let d = flat_distance(Vec2::new(vx as f32, vz as f32), from, to);
            if d > reach {
                continue;
            }
            // Внутри полуширины — дно, дальше плавный подъём к нетронутому.
            let edge = ((d - TRENCH_HALF_WIDTH) / (reach - TRENCH_HALF_WIDTH)).clamp(0.0, 1.0);
            let limit = floor + edge * edge * 3.0;
            let current = terrain.vertex_height(vx, vz);
            if current > limit {
                paint(terrain, &mut result.chunks, vx, vz, Some(limit));
                result.cut.push(IVec2::new(vx, vz));
            } else if d <= SCORCH_HALF_WIDTH && current > beam_y - SCORCH_DEPTH {
                paint(terrain, &mut result.chunks, vx, vz, None);
                result.scorched.push(IVec2::new(vx, vz));
            }
        }
    }
    result
}

/// Помечает на пересборку участки и их соседей по краю.
fn mark_dirty(commands: &mut Commands, visible: &VisibleChunks, chunks: impl IntoIterator<Item = IVec2>) {
    let mut dirty = HashSet::new();
    for c in chunks {
        for o in [IVec2::ZERO, IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y] {
            dirty.insert(c + o);
        }
    }
    for c in dirty {
        if let Some(e) = visible.entity(c) {
            commands.entity(e).try_insert(ChunkDirty);
        }
    }
}

/// Просьба выстрелить импульсом лазера: так стреляет и игрок, и тяжёлый танк врагов.
#[derive(Message, Debug, Clone, Copy)]
pub struct FireLaser {
    /// Начало луча.
    pub origin: Vec3,
    /// Куда идёт луч: берётся только направление в плане.
    pub direction: Vec3,
    pub team: Team,
    pub damage: f32,
}

/// Импульс лазера игрока по левой кнопке мыши.
fn fire_laser(
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    weapon: Res<CurrentWeapon>,
    aim: Res<AimPoint>,
    mut inventory: ResMut<Inventory>,
    mut reload: ResMut<LaserReload>,
    rover: Single<(&Transform, &Health), With<Rover>>,
    mut shots: MessageWriter<FireLaser>,
) {
    let cooldown = &mut reload.0;
    *cooldown = (*cooldown - time.delta_secs()).max(0.0);
    let (rover, health) = *rover;
    let trigger = mouse.pressed(MouseButton::Left)
        && *weapon == CurrentWeapon::Laser
        && inventory.usable(CurrentWeapon::Laser)
        && !health.is_dead();
    if !trigger || *cooldown > 0.0 {
        return;
    }
    *cooldown = RELOAD;
    inventory.spend(CurrentWeapon::Laser);
    let origin = rover.translation + Vec3::Y * BEAM_HEIGHT;
    shots.write(FireLaser {
        origin,
        direction: aim.position - origin,
        team: Team::Player,
        damage: BEAM_DAMAGE,
    });
}

/// Импульс: урон всем чужим на отрезке, траншея в рельефе, гаснущий луч.
#[allow(clippy::too_many_arguments)]
fn apply_laser_shots(
    mut commands: Commands,
    mut shots: MessageReader<FireLaser>,
    assets: Res<LaserAssets>,
    mut terrain: ResMut<Terrain>,
    visible: Res<VisibleChunks>,
    mut hot: ResMut<HotEdges>,
    targets: Query<(Entity, &GlobalTransform, &Collider, &Team, &Health)>,
    obstacles: Query<(Entity, &Obstacle), With<Health>>,
    fx: Query<(), With<LaserFx>>,
    mut damage: MessageWriter<Damage>,
    mut changed: MessageWriter<TerrainChanged>,
) {
    let mut fx_count = fx.iter().count();
    for shot in shots.read() {
        let start = shot.origin;
        let dir = shot.direction.with_y(0.0).normalize_or(Vec3::NEG_Z);
        let end = start + dir * BEAM_LENGTH;
        let (a, b) = (Vec2::new(start.x, start.z), Vec2::new(end.x, end.z));

        // Урон всем на отрезке, кроме своих.
        for (entity, transform, collider, team, target_health) in &targets {
            if *team == shot.team || target_health.is_dead() {
                continue;
            }
            let p = transform.translation();
            if flat_distance(Vec2::new(p.x, p.z), a, b) <= collider.radius {
                damage.write(Damage {
                    target: entity,
                    amount: shot.damage,
                    source_team: shot.team,
                });
            }
        }
        let steps = (BEAM_LENGTH / 0.5) as usize;
        for (entity, obstacle) in &obstacles {
            let hit = (0..=steps).any(|s| {
                let p = start.lerp(end, s as f32 / steps as f32);
                p.y <= obstacle.top && obstacle.shape.contains(Vec2::new(p.x, p.z))
            });
            if hit {
                damage.write(Damage {
                    target: entity,
                    amount: shot.damage,
                    source_team: shot.team,
                });
            }
        }

        // Траншея и выжженная полоса.
        let trench = cut_trench(&mut terrain, a, b, start.y);
        if !trench.cut.is_empty() {
            let mid = (start + end) * 0.5;
            changed.write(TerrainChanged {
                center: mid,
                radius: BEAM_LENGTH * 0.5 + TRENCH_HALF_WIDTH,
            });
        }
        mark_dirty(&mut commands, &visible, trench.chunks);
        let burnt: Vec<IVec2> = trench.cut.into_iter().chain(trench.scorched).collect();
        // Искры и дым поровну вдоль прореза.
        for (i, v) in spread(&burnt, EMBERS_PER_SHOT + SMOKE_PER_SHOT).into_iter().enumerate() {
            if fx_count >= MAX_FX {
                break;
            }
            fx_count += 1;
            let ground = Vec3::new(v.x as f32, terrain.vertex_height(v.x, v.y) + 0.1, v.y as f32);
            let smoke = i % ((EMBERS_PER_SHOT + SMOKE_PER_SHOT) / SMOKE_PER_SHOT) == 0;
            // Направления разлёта искр — по золотому углу: без случайности, но вразнобой.
            let angle = i as f32 * 2.399_963;
            let side = Vec3::new(angle.cos(), 0.0, angle.sin());
            let (mesh, material, velocity, remaining) = if smoke {
                (assets.smoke.clone(), assets.smoke_material.clone(), Vec3::Y * 0.8 + side * 0.2, SMOKE_SECONDS)
            } else {
                let lift = 3.0 + (i % 3) as f32 * 1.2;
                (assets.ember.clone(), assets.ember_material.clone(), Vec3::Y * lift + side * 2.0, EMBER_SECONDS)
            };
            commands.spawn((
                LaserFx {
                    velocity,
                    remaining,
                    smoke,
                },
                RunScoped,
                NotShadowCaster,
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::from_translation(ground),
            ));
        }
        hot.vertices.extend(burnt.into_iter().map(|v| (v, 0.0)));

        // Луч: у врагов красный.
        let material = if shot.team == Team::Player {
            assets.beam_material.clone()
        } else {
            assets.enemy_beam_material.clone()
        };
        let color = if shot.team == Team::Player {
            Color::srgb(0.6, 0.9, 1.0)
        } else {
            Color::srgb(1.0, 0.35, 0.25)
        };
        commands.spawn((
            Beam {
                remaining: BEAM_SECONDS,
                thickness: 0.18,
            },
            RunScoped,
            NotShadowCaster,
            Mesh3d(assets.beam.clone()),
            MeshMaterial3d(material),
            Transform::from_translation((start + end) * 0.5)
                .with_rotation(Quat::from_rotation_arc(Vec3::Y, dir))
                .with_scale(Vec3::new(0.18, BEAM_LENGTH, 0.18)),
            PointLight {
                color,
                intensity: 2_000_000.0,
                range: 25.0,
                shadow_maps_enabled: false,
                ..default()
            },
        ));
    }
}

/// До `count` вершин, равномерно разнесённых по списку.
fn spread(vertices: &[IVec2], count: usize) -> Vec<IVec2> {
    if vertices.is_empty() || count == 0 {
        return Vec::new();
    }
    let count = count.min(vertices.len());
    (0..count).map(|i| vertices[i * vertices.len() / count]).collect()
}

/// Искры летят по баллистике и гаснут, дым всплывает, растёт и тает.
fn animate_fx(mut commands: Commands, time: Res<Time>, mut fx: Query<(Entity, &mut Transform, &mut LaserFx)>) {
    let dt = time.delta_secs();
    for (entity, mut transform, mut fx) in &mut fx {
        fx.remaining -= dt;
        if fx.remaining <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        if fx.smoke {
            let t = fx.remaining / SMOKE_SECONDS;
            transform.scale = Vec3::splat((0.6 + (1.0 - t) * 2.2) * t.sqrt().min(1.0));
        } else {
            fx.velocity.y -= 14.0 * dt;
            transform.scale = Vec3::splat(fx.remaining / EMBER_SECONDS);
        }
        transform.translation += fx.velocity * dt;
    }
}

/// Луч тоньшает и гаснет.
fn fade_beams(mut commands: Commands, time: Res<Time>, mut beams: Query<(Entity, &mut Transform, &mut Beam, &mut PointLight)>) {
    for (entity, mut transform, mut beam, mut light) in &mut beams {
        beam.remaining -= time.delta_secs();
        if beam.remaining <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let t = beam.remaining / BEAM_SECONDS;
        transform.scale.x = beam.thickness * t;
        transform.scale.z = beam.thickness * t;
        light.intensity = 2_000_000.0 * t;
    }
}

/// Края траншеи остывают: цвет вершин смешивается к тёмному, участки пересобираются раз в четверть секунды.
fn cool_edges(
    mut commands: Commands,
    time: Res<Time>,
    mut hot: ResMut<HotEdges>,
    mut terrain: ResMut<Terrain>,
    visible: Res<VisibleChunks>,
) {
    if hot.vertices.is_empty() {
        return;
    }
    let dt = time.delta_secs();
    hot.since_step += dt;
    for v in &mut hot.vertices {
        v.1 += dt;
    }
    if hot.since_step < COOL_STEP {
        return;
    }
    hot.since_step = 0.0;
    let mut chunks = Vec::new();
    for &(v, age) in &hot.vertices {
        let t = (age / COOL_SECONDS).min(1.0);
        let k = t * t * (3.0 - 2.0 * t);
        let color = [0, 1, 2].map(|i| HOT[i] + (COLD[i] - HOT[i]) * k);
        for c in terrain.edit_vertex(v.x, v.y, |_, c| {
            c[0] = color[0];
            c[1] = color[1];
            c[2] = color[2];
        }) {
            if !chunks.contains(&c) {
                chunks.push(c);
            }
        }
    }
    hot.vertices.retain(|v| v.1 < COOL_SECONDS);
    mark_dirty(&mut commands, &visible, chunks);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trench_never_raises_and_keeps_low_ground() {
        let mut terrain = Terrain::new(3);
        let (from, to) = (Vec2::new(0.0, 0.0), Vec2::new(30.0, 0.0));
        let before: Vec<f32> = (-4..=34).flat_map(|x| (-4..=4).map(move |z| (x, z))).map(|(x, z)| terrain.vertex_height(x, z)).collect();
        let lowest = before.iter().copied().fold(f32::MAX, f32::min);
        // Луч выше всего рельефа: ничего не меняется.
        let high = cut_trench(&mut terrain, from, to, lowest + 100.0);
        assert!(high.cut.is_empty() && high.scorched.is_empty());
        // Луч посередине: высоты только опускаются.
        let mid = before.iter().sum::<f32>() / before.len() as f32;
        cut_trench(&mut terrain, from, to, mid);
        let after: Vec<f32> = (-4..=34).flat_map(|x| (-4..=4).map(move |z| (x, z))).map(|(x, z)| terrain.vertex_height(x, z)).collect();
        for (b, a) in before.iter().zip(&after) {
            assert!(a <= b);
            if *b <= mid - TRENCH_BELOW_BEAM {
                assert_eq!(a, b);
            }
        }
    }

    #[test]
    fn low_beam_scorches_flat_ground_without_digging() {
        let mut terrain = Terrain::new(3);
        let chunk = terrain.chunk_mut(IVec2::ZERO);
        for iz in 0..crate::terrain::CHUNK_VERTS {
            for ix in 0..crate::terrain::CHUNK_VERTS {
                chunk.heights[crate::terrain::ChunkData::index(ix, iz)] = 0.0;
            }
        }
        // Ровная площадка, луч в метре над ней: рельеф не срезан, полоса выжжена.
        let trench = cut_trench(&mut terrain, Vec2::new(4.0, 10.0), Vec2::new(20.0, 10.0), 1.0);
        assert!(trench.cut.is_empty());
        assert!(trench.scorched.contains(&IVec2::new(12, 10)));
        assert!(!trench.scorched.contains(&IVec2::new(12, 12)));
        assert_eq!(terrain.vertex_height(12, 10), 0.0);
        let color = terrain.vertex_color(12, 10).expect("участок есть");
        assert_eq!(&color[..3], &HOT[..]);
    }
}
