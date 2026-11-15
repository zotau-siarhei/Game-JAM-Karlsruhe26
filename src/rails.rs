//! Звенья путей: два рельса, шпалы и подсыпка вдоль линий железной дороги.
//!
//! Звенья по 4 м создаются и удаляются вместе с участками рельефа. Звено лежит на текущей
//! поверхности и наклоняется по ней. В туннеле звеньев нет: там путь скрыт горой.
//!
//! Звено рвётся от взрыва не слабее ракеты и когда рельеф под его концом просел ниже уровня путей.
//! Разрушенные звенья хранятся в [`TrackDamage`] до конца забега. Разрушенное звено рисуется
//! изломанными кусками рельсов, свисающими к текущей поверхности, и сбитыми шпалами; вид задаётся
//! хешем индекса звена, поэтому после отъезда и возвращения он тот же.
//!
//! На входах в туннель стоят каменные порталы с тёмным проёмом и коллайдером. Если выстрелами
//! срыть гору над туннелем ниже уровня путей плюс 6 м, участок туннеля в 8 м вскрывается: полоса
//! над путями опускается до их уровня с откосами, летят камни, звенья под ним — завал.

use std::collections::{HashMap, HashSet};

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::explosion::Explosion;
use crate::debris::SpawnDebris;
use crate::obstacles::{Obstacle, Shape};
use crate::railway::{Family, LineId, Lines, TRACK_BAND, TRACK_FLAT, TUNNEL_DEPTH};
use crate::settlements::Footprint;
use crate::run::{RunScoped, RunStarted};
use crate::subsidence::TerrainChanged;
use crate::terrain::{CHUNK_SIZE, ChunkDirty, ChunkMesh, Terrain, VisibleChunks, chunk_coord_at, chunk_origin};

/// Длина звена, м.
pub const LINK_LENGTH: f32 = 4.0;

/// Половина ширины колеи, м.
const GAUGE_HALF: f32 = 0.72;

/// Шпал на звено.
const SLEEPERS: usize = 6;

/// Взрыв слабее этого путей не рвёт: пулемёт и мелочь безопасны, ракета уже рвёт.
const BREAK_POWER: f32 = 0.45;

/// Рельеф под концом звена ниже уровня путей на столько — звено разорвано, м.
const BREAK_SAG: f32 = 0.5;

/// Длина участка туннеля, который вскрывается целиком, м.
const TUNNEL_SEGMENT: f32 = 8.0;

/// Гора над туннелем ниже уровня путей плюс столько — свод вскрыт, м.
const OPEN_DEPTH: f32 = 6.0;

/// Портал: полуширина, высота и толщина, м.
const PORTAL_HALF_WIDTH: f32 = 3.4;
const PORTAL_HEIGHT: f32 = 6.5;
const PORTAL_DEPTH: f32 = 2.0;

/// Подключает звенья путей.
pub struct RailsPlugin;

impl Plugin for RailsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpawnedLinks>()
            .init_resource::<TrackDamage>()
            .add_systems(Startup, create_rail_assets)
            .add_systems(Update, (forget_links, open_tunnels, stream_links, break_links).chain());
    }
}

/// Адрес звена: линия и номер звена вдоль неё.
pub type LinkKey = (LineId, i32);

/// Разрушенные звенья забега.
#[derive(Resource, Default)]
pub struct TrackDamage {
    broken: HashSet<LinkKey>,
    /// Вскрытые участки туннелей: линия и номер участка длиной [`TUNNEL_SEGMENT`].
    opened: HashSet<LinkKey>,
}

impl TrackDamage {
    /// Проходит ли путь на параметре `s` в невскрытом туннеле.
    pub fn in_tunnel(&self, terrain: &Terrain, line: LineId, s: f32) -> bool {
        let p = terrain.lines().point(line, s);
        terrain.raw_height(p.x, p.y) > terrain.track_level(line, s) + TUNNEL_DEPTH
            && !self.opened.contains(&(line, (s / TUNNEL_SEGMENT).floor() as i32))
    }

