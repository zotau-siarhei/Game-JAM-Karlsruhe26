//! Ввод геймпада через системный GameController на macOS.
//!
//! На новых macOS gilrs видит подключение DualShock 4, но кнопок и стиков от него не получает:
//! ввод поддерживаемых геймпадов система отдаёт через фреймворк GameController. Здесь он
//! опрашивается каждый кадр, и изменения уходят в Bevy теми же сообщениями, что шлёт gilrs
//! (`GamepadConnectionEvent`, `RawGamepadButtonChangedEvent`, `RawGamepadAxisChangedEvent`):
//! для остальной игры это обычный `Gamepad`. На других системах модуль ничего не делает.

use bevy::prelude::*;

/// Плагин ввода через GameController.
pub struct GcInputPlugin;

impl Plugin for GcInputPlugin {
    #[allow(unused_variables)]
    fn build(&self, app: &mut App) {
        #[cfg(target_os = "macos")]
        app.insert_non_send(mac::GcPads::default()).add_systems(
            PreUpdate,
            mac::poll.before(bevy::input::InputSystems),
        );
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use bevy::input::gamepad::{
        GamepadAxis, GamepadButton, GamepadConnection, GamepadConnectionEvent, RawGamepadAxisChangedEvent,
        RawGamepadButtonChangedEvent, RawGamepadEvent,
    };
    use bevy::prelude::*;
    use objc2::rc::Retained;
    use objc2_game_controller::{GCController, GCControllerButtonInput, GCDevice, GCExtendedGamepad};

    /// Кнопки по порядку в снимке.
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

    /// Оси по порядку в снимке.
    const AXES: [GamepadAxis; 4] = [
        GamepadAxis::LeftStickX,
        GamepadAxis::LeftStickY,
        GamepadAxis::RightStickX,
        GamepadAxis::RightStickY,
    ];

    /// Снимок состояния геймпада: значения кнопок и осей.
    #[derive(Default, Clone, Copy, PartialEq)]
    struct Snapshot {
        buttons: [f32; 16],
        axes: [f32; 4],
    }

    fn read(pad: &GCExtendedGamepad) -> Snapshot {
        let value = |button: Option<Retained<GCControllerButtonInput>>| {
            // SAFETY: чтение значения кнопки.
            button.map_or(0.0, |button| unsafe { button.value() })
        };
        // SAFETY: свойства геймпада только читаются.
        unsafe {
            let dpad = pad.dpad();
            let (left, right) = (pad.leftThumbstick(), pad.rightThumbstick());
            Snapshot {
                buttons: [
                    value(Some(pad.buttonA())),
                    value(Some(pad.buttonB())),
                    value(Some(pad.buttonX())),
                    value(Some(pad.buttonY())),
                    value(Some(pad.leftShoulder())),
                    value(Some(pad.rightShoulder())),
                    value(Some(pad.leftTrigger())),
                    value(Some(pad.rightTrigger())),
                    value(Some(pad.buttonMenu())),
                    value(pad.buttonOptions()),
                    value(pad.leftThumbstickButton()),
                    value(pad.rightThumbstickButton()),
                    value(Some(dpad.up())),
                    value(Some(dpad.down())),
                    value(Some(dpad.left())),
                    value(Some(dpad.right())),
                ],
                axes: [
                    left.xAxis().value(),
                    left.yAxis().value(),
                    right.xAxis().value(),
                    right.yAxis().value(),
                ],
            }
        }
    }

    struct Pad {
        controller: Retained<GCController>,
        entity: Entity,
        last: Snapshot,
    }

    #[derive(Default)]
    pub(super) struct GcPads {
        pads: Vec<Pad>,
    }

    pub(super) fn poll(
        mut commands: Commands,
        mut pads: NonSendMut<GcPads>,
        mut events: MessageWriter<RawGamepadEvent>,
        mut connections: MessageWriter<GamepadConnectionEvent>,
        mut buttons: MessageWriter<RawGamepadButtonChangedEvent>,
        mut axes: MessageWriter<RawGamepadAxisChangedEvent>,
    ) {
        // SAFETY: список подключённых геймпадов; объекты удерживаются Retained.
        let current: Vec<Retained<GCController>> = unsafe { GCController::controllers() }.to_vec();
        let same = |a: &Retained<GCController>, b: &Retained<GCController>| Retained::as_ptr(a) == Retained::as_ptr(b);

        pads.pads.retain(|pad| {
            let alive = current.iter().any(|c| same(c, &pad.controller));
            if !alive {
                let event = GamepadConnectionEvent::new(pad.entity, GamepadConnection::Disconnected);
                events.write(event.clone().into());
                connections.write(event);
            }
            alive
        });

        for controller in current {
            // SAFETY: свойство только читается.
            let Some(gamepad) = (unsafe { controller.extendedGamepad() }) else {
                continue;
            };
            let index = match pads.pads.iter().position(|pad| same(&pad.controller, &controller)) {
                Some(index) => index,
                None => {
                    // SAFETY: свойства только читаются.
                    let (name, category) = unsafe { (controller.vendorName(), controller.productCategory()) };
                    let name = name.map_or_else(|| category.to_string(), |name| name.to_string());
                    let category = category.to_string();
                    let sony = category.contains("DualShock") || category.contains("DualSense");
                    let entity = commands.spawn(Name::new(format!("GameController: {name}"))).id();
                    let event = GamepadConnectionEvent::new(
                        entity,
                        GamepadConnection::Connected {
                            name: name.clone(),
                            vendor_id: sony.then_some(0x054C),
                            product_id: None,
                        },
                    );
                    events.write(event.clone().into());
                    connections.write(event);
                    info!("Ввод геймпада через GameController: {name} ({category})");
                    pads.pads.push(Pad {
                        controller: controller.clone(),
                        entity,
                        last: Snapshot::default(),
                    });
                    pads.pads.len() - 1
                }
            };
            let pad = &mut pads.pads[index];
            let now = read(&gamepad);
            if now == pad.last {
                continue;
            }
            for (i, button) in BUTTONS.iter().enumerate() {
                if now.buttons[i] != pad.last.buttons[i] {
                    let event = RawGamepadButtonChangedEvent::new(pad.entity, *button, now.buttons[i]);
                    events.write(event.clone().into());
                    buttons.write(event);
                }
            }
            for (i, axis) in AXES.iter().enumerate() {
                if now.axes[i] != pad.last.axes[i] {
                    let event = RawGamepadAxisChangedEvent::new(pad.entity, *axis, now.axes[i]);
                    events.write(event.clone().into());
                    axes.write(event);
                }
            }
            pad.last = now;
        }
    }
}
