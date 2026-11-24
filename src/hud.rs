//! Интерфейс во время забега: полоса здоровья слева сверху, время выживания по центру
//! и выбранное оружие справа сверху.
//!
//! Интерфейс забега прячется в меню одной системой: корню любой панели забега достаточно
//! компонента [`RunUi`].

use bevy::color::Mix;
use bevy::prelude::*;

use crate::combat::{Health, SurvivalTime};
use crate::frame::GameSet;
use crate::regen::Regen;
use crate::rover::Rover;
use crate::run::GameState;
use crate::weapons::CurrentWeapon;

/// Отступ интерфейса от краёв окна.
pub const MARGIN: f32 = 16.0;

/// Размер полосы здоровья.
const BAR_SIZE: Vec2 = Vec2::new(260.0, 22.0);

/// Тёмная подложка под элементами, чтобы они читались на зелёном и коричневом.
const PANEL_COLOR: Color = Color::srgba(0.0, 0.0, 0.0, 0.65);

/// Подключает интерфейс забега.
pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_hud)
            .add_systems(
                Update,
                (update_health_bar, update_time, update_weapon, place_hud, place_safe_anchors, hide_run_ui_in_menu)
                    .in_set(GameSet::Presentation),
            );
    }
}

/// Корень панели интерфейса забега: в меню спрятан, в забеге, на паузе и после гибели виден.
#[derive(Component, Default)]
pub struct RunUi;

/// Заполненная часть полосы здоровья.
#[derive(Component)]
struct HealthFill;

/// Число здоровья на полосе.
#[derive(Component)]
struct HealthText;

/// Текст времени выживания.
#[derive(Component)]
struct TimeText;

/// Название выбранного оружия.
#[derive(Component)]
struct WeaponText;

/// Корень полосы здоровья.
#[derive(Component)]
struct HealthBarRoot;

/// Строка времени выживания.
#[derive(Component)]
struct TimeRow;

/// Отступы от вырезов экрана (слева, сверху, справа, снизу) в пикселях интерфейса: интерфейс
/// забега их обходит, как и сенсорные кнопки (`platform::safe_area`).
pub fn safe_area_ui(ui_scale: &UiScale) -> Vec4 {
    crate::platform::safe_area() / crate::platform::canvas_scale() / ui_scale.0.max(f32::EPSILON)
}

/// Узел интерфейса, прижатый к краю экрана: отступы от края в пикселях интерфейса, к ним
/// прибавляется вырез экрана с той же стороны (`place_safe_anchors`). Задавай только стороны,
/// которые отсчитываются от края; `None` — сторону не трогать.
#[derive(Component, Clone, Copy, Default)]
pub struct SafeAnchor {
    pub left: Option<f32>,
    pub top: Option<f32>,
    pub right: Option<f32>,
    pub bottom: Option<f32>,
}

fn place_safe_anchors(ui_scale: Res<UiScale>, mut nodes: Query<(&SafeAnchor, &mut Node)>) {
    let safe = safe_area_ui(&ui_scale);
    for (anchor, mut node) in &mut nodes {
        let px = |side: Option<f32>, inset: f32| side.map(|margin| Val::Px(margin + inset));
        set_edges(
            &mut node,
            [
                px(anchor.left, safe.x),
                px(anchor.top, safe.y),
                px(anchor.right, safe.z),
                px(anchor.bottom, safe.w),
            ],
        );
    }
}

/// Края узла — слева, сверху, справа, снизу; `None` — край не трогать. Узел берётся на запись,
/// только если значение другое: иначе он каждый кадр считался бы изменённым и вёрстка
/// пересчитывалась бы заново.
pub fn set_edges(node: &mut Mut<Node>, edges: [Option<Val>; 4]) {
    let now = [node.left, node.top, node.right, node.bottom];
    if edges.iter().zip(now).all(|(want, now)| want.is_none_or(|want| want == now)) {
        return;
    }
    let node = &mut **node;
    for (want, field) in edges.into_iter().zip([&mut node.left, &mut node.top, &mut node.right, &mut node.bottom]) {
        if let Some(want) = want {
            *field = want;
        }
    }
}

