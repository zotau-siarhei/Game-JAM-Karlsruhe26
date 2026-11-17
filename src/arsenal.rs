//! Арсенал: что есть у лунохода, сколько осталось и какое оружие выбрано, плюс панель
//! оружия внизу экрана.
//!
//! Вид оружия — [`CurrentWeapon`]: он же выбран сейчас. Боезапас — ресурс [`Inventory`].
//! Оружие перед выстрелом проверяет `inventory.usable(вид)`, после выстрела зовёт
//! `inventory.spend(вид)`. Ящики и трофеи пополняют запас через `inventory.add`.

use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;

use crate::frame::{GameSet, RunMessages};
use crate::run::{GameState, WorldRebuilt};
use crate::upgrades::{MAX_LEVEL, WeaponLevels, WeaponUpgraded};
use crate::weapons::CurrentWeapon;

/// Пауза после переключения колесом, с: одна прокрутка растянута на несколько кадров.
const SCROLL_COOLDOWN: f32 = 0.25;

/// Сколько мигает ячейка, когда выбрали пустое оружие, с.
const DENIED_SECONDS: f32 = 0.4;

/// Сколько вспыхивает новая ячейка, с.
const NEW_CELL_SECONDS: f32 = 0.6;

/// Сколько вспыхивает ячейка улучшенного оружия, с.
const UPGRADE_FLASH_SECONDS: f32 = 1.2;

/// Параметры вида оружия.
pub struct WeaponSpec {
    pub key: KeyCode,
    /// Буква на значке.
    pub icon: &'static str,
    pub color: Color,
    /// Предел боезапаса. У пулемёта запас бесконечный.
    pub limit: u16,
    /// Сколько даёт ящик.
    pub per_crate: u16,
    /// Сколько есть в начале забега. `None` — вида нет.
    pub start: Option<u16>,
}

/// Таблица оружия по порядку клавиш: всё для подбора баланса в одном месте.
pub fn spec(weapon: CurrentWeapon) -> WeaponSpec {
    match weapon {
        CurrentWeapon::MachineGun => WeaponSpec {
            key: KeyCode::Digit1,
            icon: "MG",
            color: Color::srgb(0.85, 0.85, 0.85),
            limit: u16::MAX,
            per_crate: 0,
            start: Some(u16::MAX),
        },
        CurrentWeapon::Mortar => WeaponSpec {
            key: KeyCode::Digit2,
            icon: "M",
            color: Color::srgb(0.95, 0.65, 0.2),
            limit: 150,
            per_crate: 6,
            start: Some(24),
        },
        CurrentWeapon::Rockets => WeaponSpec {
            key: KeyCode::Digit3,
            icon: "R",
            color: Color::srgb(0.95, 0.85, 0.25),
            limit: 150,
            per_crate: 10,
            start: None,
        },
        CurrentWeapon::HomingRockets => WeaponSpec {
            key: KeyCode::Digit4,
            icon: "H",
            color: Color::srgb(0.55, 0.9, 0.35),
            limit: 30,
            per_crate: 2,
            start: None,
        },
        CurrentWeapon::Laser => WeaponSpec {
            key: KeyCode::Digit5,
            icon: "L",
            color: Color::srgb(0.35, 0.85, 1.0),
            limit: 60,
            per_crate: 4,
            start: None,
        },
        CurrentWeapon::Airstrike => WeaponSpec {
            key: KeyCode::Digit6,
            icon: "A",
            color: Color::srgb(0.75, 0.5, 1.0),
            limit: 15,
            per_crate: 1,
            start: None,
        },
    }
}

/// Подключает инвентарь, выбор оружия и панель.
pub struct ArsenalPlugin;

