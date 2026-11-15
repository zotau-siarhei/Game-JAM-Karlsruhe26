//! Какие игровые события чем озвучены: единственное место, где события превращаются
//! в звуки, поводы для реплик и смену музыки.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::airstrike::Strike;
use crate::arsenal::AmmoRanOut;
use crate::buildings::BuildingDestroyed;
use crate::bullet::FireBullet;
use crate::capsule::{Capsule, FALL_SECONDS, MARK_SECONDS};
use crate::collapse::CollapseHouse;
use crate::combat::{Damage, Died, Health, Team};
use crate::enemy::Enemy;
use crate::laser::Beam;
use crate::mortar::{Shell, FLIGHT_TIME};
use crate::pickups::Popup;
use crate::props::{Prop, PropKind};
use crate::rockets::Rocket;
use crate::waves::{Threat, WaveStarted};
use crate::explosion::Explosion;
use crate::frame::GameSet;
use crate::hq::{HqMemory, VoiceCue, HqTone};
use crate::music::{MusicState, Track};
use crate::repair::{MedkitPicked, RepairChanged};
use crate::rover::Rover;
use crate::run::{GameState, RunStarted};
use crate::sound::PlaySound;

/// Доля здоровья, ниже которой урон считается критическим.
const CRITICAL_HEALTH: f32 = 0.3;

/// Доля здоровья, выше которой критический урон может прозвучать снова.
const RECOVERED_HEALTH: f32 = 0.5;

/// Взрыв слабее этого — хлопок.
const SMALL_EXPLOSION_POWER: f32 = 0.5;

/// Взрыв мощнее этого — раскатистый грохот и повод для штаба.
const BIG_EXPLOSION_POWER: f32 = 1.5;

/// За сколько секунд до падения снаряда начинается свист: длина звука `mortar_whistle`.
const WHISTLE_LEAD: f32 = 0.95;

/// За сколько секунд до удара капсулы о землю начинается гул её падения.
const CAPSULE_FALL_LEAD: f32 = 1.2;

/// Подключает озвучку игровых событий.
pub struct AudioEventsPlugin;

