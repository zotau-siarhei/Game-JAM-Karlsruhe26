//! Провал в кратер: дома и крупные объекты оседают на новую поверхность и кренятся по ней,
//! дом, просевший слишком глубоко или накренившийся слишком сильно, рушится. Руины, пни
//! и упавшие стволы опускаются на новую поверхность.
//!
//! Рельеф меняют модули `craters`, `laser`, `rails`: каждый после деформации сам шлёт
//! [`TerrainChanged`], на него отвечают дома, объекты, пути и лава.

use bevy::prelude::*;

use crate::buildings::{House, Ruin};
use crate::combat::{Damage, Health, Team};
use crate::craters;
use crate::frame::{GameSet, RunMessages};
use crate::obstacles::Obstacle;
use crate::props::{Prop, PropKind, PropStatus, Props, Remains};
use crate::settlements::{HouseStatus, Settlements};
use crate::terrain::Terrain;

/// Средняя просадка, с которой дом проваливается, м.
const SINK_DEPTH: f32 = 1.0;

/// Просадка, при которой дом рушится, м.
const COLLAPSE_DEPTH: f32 = 4.0;

/// Крен, при котором дом рушится, рад.
const COLLAPSE_TILT: f32 = 25.0 * std::f32::consts::PI / 180.0;

/// Просадка, с которой оседает крупный объект, м.
const PROP_SINK_DEPTH: f32 = 0.3;

/// Сколько длится оседание, с.
const SETTLE_SECONDS: f32 = 0.4;

/// Подключает провал в кратер.
pub struct SubsidencePlugin;

impl Plugin for SubsidencePlugin {
    fn build(&self, app: &mut App) {
        app.add_run_message::<TerrainChanged>().add_systems(
            Update,
            (
                // В одном кадре с кратером: дом проваливается и получает урон до применения урона.
                (sink_houses, sink_props, sink_remains, sink_ruins)
                    .chain()
                    .after(craters::dig_craters)
                    .in_set(GameSet::Damage),
                settle.in_set(GameSet::Simulation),
            ),
        );
    }
}

/// Рельеф изменился в круге: дома, объекты и обломки в нём должны осесть.
///
/// Пишет тот, кто изменил рельеф, сразу после изменения: читатель видит уже новый рельеф.
#[derive(Message, Debug, Clone, Copy)]
pub struct TerrainChanged {
    pub center: Vec3,
    pub radius: f32,
}

/// Дом рушится от провала: причина разрушения — `Collapse`.
#[derive(Component, Debug, Clone, Copy)]
pub struct Collapsing;

/// Плавное оседание к новому положению.
#[derive(Component)]
struct Settling {
    from: Transform,
    to: Transform,
    elapsed: f32,
}

/// Дома, задетые кратером, оседают по четырём углам или рушатся.
fn sink_houses(
    mut commands: Commands,
    mut changes: MessageReader<TerrainChanged>,
    terrain: Res<Terrain>,
    mut settlements: ResMut<Settlements>,
    houses: Query<(Entity, &House, &Transform, &Health)>,
    mut damage: MessageWriter<Damage>,
) {
    for change in changes.read() {
        let center = Vec2::new(change.center.x, change.center.z);
        for (entity, house, transform, health) in &houses {
            let Some(state) = settlements.house_mut(house.0) else {
                continue;
            };
            let footprint = state.footprint();
            if footprint.distance(center) > change.radius || health.is_dead() {
                continue;
            }
            let corners = footprint.corners().map(|c| Vec3::new(c.x, terrain.height_at(c.x, c.y), c.y));
            let ground = corners.iter().map(|c| c.y).sum::<f32>() / 4.0;
            // Плоскость через углы: нормаль по диагоналям.
            let normal = (corners[2] - corners[0]).cross(corners[3] - corners[1]).normalize_or(Vec3::Y);
            let normal = if normal.y < 0.0 { -normal } else { normal };
            let tilt = normal.angle_between(Vec3::Y);
            let rest_y = match state.status {
                HouseStatus::Sunk(t) => t.translation.y,
                _ => transform.translation.y,
            };
            let sink = rest_y - ground;
            if sink > COLLAPSE_DEPTH || tilt > COLLAPSE_TILT {
                commands.entity(entity).try_insert(Collapsing);
                damage.write(Damage {
                    target: entity,
                    amount: health.current + 1.0,
                    source_team: Team::Player,
                });
                continue;
            }
            if sink < SINK_DEPTH && !matches!(state.status, HouseStatus::Sunk(_)) {
                continue;
            }
            let to = Transform::from_xyz(transform.translation.x, ground, transform.translation.z)
                .with_rotation(Quat::from_rotation_arc(Vec3::Y, normal) * Quat::from_rotation_y(state.yaw));
            state.status = HouseStatus::Sunk(to);
            commands.entity(entity).try_insert(Settling {
                from: *transform,
                to,
                elapsed: 0.0,
            });
        }
    }
}

/// Поворот вокруг вертикали, из которого собран поворот объекта с креном.
fn yaw_of(rotation: Quat) -> f32 {
    let x = rotation * Vec3::X;
    (-x.z).atan2(x.x)
}

