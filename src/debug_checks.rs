//! Сцены проверки без рук: забег идёт по сценарию, итог пишется в лог, по ходу — снимки.
//!
//! Запуск: `SPACE_GUARD_AUTOSTART=1 SPACE_GUARD_CHECK=<сцена> SPACE_GUARD_SHOT_DIR=/папка
//! CARGO_MANIFEST_DIR=$PWD ./target/release/space_guard --immortal`. Строки лога начинаются
//! с «Проверка», в конце игра закрывается сама.
//!
//! - `ruins` — луноход у посёлка: взрыв рядом с домом пугает жителей, потом посёлок сносится;
//!   луноход уезжает за 450 м и возвращается. В лог — жители по домам: живые, прячутся, в сцене.
//! - `trees` — взрыв у края качает дерево, взрыв рядом валит его, второй взрыв в кратере опускает
//!   пень и ствол; три взрыва у валуна — наклон к склону; надстреленный валун после отъезда
//!   и возвращения хранит здоровье.
//! - `alarm` — враги всех видов, один выезжает из дома; тревога на третьей ступени зовёт Службу,
//!   потом тревога сбрасывается и Служба уезжает. В лог — расстояния, линия прицела танков.
//! - `paths` — тропинки посёлков сверху: стартовый посёлок и два соседних, снимок каждого
//!   жители на тропинках.
//! - `lava_slope` — шахта прорыва на склоне горы (нужен `SPACE_GUARD_SEED=3`, та же расстановка, что
//!   в тесте `lava::tests::pit_on_slope`): шесть выстрелов мортиры пробивают кору, два выстрела
//!   в стенку ниже по склону опускают край ниже уровня лавы. Снимки сверху и сбоку после
//!   наполнения, в лог — замер `lava::Overhang`: сколько лавы за краем ямы и как высоко она там.

use bevy::prelude::*;
use bevy::render::view::window::screenshot::{Screenshot, save_to_disk};

use crate::alarm::Alarm;
use crate::buildings::{House, Ruin};
use crate::combat::{Damage, Health, Team};
use crate::enemy::{Enemy, EnemyKind, HeavyTank, Mortarman, SpawnEnemy, SpawnPlace};
use crate::explosion::Explosion;
use crate::frame::GameSet;
use crate::interceptor::Interceptor;
use crate::props::{Prop, PropKind, PropStatus, Props, Remains};
use crate::residents::Resident;
use crate::rover::Rover;
use crate::run::GameState;
use crate::service::ServiceVehicle;
use crate::settlements::Settlements;
use crate::terrain::Terrain;

pub struct DebugChecksPlugin;

impl Plugin for DebugChecksPlugin {
    fn build(&self, app: &mut App) {
        let Ok(scene) = std::env::var("SPACE_GUARD_CHECK") else {
            return;
        };
        let dir = std::env::var("SPACE_GUARD_SHOT_DIR").unwrap_or_else(|_| ".".into());
        app.insert_resource(Check {
            scene,
            dir,
            clock: 0.0,
            step: 0,
            anchor: Vec3::ZERO,
            away: Vec3::X,
            target: None,
            remains: None,
            shelter: None,
        })
        .add_systems(
            Update,
            (tick, check_ruins, check_trees, check_alarm, check_paths, check_lava_slope)
                .chain()
                .after(crate::camera::follow_focus)
                .in_set(GameSet::Presentation),
        );
    }
}

#[derive(Resource)]
struct Check {
    scene: String,
    dir: String,
    /// Секунды забега.
    clock: f32,
    /// Сколько шагов сценария пройдено.
    step: u32,
    /// Точка сценария: центр посёлка, дерево, валун.
    anchor: Vec3,
    /// Направление от точки к луноходу.
    away: Vec3,
    /// Адрес проверяемого объекта: участок и номер.
    target: Option<(IVec2, usize)>,
    /// Здоровье валуна перед отъездом.
    remains: Option<f32>,
    /// Дом, к которому бегут жители.
    shelter: Option<Entity>,
}

impl Check {
    /// Шаг `n` наступил: сценарий дошёл до него и время пришло.
    fn at(&mut self, n: u32, time: f32) -> bool {
        if self.step == n && self.clock >= time {
            self.step += 1;
            return true;
        }
        false
    }

    fn shot(&self, commands: &mut Commands, name: &str) {
        let path = format!("{}/{}_{}.png", self.dir, self.scene, name);
        info!("Проверка: снимок {path}");
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
    }
}

