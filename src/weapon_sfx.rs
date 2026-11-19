//! Мемные реплики оружия: каждый N-й выстрел мортиры и ракет игрока — повод для штаба.
//!
//! Реплики идут поводами [`VoiceCue`] через отбор штаба (`src/hq.rs`). У мем-реплик приоритет 0:
//! они не перебивают речь, мем звучит не чаще раза в 10 с, один и тот же — раз в 30 с. Мем, который
//! штаб не пропустил, не теряется: повод повторяется на каждом следующем выстреле, пока реплика
//! не прозвучит, и только потом счёт выстрелов начинается заново.
//! Обычный выстрел звучит обычным звуком из `src/audio_events.rs`.
//!
//! Перезарядки в игре нет: у пулемёта запас бесконечный. Поэтому после долгой очереди
//! пулемёта, когда игрок отпустил спуск, звучит лязг `reload` и «Six... seven.» (`mg_reload`).
//! Холостой щелчок `dry_fire` на последнем заряде шлёт `src/audio_events.rs` по
//! `arsenal::AmmoRanOut`.

use bevy::prelude::*;

use crate::bullet::FireBullet;
use crate::combat::Team;
use crate::frame::GameSet;
use crate::hq::VoiceCue;
use crate::voice::PlayVoice;
use crate::mortar::Shell;
use crate::rockets::Rocket;
use crate::run::WorldRebuilt;
use crate::sound::PlaySound;

/// Каждый какой снаряд мортиры игрока — повод для «Bombardiro!».
const MORTAR_MEME_EVERY: u32 = 8;

/// Каждая какая ракета игрока — повод для «Ballerina Cappuccina!».
const ROCKET_MEME_EVERY: u32 = 12;

/// Очередь пулемёта не короче стольких пуль заканчивается перезарядкой.
const RELOAD_AFTER_BULLETS: u32 = 40;

/// Пауза в стрельбе, после которой очередь считается законченной, с.
const BURST_GAP: f32 = 0.4;

/// Подключает мемные реплики оружия.
pub struct WeaponSfxPlugin;

impl Plugin for WeaponSfxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShotCounts>().add_systems(
            Update,
            (
                reset_counts.in_set(GameSet::Reset),
                (meme_shots, reload_after_burst).in_set(GameSet::Presentation),
            ),
        );
    }
}

/// Сколько выстрелов прошло с последнего мема.
#[derive(Resource, Default)]
struct ShotCounts {
    mortar: u32,
    rockets: u32,
    /// Мем заслужен, но ещё не прозвучал: повод повторяется на каждом выстреле.
    mortar_due: bool,
    rockets_due: bool,
    /// Пуль в текущей очереди пулемёта и секунд с последней.
    burst: u32,
    since_bullet: f32,
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

/// Выстрел к мему: считает выстрелы, пока мем не заслужен, и отвечает, слать ли повод сейчас.
fn meme_shot(count: &mut u32, due: &mut bool, every: u32) -> bool {
    if !*due {
        *due = meme_due(count, every);
    }
    *due
}

/// Новые снаряды и ракеты игрока: на каждый N-й — мем-повод, и дальше на каждом, пока не прозвучит.
fn meme_shots(
    shells: Query<&Shell, Added<Shell>>,
    rockets: Query<&Rocket, Added<Rocket>>,
    mut played: MessageReader<PlayVoice>,
    mut counts: ResMut<ShotCounts>,
    mut cues: MessageWriter<VoiceCue>,
) {
    for PlayVoice(id) in played.read() {
        if id.starts_with("mortar_fire_meme") {
            counts.mortar_due = false;
        }
        if id.starts_with("rockets_fire_meme") {
            counts.rockets_due = false;
        }
    }
    let counts = &mut *counts;
    for shell in &shells {
        if shell.team() == Team::Player
            && meme_shot(&mut counts.mortar, &mut counts.mortar_due, MORTAR_MEME_EVERY)
        {
            cues.write(VoiceCue::new("mortar_fire_meme"));
        }
    }
    for rocket in &rockets {
        if rocket.team() == Team::Player
            && meme_shot(&mut counts.rockets, &mut counts.rockets_due, ROCKET_MEME_EVERY)
        {
            cues.write(VoiceCue::new("rockets_fire_meme"));
        }
    }
}

/// Очередь кончилась, если пуль не было `BURST_GAP` секунд. Длинная очередь — повод для перезарядки.
fn burst_ended(burst: &mut u32, since_bullet: f32) -> bool {
    if since_bullet < BURST_GAP || *burst == 0 {
        return false;
    }
    let long = *burst >= RELOAD_AFTER_BULLETS;
    *burst = 0;
    long
}

/// Конец длинной очереди пулемёта игрока: лязг перезарядки и «Six... seven.».
fn reload_after_burst(
    time: Res<Time>,
    mut bullets: MessageReader<FireBullet>,
    mut counts: ResMut<ShotCounts>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    let fired = bullets.read().filter(|b| b.team == Team::Player).count() as u32;
    if fired > 0 {
        counts.burst += fired;
        counts.since_bullet = 0.0;
        return;
    }
    counts.since_bullet += time.delta_secs();
    let since = counts.since_bullet;
    if burst_ended(&mut counts.burst, since) {
        sounds.write(PlaySound::new("reload"));
        cues.write(VoiceCue::new("mg_reload"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn незвучавший_мем_повторяется_до_реплики() {
        let (mut count, mut due) = (0, false);
        let shots: Vec<bool> = (0..10).map(|_| meme_shot(&mut count, &mut due, 8)).collect();
        // С 8-го выстрела повод идёт на каждом, пока штаб не сказал реплику.
        assert_eq!(shots.iter().filter(|s| **s).count(), 3);
        assert!(!shots[6] && shots[7] && shots[9]);
        due = false;
        assert!(!meme_shot(&mut count, &mut due, 8));
    }

    #[test]
    fn meme_comes_on_every_nth_shot() {
        let mut count = 0;
        let memes: Vec<bool> = (0..16).map(|_| meme_due(&mut count, 8)).collect();
        assert_eq!(memes.iter().filter(|m| **m).count(), 2);
        assert!(memes[7] && memes[15]);
        assert!(!memes[0]);
    }

    #[test]
    fn перезарядка_только_после_длинной_очереди() {
        let mut burst = RELOAD_AFTER_BULLETS;
        assert!(!burst_ended(&mut burst, BURST_GAP * 0.5));
        assert!(burst_ended(&mut burst, BURST_GAP));
        assert_eq!(burst, 0);
        let mut short = 5;
        assert!(!burst_ended(&mut short, BURST_GAP));
        assert_eq!(short, 0);
    }
}
