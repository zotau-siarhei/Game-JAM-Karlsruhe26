//! Интро перед первым забегом и обучающие реплики штаба.
//!
//! Интро — цепочка реплик `intro_1` … `intro_5` на стартовом экране. Любая клавиша или кнопка
//! мыши его обрывает, и это нажатие больше ничего не делает: забег начинается следующим.
//! За время работы приложения интро звучит один раз.
//!
//! Обучение: механика при первом появлении в забеге отправляет [`Tutorial`]. Отправители
//! собраны здесь же, в [`send_tutorials`]:
//!
//! ```ignore
//! fn on_first_pickup(mut tutorial: MessageWriter<Tutorial>) {
//!     tutorial.write(Tutorial::new("tut_pickup"));
//! }
//! ```
//!
//! Повторные сообщения о той же механике в забеге игнорируются, поэтому отправлять можно
//! при каждом событии. Реплика ждёт, пока закончится интро и штаб замолчит, и не теряется.
//! Объяснённое забывается при перестройке мира.

use std::collections::{HashSet, VecDeque};

use bevy::input::InputSystems;
use bevy::prelude::*;

use crate::buildings::BuildingDestroyed;
use crate::combat::{Health, Team};
use crate::enemy::Enemy;
use crate::explosion::Explosion;
use crate::frame::GameSet;
use crate::hq::VoiceCue;
use crate::mortar::Shell;
use crate::pickups::Pickup;
use crate::rover::Rover;
use crate::run::{GameState, WorldRebuilt};
use crate::voice::{PlayVoice, VoiceLines, VoicePlayback};
use crate::weapons::CurrentWeapon;

/// Реплики интро по порядку.
const INTRO_LINES: [&str; 5] = ["intro_1", "intro_2", "intro_3", "intro_4", "intro_5"];

/// Сколько секунд ждать появления звучащей реплики. Если её нет — файл не озвучен, идём дальше.
const START_GRACE: f64 = 0.5;

/// Доля здоровья лунохода, ниже которой штаб объясняет ремонт.
const REPAIR_HINT_HEALTH: f32 = 0.7;

/// Подключает интро и обучение.
pub struct IntroPlugin;

impl Plugin for IntroPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Intro>()
            .init_resource::<TutorialMemory>()
            .add_message::<Tutorial>()
            // Пропуск — сразу после чтения ввода, до экранов: нажатие, оборвавшее интро,
            // меню уже не видит и забег им не начинает.
            .add_systems(PreUpdate, skip_intro.after(InputSystems))
            .add_systems(Update, forget_tutorials.in_set(GameSet::Reset))
            .add_systems(
                Update,
                (
                    run_intro,
                    send_tutorials.run_if(in_state(GameState::Playing)),
                    remember_tutorials,
                    speak_tutorials.run_if(in_state(GameState::Playing)),
                )
                    .chain()
                    .in_set(GameSet::Presentation),
            );
    }
}

/// Механика впервые появилась в забеге: основа реплики обучения, например `tut_mortar`.
#[derive(Message, Debug, Clone)]
pub struct Tutorial(pub String);

impl Tutorial {
    pub fn new(base: impl Into<String>) -> Self {
        Self(base.into())
    }
}

/// Ход интро.
#[derive(Resource, Default)]
pub struct Intro {
    /// Сколько реплик интро уже отправлено.
    next: usize,
    /// Когда отправлена последняя реплика.
    sent_at: Option<f64>,
    /// Звучала ли отправленная реплика.
    started: bool,
    /// Интро закончилось или пропущено.
    finished: bool,
}

impl Intro {
    /// Идёт ли интро сейчас.
    pub fn active(&self) -> bool {
        !self.finished
    }