impl Plugin for ArsenalPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Inventory>()
            .add_run_message::<WeaponDenied>()
            .add_run_message::<AmmoRanOut>()
            .add_systems(Startup, spawn_panel)
            .add_systems(
                Update,
                (
                    reset_inventory.in_set(GameSet::Reset),
                    (select_weapon, fall_back_to_machine_gun).chain().in_set(GameSet::Input),
                    // После всех выстрелов кадра: оружие тратит заряды в `GameSet::Input`.
                    report_empty.in_set(GameSet::Simulation),
                    update_panel.in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Боезапас лунохода по видам оружия в порядке [`CurrentWeapon::ALL`].
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct Inventory {
    ammo: [Option<u16>; CurrentWeapon::ALL.len()],
}

impl Default for Inventory {
    fn default() -> Self {
        Self {
            ammo: CurrentWeapon::ALL.map(|w| spec(w).start),
        }
    }
}

fn slot(weapon: CurrentWeapon) -> usize {
    CurrentWeapon::ALL.iter().position(|w| *w == weapon).unwrap_or(0)
}

impl Inventory {
    /// Боезапас вида. `None` — вида нет.
    pub fn ammo(&self, weapon: CurrentWeapon) -> Option<u16> {
        self.ammo[slot(weapon)]
    }

    /// Вид есть у лунохода, хотя бы пустой.
    pub fn owns(&self, weapon: CurrentWeapon) -> bool {
        self.ammo(weapon).is_some()
    }

    /// Из вида можно выстрелить.
    pub fn usable(&self, weapon: CurrentWeapon) -> bool {
        weapon == CurrentWeapon::MachineGun || self.ammo(weapon).is_some_and(|a| a > 0)
    }

    /// Тратит заряд. `false` — стрелять нечем.
    pub fn spend(&mut self, weapon: CurrentWeapon) -> bool {
        if weapon == CurrentWeapon::MachineGun {
            return true;
        }
        match &mut self.ammo[slot(weapon)] {
            Some(a) if *a > 0 => {
                *a -= 1;
                true
            }
            _ => false,
        }
    }

    /// Добавляет заряды не выше предела и возвращает, сколько добавилось.
    pub fn add(&mut self, weapon: CurrentWeapon, amount: u16) -> u16 {
        let limit = spec(weapon).limit;
        let entry = &mut self.ammo[slot(weapon)];
        let before = entry.unwrap_or(0);
        let after = before.saturating_add(amount).min(limit);
        *entry = Some(after);
        after - before
    }

    /// Следующий вид по кругу, из которого можно стрелять: `step` больше нуля — вперёд.
    pub fn cycled(&self, current: CurrentWeapon, step: i32) -> CurrentWeapon {
        let count = CurrentWeapon::ALL.len() as i32;
        let start = slot(current) as i32;
        (1..=count)
            .map(|i| CurrentWeapon::ALL[(start + i * step.signum()).rem_euclid(count) as usize])
            .find(|w| self.usable(*w))
            .unwrap_or(CurrentWeapon::MachineGun)
    }
}

/// Игрок попробовал выбрать оружие, из которого нечем стрелять: ячейка мигает.
#[derive(Message, Debug, Clone, Copy)]
pub struct WeaponDenied(pub CurrentWeapon);

/// Боезапас вида кончился: потрачен последний заряд. По нему штаб замечает, что стрелять нечем.
#[derive(Message, Debug, Clone, Copy)]
pub struct AmmoRanOut;

/// Виды, у которых боезапас был и кончился между `before` и `after`.
fn ran_out(before: &Inventory, after: &Inventory) -> Vec<CurrentWeapon> {
    CurrentWeapon::ALL
        .into_iter()
        .filter(|w| before.ammo(*w).is_some_and(|a| a > 0) && after.ammo(*w) == Some(0))
        .collect()
}

/// Сравнивает боезапас с прошлым кадром и сообщает о кончившемся.
fn report_empty(inventory: Res<Inventory>, mut last: Local<Option<Inventory>>, mut out: MessageWriter<AmmoRanOut>) {
    if let Some(before) = last.as_ref() {
        if !ran_out(before, &inventory).is_empty() {
            out.write(AmmoRanOut);
        }
    }
    if last.as_ref() != Some(&*inventory) {
        *last = Some(inventory.clone());
    }
}

fn reset_inventory(
    mut rebuilt: MessageReader<WorldRebuilt>,
    mut inventory: ResMut<Inventory>,
    mut weapon: ResMut<CurrentWeapon>,
) {
    if rebuilt.read().last().is_some() {
        *inventory = Inventory::default();
        *weapon = CurrentWeapon::MachineGun;
    }
}

/// Клавиши 1–6 выбирают оружие, колесо, бамперы и крестовина геймпада листают по кругу только
/// то, из чего можно стрелять.
fn select_weapon(
    keys: Res<ButtonInput<KeyCode>>,
    pad: Res<crate::gamepad::PadInput>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    inventory: Res<Inventory>,
    mut cooldown: Local<f32>,
    mut weapon: ResMut<CurrentWeapon>,
    mut denied: MessageWriter<WeaponDenied>,
    targeting: Res<crate::airstrike::Targeting>,
) {
    // В наведении авиаудара колесо и бамперы поворачивают серию.
    if targeting.open {
        return;
    }
    *cooldown = (*cooldown - time.delta_secs()).max(0.0);
    let mut chosen = *weapon;
    if let Some(&pressed) = CurrentWeapon::ALL
        .iter()
        .find(|w| keys.just_pressed(spec(**w).key))
    {
        if inventory.usable(pressed) {
            chosen = pressed;
        } else if inventory.owns(pressed) {
            denied.write(WeaponDenied(pressed));
        }
    } else if pad.weapon_step != 0 {
        chosen = inventory.cycled(*weapon, pad.weapon_step);
    } else if scroll.delta.y != 0.0 {
        // Пока колесо крутится, пауза продлевается: переключение только в начале прокрутки.
        if *cooldown == 0.0 {
            chosen = inventory.cycled(*weapon, if scroll.delta.y > 0.0 { -1 } else { 1 });
        }
        *cooldown = SCROLL_COOLDOWN;
    }
    if chosen != *weapon {
        *weapon = chosen;
    }
}

/// Опустевшее оружие сменяется пулемётом.
fn fall_back_to_machine_gun(inventory: Res<Inventory>, mut weapon: ResMut<CurrentWeapon>) {
    if !inventory.usable(*weapon) {
        *weapon = CurrentWeapon::MachineGun;
    }
}

/// Ячейка панели.
#[derive(Component)]
struct Cell {
    weapon: CurrentWeapon,
    /// Вид уже был у лунохода в прошлом кадре: новая ячейка вспыхивает.
    shown: bool,
    flash: f32,
    denied: f32,
}

/// Число патронов в ячейке.
#[derive(Component)]
struct AmmoText(CurrentWeapon);

/// Отметка уровня в ячейке: `index` от 0, горит, если уровень оружия больше него.
#[derive(Component)]
struct LevelPip {
    weapon: CurrentWeapon,
    index: u8,
}

/// Цвет горящей и пустой отметки уровня.
const PIP_ON: Color = Color::srgb(1.0, 0.78, 0.2);
const PIP_OFF: Color = Color::srgba(1.0, 1.0, 1.0, 0.18);

/// Корень панели.
#[derive(Component)]
struct Panel;

fn spawn_panel(mut commands: Commands) {
    commands
        .spawn((
            Panel,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(14.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                column_gap: Val::Px(8.0),
                ..default()
            },
        ))
        .with_children(|row| {
            for weapon in CurrentWeapon::ALL {
                let s = spec(weapon);
                let key = slot(weapon) + 1;
                row.spawn((
                    Cell {
                        weapon,
                        shown: false,
                        flash: 0.0,
                        denied: 0.0,
                    },
                    Node {
                        width: Val::Px(64.0),
                        height: Val::Px(64.0),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::SpaceBetween,
                        padding: UiRect::all(Val::Px(4.0)),
                        border: UiRect::all(Val::Px(3.0)),
                        display: Display::None,
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.65)),
                    BorderColor::all(Color::NONE),
                ))
                .with_children(|cell| {
                    // Клавиша и отметки уровня в одной строке.
                    cell.spawn(Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(3.0),
                        ..default()
                    })
                    .with_children(|top| {
                        top.spawn((
                            Text::new(key.to_string()),
                            TextFont {
                                font_size: FontSize::Px(11.0),
                                ..default()
                            },
                            TextColor(Color::srgb(0.7, 0.7, 0.7)),
                        ));
                        for index in 0..MAX_LEVEL {
                            top.spawn((
                                LevelPip { weapon, index },
                                Node {
                                    width: Val::Px(6.0),
                                    height: Val::Px(6.0),
                                    ..default()
                                },
                                BackgroundColor(PIP_OFF),
                            ));
                        }
                    });
                    cell.spawn((
                        Text::new(s.icon),
                        TextFont {
                            font_size: FontSize::Px(20.0),
                            ..default()
                        },
                        TextColor(s.color),
                    ));
                    cell.spawn((
                        AmmoText(weapon),
                        Text::new(""),
                        TextFont {
                            font_size: FontSize::Px(12.0),
                            ..default()
                        },
                        TextColor(Color::WHITE),
                    ));
                });
            }
        });
}

