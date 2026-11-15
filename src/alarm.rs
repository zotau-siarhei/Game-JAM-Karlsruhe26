//! Тревога: насколько встревожены жители. Растёт от каждого разрушенного дома, кто бы его ни
//! разрушил, и спадает в затишье. Управляет только Службой защиты жителей, волны от неё не
//! зависят: волнами управляет угроза из `src/waves.rs`.
//!
//! Пять ступеней по 10 очков. Ступень — сколько десятков очков начато: 6 очков — первая ступень.
//! Индикатор — пять ромбов под полосой здоровья, белые с оранжевым, без синего и красного.

use bevy::prelude::*;
use bevy::ui::UiTransform;

use crate::buildings::BuildingDestroyed;
use crate::frame::GameSet;
use crate::run::{GameState, WorldRebuilt};

/// Числа тревоги.
pub mod balance {
    /// Число ступеней.
    pub const LEVELS: u8 = 5;
    /// Очков на ступень.
    pub const POINTS_PER_LEVEL: f32 = 10.0;
    /// Предел очков.
    pub const MAX_POINTS: f32 = LEVELS as f32 * POINTS_PER_LEVEL;
    /// Очков за разрушенный дом.
    pub const HOUSE_POINTS: f32 = 6.0;
    /// Спад: столько очков…
    pub const DECAY_POINTS: f32 = 5.0;
    /// …за столько секунд без новых очков.
    pub const DECAY_PERIOD: f32 = 10.0;
    /// Пока затишье короче этого, спад не опускает ниже начала текущей ступени, с.
    pub const HOLD_LEVEL: f32 = 15.0;
}

/// Длительность вспышки индикатора при росте ступени, с.
const FLASH_SECONDS: f32 = 0.6;

/// Размер ромба, px.
const DIAMOND: f32 = 14.0;

/// Отступ индикатора от левого края и сверху: под полосой здоровья, px.
const POSITION: Vec2 = Vec2::new(16.0, 48.0);

/// Оранжевый заполненной ступени.
const LIT: Color = Color::srgb(1.0, 0.55, 0.1);

/// Пустая ступень: тусклый белый.
const DIM: Color = Color::srgba(1.0, 1.0, 1.0, 0.25);

/// Вспышка: почти белый.
const FLASH: Color = Color::srgb(1.0, 0.95, 0.8);

/// Подложка индикатора.
const PANEL_COLOR: Color = Color::srgba(0.0, 0.0, 0.0, 0.65);

/// Подключает тревогу и её индикатор.
pub struct AlarmPlugin;

