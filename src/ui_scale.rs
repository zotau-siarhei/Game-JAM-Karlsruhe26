//! Интерфейс на маленьком экране: на телефоне надписи, панели и мини-карта рассчитаны на окно
//! ноутбука и закрывают почти весь кадр. Если окно меньше 1280×720 логических точек, весь
//! интерфейс (`UiScale`) уменьшается в той же доле, что и окно; на больших окнах масштаб 1.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::frame::GameSet;

/// Окно, под которое нарисован интерфейс, логические точки.
const DESIGN_SIZE: Vec2 = Vec2::new(1280.0, 720.0);
/// Мельче этого интерфейс не становится: иначе текст не прочитать.
const MIN_SCALE: f32 = 0.5;

/// Плагин масштаба интерфейса.
pub struct UiScalePlugin;

impl Plugin for UiScalePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, fit_ui.in_set(GameSet::Presentation));
    }
}

/// Масштаб интерфейса для окна `size`: 1 на окне не меньше `DESIGN_SIZE`, меньше — по более
/// тесной стороне.
fn scale_for(size: Vec2) -> f32 {
    let fit = (size / DESIGN_SIZE).min_element();
    fit.clamp(MIN_SCALE, 1.0)
}

fn fit_ui(windows: Query<&Window, With<PrimaryWindow>>, mut ui_scale: ResMut<UiScale>) {
    let Ok(window) = windows.single() else {
        return;
    };
    let size = window.resolution.size();
    if size.min_element() <= 0.0 {
        return;
    }
    let scale = scale_for(size);
    if (ui_scale.0 - scale).abs() > 0.01 {
        ui_scale.0 = scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn большое_окно_без_изменений() {
        assert_eq!(scale_for(Vec2::new(1280.0, 720.0)), 1.0);
        assert_eq!(scale_for(Vec2::new(1470.0, 956.0)), 1.0);
    }

    #[test]
    fn телефон_уменьшает_по_тесной_стороне() {
        let scale = scale_for(Vec2::new(1020.0, 461.0));
        assert!((scale - 461.0 / 720.0).abs() < 1e-4, "{scale}");
    }

    #[test]
    fn не_мельче_предела() {
        assert_eq!(scale_for(Vec2::new(320.0, 200.0)), MIN_SCALE);
    }
}
