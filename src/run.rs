//! Забег и экраны вокруг него: меню, гибель лунохода, экран конца забега и рестарт.
//!
//! Игра открывается в меню: планета летит под камерой, забега нет. Пауза останавливает
//! виртуальное время: всё, что считает по `Time`, замирает само. Новый забег объявляется
//! сообщением [`RunStarted`]: каждый модуль, который что-то хранит на весь забег, сбрасывает это
//! по сообщению. Конец забега — сообщение [`RunEnded`]: его отправляют рестарт и выход в меню,
//! вместе с ним удаляются сущности с [`RunScoped`] и заменяется рельеф.

use bevy::app::AppExit;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::WindowFocused;

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
            .init_resource::<PauseChoice>()
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
                    pause_on_escape_or_blur.run_if(in_state(GameState::Playing)),
                    (pause_input, highlight_pause_choice)
                        .chain()
                        .run_if(in_state(GameState::Paused)),
                    (game_over_input, draw_game_over)
                        .chain()
                        .run_if(in_state(GameState::GameOver)),
                ),
            )
            .add_systems(OnEnter(GameState::Menu), show_menu)
            .add_systems(OnExit(GameState::Menu), hide_screen::<MenuScreen>)
            .add_systems(OnEnter(GameState::Paused), (stop_time, show_pause))
            .add_systems(OnExit(GameState::Paused), (resume_time, hide_screen::<PauseScreen>))
            .add_systems(OnEnter(GameState::GameOver), prepare_result)
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

/// Итог последнего забега для экрана конца забега.
#[derive(Resource, Default)]
struct RunResult {
    seconds: f32,
    /// Место в таблице рекордов, считая с нуля. `None` — не попал.
    place: Option<usize>,
    /// Имя, которое игрок набирает. `None` — имя уже сохранено или результат не попал в таблицу.
    entry: Option<NameEntry>,
}

/// Ввод имени из трёх символов: буква или цифра встаёт в ячейку под курсором и двигает его.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NameEntry {
    cells: [Option<char>; records::NAME_LENGTH],
    cursor: usize,
}

impl NameEntry {
    /// Ввод с подставленным именем и курсором на первой ячейке.
    fn new(default_name: &str) -> Self {
        let mut cells = [None; records::NAME_LENGTH];
        for (cell, letter) in cells.iter_mut().zip(default_name.chars()) {
            *cell = Some(letter.to_ascii_uppercase());
        }
        Self { cells, cursor: 0 }
    }

    fn type_char(&mut self, letter: char) {
        if self.cursor < self.cells.len() && letter.is_ascii_alphanumeric() {
            self.cells[self.cursor] = Some(letter.to_ascii_uppercase());
            self.cursor += 1;
        }
    }

    /// Стирает ячейку перед курсором и возвращает курсор на неё.
    fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.cells[self.cursor] = None;
        }
    }

    /// Имя, если заполнены все ячейки.
    fn name(&self) -> Option<String> {
        self.cells.iter().copied().collect()
    }
}

/// Буква или цифра клавиши. Берётся из кода клавиши, поэтому не зависит от раскладки.
fn key_char(key: KeyCode) -> Option<char> {
    const LETTERS: [KeyCode; 26] = [
        KeyCode::KeyA, KeyCode::KeyB, KeyCode::KeyC, KeyCode::KeyD, KeyCode::KeyE, KeyCode::KeyF,
        KeyCode::KeyG, KeyCode::KeyH, KeyCode::KeyI, KeyCode::KeyJ, KeyCode::KeyK, KeyCode::KeyL,
        KeyCode::KeyM, KeyCode::KeyN, KeyCode::KeyO, KeyCode::KeyP, KeyCode::KeyQ, KeyCode::KeyR,
        KeyCode::KeyS, KeyCode::KeyT, KeyCode::KeyU, KeyCode::KeyV, KeyCode::KeyW, KeyCode::KeyX,
        KeyCode::KeyY, KeyCode::KeyZ,
    ];
    const DIGITS: [KeyCode; 10] = [
        KeyCode::Digit0, KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4,
        KeyCode::Digit5, KeyCode::Digit6, KeyCode::Digit7, KeyCode::Digit8, KeyCode::Digit9,
    ];
    let at = |keys: &[KeyCode], first: u8| {
        keys.iter()
            .position(|candidate| *candidate == key)
            .map(|index| (first + index as u8) as char)
    };
    at(&LETTERS, b'A').or_else(|| at(&DIGITS, b'0'))
}

/// Таблица рекордов в памяти и путь к её файлу.
#[derive(Resource)]
struct RecordsTable {
    table: Records,
    path: Option<std::path::PathBuf>,
}

impl RecordsTable {
    /// Вносит результат в таблицу и записывает файл. Возвращает занятое место.
    fn add(&mut self, name: &str, seconds: f32) -> Option<usize> {
        let place = self.table.insert(name, f64::from(seconds));
        if let Some(path) = self.path.as_deref()
            && let Err(error) = self.table.save(path)
        {
            warn!("Рекорды: не записан {}: {error}", path.display());
        }
        place
    }

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

