//! Оружие лунохода: выбор оружия, башня, которая смотрит на точку прицела, и пулемёт.
//!
//! Выбранное оружие лежит в ресурсе [`CurrentWeapon`]. Каждое оружие само проверяет,
//! выбрано ли оно, прежде чем стрелять по левой кнопке мыши.

use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;

use crate::aim::AimPoint;
use crate::bullet::FireBullet;
use crate::combat::{Health, Team};
use crate::rover::{self, Rover};
use crate::run::GameState;
use crate::sound_list::Dice;
use crate::vehicle;

/// Выстрелов пулемёта в секунду.
const MG_RATE: f32 = 8.0;

/// Урон пули пулемёта.
const MG_DAMAGE: f32 = 6.0;

/// Разброс пуль пулемёта по горизонтали в каждую сторону, рад.
const MG_SPREAD: f32 = 1.2 * std::f32::consts::PI / 180.0;

/// Высота башни над центром корпуса, м.
const TURRET_HEIGHT: f32 = 0.45;

/// Высота дула над центром корпуса, м.
const MUZZLE_HEIGHT: f32 = 0.5;

/// Вынос дула вперёд от оси башни, м.
const MUZZLE_FORWARD: f32 = 1.0;

/// На какой высоте над точкой прицела проходит пуля, м: примерно середина корпуса врага.
const AIM_HEIGHT: f32 = 0.6;

/// Цвет башни.
const TURRET_COLOR: Color = Color::srgb(0.32, 0.34, 0.38);

/// Подключает выбор оружия, башню и пулемёт.
pub struct WeaponsPlugin;

impl Plugin for WeaponsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CurrentWeapon>()
            .init_resource::<Dice>()
            .add_systems(
                Update,
                (
                    attach_turret,
                    (select_weapon, aim_turret, fire_machine_gun)
                        .chain()
                        .run_if(in_state(GameState::Playing)),
                )
                    .chain()
                    .after(rover::drive_rover),
            );
    }
}

/// Оружие, которым сейчас стреляет левая кнопка мыши.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CurrentWeapon {
    #[default]
    MachineGun,
    Mortar,
}

impl CurrentWeapon {
    /// Всё доступное оружие по порядку клавиш 1, 2.
    pub const ALL: [CurrentWeapon; 2] = [CurrentWeapon::MachineGun, CurrentWeapon::Mortar];

    /// Название для интерфейса.
    pub fn name(self) -> &'static str {
        match self {
            CurrentWeapon::MachineGun => "MACHINE GUN",
            CurrentWeapon::Mortar => "MORTAR",
        }
    }

    /// Соседнее оружие по кругу: `step` больше нуля — следующее, меньше — предыдущее.
    pub fn cycled(self, step: i32) -> Self {
        let count = Self::ALL.len() as i32;
        let index = Self::ALL.iter().position(|weapon| *weapon == self).unwrap_or(0) as i32;
        Self::ALL[(index + step.signum()).rem_euclid(count) as usize]
    }
}

/// Башня лунохода: вращается к точке прицела независимо от корпуса.
#[derive(Component)]
struct Turret;

/// Время до следующего выстрела пулемёта, с.
#[derive(Component, Default)]
struct MachineGun {
    cooldown: f32,
}

impl MachineGun {
    /// Сколько пуль выпустить за кадр длиной `dt`, пока зажата кнопка.
    fn shots(&mut self, dt: f32, trigger: bool) -> u32 {
        self.cooldown -= dt;
        if !trigger {
            self.cooldown = self.cooldown.max(0.0);
            return 0;
        }
        let mut shots = 0;
        while self.cooldown <= 0.0 {
            self.cooldown += 1.0 / MG_RATE;
            shots += 1;
        }
        shots
    }
}

/// Ставит на луноход башню со стволом, как только он появился.
fn attach_turret(
    mut commands: Commands,
    rovers: Query<Entity, Added<Rover>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for rover in &rovers {
        let material = materials.add(TURRET_COLOR);
        let base = meshes.add(Cuboid::new(0.6, 0.3, 0.6));
        let barrel = meshes.add(Cuboid::new(0.12, 0.12, MUZZLE_FORWARD));
        commands.entity(rover).insert(MachineGun::default()).with_children(|parent| {
            parent
                .spawn((
                    Turret,
                    Mesh3d(base),
                    MeshMaterial3d(material.clone()),
                    Transform::from_xyz(0.0, TURRET_HEIGHT, 0.0),
                ))
                .with_children(|turret| {
                    turret.spawn((
                        Mesh3d(barrel),
                        MeshMaterial3d(material.clone()),
                        Transform::from_xyz(0.0, 0.05, -MUZZLE_FORWARD * 0.5 - 0.2),
                    ));
                });
        });
    }
}

/// Клавиши 1 и 2 выбирают оружие, колесо мыши листает по кругу.
fn select_weapon(
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    mut weapon: ResMut<CurrentWeapon>,
) {
    let mut chosen = *weapon;
    if keys.just_pressed(KeyCode::Digit1) {
        chosen = CurrentWeapon::MachineGun;
    } else if keys.just_pressed(KeyCode::Digit2) {
        chosen = CurrentWeapon::Mortar;
    } else if scroll.delta.y != 0.0 {
        chosen = weapon.cycled(if scroll.delta.y > 0.0 { -1 } else { 1 });
    }
    if chosen != *weapon {
        *weapon = chosen;
    }
}

