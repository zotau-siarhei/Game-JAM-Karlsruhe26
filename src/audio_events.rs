//! Какие игровые события чем озвучены: единственное место, где события превращаются
//! в звуки, поводы для реплик и смену музыки.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::airstrike::Strike;
use crate::alarm::Alarm;
use crate::arsenal::{AmmoRanOut, WeaponDenied};
use crate::boss::{Boss, SpawnBoss};
use crate::buildings::{BreakDoor, BuildingDestroyed};
use crate::birds::BirdsTookOff;
use crate::bullet::FireBullet;
use crate::casings::CasingClink;
use crate::capsule::{Capsule, FALL_SECONDS, MARK_SECONDS};
use crate::collapse::CollapseHouse;
use crate::comic::ComicShown;
use crate::combat::{Damage, Died, Health, SurvivalTime, Team};
use crate::enemy::Enemy;
use crate::explosion::Explosion;
use crate::frame::GameSet;
use crate::hq::{HqMemory, HqTone, VoiceCue};
use crate::laser::Beam;
use crate::lava::CrustBreached;
use crate::mortar::{FLIGHT_TIME, Shell};
use crate::obstacles::Rammed;
use crate::music::{MusicState, Track};
use crate::pickups::WeaponPicked;
use crate::upgrades::WeaponUpgraded;
use crate::props::{Prop, PropKind};
use crate::puddles::WaterSplash;
use crate::records::{Records, records_path};
use crate::repair::{MedkitPicked, RepairChanged};
use crate::residents::{ResidentDied, ResidentScared};
use crate::rockets::Rocket;
use crate::rover::{Rover, RoverMotion};
use crate::run::{GameState, RunStarted, WorldRebuilt};
use crate::sound::PlaySound;
use crate::sound_list::{Dice, base_id};
use crate::trains::Car;
use crate::voice::VoicePlayback;
use crate::weapons::CurrentWeapon;
use crate::waves::WaveStarted;

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

/// Звук удара по луноходу — не чаще раза в столько секунд: лава и пулемёт бьют часто.
const ROVER_HIT_SOUND_GAP: f32 = 0.3;

/// Повод `hit` — не чаще раза в столько секунд, иначе он забивает редкие поводы.
const HIT_CUE_GAP: f32 = 12.0;

/// Вскрик испуганного жителя — не чаще раза в столько секунд: паника охватывает посёлок разом.
const SCARED_SOUND_GAP: f32 = 0.5;

/// «Пуф» гибели жителей — не больше стольких за кадр.
const POOF_PER_FRAME: usize = 2;

/// Сирена Службы звучит не дальше стольких метров от лунохода: машина появляется дальше слышимости.
const SIREN_DISTANCE: f32 = 30.0;

/// Серия убийств рвётся, если следующее убийство не случилось за столько секунд.
const STREAK_GAP: f32 = 4.0;

/// Хитмаркер серии — не чаще раза в столько секунд.
const HITMARKER_GAP: f32 = 15.0;

/// «Бонк» — только от препятствий не меньше этого радиуса (дом, валун) и не чаще раза в `BONK_GAP` с.
const BONK_RADIUS: f32 = 1.4;
const BONK_GAP: f32 = 5.0;

/// Мощные взрывы чаще этого, с, — серия: следующие звучат короткими вариантами.
const BIG_SERIES_GAP: f32 = 0.6;

/// Взрыв мощнее этого (гибель босса и лунохода) получает поверх ещё и долгий раскат.
const HUGE_EXPLOSION_POWER: f32 = 2.0;

/// Дома, рухнувшие чаще этого, с, звучат короткими вариантами обвала.
const COLLAPSE_SERIES_GAP: f32 = 2.0;

/// Вражеский снаряд мортиры ближе этого к луноходу — писк тревоги, м.
const ALERT_DISTANCE: f32 = 8.0;

/// Сколько секунд игрок не трогает управление и стоит, прежде чем штаб спросит, не уснул ли он.
const IDLE_SECONDS: f32 = 20.0;

/// Через сколько секунд после брейнрот-реплики штаб её комментирует.
const BRAINROT_REPLY_DELAY: f32 = 4.5;

/// Реплики станции брейнрота: после первой из них за забег штаб отзывается `brainrot_comment`.
const BRAINROT_LINES: [&str; 6] = [
    "airstrike_call_2",
    "mortar_fire_meme",
    "rockets_fire_meme",
    "service_arrives_2",
    "weapon_pickup_3",
    "mortar_reload",
];

/// Сколько секунд без врагов боевая музыка ещё держится, прежде чем смениться передышкой.
const LULL_DELAY: f32 = 6.0;

/// Вес событий в объёме разрушений, от которого зависит тон штаба.
const HOUSE_WEIGHT: f32 = 1.0;
const RESIDENT_WEIGHT: f32 = 0.5;
const BREACH_WEIGHT: f32 = 1.0;

/// Объём разрушений, с которого штаб беспокоится, и с которого паникует.
const WORRIED_DESTRUCTION: f32 = 3.0;
const PANIC_DESTRUCTION: f32 = 8.0;

