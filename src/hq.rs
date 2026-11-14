//! Отбор реплик: какой повод озвучить, какой вариант взять, когда промолчать. Субтитры.
//!
//! Игра сообщает повод, а не конкретную реплику:
//!
//! ```ignore
//! fn on_house_destroyed(mut cues: MessageWriter<VoiceCue>) {
//!     cues.write(VoiceCue::new("house_destroyed"));
//! }
//! ```
//!
//! Повод — основа идентификатора в `assets/voice/lines.json`. Варианты повода — записи с
//! суффиксами: `house_destroyed_2`, `house_destroyed_t3`. Модуль выбирает вариант по тону штаба
//! [`HqTone`] и отправляет [`PlayVoice`]. Поводы с приоритетом 3 обрывают текущую реплику,
//! остальные отбрасываются, пока штаб говорит и несколько секунд после.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::sound_list::{Dice, SoundEntry};
use crate::voice::{PlayVoice, VoiceLines, VoicePlayback};

/// Приоритет, с которого повод обрывает текущую реплику.
const IMPORTANT_PRIORITY: u8 = 3;

/// Сколько секунд после реплики обычные поводы отбрасываются.
const QUIET_AFTER_LINE: f64 = 4.0;

/// Сколько секунд после отправки реплика считается звучащей, пока не появилась её сущность.
const SEND_GRACE: f64 = 0.3;

/// Поводы с приоритетом 0 — мемы и мелочи: не чаще одного в столько секунд.
const MINOR_GAP: f64 = 10.0;

/// Один и тот же повод с приоритетом 0 — не чаще раза в столько секунд.
const MINOR_SAME_GAP: f64 = 30.0;

/// Подключает отбор реплик и субтитры.
pub struct HqPlugin;

impl Plugin for HqPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HqTone>()
            .init_resource::<HqMemory>()
            .init_resource::<Dice>()
            .add_message::<VoiceCue>()
            .add_systems(Startup, spawn_subtitle)
            .add_systems(Update, (select_voice, update_subtitle));
    }
}

/// Повод для реплики: основа идентификатора в `lines.json`, например `house_destroyed`.
#[derive(Message, Debug, Clone)]
pub struct VoiceCue(pub String);

impl VoiceCue {
    // Игровых событий, которые сообщают поводы, пока нет.
    #[allow(dead_code)]
    pub fn new(base: impl Into<String>) -> Self {
        Self(base.into())
    }
}

/// Тон штаба: 1 вежливо, 2 беспокойство, 3 паника. Растёт с объёмом разрушений.
#[derive(Resource)]
pub struct HqTone(pub u8);

impl Default for HqTone {
    fn default() -> Self {
        Self(1)
    }
}

/// Что штаб уже говорил: для запрета повторов и пауз между репликами.
#[derive(Resource, Default)]
pub struct HqMemory {
    /// Последний сыгранный вариант каждого повода.
    last_variant: HashMap<String, String>,
    /// Когда повод с приоритетом 0 звучал в последний раз.
    last_minor: HashMap<String, f64>,
    gate: Gate,
}

impl HqMemory {
    // Сброс вызывается при новом забеге, когда появятся экраны.
    #[allow(dead_code)]
    /// Забывает всё сказанное. Вызывается при новом забеге.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Решает, пропустить ли повод, по времени и тому, говорит ли штаб.
#[derive(Default)]
struct Gate {
    /// До этого момента обычные поводы отбрасываются.
    quiet_until: f64,
    /// Когда отправлена последняя реплика.
    last_sent: Option<f64>,
    /// Когда звучал последний повод с приоритетом 0.
    last_minor_any: Option<f64>,
    /// Звучала ли реплика в прошлом кадре.
    was_playing: bool,
}

impl Gate {
    /// Обновляет состояние по тому, звучит ли реплика сейчас.
    fn observe(&mut self, now: f64, playing: bool) {
        if self.was_playing && !playing {
            self.quiet_until = now + QUIET_AFTER_LINE;
        }
        self.was_playing = playing;
    }

    /// Говорит ли штаб: реплика звучит или только что отправлена.
    fn busy(&self, now: f64, playing: bool) -> bool {
        playing || self.last_sent.is_some_and(|sent| now - sent < SEND_GRACE)
    }

    /// Пропускает ли повод с приоритетом `priority`.
    fn allows(&self, now: f64, playing: bool, priority: u8) -> bool {
        if priority >= IMPORTANT_PRIORITY {
            return true;
        }
        if self.busy(now, playing) || now < self.quiet_until {
            return false;
        }
        if priority == 0 && self.last_minor_any.is_some_and(|last| now - last < MINOR_GAP) {
            return false;
        }
        true
    }

