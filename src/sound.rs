//! Звуковые эффекты.
//!
//! Список звуков лежит в `assets/sfx/sounds.json`, файлы — рядом с ним. Чтобы проиграть звук,
//! отправь сообщение [`PlaySound`]:
//!
//! ```ignore
//! fn on_explosion(mut sounds: MessageWriter<PlaySound>) {
//!     sounds.write(PlaySound::at("explosion", position));
//! }
//! ```
//!
//! Идентификатор — основа группы: `explosion` выбирает случайный из `explosion`, `explosion_1`,
//! `explosion_2`. Громкость убывает с расстоянием до слушателя — сущности с [`SoundListener`],
//! а сторона звука совпадает со стороной экрана: событие слева от центра слышно слева.

use std::collections::{HashMap, HashSet};

use bevy::audio::{AudioSinkPlayback, SpatialAudioSink, SpatialListener, SpatialScale, Volume};
use bevy::prelude::*;

use crate::combat::Health;
use crate::rover::{MAX_FORWARD_SPEED, Rover, RoverMotion};
use crate::run::{GameState, WorldRebuilt};
use crate::settings::{Settings, SoundGroup};
use crate::sound_list::{Dice, SoundList};

/// Каталог звуков внутри `assets`.
const SFX_DIR: &str = "sfx";

/// Файл со списком звуков внутри [`SFX_DIR`].
const LIST_FILE: &str = "sounds.json";

/// Расстояние в метрах, на котором громкость падает вдвое.
const HALF_VOLUME_DISTANCE: f32 = 25.0;

/// Дальше этого расстояния звук не создаётся.
const MAX_DISTANCE: f32 = 90.0;

/// Разброс скорости воспроизведения, чтобы частые звуки не повторялись один в один.
const SPEED_SPREAD: (f32, f32) = (0.9, 1.1);

/// Расстояние между «ушами» слушателя, м. На сторону звука влияет только направление на него.
const EAR_GAP: f32 = 2.0;

/// Масштаб расстояний для расстановки по сторонам. Он сжимает все слышимые расстояния до величин
/// меньше единицы: так звуковая библиотека не добавляет своё затухание к нашему [`volume_at`].
const PAN_SCALE: f32 = 1.0 / 200.0;

/// Зацикленный звук мотора лунохода.
const ENGINE_SOUND: &str = "rover_engine";

/// Громкость мотора на полной скорости.
const ENGINE_VOLUME: f32 = 0.3;

/// Высота звука мотора на месте и прибавка к ней на полной скорости.
const ENGINE_PITCH: (f32, f32) = (0.7, 0.5);

/// Сколько экземпляров одной группы звучит одновременно, если для группы нет своего предела.
const DEFAULT_LIMIT: usize = 4;

/// Дольше этого экземпляр звука не живёт, с: самый длинный звук — 12 с, на замедлении
/// времени возраст звука мира растёт медленнее вместе с ним. Без звукового устройства
/// экземпляры никто не удаляет, кроме этого срока.
const MAX_SOUND_AGE: f32 = 20.0;

/// Свои пределы одновременных экземпляров по началу идентификатора группы.
const LIMITS: &[(&str, usize)] = &[
    ("mg_", 6),
    ("mortar_", 4),
    ("rocket_", 3),
    ("hit", 3),
    // Длинный раскат — только под самыми мощными взрывами; стоит выше общего `explosion`.
    ("explosion_long", 2),
    ("explosion", 5),
    ("building_", 3),
    ("tree_", 3),
    ("rock_", 3),
    ("capsule_", 3),
    ("resident_", 4),
    ("ui_", 2),
    // Гильзы и всплески идут пачками: больше двух сразу не слышно.
    ("casing_", 2),
    ("splash", 2),
    ("birds_", 2),
    ("airstrike_", 2),
    // Подписи и мемы звучат по одному.
    ("meme_", 1),
    ("service_", 1),
    ("upgrade", 1),
    ("reload", 1),
    ("dry_", 1),
    ("boss_", 1),
];

/// Мелодичные звуки: без разброса высоты, иначе джингл и сирена фальшивят.
const NO_PITCH_SPREAD: &[&str] = &["upgrade", "service_siren", "meme_womp", "meme_error", "meme_beep"];

/// Разброс высоты для группы: у мелодичных его нет.
fn pitch_spread(group: &str) -> (f32, f32) {
    if NO_PITCH_SPREAD.iter().any(|prefix| group.starts_with(prefix)) {
        (1.0, 1.0)
    } else {
        SPEED_SPREAD
    }
}

