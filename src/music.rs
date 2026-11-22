//! Музыка по состояниям игры.
//!
//! Список треков лежит в `assets/music/tracks.json`, файлы — рядом с ним. Чтобы сменить музыку,
//! поменяй [`MusicState`]:
//!
//! ```ignore
//! fn on_wave_start(mut music: ResMut<MusicState>) {
//!     music.track = Track::Combat;
//! }
//! ```
//!
//! Треки сменяются перекрёстным затуханием. Переход к [`Track::GameOver`] начинается коротким
//! акцентом `death_sting`. Пока звучит реплика, музыка тише.

use bevy::prelude::*;

use crate::settings::{Settings, SoundGroup};
use crate::sound_list::SoundList;
use crate::voice::VoicePlayback;

/// Каталог музыки внутри `assets`.
const MUSIC_DIR: &str = "music";

/// Файл со списком треков внутри [`MUSIC_DIR`].
const LIST_FILE: &str = "tracks.json";

/// Идентификатор акцента, которым начинается конец забега.
const DEATH_STING: &str = "death_sting";

/// Громкость музыки без приглушения.
const BASE_VOLUME: f32 = 0.6;

/// Дольше этого акцент гибели не ждём, с: сам он звучит 2 с. Без звукового устройства его
/// сущность никто не удалит, а музыка конца забега должна начаться.
const STING_SECONDS: f64 = 2.5;

/// Время перекрёстного затухания в секундах.
const CROSSFADE_SECONDS: f32 = 2.0;

/// Доля громкости, пока звучит реплика.
const VOICE_DUCK: f32 = 0.35;

/// Доля громкости на паузе.
const PAUSE_DUCK: f32 = 0.5;

/// Как быстро музыка приглушается и возвращается, долей громкости в секунду.
const DUCK_SPEED: f32 = 3.0;

/// Подключает музыку.
pub struct MusicPlugin;

impl Plugin for MusicPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MusicState>()
            .init_resource::<MusicPlayer>()
            .add_systems(Startup, load_tracks)
            .add_systems(
                Update,
                (switch_track, fade_layers).chain().in_set(crate::frame::GameSet::Presentation),
            );
    }
}

/// Что должно звучать.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Track {
    /// Тишина.
    #[default]
    Silence,
    /// Стартовый экран.
    Menu,
    /// Передышка между волнами.
    Lull,
    /// Бой.
    Combat,
    /// Босс.
    Boss,
    /// Экран конца забега. Переход начинается акцентом гибели.
    GameOver,
}

impl Track {
    /// Идентификатор записи в `tracks.json`.
    fn id(self) -> Option<&'static str> {
        match self {
            Track::Silence => None,
            Track::Menu => Some("menu"),
            Track::Lull => Some("lull"),
            Track::Combat => Some("combat"),
            Track::Boss => Some("boss"),
            Track::GameOver => Some("game_over"),
        }
    }
}

/// Желаемая музыка. Меняется игрой, плагин подстраивает звучание.
#[derive(Resource, Default)]
pub struct MusicState {
    /// Трек, который должен звучать.
    pub track: Track,
    /// Игра на паузе: музыка звучит тише.
    pub paused: bool,
}

/// Состояние проигрывателя.
#[derive(Resource)]
struct MusicPlayer {
    tracks: SoundList,
    /// Трек, на который проигрыватель уже переключился.
    current: Track,
    /// Сущность акцента гибели, после которого начнётся музыка конца забега, и когда он
    /// начался по реальному времени.
    sting: Option<(Entity, f64)>,
    /// Текущая доля громкости с учётом реплик и паузы.
    duck: f32,
}

impl Default for MusicPlayer {
    fn default() -> Self {
        Self {
            tracks: SoundList::default(),
            current: Track::Silence,
            sting: None,
            duck: 1.0,
        }
    }
}

/// Слой музыки: звучащий трек с собственным уровнем затухания.
#[derive(Component)]
struct MusicLayer {
    /// Уровень от 0 до 1.
    level: f32,
    /// Слой затухает и будет удалён.
    fading_out: bool,
}

fn load_tracks(
    mut player: ResMut<MusicPlayer>,
    asset_server: Res<AssetServer>,
    preload: Option<ResMut<crate::loading::Preload>>,
) {
    player.tracks = SoundList::load(&asset_server, MUSIC_DIR, LIST_FILE, "Музыка");
    if let Some(mut preload) = preload {
        player.tracks.preload(&mut preload);
    }
}

/// Сдвигает `value` к `target` не больше чем на `step`.
fn approach(value: f32, target: f32, step: f32) -> f32 {
    if value < target {
        (value + step).min(target)
    } else {
        (value - step).max(target)
    }
}

