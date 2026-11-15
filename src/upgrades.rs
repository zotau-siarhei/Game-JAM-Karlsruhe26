//! Уровни оружия: каждое оружие лунохода растёт за забег с первого до третьего уровня от золотых
//! ящиков улучшения.
//!
//! Уровни лежат в ресурсе [`WeaponLevels`] и держатся до перестройки мира, даже если боезапас кончился.
//! Оружие перед выстрелом берёт числа своего уровня из таблиц ниже ([`machine_gun`], [`mortar`],
//! [`rockets`], [`homing`], [`laser`], [`airstrike`]) вместо своих констант.
//!
//! Подбор золотого ящика шлёт [`UpgradePicked`]. Плагин выбирает, что улучшить ([`pick_upgrade`]):
//! выбранное оружие, иначе самое слабое из имеющихся, иначе полный боезапас всем. Об улучшении он
//! сообщает [`WeaponUpgraded`]: по нему надпись, звук, реплика штаба и вспышка на панели.
//!
//! В отладочной сборке клавиша U в забеге даёт улучшение, как золотой ящик.

use bevy::prelude::*;

use crate::arsenal::{Inventory, spec};
use crate::frame::{GameSet, RunMessages};
use crate::laser::{BEAM_LENGTH, TRENCH_HALF_WIDTH};
use crate::rockets::{SALVO, Warhead};
use crate::run::WorldRebuilt;
use crate::weapons::{CurrentWeapon, MG_DAMAGE, MG_RATE};

/// Наибольший уровень оружия.
pub const MAX_LEVEL: u8 = 3;

/// Подключает уровни оружия.
pub struct UpgradesPlugin;

impl Plugin for UpgradesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WeaponLevels>()
            .add_run_message::<UpgradePicked>()
            .add_run_message::<WeaponUpgraded>()
            .add_systems(
                Update,
                (
                    reset_levels.in_set(GameSet::Reset),
                    apply_picked.in_set(GameSet::Simulation),
                ),
            );
        // Отладка: клавиша U в забеге — как подбор золотого ящика. Только в отладочной сборке.
        #[cfg(debug_assertions)]
        app.add_systems(Update, debug_upgrade.in_set(GameSet::Input));
    }
}

/// Отладочное улучшение по клавише U: проверить уровни оружия, не ища золотой ящик.
#[cfg(debug_assertions)]
fn debug_upgrade(
    keys: Res<ButtonInput<KeyCode>>,
    rover: Query<&Transform, With<crate::rover::Rover>>,
    mut picked: MessageWriter<UpgradePicked>,
) {
    if keys.just_pressed(KeyCode::KeyU) {
        if let Some(rover) = rover.iter().next() {
            picked.write(UpgradePicked {
                position: rover.translation,
            });
        }
    }
}

/// Уровни оружия по видам в порядке [`CurrentWeapon::ALL`], от 1 до [`MAX_LEVEL`].
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct WeaponLevels([u8; CurrentWeapon::ALL.len()]);

impl Default for WeaponLevels {
    fn default() -> Self {
        Self([1; CurrentWeapon::ALL.len()])
    }
}

fn slot(weapon: CurrentWeapon) -> usize {
    CurrentWeapon::ALL.iter().position(|w| *w == weapon).unwrap_or(0)
}

impl WeaponLevels {
    /// Уровень вида.
    pub fn level(&self, weapon: CurrentWeapon) -> u8 {
        self.0[slot(weapon)]
    }

    /// Поднимает уровень вида на один, не выше [`MAX_LEVEL`]. Возвращает новый уровень.
    fn raise(&mut self, weapon: CurrentWeapon) -> u8 {
        let level = &mut self.0[slot(weapon)];
        *level = (*level + 1).min(MAX_LEVEL);
        *level
    }
}

/// Луноход подобрал золотой ящик.
#[derive(Message, Debug, Clone, Copy)]
pub struct UpgradePicked {
    pub position: Vec3,
}

