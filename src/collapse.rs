//! Поэтапное обрушение дома: крыша проваливается внутрь, панели стен заваливаются наружу
//! и отрываются, от основания расходится кольцо пыли, вылетают искры, на руинах горит огонь.
//!
//! Обрушение запускает сообщение [`CollapseHouse`]. Каждая часть в конце становится обычным
//! обломком через [`SpawnDebris`], поэтому всё, что упало, остаётся лежать.
//!
//! Если модель дома нарезана (`src/fragments.rs`), дом рушится своими кусками в копоти: верхние
//! проваливаются внутрь, как крыша, средние заваливаются наружу, как стены, а нижние остаются
//! руинами (`buildings::spawn_ruin`). Без кусков — плиты и панели в цветах дома.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::buildings::{house_fit, house_model};
use crate::debris::{DebrisShape, SpawnDebris};
use crate::debris_forms::FormKind;
use crate::fire::Fire;
use crate::fragments::ModelPieces;
use crate::frame::{GameSet, RunMessages};
use crate::run::RunScoped;
use crate::settlements::HouseState;

/// Гравитация для падающих кусков крыши и искр, м/с².
const GRAVITY: f32 = 20.0;

/// Сколько секунд панель стены заваливается, прежде чем оторваться.
const TIP_SECONDS: f32 = 0.35;

/// Угол, на котором панель отрывается, рад.
const TIP_ANGLE: f32 = 1.1;

/// Сколько секунд расходится кольцо пыли.
const DUST_SECONDS: f32 = 1.5;

/// Сколько горят руины, с.
const RUIN_FIRE_SECONDS: f32 = 6.0;
/// Сколько руина дымит после огня, с.
const RUIN_SMOKE: f32 = 20.0;

/// Сколько живёт искра, с.
const SPARK_SECONDS: f32 = 0.7;

/// Куски модели с центром ниже этой доли высоты остаются руинами.
pub const RUIN_LEVEL: f32 = 0.2;
/// Куски модели выше этой доли высоты падают внутрь, как крыша; между — заваливаются, как стены.
const ROOF_LEVEL: f32 = 0.6;

/// Подключает анимацию обрушения.
pub struct CollapsePlugin;

impl Plugin for CollapsePlugin {
    fn build(&self, app: &mut App) {
        app.add_run_message::<CollapseHouse>()
            .add_systems(Startup, create_collapse_assets)
            .add_systems(
                Update,
                (start_collapse, animate_parts, spread_dust, fly_sparks, burn_ruins)
                    .chain()
                    .in_set(GameSet::Presentation),
            );
    }
}

/// Дом начал рушиться.
#[derive(Message, Debug, Clone)]
pub struct CollapseHouse {
    pub house: HouseState,
    /// Основание дома в мире.
    pub base: Vec3,
    /// Цвета стен с материалами: панели перебирают их по кругу. Не пустой.
    pub walls: Vec<(Color, Handle<StandardMaterial>)>,
    /// Цвета крыши с материалами. Не пустой.
    pub roofs: Vec<(Color, Handle<StandardMaterial>)>,
    /// Центр разрушившего взрыва: стены с его стороны падают раньше и летят дальше.
    pub blast: Option<Vec3>,
}

/// Часть рушащегося дома.
#[derive(Component)]
struct Part {
    /// Через сколько секунд часть начнёт двигаться.
    delay: f32,
    motion: Motion,
    color: Color,
    /// Куб или кусок модели: таким станет и обломок.
    shape: DebrisShape,
}

enum Motion {
    /// Кусок крыши падает внутрь дома до уровня пола.
    Falling { velocity: Vec3, floor: f32 },
    /// Панель стены поворачивается вокруг нижнего ребра наружу.
    Tipping {
        pivot: Vec3,
        axis: Vec3,
        rest: Transform,
        elapsed: f32,
        /// Скорость отрыва наружу, м/с.
        push: f32,
        outward: Vec3,
    },
}

/// Кольцо пыли от основания.
#[derive(Component)]
struct Dust {
    elapsed: f32,
    radius: f32,
    material: Handle<StandardMaterial>,
}

/// Искра.
#[derive(Component)]
struct Spark {
    velocity: Vec3,
    remaining: f32,
}

/// Очаг огня на руинах: сколько ещё, с. Пламя и дым после него ставит модуль огня.
#[derive(Component)]
struct RuinFire {
    remaining: f32,
}

#[derive(Resource)]
struct CollapseAssets {
    cube: Handle<Mesh>,
    ring: Handle<Mesh>,
    spark: Handle<StandardMaterial>,
}

fn create_collapse_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(CollapseAssets {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        ring: meshes.add(Torus::new(0.75, 1.0)),
        spark: materials.add(StandardMaterial {
            // Огонь ярче единицы: светится через свечение камеры (`unlit` выводит только `base_color`).
            base_color: Color::linear_rgb(4.8, 3.2, 0.8),
            unlit: true,
            ..default()
        }),
    });
}

