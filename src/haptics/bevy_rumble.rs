//! Вибрация средствами Bevy (gilrs): Linux (evdev) и Windows (Windows.Gaming.Input).
//! На Linux для неё нужна запись в `/dev/input/event*` геймпада: обычно её даёт правило udev
//! `uaccess` активному пользователю.

#![cfg_attr(any(target_os = "macos", target_os = "ios"), allow(dead_code))]

use std::time::Duration;

use bevy::input::gamepad::{GamepadRumbleIntensity, GamepadRumbleRequest};
use bevy::prelude::*;

use super::Rumble;
use crate::gamepad::PadInput;

/// Каждая отправка вибрирует столько: если игра встанет, вибрация не потянется дальше.
const PULSE: Duration = Duration::from_millis(120);
/// Пока уровни не меняются, отправка повторяется так часто, с.
const REFRESH: f32 = 0.08;

pub(super) fn play(
    rumble: Res<Rumble>,
    input: Res<PadInput>,
    time: Res<Time<Real>>,
    mut requests: MessageWriter<GamepadRumbleRequest>,
    mut sent: Local<(f32, f32, f32)>,
) {
    let Some(gamepad) = input.pad else {
        return;
    };
    let (strong, weak, age) = *sent;
    let age = age + time.delta_secs();
    let (now_strong, now_weak) = rumble.output();
    let on = now_strong > 0.0 || now_weak > 0.0;
    let changed = (now_strong - strong).abs() > 0.03 || (now_weak - weak).abs() > 0.03;
    let stopped = !on && (strong > 0.0 || weak > 0.0);
    if !(changed || stopped || (on && age > REFRESH)) {
        sent.2 = age;
        return;
    }
    requests.write(GamepadRumbleRequest::Stop { gamepad });
    if on {
        requests.write(GamepadRumbleRequest::Add {
            gamepad,
            duration: PULSE,
            intensity: GamepadRumbleIntensity {
                strong_motor: now_strong,
                weak_motor: now_weak,
            },
        });
    }
    *sent = (now_strong, now_weak, 0.0);
}
