//! Вибрация и световая панель геймпада.
//!
//! Игровые события не шлют вибрацию сами, а толкают микшер [`Rumble`]: уровни сильного (низкий
//! гул: тяжёлые удары, взрывы) и слабого (резкий: пули, выстрелы) мотора от 0 до 1. Толчки
//! складываются с насыщением и гаснут сами, поэтому серия попаданий не превращается в
//! бесконечную тряску, а одиночный тик не теряется. Бэкенд несколько раз в секунду отдаёт уровни
//! геймпаду: на macOS — через системный GameController и CoreHaptics (gilrs там вибрацию не
//! умеет), на других системах — сообщением Bevy `GamepadRumbleRequest`.
//!
//! Цвет световой панели ([`PadLight`]) показывает здоровье лунохода и вспыхивает при попадании.
//! Панель есть у DualShock 4 и DualSense, менять её цвет умеет только бэкенд macOS.
//!
//! Вибрация только в забеге и только когда игрок играет с геймпада: на паузе и в меню она сразу
//! замолкает, а геймпад, лежащий на столе, пока играют мышью, не жужжит.

use bevy::prelude::*;

use crate::arsenal::Inventory;
use crate::bullet::FireBullet;
use crate::combat::{Damage, Died, Health, Team};
use crate::explosion::Explosion;
use crate::frame::GameSet;
use crate::gamepad::InputDevice;
use crate::rover::{Impact, Rover};
use crate::run::GameState;
use crate::weapons::CurrentWeapon;

/// Общая сила вибрации: уровни микшера умножаются на неё перед отправкой в геймпад.
const GAIN: f32 = 1.35;
/// Урон, от которого вибрация в полную силу: ракета, таран босса, взрыв вплотную.
const DAMAGE_FULL: f32 = 150.0;
/// Скорость угасания моторов, доля в секунду (экспонента): сильный гудит дольше.
const STRONG_DECAY: f32 = 5.0;
const WEAK_DECAY: f32 = 9.0;
/// Ниже этого уровня мотор выключается.
const SILENT: f32 = 0.02;
/// Тряска камеры от взрыва переводится в сильный мотор с этим множителем.
const EXPLOSION_GAIN: f32 = 1.3;
/// Толчок своего выстрела тяжёлым оружием: сильный, слабый.
const HEAVY_SHOT: (f32, f32) = (0.3, 0.45);
/// Тик слабого мотора на свой выстрел пулемёта: очередь ощущается мелкой дрожью.
const BULLET_TICK: f32 = 0.12;
/// Удар (таран, столкновение): сильный, слабый.
const KNOCK: (f32, f32) = (0.85, 0.5);
/// Гибель лунохода: полная сила держится столько, с, потом гаснет.
const DEATH_HOLD: f32 = 0.45;

/// Вспышка панели при попадании, с.
const FLASH_SECONDS: f32 = 0.15;
/// Цвета панели: полное здоровье, половина, ноль, меню.
const LIGHT_FULL: Vec3 = Vec3::new(0.0, 1.0, 0.25);
const LIGHT_HALF: Vec3 = Vec3::new(1.0, 0.75, 0.0);
const LIGHT_EMPTY: Vec3 = Vec3::new(1.0, 0.0, 0.0);
const LIGHT_MENU: Vec3 = Vec3::new(0.1, 0.35, 1.0);

/// Плагин вибрации и света.
pub struct HapticsPlugin;

impl Plugin for HapticsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Rumble>()
            .init_resource::<PadLight>()
            .add_systems(
                Update,
                (rumble_from_hits, rumble_from_shots, settle_rumble, light_from_health)
                    .chain()
                    .in_set(GameSet::Presentation),
            );
        #[cfg(target_os = "macos")]
        mac::build(app);
        #[cfg(not(target_os = "macos"))]
        app.add_systems(Update, bevy_backend::play.after(GameSet::Presentation));
    }
}

/// Уровни моторов геймпада от 0 до 1.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct Rumble {
    /// Сильный мотор: низкий гул.
    pub strong: f32,
    /// Слабый мотор: мелкая резкая дрожь.
    pub weak: f32,
    /// Сколько ещё уровни не гаснут, с.
    hold: f32,
}

/// Сложение с насыщением: два толчка по 0,5 дают 0,75, а не 1.
fn mix(level: f32, kick: f32) -> f32 {
    1.0 - (1.0 - level) * (1.0 - kick.clamp(0.0, 1.0))
}

impl Rumble {
    /// Толчок: уровни растут с насыщением.
    pub fn kick(&mut self, strong: f32, weak: f32) {
        self.strong = mix(self.strong, strong);
        self.weak = mix(self.weak, weak);
    }

