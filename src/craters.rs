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

/// Мельче этого чаша взрыва в воздухе не роется, м: землю он только опаляет.
const MIN_DEPTH: f32 = 0.05;

/// Высота взрыва над землёй в долях его радиуса: до первой кратер полный, от второй рельеф цел.
/// Ракета на бреющем полёте (1,2 м при радиусе 4 м) роет полностью, попадание в корпус босса
/// (3,4 м) — нет: иначе он опускался в собственную воронку.
const AIR_BLAST: (f32, f32) = (0.35, 0.85);

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

/// Доля глубины кратера у взрыва радиуса `radius` на высоте `height` над землёй: в воздухе взрыв
/// роет тем меньше, чем он выше. Под землёй и у самой земли — 1.
fn dig_share(height: f32, radius: f32) -> f32 {
    let (full, none) = (AIR_BLAST.0 * radius, AIR_BLAST.1 * radius);
    1.0 - smoothstep(full, none, height)
}

fn smoothstep(from: f32, to: f32, x: f32) -> f32 {
    let t = ((x - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
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
    // Высоты взрывов — до первого кратера кадра: снаряд залпа, упавший в ту же воронку, не «в воздухе».
    let blasts: Vec<(Explosion, f32)> = explosions
        .read()
        .filter(|explosion| explosion.power >= MIN_POWER)
        .map(|explosion| {
            let p = explosion.position;
            (*explosion, dig_share(p.y - terrain.height_at(p.x, p.z), explosion.radius))
        })
        .collect();
    for (explosion, share) in blasts {
        let r = explosion.radius;
        let depth = r * DEPTH_PER_RADIUS * explosion.power * share;
        if depth >= MIN_DEPTH {
            changed.extend(terrain.apply_crater(
                explosion.position,
                r,
                depth,
                r * RIM_PER_RADIUS * explosion.power * share,
            ));
            changes.write(TerrainChanged {
                center: explosion.position,
                radius: r * CRATER_EXTENT,
            });
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::Team;

    #[test]
    fn взрыв_в_воздухе_роет_слабее() {
        // У земли, под землёй и на бреющем полёте ракеты кратер полный.
        assert_eq!(dig_share(0.0, 4.0), 1.0);
        assert_eq!(dig_share(-10.0, 4.0), 1.0);
        assert_eq!(dig_share(1.2, 4.0), 1.0);
        // Ракета в корпусе босса рельеф не трогает.
        assert_eq!(dig_share(3.4, 4.0), 0.0);
        let shares: Vec<f32> = (0..40).map(|i| dig_share(i as f32 * 0.1, 4.0)).collect();
        assert!(shares.windows(2).all(|w| w[1] <= w[0]), "{shares:?}");
    }

    /// Глубина под точкой после одного кадра со взрывами на высотах `heights`.
    fn depth_after(heights: &[f32]) -> f32 {
        let mut app = App::new();
        app.insert_resource(Terrain::new(7))
            .init_resource::<VisibleChunks>()
            .add_message::<Explosion>()
            .add_message::<TerrainChanged>()
            .add_systems(Update, dig_craters);
        let at = |app: &App| app.world().resource::<Terrain>().height_at(10.0, 10.0);
        let before = at(&app);
        for height in heights {
            app.world_mut().write_message(Explosion {
                position: Vec3::new(10.0, before + height, 10.0),
                radius: 4.0,
                power: 1.0,
                source_team: Team::Player,
            });
        }
        app.update();
        before - at(&app)
    }

    #[test]
    fn кратер_зависит_от_высоты_взрыва() {
        assert!((depth_after(&[0.0]) - 2.0).abs() < 1e-3);
        assert_eq!(depth_after(&[3.6]), 0.0);
        let middle = depth_after(&[2.4]);
        assert!(middle > 0.2 && middle < 1.8, "{middle}");
    }

    /// Высоты берутся до первого кратера кадра: второй снаряд залпа роет столько же.
    #[test]
    fn взрывы_одного_кадра_роют_полностью() {
        assert!((depth_after(&[0.0, 0.0, 0.0]) - 6.0).abs() < 1e-3);
    }
}
