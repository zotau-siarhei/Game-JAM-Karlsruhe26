//! Облик лунохода: корпус и два гусеничных блока из моделей Meshy и бегущие грунтозацепы.
//!
//! Модели — `assets/models/rover_body.glb` и `assets/models/rover_track.glb`. Корпус в модели
//! смотрит на +Z, а луноход едет на -Z, поэтому корпус развёрнут. Гусеничный блок в модели
//! лежит вдоль X катками к -Z, поэтому каждый блок повёрнут катками наружу.
//!
//! Гусеница в модели неподвижна, поэтому поверх неё по контуру «стадион» бегут грунтозацепы.
//! Путь каждого борта считается по фактическому движению лунохода: при езде вперёд верхняя
//! ветвь бежит вперёд, нижняя назад, а на повороте на месте борта бегут в разные стороны.

use std::f32::consts::{FRAC_PI_2, PI};

use bevy::prelude::*;

use crate::rover::{RIDE_HEIGHT, Rover};

/// Модель корпуса.
const BODY_MODEL: &str = "models/rover_body.glb";
/// Модель гусеничного блока.
const TRACK_MODEL: &str = "models/rover_track.glb";

/// Масштаб моделей: корпус около 1,5 м в длину.
const MODEL_SCALE: f32 = 0.8;

/// Высота центра корпуса над центром лунохода, м: обод корпуса над гусеницами.
const BODY_Y: f32 = -RIDE_HEIGHT + 0.86 * MODEL_SCALE;

/// Полуширина колеи: центр гусеничного блока от оси лунохода, м.
pub const TRACK_X: f32 = 0.72;

/// Радиус закругления гусеницы по наружной поверхности ленты, м.
const TRACK_RADIUS: f32 = 0.43 * MODEL_SCALE;

/// Половина длины прямого участка гусеницы, м.
const TRACK_HALF_STRAIGHT: f32 = 0.52 * MODEL_SCALE;

/// Высота центра гусеничного блока над центром лунохода, м: лента касается земли.
const TRACK_Y: f32 = -RIDE_HEIGHT + TRACK_RADIUS;

/// Грунтозацепов на одну гусеницу.
const GROUSERS: usize = 18;

/// Размер грунтозацепа: поперёк ленты, толщина, вдоль ленты, м.
const GROUSER_SIZE: Vec3 = Vec3::new(0.28, 0.035, 0.07);

/// Цвет грунтозацепов: тёмная сталь, чуть светлее резины, чтобы бег ленты был заметен.
const GROUSER_COLOR: Color = Color::srgb(0.36, 0.38, 0.42);

/// Больше такого сдвига за кадр — это перенос лунохода (рестарт), а не езда, м.
const MAX_STEP: f32 = 3.0;

/// Плагин облика лунохода.
pub struct RoverLookPlugin;

impl Plugin for RoverLookPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (attach_look, run_tracks).chain());
    }
}

/// Борт гусеницы: -1 левый, 1 правый.
#[derive(Component, Clone, Copy)]
struct TrackSide(f32);

/// Грунтозацеп: номер на ленте.
#[derive(Component)]
struct Grouser(usize);

/// Пройденный путь гусениц по бортам и прошлое положение лунохода.
#[derive(Component, Default)]
struct TrackDrive {
    /// Путь левой и правой ленты, м.
    travel: [f32; 2],
    /// Положение и направление «вперёд» в прошлом кадре.
    last: Option<(Vec3, Vec3)>,
}

/// Длина ленты по контуру, м.
pub fn track_length(half_straight: f32, radius: f32) -> f32 {
    4.0 * half_straight + 2.0 * PI * radius
}

/// Точка ленты на расстоянии `u` по контуру: координаты (z, y) от центра блока и наружная нормаль.
///
/// Отсчёт идёт от передней точки нижней ветви назад, затем вверх по заднему закруглению,
/// вперёд по верхней ветви и вниз по переднему закруглению. Перёд — это -Z.
pub fn track_point(u: f32, half_straight: f32, radius: f32) -> (Vec2, Vec2) {
    let (a, r) = (half_straight, radius);
    let u = u.rem_euclid(track_length(a, r));
    let arc = PI * r;
    if u < 2.0 * a {
        (Vec2::new(-a + u, -r), Vec2::new(0.0, -1.0))
    } else if u < 2.0 * a + arc {
        let t = (u - 2.0 * a) / r;
        let n = Vec2::new(t.sin(), -t.cos());
        (Vec2::new(a, 0.0) + n * r, n)
    } else if u < 4.0 * a + arc {
        (Vec2::new(a - (u - 2.0 * a - arc), r), Vec2::new(0.0, 1.0))
    } else {
        let t = (u - 4.0 * a - arc) / r;
        let n = Vec2::new(-t.sin(), t.cos());
        (Vec2::new(-a, 0.0) + n * r, n)
    }
}

/// Сколько пробегают ленты левого и правого борта, когда луноход сместился на `step` вперёд
/// и повернулся на `turn` радиан влево. Полуширина колеи — `half_gauge`.
pub fn track_travel(step: f32, turn: f32, half_gauge: f32) -> [f32; 2] {
    [step - turn * half_gauge, step + turn * half_gauge]
}

