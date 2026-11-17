//! Геймпад: езда и прицел стиками, кнопки, подсказки под устройство.
//!
//! Кнопки геймпада нажимают виртуальные клавиши: в `PreUpdate` после ввода Bevy кнопка жмёт в
//! `ButtonInput` ту клавишу или кнопку мыши, которую заменяет в текущем состоянии игры (в забеге
//! крест — пробел ремонта, в меню — Enter), и отпускает её вместе с собой. Поэтому стрельба, меню,
//! пауза и ремонт работают с геймпада без правок в своих модулях. Аналоговое — в ресурсе
//! [`PadInput`]: езду читает `rover`, смену оружия — `arsenal`, прицел — `stick_aim`.
//!
//! [`InputDevice`] — чем игрок пользовался последним: от него зависят подсказки ([`DeviceHint`]),
//! курсор и прицел.

use bevy::input::InputSystems;
use bevy::input::gamepad::{Gamepad, GamepadButton, GamepadConnection, GamepadConnectionEvent};
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorOptions, PrimaryWindow};

#[cfg(target_os = "macos")]
mod macos;

use crate::camera::GameCamera;
use crate::frame::GameSet;
use crate::run::GameState;

/// Мёртвая зона стиков: радиальная, доля полного наклона.
const STICK_DEADZONE: f32 = 0.2;
/// Наклон стика, который переключает устройство на геймпад и листает пункты паузы.
const STICK_ACTIVE: f32 = 0.5;
/// Сдвиг мыши за кадр, точек, который возвращает устройство на мышь: дрожь стола не считается.
const MOUSE_ACTIVE: f32 = 4.0;
/// Производитель геймпадов PlayStation (USB vendor id Sony).
const SONY_VENDOR: u16 = 0x054C;

/// Плагин геймпада.
pub struct GamepadPlugin;

