//! Вибрация и свет через GameController и CoreHaptics: macOS 11+, DualShock 4, DualSense, Xbox.
//! gilrs на macOS вибрировать не умеет.

use bevy::prelude::*;
use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_haptics::{
    CHHapticDynamicParameter, CHHapticDynamicParameterIDHapticIntensityControl,
    CHHapticEngine, CHHapticEvent, CHHapticEventParameter, CHHapticEventParameterIDHapticIntensity,
    CHHapticEventParameterIDHapticSharpness, CHHapticEventTypeHapticContinuous, CHHapticPattern,
    CHHapticPatternPlayer, CHHapticTimeImmediate,
};
use objc2_foundation::{NSArray, NSString};
use objc2_game_controller::{
    GCColor, GCController, GCDevice, GCDeviceHaptics, GCHapticsLocalityDefault, GCHapticsLocalityLeftHandle,
    GCHapticsLocalityRightHandle,
};

use super::{PadLight, Rumble};

/// Как часто перечитывать список геймпадов, с.
const RESCAN: f32 = 2.0;
/// Уровень мотора меняется, только если сдвинулся больше этого.
const STEP: f32 = 0.03;
/// Длительность непрерывного события паттерна, с: плеер его зацикливает.
const EVENT_SECONDS: f64 = 30.0;
/// Через сколько секунд снова пробовать создать моторы, если CoreHaptics отказал.
const MOTOR_RETRY: f32 = 3.0;

pub(super) fn build(app: &mut App) {
    app.insert_non_send(MacPads::default())
        .add_systems(Update, play.after(crate::frame::GameSet::Presentation));
}

/// Один мотор: движок CoreHaptics на части геймпада и длинный непрерывный паттерн, громкость
/// которого — динамический параметр. Плеер обычный: продвинутый (с зацикливанием) у геймпадов
/// PlayStation рвёт связь со службой вибрации (ошибка 4097, developer.apple.com/forums/thread/773615),
/// поэтому паттерн перезапускается сам до того, как кончится.
struct Motor {
    _engine: Retained<CHHapticEngine>,
    player: Retained<ProtocolObject<dyn CHHapticPatternPlayer>>,
    level: f32,
    /// Сколько секунд паттерн уже звучит; `None` — стоит.
    playing: Option<f32>,
}

impl Motor {
    fn new(haptics: &GCDeviceHaptics, locality: &NSString, sharpness: f32) -> Option<Self> {
        // SAFETY: вызовы фреймворков Apple с корректными аргументами; ошибки — через Result/Option.
        unsafe {
            // В привязках метод помечен только для iOS, а в macOS он есть с версии 11.
            let engine: Option<Retained<CHHapticEngine>> =
                objc2::msg_send![haptics, createEngineWithLocality: locality];
            let engine = engine?;
            if let Err(error) = engine.startAndReturnError() {
                debug!("Вибрация геймпада: движок не запустился: {error:?}");
                return None;
            }
            let parameter = |id, value| {
                CHHapticEventParameter::initWithParameterID_value(CHHapticEventParameter::alloc(), id, value)
            };
            let parameters = NSArray::from_retained_slice(&[
                parameter(CHHapticEventParameterIDHapticIntensity, 1.0),
                parameter(CHHapticEventParameterIDHapticSharpness, sharpness),
            ]);
            let event = CHHapticEvent::initWithEventType_parameters_relativeTime_duration(
                CHHapticEvent::alloc(),
                CHHapticEventTypeHapticContinuous,
                &parameters,
                0.0,
                EVENT_SECONDS,
            );
            let pattern = CHHapticPattern::initWithEvents_parameters_error(
                CHHapticPattern::alloc(),
                &NSArray::from_retained_slice(&[event]),
                &NSArray::new(),
            )
            .inspect_err(|error| debug!("Вибрация геймпада: паттерн: {error:?}"))
            .ok()?;
            let player = engine
                .createPlayerWithPattern_error(&pattern)
                .inspect_err(|error| debug!("Вибрация геймпада: плеер: {error:?}"))
                .ok()?;
            Some(Self {
                _engine: engine,
                player,
                level: 0.0,
                playing: None,
            })
        }
    }

    /// Громкость мотора; `dt` — сколько прошло с прошлого вызова. `false` — плеер или движок
    /// отказал: моторы надо создать заново (так бывает, когда игра уходит в фон и система
    /// останавливает движок).
    fn set(&mut self, level: f32, dt: f32) -> bool {
        let level = level.clamp(0.0, 1.0);
        if let Some(age) = &mut self.playing {
            *age += dt;
        }
        // Паттерн кончится через EVENT_SECONDS: перезапустить заранее.
        let expiring = self.playing.is_some_and(|age| age > EVENT_SECONDS as f32 - 2.0);
        let stop = level == 0.0 && self.playing.is_some();
        if !stop && !expiring && (level - self.level).abs() < STEP {
            return true;
        }
        self.level = level;
        // SAFETY: плеер живёт вместе с движком в этой структуре.
        unsafe {
            if level == 0.0 || expiring {
                self.playing = None;
                let stopped = self.player.stopAtTime_error(CHHapticTimeImmediate).is_ok();
                if level == 0.0 || !stopped {
                    return stopped;
                }
            }
            let control = CHHapticDynamicParameter::initWithParameterID_value_relativeTime(
                CHHapticDynamicParameter::alloc(),
                CHHapticDynamicParameterIDHapticIntensityControl,
                level,
                0.0,
            );
            let controls = NSArray::from_retained_slice(&[control]);
            if self.playing.is_none() {
                if self.player.startAtTime_error(CHHapticTimeImmediate).is_err() {
                    return false;
                }
                self.playing = Some(0.0);
            }
            self.player.sendParameters_atTime_error(&controls, CHHapticTimeImmediate).is_ok()
        }
    }
}