/// Простая псевдослучайность от числа: у каждого дома своя.
struct Jitter(f32);

impl Jitter {
    fn next(&mut self) -> f32 {
        self.0 = ((self.0 * 12.9898 + 4.1414).sin() * 43758.547).fract().abs();
        self.0
    }

    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next()
    }
}

/// Раскладывает дом на части и запускает хореографию.
fn start_collapse(
    mut commands: Commands,
    mut requests: MessageReader<CollapseHouse>,
    assets: Res<CollapseAssets>,
    pieces: Res<ModelPieces>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut debris: MessageWriter<SpawnDebris>,
) {
    for request in requests.read() {
        let house = &request.house;
        let footprint = house.footprint();
        let half = footprint.half;
        let height = house.kind.wall_height();
        let rotation = Quat::from_rotation_y(house.yaw);
        let base = request.base;
        let mut rng = Jitter(house.position.x * 0.37 + house.position.y * 0.91);
        let to_world = |local: Vec3| base + rotation * local;
        let blast_dir = request.blast.map(|b| {
            let d = Vec3::new(house.position.x - b.x, 0.0, house.position.y - b.z);
            d.normalize_or(Vec3::X)
        });

        if let Some(set) = pieces.get(house_model(house.kind)) {
            // Доски перекрытий и крошка штукатурки из библиотеки форм, в цветах дома.
            for i in 0..10 {
                let plank = i % 2 == 0;
                let a = rng.range(0.0, std::f32::consts::TAU);
                let out = Vec3::new(a.cos(), 0.0, a.sin());
                let color = if plank { Color::srgb(0.42, 0.29, 0.17) } else { request.walls[i % request.walls.len()].0 };
                let s = rng.range(0.7, 1.2);
                debris.write(SpawnDebris {
                    position: base + Vec3::Y * height * rng.range(0.3, 0.8) + out * half.min_element() * 0.5,
                    velocity: out * rng.range(3.0, 7.0) + Vec3::Y * rng.range(4.0, 8.0),
                    size: if plank { Vec3::new(0.18, 0.09, 1.6) * s } else { Vec3::splat(0.35) * s },
                    color,
                    burning: i == 0,
                    shape: DebrisShape::Form(if plank { FormKind::Plank } else { FormKind::Crumb }),
                    rotation: None,
                });
            }
            // Куски своей модели: дом стоит в `base` с поворотом, модель — под растяжкой `fit`.
            let fit = house_fit(house.kind);
            let frame = Transform::from_translation(base).with_rotation(rotation);
            for piece in &set.pieces {
                let level = set.level(piece);
                if level < RUIN_LEVEL {
                    continue;
                }
                let local = set.local(piece, fit);
                let world = frame * local;
                let flat = local.translation.with_y(0.0).normalize_or(Vec3::X);
                let outward = rotation * flat;
                let facing_blast = blast_dir.map_or(0.0, |d| (-d).dot(outward).max(0.0));
                let motion = if level > ROOF_LEVEL {
                    Motion::Falling {
                        velocity: -outward * rng.range(0.5, 1.5) - Vec3::Y,
                        floor: base.y + rng.range(0.4, 1.0),
                    }
                } else {
                    // Заваливается вокруг своего нижнего края.
                    let bottom = (local.translation.y - piece.half.y * local.scale.y).max(0.0);
                    Motion::Tipping {
                        pivot: frame.transform_point(local.translation.with_y(bottom)),
                        axis: rotation * Vec3::Y.cross(flat),
                        rest: world,
                        elapsed: 0.0,
                        push: 2.0 + rng.range(0.0, 2.0) + facing_blast * 7.0,
                        outward,
                    }
                };
                let delay = match motion {
                    Motion::Falling { .. } => rng.range(0.0, 0.3),
                    Motion::Tipping { .. } => 0.2 + rng.range(0.0, 0.35) * (1.0 - facing_blast * 0.8),
                };
                commands.spawn((
                    Part {
                        delay,
                        motion,
                        color: Color::WHITE,
                        shape: DebrisShape::Mesh {
                            mesh: piece.mesh.clone(),
                            material: set.charred.clone(),
                            half: piece.half,
                        },
                    },
                    RunScoped,
                    Mesh3d(piece.mesh.clone()),
                    MeshMaterial3d(set.charred.clone()),
                    world,
                ));
            }
        } else {
        // Крыша: 4–6 кусков, проседают и падают внутрь со случайными задержками.
        let roof_pieces = 4 + (rng.next() * 3.0) as usize;
        for i in 0..roof_pieces {
            let (roof_color, roof_material) = &request.roofs[i % request.roofs.len()];
            let fx = (i % 2) as f32 - 0.5;
            let fz = (i / 2) as f32 / ((roof_pieces as f32 / 2.0).ceil()).max(1.0) - 0.4;
            let size = Vec3::new(half.x, 0.35, (half.y * 2.0 / (roof_pieces as f32 / 2.0).ceil()).max(1.0));
            let local = Vec3::new(fx * half.x, height + 0.4, fz * half.y * 2.0);
            commands.spawn((
                Part {
                    delay: rng.range(0.0, 0.3),
                    motion: Motion::Falling {
                        velocity: Vec3::new(-fx * 1.5, -1.0, 0.0),
                        floor: base.y + 0.4,
                    },
                    color: *roof_color,
                    shape: DebrisShape::Cube,
                },
                RunScoped,
                Mesh3d(assets.cube.clone()),
                MeshMaterial3d(roof_material.clone()),
                Transform::from_translation(to_world(local))
                    .with_rotation(rotation * Quat::from_rotation_z(fx * 0.5))
                    .with_scale(size),
            ));
        }

        // Стены: каждая из 3–4 панелей заваливается наружу вокруг нижнего ребра.
        let walls = [
            (Vec3::X, half.x, half.y, Vec3::Z),
            (Vec3::NEG_X, half.x, half.y, Vec3::Z),
            (Vec3::Z, half.y, half.x, Vec3::X),
            (Vec3::NEG_Z, half.y, half.x, Vec3::X),
        ];
        let mut panel_count = 0;
        for (normal, offset, span, along) in walls {
            let outward = rotation * normal;
            let facing_blast = blast_dir.map_or(0.0, |d| (-d).dot(outward).max(0.0));
            let panels = 3 + (rng.next() * 2.0) as usize;
            let width = span * 2.0 / panels as f32;
            for p in 0..panels {
                let (wall_color, wall_material) = &request.walls[panel_count % request.walls.len()];
                panel_count += 1;
                let t = -span + width * (p as f32 + 0.5);
                let center_local = normal * offset + along * t + Vec3::Y * height * 0.5;
                let pivot_local = normal * offset + along * t;
                let scale = if normal.x != 0.0 {
                    Vec3::new(0.3, height, width * 0.95)
                } else {
                    Vec3::new(width * 0.95, height, 0.3)
                };
                let rest = Transform::from_translation(to_world(center_local))
                    .with_rotation(rotation)
                    .with_scale(scale);
                // Стена со стороны взрыва падает раньше и улетает дальше.
                let delay = 0.2 + rng.range(0.0, 0.35) * (1.0 - facing_blast * 0.8);
                commands.spawn((
                    Part {
                        delay,
                        motion: Motion::Tipping {
                            pivot: to_world(pivot_local),
                            axis: rotation * Vec3::Y.cross(normal),
                            rest,
                            elapsed: 0.0,
                            push: 2.0 + rng.range(0.0, 2.0) + facing_blast * 7.0,
                            outward,
                        },
                        color: *wall_color,
                        shape: DebrisShape::Cube,
                    },
                    RunScoped,
                    Mesh3d(assets.cube.clone()),
                    MeshMaterial3d(wall_material.clone()),
                    rest,
                ));
            }
        }
        }

        // Кольцо пыли от основания.
        let dust = materials.add(StandardMaterial {
            base_color: Color::srgba(0.72, 0.66, 0.58, 0.7),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..default()
        });
        commands.spawn((
            Dust {
                elapsed: 0.0,
                radius: half.length() + 1.0,
                material: dust.clone(),
            },
            RunScoped,
            NotShadowCaster,
            Mesh3d(assets.ring.clone()),
            MeshMaterial3d(dust),
            Transform::from_translation(base + Vec3::Y * 0.3).with_scale(Vec3::new(1.0, 0.6, 1.0)),
        ));

        // Искры.
        for _ in 0..14 {
            let dir = Vec3::new(rng.range(-1.0, 1.0), rng.range(0.6, 1.4), rng.range(-1.0, 1.0));
            commands.spawn((
                Spark {
                    velocity: dir * rng.range(5.0, 10.0),
                    remaining: SPARK_SECONDS * rng.range(0.6, 1.0),
                },
                RunScoped,
                NotShadowCaster,
                Mesh3d(assets.cube.clone()),
                MeshMaterial3d(assets.spark.clone()),
                Transform::from_translation(base + Vec3::Y * height * 0.5).with_scale(Vec3::splat(0.12)),
            ));
        }

        // Огонь на руинах: несколько очагов, гаснут за несколько секунд.
        for i in 0..3 {
            let local = Vec3::new(rng.range(-0.6, 0.6) * half.x, 0.3, rng.range(-0.6, 0.6) * half.y);
            let burn = RUIN_FIRE_SECONDS * rng.range(0.6, 1.0) + i as f32 * 0.3;
            commands.spawn((
                RuinFire { remaining: burn + 0.2 },
                Fire::new(rng.range(1.2, 2.0), 1.0).for_seconds(burn).then_smoke(RUIN_SMOKE),
                RunScoped,
                Transform::from_translation(to_world(local)),
            ));
        }
    }
}

