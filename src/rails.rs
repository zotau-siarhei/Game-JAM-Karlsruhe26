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
//! над путями опускается до их уровня с откосами, летят камни, встаёт облако пыли, звенья под
//! ним — завал. Туннель — только под горой длиной не меньше 30 м, короткий гребень прорезан выемкой.

use std::collections::{HashMap, HashSet};

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy_hanabi::prelude::*;

use crate::combat::Team;
use crate::craters;
use crate::explosion::Explosion;
use crate::debris::SpawnDebris;
use crate::frame::GameSet;
use crate::obstacles::{Obstacle, Shape};
use crate::railway::{EMBANKMENT_HEIGHT, Family, LineId, Lines, TRACK_BAND, TRACK_FLAT, embankment};
use crate::settlements::Footprint;
use crate::run::{RunScoped, WorldRebuilt};
use crate::subsidence::TerrainChanged;
use crate::terrain::{self, CHUNK_SIZE, ChunkDirty, ChunkMesh, Terrain, VisibleChunks, chunk_coord_at, chunk_origin};

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

/// Насколько портал заходит в склон за границей туннеля, м: проём вплотную к стене горы.
const PORTAL_INSET: f32 = 0.3;

/// Целое звено ложится на рельеф, пока тот не выше уровня путей на столько; выше — склон горы
/// у входа в туннель, и звено идёт на уровне путей, а не взбирается на обрыв, м.
const RAIL_CLAMP: f32 = 2.0;

/// Через сколько метров вдоль звена проверяется просадка: траншея поперёк звена не проскочит
/// между точками, м.
const SAG_STEP: f32 = 1.0;

/// Сколько участков звеньев создаётся за кадр: остальные — в следующих кадрах, без рывка на
/// границе участка.
const CHUNKS_PER_FRAME: usize = 2;

/// Клубов пыли при обвале свода.
const CAVE_IN_DUST: f32 = 120.0;

/// Сколько живёт облако пыли обвала, с: дольше самого долгого клуба.
const CAVE_IN_DUST_TIME: f32 = 4.5;

/// Подключает звенья путей.
pub struct RailsPlugin;

impl Plugin for RailsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpawnedLinks>()
            .init_resource::<TrackDamage>()
            .add_systems(Startup, create_rail_assets)
            .add_systems(
                Update,
                (
                    forget_links.in_set(GameSet::Reset),
                    // После кратеров этого кадра: пути рвутся и туннели вскрываются по новому рельефу.
                    (open_tunnels, break_links).chain().after(craters::dig_craters).in_set(GameSet::Damage),
                    stream_links.after(terrain::stream_chunks).in_set(GameSet::Streaming),
                    settle_dust.in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Адрес звена: линия и номер звена вдоль неё.
pub type LinkKey = (LineId, i32);

/// Разрушенные звенья забега.
#[derive(Resource, Default)]
pub struct TrackDamage {
    /// Разрушенные звенья и чья атака их порвала: от этого зависит, на кого записать крушение.
    broken: HashMap<LinkKey, Team>,
    /// Вскрытые участки туннелей: линия и номер участка длиной [`TUNNEL_SEGMENT`].
    opened: HashSet<LinkKey>,
}

impl TrackDamage {
    /// Проходит ли путь на параметре `s` в невскрытом туннеле.
    pub fn in_tunnel(&self, terrain: &Terrain, line: LineId, s: f32) -> bool {
        terrain.is_tunnel(line, s) && !self.opened.contains(&(line, (s / TUNNEL_SEGMENT).floor() as i32))
    }

    /// Разрушено ли звено, на которое приходится параметр пути `s`.
    pub fn is_broken(&self, line: LineId, s: f32) -> bool {
        self.broken.contains_key(&link_at(line, s))
    }

    /// Кто порвал звено на параметре `s`, если оно разрушено.
    pub fn broken_by(&self, line: LineId, s: f32) -> Option<Team> {
        self.broken.get(&link_at(line, s)).copied()
    }

    /// Отмечает звено разрушенным без виновника (лава, обвал); `true`, если раньше оно было целым.
    pub fn break_link(&mut self, key: LinkKey) -> bool {
        self.break_link_by(key, Team::Neutral)
    }

    /// Отмечает звено разрушенным атакой команды `team`. Виновник — тот, кто порвал первым.
    pub fn break_link_by(&mut self, key: LinkKey, team: Team) -> bool {
        if self.broken.contains_key(&key) {
            return false;
        }
        self.broken.insert(key, team);
        true
    }

    /// Граница невскрытого туннеля между параметрами `outside` (вне туннеля) и `inside` (в нём).
    pub fn tunnel_entrance(&self, terrain: &Terrain, line: LineId, outside: f32, inside: f32) -> f32 {
        let (mut out, mut ins) = (outside, inside);
        for _ in 0..10 {
            let mid = 0.5 * (out + ins);
            if self.in_tunnel(terrain, line, mid) {
                ins = mid;
            } else {
                out = mid;
            }
        }
        0.5 * (out + ins)
    }
}

/// Звено, на которое приходится параметр пути `s`.
fn link_at(line: LineId, s: f32) -> LinkKey {
    (line, (s / LINK_LENGTH).floor() as i32)
}

/// Высота, на которой лежит звено в точке `p`: на рельефе, а у входа в туннель, где рельеф —
/// склон горы, — на уровне путей.
fn rail_height(terrain: &Terrain, line: LineId, p: Vec2) -> f32 {
    let ground = terrain.height_at(p.x, p.y);
    let level = terrain.track_level(line, Lines::param(line, p));
    if ground > level + RAIL_CLAMP { level } else { ground }
}

/// Точки звена от `a` до `b` через [`SAG_STEP`], включая концы.
fn link_samples(a: Vec2, b: Vec2) -> impl Iterator<Item = Vec2> {
    let n = (a.distance(b) / SAG_STEP).ceil().max(1.0) as usize;
    (0..=n).map(move |k| a.lerp(b, k as f32 / n as f32))
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
    /// Не все видимые участки получили звенья: предел [`CHUNKS_PER_FRAME`], продолжить в следующем кадре.
    pending: bool,
}

#[derive(Resource)]
struct RailAssets {
    cube: Handle<Mesh>,
    rail: Handle<StandardMaterial>,
    sleeper: Handle<StandardMaterial>,
    ballast: Handle<StandardMaterial>,
    stone: Handle<StandardMaterial>,
    dark: Handle<StandardMaterial>,
    /// Пыль обвала свода.
    dust: Handle<EffectAsset>,
}

/// Облако пыли обвала: удаляется, когда осядет.
#[derive(Component)]
struct CaveInDust {
    left: f32,
}

fn create_rail_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut effects: ResMut<Assets<EffectAsset>>,
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
        dust: effects.add(cave_in_dust_effect()),
    });
}