/// Поворачивает башню к точке прицела.
fn aim_turret(
    aim: Res<AimPoint>,
    rover: Query<&Transform, (With<Rover>, Without<Turret>)>,
    mut turrets: Query<&mut Transform, With<Turret>>,
) {
    let Some(rover) = rover.iter().next() else {
        return;
    };
    let mut to_aim = aim.position - rover.translation;
    to_aim.y = 0.0;
    if to_aim.length_squared() < 0.25 {
        return;
    }
    let world_yaw = Quat::from_rotation_y(vehicle::heading_towards(to_aim));
    for mut turret in &mut turrets {
        // Башня — дочерняя сущность: мировой поворот переводится в систему корпуса.
        turret.rotation = rover.rotation.inverse() * world_yaw;
    }
}

/// Откуда и куда летит пуля пулемёта при таком положении лунохода и прицела.
fn machine_gun_shot(rover: Vec3, aim: Vec3) -> (Vec3, Vec3) {
    let mut flat = aim - rover;
    flat.y = 0.0;
    let forward = flat.try_normalize().unwrap_or(Vec3::NEG_Z);
    let origin = rover + Vec3::Y * MUZZLE_HEIGHT + forward * MUZZLE_FORWARD;
    let target = aim + Vec3::Y * AIM_HEIGHT;
    // Прицел у самого лунохода: стреляем вперёд по башне, а не себе под колёса.
    let direction = if flat.length() > MUZZLE_FORWARD + 0.5 {
        target - origin
    } else {
        forward
    };
    (origin, direction)
}

/// Пулемётная очередь, пока зажата левая кнопка мыши и выбран пулемёт.
fn fire_machine_gun(
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    weapon: Res<CurrentWeapon>,
    aim: Res<AimPoint>,
    mut dice: ResMut<Dice>,
    mut rover: Query<(&Transform, &Health, &mut MachineGun), With<Rover>>,
    mut bullets: MessageWriter<FireBullet>,
) {
    let Some((transform, health, mut gun)) = rover.iter_mut().next() else {
        return;
    };
    let trigger = mouse.pressed(MouseButton::Left)
        && *weapon == CurrentWeapon::MachineGun
        && !health.is_dead();
    for _ in 0..gun.shots(time.delta_secs(), trigger) {
        let (origin, direction) = machine_gun_shot(transform.translation, aim.position);
        let spread = Quat::from_rotation_y(dice.range(-MG_SPREAD, MG_SPREAD));
        bullets.write(FireBullet {
            origin,
            direction: spread * direction,
            team: Team::Player,
            damage: MG_DAMAGE,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weapons_cycle_in_both_directions() {
        assert_eq!(CurrentWeapon::default(), CurrentWeapon::MachineGun);
        assert_eq!(CurrentWeapon::MachineGun.cycled(1), CurrentWeapon::Mortar);
        assert_eq!(CurrentWeapon::Mortar.cycled(1), CurrentWeapon::MachineGun);
        assert_eq!(CurrentWeapon::MachineGun.cycled(-1), CurrentWeapon::Mortar);
        assert_eq!(CurrentWeapon::Mortar.cycled(-5), CurrentWeapon::MachineGun);
    }

    #[test]
    fn machine_gun_fires_eight_shots_a_second_while_held() {
        let mut gun = MachineGun::default();
        let shots: u32 = (0..60).map(|_| gun.shots(1.0 / 60.0, true)).sum();
        assert!((8..=9).contains(&shots), "{shots}");
        // Кнопка отпущена: выстрелов нет, запас времени не копится.
        let idle: u32 = (0..120).map(|_| gun.shots(1.0 / 60.0, false)).sum();
        assert_eq!(idle, 0);
        assert_eq!(gun.shots(1.0 / 60.0, true), 1);
    }

    #[test]
    fn slow_frame_does_not_lose_shots() {
        let mut gun = MachineGun::default();
        // Кадр в полсекунды: четыре выстрела с прошлого кадра плюс текущий.
        assert_eq!(gun.shots(0.5, true), 5);
    }

    #[test]
    fn bullet_flies_from_the_muzzle_towards_the_aim_point() {
        let rover = Vec3::new(10.0, 2.0, 10.0);
        let aim = Vec3::new(10.0, 5.0, -10.0);
        let (origin, direction) = machine_gun_shot(rover, aim);
        // Дуло вынесено вперёд, в сторону прицела, и приподнято.
        assert!((origin - Vec3::new(10.0, 2.0 + MUZZLE_HEIGHT, 10.0 - MUZZLE_FORWARD)).length() < 1e-4);
        let direction = direction.normalize();
        assert!(direction.z < -0.9);
        // Прицел выше лунохода: пуля летит вверх по склону.
        assert!(direction.y > 0.0);
    }

    #[test]
    fn aim_point_under_the_rover_shoots_forward() {
        let rover = Vec3::new(0.0, 1.0, 0.0);
        let (_, direction) = machine_gun_shot(rover, Vec3::new(0.3, 0.2, 0.0));
        assert!(direction.y.abs() < 1e-4);
        assert!(direction.x > 0.9);
    }
}