    /// Продолжение после паузы. Кнопка мыши, зажатая на паузе, не стреляет.
    fn resume(&mut self) {
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

/// Итог забега: если время достойно таблицы, игрок вводит имя.
fn prepare_result(survival: Res<SurvivalTime>, records: Res<RecordsTable>, mut result: ResMut<RunResult>) {
    let place = records.table.place_for(f64::from(survival.0));
    *result = RunResult {
        seconds: survival.0,
        place,
        entry: place.map(|_| NameEntry::new(&records.table.last_name)),
    };
}

/// Строка о месте в таблице.
fn rank_line(place: Option<usize>) -> String {
    match place {
        Some(0) => "NEW RECORD!".to_string(),
        Some(place) => format!("PLACE {}", place + 1),
        None => format!("NOT IN TOP {}", records::MAX_RECORDS),
    }
}

/// Строка таблицы рекордов.
fn table_row(place: usize, record: &Record) -> String {
    format!("{:>2}. {}  {:>7}", place + 1, record.name, format_time(record.seconds as f32))
}

/// Перерисовывает экран конца забега, когда итог изменился: ввод имени или таблица.
fn draw_game_over(
    mut commands: Commands,
    result: Res<RunResult>,
    records: Res<RecordsTable>,
    old: Query<Entity, With<GameOverScreen>>,
) {
    if !result.is_changed() {
        return;
    }
    for screen in &old {
        commands.entity(screen).despawn();
    }
    commands
        .spawn((GameOverScreen, screen_root(0.62)))
        .with_children(|screen| {
            screen.spawn(screen_line("ROVER LOST", 56.0, Color::srgb(1.0, 0.35, 0.25)));
            screen.spawn(screen_line(format!("TIME  {}", format_time(result.seconds)), 34.0, Color::WHITE));
            screen.spawn(screen_line(rank_line(result.place), 26.0, TITLE_COLOR));

            if let Some(entry) = &result.entry {
                screen.spawn(screen_line("ENTER YOUR NAME", 22.0, Color::WHITE));
                screen
                    .spawn(Node {
                        column_gap: Val::Px(14.0),
                        ..default()
                    })
                    .with_children(|cells| {
                        for (index, cell) in entry.cells.iter().enumerate() {
                            let color = if index == entry.cursor { TITLE_COLOR } else { Color::WHITE };
                            cells.spawn(screen_line(cell.unwrap_or('_').to_string(), 64.0, color));
                        }
                    });
                screen.spawn(screen_line("LETTERS, DIGITS, BACKSPACE     ENTER - SAVE", 20.0, HINT_COLOR));
                return;
            }

            screen
                .spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(4.0),
                    ..default()
                })
                .with_children(|table| {
                    for (place, record) in records.table.records.iter().enumerate() {
                        let color = if result.place == Some(place) { TITLE_COLOR } else { Color::WHITE };
                        table.spawn(screen_line(table_row(place, record), 22.0, color));
                    }
                });
            screen.spawn(screen_line("ENTER - RESTART     ESC - MENU", 22.0, HINT_COLOR));
        });
}

/// Конец забега. Пока вводится имя: буквы, цифры, Backspace, Enter сохраняет результат.
/// После этого Enter начинает новый забег, Esc возвращает в меню.
fn game_over_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut result: ResMut<RunResult>,
    mut records: ResMut<RecordsTable>,
    mut world: RunWorld,
) {
    let enter = keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]);
    let escape = keys.just_pressed(KeyCode::Escape);

    if let Some(mut entry) = result.entry.clone() {
        for letter in keys.get_just_pressed().copied().filter_map(key_char) {
            entry.type_char(letter);
        }
        if keys.just_pressed(KeyCode::Backspace) {
            entry.backspace();
        }
        // Esc с недописанным именем сохраняет результат под прежним именем: рекорд не теряется.
        let name = entry.name().or_else(|| escape.then(|| records.table.last_name.clone()));
        if let (true, Some(name)) = (enter || escape, name) {
            result.place = records.add(&name, result.seconds);
            result.entry = None;
        } else if result.entry.as_ref() != Some(&entry) {
            result.entry = Some(entry);
        }
        if escape {
            world.open_menu();
        }
        return;
    }

    if enter {
        world.start_run();
    } else if escape {
        world.open_menu();
    }
}

/// Пункты экрана паузы по порядку и их клавиши-подсказки.
const PAUSE_ITEMS: [(&str, KeyCode); 3] = [
    ("CONTINUE - ESC", KeyCode::Escape),
    ("RESTART - R", KeyCode::KeyR),
    ("MENU - M", KeyCode::KeyM),
];

/// Корень экрана паузы.
#[derive(Component)]
struct PauseScreen;

/// Пункт экрана паузы с его номером.
#[derive(Component)]
struct PauseItem(usize);

/// Выбранный пункт экрана паузы.
#[derive(Resource, Default)]
struct PauseChoice(usize);

/// Номер пункта после шага вверх или вниз: список замкнут в кольцо.
fn moved_choice(current: usize, step: i32) -> usize {
    (current as i32 + step).rem_euclid(PAUSE_ITEMS.len() as i32) as usize
}

