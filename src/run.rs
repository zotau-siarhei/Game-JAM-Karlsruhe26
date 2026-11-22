//! Забег и экраны вокруг него: пролёт над планетой в меню, гибель лунохода, экран конца забега
//! и рестарт. Экраны главного меню — в `src/menu.rs`.
//!
//! Игра открывается загрузкой (`src/loading.rs`), после неё — меню: планета летит под камерой,
//! забега нет. Начало забега, рестарт и выход в меню идут через затемнение (`src/fade.rs`):
//! экран гаснет, мир перестраивается под чёрным занавесом, кадры выравниваются, экран проявляется. Пауза останавливает
//! виртуальное время: всё, что считает по `Time`, замирает само, а игровая симуляция стоит во
//! всех состояниях, кроме `Playing` (см. `frame::GameSet`).
//!
//! Мир строится заново в одном месте — [`RunWorld::reset`]: при рестарте и при выходе в меню.
//! Он заменяет рельеф, удаляет сущности с [`RunScoped`] и шлёт [`WorldRebuilt`] с новым сидом.
//! Каждый модуль, который хранит что-то о мире или о забеге, сбрасывает это по [`WorldRebuilt`]
//! в наборе `GameSet::Reset`. [`RunStarted`] приходит только в начале забега, сразу за
//! [`WorldRebuilt`]: на него подписано то, чего не должно быть при выходе в меню.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::ui::UiGlobalTransform;
use bevy::window::WindowFocused;

use crate::camera::Focus;
use crate::combat::{Died, Health, SurvivalTime, Team, ROVER_MAX_HEALTH};
use crate::difficulty::{Difficulty, RunDifficulty};
use crate::explosion::Explosion;
use crate::fade::{Appear, Curtain};
use crate::gamepad::{DeviceHint, InputDevice};
use crate::frame::GameSet;
use crate::hud::format_time;
use crate::records::{self, Record, RecordPlaced, Records};
use crate::rover::{Rover, RoverMotion};
use crate::terrain::{ChunkMesh, RunSeed, Terrain, VisibleChunks};

/// Сколько секунд после гибели до экрана конца забега.
const GAME_OVER_DELAY: f32 = 1.0;

/// Взрыв при гибели лунохода: радиус, м, и мощность.
const DEATH_BLAST: (f32, f32) = (9.0, 2.5);

/// Высота центра корпуса лунохода над поверхностью при возвращении на старт, м.
const ROVER_RIDE_HEIGHT: f32 = crate::rover::RIDE_HEIGHT * crate::rover::ROVER_SCALE;

/// Сколько секунд экран конца забега не принимает клавиши, с: нажатия, начатые ещё в бою
/// (WASD, пробел, цифры оружия), не затирают имя и не начинают новый забег.
const GAME_OVER_INPUT_DELAY: f32 = 0.8;

/// Затемнение перед перестройкой мира и проявление после неё, с.
const CLOSE_SECONDS: f32 = 0.35;
const OPEN_SECONDS: f32 = 0.6;
/// Мир под занавесом «осел»: не меньше стольких кадров после перестройки и столько кадров подряд
/// короче [`STEADY_FRAME`], но не дольше [`SETTLE_LIMIT`] секунд.
const SETTLE_FRAMES: u32 = 8;
const STEADY_FRAMES: u32 = 5;
const STEADY_FRAME: f32 = 0.05;
const SETTLE_LIMIT: f32 = 1.5;
/// Время проявления экранов, с.
pub(crate) const MENU_APPEAR: f32 = 0.5;
const PAUSE_APPEAR: f32 = 0.15;
const GAME_OVER_APPEAR: f32 = 0.6;

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
            .init_resource::<Transition>()
            .insert_resource(RecordsTable::load())
            .add_message::<WorldRebuilt>()
            .add_message::<RunStarted>()
            .add_message::<RecordPlaced>()
            .add_systems(Startup, announce_first_world)
            .add_systems(
                Update,
                (
                    // Ввод меню (`menu.rs`) идёт после полёта: иначе полёт спрятал бы луноход, только что
                    // поставленный на старт.
                    fly_over_planet.run_if(in_state(GameState::Menu)),
                    pause_on_escape_or_blur.run_if(in_state(GameState::Playing)),
                    (pause_input, highlight_pause_choice)
                        .chain()
                        .run_if(in_state(GameState::Paused)),
                    (game_over_input, draw_game_over)
                        .chain()
                        .run_if(in_state(GameState::GameOver)),
                    // После полёта: забег начинается за занавесом ещё в состоянии меню, и полёт,
                    // пройдя позже в том же кадре, снова спрятал бы луноход на весь забег.
                    run_transition.after(fly_over_planet),
                )
                    .in_set(GameSet::Screens),
            )
            // После всех, кто двигает луноход в этом кадре: погибший стоит на месте.
            .add_systems(Update, (watch_rover_death, hold_dead_rover).chain().in_set(GameSet::Death))
            .add_systems(OnEnter(GameState::Paused), (stop_time, show_pause))
            .add_systems(OnExit(GameState::Paused), (resume_time, hide_screen::<PauseScreen>))
            .add_systems(OnEnter(GameState::GameOver), prepare_result)
            .add_systems(OnExit(GameState::GameOver), hide_screen::<GameOverScreen>);
    }
}

