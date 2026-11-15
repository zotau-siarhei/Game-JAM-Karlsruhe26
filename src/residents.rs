//! Жители посёлков: бродят и болтают, пугаются взрывов и машин, прячутся в домах, гибнут
//! с мультяшным «пуф».
//!
//! Позиции жителей не хранятся: дом помнит, сколько жителей к нему приписано и сколько прячется
//! внутри. Когда дом появляется в сцене, рядом с ним создаются его жители, кроме прячущихся.
//! Паника передаётся соседям: паникующий житель сам пугает тех, кто рядом.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::buildings::House;
use crate::bullet::FireBullet;
use crate::combat::{Collider, Damage, Died, Health, Team};
use crate::enemy::Enemy;
use crate::explosion::Explosion;
use crate::frame::{GameSet, RunMessages};
use crate::models::Models;
use crate::mortar::FireShell;
use crate::obstacles::Obstacle;
use crate::rng::GameRng;
use crate::rover::{Rover, RoverMotion};
use crate::run::{RunScoped, WorldRebuilt};
use crate::settlements::{HouseId, HouseStatus, Settlements};
use crate::sound_list::Dice;
use crate::terrain::Terrain;
use crate::vehicle::GroundVehicle;

/// Скорость прогулки и бега, м/с.
const WALK_SPEED: f32 = 1.6;
const RUN_SPEED: f32 = 4.5;
/// Житель бродит в этом радиусе от своего дома, м.
const WANDER_RADIUS: f32 = 12.0;
/// Укрытие ищется в этом радиусе, м.
const HIDE_RADIUS: f32 = 30.0;
/// Бегство от опасности на столько, м.
const FLEE_DISTANCE: f32 = 25.0;
/// Радиусы испуга: взрыв, отметка снаряда, выстрел, вражеская машина, быстрый луноход, паникующий сосед.
const FEAR_EXPLOSION: f32 = 35.0;
const FEAR_SHELL: f32 = 25.0;
const FEAR_SHOT: f32 = 15.0;
const FEAR_ENEMY: f32 = 12.0;
const FEAR_ROVER: f32 = 10.0;
const FEAR_NEIGHBOUR: f32 = 8.0;
/// Луноход пугает только быстрее этого, м/с.
const SCARY_SPEED: f32 = 8.0;
/// Сколько паникующий житель сам пугает соседей, с.
const CONTAGION_TIME: f32 = 2.0;
/// Сколько длится паника без новых поводов, с.
const PANIC_TIME: f32 = 6.0;
/// Сколько затишья нужно, чтобы прячущиеся вышли, с, и пауза между выходами, с.
const CALM_TIME: f32 = 20.0;
const EXIT_GAP: f32 = 0.7;
/// Машина быстрее этого сбивает жителя, медленнее — житель отскакивает, м/с.
const LETHAL_SPEED: f32 = 9.0;
const DODGE_SPEED: f32 = 3.0;

/// Цвета обычной одежды: джинсовый, зелёный свитер, горчичный, серый, светлая рубашка,
/// коричневая куртка.
const CLOTHES_COLORS: [Color; 6] = [
    Color::srgb(0.27, 0.42, 0.62),
    Color::srgb(0.33, 0.50, 0.33),
    Color::srgb(0.80, 0.66, 0.30),
    Color::srgb(0.55, 0.56, 0.58),
    Color::srgb(0.90, 0.90, 0.86),
    Color::srgb(0.48, 0.34, 0.24),
];

/// Цвета кожи.
const SKIN_COLORS: [Color; 3] = [
    Color::srgb(0.96, 0.80, 0.68),
    Color::srgb(0.82, 0.62, 0.46),
    Color::srgb(0.55, 0.38, 0.27),
];

/// Цвета волос: чёрные, каштановые, русые, седые.
const HAIR_COLORS: [Color; 4] = [
    Color::srgb(0.12, 0.09, 0.07),
    Color::srgb(0.40, 0.26, 0.14),
    Color::srgb(0.80, 0.66, 0.38),
    Color::srgb(0.66, 0.66, 0.64),
];

/// Идентификатор модели жителя в списке моделей. Пока модели нет, житель собран из примитивов.
const RESIDENT_MODEL: &str = "resident";

/// Плагин жителей.
pub struct ResidentsPlugin;

