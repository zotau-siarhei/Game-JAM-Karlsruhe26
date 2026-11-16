//! Оружие лунохода: выбор оружия, башня, которая смотрит на точку прицела, и пулемёт.
//!
//! Выбранное оружие лежит в ресурсе [`CurrentWeapon`]. Каждое оружие само проверяет,
//! выбрано ли оно, прежде чем стрелять по левой кнопке мыши.

use bevy::prelude::*;

use crate::aim::AimPoint;
use crate::bullet::FireBullet;
use crate::combat::{Health, Team};
use crate::frame::GameSet;
use crate::models::Models;
use crate::rng::GameRng;
use crate::rover::{self, Rover};
use crate::upgrades::{self, WeaponLevels};
use crate::vehicle;

/// Выстрелов пулемёта в секунду.
pub const MG_RATE: f32 = 8.0;

/// Урон пули пулемёта.
pub const MG_DAMAGE: f32 = 6.0;

/// Разброс пуль пулемёта по горизонтали в каждую сторону, рад.
const MG_SPREAD: f32 = 1.2 * std::f32::consts::PI / 180.0;

/// Ось башни в системе лунохода, м: на солнечной панели позади мачты с глазами.
const TURRET_OFFSET: Vec3 = Vec3::new(0.0, 0.0, 0.3);

/// Высота башни над центром лунохода, м: башня стоит на крышке корпуса.
const TURRET_HEIGHT: f32 = 0.46;
/// Место башни на корпусе без отдачи (`src/recoil.rs`), м в мерах модели.
pub(crate) const TURRET_BASE: Vec3 = Vec3::new(TURRET_OFFSET.x, TURRET_OFFSET.y + TURRET_HEIGHT, TURRET_OFFSET.z);

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

/// Модель оружия стоит на крышке башни и вынесена вперёд от её оси, м: (высота, вынос).
const WEAPON_MOUNT: (f32, f32) = (0.11, 0.1);

/// Подключает выбор оружия, башню и пулемёт.
pub struct WeaponsPlugin;

impl Plugin for WeaponsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CurrentWeapon>()
            .add_systems(
                Update,
                (
                    attach_turret.in_set(GameSet::Reset),
                    show_weapon.after(attach_turret).in_set(GameSet::Presentation),
                    show_barrels.after(show_weapon).in_set(GameSet::Presentation),
                    (aim_turret, fire_machine_gun)
                        .chain()
                        .after(rover::drive_rover)
                        .in_set(GameSet::Input),
                ),
            );
    }
}

/// Оружие, которым сейчас стреляет левая кнопка мыши.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CurrentWeapon {
    #[default]
    MachineGun,
    Mortar,
    Rockets,
    HomingRockets,
    Laser,
    Airstrike,
}

impl CurrentWeapon {
    /// Модель насадки на башне из `models.json`. У авиаудара насадки нет.
    fn model(self) -> Option<&'static str> {
        match self {
            CurrentWeapon::MachineGun => Some("weapon_mg"),
            CurrentWeapon::Mortar => Some("weapon_mortar"),
            CurrentWeapon::Rockets | CurrentWeapon::HomingRockets => Some("weapon_rockets"),
            CurrentWeapon::Laser => Some("weapon_laser"),
            CurrentWeapon::Airstrike => None,
        }
    }

    /// Все виды оружия по порядку клавиш 1–6. Боезапас и параметры — в модуле `arsenal`.
    pub const ALL: [CurrentWeapon; 6] = [
        CurrentWeapon::MachineGun,
        CurrentWeapon::Mortar,
        CurrentWeapon::Rockets,
        CurrentWeapon::HomingRockets,
        CurrentWeapon::Laser,
        CurrentWeapon::Airstrike,
    ];

    /// Название для интерфейса.
    pub fn name(self) -> &'static str {
        match self {
            CurrentWeapon::MachineGun => "MACHINE GUN",
            CurrentWeapon::Mortar => "MORTAR",
            CurrentWeapon::Rockets => "ROCKETS",
            CurrentWeapon::HomingRockets => "HOMING ROCKETS",
            CurrentWeapon::Laser => "LASER",
            CurrentWeapon::Airstrike => "AIRSTRIKE",
        }
    }
}

/// Башня лунохода: вращается к точке прицела независимо от корпуса.
#[derive(Component, Default)]
pub(crate) struct Turret {
    /// Поворот башни относительно корпуса вокруг его оси «вверх», рад. Ноль — вперёд.
    yaw: f32,
}

