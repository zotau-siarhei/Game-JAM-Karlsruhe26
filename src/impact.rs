//! Ощущение удара от взрывов: тряска камеры по мощности и расстоянию до лунохода
//! и короткое замедление времени на самых мощных взрывах.

use bevy::prelude::*;
use bevy_trauma_shake::{Shake, ShakeSettings, TraumaPlugin};

use crate::camera::GameCamera;
use crate::explosion::Explosion;
use crate::frame::GameSet;
use crate::rover::Rover;
use crate::run::{GameState, WorldRebuilt};

/// Дальше этого расстояния от лунохода взрыв не трясёт камеру, м.
const SHAKE_RANGE: f32 = 60.0;
/// Доля мощности взрыва, которая уходит в травму вплотную к луноходу.
const TRAUMA_PER_POWER: f32 = 0.45;
/// Наибольшее смещение камеры при полной травме, м.
const SHAKE_AMPLITUDE: f32 = 1.4;

/// Взрыв мощнее порога замедляет время: мортира (1,4) ниже, авиаудар и гибель лунохода выше.
const SLOWMO_POWER: f32 = 1.5;
/// Скорость времени в самый медленный момент.
const SLOWMO_SPEED: f32 = 0.25;
/// Сколько держится самое медленное время, с реального времени.
const SLOWMO_HOLD: f32 = 0.15;
/// За сколько время плавно возвращается к обычному, с реального времени.
const SLOWMO_RETURN: f32 = 0.35;
/// Дальше этого расстояния от лунохода мощный взрыв время не замедляет, м: дальность прицела
/// 40 м плюс половина линии авиаудара, дальше — уже за краем кадра.
const SLOWMO_RANGE: f32 = 55.0;
/// Сколько после конца замедления новое не начинается, с реального времени: серия бомб
/// авиаудара даёт одно замедление, а не цепочку.
const SLOWMO_COOLDOWN: f32 = 1.0;

/// Плагин тряски и замедления.
pub struct ImpactPlugin;

impl Plugin for ImpactPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(TraumaPlugin)
            .init_resource::<SlowMotion>()
            .add_systems(PostStartup, add_shake.after(crate::camera::spawn_camera))
            .add_systems(
                Update,
                (
                    reset_slowmo.in_set(GameSet::Reset),
                    // Замедление идёт по реальному времени и должно закончиться и после гибели.
                    (shake_on_explosion, start_slowmo, run_slowmo).chain().in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Замедление времени.
#[derive(Resource, Default)]
struct SlowMotion {
    /// Сколько прошло реального времени с начала замедления, с; `None` — замедления нет.
    age: Option<f32>,
    /// Сколько ещё нельзя начать новое замедление, с реального времени.
    cooldown: f32,
}

impl SlowMotion {
    /// Начинает замедление, если оно не идёт и не остывает: идущее не продлевается.
    fn try_start(&mut self) -> bool {
        if self.age.is_some() || self.cooldown > 0.0 {
            return false;
        }
        self.age = Some(0.0);
        true
    }
}

fn add_shake(mut commands: Commands, camera: Single<Entity, With<GameCamera>>) {
    commands.entity(*camera).insert((
        Shake::default(),
        ShakeSettings {
            amplitude: SHAKE_AMPLITUDE,
            trauma_power: 2.0,
            decay_per_second: 1.2,
            frequency: 18.0,
            octaves: 2,
        },
    ));
}

/// Травма от взрыва: растёт с мощностью и линейно гаснет к пределу дальности.
pub fn trauma(power: f32, distance: f32) -> f32 {
    TRAUMA_PER_POWER * power * (1.0 - distance / SHAKE_RANGE).clamp(0.0, 1.0)
}

fn shake_on_explosion(
    mut explosions: MessageReader<Explosion>,
    rover: Single<&Transform, With<Rover>>,
    mut shake: Single<&mut Shake>,
) {
    for e in explosions.read() {
        let amount = trauma(e.power, e.position.distance(rover.translation));
        if amount > 0.0 {
            shake.add_trauma(amount);
        }
    }
}

/// Замедляет время на мощном взрыве рядом с луноходом.
fn start_slowmo(
    mut explosions: MessageReader<Explosion>,
    mut slowmo: ResMut<SlowMotion>,
    rover: Single<&Transform, With<Rover>>,
) {
    if explosions.read().any(|e| slows_time(e.power, e.position.distance(rover.translation))) {
        slowmo.try_start();
    }
}

/// Замедляет ли время взрыв мощности `power` в `distance` метрах от лунохода.
pub fn slows_time(power: f32, distance: f32) -> bool {
    power > SLOWMO_POWER && distance <= SLOWMO_RANGE
}

/// Скорость времени через `age` секунд реального времени после начала замедления.
pub fn slowmo_speed(age: f32) -> f32 {
    if age < SLOWMO_HOLD {
        return SLOWMO_SPEED;
    }
    let t = ((age - SLOWMO_HOLD) / SLOWMO_RETURN).clamp(0.0, 1.0);
    let smooth = t * t * (3.0 - 2.0 * t);
    SLOWMO_SPEED + (1.0 - SLOWMO_SPEED) * smooth
}

/// Ведёт замедление по реальному времени. На паузе стоит: пауза останавливает время сама.
fn run_slowmo(
    real: Res<Time<Real>>,
    mut time: ResMut<Time<Virtual>>,
    mut slowmo: ResMut<SlowMotion>,
    state: Res<State<GameState>>,
) {
    if *state.get() == GameState::Paused {
        return;
    }
    let dt = real.delta_secs();
    let Some(age) = slowmo.age else {
        slowmo.cooldown = (slowmo.cooldown - dt).max(0.0);
        return;
    };
    let age = age + dt;
    if age >= SLOWMO_HOLD + SLOWMO_RETURN {
        slowmo.age = None;
        slowmo.cooldown = SLOWMO_COOLDOWN;
        time.set_relative_speed(1.0);
    } else {
        slowmo.age = Some(age);
        time.set_relative_speed(slowmo_speed(age));
    }
}

fn reset_slowmo(
    mut started: MessageReader<WorldRebuilt>,
    mut slowmo: ResMut<SlowMotion>,
    mut time: ResMut<Time<Virtual>>,
) {
    if started.read().count() > 0 {
        *slowmo = SlowMotion::default();
        time.set_relative_speed(1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn близкий_взрыв_трясёт_сильнее_далёкого() {
        assert!(trauma(1.4, 5.0) > trauma(1.4, 40.0));
        assert_eq!(trauma(1.4, 80.0), 0.0);
    }

    #[test]
    fn замедление_возвращается_к_обычной_скорости() {
        assert_eq!(slowmo_speed(0.0), SLOWMO_SPEED);
        assert!((slowmo_speed(SLOWMO_HOLD + SLOWMO_RETURN) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn далёкий_взрыв_время_не_замедляет() {
        assert!(slows_time(1.6, 10.0));
        assert!(!slows_time(1.6, SLOWMO_RANGE + 1.0));
        assert!(!slows_time(1.4, 0.0));
    }

    #[test]
    fn идущее_замедление_не_продлевается() {
        let mut slowmo = SlowMotion::default();
        assert!(slowmo.try_start());
        slowmo.age = Some(0.3);
        assert!(!slowmo.try_start());
        assert_eq!(slowmo.age, Some(0.3));
        slowmo.age = None;
        slowmo.cooldown = SLOWMO_COOLDOWN;
        assert!(!slowmo.try_start(), "после замедления новое не начинается сразу");
    }
}