impl Plugin for ResidentsPlugin {
    fn build(&self, app: &mut App) {
        app.add_run_message::<ResidentDied>()
            .init_resource::<Danger>()
            .add_systems(Startup, create_assets)
            .add_systems(
                Update,
                (
                    reset_danger.in_set(GameSet::Reset),
                    // Походка считается от высоты, которую ставит `walk`: только вместе с ним.
                    (
                        spawn_residents,
                        gather_danger,
                        react_to_danger,
                        think,
                        walk,
                        dodge_vehicles,
                        animate,
                        leave_shelters,
                    )
                        .chain()
                        .in_set(GameSet::Simulation),
                    (resident_deaths, hidden_die_with_house).chain().in_set(GameSet::Death),
                    remove_far_residents.in_set(GameSet::Cleanup),
                    (window_eyes, fade_puffs).in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Житель погиб: на это подпишется тревога.
#[derive(Message, Debug, Clone, Copy)]
pub struct ResidentDied {
    #[allow(dead_code)]
    pub position: Vec3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mood {
    /// Идёт к точке у дома или стоит в паузе.
    Wander,
    /// Болтает с соседом: осталось секунд.
    Chat(f32),
    /// Бежит прятаться или от опасности: осталось секунд паники.
    Panic(f32),
    /// Отскакивает вбок от машины: осталось секунд.
    Dodge(f32),
}

/// Житель.
#[derive(Component)]
pub struct Resident {
    home: HouseId,
    mood: Mood,
    target: Vec2,
    pause: f32,
    /// Куда прятаться: дом и его сущность.
    shelter: Option<(HouseId, Entity)>,
    /// Пока больше нуля, житель сам пугает соседей.
    contagious: f32,
    stuck: f32,
    phase: f32,
    velocity: Vec2,
}

/// Рука: сторона, 1 или -1.
#[derive(Component)]
struct Arm(f32);

/// Восклицательный знак над головой.
#[derive(Component)]
struct Alert;

/// Облачко «пуф».
#[derive(Component)]
struct Puff {
    velocity: Vec3,
    age: f32,
}

/// Глаза в окнах дома, где прячутся жители.
#[derive(Component)]
struct WindowEyes;

/// Точки опасности за кадр: центр и радиус.
#[derive(Resource, Default)]
struct Danger {
    points: Vec<(Vec2, f32)>,
    /// Сколько времени прошло без опасности рядом с каждым домом-укрытием: дом и затишье, с.
    calm: Vec<(HouseId, f32, f32)>,
}

#[derive(Resource)]
struct ResidentAssets {
    body: Handle<Mesh>,
    head: Handle<Mesh>,
    arm: Handle<Mesh>,
    ball: Handle<Mesh>,
    cube: Handle<Mesh>,
    hat: Handle<Mesh>,
    /// Одежда: тело и руки.
    clothes: Vec<Handle<StandardMaterial>>,
    skin: Vec<Handle<StandardMaterial>>,
    hair: Vec<Handle<StandardMaterial>>,
    white: Handle<StandardMaterial>,
    black: Handle<StandardMaterial>,
    alert: Handle<StandardMaterial>,
    puff: Handle<StandardMaterial>,
}

fn create_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let unlit = |m: &mut Assets<StandardMaterial>, c: Color| {
        m.add(StandardMaterial {
            base_color: c,
            unlit: true,
            ..default()
        })
    };
    commands.insert_resource(ResidentAssets {
        body: meshes.add(Capsule3d::new(0.22, 0.3)),
        head: meshes.add(Sphere::new(0.2).mesh().ico(2).expect("голова жителя")),
        arm: meshes.add(Capsule3d::new(0.06, 0.25)),
        ball: meshes.add(Sphere::new(1.0).mesh().ico(1).expect("шарик")),
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        hat: meshes.add(Cylinder::new(0.17, 0.18)),
        clothes: CLOTHES_COLORS.iter().map(|c| materials.add(*c)).collect(),
        skin: SKIN_COLORS.iter().map(|c| materials.add(*c)).collect(),
        hair: HAIR_COLORS.iter().map(|c| materials.add(*c)).collect(),
        white: unlit(&mut materials, Color::WHITE),
        black: unlit(&mut materials, Color::srgb(0.05, 0.05, 0.05)),
        alert: unlit(&mut materials, Color::srgb(1.0, 0.85, 0.1)),
        puff: materials.add(StandardMaterial {
            base_color: Color::srgba(1.0, 1.0, 1.0, 0.9),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            ..default()
        }),
    });
}

/// Мир перестроен: дома-укрытия прошлого мира забываются.
fn reset_danger(mut rebuilt: MessageReader<WorldRebuilt>, mut danger: ResMut<Danger>) {
    if rebuilt.read().last().is_some() {
        *danger = Danger::default();
    }
}

/// Когда дом появляется в сцене, рядом с ним выходят его жители, кроме прячущихся.
fn spawn_residents(
    mut commands: Commands,
    assets: Res<ResidentAssets>,
    mut models: Models,
    terrain: Res<Terrain>,
    settlements: Res<Settlements>,
    mut dice: ResMut<GameRng>,
    houses: Query<&House, Added<House>>,
) {
    for house in &houses {
        let Some(state) = settlements.house(house.0) else {
            continue;
        };
        if state.status == HouseStatus::Destroyed {
            continue;
        }
        let outside = state.residents.saturating_sub(state.hiding);
        for _ in 0..outside {
            let angle = dice.range(0.0, std::f32::consts::TAU);
            let r = state.kind.half_extents().length() + dice.range(1.0, 4.0);
            let p = state.position + Vec2::from_angle(angle) * r;
            spawn_resident(&mut commands, &assets, &mut models, &terrain, &mut dice, house.0, p);
        }
    }
}

fn spawn_resident(
    commands: &mut Commands,
    assets: &ResidentAssets,
    models: &mut Models,
    terrain: &Terrain,
    dice: &mut Dice,
    home: HouseId,
    p: Vec2,
) -> Entity {
    let clothes = assets.clothes[dice.below(assets.clothes.len())].clone();
    let skin = assets.skin[dice.below(assets.skin.len())].clone();
    let hair = assets.hair[dice.below(assets.hair.len())].clone();
    let accessory = dice.below(3);
    let y = terrain.height_at(p.x, p.y);
    commands
        .spawn((
            Resident {
                home,
                mood: Mood::Wander,
                target: p,
                pause: dice.range(0.0, 2.0),
                shelter: None,
                contagious: 0.0,
                stuck: 0.0,
                phase: dice.range(0.0, 10.0),
                velocity: Vec2::ZERO,
            },
            RunScoped,
            Health::new(1.0),
            Team::Neutral,
            Collider { radius: 0.35 },
            Transform::from_xyz(p.x, y, p.y),
            Visibility::default(),
        ))
        .with_children(|r| {
            // Модель из списка или человечек из примитивов; значок испуга общий для обоих.
            models.spawn_visual(r, RESIDENT_MODEL, Transform::IDENTITY, |r| {
                r.spawn((Mesh3d(assets.body.clone()), MeshMaterial3d(clothes.clone()), NotShadowCaster, Transform::from_xyz(0.0, 0.45, 0.0)));
                r.spawn((Mesh3d(assets.head.clone()), MeshMaterial3d(skin.clone()), NotShadowCaster, Transform::from_xyz(0.0, 0.95, 0.0)));
                // Волосы: шапочка на макушке и затылке, лицо открыто.
                r.spawn((Mesh3d(assets.ball.clone()), MeshMaterial3d(hair.clone()), NotShadowCaster,
                    Transform::from_xyz(0.0, 1.07, 0.04).with_scale(Vec3::new(0.21, 0.13, 0.21))));
                // Большие глаза смотрят вперёд, на -Z.
                for x in [-0.08, 0.08] {
                    r.spawn((Mesh3d(assets.ball.clone()), MeshMaterial3d(assets.white.clone()), NotShadowCaster,
                        Transform::from_xyz(x, 1.0, -0.15).with_scale(Vec3::splat(0.075))));
                    r.spawn((Mesh3d(assets.ball.clone()), MeshMaterial3d(assets.black.clone()), NotShadowCaster,
                        Transform::from_xyz(x, 1.0, -0.21).with_scale(Vec3::splat(0.035))));
                }
                for side in [-1.0, 1.0] {
                    r.spawn((Arm(side), Mesh3d(assets.arm.clone()), MeshMaterial3d(clothes.clone()), NotShadowCaster,
                        Transform::from_xyz(side * 0.28, 0.6, 0.0)));
                }
                match accessory {
                    0 => {
                        r.spawn((Mesh3d(assets.hat.clone()), MeshMaterial3d(assets.black.clone()), NotShadowCaster,
                            Transform::from_xyz(0.0, 1.15, 0.0)));
                    }
                    1 => {
                        r.spawn((Mesh3d(assets.cube.clone()), MeshMaterial3d(assets.white.clone()), NotShadowCaster,
                            Transform::from_xyz(0.0, 0.75, 0.0).with_scale(Vec3::new(0.42, 0.08, 0.42))));
                    }
                    _ => {}
                }
            });
            r.spawn((Alert, Mesh3d(assets.cube.clone()), MeshMaterial3d(assets.alert.clone()), NotShadowCaster,
                Visibility::Hidden, Transform::from_xyz(0.0, 1.7, 0.0).with_scale(Vec3::new(0.09, 0.3, 0.09))));
        })
        .id()
}

/// Собирает точки опасности кадра.
#[allow(clippy::too_many_arguments)]
fn gather_danger(
    mut danger: ResMut<Danger>,
    mut explosions: MessageReader<Explosion>,
    mut shells: MessageReader<FireShell>,
    mut bullets: MessageReader<FireBullet>,
    enemies: Query<&Transform, With<Enemy>>,
    rover: Query<(&Transform, &RoverMotion), With<Rover>>,
    residents: Query<(&Transform, &Resident)>,
) {
    let flat = |v: Vec3| Vec2::new(v.x, v.z);
    danger.points.clear();
    danger.points.extend(explosions.read().map(|e| (flat(e.position), FEAR_EXPLOSION)));
    danger.points.extend(shells.read().map(|s| (flat(s.target), FEAR_SHELL)));
    danger.points.extend(bullets.read().map(|b| (flat(b.origin), FEAR_SHOT)));
    danger.points.extend(enemies.iter().map(|t| (flat(t.translation), FEAR_ENEMY)));
    for (t, motion) in &rover {
        if motion.speed.abs() > SCARY_SPEED {
            danger.points.push((flat(t.translation), FEAR_ROVER));
        }
    }
    let contagious: Vec<_> = residents
        .iter()
        .filter(|(_, r)| r.contagious > 0.0)
        .map(|(t, _)| (flat(t.translation), FEAR_NEIGHBOUR))
        .collect();
    danger.points.extend(contagious);
}

/// Испуганный житель паникует: бежит к ближайшему целому дому или прочь от опасности.
fn react_to_danger(
    danger: Res<Danger>,
    settlements: Res<Settlements>,
    houses: Query<(Entity, &House, &Transform), Without<Resident>>,
    mut residents: Query<(&Transform, &mut Resident)>,
) {
    if danger.points.is_empty() {
        return;
    }
    for (transform, mut resident) in &mut residents {
        let p = Vec2::new(transform.translation.x, transform.translation.z);
        let scared: Vec<Vec2> = danger
            .points
            .iter()
            .filter(|(c, r)| c.distance(p) < *r && c.distance(p) > 0.01)
            .map(|(c, _)| *c)
            .collect();
        if scared.is_empty() {
            continue;
        }
        let was_calm = !matches!(resident.mood, Mood::Panic(_));
        resident.mood = Mood::Panic(PANIC_TIME);
        if was_calm {
            resident.contagious = CONTAGION_TIME;
        }
        // Ближайший целый дом в 30 м.
        let shelter = houses
            .iter()
            .filter(|(_, h, _)| settlements.house(h.0).is_some_and(|s| s.status == HouseStatus::Intact))
            .map(|(e, h, t)| (e, h.0, Vec2::new(t.translation.x, t.translation.z).distance(p)))
            .filter(|(_, _, d)| *d < HIDE_RADIUS)
            .min_by(|a, b| a.2.total_cmp(&b.2));
        if let Some((entity, id, _)) = shelter {
            resident.shelter = Some((id, entity));
            resident.target = settlements.house(id).map_or(p, |s| s.position);
        } else {
            resident.shelter = None;
            let center = scared.iter().sum::<Vec2>() / scared.len() as f32;
            resident.target = p + (p - center).normalize_or(Vec2::X) * FLEE_DISTANCE;
        }
    }
}

/// Решения: пауза, новая цель у дома, разговоры парами, конец паники.
fn think(
    time: Res<Time>,
    mut dice: ResMut<GameRng>,
    settlements: Res<Settlements>,
    mut residents: Query<(Entity, &Transform, &mut Resident)>,
) {
    let dt = time.delta_secs();
    let positions: Vec<(Entity, Vec2, bool)> = residents
        .iter()
        .map(|(e, t, r)| (e, Vec2::new(t.translation.x, t.translation.z), r.mood == Mood::Wander && r.pause > 0.0))
        .collect();
    for (entity, transform, mut resident) in &mut residents {
        let p = Vec2::new(transform.translation.x, transform.translation.z);
        resident.contagious = (resident.contagious - dt).max(0.0);
        let home = settlements.house(resident.home).map_or(p, |s| s.position);
        match resident.mood {
            Mood::Wander => {
                if resident.pause > 0.0 {
                    resident.pause -= dt;
                    // Двое в паузе рядом — болтают.
                    let partner = positions.iter().find(|(e, q, idle)| *e != entity && *idle && q.distance(p) < 3.0);
                    if let Some((_, q, _)) = partner
                        && dice.range(0.0, 1.0) < dt * 0.8
                    {
                        resident.target = *q;
                        resident.mood = Mood::Chat(dice.range(3.0, 6.0));
                    }
                } else if resident.target.distance(p) < 0.5 || resident.stuck > 1.5 {
                    resident.stuck = 0.0;
                    resident.pause = dice.range(1.0, 4.0);
                    resident.target = home + Vec2::from_angle(dice.range(0.0, std::f32::consts::TAU)) * dice.range(3.0, WANDER_RADIUS);
                }
            }
            Mood::Chat(left) => {
                let left = left - dt;
                resident.mood = if left > 0.0 { Mood::Chat(left) } else { Mood::Wander };
                if left <= 0.0 {
                    resident.target = p;
                }
            }
            Mood::Panic(left) => {
                let left = left - dt;
                if left <= 0.0 && resident.shelter.is_none() {
                    // Паника прошла: возвращается к дому шагом.
                    resident.mood = Mood::Wander;
                    resident.target = home;
                } else {
                    resident.mood = Mood::Panic(left.max(0.0));
                }
                if resident.stuck > 1.5 {
                    resident.stuck = 0.0;
                    resident.target = p + Vec2::from_angle(dice.range(0.0, std::f32::consts::TAU)) * 8.0;
                }
            }
            Mood::Dodge(left) => {
                let left = left - dt;
                resident.mood = if left > 0.0 { Mood::Dodge(left) } else { Mood::Wander };
            }
        }
    }
}

/// Ходьба к цели с объездом домов и объектов; прячущийся у двери исчезает в доме.
fn walk(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut settlements: ResMut<Settlements>,
    obstacles: Query<&Obstacle, Without<Resident>>,
    mut residents: Query<(Entity, &mut Transform, &mut Resident)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for (entity, mut transform, mut resident) in &mut residents {
        let p = Vec2::new(transform.translation.x, transform.translation.z);
        let (speed, facing_target) = match resident.mood {
            Mood::Wander if resident.pause > 0.0 => (0.0, false),
            Mood::Wander => (WALK_SPEED, true),
            Mood::Chat(_) => (0.0, true),
            Mood::Panic(_) => (RUN_SPEED, true),
            Mood::Dodge(_) => (0.0, false),
        };
        let to = resident.target - p;
        let mut step = if matches!(resident.mood, Mood::Dodge(_)) {
            resident.velocity * dt
        } else {
            to.normalize_or_zero() * speed * dt
        };
        if step.length() > to.length() && !matches!(resident.mood, Mood::Dodge(_)) {
            step = to;
        }
        // Укрытие: у стены своего убежища житель заходит внутрь.
        if let (Mood::Panic(_), Some((id, _))) = (resident.mood, resident.shelter)
            && let Some(shelter) = settlements.house(id)
            && shelter.footprint().distance(p) < 0.8
        {
            let home = resident.home;
            if shelter.status == HouseStatus::Intact {
                if let Some(old) = settlements.house_mut(home) {
                    old.residents = old.residents.saturating_sub(1);
                }
                if let Some(new) = settlements.house_mut(id) {
                    new.residents += 1;
                    new.hiding += 1;
                }
                commands.entity(entity).despawn();
                continue;
            }
        }
        let mut next = p + step;
        for obstacle in &obstacles {
            // В своё убежище можно упереться: он и есть цель.
            if let Some(push) = obstacle.shape.push_out(next, 0.3) {
                next += push;
            }
        }
        let moved = next.distance(p);
        if speed > 0.0 && moved < speed * dt * 0.2 {
            resident.stuck += dt;
        } else {
            resident.stuck = 0.0;
        }
        transform.translation.x = next.x;
        transform.translation.z = next.y;
        transform.translation.y = terrain.height_at(next.x, next.y);
        if facing_target && to.length() > 0.05 {
            let dir = to.normalize();
            transform.rotation = Quat::from_rotation_y((-dir.x).atan2(-dir.y));
        }
    }
}

/// Медленная машина — житель отскакивает вбок, быстрая — сбивает насмерть.
fn dodge_vehicles(
    mut dice: ResMut<GameRng>,
    rover: Query<(&Transform, &RoverMotion), With<Rover>>,
    vehicles: Query<(&Transform, &GroundVehicle), Without<Resident>>,
    mut residents: Query<(Entity, &Transform, &mut Resident)>,
    mut damage: MessageWriter<Damage>,
) {
    let movers: Vec<(Vec2, Vec2, f32, Team)> = rover
        .iter()
        .map(|(t, m)| (t, crate::vehicle::forward_of(m.heading) * m.speed.signum(), m.speed.abs(), Team::Player))
        .chain(vehicles.iter().map(|(t, v)| (t, v.forward(), v.speed.abs(), Team::Enemy)))
        .map(|(t, f, s, team)| (Vec2::new(t.translation.x, t.translation.z), Vec2::new(f.x, f.z), s, team))
        .filter(|(_, _, s, _)| *s > DODGE_SPEED)
        .collect();
    for (entity, transform, mut resident) in &mut residents {
        if matches!(resident.mood, Mood::Dodge(_)) {
            continue;
        }
        let p = Vec2::new(transform.translation.x, transform.translation.z);
        for (c, forward, speed, team) in &movers {
            let d = p - *c;
            if d.length() > 4.0 || d.dot(*forward) < -1.0 {
                continue;
            }
            if *speed >= LETHAL_SPEED {
                if d.length() < 2.2 {
                    damage.write(Damage {
                        target: entity,
                        amount: 5.0,
                        source_team: *team,
                    });
                }
            } else {
                let side = Vec2::new(-forward.y, forward.x);
                let sign = if side.dot(d) >= 0.0 { 1.0 } else { -1.0 };
                resident.velocity = side * sign * dice.range(6.0, 8.0);
                resident.mood = Mood::Dodge(0.4);
            }
            break;
        }
    }
}

/// Покачивание при ходьбе, руки маятником; в панике руки вверх с тряской и подпрыгивание.
fn animate(
    time: Res<Time>,
    mut residents: Query<(&mut Resident, &Children, &mut Transform)>,
    mut arms: Query<(&Arm, &mut Transform), Without<Resident>>,
    mut alerts: Query<&mut Visibility, With<Alert>>,
) {
    let dt = time.delta_secs();
    for (mut resident, children, mut transform) in &mut residents {
        let (rate, panic) = match resident.mood {
            Mood::Panic(_) => (18.0, true),
            Mood::Wander if resident.pause <= 0.0 => (9.0, false),
            Mood::Chat(_) => (4.0, false),
            Mood::Dodge(_) => (20.0, true),
            _ => (0.0, false),
        };
        resident.phase += dt * rate;
        let phase = resident.phase;
        let hop = if panic { (phase * 0.5).sin().abs() * 0.25 } else { (phase).sin().abs() * 0.05 };
        transform.translation.y += hop;
        let chatting = matches!(resident.mood, Mood::Chat(_));
        for child in children.iter() {
            if let Ok((arm, mut t)) = arms.get_mut(child) {
                let angle = if panic {
                    std::f32::consts::PI * 0.85 + (phase * 2.0 + arm.0).sin() * 0.3
                } else if chatting && arm.0 > 0.0 {
                    1.2 + (phase * 1.5).sin() * 0.5
                } else {
                    phase.sin() * 0.5 * arm.0 * if rate > 0.0 { 1.0 } else { 0.0 }
                };
                t.rotation = Quat::from_rotation_x(angle) * Quat::from_rotation_z(arm.0 * 0.15);
                t.translation.y = if panic { 0.75 } else { 0.6 };
            }
            if let Ok(mut v) = alerts.get_mut(child) {
                let want = if panic { Visibility::Inherited } else { Visibility::Hidden };
                if *v != want {
                    *v = want;
                }
            }
        }
    }
}

/// Гибель: облачко «пуф», отлетающая шляпа-кубик, учёт в доме и сообщение для тревоги.
fn resident_deaths(
    mut commands: Commands,
    mut died: MessageReader<Died>,
    assets: Res<ResidentAssets>,
    mut settlements: ResMut<Settlements>,
    mut dice: ResMut<GameRng>,
    residents: Query<(&Transform, &Resident)>,
    mut deaths: MessageWriter<ResidentDied>,
) {
    for death in died.read() {
        let Ok((transform, resident)) = residents.get(death.entity) else {
            continue;
        };
        let at = transform.translation + Vec3::Y * 0.6;
        if let Some(home) = settlements.house_mut(resident.home) {
            home.residents = home.residents.saturating_sub(1);
        }
        spawn_puff(&mut commands, &assets, &mut dice, at);
        deaths.write(ResidentDied { position: at });
        commands.entity(death.entity).despawn();
    }
}

fn spawn_puff(commands: &mut Commands, assets: &ResidentAssets, dice: &mut Dice, at: Vec3) {
    for i in 0..9 {
        let dir = Vec3::new(dice.range(-1.0, 1.0), dice.range(0.2, 1.0), dice.range(-1.0, 1.0)).normalize_or(Vec3::Y);
        let star = i >= 6;
        commands.spawn((
            Puff {
                velocity: dir * if star { 6.0 } else { 2.0 },
                age: 0.0,
            },
            RunScoped,
            NotShadowCaster,
            Mesh3d(if star { assets.cube.clone() } else { assets.ball.clone() }),
            MeshMaterial3d(if star { assets.alert.clone() } else { assets.puff.clone() }),
            Transform::from_translation(at).with_scale(Vec3::splat(if star { 0.15 } else { 0.35 })),
        ));
    }
}

/// Облачка разлетаются, раздуваются и тают; звёздочки крутятся.
fn fade_puffs(mut commands: Commands, time: Res<Time>, mut puffs: Query<(Entity, &mut Puff, &mut Transform)>) {
    let dt = time.delta_secs();
    for (entity, mut puff, mut t) in &mut puffs {
        puff.age += dt;
        if puff.age > 0.6 {
            commands.entity(entity).despawn();
            continue;
        }
        t.translation += puff.velocity * dt;
        puff.velocity *= 1.0 - 3.0 * dt;
        t.scale *= 1.0 + 2.5 * dt;
        t.rotate_y(dt * 8.0);
    }
}

/// Прятавшиеся в разрушенном доме гибнут вместе с ним.
fn hidden_die_with_house(
    mut commands: Commands,
    assets: Res<ResidentAssets>,
    mut dice: ResMut<GameRng>,
    mut settlements: ResMut<Settlements>,
    mut destroyed: MessageReader<crate::buildings::BuildingDestroyed>,
    mut deaths: MessageWriter<ResidentDied>,
    mut danger: ResMut<Danger>,
) {
    if destroyed.read().count() == 0 {
        return;
    }
    // Укрытия, которые больше не целы.
    let mut gone = Vec::new();
    danger.calm.retain(|(id, _, _)| {
        let intact = settlements.house(*id).is_some_and(|s| s.status != HouseStatus::Destroyed);
        if !intact {
            gone.push(*id);
        }
        intact
    });
    for id in gone {
        let Some(state) = settlements.house_mut(id) else {
            continue;
        };
        let victims = state.hiding;
        state.residents = state.residents.saturating_sub(victims);
        state.hiding = 0;
        let at = Vec3::new(state.position.x, 0.0, state.position.y);
        for _ in 0..victims {
            deaths.write(ResidentDied { position: at });
        }
        if victims > 0 {
            spawn_puff(&mut commands, &assets, &mut dice, at + Vec3::Y * 2.0);
        }
    }
}

/// После 20 с затишья у дома прячущиеся выходят по одному.
fn leave_shelters(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<ResidentAssets>,
    mut models: Models,
    terrain: Res<Terrain>,
    mut dice: ResMut<GameRng>,
    mut danger: ResMut<Danger>,
    mut settlements: ResMut<Settlements>,
    houses: Query<(&House, &Transform)>,
) {
    let dt = time.delta_secs();
    // Дома с прячущимися, которых ещё нет в списке затишья.
    for (house, _) in &houses {
        if settlements.house(house.0).is_some_and(|s| s.hiding > 0) && !danger.calm.iter().any(|(id, ..)| *id == house.0) {
            danger.calm.push((house.0, 0.0, 0.0));
        }
    }
    let points = danger.points.clone();
    let danger = &mut *danger;
    for (id, calm, gap) in danger.calm.iter_mut() {
        let Some(state) = settlements.house_mut(*id) else {
            continue;
        };
        if points.iter().any(|(c, r)| c.distance(state.position) < HIDE_RADIUS.max(*r)) {
            *calm = 0.0;
            continue;
        }
        *calm += dt;
        *gap -= dt;
        if *calm < CALM_TIME || *gap > 0.0 || state.hiding == 0 {
            continue;
        }
        *gap = EXIT_GAP;
        state.hiding -= 1;
        let side = dice.below(4) as u8;
        let door = state.wall_center(side);
        let out = (door - state.position).normalize_or(Vec2::X);
        let p = door + out * 0.8;
        spawn_resident(&mut commands, &assets, &mut models, &terrain, &mut dice, *id, p);
    }
    danger.calm.retain(|(id, ..)| settlements.house(*id).is_some_and(|s| s.hiding > 0));
}

/// Глаза в окнах дома, пока внутри кто-то прячется; моргают.
fn window_eyes(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<ResidentAssets>,
    settlements: Res<Settlements>,
    houses: Query<(Entity, &House, Option<&Children>)>,
    mut eyes: Query<&mut Transform, With<WindowEyes>>,
) {
    let blink = (time.elapsed_secs() * 0.7).fract() < 0.06;
    for (entity, house, children) in &houses {
        let Some(state) = settlements.house(house.0) else {
            continue;
        };
        let existing: Vec<Entity> = children
            .map(|c| c.iter().filter(|c| eyes.get(*c).is_ok()).collect())
            .unwrap_or_default();
        if state.hiding == 0 || state.status == HouseStatus::Destroyed {
            for e in existing {
                commands.entity(e).despawn();
            }
            continue;
        }
        if existing.is_empty() {
            let half = state.kind.half_extents();
            commands.entity(entity).with_children(|h| {
                for x in [-0.15, 0.15] {
                    h.spawn((WindowEyes, Mesh3d(assets.ball.clone()), MeshMaterial3d(assets.white.clone()), NotShadowCaster,
                        Transform::from_xyz(half.x * 0.4 + x, 2.0, half.y + 0.06).with_scale(Vec3::new(0.12, 0.12, 0.04))));
                }
            });
        }
        for e in existing {
            if let Ok(mut t) = eyes.get_mut(e) {
                t.scale.y = if blink { 0.01 } else { 0.12 };
            }
        }
    }
}

/// Жители, чей дом выгружен вместе с участком, уходят из сцены: их число хранится в доме.
/// Жители разрушенного дома остаются бродить у руин.
fn remove_far_residents(
    mut commands: Commands,
    settlements: Res<Settlements>,
    houses: Query<&House>,
    residents: Query<(Entity, &Resident)>,
) {
    let spawned: Vec<HouseId> = houses.iter().map(|h| h.0).collect();
    for (entity, resident) in &residents {
        let ruined = settlements.house(resident.home).is_none_or(|s| s.status == HouseStatus::Destroyed);
        if !ruined && !spawned.contains(&resident.home) {
            commands.entity(entity).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settlements::generate_region;

    #[test]
    fn в_посёлке_от_8_до_12_жителей() {
        let terrain = Terrain::new(4);
        for x in -3..3 {
            let region = generate_region(4, IVec2::new(x, 1), &terrain);
            if region.houses.is_empty() {
                continue;
            }
            let total: u32 = region.houses.iter().map(|h| h.residents as u32).sum();
            assert!((8..=12).contains(&total), "{total}");
        }
    }
}
