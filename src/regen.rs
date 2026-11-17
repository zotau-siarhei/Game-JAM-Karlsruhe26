//! Самовосстановление лунохода: если его никто не ранит дольше паузы, здоровье понемногу
//! растёт само, пока не станет полным.
//!
//! Скорость и пауза — ресурс [`RegenSettings`]: его выставляет сложность забега при перестройке
//! мира (`difficulty::Difficulty::regen`).
//! Восстановление идёт сообщением `Heal`, как ремонт и аптечки, и складывается с ними. Урон
//! приходит сообщением `Damage`, поэтому и в режиме `--immortal` попадание сбивает отсчёт.

use bevy::prelude::*;

use crate::combat::{Damage, Heal, Health};
use crate::difficulty::RunDifficulty;
use crate::frame::GameSet;
use crate::rover::Rover;
use crate::run::WorldRebuilt;

/// Скорость и пауза восстановления. Значения по умолчанию — нормальная сложность.
#[derive(Resource, Debug, Clone, Copy)]
pub struct RegenSettings {
    /// Сколько секунд без урона до начала восстановления.
    pub delay: f32,
    /// Доля полного здоровья в секунду.
    pub rate: f32,
}

impl Default for RegenSettings {
    fn default() -> Self {
        Self {
            delay: 5.0,
            rate: 0.01,
        }
    }
}

/// Состояние восстановления в забеге.
#[derive(Resource, Debug, Default)]
pub struct Regen {
    /// Секунд с последнего урона лунохода.
    since_hit: f32,
    /// Восстановление идёт сейчас: полоса здоровья пульсирует.
    pub active: bool,
}

/// Плагин самовосстановления.
pub struct RegenPlugin;

impl Plugin for RegenPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RegenSettings>()
            .init_resource::<Regen>()
            .add_systems(
                Update,
                (
                    reset_regen.in_set(GameSet::Reset),
                    // Здесь видны все `Damage` этого кадра: попадание сразу сбивает отсчёт.
                    regenerate.in_set(GameSet::Death),
                ),
            );
    }
}

/// Мир перестроен: отсчёт начинается заново, скорость и пауза — от сложности забега.
fn reset_regen(
    mut rebuilt: MessageReader<WorldRebuilt>,
    difficulty: Option<Res<RunDifficulty>>,
    mut regen: ResMut<Regen>,
    mut settings: ResMut<RegenSettings>,
) {
    if rebuilt.read().last().is_some() {
        *regen = Regen::default();
        *settings = difficulty.map_or_else(RegenSettings::default, |d| d.0.regen());
    }
}

/// Сколько здоровья добавить за шаг `dt`: ноль, если урон был недавно, луноход погиб или здоров.
fn regen_amount(settings: &RegenSettings, since_hit: f32, health: &Health, dt: f32) -> f32 {
    if since_hit < settings.delay || health.is_dead() || health.current >= health.max {
        return 0.0;
    }
    (settings.rate * health.max * dt).min(health.max - health.current)
}

/// Урон лунохода обнуляет отсчёт; после паузы здоровье растёт сообщением `Heal`.
fn regenerate(
    time: Res<Time>,
    settings: Res<RegenSettings>,
    mut regen: ResMut<Regen>,
    mut damages: MessageReader<Damage>,
    rover: Query<(Entity, &Health), With<Rover>>,
    mut heals: MessageWriter<Heal>,
) {
    let Ok((rover, health)) = rover.single() else {
        damages.clear();
        regen.active = false;
        return;
    };
    let hit = damages
        .read()
        .any(|damage| damage.target == rover && damage.amount > 0.0);
    let dt = time.delta_secs();
    regen.since_hit = if hit { 0.0 } else { regen.since_hit + dt };
    let amount = regen_amount(&settings, regen.since_hit, health, dt);
    regen.active = amount > 0.0;
    if regen.active {
        heals.write(Heal {
            target: rover,
            amount,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wounded() -> Health {
        Health {
            current: 500.0,
            max: 1500.0,
        }
    }

    #[test]
    fn здоровье_растёт_после_паузы() {
        let settings = RegenSettings::default();
        let amount = regen_amount(&settings, settings.delay + 0.1, &wounded(), 1.0);
        assert!((amount - 15.0).abs() < 1e-3);
    }

    #[test]
    fn сразу_после_урона_не_растёт() {
        let settings = RegenSettings::default();
        assert_eq!(regen_amount(&settings, 2.0, &wounded(), 1.0), 0.0);
    }

    #[test]
    fn не_выше_максимума() {
        let settings = RegenSettings::default();
        let almost = Health {
            current: 1495.0,
            max: 1500.0,
        };
        assert!((regen_amount(&settings, 10.0, &almost, 1.0) - 5.0).abs() < 1e-3);
        let full = Health {
            current: 1500.0,
            max: 1500.0,
        };
        assert_eq!(regen_amount(&settings, 10.0, &full, 1.0), 0.0);
    }

    #[test]
    fn погибший_не_восстанавливается() {
        let settings = RegenSettings::default();
        let dead = Health {
            current: 0.0,
            max: 1500.0,
        };
        assert_eq!(regen_amount(&settings, 10.0, &dead, 1.0), 0.0);
    }

    #[test]
    fn на_тяжёлой_через_6_секунд_ещё_не_растёт() {
        let hard = crate::difficulty::Difficulty::Hard.regen();
        assert_eq!(regen_amount(&hard, 6.0, &wounded(), 1.0), 0.0);
        assert!(regen_amount(&hard, 8.5, &wounded(), 1.0) > 0.0);
    }

    #[test]
    fn скорость_задаётся_настройкой() {
        let double = RegenSettings {
            rate: 0.02,
            ..RegenSettings::default()
        };
        let base = regen_amount(&RegenSettings::default(), 10.0, &wounded(), 1.0);
        assert!((regen_amount(&double, 10.0, &wounded(), 1.0) - 2.0 * base).abs() < 1e-3);
    }
}
