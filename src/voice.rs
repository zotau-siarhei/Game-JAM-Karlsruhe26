//! Голос штаба и радио лунохода: заранее озвученные реплики.
//!
//! Список реплик лежит в `assets/voice/lines.json`, звуковые файлы — рядом с ним.
//! Чтобы проиграть конкретную реплику, отправь сообщение [`PlayVoice`] с её идентификатором:
//!
//! ```ignore
//! fn on_run_start(mut voice: MessageWriter<PlayVoice>) {
//!     voice.write(PlayVoice::new("run_start"));
//! }
//! ```
//!
//! Обычно игре нужна не конкретная реплика, а повод: см. [`crate::hq::VoiceCue`].

use bevy::prelude::*;

use crate::frame::GameSet;
use crate::run::WorldRebuilt;
use crate::settings::{Settings, SoundGroup};
use crate::sound_list::{SoundEntry, SoundList};

/// Каталог реплик внутри `assets`.
const VOICE_DIR: &str = "voice";

/// Файл со списком реплик внутри [`VOICE_DIR`].
const LIST_FILE: &str = "lines.json";

/// Сколько секунд реплика без проигрывателя считается звучащей при пустом тексте. Проигрывателя
/// нет, пока грузится звук, и всегда, если нет звукового устройства.
const SILENT_BASE_SECONDS: f64 = 1.5;

/// Прибавка к этому сроку на каждый символ текста реплики, с.
const SILENT_SECONDS_PER_CHAR: f64 = 0.06;

/// Дольше этого реплика без проигрывателя не живёт, с.
const SILENT_MAX_SECONDS: f64 = 10.0;

/// Дольше этого не живёт никакая реплика: самая длинная звучит 10 с.
const LINE_MAX_SECONDS: f64 = 15.0;

/// Подключает загрузку реплик и их воспроизведение по сообщению [`PlayVoice`].
pub struct VoicePlugin;

impl Plugin for VoicePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VoiceLines>()
            .add_message::<PlayVoice>()
            .add_systems(Startup, load_voice_lines)
            .add_systems(Update, silence_on_rebuild.in_set(GameSet::Reset))
            .add_systems(
                Update,
                (play_voice, expire_lines, follow_volume)
                    .chain()
                    // Пока идёт загрузка, штаб молчит (src/loading.rs).
                    .run_if(not(in_state(crate::run::GameState::Loading)))
                    .in_set(GameSet::Presentation),
            );
    }
}

/// Просьба проиграть реплику с указанным идентификатором.
///
/// Новая реплика обрывает ту, что звучит сейчас. Если реплики нет в списке или она
/// ещё не озвучена, в лог пишется предупреждение, игра продолжается.
#[derive(Message, Debug, Clone)]
pub struct PlayVoice(pub String);

impl PlayVoice {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// Все реплики из `lines.json` и их загруженные звуки.
#[derive(Resource, Default)]
pub struct VoiceLines {
    list: SoundList,
}

impl VoiceLines {
    /// Реплика по идентификатору.
    pub fn get(&self, id: &str) -> Option<&SoundEntry> {
        self.list.get(id).map(|loaded| &loaded.entry)
    }

    /// Озвученные реплики повода: основа и её варианты.
    pub fn group<'a>(&'a self, base: &'a str) -> impl Iterator<Item = &'a SoundEntry> + 'a {
        self.list.group(base).map(|loaded| &loaded.entry)
    }
}

/// Метка сущности, которая сейчас проигрывает реплику. Пока такая сущность есть, штаб говорит.
///
/// Обычно сущность удаляет звуковая подсистема, когда звук кончился. Без звукового устройства
/// проигрыватель не появляется, и реплику удаляет [`expire_lines`] по сроку из длины текста.
#[derive(Component)]
pub struct VoicePlayback {
    /// Идентификатор звучащей реплики.
    pub id: String,
    /// Когда реплика отправлена, по реальному времени.
    started: f64,
    /// Сколько секунд реплика без проигрывателя считается звучащей.
    silent_seconds: f64,
}

/// Сколько секунд считать звучащей реплику с текстом `text`, если проигрывателя нет.
fn silent_seconds(text: &str) -> f64 {
    (SILENT_BASE_SECONDS + SILENT_SECONDS_PER_CHAR * text.chars().count() as f64)
        .min(SILENT_MAX_SECONDS)
}

