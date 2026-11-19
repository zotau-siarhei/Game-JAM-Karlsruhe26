//! Авиаудар: отметка на земле по направлению от лунохода к прицелу, через две секунды над ней
//! пролетает самолёт и кладёт серию бомб с мощными взрывами и большими кратерами.
//!
//! Выстрел авиаударом открывает режим наведения ([`Targeting`]): время замедляется до 20 %,
//! камера поднимается почти вертикально и отъезжает, прицел ходит до 80 м от лунохода, на нём
//! видна будущая отметка. Колесо и бамперы поворачивают серию шагом 15°, левая кнопка или R2
//! вызывают удар, правая кнопка, Esc или B отменяют без траты заряда. Через 6 с реального
//! времени удар вызывается сам в текущей точке.
//!
//! Длина серии и число самолётов — по уровню (`upgrades::airstrike`): на третьем уровне второй
//! самолёт бомбит поперёк первого, линии кратеров пересекаются в точке прицела.

use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::aim::AimPoint;
use crate::arsenal::Inventory;
use crate::combat::{Health, Team};
use crate::explosion::Explosion;
use crate::frame::GameSet;
use crate::gamepad::{DeviceHint, PadInput};
use crate::rover::{self, Rover};
use crate::run::{GameState, RunScoped, WorldRebuilt};
use crate::settlements::Footprint;
use crate::terrain::Terrain;
use crate::upgrades::{self, WeaponLevels};
use crate::weapons::CurrentWeapon;

/// Скорость времени в режиме наведения.
const TARGET_TIME: f32 = 0.2;
/// За сколько реальных секунд время и камера переходят в режим наведения и обратно.
const BLEND_SECONDS: f32 = 0.3;
/// Сколько реальных секунд длится наведение, потом удар вызывается сам.
const TARGET_SECONDS: f32 = 6.0;
/// Дальность наведения от лунохода, м.
pub const TARGET_RANGE: f32 = 80.0;
/// Шаг поворота серии колесом и бамперами, рад.
const TURN_STEP: f32 = 15.0 * std::f32::consts::PI / 180.0;
/// Пока колесо крутится, следующий шаг поворота не раньше этого, с реального времени.
const SCROLL_COOLDOWN: f32 = 0.12;

/// Ширина отметки, м. Длина — по числу бомб: [`mark_length`].
const MARK_WIDTH: f32 = 6.0;

/// Второй самолёт заходит позже первого, с.
const SECOND_PLANE_DELAY: f32 = 0.4;

/// Через сколько после вызова появляется самолёт, с.
pub const DELAY: f32 = 2.0;

/// Высота и скорость самолёта.
/// Высота пролёта над отметкой, м: выше камеры (обычная — 25–35 м над луноходом, на узком экране
/// до 55 м), иначе самолёт пролетал сквозь неё и закрывал экран.
const PLANE_HEIGHT: f32 = 55.0;
/// Масштаб самолёта: с такой высоты он виден, как прежде с 25 м.
const PLANE_SCALE: f32 = 1.8;
const PLANE_SPEED: f32 = 60.0;

/// Запас захода сверх упреждения первой бомбы, м: самолёт появляется до точки её сброса.
const APPROACH_MARGIN: f32 = 15.0;

/// Бомбы: число на первом уровне, шаг, мощность и радиус взрыва.
pub const BOMBS: usize = 6;
const BOMB_STEP: f32 = 5.0;
const BOMB_POWER: f32 = 1.6;
const BOMB_RADIUS: f32 = 7.0;

/// Гравитация для бомб, м/с².
const GRAVITY: f32 = 20.0;

/// Сегментов отметки на длинной стороне.
const MARK_SEGMENTS: usize = 8;

/// Время падения бомбы с высоты пролёта, с.
fn fall_time() -> f32 {
    (2.0 * PLANE_HEIGHT / GRAVITY).sqrt()
}

/// Смещение точки падения бомбы `index` из `bombs` от центра отметки вдоль пролёта, м.
fn bomb_offset(index: usize, bombs: usize) -> f32 {
    -(bombs as f32 - 1.0) * BOMB_STEP * 0.5 + index as f32 * BOMB_STEP
}

