//! Версия игры всегда на экране: маленькая подпись сверху, справа от полосы здоровья — на одной строке
//! с ней, но не на ней и не под таймером. Видна в загрузке, меню, забеге и на паузе, поверх занавеса.
//! Где она нужна, решает профиль платформы (`menu.version`: на Android, пока идут тесты); выключить без
//! пересборки — `adb shell setprop debug.spaceguard.menu.version false`.

use bevy::prelude::*;

use crate::hud::{BAR_SIZE, MARGIN, SafeAnchor};

/// Подключает подпись версии.
pub struct VersionLabelPlugin;

impl Plugin for VersionLabelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_label);
    }
}

fn spawn_label(mut commands: Commands) {
    if !crate::profile::get().menu.version {
        return;
    }
    let version = crate::platform::app_version();
    info!("Версия на экране: {version}");
    commands.spawn((
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
        // На одной высоте с полосой здоровья, правее неё; вырез экрана обходится, как у HUD.
        SafeAnchor {
            left: Some(MARGIN + BAR_SIZE.x + 12.0),
            top: Some(MARGIN + 1.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
        GlobalZIndex(crate::fade::ABOVE_CURTAIN_Z),
        Pickable::IGNORE,
    ));
}
