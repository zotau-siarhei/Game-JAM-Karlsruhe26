//! Мини-карта: круг в углу экрана с луноходом в центре и метками того, что за кадром.
//!
//! Камера не вращается, поэтому верх мини-карты всегда совпадает с верхом экрана.
//! Метки берутся из заранее созданного набора узлов: во время забега узлы не создаются.
//!
//! Под метками — местность: текстура 128×128 из цветов вершин рельефа, поверх пути, дома
//! и рамка кадра. Пересчитывается раз в полсекунды, поэтому новые кратеры видны почти сразу.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::airstrike::Strike;
use crate::boss::Boss;
use crate::buildings::House;
use crate::capsule::Capsule;
use crate::camera::GameCamera;
use crate::enemy::Enemy;
use crate::frame::GameSet;
use crate::hud::RunUi;
use crate::mortar::FireShell;
use crate::pickups::{GoldenCrate, Pickup};
use crate::repair::Medkit;
use crate::rover::Rover;
use crate::run::{GameState, WorldRebuilt};
use crate::settlements::Settlements;
use crate::terrain::Terrain;
use crate::trains::Car;

/// Радиус мини-карты на экране, пиксели.
const RADIUS: f32 = 100.0;

/// Расстояние от лунохода до края мини-карты, м: охват 160 м.
const RANGE: f32 = 80.0;

/// Сторона текстуры местности, пиксели.
const TEXTURE: usize = 128;

/// Как часто пересчитывается местность, с.
const REDRAW: f32 = 0.5;

/// Сколько мигает отметка падения снаряда, с.
const SHELL_MARK_TIME: f32 = 1.4;

/// Отступ мини-карты от края окна, пиксели.
const MARGIN: f32 = 16.0;

/// Размер метки, пиксели.
const MARKER: f32 = 7.0;

/// Сколько меток в наборе: предел живых врагов и аптечки.
const POOL: usize = 64;

const ENEMY_COLOR: Color = Color::srgb(1.0, 0.3, 0.2);
const MEDKIT_COLOR: Color = Color::srgb(1.0, 1.0, 1.0);
const CRATE_COLOR: Color = Color::srgb(1.0, 0.85, 0.15);
/// Золотой ящик улучшения: золото с белым кольцом, крупнее обычного ящика.
const UPGRADE_COLOR: Color = Color::srgb(1.0, 0.7, 0.1);
const UPGRADE_RING: Color = Color::srgb(1.0, 1.0, 1.0);
const TRAIN_COLOR: Color = Color::srgb(0.7, 0.7, 0.72);
const BOSS_COLOR: Color = Color::srgb(1.0, 0.1, 0.4);
/// Служба защиты жителей: оранжевые кружки.
const SERVICE_COLOR: Color = Color::srgb(1.0, 0.55, 0.1);
const DANGER_COLOR: Color = Color::srgb(1.0, 0.15, 0.1);
/// Кольцо места падения капсулы: цвет врагов.
const CAPSULE_COLOR: Color = ENEMY_COLOR;
/// Кольцо авиаудара игрока: жёлтое, как отметка на земле.
const STRIKE_COLOR: Color = Color::srgb(1.0, 0.85, 0.2);

/// Вид метки: цвет, размер, форма.
#[derive(Clone, Copy, PartialEq)]
enum Mark {
    Enemy,
    Service,
    Boss,
    Train,
    Crate,
    /// Золотой ящик улучшения.
    Upgrade,
    Medkit,
    Danger,
    /// Место падения десантной капсулы.
    Capsule,
    /// Вызванный авиаудар.
    Strike,
}

impl Mark {
    fn color(self) -> Color {
        match self {
            Mark::Enemy => ENEMY_COLOR,
            Mark::Service => SERVICE_COLOR,
            Mark::Boss => BOSS_COLOR,
            Mark::Train => TRAIN_COLOR,
            Mark::Crate => CRATE_COLOR,
            Mark::Upgrade => UPGRADE_COLOR,
            Mark::Medkit => MEDKIT_COLOR,
            Mark::Danger | Mark::Capsule | Mark::Strike => Color::NONE,
        }
    }

    /// Цвет кольца: у меток-колец есть только оно.
    fn ring(self) -> Color {
        match self {
            Mark::Danger => DANGER_COLOR,
            Mark::Capsule => CAPSULE_COLOR,
            Mark::Strike => STRIKE_COLOR,
            Mark::Upgrade => UPGRADE_RING,
            _ => Color::NONE,
        }
    }