    /// Разрушено ли звено, на которое приходится параметр пути `s`.
    // Читают поезда (задача 4.1).
    #[allow(dead_code)]
    pub fn is_broken(&self, line: LineId, s: f32) -> bool {
        self.broken.contains(&(line, (s / LINK_LENGTH).floor() as i32))
    }

    /// Отмечает звено разрушенным; `true`, если раньше оно было целым.
    pub fn break_link(&mut self, key: LinkKey) -> bool {
        self.broken.insert(key)
    }
}

/// Звено в сцене: адрес, концы в плане и сущность.
struct SpawnedLink {
    key: LinkKey,
    a: Vec2,
    b: Vec2,
    entity: Entity,
}

/// Звенья и порталы по участкам рельефа, которые сейчас в сцене.
#[derive(Resource, Default)]
struct SpawnedLinks {
    links: HashMap<IVec2, Vec<SpawnedLink>>,
    portals: HashMap<IVec2, Vec<Entity>>,
    /// Участки, звенья которых надо создать заново: туннель над ними вскрыт.
    refresh: HashSet<IVec2>,
}

#[derive(Resource)]
struct RailAssets {
    cube: Handle<Mesh>,
    rail: Handle<StandardMaterial>,
    sleeper: Handle<StandardMaterial>,
    ballast: Handle<StandardMaterial>,
    stone: Handle<StandardMaterial>,
    dark: Handle<StandardMaterial>,
}

fn create_rail_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(RailAssets {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        rail: materials.add(StandardMaterial {
            base_color: Color::srgb(0.62, 0.64, 0.68),
            metallic: 0.8,
            perceptual_roughness: 0.35,
            ..default()
        }),
        sleeper: materials.add(Color::srgb(0.36, 0.25, 0.16)),
        ballast: materials.add(StandardMaterial {
            base_color: Color::srgb(0.42, 0.4, 0.38),
            perceptual_roughness: 1.0,
            ..default()
        }),
        stone: materials.add(StandardMaterial {
            base_color: Color::srgb(0.5, 0.48, 0.45),
            perceptual_roughness: 0.95,
            ..default()
        }),
        dark: materials.add(StandardMaterial {
            base_color: Color::srgb(0.02, 0.02, 0.025),
            unlit: true,
            ..default()
        }),
    });
}

fn forget_links(
    mut started: MessageReader<RunStarted>,
    mut spawned: ResMut<SpawnedLinks>,
    mut damage: ResMut<TrackDamage>,
) {
    if started.read().last().is_some() {
        // Сущности прошлого забега удалены через RunScoped.
        *spawned = SpawnedLinks::default();
        *damage = TrackDamage::default();
    }
}

/// Концы звеньев, которые начинаются в участке: по линиям, проходящим рядом с ним.
fn links_in_chunk(lines: &Lines, coord: IVec2) -> Vec<(LinkKey, Vec2, Vec2)> {
    let origin = chunk_origin(coord);
    let size = CHUNK_SIZE as f32;
    let center = origin + Vec2::splat(size * 0.5);
    let mut links = Vec::new();
    for near in lines.near(center, size) {
        let line = near.line;
        let (from, to) = match line.family {
            Family::AlongX => (origin.x, origin.x + size),
            Family::AlongZ => (origin.y, origin.y + size),
        };
        let first = (from / LINK_LENGTH).floor() as i32;
        let last = (to / LINK_LENGTH).ceil() as i32;
        for i in first..=last {
            let s0 = i as f32 * LINK_LENGTH;
            let a = lines.point(line, s0);
            if chunk_coord_at(a.x, a.y) != coord {
                continue;
            }
            links.push(((line, i), a, lines.point(line, s0 + LINK_LENGTH)));
        }
    }
    links
}