fn tick(time: Res<Time<Real>>, state: Res<State<GameState>>, mut check: ResMut<Check>) {
    if *state.get() == GameState::Playing {
        check.clock += time.delta_secs();
    }
}

/// Ставит луноход в точку на рельефе лицом к `look`.
fn place_rover(rover: &mut Transform, terrain: &Terrain, at: Vec3, look: Vec3) {
    rover.translation = Vec3::new(at.x, terrain.height_at(at.x, at.z) + 0.5, at.z);
    let d = (at - look).with_y(0.0).normalize_or(Vec3::Z);
    // Перёд лунохода — −Z: смотрит на `look`.
    rover.rotation = Quat::from_rotation_y(d.x.atan2(d.z));
}

fn blast(
    explosions: &mut MessageWriter<Explosion>,
    terrain: &Terrain,
    at: Vec3,
    radius: f32,
    power: f32,
) {
    explosions.write(Explosion {
        position: Vec3::new(at.x, terrain.height_at(at.x, at.z), at.z),
        radius,
        power,
        source_team: Team::Player,
    });
}

#[allow(clippy::too_many_arguments)]
fn check_ruins(
    mut commands: Commands,
    mut check: ResMut<Check>,
    terrain: Res<Terrain>,
    settlements: Res<Settlements>,
    mut rover: Query<&mut Transform, With<Rover>>,
    houses: Query<(Entity, &House, &Transform), Without<Rover>>,
    ruins: Query<(&Ruin, &Transform), Without<Rover>>,
    residents: Query<(&Resident, &Transform), Without<Rover>>,
    mut damage: MessageWriter<Damage>,
    mut explosions: MessageWriter<Explosion>,
    mut exit: MessageWriter<AppExit>,
    mut deaths: MessageReader<crate::residents::ResidentDied>,
    mut dead: Local<u32>,
    mut ids: Local<Vec<crate::settlements::HouseId>>,
    service: Query<&ServiceVehicle>,
) {
    if !check.scene.starts_with("ruins") {
        return;
    }
    *dead += deaths.read().count() as u32;
    let Ok(mut rover) = rover.single_mut() else {
        return;
    };
    const VILLAGE: f32 = 45.0;
    let report = |check: &Check, ids: &[crate::settlements::HouseId], label: &str| {
        let c = check.anchor;
        let near = |p: Vec3| (p - c).with_y(0.0).length() < VILLAGE + 15.0;
        let house_count = houses
            .iter()
            .filter(|(_, _, t)| near(t.translation))
            .count();
        let ruin_ids: Vec<_> = ruins
            .iter()
            .filter(|(_, t)| near(t.translation))
            .map(|(r, _)| r.0)
            .collect();
        let people: Vec<&Resident> = residents
            .iter()
            .filter(|(_, t)| near(t.translation))
            .map(|(r, _)| r)
            .collect();
        let panicking = people.iter().filter(|r| r.is_panicking()).count();
        let chatting = people.iter().filter(|r| r.is_chatting()).count();
        info!(
            "Проверка ruins [{label}]: домов {house_count}, руин {}, жителей в сцене {} (паника {panicking}, болтают {chatting})",
            ruin_ids.len(),
            people.len()
        );
        info!(
            "Проверка ruins [{label}]:   погибло жителей с начала {}, машин Службы {}",
            *dead,
            service.iter().count()
        );
        for &id in ids {
            let Some(state) = settlements.house(id) else {
                continue;
            };
            let present = people.iter().filter(|r| r.home() == id).count();
            let expected = state.residents.saturating_sub(state.hiding);
            info!(
                "Проверка ruins [{label}]:   дом {id:?} {:?}: живых {}, прячутся {}, в сцене {present}{}",
                state.status,
                state.residents,
                state.hiding,
                if present as u8 == expected {
                    ""
                } else {
                    "  <-- НЕ СХОДИТСЯ"
                }
            );
        }
    };

    if check.at(0, 2.0) {
        let me = rover.translation;
        // Самый большой посёлок: дом, у которого больше всего соседей.
        let neighbours = |p: Vec3| {
            houses
                .iter()
                .filter(|(_, _, t)| t.translation.distance(p) < VILLAGE)
                .count()
        };
        let Some((_, _, nearest)) = houses
            .iter()
            .max_by_key(|(_, _, t)| neighbours(t.translation))
        else {
            check.step = 0;
            return;
        };
        check.anchor = nearest.translation;
        check.away = (me - check.anchor).with_y(0.0).normalize_or(Vec3::X);
        let at = check.anchor + check.away * 35.0;
        place_rover(&mut rover, &terrain, at, check.anchor);
        info!("Проверка ruins: посёлок у {:.0?}", check.anchor);
    } else if check.at(1, 7.0) {
        let c = check.anchor;
        *ids = houses
            .iter()
            .filter(|(_, _, t)| (t.translation - c).with_y(0.0).length() < VILLAGE)
            .map(|(_, h, _)| h.0)
            .collect();
        report(&check, &ids, "до");
        if check.scene == "ruins_all" {
            // Вариант: все дома рушатся разом, жители снаружи не успевают спрятаться.
            for (e, _, t) in &houses {
                if (t.translation - c).with_y(0.0).length() < VILLAGE {
                    damage.write(Damage {
                        target: e,
                        amount: 1.0e4,
                        source_team: Team::Player,
                    });
                }
            }
            check.step = 5;
            return;
        }
        // Хлопок в стороне: жители бегут прятаться в ближайший дом.
        blast(
            &mut explosions,
            &terrain,
            check.anchor + check.away.cross(Vec3::Y) * 16.0,
            3.0,
            0.4,
        );
    } else if check.at(2, 7.15) {
        // Дом, к которому бегут жители (ближайший к ним), рушится, пока они не добежали.
        let c = check.anchor;
        let outside: Vec<Vec3> = residents
            .iter()
            .filter(|(_, t)| (t.translation - c).with_y(0.0).length() < VILLAGE + 15.0)
            .map(|(_, t)| t.translation)
            .collect();
        let mean = outside.iter().copied().sum::<Vec3>() / outside.len().max(1) as f32;
        check.shelter = houses
            .iter()
            .filter(|(_, _, t)| (t.translation - c).with_y(0.0).length() < VILLAGE)
            .min_by(|a, b| {
                a.2.translation
                    .distance_squared(mean)
                    .total_cmp(&b.2.translation.distance_squared(mean))
            })
            .map(|(e, _, _)| e);
        if let Some(shelter) = check.shelter {
            damage.write(Damage {
                target: shelter,
                amount: 1.0e4,
                source_team: Team::Player,
            });
        }
        report(&check, &ids, "испуг, укрытие рушится");
    } else if check.at(3, 9.0) {
        report(&check, &ids, "укрытие снесено +2 с");
    } else if check.at(4, 16.0) {
        report(&check, &ids, "укрытие снесено +9 с");
        let c = check.anchor;
        for (e, _, t) in &houses {
            if (t.translation - c).with_y(0.0).length() < VILLAGE {
                damage.write(Damage {
                    target: e,
                    amount: 1.0e4,
                    source_team: Team::Player,
                });
            }
        }
    } else if check.at(5, 26.0) {
        report(&check, &ids, "весь посёлок снесён +10 с");
        check.shot(&mut commands, "1_after");
        // Кратер под руинами: руины должны опуститься на новую поверхность.
        if let Some((_, t)) = ruins.iter().next() {
            let p = t.translation;
            info!(
                "Проверка ruins [руины до взрыва]: y {:.2}, рельеф {:.2}",
                p.y,
                terrain.height_at(p.x, p.z)
            );
            blast(
                &mut explosions,
                &terrain,
                p,
                crate::mortar::EXPLOSION_RADIUS,
                crate::mortar::EXPLOSION_POWER,
            );
        }
    } else if check.at(6, 27.0) {
        for (_, t) in ruins.iter().take(1) {
            let p = t.translation;
            info!(
                "Проверка ruins [руины после взрыва]: y {:.2}, рельеф {:.2}",
                p.y,
                terrain.height_at(p.x, p.z)
            );
        }
        report(&check, &ids, "перед отъездом");
        let at = check.anchor + check.away * 450.0;
        place_rover(&mut rover, &terrain, at, check.anchor);
    } else if check.at(7, 35.0) {
        report(&check, &ids, "луноход за 450 м");
    } else if check.at(8, 36.0) {
        let at = check.anchor + check.away * 35.0;
        place_rover(&mut rover, &terrain, at, check.anchor);
    } else if check.at(9, 46.0) {
        report(&check, &ids, "вернулся");
        check.shot(&mut commands, "2_back");
    } else if check.at(10, 48.0) {
        exit.write(AppExit::Success);
    }
}

