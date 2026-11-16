//! Отладочные снимки: игра сама начинает забег, ставит сцену и сохраняет кадр.
//!
//! Включается переменными окружения, без них плагин ничего не добавляет:
//! - `SPACE_GUARD_SHOT=путь.png` — снимок окна через `SPACE_GUARD_SHOT_AT` секунд (по умолчанию 8)
//!   реального времени, затем выход;
//! - `SPACE_GUARD_AUTOSTART=1` — забег начинается сам, как по Enter;
//! - `SPACE_GUARD_SCENE=fire` — через 2 с забега перед луноходом встают огни трёх размеров,
//!   взрывы и горящие обломки;
//! - `SPACE_GUARD_SCENE=enemies` — через 2 с забега перед луноходом в ряд встают враги всех
//!   видов, кроме босса: сравнить их размер с луноходом (снимок на `SPACE_GUARD_SHOT_AT=3`);
//! - `SPACE_GUARD_SCENE=shock` — перед луноходом встаёт перехватчик, а луноход всё время под
//!   разрядом перехватчика: видны молнии на корпусе и электроды на носу;
//! - `SPACE_GUARD_SCENE=drive` — луноход сам едет с газом и на 3–5-й секунде поворачивает налево:
//!   посмотреть колеи (`tracks`).
//! - `SPACE_GUARD_SCENE=train` — через 2 с забега луноход переносится на 24 м от ближайшего пути,
//!   поезд выходит через секунду: посмотреть вагоны и крутящиеся колёса (`trains`).

use bevy::prelude::*;
use bevy::render::view::window::screenshot::{Screenshot, save_to_disk};

use crate::combat::Team;
use crate::debris::SpawnDebris;
use crate::enemy::{EnemyKind, SpawnEnemy, SpawnPlace};
use crate::explosion::Explosion;
use crate::fire::Fire;
use crate::frame::GameSet;
use crate::rover::Rover;
use crate::run::{GameState, RunScoped};
use crate::terrain::Terrain;

/// Подключает отладочные снимки, если они заданы в окружении.
pub struct DebugShotPlugin;

impl Plugin for DebugShotPlugin {
    fn build(&self, app: &mut App) {
        let shot = std::env::var("SPACE_GUARD_SHOT").ok();
        let autostart = std::env::var("SPACE_GUARD_AUTOSTART").is_ok();
        let scene = std::env::var("SPACE_GUARD_SCENE").ok();
        if shot.is_none() && !autostart && scene.is_none() {
            return;
        }
        let at = std::env::var("SPACE_GUARD_SHOT_AT").ok().and_then(|s| s.parse().ok()).unwrap_or(8.0);
        app.insert_resource(DebugShot {
            path: shot,
            at,
            autostart,
            scene,
            taken: None,
            staged: false,
        })
        .add_systems(Update, (press_start, stage_scene, stage_train, keep_shocked, drive, take_shot).before(GameSet::Screens));
    }
}

#[derive(Resource)]
struct DebugShot {
    path: Option<String>,
    at: f32,
    autostart: bool,
    scene: Option<String>,
    /// Когда снимок заказан, по реальному времени.
    taken: Option<f32>,
    staged: bool,
}

/// Начинает забег, как Enter в меню.
fn press_start(
    time: Res<Time<Real>>,
    debug: Res<DebugShot>,
    state: Res<State<GameState>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
) {
    if debug.autostart && *state.get() == GameState::Menu && time.elapsed_secs() > 1.0 {
        keys.press(KeyCode::Enter);
    }
}

/// Сцена `drive`: газ всё время, с 3-й по 5-ю секунду забега — поворот налево.
fn drive(
    time: Res<Time>,
    debug: Res<DebugShot>,
    state: Res<State<GameState>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut clock: Local<f32>,
) {
    if debug.scene.as_deref() != Some("drive") || *state.get() != GameState::Playing {
        return;
    }
    *clock += time.delta_secs();
    keys.press(KeyCode::KeyW);
    if (3.0..5.0).contains(&*clock) {
        keys.press(KeyCode::KeyA);
    } else {
        keys.release(KeyCode::KeyA);
    }
}