/// Чем кончился подбор золотого ящика.
#[derive(Message, Debug, Clone, Copy, PartialEq)]
pub enum WeaponUpgraded {
    /// Вид поднялся до уровня `level`.
    Level { weapon: CurrentWeapon, level: u8, position: Vec3 },
    /// Всё имеющееся оружие на максимуме: боезапас пополнен до предела.
    Refill { position: Vec3 },
}

/// Что улучшить золотым ящиком.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upgrade {
    Level(CurrentWeapon),
    Refill,
}

/// Выбранное оружие, если оно не на максимуме. Иначе — имеющееся оружие самого низкого уровня,
/// при равенстве первое по порядку клавиш. Если всё имеющееся на максимуме — пополнение.
pub fn pick_upgrade(levels: &WeaponLevels, inventory: &Inventory, current: CurrentWeapon) -> Upgrade {
    if inventory.owns(current) && levels.level(current) < MAX_LEVEL {
        return Upgrade::Level(current);
    }
    CurrentWeapon::ALL
        .into_iter()
        .filter(|w| inventory.owns(*w) && levels.level(*w) < MAX_LEVEL)
        .min_by_key(|w| levels.level(*w))
        .map_or(Upgrade::Refill, Upgrade::Level)
}

/// Применяет улучшение: поднимает уровень или пополняет боезапас всего имеющегося оружия.
pub fn apply_upgrade(levels: &mut WeaponLevels, inventory: &mut Inventory, upgrade: Upgrade) -> Option<u8> {
    match upgrade {
        Upgrade::Level(weapon) => Some(levels.raise(weapon)),
        Upgrade::Refill => {
            for weapon in CurrentWeapon::ALL {
                if inventory.owns(weapon) {
                    inventory.add(weapon, spec(weapon).limit);
                }
            }
            None
        }
    }
}

fn reset_levels(mut rebuilt: MessageReader<WorldRebuilt>, mut levels: ResMut<WeaponLevels>) {
    if rebuilt.read().last().is_some() {
        *levels = WeaponLevels::default();
    }
}

fn apply_picked(
    mut picked: MessageReader<UpgradePicked>,
    mut levels: ResMut<WeaponLevels>,
    mut inventory: ResMut<Inventory>,
    current: Res<CurrentWeapon>,
    mut upgraded: MessageWriter<WeaponUpgraded>,
) {
    for p in picked.read() {
        let upgrade = pick_upgrade(&levels, &inventory, *current);
        let result = match (upgrade, apply_upgrade(&mut levels, &mut inventory, upgrade)) {
            (Upgrade::Level(weapon), Some(level)) => WeaponUpgraded::Level {
                weapon,
                level,
                position: p.position,
            },
            _ => WeaponUpgraded::Refill { position: p.position },
        };
        info!("Золотой ящик: {result:?}");
        upgraded.write(result);
    }
}

/// Надпись улучшения: название нового уровня.
pub fn title(weapon: CurrentWeapon, level: u8) -> &'static str {
    use CurrentWeapon::*;
    match (weapon, level) {
        (MachineGun, 2) => "ДВОЙНОЙ ПУЛЕМЁТ!",
        (MachineGun, _) => "ТЯЖЁЛЫЕ ПУЛИ!",
        (Mortar, 2) => "ТЯЖЁЛЫЙ СНАРЯД!",
        (Mortar, _) => "КАССЕТНАЯ МОРТИРА!",
        (Rockets, 2) => "СПАРЕННЫЕ РАКЕТЫ!",
        (Rockets, _) => "ТЕРМОБАРИЧЕСКИЕ РАКЕТЫ!",
        (HomingRockets, 2) => "БОЛЬШОЙ ЗАЛП!",
        (HomingRockets, _) => "ТЯЖЁЛЫЕ БОЕГОЛОВКИ!",
        (Laser, 2) => "ДАЛЬНИЙ ЛУЧ!",
        (Laser, _) => "ВЗРЫВНОЙ ЛУЧ!",
        (Airstrike, 2) => "КОВРОВАЯ БОМБЁЖКА!",
        (Airstrike, _) => "ДВА САМОЛЁТА!",
    }
}