/// Место насадки оружия на башне. Пока модель грузится или без неё насадка — простой ствол.
#[derive(Component)]
struct WeaponMount {
    barrel: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

/// Время до следующего выстрела пулемёта, с, и ствол следующего выстрела.
#[derive(Component, Default)]
struct MachineGun {
    cooldown: f32,
    next_barrel: u8,
}

/// Ствол пулемёта на башне: `0` — основной, `1` — второй, виден со второго уровня.
#[derive(Component)]
struct Barrel(u8);

/// Сдвиг стволов спаренного пулемёта вбок от оси башни, м в мерах модели.
const BARREL_SPACING: f32 = 0.1;

impl MachineGun {
    /// Сколько пуль выпустить за кадр длиной `dt`, пока зажата кнопка, при темпе `rate` в секунду.
    fn shots(&mut self, dt: f32, trigger: bool, rate: f32) -> u32 {
        self.cooldown -= dt;
        if !trigger {
            self.cooldown = self.cooldown.max(0.0);
            return 0;
        }
        let mut shots = 0;
        while self.cooldown <= 0.0 {
            self.cooldown += 1.0 / rate;
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
                    Transform::from_translation(TURRET_BASE),
                ))
                .with_children(|turret| {
                    turret.spawn((
                        WeaponMount {
                            barrel: barrel.clone(),
                            material: barrel_material.clone(),
                        },
                        Transform::default(),
                        Visibility::default(),
                    ));
                    // Второй ствол спаренного пулемёта: виден со второго уровня (`show_barrels`).
                    turret.spawn((
                        Barrel(1),
                        Mesh3d(barrel.clone()),
                        MeshMaterial3d(barrel_material.clone()),
                        Transform::from_xyz(0.0, MUZZLE_HEIGHT - TURRET_HEIGHT, -MUZZLE_FORWARD * 0.5)
                            .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                        Visibility::Hidden,
                    ));
                });
        });
    }
}

