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

use crate::sound_list::{SoundEntry, SoundList};

/// Каталог реплик внутри `assets`.
const VOICE_DIR: &str = "voice";

/// Файл со списком реплик внутри [`VOICE_DIR`].
const LIST_FILE: &str = "lines.json";

/// Подключает загрузку реплик и их воспроизведение по сообщению [`PlayVoice`].
pub struct VoicePlugin;

impl Plugin for VoicePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VoiceLines>()
            .add_message::<PlayVoice>()
            .add_systems(Startup, load_voice_lines)
            .add_systems(Update, play_voice.in_set(crate::frame::GameSet::Presentation));
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
#[derive(Component)]
pub struct VoicePlayback {
    /// Идентификатор звучащей реплики.
    pub id: String,
}

/// Читает список реплик и ставит на загрузку те звуки, файлы которых уже есть.
fn load_voice_lines(mut voice: ResMut<VoiceLines>, asset_server: Res<AssetServer>) {
    voice.list = SoundList::load(&asset_server, VOICE_DIR, LIST_FILE, "Голос штаба");
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
        VoicePlayback { id: id.clone() },
        AudioPlayer::new(sound.clone()),
        PlaybackSettings::DESPAWN,
    ));
}
