//! Забег и экраны вокруг него: меню, гибель лунохода, экран конца забега и рестарт.
//!
//! Игра открывается в меню: планета летит под камерой, забега нет. Новый забег объявляется
//! сообщением [`RunStarted`]: каждый модуль, который что-то хранит на весь забег, сбрасывает это
//! по сообщению. Конец забега — сообщение [`RunEnded`]: его отправляют рестарт и выход в меню,
//! вместе с ним удаляются сущности с [`RunScoped`] и заменяется рельеф.

use bevy::app::AppExit;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::combat::{Died, Health, SurvivalTime, Team, ROVER_MAX_HEALTH};
use crate::explosion::Explosion;
use crate::hud::format_time;
use crate::records::{self, Record, Records};
use crate::rover::{self, Rover, RoverMotion};
use crate::terrain::{ChunkMesh, RunSeed, Terrain, VisibleChunks};

/// Сколько секунд после гибели до экрана конца забега.
const GAME_OVER_DELAY: f32 = 1.0;

/// Взрыв при гибели лунохода: радиус, м, и мощность.
const DEATH_BLAST: (f32, f32) = (9.0, 2.5);

/// Высота центра корпуса лунохода над поверхностью при возвращении на старт, м.
const ROVER_RIDE_HEIGHT: f32 = 0.5;

/// Скорость, с которой планета летит под камерой в меню, м/с.
const MENU_FLIGHT: Vec3 = Vec3::new(5.0, 0.0, -2.5);

/// Подключает состояния игры, меню, гибель и рестарт.
pub struct RunPlugin;

impl Plugin for RunPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<GameState>()
            .init_resource::<Death>()
            .init_resource::<RunResult>()
            .insert_resource(RecordsTable::load())
            .add_message::<RunStarted>()
            .add_message::<RunEnded>()
            .add_systems(
                Update,
                (
                    // Сначала полёт, потом ввод: иначе полёт спрятал бы луноход, только что поставленный на старт.
                    (fly_over_planet, menu_input).chain().run_if(in_state(GameState::Menu)),
                    (watch_rover_death, hold_dead_rover)
                        .chain()
                        .after(rover::drive_rover)
                        .run_if(in_state(GameState::Playing)),
                    game_over_input.run_if(in_state(GameState::GameOver)),
                ),
            )
            .add_systems(OnEnter(GameState::Menu), show_menu)
            .add_systems(OnExit(GameState::Menu), hide_screen::<MenuScreen>)
            .add_systems(OnEnter(GameState::GameOver), (record_result, show_game_over).chain())
            .add_systems(OnExit(GameState::GameOver), hide_screen::<GameOverScreen>);
    }
}

/// Состояние игры.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GameState {
    /// Стартовый экран: планета летит под камерой, забега нет.
    #[default]
    Menu,
    /// Идёт забег.
    Playing,
    // Пауза появится в задаче 2.1.
    #[allow(dead_code)]
    /// Забег остановлен, показан экран паузы.
    Paused,
    /// Луноход погиб, показан экран конца забега.
    GameOver,
}

/// Забег закончился и его мир убран: рестарт или выход в меню.
#[derive(Message, Debug, Clone, Copy)]
pub struct RunEnded;

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

/// Корень стартового экрана.
#[derive(Component)]
struct MenuScreen;

/// Всё, что меняется, когда забег заканчивается и мир строится заново.
#[derive(SystemParam)]
struct RunWorld<'w, 's> {
    commands: Commands<'w, 's>,
    death: ResMut<'w, Death>,
    survival: ResMut<'w, SurvivalTime>,
    visible: ResMut<'w, VisibleChunks>,
    ended: MessageWriter<'w, RunEnded>,
    started: MessageWriter<'w, RunStarted>,
    next: ResMut<'w, NextState<GameState>>,
    mouse: ResMut<'w, ButtonInput<MouseButton>>,
    scoped: Query<'w, 's, Entity, With<RunScoped>>,
    chunks: Query<'w, 's, Entity, With<ChunkMesh>>,
    rover: Query<
        'w,
        's,
        (
            &'static mut Transform,
            &'static mut RoverMotion,
            &'static mut Health,
            &'static mut Visibility,
        ),
        With<Rover>,
    >,
}

impl RunWorld<'_, '_> {
    /// Убирает мир прошлого забега: сущности забега, рельеф с кратерами; ставит целый луноход
    /// на старт нового рельефа. Возвращает новый сид.
    fn reset(&mut self) -> u32 {
        for entity in self.scoped.iter().chain(self.chunks.iter()) {
            self.commands.entity(entity).try_despawn();
        }

        // Новый рельеф: старое хранилище с кратерами выбрасывается целиком.
        let seed = RunSeed::from_clock();
        let terrain = Terrain::new(seed.0);
        *self.visible = VisibleChunks::default();

        for (mut transform, mut motion, mut health, mut visibility) in &mut self.rover {
            let up = terrain.normal_at(0.0, 0.0);
            transform.translation = Vec3::new(0.0, terrain.height_at(0.0, 0.0) + ROVER_RIDE_HEIGHT, 0.0);
            transform.rotation = Quat::from_rotation_arc(Vec3::Y, up);
            *motion = RoverMotion { up, ..default() };
            *health = Health::new(ROVER_MAX_HEALTH);
            *visibility = Visibility::Inherited;
        }

        self.commands.insert_resource(seed);
        self.commands.insert_resource(terrain);
        *self.death = Death::default();
        self.survival.0 = 0.0;
        self.ended.write(RunEnded);
        seed.0
    }

