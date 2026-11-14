//! Голос штаба: заранее озвученные реплики.
//!
//! Список реплик лежит в `assets/voice/lines.json`, звуковые файлы — рядом с ним.
//! Чтобы проиграть реплику, отправь сообщение [`PlayVoice`] с её идентификатором:
//!
//! ```ignore
//! fn on_run_start(mut voice: MessageWriter<PlayVoice>) {
//!     voice.write(PlayVoice::new("run_start"));
//! }
//! ```

use std::collections::HashMap;

use bevy::asset::io::file::FileAssetReader;
use bevy::prelude::*;
use serde::Deserialize;

/// Каталог реплик внутри `assets`.
const VOICE_DIR: &str = "voice";

/// Файл со списком реплик внутри [`VOICE_DIR`].
const MANIFEST_FILE: &str = "lines.json";

/// Подключает загрузку реплик и их воспроизведение по сообщению [`PlayVoice`].
pub struct VoicePlugin;

impl Plugin for VoicePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VoiceLines>()
            .add_message::<PlayVoice>()
            .add_systems(Startup, load_voice_lines)
            .add_systems(Update, play_voice);
    }
}

/// Одна реплика из `lines.json`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct VoiceLine {
    /// Идентификатор, по которому код игры вызывает реплику.
    pub id: String,
    /// Описание события, на которое звучит реплика.
    #[serde(default)]
    pub event: String,
    /// Текст реплики.
    #[serde(default)]
    pub text: String,
    /// Имя звукового файла в каталоге реплик. Пустое, пока реплика не озвучена.
    #[serde(default)]
    pub file: String,
}

/// Содержимое `lines.json`.
#[derive(Deserialize)]
struct Manifest {
    lines: Vec<VoiceLine>,
}

/// Просьба проиграть реплику с указанным идентификатором.
///
/// Новая реплика обрывает ту, что звучит сейчас. Если реплики нет в списке или она
/// ещё не озвучена, в лог пишется предупреждение, игра продолжается.
#[derive(Message, Debug, Clone)]
pub struct PlayVoice(pub String);

impl PlayVoice {
    // Триггеров реплик в игре пока нет, поэтому конструктор ещё никто не вызывает.
    #[allow(dead_code)]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// Все реплики из `lines.json` и их загруженные звуки.
#[derive(Resource, Default)]
pub struct VoiceLines {
    lines: HashMap<String, LoadedLine>,
}

struct LoadedLine {
    line: VoiceLine,
    /// `None`, если у реплики ещё нет звукового файла.
    sound: Option<Handle<AudioSource>>,
}

/// Метка сущности, которая сейчас проигрывает реплику.
#[derive(Component)]
struct VoicePlayback;

/// Разбирает текст `lines.json`.
fn parse_manifest(json: &str) -> Result<Vec<VoiceLine>, serde_json::Error> {
    serde_json::from_str::<Manifest>(json).map(|manifest| manifest.lines)
}

/// Читает список реплик и ставит на загрузку те звуки, файлы которых уже есть.
fn load_voice_lines(mut voice: ResMut<VoiceLines>, asset_server: Res<AssetServer>) {
    let dir = FileAssetReader::get_base_path()
        .join("assets")
        .join(VOICE_DIR);
    let manifest_path = dir.join(MANIFEST_FILE);

    let json = match std::fs::read_to_string(&manifest_path) {
        Ok(json) => json,
        Err(error) => {
            warn!(
                "Голос штаба: не прочитан {}: {error}",
                manifest_path.display()
            );
            return;
        }
    };
    let lines = match parse_manifest(&json) {
        Ok(lines) => lines,
        Err(error) => {
            warn!(
                "Голос штаба: ошибка в {}: {error}",
                manifest_path.display()
            );
            return;
        }
    };

    let mut silent = Vec::new();
    for line in lines {
        let sound = if !line.file.is_empty() && dir.join(&line.file).is_file() {
            Some(asset_server.load(format!("{VOICE_DIR}/{}", line.file)))
        } else {
            silent.push(line.id.clone());
            None
        };
        let id = line.id.clone();
        if voice.lines.insert(id.clone(), LoadedLine { line, sound }).is_some() {
            warn!("Голос штаба: идентификатор «{id}» повторяется, взята последняя реплика");
        }
    }

    info!(
        "Голос штаба: реплик {}, со звуком {}",
        voice.lines.len(),
        voice.lines.values().filter(|l| l.sound.is_some()).count()
    );
    if !silent.is_empty() {
        warn!("Голос штаба: нет звуковых файлов для реплик: {}", silent.join(", "));
    }
}

/// Проигрывает запрошенную реплику. Из нескольких запросов за кадр звучит последний.
fn play_voice(
    mut commands: Commands,
    mut requests: MessageReader<PlayVoice>,
    voice: Res<VoiceLines>,
    playing: Query<Entity, With<VoicePlayback>>,
) {
    let Some(PlayVoice(id)) = requests.read().last() else {
        return;
    };
    let Some(loaded) = voice.lines.get(id) else {
        warn!("Голос штаба: реплики «{id}» нет в {MANIFEST_FILE}");
        return;
    };
    let Some(sound) = &loaded.sound else {
        warn!("Голос штаба: реплика «{id}» ещё не озвучена");
        return;
    };

    for entity in &playing {
        commands.entity(entity).despawn();
    }
    info!("Штаб ({}): {}", loaded.line.event, loaded.line.text);
    commands.spawn((
        VoicePlayback,
        AudioPlayer::new(sound.clone()),
        PlaybackSettings::DESPAWN,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_parsed() {
        let json = r#"{ "lines": [
            { "id": "run_start", "event": "Старт забега", "text": "Поехали!", "file": "run_start.mp3" }
        ] }"#;
        let lines = parse_manifest(json).unwrap();
        assert_eq!(
            lines,
            vec![VoiceLine {
                id: "run_start".into(),
                event: "Старт забега".into(),
                text: "Поехали!".into(),
                file: "run_start.mp3".into(),
            }]
        );
    }

    #[test]
    fn line_without_text_and_file_is_allowed() {
        let lines = parse_manifest(r#"{ "lines": [ { "id": "death" } ] }"#).unwrap();
        assert_eq!(lines[0].id, "death");
        assert!(lines[0].file.is_empty());
    }

    #[test]
    fn broken_manifest_is_an_error() {
        assert!(parse_manifest(r#"{ "lines": [ { "text": "без id" } ] }"#).is_err());
        assert!(parse_manifest("не json").is_err());
    }

    /// Файл, который лежит в репозитории, должен читаться игрой.
    #[test]
    fn repository_manifest_is_valid() {
        let lines = parse_manifest(include_str!("../assets/voice/lines.json")).unwrap();
        let mut ids: Vec<_> = lines.iter().map(|line| line.id.as_str()).collect();
        ids.sort_unstable();
        let total = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), total, "идентификаторы реплик повторяются");
    }
}