/// Пыль обвала свода: светлые клубы встают над выемкой, расходятся и сносятся ветром.
fn cave_in_dust_effect() -> EffectAsset {
    let writer = ExprWriter::new();
    let init_pos = SetPositionSphereModifier {
        center: writer.lit(Vec3::Y).expr(),
        radius: writer.lit(TUNNEL_SEGMENT * 0.6).expr(),
        dimension: ShapeDimension::Volume,
    };
    let init_vel = SetVelocitySphereModifier {
        center: writer.lit(Vec3::Y * -3.0).expr(),
        speed: writer.lit(2.0).uniform(writer.lit(7.0)).expr(),
    };
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, writer.lit(2.0).uniform(writer.lit(4.0)).expr());
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    // Ветер в ту же сторону, что у дыма взрывов.
    let update_wind = AccelModifier::new(writer.lit(Vec3::new(0.8, 0.6, -0.4)).expr());
    let update_drag = LinearDragModifier::new(writer.lit(1.5).expr());
    let round = RoundModifier {
        roundness: writer.lit(1.0).expr(),
    };

    let mut color = bevy_hanabi::Gradient::new();
    color.add_key(0.0, Vec4::new(0.62, 0.58, 0.52, 0.0));
    color.add_key(0.1, Vec4::new(0.62, 0.58, 0.52, 0.6));
    color.add_key(0.6, Vec4::new(0.56, 0.53, 0.48, 0.35));
    color.add_key(1.0, Vec4::new(0.5, 0.48, 0.45, 0.0));
    let mut size = bevy_hanabi::Gradient::new();
    size.add_key(0.0, Vec3::splat(1.5));
    size.add_key(1.0, Vec3::splat(6.0));

    EffectAsset::new(256, SpawnerSettings::once(CAVE_IN_DUST.into()), writer.finish())
        .with_name("tunnel_cave_in_dust")
        .init(init_pos)
        .init(init_vel)
        .init(init_lifetime)
        .init(init_age)
        .update(update_wind)
        .update(update_drag)
        .render(OrientModifier::new(OrientMode::FaceCameraPosition))
        .render(round)
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Overwrite,
            mask: ColorBlendMask::RGBA,
        })
        .render(SizeOverLifetimeModifier {
            gradient: size,
            screen_space_size: false,
        })
}