/// Состояние игры.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GameState {
    /// Загрузка при запуске: под чёрным занавесом грузятся звуки и модели, прогреваются шейдеры.
    /// Ничего не звучит, меню не показано (`src/loading.rs`).
    #[default]
    Loading,
    /// Стартовый экран: планета летит под камерой, забега нет.
    Menu,
    /// Идёт забег.
    Playing,
    /// Забег остановлен, показан экран паузы.
    Paused,
    /// Луноход погиб, показан экран конца забега.
    GameOver,
}

/// Мир перестроен: рельеф заменён новым с этим сидом, сущности с [`RunScoped`] удалены, луноход
/// стоит на старте. Приходит при запуске игры, при каждом начале забега и при выходе в меню.
///
/// По нему сбрасывается всё, что хранит состояние мира или забега: посёлки, объекты, пути,
/// обломки, лава, волны, тревога, инвентарь, таймеры. Читать в наборе `GameSet::Reset`.
#[derive(Message, Debug, Clone, Copy)]
pub struct WorldRebuilt {
    /// Сид нового мира: от него строятся посёлки, объекты, пути и игровой генератор.
    pub seed: u32,
}

/// Начался новый забег: приходит сразу за [`WorldRebuilt`], в том же кадре. Только для того,
/// что случается именно в начале забега и не нужно в меню: приветствие штаба.
#[derive(Message, Debug, Clone, Copy)]
pub struct RunStarted;

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
    /// Сколько ещё секунд реального времени экран не принимает клавиши.
    input_delay: f32,
}

/// Ввод имени до пяти символов: буква или цифра встаёт в ячейку под курсором и двигает его.
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

    /// Имя — заполненные ячейки от первой до первой пустой; `None`, если первая пуста.
    fn name(&self) -> Option<String> {
        let name: String = self.cells.iter().map_while(|cell| *cell).collect();
        (!name.is_empty()).then_some(name)
    }
}

/// Буква или цифра клавиши. Берётся из кода клавиши, поэтому не зависит от раскладки.
fn key_char(key: KeyCode) -> Option<char> {
    let at = |keys: &[KeyCode], first: u8| {
        keys.iter()
            .position(|candidate| *candidate == key)
            .map(|index| (first + index as u8) as char)
    };
    at(&LETTERS, b'A').or_else(|| at(&DIGITS, b'0'))
}

/// Клавиша буквы или цифры: обратное к [`key_char`], для экранной клавиатуры.
fn char_key(letter: char) -> Option<KeyCode> {
    match letter {
        'A'..='Z' => LETTERS.get((letter as u8 - b'A') as usize).copied(),
        '0'..='9' => DIGITS.get((letter as u8 - b'0') as usize).copied(),
        _ => None,
    }
}

/// Клавиши букв и цифр по порядку.
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

/// Ряды экранной клавиатуры для имени рекорда.
const KEYBOARD_ROWS: [&str; 4] = ["1234567890", "QWERTYUIOP", "ASDFGHJKL", "ZXCVBNM"];
/// Клавиша экранной клавиатуры: ширина и высота, dp.
const KEY_SIZE_DP: (f32, f32) = (52.0, 40.0);
/// Промежуток между строками экрана конца забега, пока имя вводится касаниями.
const COMPACT_GAP: f32 = 8.0;

