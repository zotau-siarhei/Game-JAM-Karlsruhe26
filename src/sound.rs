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

/// Сколько звуков мира звучит одновременно всего, по всем группам. Каждый — поток mp3,
/// без общего предела бой даёт 30–40 потоков. Звуки интерфейса в предел не входят.
const TOTAL_LIMIT: usize = 20;

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
}

/// Звук игрового мира, а не интерфейса. Звуки интерфейса начинаются с `ui_`.
fn is_world_sound(id: &str) -> bool {
    !id.starts_with("ui_")
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
fn has_room(group: &str, in_group: usize, in_world: usize) -> bool {
    in_group < limit_for(group) && (!is_world_sound(group) || in_world < TOTAL_LIMIT)
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
    instances: Query<&SoundInstance>,
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
    for instance in &instances {
        *in_group.entry(instance.group.clone()).or_default() += 1;
        in_world += usize::from(instance.world);
    }

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
        if !has_room(&request.id, active, in_world) {
            continue;
        }
        let Some(sound) = &chosen.sound else {
            continue;
        };
        let world = is_world_sound(&request.id);
        *in_group.entry(request.id.clone()).or_default() += 1;
        in_world += usize::from(world);
        let (low, high) = pitch_spread(&request.id);
        let pitch = if low < high { dice.range(low, high) } else { low };
        // Замедление времени растягивает звук мира.
        let speed = if world { pitch * time_stretch(&time) } else { pitch };
        let instance = SoundInstance {
            group: request.id.clone(),
            world,
            pitch,
            age: 0.0,
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

/// После паузы звуки мира продолжаются с того же места.
fn resume_world_sounds<S: Component + AudioSinkPlayback>(instances: Query<(&SoundInstance, &S)>) {
    for (instance, sink) in &instances {
        if instance.world {
            sink.play();
        }
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
    fn мелодичные_звуки_без_разброса_высоты() {
        assert_eq!(pitch_spread("upgrade"), (1.0, 1.0));
        assert_eq!(pitch_spread("meme_womp"), (1.0, 1.0));
        assert_eq!(pitch_spread("explosion"), SPEED_SPREAD);
    }

    #[test]
    fn world_sounds_share_a_total_limit() {
        assert!(has_room("explosion", 0, TOTAL_LIMIT - 1));
        assert!(!has_room("explosion", 0, TOTAL_LIMIT));
        assert!(!has_room("explosion", limit_for("explosion"), 0));
        // Интерфейс звучит и при полном мире.
        assert!(has_room("ui_click", 0, TOTAL_LIMIT));
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
