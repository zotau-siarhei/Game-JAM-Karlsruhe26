//! Кратеры: взрыв вырывает в рельефе чашу с валом и перекрашивает землю в коричневый.
//!
//! Сразу после деформации отсюда же уходит [`TerrainChanged`]: читатель сообщения всегда видит
//! уже изменённый рельеф, в каком бы порядке ни шли системы.

use bevy::prelude::*;

use crate::explosion::Explosion;
use crate::frame::GameSet;
use crate::subsidence::TerrainChanged;
use crate::terrain::{CRATER_EXTENT, Terrain, VisibleChunks};

/// Взрыв слабее этого рельеф не меняет.
const MIN_POWER: f32 = 0.3;

/// Глубина чаши на единицу мощности в долях радиуса: мортира (R = 5 м) даёт 2,5 м.
const DEPTH_PER_RADIUS: f32 = 0.5;

/// Высота вала на единицу мощности в долях радиуса: у мортиры 0,6 м.
const RIM_PER_RADIUS: f32 = 0.12;

/// Радиус выжженного пятна в радиусах взрыва: шире вала кратера.
const SCORCH_REACH: f32 = 2.6;
/// Насколько темнеет земля в центре пятна за один взрыв мощности 1.
const SCORCH_STRENGTH: f32 = 0.35;
/// Темнее этой яркости (линейной) пятно землю не делает: повторные взрывы не дают черноту.
const SCORCH_FLOOR: f32 = 0.025;

/// Подключает деформацию рельефа взрывами.
pub struct CratersPlugin;

impl Plugin for CratersPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, dig_craters.in_set(GameSet::Damage));
    }
}

/// Вырывает кратеры по взрывам, помечает на пересборку изменённые участки и их соседей по краю
/// и сообщает об изменении рельефа во всей зоне кратера: чаша и вал.
pub fn dig_craters(
    mut commands: Commands,
    mut explosions: MessageReader<Explosion>,
    mut terrain: ResMut<Terrain>,
    visible: Res<VisibleChunks>,
    mut changes: MessageWriter<TerrainChanged>,
) {
    let mut changed: Vec<IVec2> = Vec::new();
    for explosion in explosions.read() {
        if explosion.power < MIN_POWER {
            continue;
        }
        let r = explosion.radius;
        changed.extend(terrain.apply_crater(
            explosion.position,
            r,
            r * DEPTH_PER_RADIUS * explosion.power,
            r * RIM_PER_RADIUS * explosion.power,
        ));
        changes.write(TerrainChanged {
            center: explosion.position,
            radius: r * CRATER_EXTENT,
        });
        changed.extend(scorch(&mut terrain, explosion.position, r * SCORCH_REACH, explosion.power));
    }
    visible.mark_dirty(&mut commands, changed);
}

/// Затемняет цвета вершин вокруг взрыва, сохраняя оттенок рельефа: вокруг кратера остаётся
/// выжженное пятно с рваным краем. Возвращает изменённые участки.
fn scorch(terrain: &mut Terrain, center: Vec3, reach: f32, power: f32) -> Vec<IVec2> {
    let mut changed = Vec::new();
    let (x0, x1) = ((center.x - reach).floor() as i32, (center.x + reach).ceil() as i32);
    let (z0, z1) = ((center.z - reach).floor() as i32, (center.z + reach).ceil() as i32);
    let strength = (SCORCH_STRENGTH * power).min(0.6);
    for vz in z0..=z1 {
        for vx in x0..=x1 {
            let d = Vec2::new(vx as f32 - center.x, vz as f32 - center.z).length();
            // Рваный край: радиус колеблется по направлению на ±15 %.
            let angle = (vz as f32 - center.z).atan2(vx as f32 - center.x);
            let edge = reach * (1.0 + 0.15 * (angle * 5.0 + center.x).sin() * (angle * 3.0 - center.z).cos());
            if d >= edge {
                continue;
            }
            let t = d / edge;
            let k = strength * (1.0 - t * t);
            for coord in terrain.edit_vertex(vx, vz, |_, c| {
                for channel in &mut c[..3] {
                    *channel = (*channel * (1.0 - k)).max(SCORCH_FLOOR.min(*channel));
                }
            }) {
                if !changed.contains(&coord) {
                    changed.push(coord);
                }
            }
        }
    }
    changed
}