/// Моторы геймпада: сильный и слабый (левая и правая рукоять) или один на весь геймпад.
enum Motors {
    Two { strong: Motor, weak: Motor },
    One(Motor),
}

impl Motors {
    fn new(controller: &GCController) -> Option<Self> {
        // SAFETY: свойства геймпада и статические строки фреймворка только читаются.
        unsafe {
            let haptics = controller.haptics()?;
            let (left, right) = (GCHapticsLocalityLeftHandle, GCHapticsLocalityRightHandle);
            let supported = haptics.supportedLocalities();
            if supported.containsObject(left) && supported.containsObject(right) {
                if let (Some(strong), Some(weak)) =
                    (Motor::new(&haptics, left, 0.15), Motor::new(&haptics, right, 0.85))
                {
                    return Some(Motors::Two { strong, weak });
                }
            }
            Motor::new(&haptics, GCHapticsLocalityDefault, 0.4).map(Motors::One)
        }
    }

    fn set(&mut self, rumble: &Rumble, dt: f32) -> bool {
        let (strong_level, weak_level) = rumble.output();
        match self {
            Motors::Two { strong, weak } => strong.set(strong_level, dt) & weak.set(weak_level, dt),
            Motors::One(only) => only.set(strong_level.max(weak_level * 0.7), dt),
        }
    }
}

/// Геймпад GameController: моторы и цвет панели.
struct Pad {
    controller: Retained<GCController>,
    name: String,
    /// Моторы создаются при первой вибрации: CoreHaptics даёт их только активному окну.
    motors: Option<Motors>,
    /// Через сколько секунд снова пробовать создать моторы после отказа.
    retry: f32,
    /// Отказ уже записан в лог: не повторять его каждые несколько секунд.
    warned: bool,
    light: Option<Vec3>,
}

impl Pad {
    fn new(controller: Retained<GCController>) -> Self {
        // SAFETY: свойства GCController только читаются.
        let (name, haptics) = unsafe { (controller.vendorName(), controller.haptics().is_some()) };
        let name = name.map_or_else(|| "?".to_string(), |name| name.to_string());
        info!("Геймпад GameController: {name}, вибрация {}", if haptics { "есть" } else { "нет" });
        Self {
            controller,
            name,
            motors: None,
            retry: 0.0,
            warned: false,
            light: None,
        }
    }

    fn rumble(&mut self, rumble: &Rumble, dt: f32) {
        self.retry = (self.retry - dt).max(0.0);
        let (strong, weak) = rumble.output();
        let on = strong > 0.0 || weak > 0.0;
        if self.motors.is_none() {
            if !on || self.retry > 0.0 {
                return;
            }
            self.motors = Motors::new(&self.controller);
            match &self.motors {
                Some(motors) => {
                    let kind = if matches!(motors, Motors::Two { .. }) { "два мотора" } else { "один мотор" };
                    info!("Вибрация геймпада {}: {kind}", self.name);
                    self.warned = false;
                }
                None => {
                    self.retry = MOTOR_RETRY;
                    if !self.warned {
                        warn!("Вибрация геймпада {}: CoreHaptics отказал, повтор через {MOTOR_RETRY} с", self.name);
                        self.warned = true;
                    }
                    return;
                }
            }
        }
        if let Some(motors) = &mut self.motors {
            if !motors.set(rumble, dt) {
                // Движок остановлен системой: при следующей вибрации создадим заново.
                self.motors = None;
            }
        }
    }

    fn color(&mut self, color: Vec3) {
        if self.light.is_some_and(|light| light.distance(color) < 0.02) {
            return;
        }
        self.light = Some(color);
        // SAFETY: цвет создаётся и передаётся панели геймпада, если она есть.
        unsafe {
            if let Some(light) = self.controller.light() {
                let color = GCColor::initWithRed_green_blue(GCColor::alloc(), color.x, color.y, color.z);
                light.setColor(&color);
            }
        }
    }
}

#[derive(Default)]
struct MacPads {
    pads: Vec<Pad>,
    /// Сколько осталось до перечитывания списка геймпадов, с.
    rescan: f32,
}

fn play(mut pads: NonSendMut<MacPads>, rumble: Res<Rumble>, light: Res<PadLight>, time: Res<Time<Real>>) {
    let dt = time.delta_secs();
    pads.rescan -= dt;
    if pads.rescan <= 0.0 {
        pads.rescan = RESCAN;
        // SAFETY: список подключённых геймпадов; объекты удерживаются Retained.
        let list = unsafe { GCController::controllers() };
        let current: Vec<Retained<GCController>> = list.to_vec();
        pads.pads.retain(|pad| current.iter().any(|c| Retained::as_ptr(c) == Retained::as_ptr(&pad.controller)));
        for controller in current {
            if !pads.pads.iter().any(|pad| Retained::as_ptr(&pad.controller) == Retained::as_ptr(&controller)) {
                pads.pads.push(Pad::new(controller));
            }
        }
    }
    for pad in &mut pads.pads {
        pad.rumble(&rumble, dt);
        pad.color(light.0);
    }
}