/// Подключает загрузку звуков и их воспроизведение по сообщению [`PlaySound`].
pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Sounds>()
            .init_resource::<Dice>()
            .add_message::<PlaySound>()
            .add_systems(Startup, load_sounds)
            .init_resource::<InBackground>()
            .init_resource::<SystemSilence>()
            .add_systems(
                Update,
                pause_audio_in_background
                    .run_if(|| crate::profile::get().audio.pause_in_background)
                    .in_set(crate::frame::GameSet::Presentation)
                    // Звуки мира на паузе игры замирают раньше, чем возврат из фона их разбудит.
                    .after(pause_world_sounds::<AudioSink>)
                    .after(pause_world_sounds::<SpatialAudioSink>),
            )
            .add_systems(
                PostUpdate,
                (
                    hold_new_sounds_in_background.before(bevy::audio::AudioPlaybackSystems),
                    silence_in_background.after(bevy::audio::AudioPlaybackSystems),
                )
                    .run_if(|| crate::profile::get().audio.pause_in_background),
            )
            .add_systems(
                Update,
                forget_world_sounds.in_set(crate::frame::GameSet::Reset),
            )
            .add_systems(
                Update,
                (
                    expire_sounds,
                    attach_listener,
                    attach_ears,
                    play_sounds,
                    drive_engine,
                    stretch_world_sounds::<AudioSink>,
                    stretch_world_sounds::<SpatialAudioSink>,
                    pause_world_sounds::<AudioSink>.run_if(in_state(GameState::Paused)),
                    pause_world_sounds::<SpatialAudioSink>.run_if(in_state(GameState::Paused)),
                )
                    // Пока идёт загрузка, ничего не звучит (src/loading.rs).
                    .run_if(not(in_state(GameState::Loading)))
                    .in_set(crate::frame::GameSet::Presentation),
            )
            .add_systems(
                OnExit(GameState::Paused),
                (resume_world_sounds::<AudioSink>, resume_world_sounds::<SpatialAudioSink>),
            );
    }
}

/// Просьба проиграть звук.
#[derive(Message, Debug, Clone)]
pub struct PlaySound {
    /// Основа группы звуков или идентификатор конкретной записи.
    pub id: String,
    /// Место события в мире. `None` — звук без расстояния: интерфейс, сам луноход.
    pub position: Option<Vec3>,
}

impl PlaySound {
    #[allow(dead_code)]
    /// Звук без места в мире: всегда на полной громкости.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            position: None,
        }
    }

    #[allow(dead_code)]
    /// Звук события в точке мира: громкость убывает с расстоянием до слушателя.
    pub fn at(id: impl Into<String>, position: Vec3) -> Self {
        Self {
            id: id.into(),
            position: Some(position),
        }
    }
}

/// Метка слушателя: от этой сущности считается расстояние до звуков. Ставится на луноход.
#[derive(Component)]
pub struct SoundListener;

/// Делает слушателем луноход, как только он появился.
fn attach_listener(mut commands: Commands, rovers: Query<Entity, Added<Rover>>) {
    for rover in &rovers {
        commands.entity(rover).insert(SoundListener);
    }
}

/// Ставит «уши» на камеру: звук слева от центра экрана слышен в основном в левом канале.
///
/// Уши стоят на камере, а не на луноходе: камера не вращается, поэтому стороны звука совпадают
/// со сторонами экрана, а сдвиг в сторону растёт плавно, как положение события на экране.
///
/// Левое и правое ухо нарочно поменяны местами. Звуковая библиотека делает дальнее ухо громче
/// и полагается на собственное затухание с расстоянием, а оно у нас выключено масштабом
/// [`PAN_SCALE`]: без перестановки звук слева звучал бы справа.
fn attach_ears(
    mut commands: Commands,
    cameras: Query<Entity, With<Camera3d>>,
    ears: Query<(), With<SpatialListener>>,
) {
    if !ears.is_empty() {
        return;
    }
    if let Some(camera) = cameras.iter().next() {
        commands.entity(camera).insert(SpatialListener {
            left_ear_offset: Vec3::X * EAR_GAP / 2.0,
            right_ear_offset: Vec3::X * EAR_GAP / -2.0,
        });
    }
}