    fn sent(&mut self, now: f64, priority: u8) {
        self.last_sent = Some(now);
        if priority == 0 {
            self.last_minor_any = Some(now);
        }
    }
}

/// Выбирает вариант реплики по тону.
///
/// Берутся варианты самого высокого тона не выше текущего и варианты без тона. Если таких нет —
/// любые. Вариант, сыгранный последним, не повторяется, если есть другие. `roll` — случайное число.
fn pick_variant<'a>(
    variants: &[&'a SoundEntry],
    tone: u8,
    last: Option<&str>,
    roll: usize,
) -> Option<&'a SoundEntry> {
    let level = variants
        .iter()
        .filter_map(|entry| entry.tone)
        .filter(|entry_tone| *entry_tone <= tone)
        .max();
    let mut pool: Vec<&SoundEntry> = variants
        .iter()
        .copied()
        .filter(|entry| entry.tone.is_none() || entry.tone == level)
        .collect();
    if pool.is_empty() {
        pool = variants.to_vec();
    }
    if pool.len() > 1 {
        pool.retain(|entry| Some(entry.id.as_str()) != last);
    }
    if pool.is_empty() {
        return None;
    }
    Some(pool[roll % pool.len()])
}

/// Читает поводы и отправляет не больше одной реплики за кадр.
fn select_voice(
    mut cues: MessageReader<VoiceCue>,
    mut play: MessageWriter<PlayVoice>,
    lines: Res<VoiceLines>,
    tone: Res<HqTone>,
    mut memory: ResMut<HqMemory>,
    mut dice: ResMut<Dice>,
    time: Res<Time<Real>>,
    playing: Query<(), With<VoicePlayback>>,
) {
    let now = time.elapsed_secs_f64();
    let is_playing = !playing.is_empty();
    memory.gate.observe(now, is_playing);

    // Из поводов кадра остаётся самый важный, остальные отбрасываются.
    let mut best: Option<(&str, u8, Vec<&SoundEntry>)> = None;
    for VoiceCue(base) in cues.read() {
        let variants: Vec<&SoundEntry> = lines.group(base).collect();
        let Some(priority) = variants.iter().map(|entry| entry.pri).max() else {
            debug!("Штаб: нет озвученной реплики на повод «{base}»");
            continue;
        };
        if best.as_ref().is_none_or(|(_, top, _)| priority > *top) {
            best = Some((base, priority, variants));
        }
    }
    let Some((base, priority, variants)) = best else {
        return;
    };

    if !memory.gate.allows(now, is_playing, priority) {
        return;
    }
    if priority == 0
        && memory
            .last_minor
            .get(base)
            .is_some_and(|last| now - last < MINOR_SAME_GAP)
    {
        return;
    }

    let last = memory.last_variant.get(base).map(String::as_str);
    let Some(entry) = pick_variant(&variants, tone.0, last, dice.below(usize::MAX)) else {
        return;
    };
    play.write(PlayVoice::new(entry.id.clone()));
    memory
        .last_variant
        .insert(base.to_string(), entry.id.clone());
    if priority == 0 {
        memory.last_minor.insert(base.to_string(), now);
    }
    memory.gate.sent(now, priority);
}

/// Метка узла субтитра.
#[derive(Component)]
struct Subtitle;

/// Метка текста внутри субтитра.
#[derive(Component)]
struct SubtitleText;