/// Пулемёт уровня.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MachineGunLevel {
    /// Стволов на башне: выстрелы чередуют стволы.
    pub barrels: u8,
    /// Выстрелов в секунду из всех стволов.
    pub rate: f32,
    pub damage: f32,
    /// Тяжёлые пули выбивают щербины в рельефе.
    pub heavy: bool,
}

pub fn machine_gun(level: u8) -> MachineGunLevel {
    match level {
        1 => MachineGunLevel { barrels: 1, rate: MG_RATE, damage: MG_DAMAGE, heavy: false },
        2 => MachineGunLevel { barrels: 2, rate: MG_RATE * 2.0, damage: MG_DAMAGE, heavy: false },
        _ => MachineGunLevel { barrels: 2, rate: MG_RATE * 2.0, damage: MG_DAMAGE * 1.5, heavy: true },
    }
}

/// Мортира уровня.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MortarLevel {
    pub radius: f32,
    pub power: f32,
    /// Суббоеприпасов кассеты вокруг основного взрыва.
    pub cluster: u8,
}

/// Суббоеприпас кассеты: радиус, мощность и разброс от точки взрыва, м.
pub const CLUSTER_RADIUS: f32 = 4.0;
pub const CLUSTER_POWER: f32 = 0.7;
pub const CLUSTER_SPREAD: (f32, f32) = (3.0, 6.0);

pub fn mortar(level: u8) -> MortarLevel {
    match level {
        1 => MortarLevel { radius: crate::mortar::EXPLOSION_RADIUS, power: crate::mortar::EXPLOSION_POWER, cluster: 0 },
        2 => MortarLevel { radius: 8.5, power: 1.8, cluster: 0 },
        _ => MortarLevel { radius: 8.5, power: 1.8, cluster: 4 },
    }
}

/// Обычные ракеты уровня.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RocketsLevel {
    /// Ракет за выстрел, рядом друг с другом.
    pub per_shot: u8,
    pub warhead: Warhead,
}

/// Огонь термобарической ракеты: сколько горит, урон в секунду и доля радиуса взрыва.
pub const FIRE_SECONDS: f32 = 3.0;
pub const FIRE_DAMAGE: f32 = 15.0;
pub const FIRE_REACH: f32 = 0.6;

pub fn rockets(level: u8) -> RocketsLevel {
    match level {
        1 => RocketsLevel { per_shot: 1, warhead: Warhead::ROCKET },
        2 => RocketsLevel { per_shot: 2, warhead: Warhead::ROCKET },
        _ => RocketsLevel { per_shot: 1, warhead: Warhead::THERMOBARIC },
    }
}

/// Самонаводящиеся ракеты уровня.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HomingLevel {
    pub salvo: usize,
    pub warhead: Warhead,
}

/// Залп самонаводящихся ракет со второго уровня.
pub const BIG_SALVO: usize = 9;

pub fn homing(level: u8) -> HomingLevel {
    match level {
        1 => HomingLevel { salvo: SALVO, warhead: Warhead::HOMING },
        2 => HomingLevel { salvo: BIG_SALVO, warhead: Warhead::HOMING },
        _ => HomingLevel { salvo: BIG_SALVO, warhead: Warhead { burn: false, ..Warhead::THERMOBARIC } },
    }
}

/// Лазер уровня.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LaserLevel {
    /// Длина луча, м.
    pub length: f32,
    /// Половина ширины траншеи, м.
    pub trench_half_width: f32,
    /// Взрыв на конце луча: радиус и мощность.
    pub end_blast: Option<(f32, f32)>,
}

pub fn laser(level: u8) -> LaserLevel {
    match level {
        1 => LaserLevel { length: BEAM_LENGTH, trench_half_width: TRENCH_HALF_WIDTH, end_blast: None },
        2 => LaserLevel { length: 60.0, trench_half_width: 2.0, end_blast: None },
        _ => LaserLevel { length: 60.0, trench_half_width: 2.0, end_blast: Some((5.0, 1.0)) },
    }
}

/// Авиаудар уровня.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AirstrikeLevel {
    /// Бомб у каждого самолёта.
    pub bombs: usize,
    /// Самолётов: второй летит крест-накрест.
    pub planes: u8,
}

