//! Интерфейс во время забега: полоса здоровья слева сверху, время выживания по центру
//! и выбранное оружие справа сверху.

use bevy::prelude::*;

use crate::combat::{Health, SurvivalTime};
use crate::rover::Rover;
use crate::run::GameState;
use crate::weapons::CurrentWeapon;

/// Отступ интерфейса от краёв окна.
const MARGIN: f32 = 16.0;

/// Размер полосы здоровья.
const BAR_SIZE: Vec2 = Vec2::new(260.0, 22.0);

/// Тёмная подложка под элементами, чтобы они читались на зелёном и коричневом.
const PANEL_COLOR: Color = Color::srgba(0.0, 0.0, 0.0, 0.65);

/// Подключает интерфейс забега.
pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_hud)
            .add_systems(Update, (update_health_bar, update_time, update_weapon, hide_in_menu));
    }
}

/// Корень полосы здоровья: прячется в меню вместе с временем и оружием.
#[derive(Component)]
struct HealthBar;

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

fn spawn_hud(mut commands: Commands) {
    // Полоса здоровья.
    commands
        .spawn((
            HealthBar,
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
        .spawn(Node {
            position_type: PositionType::Absolute,
            top: Val::Px(MARGIN),
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            justify_content: JustifyContent::Center,
            ..default()
        })
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

/// В меню интерфейса забега нет.
fn hide_in_menu(
    state: Res<State<GameState>>,
    mut parts: Query<&mut Visibility, Or<(With<HealthBar>, With<TimeText>, With<WeaponText>)>>,
) {
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

fn update_health_bar(
    rover: Query<&Health, With<Rover>>,
    mut fill: Query<(&mut Node, &mut BackgroundColor), With<HealthFill>>,
    mut text: Query<&mut Text, With<HealthText>>,
) {
    let Some(health) = rover.iter().next() else {
        return;
    };
    let fraction = health.fraction();
    for (mut node, mut color) in &mut fill {
        node.width = Val::Percent(fraction * 100.0);
        color.0 = health_color(fraction);
    }
    for mut text in &mut text {
        let label = format!("{:.0}", health.current.ceil());
        if text.0 != label {
            text.0 = label;
        }
    }
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
    fn health_color_goes_from_green_to_red() {
        let full = health_color(1.0).to_srgba();
        let empty = health_color(0.0).to_srgba();
        assert!(full.green > full.red);
        assert!(empty.red > empty.green);
    }
}