/// Android и iOS не глушат приложение в фоне: уходя с экрана, игра ставит звук на паузу сама
/// (`audio.pause_in_background` профиля платформы). Сообщения `AppLifecycle` шлёт winit; Bevy 0.20
/// переводит `WillSuspend` в `Suspended` и `WillResume` в `Running` раньше, чем отправит сообщение,
/// поэтому приходят только они.
/// Звук придержан: игра в фоне или идёт звонок ([`SystemSilence`]).
#[derive(Resource, Default)]
struct InBackground(bool);

/// Система просит игру молчать, хотя окно на экране: на Android — пока звонит телефон или идёт
/// разговор (`android::watch_audio`). Звук придерживается так же, как в фоне.
#[derive(Resource, Default)]
pub struct SystemSilence(pub bool);

/// Звук, который ещё не начался и родится на паузе, потому что игра в фоне: при возврате
/// его настройка паузы снимается.
#[derive(Component)]
struct HeldInBackground;

/// Уход в фон или звонок — пауза всего, что звучит; возврат — снова звучит всё, кроме звуков мира
/// на паузе игры (их снимет `resume_world_sounds`, когда игра продолжится).
#[allow(clippy::too_many_arguments)]
fn pause_audio_in_background(
    mut commands: Commands,
    mut lifecycle: MessageReader<bevy::window::AppLifecycle>,
    mut away: Local<bool>,
    silence: Res<SystemSilence>,
    mut background: ResMut<InBackground>,
    mut output: ResMut<bevy::audio::AudioOutputSuspended>,
    state: Res<State<GameState>>,
    sinks: Query<(&AudioSink, Option<&SoundInstance>)>,
    spatial_sinks: Query<(&SpatialAudioSink, Option<&SoundInstance>)>,
    mut held: Query<(Entity, &mut PlaybackSettings, Option<&SoundInstance>), With<HeldInBackground>>,
) {
    use bevy::window::AppLifecycle;
    for event in lifecycle.read() {
        match event {
            AppLifecycle::WillSuspend | AppLifecycle::Suspended => *away = true,
            AppLifecycle::WillResume | AppLifecycle::Running => *away = false,
            _ => {}
        }
    }
    let quiet = *away || silence.0;
    if quiet && !background.0 {
        background.0 = true;
        // Поток вывода закрывается (`vendor/bevy_audio`): пауза звука его не останавливает.
        output.0 = true;
        sinks.iter().for_each(|(sink, _)| sink.pause());
        spatial_sinks.iter().for_each(|(sink, _)| sink.pause());
    } else if !quiet && background.0 {
        background.0 = false;
        output.0 = false;
        let game_paused = *state.get() == GameState::Paused;
        let wakes = |instance: Option<&SoundInstance>| !(game_paused && instance.is_some_and(|i| i.world));
        sinks.iter().filter(|(_, i)| wakes(*i)).for_each(|(sink, _)| sink.play());
        spatial_sinks.iter().filter(|(_, i)| wakes(*i)).for_each(|(sink, _)| sink.play());
        // Звук мира на паузе игры остаётся придержанным: его отпустит `resume_world_sounds`.
        for (entity, mut settings, instance) in &mut held {
            if wakes(instance) {
                settings.paused = false;
                commands.entity(entity).remove::<HeldInBackground>();
            }
        }
    }
}

/// Звук, заказанный в фоне (или пересозданный при смене устройства вывода, `follow_output_device`),
/// рождается на паузе: `bevy_audio` заводит его в `PostUpdate`, уже после `pause_audio_in_background`.
fn hold_new_sounds_in_background(
    mut commands: Commands,
    background: Res<InBackground>,
    mut pending: Query<
        (Entity, &mut PlaybackSettings),
        (Without<AudioSink>, Without<SpatialAudioSink>, Without<HeldInBackground>),
    >,
) {
    if !background.0 {
        return;
    }
    for (entity, mut settings) in &mut pending {
        if !settings.paused {
            settings.paused = true;
            commands.entity(entity).insert(HeldInBackground);
        }
    }
}

/// Страховка: что зазвучало в фоне в обход паузы, замолкает в том же кадре.
fn silence_in_background(background: Res<InBackground>, sinks: Query<&AudioSink>, spatial_sinks: Query<&SpatialAudioSink>) {
    if !background.0 {
        return;
    }
    sinks.iter().filter(|sink| !sink.is_paused()).for_each(|sink| sink.pause());
    spatial_sinks.iter().filter(|sink| !sink.is_paused()).for_each(|sink| sink.pause());
}

