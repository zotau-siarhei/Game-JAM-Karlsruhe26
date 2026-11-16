//! Гильзы пулемётных очередей: на каждый `FireBullet` вбок от ствола вылетает латунная гильза,
//! кувыркается, один-два раза отскакивает от рельефа и ложится набок по склону.
//!
//! Гильз не больше [`MAX_CASINGS`]: сверх предела исчезает самая старая. Лежащая гильза через
//! [`REST_TIME`] секунд уменьшается до нуля за [`SHRINK_TIME`]. Кратер (`TerrainChanged`) убирает
//! гильзы в своём круге, на лаве и в воде гильза исчезает при касании. Гильзы ни с чем не сталкиваются:
//! у них нет коллайдера и здоровья. Звон о землю — только у гильз лунохода, сообщением
//! [`CasingClink`], звук ставит `audio_events`.

use std::collections::VecDeque;
use std::f32::consts::FRAC_PI_2;

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::bullet::FireBullet;
use crate::combat::Team;
use crate::frame::GameSet;
use crate::lava::{self, LavaPools};
use crate::puddles::Puddles;
use crate::run::{RunScoped, WorldRebuilt};
use crate::subsidence::TerrainChanged;
use crate::terrain::Terrain;

/// Сколько гильз одновременно, не больше.
pub const MAX_CASINGS: usize = 300;

/// Сколько лежит гильза, прежде чем начнёт исчезать, с.
const REST_TIME: f32 = 30.0;

/// За сколько лежащая гильза уменьшается до нуля, с.
const SHRINK_TIME: f32 = 2.0;

/// Размер гильзы: радиус и длина, м. Крупнее настоящей: камера смотрит издалека.
const RADIUS: f32 = 0.03;
const LENGTH: f32 = 0.14;

/// Скорость вылета вбок и вверх, м/с.
const SIDE_SPEED: (f32, f32) = (3.0, 5.0);
const UP_SPEED: (f32, f32) = (2.0, 3.0);

/// Скорость кувыркания, рад/с.
const SPIN: f32 = 25.0;

/// Ускорение свободного падения, м/с².
const GRAVITY: f32 = 9.8;

/// Какая доля скорости остаётся после отскока.
const BOUNCE_KEEP: f32 = 0.3;

/// После стольких отскоков или медленнее этого гильза ложится, м/с.
const MAX_BOUNCES: u8 = 2;
const REST_SPEED: f32 = 0.5;

/// Звон гильз лунохода не чаще этого, с.
const CLINK_PERIOD: f32 = 0.25;

/// Плагин гильз.
pub struct CasingsPlugin;