impl Plugin for AlarmPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Alarm>()
            .add_systems(Startup, spawn_indicator)
            .add_systems(
                Update,
                (
                    reset_alarm.in_set(GameSet::Reset),
                    decay_alarm.in_set(GameSet::Simulation),
                    raise_on_destruction.in_set(GameSet::Death),
                    show_alarm.in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Тревога забега.
#[derive(Resource, Default, Debug)]
pub struct Alarm {
    points: f32,
    /// Сколько секунд не было новых очков.
    quiet: f32,
    /// Сколько секунд накоплено к следующему шагу спада.
    decay_timer: f32,
}

impl Alarm {
    /// Добавить очки тревоги: обрезается пределом, затишье начинается заново.
    pub fn add(&mut self, points: f32) {
        self.points = (self.points + points).clamp(0.0, balance::MAX_POINTS);
        self.quiet = 0.0;
        self.decay_timer = 0.0;
    }

    /// Ступень тревоги, от 0 до [`balance::LEVELS`].
    pub fn level(&self) -> u8 {
        level_of(self.points)
    }

    /// Ход времени без новых очков.
    pub fn tick(&mut self, dt: f32) {
        self.quiet += dt;
        self.decay_timer += dt;
        while self.decay_timer >= balance::DECAY_PERIOD {
            self.decay_timer -= balance::DECAY_PERIOD;
            let floor = if self.quiet < balance::HOLD_LEVEL {
                level_start(self.level())
            } else {
                0.0
            };
            self.points = (self.points - balance::DECAY_POINTS).max(floor.min(self.points));
        }
    }
}

/// Ступень для числа очков: сколько десятков начато.
fn level_of(points: f32) -> u8 {
    ((points / balance::POINTS_PER_LEVEL).ceil() as u8).min(balance::LEVELS)
}

/// Наименьшее число очков, при котором ещё держится ступень `level`.
fn level_start(level: u8) -> f32 {
    if level == 0 {
        0.0
    } else {
        (level - 1) as f32 * balance::POINTS_PER_LEVEL + 0.01
    }
}

fn reset_alarm(mut started: MessageReader<WorldRebuilt>, mut alarm: ResMut<Alarm>) {
    if started.read().count() > 0 {
        *alarm = Alarm::default();
    }
}

/// Каждый разрушенный дом поднимает тревогу, кто бы его ни разрушил.
fn raise_on_destruction(mut destroyed: MessageReader<BuildingDestroyed>, mut alarm: ResMut<Alarm>) {
    for _ in destroyed.read() {
        alarm.add(balance::HOUSE_POINTS);
    }
}

fn decay_alarm(time: Res<Time>, mut alarm: ResMut<Alarm>) {
    alarm.tick(time.delta_secs());
}

/// Корень индикатора.
#[derive(Component)]
struct AlarmIndicator {
    /// Ступень, которую индикатор показывает сейчас.
    shown: u8,
    /// Сколько ещё длится вспышка, с.
    flash: f32,
}

/// Ромб ступени `0` (номер с нуля).
#[derive(Component)]
struct AlarmDiamond(u8);

fn spawn_indicator(mut commands: Commands) {
    commands
        .spawn((
            AlarmIndicator { shown: 0, flash: 0.0 },
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(POSITION.x),
                top: Val::Px(POSITION.y),
                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                column_gap: Val::Px(10.0),
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(PANEL_COLOR),
            Visibility::Hidden,
        ))
        .with_children(|row| {
            row.spawn((
                Text::new("ALARM"),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
            for i in 0..balance::LEVELS {
                row.spawn((
                    AlarmDiamond(i),
                    Node {
                        width: Val::Px(DIAMOND),
                        height: Val::Px(DIAMOND),
                        ..default()
                    },
                    UiTransform::from_rotation(Rot2::degrees(45.0)),
                    BackgroundColor(DIM),
                ));
            }
        });
}

/// Обновляет ромбы, вспыхивает при росте ступени и прячет индикатор в меню.
fn show_alarm(
    time: Res<Time>,
    alarm: Res<Alarm>,
    state: Res<State<GameState>>,
    mut indicator: Query<(&mut AlarmIndicator, &mut Visibility)>,
    mut diamonds: Query<(&AlarmDiamond, &mut BackgroundColor, &mut UiTransform)>,
) {
    let Ok((mut indicator, mut visibility)) = indicator.single_mut() else {
        return;
    };
    let wanted = if *state.get() == GameState::Menu {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    if *visibility != wanted {
        *visibility = wanted;
    }
    let level = alarm.level();
    if level > indicator.shown {
        indicator.flash = FLASH_SECONDS;
    }
    indicator.shown = level;
    indicator.flash = (indicator.flash - time.delta_secs()).max(0.0);
    let flash = indicator.flash / FLASH_SECONDS;
    for (diamond, mut color, mut transform) in &mut diamonds {
        let lit = diamond.0 < level;
        let wanted = if !lit {
            DIM
        } else if flash > 0.0 {
            LIT.mix(&FLASH, flash)
        } else {
            LIT
        };
        if color.0 != wanted {
            color.0 = wanted;
        }
        let scale = if lit { 1.0 + 0.5 * flash } else { 1.0 };
        if transform.scale.x != scale {
            transform.scale = Vec2::splat(scale);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Затишье длиной `seconds` кадрами по 0,1 с, как в игре.
    fn wait(alarm: &mut Alarm, seconds: f32) {
        for _ in 0..(seconds * 10.0).round() as u32 {
            alarm.tick(0.1);
        }
    }

    #[test]
    fn destroyed_house_raises_alarm() {
        let mut alarm = Alarm::default();
        assert_eq!(alarm.level(), 0);
        alarm.add(balance::HOUSE_POINTS);
        assert_eq!(alarm.level(), 1);
        alarm.add(balance::HOUSE_POINTS);
        assert_eq!(alarm.level(), 2);
    }

    #[test]
    fn alarm_does_not_exceed_fifth_level() {
        let mut alarm = Alarm::default();
        for _ in 0..30 {
            alarm.add(balance::HOUSE_POINTS);
        }
        assert_eq!(alarm.level(), balance::LEVELS);
        assert!(alarm.points <= balance::MAX_POINTS);
        // С предела тревога спадает по одной ступени.
        wait(&mut alarm, 25.0);
        assert_eq!(alarm.level(), balance::LEVELS - 1);
    }

    #[test]
    fn alarm_decays_after_quiet() {
        let mut alarm = Alarm::default();
        for _ in 0..4 {
            alarm.add(balance::HOUSE_POINTS);
        }
        // 24 очка — третья ступень.
        assert_eq!(alarm.level(), 3);
        // Первые 15 с ступень держится.
        wait(&mut alarm, balance::HOLD_LEVEL - 1.0);
        assert_eq!(alarm.level(), 3);
        // Дальше опускается ступень за ступенью до нуля.
        let mut levels = vec![alarm.level()];
        for _ in 0..200 {
            wait(&mut alarm, 1.0);
            if *levels.last().unwrap() != alarm.level() {
                levels.push(alarm.level());
            }
        }
        assert_eq!(levels, vec![3, 2, 1, 0]);
    }

    #[test]
    fn new_destruction_restarts_quiet() {
        let mut alarm = Alarm::default();
        alarm.add(balance::HOUSE_POINTS);
        alarm.add(balance::HOUSE_POINTS);
        wait(&mut alarm, balance::HOLD_LEVEL - 1.0);
        alarm.add(balance::HOUSE_POINTS);
        // Новое затишье снова держит ступень первые 15 с.
        wait(&mut alarm, balance::HOLD_LEVEL - 1.0);
        assert_eq!(alarm.level(), 2);
    }
}