/// Что озвучка помнит о текущем забеге. Сбрасывается по `WorldRebuilt`.
#[derive(Resource, Default)]
struct AudioMemory {
    /// Свисты снарядов мортиры: сколько секунд до начала и где.
    whistles: Vec<(f32, Vec3)>,
    /// Гул падающих капсул: сколько секунд до начала и где.
    falls: Vec<(f32, Vec3)>,
    /// Сколько секунд до следующего звука удара по луноходу.
    rover_hit_cooldown: f32,
    /// Сколько секунд до следующего повода `hit`.
    hit_cue_cooldown: f32,
    /// Критический урон уже прозвучал и здоровье с тех пор не поднялось.
    critical: bool,
    /// Сколько секунд в мире нет врагов.
    calm: f32,
    /// Самая высокая ступень тревоги за забег: о росте штаб говорит только выше неё.
    alarm_peak: u8,
    /// Объём разрушений за забег: дома, жители, прорывы коры.
    destruction: f32,
    /// Лучшее время из таблицы рекордов на начало забега.
    best_record: Option<f64>,
    /// О новом рекорде уже сказано.
    record_announced: bool,
    /// Секунд до следующего вскрика жителя.
    scared_cooldown: f32,
    /// Серия убийств игрока и секунд с последнего из них.
    streak: u32,
    since_kill: f32,
    /// Секунд до следующего хитмаркера серии и до следующего «бонка».
    hitmarker_cooldown: f32,
    bonk_cooldown: f32,
    /// Какие виды оружия уже подбирали за забег: первая встреча — своя реплика.
    picked: [bool; 6],
    /// Пролёты самолётов авиаудара: сколько секунд до начала звука.
    flybys: Vec<f32>,
    /// Секунд с последнего мощного взрыва и с последнего обвала дома.
    since_big_explosion: f32,
    since_collapse: f32,
    /// Секунд без касания управления, пока луноход стоит.
    idle: f32,
    /// В прошлом кадре враги были.
    enemies_seen: bool,
    /// Реплика, звучавшая в прошлом кадре.
    last_line: Option<String>,
    /// Отложенный ответ штаба: сколько секунд ждать и какой повод.
    reply: Option<(f32, &'static str)>,
    /// Брейнрот за забег уже прокомментирован.
    brainrot_commented: bool,
}

/// Подключает озвучку игровых событий.
pub struct AudioEventsPlugin;

impl Plugin for AudioEventsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AudioMemory>()
            .add_systems(OnEnter(GameState::Menu), menu_music)
            .add_systems(Update, sound_trains.in_set(GameSet::Presentation))
            .add_systems(Update, (sound_casings, sound_birds, sound_splashes).in_set(GameSet::Presentation))
            .add_systems(
                Update,
                (
                    sound_comics,
                    sound_denied,
                    sound_rams,
                    sound_boss_fall,
                    voice_replies,
                    voice_idle.run_if(in_state(GameState::Playing)),
                )
                    .in_set(GameSet::Presentation),
            )
            .add_systems(OnExit(GameState::Menu), click)
            .add_systems(OnExit(GameState::GameOver), click)
            .add_systems(OnEnter(GameState::Paused), pause_blip)
            .add_systems(OnExit(GameState::Paused), click)
            .add_systems(OnEnter(GameState::Paused), |mut music: ResMut<MusicState>| music.paused = true)
            .add_systems(OnExit(GameState::Paused), |mut music: ResMut<MusicState>| music.paused = false)
            .add_systems(
                Update,
                (
                    (forget_run, announce_run_start).chain().in_set(GameSet::Reset),
                    // Читает компоненты погибшего врага: пока он ещё существует.
                    (voice_enemies, sound_train_cars).in_set(GameSet::Death),
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
                sound_upgrades,
                sound_hits,
                sound_explosions,
                sound_buildings,
                sound_props,
                sound_capsules,
                voice_weapon_denied,
                voice_repair,
            )
                .in_set(GameSet::Presentation),
        )
        .add_systems(
            Update,
            (
                voice_service,
                voice_interceptors,
                sound_interceptor_hits,
                voice_boss,
                voice_alarm,
                voice_residents,
                voice_crust,
                voice_record.run_if(in_state(GameState::Playing)),
                sound_pause_menu.run_if(in_state(GameState::Paused)),
                music_follows_enemies.run_if(in_state(GameState::Playing)),
                tone_follows_destruction,
            )
                .in_set(GameSet::Presentation),
        );
    }
}

/// Мир перестроен: отложенные свисты и гул капсул, счётчики и пороги прошлого забега забыты.
fn forget_run(mut rebuilt: MessageReader<WorldRebuilt>, mut memory: ResMut<AudioMemory>) {
    if rebuilt.read().last().is_some() {
        *memory = AudioMemory::default();
    }
}

/// Новый забег: штаб забывает сказанное, приветствует экипаж, звучит музыка передышки.
/// Лучшее время читается из таблицы рекордов: его превышение — повод для штаба.
fn announce_run_start(
    mut started: MessageReader<RunStarted>,
    mut hq: ResMut<HqMemory>,
    mut memory: ResMut<AudioMemory>,
    mut cues: MessageWriter<VoiceCue>,
    mut music: ResMut<MusicState>,
) {
    if started.read().count() == 0 {
        return;
    }
    hq.reset();
    memory.best_record = records_path()
        .map(|path| Records::load(&path))
        .and_then(|table| table.best().map(|record| record.seconds));
    cues.write(VoiceCue::new("run_start"));
    music.track = Track::Lull;
}