impl Plugin for AudioEventsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Menu), menu_music)
            .add_systems(OnExit(GameState::Menu), click)
            .add_systems(OnExit(GameState::GameOver), click)
            .add_systems(OnEnter(GameState::Paused), pause_blip)
            .add_systems(OnExit(GameState::Paused), click)
            .add_systems(OnEnter(GameState::Paused), |mut music: ResMut<MusicState>| music.paused = true)
            .add_systems(OnExit(GameState::Paused), |mut music: ResMut<MusicState>| music.paused = false)
            .add_systems(
                Update,
                (
                    announce_run_start.in_set(GameSet::Reset),
                    // Читает компоненты погибшего врага: пока он ещё существует.
                    voice_enemies.in_set(GameSet::Death),
                ),
            )
            .add_systems(
            Update,
            (
                voice_rover_damage,
                voice_rover_death,
                sound_shots,
                sound_shells,
                sound_rockets,
                sound_special_weapons,
                sound_pickups,
                sound_hits,
                sound_explosions,
                sound_buildings,
                sound_props,
                sound_capsules,
                voice_weapon_denied,
                voice_repair,
                sound_pause_menu.run_if(in_state(GameState::Paused)),
                music_follows_enemies.run_if(in_state(GameState::Playing)),
                tone_follows_threat,
            )
                .in_set(GameSet::Presentation),
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

/// Новая волна — реплика о ней; гибель врага — звук и редкий комментарий лунохода.
fn voice_enemies(
    mut waves: MessageReader<WaveStarted>,
    mut died: MessageReader<Died>,
    enemies: Query<(), With<Enemy>>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    if waves.read().count() > 0 {
        cues.write(VoiceCue::new("wave_start"));
    }
    for message in died.read() {
        if enemies.contains(message.entity) {
            sounds.write(PlaySound::at("enemy_death", message.position));
            cues.write(VoiceCue::new("kill"));
        }
    }
}

/// Тон штаба по угрозе: вежливо, беспокойство, паника.
fn tone_for(threat: f32) -> u8 {
    match threat {
        t if t < 1.0 => 1,
        t if t < 2.5 => 2,
        _ => 3,
    }
}

/// Чем выше угроза, тем нервнее штаб.
fn tone_follows_threat(threat: Res<Threat>, mut tone: ResMut<HqTone>) {
    let wanted = tone_for(threat.value());
    if tone.0 != wanted {
        tone.0 = wanted;
    }
}

/// В меню играет своя музыка.
fn menu_music(mut music: ResMut<MusicState>) {
    music.track = Track::Menu;
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

/// Выстрел мортиры, своей или вражеской: хлопок у ствола, а перед падением — свист там,
/// куда летит снаряд. Свист снаряда, летящего в луноход, слышен громче всего.
fn sound_shells(
    time: Res<Time>,
    shells: Query<(&Transform, &Shell), Added<Shell>>,
    mut whistles: Local<Vec<(f32, Vec3)>>,
    mut sounds: MessageWriter<PlaySound>,
) {
    for (transform, shell) in &shells {
        sounds.write(PlaySound::at("mortar_fire", transform.translation));
        whistles.push(((FLIGHT_TIME - WHISTLE_LEAD).max(0.0), shell.target()));
    }
    let dt = time.delta_secs();
    whistles.retain_mut(|(delay, target)| {
        *delay -= dt;
        if *delay > 0.0 {
            return true;
        }
        sounds.write(PlaySound::at("mortar_whistle", *target));
        false
    });
}

/// Гибель объекта поверхности: дерево падает с треском, валун и кристалл раскалываются.
///
/// Валун и кристалл исчезают в том же кадре, где приходит весть о гибели, поэтому вид и место
/// каждого прочного объекта запоминаются при появлении.
fn sound_props(
    mut died: MessageReader<Died>,
    added: Query<(Entity, &Prop, &Transform), Added<Health>>,
    mut removed: RemovedComponents<Prop>,
    mut known: Local<HashMap<Entity, (PropKind, Vec3)>>,
    mut sounds: MessageWriter<PlaySound>,
) {
    for (entity, prop, transform) in &added {
        known.insert(entity, (prop.kind, transform.translation));
    }
    for death in died.read() {
        let Some((kind, position)) = known.get(&death.entity) else {
            continue;
        };
        if let Some(sound) = prop_death_sound(*kind) {
            sounds.write(PlaySound::at(sound, *position));
        }
    }
    for entity in removed.read() {
        known.remove(&entity);
    }
}

/// Звук гибели объекта поверхности. У мелких объектов своего звука нет.
fn prop_death_sound(kind: PropKind) -> Option<&'static str> {
    match kind {
        PropKind::Tree => Some("tree_fall"),
        PropKind::Boulder | PropKind::Crystal => Some("rock_break"),
        _ => None,
    }
}

/// Десантная капсула: гул падения начинается заранее и кончается ударом о землю.
fn sound_capsules(
    time: Res<Time>,
    capsules: Query<&Transform, Added<Capsule>>,
    mut falls: Local<Vec<(f32, Vec3)>>,
    mut sounds: MessageWriter<PlaySound>,
) {
    for transform in &capsules {
        falls.push(((MARK_SECONDS + FALL_SECONDS - CAPSULE_FALL_LEAD).max(0.0), transform.translation));
    }
    let dt = time.delta_secs();
    falls.retain_mut(|(delay, place)| {
        *delay -= dt;
        if *delay > 0.0 {
            return true;
        }
        sounds.write(PlaySound::at("capsule_fall", *place));
        false
    });
}

/// Пуск ракеты — шипение в месте пуска.
fn sound_rockets(rockets: Query<&Transform, Added<Rocket>>, mut sounds: MessageWriter<PlaySound>) {
    for transform in &rockets {
        sounds.write(PlaySound::at("rocket_fire", transform.translation));
    }
}

/// Импульс лазера — разряд у лунохода. Вызов авиаудара — пролёт самолёта и подтверждение штаба.
fn sound_special_weapons(
    beams: Query<(), Added<Beam>>,
    strikes: Query<(), Added<Strike>>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    for _ in &beams {
        sounds.write(PlaySound::new("laser_fire"));
    }
    for _ in &strikes {
        sounds.write(PlaySound::new("airstrike_flyby"));
        cues.write(VoiceCue::new("airstrike_call"));
    }
}

/// Подбор ящика или трофея: звон и реплика лунохода. Надпись подбора появляется ровно при подборе.
fn sound_pickups(
    popups: Query<(), Added<Popup>>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    for _ in &popups {
        sounds.write(PlaySound::new("pickup"));
        cues.write(VoiceCue::new("weapon_pickup"));
    }
}

/// Попадание игрока по врагу — лязг по броне там, где стоит враг.
fn sound_hits(
    mut damages: MessageReader<Damage>,
    enemies: Query<&Transform, With<Enemy>>,
    mut sounds: MessageWriter<PlaySound>,
) {
    for damage in damages.read() {
        if damage.source_team != Team::Player {
            continue;
        }
        if let Ok(transform) = enemies.get(damage.target) {
            sounds.write(PlaySound::at("hit", transform.translation));
        }
    }
}

/// Идентификатор звука взрыва по его мощности: хлопок, взрыв или раскатистый грохот.
fn explosion_sound(power: f32) -> &'static str {
    if power < SMALL_EXPLOSION_POWER {
        "explosion_small"
    } else if power > BIG_EXPLOSION_POWER {
        "explosion_big"
    } else {
        "explosion"
    }
}

