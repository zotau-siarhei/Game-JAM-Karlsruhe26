//! Отладочные снимки: игра сама начинает забег, ставит сцену и сохраняет кадр.
//!
//! Включается переменными окружения, без них плагин ничего не добавляет:
//! - `SPACE_GUARD_SHOT=путь.png` — снимок окна через `SPACE_GUARD_SHOT_AT` секунд (по умолчанию 8)
//!   реального времени, затем выход;
//! - `SPACE_GUARD_AUTOSTART=1` — забег начинается сам, как по Enter;
//! - `SPACE_GUARD_SCENE=fire` — через 2 с забега перед луноходом встают огни трёх размеров,
//!   взрывы и горящие обломки;
//! - `SPACE_GUARD_SCENE=enemies` — через 2 с забега перед луноходом в ряд встают враги всех
//!   видов, кроме босса, а по бокам лунохода золотой ящик и трофей: сравнить их размер с
//!   луноходом (снимок на `SPACE_GUARD_SHOT_AT=3`);
//! - `SPACE_GUARD_SCENE=shock` — перед луноходом встаёт перехватчик, а луноход всё время под
//!   разрядом перехватчика: видны молнии на корпусе и электроды на носу;
//! - `SPACE_GUARD_SCENE=drive` — луноход сам едет с газом и на 3–5-й секунде поворачивает налево:
//!   посмотреть колеи (`tracks`).
//! - `SPACE_GUARD_SCENE=train` — через 2 с забега луноход переносится на 24 м от ближайшего пути,
//!   поезд выходит через секунду: посмотреть вагоны и крутящиеся колёса (`trains`).
//! - `SPACE_GUARD_SCENE=airstrike` — на 3-й секунде забега луноход получает авиаудар и открывает
//!   наведение: посмотреть вид сверху, отметку и полосу времени (`airstrike::Targeting`).
//! - `SPACE_GUARD_SCENE=weapons` — оружие `SPACE_GUARD_WEAPON` (mg, mortar, rockets, homing, laser,
//!   airstrike) уровня `SPACE_GUARD_LEVEL` (1–3) стреляет вперёд на 18 м каждые 0,7 с с 3-й секунды:
//!   посмотреть оружие по уровням (`upgrades`). Авиаудар подтверждается следующим нажатием.
//! - `SPACE_GUARD_SCENE=gadflies` — на 2-й секунде забега у лунохода появляются два дрона-овода:
//!   кружат, по очереди разгоняют ядро и бросаются (`interceptor::Gadfly`).
//! - `SPACE_GUARD_SCENE=lava` — через 2 с забега в 14 м перед луноходом раз в 1,5 с бьёт
//!   взрыв мортиры первого уровня, 8 раз в одну точку; глубина кратера после каждого — в лог:
//!   сколько выстрелов нужно до прорыва коры (`lava::CRUST`).
//! - `SPACE_GUARD_SCENE=collapse` — через 2 с забега луноход встаёт в 9 м от ближайшего дома
//!   лицом к нему, на 5-й секунде дом рушится: посмотреть куски модели (`fragments`), снимок
//!   на `SPACE_GUARD_SHOT_AT` (обрушение — около 6,5, руины — 10).
//! - `SPACE_GUARD_SCENE=pieces` — через 2 с забега перед луноходом раскладываются куски
//!   нарезанных моделей (`fragments`), каждая модель разнесена от центра и уменьшена до ~5 м,
//!   и в ряд — формы библиотеки обломков (`debris_forms`) по вариантам. `SPACE_GUARD_PIECES` —
//!   через запятую идентификаторы моделей (или `forms`), иначе всё.
//! - `SPACE_GUARD_SCENE=wrecks` — через 2 с перед луноходом встают враги всех видов и машины
//!   Службы, на 4-й секунде все гибнут: посмотреть куски корпуса и колёса.
//! - `SPACE_GUARD_SCENE=derail` — как `train`, но первый вагон, подошедший ближе 30 м, гибнет:
//!   состав сходит, локомотив и цистерны взрываются кусками корпуса и листами.
//! - `SPACE_GUARD_SCENE=rocks` — на 3-й секунде забега три ближайших к луноходу валуна и кварца
//!   разбиваются: куски модели и глыбы с осколками.
//! - `SPACE_GUARD_SCENE=boss` — босс выходит сразу, после посадки встаёт в 26 м от лунохода
//!   и гибнет: куски корпуса и ноги.
//! - `SPACE_GUARD_SCENE=gameover` — на 2-й секунде забега луноход гибнет: экран конца забега, ввод
//!   имени рекорда (с `--touch` или на телефоне — экранная клавиатура).
//! - `SPACE_GUARD_MENU=difficulty|settings|records|records-hard|authors|online` — меню сразу открывает
//!   этот подэкран (`menu`): снять его без рук.
//! - `SPACE_GUARD_KEYS=down,down,enter,left` — через секунду после входа в меню клавиши нажимаются
//!   по одной раз в 0,4 с (`up`, `down`, `left`, `right`, `enter`, `space`, `esc`, `q`): пройти
//!   меню без рук.