/// Кнопка экрана, которую нажимают пальцем: клавиша или пункт.
fn touch_key(
    action: crate::touch::TapAction,
    text: &str,
    width_dp: f32,
    ui_scale: &UiScale,
    color: Color,
) -> impl Bundle {
    let px = |dp: f32| Val::Px(crate::touch::dp_to_ui(dp, ui_scale));
    (
        action,
        Node {
            width: px(width_dp),
            height: px(KEY_SIZE_DP.1),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(px(8.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.1, 0.14, 0.19, 0.9)),
        BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.3)),
        children![(
            Text::new(text),
            TextFont {
                font_size: FontSize::Px(crate::touch::dp_to_ui(17.0, ui_scale)),
                ..default()
            },
            TextColor(color),
        )],
    )
}

/// Экранная клавиатура имени рекорда: ряды букв и цифр, стереть, сохранить.
fn spawn_keyboard(screen: &mut ChildSpawnerCommands, ui_scale: &UiScale) {
    use crate::touch::TapAction;
    let gap = Val::Px(crate::touch::dp_to_ui(6.0, ui_scale));
    screen
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: gap,
            margin: UiRect::top(gap),
            ..default()
        })
        .with_children(|keyboard| {
            for (index, row) in KEYBOARD_ROWS.iter().enumerate() {
                keyboard
                    .spawn(Node {
                        column_gap: gap,
                        ..default()
                    })
                    .with_children(|line| {
                        for letter in row.chars() {
                            if let Some(key) = char_key(letter) {
                                let text = letter.to_string();
                                line.spawn(touch_key(TapAction::Key(key), &text, KEY_SIZE_DP.0, ui_scale, Color::WHITE));
                            }
                        }
                        // Последний ряд: стереть и сохранить.
                        if index == KEYBOARD_ROWS.len() - 1 {
                            line.spawn(touch_key(TapAction::Key(KeyCode::Backspace), "DEL", 70.0, ui_scale, Color::WHITE));
                            line.spawn(touch_key(TapAction::Key(KeyCode::Enter), "SAVE", 90.0, ui_scale, TITLE_COLOR));
                        }
                    });
            }
        });
}

/// Таблицы рекордов всех уровней сложности в памяти и пути к их файлам.
#[derive(Resource, Default)]
pub(crate) struct RecordsTable {
    tables: [Records; 3],
    paths: [Option<std::path::PathBuf>; 3],
}

impl RecordsTable {
    /// Таблица уровня сложности.
    pub(crate) fn table(&self, difficulty: Difficulty) -> &Records {
        &self.tables[difficulty.index()]
    }

    /// Вносит результат в таблицу уровня и записывает её файл. Возвращает занятое место.
    fn add(&mut self, difficulty: Difficulty, name: &str, seconds: f32) -> Option<usize> {
        let index = difficulty.index();
        let table = &mut self.tables[index];
        let place = table.insert(name, f64::from(seconds));
        if let Some(path) = self.paths[index].as_deref()
            && let Err(error) = table.save(path)
        {
            warn!("Рекорды: не записан {}: {error}", path.display());
        }
        place
    }

    fn load() -> Self {
        let paths = Difficulty::ALL.map(records::records_path_for);
        let tables = paths.clone().map(|path| path.as_deref().map(Records::load).unwrap_or_default());
        Self { tables, paths }
    }
}

/// Корень экрана конца забега.
#[derive(Component)]
struct GameOverScreen;

