//! Взрыв — общее сообщение, на которое реагируют кратеры, эффекты, тряска, а позже урон
//! и разрушение домов. Отправить взрыв может любое оружие:
//!
//! ```ignore
//! fn on_hit(mut explosions: MessageWriter<Explosion>) {
//!     explosions.write(Explosion { position, radius: 5.0, power: 1.0, source_team: Team::Player });
//! }
//! ```

use bevy::prelude::*;

use crate::combat::Team;

/// Взрыв в точке мира.
#[derive(Message, Clone, Copy, Debug)]
pub struct Explosion {
    /// Центр взрыва, обычно точка касания поверхности.
    pub position: Vec3,
    /// Радиус воздействия, м.
    pub radius: f32,
    /// Мощность: 1 — снаряд мортиры, от неё масштабируются кратер, эффекты и тряска.
    pub power: f32,
    /// Чей взрыв: своей команде он наносит лишь малую часть урона.
    pub source_team: Team,
}

/// Плагин взрывов: регистрирует сообщение и пишет взрывы в отладочный лог.
pub struct ExplosionPlugin;

impl Plugin for ExplosionPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Explosion>()
            .add_systems(Update, log_explosions);
    }
}

/// Пишет каждый взрыв в отладочный лог: видно при `RUST_LOG=lunokhod=debug`.
fn log_explosions(mut explosions: MessageReader<Explosion>) {
    for e in explosions.read() {
        debug!(
            "Взрыв в ({:.1}, {:.1}, {:.1}), радиус {:.1} м, мощность {:.2}",
            e.position.x, e.position.y, e.position.z, e.radius, e.power
        );
    }
}