/// Панель повторяет инвентарь: имеющиеся виды, патроны, текущее оружие, пустое приглушено.
#[allow(clippy::too_many_arguments)]
fn update_panel(
    time: Res<Time>,
    state: Res<State<GameState>>,
    inventory: Res<Inventory>,
    levels: Res<WeaponLevels>,
    weapon: Res<CurrentWeapon>,
    mut denied: MessageReader<WeaponDenied>,
    mut upgraded: MessageReader<WeaponUpgraded>,
    mut panel: Query<&mut Visibility, With<Panel>>,
    mut cells: Query<(&mut Cell, &mut Node, &mut BorderColor, &mut BackgroundColor), Without<LevelPip>>,
    mut texts: Query<(&AmmoText, &mut Text)>,
    mut pips: Query<(&LevelPip, &mut BackgroundColor), Without<Cell>>,
) {
    let wanted = if *state.get() == GameState::Menu {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    for mut visibility in &mut panel {
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
    let denied: Vec<CurrentWeapon> = denied.read().map(|d| d.0).collect();
    let upgraded: Vec<CurrentWeapon> = upgraded
        .read()
        .filter_map(|u| match u {
            WeaponUpgraded::Level { weapon, .. } => Some(*weapon),
            WeaponUpgraded::Refill { .. } => None,
        })
        .collect();
    let dt = time.delta_secs();
    for (mut cell, mut node, mut border, mut background) in &mut cells {
        let owned = inventory.owns(cell.weapon);
        let display = if owned { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
        if owned && !cell.shown {
            cell.flash = NEW_CELL_SECONDS;
        }
        cell.shown = owned;
        if denied.contains(&cell.weapon) {
            cell.denied = DENIED_SECONDS;
        }
        if upgraded.contains(&cell.weapon) {
            cell.flash = UPGRADE_FLASH_SECONDS;
        }
        cell.flash = (cell.flash - dt).max(0.0);
        cell.denied = (cell.denied - dt).max(0.0);

        let current = *weapon == cell.weapon;
        let usable = inventory.usable(cell.weapon);
        let blink = cell.denied > 0.0 && (cell.denied * 20.0).sin() > 0.0;
        let color = if blink {
            Color::srgb(1.0, 0.2, 0.2)
        } else if cell.flash > 0.0 {
            Color::WHITE
        } else if current {
            spec(cell.weapon).color
        } else {
            Color::NONE
        };
        *border = BorderColor::all(color);
        // Текущая ячейка приподнята, пустая — прозрачнее.
        node.margin.bottom = Val::Px(if current { 8.0 } else { 0.0 });
        let alpha = if usable { 0.65 } else { 0.26 };
        background.0 = Color::srgba(0.0, 0.0, 0.0, alpha);
    }
    // Отметки уровня: горят до уровня оружия, при улучшении вспыхивают вместе с рамкой.
    let flashing: Vec<CurrentWeapon> = cells.iter().filter(|c| c.0.flash > 0.0).map(|c| c.0.weapon).collect();
    for (pip, mut background) in &mut pips {
        let lit = pip.index < levels.level(pip.weapon);
        let color = match (lit, flashing.contains(&pip.weapon)) {
            (true, true) => Color::WHITE,
            (true, false) => PIP_ON,
            (false, _) => PIP_OFF,
        };
        if background.0 != color {
            background.0 = color;
        }
    }
    for (ammo, mut text) in &mut texts {
        let label = match inventory.ammo(ammo.0) {
            _ if ammo.0 == CurrentWeapon::MachineGun => "∞".to_string(),
            Some(a) => a.to_string(),
            None => String::new(),
        };
        if text.0 != label {
            text.0 = label;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pickup_does_not_exceed_limit() {
        let mut inventory = Inventory::default();
        assert_eq!(inventory.ammo(CurrentWeapon::Mortar), Some(24));
        assert_eq!(inventory.add(CurrentWeapon::Mortar, 200), 126);
        assert_eq!(inventory.ammo(CurrentWeapon::Mortar), Some(150));
        assert_eq!(inventory.add(CurrentWeapon::Laser, 4), 4);
        assert_eq!(inventory.add(CurrentWeapon::Laser, 100), 56);
    }

    #[test]
    fn empty_weapon_is_skipped_by_wheel() {
        let mut inventory = Inventory::default();
        assert_eq!(inventory.cycled(CurrentWeapon::MachineGun, 1), CurrentWeapon::Mortar);
        for _ in 0..24 {
            assert!(inventory.spend(CurrentWeapon::Mortar));
        }
        assert!(!inventory.spend(CurrentWeapon::Mortar));
        assert_eq!(inventory.cycled(CurrentWeapon::MachineGun, 1), CurrentWeapon::MachineGun);
        inventory.add(CurrentWeapon::Laser, 1);
        assert_eq!(inventory.cycled(CurrentWeapon::MachineGun, 1), CurrentWeapon::Laser);
        assert_eq!(inventory.cycled(CurrentWeapon::MachineGun, -1), CurrentWeapon::Laser);
    }

    #[test]
    fn ammo_runs_out_on_the_last_shot_only() {
        let mut inventory = Inventory::default();
        inventory.add(CurrentWeapon::Laser, 1);
        let before = inventory.clone();
        inventory.spend(CurrentWeapon::Mortar);
        assert!(ran_out(&before, &inventory).is_empty());
        let before = inventory.clone();
        inventory.spend(CurrentWeapon::Laser);
        assert_eq!(ran_out(&before, &inventory), vec![CurrentWeapon::Laser]);
        // Пустое оружие осталось пустым: повторно не сообщается.
        assert!(ran_out(&inventory.clone(), &inventory).is_empty());
    }
}