pub fn airstrike(level: u8) -> AirstrikeLevel {
    match level {
        1 => AirstrikeLevel { bombs: crate::airstrike::BOMBS, planes: 1 },
        2 => AirstrikeLevel { bombs: 10, planes: 1 },
        _ => AirstrikeLevel { bombs: 8, planes: 2 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(weapons: &[CurrentWeapon]) -> Inventory {
        let mut inventory = Inventory::default();
        for w in weapons {
            inventory.add(*w, 1);
        }
        inventory
    }

    fn levels(pairs: &[(CurrentWeapon, u8)]) -> WeaponLevels {
        let mut levels = WeaponLevels::default();
        for (w, l) in pairs {
            levels.0[slot(*w)] = *l;
        }
        levels
    }

    #[test]
    fn selected_weapon_is_upgraded() {
        let mut levels = WeaponLevels::default();
        let mut inventory = Inventory::default();
        let upgrade = pick_upgrade(&levels, &inventory, CurrentWeapon::MachineGun);
        assert_eq!(upgrade, Upgrade::Level(CurrentWeapon::MachineGun));
        assert_eq!(apply_upgrade(&mut levels, &mut inventory, upgrade), Some(2));
        assert_eq!(levels.level(CurrentWeapon::MachineGun), 2);
    }

    #[test]
    fn maxed_selection_upgrades_the_weakest_owned_weapon() {
        use CurrentWeapon::*;
        let inventory = owned(&[Rockets]);
        let levels = levels(&[(Mortar, 3), (Rockets, 2), (MachineGun, 1)]);
        assert_eq!(pick_upgrade(&levels, &inventory, Mortar), Upgrade::Level(MachineGun));
        // Не имеющееся оружие не улучшается, даже если его уровень ниже.
        let levels = self::levels(&[(Mortar, 3), (MachineGun, 3), (Rockets, 2)]);
        assert_eq!(pick_upgrade(&levels, &inventory, Mortar), Upgrade::Level(Rockets));
    }

    #[test]
    fn everything_maxed_refills_all_ammo() {
        use CurrentWeapon::*;
        let mut inventory = owned(&[Rockets]);
        let mut levels = levels(&[(MachineGun, 3), (Mortar, 3), (Rockets, 3)]);
        let upgrade = pick_upgrade(&levels, &inventory, Rockets);
        assert_eq!(upgrade, Upgrade::Refill);
        assert_eq!(apply_upgrade(&mut levels, &mut inventory, upgrade), None);
        assert_eq!(inventory.ammo(Rockets), Some(spec(Rockets).limit));
        assert_eq!(inventory.ammo(Mortar), Some(spec(Mortar).limit));
        assert_eq!(inventory.ammo(Laser), None);
    }

    #[test]
    fn new_run_starts_at_first_level() {
        let mut app = App::new();
        app.add_message::<WorldRebuilt>()
            .insert_resource(levels(&[(CurrentWeapon::Mortar, 3)]))
            .add_systems(Update, reset_levels);
        app.world_mut().write_message(WorldRebuilt { seed: 1 });
        app.update();
        assert_eq!(*app.world().resource::<WeaponLevels>(), WeaponLevels::default());
    }

    #[test]
    fn every_level_is_stronger_than_the_previous_one() {
        for l in 1..MAX_LEVEL {
            let (a, b) = (mortar(l), mortar(l + 1));
            assert!(b.radius * b.power >= a.radius * a.power && (b.radius > a.radius || b.cluster > a.cluster));
            let (a, b) = (rockets(l), rockets(l + 1));
            assert!(
                b.per_shot as f32 * b.warhead.radius * b.warhead.power
                    > a.per_shot as f32 * a.warhead.radius * a.warhead.power
                    || b.warhead.radius > a.warhead.radius
            );
            let (a, b) = (homing(l), homing(l + 1));
            assert!(b.salvo > a.salvo || b.warhead.radius > a.warhead.radius);
            let (a, b) = (machine_gun(l), machine_gun(l + 1));
            assert!(b.rate * b.damage > a.rate * a.damage);
        }
    }
}