/// Кончилась ли реплика по сроку: без проигрывателя — по длине текста, с ним — только по
/// общему пределу, остальное решает звуковая подсистема.
fn line_expired(age: f64, silent_seconds: f64, has_sink: bool) -> bool {
    age > LINE_MAX_SECONDS || (!has_sink && age > silent_seconds)
}

/// Читает список реплик и ставит на загрузку те звуки, файлы которых уже есть.
fn load_voice_lines(
    mut voice: ResMut<VoiceLines>,
    asset_server: Res<AssetServer>,
    preload: Option<ResMut<crate::loading::Preload>>,
) {
    voice.list = SoundList::load(&asset_server, VOICE_DIR, LIST_FILE, "Голос штаба");
    if let Some(mut preload) = preload {
        voice.list.preload(&mut preload);
    }
}

/// Проигрывает запрошенную реплику. Из нескольких запросов за кадр звучит последний.
fn play_voice(
    mut commands: Commands,
    mut requests: MessageReader<PlayVoice>,
    voice: Res<VoiceLines>,
    settings: Res<Settings>,
    time: Res<Time<Real>>,
    playing: Query<Entity, With<VoicePlayback>>,
) {
    let Some(PlayVoice(id)) = requests.read().last() else {
        return;
    };
    let Some(loaded) = voice.list.get(id) else {
        warn!("Голос штаба: реплики «{id}» нет в {LIST_FILE}");
        return;
    };
    let Some(sound) = &loaded.sound else {
        warn!("Голос штаба: реплика «{id}» ещё не озвучена");
        return;
    };

    for entity in &playing {
        commands.entity(entity).despawn();
    }
    info!("Штаб ({}): {}", loaded.entry.event, loaded.entry.text);
    commands.spawn((
        VoicePlayback {
            id: id.clone(),
            started: time.elapsed_secs_f64(),
            silent_seconds: silent_seconds(&loaded.entry.text),
        },
        AudioPlayer::new(sound.clone()),
        // При нулевой громкости реплика всё равно идёт: субтитры держатся, пока она звучит.
        PlaybackSettings {
            volume: bevy::audio::Volume::Linear(settings.gain(SoundGroup::Voice)),
            ..PlaybackSettings::DESPAWN
        },
    ));
}

/// Громкость голоса из настроек меняется и у звучащей реплики.
fn follow_volume(settings: Res<Settings>, mut playing: Query<&mut AudioSink, With<VoicePlayback>>) {
    if !settings.is_changed() {
        return;
    }
    for mut sink in &mut playing {
        sink.set_volume(bevy::audio::Volume::Linear(settings.gain(SoundGroup::Voice)));
    }
}

/// Убирает реплики, которые звуковая подсистема не удалит: без устройства, с незагрузившимся звуком.
fn expire_lines(
    mut commands: Commands,
    time: Res<Time<Real>>,
    playing: Query<(Entity, &VoicePlayback, Has<AudioSink>)>,
) {
    let now = time.elapsed_secs_f64();
    for (entity, playback, has_sink) in &playing {
        if line_expired(now - playback.started, playback.silent_seconds, has_sink) {
            commands.entity(entity).despawn();
        }
    }
}

/// Мир перестроен: реплика прошлого забега обрывается, чтобы не заглушить приветствие нового.
fn silence_on_rebuild(
    mut commands: Commands,
    mut rebuilt: MessageReader<WorldRebuilt>,
    playing: Query<Entity, With<VoicePlayback>>,
) {
    if rebuilt.read().last().is_none() {
        return;
    }
    for entity in &playing {
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_without_device_ends_by_text_length() {
        let short = silent_seconds("Bruh.");
        let long = silent_seconds("Every house you flatten raises the alarm. The residents have opinions.");
        assert!(short >= SILENT_BASE_SECONDS && short < long);
        assert!(silent_seconds(&"a".repeat(1000)) <= SILENT_MAX_SECONDS);
        assert!(!line_expired(short - 0.1, short, false));
        assert!(line_expired(short + 0.1, short, false));
    }

    #[test]
    fn playing_line_is_left_to_the_audio_system() {
        assert!(!line_expired(SILENT_MAX_SECONDS + 1.0, 2.0, true));
        assert!(line_expired(LINE_MAX_SECONDS + 0.1, 2.0, true));
    }
}
