//! Ремонт и аптечки: два способа вернуть здоровье.
//!
//! Ремонт идёт, пока зажат пробел, и замедляет луноход. Аптечки появляются вокруг лунохода
//! и подбираются проездом, если здоровье неполное.

use std::f32::consts::TAU;

use bevy::prelude::*;

use crate::combat::{Heal, Health, ROVER_MAX_HEALTH};
use crate::frame::{GameSet, RunMessages};
use crate::models::Models;
use crate::rng::GameRng;
use crate::rover::{self, Rover, SpeedFactor};
use crate::run::{RunScoped, WorldRebuilt};
use crate::terrain::Terrain;

/// Сколько здоровья в секунду возвращает ремонт: доля полного здоровья. Ремонт 10 в секунду
/// и аптечка 35 играли при здоровье 300, как и урон врагов в `waves::enemy_damage` и удар поезда:
/// доли сохраняют пропорцию того баланса.
const REPAIR_RATE: f32 = ROVER_MAX_HEALTH * (10.0 / 300.0);

/// Во сколько раз луноход медленнее во время ремонта.
const REPAIR_SLOWDOWN: f32 = 0.4;

/// Искр ремонта в секунду.
const SPARK_RATE: f32 = 24.0;

/// Сколько секунд живёт искра.
const SPARK_LIFE: f32 = 0.45;

/// Как часто появляется аптечка, с.
const MEDKIT_INTERVAL: f32 = 20.0;

/// Сколько аптечек может лежать рядом с луноходом.
const MAX_MEDKITS: usize = 2;

/// Аптечки ближе этого к луноходу входят в предел [`MAX_MEDKITS`], м.
const MEDKIT_COUNT_RADIUS: f32 = 70.0;

/// Аптечка дальше этого от лунохода убирается, м.
const MEDKIT_FORGET_RADIUS: f32 = 150.0;

/// Кольцо появления аптечек вокруг лунохода, м.
const MEDKIT_RING: (f32, f32) = (15.0, 30.0);

/// Сколько здоровья даёт аптечка: до полного (лечение выше максимума не поднимает).
pub const MEDKIT_HEAL: f32 = ROVER_MAX_HEALTH;

/// С какого расстояния аптечка подбирается, м.
const PICKUP_DISTANCE: f32 = 2.8;

/// Размер аптечки, м: заметна рядом с луноходом (3,6 × 3,0 м).
const MEDKIT_SIZE: f32 = 1.6;

/// Высота центра аптечки над поверхностью, м: чуть парит.
const MEDKIT_HEIGHT: f32 = MEDKIT_SIZE * 0.5 + 0.25;

/// Подключает ремонт и аптечки.
pub struct RepairPlugin;

