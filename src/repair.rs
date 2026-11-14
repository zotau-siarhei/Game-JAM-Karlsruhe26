//! Ремонт и аптечки: два способа вернуть здоровье.
//!
//! Ремонт идёт, пока зажат пробел, и замедляет луноход. Аптечки появляются вокруг лунохода
//! и подбираются проездом, если здоровье неполное.

use std::f32::consts::TAU;

use bevy::prelude::*;

use crate::combat::Health;
use crate::rover::{self, Rover, SpeedFactor};
use crate::run::{GameState, RunScoped, RunStarted};
use crate::sound_list::Dice;
use crate::terrain::Terrain;

/// Сколько здоровья в секунду возвращает ремонт.
const REPAIR_RATE: f32 = 10.0;

/// Во сколько раз луноход медленнее во время ремонта.
const REPAIR_SLOWDOWN: f32 = 0.4;

/// Искр ремонта в секунду.
const SPARK_RATE: f32 = 24.0;

/// Сколько секунд живёт искра.
const SPARK_LIFE: f32 = 0.45;

/// Как часто появляется аптечка, с.
const MEDKIT_INTERVAL: f32 = 20.0;

/// Сколько аптечек может лежать одновременно.
const MAX_MEDKITS: usize = 2;

/// Кольцо появления аптечек вокруг лунохода, м.
const MEDKIT_RING: (f32, f32) = (15.0, 30.0);

/// Сколько здоровья даёт аптечка.
pub const MEDKIT_HEAL: f32 = 35.0;

/// С какого расстояния аптечка подбирается, м.
const PICKUP_DISTANCE: f32 = 2.0;

/// Высота аптечки над поверхностью, м.
const MEDKIT_HEIGHT: f32 = 0.7;

/// Подключает ремонт и аптечки.
pub struct RepairPlugin;

impl Plugin for RepairPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Dice>()
            .init_resource::<MedkitTimer>()
            .add_message::<RepairChanged>()
            .add_message::<MedkitPicked>()
            .add_systems(Startup, create_assets)
            .add_systems(
                Update,
                (
                    repair.before(rover::drive_rover).run_if(in_state(GameState::Playing)),
                    (spawn_sparks, move_sparks, reset_medkits, spawn_medkits.run_if(in_state(GameState::Playing)), pick_medkits, spin_medkits)
                        .chain()
                        .after(rover::drive_rover),
                ),
            );
    }
}

/// Ремонт начался или закончился.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairChanged {
    /// Игрок зажал пробел, здоровье начало расти.
    Started,
    /// Здоровье дошло до полного.
    Done,
}

/// Луноход подобрал аптечку.
#[derive(Message, Debug, Clone, Copy)]
pub struct MedkitPicked {
    /// Где лежала аптечка.
    pub position: Vec3,
}

/// Луноход сейчас ремонтируется.
#[derive(Component)]
struct Repairing;

/// Аптечка на поверхности.
#[derive(Component)]
struct Medkit;

/// Искра ремонта: летит по баллистике и гаснет.
#[derive(Component)]
struct Spark {
    velocity: Vec3,
    life: f32,
}

/// Время до появления следующей аптечки, с.
#[derive(Resource)]
struct MedkitTimer(f32);

impl Default for MedkitTimer {
    fn default() -> Self {
        Self(MEDKIT_INTERVAL)
    }
}

/// Общие меши и материалы аптечек и искр.
#[derive(Resource)]
struct RepairAssets {
    box_mesh: Handle<Mesh>,
    cross_mesh: Handle<Mesh>,
    spark_mesh: Handle<Mesh>,
    white: Handle<StandardMaterial>,
    red: Handle<StandardMaterial>,
    spark: Handle<StandardMaterial>,
}

fn create_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(RepairAssets {
        box_mesh: meshes.add(Cuboid::new(0.9, 0.9, 0.9)),
        cross_mesh: meshes.add(Cuboid::new(0.7, 0.22, 0.96)),
        spark_mesh: meshes.add(Cuboid::new(0.08, 0.08, 0.08)),
        white: materials.add(StandardMaterial {
            base_color: Color::WHITE,
            emissive: LinearRgba::rgb(0.6, 0.6, 0.6),
            ..default()
        }),
        red: materials.add(StandardMaterial {
            base_color: Color::srgb(0.9, 0.1, 0.1),
            emissive: LinearRgba::rgb(1.5, 0.1, 0.1),
            ..default()
        }),
        spark: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.85, 0.3),
            emissive: LinearRgba::rgb(8.0, 5.0, 1.0),
            unlit: true,
            ..default()
        }),
    });
}

