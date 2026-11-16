//! Электрошок перехватчика: пока луноход замедлен ударом ([`Impact`]), по его корпусу
//! бегают голубые молнии — так видно, почему он не может разогнаться.
//!
//! Молнии — заранее поставленные тонкие бруски, дети лунохода. Каждые [`REDRAW`] секунды
//! у каждой дуги новый излом и новые концы на корпусе; чем больше осталось замедления,
//! тем больше дуг горит. На паузе молнии замирают вместе с `Time<Virtual>`.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::frame::GameSet;
use crate::rover::{Impact, Rover};
use crate::waves::balance;

/// Свечение разряда: насыщенно-голубое, чтобы не выгорало в белое.
pub const ARC_GLOW: LinearRgba = LinearRgba::rgb(1.2, 2.6, 9.0);
/// Цвет разряда без свечения.
pub const ARC_COLOR: Color = Color::srgb(0.35, 0.6, 1.0);

/// Сколько дуг горит сразу при полном замедлении.
const ARCS: usize = 4;
/// Отрезков в одной дуге.
const SEGMENTS: usize = 5;
/// Как часто дуги перескакивают на новое место, с.
const REDRAW: f32 = 0.06;
/// Толщина дуги в системе лунохода, м (корень лунохода увеличен вдвое).
const THICKNESS: f32 = 0.05;
/// Насколько излом отходит от прямой между концами, м.
const JITTER: f32 = 0.16;
/// Концы дуг: полуширина, полудлина и высоты корпуса в системе лунохода, м.
const HALF_WIDTH: f32 = 0.72;
const HALF_LENGTH: f32 = 0.85;
const TOP: f32 = 0.38;
const BOTTOM: f32 = -0.15;

/// Плагин молний на луноходе.
pub struct ShockPlugin;

impl Plugin for ShockPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShockClock>()
            .add_systems(Startup, create_shock_assets)
            .add_systems(PostStartup, attach_arcs)
            .add_systems(Update, draw_arcs.in_set(GameSet::Presentation));
    }
}

/// Сетка и материал молнии: создаются при старте, чтобы прогрев их застал.
#[derive(Resource)]
pub struct ShockAssets {
    /// Единичный брусок: растягивается по длине отрезка.
    pub segment: Handle<Mesh>,
    /// Светящийся материал разряда, им же светятся электроды перехватчика.
    pub glow: Handle<StandardMaterial>,
}

/// Отрезок молнии: какой дуге и по счёту какой он.
#[derive(Component)]
struct ArcSegment {
    arc: usize,
    index: usize,
}

/// Счётчик перерисовки и номер кадра молний для случайности облика.
#[derive(Resource, Default)]
struct ShockClock {
    timer: f32,
    frame: u32,
}

fn create_shock_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(ShockAssets {
        segment: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        glow: materials.add(StandardMaterial {
            base_color: ARC_COLOR,
            emissive: ARC_GLOW,
            unlit: true,
            ..default()
        }),
    });
}

/// Ставит луноходу спрятанные отрезки молний.
fn attach_arcs(mut commands: Commands, assets: Res<ShockAssets>, rover: Query<Entity, With<Rover>>) {
    let Ok(rover) = rover.single() else {
        return;
    };
    commands.entity(rover).with_children(|parent| {
        for arc in 0..ARCS {
            for index in 0..SEGMENTS {
                parent.spawn((
                    ArcSegment { arc, index },
                    Mesh3d(assets.segment.clone()),
                    MeshMaterial3d(assets.glow.clone()),
                    Transform::default(),
                    Visibility::Hidden,
                    NotShadowCaster,
                ));
            }
        }
    });
}

/// Сколько дуг горит при оставшемся замедлении `left`: ноль без удара, все — у предела.
pub fn lit_arcs(left: f32) -> usize {
    if left <= 0.0 {
        return 0;
    }
    ((left / balance::IMPACT_MAX * ARCS as f32).ceil() as usize).clamp(1, ARCS)
}