/// Длина отметки вдоль пролёта при `bombs` бомбах, м: шесть бомб — 30 м.
fn mark_length(bombs: usize) -> f32 {
    bombs as f32 * BOMB_STEP
}

/// С какого расстояния до центра самолёт заходит и где исчезает, м.
///
/// Бомба сбрасывается за `PLANE_SPEED · fall_time` до своей точки, поэтому заход дальше
/// упреждения первой бомбы: иначе в первом же кадре падают сразу несколько бомб в одну точку.
fn approach(bombs: usize) -> f32 {
    PLANE_SPEED * fall_time() - bomb_offset(0, bombs) + APPROACH_MARGIN
}

/// Подключает авиаудар.
pub struct AirstrikePlugin;

impl Plugin for AirstrikePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Targeting>()
            .add_systems(Startup, create_airstrike_assets)
            .add_systems(
                Update,
                (
                    reset_targeting.in_set(GameSet::Reset),
                    (call_airstrike, aim_strike).chain().after(rover::drive_rover).in_set(GameSet::Input),
                    (run_strikes, pulse_marks, drop_bombs).chain().in_set(GameSet::Simulation),
                    (blend_targeting, preview_strike, show_targeting_ui).chain().in_set(GameSet::Presentation),
                ),
            );
    }
}

/// Режим наведения авиаудара.
#[derive(Resource, Default)]
pub struct Targeting {
    /// Режим открыт: игрок выбирает место удара.
    pub open: bool,
    /// Переход к виду наведения: 0 — обычный вид и время, 1 — полностью. Идёт по реальному времени.
    pub blend: f32,
    /// Сколько реальных секунд осталось до удара самого.
    left: f32,
    /// Поворот серии в шагах [`TURN_STEP`] от направления «от лунохода к прицелу».
    turns: i32,
    scroll_cooldown: f32,
    /// Режим открыт в этом кадре: тот же клик не подтверждает удар.
    just_opened: bool,
}

impl Targeting {
    /// Режим открыт или камера и время ещё возвращаются из него.
    pub fn active(&self) -> bool {
        self.open || self.blend > 0.0
    }

    fn close(&mut self) {
        self.open = false;
        self.just_opened = false;
    }
}

/// Отметка будущего удара в режиме наведения: пересоздаётся каждый кадр.
#[derive(Component)]
struct Preview;

/// Полоса оставшегося времени наведения и подсказка.
#[derive(Component)]
struct TargetingUi;
#[derive(Component)]
struct TargetingBar;

/// Вызванный авиаудар. Живёт от вызова до ухода самолёта и падения последней бомбы.
#[derive(Component)]
pub struct Strike {
    /// Центр отметки на земле.
    pub center: Vec3,
    /// Направление пролёта в плане.
    pub dir: Vec3,
    /// Бомб в серии.
    bombs: usize,
    elapsed: f32,
    /// Самолёт уже появился.
    plane_spawned: bool,
    plane: Option<Entity>,
    shadow: Option<Entity>,
    dropped: usize,
    /// Сколько бомб уже взорвалось: отметка держится до последней.
    landed: usize,
    marks: Vec<Entity>,
}

/// Сегмент отметки на земле.
#[derive(Component)]
pub struct Mark {
    pub strike: Entity,
    /// Высота сегмента без пульсации: у рамки тонкая полоса, у сигнального дыма столб.
    base_height: f32,
}

/// Падающая бомба.
#[derive(Component)]
struct Bomb {
    velocity: Vec3,
    strike: Entity,
}

#[derive(Resource)]
struct AirstrikeAssets {
    cube: Handle<Mesh>,
    safe: Handle<StandardMaterial>,
    danger: Handle<StandardMaterial>,
    plane: Handle<StandardMaterial>,
    shadow: Handle<StandardMaterial>,
    bomb: Handle<StandardMaterial>,
    smoke: Handle<StandardMaterial>,
    sphere: Handle<Mesh>,
}