/// Можно ли чинить: луноход жив и здоровье неполное.
fn can_repair(health: &Health) -> bool {
    !health.is_dead() && health.current < health.max
}

/// Ремонт: пока зажат пробел, здоровье растёт, а луноход едет медленнее.
fn repair(
    mut commands: Commands,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut rover: Query<(Entity, &mut Health, Has<Repairing>), With<Rover>>,
    mut changed: MessageWriter<RepairChanged>,
) {
    let Some((entity, mut health, was_repairing)) = rover.iter_mut().next() else {
        return;
    };
    let repairing = keys.pressed(KeyCode::Space) && can_repair(&health);
    if repairing {
        health.heal(REPAIR_RATE * time.delta_secs());
        if !was_repairing {
            commands.entity(entity).insert((Repairing, SpeedFactor(REPAIR_SLOWDOWN)));
            changed.write(RepairChanged::Started);
        }
        if !can_repair(&health) {
            changed.write(RepairChanged::Done);
        }
    } else if was_repairing {
        commands.entity(entity).remove::<(Repairing, SpeedFactor)>();
    }
}

/// Пока идёт ремонт, из-под корпуса летят искры.
fn spawn_sparks(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<RepairAssets>,
    mut dice: ResMut<Dice>,
    mut debt: Local<f32>,
    rover: Query<&Transform, (With<Rover>, With<Repairing>)>,
) {
    let Some(rover) = rover.iter().next() else {
        *debt = 0.0;
        return;
    };
    *debt += SPARK_RATE * time.delta_secs();
    while *debt >= 1.0 {
        *debt -= 1.0;
        let angle = dice.range(0.0, TAU);
        let side = Vec3::new(angle.cos(), 0.0, angle.sin());
        commands.spawn((
            RunScoped,
            Spark {
                velocity: side * dice.range(1.5, 4.0) + Vec3::Y * dice.range(3.0, 6.0),
                life: SPARK_LIFE,
            },
            Mesh3d(assets.spark_mesh.clone()),
            MeshMaterial3d(assets.spark.clone()),
            Transform::from_translation(rover.translation + side * 0.7 + Vec3::Y * 0.2),
        ));
    }
}

fn move_sparks(
    mut commands: Commands,
    time: Res<Time>,
    mut sparks: Query<(Entity, &mut Transform, &mut Spark)>,
) {
    let dt = time.delta_secs();
    for (entity, mut transform, mut spark) in &mut sparks {
        spark.life -= dt;
        if spark.life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        spark.velocity.y -= 18.0 * dt;
        transform.translation += spark.velocity * dt;
    }
}

/// Новый забег: отсчёт до первой аптечки начинается заново.
fn reset_medkits(mut started: MessageReader<RunStarted>, mut timer: ResMut<MedkitTimer>) {
    if started.read().count() > 0 {
        *timer = MedkitTimer::default();
    }
}

/// Раз в несколько секунд кладёт аптечку в случайной точке кольца вокруг лунохода.
fn spawn_medkits(
    mut commands: Commands,
    time: Res<Time>,
    mut timer: ResMut<MedkitTimer>,
    mut dice: ResMut<Dice>,
    assets: Res<RepairAssets>,
    terrain: Res<Terrain>,
    rover: Query<(&Transform, &Health), With<Rover>>,
    medkits: Query<(), With<Medkit>>,
) {
    let Some((rover, health)) = rover.iter().next() else {
        return;
    };
    if health.is_dead() {
        return;
    }
    timer.0 -= time.delta_secs();
    if timer.0 > 0.0 {
        return;
    }
    timer.0 = MEDKIT_INTERVAL;
    if medkits.iter().count() >= MAX_MEDKITS {
        return;
    }

    let angle = dice.range(0.0, TAU);
    let distance = dice.range(MEDKIT_RING.0, MEDKIT_RING.1);
    let mut position = rover.translation + Vec3::new(angle.cos(), 0.0, angle.sin()) * distance;
    position.y = terrain.height_at(position.x, position.z) + MEDKIT_HEIGHT;
    commands
        .spawn((
            Medkit,
            RunScoped,
            Mesh3d(assets.box_mesh.clone()),
            MeshMaterial3d(assets.white.clone()),
            Transform::from_translation(position),
        ))
        .with_children(|parent| {
            // Красный крест из двух брусков, видимый сверху и с боков.
            for turn in [0.0, std::f32::consts::FRAC_PI_2] {
                parent.spawn((
                    Mesh3d(assets.cross_mesh.clone()),
                    MeshMaterial3d(assets.red.clone()),
                    Transform::from_rotation(Quat::from_rotation_z(turn)),
                ));
                parent.spawn((
                    Mesh3d(assets.cross_mesh.clone()),
                    MeshMaterial3d(assets.red.clone()),
                    Transform::from_rotation(
                        Quat::from_rotation_x(std::f32::consts::FRAC_PI_2) * Quat::from_rotation_z(turn),
                    ),
                ));
            }
        });
}

