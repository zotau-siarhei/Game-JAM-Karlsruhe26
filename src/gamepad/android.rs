//! Ввод геймпада на Android.
//!
//! gilrs на Android не работает, а winit превращает события джойстика в касания и теряет правый
//! стик и курки. Поэтому события геймпада забирает фильтр очереди ввода (`set_input_filter`, патч
//! android-activity в `android/patch_android_activity.py`), а система раз в кадр отдаёт изменения
//! в Bevy теми же сообщениями, что шлёт gilrs: для остальной игры это обычный `Gamepad`.
//! Отключение система Android в очередь ввода не сообщает, поэтому геймпад остаётся до выхода.

use std::sync::{Mutex, MutexGuard, PoisonError};

use bevy::android::ANDROID_APP;
use bevy::android::android_activity::input::{Axis, InputEvent, KeyAction, Keycode, set_input_filter};
use bevy::input::gamepad::{
    GamepadAxis, GamepadButton, GamepadConnection, GamepadConnectionEvent, RawGamepadAxisChangedEvent,
    RawGamepadButtonChangedEvent, RawGamepadEvent,
};
use bevy::prelude::*;
use jni::JavaVM;
use jni::objects::{JString, JValue};

/// Биты источника события `AINPUT_SOURCE_GAMEPAD` и `AINPUT_SOURCE_JOYSTICK`.
const SOURCE_GAMEPAD: u32 = 0x0000_0401;
const SOURCE_JOYSTICK: u32 = 0x0100_0010;

/// Кнопки по порядку в состоянии.
const BUTTONS: [GamepadButton; 16] = [
    GamepadButton::South,
    GamepadButton::East,
    GamepadButton::West,
    GamepadButton::North,
    GamepadButton::LeftTrigger,
    GamepadButton::RightTrigger,
    GamepadButton::LeftTrigger2,
    GamepadButton::RightTrigger2,
    GamepadButton::Start,
    GamepadButton::Select,
    GamepadButton::LeftThumb,
    GamepadButton::RightThumb,
    GamepadButton::DPadUp,
    GamepadButton::DPadDown,
    GamepadButton::DPadLeft,
    GamepadButton::DPadRight,
];

/// Места курков и крестовины в [`BUTTONS`]: у них есть и аналоговое значение.
const LEFT_TRIGGER2: usize = 6;
const RIGHT_TRIGGER2: usize = 7;
const DPAD_UP: usize = 12;
const DPAD_DOWN: usize = 13;
const DPAD_LEFT: usize = 14;
const DPAD_RIGHT: usize = 15;

/// Оси по порядку в состоянии.
const AXES: [GamepadAxis; 4] = [
    GamepadAxis::LeftStickX,
    GamepadAxis::LeftStickY,
    GamepadAxis::RightStickX,
    GamepadAxis::RightStickY,
];

/// Состояние геймпада: пишет фильтр очереди ввода, читает система кадра.
struct Shared {
    /// Устройство, от которого пришло последнее событие.
    device: Option<i32>,
    keys: [bool; 16],
    /// Нажатия с прошлого кадра: короткое нажатие не теряется, даже если кадр был долгим.
    pressed: [bool; 16],
    /// Аналоговые значения: курки и крестовина приходят ещё и осями.
    analog: [f32; 16],
    axes: [f32; 4],
}

static SHARED: Mutex<Shared> = Mutex::new(Shared {
    device: None,
    keys: [false; 16],
    pressed: [false; 16],
    analog: [0.0; 16],
    axes: [0.0; 4],
});

fn shared() -> MutexGuard<'static, Shared> {
    SHARED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Место кнопки в [`BUTTONS`].
fn button_index(key: Keycode) -> Option<usize> {
    Some(match key {
        Keycode::ButtonA => 0,
        Keycode::ButtonB => 1,
        Keycode::ButtonX => 2,
        Keycode::ButtonY => 3,
        Keycode::ButtonL1 => 4,
        Keycode::ButtonR1 => 5,
        Keycode::ButtonL2 => LEFT_TRIGGER2,
        Keycode::ButtonR2 => RIGHT_TRIGGER2,
        Keycode::ButtonStart => 8,
        Keycode::ButtonSelect => 9,
        Keycode::ButtonThumbl => 10,
        Keycode::ButtonThumbr => 11,
        Keycode::DpadUp => DPAD_UP,
        Keycode::DpadDown => DPAD_DOWN,
        Keycode::DpadLeft => DPAD_LEFT,
        Keycode::DpadRight => DPAD_RIGHT,
        _ => return None,
    })
}

