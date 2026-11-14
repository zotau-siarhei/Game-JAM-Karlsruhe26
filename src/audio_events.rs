//! Какие игровые события чем озвучены: единственное место, где события превращаются
//! в звуки, поводы для реплик и смену музыки.

use bevy::prelude::*;

use crate::bullet::FireBullet;
use crate::combat::{Damage, Died, Health};
use crate::enemy::{Enemy, EnemySpawned};
use crate::explosion::Explosion;
use crate::hq::{HqMemory, VoiceCue};
use crate::music::{MusicState, Track};
use crate::rover::Rover;
use crate::run::RunStarted;
use crate::sound::PlaySound;

/// Доля здоровья, ниже которой урон считается критическим.
const CRITICAL_HEALTH: f32 = 0.3;

/// Доля здоровья, выше которой критический урон может прозвучать снова.
const RECOVERED_HEALTH: f32 = 0.5;

/// Подключает озвучку игровых событий.
pub struct AudioEventsPlugin;

impl Plugin for AudioEventsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                announce_run_start,
                voice_rover_damage,
                voice_rover_death,
                voice_enemies,
                sound_shots,
                sound_explosions,
                music_follows_enemies,
            ),
        );
    }
}

/// Новый забег: штаб забывает сказанное, приветствует экипаж, звучит музыка передышки.
fn announce_run_start(
    mut started: MessageReader<RunStarted>,
    mut memory: ResMut<HqMemory>,
    mut cues: MessageWriter<VoiceCue>,
    mut music: ResMut<MusicState>,
) {
    if started.read().count() == 0 {
        return;
    }
    memory.reset();
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

/// Первые враги — реплика о волне; гибель врага — звук и редкий комментарий лунохода.
fn voice_enemies(
    mut started: MessageReader<RunStarted>,
    mut spawned: MessageReader<EnemySpawned>,
    mut died: MessageReader<Died>,
    enemies: Query<&Transform, With<Enemy>>,
    mut announced: Local<bool>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    if started.read().count() > 0 {
        *announced = false;
    }
    if spawned.read().count() > 0 && !*announced {
        *announced = true;
        cues.write(VoiceCue::new("wave_start"));
    }
    for message in died.read() {
        if let Ok(transform) = enemies.get(message.entity) {
            sounds.write(PlaySound::at("enemy_death", transform.translation));
            cues.write(VoiceCue::new("kill"));
        }
    }
}

/// Пока рядом есть враги, играет боевая музыка, иначе музыка передышки.
fn music_follows_enemies(
    enemies: Query<(), With<Enemy>>,
    rover: Query<&Health, With<Rover>>,
    mut music: ResMut<MusicState>,
) {
    let alive = rover.iter().any(|health| !health.is_dead());
    let wanted = match (alive, enemies.is_empty(), music.track) {
        (false, _, _) | (_, _, Track::Silence | Track::GameOver) => return,
        (true, false, _) => Track::Combat,
        (true, true, _) => Track::Lull,
    };
    if music.track != wanted {
        music.track = wanted;
    }
}

/// Выстрел пулей — звук пулемёта в месте выстрела.
fn sound_shots(mut shots: MessageReader<FireBullet>, mut sounds: MessageWriter<PlaySound>) {
    for shot in shots.read() {
        sounds.write(PlaySound::at("mg_fire", shot.origin));
    }
}

/// Идентификатор звука взрыва по его мощности: хлопок, взрыв или раскатистый грохот.
fn explosion_sound(power: f32) -> &'static str {
    if power < 0.5 {
        "explosion_small"
    } else if power > 1.5 {
        "explosion_big"
    } else {
        "explosion"
    }
}

/// Взрыв — звук по мощности в месте взрыва.
fn sound_explosions(mut explosions: MessageReader<Explosion>, mut sounds: MessageWriter<PlaySound>) {
    for explosion in explosions.read() {
        sounds.write(PlaySound::at(explosion_sound(explosion.power), explosion.position));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explosion_sound_matches_power() {
        assert_eq!(explosion_sound(0.25), "explosion_small");
        assert_eq!(explosion_sound(1.0), "explosion");
        assert_eq!(explosion_sound(2.5), "explosion_big");
    }
}