/// Всё, что меняется, когда забег заканчивается и мир строится заново.
#[derive(SystemParam)]
pub(crate) struct RunWorld<'w, 's> {
    commands: Commands<'w, 's>,
    death: ResMut<'w, Death>,
    survival: ResMut<'w, SurvivalTime>,
    seed: ResMut<'w, RunSeed>,
    terrain: ResMut<'w, Terrain>,
    visible: ResMut<'w, VisibleChunks>,
    focus: ResMut<'w, Focus>,
    rebuilt: MessageWriter<'w, WorldRebuilt>,
    started: MessageWriter<'w, RunStarted>,
    next: ResMut<'w, NextState<GameState>>,
    mouse: ResMut<'w, ButtonInput<MouseButton>>,
    transition: ResMut<'w, Transition>,
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
    /// Строит мир заново: убирает сущности забега и рельеф с кратерами, ставит целый луноход
    /// на старт нового рельефа и объявляет [`WorldRebuilt`]. `rover` — виден ли луноход.
    ///
    /// Рельеф и сид заменяются сразу, а не отложенной командой: подгрузка участков в этом же
    /// кадре собирает меши уже из нового рельефа.
    fn reset(&mut self, rover: Visibility) {
        for entity in self.scoped.iter().chain(self.chunks.iter()) {
            self.commands.entity(entity).try_despawn();
        }

        // Новый рельеф: старое хранилище с кратерами выбрасывается целиком.
        let seed = RunSeed::from_clock();
        *self.seed = seed;
        *self.terrain = Terrain::new(seed.0);
        *self.visible = VisibleChunks::default();

        let start = Vec3::new(0.0, self.terrain.height_at(0.0, 0.0) + ROVER_RIDE_HEIGHT, 0.0);
        let up = self.terrain.normal_at(0.0, 0.0);
        for (mut transform, mut motion, mut health, mut visibility) in &mut self.rover {
            transform.translation = start;
            transform.rotation = Quat::from_rotation_arc(Vec3::Y, up);
            *motion = RoverMotion { up, ..default() };
            *health = Health::new(ROVER_MAX_HEALTH);
            *visibility = rover;
        }
        self.focus.0 = start;

        *self.death = Death::default();
        self.survival.0 = 0.0;
        self.rebuilt.write(WorldRebuilt { seed: seed.0 });
    }

    /// Идёт переход через затемнение: экраны ввод не принимают.
    pub(crate) fn busy(&self) -> bool {
        self.transition.busy()
    }

    /// Левая кнопка мыши нажата в этом кадре.
    pub(crate) fn clicked(&self) -> bool {
        self.mouse.just_pressed(MouseButton::Left)
    }

    /// Новый забег на новом рельефе — через затемнение.
    pub(crate) fn start_run(&mut self) {
        // Нажатие, которое начало забег, не стреляет: кнопка считается отпущенной до нового нажатия.
        self.mouse.reset_all();
        self.transition.request(Action::StartRun);
    }

    /// Новый забег сейчас: занавес уже закрыт.
    fn start_run_now(&mut self) {
        self.reset(Visibility::Inherited);
        self.started.write(RunStarted);
        self.mouse.reset_all();
        self.next.set(GameState::Playing);
    }

    /// Продолжение после паузы. Кнопка мыши, зажатая на паузе, не стреляет.
    fn resume(&mut self) {
        self.mouse.reset_all();
        self.next.set(GameState::Playing);
    }

    /// Выход в меню через затемнение.
    fn open_menu(&mut self) {
        self.transition.request(Action::OpenMenu);
    }

    /// Выход в меню сейчас: мир забега убран, планета снова летит под камерой.
    fn open_menu_now(&mut self) {
        self.reset(Visibility::Hidden);
        self.next.set(GameState::Menu);
    }
}

/// Что сделать за закрытым занавесом.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    StartRun,
    OpenMenu,
}

/// Ход перехода через затемнение.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
enum Stage {
    #[default]
    Idle,
    /// Занавес закрывается.
    Closing(Action),
    /// Мир перестроен, ждём, пока кадры выровняются.
    Settling { frames: u32, steady: u32, waited: f32 },
}

/// Переход между меню и забегом: пока он идёт, экраны не принимают ввод.
#[derive(Resource, Debug, Default)]
pub struct Transition {
    stage: Stage,
}

impl Transition {
    /// Заказать переход; повторный заказ во время перехода не действует.
    fn request(&mut self, action: Action) {
        if self.stage == Stage::Idle {
            self.stage = Stage::Closing(action);
        }
    }

    /// Идёт переход.
    pub fn busy(&self) -> bool {
        self.stage != Stage::Idle
    }

    /// Шаг перехода за кадр длиной `dt`: что выполнить сейчас и пора ли открывать занавес.
    fn advance(&mut self, closed: bool, dt: f32) -> (Option<Action>, bool) {
        match &mut self.stage {
            Stage::Idle => (None, false),
            Stage::Closing(action) => {
                if !closed {
                    return (None, false);
                }
                let action = *action;
                self.stage = Stage::Settling {
                    frames: 0,
                    steady: 0,
                    waited: 0.0,
                };
                (Some(action), false)
            }
            Stage::Settling { frames, steady, waited } => {
                *frames += 1;
                *waited += dt;
                *steady = if dt < STEADY_FRAME { *steady + 1 } else { 0 };
                let settled = (*frames >= SETTLE_FRAMES && *steady >= STEADY_FRAMES) || *waited > SETTLE_LIMIT;
                if settled {
                    self.stage = Stage::Idle;
                }
                (None, settled)
            }
        }
    }
}