use bevy::prelude::*;
use bevy::render::view::window::screenshot::{Screenshot, save_to_disk};

use crate::combat::Team;
use crate::debris::SpawnDebris;
use crate::enemy::{EnemyKind, SpawnEnemy, SpawnPlace};
use crate::explosion::Explosion;
use crate::fire::Fire;
use crate::frame::GameSet;
use crate::pickups::{SpawnLoot, SpawnUpgrade};
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
        let menu = std::env::var("SPACE_GUARD_MENU").ok();
        let keys: Vec<KeyCode> = std::env::var("SPACE_GUARD_KEYS")
            .unwrap_or_default()
            .split(',')
            .filter_map(|name| match name.trim() {
                "up" => Some(KeyCode::ArrowUp),
                "down" => Some(KeyCode::ArrowDown),
                "left" => Some(KeyCode::ArrowLeft),
                "right" => Some(KeyCode::ArrowRight),
                "enter" => Some(KeyCode::Enter),
                "space" => Some(KeyCode::Space),
                "esc" => Some(KeyCode::Escape),
                "q" => Some(KeyCode::KeyQ),
                _ => None,
            })
            .collect();
        if shot.is_none() && !autostart && scene.is_none() && menu.is_none() && keys.is_empty() {
            return;
        }
        let at = std::env::var("SPACE_GUARD_SHOT_AT").ok().and_then(|s| s.parse().ok()).unwrap_or(8.0);
        app.insert_resource(DebugShot {
            path: shot,
            at,
            autostart,
            scene,
            menu,
            keys,
            taken: None,
            staged: false,
        })
        .add_systems(Update, (open_menu_page, press_keys).before(GameSet::Screens))
        .add_systems(Update, (press_start, stage_scene, stage_train, stage_collapse, stage_lava, stage_airstrike, stage_weapons, stage_gadflies, stage_pieces, stage_wrecks, stage_gameover, keep_shocked, drive, take_shot).before(GameSet::Screens));
    }
}

#[derive(Resource)]
struct DebugShot {
    path: Option<String>,
    at: f32,
    autostart: bool,
    scene: Option<String>,
    /// Подэкран меню, который открыть сразу.
    menu: Option<String>,
    /// Клавиши, которые нажать по очереди после входа в меню.
    keys: Vec<KeyCode>,
    /// Когда снимок заказан, по реальному времени.
    taken: Option<f32>,
    staged: bool,
}

/// Открывает подэкран меню из `SPACE_GUARD_MENU` при первом входе в меню.
fn open_menu_page(
    debug: Res<DebugShot>,
    state: Res<State<GameState>>,
    mut menu: ResMut<crate::menu::Menu>,
    mut done: Local<bool>,
) {
    use crate::difficulty::Difficulty;
    use crate::menu::MenuPage;
    if *done || *state.get() != GameState::Menu {
        return;
    }
    *done = true;
    let page = match debug.menu.as_deref() {
        Some("difficulty") => MenuPage::Difficulty,
        Some("settings") => MenuPage::Settings,
        Some("records") => MenuPage::Records(Difficulty::Normal),
        Some("records-hard") => MenuPage::Records(Difficulty::Hard),
        Some("authors") => MenuPage::Authors,
        Some("online") => MenuPage::Online,
        _ => return,
    };
    menu.page = page;
}