fn create_airstrike_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let glow = |materials: &mut Assets<StandardMaterial>, color: Color, e: LinearRgba| {
        materials.add(StandardMaterial {
            base_color: color,
            emissive: e,
            unlit: true,
            ..default()
        })
    };
    commands.insert_resource(AirstrikeAssets {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        sphere: meshes.add(Sphere::new(0.5).mesh().ico(1).unwrap()),
        safe: glow(&mut materials, Color::srgb(1.0, 0.85, 0.2), LinearRgba::rgb(3.0, 2.4, 0.3)),
        danger: glow(&mut materials, Color::srgb(1.0, 0.15, 0.1), LinearRgba::rgb(4.0, 0.4, 0.2)),
        plane: materials.add(Color::srgb(0.55, 0.6, 0.68)),
        shadow: materials.add(StandardMaterial {
            base_color: Color::srgba(0.0, 0.0, 0.0, 0.45),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..default()
        }),
        bomb: materials.add(Color::srgb(0.15, 0.16, 0.18)),
        smoke: glow(&mut materials, Color::srgb(1.0, 0.3, 0.6), LinearRgba::rgb(2.0, 0.5, 1.2)),
    });
}

/// Прямоугольник отметки в плане.
fn footprint(center: Vec3, dir: Vec3, bombs: usize) -> Footprint {
    Footprint {
        center: Vec2::new(center.x, center.z),
        // Локальная ось Z — вдоль пролёта.
        yaw: (dir.x).atan2(dir.z),
        half: Vec2::new(MARK_WIDTH * 0.5, mark_length(bombs) * 0.5),
    }
}

/// Выстрел авиаударом открывает режим наведения: заряд тратится только при ударе.
fn call_airstrike(
    mouse: Res<ButtonInput<MouseButton>>,
    weapon: Res<CurrentWeapon>,
    inventory: Res<Inventory>,
    mut targeting: ResMut<Targeting>,
    rover: Single<&Health, With<Rover>>,
) {
    if targeting.open
        || !mouse.just_pressed(MouseButton::Left)
        || *weapon != CurrentWeapon::Airstrike
        || !inventory.usable(CurrentWeapon::Airstrike)
        || rover.is_dead()
    {
        return;
    }
    *targeting = Targeting {
        open: true,
        blend: targeting.blend,
        left: TARGET_SECONDS,
        turns: 0,
        scroll_cooldown: 0.0,
        just_opened: true,
    };
}

/// Направление серии: от лунохода к прицелу, повёрнутое на `turns` шагов.
fn strike_dir(rover: Vec3, point: Vec3, turns: i32) -> Vec3 {
    let dir = (point - rover).with_y(0.0).normalize_or(Vec3::NEG_Z);
    Quat::from_rotation_y(turns as f32 * TURN_STEP) * dir
}

/// Наведение: поворот серии, подтверждение, отмена и удар по истечении времени.
#[allow(clippy::too_many_arguments)]
fn aim_strike(
    mut commands: Commands,
    real: Res<Time<Real>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    pad: Res<PadInput>,
    weapon: Res<CurrentWeapon>,
    aim: Res<AimPoint>,
    assets: Res<AirstrikeAssets>,
    terrain: Res<Terrain>,
    levels: Res<WeaponLevels>,
    mut inventory: ResMut<Inventory>,
    mut targeting: ResMut<Targeting>,
    rover: Single<(&Transform, &Health), With<Rover>>,
) {
    if !targeting.open {
        return;
    }
    let (rover, health) = *rover;
    if health.is_dead() || *weapon != CurrentWeapon::Airstrike || !inventory.usable(CurrentWeapon::Airstrike) {
        targeting.close();
        return;
    }
    let dt = real.delta_secs();
    targeting.scroll_cooldown = (targeting.scroll_cooldown - dt).max(0.0);
    if pad.weapon_step != 0 {
        targeting.turns += pad.weapon_step;
    } else if scroll.delta.y != 0.0 {
        if targeting.scroll_cooldown == 0.0 {
            targeting.turns += if scroll.delta.y > 0.0 { 1 } else { -1 };
        }
        targeting.scroll_cooldown = SCROLL_COOLDOWN;
    }
    if mouse.just_pressed(MouseButton::Right) || keys.just_pressed(KeyCode::Escape) {
        targeting.close();
        return;
    }
    let confirmed = !targeting.just_opened && mouse.just_pressed(MouseButton::Left);
    targeting.just_opened = false;
    targeting.left -= dt;
    if !confirmed && targeting.left > 0.0 {
        return;
    }
    inventory.spend(CurrentWeapon::Airstrike);
    let level = upgrades::airstrike(levels.level(CurrentWeapon::Airstrike));
    let dir = strike_dir(rover.translation, aim.position, targeting.turns);
    for plane in 0..level.planes {
        // Второй самолёт — поперёк первого и чуть позже.
        let dir = Quat::from_rotation_y(plane as f32 * std::f32::consts::FRAC_PI_2) * dir;
        let delay = plane as f32 * SECOND_PLANE_DELAY;
        spawn_strike(&mut commands, &assets, &terrain, aim.position, dir, level.bombs, delay);
    }
    targeting.close();
}

