//! Провал в кратер: дома и крупные объекты оседают на новую поверхность и кренятся по ней,
//! дом, просевший слишком глубоко или накренившийся слишком сильно, рушится.
//!
//! Рельеф меняют модули `craters`, `laser`, `rails`: каждый после деформации сам шлёт
//! [`TerrainChanged`], на него отвечают дома, объекты, пути и лава.

use bevy::prelude::*;

use crate::buildings::House;
use crate::combat::{Damage, Health, Team};
use crate::craters;
use crate::frame::{GameSet, RunMessages};
use crate::props::{Prop, PropKind, PropStatus, Props};
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
                (sink_houses, sink_props).chain().after(craters::dig_craters).in_set(GameSet::Damage),
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

/// Крупные объекты в кратере опускаются на новую поверхность и кренятся по склону.
fn sink_props(
    mut commands: Commands,
    mut changes: MessageReader<TerrainChanged>,
    terrain: Res<Terrain>,
    mut props: ResMut<Props>,
    objects: Query<(Entity, &Prop, &Transform)>,
) {
    for change in changes.read() {
        for (entity, prop, transform) in &objects {
            if !prop.kind.is_large() {
                continue;
            }
            let p = transform.translation;
            if Vec2::new(p.x - change.center.x, p.z - change.center.z).length() > change.radius {
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
            // Деревья кренятся по склону кратера, камни ложатся на него.
            let lean = if prop.kind == PropKind::Tree {
                Quat::from_rotation_arc(Vec3::Y, up.lerp(Vec3::Y, 0.4).normalize())
            } else {
                Quat::from_rotation_arc(Vec3::Y, up)
            };
            let to = Transform {
                translation: Vec3::new(p.x, ground, p.z),
                rotation: lean * transform.rotation,
                scale: transform.scale,
            };
            state.transform = to;
            commands.entity(entity).try_insert(Settling {
                from: *transform,
                to,
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