/// Громкости левого и правого каналов для звука, смещённого от слушателя на долю `side`:
/// −1 — точно слева, 0 — прямо, 1 — точно справа. Повторяет расчёт звуковой библиотеки
/// при наших настройках ушей и нужна тестам.
#[cfg(test)]
fn channel_gains(side: f32) -> (f32, f32) {
    // Библиотека считает (расстояние до своего уха − до другого) / расстояние между ушами.
    // «Левое» ухо у нас стоит справа, поэтому для звука слева эта доля у него положительна.
    let left = ((-side + 1.0) / 4.0 + 0.5).min(1.0);
    let right = ((side + 1.0) / 4.0 + 0.5).min(1.0);
    (left, right)
}

/// Метка зацикленного звука мотора.
#[derive(Component)]
struct EngineSound;

/// Мотор лунохода: один зацикленный звук, громкость и высота растут со скоростью. Вне забега,
/// на паузе и после гибели мотор молчит.
fn drive_engine(
    mut commands: Commands,
    sounds: Res<Sounds>,
    settings: Res<Settings>,
    state: Res<State<GameState>>,
    time: Res<Time<Virtual>>,
    rover: Query<(&RoverMotion, &Health), With<Rover>>,
    started: Query<(), With<EngineSound>>,
    mut engine: Query<&mut AudioSink, With<EngineSound>>,
) {
    if started.is_empty() {
        if let Some(sound) = sounds.list.get(ENGINE_SOUND).and_then(|loaded| loaded.sound.clone()) {
            commands.spawn((
                EngineSound,
                AudioPlayer::new(sound),
                PlaybackSettings {
                    volume: Volume::Linear(0.0),
                    ..PlaybackSettings::LOOP
                },
            ));
        }
        return;
    }
    let load = if *state.get() == GameState::Playing {
        rover
            .iter()
            .next()
            .filter(|(_, health)| !health.is_dead())
            .map_or(0.0, |(motion, _)| engine_load(motion.speed))
    } else {
        0.0
    };
    for mut sink in &mut engine {
        sink.set_volume(Volume::Linear(ENGINE_VOLUME * load * settings.gain(SoundGroup::Effects)));
        sink.set_speed((ENGINE_PITCH.0 + ENGINE_PITCH.1 * load) * time_stretch(&time));
    }
}

/// Нагрузка мотора от 0 до 1 по скорости лунохода, вперёд или назад.
fn engine_load(speed: f32) -> f32 {
    (speed.abs() / MAX_FORWARD_SPEED).min(1.0)
}

/// Все звуки из `sounds.json`.
#[derive(Resource, Default)]
struct Sounds {
    list: SoundList,
}

/// Звучащий экземпляр звука и его группа, для предела одновременных экземпляров.
#[derive(Component)]
struct SoundInstance {
    group: String,
    /// Звук игрового мира: замирает на паузе и растягивается замедлением времени.
    world: bool,
    /// Случайный разброс высоты этого экземпляра, без учёта замедления времени.
    pitch: f32,
    /// Сколько секунд звучит: для звуков мира по игровому времени, для интерфейса по реальному.
    age: f32,
    /// Громкость по расстоянию при создании: самый тихий звук уступает место громкому.
    volume: f32,
}

/// Звук игрового мира, а не интерфейса. Звуки интерфейса начинаются с `ui_`.
fn is_world_sound(id: &str) -> bool {
    !id.starts_with("ui_")
}

/// Звук вне общего предела: интерфейс и мемы. Мемы редкие и звучат по одному, а нужны как раз
/// в разгар боя, когда предел занят взрывами.
fn outside_total(id: &str) -> bool {
    !is_world_sound(id) || id.starts_with("meme_")
}

fn load_sounds(
    mut sounds: ResMut<Sounds>,
    asset_server: Res<AssetServer>,
    preload: Option<ResMut<crate::loading::Preload>>,
) {
    sounds.list = SoundList::load(&asset_server, SFX_DIR, LIST_FILE, "Звуки");
    if let Some(mut preload) = preload {
        sounds.list.preload(&mut preload);
    }
}

/// Громкость звука на расстоянии `distance` метров от слушателя. `None` — слишком далеко.
fn volume_at(distance: f32) -> Option<f32> {
    if distance > MAX_DISTANCE {
        return None;
    }
    let ratio = distance / HALF_VOLUME_DISTANCE;
    Some(1.0 / (1.0 + ratio * ratio))
}