/// Взрыв — звук по мощности в месте взрыва. Первый взрыв игрока за забег и его мощные взрывы —
/// поводы для штаба.
fn sound_explosions(
    mut started: MessageReader<RunStarted>,
    mut explosions: MessageReader<Explosion>,
    mut cratered: Local<bool>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    if started.read().count() > 0 {
        *cratered = false;
    }
    for explosion in explosions.read() {
        sounds.write(PlaySound::at(explosion_sound(explosion.power), explosion.position));
        if explosion.source_team != Team::Player {
            continue;
        }
        if !*cratered {
            *cratered = true;
            cues.write(VoiceCue::new("first_crater"));
        } else if explosion.power > BIG_EXPLOSION_POWER {
            cues.write(VoiceCue::new("big_explosion"));
        }
    }
}

/// Дом рушится — грохот там, где он стоял; разрушенный дом — повод для штаба.
fn sound_buildings(
    mut collapses: MessageReader<CollapseHouse>,
    mut destroyed: MessageReader<BuildingDestroyed>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    for collapse in collapses.read() {
        sounds.write(PlaySound::at("building_collapse", collapse.base));
    }
    if destroyed.read().count() > 0 {
        cues.write(VoiceCue::new("house_destroyed"));
    }
}

/// Боезапас кончился на последнем выстреле — штаб замечает, что стрелять нечем.
fn voice_weapon_denied(mut denied: MessageReader<AmmoRanOut>, mut cues: MessageWriter<VoiceCue>) {
    if denied.read().count() > 0 {
        cues.write(VoiceCue::new("ammo_empty"));
    }
}

/// Щелчок интерфейса: старт из меню, выход с паузы и с экрана конца забега.
fn click(mut sounds: MessageWriter<PlaySound>) {
    sounds.write(PlaySound::new("ui_click"));
}

/// Звук постановки на паузу.
fn pause_blip(mut sounds: MessageWriter<PlaySound>) {
    sounds.write(PlaySound::new("ui_pause"));
}

/// Переход по пунктам меню паузы — щелчок.
fn sound_pause_menu(keys: Res<ButtonInput<KeyCode>>, mut sounds: MessageWriter<PlaySound>) {
    let moved = keys.any_just_pressed([KeyCode::ArrowUp, KeyCode::ArrowDown, KeyCode::KeyW, KeyCode::KeyS]);
    if moved {
        sounds.write(PlaySound::new("ui_click"));
    }
}

/// Ремонт и аптечки: звук и короткие реплики лунохода.
fn voice_repair(
    mut repair: MessageReader<RepairChanged>,
    mut picked: MessageReader<MedkitPicked>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    for change in repair.read() {
        match change {
            RepairChanged::Started => {
                sounds.write(PlaySound::new("repair"));
                cues.write(VoiceCue::new("repair_start"));
            }
            RepairChanged::Done => {
                cues.write(VoiceCue::new("repair_done"));
            }
        }
    }
    for medkit in picked.read() {
        sounds.write(PlaySound::at("pickup", medkit.position));
        cues.write(VoiceCue::new("medkit"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hq_tone_rises_with_threat() {
        assert_eq!(tone_for(0.0), 1);
        assert_eq!(tone_for(1.5), 2);
        assert_eq!(tone_for(4.0), 3);
    }

    #[test]
    fn large_props_have_death_sounds() {
        assert_eq!(prop_death_sound(PropKind::Tree), Some("tree_fall"));
        assert_eq!(prop_death_sound(PropKind::Boulder), Some("rock_break"));
        assert_eq!(prop_death_sound(PropKind::Crystal), Some("rock_break"));
        assert_eq!(prop_death_sound(PropKind::Fence), None);
    }

    #[test]
    fn explosion_sound_matches_power() {
        assert_eq!(explosion_sound(0.25), "explosion_small");
        assert_eq!(explosion_sound(1.0), "explosion");
        assert_eq!(explosion_sound(2.5), "explosion_big");
    }
}