/// Наклон от вертикали, градусы.
fn tilt(rotation: Quat) -> f32 {
    (rotation * Vec3::Y).angle_between(Vec3::Y).to_degrees()
}

#[allow(clippy::too_many_arguments)]
fn check_trees(
    mut commands: Commands,
    mut check: ResMut<Check>,
    terrain: Res<Terrain>,
    mut props: ResMut<Props>,
    mut rover: Query<&mut Transform, With<Rover>>,
    objects: Query<(Entity, &Prop, &Transform, Option<&Health>), Without<Rover>>,
    remains: Query<(&Remains, &Transform), Without<Rover>>,
    mut damage: MessageWriter<Damage>,
    mut explosions: MessageWriter<Explosion>,
    mut exit: MessageWriter<AppExit>,
) {
    if check.scene != "trees" {
        return;
    }
    let Ok(mut rover) = rover.single_mut() else {
        return;
    };
    let r = crate::mortar::EXPLOSION_RADIUS;
    let power = crate::mortar::EXPLOSION_POWER;
    let find = |check: &Check| {
        let (chunk, index) = check.target?;
        objects
            .iter()
            .find(|(_, p, _, _)| p.chunk == chunk && p.index == index)
    };
    let remains_report = |check: &Check, label: &str| {
        let Some((chunk, index)) = check.target else {
            return;
        };
        for (rem, t) in &remains {
            if rem.chunk == chunk && rem.index == index {
                let ground = terrain.height_at(t.translation.x, t.translation.z);
                info!(
                    "Проверка trees [{label}]: {} y {:.2}, рельеф {:.2}, наклон {:.0}°",
                    if rem.stump { "пень" } else { "ствол" },
                    t.translation.y,
                    ground,
                    tilt(t.rotation)
                );
            }
        }
    };

    if check.at(0, 2.0) {
        let me = rover.translation;
        let tree = objects
            .iter()
            .filter(|(_, p, t, h)| {
                p.kind == PropKind::Tree && h.is_some() && t.translation.distance(me) > 12.0
            })
            .min_by(|a, b| {
                a.2.translation
                    .distance_squared(me)
                    .total_cmp(&b.2.translation.distance_squared(me))
            });
        let Some((_, prop, t, _)) = tree else {
            check.step = 0;
            return;
        };
        check.target = Some((prop.chunk, prop.index));
        check.anchor = t.translation;
        check.away = (me - check.anchor).with_y(0.0).normalize_or(Vec3::X);
        place_rover(
            &mut rover,
            &terrain,
            check.anchor + check.away * 22.0,
            check.anchor,
        );
        info!(
            "Проверка trees: дерево {:?} у {:.0?}",
            check.target, check.anchor
        );
    } else if check.at(1, 4.0) {
        // Край: дальше радиуса, ближе предела качания.
        let side = check.away.cross(Vec3::Y);
        blast(
            &mut explosions,
            &terrain,
            check.anchor + side * r * 1.35,
            r,
            power,
        );
    } else if (2..=5).contains(&check.step) {
        let times = [4.25, 4.6, 5.2, 6.5];
        let i = (check.step - 2) as usize;
        let step = check.step;
        if check.at(step, times[i]) {
            if let Some((_, _, t, h)) = find(&check) {
                info!(
                    "Проверка trees [качание +{:.2} с]: наклон {:.1}°, здоровье {:?}",
                    times[i] - 4.0,
                    tilt(t.rotation),
                    h.map(|h| h.current)
                );
            }
        }
    } else if check.at(6, 8.0) {
        let side = check.away.cross(Vec3::Y);
        blast(
            &mut explosions,
            &terrain,
            check.anchor + side * 2.0,
            r,
            power,
        );
    } else if check.at(7, 12.0) {
        if let Some((chunk, index)) = check.target {
            let status = props.get_mut(chunk, index).map(|s| s.status);
            info!(
                "Проверка trees [свален]: состояние {}",
                match status {
                    Some(PropStatus::Fallen(_)) => "Fallen",
                    Some(PropStatus::Intact) => "Intact",
                    Some(PropStatus::Destroyed) => "Destroyed",
                    None => "нет",
                }
            );
        }
        remains_report(&check, "свален");
        check.shot(&mut commands, "1_fallen");
    } else if check.at(8, 13.0) {
        let side = check.away.cross(Vec3::Y);
        blast(
            &mut explosions,
            &terrain,
            check.anchor + side * 2.0,
            r,
            power,
        );
    } else if check.at(9, 16.0) {
        remains_report(&check, "второй взрыв");
        check.shot(&mut commands, "2_second");
    } else if check.at(10, 17.0) {
        let me = rover.translation;
        let boulder = objects
            .iter()
            .filter(|(_, p, _, h)| p.kind == PropKind::Boulder && h.is_some())
            .min_by(|a, b| {
                a.2.translation
                    .distance_squared(me)
                    .total_cmp(&b.2.translation.distance_squared(me))
            });
        let Some((_, prop, t, h)) = boulder else {
            info!("Проверка trees: валуна рядом нет");
            check.step = 99;
            return;
        };
        check.target = Some((prop.chunk, prop.index));
        check.anchor = t.translation;
        check.away = (me - check.anchor).with_y(0.0).normalize_or(Vec3::X);
        place_rover(
            &mut rover,
            &terrain,
            check.anchor + check.away * 22.0,
            check.anchor,
        );
        info!(
            "Проверка trees: валун {:?} у {:.0?}, здоровье {:?}, наклон {:.0}°, низ над рельефом {:.2}",
            check.target,
            check.anchor,
            h.map(|h| h.current),
            tilt(t.rotation),
            t.translation.y - terrain.height_at(t.translation.x, t.translation.z)
        );
    } else if (11..=13).contains(&check.step) {
        let times = [19.0, 21.0, 23.0];
        let i = (check.step - 11) as usize;
        let step = check.step;
        if check.at(step, times[i]) {
            let angle = i as f32 * 2.1;
            let dir = Quat::from_rotation_y(angle) * check.away.cross(Vec3::Y);
            blast(
                &mut explosions,
                &terrain,
                check.anchor + dir * (r * 0.6),
                r,
                power,
            );
        }
    } else if check.at(14, 25.5) {
        match find(&check) {
            Some((_, _, t, h)) => {
                let p = t.translation;
                let normal = terrain.normal_at(p.x, p.z);
                let up = t.rotation * Vec3::Y;
                info!(
                    "Проверка trees [валун после трёх взрывов]: от вертикали {:.0}°, от нормали склона {:.0}°, склон {:.0}°, здоровье {:?}, низ над рельефом {:.2}",
                    tilt(t.rotation),
                    up.angle_between(normal).to_degrees(),
                    normal.angle_between(Vec3::Y).to_degrees(),
                    h.map(|h| h.current),
                    p.y - terrain.height_at(p.x, p.z)
                );
            }
            None => info!("Проверка trees [валун после трёх взрывов]: валуна нет (разрушен)"),
        }
        check.shot(&mut commands, "3_boulder");
    } else if check.at(15, 26.0) {
        if let Some((e, _, _, Some(h))) = find(&check) {
            let amount = h.current * 0.5;
            damage.write(Damage {
                target: e,
                amount,
                source_team: Team::Player,
            });
        }
    } else if check.at(16, 27.0) {
        let health = find(&check).and_then(|(_, _, _, h)| h.map(|h| h.current));
        check.remains = health;
        info!("Проверка trees [валун надстрелен]: здоровье {health:?}");
        let at = check.anchor + check.away * 450.0;
        place_rover(&mut rover, &terrain, at, check.anchor);
    } else if check.at(17, 35.0) {
        info!(
            "Проверка trees [луноход за 450 м]: валун в сцене {}",
            find(&check).is_some()
        );
        let at = check.anchor + check.away * 22.0;
        place_rover(&mut rover, &terrain, at, check.anchor);
    } else if check.at(18, 43.0) {
        let health = find(&check).and_then(|(_, _, _, h)| h.map(|h| h.current));
        info!(
            "Проверка trees [вернулся]: здоровье валуна {health:?}, было {:?}{}",
            check.remains,
            if health == check.remains {
                ""
            } else {
                "  <-- НЕ СХОДИТСЯ"
            }
        );
    } else if check.at(19, 44.0) || check.step == 99 {
        exit.write(AppExit::Success);
    }
}