/// Забег встаёт на паузу по Esc и когда окно теряет фокус.
fn pause_on_escape_or_blur(
    keys: Res<ButtonInput<KeyCode>>,
    mut focus: MessageReader<WindowFocused>,
    mut next: ResMut<NextState<GameState>>,
) {
    let blurred = focus.read().any(|change| !change.focused);
    if blurred || keys.just_pressed(KeyCode::Escape) {
        next.set(GameState::Paused);
    }
}

fn stop_time(mut time: ResMut<Time<Virtual>>, mut choice: ResMut<PauseChoice>) {
    time.pause();
    choice.0 = 0;
}

fn resume_time(mut time: ResMut<Time<Virtual>>) {
    time.unpause();
}

fn show_pause(mut commands: Commands) {
    commands
        .spawn((PauseScreen, screen_root(0.62)))
        .with_children(|screen| {
            screen.spawn(screen_line("PAUSED", 56.0, TITLE_COLOR));
            screen.spawn(screen_line("", 10.0, Color::NONE));
            for (index, (label, _)) in PAUSE_ITEMS.iter().enumerate() {
                screen.spawn((PauseItem(index), screen_line(*label, 30.0, HINT_COLOR)));
            }
            screen.spawn(screen_line("", 10.0, Color::NONE));
            screen.spawn(screen_line("UP, DOWN AND ENTER TO CHOOSE", 18.0, HINT_COLOR));
        });
}

/// Пауза: стрелки и Enter выбирают пункт, клавиша-подсказка срабатывает сразу.
fn pause_input(keys: Res<ButtonInput<KeyCode>>, mut choice: ResMut<PauseChoice>, mut world: RunWorld) {
    if keys.any_just_pressed([KeyCode::ArrowUp, KeyCode::KeyW]) {
        choice.0 = moved_choice(choice.0, -1);
    }
    if keys.any_just_pressed([KeyCode::ArrowDown, KeyCode::KeyS]) {
        choice.0 = moved_choice(choice.0, 1);
    }
    let by_hint = PAUSE_ITEMS.iter().position(|(_, key)| keys.just_pressed(*key));
    let by_enter = keys
        .any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter])
        .then_some(choice.0);
    match by_hint.or(by_enter) {
        Some(0) => world.resume(),
        Some(1) => world.start_run(),
        Some(2) => world.open_menu(),
        _ => {}
    }
}

/// Выбранный пункт паузы белый, остальные серые.
fn highlight_pause_choice(choice: Res<PauseChoice>, mut items: Query<(&PauseItem, &mut TextColor)>) {
    for (item, mut color) in &mut items {
        let wanted = if item.0 == choice.0 { Color::WHITE } else { HINT_COLOR };
        if color.0 != wanted {
            color.0 = wanted;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_line_names_the_place() {
        assert_eq!(rank_line(Some(0)), "NEW RECORD!");
        assert_eq!(rank_line(Some(2)), "PLACE 3");
        assert_eq!(rank_line(None), "NOT IN TOP 10");
    }

    #[test]
    fn table_row_is_aligned() {
        let record = Record {
            name: "IVA".to_string(),
            seconds: 187.25,
        };
        assert_eq!(table_row(0, &record), " 1. IVA   3:07.2");
        assert_eq!(table_row(9, &record), "10. IVA   3:07.2");
    }

    #[test]
    fn name_entry_starts_with_default_and_types_over_it() {
        let mut entry = NameEntry::new("lun");
        assert_eq!(entry.name().as_deref(), Some("LUN"));
        entry.type_char('i');
        entry.type_char('?');
        entry.type_char('v');
        assert_eq!(entry.name().as_deref(), Some("IVN"));
        entry.type_char('a');
        entry.type_char('x');
        assert_eq!(entry.name().as_deref(), Some("IVA"));
    }

    #[test]
    fn backspace_clears_the_cell_and_incomplete_name_is_not_a_name() {
        let mut entry = NameEntry::new("LUN");
        entry.backspace();
        assert_eq!(entry.name().as_deref(), Some("LUN"));
        entry.type_char('A');
        entry.type_char('B');
        entry.backspace();
        assert_eq!(entry.name(), None);
        assert_eq!(entry.cursor, 1);
        entry.type_char('7');
        assert_eq!(entry.name().as_deref(), Some("A7N"));
    }

    #[test]
    fn keys_give_letters_and_digits() {
        assert_eq!(key_char(KeyCode::KeyA), Some('A'));
        assert_eq!(key_char(KeyCode::KeyZ), Some('Z'));
        assert_eq!(key_char(KeyCode::Digit0), Some('0'));
        assert_eq!(key_char(KeyCode::Digit9), Some('9'));
        assert_eq!(key_char(KeyCode::Space), None);
    }

    #[test]
    fn pause_choice_wraps_around() {
        assert_eq!(moved_choice(0, 1), 1);
        assert_eq!(moved_choice(2, 1), 0);
        assert_eq!(moved_choice(0, -1), 2);
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
