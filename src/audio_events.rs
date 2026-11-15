//! Какие игровые события чем озвучены: единственное место, где события превращаются
//! в звуки, поводы для реплик и смену музыки.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::airstrike::Strike;
use crate::alarm::Alarm;
use crate::arsenal::AmmoRanOut;
use crate::boss::Boss;
use crate::buildings::BuildingDestroyed;
use crate::birds::BirdsTookOff;
use crate::bullet::FireBullet;
use crate::casings::CasingClink;
use crate::capsule::{Capsule, FALL_SECONDS, MARK_SECONDS};
use crate::collapse::CollapseHouse;
use crate::combat::{Damage, Died, Health, SurvivalTime, Team};
use crate::enemy::Enemy;
use crate::explosion::Explosion;
use crate::frame::GameSet;
use crate::hq::{HqMemory, HqTone, VoiceCue};
use crate::laser::Beam;
use crate::lava::CrustBreached;
use crate::mortar::{FLIGHT_TIME, Shell};
use crate::music::{MusicState, Track};
use crate::pickups::Popup;
use crate::upgrades::WeaponUpgraded;
use crate::props::{Prop, PropKind};
use crate::records::{Records, records_path};
use crate::repair::{MedkitPicked, RepairChanged};
use crate::residents::ResidentDied;
use crate::rockets::Rocket;
use crate::rover::Rover;
use crate::run::{GameState, RunStarted, WorldRebuilt};
use crate::sound::PlaySound;
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
}

/// Подключает озвучку игровых событий.
pub struct AudioEventsPlugin;

impl Plugin for AudioEventsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AudioMemory>()
            .add_systems(OnEnter(GameState::Menu), menu_music)
            .add_systems(Update, sound_trains.in_set(GameSet::Presentation))
            .add_systems(Update, (sound_casings, sound_birds).in_set(GameSet::Presentation))
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
) {
    if died.read().any(|message| rover.contains(message.entity)) {
        cues.write(VoiceCue::new("death"));
        music.track = Track::GameOver;
    }
}

/// Новая волна — реплика о ней; гибель врага — звук и редкий комментарий лунохода, гибель
/// босса — важная реплика.
fn voice_enemies(
    mut waves: MessageReader<WaveStarted>,
    mut died: MessageReader<Died>,
    enemies: Query<(), With<Enemy>>,
    bosses: Query<(), With<Boss>>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
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
        } else {
            cues.write(VoiceCue::new("kill"));
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

/// Гибель жителя — реплика штаба и вклад в объём разрушений.
fn voice_residents(
    mut deaths: MessageReader<ResidentDied>,
    mut memory: ResMut<AudioMemory>,
    mut cues: MessageWriter<VoiceCue>,
) {
    let count = deaths.read().count();
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
    mut cues: MessageWriter<VoiceCue>,
) {
    let count = breaches.read().count();
    if count > 0 {
        memory.destruction += BREACH_WEIGHT * count as f32;
        cues.write(VoiceCue::new("big_explosion"));
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

/// Музыка следует за врагами и боссом.
fn music_follows_enemies(
    time: Res<Time>,
    enemies: Query<(), With<Enemy>>,
    bosses: Query<&Health, With<Boss>>,
    rover: Query<&Health, With<Rover>>,
    mut memory: ResMut<AudioMemory>,
    mut music: ResMut<MusicState>,
) {
    let alive = rover.iter().any(|health| !health.is_dead());
    if !alive || matches!(music.track, Track::Silence | Track::GameOver) {
        return;
    }
    let enemies = !enemies.is_empty();
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
/// куда летит снаряд. Свист снаряда, летящего в луноход, слышен громче всего.
fn sound_shells(
    time: Res<Time>,
    shells: Query<(&Transform, &Shell), Added<Shell>>,
    mut memory: ResMut<AudioMemory>,
    mut sounds: MessageWriter<PlaySound>,
) {
    for (transform, shell) in &shells {
        sounds.write(PlaySound::at("mortar_fire", transform.translation));
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

/// Золотой ящик: звон подбора и восторг механика «UPGRADE!».
fn sound_upgrades(
    mut upgraded: MessageReader<WeaponUpgraded>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    for _ in upgraded.read() {
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

/// Взрыв — звук по мощности в месте взрыва. Мощный взрыв игрока — повод для штаба. Первый
/// кратер объясняет обучение (`src/intro.rs`): там реплика ждёт, пока штаб замолчит.
fn sound_explosions(
    mut explosions: MessageReader<Explosion>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    for explosion in explosions.read() {
        sounds.write(PlaySound::at(explosion_sound(explosion.power), explosion.position));
        if explosion.source_team == Team::Player && explosion.power > BIG_EXPLOSION_POWER {
            cues.write(VoiceCue::new("big_explosion"));
        }
    }
}

/// Дом рушится — грохот там, где он стоял; разрушенный дом — повод для штаба.
fn sound_buildings(
    mut collapses: MessageReader<CollapseHouse>,
    mut destroyed: MessageReader<BuildingDestroyed>,
    mut memory: ResMut<AudioMemory>,
    mut sounds: MessageWriter<PlaySound>,
    mut cues: MessageWriter<VoiceCue>,
) {
    for collapse in collapses.read() {
        sounds.write(PlaySound::at("building_collapse", collapse.base));
    }
    let count = destroyed.read().count();
    if count > 0 {
        memory.destruction += HOUSE_WEIGHT * count as f32;
        cues.write(VoiceCue::new("house_destroyed"));
    }
}

/// Служба защиты жителей приехала по тревоге — штаб предупреждает.
fn voice_service(mut arrived: MessageReader<crate::service::ServiceArrived>, mut cues: MessageWriter<VoiceCue>) {
    if arrived.read().count() > 0 {
        cues.write(VoiceCue::new("service_arrives"));
    }
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
    fn boss_has_own_music_until_dead() {
        assert_eq!(music_for(true, true, 0.0, Track::Combat), Track::Boss);
        assert_eq!(music_for(false, false, 1.0, Track::Boss), Track::Combat);
        assert_eq!(music_for(false, false, LULL_DELAY + 1.0, Track::Boss), Track::Lull);
    }

    /// Каждый повод и звук, который шлёт озвучка, есть в списках.
    #[test]
    fn cues_and_sounds_exist_in_lists() {
        let cues = [
            "run_start", "crit", "hit", "death", "wave_start", "kill", "boss_dead", "boss_appears",
            "airstrike_call", "weapon_pickup", "big_explosion", "house_destroyed", "ammo_empty",
            "repair_start", "repair_done", "medkit", "resident_died", "alarm_rising", "record",
            "tut_shoot", "tut_mortar", "first_crater", "tut_repair", "tut_pickup", "tut_alarm",
            "interceptors",
        ];
        let sounds = [
            "rover_hit", "enemy_death", "mg_fire", "mortar_fire", "mortar_whistle", "tree_fall",
            "rock_break", "capsule_fall", "rocket_fire", "laser_fire", "airstrike_flyby", "pickup",
            "hit", "explosion_small", "explosion", "explosion_big", "building_collapse", "ui_click",
            "ui_pause", "repair", "interceptor_hit",
        ];
        let lists = [
            (&cues[..], include_str!("../assets/voice/lines.json")),
            (&sounds[..], include_str!("../assets/sfx/sounds.json")),
        ];
        for (ids, json) in lists {
            let entries = parse_list(json).unwrap();
            for id in ids {
                assert!(entries.iter().any(|entry| in_group(&entry.id, id)), "нет записей «{id}»");
            }
        }
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
