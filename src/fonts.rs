//! Шрифт интерфейса: DejaVu Sans Mono вместо встроенного в Bevy FiraMono-subset.
//!
//! Во встроенном шрифте только латиница, и кириллица надписей («ПОЛНЫЙ БОЕЗАПАС!») и знак «∞»
//! у пулемёта рисовались квадратами. Шрифт вшит в бинарник и подменяет ручку шрифта по
//! умолчанию, поэтому `TextFont::default()` везде, в браузере тоже, берёт его без загрузки файла.

use bevy::asset::AssetId;
use bevy::prelude::*;

/// DejaVu Sans Mono: латиница, кириллица, «∞». Лицензия — `assets/fonts/DejaVu-LICENSE`.
const FONT_DATA: &[u8] = include_bytes!("../assets/fonts/DejaVuSansMono.ttf");

/// Подменяет шрифт по умолчанию. Ставится после `DefaultPlugins`.
pub struct FontsPlugin;

impl Plugin for FontsPlugin {
    fn build(&self, app: &mut App) {
        let mut fonts = app.world_mut().resource_mut::<Assets<Font>>();
        fonts
            .insert(AssetId::default(), Font::from_bytes(FONT_DATA.to_vec()))
            .expect("ручка шрифта по умолчанию свободна для замены");
    }
}