fn spawn_subtitle(mut commands: Commands) {
    commands
        .spawn((
            Subtitle,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(110.0),
                left: Val::Percent(15.0),
                right: Val::Percent(15.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_children(|parent| {
            parent.spawn((
                SubtitleText,
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(22.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                TextLayout::justify(Justify::Center),
                Node {
                    padding: UiRect::axes(Val::Px(14.0), Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.72)),
            ));
        });
}

/// Имя говорящего для субтитра.
fn speaker_name(speaker: &str) -> &'static str {
    match speaker {
        "bolt" => "BOLT",
        _ => "HQ",
    }
}

/// Показывает текст звучащей реплики и прячет субтитр, когда реплика закончилась.
fn update_subtitle(
    lines: Res<VoiceLines>,
    playing: Query<&VoicePlayback>,
    mut subtitle: Query<&mut Visibility, With<Subtitle>>,
    mut text: Query<&mut Text, With<SubtitleText>>,
    mut shown: Local<Option<String>>,
) {
    let current = playing.iter().next().map(|playback| playback.id.clone());
    if *shown == current {
        return;
    }
    *shown = current.clone();

    let line = current
        .as_deref()
        .and_then(|id| lines.get(id))
        .filter(|entry| entry.kind != "sfx" && !entry.text.is_empty());
    let (Ok(mut visibility), Ok(mut text)) = (subtitle.single_mut(), text.single_mut()) else {
        return;
    };
    match line {
        Some(entry) => {
            text.0 = format!("{}: {}", speaker_name(&entry.speaker), entry.text);
            *visibility = Visibility::Visible;
        }
        None => *visibility = Visibility::Hidden,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, tone: Option<u8>) -> SoundEntry {
        SoundEntry {
            id: id.into(),
            tone,
            ..default()
        }
    }

    #[test]
    fn three_cues_in_a_second_give_one_line() {
        let mut gate = Gate::default();
        let mut sent = 0;
        for now in [10.0, 10.4, 10.9] {
            // Реплика длится дольше секунды: после первой отправки штаб говорит.
            let playing = sent > 0;
            gate.observe(now, playing);
            if gate.allows(now, playing, 1) {
                gate.sent(now, 1);
                sent += 1;
            }
        }
        assert_eq!(sent, 1);
    }

    #[test]
    fn cue_right_after_send_is_dropped() {
        let mut gate = Gate::default();
        gate.sent(5.0, 1);
        // Сущность реплики ещё не появилась, но повод уже отбрасывается.
        assert!(!gate.allows(5.1, false, 1));
    }

    #[test]
    fn important_cue_passes_during_a_line() {
        let mut gate = Gate::default();
        gate.observe(1.0, true);
        assert!(!gate.allows(1.0, true, 2));
        assert!(gate.allows(1.0, true, IMPORTANT_PRIORITY));
    }

    #[test]
    fn normal_cue_waits_after_a_line() {
        let mut gate = Gate::default();
        gate.observe(1.0, true);
        gate.observe(2.0, false);
        assert!(!gate.allows(2.0 + QUIET_AFTER_LINE - 0.1, false, 1));
        assert!(gate.allows(2.0 + QUIET_AFTER_LINE + 0.1, false, 1));
    }

    #[test]
    fn minor_cues_are_rare() {
        let mut gate = Gate::default();
        assert!(gate.allows(100.0, false, 0));
        gate.sent(100.0, 0);
        assert!(!gate.allows(100.0 + MINOR_GAP - 1.0, false, 0));
        assert!(gate.allows(100.0 + MINOR_GAP + 1.0, false, 0));
        // Обычный повод пауза между мелочами не задерживает.
        assert!(gate.allows(100.0 + 1.0, false, 1));
    }

    #[test]
    fn variant_follows_tone() {
        let calm = entry("house_destroyed", Some(1));
        let worried = entry("house_destroyed_t2", Some(2));
        let panic = entry("house_destroyed_t3", Some(3));
        let variants = [&calm, &worried, &panic];
        for (tone, expected) in [(1, "house_destroyed"), (2, "house_destroyed_t2"), (3, "house_destroyed_t3")] {
            for roll in 0..5 {
                assert_eq!(pick_variant(&variants, tone, None, roll).unwrap().id, expected);
            }
        }
    }

    #[test]
    fn without_tone_variants_calmer_or_base_is_used() {
        let base = entry("train_incoming", Some(1));
        assert_eq!(pick_variant(&[&base], 3, None, 0).unwrap().id, "train_incoming");

        let calm = entry("alarm_rising", Some(1));
        let worried = entry("alarm_rising_t2", Some(2));
        let picked = pick_variant(&[&calm, &worried], 3, None, 1).unwrap();
        assert_eq!(picked.id, "alarm_rising_t2");

        // Есть только более тревожная реплика: берётся она.
        let only_worried = entry("resident_died", Some(2));
        assert_eq!(pick_variant(&[&only_worried], 1, None, 0).unwrap().id, "resident_died");
    }

    #[test]
    fn toneless_variants_fit_any_tone() {
        let hq = entry("boss_appears", Some(2));
        let radio = entry("boss_appears_2", None);
        // На спокойном тоне реплики штаба нет, остаётся радио лунохода.
        assert_eq!(pick_variant(&[&hq, &radio], 1, None, 0).unwrap().id, "boss_appears_2");
        // На тревожном годятся обе.
        let ids: Vec<_> = (0..2)
            .map(|roll| pick_variant(&[&hq, &radio], 2, None, roll).unwrap().id.clone())
            .collect();
        assert!(ids.contains(&"boss_appears".to_string()));
        assert!(ids.contains(&"boss_appears_2".to_string()));
    }

    #[test]
    fn variant_is_not_repeated_twice_in_a_row() {
        let first = entry("kill_1", None);
        let second = entry("kill_2", None);
        let variants = [&first, &second];
        let mut last: Option<String> = None;
        for roll in 0..20 {
            let picked = pick_variant(&variants, 1, last.as_deref(), roll).unwrap();
            assert_ne!(Some(picked.id.clone()), last);
            last = Some(picked.id.clone());
        }
        // Единственный вариант повторяется: других нет.
        assert_eq!(pick_variant(&[&first], 1, Some("kill_1"), 0).unwrap().id, "kill_1");
    }

    #[test]
    fn no_variants_gives_nothing() {
        assert!(pick_variant(&[], 1, None, 0).is_none());
    }
}
