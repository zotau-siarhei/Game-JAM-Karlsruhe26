//! Обломки: летят по баллистике, кувыркаются, отскакивают от рельефа и засыпают.
//!
//! Обломок создаётся сообщением [`SpawnDebris`]. Уснувший обломок записывается в хранилище
//! [`DebrisStore`] по участку рельефа и восстанавливается, когда участок снова загружен,
//! поэтому обломки лежат на месте после отъезда и возвращения. Коллайдеров у обломков нет:
//! машины проезжают сквозь них.

use std::collections::HashMap;

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::run::{RunScoped, RunStarted};
use crate::terrain::{Terrain, VisibleChunks, chunk_coord_at};

/// Гравитация, м/с².
const GRAVITY: f32 = 20.0;

/// Упругость отскока.
const BOUNCINESS: f32 = 0.3;

/// Сколько раз обломок отскакивает, прежде чем лечь.
const MAX_BOUNCES: u32 = 2;

/// Доля горизонтальной скорости, которая остаётся после удара о землю.
const GROUND_FRICTION: f32 = 0.45;

/// Медленнее этого обломок на земле засыпает, м/с.
const SLEEP_SPEED: f32 = 0.8;

/// Обломки меньше этого не отбрасывают тени, м.
const SHADOW_SIZE: f32 = 0.8;

/// Сколько горящих обломков может быть одновременно.
const MAX_BURNING: usize = 12;

/// Подключает обломки.
pub struct DebrisPlugin;

impl Plugin for DebrisPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebrisStore>()
            .add_message::<SpawnDebris>()
            .add_systems(Startup, create_debris_assets)
            .add_systems(
                Update,
                (reset_debris, spawn_debris, fly_debris, burn_out, stream_debris).chain(),
            );
    }
}

/// Просьба создать летящий обломок.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpawnDebris {
    pub position: Vec3,
    pub velocity: Vec3,
    /// Размеры куска, м.
    pub size: Vec3,
    pub color: Color,
    /// Обломок горит несколько секунд.
    pub burning: bool,
}

/// Уснувший обломок в хранилище.
#[derive(Debug, Clone, Copy)]
struct DebrisPiece {
    transform: Transform,
    color: Color,
}

/// Уснувшие обломки забега по участкам рельефа.
#[derive(Resource, Default)]
struct DebrisStore {
    chunks: HashMap<IVec2, Vec<DebrisPiece>>,
    /// Участки, обломки которых сейчас в сцене.
    shown: HashMap<IVec2, Vec<Entity>>,
}

/// Общий меш и материалы по цвету.
#[derive(Resource)]
struct DebrisAssets {
    cube: Handle<Mesh>,
    fire_mesh: Handle<Mesh>,
    fire: Handle<StandardMaterial>,
    materials: HashMap<[u8; 3], Handle<StandardMaterial>>,
}

impl DebrisAssets {
    fn material(&mut self, color: Color, materials: &mut Assets<StandardMaterial>) -> Handle<StandardMaterial> {
        let srgb = color.to_srgba();
        let key = [srgb.red, srgb.green, srgb.blue].map(|c| (c * 255.0) as u8);
        self.materials
            .entry(key)
            .or_insert_with(|| {
                materials.add(StandardMaterial {
                    base_color: color,
                    perceptual_roughness: 0.9,
                    ..default()
                })
            })
            .clone()
    }
}

/// Летящий обломок.
#[derive(Component)]
struct Flying {
    velocity: Vec3,
    spin: Vec3,
    bounces: u32,
    color: Color,
}

/// Огонь на обломке: гаснет по таймеру.
#[derive(Component)]
struct Burning {
    remaining: f32,
    flame: Entity,
}

fn create_debris_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(DebrisAssets {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        fire_mesh: meshes.add(Sphere::new(0.5).mesh().ico(1).unwrap()),
        fire: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.55, 0.1),
            emissive: LinearRgba::rgb(8.0, 3.0, 0.4),
            unlit: true,
            ..default()
        }),
        materials: HashMap::new(),
    });
}

fn reset_debris(mut started: MessageReader<RunStarted>, mut store: ResMut<DebrisStore>) {
    if started.read().last().is_some() {
        // Сущности прошлого забега удалены через RunScoped.
        *store = DebrisStore::default();
    }
}