/// Найдётся ли место новому звуку группы `group`: в ней звучит `in_group`, всего в мире —
/// `in_world` звуков.
/// Есть ли место для звука группы: предел группы и общий предел звуков мира `total`. Каждый звук —
/// поток mp3, без общего предела бой даёт 30–40 потоков. Звуки интерфейса и мемы в общий предел не
/// входят. Общий предел — `audio.world_sounds` профиля платформы.
fn has_room(group: &str, in_group: usize, in_world: usize, total: usize) -> bool {
    in_group < limit_for(group) && (outside_total(group) || in_world < total)
}

/// Кого вытеснить, когда общий предел занят: самый тихий звучащий звук мира, если он тише нового.
fn quietest_below<'a>(
    playing: impl Iterator<Item = (Entity, &'a SoundInstance)>,
    volume: f32,
) -> Option<(Entity, &'a SoundInstance)> {
    playing
        .filter(|(_, instance)| !outside_total(&instance.group) && instance.volume < volume)
        .min_by(|(_, a), (_, b)| a.volume.total_cmp(&b.volume))
}

/// Предел одновременных экземпляров для группы.
fn limit_for(group: &str) -> usize {
    LIMITS
        .iter()
        .find(|(prefix, _)| group.starts_with(prefix))
        .map_or(DEFAULT_LIMIT, |(_, limit)| *limit)
}

fn play_sounds(
    mut commands: Commands,
    mut requests: MessageReader<PlaySound>,
    sounds: Res<Sounds>,
    settings: Res<Settings>,
    mut dice: ResMut<Dice>,
    time: Res<Time<Virtual>>,
    listener: Query<&GlobalTransform, With<SoundListener>>,
    instances: Query<(Entity, &SoundInstance)>,
    mut reported: Local<HashSet<String>>,
) {
    if requests.is_empty() {
        return;
    }
    let listener_position = listener.iter().next().map(GlobalTransform::translation);
    // Громкость эффектов из настроек: звуки мира и интерфейса.
    let gain = settings.gain(SoundGroup::Effects);
    // Сколько экземпляров каждой группы и всех звуков мира уже звучит, вместе с созданными
    // в этом кадре.
    let mut in_group: HashMap<String, usize> = HashMap::new();
    let mut in_world = 0;
    for (_, instance) in &instances {
        *in_group.entry(instance.group.clone()).or_default() += 1;
        in_world += usize::from(!outside_total(&instance.group));
    }
    let total = crate::profile::get().audio.world_sounds;
    // Вытесненные в этом кадре: второй раз их не вытесняем.
    let mut evicted: HashSet<Entity> = HashSet::new();

    for request in requests.read() {
        let volume = match (request.position, listener_position) {
            (Some(position), Some(listener)) => {
                let Some(volume) = volume_at(position.distance(listener)) else {
                    continue;
                };
                volume
            }
            _ => 1.0,
        };

        let variants: Vec<_> = sounds.list.group(&request.id).collect();
        let Some(chosen) = variants.get(dice.below(variants.len())) else {
            // О каждом идентификаторе пишем один раз, иначе пулемёт забьёт лог.
            if reported.insert(request.id.clone()) {
                if sounds.list.has_group(&request.id) {
                    warn!(
                        "Звуки: у «{}» нет звукового файла в assets/{SFX_DIR}",
                        request.id
                    );
                } else {
                    warn!(
                        "Звуки: «{}» нет в {SFX_DIR}/{LIST_FILE} — опечатка в идентификаторе?",
                        request.id
                    );
                }
            }
            continue;
        };

        let active = in_group.get(&request.id).copied().unwrap_or(0);
        if !has_room(&request.id, active, in_world, total) {
            // Общий предел занят, а звук громче самого тихого из звучащих: тот уступает место.
            // Близкий взрыв не теряется из-за далёкой стрельбы.
            let victim = (has_room(&request.id, active, 0, total))
                .then(|| quietest_below(instances.iter().filter(|(e, _)| !evicted.contains(e)), volume))
                .flatten();
            let Some((victim, quiet)) = victim else {
                continue;
            };
            evicted.insert(victim);
            commands.entity(victim).despawn();
            in_world -= 1;
            if let Some(count) = in_group.get_mut(&quiet.group) {
                *count = count.saturating_sub(1);
            }
        }
        let Some(sound) = &chosen.sound else {
            continue;
        };
        let world = is_world_sound(&request.id);
        *in_group.entry(request.id.clone()).or_default() += 1;
        in_world += usize::from(!outside_total(&request.id));
        let (low, high) = pitch_spread(&request.id);
        let pitch = if low < high { dice.range(low, high) } else { low };
        // Замедление времени растягивает звук мира.
        let speed = if world { pitch * time_stretch(&time) } else { pitch };
        let instance = SoundInstance {
            group: request.id.clone(),
            world,
            pitch,
            age: 0.0,
            volume,
        };
        let settings = PlaybackSettings {
            volume: Volume::Linear(volume * gain),
            speed,
            ..PlaybackSettings::DESPAWN
        };
        match request.position {
            // Звук с местом в мире расставляется по сторонам: левее центра экрана — в левый канал.
            Some(position) => commands.spawn((
                instance,
                AudioPlayer::new(sound.clone()),
                PlaybackSettings {
                    spatial: true,
                    spatial_scale: Some(SpatialScale::new(PAN_SCALE)),
                    ..settings
                },
                Transform::from_translation(position),
            )),
            None => commands.spawn((instance, AudioPlayer::new(sound.clone()), settings)),
        };
    }
}