impl Plugin for RepairPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MedkitTimer>()
            .add_run_message::<RepairChanged>()
            .add_run_message::<MedkitPicked>()
            .add_systems(Startup, create_assets)
            .add_systems(
                Update,
                (
                    reset_repair.in_set(GameSet::Reset),
                    repair.before(rover::drive_rover).in_set(GameSet::Input),
                    (spawn_sparks, spawn_medkits, pick_medkits).chain().in_set(GameSet::Simulation),
                    (move_sparks, spin_medkits).in_set(GameSet::Presentation),
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
pub struct Medkit;

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
        box_mesh: meshes.add(Cuboid::new(MEDKIT_SIZE, MEDKIT_SIZE, MEDKIT_SIZE)),
        cross_mesh: meshes.add(Cuboid::new(MEDKIT_SIZE * 0.78, MEDKIT_SIZE * 0.24, MEDKIT_SIZE * 1.07)),
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
///
/// Здоровье прибавляет сообщение [`Heal`] позже в этом кадре, поэтому конец ремонта
/// считается по здоровью после лечения.
fn repair(
    mut commands: Commands,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    rover: Query<(Entity, &Health, Has<Repairing>), With<Rover>>,
    mut changed: MessageWriter<RepairChanged>,
    mut heals: MessageWriter<Heal>,
) {
    let Some((entity, health, was_repairing)) = rover.iter().next() else {
        return;
    };
    let repairing = keys.pressed(KeyCode::Space) && can_repair(health);
    if repairing {
        let amount = REPAIR_RATE * time.delta_secs();
        heals.write(Heal { target: entity, amount });
        if !was_repairing {
            commands.entity(entity).try_insert((Repairing, SpeedFactor(REPAIR_SLOWDOWN)));
            changed.write(RepairChanged::Started);
        }
        if health.current + amount >= health.max {
            changed.write(RepairChanged::Done);
        }
    } else if was_repairing {
        commands.entity(entity).try_remove::<(Repairing, SpeedFactor)>();
    }
}

/// Пока идёт ремонт, из-под корпуса летят искры.
fn spawn_sparks(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<RepairAssets>,
    mut dice: ResMut<GameRng>,
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

/// Мир перестроен: отсчёт до первой аптечки начинается заново, луноход не чинится. Ремонт,
/// зажатый в конце прошлого забега, в новый не переходит.
fn reset_repair(
    mut commands: Commands,
    mut rebuilt: MessageReader<WorldRebuilt>,
    mut timer: ResMut<MedkitTimer>,
    repairing: Query<Entity, With<Repairing>>,
) {
    if rebuilt.read().count() > 0 {
        *timer = MedkitTimer::default();
        for entity in &repairing {
            commands.entity(entity).try_remove::<(Repairing, SpeedFactor)>();
        }
    }
}

/// Раз в несколько секунд кладёт аптечку в случайной точке кольца вокруг лунохода.
fn spawn_medkits(
    mut commands: Commands,
    time: Res<Time>,
    mut timer: ResMut<MedkitTimer>,
    mut dice: ResMut<GameRng>,
    assets: Res<RepairAssets>,
    mut models: Models,
    terrain: Res<Terrain>,
    rover: Query<(&Transform, &Health), With<Rover>>,
    medkits: Query<(Entity, &Transform), (With<Medkit>, Without<Rover>)>,
) {
    let Some((rover, health)) = rover.iter().next() else {
        return;
    };
    if health.is_dead() {
        return;
    }
    // Далёкие аптечки убираются, в предел входят только ближние.
    let mut near = 0;
    for (entity, medkit) in &medkits {
        let distance = medkit.translation.with_y(0.0).distance(rover.translation.with_y(0.0));
        if distance > MEDKIT_FORGET_RADIUS {
            commands.entity(entity).despawn();
        } else if distance <= MEDKIT_COUNT_RADIUS {
            near += 1;
        }
    }
    timer.0 -= time.delta_secs();
    if timer.0 > 0.0 {
        return;
    }
    timer.0 = MEDKIT_INTERVAL;
    if near >= MAX_MEDKITS {
        return;
    }

    let angle = dice.range(0.0, TAU);
    let distance = dice.range(MEDKIT_RING.0, MEDKIT_RING.1);
    let mut position = rover.translation + Vec3::new(angle.cos(), 0.0, angle.sin()) * distance;
    position.y = terrain.height_at(position.x, position.z) + MEDKIT_HEIGHT;
    commands
        .spawn((Medkit, RunScoped, Transform::from_translation(position), Visibility::default()))
        .with_children(|parent| {
            // Модель стоит на дне куба аптечки; без неё — белый куб с крестом из двух брусков,
            // видимым сверху и с боков.
            let ground = Transform::from_xyz(0.0, -MEDKIT_SIZE * 0.5, 0.0);
            models.spawn_visual(parent, "pickup_medkit", ground, |spare| {
                spare.spawn((Mesh3d(assets.box_mesh.clone()), MeshMaterial3d(assets.white.clone())));
                for turn in [0.0, std::f32::consts::FRAC_PI_2] {
                    spare.spawn((
                        Mesh3d(assets.cross_mesh.clone()),
                        MeshMaterial3d(assets.red.clone()),
                        Transform::from_rotation(Quat::from_rotation_z(turn)),
                    ));
                    spare.spawn((
                        Mesh3d(assets.cross_mesh.clone()),
                        MeshMaterial3d(assets.red.clone()),
                        Transform::from_rotation(
                            Quat::from_rotation_x(std::f32::consts::FRAC_PI_2) * Quat::from_rotation_z(turn),
                        ),
                    ));
                }
            });
        });
}

/// Подбирается ли аптечка: луноход рядом, жив и ранен.
fn can_pick(distance: f32, health: &Health) -> bool {
    distance < PICKUP_DISTANCE && can_repair(health)
}

/// Подбор проездом. При полном здоровье аптечка остаётся лежать. За кадр подбирается одна:
/// лечение применится позже в кадре, и вторая аптечка рядом не должна уйти на полное здоровье.
fn pick_medkits(
    mut commands: Commands,
    rover: Query<(Entity, &Transform, &Health), With<Rover>>,
    medkits: Query<(Entity, &Transform), With<Medkit>>,
    mut picked: MessageWriter<MedkitPicked>,
    mut heals: MessageWriter<Heal>,
) {
    let Some((rover_entity, rover, health)) = rover.iter().next() else {
        return;
    };
    for (entity, medkit) in &medkits {
        let mut offset = medkit.translation - rover.translation;
        offset.y = 0.0;
        if can_pick(offset.length(), health) {
            heals.write(Heal {
                target: rover_entity,
                amount: MEDKIT_HEAL,
            });
            commands.entity(entity).despawn();
            picked.write(MedkitPicked {
                position: medkit.translation,
            });
            break;
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
    }

    /// Ремонт — та же доля здоровья, что 10 в секунду при здоровье 300; аптечка лечит до полного.
    #[test]
    fn healing_is_a_share_of_full_health() {
        assert!((REPAIR_RATE / ROVER_MAX_HEALTH - 10.0 / 300.0).abs() < 1e-6);
        assert_eq!(MEDKIT_HEAL, ROVER_MAX_HEALTH);
    }

    #[test]
    fn holding_space_heals_and_slows_then_stops_at_full_health() {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Events>()
            .add_message::<RepairChanged>()
            .add_message::<Heal>()
            .add_systems(Update, (repair, crate::combat::apply_heal, collect).chain());
        // Ремонт за полсекунды возвращает 1/60 здоровья: ранен на 4,5 % — три шага до полного.
        let mut health = Health::new(ROVER_MAX_HEALTH);
        health.damage(0.045 * ROVER_MAX_HEALTH);
        let rover = app.world_mut().spawn((Rover, health)).id();
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Space);

        let step = |app: &mut App| {
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(std::time::Duration::from_millis(500));
            app.update();
        };
        let current = |app: &App| app.world().get::<Health>(rover).unwrap().current;
        step(&mut app);
        assert!((current(&app) - (0.955 + 1.0 / 60.0) * ROVER_MAX_HEALTH).abs() < 1e-2, "{}", current(&app));
        assert_eq!(app.world().get::<SpeedFactor>(rover).unwrap().0, REPAIR_SLOWDOWN);

        step(&mut app);
        step(&mut app);
        assert_eq!(current(&app), ROVER_MAX_HEALTH);
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