fn spawn_hud(mut commands: Commands) {
    // Полоса здоровья.
    commands
        .spawn((
            RunUi,
            HealthBarRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(MARGIN),
                top: Val::Px(MARGIN),
                width: Val::Px(BAR_SIZE.x),
                height: Val::Px(BAR_SIZE.y),
                padding: UiRect::all(Val::Px(3.0)),
                ..default()
            },
            BackgroundColor(PANEL_COLOR),
        ))
        .with_children(|bar| {
            bar.spawn((
                HealthFill,
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                BackgroundColor(health_color(1.0)),
            ));
            bar.spawn((
                HealthText,
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(8.0),
                    top: Val::Px(2.0),
                    ..default()
                },
            ));
        });

    // Время выживания.
    commands
        .spawn((
            RunUi,
            TimeRow,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(MARGIN),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|row| {
            row.spawn((
                TimeText,
                Text::new(format_time(0.0)),
                TextFont {
                    font_size: FontSize::Px(26.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                Node {
                    padding: UiRect::axes(Val::Px(12.0), Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(PANEL_COLOR),
            ));
        });

    // Выбранное оружие.
    commands.spawn((
        WeaponText,
        RunUi,
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(18.0),
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(MARGIN),
            right: Val::Px(MARGIN),
            padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
            ..default()
        },
        BackgroundColor(PANEL_COLOR),
    ));
}

/// В меню интерфейса забега нет: прячет всё с [`RunUi`].
fn hide_run_ui_in_menu(state: Res<State<GameState>>, mut parts: Query<&mut Visibility, With<RunUi>>) {
    let wanted = if *state.get() == GameState::Menu {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    for mut visibility in &mut parts {
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

/// Места частей интерфейса забега: обходят вырез камеры и закруглённые углы (`safe_area_ui`),
/// а при касаниях в правом верхнем углу пауза и мини-карта — название оружия под мини-картой.
/// Вырез переходит на другой бок при повороте телефона, поэтому места пересчитываются каждый кадр.
fn place_hud(
    device: Res<crate::gamepad::InputDevice>,
    ui_scale: Res<UiScale>,
    settings: Res<crate::settings::Settings>,
    mut health: Query<&mut Node, (With<HealthBarRoot>, Without<TimeRow>, Without<WeaponText>)>,
    mut time: Query<&mut Node, (With<TimeRow>, Without<HealthBarRoot>, Without<WeaponText>)>,
    mut label: Query<&mut Node, (With<WeaponText>, Without<HealthBarRoot>, Without<TimeRow>)>,
) {
    let safe = safe_area_ui(&ui_scale);
    let px = |value: f32| Some(Val::Px(value));
    for mut node in &mut health {
        set_edges(&mut node, [px(MARGIN + safe.x), px(MARGIN + safe.y), None, None]);
    }
    for mut node in &mut time {
        set_edges(&mut node, [None, px(MARGIN + safe.y), None, None]);
    }
    let touch = *device == crate::gamepad::InputDevice::Touch;
    for mut node in &mut label {
        let (top, right) = if touch {
            let pause = crate::touch::pause_footprint_ui(&settings, &ui_scale);
            (MARGIN + safe.y + crate::minimap::DIAMETER + 8.0, MARGIN + safe.z + pause)
        } else {
            (MARGIN + safe.y, MARGIN + safe.z)
        };
        set_edges(&mut node, [None, px(top), px(right), None]);
    }
}

fn update_weapon(weapon: Res<CurrentWeapon>, mut text: Query<&mut Text, With<WeaponText>>) {
    for mut text in &mut text {
        if text.0 != weapon.name() {
            text.0 = weapon.name().to_string();
        }
    }
}

/// Цвет полосы: зелёный при полном здоровье, жёлтый на половине, красный у нуля.
fn health_color(fraction: f32) -> Color {
    let fraction = fraction.clamp(0.0, 1.0);
    let red = (2.0 * (1.0 - fraction)).min(1.0);
    let green = (2.0 * fraction).min(1.0);
    Color::srgb(0.15 + 0.75 * red, 0.15 + 0.65 * green, 0.15)
}

/// Время в виде `минуты:секунды.десятые`.
pub fn format_time(seconds: f32) -> String {
    let tenths = (seconds.max(0.0) * 10.0).floor() as u32;
    format!("{}:{:02}.{}", tenths / 600, tenths / 10 % 60, tenths % 10)
}

/// Насколько полоса здоровья светлеет при восстановлении: 0–25 % белого, ~1,5 Гц.
fn regen_glow(seconds: f32) -> f32 {
    0.125 * (1.0 + (seconds * 1.5 * std::f32::consts::TAU).sin())
}

fn update_health_bar(
    time: Res<Time>,
    regen: Res<Regen>,
    rover: Query<&Health, With<Rover>>,
    mut fill: Query<(&mut Node, &mut BackgroundColor), With<HealthFill>>,
    mut text: Query<&mut Text, With<HealthText>>,
) {
    let Some(health) = rover.iter().next() else {
        return;
    };
    let fraction = health.fraction();
    // Пишется только изменившееся: запись в `Node` пересчитывает вёрстку интерфейса.
    let width = Val::Percent(fraction * 100.0);
    let base = health_color(fraction);
    // Пока здоровье само растёт, полоса мягко пульсирует светлее своего цвета.
    let wanted = if regen.active {
        Color::from(base.to_srgba().mix(&Srgba::WHITE, regen_glow(time.elapsed_secs())))
    } else {
        base
    };
    for (mut node, mut color) in &mut fill {
        if node.width != width {
            node.width = width;
        }
        if color.0 != wanted {
            color.0 = wanted;
        }
    }
    for mut text in &mut text {
        let label = health_label(fraction);
        if text.0 != label {
            text.0 = label;
        }
    }
}

/// Надпись на полосе здоровья — проценты: 100 только у целого лунохода, 0 только у погибшего.
/// Любая царапина уже даёт 99, последний клочок здоровья — 1.
fn health_label(fraction: f32) -> String {
    let percent = if fraction >= 1.0 {
        100.0
    } else if fraction <= 0.0 {
        0.0
    } else {
        (fraction * 100.0).ceil().clamp(1.0, 99.0)
    };
    format!("{percent:.0}%")
}

fn update_time(survival: Res<SurvivalTime>, mut text: Query<&mut Text, With<TimeText>>) {
    let label = format_time(survival.0);
    for mut text in &mut text {
        if text.0 != label {
            text.0.clone_from(&label);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_is_formatted_as_minutes_seconds_tenths() {
        assert_eq!(format_time(0.0), "0:00.0");
        assert_eq!(format_time(7.38), "0:07.3");
        assert_eq!(format_time(59.99), "0:59.9");
        assert_eq!(format_time(60.0), "1:00.0");
        assert_eq!(format_time(187.25), "3:07.2");
        assert_eq!(format_time(-3.0), "0:00.0");
    }

    #[test]
    fn health_label_is_percent() {
        assert_eq!(health_label(1.0), "100%");
        assert_eq!(health_label(1499.0 / 1500.0), "99%");
        assert_eq!(health_label(0.5), "50%");
        assert_eq!(health_label(0.421), "43%");
        assert_eq!(health_label(1.0 / 1500.0), "1%");
        assert_eq!(health_label(0.0), "0%");
    }

    #[test]
    fn health_color_goes_from_green_to_red() {
        let full = health_color(1.0).to_srgba();
        let empty = health_color(0.0).to_srgba();
        assert!(full.green > full.red);
        assert!(empty.red > empty.green);
    }
}
