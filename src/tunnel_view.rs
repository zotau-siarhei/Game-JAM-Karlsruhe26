//! Окно в туннель: облицовка вскрытых участков.
//!
//! Вскрытие и данные — в `rails.rs` ([`TrackDamage`], [`TunnelOpened`]). Здесь на каждом вскрытом
//! участке, который сейчас в сцене, стоят две стены из бетонных блоков с рваным верхом, обломки
//! рёбер свода, лампы со свечением и кабель. Где окно упирается в невскрытый туннель, стоит срез
//! трубы: арка с обломанной перемычкой, тёмный проём и коллайдер. Вид задаётся хешем участка,
//! поэтому после отъезда и возвращения он тот же: хранится только список вскрытых участков.

use std::collections::HashMap;

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::frame::GameSet;
use crate::obstacles::{Obstacle, Shape};
use crate::rails::{GAUGE, LinkKey, TUNNEL_SEGMENT, TrackDamage, TunnelOpened, link_hash};
use crate::railway::LineId;
use crate::run::{RunScoped, WorldRebuilt};
use crate::settlements::Footprint;
use crate::terrain::{self, Terrain, VisibleChunks, chunk_coord_at};

/// Ось стены от оси пути, м.
const WALL_OFFSET: f32 = GAUGE * 0.5 + 2.4;
/// Толщина стены, м.
const WALL_THICKNESS: f32 = 0.6;
/// Длина бетонного блока вдоль пути, м.
const BLOCK_LENGTH: f32 = 2.0;
/// Высота блока над уровнем путей: от и до, м. Верх рваный.
const WALL_HEIGHT: (f32, f32) = (3.5, 6.0);
/// Блок уходит под уровень путей на столько, чтобы не висел над откосом, м.
const WALL_FOOT: f32 = 1.0;
/// Шаг обломков рёбер свода и ламп вдоль пути, м.
const RIB_STEP: f32 = 4.0;
const LAMP_STEP: f32 = 6.0;
/// Высота ламп и кабеля над уровнем путей, м.
const LAMP_HEIGHT: f32 = 2.6;
const CABLE_HEIGHT: f32 = 3.2;
/// Срез трубы: высота арки и глубина тёмного проёма, м.
const ARCH_HEIGHT: f32 = 6.0;
const OPENING_DEPTH: f32 = 0.5;

/// Подключает облицовку окон в туннель.
pub struct TunnelViewPlugin;

impl Plugin for TunnelViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Linings>()
            .add_systems(Startup, create_lining_assets)
            .add_systems(
                Update,
                (
                    forget_linings.in_set(GameSet::Reset),
                    stream_linings.after(terrain::stream_chunks).in_set(GameSet::Streaming),
                ),
            );
    }
}

/// Облицовка участков, которые сейчас в сцене: корень на участок.
#[derive(Resource, Default)]
struct Linings {
    built: HashMap<LinkKey, Entity>,
}

#[derive(Resource)]
struct LiningAssets {
    cube: Handle<Mesh>,
    concrete: Handle<StandardMaterial>,
    rib: Handle<StandardMaterial>,
    lamp: Handle<StandardMaterial>,
    cable: Handle<StandardMaterial>,
    dark: Handle<StandardMaterial>,
}

fn create_lining_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(LiningAssets {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        concrete: materials.add(StandardMaterial {
            base_color: Color::srgb(0.56, 0.55, 0.52),
            perceptual_roughness: 0.95,
            ..default()
        }),
        rib: materials.add(StandardMaterial {
            base_color: Color::srgb(0.42, 0.33, 0.27),
            metallic: 0.5,
            perceptual_roughness: 0.7,
            ..default()
        }),
        // Эмиссия больше единицы: лампа светится через свечение камеры.
        lamp: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.85, 0.55),
            emissive: LinearRgba::rgb(9.0, 6.5, 2.8),
            ..default()
        }),
        cable: materials.add(Color::srgb(0.08, 0.08, 0.09)),
        dark: materials.add(StandardMaterial {
            base_color: Color::srgb(0.02, 0.02, 0.025),
            unlit: true,
            ..default()
        }),
    });
}

/// Новый мир: облицовка прошлого забега удалена через `RunScoped`.
fn forget_linings(mut rebuilt: MessageReader<WorldRebuilt>, mut linings: ResMut<Linings>) {
    if rebuilt.read().count() > 0 {
        linings.built.clear();
    }
}