/// Создаёт звенья участков, появившихся в сцене, и убирает звенья выгруженных.
fn stream_links(
    mut commands: Commands,
    mut spawned: ResMut<SpawnedLinks>,
    terrain: Res<Terrain>,
    damage: Res<TrackDamage>,
    visible: Res<VisibleChunks>,
    chunks: Query<&ChunkMesh>,
    assets: Res<RailAssets>,
) {
    if !visible.is_changed() && spawned.refresh.is_empty() {
        return;
    }
    let spawned = &mut *spawned;
    // Участки со вскрытым туннелем пересоздаются целиком.
    for coord in std::mem::take(&mut spawned.refresh) {
        for link in spawned.links.remove(&coord).unwrap_or_default() {
            commands.entity(link.entity).try_despawn();
        }
        for portal in spawned.portals.remove(&coord).unwrap_or_default() {
            commands.entity(portal).try_despawn();
        }
    }
    spawned.links.retain(|&coord, links| {
        let keep = visible.entity(coord).is_some();
        if !keep {
            for link in links.iter() {
                commands.entity(link.entity).try_despawn();
            }
        }
        keep
    });
    spawned.portals.retain(|&coord, portals| {
        let keep = visible.entity(coord).is_some();
        if !keep {
            for &portal in portals.iter() {
                commands.entity(portal).try_despawn();
            }
        }
        keep
    });
    for chunk in &chunks {
        let coord = chunk.0;
        if spawned.links.contains_key(&coord) || visible.entity(coord).is_none() {
            continue;
        }
        let mut links = Vec::new();
        let mut portals = Vec::new();
        for (key, a, b) in links_in_chunk(terrain.lines(), coord) {
            let (line, i) = key;
            let mid_s = |k: i32| (k as f32 + 0.5) * LINK_LENGTH;
            // В туннеле путь скрыт горой.
            if damage.in_tunnel(&terrain, line, mid_s(i)) {
                continue;
            }
            // Соседнее звено в туннеле — здесь вход: портал на конце звена.
            for (k, end, toward) in [(i + 1, b, b - a), (i - 1, a, a - b)] {
                if damage.in_tunnel(&terrain, line, mid_s(k)) {
                    let level = terrain.track_level(line, Lines::param(line, end));
                    portals.push(spawn_portal(&mut commands, &assets, end, toward, level));
                }
            }
            let broken = damage.broken.contains(&key);
            let entity = spawn_link(&mut commands, &assets, &terrain, key, a, b, broken);
            links.push(SpawnedLink { key, a, b, entity });
        }
        spawned.links.insert(coord, links);
        spawned.portals.insert(coord, portals);
    }
}

/// Портал туннеля: каменная арка с тёмным проёмом, утопленная в склон, и коллайдер.
/// `at` — вход на оси пути, `inward` — направление в туннель.
fn spawn_portal(commands: &mut Commands, assets: &RailAssets, at: Vec2, inward: Vec2, level: f32) -> Entity {
    let dir = inward.normalize_or(Vec2::X);
    let yaw = dir.x.atan2(dir.y);
    // Портал стоит чуть внутри склона, чтобы не висел в воздухе.
    let center = at + dir * (PORTAL_DEPTH * 0.5);
    let w = PORTAL_HALF_WIDTH;
    let pillar = 1.1;
    let parts = [
        // Пилоны.
        (assets.stone.clone(), Vec3::new(-w + pillar * 0.5, PORTAL_HEIGHT * 0.5, 0.0), Vec3::new(pillar, PORTAL_HEIGHT + 2.0, PORTAL_DEPTH)),
        (assets.stone.clone(), Vec3::new(w - pillar * 0.5, PORTAL_HEIGHT * 0.5, 0.0), Vec3::new(pillar, PORTAL_HEIGHT + 2.0, PORTAL_DEPTH)),
        // Перемычка и карниз.
        (assets.stone.clone(), Vec3::new(0.0, PORTAL_HEIGHT - 0.6, 0.0), Vec3::new(w * 2.0, 1.2, PORTAL_DEPTH)),
        (assets.stone.clone(), Vec3::new(0.0, PORTAL_HEIGHT + 0.15, -0.2), Vec3::new(w * 2.0 + 0.6, 0.4, PORTAL_DEPTH + 0.4)),
        // Тёмный проём.
        (assets.dark.clone(), Vec3::new(0.0, (PORTAL_HEIGHT - 1.2) * 0.5, 0.3), Vec3::new(w * 2.0 - pillar * 2.0, PORTAL_HEIGHT - 1.2, PORTAL_DEPTH)),
    ];
    commands
        .spawn((
            RunScoped,
            Transform::from_xyz(center.x, level - 0.5, center.y).with_rotation(Quat::from_rotation_y(yaw)),
            Visibility::default(),
            Obstacle {
                shape: Shape::Rect(Footprint {
                    center,
                    yaw,
                    half: Vec2::new(w, PORTAL_DEPTH * 0.5),
                }),
                top: level + PORTAL_HEIGHT,
            },
        ))
        .with_children(|portal| {
            for (material, position, size) in parts {
                portal.spawn((
                    Mesh3d(assets.cube.clone()),
                    MeshMaterial3d(material),
                    Transform::from_translation(position).with_scale(size),
                ));
            }
        })
        .id()
}