/// Попала ли точка плана в круг изменения.
fn touched(change: &TerrainChanged, p: Vec3) -> bool {
    Vec2::new(p.x - change.center.x, p.z - change.center.z).length() <= change.radius
}

/// Крупные стоящие объекты в кратере опускаются на новую поверхность и кренятся по склону.
fn sink_props(
    mut commands: Commands,
    mut changes: MessageReader<TerrainChanged>,
    terrain: Res<Terrain>,
    mut props: ResMut<Props>,
    mut objects: Query<(Entity, &Prop, &Transform, Option<&mut Obstacle>), Without<Remains>>,
) {
    for change in changes.read() {
        for (entity, prop, transform, obstacle) in &mut objects {
            if !prop.kind.is_large() {
                continue;
            }
            let p = transform.translation;
            if !touched(change, p) {
                continue;
            }
            let ground = terrain.height_at(p.x, p.z);
            if p.y - ground < PROP_SINK_DEPTH {
                continue;
            }
            let Some(state) = props.get_mut(prop.chunk, prop.index) else {
                continue;
            };
            if state.status != PropStatus::Intact {
                continue;
            }
            let up = terrain.normal_at(p.x, p.z);
            // Деревья кренятся по склону кратера, камни ложатся на него. Крен ставится заново
            // от поворота вокруг вертикали, а не добавляется к прошлому: не копится от взрыва к взрыву.
            let lean = if prop.kind == PropKind::Tree {
                Quat::from_rotation_arc(Vec3::Y, up.lerp(Vec3::Y, 0.4).normalize())
            } else {
                Quat::from_rotation_arc(Vec3::Y, up)
            };
            let to = Transform {
                translation: Vec3::new(p.x, ground, p.z),
                rotation: lean * Quat::from_rotation_y(yaw_of(state.transform.rotation)),
                scale: transform.scale,
            };
            state.transform = to;
            // Верх препятствия опускается вместе с объектом: пули над ним пролетают.
            if let Some(mut obstacle) = obstacle {
                obstacle.top -= p.y - ground;
            }
            commands.entity(entity).try_insert(Settling {
                from: *transform,
                to,
                elapsed: 0.0,
            });
        }
    }
}

/// Пни и упавшие стволы в кратере опускаются на новую поверхность, наклон не меняется.
fn sink_remains(
    mut commands: Commands,
    mut changes: MessageReader<TerrainChanged>,
    terrain: Res<Terrain>,
    mut props: ResMut<Props>,
    remains: Query<(Entity, &Remains, &Transform)>,
) {
    for change in changes.read() {
        for (entity, remains, transform) in &remains {
            if !touched(change, transform.translation) {
                continue;
            }
            let Some(state) = props.get_mut(remains.chunk, remains.index) else {
                continue;
            };
            // Опора: основание дерева у пня, точка падения у ствола.
            let base = if remains.stump {
                &mut state.transform
            } else if let PropStatus::Fallen(trunk) = &mut state.status {
                trunk
            } else {
                continue;
            };
            let drop = base.translation.y - terrain.height_at(base.translation.x, base.translation.z);
            if drop < PROP_SINK_DEPTH {
                continue;
            }
            base.translation.y -= drop;
            let mut to = *transform;
            to.translation.y -= drop;
            // Падающий ствол заканчивает поворот сам: оседание двигает только высоту.
            if !remains.stump {
                to.rotation = base.rotation;
            }
            commands.entity(entity).try_insert(Settling {
                from: *transform,
                to,
                elapsed: 0.0,
            });
        }
    }
}

/// Руины в кратере опускаются на новую поверхность. Положение руин не хранится: при загрузке
/// они ставятся на рельеф заново.
fn sink_ruins(
    mut commands: Commands,
    mut changes: MessageReader<TerrainChanged>,
    terrain: Res<Terrain>,
    ruins: Query<(Entity, &Transform), With<Ruin>>,
) {
    for change in changes.read() {
        for (entity, transform) in &ruins {
            let p = transform.translation;
            if !touched(change, p) {
                continue;
            }
            let ground = terrain.height_at(p.x, p.z);
            if p.y - ground < PROP_SINK_DEPTH {
                continue;
            }
            commands.entity(entity).try_insert(Settling {
                from: *transform,
                to: transform.with_translation(Vec3::new(p.x, ground, p.z)),
                elapsed: 0.0,
            });
        }
    }
}

/// Сглаженное оседание за [`SETTLE_SECONDS`].
fn settle(mut commands: Commands, time: Res<Time>, mut moving: Query<(Entity, &mut Transform, &mut Settling)>) {
    for (entity, mut transform, mut settling) in &mut moving {
        settling.elapsed += time.delta_secs();
        let t = (settling.elapsed / SETTLE_SECONDS).min(1.0);
        let k = t * t * (3.0 - 2.0 * t);
        transform.translation = settling.from.translation.lerp(settling.to.translation, k);
        transform.rotation = settling.from.rotation.slerp(settling.to.rotation, k);
        if t >= 1.0 {
            commands.entity(entity).try_remove::<Settling>();
        }
    }
}