    /// Толчок, который держится `hold` секунд, прежде чем гаснуть.
    pub fn kick_hold(&mut self, strong: f32, weak: f32, hold: f32) {
        self.kick(strong, weak);
        self.hold = self.hold.max(hold);
    }

    /// Уровни для моторов с общей силой [`GAIN`]: сильный, слабый.
    pub fn output(&self) -> (f32, f32) {
        ((self.strong * GAIN).min(1.0), (self.weak * GAIN).min(1.0))
    }

    /// Тишина сразу.
    pub fn silence(&mut self) {
        *self = Self::default();
    }

    fn decay(&mut self, dt: f32) {
        if self.hold > 0.0 {
            self.hold = (self.hold - dt).max(0.0);
            return;
        }
        let fade = |level: f32, rate: f32| {
            let next = level * (-rate * dt).exp();
            if next < SILENT { 0.0 } else { next }
        };
        self.strong = fade(self.strong, STRONG_DECAY);
        self.weak = fade(self.weak, WEAK_DECAY);
    }
}

/// Толчок от урона: сила растёт с уроном, лёгкое попадание — в основном слабый мотор.
pub fn damage_kick(amount: f32) -> (f32, f32) {
    let share = (amount / DAMAGE_FULL).max(0.0).powf(0.7).min(1.0);
    let strong = ((share - 0.2) / 0.8).max(0.0);
    let weak = 0.15 + 0.6 * share;
    (strong, weak)
}

/// Вибрация только в забеге и только у играющего с геймпада.
fn rumble_on(state: &State<GameState>, device: InputDevice) -> bool {
    *state.get() == GameState::Playing && device.gamepad()
}

/// Попадания, удары, взрывы рядом и гибель лунохода.
#[allow(clippy::too_many_arguments)]
fn rumble_from_hits(
    state: Res<State<GameState>>,
    device: Res<InputDevice>,
    mut rumble: ResMut<Rumble>,
    mut damage: MessageReader<Damage>,
    mut died: MessageReader<Died>,
    mut explosions: MessageReader<Explosion>,
    rover: Query<(Entity, &Transform, Option<&Impact>), With<Rover>>,
    mut last_impact: Local<f32>,
) {
    let Some((me, transform, impact)) = rover.iter().next() else {
        return;
    };
    let impact_left = impact.map_or(0.0, |impact| impact.left);
    let knocked = impact_left > *last_impact + 0.01;
    *last_impact = impact_left;
    let hurt: f32 = damage.read().filter(|d| d.target == me).map(|d| d.amount).sum();
    let dead = died.read().any(|d| d.entity == me);
    let blast = explosions
        .read()
        .map(|e| crate::impact::trauma(e.power, e.position.distance(transform.translation)))
        .fold(0.0, f32::max);
    // Сообщения прочитаны и тогда, когда вибрация выключена: иначе они сработали бы позже.
    if !rumble_on(&state, *device) {
        return;
    }
    if hurt > 0.0 {
        let (strong, weak) = damage_kick(hurt);
        rumble.kick(strong, weak);
    }
    if knocked {
        rumble.kick(KNOCK.0, KNOCK.1);
    }
    if blast > 0.0 {
        rumble.kick((blast * EXPLOSION_GAIN).min(1.0), blast * 0.4);
    }
    if dead {
        rumble.kick_hold(1.0, 1.0, DEATH_HOLD);
    }
}

/// Свои выстрелы: пулемёт — мелкая дрожь, тяжёлое оружие — толчок отдачи.
fn rumble_from_shots(
    state: Res<State<GameState>>,
    device: Res<InputDevice>,
    weapon: Res<CurrentWeapon>,
    inventory: Res<Inventory>,
    mut rumble: ResMut<Rumble>,
    mut bullets: MessageReader<FireBullet>,
    mut last_ammo: Local<Option<(CurrentWeapon, u16)>>,
) {
    let own_bullets = bullets.read().filter(|b| b.team == Team::Player).count();
    // Тяжёлое оружие стреляет без общего сообщения: выстрел виден по убыли его патронов.
    let ammo = inventory.ammo(*weapon).map(|ammo| (*weapon, ammo));
    let fired = matches!((*last_ammo, ammo), (Some((was, before)), Some((now, after))) if was == now && after < before);
    *last_ammo = ammo;
    if !rumble_on(&state, *device) {
        return;
    }
    if own_bullets > 0 {
        rumble.kick(0.0, BULLET_TICK);
    }
    let heavy = matches!(
        *weapon,
        CurrentWeapon::Mortar | CurrentWeapon::Rockets | CurrentWeapon::HomingRockets | CurrentWeapon::Airstrike
    );
    if fired && heavy {
        rumble.kick(HEAVY_SHOT.0, HEAVY_SHOT.1);
    }
}