    /// Ширина и высота, пиксели, и круглая ли.
    fn shape(self) -> (f32, f32, bool) {
        match self {
            Mark::Enemy | Mark::Service => (MARKER, MARKER, true),
            Mark::Boss => (15.0, 15.0, true),
            Mark::Train => (10.0, 4.0, false),
            Mark::Crate => (6.0, 6.0, false),
            Mark::Upgrade => (9.0, 9.0, false),
            Mark::Medkit => (6.0, 6.0, false),
            Mark::Danger => (12.0, 12.0, true),
            Mark::Capsule => (11.0, 11.0, true),
            Mark::Strike => (16.0, 16.0, true),
        }
    }
}

/// Местность мини-карты: текстура и время до пересчёта.
#[derive(Resource)]
struct MinimapTexture {
    image: Handle<Image>,
    timer: f32,
}

/// Отметки падения вражеских снарядов: точка и сколько ещё мигать.
#[derive(Resource, Default)]
struct DangerMarks(Vec<(Vec2, f32)>);

/// Подключает мини-карту.
pub struct MinimapPlugin;

impl Plugin for MinimapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DangerMarks>()
            .add_systems(Startup, spawn_minimap)
            .add_systems(
                Update,
                (collect_dangers, update_markers, draw_terrain)
                    .chain()
                    .in_set(GameSet::Presentation),
            );
    }
}

/// Метка из набора с её номером.
#[derive(Component)]
struct Marker(usize);

/// Смещение метки от центра мини-карты в пикселях для цели на `delta` метров от лунохода
/// (x — восток, y — юг). Цель за пределами карты прижимается к её краю.
fn marker_offset(delta: Vec2) -> Vec2 {
    let scaled = delta * (RADIUS / RANGE);
    scaled.clamp_length_max(RADIUS - MARKER)
}

fn spawn_minimap(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let image = images.add(Image::new_fill(
        Extent3d {
            width: TEXTURE as u32,
            height: TEXTURE as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 0],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    ));
    commands.insert_resource(MinimapTexture {
        image: image.clone(),
        timer: 0.0,
    });
    commands
        .spawn((
            // В меню мини-карту прячет `hud`.
            RunUi,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(MARGIN),
                bottom: Val::Px(MARGIN),
                width: Val::Px(RADIUS * 2.0),
                height: Val::Px(RADIUS * 2.0),
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.5)),
            Visibility::Hidden,
        ))
        .with_children(|map| {
            // Местность во весь круг.
            map.spawn((
                ImageNode::new(image),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(0.0),
                    width: Val::Px(RADIUS * 2.0 - 4.0),
                    height: Val::Px(RADIUS * 2.0 - 4.0),
                    ..default()
                },
            ));
            // Луноход всегда в центре.
            map.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(RADIUS - MARKER * 0.5 - 2.0),
                    top: Val::Px(RADIUS - MARKER * 0.5 - 2.0),
                    width: Val::Px(MARKER),
                    height: Val::Px(MARKER),
                    ..default()
                },
                BackgroundColor(Color::WHITE),
            ));
            for index in 0..POOL {
                map.spawn((
                    Marker(index),
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Px(MARKER),
                        height: Val::Px(MARKER),
                        border_radius: BorderRadius::MAX,
                        display: Display::None,
                        ..default()
                    },
                    BackgroundColor(ENEMY_COLOR),
                ));
            }
        });
}

/// Запоминает отметки падения снарядов врагов: на мини-карте они мигают красными кольцами.
fn collect_dangers(
    time: Res<Time>,
    mut shells: MessageReader<FireShell>,
    mut started: MessageReader<WorldRebuilt>,
    mut marks: ResMut<DangerMarks>,
) {
    if started.read().count() > 0 {
        marks.0.clear();
    }
    let dt = time.delta_secs();
    for (_, left) in &mut marks.0 {
        *left -= dt;
    }
    marks.0.retain(|(_, left)| *left > 0.0);
    for shell in shells.read() {
        if shell.team == crate::combat::Team::Enemy {
            marks.0.push((Vec2::new(shell.target.x, shell.target.z), SHELL_MARK_TIME));
        }
    }
}