/// Псевдослучайное число 0..1 по номеру: облик молнии, на игру не влияет.
fn hash01(seed: u32) -> f32 {
    let mut x = seed.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^= x >> 15;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

/// Точка на краю корпуса: по периметру сверху или внизу у гусениц.
fn hull_point(seed: u32) -> Vec3 {
    let along = hash01(seed) * 4.0;
    let side = along.floor() as u32;
    let t = along.fract() * 2.0 - 1.0;
    let (x, z) = match side {
        0 => (t * HALF_WIDTH, -HALF_LENGTH),
        1 => (HALF_WIDTH, t * HALF_LENGTH),
        2 => (t * HALF_WIDTH, HALF_LENGTH),
        _ => (-HALF_WIDTH, t * HALF_LENGTH),
    };
    let y = if hash01(seed ^ 0xA5A5) < 0.6 { TOP } else { BOTTOM };
    Vec3::new(x, y, z)
}

/// Точки излома дуги: концы на корпусе, между ними — смещения поперёк и вверх.
fn arc_points(seed: u32) -> [Vec3; SEGMENTS + 1] {
    let start = hull_point(seed);
    let end = hull_point(seed.wrapping_add(7919));
    let mut points = [Vec3::ZERO; SEGMENTS + 1];
    for (i, point) in points.iter_mut().enumerate() {
        let t = i as f32 / SEGMENTS as f32;
        let mut p = start.lerp(end, t);
        // Дуга выгибается над палубой, чтобы не прятаться в корпусе.
        p.y = p.y.max(TOP) + (t * std::f32::consts::PI).sin() * 0.18;
        if i > 0 && i < SEGMENTS {
            let k = seed.wrapping_mul(31).wrapping_add(i as u32 * 101);
            p += Vec3::new(hash01(k) - 0.5, (hash01(k + 1) - 0.5) * 0.5, hash01(k + 2) - 0.5) * 2.0 * JITTER;
        }
        *point = p;
    }
    points
}

/// Перерисовывает молнии по оставшемуся замедлению лунохода.
fn draw_arcs(
    time: Res<Time>,
    mut clock: ResMut<ShockClock>,
    rover: Query<&Impact, With<Rover>>,
    mut segments: Query<(&ArcSegment, &mut Transform, &mut Visibility)>,
) {
    let lit = rover.single().map_or(0, |impact| lit_arcs(impact.left));
    clock.timer -= time.delta_secs();
    let redraw = clock.timer <= 0.0;
    if redraw {
        clock.timer = REDRAW;
        clock.frame = clock.frame.wrapping_add(1);
    }
    for (segment, mut transform, mut visibility) in &mut segments {
        let show = segment.arc < lit;
        let wanted = if show { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != wanted {
            *visibility = wanted;
        }
        if !show || !redraw {
            continue;
        }
        let points = arc_points(clock.frame.wrapping_mul(97).wrapping_add(segment.arc as u32 * 13));
        let (from, to) = (points[segment.index], points[segment.index + 1]);
        let length = from.distance(to).max(1e-3);
        // Брусок вытянут по Z: поворачиваем его ось вдоль отрезка.
        *transform = Transform::from_translation((from + to) * 0.5)
            .with_rotation(Quat::from_rotation_arc(Vec3::NEG_Z, (to - from) / length))
            .with_scale(Vec3::new(THICKNESS, THICKNESS, length));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn без_удара_молний_нет() {
        assert_eq!(lit_arcs(0.0), 0);
    }

    #[test]
    fn у_предела_горят_все_дуги() {
        assert_eq!(lit_arcs(balance::IMPACT_MAX), ARCS);
        assert_eq!(lit_arcs(0.01), 1);
    }

    #[test]
    fn концы_дуги_на_корпусе() {
        for seed in 0..200 {
            let p = hull_point(seed);
            assert!(p.x.abs() <= HALF_WIDTH + 1e-4 && p.z.abs() <= HALF_LENGTH + 1e-4);
        }
    }
}
