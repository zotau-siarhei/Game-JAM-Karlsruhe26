//! Мемные реплики оружия: каждый N-й выстрел мортиры и ракет игрока — повод для штаба.
//!
//! Реплики идут поводами [`VoiceCue`] через отбор штаба (`src/hq.rs`). У мем-реплик приоритет 0:
//! они не перебивают речь, мем звучит не чаще раза в 10 с, один и тот же — раз в 30 с.
//! Обычный выстрел звучит обычным звуком из `src/audio_events.rs`.
//!
//! Звуков механизма перезарядки и холостого щелчка в `assets/sfx` пока нет, поэтому под них
//! здесь кода нет. Повод «боезапас кончился» шлёт `arsenal::AmmoRanOut`.

use bevy::prelude::*;

use crate::combat::Team;
use crate::frame::GameSet;
use crate::hq::VoiceCue;
use crate::mortar::Shell;
use crate::rockets::Rocket;
use crate::run::WorldRebuilt;

/// Каждый какой снаряд мортиры игрока — повод для «Bombardiro!».
const MORTAR_MEME_EVERY: u32 = 8;

/// Каждая какая ракета игрока — повод для «Ballerina Cappuccina!».
const ROCKET_MEME_EVERY: u32 = 12;

/// Подключает мемные реплики оружия.
pub struct WeaponSfxPlugin;

impl Plugin for WeaponSfxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShotCounts>().add_systems(
            Update,
            (
                reset_counts.in_set(GameSet::Reset),
                meme_shots.in_set(GameSet::Presentation),
            ),
        );
    }
}

/// Сколько выстрелов прошло с последнего мема.
#[derive(Resource, Default)]
struct ShotCounts {
    mortar: u32,
    rockets: u32,
}

/// Мир перестроен: счёт выстрелов начинается заново.
fn reset_counts(mut rebuilt: MessageReader<WorldRebuilt>, mut counts: ResMut<ShotCounts>) {
    if rebuilt.read().last().is_some() {
        *counts = ShotCounts::default();
    }
}

/// Считает выстрел и отвечает, пора ли мему: на каждый `every`-й.
fn meme_due(count: &mut u32, every: u32) -> bool {
    *count += 1;
    if *count >= every {
        *count = 0;
        true
    } else {
        false
    }
}

/// Новые снаряды и ракеты игрока: на каждый N-й — мем-повод.
fn meme_shots(
    shells: Query<&Shell, Added<Shell>>,
    rockets: Query<&Rocket, Added<Rocket>>,
    mut counts: ResMut<ShotCounts>,
    mut cues: MessageWriter<VoiceCue>,
) {
    for shell in &shells {
        if shell.team() == Team::Player && meme_due(&mut counts.mortar, MORTAR_MEME_EVERY) {
            cues.write(VoiceCue::new("mortar_fire_meme"));
        }
    }
    for rocket in &rockets {
        if rocket.team() == Team::Player && meme_due(&mut counts.rockets, ROCKET_MEME_EVERY) {
            cues.write(VoiceCue::new("rockets_fire_meme"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meme_comes_on_every_nth_shot() {
        let mut count = 0;
        let memes: Vec<bool> = (0..16).map(|_| meme_due(&mut count, 8)).collect();
        assert_eq!(memes.iter().filter(|m| **m).count(), 2);
        assert!(memes[7] && memes[15]);
        assert!(!memes[0]);
    }
}