#[allow(clippy::too_many_arguments)]
fn check_alarm(
    mut commands: Commands,
    mut check: ResMut<Check>,
    mut alarm: ResMut<Alarm>,
    settlements: Res<Settlements>,
    rover: Query<&Transform, With<Rover>>,
    houses: Query<(Entity, &House, &Transform), Without<Rover>>,
    enemies: Query<
        (
            Entity,
            &Transform,
            Has<Mortarman>,
            Option<&HeavyTank>,
            Has<Interceptor>,
        ),
        (With<Enemy>, Without<Rover>),
    >,
    service: Query<(Entity, &ServiceVehicle, &Transform), Without<Rover>>,
    mut spawn: MessageWriter<SpawnEnemy>,
    mut exit: MessageWriter<AppExit>,
    mut next_log: Local<f32>,
    mut door: Local<Option<(Entity, crate::settlements::HouseId)>>,
    mut last: Local<std::collections::HashMap<Entity, (Vec3, u32)>>,
) {
    if check.scene != "alarm" {
        return;
    }
    let Ok(me) = rover.single().map(|t| t.translation) else {
        return;
    };
    if check.at(0, 2.0) {
        let nearest = houses.iter().min_by(|a, b| {
            a.2.translation
                .distance_squared(me)
                .total_cmp(&b.2.translation.distance_squared(me))
        });
        if let Some((e, h, t)) = nearest {
            info!(
                "Проверка alarm: враг выезжает из дома {:?} в {:.0} м",
                h.0,
                t.translation.distance(me)
            );
            *door = Some((e, h.0));
            spawn.write(SpawnEnemy {
                place: SpawnPlace::House(e),
                kind: EnemyKind::RamShooter,
            });
        }
        for (i, kind) in [
            EnemyKind::Mortar,
            EnemyKind::HeavyTank,
            EnemyKind::LaserTank,
            EnemyKind::Interceptor,
        ]
        .into_iter()
        .enumerate()
        {
            spawn.write(SpawnEnemy {
                place: SpawnPlace::Edge {
                    angle: i as f32 * 1.6,
                },
                kind,
            });
        }
    } else if check.at(1, 5.0) {
        if let Some((e, id)) = *door {
            let state = settlements.house(id);
            info!(
                "Проверка alarm: дом после выезда — сущность {}, состояние {:?}, выбитая дверь {:?}",
                if houses.get(e).is_ok() {
                    "есть"
                } else {
                    "нет"
                },
                state.map(|s| s.status),
                state.and_then(|s| s.broken_door)
            );
        }
        alarm.add(30.0);
        info!(
            "Проверка alarm: тревога поднята до ступени {}",
            alarm.level()
        );
    } else if check.at(2, 40.0) {
        *alarm = Alarm::default();
        info!("Проверка alarm: тревога сброшена");
    }
    if check.step == 2 && check.clock >= 20.0 && check.remains.is_none() {
        check.remains = Some(0.0);
        check.shot(&mut commands, "1_ring");
    }
    if check.step == 3 && check.clock >= 58.0 {
        exit.write(AppExit::Success);
    }

    if check.step == 0 || check.clock < *next_log {
        return;
    }
    *next_log = check.clock + 2.0;
    let mut line = format!(
        "Проверка alarm [{:>4.0} с, ступень {}]:",
        check.clock,
        alarm.level()
    );
    // Сколько замеров подряд машина не сдвинулась больше чем на метр.
    let mut still = |e: Entity, p: Vec3| {
        let entry = last.entry(e).or_insert((p, 0));
        entry.1 = if entry.0.distance(p) < 1.0 {
            entry.1 + 1
        } else {
            0
        };
        entry.0 = p;
        entry.1
    };
    for (e, t, mortar, tank, interceptor) in &enemies {
        let d = t.translation.distance(me);
        let stuck = still(e, t.translation);
        let in_house = houses.iter().any(|(_, h, _)| {
            settlements
                .house(h.0)
                .is_some_and(|s| s.footprint().distance(t.translation.xz()) < 0.5)
        });
        let name = if mortar {
            "мортира"
        } else if let Some(tank) = tank {
            if tank.arm == crate::enemy::TankArm::Laser {
                "лазер-танк"
            } else {
                "ракет-танк"
            }
        } else if interceptor {
            "перехватчик"
        } else {
            "таранщик"
        };
        let aim = tank.is_some_and(|t| t.aiming());
        line += &format!(
            " {name} {d:.0} м{}{}{};",
            if aim { " ЦЕЛИТСЯ" } else { "" },
            if in_house { " В ДОМЕ" } else { "" },
            if stuck >= 3 && d > 25.0 {
                format!(" СТОИТ {}с @{:.0?}", stuck * 2, t.translation.xz())
            } else {
                String::new()
            }
        );
    }
    let ring: Vec<String> = service
        .iter()
        .map(|(e, v, t)| {
            let stuck = still(e, t.translation);
            format!(
                "{:.0}{}{}",
                t.translation.distance(me),
                if v.is_leaving() { "↗" } else { "" },
                if stuck >= 3 && v.is_leaving() {
                    format!(" СТОИТ {}с @{:.0?}", stuck * 2, t.translation.xz())
                } else {
                    String::new()
                }
            )
        })
        .collect();
    line += &format!(" | Служба {}: [{}]", ring.len(), ring.join(" "));
    info!("{line}");
}