    /// Новый забег на новом рельефе.
    fn start_run(&mut self) {
        let seed = self.reset();
        self.started.write(RunStarted { seed });
        // Нажатие, которое начало забег, не стреляет: кнопка считается отпущенной до нового нажатия.
        self.mouse.reset_all();
        self.next.set(GameState::Playing);
    }

    /// Выход в меню: мир забега убран, планета снова летит под камерой.
    fn open_menu(&mut self) {
        self.reset();
        self.next.set(GameState::Menu);
    }
}

/// Фон меню: точка, за которой следят камера и подгрузка рельефа, медленно летит по прямой.
/// Эта точка — спрятанный луноход, поэтому камере и рельефу меню ничем не отличается от забега.
fn fly_over_planet(
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut rover: Query<(&mut Transform, &mut Visibility), With<Rover>>,
) {
    for (mut transform, mut visibility) in &mut rover {
        if *visibility != Visibility::Hidden {
            *visibility = Visibility::Hidden;
        }
        let mut position = transform.translation + MENU_FLIGHT * time.delta_secs();
        position.y = terrain.height_at(position.x, position.z) + ROVER_RIDE_HEIGHT;
        transform.translation = position;
    }
}

/// Меню: Enter, пробел или клик начинают забег, Esc закрывает игру.
fn menu_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut world: RunWorld,
    mut exit: MessageWriter<AppExit>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
        return;
    }
    let start = keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter, KeyCode::Space])
        || world.mouse.just_pressed(MouseButton::Left);
    if start {
        world.start_run();
    }
}

/// Строка лучшего результата для меню.
fn best_line(best: Option<&Record>) -> Option<String> {
    best.map(|record| format!("BEST {}  {}", format_time(record.seconds as f32), record.name))
}

/// Строка текста на экране.
fn screen_line(text: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
    )
}

/// Корневой узел экрана на весь кадр с затемнением `shade`.
fn screen_root(shade: f32) -> impl Bundle {
    (
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
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, shade)),
    )
}

const TITLE_COLOR: Color = Color::srgb(1.0, 0.85, 0.3);
const HINT_COLOR: Color = Color::srgb(0.8, 0.8, 0.8);

fn show_menu(mut commands: Commands, records: Res<RecordsTable>) {
    let best = best_line(records.table.best());
    commands
        .spawn((MenuScreen, screen_root(0.35)))
        .with_children(|screen| {
            screen.spawn(screen_line("LUNOKHOD", 96.0, TITLE_COLOR));
            screen.spawn(screen_line("SURVIVE AS LONG AS YOU CAN", 26.0, Color::WHITE));
            if let Some(best) = best {
                screen.spawn(screen_line(best, 26.0, TITLE_COLOR));
            }
            screen.spawn(screen_line("", 10.0, Color::NONE));
            screen.spawn(screen_line("WASD - DRIVE     MOUSE - AIM     LEFT BUTTON - FIRE", 20.0, HINT_COLOR));
            screen.spawn(screen_line("1, 2, WHEEL - WEAPON     SPACE - REPAIR", 20.0, HINT_COLOR));
            screen.spawn(screen_line("", 10.0, Color::NONE));
            screen.spawn(screen_line("ENTER, SPACE OR CLICK - START", 32.0, Color::WHITE));
            screen.spawn(screen_line("ESC - QUIT", 20.0, HINT_COLOR));
        });
}

/// Убирает экран с маркером `T`.
fn hide_screen<T: Component>(mut commands: Commands, screens: Query<Entity, With<T>>) {
    for screen in &screens {
        commands.entity(screen).despawn();
    }
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
    commands
        .spawn((GameOverScreen, screen_root(0.62)))
        .with_children(|screen| {
            screen.spawn(screen_line("ROVER LOST", 56.0, Color::srgb(1.0, 0.35, 0.25)));
            screen.spawn(screen_line(time, 34.0, Color::WHITE));
            if !rank.is_empty() {
                screen.spawn(screen_line(rank, 26.0, TITLE_COLOR));
            }
            screen.spawn(screen_line("ENTER - RESTART     ESC - MENU", 22.0, HINT_COLOR));
        });
}

/// Конец забега: Enter начинает новый забег, Esc возвращает в меню.
fn game_over_input(keys: Res<ButtonInput<KeyCode>>, mut world: RunWorld) {
    if keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]) {
        world.start_run();
    } else if keys.just_pressed(KeyCode::Escape) {
        world.open_menu();
    }
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

    #[test]
    fn menu_shows_best_record_when_there_is_one() {
        assert_eq!(best_line(None), None);
        let record = Record {
            name: "IVA".to_string(),
            seconds: 187.25,
        };
        assert_eq!(best_line(Some(&record)).as_deref(), Some("BEST 3:07.2  IVA"));
    }
}