/// Убирает звуки старше [`MAX_SOUND_AGE`]: их не удалит звуковая подсистема, если нет
/// устройства или звук не загрузился. Иначе они навсегда заняли бы место в пределах.
fn expire_sounds(
    mut commands: Commands,
    time: Res<Time<Virtual>>,
    real: Res<Time<Real>>,
    mut instances: Query<(Entity, &mut SoundInstance)>,
) {
    for (entity, mut instance) in &mut instances {
        instance.age += if instance.world { time.delta_secs() } else { real.delta_secs() };
        if instance.age > MAX_SOUND_AGE {
            commands.entity(entity).despawn();
        }
    }
}

/// Мир перестроен: звуки прошлого забега, замершие на паузе, не доигрывают в новом.
fn forget_world_sounds(
    mut commands: Commands,
    mut rebuilt: MessageReader<WorldRebuilt>,
    instances: Query<(Entity, &SoundInstance)>,
) {
    if rebuilt.read().last().is_none() {
        return;
    }
    for (entity, instance) in &instances {
        if instance.world {
            commands.entity(entity).despawn();
        }
    }
}

/// Во сколько раз замедление времени меняет скорость звуков мира.
fn time_stretch(time: &Time<Virtual>) -> f32 {
    time.relative_speed().max(0.1)
}

/// Замедление времени растягивает и те звуки мира, что уже звучат. `S` — вид проигрывателя:
/// обычный или расставленный по сторонам.
fn stretch_world_sounds<S: Component + AudioSinkPlayback>(
    time: Res<Time<Virtual>>,
    mut applied: Local<Option<f32>>,
    instances: Query<(&SoundInstance, &S)>,
) {
    let stretch = time_stretch(&time);
    if *applied == Some(stretch) {
        return;
    }
    *applied = Some(stretch);
    for (instance, sink) in &instances {
        if instance.world {
            sink.set_speed(instance.pitch * stretch);
        }
    }
}

/// На паузе звуки мира замирают. Система идёт каждый кадр паузы: у звука, созданного
/// в последнем кадре игры, проигрыватель появляется уже на паузе.
fn pause_world_sounds<S: Component + AudioSinkPlayback>(instances: Query<(&SoundInstance, &S)>) {
    for (instance, sink) in &instances {
        if instance.world && !sink.is_paused() {
            sink.pause();
        }
    }
}