/// В сцене ли участок туннеля: по участку рельефа под его серединой.
fn segment_visible(terrain: &Terrain, visible: &VisibleChunks, line: LineId, segment: i32) -> bool {
    let mid = terrain.lines().point(line, (segment as f32 + 0.5) * TUNNEL_SEGMENT);
    visible.entity(chunk_coord_at(mid.x, mid.y)).is_some()
}

/// Строит облицовку вскрытых участков в сцене и убирает выгруженные. Когда вскрывается сосед,
/// участок пересобирается: срез на общей границе больше не нужен, стены идут дальше.
fn stream_linings(
    mut commands: Commands,
    mut linings: ResMut<Linings>,
    mut opened: MessageReader<TunnelOpened>,
    damage: Res<TrackDamage>,
    terrain: Res<Terrain>,
    visible: Res<VisibleChunks>,
    assets: Res<LiningAssets>,
) {
    let fresh: Vec<TunnelOpened> = opened.read().copied().collect();
    if fresh.is_empty() && !visible.is_changed() {
        return;
    }
    for open in fresh {
        for segment in [open.segment - 1, open.segment + 1] {
            if let Some(root) = linings.built.remove(&(open.line, segment)) {
                commands.entity(root).try_despawn();
            }
        }
    }
    linings.built.retain(|&(line, segment), root| {
        let keep = segment_visible(&terrain, &visible, line, segment);
        if !keep {
            commands.entity(*root).try_despawn();
        }
        keep
    });
    for (line, segment) in damage.opened() {
        if linings.built.contains_key(&(line, segment)) || !segment_visible(&terrain, &visible, line, segment) {
            continue;
        }
        let root = spawn_lining(&mut commands, &assets, &terrain, &damage, line, segment);
        linings.built.insert((line, segment), root);
    }
}

/// Поворот вокруг вертикали, при котором локальная ось Z смотрит вдоль `dir` в плане.
fn yaw_of(dir: Vec2) -> f32 {
    dir.x.atan2(dir.y)
}

/// Брусок от `from` до `to` с сечением `section`.
fn beam(from: Vec3, to: Vec3, section: Vec2) -> Transform {
    let along = (to - from).normalize_or(Vec3::Z);
    Transform::from_translation((from + to) * 0.5)
        .with_rotation(Quat::from_rotation_arc(Vec3::Z, along))
        .with_scale(Vec3::new(section.x, section.y, from.distance(to)))
}

/// Высота блока номер `block` над уровнем путей: рваный верх по хешу.
fn block_height(line: LineId, block: i32, side: f32) -> f32 {
    let salt = if side > 0.0 { 1 } else { 2 };
    WALL_HEIGHT.0 + link_hash((line, block), salt) * (WALL_HEIGHT.1 - WALL_HEIGHT.0)
}

/// Параметры пути на участке, где стоят точки с шагом `step`: общая сетка по линии, поэтому на
/// стыке соседних участков шаг не сбивается.
fn grid_points(segment: i32, step: f32) -> impl Iterator<Item = f32> {
    let s0 = segment as f32 * TUNNEL_SEGMENT;
    let first = (s0 / step).ceil() as i32;
    let last = ((s0 + TUNNEL_SEGMENT) / step).ceil() as i32;
    (first..last).map(move |i| i as f32 * step)
}