/// Забирает события геймпада. Все его кнопки считаются обработанными: иначе система сделала бы
/// из кнопки B «назад» и закрыла игру.
fn filter(event: &InputEvent<'_>) -> bool {
    match event {
        InputEvent::MotionEvent(motion) => {
            if u32::from(motion.source()) & SOURCE_JOYSTICK != SOURCE_JOYSTICK || motion.pointer_count() == 0 {
                return false;
            }
            let pointer = motion.pointer_at_index(0);
            let axis = |axis: Axis| pointer.axis_value(axis);
            let (hat_x, hat_y) = (axis(Axis::HatX), axis(Axis::HatY));
            let mut state = shared();
            state.device = Some(motion.device_id());
            // На Android ось Y стика направлена вниз, в Bevy — вверх.
            state.axes = [axis(Axis::X), -axis(Axis::Y), axis(Axis::Z), -axis(Axis::Rz)];
            state.analog[LEFT_TRIGGER2] = axis(Axis::Ltrigger).max(axis(Axis::Brake));
            state.analog[RIGHT_TRIGGER2] = axis(Axis::Rtrigger).max(axis(Axis::Gas));
            state.analog[DPAD_UP] = (-hat_y).max(0.0);
            state.analog[DPAD_DOWN] = hat_y.max(0.0);
            state.analog[DPAD_LEFT] = (-hat_x).max(0.0);
            state.analog[DPAD_RIGHT] = hat_x.max(0.0);
            true
        }
        InputEvent::KeyEvent(key) => {
            if u32::from(key.source()) & SOURCE_GAMEPAD != SOURCE_GAMEPAD {
                return false;
            }
            let mut state = shared();
            state.device = Some(key.device_id());
            if let Some(index) = button_index(key.key_code()) {
                match key.action() {
                    KeyAction::Down => {
                        state.keys[index] = true;
                        state.pressed[index] = true;
                    }
                    KeyAction::Up => state.keys[index] = false,
                    _ => {}
                }
            }
            true
        }
        _ => false,
    }
}

/// Имя, производитель и модель устройства: их знает только Java-класс `InputDevice`.
fn describe(device: i32) -> Option<(String, u16, u16)> {
    let app = ANDROID_APP.get()?;
    // SAFETY: указатель на JavaVM выдаёт система, он действует всё время жизни процесса.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }.ok()?;
    let mut env = vm.attach_current_thread_permanently().ok()?;
    let found = (|| -> jni::errors::Result<Option<(String, u16, u16)>> {
        let input_device = env
            .call_static_method(
                "android/view/InputDevice",
                "getDevice",
                "(I)Landroid/view/InputDevice;",
                &[JValue::Int(device)],
            )?
            .l()?;
        if input_device.is_null() {
            return Ok(None);
        }
        let vendor = env.call_method(&input_device, "getVendorId", "()I", &[])?.i()?;
        let product = env.call_method(&input_device, "getProductId", "()I", &[])?.i()?;
        let name = JString::from(env.call_method(&input_device, "getName", "()Ljava/lang/String;", &[])?.l()?);
        let name: String = env.get_string(&name)?.into();
        Ok(Some((name, vendor as u16, product as u16)))
    })();
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    found.ok().flatten()
}

/// Что уже отдано в Bevy.
#[derive(Default)]
struct Sent {
    entity: Option<Entity>,
    buttons: [f32; 16],
    axes: [f32; 4],
}

fn poll(
    mut commands: Commands,
    mut sent: Local<Sent>,
    mut events: MessageWriter<RawGamepadEvent>,
    mut connections: MessageWriter<GamepadConnectionEvent>,
    mut buttons: MessageWriter<RawGamepadButtonChangedEvent>,
    mut axes: MessageWriter<RawGamepadAxisChangedEvent>,
) {
    let Some(entity) = sent.entity else {
        // Сначала только подключение: компонент `Gamepad` появится к следующему кадру, и нажатие,
        // с которого всё началось, уйдёт уже ему.
        let Some(device) = shared().device else {
            return;
        };
        let (name, vendor_id, product_id) = match describe(device) {
            Some((name, vendor, product)) => (name, Some(vendor), Some(product)),
            None => ("Android gamepad".to_owned(), None, None),
        };
        let entity = commands.spawn(Name::new(format!("Геймпад Android: {name}"))).id();
        let event =
            GamepadConnectionEvent::new(entity, GamepadConnection::Connected { name, vendor_id, product_id });
        events.write(event.clone().into());
        connections.write(event);
        sent.entity = Some(entity);
        return;
    };
    let (now_buttons, now_axes) = {
        let mut state = shared();
        let values: [f32; 16] = std::array::from_fn(|i| {
            state.analog[i].max(if state.keys[i] || state.pressed[i] { 1.0 } else { 0.0 })
        });
        state.pressed = [false; 16];
        (values, state.axes)
    };
    for (i, button) in BUTTONS.iter().enumerate() {
        if now_buttons[i] != sent.buttons[i] {
            let event = RawGamepadButtonChangedEvent::new(entity, *button, now_buttons[i]);
            events.write(event.clone().into());
            buttons.write(event);
        }
    }
    for (i, axis) in AXES.iter().enumerate() {
        if now_axes[i] != sent.axes[i] {
            let event = RawGamepadAxisChangedEvent::new(entity, *axis, now_axes[i]);
            events.write(event.clone().into());
            axes.write(event);
        }
    }
    sent.buttons = now_buttons;
    sent.axes = now_axes;
}

pub(super) fn build(app: &mut App) {
    set_input_filter(filter);
    app.add_systems(PreUpdate, poll.before(bevy::input::InputSystems));
}