/// Доля громкости музыки при реплике и на паузе.
fn duck_target(voice_playing: bool, paused: bool) -> f32 {
    let mut target = 1.0_f32;
    if voice_playing {
        target = target.min(VOICE_DUCK);
    }
    if paused {
        target = target.min(PAUSE_DUCK);
    }
    target
}

fn spawn_layer(commands: &mut Commands, player: &MusicPlayer, track: Track, level: f32, gain: f32) {
    let Some(sound) = track
        .id()
        .and_then(|id| player.tracks.get(id))
        .and_then(|loaded| loaded.sound.clone())
    else {
        return;
    };
    commands.spawn((
        MusicLayer {
            level,
            fading_out: false,
        },
        // В браузере трек играет сам браузер (`web_audio`).
        crate::web_audio::player(sound, BASE_VOLUME * level * player.duck * gain, true),
    ));
}

/// Переключает треки, когда меняется [`MusicState`].
fn switch_track(
    mut commands: Commands,
    time: Res<Time<Real>>,
    state: Res<MusicState>,
    settings: Res<Settings>,
    mut player: ResMut<MusicPlayer>,
    mut layers: Query<(Entity, &mut MusicLayer)>,
    stings: Query<(), Or<(With<AudioPlayer>, With<crate::web_audio::WebSound>)>>,
) {
    let gain = settings.gain(SoundGroup::Music);
    // Акцент гибели отзвучал или не дождались его конца: начинается музыка конца забега.
    let now = time.elapsed_secs_f64();
    if let Some((sting, started)) = player.sting
        && (!stings.contains(sting) || now - started > STING_SECONDS)
    {
        player.sting = None;
        commands.entity(sting).try_despawn();
        if player.current == Track::GameOver {
            spawn_layer(&mut commands, &player, Track::GameOver, 1.0, gain);
        }
    }

    if state.track == player.current {
        return;
    }
    player.current = state.track;
    if let Some((sting, _)) = player.sting.take() {
        commands.entity(sting).try_despawn();
    }

    let sting_sound = player
        .tracks
        .get(DEATH_STING)
        .and_then(|loaded| loaded.sound.clone());
    if state.track == Track::GameOver
        && let Some(sound) = sting_sound
    {
        // Музыка обрывается, звучит акцент, музыка конца забега начнётся после него.
        for (entity, _) in &layers {
            commands.entity(entity).despawn();
        }
        let sting = commands
            .spawn(crate::web_audio::player(sound, BASE_VOLUME * gain, false))
            .id();
        player.sting = Some((sting, now));
        return;
    }

    for (_, mut layer) in &mut layers {
        layer.fading_out = true;
    }
    spawn_layer(&mut commands, &player, state.track, 0.0, gain);
}

/// Ведёт затухание слоёв и приглушение под реплику и паузу.
fn fade_layers(
    mut commands: Commands,
    time: Res<Time<Real>>,
    state: Res<MusicState>,
    settings: Res<Settings>,
    mut player: ResMut<MusicPlayer>,
    voice: Query<(), With<VoicePlayback>>,
    mut layers: Query<(Entity, &mut MusicLayer, Option<&mut AudioSink>, Option<&mut crate::web_audio::WebSound>)>,
) {
    let delta = time.delta_secs();
    // Неслышный голос музыку не приглушает.
    let speaking = !voice.is_empty() && settings.gain(SoundGroup::Voice) > 0.0;
    let target = duck_target(speaking, state.paused);
    // Громкость из настроек: играющие слои подхватывают её в этом же кадре.
    let gain = settings.gain(SoundGroup::Music);
    player.duck = approach(player.duck, target, DUCK_SPEED * delta);

    for (entity, mut layer, sink, web) in &mut layers {
        let goal = if layer.fading_out { 0.0 } else { 1.0 };
        layer.level = approach(layer.level, goal, delta / CROSSFADE_SECONDS);
        if layer.fading_out && layer.level <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        crate::web_audio::set_volume(sink, web, BASE_VOLUME * layer.level * player.duck * gain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approach_does_not_overshoot() {
        assert_eq!(approach(0.0, 1.0, 0.25), 0.25);
        assert_eq!(approach(0.9, 1.0, 0.25), 1.0);
        assert_eq!(approach(0.1, 0.0, 0.25), 0.0);
        assert_eq!(approach(0.5, 0.5, 0.25), 0.5);
    }

    #[test]
    fn music_is_quieter_under_voice_and_on_pause() {
        assert_eq!(duck_target(false, false), 1.0);
        assert_eq!(duck_target(true, false), VOICE_DUCK);
        assert_eq!(duck_target(false, true), PAUSE_DUCK);
        assert_eq!(duck_target(true, true), VOICE_DUCK);
    }

    #[test]
    fn every_track_except_silence_has_an_id() {
        assert_eq!(Track::Silence.id(), None);
        for track in [Track::Menu, Track::Lull, Track::Combat, Track::Boss, Track::GameOver] {
            assert!(track.id().is_some());
        }
    }
}