/// Двигает части по хореографии; закончившая часть становится обломком.
fn animate_parts(
    mut commands: Commands,
    time: Res<Time>,
    mut parts: Query<(Entity, &mut Transform, &mut Part)>,
    mut debris: MessageWriter<SpawnDebris>,
) {
    let dt = time.delta_secs();
    for (entity, mut transform, mut part) in &mut parts {
        if part.delay > 0.0 {
            part.delay -= dt;
            continue;
        }
        let color = part.color;
        let shape = part.shape.clone();
        let model_piece = matches!(shape, DebrisShape::Mesh { .. });
        match &mut part.motion {
            Motion::Falling { velocity, floor } => {
                velocity.y -= GRAVITY * dt;
                transform.translation += *velocity * dt;
                if transform.translation.y <= *floor {
                    let v = *velocity;
                    commands.entity(entity).despawn();
                    // Кусок модели остаётся своего размера и поворота, плита крыши — чуть меньше.
                    debris.write(SpawnDebris {
                        position: transform.translation,
                        velocity: Vec3::new(v.x, 2.0, v.z),
                        size: if model_piece { transform.scale } else { transform.scale * 0.8 },
                        color,
                        burning: false,
                        rotation: model_piece.then_some(transform.rotation),
                        shape: shape.clone(),
                    });
                }
            }
            Motion::Tipping {
                pivot,
                axis,
                rest,
                elapsed,
                push,
                outward,
            } => {
                *elapsed += dt;
                let t = (*elapsed / TIP_SECONDS).min(1.0);
                let angle = TIP_ANGLE * t * t;
                let turn = Quat::from_axis_angle(*axis, angle);
                transform.translation = *pivot + turn * (rest.translation - *pivot);
                transform.rotation = turn * rest.rotation;
                if t >= 1.0 && model_piece {
                    // Кусок модели отрывается целиком.
                    commands.entity(entity).despawn();
                    debris.write(SpawnDebris {
                        position: transform.translation,
                        velocity: *outward * *push + Vec3::Y * 3.0,
                        size: transform.scale,
                        color,
                        burning: false,
                        rotation: Some(transform.rotation),
                        shape: shape.clone(),
                    });
                } else if t >= 1.0 {
                    // Панель отрывается и раскалывается на два куска.
                    commands.entity(entity).despawn();
                    let up = transform.rotation * Vec3::Y;
                    let piece = transform.scale * Vec3::new(1.0, 0.5, 1.0);
                    for k in [-0.25, 0.25] {
                        debris.write(SpawnDebris {
                            position: transform.translation + up * transform.scale.y * k,
                            velocity: *outward * (*push + k * 2.0) + Vec3::Y * (3.0 + k * 4.0),
                            size: piece,
                            color,
                            burning: false,
                            ..default()
                        });
                    }
                }
            }
        }
    }
}