/// Попадание по луноходу: звук удара, реплика, а при падении здоровья ниже порога — вопль.
/// Звук и реплика ограничены по частоте: урон от лавы и пулемёта идёт часто.
fn voice_rover_damage(
    time: Res<Time>,
    mut damages: MessageReader<Damage>,
    rover: Query<(Entity, &Health), With<Rover>>,
    mut memory: ResMut<AudioMemory>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    let dt = time.delta_secs();
    memory.rover_hit_cooldown = (memory.rover_hit_cooldown - dt).max(0.0);
    memory.hit_cue_cooldown = (memory.hit_cue_cooldown - dt).max(0.0);
    let Some((rover, health)) = rover.iter().next() else {
        damages.clear();
        return;
    };
    if health.fraction() > RECOVERED_HEALTH {
        memory.critical = false;
    }
    if !damages.read().any(|damage| damage.target == rover) || health.is_dead() {
        return;
    }
    if memory.rover_hit_cooldown <= 0.0 {
        memory.rover_hit_cooldown = ROVER_HIT_SOUND_GAP;
        sounds.write(PlaySound::new("rover_hit"));
    }
    if health.fraction() < CRITICAL_HEALTH && !memory.critical {
        memory.critical = true;
        cues.write(VoiceCue::new("crit"));
    } else if memory.hit_cue_cooldown <= 0.0 {
        memory.hit_cue_cooldown = HIT_CUE_GAP;
        cues.write(VoiceCue::new("hit"));
    }
}

/// Гибель лунохода: реплика и музыка конца забега.
fn voice_rover_death(
    mut died: MessageReader<Died>,
    rover: Query<(), With<Rover>>,
    mut cues: MessageWriter<VoiceCue>,
    mut music: ResMut<MusicState>,
    mut sounds: MessageWriter<PlaySound>,
) {
    if died.read().any(|message| rover.contains(message.entity)) {
        cues.write(VoiceCue::new("death"));
        sounds.write(PlaySound::new("meme_oof"));
        music.track = Track::GameOver;
    }
}

/// Повод серии убийств, если серия только что достигла порога: реплика и мем-звук.
fn streak_cue(streak: u32) -> Option<(&'static str, &'static str)> {
    match streak {
        3 => Some(("streak3", "meme_hitmarker")),
        5 => Some(("streak5", "meme_airhorn")),
        10 => Some(("streak10", "meme_airhorn")),
        _ => None,
    }
}

/// Новая волна — реплика о ней; гибель врага — звук и редкий комментарий лунохода, гибель
/// босса — важная реплика. Убийства игрока подряд, не реже раза в [`STREAK_GAP`] с, — серия:
/// на 3, 5 и 10 вместо обычного комментария звучит реплика серии с мем-звуком.
#[allow(clippy::too_many_arguments)]
fn voice_enemies(
    time: Res<Time>,
    mut waves: MessageReader<WaveStarted>,
    mut died: MessageReader<Died>,
    enemies: Query<(), With<Enemy>>,
    bosses: Query<(), With<Boss>>,
    mut memory: ResMut<AudioMemory>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    let dt = time.delta_secs();
    memory.since_kill += dt;
    memory.hitmarker_cooldown = (memory.hitmarker_cooldown - dt).max(0.0);
    if memory.since_kill > STREAK_GAP {
        memory.streak = 0;
    }
    if waves.read().count() > 0 {
        cues.write(VoiceCue::new("wave_start"));
    }
    for message in died.read() {
        if !enemies.contains(message.entity) {
            continue;
        }
        sounds.write(PlaySound::at("enemy_death", message.position));
        if bosses.contains(message.entity) {
            cues.write(VoiceCue::new("boss_dead"));
            continue;
        }
        let by_player = message.killer == Team::Player;
        if by_player {
            memory.streak += 1;
            memory.since_kill = 0.0;
        }
        match streak_cue(memory.streak).filter(|_| by_player) {
            Some((cue, meme)) => {
                cues.write(VoiceCue::new(cue));
                let hitmarker = meme == "meme_hitmarker";
                if !hitmarker || memory.hitmarker_cooldown <= 0.0 {
                    if hitmarker {
                        memory.hitmarker_cooldown = HITMARKER_GAP;
                    }
                    sounds.write(PlaySound::new(meme));
                }
            }
            None => {
                cues.write(VoiceCue::new("kill"));
            }
        }
    }
}

/// Вагон поезда уничтожен — короткий грохот там, где он стоял. Сход всего поезда звучит
/// отдельно, в [`sound_trains`].
fn sound_train_cars(mut died: MessageReader<Died>, cars: Query<(), With<Car>>, mut sounds: MessageWriter<PlaySound>) {
    for death in died.read() {
        if cars.contains(death.entity) {
            sounds.write(PlaySound::at("train_crash_short", death.position));
        }
    }
}

/// Появился босс — важная реплика.
fn voice_boss(bosses: Query<(), Added<Boss>>, mut cues: MessageWriter<VoiceCue>) {
    if !bosses.is_empty() {
        cues.write(VoiceCue::new("boss_appears"));
    }
}

/// Тревога поднялась выше, чем была в этом забеге, — штаб об этом говорит. Спад и повторный
/// рост до той же ступени не озвучиваются.
fn voice_alarm(alarm: Res<Alarm>, mut memory: ResMut<AudioMemory>, mut cues: MessageWriter<VoiceCue>) {
    let level = alarm.level();
    if level > memory.alarm_peak {
        memory.alarm_peak = level;
        cues.write(VoiceCue::new("alarm_rising"));
    }
}

/// Испуг жителя — вскрик, не чаще раза в [`SCARED_SOUND_GAP`] с. Гибель жителя — мультяшный «пуф»,
/// реплика штаба и вклад в объём разрушений.
fn voice_residents(
    time: Res<Time>,
    mut scared: MessageReader<ResidentScared>,
    mut deaths: MessageReader<ResidentDied>,
    mut memory: ResMut<AudioMemory>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    memory.scared_cooldown = (memory.scared_cooldown - time.delta_secs()).max(0.0);
    for fright in scared.read() {
        if memory.scared_cooldown <= 0.0 {
            memory.scared_cooldown = SCARED_SOUND_GAP;
            sounds.write(PlaySound::at("resident_scared", fright.position));
        }
    }
    let mut count = 0;
    for death in deaths.read() {
        if count < POOF_PER_FRAME {
            sounds.write(PlaySound::at("resident_poof", death.position));
        }
        count += 1;
    }
    if count > 0 {
        memory.destruction += RESIDENT_WEIGHT * count as f32;
        cues.write(VoiceCue::new("resident_died"));
    }
}