/// Новый мир: режим наведения закрыт, время обычное (его ставит `impact::reset_slowmo`).
fn reset_targeting(mut rebuilt: MessageReader<WorldRebuilt>, mut targeting: ResMut<Targeting>) {
    if rebuilt.read().count() > 0 {
        *targeting = Targeting::default();
    }
}

/// Переход в режим наведения и обратно по реальному времени; режим владеет скоростью времени,
/// пока открыт или возвращается. Пауза и выход из забега закрывают режим без удара.
fn blend_targeting(
    real: Res<Time<Real>>,
    state: Res<State<GameState>>,
    mut time: ResMut<Time<Virtual>>,
    mut targeting: ResMut<Targeting>,
) {
    if *state.get() != GameState::Playing {
        if targeting.active() {
            *targeting = Targeting::default();
            time.set_relative_speed(1.0);
        }
        return;
    }
    if !targeting.active() {
        return;
    }
    let step = real.delta_secs() / BLEND_SECONDS;
    targeting.blend = if targeting.open {
        (targeting.blend + step).min(1.0)
    } else {
        (targeting.blend - step).max(0.0)
    };
    let k = targeting.blend * targeting.blend * (3.0 - 2.0 * targeting.blend);
    time.set_relative_speed(1.0 + (TARGET_TIME - 1.0) * k);
}

/// Отметка будущего удара под прицелом: красная, если задевает луноход.
#[allow(clippy::too_many_arguments)]
fn preview_strike(
    mut commands: Commands,
    targeting: Res<Targeting>,
    aim: Res<AimPoint>,
    assets: Res<AirstrikeAssets>,
    terrain: Res<Terrain>,
    levels: Res<WeaponLevels>,
    rover: Single<&Transform, With<Rover>>,
    shown: Query<Entity, With<Preview>>,
) {
    for entity in &shown {
        commands.entity(entity).despawn();
    }
    if !targeting.open {
        return;
    }
    let level = upgrades::airstrike(levels.level(CurrentWeapon::Airstrike));
    let dir = strike_dir(rover.translation, aim.position, targeting.turns);
    let me = Vec2::new(rover.translation.x, rover.translation.z);
    for plane in 0..level.planes {
        let dir = Quat::from_rotation_y(plane as f32 * std::f32::consts::FRAC_PI_2) * dir;
        let inside = footprint(aim.position, dir, level.bombs).distance(me) < 1.5;
        let material = if inside { &assets.danger } else { &assets.safe };
        for transform in outline(&terrain, aim.position, dir, level.bombs) {
            commands.spawn((
                Preview,
                RunScoped,
                NotShadowCaster,
                Mesh3d(assets.cube.clone()),
                MeshMaterial3d(material.clone()),
                transform,
            ));
        }
    }
}