impl Plugin for CasingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Casings>()
            .add_message::<CasingClink>()
            .add_systems(Startup, create_assets)
            .add_systems(
                Update,
                (
                    reset_casings.in_set(GameSet::Reset),
                    (eject_casings, fly_casings, settle_casings).chain().in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Гильза лунохода звякнула о землю: тихий звук рядом с ним.
#[derive(Message, Debug, Clone, Copy)]
pub struct CasingClink {
    pub position: Vec3,
}

/// Гильза.
#[derive(Component)]
struct Casing {
    velocity: Vec3,
    /// Ось и скорость кувыркания, рад/с.
    spin: Vec3,
    bounces: u8,
    /// Гильза из пулемёта лунохода: её звон слышен.
    player: bool,
    /// Сколько уже лежит, с; `None` — ещё летит.
    resting: Option<f32>,
}

/// Гильзы забега по времени появления и генератор разброса.
#[derive(Resource)]
struct Casings {
    queue: VecDeque<Entity>,
    /// Состояние генератора разброса: гильзы — украшение, игровой `GameRng` им не нужен.
    rng: u32,
    /// Сколько прошло с последнего звона, с.
    clink: f32,
}

impl Default for Casings {
    fn default() -> Self {
        Self {
            queue: VecDeque::new(),
            rng: 0x9E37_79B9,
            clink: 0.0,
        }
    }
}

impl Casings {
    /// Случайное число от `lo` до `hi`.
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        // xorshift32
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        lo + (hi - lo) * (x >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Забывает гильзу, удалённую не по пределу.
    fn forget(&mut self, entity: Entity) {
        if let Some(i) = self.queue.iter().position(|&e| e == entity) {
            self.queue.remove(i);
        }
    }
}

/// Общий меш и латунный материал гильз.
#[derive(Resource)]
struct CasingAssets {
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

fn create_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(CasingAssets {
        mesh: meshes.add(Cylinder::new(RADIUS, LENGTH)),
        material: materials.add(StandardMaterial {
            base_color: Color::srgb(0.85, 0.62, 0.22),
            metallic: 1.0,
            perceptual_roughness: 0.25,
            // Чуть светится, чтобы блестела и в тени.
            emissive: LinearRgba::rgb(0.25, 0.16, 0.04),
            ..default()
        }),
    });
}

/// Новый мир: гильзы с `RunScoped` уже удалены, очередь пуста.
fn reset_casings(mut rebuilt: MessageReader<WorldRebuilt>, mut casings: ResMut<Casings>) {
    if rebuilt.read().last().is_some() {
        casings.queue.clear();
    }
}

/// На каждый выстрел пулемёта — гильза вбок от ствола.
fn eject_casings(
    mut commands: Commands,
    mut shots: MessageReader<FireBullet>,
    mut casings: ResMut<Casings>,
    assets: Res<CasingAssets>,
) {
    for shot in shots.read() {
        let forward = shot.direction.with_y(0.0).normalize_or(Vec3::NEG_Z);
        // Окно выброса — справа по ходу ствола.
        let side = forward.cross(Vec3::Y);
        let velocity = side * casings.range(SIDE_SPEED.0, SIDE_SPEED.1)
            + Vec3::Y * casings.range(UP_SPEED.0, UP_SPEED.1)
            + forward * casings.range(-0.8, 0.3);
        let spin = Vec3::new(casings.range(-1.0, 1.0), casings.range(-1.0, 1.0), casings.range(-1.0, 1.0))
            .normalize_or(Vec3::X)
            * SPIN;
        let entity = commands
            .spawn((
                Casing {
                    velocity,
                    spin,
                    bounces: 0,
                    player: shot.team == Team::Player,
                    resting: None,
                },
                RunScoped,
                NotShadowCaster,
                Mesh3d(assets.mesh.clone()),
                MeshMaterial3d(assets.material.clone()),
                Transform::from_translation(shot.origin + side * 0.2),
            ))
            .id();
        casings.queue.push_back(entity);
        while casings.queue.len() > MAX_CASINGS {
            if let Some(old) = casings.queue.pop_front() {
                commands.entity(old).try_despawn();
            }
        }
    }
}

/// Полёт: гравитация, кувыркание, отскок от рельефа, укладка набок по склону.
fn fly_casings(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    pools: Res<LavaPools>,
    puddles: Res<Puddles>,
    mut casings: ResMut<Casings>,
    mut flying: Query<(Entity, &mut Transform, &mut Casing)>,
    mut clinks: MessageWriter<CasingClink>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    casings.clink += dt;
    for (entity, mut transform, mut casing) in &mut flying {
        if casing.resting.is_some() {
            continue;
        }
        casing.velocity.y -= GRAVITY * dt;
        transform.translation += casing.velocity * dt;
        let turn = Quat::from_scaled_axis(casing.spin * dt);
        transform.rotation = turn * transform.rotation;
        let p = transform.translation;
        let ground = terrain.height_at(p.x, p.z) + RADIUS;
        if p.y > ground {
            continue;
        }
        if lava::is_lava(&pools, p) || puddles.at(p.xz()) {
            commands.entity(entity).try_despawn();
            casings.forget(entity);
            continue;
        }
        if casing.player && casings.clink >= CLINK_PERIOD {
            casings.clink = 0.0;
            clinks.write(CasingClink { position: p });
        }
        let normal = terrain.normal_at(p.x, p.z);
        casing.bounces += 1;
        transform.translation.y = ground;
        let bounced = (casing.velocity - 2.0 * casing.velocity.dot(normal) * normal) * BOUNCE_KEEP;
        if casing.bounces >= MAX_BOUNCES || bounced.length() < REST_SPEED {
            // Набок: ось цилиндра вдоль склона, поворот вокруг нормали случайный.
            let yaw = casings.range(0.0, std::f32::consts::TAU);
            transform.rotation =
                Quat::from_rotation_arc(Vec3::Y, normal) * Quat::from_rotation_y(yaw) * Quat::from_rotation_z(FRAC_PI_2);
            casing.resting = Some(0.0);
        } else {
            casing.velocity = bounced;
            casing.spin *= 0.6;
        }
    }
}

/// Лежащие гильзы через [`REST_TIME`] исчезают, а кратер убирает гильзы в своём круге.
fn settle_casings(
    mut commands: Commands,
    time: Res<Time>,
    mut changes: MessageReader<TerrainChanged>,
    mut casings: ResMut<Casings>,
    mut all: Query<(Entity, &mut Transform, &mut Casing)>,
) {
    let dt = time.delta_secs();
    let changes: Vec<_> = changes.read().copied().collect();
    for (entity, mut transform, mut casing) in &mut all {
        let p = transform.translation;
        let in_crater = changes
            .iter()
            .any(|c| c.center.xz().distance_squared(p.xz()) < c.radius * c.radius);
        let mut gone = in_crater;
        if let Some(age) = casing.resting.as_mut() {
            *age += dt;
            let shrink = (*age - REST_TIME) / SHRINK_TIME;
            if shrink >= 1.0 {
                gone = true;
            } else if shrink > 0.0 {
                transform.scale = Vec3::splat(1.0 - shrink);
            }
        }
        if gone {
            commands.entity(entity).try_despawn();
            casings.forget(entity);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn гильз_не_больше_предела_после_1000_выстрелов() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<FireBullet>()
            .init_resource::<Casings>()
            .insert_resource(CasingAssets {
                mesh: Handle::default(),
                material: Handle::default(),
            })
            .add_systems(Update, eject_casings);
        for frame in 0..10 {
            for i in 0..100 {
                app.world_mut().write_message(FireBullet {
                    origin: Vec3::new(i as f32, 5.0, frame as f32),
                    direction: Vec3::NEG_Z,
                    team: Team::Player,
                    damage: 1.0,
                    heavy: false,
                });
            }
            app.update();
        }
        let count = app.world_mut().query::<&Casing>().iter(app.world()).count();
        assert_eq!(count, MAX_CASINGS);
        assert_eq!(app.world().resource::<Casings>().queue.len(), MAX_CASINGS);
    }

    #[test]
    fn разброс_в_пределах() {
        let mut casings = Casings::default();
        for _ in 0..1000 {
            let v = casings.range(3.0, 5.0);
            assert!((3.0..5.0).contains(&v));
        }
    }
}