/// Ведёт переход: закрывает занавес, за ним перестраивает мир, ждёт ровных кадров и открывает.
fn run_transition(time: Res<Time<Real>>, mut curtain: ResMut<Curtain>, mut world: RunWorld) {
    if let Stage::Closing(_) = world.transition.stage {
        curtain.close(CLOSE_SECONDS);
    }
    let (action, open) = world.transition.advance(curtain.is_closed(), time.delta_secs());
    match action {
        Some(Action::StartRun) => world.start_run_now(),
        Some(Action::OpenMenu) => world.open_menu_now(),
        None => {}
    }
    if open {
        curtain.open(OPEN_SECONDS);
    }
}

/// Запуск игры: первый мир объявляется так же, как перестроенный, и меню показывает планету
/// с посёлками и путями.
fn announce_first_world(seed: Res<RunSeed>, mut rebuilt: MessageWriter<WorldRebuilt>) {
    rebuilt.write(WorldRebuilt { seed: seed.0 });
}

/// Фон меню: фокус, за которым следят камера и подгрузка мира, медленно летит по прямой.
/// Луноход в меню спрятан и стоит на старте: игровые системы его не видят и не двигают.
pub(crate) fn fly_over_planet(
    time: Res<Time>,
    terrain: Res<Terrain>,
    mut focus: ResMut<Focus>,
    mut rover: Query<&mut Visibility, With<Rover>>,
) {
    for mut visibility in &mut rover {
        if *visibility != Visibility::Hidden {
            *visibility = Visibility::Hidden;
        }
    }
    let mut position = focus.0 + MENU_FLIGHT * time.delta_secs();
    position.y = terrain.height_at(position.x, position.z) + ROVER_RIDE_HEIGHT;
    focus.0 = position;
}

/// Строка лучшего результата для меню.
pub(crate) fn best_line(best: Option<&Record>) -> Option<String> {
    best.map(|record| format!("BEST {}  {}", format_time(record.seconds as f32), record.name))
}

/// Строка-подсказка: свой текст для клавиатуры и для геймпада (метки кнопок — см. `DeviceHint`).
pub(crate) fn hint_line(keys: impl Into<String>, pad: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    let hint = DeviceHint::new(keys, pad);
    (screen_line(hint.keys.clone(), size, color), hint)
}

/// Строка-подсказка со своим текстом и для касаний.
pub(crate) fn hint_line_touch(
    keys: impl Into<String>,
    pad: impl Into<String>,
    touch: impl Into<String>,
    size: f32,
    color: Color,
) -> impl Bundle {
    let hint = DeviceHint::new(keys, pad).with_touch(touch);
    (screen_line(hint.keys.clone(), size, color), hint)
}