/// Кольцо пыли расходится, поднимается и тает.
fn spread_dust(
    mut commands: Commands,
    time: Res<Time>,
    mut dust: Query<(Entity, &mut Transform, &mut Dust)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (entity, mut transform, mut ring) in &mut dust {
        ring.elapsed += time.delta_secs();
        let t = ring.elapsed / DUST_SECONDS;
        if t >= 1.0 {
            commands.entity(entity).despawn();
            materials.remove(&ring.material);
            continue;
        }
        let r = ring.radius * (0.6 + t * 1.2);
        transform.scale = Vec3::new(r, 0.6 + t * 2.5, r);
        transform.translation.y += time.delta_secs() * 0.8;
        if let Some(mut material) = materials.get_mut(&ring.material) {
            material.base_color.set_alpha(0.7 * (1.0 - t));
        }
    }
}

fn fly_sparks(mut commands: Commands, time: Res<Time>, mut sparks: Query<(Entity, &mut Transform, &mut Spark)>) {
    let dt = time.delta_secs();
    for (entity, mut transform, mut spark) in &mut sparks {
        spark.remaining -= dt;
        if spark.remaining <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        spark.velocity.y -= GRAVITY * dt;
        transform.translation += spark.velocity * dt;
    }
}

/// Носитель очага удаляется, когда огонь догорел: руина дымит дальше по `then_smoke`.
fn burn_ruins(mut commands: Commands, time: Res<Time>, mut fires: Query<(Entity, &mut RuinFire)>) {
    for (entity, mut fire) in &mut fires {
        fire.remaining -= time.delta_secs();
        if fire.remaining <= 0.0 {
            commands.entity(entity).despawn();
        }
    }
}
