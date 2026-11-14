//! Кратеры: взрыв вырывает в рельефе чашу с валом и перекрашивает землю в коричневый.

use bevy::prelude::*;

use crate::explosion::Explosion;
use crate::terrain::{self, ChunkDirty, Terrain, VisibleChunks};

/// Взрыв слабее этого рельеф не меняет.
const MIN_POWER: f32 = 0.3;

/// Глубина чаши на единицу мощности в долях радиуса: мортира (R = 5 м) даёт 2,5 м.
const DEPTH_PER_RADIUS: f32 = 0.5;

/// Высота вала на единицу мощности в долях радиуса: у мортиры 0,6 м.
const RIM_PER_RADIUS: f32 = 0.12;

/// Подключает деформацию рельефа взрывами.
pub struct CratersPlugin;

impl Plugin for CratersPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            dig_craters.before(terrain::rebuild_dirty_chunks),
        );
    }
}

/// Вырывает кратеры по взрывам и помечает на пересборку изменённые участки и их соседей по краю.
fn dig_craters(
    mut commands: Commands,
    mut explosions: MessageReader<Explosion>,
    mut terrain: ResMut<Terrain>,
    visible: Res<VisibleChunks>,
) {
    let mut dirty: Vec<IVec2> = Vec::new();
    for explosion in explosions.read() {
        if explosion.power < MIN_POWER {
            continue;
        }
        let r = explosion.radius;
        let changed = terrain.apply_crater(
            explosion.position,
            r,
            r * DEPTH_PER_RADIUS * explosion.power,
            r * RIM_PER_RADIUS * explosion.power,
        );
        for coord in changed {
            // Соседи по краю: их краевые нормали считаются по изменённым высотам.
            for offset in [IVec2::ZERO, IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y] {
                let c = coord + offset;
                if !dirty.contains(&c) {
                    dirty.push(c);
                }
            }
        }
    }
    for coord in dirty {
        if let Some(entity) = visible.entity(coord) {
            commands.entity(entity).insert(ChunkDirty);
        }
    }
}