/// Подбирается ли аптечка: луноход рядом, жив и ранен.
fn can_pick(distance: f32, health: &Health) -> bool {
    distance < PICKUP_DISTANCE && can_repair(health)
}

/// Подбор проездом. При полном здоровье аптечка остаётся лежать.
fn pick_medkits(
    mut commands: Commands,
    mut rover: Query<(&Transform, &mut Health), With<Rover>>,
    medkits: Query<(Entity, &Transform), With<Medkit>>,
    mut picked: MessageWriter<MedkitPicked>,
) {
    let Some((rover, mut health)) = rover.iter_mut().next() else {
        return;
    };
    for (entity, medkit) in &medkits {
        let mut offset = medkit.translation - rover.translation;
        offset.y = 0.0;
        if can_pick(offset.length(), &health) {
            health.heal(MEDKIT_HEAL);
            commands.entity(entity).despawn();
            picked.write(MedkitPicked {
                position: medkit.translation,
            });
        }
    }
}

fn spin_medkits(time: Res<Time>, mut medkits: Query<&mut Transform, With<Medkit>>) {
    for mut transform in &mut medkits {
        transform.rotate_y(1.2 * time.delta_secs());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repair_needs_a_living_damaged_rover() {
        let mut health = Health::new(100.0);
        assert!(!can_repair(&health));
        health.damage(40.0);
        assert!(can_repair(&health));
        health.damage(100.0);
        assert!(!can_repair(&health));
    }

    #[test]
    fn medkit_is_picked_only_when_close_and_damaged() {
        let mut health = Health::new(100.0);
        // Полное здоровье: аптечка остаётся.
        assert!(!can_pick(0.5, &health));
        health.damage(50.0);
        assert!(can_pick(0.5, &health));
        assert!(!can_pick(PICKUP_DISTANCE + 0.1, &health));
        health.heal(MEDKIT_HEAL);
        assert_eq!(health.current, 85.0);
    }

    #[test]
    fn holding_space_heals_and_slows_then_stops_at_full_health() {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Events>()
            .add_message::<RepairChanged>()
            .add_systems(Update, (repair, collect).chain());
        let mut health = Health::new(100.0);
        health.damage(15.0);
        let rover = app.world_mut().spawn((Rover, health)).id();
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Space);

        let step = |app: &mut App| {
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(std::time::Duration::from_millis(500));
            app.update();
        };
        step(&mut app);
        assert_eq!(app.world().get::<Health>(rover).unwrap().current, 90.0);
        assert_eq!(app.world().get::<SpeedFactor>(rover).unwrap().0, REPAIR_SLOWDOWN);

        step(&mut app);
        step(&mut app);
        assert_eq!(app.world().get::<Health>(rover).unwrap().current, 100.0);
        // Здоровье полное: на следующем кадре ремонт и замедление сняты.
        step(&mut app);
        assert!(app.world().get::<SpeedFactor>(rover).is_none());
        assert_eq!(
            app.world().resource::<Events>().0,
            vec![RepairChanged::Started, RepairChanged::Done]
        );
    }

    #[derive(Resource, Default)]
    struct Events(Vec<RepairChanged>);

    fn collect(mut changed: MessageReader<RepairChanged>, mut events: ResMut<Events>) {
        events.0.extend(changed.read().copied());
    }
}