/// Прорыв коры: своей реплики нет, штаб отзывается репликой о мощном взрыве. Звук извержения
/// даёт взрыв, который лава шлёт вместе с прорывом.
fn voice_crust(
    mut breaches: MessageReader<CrustBreached>,
    mut memory: ResMut<AudioMemory>,
    mut music: ResMut<MusicState>,
    mut cues: MessageWriter<VoiceCue>,
) {
    let count = breaches.read().count();
    if count > 0 {
        memory.destruction += BREACH_WEIGHT * count as f32;
        cues.write(VoiceCue::new("big_explosion"));
        // Музыкальный акцент: прорыв поднимает бой, и он держится не меньше `LULL_DELAY`.
        let wanted = music_after_breach(music.track);
        if wanted != music.track {
            music.track = wanted;
        }
        memory.calm = 0.0;
    }
}

/// Музыка после прорыва коры: из передышки — бой; тема босса, меню и конец забега не меняются.
fn music_after_breach(current: Track) -> Track {
    match current {
        Track::Lull | Track::Combat => Track::Combat,
        other => other,
    }
}

/// Время забега превысило лучшее из таблицы рекордов — важная реплика, раз за забег.
/// Пока таблица пуста, рекордом считать нечего.
fn voice_record(survival: Res<SurvivalTime>, mut memory: ResMut<AudioMemory>, mut cues: MessageWriter<VoiceCue>) {
    if memory.record_announced {
        return;
    }
    if memory
        .best_record
        .is_some_and(|best| f64::from(survival.0) > best)
    {
        memory.record_announced = true;
        cues.write(VoiceCue::new("record"));
    }
}

/// Тон штаба по объёму разрушений: вежливо, беспокойство, паника.
fn tone_for(destruction: f32) -> u8 {
    match destruction {
        d if d < WORRIED_DESTRUCTION => 1,
        d if d < PANIC_DESTRUCTION => 2,
        _ => 3,
    }
}

/// Чем больше разрушено, тем нервнее штаб. Время забега на тон не влияет.
fn tone_follows_destruction(memory: Res<AudioMemory>, mut tone: ResMut<HqTone>) {
    let wanted = tone_for(memory.destruction);
    if tone.0 != wanted {
        tone.0 = wanted;
    }
}

/// В меню играет своя музыка.
fn menu_music(mut music: ResMut<MusicState>) {
    music.track = Track::Menu;
}

/// Музыка забега: пока жив босс — его тема, пока есть враги — бой. Бой сменяется передышкой
/// только после [`LULL_DELAY`] секунд без врагов, чтобы музыка не дёргалась.
fn music_for(boss_alive: bool, enemies: bool, calm: f32, current: Track) -> Track {
    if boss_alive {
        Track::Boss
    } else if enemies || (matches!(current, Track::Combat | Track::Boss) && calm < LULL_DELAY) {
        Track::Combat
    } else {
        Track::Lull
    }
}

/// Музыка следует за врагами и боссом. Когда враги кончились, луноход радостно пищит.
fn music_follows_enemies(
    time: Res<Time>,
    enemies: Query<(), With<Enemy>>,
    bosses: Query<&Health, With<Boss>>,
    rover: Query<&Health, With<Rover>>,
    mut memory: ResMut<AudioMemory>,
    mut music: ResMut<MusicState>,
    mut cues: MessageWriter<VoiceCue>,
) {
    let alive = rover.iter().any(|health| !health.is_dead());
    if !alive || matches!(music.track, Track::Silence | Track::GameOver) {
        return;
    }
    let enemies = !enemies.is_empty();
    // Последний враг пал — луноход радостно пищит.
    if memory.enemies_seen && !enemies {
        cues.write(VoiceCue::new("beep_happy"));
    }
    memory.enemies_seen = enemies;
    memory.calm = if enemies { 0.0 } else { memory.calm + time.delta_secs() };
    let boss_alive = bosses.iter().any(|health| !health.is_dead());
    let wanted = music_for(boss_alive, enemies, memory.calm, music.track);
    if music.track != wanted {
        music.track = wanted;
    }
}

/// Гильза лунохода звякнула о землю. Записи `casing_clink` в списке звуков пока нет: звук
/// молчит, пока его не сгенерируют.
fn sound_casings(mut clinks: MessageReader<CasingClink>, mut sounds: MessageWriter<PlaySound>) {
    for clink in clinks.read() {
        sounds.write(PlaySound::at("casing_clink", clink.position));
    }
}

/// Луноход въехал в воду: всплеск. Записи `splash` в списке звуков пока нет, как и у гильз.
fn sound_splashes(mut splashes: MessageReader<WaterSplash>, mut sounds: MessageWriter<PlaySound>) {
    for splash in splashes.read() {
        sounds.write(PlaySound::at("splash", splash.position));
    }
}

/// Стая птиц взлетела. Записи `birds_takeoff` в списке звуков пока нет, как и у гильз.
fn sound_birds(mut flocks: MessageReader<BirdsTookOff>, mut sounds: MessageWriter<PlaySound>) {
    for flock in flocks.read() {
        sounds.write(PlaySound::at("birds_takeoff", flock.position));
    }
}

/// Выстрел пулей — звук пулемёта в месте выстрела.
fn sound_shots(mut shots: MessageReader<FireBullet>, mut sounds: MessageWriter<PlaySound>) {
    for shot in shots.read() {
        sounds.write(PlaySound::at("mg_fire", shot.origin));
    }
}