    /// Решает, отправлять ли следующую реплику. `playing` — звучит ли реплика интро сейчас.
    fn step(&mut self, now: f64, playing: bool) -> Option<&'static str> {
        if self.finished {
            return None;
        }
        if let Some(sent) = self.sent_at {
            if playing {
                self.started = true;
                return None;
            }
            // Ждём, пока отправленная реплика начнёт звучать.
            if !self.started && now - sent < START_GRACE {
                return None;
            }
        }
        let Some(id) = INTRO_LINES.get(self.next) else {
            self.finished = true;
            return None;
        };
        self.next += 1;
        self.sent_at = Some(now);
        self.started = false;
        Some(id)
    }
}

/// Какие механики уже объяснены в этом забеге и какие ждут очереди.
#[derive(Resource, Default)]
pub struct TutorialMemory {
    seen: HashSet<String>,
    pending: VecDeque<String>,
}

impl TutorialMemory {
    /// Забывает объяснённое. Вызывается при перестройке мира.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Запоминает механику. Новая встаёт в очередь, уже виденная игнорируется.
    fn remember(&mut self, base: &str) {
        if self.seen.insert(base.to_string()) {
            self.pending.push_back(base.to_string());
        }
    }
}

/// Принадлежит ли реплика `id` поводу `base`.
fn belongs_to(id: &str, base: &str) -> bool {
    id.strip_prefix(base)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('_'))
}

/// Обрывает интро по любой клавише или кнопке мыши. Нажатие гасится: меню его не видит и забег
/// не начинает. Esc не гасится — он по-прежнему закрывает игру.
fn skip_intro(
    mut commands: Commands,
    mut intro: ResMut<Intro>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    playing: Query<(Entity, &VoicePlayback)>,
) {
    if intro.finished {
        return;
    }
    if keys.get_just_pressed().next().is_none() && mouse.get_just_pressed().next().is_none() {
        return;
    }
    intro.finished = true;
    if !keys.just_pressed(KeyCode::Escape) {
        keys.clear();
    }
    mouse.clear();
    for (entity, playback) in &playing {
        if belongs_to(&playback.id, "intro") {
            commands.entity(entity).despawn();
        }
    }
    info!("Интро пропущено");
}

/// Проигрывает реплики интро одну за другой.
fn run_intro(
    mut intro: ResMut<Intro>,
    mut play: MessageWriter<PlayVoice>,
    time: Res<Time<Real>>,
    playing: Query<&VoicePlayback>,
) {
    let intro_playing = playing
        .iter()
        .any(|playback| belongs_to(&playback.id, "intro"));
    if let Some(id) = intro.step(time.elapsed_secs_f64(), intro_playing) {
        play.write(PlayVoice::new(id));
    }
}

/// Отправители обучения: первая встреча с механикой в забеге.
///
/// - первый враг — как стрелять;
/// - первый выстрел мортиры игрока — как она бьёт по точке;
/// - первый взрыв игрока — кратер как укрытие;
/// - здоровье ниже [`REPAIR_HINT_HEALTH`] — ремонт;
/// - первый ящик или трофей — подбор;
/// - первый разрушенный дом — тревога.
#[allow(clippy::too_many_arguments)]
fn send_tutorials(
    enemies: Query<(), Added<Enemy>>,
    shells: Query<(), Added<Shell>>,
    pickups: Query<(), Added<Pickup>>,
    rover: Query<&Health, With<Rover>>,
    weapon: Res<CurrentWeapon>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut explosions: MessageReader<Explosion>,
    mut destroyed: MessageReader<BuildingDestroyed>,
    mut tutorial: MessageWriter<Tutorial>,
) {
    if !enemies.is_empty() {
        tutorial.write(Tutorial::new("tut_shoot"));
    }
    // Снаряд игрока: мортира выбрана и спуск нажат. Снаряды врагов летят и без этого.
    if !shells.is_empty() && *weapon == CurrentWeapon::Mortar && buttons.pressed(MouseButton::Left) {
        tutorial.write(Tutorial::new("tut_mortar"));
    }
    if explosions.read().any(|explosion| explosion.source_team == Team::Player) {
        tutorial.write(Tutorial::new("first_crater"));
    }
    if rover
        .iter()
        .any(|health| !health.is_dead() && health.fraction() < REPAIR_HINT_HEALTH)
    {
        tutorial.write(Tutorial::new("tut_repair"));
    }
    if !pickups.is_empty() {
        tutorial.write(Tutorial::new("tut_pickup"));
    }
    if destroyed.read().count() > 0 {
        tutorial.write(Tutorial::new("tut_alarm"));
    }
}