/// Полоса оставшегося времени и подсказка внизу экрана, пока открыт режим.
fn show_targeting_ui(
    mut commands: Commands,
    targeting: Res<Targeting>,
    ui: Query<Entity, With<TargetingUi>>,
    mut bar: Query<&mut Node, With<TargetingBar>>,
) {
    if !targeting.open {
        for entity in &ui {
            commands.entity(entity).despawn();
        }
        return;
    }
    if let Ok(mut node) = bar.single_mut() {
        node.width = Val::Percent(100.0 * (targeting.left / TARGET_SECONDS).clamp(0.0, 1.0));
        return;
    }
    let hint = DeviceHint::new(
        "CLICK - STRIKE     WHEEL - TURN     RIGHT CLICK / ESC - CANCEL",
        "{FIRE} - STRIKE     {BUMPERS} - TURN     {B} - CANCEL",
    )
    .with_touch("DRAG - MOVE     TWO FINGERS OR < > - TURN");
    commands
        .spawn((
            TargetingUi,
            RunScoped,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(10.0),
                right: Val::Percent(10.0),
                // Сверху, под временем забега: внизу субтитры штаба.
                top: Val::Px(80.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(8.0),
                ..default()
            },
        ))
        .with_children(|ui| {
            ui.spawn((
                Text::new("AIRSTRIKE - CHOOSE TARGET"),
                TextFont {
                    font_size: FontSize::Px(24.0),
                    ..default()
                },
                TextColor(Color::srgb(1.0, 0.85, 0.2)),
            ));
            ui.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(8.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
            ))
            .with_children(|track| {
                track.spawn((
                    TargetingBar,
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(1.0, 0.85, 0.2)),
                ));
            });
            ui.spawn((
                Text::new(hint.keys.clone()),
                TextFont {
                    font_size: FontSize::Px(18.0),
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.85, 0.85)),
                hint,
            ));
        });
}

/// Сегменты контура отметки на рельефе.
fn outline(terrain: &Terrain, center: Vec3, dir: Vec3, bombs: usize) -> Vec<Transform> {
    let rect = footprint(center, dir, bombs);
    let h = rect.half;
    let segments = MARK_SEGMENTS * bombs / BOMBS;
    let edges = [
        (Vec2::new(-h.x, -h.y), Vec2::new(-h.x, h.y), segments),
        (Vec2::new(h.x, -h.y), Vec2::new(h.x, h.y), segments),
        (Vec2::new(-h.x, -h.y), Vec2::new(h.x, -h.y), 2),
        (Vec2::new(-h.x, h.y), Vec2::new(h.x, h.y), 2),
    ];
    let mut out = Vec::new();
    for (a, b, count) in edges {
        for i in 0..count {
            let p0 = a.lerp(b, i as f32 / count as f32);
            let p1 = a.lerp(b, (i as f32 + 0.7) / count as f32);
            let (w0, w1) = (rect.to_world(p0), rect.to_world(p1));
            let mid = (w0 + w1) * 0.5;
            let along = (w1 - w0).extend(0.0);
            let y = terrain.height_at(mid.x, mid.y) + 0.15;
            out.push(
                Transform::from_xyz(mid.x, y, mid.y)
                    .with_rotation(Quat::from_rotation_y((-along.y).atan2(along.x)))
                    .with_scale(Vec3::new((w1 - w0).length(), 0.15, 0.35)),
            );
        }
    }
    out
}

