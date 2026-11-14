//! Забег: состояния игры, гибель лунохода, экран конца забега и рестарт.
//!
//! Новый забег объявляется сообщением [`RunStarted`]: каждый модуль, который что-то хранит
//! на весь забег, сбрасывает это по сообщению. Сущности, живущие один забег, помечаются
//! [`RunScoped`] и удаляются при рестарте.

use bevy::prelude::*;

use crate::combat::{Died, Health, SurvivalTime, Team, ROVER_MAX_HEALTH};
use crate::explosion::Explosion;
use crate::hud::format_time;
use crate::records::{self, Records};
use crate::rover::{self, Rover, RoverMotion};
use crate::terrain::{ChunkMesh, RunSeed, Terrain, VisibleChunks};

/// Сколько секунд после гибели до экрана конца забега.
const GAME_OVER_DELAY: f32 = 1.0;

/// Взрыв при гибели лунохода: радиус, м, и мощность.
const DEATH_BLAST: (f32, f32) = (9.0, 2.5);

/// Высота центра корпуса лунохода над поверхностью при возвращении на старт, м.
const ROVER_RIDE_HEIGHT: f32 = 0.5;

/// Подключает состояния игры, гибель и рестарт.
pub struct RunPlugin;

impl Plugin for RunPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<GameState>()
            .init_resource::<Death>()
            .init_resource::<RunResult>()
            .insert_resource(RecordsTable::load())
            .add_message::<RunStarted>()
            .add_systems(Startup, announce_first_run)
            .add_systems(
                Update,
                (
                    (watch_rover_death, hold_dead_rover)
                        .chain()
                        .after(rover::drive_rover)
                        .run_if(in_state(GameState::Playing)),
                    restart_on_enter.run_if(in_state(GameState::GameOver)),
                ),
            )
            .add_systems(OnEnter(GameState::GameOver), (record_result, show_game_over).chain())
            .add_systems(OnExit(GameState::GameOver), hide_game_over);
    }
}

/// Состояние игры.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GameState {
    /// Идёт забег.
    #[default]
    Playing,
    /// Луноход погиб, показан экран конца забега.
    GameOver,
}

/// Начался новый забег. Модули сбрасывают по нему всё, что хранили на прошлый.
#[derive(Message, Debug, Clone, Copy)]
pub struct RunStarted {
    // Сид читают модули, которые строят мир: посёлки, пути.
    #[allow(dead_code)]
    pub seed: u32,
}

/// Сущность живёт один забег и удаляется при рестарте: враги, пули, аптечки.
#[derive(Component, Default)]
pub struct RunScoped;

/// Гибель лунохода в текущем забеге.
#[derive(Resource, Default)]
struct Death {
    /// Сколько секунд осталось до экрана конца забега.
    countdown: Option<f32>,
    /// Где луноход погиб: там он и остаётся.
    position: Vec3,
}

/// Итог последнего забега для экрана.
#[derive(Resource, Default)]
struct RunResult {
    seconds: f32,
    /// Место в таблице рекордов, считая с нуля. `None` — не попал.
    place: Option<usize>,
}

/// Таблица рекордов в памяти и путь к её файлу.
#[derive(Resource)]
struct RecordsTable {
    table: Records,
    path: Option<std::path::PathBuf>,
}

impl RecordsTable {
    fn load() -> Self {
        let path = records::records_path();
        let table = path.as_deref().map(Records::load).unwrap_or_default();
        Self { table, path }
    }
}

/// Корень экрана конца забега.
#[derive(Component)]
struct GameOverScreen;

fn announce_first_run(seed: Option<Res<RunSeed>>, mut started: MessageWriter<RunStarted>) {
    started.write(RunStarted {
        seed: seed.map_or(0, |seed| seed.0),
    });
}

/// Гибель лунохода: крупный взрыв, луноход исчезает, через секунду экран конца забега.
fn watch_rover_death(
    time: Res<Time>,
    mut died: MessageReader<Died>,
    mut death: ResMut<Death>,
    mut rover: Query<(Entity, &Transform, &mut Visibility), With<Rover>>,
    mut next: ResMut<NextState<GameState>>,
    mut explosions: MessageWriter<Explosion>,
) {
    let Some((entity, transform, mut visibility)) = rover.iter_mut().next() else {
        return;
    };
    if died.read().any(|message| message.entity == entity) && death.countdown.is_none() {
        death.countdown = Some(GAME_OVER_DELAY);
        death.position = transform.translation;
        *visibility = Visibility::Hidden;
        explosions.write(Explosion {
            position: transform.translation,
            radius: DEATH_BLAST.0,
            power: DEATH_BLAST.1,
            source_team: Team::Player,
        });
    }
    if let Some(countdown) = death.countdown.as_mut() {
        *countdown -= time.delta_secs();
        if *countdown <= 0.0 {
            next.set(GameState::GameOver);
        }
    }
}