/// Мир перестроен: в новом забеге механики объясняются заново.
fn forget_tutorials(mut rebuilt: MessageReader<WorldRebuilt>, mut memory: ResMut<TutorialMemory>) {
    if rebuilt.read().last().is_some() {
        memory.reset();
    }
}

fn remember_tutorials(mut tutorials: MessageReader<Tutorial>, mut memory: ResMut<TutorialMemory>) {
    for Tutorial(base) in tutorials.read() {
        memory.remember(base);
    }
}

/// Предлагает штабу первую механику из очереди, пока он её не произнесёт.
fn speak_tutorials(
    intro: Res<Intro>,
    mut memory: ResMut<TutorialMemory>,
    mut cues: MessageWriter<VoiceCue>,
    lines: Res<VoiceLines>,
    playing: Query<&VoicePlayback>,
) {
    if intro.active() {
        return;
    }
    let Some(base) = memory.pending.front().cloned() else {
        return;
    };
    if lines.group(&base).next().is_none() {
        warn!("Обучение: нет озвученной реплики «{base}»");
        memory.pending.pop_front();
        return;
    }
    if playing.iter().any(|playback| belongs_to(&playback.id, &base)) {
        memory.pending.pop_front();
        return;
    }
    // Штаб может отбросить повод, пока говорит: повторяем каждый кадр.
    cues.write(VoiceCue::new(base));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intro_plays_lines_in_order() {
        let mut intro = Intro::default();
        let mut now = 0.0;
        for expected in INTRO_LINES {
            assert_eq!(intro.step(now, false), Some(expected));
            // Реплика звучит две секунды.
            assert_eq!(intro.step(now + 0.1, true), None);
            assert_eq!(intro.step(now + 2.0, true), None);
            now += 2.1;
        }
        assert_eq!(intro.step(now, false), None);
        assert!(!intro.active());
    }

    #[test]
    fn intro_waits_for_line_to_start() {
        let mut intro = Intro::default();
        assert_eq!(intro.step(0.0, false), Some("intro_1"));
        // Звук ещё грузится: следующую реплику не отправляем.
        assert_eq!(intro.step(0.2, false), None);
        assert_eq!(intro.step(0.4, true), None);
        assert_eq!(intro.step(3.0, false), Some("intro_2"));
    }

    #[test]
    fn missing_intro_line_is_skipped() {
        let mut intro = Intro::default();
        assert_eq!(intro.step(0.0, false), Some("intro_1"));
        assert_eq!(intro.step(START_GRACE + 0.1, false), Some("intro_2"));
    }

    #[test]
    fn tutorial_is_queued_once_per_run() {
        let mut memory = TutorialMemory::default();
        memory.remember("tut_mortar");
        memory.remember("tut_mortar");
        memory.remember("tut_shoot");
        assert_eq!(memory.pending, ["tut_mortar", "tut_shoot"]);

        memory.pending.clear();
        memory.remember("tut_mortar");
        assert!(memory.pending.is_empty());

        memory.reset();
        memory.remember("tut_mortar");
        assert_eq!(memory.pending, ["tut_mortar"]);
    }

    #[test]
    fn variant_belongs_to_its_cue() {
        assert!(belongs_to("tut_mortar", "tut_mortar"));
        assert!(belongs_to("tut_mortar_2", "tut_mortar"));
        assert!(belongs_to("intro_3", "intro"));
        assert!(!belongs_to("tut_mortars", "tut_mortar"));
    }
}