/// Удар: отметка на земле по контуру, сигнальный дым и сам [`Strike`].
fn spawn_strike(
    commands: &mut Commands,
    assets: &AirstrikeAssets,
    terrain: &Terrain,
    center: Vec3,
    dir: Vec3,
    bombs: usize,
    delay: f32,
) {
    let strike = commands.spawn(RunScoped).id();
    // Отметка: сегменты по контуру прямоугольника, каждый на рельефе.
    let mut marks: Vec<Entity> = outline(terrain, center, dir, bombs)
        .into_iter()
        .map(|transform| {
            commands
                .spawn((
                    Mark {
                        strike,
                        base_height: 0.15,
                    },
                    RunScoped,
                    NotShadowCaster,
                    Mesh3d(assets.cube.clone()),
                    MeshMaterial3d(assets.safe.clone()),
                    transform,
                ))
                .id()
        })
        .collect();
    // Сигнальный дым — светящийся столб в центре.
    marks.push(
        commands
            .spawn((
                Mark {
                    strike,
                    base_height: 3.0,
                },
                RunScoped,
                NotShadowCaster,
                Mesh3d(assets.sphere.clone()),
                MeshMaterial3d(assets.smoke.clone()),
                Transform::from_xyz(center.x, center.y + 1.5, center.z).with_scale(Vec3::new(0.8, 3.0, 0.8)),
            ))
            .id(),
    );
    commands.entity(strike).insert(Strike {
        center,
        dir,
        bombs,
        elapsed: -delay,
        plane_spawned: false,
        plane: None,
        shadow: None,
        dropped: 0,
        landed: 0,
        marks,
    });
}

/// Отметка пульсирует и краснеет, если луноход внутри неё.
fn pulse_marks(
    time: Res<Time>,
    assets: Res<AirstrikeAssets>,
    strikes: Query<&Strike>,
    rover: Single<&Transform, (With<Rover>, Without<Mark>)>,
    mut marks: Query<(&Mark, &mut Transform, &mut MeshMaterial3d<StandardMaterial>), Without<Rover>>,
) {
    let pulse = 1.0 + (time.elapsed_secs() * 10.0).sin() * 0.25;
    for (mark, mut transform, mut material) in &mut marks {
        let Ok(strike) = strikes.get(mark.strike) else {
            continue;
        };
        let inside = footprint(strike.center, strike.dir, strike.bombs).distance(Vec2::new(rover.translation.x, rover.translation.z)) < 1.5;
        let wanted = if inside { &assets.danger } else { &assets.safe };
        if material.0 != *wanted {
            material.0 = wanted.clone();
        }
        transform.scale.y = mark.base_height * pulse;
    }
}

/// Самолёт заходит, пролетает над отметкой и сбрасывает бомбы над её точками.
fn run_strikes(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    assets: Res<AirstrikeAssets>,
    mut strikes: Query<(Entity, &mut Strike)>,
    mut transforms: Query<&mut Transform>,
) {
    let dt = time.delta_secs();
    for (entity, mut strike) in &mut strikes {
        let approach = approach(strike.bombs);
        strike.elapsed += dt;
        if strike.elapsed < DELAY {
            continue;
        }
        let flown = (strike.elapsed - DELAY) * PLANE_SPEED - approach;
        let ground = strike.center + strike.dir * flown;
        let plane_pos = Vec3::new(ground.x, strike.center.y + PLANE_HEIGHT, ground.z);
        let rotation = Quat::from_rotation_arc(Vec3::NEG_Z, strike.dir);
        if !strike.plane_spawned {
            strike.plane_spawned = true;
            let plane = commands
                .spawn((
                    RunScoped,
                    Transform::from_translation(plane_pos)
                        .with_rotation(rotation)
                        .with_scale(Vec3::splat(PLANE_SCALE)),
                    Visibility::default(),
                ))
                .with_children(|p| {
                    // Фюзеляж, крылья, хвост.
                    for (offset, scale) in [
                        (Vec3::ZERO, Vec3::new(1.4, 1.2, 9.0)),
                        (Vec3::new(0.0, 0.0, 0.5), Vec3::new(12.0, 0.3, 2.4)),
                        (Vec3::new(0.0, 1.0, 4.0), Vec3::new(0.3, 2.2, 1.6)),
                        (Vec3::new(0.0, 0.2, 4.0), Vec3::new(4.5, 0.25, 1.2)),
                    ] {
                        p.spawn((
                            Mesh3d(assets.cube.clone()),
                            MeshMaterial3d(assets.plane.clone()),
                            Transform::from_translation(offset).with_scale(scale),
                        ));
                    }
                })
                .id();
            let shadow = commands
                .spawn((
                    RunScoped,
                    NotShadowCaster,
                    Mesh3d(assets.cube.clone()),
                    MeshMaterial3d(assets.shadow.clone()),
                    Transform::default(),
                ))
                .id();
            strike.plane = Some(plane);
            strike.shadow = Some(shadow);
        }
        if let Some(mut t) = strike.plane.and_then(|p| transforms.get_mut(p).ok()) {
            t.translation = plane_pos;
            t.rotation = rotation;
        }
        if let Some(mut t) = strike.shadow.and_then(|p| transforms.get_mut(p).ok()) {
            let y = terrain.height_at(plane_pos.x, plane_pos.z) + 0.1;
            *t = Transform::from_xyz(plane_pos.x, y, plane_pos.z)
                .with_rotation(rotation)
                .with_scale(Vec3::new(10.0 * PLANE_SCALE, 0.05, 8.0 * PLANE_SCALE));
        }
        // Бомба сбрасывается над своей точкой отметки с упреждением на падение.
        let lead = PLANE_SPEED * fall_time();
        while strike.dropped < strike.bombs {
            if flown + lead < bomb_offset(strike.dropped, strike.bombs) {
                break;
            }
            strike.dropped += 1;
            commands.spawn((
                Bomb {
                    velocity: strike.dir * PLANE_SPEED,
                    strike: entity,
                },
                RunScoped,
                Mesh3d(assets.sphere.clone()),
                MeshMaterial3d(assets.bomb.clone()),
                Transform::from_translation(plane_pos - Vec3::Y).with_scale(Vec3::new(0.7, 0.7, 1.4)),
            ));
        }
        if flown > approach {
            for e in [strike.plane.take(), strike.shadow.take()].into_iter().flatten() {
                commands.entity(e).try_despawn();
            }
        }
        // Отметка и сам удар живут, пока не упала последняя бомба.
        if strike.landed >= strike.bombs {
            for mark in strike.marks.drain(..) {
                commands.entity(mark).try_despawn();
            }
            if strike.plane.is_none() {
                commands.entity(entity).despawn();
            }
        }
    }
}

