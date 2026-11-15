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

use bevy::audio::Volume;
use bevy::prelude::*;

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
            .add_systems(Update, (switch_track, fade_layers).chain());
    }
}

/// Что должно звучать.
// `Boss` пока никто не включает: бой с боссом идёт под `Combat`.
#[allow(dead_code)]
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
    /// Сущность акцента гибели, после которого начнётся музыка конца забега.
    sting: Option<Entity>,
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

fn load_tracks(mut player: ResMut<MusicPlayer>, asset_server: Res<AssetServer>) {
    player.tracks = SoundList::load(&asset_server, MUSIC_DIR, LIST_FILE, "Музыка");
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

fn spawn_layer(commands: &mut Commands, player: &MusicPlayer, track: Track, level: f32) {
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
        AudioPlayer::new(sound),
        PlaybackSettings {
            volume: Volume::Linear(BASE_VOLUME * level * player.duck),
            ..PlaybackSettings::LOOP
        },
    ));
}

/// Переключает треки, когда меняется [`MusicState`].
fn switch_track(
    mut commands: Commands,
    state: Res<MusicState>,
    mut player: ResMut<MusicPlayer>,
    mut layers: Query<(Entity, &mut MusicLayer)>,
    stings: Query<(), With<AudioPlayer>>,
) {
    // Акцент гибели отзвучал: начинается музыка конца забега.
    if let Some(sting) = player.sting
        && !stings.contains(sting)
    {
        player.sting = None;
        if player.current == Track::GameOver {
            spawn_layer(&mut commands, &player, Track::GameOver, 1.0);
        }
    }

    if state.track == player.current {
        return;
    }
    player.current = state.track;
    player.sting = None;

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
            .spawn((
                AudioPlayer::new(sound),
                PlaybackSettings {
                    volume: Volume::Linear(BASE_VOLUME),
                    ..PlaybackSettings::DESPAWN
                },
            ))
            .id();
        player.sting = Some(sting);
        return;
    }

    for (_, mut layer) in &mut layers {
        layer.fading_out = true;
    }
    spawn_layer(&mut commands, &player, state.track, 0.0);
}

/// Ведёт затухание слоёв и приглушение под реплику и паузу.
fn fade_layers(
    mut commands: Commands,
    time: Res<Time<Real>>,
    state: Res<MusicState>,
    mut player: ResMut<MusicPlayer>,
    voice: Query<(), With<VoicePlayback>>,
    mut layers: Query<(Entity, &mut MusicLayer, Option<&mut AudioSink>)>,
) {
    let delta = time.delta_secs();
    let target = duck_target(!voice.is_empty(), state.paused);
    player.duck = approach(player.duck, target, DUCK_SPEED * delta);

    for (entity, mut layer, sink) in &mut layers {
        let goal = if layer.fading_out { 0.0 } else { 1.0 };
        layer.level = approach(layer.level, goal, delta / CROSSFADE_SECONDS);
        if layer.fading_out && layer.level <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        if let Some(mut sink) = sink {
            sink.set_volume(Volume::Linear(BASE_VOLUME * layer.level * player.duck));
        }
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