/// Ставит на башню насадку выбранного оружия: модель из списка, пока её нет — ствол.
fn show_weapon(
    mut commands: Commands,
    current: Res<CurrentWeapon>,
    mut models: Models,
    mounts: Query<(Entity, Ref<WeaponMount>)>,
) {
    for (entity, mount) in &mounts {
        if !current.is_changed() && !mount.is_added() {
            continue;
        }
        let mut mount_commands = commands.entity(entity);
        mount_commands.despawn_related::<Children>();
        let Some(id) = current.model() else {
            continue;
        };
        let at = Transform::from_xyz(0.0, WEAPON_MOUNT.0, -WEAPON_MOUNT.1);
        mount_commands.with_children(|parent| {
            models.spawn_visual(parent, id, at, |spare| {
                // Цилиндр стоит вдоль Y: кладём его стволом вперёд, на -Z.
                spare.spawn((
                    Barrel(0),
                    Mesh3d(mount.barrel.clone()),
                    MeshMaterial3d(mount.material.clone()),
                    Transform::from_xyz(0.0, MUZZLE_HEIGHT - TURRET_HEIGHT, -MUZZLE_FORWARD * 0.5)
                        .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                ));
            });
        });
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
    let scale = crate::rover::ROVER_SCALE;
    let origin = rover + (Vec3::Y * MUZZLE_HEIGHT + forward * MUZZLE_FORWARD) * scale;
    let target = aim + Vec3::Y * AIM_HEIGHT;
    // Прицел у самого лунохода: стреляем вперёд по башне, а не себе под колёса.
    let direction = if flat.length() > MUZZLE_FORWARD + 0.5 {
        target - origin
    } else {
        forward
    };
    (origin, direction)
}

/// Сдвиг ствола `index` вбок от оси башни при `barrels` стволах, м в мерах модели.
fn barrel_offset(barrels: u8, index: u8) -> f32 {
    if barrels < 2 {
        0.0
    } else if index == 0 {
        -BARREL_SPACING
    } else {
        BARREL_SPACING
    }
}

/// Стволы на башне по уровню пулемёта: со второго уровня их два, рядом.
///
/// Основной ствол — запасной облик насадки пулемёта (`show_weapon`), он есть, пока модели нет;
/// второй ствол стоит на башне всегда и виден только у выбранного пулемёта.
fn show_barrels(
    levels: Res<WeaponLevels>,
    current: Res<CurrentWeapon>,
    mut barrels: Query<(&Barrel, &mut Transform, &mut Visibility)>,
) {
    let count = upgrades::machine_gun(levels.level(CurrentWeapon::MachineGun)).barrels;
    let machine_gun = *current == CurrentWeapon::MachineGun;
    for (barrel, mut transform, mut visibility) in &mut barrels {
        transform.translation.x = barrel_offset(count, barrel.0);
        let shown = barrel.0 == 0 || (machine_gun && barrel.0 < count);
        let wanted = if shown { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

/// Пулемётная очередь, пока зажата левая кнопка мыши и выбран пулемёт. Число стволов, темп, урон
/// и тяжесть пуль — по уровню (`upgrades::machine_gun`); выстрелы чередуют стволы.
#[allow(clippy::too_many_arguments)]
fn fire_machine_gun(
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    weapon: Res<CurrentWeapon>,
    levels: Res<WeaponLevels>,
    aim: Res<AimPoint>,
    mut dice: ResMut<GameRng>,
    mut rover: Query<(&Transform, &Health, &mut MachineGun), With<Rover>>,
    mut bullets: MessageWriter<FireBullet>,
) {
    let Some((transform, health, mut gun)) = rover.iter_mut().next() else {
        return;
    };
    let trigger = mouse.pressed(MouseButton::Left)
        && *weapon == CurrentWeapon::MachineGun
        && !health.is_dead();
    let level = upgrades::machine_gun(levels.level(CurrentWeapon::MachineGun));
    for _ in 0..gun.shots(time.delta_secs(), trigger, level.rate) {
        // Пуля вылетает из ствола: ось башни позади центра лунохода.
        let pivot = transform.translation + transform.rotation * TURRET_OFFSET;
        let (origin, direction) = machine_gun_shot(pivot, aim.position);
        let barrel = gun.next_barrel % level.barrels.max(1);
        gun.next_barrel = gun.next_barrel.wrapping_add(1);
        let side = Vec3::Y.cross(direction.with_y(0.0)).normalize_or_zero();
        let origin = origin - side * barrel_offset(level.barrels, barrel) * crate::rover::ROVER_SCALE;
        let spread = Quat::from_rotation_y(dice.range(-MG_SPREAD, MG_SPREAD));
        bullets.write(FireBullet {
            origin,
            direction: spread * direction,
            team: Team::Player,
            damage: level.damage,
            heavy: level.heavy,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;

    use super::*;

    #[test]
    fn machine_gun_fires_eight_shots_a_second_while_held() {
        let mut gun = MachineGun::default();
        let shots: u32 = (0..60).map(|_| gun.shots(1.0 / 60.0, true, MG_RATE)).sum();
        assert!((8..=9).contains(&shots), "{shots}");
        // Кнопка отпущена: выстрелов нет, запас времени не копится.
        let idle: u32 = (0..120).map(|_| gun.shots(1.0 / 60.0, false, MG_RATE)).sum();
        assert_eq!(idle, 0);
        assert_eq!(gun.shots(1.0 / 60.0, true, MG_RATE), 1);
    }

    #[test]
    fn slow_frame_does_not_lose_shots() {
        let mut gun = MachineGun::default();
        // Кадр в полсекунды: четыре выстрела с прошлого кадра плюс текущий.
        assert_eq!(gun.shots(0.5, true, MG_RATE), 5);
    }

    #[test]
    fn twin_machine_gun_fires_twice_as_fast() {
        let rate = upgrades::machine_gun(2).rate;
        let mut gun = MachineGun::default();
        let shots: u32 = (0..60).map(|_| gun.shots(1.0 / 60.0, true, rate)).sum();
        assert!((16..=17).contains(&shots), "{shots}");
        assert_eq!(barrel_offset(2, 0), -barrel_offset(2, 1));
        assert_eq!(barrel_offset(1, 0), 0.0);
    }

    #[test]
    fn bullet_flies_from_the_muzzle_towards_the_aim_point() {
        let rover = Vec3::new(10.0, 2.0, 10.0);
        let aim = Vec3::new(10.0, 5.0, -10.0);
        let (origin, direction) = machine_gun_shot(rover, aim);
        // Дуло вынесено вперёд, в сторону прицела, и приподнято: вынос и высота заданы в мерах
        // модели и растут вместе с луноходом.
        let scale = crate::rover::ROVER_SCALE;
        let muzzle = Vec3::new(10.0, 2.0 + MUZZLE_HEIGHT * scale, 10.0 - MUZZLE_FORWARD * scale);
        assert!((origin - muzzle).length() < 1e-4, "{origin}");
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
