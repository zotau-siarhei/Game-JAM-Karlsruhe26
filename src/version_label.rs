//! Версия игры на стартовом экране: маленькая подпись в левом верхнем углу, пока открыто меню (на любой
//! его странице). В загрузке, забеге, на паузе и после гибели её нет. Под затемнением переходов она
//! прячется вместе с меню. Где она нужна, решает профиль платформы (`menu.version`: на Android, пока идут
//! тесты); выключить без пересборки — `adb shell setprop debug.spaceguard.menu.version false`.

use bevy::prelude::*;

use crate::frame::GameSet;
use crate::hud::{MARGIN, SafeAnchor};
use crate::run::GameState;

/// Подключает подпись версии.
pub struct VersionLabelPlugin;

impl Plugin for VersionLabelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_label)
            .add_systems(Update, show_in_menu.in_set(GameSet::Presentation));
    }
}

/// Подпись версии.
#[derive(Component)]
struct VersionLabel;

fn spawn_label(mut commands: Commands) {
    if !crate::profile::get().menu.version {
        return;
    }
    let version = crate::platform::app_version();
    info!("Версия на стартовом экране: {version}");
    commands.spawn((
        VersionLabel,
        Text::new(version),
        TextFont {
            font_size: FontSize::Px(15.0),
            ..default()
        },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.9)),
        Node {
            position_type: PositionType::Absolute,
            padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
            ..default()
        },
        // Левый верхний угол; вырез экрана обходится, как у интерфейса забега.
        SafeAnchor {
            left: Some(MARGIN),
            top: Some(MARGIN),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
        Visibility::Hidden,
        Pickable::IGNORE,
    ));
}

fn show_in_menu(state: Res<State<GameState>>, mut labels: Query<&mut Visibility, With<VersionLabel>>) {
    let wanted = if *state.get() == GameState::Menu { Visibility::Inherited } else { Visibility::Hidden };
    for mut visibility in &mut labels {
        visibility.set_if_neq(wanted);
    }
}