/// Выстрел мортиры, своей или вражеской: хлопок у ствола, а перед падением — свист там,
/// куда летит снаряд. Свист снаряда, летящего в луноход, слышен громче всего. Вражеский
/// снаряд, который упадёт рядом с луноходом, — повод для писка тревоги.
fn sound_shells(
    time: Res<Time>,
    shells: Query<(&Transform, &Shell), Added<Shell>>,
    rover: Query<&Transform, With<Rover>>,
    mut memory: ResMut<AudioMemory>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    let rover = rover.iter().next().map(|t| t.translation);
    for (transform, shell) in &shells {
        sounds.write(PlaySound::at("mortar_fire", transform.translation));
        let close = rover.is_some_and(|r| r.with_y(0.0).distance(shell.target().with_y(0.0)) < ALERT_DISTANCE);
        if shell.team() != Team::Player && close {
            cues.write(VoiceCue::new("beep_alert"));
        }
        memory.whistles.push(((FLIGHT_TIME - WHISTLE_LEAD).max(0.0), shell.target()));
    }
    let dt = time.delta_secs();
    memory.whistles.retain_mut(|(delay, target)| {
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
    mut memory: ResMut<AudioMemory>,
    mut sounds: MessageWriter<PlaySound>,
) {
    for transform in &capsules {
        memory
            .falls
            .push(((MARK_SECONDS + FALL_SECONDS - CAPSULE_FALL_LEAD).max(0.0), transform.translation));
    }
    let dt = time.delta_secs();
    memory.falls.retain_mut(|(delay, place)| {
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

/// Импульс лазера — разряд у лунохода. Вызов авиаудара — подтверждение штаба, а когда самолёт
/// появляется (через [`airstrike::DELAY`](crate::airstrike::DELAY) с), — гул его пролёта.
fn sound_special_weapons(
    time: Res<Time>,
    beams: Query<(), Added<Beam>>,
    strikes: Query<(), Added<Strike>>,
    mut memory: ResMut<AudioMemory>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    for _ in &beams {
        sounds.write(PlaySound::new("laser_fire"));
    }
    for _ in &strikes {
        memory.flybys.push(crate::airstrike::DELAY);
        cues.write(VoiceCue::new("airstrike_call"));
    }
    let dt = time.delta_secs();
    memory.flybys.retain_mut(|delay| {
        *delay -= dt;
        if *delay > 0.0 {
            return true;
        }
        sounds.write(PlaySound::new("airstrike_flyby"));
        false
    });
}

/// Реплика первого за забег подбора вида оружия. У пулемёта своей нет — общий восторг.
fn first_pickup_cue(weapon: CurrentWeapon) -> &'static str {
    match weapon {
        CurrentWeapon::Mortar => "weapon_mortar",
        CurrentWeapon::Rockets | CurrentWeapon::HomingRockets => "weapon_rockets",
        CurrentWeapon::Laser => "weapon_laser",
        CurrentWeapon::Airstrike => "weapon_airstrike",
        CurrentWeapon::MachineGun => "weapon_pickup",
    }
}

/// Реплика повторного подбора: пополнение боезапаса. `roll` — случайное число от 0 до 1.
fn refill_cue(weapon: CurrentWeapon, roll: f32) -> &'static str {
    let own = match weapon {
        CurrentWeapon::Mortar => Some("mortar_reload"),
        CurrentWeapon::Rockets | CurrentWeapon::HomingRockets => Some("rockets_reload"),
        _ => None,
    };
    match (own, roll) {
        (Some(cue), r) if r < 0.4 => cue,
        (_, r) if r < 0.7 => "ammo_crate",
        _ => "weapon_pickup",
    }
}

/// Подбор ящика или трофея: звон и реплика лунохода. Первый за забег ящик вида — своя реплика
/// о нём; следующие — лязг перезарядки и реплика о пополнении.
fn sound_pickups(
    mut picked: MessageReader<WeaponPicked>,
    mut memory: ResMut<AudioMemory>,
    mut dice: ResMut<Dice>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    for pick in picked.read() {
        sounds.write(PlaySound::new("pickup"));
        let seen = &mut memory.picked[pick.weapon as usize];
        if pick.new || !*seen {
            *seen = true;
            cues.write(VoiceCue::new(first_pickup_cue(pick.weapon)));
        } else {
            sounds.write(PlaySound::new("reload"));
            cues.write(VoiceCue::new(refill_cue(pick.weapon, dice.range(0.0, 1.0))));
        }
    }
}

/// Золотой ящик: джингл улучшения и восторг механика; когда улучшать нечего и боезапас
/// пополнен до предела — лязг перезарядки и реплика о боеприпасах.
fn sound_upgrades(
    mut upgraded: MessageReader<WeaponUpgraded>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    for upgrade in upgraded.read() {
        match upgrade {
            WeaponUpgraded::Level { .. } => {
                sounds.write(PlaySound::new("upgrade"));
                cues.write(VoiceCue::new("weapon_upgrade"));
            }
            WeaponUpgraded::Refill { .. } => {
                sounds.write(PlaySound::new("upgrade"));
                sounds.write(PlaySound::new("reload"));
                cues.write(VoiceCue::new("ammo_crate"));
            }
        }
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

/// Мощный взрыв в серии (бомбы авиаудара идут подряд) звучит коротким вариантом, чтобы
/// раскаты не забивали предел группы и друг друга.
fn in_series(sound: &'static str, since_big: f32) -> &'static str {
    if sound == "explosion_big" && since_big < BIG_SERIES_GAP {
        "explosion_big_short"
    } else {
        sound
    }
}

/// Взрыв — звук по мощности в месте взрыва; под самым мощным — ещё и долгий раскат. Мощный взрыв
/// игрока — повод для штаба. Первый кратер объясняет обучение (`src/intro.rs`): там реплика
/// ждёт, пока штаб замолчит.
fn sound_explosions(
    time: Res<Time>,
    mut explosions: MessageReader<Explosion>,
    mut memory: ResMut<AudioMemory>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    memory.since_big_explosion += time.delta_secs();
    for explosion in explosions.read() {
        let sound = explosion_sound(explosion.power);
        let played = in_series(sound, memory.since_big_explosion);
        if sound == "explosion_big" {
            memory.since_big_explosion = 0.0;
        }
        sounds.write(PlaySound::at(played, explosion.position));
        if explosion.power > HUGE_EXPLOSION_POWER {
            sounds.write(PlaySound::at("explosion_long", explosion.position));
        }
        if explosion.source_team == Team::Player && explosion.power > BIG_EXPLOSION_POWER {
            cues.write(VoiceCue::new("big_explosion"));
        }
    }
}

/// Дом рушится — грохот там, где он стоял; дома, рухнувшие один за другим, звучат короткими
/// вариантами. Выбитая дверь — короткий обвал. Разрушенный дом — повод для штаба.
#[allow(clippy::too_many_arguments)]
fn sound_buildings(
    time: Res<Time>,
    mut collapses: MessageReader<CollapseHouse>,
    mut doors: MessageReader<BreakDoor>,
    mut destroyed: MessageReader<BuildingDestroyed>,
    houses: Query<&GlobalTransform>,
    mut memory: ResMut<AudioMemory>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    memory.since_collapse += time.delta_secs();
    for collapse in collapses.read() {
        let sound = if memory.since_collapse < COLLAPSE_SERIES_GAP {
            "building_collapse_short"
        } else {
            "building_collapse"
        };
        memory.since_collapse = 0.0;
        sounds.write(PlaySound::at(sound, collapse.base));
    }
    for door in doors.read() {
        if let Ok(house) = houses.get(door.house) {
            sounds.write(PlaySound::at("building_collapse_short", house.translation()));
        }
    }
    let count = destroyed.read().count();
    if count > 0 {
        memory.destruction += HOUSE_WEIGHT * count as f32;
        cues.write(VoiceCue::new("house_destroyed"));
    }
}

/// Служба защиты жителей приехала по тревоге — сирена со стороны приезда и предупреждение штаба.
/// Машина появляется дальше слышимости, поэтому сирена звучит не дальше [`SIREN_DISTANCE`] м.
fn voice_service(
    mut arrived: MessageReader<crate::service::ServiceArrived>,
    rover: Query<&Transform, With<Rover>>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    let Some(arrival) = arrived.read().last() else {
        return;
    };
    let at = rover.single().map_or(arrival.position, |r| {
        r.translation + (arrival.position - r.translation).clamp_length_max(SIREN_DISTANCE)
    });
    sounds.write(PlaySound::at("service_siren", at));
    cues.write(VoiceCue::new("service_arrives"));
}

/// Начался перехват — штаб предупреждает о перехватчиках, не чаще раза в
/// [`balance::VOICE_GAP`](crate::waves::balance::VOICE_GAP).
fn voice_interceptors(
    time: Res<Time>,
    mut started: MessageReader<crate::interceptor::InterceptStarted>,
    mut last: Local<Option<f32>>,
    mut cues: MessageWriter<VoiceCue>,
) {
    if started.read().count() == 0 {
        return;
    }
    let now = time.elapsed_secs();
    if last.is_some_and(|at| now - at < crate::waves::balance::VOICE_GAP) {
        return;
    }
    *last = Some(now);
    cues.write(VoiceCue::new("interceptors"));
}

/// Удар перехватчика по луноходу.
fn sound_interceptor_hits(
    mut hits: MessageReader<crate::interceptor::InterceptorHit>,
    mut sounds: MessageWriter<PlaySound>,
) {
    for hit in hits.read() {
        sounds.write(PlaySound::at("interceptor_hit", hit.position));
    }
}

/// Боезапас кончился на последнем выстреле — холостой щелчок, и штаб замечает, что стрелять нечем.
fn voice_weapon_denied(
    mut denied: MessageReader<AmmoRanOut>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    if denied.read().count() > 0 {
        sounds.write(PlaySound::new("dry_fire"));
        cues.write(VoiceCue::new("ammo_empty"));
    }
}

/// Мем-звук под комикс-надпись: редкость уже посчитана в `src/comic.rs`.
fn comic_sound(caption: &str) -> Option<&'static str> {
    match caption {
        "BOMBARDIRO!" | "KA-BOOM!" => Some("meme_boom"),
        "TUNG TUNG TUNG" => Some("meme_tung"),
        "WOMP WOMP" => Some("meme_womp"),
        "6-7" => Some("meme_beep"),
        _ => None,
    }
}

/// Комикс-надпись показана — под неё мем-звук.
fn sound_comics(mut shown: MessageReader<ComicShown>, mut sounds: MessageWriter<PlaySound>) {
    for comic in shown.read() {
        if let Some(sound) = comic_sound(comic.caption) {
            sounds.write(PlaySound::new(sound));
        }
    }
}

/// Игрок выбрал оружие, из которого нечем стрелять: звук ошибки и обиженный писк лунохода.
fn sound_denied(
    mut denied: MessageReader<WeaponDenied>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    if denied.read().count() > 0 {
        sounds.write(PlaySound::new("meme_error"));
        cues.write(VoiceCue::new("beep_sad"));
    }
}

/// Луноход снёс корпусом крупное препятствие — мультяшный «бонк», не чаще раза в [`BONK_GAP`] с.
fn sound_rams(
    time: Res<Time>,
    mut rammed: MessageReader<Rammed>,
    mut memory: ResMut<AudioMemory>,
    mut sounds: MessageWriter<PlaySound>,
) {
    memory.bonk_cooldown = (memory.bonk_cooldown - time.delta_secs()).max(0.0);
    for ram in rammed.read() {
        if ram.radius >= BONK_RADIUS && memory.bonk_cooldown <= 0.0 {
            memory.bonk_cooldown = BONK_GAP;
            sounds.write(PlaySound::at("meme_bonk", ram.position));
        }
    }
}

/// Босс падает с неба — нарастающий свист падения.
fn sound_boss_fall(mut requests: MessageReader<SpawnBoss>, mut sounds: MessageWriter<PlaySound>) {
    if requests.read().count() > 0 {
        sounds.write(PlaySound::new("boss_fall"));
    }
}

/// Ответ штаба на реплику, которая только что кончилась: на вопль о критическом уроне — сразу,
/// на первую за забег брейнрот-реплику — через [`BRAINROT_REPLY_DELAY`] с.
fn reply_to(line: &str, brainrot_commented: bool) -> Option<(f32, &'static str)> {
    if base_id(line) == "crit" {
        Some((0.0, "crit_reply"))
    } else if !brainrot_commented && BRAINROT_LINES.contains(&line) {
        Some((BRAINROT_REPLY_DELAY, "brainrot_comment"))
    } else {
        None
    }
}

/// Следит за репликами: когда реплика кончилась, штаб может на неё ответить.
fn voice_replies(
    time: Res<Time>,
    playing: Query<&VoicePlayback>,
    mut memory: ResMut<AudioMemory>,
    mut cues: MessageWriter<VoiceCue>,
) {
    let now = playing.iter().next().map(|line| line.id.clone());
    if now.is_none()
        && let Some(ended) = memory.last_line.take()
        && memory.reply.is_none()
        && let Some(reply) = reply_to(&ended, memory.brainrot_commented)
    {
        if reply.1 == "brainrot_comment" {
            memory.brainrot_commented = true;
        }
        memory.reply = Some(reply);
    }
    if now.is_some() {
        memory.last_line = now;
    }
    let dt = time.delta_secs();
    if let Some((delay, cue)) = memory.reply.as_mut() {
        *delay -= dt;
        if *delay <= 0.0 {
            cues.write(VoiceCue::new(*cue));
            memory.reply = None;
        }
    }
}

/// Игрок долго не трогает управление и стоит — штаб спрашивает, не уснул ли он.
fn voice_idle(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    rover: Query<&RoverMotion, With<Rover>>,
    mut memory: ResMut<AudioMemory>,
    mut cues: MessageWriter<VoiceCue>,
) {
    let touched = keys.get_pressed().next().is_some() || mouse.get_pressed().next().is_some();
    let moving = rover.iter().any(|motion| motion.speed.abs() > 0.1);
    if touched || moving {
        memory.idle = 0.0;
        return;
    }
    memory.idle += time.delta_secs();
    if memory.idle >= IDLE_SECONDS {
        memory.idle = 0.0;
        cues.write(VoiceCue::new("idle"));
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

/// Поезд объявлен — гудок со стороны, откуда он придёт, и предупреждение штаба; сход — грохот
/// крушения, а если поезд сбросил игрок, штаб это комментирует.
fn sound_trains(
    mut incoming: MessageReader<crate::trains::TrainIncoming>,
    mut derailed: MessageReader<crate::trains::TrainDerailed>,
    rover: Query<&Transform, With<Rover>>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    for train in incoming.read() {
        // Поезд появляется дальше слышимости: гудок звучит с его стороны, но в 40 м от лунохода.
        let at = rover
            .single()
            .map_or(train.position, |r| r.translation + (train.position - r.translation).clamp_length_max(40.0));
        sounds.write(PlaySound::at("train_horn", at));
        cues.write(VoiceCue::new("train_incoming"));
    }
    for crash in derailed.read() {
        sounds.write(PlaySound::at("train_crash", crash.position));
        if crash.cause == Team::Player {
            cues.write(VoiceCue::new("train_derailed"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::sound_list::{in_group, parse_list};

    /// Поводы, которые шлёт озвучка.
    const CUES: [&str; 50] = [
        "run_start", "crit", "crit_reply", "hit", "death", "wave_start", "kill", "boss_dead", "boss_appears",
        "airstrike_call", "weapon_pickup", "weapon_upgrade", "weapon_mortar", "weapon_rockets", "weapon_laser",
        "weapon_airstrike", "ammo_crate", "mortar_reload", "rockets_reload", "big_explosion", "house_destroyed",
        "ammo_empty", "repair_start", "repair_done", "medkit", "resident_died", "alarm_rising", "record",
        "tut_shoot", "tut_mortar", "first_crater", "tut_repair", "tut_pickup", "tut_alarm", "interceptors",
        "service_arrives", "train_incoming", "train_derailed", "streak3", "streak5", "streak10", "idle",
        "brainrot_comment", "beep_happy", "beep_sad", "beep_alert", "mortar_fire_meme", "rockets_fire_meme",
        "mg_reload", "intro",
    ];

    /// Звуки, которые шлёт озвучка, и звук мотора лунохода из `src/sound.rs`.
    const SOUNDS: [&str; 47] = [
        "rover_hit", "enemy_death", "mg_fire", "mortar_fire", "mortar_whistle", "tree_fall", "rock_break",
        "capsule_fall", "rocket_fire", "laser_fire", "airstrike_flyby", "pickup", "hit", "explosion_small",
        "explosion", "explosion_big", "explosion_big_short", "explosion_long", "building_collapse",
        "building_collapse_short", "ui_click", "ui_pause", "repair", "interceptor_hit", "train_horn", "train_crash",
        "train_crash_short", "casing_clink", "splash", "birds_takeoff", "service_siren", "resident_scared",
        "resident_poof", "upgrade", "reload", "dry_fire", "boss_fall", "meme_boom", "meme_airhorn", "meme_hitmarker",
        "meme_oof", "meme_bonk", "meme_beep", "meme_womp", "meme_error", "meme_tung", "rover_engine",
    ];

    #[test]
    fn hq_tone_rises_with_destruction() {
        assert_eq!(tone_for(0.0), 1);
        assert_eq!(tone_for(WORRIED_DESTRUCTION), 2);
        assert_eq!(tone_for(PANIC_DESTRUCTION), 3);
    }

    #[test]
    fn combat_music_waits_before_lull() {
        assert_eq!(music_for(false, true, 0.0, Track::Lull), Track::Combat);
        // Последний враг погиб: бой держится ещё несколько секунд.
        assert_eq!(music_for(false, false, LULL_DELAY - 0.1, Track::Combat), Track::Combat);
        assert_eq!(music_for(false, false, LULL_DELAY + 0.1, Track::Combat), Track::Lull);
        // Из передышки без врагов в бой не переходит.
        assert_eq!(music_for(false, false, 0.5, Track::Lull), Track::Lull);
    }

    #[test]
    fn прорыв_коры_поднимает_боевую_музыку() {
        assert_eq!(music_after_breach(Track::Lull), Track::Combat);
        assert_eq!(music_after_breach(Track::Boss), Track::Boss);
        assert_eq!(music_after_breach(Track::GameOver), Track::GameOver);
        // Без врагов бой держится `LULL_DELAY`, затем передышка.
        assert_eq!(music_for(false, false, 0.0, Track::Combat), Track::Combat);
    }

    #[test]
    fn boss_has_own_music_until_dead() {
        assert_eq!(music_for(true, true, 0.0, Track::Combat), Track::Boss);
        assert_eq!(music_for(false, false, 1.0, Track::Boss), Track::Combat);
        assert_eq!(music_for(false, false, LULL_DELAY + 1.0, Track::Boss), Track::Lull);
    }

    /// Каждый повод и звук, который шлёт озвучка, есть в списках.
    #[test]
    fn cues_and_sounds_exist_in_lists() {
        let lists = [
            (&CUES[..], include_str!("../assets/voice/lines.json")),
            (&SOUNDS[..], include_str!("../assets/sfx/sounds.json")),
        ];
        for (ids, json) in lists {
            let entries = parse_list(json).unwrap();
            for id in ids {
                assert!(entries.iter().any(|entry| in_group(&entry.id, id)), "нет записей «{id}»");
            }
        }
    }

    /// Обратная проверка: каждая запись списков звучит в игре — её группа есть среди поводов
    /// и звуков. Неиспользуемый файл в `assets` роняет тест.
    #[test]
    fn every_list_entry_is_used() {
        let lists = [
            (&CUES[..], include_str!("../assets/voice/lines.json")),
            (&SOUNDS[..], include_str!("../assets/sfx/sounds.json")),
        ];
        for (ids, json) in lists {
            for entry in parse_list(json).unwrap() {
                assert!(ids.iter().any(|id| in_group(&entry.id, id)), "запись «{}» нигде не звучит", entry.id);
            }
        }
    }

    #[test]
    fn серии_убийств_на_3_5_и_10() {
        assert_eq!(streak_cue(3), Some(("streak3", "meme_hitmarker")));
        assert_eq!(streak_cue(5), Some(("streak5", "meme_airhorn")));
        assert_eq!(streak_cue(10), Some(("streak10", "meme_airhorn")));
        assert_eq!(streak_cue(4), None);
    }

    #[test]
    fn подбор_оружия_даёт_свои_реплики() {
        assert_eq!(first_pickup_cue(CurrentWeapon::HomingRockets), "weapon_rockets");
        assert_eq!(first_pickup_cue(CurrentWeapon::Airstrike), "weapon_airstrike");
        assert_eq!(refill_cue(CurrentWeapon::Mortar, 0.1), "mortar_reload");
        assert_eq!(refill_cue(CurrentWeapon::Laser, 0.1), "ammo_crate");
        assert_eq!(refill_cue(CurrentWeapon::Rockets, 0.9), "weapon_pickup");
    }

    #[test]
    fn мем_звуки_под_надписи() {
        assert_eq!(comic_sound("KA-BOOM!"), Some("meme_boom"));
        assert_eq!(comic_sound("TUNG TUNG TUNG"), Some("meme_tung"));
        assert_eq!(comic_sound("6-7"), Some("meme_beep"));
        assert_eq!(comic_sound("BRUH"), None);
    }

    #[test]
    fn штаб_отвечает_на_вопль_и_брейнрот() {
        assert_eq!(reply_to("crit_4", false), Some((0.0, "crit_reply")));
        assert_eq!(reply_to("crit_reply_1", false), None);
        assert_eq!(reply_to("weapon_pickup_3", false), Some((BRAINROT_REPLY_DELAY, "brainrot_comment")));
        assert_eq!(reply_to("weapon_pickup_3", true), None);
    }

    #[test]
    fn серия_мощных_взрывов_звучит_коротко() {
        assert_eq!(in_series("explosion_big", BIG_SERIES_GAP * 0.5), "explosion_big_short");
        assert_eq!(in_series("explosion_big", BIG_SERIES_GAP * 2.0), "explosion_big");
        assert_eq!(in_series("explosion", 0.0), "explosion");
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