/// Тропинки сверху: луноход у посёлка, камера над его серединой, снимок; потом следующий посёлок.
fn check_paths(
    mut commands: Commands,
    mut check: ResMut<Check>,
    terrain: Res<Terrain>,
    mut settlements: ResMut<Settlements>,
    mut rover: Query<&mut Transform, With<Rover>>,
    mut camera: Query<&mut Transform, (With<crate::camera::GameCamera>, Without<Rover>)>,
    mut villages: Local<Vec<Vec2>>,
    mut exit: MessageWriter<AppExit>,
) {
    if check.scene != "paths" {
        return;
    }
    let Ok(mut rover) = rover.single_mut() else {
        return;
    };
    if villages.is_empty() && check.clock > 1.0 {
        // Посёлки ближе всего к старту: стартовый и ещё два.
        let mut found: Vec<Vec2> = Vec::new();
        for r in 1..=3 {
            for z in -r..=r {
                for x in -r..=r {
                    let coord = IVec2::new(x, z);
                    let houses = &settlements.region(coord, &terrain).houses;
                    if houses.len() >= 3 {
                        let c = houses.iter().map(|h| h.position).sum::<Vec2>() / houses.len() as f32;
                        if !found.iter().any(|f| f.distance(c) < 1.0) {
                            found.push(c);
                        }
                    }
                }
            }
        }
        found.sort_by(|a, b| a.length().total_cmp(&b.length()));
        found.truncate(3);
        info!("Проверка paths: посёлков {}", found.len());
        *villages = found;
    }
    if villages.is_empty() {
        return;
    }
    // Каждый посёлок: 4 с на подгрузку и снимок сверху, через 10 с — ближе, жители гуляют.
    const SPAN: f32 = 15.0;
    let index = (check.step / 3) as usize;
    if index >= villages.len() {
        if check.clock > 1.0 + SPAN * villages.len() as f32 {
            exit.write(AppExit::Success);
        }
        return;
    }
    let c = villages[index];
    let begin = 1.0 + SPAN * index as f32;
    let step = index as u32 * 3;
    if check.at(step, begin) {
        let at = Vec3::new(c.x + 30.0, 0.0, c.y + 30.0);
        place_rover(&mut rover, &terrain, at, Vec3::new(c.x, 0.0, c.y));
    }
    let close = check.step > step + 1;
    if let Ok(mut cam) = camera.single_mut() {
        let h = terrain.height_at(c.x, c.y);
        let (up, back) = if close { (34.0, 10.0) } else { (68.0, 14.0) };
        *cam = Transform::from_xyz(c.x, h + up, c.y + back).looking_at(Vec3::new(c.x, h, c.y), Vec3::NEG_Z);
    }
    if check.at(step + 1, begin + 4.0) {
        check.shot(&mut commands, &format!("village_{index}"));
    }
    if check.at(step + 2, begin + 14.0) {
        check.shot(&mut commands, &format!("village_{index}_close"));
    }
}