/// Раздаёт метки набора врагам, боссу, поездам, ящикам, аптечкам, отметкам опасности,
/// капсулам и авиаударам. Новая метка — свой запрос в параметрах и строка `targets.extend`
/// в порядке важности.
#[allow(clippy::too_many_arguments)]
fn update_markers(
    time: Res<Time>,
    rover: Query<&Transform, With<Rover>>,
    enemies: Query<(&Transform, Has<Boss>), With<Enemy>>,
    service: Query<&Transform, With<crate::service::ServiceVehicle>>,
    capsules: Query<&Transform, With<Capsule>>,
    strikes: Query<&Strike>,
    medkits: Query<&Transform, With<Medkit>>,
    pickups: Query<(&Transform, Has<GoldenCrate>), With<Pickup>>,
    cars: Query<(&Transform, &Visibility), With<Car>>,
    dangers: Res<DangerMarks>,
    mut markers: Query<(&Marker, &mut Node, &mut BackgroundColor, &mut BorderColor)>,
) {
    let Some(rover) = rover.iter().next() else {
        return;
    };
    let flat = |t: &Transform| Vec2::new(t.translation.x, t.translation.z);
    let center = flat(rover);
    let blink = (time.elapsed_secs() * 5.0).fract() < 0.6;
    // Важные метки первыми: набор ограничен.
    let mut targets: Vec<(Vec2, Mark)> = Vec::new();
    targets.extend(enemies.iter().filter(|(_, b)| *b).map(|(t, _)| (flat(t), Mark::Boss)));
    if blink {
        targets.extend(dangers.0.iter().map(|(p, _)| (*p, Mark::Danger)));
        targets.extend(capsules.iter().map(|t| (flat(t), Mark::Capsule)));
    }
    targets.extend(strikes.iter().map(|s| (Vec2::new(s.center.x, s.center.z), Mark::Strike)));
    targets.extend(enemies.iter().filter(|(_, b)| !*b).map(|(t, _)| (flat(t), Mark::Enemy)));
    targets.extend(service.iter().map(|t| (flat(t), Mark::Service)));
    targets.extend(cars.iter().filter(|(_, v)| **v != Visibility::Hidden).map(|(t, _)| (flat(t), Mark::Train)));
    targets.extend(
        pickups
            .iter()
            .map(|(t, golden)| (flat(t), if golden { Mark::Upgrade } else { Mark::Crate })),
    );
    targets.extend(medkits.iter().map(|t| (flat(t), Mark::Medkit)));
    targets.truncate(POOL);

    for (marker, mut node, mut color, mut border) in &mut markers {
        let Some((p, mark)) = targets.get(marker.0) else {
            if node.display != Display::None {
                node.display = Display::None;
            }
            continue;
        };
        let offset = marker_offset(*p - center);
        let (w, h, round) = mark.shape();
        // Центр карты — на RADIUS от внешнего края, рамка съедает два пикселя.
        let left = Val::Px(RADIUS + offset.x - w * 0.5 - 2.0);
        let top = Val::Px(RADIUS + offset.y - h * 0.5 - 2.0);
        if node.left != left || node.top != top || node.display != Display::Flex || node.width != Val::Px(w) {
            node.left = left;
            node.top = top;
            node.width = Val::Px(w);
            node.height = Val::Px(h);
            node.display = Display::Flex;
            node.border_radius = if round { BorderRadius::MAX } else { BorderRadius::ZERO };
            node.border = if mark.ring() != Color::NONE { UiRect::all(Val::Px(2.0)) } else { UiRect::ZERO };
        }
        if color.0 != mark.color() {
            color.0 = mark.color();
        }
        let ring = mark.ring();
        if *border != BorderColor::all(ring) {
            *border = BorderColor::all(ring);
        }
    }
}

/// Пиксель текстуры для точки `delta` метров от лунохода, если она внутри круга.
fn pixel_of(delta: Vec2) -> Option<(usize, usize)> {
    if delta.length() >= RANGE {
        return None;
    }
    let k = TEXTURE as f32 / (RANGE * 2.0);
    let x = (delta.x * k + TEXTURE as f32 * 0.5).floor();
    let y = (delta.y * k + TEXTURE as f32 * 0.5).floor();
    (x >= 0.0 && y >= 0.0 && (x as usize) < TEXTURE && (y as usize) < TEXTURE).then_some((x as usize, y as usize))
}

