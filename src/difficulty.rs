//! Уровни сложности: Лёгкая, Нормальная, Тяжёлая.
//!
//! Уровень выбирается в меню и хранится в настройках (`settings::Settings::difficulty`). Забег
//! идёт на уровне из ресурса [`RunDifficulty`]: меню ставит его, когда начинает забег, поэтому
//! смена уровня в меню не трогает идущий забег, а рестарт идёт на том же уровне.
//!
//! Уровень меняет только числа ниже (спецификация `difficulty`): урон по луноходу
//! (`combat::apply_damage`), рост угрозы волн (`waves::Threat`), самовосстановление
//! (`regen::RegenSettings`) и помощь прицела стиком (`stick_aim`).

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::regen::RegenSettings;

/// Уровень сложности.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Difficulty {
    Easy,
    #[default]
    Normal,
    Hard,
}

/// Помощь прицела с геймпада: полоса захвата и удержания врага, м, и замедление курсора над врагом.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AimAssist {
    /// Враг ловится, если отклонение от линии прицела не больше этого, м.
    pub catch_width: f32,
    /// Пойманный враг держится, пока отклонение не больше этого, м.
    pub keep_width: f32,
    /// Множитель скорости курсора над врагом (L2).
    pub cursor_friction: f32,
}

impl Difficulty {
    /// Все уровни по порядку.
    pub const ALL: [Difficulty; 3] = [Difficulty::Easy, Difficulty::Normal, Difficulty::Hard];

    /// Название для экранов.
    pub fn title(self) -> &'static str {
        match self {
            Difficulty::Easy => "EASY",
            Difficulty::Normal => "NORMAL",
            Difficulty::Hard => "HARD",
        }
    }

    /// Номер уровня в [`Difficulty::ALL`].
    pub fn index(self) -> usize {
        match self {
            Difficulty::Easy => 0,
            Difficulty::Normal => 1,
            Difficulty::Hard => 2,
        }
    }

    /// Соседний уровень на шаг `step` по кругу.
    pub fn step(self, step: i32) -> Self {
        Self::ALL[(self.index() as i32 + step).rem_euclid(Self::ALL.len() as i32) as usize]
    }

    /// Множитель урона по луноходу.
    pub fn damage_to_rover(self) -> f32 {
        match self {
            Difficulty::Easy => 0.6,
            Difficulty::Normal => 1.0,
            Difficulty::Hard => 1.5,
        }
    }

    /// Множитель роста угрозы волн.
    pub fn threat_rate(self) -> f32 {
        match self {
            Difficulty::Easy => 0.75,
            Difficulty::Normal => 1.0,
            Difficulty::Hard => 1.35,
        }
    }

    /// Пауза и скорость самовосстановления.
    pub fn regen(self) -> RegenSettings {
        let (delay, rate) = match self {
            Difficulty::Easy => (3.0, 0.02),
            Difficulty::Normal => (5.0, 0.01),
            Difficulty::Hard => (8.0, 0.005),
        };
        RegenSettings { delay, rate }
    }

    /// Помощь прицела стиком.
    pub fn aim_assist(self) -> AimAssist {
        let (catch_width, keep_width, cursor_friction) = match self {
            Difficulty::Easy => (4.0, 6.0, 0.25),
            Difficulty::Normal => (2.5, 4.0, 0.4),
            Difficulty::Hard => (1.8, 3.0, 0.6),
        };
        AimAssist {
            catch_width,
            keep_width,
            cursor_friction,
        }
    }
}

/// Уровень идущего забега: меню ставит его из настроек, когда начинает забег.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunDifficulty(pub Difficulty);

/// Плагин сложности.
pub struct DifficultyPlugin;

impl Plugin for DifficultyPlugin {
    fn build(&self, app: &mut App) {
        // До первого забега уровень — из настроек: так его видят и мир меню, и отладочный автозапуск.
        let chosen = crate::settings::stored(app).difficulty;
        app.insert_resource(RunDifficulty(chosen));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn урон_10_на_тяжёлой_даёт_15() {
        assert!((10.0 * Difficulty::Hard.damage_to_rover() - 15.0).abs() < 1e-5);
        assert!((10.0 * Difficulty::Easy.damage_to_rover() - 6.0).abs() < 1e-5);
        assert_eq!(Difficulty::Normal.damage_to_rover(), 1.0);
    }

    #[test]
    fn уровни_по_кругу() {
        assert_eq!(Difficulty::Normal.step(1), Difficulty::Hard);
        assert_eq!(Difficulty::Hard.step(1), Difficulty::Easy);
        assert_eq!(Difficulty::Easy.step(-1), Difficulty::Hard);
    }

    #[test]
    fn нормальная_совпадает_с_прежними_числами() {
        let regen = Difficulty::Normal.regen();
        let old = RegenSettings::default();
        assert_eq!((regen.delay, regen.rate), (old.delay, old.rate));
        assert_eq!(Difficulty::Hard.regen().delay, 8.0);
    }
}