impl Plugin for GamepadPlugin {
    fn build(&self, app: &mut App) {
        // Ввод на macOS — через GameController, на остальных платформах его даёт gilrs в Bevy.
        #[cfg(target_os = "macos")]
        macos::build(app);
        app.init_resource::<InputDevice>()
            .init_resource::<PadStyle>()
            .init_resource::<PadInput>()
            .init_resource::<Held>()
            .add_systems(PreUpdate, read_pad.after(InputSystems))
            .add_systems(
                Update,
                (
                    log_connections.in_set(GameSet::Presentation),
                    (show_cursor, update_hints).in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Чем игрок пользовался последним.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputDevice {
    #[default]
    KeyboardMouse,
    Gamepad,
}

impl InputDevice {
    pub fn gamepad(self) -> bool {
        self == InputDevice::Gamepad
    }
}

/// Подписи кнопок: по производителю геймпада.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadStyle {
    #[default]
    Xbox,
    PlayStation,
}

/// Аналоговый ввод активного геймпада за этот кадр.
#[derive(Resource, Default, Debug)]
pub struct PadInput {
    /// Активный геймпад: последний, которым пользовались.
    pub pad: Option<Entity>,
    /// Левый стик после мёртвой зоны на плоскости мира (x, z): куда должен ехать луноход, длина до 1.
    pub drive_world: Vec2,
    /// Правый стик после мёртвой зоны, в осях экрана.
    pub aim: Vec2,
    /// Правый стик на плоскости мира (x, z), длина до 1.
    pub aim_world: Vec2,
    /// Шаг по оружию в этом кадре: −1, 0 или 1.
    pub weapon_step: i32,
    /// Зажат L2: правый стик двигает прицел как курсор.
    pub cursor: bool,
}

/// Что нажимает кнопка геймпада.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Press {
    Key(KeyCode),
    Mouse(MouseButton),
}

/// Нажатые кнопками виртуальные клавиши: отпускается ровно то, что было нажато.
#[derive(Resource, Default)]
struct Held {
    buttons: Vec<(Entity, GamepadButton, Press)>,
    /// Клавиши, нажатые стиком на один кадр.
    taps: Vec<KeyCode>,
    /// Наклон левого стика по вертикали в прошлом кадре: пункты паузы листаются по пересечению порога.
    last_stick_y: f32,
}

/// Клавиша или кнопка мыши, которую кнопка геймпада заменяет в состоянии игры.
fn virtual_for(state: GameState, button: GamepadButton) -> Option<Press> {
    use GamepadButton as B;
    use Press::{Key, Mouse};
    match (state, button) {
        (GameState::Playing, B::RightTrigger2) => Some(Mouse(MouseButton::Left)),
        (GameState::Playing, B::South) => Some(Key(KeyCode::Space)),
        (GameState::Playing, B::Start) => Some(Key(KeyCode::Escape)),
        // Правая кнопка мыши: отмена наведения авиаудара.
        (GameState::Playing, B::East) => Some(Mouse(MouseButton::Right)),
        (GameState::Paused, B::South) => Some(Key(KeyCode::Enter)),
        (GameState::Paused, B::East | B::Start) => Some(Key(KeyCode::Escape)),
        (GameState::Paused, B::Select) => Some(Key(KeyCode::KeyQ)),
        (GameState::Paused, B::DPadUp) => Some(Key(KeyCode::ArrowUp)),
        (GameState::Paused, B::DPadDown) => Some(Key(KeyCode::ArrowDown)),
        // В меню «назад» ничего не делает: Esc там закрывает игру.
        (GameState::Menu, B::South | B::Start) => Some(Key(KeyCode::Enter)),
        (GameState::GameOver, B::South | B::Start) => Some(Key(KeyCode::Enter)),
        (GameState::GameOver, B::East) => Some(Key(KeyCode::Escape)),
        _ => None,
    }
}

/// Радиальная мёртвая зона: наклон меньше `deadzone` — ноль, дальше растёт от нуля до 1.
pub fn deadzone(stick: Vec2, deadzone: f32) -> Vec2 {
    let length = stick.length();
    if length <= deadzone {
        return Vec2::ZERO;
    }
    stick / length * ((length - deadzone) / (1.0 - deadzone)).min(1.0)
}

/// Оси экрана на плоскости мира (x, z): куда на земле смотрят «вправо» и «вверх» экрана.
fn screen_axes(camera: &Transform) -> (Vec2, Vec2) {
    let flat = |v: Vec3| Vec2::new(v.x, v.z).normalize_or_zero();
    let right = flat(*camera.right());
    let up = flat(*camera.up());
    if right == Vec2::ZERO || up == Vec2::ZERO {
        return (Vec2::X, Vec2::NEG_Y);
    }
    (right, up)
}

/// Наклон стика в осях экрана — на плоскость мира (x, z).
fn to_world(stick: Vec2, (right, up): (Vec2, Vec2)) -> Vec2 {
    right * stick.x + up * stick.y
}

#[allow(clippy::too_many_arguments)]
fn read_pad(
    state: Option<Res<State<GameState>>>,
    pads: Query<(Entity, &Gamepad)>,
    camera: Query<&Transform, With<GameCamera>>,
    motion: Res<AccumulatedMouseMotion>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut held: ResMut<Held>,
    mut input: ResMut<PadInput>,
    mut device: ResMut<InputDevice>,
    mut style: ResMut<PadStyle>,
) {
    for key in std::mem::take(&mut held.taps) {
        keys.release(key);
    }

    // Клавиатура и мышь — до того, как кнопки геймпада нажмут свои клавиши.
    let desk_used = keys.get_just_pressed().next().is_some()
        || mouse.get_just_pressed().next().is_some()
        || motion.delta.length() > MOUSE_ACTIVE;

    let used = |pad: &Gamepad| {
        pad.digital().get_just_pressed().next().is_some()
            || pad.left_stick().length() > STICK_ACTIVE
            || pad.right_stick().length() > STICK_ACTIVE
    };
    let active = pads
        .iter()
        .find(|(_, pad)| used(pad))
        .map(|(entity, _)| entity)
        .or(input.pad.filter(|entity| pads.contains(*entity)))
        .or(pads.iter().next().map(|(entity, _)| entity));
    let pad_used = pads.iter().any(|(_, pad)| used(pad));
    if pad_used && !device.gamepad() {
        info!("Управление: геймпад");
        *device = InputDevice::Gamepad;
    } else if desk_used && !pad_used && device.gamepad() {
        info!("Управление: клавиатура и мышь");
        *device = InputDevice::KeyboardMouse;
    }
    for (_, pad) in &pads {
        for button in pad.digital().get_just_pressed() {
            debug!("Кнопка геймпада: {button:?}");
        }
    }
    if input.pad != active {
        input.pad = active;
        if let Some((_, pad)) = active.and_then(|entity| pads.get(entity).ok()) {
            let wanted = if pad.vendor_id() == Some(SONY_VENDOR) { PadStyle::PlayStation } else { PadStyle::Xbox };
            if *style != wanted {
                *style = wanted;
            }
        }
    }

    // Отпустить то, что держали отпущенные кнопки или отключённый геймпад.
    held.buttons.retain(|&(entity, button, virt)| {
        let still = pads.get(entity).is_ok_and(|(_, pad)| pad.pressed(button));
        if !still {
            match virt {
                Press::Key(key) => keys.release(key),
                Press::Mouse(button) => mouse.release(button),
            }
        }
        still
    });

    let state = state.map(|s| *s.get());
    let axes = camera.iter().next().map(screen_axes).unwrap_or((Vec2::X, Vec2::NEG_Y));
    let Some((entity, pad)) = active.and_then(|entity| pads.get(entity).ok()) else {
        *input = PadInput::default();
        return;
    };
    if let Some(state) = state {
        for &button in pad.digital().get_just_pressed() {
            let Some(virt) = virtual_for(state, button) else {
                continue;
            };
            match virt {
                Press::Key(key) => keys.press(key),
                Press::Mouse(button) => mouse.press(button),
            }
            held.buttons.push((entity, button, virt));
        }
        // Пункты паузы листаются и стиком: по пересечению порога, как по нажатию.
        let y = pad.left_stick().y;
        if state == GameState::Paused {
            let last = held.last_stick_y;
            let tap = if y > STICK_ACTIVE && last <= STICK_ACTIVE {
                Some(KeyCode::ArrowUp)
            } else if y < -STICK_ACTIVE && last >= -STICK_ACTIVE {
                Some(KeyCode::ArrowDown)
            } else {
                None
            };
            if let Some(key) = tap {
                keys.press(key);
                held.taps.push(key);
            }
        }
        held.last_stick_y = y;
    }

    let drive = deadzone(pad.left_stick(), STICK_DEADZONE);
    let aim = deadzone(pad.right_stick(), STICK_DEADZONE);
    let pressed = |b: GamepadButton| pad.just_pressed(b);
    let weapon_step = i32::from(pressed(GamepadButton::RightTrigger) || pressed(GamepadButton::DPadRight))
        - i32::from(pressed(GamepadButton::LeftTrigger) || pressed(GamepadButton::DPadLeft));
    *input = PadInput {
        pad: Some(entity),
        drive_world: to_world(drive, axes),
        aim,
        aim_world: to_world(aim, axes),
        weapon_step,
        cursor: pad.pressed(GamepadButton::LeftTrigger2),
    };
}

/// В лог: какой геймпад подключился, чтобы было видно, что игра его увидела.
fn log_connections(mut events: MessageReader<GamepadConnectionEvent>) {
    for event in events.read() {
        match &event.connection {
            GamepadConnection::Connected { name, vendor_id, product_id } => {
                info!("Геймпад подключён: {name} (vendor {vendor_id:04X?}, product {product_id:04X?})");
            }
            GamepadConnection::Disconnected => info!("Геймпад отключён"),
        }
    }
}

/// С геймпадом курсор мыши не виден: он только мешал бы.
fn show_cursor(device: Res<InputDevice>, mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>) {
    if device.is_changed() {
        cursor.visible = !device.gamepad();
    }
}

/// Подсказка на экране для клавиатуры и для геймпада. В тексте для геймпада метки кнопок
/// подставляются по [`PadStyle`]: `{A}` — «да», `{B}` — «назад», `{FIRE}`, `{BUMPERS}`,
/// `{START}`, `{SELECT}`, `{CURSOR}`.
#[derive(Component, Debug, Clone)]
pub struct DeviceHint {
    pub keys: String,
    pub pad: String,
}

impl DeviceHint {
    pub fn new(keys: impl Into<String>, pad: impl Into<String>) -> Self {
        Self {
            keys: keys.into(),
            pad: pad.into(),
        }
    }
}

/// Текст подсказки для устройства.
pub fn hint_text(device: InputDevice, style: PadStyle, keys: &str, pad: &str) -> String {
    if !device.gamepad() {
        return keys.to_string();
    }
    let labels: [(&str, &str); 7] = match style {
        PadStyle::PlayStation => [
            ("{A}", "CROSS"),
            ("{B}", "CIRCLE"),
            ("{FIRE}", "R2"),
            ("{BUMPERS}", "L1/R1"),
            ("{START}", "OPTIONS"),
            ("{SELECT}", "SHARE"),
            ("{CURSOR}", "L2"),
        ],
        PadStyle::Xbox => [
            ("{A}", "A"),
            ("{B}", "B"),
            ("{FIRE}", "RT"),
            ("{BUMPERS}", "LB/RB"),
            ("{START}", "MENU"),
            ("{SELECT}", "VIEW"),
            ("{CURSOR}", "LT"),
        ],
    };
    labels
        .iter()
        .fold(pad.to_string(), |text, (label, name)| text.replace(label, name))
}

fn update_hints(
    device: Res<InputDevice>,
    style: Res<PadStyle>,
    mut hints: Query<(Ref<DeviceHint>, &mut Text)>,
) {
    let all = device.is_changed() || style.is_changed();
    for (hint, mut text) in &mut hints {
        if !all && !hint.is_changed() {
            continue;
        }
        let wanted = hint_text(*device, *style, &hint.keys, &hint.pad);
        if text.0 != wanted {
            text.0 = wanted;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn мёртвая_зона_режет_дрожь_и_растягивает_остальное() {
        assert_eq!(deadzone(Vec2::new(0.1, 0.1), 0.2), Vec2::ZERO);
        let full = deadzone(Vec2::new(1.0, 0.0), 0.2);
        assert!((full.x - 1.0).abs() < 1e-5);
        let half = deadzone(Vec2::new(0.0, 0.6), 0.2);
        assert!((half.y - 0.5).abs() < 1e-5);
    }

    #[test]
    fn назад_в_меню_не_закрывает_игру() {
        assert_eq!(virtual_for(GameState::Menu, GamepadButton::East), None);
        assert_eq!(
            virtual_for(GameState::Menu, GamepadButton::South),
            Some(Press::Key(KeyCode::Enter))
        );
        assert_eq!(
            virtual_for(GameState::Playing, GamepadButton::RightTrigger2),
            Some(Press::Mouse(MouseButton::Left))
        );
    }

    #[test]
    fn подсказка_подставляет_кнопки_геймпада() {
        let pad = "{A} - START";
        assert_eq!(hint_text(InputDevice::Gamepad, PadStyle::Xbox, "", "{CURSOR}"), "LT");
        assert_eq!(hint_text(InputDevice::KeyboardMouse, PadStyle::PlayStation, "ENTER", pad), "ENTER");
        assert_eq!(hint_text(InputDevice::Gamepad, PadStyle::PlayStation, "ENTER", pad), "CROSS - START");
        assert_eq!(hint_text(InputDevice::Gamepad, PadStyle::Xbox, "ENTER", pad), "A - START");
    }

    #[test]
    fn оси_экрана_камеры_сверху() {
        // Камера над землёй смотрит вниз, верх экрана — на −Z (север).
        let camera = Transform::from_xyz(0.0, 30.0, 20.0).looking_at(Vec3::ZERO, Vec3::Y);
        let (right, up) = screen_axes(&camera);
        assert!((right - Vec2::X).length() < 1e-4, "{right}");
        assert!((up - Vec2::NEG_Y).length() < 1e-4, "{up}");
    }
}
