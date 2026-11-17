//! Настройки игрока: качество картинки, громкости, субтитры, вибрация и сложность.
//!
//! Всё лежит в одном ресурсе [`Settings`] и в файле `settings.json` рядом с рекордами. Файл
//! пишется через временный при каждом изменении ресурса. Каждое поле читается с умолчанием,
//! поэтому прежний файл с одним качеством (`{"quality":"high"}`) читается, а остальное берётся
//! по умолчанию; файл, который не разбирается, даёт настройки по умолчанию и не мешает запуску.
//! В браузере файлов нет: настройки живут до перезагрузки страницы.
//!
//! Качество применяет `quality.rs` (ресурс `Quality`, его же переключает Q на паузе), громкость —
//! музыка, звуки и голос через [`Settings::gain`], субтитры — `hq.rs`, вибрацию — `haptics.rs`, сложность — меню,
//! когда начинает забег (`difficulty::RunDifficulty`).

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::difficulty::Difficulty;
use crate::frame::GameSet;
use crate::quality::Quality;

/// Имя файла настроек.
const FILE_NAME: &str = "settings.json";

/// Шаг громкости в меню, %.
pub const VOLUME_STEP: u8 = 10;

/// Группа звука для громкости.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundGroup {
    Music,
    /// Звуки мира и интерфейса, мотор лунохода.
    Effects,
    /// Реплики штаба.
    Voice,
}

/// Настройки игрока.
#[derive(Resource, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub quality: Quality,
    /// Общая громкость, % от 0 до 100.
    pub master: u8,
    /// Громкость музыки, %.
    pub music: u8,
    /// Громкость эффектов, %.
    pub effects: u8,
    /// Громкость голоса штаба, %.
    pub voice: u8,
    /// Субтитры реплик штаба.
    pub subtitles: bool,
    /// Вибрация и световая панель геймпада.
    pub vibration: bool,
    /// Сложность следующего забега.
    pub difficulty: Difficulty,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            quality: Quality::default(),
            master: 100,
            music: 100,
            effects: 100,
            voice: 100,
            subtitles: true,
            vibration: true,
            difficulty: Difficulty::default(),
        }
    }
}

impl Settings {
    /// Множитель громкости группы: общая × группа, от 0 до 1.
    pub fn gain(&self, group: SoundGroup) -> f32 {
        let part = match group {
            SoundGroup::Music => self.music,
            SoundGroup::Effects => self.effects,
            SoundGroup::Voice => self.voice,
        };
        f32::from(self.master.min(100)) / 100.0 * f32::from(part.min(100)) / 100.0
    }

    /// Разбирает текст файла: недостающие поля — по умолчанию, громкости не больше 100 %.
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        let mut settings: Settings = serde_json::from_str(json)?;
        for volume in [&mut settings.master, &mut settings.music, &mut settings.effects, &mut settings.voice] {
            *volume = (*volume).min(100);
        }
        Ok(settings)
    }

    /// Читает настройки из файла. Нет файла — по умолчанию с качеством, которое предлагает
    /// страница браузера; испорченный файл — по умолчанию.
    pub fn load(path: Option<&Path>) -> Self {
        let Some(json) = path.and_then(|path| std::fs::read_to_string(path).ok()) else {
            let quality = crate::platform::quality_hint()
                .and_then(|name| serde_json::from_value(serde_json::Value::String(name)).ok())
                .unwrap_or_default();
            return Self { quality, ..default() };
        };
        Self::parse(&json).unwrap_or_else(|error| {
            warn!("Настройки: файл не разобран: {error}");
            Self::default()
        })
    }

    /// Записывает настройки через временный файл, чтобы прерванная запись не испортила их.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(directory) = path.parent() {
            std::fs::create_dir_all(directory)?;
        }
        let temporary = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self).expect("настройки всегда записываются в JSON");
        std::fs::write(&temporary, json)?;
        std::fs::rename(&temporary, path)
    }
}

/// Путь к файлу настроек: рядом с файлом рекордов. `None` — настройки только в памяти (браузер).
fn settings_path() -> Option<PathBuf> {
    Some(crate::records::records_path()?.parent()?.join(FILE_NAME))
}

/// Где лежит файл настроек.
#[derive(Resource)]
struct SettingsFile(Option<PathBuf>);

/// Настройки, прочитанные при запуске: плагины качества и сложности берут из них начальные
/// значения ещё при сборке приложения. Читает файл один раз.
pub fn stored(app: &mut App) -> Settings {
    if let Some(settings) = app.world().get_resource::<Settings>() {
        return settings.clone();
    }
    let path = settings_path();
    let settings = Settings::load(path.as_deref());
    app.insert_resource(settings.clone()).insert_resource(SettingsFile(path));
    settings
}

/// Плагин настроек.
pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        stored(app);
        app.add_systems(
            Update,
            save_settings
                .run_if(resource_changed::<Settings>)
                .in_set(GameSet::Presentation),
        );
    }
}

/// Записывает файл при каждом изменении настроек.
fn save_settings(settings: Res<Settings>, file: Res<SettingsFile>) {
    if settings.is_added() {
        return;
    }
    if let Some(path) = file.0.as_deref()
        && let Err(error) = settings.save(path)
    {
        warn!("Настройки: не записан {}: {error}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn прежний_файл_с_одним_качеством_читается() {
        let settings = Settings::parse(r#"{"quality":"high"}"#).unwrap();
        assert_eq!(
            settings,
            Settings {
                quality: Quality::High,
                ..Settings::default()
            }
        );
        assert_eq!(settings.gain(SoundGroup::Voice), 1.0);
        assert!(settings.vibration);
        assert!(settings.subtitles);
        assert_eq!(settings.difficulty, Difficulty::Normal);
    }

    #[test]
    fn запись_и_чтение_дают_те_же_настройки() {
        let directory = std::env::temp_dir().join(format!("space-guard-settings-{}", std::process::id()));
        let path = directory.join(FILE_NAME);
        let settings = Settings {
            quality: Quality::Low,
            master: 80,
            music: 30,
            effects: 0,
            voice: 50,
            subtitles: false,
            vibration: false,
            difficulty: Difficulty::Easy,
        };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(Some(&path)), settings);
        assert!(!path.with_extension("json.tmp").exists());
        std::fs::write(&path, "{ испорчено").unwrap();
        assert_eq!(Settings::load(Some(&path)), Settings::default());
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn громкость_общая_на_группу() {
        let settings = Settings {
            master: 50,
            music: 40,
            voice: 0,
            ..Settings::default()
        };
        assert!((settings.gain(SoundGroup::Music) - 0.2).abs() < 1e-6);
        assert_eq!(settings.gain(SoundGroup::Voice), 0.0);
        assert!((settings.gain(SoundGroup::Effects) - 0.5).abs() < 1e-6);
    }
}