/// Строка текста на экране.
pub(crate) fn screen_line(text: impl Into<String>, size: f32, color: Color) -> impl Bundle {
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
pub(crate) fn screen_root(shade: f32) -> impl Bundle {
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

pub(crate) const TITLE_COLOR: Color = Color::srgb(1.0, 0.85, 0.3);
pub(crate) const HINT_COLOR: Color = Color::srgb(0.8, 0.8, 0.8);

/// Убирает экран с маркером `T`.
pub(crate) fn hide_screen<T: Component>(mut commands: Commands, screens: Query<Entity, With<T>>) {
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

/// Итог забега: если время достойно таблицы, игрок вводит имя, а наружу уходит [`RecordPlaced`].
fn prepare_result(
    survival: Res<SurvivalTime>,
    records: Res<RecordsTable>,
    difficulty: Res<RunDifficulty>,
    mut result: ResMut<RunResult>,
    mut placed: MessageWriter<RecordPlaced>,
) {
    // Таблица — уровня сложности этого забега.
    let table = records.table(difficulty.0);
    let place = table.place_for(f64::from(survival.0));
    *result = RunResult {
        seconds: survival.0,
        place,
        entry: place.map(|_| NameEntry::new(&table.last_name)),
        input_delay: GAME_OVER_INPUT_DELAY,
    };
    if let Some(place) = place {
        placed.write(RecordPlaced {
            place,
            seconds: survival.0,
        });
    }
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
pub(crate) fn table_row(place: usize, record: &Record) -> String {
    format!("{:>2}. {:<5}  {:>7}", place + 1, record.name, format_time(record.seconds as f32))
}

/// Перерисовывает экран конца забега, когда итог изменился: ввод имени или таблица.
fn draw_game_over(
    mut commands: Commands,
    result: Res<RunResult>,
    records: Res<RecordsTable>,
    difficulty: Res<RunDifficulty>,
    device: Res<InputDevice>,
    ui_scale: Res<UiScale>,
    old: Query<Entity, With<GameOverScreen>>,
) {
    // Устройство сменилось — перерисовать: у касаний своя клавиатура и кнопки.
    if !result.is_changed() && !(device.is_changed() && !old.is_empty()) {
        return;
    }
    let touch = *device == InputDevice::Touch;
    let run_table = records.table(difficulty.0);
    // Первый показ проявляется, перерисовка при вводе имени — сразу.
    let appear = if old.is_empty() { Appear::over(GAME_OVER_APPEAR) } else { Appear::shown() };
    for screen in &old {
        commands.entity(screen).despawn();
    }
    // Ввод имени касаниями: клавиатура в dp высокая, и на телефоне высотой 360 dp экран не влез бы.
    // Строки сложности и подсказки нет, промежутки меньше.
    let compact = touch && result.entry.is_some();
    let mut root = commands.spawn((GameOverScreen, screen_root(0.62), appear));
    if compact {
        root.entry::<Node>().and_modify(|mut node| node.row_gap = Val::Px(COMPACT_GAP));
    }
    root.with_children(|screen| {
        screen.spawn(screen_line("ROVER LOST", 56.0, Color::srgb(1.0, 0.35, 0.25)));
        screen.spawn(screen_line(format!("TIME  {}", format_time(result.seconds)), 34.0, Color::WHITE));
        screen.spawn(screen_line(rank_line(result.place), 26.0, TITLE_COLOR));
        if !compact {
            screen.spawn(screen_line(format!("DIFFICULTY: {}", difficulty.0.title()), 20.0, HINT_COLOR));
        }

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
            if compact {
                spawn_keyboard(screen, &ui_scale);
            } else {
                screen.spawn(hint_line_touch(
                    "LETTERS, DIGITS, BACKSPACE     ENTER - SAVE",
                    format!("{{A}} - SAVE AS {}", run_table.last_name),
                    format!("TYPE A NAME OR SAVE AS {}", run_table.last_name),
                    20.0,
                    HINT_COLOR,
                ));
            }
            return;
        }

        screen
            .spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                ..default()
            })
            .with_children(|table| {
                for (place, record) in run_table.records.iter().enumerate() {
                    let color = if result.place == Some(place) { TITLE_COLOR } else { Color::WHITE };
                    table.spawn(screen_line(table_row(place, record), 22.0, color));
                }
            });
        if touch {
            let gap = Val::Px(crate::touch::dp_to_ui(16.0, &ui_scale));
            screen
                .spawn(Node {
                    column_gap: gap,
                    margin: UiRect::top(gap),
                    ..default()
                })
                .with_children(|buttons| {
                    use crate::touch::TapAction;
                    buttons.spawn(touch_key(TapAction::Key(KeyCode::Enter), "RESTART", 130.0, &ui_scale, TITLE_COLOR));
                    buttons.spawn(touch_key(TapAction::Key(KeyCode::Escape), "MENU", 130.0, &ui_scale, Color::WHITE));
                });
        } else {
            screen.spawn(hint_line("ENTER - RESTART     ESC - MENU", "{A} - RESTART     {B} - MENU", 22.0, HINT_COLOR));
        }
    });
}

/// Конец забега. Пока вводится имя: буквы, цифры, Backspace, Enter сохраняет результат.
/// После этого Enter начинает новый забег, Esc возвращает в меню. Первые
/// [`GAME_OVER_INPUT_DELAY`] секунд клавиши не принимаются.
fn game_over_input(
    time: Res<Time<Real>>,
    keys: Res<ButtonInput<KeyCode>>,
    device: Res<InputDevice>,
    difficulty: Res<RunDifficulty>,
    mut result: ResMut<RunResult>,
    mut records: ResMut<RecordsTable>,
    mut world: RunWorld,
) {
    if world.transition.busy() {
        return;
    }
    if result.input_delay > 0.0 {
        // Без отметки об изменении: экран перерисовывается только по вводу.
        result.bypass_change_detection().input_delay -= time.delta_secs();
        return;
    }
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
        // С геймпада букв не набрать: «да» сохраняет под прежним именем.
        let keep_last = escape || (enter && device.stick());
        let last = &records.table(difficulty.0).last_name;
        let name = entry.name().or_else(|| keep_last.then(|| last.clone()));
        if let (true, Some(name)) = (enter || escape, name) {
            result.place = records.add(difficulty.0, &name, result.seconds);
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

/// Пункты экрана паузы по порядку, их клавиши-подсказки и подписи для геймпада.
const PAUSE_ITEMS: [(&str, KeyCode, &str); 3] = [
    ("CONTINUE - ESC", KeyCode::Escape, "CONTINUE - {B}"),
    ("RESTART - R", KeyCode::KeyR, "RESTART"),
    ("MENU - M", KeyCode::KeyM, "MENU"),
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
    bench: Option<Res<crate::perf::Bench>>,
    mut focus: MessageReader<WindowFocused>,
    mut lifecycle: MessageReader<bevy::window::AppLifecycle>,
    mut next: ResMut<NextState<GameState>>,
    targeting: Res<crate::airstrike::Targeting>,
    silence: Option<Res<crate::sound::SystemSilence>>,
) {
    // В прогоне замера и при отладочном автозапуске окно может быть и не в фокусе.
    let unattended = bench.is_some_and(|b| b.on) || std::env::var_os("SPACE_GUARD_AUTOSTART").is_some();
    // На телефоне уход в фон — ещё и `Suspended`: фокус winit тоже теряет, но пауза не должна
    // зависеть от порядка этих событий.
    let suspended = lifecycle.read().any(|event| {
        matches!(event, bevy::window::AppLifecycle::WillSuspend | bevy::window::AppLifecycle::Suspended)
    });
    // Звонок поверх игры (Android): окно на экране, но играть сейчас некогда.
    let call = silence.is_some_and(|silence| silence.is_changed() && silence.0);
    let blurred = (focus.read().any(|change| !change.focused) || suspended || call) && !unattended;
    // Esc в наведении авиаудара отменяет его, а не ставит паузу.
    if blurred || (keys.just_pressed(KeyCode::Escape) && !targeting.open) {
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
        .spawn((PauseScreen, screen_root(0.62), Appear::over(PAUSE_APPEAR)))
        .with_children(|screen| {
            screen.spawn(screen_line("PAUSED", 56.0, TITLE_COLOR));
            screen.spawn(screen_line("", 10.0, Color::NONE));
            for (index, (label, _, pad)) in PAUSE_ITEMS.iter().enumerate() {
                let touch = label.split(" - ").next().unwrap_or(label);
                screen.spawn((PauseItem(index), hint_line_touch(*label, *pad, touch, 30.0, HINT_COLOR)));
            }
            screen.spawn(screen_line("", 10.0, Color::NONE));
            screen.spawn(hint_line_touch(
                "UP, DOWN AND ENTER TO CHOOSE",
                "D-PAD AND {A} TO CHOOSE",
                "TAP TO CHOOSE",
                18.0,
                HINT_COLOR,
            ));
        });
}

/// Пауза: стрелки и Enter выбирают пункт, клавиша-подсказка срабатывает сразу.
fn pause_input(
    keys: Res<ButtonInput<KeyCode>>,
    tap: Res<crate::touch::TouchTap>,
    items: Query<(&PauseItem, &ComputedNode, &UiGlobalTransform)>,
    window: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut choice: ResMut<PauseChoice>,
    mut world: RunWorld,
) {
    if world.transition.busy() {
        return;
    }
    // Касание пункта выполняет его сразу.
    let by_tap = tap
        .0
        .and_then(|point| {
            let scale = window.single().map_or(1.0, |w| w.scale_factor());
            crate::touch::nearest_row(point, scale, items.iter().map(|(item, node, t)| (item.0, node, t)))
        });
    if keys.any_just_pressed([KeyCode::ArrowUp, KeyCode::KeyW]) {
        choice.0 = moved_choice(choice.0, -1);
    }
    if keys.any_just_pressed([KeyCode::ArrowDown, KeyCode::KeyS]) {
        choice.0 = moved_choice(choice.0, 1);
    }
    let by_hint = PAUSE_ITEMS.iter().position(|(_, key, _)| keys.just_pressed(*key));
    let by_enter = keys
        .any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter])
        .then_some(choice.0);
    match by_tap.or(by_hint).or(by_enter) {
        Some(0) => world.resume(),
        Some(1) => world.start_run(),
        Some(2) => world.open_menu(),
        _ => {}
    }
}

/// Выбранный пункт паузы белый, остальные серые.
fn highlight_pause_choice(choice: Res<PauseChoice>, mut items: Query<(&PauseItem, &mut TextColor)>) {
    for (item, mut color) in &mut items {
        // Прозрачность не трогаем: ей управляет проявление экрана.
        let wanted = if item.0 == choice.0 { Color::WHITE } else { HINT_COLOR }.with_alpha(color.0.alpha());
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
        assert_eq!(table_row(0, &record), " 1. IVA     3:07.2");
        assert_eq!(table_row(9, &record), "10. IVA     3:07.2");
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
        entry.type_char('n');
        entry.type_char('x');
        entry.type_char('y');
        assert_eq!(entry.name().as_deref(), Some("IVANX"));
    }

    #[test]
    fn backspace_clears_the_cell_and_name_ends_at_first_empty_cell() {
        let mut entry = NameEntry::new("LUN");
        entry.backspace();
        assert_eq!(entry.name().as_deref(), Some("LUN"));
        entry.type_char('A');
        entry.type_char('B');
        entry.backspace();
        assert_eq!(entry.name().as_deref(), Some("A"));
        assert_eq!(entry.cursor, 1);
        entry.type_char('7');
        assert_eq!(entry.name().as_deref(), Some("A7N"));
        entry.backspace();
        entry.backspace();
        assert_eq!(entry.name(), None);
    }

    #[test]
    fn keys_give_letters_and_digits() {
        assert_eq!(key_char(KeyCode::KeyA), Some('A'));
        assert_eq!(key_char(KeyCode::KeyZ), Some('Z'));
        assert_eq!(key_char(KeyCode::Digit0), Some('0'));
        assert_eq!(key_char(KeyCode::Digit9), Some('9'));
        assert_eq!(key_char(KeyCode::Space), None);
        for letter in ['A', 'Q', 'Z', '0', '7', '9'] {
            assert_eq!(char_key(letter).and_then(key_char), Some(letter));
        }
    }

    #[test]
    fn переход_один_на_несколько_нажатий_и_ждёт_ровных_кадров() {
        let mut transition = Transition::default();
        transition.request(Action::StartRun);
        transition.request(Action::OpenMenu);
        assert!(transition.busy());
        // Занавес ещё закрывается: мир не трогаем.
        assert_eq!(transition.advance(false, 0.016), (None, false));
        // Закрылся: выполняется первый заказ, второй забыт.
        assert_eq!(transition.advance(true, 0.016), (Some(Action::StartRun), false));
        // Долгий кадр перестройки сбрасывает счёт ровных кадров.
        for _ in 0..SETTLE_FRAMES {
            assert_eq!(transition.advance(true, 0.1), (None, false));
        }
        let mut opened = false;
        for _ in 0..STEADY_FRAMES {
            let (action, open) = transition.advance(true, 0.016);
            assert_eq!(action, None);
            opened = open;
        }
        assert!(opened);
        assert!(!transition.busy());
    }

    #[test]
    fn pause_choice_wraps_around() {
        assert_eq!(moved_choice(0, 1), 1);
        assert_eq!(moved_choice(2, 1), 0);
        assert_eq!(moved_choice(0, -1), 2);
    }

    #[test]
    fn результат_на_лёгкой_не_попадает_в_таблицу_нормальной() {
        let mut records = RecordsTable::default();
        assert_eq!(records.add(Difficulty::Easy, "IVA", 90.0), Some(0));
        assert_eq!(records.table(Difficulty::Easy).records.len(), 1);
        assert!(records.table(Difficulty::Normal).records.is_empty());
        assert!(records.table(Difficulty::Hard).records.is_empty());
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