/// Сцена `train`: луноход встаёт сбоку от ближайшего пути, лицом к нему.
fn stage_train(
    time: Res<Time>,
    debug: Res<DebugShot>,
    terrain: Res<Terrain>,
    mut rover: Query<&mut Transform, With<Rover>>,
    mut done: Local<bool>,
) {
    if *done || debug.scene.as_deref() != Some("train") || time.elapsed_secs() < 2.0 {
        return;
    }
    let Some(mut rover) = rover.iter_mut().next() else {
        return;
    };
    *done = true;
    let lines = terrain.lines();
    let near = lines.nearest(Vec2::new(rover.translation.x, rover.translation.z));
    let on = lines.point(near.line, near.s);
    let t = lines.tangent(near.line, near.s);
    let at = on + Vec2::new(-t.y, t.x) * 24.0;
    rover.translation = Vec3::new(at.x, terrain.height_at(at.x, at.y) + 0.5, at.y);
    let to_line = (on - at).normalize_or(Vec2::X);
    // Перёд лунохода — −Z.
    rover.rotation = Quat::from_rotation_y((-to_line.x).atan2(-to_line.y));
}

/// Ставит отладочную сцену перед луноходом.
fn stage_scene(
    mut commands: Commands,
    time: Res<Time>,
    mut debug: ResMut<DebugShot>,
    terrain: Res<Terrain>,
    rover: Query<&Transform, With<Rover>>,
    mut explosions: MessageWriter<Explosion>,
    mut debris: MessageWriter<SpawnDebris>,
    mut enemies: MessageWriter<SpawnEnemy>,
) {
    let enemies_scene = debug.scene.as_deref() == Some("enemies");
    let shock_scene = debug.scene.as_deref() == Some("shock");
    let scene = debug.scene.as_deref();
    if debug.staged || !matches!(scene, Some("fire" | "enemies" | "shock")) || time.elapsed_secs() < 2.0 {
        return;
    }
    let Some(rover) = rover.iter().next() else {
        return;
    };
    debug.staged = true;
    let ground = |x: f32, z: f32| {
        let p = rover.translation + Vec3::new(x, 0.0, z);
        Vec3::new(p.x, terrain.height_at(p.x, p.z), p.z)
    };
    if shock_scene {
        let place = SpawnPlace::Point(ground(-4.0, -4.0));
        enemies.write(SpawnEnemy { place, kind: EnemyKind::Interceptor });
        return;
    }
    if enemies_scene {
        let kinds = [
            EnemyKind::RamShooter,
            EnemyKind::Mortar,
            EnemyKind::Rocketeer,
            EnemyKind::HeavyTank,
            EnemyKind::Interceptor,
        ];
        for (i, kind) in kinds.into_iter().enumerate() {
            let place = SpawnPlace::Point(ground(-16.0 + i as f32 * 8.0, -9.0));
            enemies.write(SpawnEnemy { place, kind });
        }
        return;
    }
    // Огни трёх размеров в ряд перед луноходом.
    for (i, size) in [0.6, 1.5, 3.0].into_iter().enumerate() {
        commands.spawn((
            RunScoped,
            Fire::new(size, 1.0),
            Transform::from_translation(ground(-8.0 + i as f32 * 8.0, -6.0)),
        ));
    }
    explosions.write(Explosion {
        position: ground(10.0, 4.0),
        radius: 4.0,
        power: 1.0,
        source_team: Team::Player,
    });
    for k in 0..4 {
        let p = ground(-10.0 + k as f32 * 2.0, 6.0) + Vec3::Y * 3.0;
        debris.write(SpawnDebris {
            position: p,
            velocity: Vec3::new(1.0, 4.0, 0.5),
            size: Vec3::splat(0.6),
            color: Color::srgb(0.4, 0.3, 0.2),
            burning: true,
        });
    }
}

/// Сцена `shock`: луноход всё время под разрядом перехватчика.
fn keep_shocked(debug: Res<DebugShot>, mut rover: Query<&mut crate::rover::Impact, With<Rover>>) {
    if debug.scene.as_deref() != Some("shock") {
        return;
    }
    for mut impact in &mut rover {
        impact.left = crate::waves::balance::IMPACT_MAX;
    }
}

/// Снимает окно в заданный момент и закрывает игру после записи.
fn take_shot(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut debug: ResMut<DebugShot>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = debug.path.clone() else {
        return;
    };
    let now = time.elapsed_secs();
    match debug.taken {
        None if now >= debug.at => {
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
            debug.taken = Some(now);
        }
        Some(at) if now - at > 1.5 => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}