/// Облицовка одного вскрытого участка: стены, рёбра, лампы, кабель и срезы трубы на концах.
fn spawn_lining(
    commands: &mut Commands,
    assets: &LiningAssets,
    terrain: &Terrain,
    damage: &TrackDamage,
    line: LineId,
    segment: i32,
) -> Entity {
    let lines = terrain.lines();
    // Точка на оси, касательная, нормаль влево и уровень путей.
    let frame = |s: f32| {
        let t = lines.tangent(line, s);
        (lines.point(line, s), t, Vec2::new(-t.y, t.x), terrain.track_level(line, s))
    };
    let world = |p: Vec2, y: f32| Vec3::new(p.x, y, p.y);
    let mut parts: Vec<(Handle<StandardMaterial>, Transform)> = Vec::new();
    let mut colliders: Vec<Obstacle> = Vec::new();
    let s0 = segment as f32 * TUNNEL_SEGMENT;

    // Стены из блоков.
    for k in 0..(TUNNEL_SEGMENT / BLOCK_LENGTH) as i32 {
        let s = s0 + (k as f32 + 0.5) * BLOCK_LENGTH;
        let block = (s / BLOCK_LENGTH).floor() as i32;
        let (p, t, n, level) = frame(s);
        for side in [1.0, -1.0] {
            let c = p + n * side * WALL_OFFSET;
            let h = block_height(line, block, side);
            let rotation = Quat::from_rotation_y(yaw_of(t));
            parts.push((
                assets.concrete.clone(),
                Transform::from_translation(world(c, level - WALL_FOOT + (h + WALL_FOOT) * 0.5))
                    .with_rotation(rotation)
                    .with_scale(Vec3::new(WALL_THICKNESS, h + WALL_FOOT, BLOCK_LENGTH + 0.04)),
            ));
            // Обломок на верху блока: верх ещё рванее.
            if link_hash((line, block), 3 + side as u32) > 0.5 {
                let shift = (link_hash((line, block), 5) - 0.5) * BLOCK_LENGTH * 0.6;
                parts.push((
                    assets.concrete.clone(),
                    Transform::from_translation(world(c + t * shift, level + h + 0.25))
                        .with_rotation(rotation * Quat::from_rotation_z(0.25 * side))
                        .with_scale(Vec3::new(WALL_THICKNESS * 0.9, 0.6, 0.7)),
                ));
            }
        }
    }

    // Обломки рёбер свода: короткие балки с верха стен внутрь и вверх. Часть рёбер выбита.
    for s in grid_points(segment, RIB_STEP) {
        let rib = (s / RIB_STEP).round() as i32;
        let block = (s / BLOCK_LENGTH).floor() as i32;
        let (p, _, n, level) = frame(s);
        for side in [1.0, -1.0] {
            if link_hash((line, rib), 7 + side as u32) < 0.3 {
                continue;
            }
            let top = level + block_height(line, block, side).min(block_height(line, block - 1, side));
            let foot = world(p + n * side * WALL_OFFSET, top - 0.2);
            let reach = 1.0 + link_hash((line, rib), 11) * 1.2;
            let tip = world(p + n * side * (WALL_OFFSET - reach), top + 0.4 + reach * 0.35);
            parts.push((assets.rib.clone(), beam(foot, tip, Vec2::new(0.35, 0.35))));
        }
    }

    // Лампы на внутренней стороне стен, по очереди слева и справа.
    for s in grid_points(segment, LAMP_STEP) {
        let side = if (s / LAMP_STEP).round() as i32 % 2 == 0 { 1.0 } else { -1.0 };
        let (p, t, n, level) = frame(s);
        let c = p + n * side * (WALL_OFFSET - WALL_THICKNESS * 0.5 - 0.1);
        parts.push((
            assets.lamp.clone(),
            Transform::from_translation(world(c, level + LAMP_HEIGHT))
                .with_rotation(Quat::from_rotation_y(yaw_of(t)))
                .with_scale(Vec3::new(0.2, 0.22, 0.45)),
        ));
    }

    // Кабель вдоль обеих стен и коллайдеры стен.
    let (pa, _, na, la) = frame(s0);
    let (pb, _, nb, lb) = frame(s0 + TUNNEL_SEGMENT);
    let (pm, tm, nm, lm) = frame(s0 + TUNNEL_SEGMENT * 0.5);
    for side in [1.0, -1.0] {
        let inner = WALL_OFFSET - WALL_THICKNESS * 0.5 - 0.04;
        parts.push((
            assets.cable.clone(),
            beam(
                world(pa + na * side * inner, la + CABLE_HEIGHT),
                world(pb + nb * side * inner, lb + CABLE_HEIGHT),
                Vec2::splat(0.07),
            ),
        ));
        colliders.push(Obstacle {
            shape: Shape::Rect(Footprint {
                center: pm + nm * side * WALL_OFFSET,
                yaw: yaw_of(tm),
                half: Vec2::new(WALL_THICKNESS * 0.5, TUNNEL_SEGMENT * 0.5),
            }),
            top: lm + WALL_HEIGHT.0,
        });
    }

    // Срезы трубы там, где за границей участка туннель не вскрыт.
    let ends = [(segment - 1, s0, -1.0), (segment + 1, s0 + TUNNEL_SEGMENT, 1.0)];
    for (neighbor, boundary, sign) in ends {
        let neighbor_mid = (neighbor as f32 + 0.5) * TUNNEL_SEGMENT;
        if !damage.in_tunnel(terrain, line, neighbor_mid) {
            continue;
        }
        let (p, t, n, level) = frame(boundary);
        let inward = t * sign;
        let rotation = Quat::from_rotation_y(yaw_of(inward));
        let key = (line, (boundary / TUNNEL_SEGMENT).round() as i32);
        // Пилоны арки продолжают стены.
        for side in [1.0, -1.0] {
            let c = p + n * side * WALL_OFFSET + inward * 0.3;
            parts.push((
                assets.concrete.clone(),
                Transform::from_translation(world(c, level - WALL_FOOT + (ARCH_HEIGHT + WALL_FOOT) * 0.5))
                    .with_rotation(rotation)
                    .with_scale(Vec3::new(0.9, ARCH_HEIGHT + WALL_FOOT, 0.8)),
            ));
            // Обломанная перемычка: с каждой стороны кусок разной длины, посередине пролом.
            let reach = WALL_OFFSET * (0.35 + 0.5 * link_hash(key, 13 + side as u32));
            let from = p + n * side * WALL_OFFSET + inward * 0.3;
            let to = p + n * side * (WALL_OFFSET - reach) + inward * 0.3;
            parts.push((
                assets.concrete.clone(),
                beam(world(from, level + ARCH_HEIGHT - 0.5), world(to, level + ARCH_HEIGHT - 0.6 - reach * 0.1), Vec2::new(0.8, 1.0)),
            ));
        }
        // Тёмный проём: продолжение туннеля в гору.
        let opening = p + inward * (0.3 + OPENING_DEPTH * 0.5);
        parts.push((
            assets.dark.clone(),
            Transform::from_translation(world(opening, level + (ARCH_HEIGHT - 0.8) * 0.5 - 0.2))
                .with_rotation(rotation)
                .with_scale(Vec3::new(WALL_OFFSET * 2.0 - 0.8, ARCH_HEIGHT - 0.6, OPENING_DEPTH)),
        ));
        colliders.push(Obstacle {
            shape: Shape::Rect(Footprint {
                center: p + inward * 0.4,
                yaw: yaw_of(inward),
                half: Vec2::new(WALL_OFFSET + 0.45, 0.4),
            }),
            top: level + ARCH_HEIGHT,
        });
    }

    commands
        .spawn((RunScoped, Transform::IDENTITY, Visibility::default()))
        .with_children(|root| {
            for (material, transform) in parts {
                root.spawn((Mesh3d(assets.cube.clone()), MeshMaterial3d(material), NotShadowCaster, transform));
            }
            for obstacle in colliders {
                root.spawn((obstacle, Transform::IDENTITY));
            }
        })
        .id()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn лампы_и_рёбра_идут_ровным_шагом_через_стыки_участков() {
        for step in [LAMP_STEP, RIB_STEP] {
            let points: Vec<f32> = (-3..4).flat_map(|segment| grid_points(segment, step)).collect();
            for pair in points.windows(2) {
                assert!((pair[1] - pair[0] - step).abs() < 1e-4, "{step}: {pair:?}");
            }
            // Каждая точка — на своём участке.
            for segment in -3..4 {
                let s0 = segment as f32 * TUNNEL_SEGMENT;
                assert!(grid_points(segment, step).all(|s| s >= s0 && s < s0 + TUNNEL_SEGMENT));
            }
        }
    }

    #[test]
    fn верх_стены_рваный_но_в_пределах() {
        let line = LineId {
            family: crate::railway::Family::AlongX,
            index: 3,
        };
        let heights: Vec<f32> = (0..40).map(|b| block_height(line, b, 1.0)).collect();
        assert!(heights.iter().all(|h| (WALL_HEIGHT.0..=WALL_HEIGHT.1).contains(h)));
        let spread = heights.iter().cloned().fold(f32::MIN, f32::max) - heights.iter().cloned().fold(f32::MAX, f32::min);
        assert!(spread > 1.0, "{spread}");
    }
}