/// Нажимает клавиши из `SPACE_GUARD_KEYS` по одной: каждая держится один кадр.
fn press_keys(
    time: Res<Time<Real>>,
    debug: Res<DebugShot>,
    state: Res<State<GameState>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut start: Local<Option<f32>>,
    mut held: Local<Option<KeyCode>>,
    mut next: Local<usize>,
) {
    if let Some(key) = held.take() {
        keys.release(key);
    }
    let now = time.elapsed_secs();
    if start.is_none() && *state.get() == GameState::Menu {
        *start = Some(now + 1.0);
    }
    let Some(begin) = *start else {
        return;
    };
    if *next < debug.keys.len() && now >= begin + 0.4 * *next as f32 {
        let key = debug.keys[*next];
        info!("Отладка: клавиша {key:?}");
        keys.press(key);
        *held = Some(key);
        *next += 1;
    }
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
    state: Res<State<GameState>>,
    mut done: Local<bool>,
    mut clock: Local<f32>,
) {
    if *done || !matches!(debug.scene.as_deref(), Some("train" | "derail")) || *state.get() != GameState::Playing {
        return;
    }
    // Время с начала забега: перенос до старта забега сбрасывается и оставляет чёрный экран.
    *clock += time.delta_secs();
    if *clock < 2.0 {
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

/// Сцена `gadflies`: два дрона рядом с луноходом сразу становятся оводами.
fn stage_gadflies(
    mut commands: Commands,
    time: Res<Time>,
    debug: Res<DebugShot>,
    state: Res<State<GameState>>,
    rover: Query<&Transform, With<Rover>>,
    fresh: Query<Entity, Added<crate::interceptor::Interceptor>>,
    mut spawns: MessageWriter<SpawnEnemy>,
    mut clock: Local<f32>,
    mut done: Local<bool>,
) {
    if debug.scene.as_deref() != Some("gadflies") || *state.get() != GameState::Playing {
        return;
    }
    for (i, drone) in fresh.iter().enumerate() {
        commands.entity(drone).try_insert(crate::interceptor::Gadfly::Circle(i as f32 * std::f32::consts::PI));
    }
    *clock += time.delta_secs();
    if *done || *clock < 2.0 {
        return;
    }
    let Some(rover) = rover.iter().next() else {
        return;
    };
    *done = true;
    for x in [-14.0, 14.0] {
        spawns.write(SpawnEnemy {
            place: SpawnPlace::Point(rover.translation + Vec3::new(x, 0.0, 0.0)),
            kind: EnemyKind::Interceptor,
        });
    }
}

/// Сцена `weapons`: выбранное оружие нужного уровня стреляет вперёд.
#[allow(clippy::too_many_arguments)]
fn stage_weapons(
    time: Res<Time<Real>>,
    debug: Res<DebugShot>,
    state: Res<State<GameState>>,
    mut inventory: ResMut<crate::arsenal::Inventory>,
    mut levels: ResMut<crate::upgrades::WeaponLevels>,
    mut weapon: ResMut<crate::weapons::CurrentWeapon>,
    mut aim: ResMut<crate::aim::AimPoint>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    terrain: Res<Terrain>,
    rover: Query<&Transform, With<Rover>>,
    mut since: Local<Option<f32>>,
    mut last_shot: Local<f32>,
) {
    use crate::weapons::CurrentWeapon as W;
    if debug.scene.as_deref() != Some("weapons") || *state.get() != GameState::Playing {
        return;
    }
    let now = time.elapsed_secs();
    let start = *since.get_or_insert(now);
    if now - start < 3.0 {
        return;
    }
    let chosen = match std::env::var("SPACE_GUARD_WEAPON").as_deref() {
        Ok("mortar") => W::Mortar,
        Ok("rockets") => W::Rockets,
        Ok("homing") => W::HomingRockets,
        Ok("laser") => W::Laser,
        Ok("airstrike") => W::Airstrike,
        _ => W::MachineGun,
    };
    let level = std::env::var("SPACE_GUARD_LEVEL").ok().and_then(|l| l.parse().ok()).unwrap_or(3);
    if levels.level(chosen) != level {
        levels.set(chosen, level);
        inventory.add(chosen, 50);
    }
    *weapon = chosen;
    if let Some(rover) = rover.iter().next() {
        let p = rover.translation + rover.forward().with_y(0.0).normalize_or(Vec3::NEG_Z) * 18.0;
        aim.position = Vec3::new(p.x, terrain.height_at(p.x, p.z), p.z);
    }
    // Пулемёт и лазер держат спуск, остальное — нажатие раз в 0,7 с.
    if matches!(chosen, W::MachineGun | W::Laser) {
        mouse.press(MouseButton::Left);
    } else if mouse.pressed(MouseButton::Left) {
        mouse.release(MouseButton::Left);
    } else if now - *last_shot > 0.7 {
        *last_shot = now;
        mouse.press(MouseButton::Left);
    }
}

/// Сцена `airstrike`: авиаудар в руки и открытое наведение.
fn stage_airstrike(
    time: Res<Time<Real>>,
    debug: Res<DebugShot>,
    state: Res<State<GameState>>,
    mut inventory: ResMut<crate::arsenal::Inventory>,
    mut weapon: ResMut<crate::weapons::CurrentWeapon>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut since: Local<Option<f32>>,
    mut step: Local<u8>,
) {
    if debug.scene.as_deref() != Some("airstrike") || *state.get() != GameState::Playing {
        return;
    }
    let now = time.elapsed_secs();
    let start = *since.get_or_insert(now);
    if *step == 0 && now - start > 2.5 {
        *step = 1;
        inventory.add(crate::weapons::CurrentWeapon::Airstrike, 2);
        *weapon = crate::weapons::CurrentWeapon::Airstrike;
    } else if *step == 1 && now - start > 3.0 {
        *step = 2;
        mouse.press(MouseButton::Left);
    } else if *step == 2 {
        *step = 3;
        mouse.release(MouseButton::Left);
    }
}

/// Сцена `gameover`: луноход гибнет на 2-й секунде забега.
fn stage_gameover(
    time: Res<Time<Real>>,
    debug: Res<DebugShot>,
    state: Res<State<GameState>>,
    rover: Query<Entity, With<Rover>>,
    mut damage: MessageWriter<crate::combat::Damage>,
    mut since: Local<Option<f32>>,
    mut done: Local<bool>,
) {
    if *done || debug.scene.as_deref() != Some("gameover") || *state.get() != GameState::Playing {
        return;
    }
    let now = time.elapsed_secs();
    if now - *since.get_or_insert(now) < 2.0 {
        return;
    }
    if let Ok(target) = rover.single() {
        *done = true;
        info!("Сцена gameover: луноход гибнет");
        damage.write(crate::combat::Damage { target, amount: 1.0e7, source_team: Team::Enemy });
    }
}

/// Сцена `collapse`: луноход встаёт у ближайшего дома, потом дом рушится.
fn stage_collapse(
    time: Res<Time>,
    debug: Res<DebugShot>,
    terrain: Res<Terrain>,
    mut rover: Query<&mut Transform, With<Rover>>,
    houses: Query<(Entity, &Transform), (With<crate::buildings::House>, Without<Rover>)>,
    mut damage: MessageWriter<crate::combat::Damage>,
    state: Res<State<GameState>>,
    mut step: Local<u8>,
    mut clock: Local<f32>,
) {
    if debug.scene.as_deref() != Some("collapse") || *state.get() != GameState::Playing {
        return;
    }
    // Время с начала забега: загрузка и меню идут дольше, чем ждёт сцена.
    *clock += time.delta_secs();
    let Some(mut rover) = rover.iter_mut().next() else {
        return;
    };
    let t = *clock;
    let nearest = houses
        .iter()
        .min_by(|a, b| {
            let d = |h: &Transform| h.translation.distance_squared(rover.translation);
            d(a.1).total_cmp(&d(b.1))
        })
        .map(|(e, h)| (e, h.translation));
    let Some((house, at)) = nearest else {
        return;
    };
    if *step == 0 && t >= 2.0 {
        *step = 1;
        // Сбоку от дома, лицом к нему; перёд лунохода — −Z.
        let away = (rover.translation - at).with_y(0.0).normalize_or(Vec3::X);
        let p = at + away * 9.0;
        rover.translation = Vec3::new(p.x, terrain.height_at(p.x, p.z) + 0.5, p.z);
        rover.rotation = Quat::from_rotation_y(away.x.atan2(away.z));
    } else if *step == 1 && t >= 5.0 {
        *step = 2;
        info!("Сцена collapse: дом рушится");
        damage.write(crate::combat::Damage {
            target: house,
            amount: 1.0e4,
            source_team: Team::Player,
        });
    }
}

/// Сцена `pieces`: куски моделей и формы библиотеки разложены перед луноходом.
fn stage_pieces(
    mut commands: Commands,
    time: Res<Time>,
    debug: Res<DebugShot>,
    state: Res<State<GameState>>,
    terrain: Res<Terrain>,
    pieces: Res<crate::fragments::ModelPieces>,
    rover: Query<&Transform, With<Rover>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut clock: Local<f32>,
    mut done: Local<bool>,
) {
    if *done || debug.scene.as_deref() != Some("pieces") || *state.get() != GameState::Playing {
        return;
    }
    *clock += time.delta_secs();
    let Some(rover) = rover.iter().next() else {
        return;
    };
    if *clock < 2.0 {
        return;
    }
    *done = true;
    let wanted = std::env::var("SPACE_GUARD_PIECES").ok();
    let wants = |id: &str| wanted.as_deref().is_none_or(|w| w.split(',').any(|x| x.trim() == id));
    let mut sets: Vec<_> = pieces.all().filter(|(id, _)| wants(id)).collect();
    sets.sort_by_key(|(id, _)| *id);
    info!("Сцена pieces: моделей {}", sets.len());
    let columns = 5usize;
    let pitch = 7.0;
    let origin = rover.translation + Vec3::new(-pitch * 2.5, 0.0, -2.0);
    for (n, (id, set)) in sets.iter().enumerate() {
        let cell = origin + Vec3::new((n % columns) as f32 * pitch, 0.0, -((n / columns) as f32) * pitch);
        let locals: Vec<Transform> = set.pieces.iter().map(|p| set.local(p, Vec3::ONE)).collect();
        let reach = set
            .pieces
            .iter()
            .zip(&locals)
            .map(|(p, l)| l.translation.length() + (p.half * l.scale).length())
            .fold(0.1, f32::max);
        let fit = 2.6 / reach;
        let ground = terrain.height_at(cell.x, cell.z);
        info!("Сцена pieces: «{id}» — {} кусков", set.pieces.len());
        for (piece, local) in set.pieces.iter().zip(&locals) {
            // Разнесено от центра в полтора раза.
            let at = local.translation * fit;
            let spread = at + at.with_y(0.0).normalize_or_zero() * 0.6 + Vec3::Y * at.y * 0.4;
            commands.spawn((
                RunScoped,
                Mesh3d(piece.mesh.clone()),
                MeshMaterial3d(set.material.clone()),
                Transform::from_translation(Vec3::new(cell.x, ground + 1.0, cell.z) + spread)
                    .with_rotation(local.rotation)
                    .with_scale(local.scale * fit),
            ));
        }
    }
    if !wants("forms") {
        return;
    }
    // Формы библиотеки: ряд на вид, столбец на вариант.
    let colors = [
        Color::srgb(0.5, 0.49, 0.47),
        Color::srgb(0.85, 0.9, 0.95),
        Color::srgb(0.45, 0.3, 0.16),
        Color::srgb(0.35, 0.36, 0.38),
        Color::srgb(0.42, 0.33, 0.24),
        Color::srgb(0.6, 0.57, 0.5),
    ];
    let sizes = [
        Vec3::new(0.9, 0.7, 0.8),
        Vec3::new(0.35, 1.0, 0.35),
        Vec3::new(0.25, 0.15, 1.4),
        Vec3::new(1.3, 0.4, 1.0),
        Vec3::new(0.8, 0.5, 0.7),
        Vec3::new(0.35, 0.35, 0.35),
    ];
    let start = rover.translation + Vec3::new(4.0, 0.0, -12.0);
    for (row, kind) in crate::debris_forms::FormKind::ALL.into_iter().enumerate() {
        let material = materials.add(StandardMaterial {
            base_color: colors[row],
            perceptual_roughness: if kind.glossy() { 0.3 } else { 0.9 },
            ..default()
        });
        for variant in 0..crate::debris_forms::VARIANTS {
            let p = start + Vec3::new(variant as f32 * 3.0, 0.0, row as f32 * 3.0);
            let y = terrain.height_at(p.x, p.z) + 0.8;
            commands.spawn((
                RunScoped,
                Mesh3d(meshes.add(crate::debris_forms::build(kind, variant))),
                MeshMaterial3d(material.clone()),
                Transform::from_xyz(p.x, y, p.z)
                    // Наклон к камере: видно и бок, и верх.
                    .with_rotation(Quat::from_rotation_x(0.7) * Quat::from_rotation_y(0.6))
                    .with_scale(sizes[row] * 2.0),
            ));
        }
    }
}

/// Сцены `wrecks` и `boss`: техника встаёт перед луноходом и гибнет.
#[allow(clippy::too_many_arguments)]
fn stage_wrecks(
    time: Res<Time>,
    debug: Res<DebugShot>,
    state: Res<State<GameState>>,
    terrain: Res<Terrain>,
    rover: Query<&Transform, With<Rover>>,
    mut vehicles: Query<
        (Entity, &mut Transform, Has<crate::boss::Boss>),
        (Or<(With<crate::enemy::Enemy>, With<crate::service::ServiceVehicle>)>, Without<Rover>),
    >,
    mut enemies: MessageWriter<SpawnEnemy>,
    mut service: MessageWriter<crate::service::SpawnService>,
    mut boss: MessageWriter<crate::boss::SpawnBoss>,
    mut damage: MessageWriter<crate::combat::Damage>,
    cars: Query<(Entity, &GlobalTransform), With<crate::trains::Car>>,
    props: Query<(Entity, &crate::props::Prop, &GlobalTransform), With<crate::combat::Health>>,
    mut clock: Local<f32>,
    mut step: Local<u8>,
) {
    let scene = debug.scene.as_deref();
    if !matches!(scene, Some("wrecks" | "boss" | "derail" | "rocks")) || *state.get() != GameState::Playing {
        return;
    }
    *clock += time.delta_secs();
    let Some(rover) = rover.iter().next().copied() else {
        return;
    };
    let ground = |x: f32, z: f32| {
        let p = rover.translation + Vec3::new(x, 0.0, z);
        Vec3::new(p.x, terrain.height_at(p.x, p.z), p.z)
    };
    let t = *clock;
    let kill = |damage: &mut MessageWriter<crate::combat::Damage>, target: Entity| {
        damage.write(crate::combat::Damage {
            target,
            amount: 1.0e5,
            source_team: Team::Player,
        });
    };
    if scene == Some("rocks") {
        // Три ближайших валуна и кварца разбиваются.
        if *step == 0 && t >= 3.0 {
            *step = 1;
            use crate::props::PropKind;
            let mut near: Vec<_> = props
                .iter()
                .filter(|(_, p, _)| matches!(p.kind, PropKind::Boulder | PropKind::Crystal))
                .map(|(e, _, g)| (e, g.translation().distance(rover.translation)))
                .collect();
            near.sort_by(|a, b| a.1.total_cmp(&b.1));
            info!("Сцена rocks: разбиваются {}", near.len().min(3));
            for (entity, _) in near.into_iter().take(3) {
                kill(&mut damage, entity);
            }
        }
        return;
    }
    if scene == Some("derail") {
        // Первый вагон, подошедший ближе 30 м, гибнет: состав сходит, локомотив взрывается.
        if *step == 0 && t >= 2.5 {
            let near = cars
                .iter()
                .map(|(e, g)| (e, g.translation().distance(rover.translation)))
                .filter(|(_, d)| *d < 30.0)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((car, _)) = near {
                *step = 1;
                info!("Сцена derail: вагон гибнет");
                kill(&mut damage, car);
            }
        }
        return;
    }
    if scene == Some("boss") {
        if *step == 0 && t >= 1.0 {
            *step = 1;
            boss.write(crate::boss::SpawnBoss);
        } else if *step == 1 && t >= 6.0 {
            for (entity, mut transform, is_boss) in &mut vehicles {
                if is_boss {
                    *step = 2;
                    let p = ground(0.0, -26.0);
                    transform.translation.x = p.x;
                    transform.translation.z = p.z;
                    info!("Сцена boss: босс гибнет");
                    kill(&mut damage, entity);
                }
            }
        }
        return;
    }
    if *step == 0 && t >= 2.0 {
        *step = 1;
        let kinds = [
            EnemyKind::RamShooter,
            EnemyKind::Mortar,
            EnemyKind::HeavyTank,
            EnemyKind::LaserTank,
            EnemyKind::Interceptor,
        ];
        for (i, kind) in kinds.into_iter().enumerate() {
            let place = SpawnPlace::Point(ground(-16.0 + i as f32 * 8.0, -12.0));
            enemies.write(SpawnEnemy { place, kind });
        }
        for kind in [crate::service::ServiceKind::Patrol, crate::service::ServiceKind::Transport, crate::service::ServiceKind::Support] {
            service.write(crate::service::SpawnService { kind, angle: 0.0 });
        }
    } else if *step == 1 && t >= 3.0 {
        *step = 2;
        // Машины Службы появились за кадром: ставим их во второй ряд.
        let mut row = 0;
        for (_, mut transform, is_boss) in &mut vehicles {
            if is_boss || transform.translation.distance(rover.translation) < 30.0 {
                continue;
            }
            let p = ground(-10.0 + row as f32 * 9.0, 3.0);
            transform.translation = p + Vec3::Y * 1.0;
            row += 1;
        }
    } else if *step == 2 && t >= 4.0 {
        *step = 3;
        info!("Сцена wrecks: техника гибнет");
        for (entity, _, is_boss) in &vehicles {
            if !is_boss {
                kill(&mut damage, entity);
            }
        }
    }
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
    mut upgrades: MessageWriter<SpawnUpgrade>,
    mut loot: MessageWriter<SpawnLoot>,
    mut crates: MessageWriter<crate::pickups::SpawnCrate>,
    state: Res<State<GameState>>,
    mut clock: Local<f32>,
) {
    // Время с начала забега: загрузка и меню идут дольше двух секунд.
    if *state.get() != GameState::Playing {
        return;
    }
    *clock += time.delta_secs();
    let enemies_scene = debug.scene.as_deref() == Some("enemies");
    let shock_scene = debug.scene.as_deref() == Some("shock");
    let scene = debug.scene.as_deref();
    if debug.staged || !matches!(scene, Some("fire" | "enemies" | "shock")) || *clock < 2.0 {
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
            EnemyKind::HeavyTank,
            EnemyKind::LaserTank,
            EnemyKind::Interceptor,
        ];
        for (i, kind) in kinds.into_iter().enumerate() {
            let place = SpawnPlace::Point(ground(-16.0 + i as f32 * 8.0, -9.0));
            enemies.write(SpawnEnemy { place, kind });
        }
        // Золотой ящик и трофей рядом с луноходом: видно, какого они размера.
        upgrades.write(SpawnUpgrade { position: ground(-5.0, 2.0) });
        loot.write(SpawnLoot {
            position: ground(5.0, 2.0),
            weapon: crate::weapons::CurrentWeapon::Rockets,
        });
        // Трофеи остальных видов и выброшенные ящики с крышками в цвет оружия.
        for (i, weapon) in [crate::weapons::CurrentWeapon::Mortar, crate::weapons::CurrentWeapon::Laser].into_iter().enumerate() {
            loot.write(SpawnLoot {
                position: ground(8.0 + i as f32 * 3.0, 2.0),
                weapon,
            });
        }
        for (i, weapon) in [
            crate::weapons::CurrentWeapon::Mortar,
            crate::weapons::CurrentWeapon::Rockets,
            crate::weapons::CurrentWeapon::Laser,
            crate::weapons::CurrentWeapon::Airstrike,
        ]
        .into_iter()
        .enumerate()
        {
            crates.write(crate::pickups::SpawnCrate {
                position: ground(-10.0 + i as f32 * 3.0, 5.0) + Vec3::Y * 4.0,
                velocity: Vec3::new(0.0, 3.0, 0.0),
                weapon,
            });
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
            ..default()
        });
    }
}