/// Погибший луноход не едет: управление его больше не двигает.
fn hold_dead_rover(
    death: Res<Death>,
    mut rover: Query<(&mut Transform, &mut RoverMotion), With<Rover>>,
) {
    if death.countdown.is_none() {
        return;
    }
    for (mut transform, mut motion) in &mut rover {
        transform.translation = death.position;
        motion.speed = 0.0;
    }
}

/// Записывает время забега в таблицу рекордов.
fn record_result(
    survival: Res<SurvivalTime>,
    mut records: ResMut<RecordsTable>,
    mut result: ResMut<RunResult>,
) {
    let name = records.table.last_name.clone();
    result.seconds = survival.0;
    result.place = records.table.insert(&name, f64::from(survival.0));
    if let (Some(_), Some(path)) = (result.place, records.path.as_deref())
        && let Err(error) = records.table.save(path)
    {
        warn!("Рекорды: не записан {}: {error}", path.display());
    }
}

/// Строки экрана конца забега: время, место и лучший результат.
fn result_lines(seconds: f32, place: Option<usize>, best: Option<f64>) -> (String, String) {
    let time = format!("TIME  {}", format_time(seconds));
    let rank = match (place, best) {
        (Some(0), _) => "NEW RECORD!".to_string(),
        (Some(place), Some(best)) => format!("PLACE {}   BEST {}", place + 1, format_time(best as f32)),
        (None, Some(best)) => format!("BEST {}", format_time(best as f32)),
        (_, None) => String::new(),
    };
    (time, rank)
}

fn show_game_over(mut commands: Commands, result: Res<RunResult>, records: Res<RecordsTable>) {
    let best = records.table.best().map(|record| record.seconds);
    let (time, rank) = result_lines(result.seconds, result.place, best);
    let line = |text: String, size: f32, color: Color| {
        (
            Text::new(text),
            TextFont {
                font_size: FontSize::Px(size),
                ..default()
            },
            TextColor(color),
        )
    };
    commands
        .spawn((
            GameOverScreen,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: Val::Px(14.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.62)),
        ))
        .with_children(|screen| {
            screen.spawn(line("ROVER LOST".into(), 56.0, Color::srgb(1.0, 0.35, 0.25)));
            screen.spawn(line(time, 34.0, Color::WHITE));
            if !rank.is_empty() {
                screen.spawn(line(rank, 26.0, Color::srgb(1.0, 0.85, 0.3)));
            }
            screen.spawn(line("ENTER - RESTART".into(), 22.0, Color::srgb(0.8, 0.8, 0.8)));
        });
}

fn hide_game_over(mut commands: Commands, screens: Query<Entity, With<GameOverScreen>>) {
    for screen in &screens {
        commands.entity(screen).despawn();
    }
}

/// Enter начинает новый забег: новый рельеф, целый луноход на старте, никаких следов прошлого.
#[allow(clippy::too_many_arguments)]
fn restart_on_enter(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut next: ResMut<NextState<GameState>>,
    mut started: MessageWriter<RunStarted>,
    mut death: ResMut<Death>,
    mut survival: ResMut<SurvivalTime>,
    mut visible: ResMut<VisibleChunks>,
    scoped: Query<Entity, With<RunScoped>>,
    chunks: Query<Entity, With<ChunkMesh>>,
    mut rover: Query<(&mut Transform, &mut RoverMotion, &mut Health, &mut Visibility), With<Rover>>,
) {
    if !keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]) {
        return;
    }

    for entity in scoped.iter().chain(chunks.iter()) {
        commands.entity(entity).try_despawn();
    }

    // Новый рельеф: старое хранилище с кратерами выбрасывается целиком.
    let seed = RunSeed::from_clock();
    let terrain = Terrain::new(seed.0);
    *visible = VisibleChunks::default();

    for (mut transform, mut motion, mut health, mut visibility) in &mut rover {
        let up = terrain.normal_at(0.0, 0.0);
        transform.translation = Vec3::new(0.0, terrain.height_at(0.0, 0.0) + ROVER_RIDE_HEIGHT, 0.0);
        transform.rotation = Quat::from_rotation_arc(Vec3::Y, up);
        *motion = RoverMotion { up, ..default() };
        *health = Health::new(ROVER_MAX_HEALTH);
        *visibility = Visibility::Inherited;
    }

    commands.insert_resource(seed);
    commands.insert_resource(terrain);
    *death = Death::default();
    survival.0 = 0.0;
    started.write(RunStarted { seed: seed.0 });
    next.set(GameState::Playing);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_shows_time_and_rank() {
        assert_eq!(
            result_lines(67.3, Some(0), Some(67.3)),
            ("TIME  1:07.3".to_string(), "NEW RECORD!".to_string())
        );
        assert_eq!(
            result_lines(30.0, Some(2), Some(187.25)),
            ("TIME  0:30.0".to_string(), "PLACE 3   BEST 3:07.2".to_string())
        );
        assert_eq!(result_lines(5.0, None, Some(187.25)).1, "BEST 3:07.2");
        assert_eq!(result_lines(5.0, None, None).1, "");
    }
}