/// Удаляет осевшие облака пыли обвала.
fn settle_dust(mut commands: Commands, time: Res<Time>, mut clouds: Query<(Entity, &mut CaveInDust)>) {
    for (entity, mut cloud) in &mut clouds {
        cloud.left -= time.delta_secs();
        if cloud.left <= 0.0 {
            commands.entity(entity).despawn();
        }
    }
}

fn forget_links(
    mut started: MessageReader<WorldRebuilt>,
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
    if !visible.is_changed() && spawned.refresh.is_empty() && !spawned.pending {
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
    spawned.pending = false;
    let mut budget = CHUNKS_PER_FRAME;
    for chunk in &chunks {
        let coord = chunk.0;
        if spawned.links.contains_key(&coord) || visible.entity(coord).is_none() {
            continue;
        }
        // Звенья участка — сотня сущностей и больше: на границе участка создаются за несколько кадров.
        if budget == 0 {
            spawned.pending = true;
            break;
        }
        budget -= 1;
        let mut links = Vec::new();
        let mut portals = Vec::new();
        for (key, a, b) in links_in_chunk(terrain.lines(), coord) {
            let (line, i) = key;
            let mid_s = |k: i32| (k as f32 + 0.5) * LINK_LENGTH;
            // В туннеле путь скрыт горой.
            if damage.in_tunnel(&terrain, line, mid_s(i)) {
                continue;
            }
            // Соседнее звено в туннеле — рядом вход: портал на границе туннеля, а не на конце
            // звена, иначе он висит перед склоном или тонет в горе.
            for (k, toward) in [(i + 1, b - a), (i - 1, a - b)] {
                if damage.in_tunnel(&terrain, line, mid_s(k)) {
                    let s = damage.tunnel_entrance(&terrain, line, mid_s(i), mid_s(k));
                    let at = terrain.lines().point(line, s);
                    let level = terrain.track_level(line, s);
                    portals.push(spawn_portal(&mut commands, &assets, at, toward, level));
                }
            }
            let broken = damage.broken.contains_key(&key);
            let entity = spawn_link(&mut commands, &assets, &terrain, key, a, b, broken);
            links.push(SpawnedLink { key, a, b, entity });
        }
        spawned.links.insert(coord, links);
        spawned.portals.insert(coord, portals);
    }
}

/// Портал туннеля: каменная арка с тёмным проёмом, вплотную к склону, и коллайдер.
/// `at` — граница туннеля на оси пути, `inward` — направление в туннель.
fn spawn_portal(commands: &mut Commands, assets: &RailAssets, at: Vec2, inward: Vec2, level: f32) -> Entity {
    let dir = inward.normalize_or(Vec2::X);
    let yaw = dir.x.atan2(dir.y);
    // За границей туннеля рельеф — стена горы: портал стоит перед ней и чуть заходит в склон,
    // чтобы не висел в воздухе и не утонул в горе.
    let center = at - dir * (PORTAL_DEPTH * 0.5 - PORTAL_INSET);
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
///
/// Об изменении рельефа сообщает [`TerrainChanged`] через команды: эта система сама читает
/// `TerrainChanged`, поэтому писать его напрямую не может. Сообщение приходит в конце набора,
/// его читатели видят его в следующем кадре; повторная проверка вскрытого участка ничего не делает.
#[allow(clippy::too_many_arguments)]
fn open_tunnels(
    mut commands: Commands,
    mut changes: MessageReader<TerrainChanged>,
    mut explosions: MessageReader<Explosion>,
    mut terrain: ResMut<Terrain>,
    mut damage: ResMut<TrackDamage>,
    mut spawned: ResMut<SpawnedLinks>,
    visible: Res<VisibleChunks>,
    assets: Res<RailAssets>,
    mut debris: MessageWriter<SpawnDebris>,
) {
    let areas: Vec<(Vec2, f32)> = changes.read().map(|c| (Vec2::new(c.center.x, c.center.z), c.radius)).collect();
    let blasts: Vec<(Vec2, f32, Team)> =
        explosions.read().map(|e| (Vec2::new(e.position.x, e.position.z), e.radius, e.source_team)).collect();
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
                // Полоса над путями опускается до уровня путей с насыпью, дальше откос к
                // нетронутому склону.
                let s0 = seg as f32 * TUNNEL_SEGMENT;
                let samples = 8;
                for k in 0..=samples * 2 {
                    let s = s0 + k as f32 * TUNNEL_SEGMENT / (samples * 2) as f32;
                    let p = terrain.lines().point(line, s);
                    let t = terrain.lines().tangent(line, s);
                    let side = Vec2::new(-t.y, t.x);
                    let ground = terrain.track_level(line, s) - EMBANKMENT_HEIGHT;
                    let reach = TRACK_BAND;
                    let mut d = -reach;
                    while d <= reach {
                        let q = p + side * d;
                        let (vx, vz) = (q.x.round() as i32, q.y.round() as i32);
                        let dist = d.abs();
                        let edge = ((dist - TRACK_FLAT) / (reach - TRACK_FLAT)).clamp(0.0, 1.0);
                        let limit = ground + embankment(dist) + edge * edge * (reach - TRACK_FLAT) * 1.4;
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
                let mid = terrain.lines().point(line, s_mid);
                commands.write_message(TerrainChanged {
                    center: Vec3::new(mid.x, level, mid.y),
                    radius: TUNNEL_SEGMENT * 0.5 + TRACK_BAND,
                });
                // Звенья вскрытого участка — завал. Виновник — ближайший взрыв этого кадра рядом.
                let culprit = blasts
                    .iter()
                    .filter(|&&(c, r, _)| c.distance(mid) <= r * 1.6 + TUNNEL_SEGMENT)
                    .min_by(|x, y| x.0.distance(mid).total_cmp(&y.0.distance(mid)))
                    .map_or(Team::Neutral, |&(_, _, team)| team);
                let li0 = (s0 / LINK_LENGTH).floor() as i32;
                for i in li0..li0 + (TUNNEL_SEGMENT / LINK_LENGTH) as i32 + 1 {
                    damage.break_link_by((line, i), culprit);
                }
                // Каменные обломки и пыль от обвала свода.
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
                commands.spawn((
                    RunScoped,
                    CaveInDust { left: CAVE_IN_DUST_TIME },
                    ParticleEffect::new(assets.dust.clone()),
                    Transform::from_translation(base),
                ));
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
    let ground = |p: Vec2| Vec3::new(p.x, rail_height(terrain, key.0, p), p.y);
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
    // Все взрывы кадра: сильные рвут звено сами, любой — виновник просадки рядом с собой.
    let all_blasts: Vec<(Vec2, f32, f32, Team)> = explosions
        .read()
        .map(|e| (Vec2::new(e.position.x, e.position.z), e.radius, e.power, e.source_team))
        .collect();
    let blasts: Vec<(Vec2, f32)> = all_blasts
        .iter()
        .filter(|(_, _, power, _)| *power >= BREAK_POWER)
        .map(|&(c, r, _, _)| (c, r))
        .collect();
    let changed: Vec<(Vec2, f32)> = changes
        .read()
        .map(|c| (Vec2::new(c.center.x, c.center.z), c.radius))
        .collect();
    if blasts.is_empty() && changed.is_empty() {
        return;
    }
    let reach = |c: Vec2, a: Vec2, b: Vec2| {
        let ab = b - a;
        let t = ((c - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
        (a + ab * t).distance(c)
    };
    let near = |list: &[(Vec2, f32)], a: Vec2, b: Vec2| list.iter().any(|&(c, r)| reach(c, a, b) <= r);
    // Чей взрыв задел звено; кратер шире радиуса взрыва, поэтому с запасом. Траншея и прочее — ничьё.
    let culprit = |a: Vec2, b: Vec2| {
        all_blasts
            .iter()
            .filter(|&&(c, r, _, _)| reach(c, a, b) <= r * 1.6)
            .min_by(|x, y| reach(x.0, a, b).total_cmp(&reach(y.0, a, b)))
            .map_or(Team::Neutral, |&(_, _, _, team)| team)
    };
    for links in spawned.links.values_mut() {
        for link in links.iter_mut() {
            let blasted = near(&blasts, link.a, link.b);
            let moved = near(&changed, link.a, link.b);
            if !blasted && !moved {
                continue;
            }
            let (line, _) = link.key;
            // Просадка проверяется по всей длине звена: траншея лазера между концами тоже рвёт путь.
            let sagged = link_samples(link.a, link.b).any(|p| {
                let s = Lines::param(line, p);
                terrain.height_at(p.x, p.y) < terrain.track_level(line, s) - BREAK_SAG
            });
            if blasted || sagged {
                damage.break_link_by(link.key, culprit(link.a, link.b));
            }
            // Звено перекладывается на новый рельеф: целое — ровно, разрушенное — свисает в кратер.
            commands.entity(link.entity).try_despawn();
            let broken = damage.broken.contains_key(&link.key);
            link.entity = spawn_link(&mut commands, &assets, &terrain, link.key, link.a, link.b, broken);
        }
    }
}
