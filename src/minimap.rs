//! Мини-карта: круг в углу экрана с луноходом в центре и метками того, что за кадром.
//!
//! Камера не вращается, поэтому верх мини-карты всегда совпадает с верхом экрана.
//! Метки берутся из заранее созданного набора узлов: во время забега узлы не создаются.

use bevy::prelude::*;

use crate::enemy::Enemy;
use crate::repair::Medkit;
use crate::rover::Rover;
use crate::run::GameState;

/// Радиус мини-карты на экране, пиксели.
const RADIUS: f32 = 90.0;

/// Расстояние от лунохода до края мини-карты, м. Враги появляются в 45–55 м.
const RANGE: f32 = 70.0;

/// Отступ мини-карты от края окна, пиксели.
const MARGIN: f32 = 16.0;

/// Размер метки, пиксели.
const MARKER: f32 = 7.0;

/// Сколько меток в наборе: предел живых врагов и аптечки.
const POOL: usize = 32;

const ENEMY_COLOR: Color = Color::srgb(1.0, 0.3, 0.2);
const MEDKIT_COLOR: Color = Color::srgb(0.3, 1.0, 0.4);

/// Подключает мини-карту.
pub struct MinimapPlugin;

impl Plugin for MinimapPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_minimap)
            .add_systems(Update, (show_in_run, update_markers).chain());
    }
}

/// Корень мини-карты.
#[derive(Component)]
struct Minimap;

/// Метка из набора с её номером.
#[derive(Component)]
struct Marker(usize);

/// Смещение метки от центра мини-карты в пикселях для цели на `delta` метров от лунохода
/// (x — восток, y — юг). Цель за пределами карты прижимается к её краю.
fn marker_offset(delta: Vec2) -> Vec2 {
    let scaled = delta * (RADIUS / RANGE);
    scaled.clamp_length_max(RADIUS - MARKER)
}

fn spawn_minimap(mut commands: Commands) {
    commands
        .spawn((
            Minimap,
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

/// Мини-карта видна только в забеге, на паузе и после гибели.
fn show_in_run(state: Res<State<GameState>>, mut maps: Query<&mut Visibility, With<Minimap>>) {
    let wanted = if *state.get() == GameState::Menu {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    for mut visibility in &mut maps {
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

/// Раздаёт метки набора врагам и аптечкам, лишние прячет.
fn update_markers(
    rover: Query<&Transform, With<Rover>>,
    enemies: Query<&Transform, With<Enemy>>,
    medkits: Query<&Transform, With<Medkit>>,
    mut markers: Query<(&Marker, &mut Node, &mut BackgroundColor)>,
) {
    let Some(rover) = rover.iter().next() else {
        return;
    };
    let targets: Vec<(Vec2, Color)> = medkits
        .iter()
        .map(|medkit| (medkit, MEDKIT_COLOR))
        .chain(enemies.iter().map(|enemy| (enemy, ENEMY_COLOR)))
        .map(|(transform, color)| {
            let delta = transform.translation - rover.translation;
            (marker_offset(Vec2::new(delta.x, delta.z)), color)
        })
        .take(POOL)
        .collect();

    for (marker, mut node, mut color) in &mut markers {
        let Some((offset, wanted)) = targets.get(marker.0) else {
            if node.display != Display::None {
                node.display = Display::None;
            }
            continue;
        };
        // Центр карты — на RADIUS от внешнего края, рамка съедает два пикселя.
        let left = Val::Px(RADIUS + offset.x - MARKER * 0.5 - 2.0);
        let top = Val::Px(RADIUS + offset.y - MARKER * 0.5 - 2.0);
        if node.left != left || node.top != top || node.display != Display::Flex {
            node.left = left;
            node.top = top;
            node.display = Display::Flex;
        }
        if color.0 != *wanted {
            color.0 = *wanted;
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