/// Вскрывает туннели, над которыми гора срыта ниже уровня путей плюс [`OPEN_DEPTH`]: опускает
/// полосу над путями до их уровня с откосами, бросает камни, рвёт звенья и пересоздаёт участок.
#[allow(clippy::too_many_arguments)]
fn open_tunnels(
    mut commands: Commands,
    mut changes: MessageReader<TerrainChanged>,
    mut terrain: ResMut<Terrain>,
    mut damage: ResMut<TrackDamage>,
    mut spawned: ResMut<SpawnedLinks>,
    visible: Res<VisibleChunks>,
    mut debris: MessageWriter<SpawnDebris>,
) {
    let areas: Vec<(Vec2, f32)> = changes.read().map(|c| (Vec2::new(c.center.x, c.center.z), c.radius)).collect();
    let mut dirty: HashSet<IVec2> = HashSet::new();
    for (center, radius) in areas {
        for near in terrain.lines().near(center, radius + TRACK_BAND) {
            let line = near.line;
            let first = ((near.s - radius) / TUNNEL_SEGMENT).floor() as i32;
            let last = ((near.s + radius) / TUNNEL_SEGMENT).ceil() as i32;
            for seg in first..=last {
                let s_mid = (seg as f32 + 0.5) * TUNNEL_SEGMENT;
                if !damage.in_tunnel(&terrain, line, s_mid) {
                    continue;
                }
                let axis = terrain.lines().point(line, s_mid);
                let level = terrain.track_level(line, s_mid);
                if terrain.height_at(axis.x, axis.y) >= level + OPEN_DEPTH {
                    continue;
                }
                damage.opened.insert((line, seg));
                // Полоса над путями опускается до уровня путей, дальше откос к нетронутому склону.
                let s0 = seg as f32 * TUNNEL_SEGMENT;
                let samples = 8;
                for k in 0..=samples * 2 {
                    let s = s0 + k as f32 * TUNNEL_SEGMENT / (samples * 2) as f32;
                    let p = terrain.lines().point(line, s);
                    let t = terrain.lines().tangent(line, s);
                    let side = Vec2::new(-t.y, t.x);
                    let level = terrain.track_level(line, s);
                    let reach = TRACK_BAND;
                    let mut d = -reach;
                    while d <= reach {
                        let q = p + side * d;
                        let (vx, vz) = (q.x.round() as i32, q.y.round() as i32);
                        let dist = d.abs();
                        let edge = ((dist - TRACK_FLAT) / (reach - TRACK_FLAT)).clamp(0.0, 1.0);
                        let limit = level + edge * edge * (reach - TRACK_FLAT) * 1.4;
                        for c in terrain.edit_vertex(vx, vz, |h, color| {
                            if *h > limit {
                                *h = limit;
                                color[0] = 0.32;
                                color[1] = 0.29;
                                color[2] = 0.26;
                            }
                        }) {
                            dirty.insert(c);
                        }
                        d += 0.7;
                    }
                }
                // Звенья вскрытого участка — завал.
                let li0 = (s0 / LINK_LENGTH).floor() as i32;
                for i in li0..li0 + (TUNNEL_SEGMENT / LINK_LENGTH) as i32 + 1 {
                    damage.break_link((line, i));
                }
                // Каменные обломки и пыль от обвала свода.
                let mid = terrain.lines().point(line, s_mid);
                let base = Vec3::new(mid.x, level + 1.0, mid.y);
                for k in 0..10u32 {
                    let a = k as f32 * 2.39996;
                    let r = 1.0 + (k % 3) as f32;
                    debris.write(SpawnDebris {
                        position: base + Vec3::new(a.cos() * r, (k % 4) as f32, a.sin() * r),
                        velocity: Vec3::new(a.cos() * 3.0, 3.0 + (k % 5) as f32, a.sin() * 3.0),
                        size: Vec3::splat(0.5 + (k % 4) as f32 * 0.25),
                        color: Color::srgb(0.46, 0.44, 0.41),
                        burning: false,
                    });
                }
            }
        }
    }
    for coord in dirty {
        for o in [IVec2::ZERO, IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y] {
            if let Some(e) = visible.entity(coord + o) {
                commands.entity(e).try_insert(ChunkDirty);
            }
        }
        spawned.refresh.insert(coord);
    }
}

