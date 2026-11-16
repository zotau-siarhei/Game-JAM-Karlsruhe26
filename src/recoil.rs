//! Отдача и вспышка у ствола: каждый выстрел лунохода толкает башню назад, а у дула пулемёта
//! и пусковой ракет вспыхивает огонь.
//!
//! Башня отъезжает назад вдоль своего ствола и мягко возвращается. Вспышка — одна светящаяся
//! капля без источника света, её переставляют к дулу на каждом выстреле: дёшево даже для очереди.
//! У мортиры своя вспышка со светом (`src/mortar.rs`), у лазера светится сам луч.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::bullet::FireBullet;
use crate::combat::Team;
use crate::frame::GameSet;
use crate::laser::FireLaser;
use crate::mortar::FireShell;
use crate::rockets::Rocket;
use crate::run::WorldRebuilt;
use crate::weapons::{TURRET_BASE, Turret};

/// Откат башни от выстрела, м в мерах модели лунохода.
const KICK_BULLET: f32 = 0.035;
const KICK_LASER: f32 = 0.06;
const KICK_ROCKET: f32 = 0.1;
const KICK_SHELL: f32 = 0.16;
/// Как быстро башня возвращается на место, 1/с.
const KICK_RETURN: f32 = 16.0;

/// Вспышка у дула: длина жизни, с, и радиус, м.
const FLASH_BULLET: (f32, f32) = (0.05, 0.22);
const FLASH_ROCKET: (f32, f32) = (0.12, 0.55);
/// Вспышка вытянута вдоль выстрела во столько раз.
const FLASH_STRETCH: f32 = 2.2;
/// Вспышка стоит чуть впереди дула, м.
const FLASH_AHEAD: f32 = 0.25;

/// Откат башни сейчас.
#[derive(Resource, Default)]
struct Recoil {
    kick: f32,
}

/// Вспышка у дула: сколько ей осталось и какой она была в начале.
#[derive(Component, Default)]
struct Flash {
    left: f32,
    life: f32,
    radius: f32,
}

/// Плагин отдачи и вспышки.
pub struct RecoilPlugin;

impl Plugin for RecoilPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Recoil>()
            .add_systems(Startup, spawn_flash)
            .add_systems(
                Update,
                (
                    reset.in_set(GameSet::Reset),
                    (kick_turret, flash_muzzle).in_set(GameSet::Presentation),
                ),
            );
    }
}

fn spawn_flash(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Flash::default(),
        Mesh3d(meshes.add(Sphere::new(1.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            // Ярче единицы: светится через свечение камеры, как вспышка мортиры.
            base_color: Color::linear_rgb(5.0, 3.0, 1.0),
            unlit: true,
            ..default()
        })),
        Transform::default(),
        Visibility::Hidden,
        NotShadowCaster,
    ));
}

/// Мир перестроен: башня на месте, вспышки нет.
fn reset(
    mut rebuilt: MessageReader<WorldRebuilt>,
    mut recoil: ResMut<Recoil>,
    mut flashes: Query<(&mut Flash, &mut Visibility)>,
) {
    if rebuilt.read().last().is_none() {
        return;
    }
    recoil.kick = 0.0;
    for (mut flash, mut visibility) in &mut flashes {
        flash.left = 0.0;
        *visibility = Visibility::Hidden;
    }
}

/// Откат после шага `dt`: экспоненциально возвращается к нулю.
fn settle(kick: f32, dt: f32) -> f32 {
    kick * (-KICK_RETURN * dt).exp()
}

/// Выстрелы игрока толкают башню назад вдоль её ствола.
#[allow(clippy::too_many_arguments)]
fn kick_turret(
    time: Res<Time>,
    mut bullets: MessageReader<FireBullet>,
    mut shells: MessageReader<FireShell>,
    mut lasers: MessageReader<FireLaser>,
    rockets: Query<&Rocket, Added<Rocket>>,
    mut recoil: ResMut<Recoil>,
    mut turrets: Query<&mut Transform, With<Turret>>,
) {
    let mut kick = settle(recoil.kick, time.delta_secs());
    let mut push = |amount: f32| kick = kick.max(amount);
    if bullets.read().any(|shot| shot.team == Team::Player) {
        push(KICK_BULLET);
    }
    if lasers.read().any(|shot| shot.team == Team::Player) {
        push(KICK_LASER);
    }
    if rockets.iter().any(|rocket| rocket.team() == Team::Player) {
        push(KICK_ROCKET);
    }
    if shells.read().any(|shot| shot.team == Team::Player) {
        push(KICK_SHELL);
    }
    recoil.kick = kick;
    for mut transform in &mut turrets {
        // Ствол смотрит на −Z башни, отдача — назад, на +Z.
        transform.translation = TURRET_BASE + transform.rotation * Vec3::Z * kick;
    }
}

/// Радиус вспышки, когда ей осталось `left` из `life` секунд: сжимается к половине.
fn flash_radius(radius: f32, left: f32, life: f32) -> f32 {
    radius * (0.5 + 0.5 * (left / life).clamp(0.0, 1.0))
}

/// Вспышка у дула пулемёта и ракет игрока: вытянутая вдоль выстрела, гаснет за доли секунды.
fn flash_muzzle(
    time: Res<Time>,
    mut bullets: MessageReader<FireBullet>,
    rockets: Query<(&Transform, &Rocket), (Added<Rocket>, Without<Flash>)>,
    mut flashes: Query<(&mut Flash, &mut Transform, &mut Visibility)>,
) {
    let mut shot = None;
    for bullet in bullets.read().filter(|shot| shot.team == Team::Player) {
        shot = Some((bullet.origin, bullet.direction, FLASH_BULLET));
    }
    for (transform, rocket) in &rockets {
        if rocket.team() == Team::Player {
            // Ракета летит вдоль своей оси Y (`rockets::spawn_rocket`), пламя — позади неё.
            shot = Some((
                transform.translation,
                transform.up().as_vec3(),
                FLASH_ROCKET,
            ));
        }
    }
    let Ok((mut flash, mut transform, mut visibility)) = flashes.single_mut() else {
        return;
    };
    if let Some((origin, direction, (life, radius))) = shot {
        let direction = direction.normalize_or(Vec3::NEG_Z);
        *flash = Flash {
            left: life,
            life,
            radius,
        };
        transform.translation = origin + direction * FLASH_AHEAD;
        transform.look_to(direction, Vec3::Y);
    } else {
        flash.left -= time.delta_secs();
    }
    if flash.left <= 0.0 {
        if *visibility != Visibility::Hidden {
            *visibility = Visibility::Hidden;
        }
        return;
    }
    let r = flash_radius(flash.radius, flash.left, flash.life);
    transform.scale = Vec3::new(r, r, r * FLASH_STRETCH);
    *visibility = Visibility::Visible;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn башня_возвращается_после_отдачи() {
        let after = settle(KICK_SHELL, 0.25);
        assert!(
            after < KICK_SHELL * 0.05,
            "за четверть секунды откат почти прошёл: {after}"
        );
        assert!(
            settle(KICK_SHELL, 0.016) > KICK_SHELL * 0.7,
            "за кадр откат не пропадает"
        );
    }

    #[test]
    fn отдача_по_мощности_оружия() {
        assert!(KICK_BULLET < KICK_LASER && KICK_LASER < KICK_ROCKET && KICK_ROCKET < KICK_SHELL);
    }

    #[test]
    fn вспышка_сжимается_к_половине() {
        assert_eq!(flash_radius(1.0, 1.0, 1.0), 1.0);
        assert_eq!(flash_radius(1.0, 0.0, 1.0), 0.5);
    }
}