/// Бомбы падают по баллистике и взрываются о землю.
fn drop_bombs(
    mut commands: Commands,
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut bombs: Query<(Entity, &mut Transform, &mut Bomb)>,
    mut strikes: Query<&mut Strike>,
    mut explosions: MessageWriter<Explosion>,
) {
    let dt = time.delta_secs();
    for (entity, mut transform, mut bomb) in &mut bombs {
        bomb.velocity.y -= GRAVITY * dt;
        transform.translation += bomb.velocity * dt;
        transform.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, bomb.velocity.normalize_or(Vec3::NEG_Y));
        let p = transform.translation;
        let ground = terrain.height_at(p.x, p.z);
        if p.y <= ground {
            commands.entity(entity).despawn();
            if let Ok(mut strike) = strikes.get_mut(bomb.strike) {
                strike.landed += 1;
            }
            explosions.write(Explosion {
                position: Vec3::new(p.x, ground, p.z),
                radius: BOMB_RADIUS,
                power: BOMB_POWER,
                source_team: Team::Player,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Самолёт появляется раньше точки сброса первой бомбы, и в первом кадре не падает ни одна.
    #[test]
    fn plane_appears_before_the_first_drop_point() {
        let lead = PLANE_SPEED * fall_time();
        for bombs in [BOMBS, upgrades::airstrike(2).bombs, upgrades::airstrike(3).bombs] {
            let flown_at_start = -approach(bombs);
            assert!(flown_at_start + lead < bomb_offset(0, bombs));
            // Бомбы сбрасываются по очереди с шагом по пролёту, а не все сразу.
            assert!(bomb_offset(1, bombs) - bomb_offset(0, bombs) > 0.0);
            // Серия симметрична относительно точки прицела: линии крест-накрест пересекаются в ней.
            assert!((bomb_offset(0, bombs) + bomb_offset(bombs - 1, bombs)).abs() < 1e-4);
        }
        assert!(approach(BOMBS) >= 110.0, "{}", approach(BOMBS));
        assert_eq!(mark_length(BOMBS), 30.0);
    }
}