/// Псевдослучайное число от 0 до 1 по звену и номеру: вид разрушения не меняется между загрузками.
fn link_hash(key: LinkKey, salt: u32) -> f32 {
    let mut h = (key.0.index as u32).wrapping_mul(0x9E37_79B9) ^ (key.1 as u32).wrapping_mul(0x85EB_CA6B) ^ salt.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// Брусок от точки `from` до точки `to` с сечением `section`.
fn beam(from: Vec3, to: Vec3, section: Vec2) -> Transform {
    let along = (to - from).normalize_or(Vec3::Z);
    Transform::from_translation((from + to) * 0.5)
        .with_rotation(Quat::from_rotation_arc(Vec3::Z, along))
        .with_scale(Vec3::new(section.x, section.y, from.distance(to) + 0.02))
}

/// Создаёт звено: целое лежит на поверхности, разрушенное — изломанными кусками по рельефу.
fn spawn_link(
    commands: &mut Commands,
    assets: &RailAssets,
    terrain: &Terrain,
    key: LinkKey,
    a: Vec2,
    b: Vec2,
    broken: bool,
) -> Entity {
    let ground = |p: Vec2| Vec3::new(p.x, terrain.height_at(p.x, p.y), p.y);
    let start = ground(a);
    let end = ground(b);
    if !broken {
        let along = (end - start).normalize_or(Vec3::Z);
        let rotation = Quat::from_rotation_arc(Vec3::Z, along);
        let length = start.distance(end);
        return commands
            .spawn((
                RunScoped,
                Transform::from_translation((start + end) * 0.5).with_rotation(rotation),
                Visibility::default(),
            ))
            .with_children(|link| {
                link.spawn((
                    Mesh3d(assets.cube.clone()),
                    MeshMaterial3d(assets.ballast.clone()),
                    NotShadowCaster,
                    Transform::from_xyz(0.0, -0.02, 0.0).with_scale(Vec3::new(3.0, 0.12, length + 0.05)),
                ));
                for i in 0..SLEEPERS {
                    let z = (i as f32 + 0.5) / SLEEPERS as f32 * length - length * 0.5;
                    link.spawn((
                        Mesh3d(assets.cube.clone()),
                        MeshMaterial3d(assets.sleeper.clone()),
                        NotShadowCaster,
                        Transform::from_xyz(0.0, 0.08, z).with_scale(Vec3::new(2.3, 0.1, 0.28)),
                    ));
                }
                for x in [-GAUGE_HALF, GAUGE_HALF] {
                    link.spawn((
                        Mesh3d(assets.cube.clone()),
                        MeshMaterial3d(assets.rail.clone()),
                        NotShadowCaster,
                        Transform::from_xyz(x, 0.2, 0.0).with_scale(Vec3::new(0.1, 0.14, length + 0.04)),
                    ));
                }
            })
            .id();
    }

    // Разрушенное звено: куски в мировых координатах под корнем без смещения.
    let dir = (b - a).normalize_or(Vec2::X);
    let side = Vec2::new(-dir.y, dir.x);
    let mut parts: Vec<(Handle<StandardMaterial>, Transform)> = Vec::new();
    for (r, x) in [-GAUGE_HALF, GAUGE_HALF].into_iter().enumerate() {
        // Рельс рвётся в точке излома и оба конца свисают к поверхности.
        let k = 0.3 + 0.4 * link_hash(key, r as u32);
        let kink = a.lerp(b, k) + side * (link_hash(key, 10 + r as u32) - 0.5) * 0.8;
        let pa = a + side * x;
        let pb = b + side * x;
        let lift = Vec3::Y * 0.15;
        let gap = dir * 0.25;
        let mid_a = ground(kink - gap) + lift + Vec3::Y * (link_hash(key, 20 + r as u32) * 0.6);
        let mid_b = ground(kink + gap) + lift;
        parts.push((assets.rail.clone(), beam(ground(pa) + lift, mid_a, Vec2::new(0.1, 0.14))));
        parts.push((assets.rail.clone(), beam(mid_b, ground(pb) + lift, Vec2::new(0.1, 0.14))));
    }
    for i in 0..SLEEPERS {
        // Часть шпал вырвана совсем.
        if link_hash(key, 40 + i as u32) < 0.3 {
            continue;
        }
        let t = (i as f32 + 0.5) / SLEEPERS as f32;
        let shift = side * (link_hash(key, 50 + i as u32) - 0.5) * 1.5 + dir * (link_hash(key, 60 + i as u32) - 0.5);
        let p = ground(a.lerp(b, t) + shift) + Vec3::Y * 0.06;
        let yaw = (link_hash(key, 70 + i as u32) - 0.5) * 1.6 + dir.x.atan2(dir.y);
        let tilt = (link_hash(key, 80 + i as u32) - 0.5) * 0.6;
        parts.push((
            assets.sleeper.clone(),
            Transform::from_translation(p)
                .with_rotation(Quat::from_rotation_y(yaw) * Quat::from_rotation_z(tilt))
                .with_scale(Vec3::new(2.3, 0.1, 0.28)),
        ));
    }
    commands
        .spawn((RunScoped, Transform::IDENTITY, Visibility::default()))
        .with_children(|link| {
            for (material, transform) in parts {
                link.spawn((Mesh3d(assets.cube.clone()), MeshMaterial3d(material), NotShadowCaster, transform));
            }
        })
        .id()
}

/// Рвёт звенья взрывами и просадкой рельефа и перекладывает звенья в изменённом месте.
fn break_links(
    mut commands: Commands,
    mut explosions: MessageReader<Explosion>,
    mut changes: MessageReader<TerrainChanged>,
    mut spawned: ResMut<SpawnedLinks>,
    mut damage: ResMut<TrackDamage>,
    terrain: Res<Terrain>,
    assets: Res<RailAssets>,
) {
    let blasts: Vec<(Vec2, f32)> = explosions
        .read()
        .filter(|e| e.power >= BREAK_POWER)
        .map(|e| (Vec2::new(e.position.x, e.position.z), e.radius))
        .collect();
    let changed: Vec<(Vec2, f32)> = changes
        .read()
        .map(|c| (Vec2::new(c.center.x, c.center.z), c.radius))
        .collect();
    if blasts.is_empty() && changed.is_empty() {
        return;
    }
    let near = |list: &[(Vec2, f32)], a: Vec2, b: Vec2| {
        list.iter().any(|(c, r)| {
            let ab = b - a;
            let t = ((*c - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            (a + ab * t).distance(*c) <= *r
        })
    };
    for links in spawned.links.values_mut() {
        for link in links.iter_mut() {
            let blasted = near(&blasts, link.a, link.b);
            let moved = near(&changed, link.a, link.b);
            if !blasted && !moved {
                continue;
            }
            let (line, _) = link.key;
            let sagged = [link.a, link.b].iter().any(|p| {
                let s = Lines::param(line, *p);
                terrain.height_at(p.x, p.y) < terrain.track_level(line, s) - BREAK_SAG
            });
            if blasted || sagged {
                damage.break_link(link.key);
            }
            // Звено перекладывается на новый рельеф: целое — ровно, разрушенное — свисает в кратер.
            commands.entity(link.entity).try_despawn();
            let broken = damage.broken.contains(&link.key);
            link.entity = spawn_link(&mut commands, &assets, &terrain, link.key, link.a, link.b, broken);
        }
    }
}