/// Уровни гаснут по реальному времени; вне забега и без геймпада — тишина.
fn settle_rumble(state: Res<State<GameState>>, device: Res<InputDevice>, time: Res<Time<Real>>, mut rumble: ResMut<Rumble>) {
    if !rumble_on(&state, *device) {
        if *rumble != Rumble::default() {
            rumble.silence();
        }
        return;
    }
    rumble.decay(time.delta_secs());
}

/// Желаемый цвет световой панели геймпада, RGB от 0 до 1.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct PadLight(pub Vec3);

impl Default for PadLight {
    fn default() -> Self {
        Self(LIGHT_MENU)
    }
}

/// Цвет по доле здоровья: зелёный → жёлтый → красный.
pub fn health_color(share: f32) -> Vec3 {
    let share = share.clamp(0.0, 1.0);
    if share >= 0.5 {
        LIGHT_HALF.lerp(LIGHT_FULL, (share - 0.5) * 2.0)
    } else {
        LIGHT_EMPTY.lerp(LIGHT_HALF, share * 2.0)
    }
}

/// Панель: в забеге — здоровье лунохода, при попадании — вспышка белым, на последней четверти
/// здоровья пульсирует; в меню — синяя.
fn light_from_health(
    state: Res<State<GameState>>,
    time: Res<Time<Real>>,
    mut damage: MessageReader<Damage>,
    rover: Query<(Entity, &Health), With<Rover>>,
    mut flash: Local<f32>,
    mut light: ResMut<PadLight>,
) {
    let Some((me, health)) = rover.iter().next() else {
        return;
    };
    if damage.read().any(|d| d.target == me && d.amount > 0.0) {
        *flash = FLASH_SECONDS;
    }
    *flash = (*flash - time.delta_secs()).max(0.0);
    let wanted = match state.get() {
        GameState::Playing | GameState::Paused | GameState::GameOver => {
            let share = health.current / health.max;
            let mut color = health_color(share);
            if share < 0.25 && *state.get() == GameState::Playing {
                let pulse = 0.55 + 0.45 * (time.elapsed_secs() * 6.0).sin().abs();
                color *= pulse;
            }
            color.lerp(Vec3::ONE, *flash / FLASH_SECONDS)
        }
        _ => LIGHT_MENU,
    };
    if light.0.distance(wanted) > 0.02 {
        light.0 = wanted;
    }
}

/// Вибрация средствами Bevy (gilrs): Linux и Windows.
#[cfg(not(target_os = "macos"))]
mod bevy_backend {
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
}

/// Вибрация и свет через GameController и CoreHaptics: macOS 11+, DualShock 4, DualSense, Xbox.
#[cfg(target_os = "macos")]
mod mac {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn пуля_лёгкая_ракета_сильная() {
        let (bullet_strong, bullet_weak) = damage_kick(20.0);
        let (rocket_strong, rocket_weak) = damage_kick(150.0);
        assert!(bullet_strong < 0.1, "{bullet_strong}");
        assert!(bullet_weak < 0.35, "{bullet_weak}");
        assert!((rocket_strong - 1.0).abs() < 1e-5);
        assert!(rocket_weak > bullet_weak);
    }

    #[test]
    fn толчки_складываются_с_насыщением_и_гаснут() {
        let mut rumble = Rumble::default();
        rumble.kick(0.5, 0.0);
        rumble.kick(0.5, 0.0);
        assert!((rumble.strong - 0.75).abs() < 1e-5);
        for _ in 0..10 {
            rumble.kick(0.9, 0.9);
        }
        assert!(rumble.strong <= 1.0);
        for _ in 0..120 {
            rumble.decay(1.0 / 60.0);
        }
        assert_eq!(rumble, Rumble::default());
    }

    #[test]
    fn гибель_держит_полную_силу() {
        let mut rumble = Rumble::default();
        rumble.kick_hold(1.0, 1.0, 0.4);
        rumble.decay(0.3);
        assert_eq!(rumble.strong, 1.0);
        rumble.decay(0.2);
        rumble.decay(0.2);
        assert!(rumble.strong < 1.0);
    }

    #[test]
    fn цвет_панели_по_здоровью() {
        assert_eq!(health_color(1.0), LIGHT_FULL);
        assert_eq!(health_color(0.5), LIGHT_HALF);
        assert_eq!(health_color(0.0), LIGHT_EMPTY);
    }
}