/// После паузы звуки мира продолжаются с того же места; придержанные при возврате из фона
/// (`pause_audio_in_background`) отпускаются.
fn resume_world_sounds<S: Component + AudioSinkPlayback>(
    mut commands: Commands,
    instances: Query<(&SoundInstance, &S)>,
    mut held: Query<(Entity, &mut PlaybackSettings), With<HeldInBackground>>,
    background: Res<InBackground>,
) {
    // Игра в фоне или звонит телефон: звук мира отпустит возврат (`pause_audio_in_background`).
    if background.0 {
        return;
    }
    for (instance, sink) in &instances {
        if instance.world {
            sink.play();
        }
    }
    for (entity, mut settings) in &mut held {
        settings.paused = false;
        commands.entity(entity).remove::<HeldInBackground>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_sounds_are_not_world_sounds() {
        assert!(is_world_sound("explosion"));
        assert!(is_world_sound("rover_hit"));
        assert!(!is_world_sound("ui_pause"));
        assert!(!is_world_sound("ui_click"));
    }

    #[test]
    fn volume_falls_with_distance() {
        assert_eq!(volume_at(0.0), Some(1.0));
        let near = volume_at(10.0).unwrap();
        let far = volume_at(60.0).unwrap();
        assert!(near > far);
        assert!((volume_at(HALF_VOLUME_DISTANCE).unwrap() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn too_far_sound_is_not_played() {
        assert_eq!(volume_at(MAX_DISTANCE + 1.0), None);
    }

    #[test]
    fn groups_have_limits() {
        assert_eq!(limit_for("mg_fire"), 6);
        assert_eq!(limit_for("explosion_big"), 5);
        assert_eq!(limit_for("pickup"), DEFAULT_LIMIT);
        assert_eq!(limit_for("tree_fall"), 3);
        assert_eq!(limit_for("rock_break"), 3);
        assert_eq!(limit_for("capsule_fall"), 3);
        assert_eq!(limit_for("explosion_long"), 2);
        assert_eq!(limit_for("casing_clink"), 2);
        assert_eq!(limit_for("meme_tung"), 1);
    }

    #[test]
    fn громкий_звук_вытесняет_самый_тихий() {
        let sound = |group: &str, volume: f32| SoundInstance { group: group.into(), world: true, pitch: 1.0, age: 0.0, volume };
        let playing = [
            (Entity::from_raw_u32(1).unwrap(), sound("mg_fire", 0.3)),
            (Entity::from_raw_u32(2).unwrap(), sound("explosion", 0.1)),
            (Entity::from_raw_u32(3).unwrap(), sound("meme_airhorn", 0.05)),
            (Entity::from_raw_u32(4).unwrap(), sound("hit", 0.6)),
        ];
        let iter = || playing.iter().map(|(e, i)| (*e, i));
        // Мем вне общего предела: вытесняется самый тихий звук мира.
        assert_eq!(quietest_below(iter(), 0.9).map(|(e, _)| e), Some(playing[1].0));
        // Новый звук тише всех: никто не уступает.
        assert!(quietest_below(iter(), 0.05).is_none());
    }

    #[test]
    fn мелодичные_звуки_без_разброса_высоты() {
        assert_eq!(pitch_spread("upgrade"), (1.0, 1.0));
        assert_eq!(pitch_spread("meme_womp"), (1.0, 1.0));
        assert_eq!(pitch_spread("explosion"), SPEED_SPREAD);
    }

    #[test]
    fn world_sounds_share_a_total_limit() {
        const TOTAL: usize = 10;
        assert!(has_room("explosion", 0, TOTAL - 1, TOTAL));
        assert!(!has_room("explosion", 0, TOTAL, TOTAL));
        assert!(!has_room("explosion", limit_for("explosion"), 0, TOTAL));
        // Интерфейс звучит и при полном мире.
        assert!(has_room("ui_click", 0, TOTAL, TOTAL));
        // Мемы звучат и при занятом пределе, но по одному.
        assert!(has_room("meme_airhorn", 0, TOTAL, TOTAL));
        assert!(!has_room("meme_airhorn", 1, 0, TOTAL));
    }

    #[test]
    fn sound_on_the_left_is_louder_in_the_left_channel() {
        let (left, right) = channel_gains(-1.0);
        assert!(left > right);
        let (left, right) = channel_gains(1.0);
        assert!(right > left);
        // Чем дальше в сторону, тем сильнее перекос.
        assert!(channel_gains(-1.0).0 > channel_gains(-0.3).0);
    }

    #[test]
    fn sound_straight_ahead_is_equal_in_both_channels() {
        let (left, right) = channel_gains(0.0);
        assert_eq!(left, right);
    }

    #[test]
    fn engine_load_follows_speed_in_both_directions() {
        assert_eq!(engine_load(0.0), 0.0);
        assert_eq!(engine_load(MAX_FORWARD_SPEED), 1.0);
        assert_eq!(engine_load(-MAX_FORWARD_SPEED * 0.5), 0.5);
        assert_eq!(engine_load(MAX_FORWARD_SPEED * 3.0), 1.0);
    }
}
