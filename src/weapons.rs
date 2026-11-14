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

/// Ось башни в системе лунохода, м: на солнечной панели позади мачты с глазами.
const TURRET_OFFSET: Vec3 = Vec3::new(0.0, 0.0, 0.3);

/// Высота башни над центром лунохода, м: башня стоит на крышке корпуса.
const TURRET_HEIGHT: f32 = 0.46;

/// Скорость поворота башни, рад/с: пол-оборота примерно за треть секунды.
const TURRET_TURN_SPEED: f32 = 9.0;

/// Высота дула над центром корпуса, м.
const MUZZLE_HEIGHT: f32 = 0.5;

/// Вынос дула вперёд от оси башни, м.
const MUZZLE_FORWARD: f32 = 1.0;

/// На какой высоте над точкой прицела проходит пуля, м: примерно середина корпуса врага.
const AIM_HEIGHT: f32 = 0.6;

/// Цвет башни: белый, как отделка корпуса.
const TURRET_COLOR: Color = Color::srgb(0.88, 0.9, 0.92);

/// Цвет ствола.
const BARREL_COLOR: Color = Color::srgb(0.25, 0.27, 0.3);

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
#[derive(Component, Default)]
struct Turret {
    /// Поворот башни относительно корпуса вокруг его оси «вверх», рад. Ноль — вперёд.
    yaw: f32,
}

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
        let barrel_material = materials.add(BARREL_COLOR);
        let base = meshes.add(Cylinder::new(0.26, 0.22));
        let barrel = meshes.add(Cylinder::new(0.06, MUZZLE_FORWARD));
        commands.entity(rover).insert(MachineGun::default()).with_children(|parent| {
            parent
                .spawn((
                    Turret::default(),
                    Mesh3d(base),
                    MeshMaterial3d(material.clone()),
                    Transform::from_translation(TURRET_OFFSET + Vec3::Y * TURRET_HEIGHT),
                ))
                .with_children(|turret| {
                    // Цилиндр стоит вдоль Y: кладём его стволом вперёд, на -Z.
                    turret.spawn((
                        Mesh3d(barrel),
                        MeshMaterial3d(barrel_material.clone()),
                        Transform::from_xyz(0.0, MUZZLE_HEIGHT - TURRET_HEIGHT, -MUZZLE_FORWARD * 0.5)
                            .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                    ));
                });
        });
    }
}

/// Пауза после переключения колесом, с: одна прокрутка растянута на несколько кадров.
const SCROLL_COOLDOWN: f32 = 0.25;

/// Клавиши 1 и 2 выбирают оружие, колесо мыши листает по кругу: одна прокрутка — одно оружие.
fn select_weapon(
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    mut cooldown: Local<f32>,
    mut weapon: ResMut<CurrentWeapon>,
) {
    *cooldown = (*cooldown - time.delta_secs()).max(0.0);
    let mut chosen = *weapon;
    if keys.just_pressed(KeyCode::Digit1) {
        chosen = CurrentWeapon::MachineGun;
    } else if keys.just_pressed(KeyCode::Digit2) {
        chosen = CurrentWeapon::Mortar;
    } else if scroll.delta.y != 0.0 {
        // Пока колесо крутится, пауза продлевается: переключение только в начале прокрутки.
        if *cooldown == 0.0 {
            chosen = weapon.cycled(if scroll.delta.y > 0.0 { -1 } else { 1 });
        }
        *cooldown = SCROLL_COOLDOWN;
    }
    if chosen != *weapon {
        *weapon = chosen;
    }
}

/// Поворачивает башню к точке прицела вокруг оси корпуса с ограниченной скоростью.
///
/// Направление на прицел переводится в систему корпуса и проецируется на его плоскость: на склоне
/// башня крутится по крышке, а не заваливается. Поворот корпуса тянет башню за собой, и она
/// догоняет прицел по кратчайшей дуге.
fn aim_turret(
    time: Res<Time>,
    aim: Res<AimPoint>,
    rover: Query<&Transform, (With<Rover>, Without<Turret>)>,
    mut turrets: Query<(&mut Turret, &mut Transform)>,
) {
    let Some(rover) = rover.iter().next() else {
        return;
    };
    let Some(target) = turret_yaw_towards(rover, aim.position) else {
        return;
    };
    for (mut turret, mut transform) in &mut turrets {
        turret.yaw = vehicle::steer(turret.yaw, target, TURRET_TURN_SPEED * time.delta_secs());
        transform.rotation = Quat::from_rotation_y(turret.yaw);
    }
}

/// Поворот башни относительно корпуса, при котором она смотрит на точку `aim`.
/// `None`, если прицел над самой башней и направление не определено.
fn turret_yaw_towards(rover: &Transform, aim: Vec3) -> Option<f32> {
    let pivot = rover.transform_point(TURRET_OFFSET);
    let mut local = rover.rotation.inverse() * (aim - pivot);
    local.y = 0.0;
    (local.length_squared() >= 0.25).then(|| vehicle::heading_towards(local))
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
        // Пуля вылетает из ствола: ось башни позади центра лунохода.
        let pivot = transform.translation + transform.rotation * TURRET_OFFSET;
        let (origin, direction) = machine_gun_shot(pivot, aim.position);
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
    use std::f32::consts::FRAC_PI_2;

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
    fn turret_turns_only_around_the_hull_axis() {
        // Луноход на склоне повёрнут на 90° влево и наклонён: прицел впереди по курсу корпуса.
        let rover = Transform::from_rotation(Quat::from_rotation_x(0.4) * Quat::from_rotation_y(FRAC_PI_2));
        let ahead = rover.transform_point(Vec3::new(0.0, 0.0, -10.0));
        let yaw = turret_yaw_towards(&rover, ahead).unwrap();
        assert!(yaw.abs() < 1e-3, "прицел по курсу — башня смотрит вперёд, а не {yaw}");
        // Прицел справа от корпуса: башня поворачивается направо (курс уменьшается).
        let right = rover.transform_point(Vec3::new(10.0, 0.0, 0.3));
        let yaw = turret_yaw_towards(&rover, right).unwrap();
        assert!((yaw + FRAC_PI_2).abs() < 0.05, "{yaw}");
        // Прицел над башней — направления нет.
        assert!(turret_yaw_towards(&rover, rover.transform_point(TURRET_OFFSET + Vec3::Y)).is_none());
    }

    #[test]
    fn turret_catches_up_with_limited_speed() {
        let mut yaw = 0.0;
        let dt = 1.0 / 60.0;
        let frames = (0..60)
            .take_while(|_| {
                yaw = vehicle::steer(yaw, 3.0, TURRET_TURN_SPEED * dt);
                (yaw - 3.0f32).abs() > 1e-4
            })
            .count();
        // Почти пол-оборота — не мгновенно, но быстрее полусекунды.
        assert!((10..30).contains(&frames), "{frames}");
    }

    #[test]
    fn aim_point_under_the_rover_shoots_forward() {
        let rover = Vec3::new(0.0, 1.0, 0.0);
        let (_, direction) = machine_gun_shot(rover, Vec3::new(0.3, 0.2, 0.0));
        assert!(direction.y.abs() < 1e-4);
        assert!(direction.x > 0.9);
    }
}