/// Перерисовывает местность раз в полсекунды: цвета рельефа, пути, дома и рамка кадра.
#[allow(clippy::too_many_arguments)]
fn draw_terrain(
    time: Res<Time<Real>>,
    terrain: Res<Terrain>,
    settlements: Res<Settlements>,
    mut texture: ResMut<MinimapTexture>,
    mut images: ResMut<Assets<Image>>,
    rover: Query<&Transform, With<Rover>>,
    houses: Query<&House>,
    camera: Query<(&Camera, &GlobalTransform), With<GameCamera>>,
    state: Res<State<GameState>>,
    lava: Res<crate::lava::LavaPools>,
) {
    texture.timer -= time.delta_secs();
    if texture.timer > 0.0 || *state.get() == GameState::Menu {
        return;
    }
    texture.timer = REDRAW;
    let Some(rover) = rover.iter().next() else {
        return;
    };
    let Some(mut image) = images.get_mut(&texture.image) else {
        return;
    };
    let Some(data) = image.data.as_mut() else {
        return;
    };
    let center = Vec2::new(rover.translation.x, rover.translation.z);
    let step = RANGE * 2.0 / TEXTURE as f32;
    let to_srgb = |c: f32| (c.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8;
    let put = |data: &mut [u8], x: usize, y: usize, rgb: [u8; 3]| {
        let i = (y * TEXTURE + x) * 4;
        data[i..i + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 230]);
    };
    for y in 0..TEXTURE {
        for x in 0..TEXTURE {
            let delta = Vec2::new((x as f32 + 0.5) * step - RANGE, (y as f32 + 0.5) * step - RANGE);
            let i = (y * TEXTURE + x) * 4;
            if delta.length() >= RANGE {
                data[i..i + 4].copy_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            let p = center + delta;
            let color = terrain.vertex_color(p.x.round() as i32, p.y.round() as i32).unwrap_or([0.1, 0.25, 0.1, 1.0]);
            // Чуть темнее настоящего, чтобы метки читались.
            put(data, x, y, [to_srgb(color[0] * 0.8), to_srgb(color[1] * 0.8), to_srgb(color[2] * 0.8)]);
        }
    }
    // Пути: серые точки вдоль ближайших линий.
    for near in terrain.lines().near(center, RANGE) {
        let mut s = near.s - RANGE;
        while s < near.s + RANGE {
            let p = terrain.lines().point(near.line, s);
            if let Some((x, y)) = pixel_of(p - center) {
                put(data, x, y, [150, 145, 140]);
            }
            s += step * 0.7;
        }
    }
    // Дома: тёмные квадраты по прямоугольнику.
    for house in &houses {
        let Some(state) = settlements.house(house.0) else {
            continue;
        };
        let footprint = state.footprint();
        let half = state.kind.half_extents();
        let mut lx = -half.x;
        while lx <= half.x {
            let mut lz = -half.y;
            while lz <= half.y {
                if let Some((x, y)) = pixel_of(footprint.to_world(Vec2::new(lx, lz)) - center) {
                    put(data, x, y, [70, 50, 45]);
                }
                lz += step * 0.7;
            }
            lx += step * 0.7;
        }
    }
    // Лава: оранжевые ячейки озёр.
    for cell in lava.cells() {
        if let Some((x, y)) = pixel_of(cell.as_vec2() - center) {
            put(data, x, y, [255, 120, 20]);
        }
    }
    // Рамка кадра: углы экрана на плоскости под луноходом.
    if let Some((camera, camera_transform)) = camera.iter().next()
        && let Some(viewport) = camera.logical_viewport_size()
    {
        let ground = rover.translation.y;
        let corners: Vec<Vec2> = [Vec2::ZERO, Vec2::new(viewport.x, 0.0), viewport, Vec2::new(0.0, viewport.y)]
            .iter()
            .filter_map(|c| {
                let ray = camera.viewport_to_world(camera_transform, *c).ok()?;
                let t = (ground - ray.origin.y) / ray.direction.y;
                (t > 0.0).then(|| {
                    let p = ray.origin + *ray.direction * t;
                    Vec2::new(p.x, p.z)
                })
            })
            .collect();
        if corners.len() == 4 {
            for i in 0..4 {
                let (a, b) = (corners[i], corners[(i + 1) % 4]);
                let n = (a.distance(b) / (step * 0.7)).ceil() as usize;
                for k in 0..=n {
                    if let Some((x, y)) = pixel_of(a.lerp(b, k as f32 / n.max(1) as f32) - center) {
                        put(data, x, y, [235, 235, 235]);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_target_is_scaled_and_far_target_sticks_to_the_edge() {
        // Половина дальности на восток — половина радиуса вправо.
        let near = marker_offset(Vec2::new(RANGE * 0.5, 0.0));
        assert!((near.x - RADIUS * 0.5).abs() < 1e-3 && near.y == 0.0);

        // Далеко на юго-западе — на краю круга, в ту же сторону.
        let far = marker_offset(Vec2::new(-500.0, 500.0));
        assert!((far.length() - (RADIUS - MARKER)).abs() < 1e-3);
        assert!(far.x < 0.0 && far.y > 0.0);
    }
}