/// Создаёт летящие обломки по сообщениям.
fn spawn_debris(
    mut commands: Commands,
    mut requests: MessageReader<SpawnDebris>,
    mut assets: ResMut<DebrisAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    burning: Query<(), With<Burning>>,
) {
    let mut burning_count = burning.iter().count();
    for (i, request) in requests.read().enumerate() {
        let material = assets.material(request.color, &mut materials);
        let k = i as f32;
        let mut entity = commands.spawn((
            Flying {
                velocity: request.velocity,
                spin: Vec3::new((k * 1.3).sin(), (k * 2.1).cos(), (k * 0.7).sin()) * 8.0,
                bounces: 0,
                color: request.color,
            },
            RunScoped,
            Mesh3d(assets.cube.clone()),
            MeshMaterial3d(material),
            Transform::from_translation(request.position)
                .with_rotation(Quat::from_euler(EulerRot::XYZ, k, k * 0.5, k * 0.3))
                .with_scale(request.size),
        ));
        if request.size.max_element() < SHADOW_SIZE {
            entity.insert(NotShadowCaster);
        }
        if request.burning && burning_count < MAX_BURNING {
            burning_count += 1;
            let flame = commands
                .spawn((
                    Mesh3d(assets.fire_mesh.clone()),
                    MeshMaterial3d(assets.fire.clone()),
                    NotShadowCaster,
                    Transform::from_xyz(0.0, 0.6, 0.0).with_scale(Vec3::splat(1.1)),
                ))
                .id();
            let parent = entity.id();
            commands.entity(parent).add_child(flame).insert(Burning {
                remaining: 3.0 + (k * 0.37).fract() * 3.0,
                flame,
            });
        }
    }
}

/// Баллистика, отскоки и засыпание. Уснувший обломок пишется в хранилище.
fn fly_debris(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut store: ResMut<DebrisStore>,
    mut flying: Query<(Entity, &mut Transform, &mut Flying)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for (entity, mut transform, mut piece) in &mut flying {
        piece.velocity.y -= GRAVITY * dt;
        transform.translation += piece.velocity * dt;
        let spin = piece.spin * dt;
        transform.rotate(Quat::from_euler(EulerRot::XYZ, spin.x, spin.y, spin.z));

        let half_height = transform.scale.y * 0.5;
        let ground = terrain.height_at(transform.translation.x, transform.translation.z) + half_height;
        if transform.translation.y > ground {
            continue;
        }
        transform.translation.y = ground;
        if piece.velocity.y < 0.0 {
            piece.velocity.y = -piece.velocity.y * BOUNCINESS;
            piece.velocity.x *= GROUND_FRICTION;
            piece.velocity.z *= GROUND_FRICTION;
            piece.spin *= 0.5;
            piece.bounces += 1;
        }
        if piece.bounces > MAX_BOUNCES || piece.velocity.length() < SLEEP_SPEED {
            // Ложится плашмя по рельефу и засыпает.
            let up = terrain.normal_at(transform.translation.x, transform.translation.z);
            let (_, yaw, _) = transform.rotation.to_euler(EulerRot::YXZ);
            transform.rotation = Quat::from_rotation_arc(Vec3::Y, up) * Quat::from_rotation_y(yaw);
            let chunk = chunk_coord_at(transform.translation.x, transform.translation.z);
            store.chunks.entry(chunk).or_default().push(DebrisPiece {
                transform: *transform,
                color: piece.color,
            });
            store.shown.entry(chunk).or_default().push(entity);
            commands.entity(entity).remove::<Flying>();
        }
    }
}

/// Огонь на обломках гаснет.
fn burn_out(mut commands: Commands, time: Res<Time>, mut burning: Query<(Entity, &mut Burning)>, mut flames: Query<&mut Transform>) {
    for (entity, mut fire) in &mut burning {
        fire.remaining -= time.delta_secs();
        if let Ok(mut t) = flames.get_mut(fire.flame) {
            let flicker = 1.0 + (fire.remaining * 23.0).sin() * 0.15;
            t.scale = Vec3::splat(fire.remaining.clamp(0.0, 1.0) * 1.1 * flicker);
        }
        if fire.remaining <= 0.0 {
            commands.entity(fire.flame).try_despawn();
            commands.entity(entity).remove::<Burning>();
        }
    }
}

/// Уснувшие обломки появляются и убираются вместе с участками рельефа.
fn stream_debris(
    mut commands: Commands,
    visible: Res<VisibleChunks>,
    mut store: ResMut<DebrisStore>,
    mut assets: ResMut<DebrisAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !visible.is_changed() {
        return;
    }
    let store = &mut *store;
    store.shown.retain(|&chunk, entities| {
        let keep = visible.entity(chunk).is_some();
        if !keep {
            for &e in entities.iter() {
                commands.entity(e).try_despawn();
            }
        }
        keep
    });
    for (&chunk, pieces) in &store.chunks {
        if store.shown.contains_key(&chunk) || visible.entity(chunk).is_none() {
            continue;
        }
        let entities = pieces
            .iter()
            .map(|piece| {
                let material = assets.material(piece.color, &mut materials);
                let mut e = commands.spawn((
                    RunScoped,
                    Mesh3d(assets.cube.clone()),
                    MeshMaterial3d(material),
                    piece.transform,
                ));
                if piece.transform.scale.max_element() < SHADOW_SIZE {
                    e.insert(NotShadowCaster);
                }
                e.id()
            })
            .collect();
        store.shown.insert(chunk, entities);
    }
}