/// Ставит на новый луноход корпус, гусеничные блоки и грунтозацепы.
fn attach_look(
    mut commands: Commands,
    rovers: Query<Entity, Added<Rover>>,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for rover in &rovers {
        let body = asset_server.load(GltfAssetLabel::Scene(0).from_asset(BODY_MODEL));
        let track = asset_server.load(GltfAssetLabel::Scene(0).from_asset(TRACK_MODEL));
        let grouser_mesh = meshes.add(Cuboid::from_size(GROUSER_SIZE));
        let grouser_material = materials.add(StandardMaterial {
            base_color: GROUSER_COLOR,
            metallic: 0.3,
            perceptual_roughness: 0.6,
            ..default()
        });

        commands
            .entity(rover)
            .insert(TrackDrive::default())
            .with_children(|parent| {
                parent.spawn((
                    WorldAssetRoot(body),
                    Transform::from_xyz(0.0, BODY_Y, 0.0)
                        .with_rotation(Quat::from_rotation_y(PI))
                        .with_scale(Vec3::splat(MODEL_SCALE)),
                ));
                for side in [-1.0f32, 1.0] {
                    parent
                        .spawn((
                            TrackSide(side),
                            Transform::from_xyz(side * TRACK_X, TRACK_Y, 0.0),
                            Visibility::default(),
                        ))
                        .with_children(|frame| {
                            // Катки модели смотрят на -Z: поворот выводит их наружу.
                            frame.spawn((
                                WorldAssetRoot(track.clone()),
                                Transform::from_rotation(Quat::from_rotation_y(-side * FRAC_PI_2))
                                    .with_scale(Vec3::splat(MODEL_SCALE)),
                            ));
                            for i in 0..GROUSERS {
                                frame.spawn((
                                    Grouser(i),
                                    Mesh3d(grouser_mesh.clone()),
                                    MeshMaterial3d(grouser_material.clone()),
                                    Transform::default(),
                                ));
                            }
                        });
                }
            });
    }
}

/// Считает путь лент по движению лунохода и раскладывает грунтозацепы по контуру.
fn run_tracks(
    mut rover: Query<(&Transform, &mut TrackDrive), With<Rover>>,
    sides: Query<&TrackSide>,
    mut grousers: Query<(&ChildOf, &Grouser, &mut Transform), Without<Rover>>,
) {
    let Some((transform, mut drive)) = rover.iter_mut().next() else {
        return;
    };
    let position = transform.translation;
    let forward = transform.forward().as_vec3();
    if let Some((last_position, last_forward)) = drive.last {
        let step = (position - last_position).dot(forward);
        let flat = |v: Vec3| Vec2::new(v.x, v.z).normalize_or_zero();
        // Поворот влево — против часовой стрелки сверху: курс растёт.
        let turn = -flat(last_forward).angle_to(flat(forward));
        if step.abs() < MAX_STEP {
            let travel = track_travel(step, turn, TRACK_X);
            drive.travel[0] += travel[0];
            drive.travel[1] += travel[1];
        }
    }
    drive.last = Some((position, forward));

    let length = track_length(TRACK_HALF_STRAIGHT, TRACK_RADIUS);
    let spacing = length / GROUSERS as f32;
    for (parent, grouser, mut transform) in &mut grousers {
        let Ok(side) = sides.get(parent.parent()) else {
            continue;
        };
        let travel = drive.travel[if side.0 < 0.0 { 0 } else { 1 }];
        let u = grouser.0 as f32 * spacing + travel;
        let (point, normal) = track_point(u, TRACK_HALF_STRAIGHT, TRACK_RADIUS);
        let normal = Vec3::new(0.0, normal.y, normal.x);
        transform.translation = Vec3::new(0.0, point.y, point.x) + normal * GROUSER_SIZE.y * 0.5;
        transform.rotation = Quat::from_rotation_arc(Vec3::Y, normal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: f32 = 0.4;
    const R: f32 = 0.3;

    fn close(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-3
    }

    #[test]
    fn track_contour_is_closed_and_continuous() {
        let length = track_length(A, R);
        // Начало и конец контура совпадают.
        assert!(close(track_point(0.0, A, R).0, track_point(length, A, R).0));
        // Стыки участков без разрывов: точки по обе стороны стыка рядом.
        for joint in [2.0 * A, 2.0 * A + PI * R, 4.0 * A + PI * R] {
            let before = track_point(joint - 1e-4, A, R).0;
            let after = track_point(joint + 1e-4, A, R).0;
            assert!(close(before, after), "разрыв на стыке {joint}");
        }
    }

    #[test]
    fn bottom_run_lies_on_the_ground_and_top_run_on_top() {
        let (bottom, normal) = track_point(A, A, R);
        assert!(close(bottom, Vec2::new(0.0, -R)));
        assert!(close(normal, Vec2::new(0.0, -1.0)));
        let (top, normal) = track_point(3.0 * A + PI * R, A, R);
        assert!(close(top, Vec2::new(0.0, R)));
        assert!(close(normal, Vec2::new(0.0, 1.0)));
        // Заднее закругление на +Z: середина дуги — самая задняя точка.
        let (back, _) = track_point(2.0 * A + PI * R * 0.5, A, R);
        assert!(close(back, Vec2::new(A + R, 0.0)));
    }

    #[test]
    fn driving_forward_moves_top_run_forward() {
        // Путь растёт: нижняя ветвь уходит назад (+Z), верхняя — вперёд (-Z).
        let bottom = track_point(A, A, R).0.x;
        let bottom_later = track_point(A + 0.1, A, R).0.x;
        assert!(bottom_later > bottom);
        let top = track_point(3.0 * A + PI * R, A, R).0.x;
        let top_later = track_point(3.0 * A + PI * R + 0.1, A, R).0.x;
        assert!(top_later < top);
    }

    #[test]
    fn turning_in_place_runs_tracks_in_opposite_directions() {
        let [left, right] = track_travel(0.0, 0.5, TRACK_X);
        assert!(left < 0.0 && right > 0.0);
        assert!((left + right).abs() < 1e-6);
        let [left, right] = track_travel(1.0, 0.0, TRACK_X);
        assert_eq!(left, right);
    }
}