/// Место ямы сцены `lava_slope` при сиде 3: склон горы (уклон около 0,73).
const LAVA_PIT: Vec2 = Vec2::new(20.0, -110.0);

/// Сцена `lava_slope`: лава из прорубленной ямы на склоне.
#[allow(clippy::too_many_arguments)]
fn check_lava_slope(
    mut commands: Commands,
    mut check: ResMut<Check>,
    terrain: Res<Terrain>,
    pools: Res<crate::lava::LavaPools>,
    mut explosions: MessageWriter<Explosion>,
    mut rover: Query<&mut Transform, With<Rover>>,
    mut camera: Query<&mut Transform, (With<crate::camera::GameCamera>, Without<Rover>)>,
    trees: Query<(&Prop, &GlobalTransform, &Health)>,
    mut exit: MessageWriter<AppExit>,
) {
    if check.scene != "lava_slope" {
        return;
    }
    let Ok(mut rover) = rover.single_mut() else {
        return;
    };
    let p = LAVA_PIT;
    let h = |dx: f32, dz: f32| terrain.original_height((p.x + dx) as i32, (p.y + dz) as i32);
    let down = -Vec2::new(h(5.0, 0.0) - h(-5.0, 0.0), h(0.0, 5.0) - h(0.0, -5.0)).normalize_or(Vec2::X);
    let pit = Vec3::new(p.x, 0.0, p.y);
    let down3 = Vec3::new(down.x, 0.0, down.y);
    let mortar = |explosions: &mut MessageWriter<Explosion>, at: Vec3| {
        blast(explosions, &terrain, at, crate::mortar::EXPLOSION_RADIUS, crate::mortar::EXPLOSION_POWER);
    };
    if check.at(0, 1.0) {
        if terrain.seed() != 3 {
            warn!("Проверка lava_slope: сид {}, а расстановка подобрана под сид 3", terrain.seed());
        }
        info!("Проверка lava_slope: яма {p}, вниз по склону {down}");
        // Луноход выше по склону, лицом к яме: лава туда не дойдёт.
        place_rover(&mut rover, &terrain, pit - down3 * 25.0, pit);
    }
    // Шесть выстрелов мортиры в яму раз в секунду: на шестом кора пробита.
    for i in 0..6 {
        if check.at(1 + i, 4.0 + i as f32) {
            mortar(&mut explosions, pit);
        }
    }
    // Два выстрела в стенку ниже по склону.
    for i in 0..2 {
        if check.at(7 + i, 11.0 + i as f32) {
            mortar(&mut explosions, pit + down3 * 7.0);
        }
    }
    // Камера: сверху над серединой склона, на втором снимке — сбоку, поперёк склона.
    let side = check.step == 10 || check.step == 12 || check.step == 14;
    if let Ok(mut cam) = camera.single_mut() {
        let mid = pit + down3 * 18.0;
        let look = Vec3::new(mid.x, terrain.height_at(mid.x, mid.z), mid.z);
        *cam = if side {
            let across = Vec3::new(-down.y, 0.0, down.x);
            Transform::from_translation(look + across * 60.0 + Vec3::Y * 25.0).looking_at(look, Vec3::Y)
        } else {
            Transform::from_translation(look + Vec3::new(0.0, 80.0, 14.0)).looking_at(look, Vec3::NEG_Z)
        };
    }
    // Целые деревья в горячей лаве и у её края (2 м): они должны гореть.
    let standing = || {
        let alive = || trees.iter().filter(|(p, _, h)| p.kind == PropKind::Tree && !h.is_dead());
        let inside = alive().filter(|(_, t, _)| pools.lava_at(t.translation().x, t.translation().z).is_some()).count();
        let edge = alive().filter(|(_, t, _)| pools.near(t.translation().xz(), 2.0)).count();
        (inside, edge)
    };
    // Наполнение — 6 с после прорыва.
    if check.at(9, 20.0) {
        info!("Проверка lava_slope: целых деревьев в лаве {}, у края {}", standing().0, standing().1);
        for (i, o) in pools.overhangs(&terrain).iter().enumerate() {
            info!(
                "Проверка lava_slope: озеро {i}: ячеек {}, за краем ямы {} (толще 1 м: {}), толщина за краем до {:.1} м, \
                 выше исходного склона на {:.1} м, дальняя ячейка {:.0} м, на нетронутой земле {} ячеек толщиной до {:.1} м",
                o.cells, o.outside, o.thick, o.max_thickness, o.above_surface, o.reach, o.intact, o.intact_thickness
            );
        }
        info!("Проверка lava_slope: (поверхность у прорыва, цель, горячих ячеек, сетка) {:?}", pools.summary());
        check.shot(&mut commands, "top");
    }
    if check.at(10, 21.0) {
        check.shot(&mut commands, "side");
    }
    // Через минуту поток остановился и застыл.
    if check.at(11, 75.0) {
        info!("Проверка lava_slope через минуту: целых деревьев в лаве {}, у края {}", standing().0, standing().1);
        for (i, o) in pools.overhangs(&terrain).iter().enumerate() {
            info!(
                "Проверка lava_slope через минуту: озеро {i}: за краем ямы {}, дальняя ячейка {:.0} м, на нетронутой земле {} ячеек толщиной до {:.1} м",
                o.outside, o.reach, o.intact, o.intact_thickness
            );
        }
        check.shot(&mut commands, "late_top");
    }
    if check.at(12, 76.0) {
        check.shot(&mut commands, "late_side");
    }
    // Через две минуты источник исчерпан, поток стоит и застыл коркой.
    if check.at(13, 120.0) {
        let crust = pools.crust_cells();
        info!("Проверка lava_slope через две минуты: горячих ячеек {}, застывших {crust}", pools.cells().count());
        check.shot(&mut commands, "cooled_top");
    }
    if check.at(14, 121.0) {
        check.shot(&mut commands, "cooled_side");
    }
    if check.at(15, 123.0) {
        exit.write(AppExit::Success);
    }
}
