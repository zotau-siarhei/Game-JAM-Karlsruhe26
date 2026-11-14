//! Какие игровые события чем озвучены: единственное место, где события превращаются
//! в звуки, поводы для реплик и смену музыки.

use bevy::prelude::*;

use crate::combat::{Damage, Died, Health};
use crate::hq::VoiceCue;
use crate::music::{MusicState, Track};
use crate::rover::Rover;
use crate::sound::PlaySound;

/// Через сколько секунд после запуска звучит приветствие штаба.
const RUN_START_DELAY: f32 = 1.5;

/// Доля здоровья, ниже которой урон считается критическим.
const CRITICAL_HEALTH: f32 = 0.3;

/// Доля здоровья, выше которой критический урон может прозвучать снова.
const RECOVERED_HEALTH: f32 = 0.5;

/// Подключает озвучку игровых событий.
pub struct AudioEventsPlugin;

impl Plugin for AudioEventsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (announce_run_start, voice_rover_damage, voice_rover_death));
    }
}

/// Приветствие штаба и музыка забега вскоре после запуска.
fn announce_run_start(
    time: Res<Time>,
    mut done: Local<bool>,
    mut cues: MessageWriter<VoiceCue>,
    mut music: ResMut<MusicState>,
) {
    if *done || time.elapsed_secs() < RUN_START_DELAY {
        return;
    }
    *done = true;
    cues.write(VoiceCue::new("run_start"));
    music.track = Track::Lull;
}

/// Попадание по луноходу: звук удара, реплика, а при падении здоровья ниже порога — вопль.
fn voice_rover_damage(
    mut damages: MessageReader<Damage>,
    rover: Query<(Entity, &Health), With<Rover>>,
    mut critical: Local<bool>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    let Some((rover, health)) = rover.iter().next() else {
        damages.clear();
        return;
    };
    if health.fraction() > RECOVERED_HEALTH {
        *critical = false;
    }
    if !damages.read().any(|damage| damage.target == rover) || health.is_dead() {
        return;
    }
    sounds.write(PlaySound::new("rover_hit"));
    if health.fraction() < CRITICAL_HEALTH && !*critical {
        *critical = true;
        cues.write(VoiceCue::new("crit"));
    } else {
        cues.write(VoiceCue::new("hit"));
    }
}

/// Гибель лунохода: реплика и музыка конца забега.
fn voice_rover_death(
    mut died: MessageReader<Died>,
    rover: Query<(), With<Rover>>,
    mut cues: MessageWriter<VoiceCue>,
    mut music: ResMut<MusicState>,
) {
    if died.read().any(|message| rover.contains(message.entity)) {
        cues.write(VoiceCue::new("death"));
        music.track = Track::GameOver;
    }
}