/// Сцена `shock`: луноход всё время под разрядом перехватчика.
/// Сцена `shock`: луноход всё время под разрядом, ближайший дрон стоит вплотную к его борту
/// и раз в 0,5 с бьёт молнией с электродов.
fn keep_shocked(
    time: Res<Time<Real>>,
    debug: Res<DebugShot>,
    mut rover: Query<(&Transform, &mut crate::rover::Impact), With<Rover>>,
    mut drones: Query<
        (Entity, &mut Transform, &mut crate::vehicle::GroundVehicle, &mut crate::steering::Steering),
        (With<crate::interceptor::Interceptor>, Without<Rover>),
    >,
    mut hits: MessageWriter<crate::interceptor::InterceptorHit>,
    mut last: Local<f32>,
) {
    if debug.scene.as_deref() != Some("shock") {
        return;
    }
    for (transform, mut impact) in &mut rover {
        impact.left = crate::waves::balance::IMPACT_MAX;
        let now = time.elapsed_secs();
        if now - *last < 0.5 {
            continue;
        }
        if let Some((entity, mut drone, mut vehicle, mut steering)) = drones.iter_mut().next() {
            // Вплотную к левому борту, носом к луноходу: курс задаёт и поворот, и посадку.
            let at = transform.translation + Vec3::new(-3.6, 0.0, 0.0);
            drone.translation = at.with_y(drone.translation.y);
            let heading = crate::vehicle::heading_towards(Vec3::X);
            vehicle.heading = heading;
            vehicle.speed = 0.0;
            steering.desired = heading;
            steering.speed = 0.0;
            drone.rotation = Quat::from_rotation_y(heading);
            *last = now;
            hits.write(crate::interceptor::InterceptorHit {
                position: transform.translation,
                prongs: crate::interceptor::prong_tips(&drone),
                drone: entity,
            });
        }
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

/// Сцена `lava`: взрывы мортиры раз за разом в одну точку перед луноходом, глубина — в лог.
fn stage_lava(
    time: Res<Time>,
    debug: Res<DebugShot>,
    terrain: Res<Terrain>,
    state: Res<State<GameState>>,
    rover: Query<&Transform, With<Rover>>,
    mut explosions: MessageWriter<Explosion>,
    pools: Res<crate::lava::LavaPools>,
    mut target: Local<Option<(Vec3, f32)>>,
    mut clock: Local<f32>,
    mut shots: Local<u32>,
) {
    if debug.scene.as_deref() != Some("lava") || *state.get() != GameState::Playing {
        return;
    }
    *clock += time.delta_secs();
    let Some(rover) = rover.iter().next() else {
        return;
    };
    let (at, start) = *target.get_or_insert_with(|| {
        let p = rover.translation + rover.forward() * 14.0;
        (Vec3::new(p.x, 0.0, p.z), terrain.height_at(p.x, p.z))
    });
    let due = 2.0 + *shots as f32 * 1.5;
    if *clock < due {
        return;
    }
    if *shots >= 8 {
        // После серии — состояние озера раз в 2 с.
        if *clock >= due + 0.5 && *shots < 20 {
            *shots += 1;
            info!("Сцена lava: {:.0} с, озёра (уровень, цель, ячеек, плоскость): {:?}", *clock, pools.summary());
        }
        return;
    }
    if *shots > 0 {
        let depth = start - terrain.height_at(at.x, at.z);
        info!("Сцена lava: после {} взрывов глубина {:.1} м, озёра (уровень, ячеек): {:?}", *shots, depth, pools.summary());
    }
    *shots += 1;
    explosions.write(Explosion {
        position: Vec3::new(at.x, terrain.height_at(at.x, at.z), at.z),
        radius: crate::mortar::EXPLOSION_RADIUS,
        power: crate::mortar::EXPLOSION_POWER,
        source_team: Team::Player,
    });
}
