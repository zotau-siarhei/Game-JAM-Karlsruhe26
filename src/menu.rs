//! Главное меню и его подэкраны: «Одиночная игра», «Мультиплеер» (вход через Google,
//! `google_auth`), «Сложность», «Настройки», «Рекорды», «Авторы», «Выход» (в браузере его нет).
//!
//! Меню — текст на затемнении поверх пролёта над планетой (`run::fly_over_planet`). Страница и
//! выделенная строка — ресурс [`Menu`]; экран перерисовывается целиком, когда меняется страница
//! или значение на ней, а подсветка строки меняется отдельно, как у паузы.
//!
//! Управление одно на все устройства: стрелки и W/S двигают выделение, стрелки влево-вправо (A/D)
//! меняют значения, Enter, пробел или клик выбирают, Esc возвращает из подэкрана. Геймпад нажимает
//! те же клавиши (`gamepad::virtual_for`), мышь выделяет строку наведением.

use bevy::app::AppExit;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::UiGlobalTransform;

use crate::difficulty::{Difficulty, RunDifficulty};
use crate::fade::Appear;
use crate::gamepad::InputDevice;
use crate::google_auth::{AuthRequest, SignIn};
use crate::touch::{TouchSwipe, TouchTap, nearest_row, node_contains};
use crate::frame::GameSet;
use crate::quality::Quality;
use crate::run::{self, GameState, RecordsTable, RunWorld, HINT_COLOR, TITLE_COLOR};
use crate::settings::{Settings, VOLUME_STEP};
use crate::sound::PlaySound;

/// Авторы игры по алфавиту фамилий: экран «Авторы».
pub const AUTHORS: &[&str] = &["Nidzhat Guseynov", "Ivan Tsarev", "Sergei Zotov"];

/// Цвет невыделенной строки.
const ROW_COLOR: Color = Color::srgb(0.7, 0.72, 0.76);
/// Размер шрифта строк меню.
const ROW_SIZE: f32 = 30.0;
/// Ширина подписи в строке со значением, точек: значения встают в столбик.
const LABEL_WIDTH: f32 = 230.0;
/// Ширина значения в строке настроек, точек: в неё входит самое длинное, «< MEDIUM >».
const VALUE_WIDTH: f32 = 200.0;

/// Плагин главного меню.
pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Menu>()
            .add_systems(OnEnter(GameState::Menu), open_main)
            .add_systems(OnExit(GameState::Menu), run::hide_screen::<MenuScreen>)
            .add_systems(
                Update,
                (menu_input, draw_menu, highlight_rows)
                    .chain()
                    // После полёта над планетой: иначе полёт спрятал бы луноход, только что
                    // поставленный на старт.
                    .after(run::fly_over_planet)
                    .run_if(in_state(GameState::Menu))
                    .in_set(MenuInput)
                    .in_set(GameSet::Screens),
            );
    }
}

/// Страница меню.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MenuPage {
    #[default]
    Main,
    Difficulty,
    Settings,
    /// Таблица рекордов этого уровня: переключается стрелками, уровень для игры не меняет.
    Records(Difficulty),
    Authors,
    /// Мультиплеер: вход через Google.
    Online,
}

/// Ввод, отрисовка и подсветка меню.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct MenuInput;

/// Открытая страница и выделенная строка.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Menu {
    pub page: MenuPage,
    pub selected: usize,
}

/// Строка настроек.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Setting {
    Quality,
    Master,
    Music,
    Effects,
    Voice,
    Subtitles,
    Vibration,
    TouchSize,
    TouchOpacity,
}

/// Выбираемая строка меню.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    /// Одиночная игра: прежний забег.
    Single,
    /// Мультиплеер: экран входа.
    Online,
    /// Действие входа на экране мультиплеера.
    Auth(AuthRequest),
    OpenDifficulty,
    OpenSettings,
    OpenRecords,
    OpenAuthors,
    Quit,
    Choose(Difficulty),
    Setting(Setting),
    /// Переключатель таблицы на экране рекордов.
    RecordsTable,
    Back,
}

/// Пункты главного меню. «Выход» — по профилю платформы (`menu.quit`): в браузере закрыть вкладку
/// может только сама страница, а на iPhone и iPad приложения не закрывают сами (правила Apple).
fn main_items() -> Vec<Item> {
    let mut items = vec![Item::Single, Item::Online, Item::OpenDifficulty, Item::OpenSettings, Item::OpenRecords, Item::OpenAuthors];
    if crate::profile::get().menu.quit {
        items.push(Item::Quit);
    }
    items
}

/// Строки страницы по порядку выделения. `pad` — подключён геймпад: без него строки вибрации нет;
/// `touch` — игрок пользуется касаниями: тогда есть строки сенсорных элементов; `online` — состояние
/// входа для экрана мультиплеера.
fn items(page: MenuPage, pad: bool, touch: bool, online: &SignIn) -> Vec<Item> {
    match page {
        MenuPage::Online => online_items(online),
        MenuPage::Main => main_items(),
        MenuPage::Difficulty => {
            let mut items: Vec<Item> = Difficulty::ALL.iter().map(|d| Item::Choose(*d)).collect();
            items.push(Item::Back);
            items
        }
        MenuPage::Settings => {
            let mut items: Vec<Item> = [
                Setting::Quality,
                Setting::Master,
                Setting::Music,
                Setting::Effects,
                Setting::Voice,
                Setting::Subtitles,
                Setting::Vibration,
                Setting::TouchSize,
                Setting::TouchOpacity,
            ]
            .map(Item::Setting)
            .to_vec();
            items.retain(|item| match item {
                // Вибрации геймпада на Android нет (`input.rumble` профиля).
                Item::Setting(Setting::Vibration) => pad && crate::profile::get().input.rumble,
                Item::Setting(Setting::TouchSize | Setting::TouchOpacity) => touch,
                _ => true,
            });
            items.push(Item::Back);
            items
        }
        MenuPage::Records(_) => vec![Item::RecordsTable, Item::Back],
        MenuPage::Authors => vec![Item::Back],
    }
}

/// Строки экрана мультиплеера по состоянию входа.
fn online_items(online: &SignIn) -> Vec<Item> {
    match online {
        SignIn::Unsupported | SignIn::NotConfigured => vec![Item::Back],
        SignIn::SignedOut | SignIn::Failed(_) => vec![Item::Auth(AuthRequest::SignIn), Item::Back],
        SignIn::Requesting => vec![Item::Auth(AuthRequest::Cancel)],
        SignIn::Waiting { .. } => vec![Item::Auth(AuthRequest::OpenPage), Item::Auth(AuthRequest::Cancel)],
        SignIn::SignedIn(_) => vec![Item::Auth(AuthRequest::SignOut), Item::Back],
    }
}

/// Строка, которая выделяется, когда сменилось состояние входа. После входа — «Назад», а не «Выйти»:
/// иначе Enter, нажатый в этот миг, сразу вышел бы из аккаунта.
fn online_default(online: &SignIn) -> usize {
    let list = online_items(online);
    match online {
        SignIn::SignedIn(_) => list.iter().position(|item| *item == Item::Back).unwrap_or(0),
        _ => 0,
    }
}

/// Номер строки после шага вверх или вниз: список замкнут в кольцо.
fn moved(selected: usize, step: i32, count: usize) -> usize {
    (selected as i32 + step).rem_euclid(count.max(1) as i32) as usize
}

/// Громкость после шага: в пределах 0–100 %, `wrap` — со 100 % на 0 по кругу.
fn stepped_volume(volume: u8, step: i32, wrap: bool) -> u8 {
    let next = i32::from(volume) + step * i32::from(VOLUME_STEP);
    if wrap && next > 100 {
        return 0;
    }
    next.clamp(0, 100) as u8
}

/// Меняет настройку на шаг `step`; `wrap` — по кругу (Enter, клик), иначе громкость упирается в край.
fn change_setting(settings: &mut Settings, setting: Setting, step: i32, wrap: bool) {
    match setting {
        Setting::Quality => settings.quality = settings.quality.step(step),
        Setting::Master => settings.master = stepped_volume(settings.master, step, wrap),
        Setting::Music => settings.music = stepped_volume(settings.music, step, wrap),
        Setting::Effects => settings.effects = stepped_volume(settings.effects, step, wrap),
        Setting::Voice => settings.voice = stepped_volume(settings.voice, step, wrap),
        Setting::Subtitles => settings.subtitles = !settings.subtitles,
        Setting::Vibration => settings.vibration = !settings.vibration,
        Setting::TouchSize => settings.touch_size = settings.touch_size.step(step),
        Setting::TouchOpacity => settings.touch_opacity = stepped_opacity(settings.touch_opacity, step, wrap),
    }
}

/// Непрозрачность сенсорных элементов после шага: 10–100 %, `wrap` — со 100 % на 10 %.
fn stepped_opacity(opacity: u8, step: i32, wrap: bool) -> u8 {
    let next = i32::from(opacity) + step * 10;
    if wrap && next > 100 {
        return 10;
    }
    next.clamp(10, 100) as u8
}

/// Главное меню после возврата с подэкрана: выделен пункт этого подэкрана.
fn back_to_main(from: MenuPage) -> Menu {
    let item = match from {
        MenuPage::Main => Item::Single,
        MenuPage::Online => Item::Online,
        MenuPage::Difficulty => Item::OpenDifficulty,
        MenuPage::Settings => Item::OpenSettings,
        MenuPage::Records(_) => Item::OpenRecords,
        MenuPage::Authors => Item::OpenAuthors,
    };
    Menu {
        page: MenuPage::Main,
        selected: main_items().iter().position(|i| *i == item).unwrap_or(0),
    }
}

/// Корень экрана меню.
#[derive(Component)]
struct MenuScreen;

/// Выбираемая строка с её номером.
#[derive(Component)]
struct MenuRow(usize);

/// Текст строки, который подсвечивается.
#[derive(Component)]
struct RowText(usize);

/// Стрелка у выделенной строки.
#[derive(Component)]
struct RowMarker(usize);

/// Значение строки настроек: касание левой половины уменьшает, правой — увеличивает.
#[derive(Component)]
struct RowValue(usize);

/// Вход в меню: главная страница, выделена «Одиночная игра».
fn open_main(mut menu: ResMut<Menu>) {
    *menu = Menu::default();
}

/// Ввод меню: выделение, значения, выбор и возврат.
#[allow(clippy::too_many_arguments)]
fn menu_input(
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    bench: Option<Res<crate::perf::Bench>>,
    real: Res<Time<Real>>,
    rows: Query<(&MenuRow, &Hovered, &ComputedNode, &UiGlobalTransform)>,
    (tap, swipe, device, values, window): (
        Res<TouchTap>,
        Res<TouchSwipe>,
        Res<InputDevice>,
        Query<(&RowValue, &ComputedNode, &UiGlobalTransform, Option<&bevy::text::TextLayoutInfo>)>,
        Query<&Window, With<bevy::window::PrimaryWindow>>,
    ),
    gamepads: Query<(), With<Gamepad>>,
    mut last_pad: Local<Option<bool>>,
    mut menu: ResMut<Menu>,
    mut settings: ResMut<Settings>,
    mut quality: ResMut<Quality>,
    mut run_difficulty: ResMut<RunDifficulty>,
    mut world: RunWorld,
    (mut sounds, mut exit, mut auth): (MessageWriter<PlaySound>, MessageWriter<AppExit>, MessageWriter<AuthRequest>),
    (online, mut last_online): (Res<SignIn>, Local<Option<std::mem::Discriminant<SignIn>>>),
) {
    if world.busy() {
        return;
    }
    // Состояние входа сменилось: на экране мультиплеера другие строки, выделяется подходящая.
    let kind = std::mem::discriminant(&*online);
    if last_online.replace(kind).is_some_and(|last| last != kind) && menu.page == MenuPage::Online {
        menu.set_if_neq(Menu {
            selected: online_default(&online),
            ..*menu
        });
    }
    // Геймпад подключили или отключили: строка вибрации появилась или исчезла, выделение
    // остаётся на той же строке, а с исчезнувшей встаёт на соседнюю выше.
    let pad = !gamepads.is_empty();
    let touch = *device == InputDevice::Touch;
    let was_pad = last_pad.replace(pad).unwrap_or(pad);
    let list = items(menu.page, pad, touch, &online);
    if was_pad != pad {
        let old = items(menu.page, was_pad, touch, &online);
        let item = old.get(menu.selected).copied();
        let selected = item
            .and_then(|item| list.iter().position(|i| *i == item))
            .unwrap_or(menu.selected.saturating_sub(1));
        menu.set_if_neq(Menu {
            selected: selected.min(list.len().saturating_sub(1)),
            ..*menu
        });
    }
    let mut next = *menu;
    let mut click = false;

    // Наведение мыши выделяет строку, но только когда мышь двигалась: иначе строка под стоящим
    // курсором отбирала бы выделение у клавиатуры.
    let hovered = rows.iter().find(|(_, hovered, ..)| hovered.get()).map(|(row, ..)| row.0);
    let scale = window.single().map_or(1.0, |w| w.scale_factor());
    // Касание строки выбирает её; касание значения настройки — левой или правой половины — меняет его.
    let tapped = tap
        .0
        .and_then(|point| nearest_row(point, scale, rows.iter().map(|(row, _, node, transform)| (row.0, node, transform))));
    let tapped_value = tap.0.and_then(|point| {
        values
            .iter()
            .find(|(_, node, transform, _)| node_contains(node, transform, point, scale))
            .map(|(value, node, transform, text)| {
                // Текст значения прижат к левому краю узла: середина — по ширине текста, а не узла,
                // иначе «>» у короткого значения («0%», «ON») попадала в левую половину и уменьшала.
                let left = transform.translation.x - node.size().x * 0.5;
                let middle = text.map_or(transform.translation.x, |text| left + text.size.x * 0.5);
                (value.0, if point.x * scale < middle { -1 } else { 1 })
            })
    });
    if let Some(row) = tapped.or(tapped_value.map(|(row, _)| row)) {
        next.selected = row;
    }
    if motion.delta != Vec2::ZERO
        && let Some(row) = hovered
    {
        next.selected = row;
    }
    if keys.any_just_pressed([KeyCode::ArrowUp, KeyCode::KeyW]) {
        next.selected = moved(next.selected, -1, list.len());
    }
    if keys.any_just_pressed([KeyCode::ArrowDown, KeyCode::KeyS]) {
        next.selected = moved(next.selected, 1, list.len());
    }
    if next.selected != menu.selected {
        click = true;
    }
    let selected = list.get(next.selected.min(list.len().saturating_sub(1))).copied();

    // Влево-вправо: значение настройки или таблица рекордов.
    let mut side = i32::from(keys.any_just_pressed([KeyCode::ArrowRight, KeyCode::KeyD]))
        - i32::from(keys.any_just_pressed([KeyCode::ArrowLeft, KeyCode::KeyA]));
    if let Some((_, step)) = tapped_value {
        side = step;
    } else if swipe.0 != 0 && matches!(menu.page, MenuPage::Records(_)) {
        // Свайп влево — следующая таблица, как перелистывание.
        side = -swipe.0;
    }
    if side != 0 {
        match (menu.page, selected) {
            (MenuPage::Records(shown), _) => {
                next.page = MenuPage::Records(shown.step(side));
                click = true;
            }
            (_, Some(Item::Setting(setting))) => {
                let mut changed = settings.clone();
                change_setting(&mut changed, setting, side, false);
                click |= changed != *settings;
                settings.set_if_neq(changed);
            }
            _ => {}
        }
    }

    // Выбор: Enter, пробел, клик по строке. Прогон замера начинает забег сам через секунду.
    let by_mouse = world.clicked() && hovered.is_some();
    if by_mouse {
        next.selected = hovered.unwrap_or(next.selected);
    }
    let by_tap = tapped.is_some() && tapped_value.is_none();
    let chosen = keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter, KeyCode::Space]) || by_mouse || by_tap;
    let bench_start = bench.is_some_and(|b| b.on) && real.elapsed_secs() > 1.0 && menu.page == MenuPage::Main;
    let activated = if bench_start {
        Some(Item::Single)
    } else if chosen {
        list.get(next.selected).copied()
    } else {
        None
    };
    if let Some(item) = activated {
        click = true;
        match item {
            Item::Single => {
                // Забег идёт на уровне, выбранном сейчас; рестарт возьмёт тот же.
                run_difficulty.set_if_neq(RunDifficulty(settings.difficulty));
                world.start_run();
            }
            Item::Online => {
                next = Menu {
                    page: MenuPage::Online,
                    selected: online_default(&online),
                };
            }
            Item::Auth(request) => {
                auth.write(request);
            }
            Item::OpenDifficulty => {
                next = Menu {
                    page: MenuPage::Difficulty,
                    selected: settings.difficulty.index(),
                };
            }
            Item::OpenSettings => next = Menu { page: MenuPage::Settings, selected: 0 },
            Item::OpenRecords => {
                next = Menu {
                    page: MenuPage::Records(settings.difficulty),
                    selected: 0,
                };
            }
            Item::OpenAuthors => next = Menu { page: MenuPage::Authors, selected: 0 },
            Item::Quit => {
                exit.write(AppExit::Success);
            }
            Item::Choose(difficulty) => {
                if settings.difficulty != difficulty {
                    settings.difficulty = difficulty;
                }
                next = back_to_main(MenuPage::Difficulty);
            }
            Item::Setting(setting) => {
                let mut changed = settings.clone();
                change_setting(&mut changed, setting, 1, true);
                settings.set_if_neq(changed);
            }
            Item::RecordsTable => {
                if let MenuPage::Records(shown) = next.page {
                    next.page = MenuPage::Records(shown.step(1));
                }
            }
            Item::Back => next = back_to_main(menu.page),
        }
    }

    // Esc и «назад» геймпада: из подэкрана — в главное меню, в главном меню — ничего (на Android
    // «назад» там сворачивает игру: `android::back_to_background`).
    // Во время входа Esc сначала отменяет вход, а экран мультиплеера остаётся.
    if keys.just_pressed(KeyCode::Escape) && menu.page != MenuPage::Main {
        if menu.page == MenuPage::Online && online.in_progress() {
            auth.write(AuthRequest::Cancel);
        } else {
            next = back_to_main(menu.page);
        }
        click = true;
    }

    // Качество меняется и ресурсом ступени: его применяет `quality.rs`.
    quality.set_if_neq(settings.quality);
    menu.set_if_neq(next);
    if click {
        sounds.write(PlaySound::new("ui_click"));
    }
}

/// Строка-подпись: подсвечивается, если это выбираемая строка.
fn row_text(text: impl Into<String>, row: usize) -> impl Bundle {
    (RowText(row), run::screen_line(text, ROW_SIZE, ROW_COLOR))
}

/// Стрелка у выделенной строки.
fn row_marker(text: &str, row: usize) -> impl Bundle {
    (RowMarker(row), run::screen_line(text, ROW_SIZE, TITLE_COLOR), Visibility::Hidden)
}

/// Выбираемая строка: подпись, необязательное значение и описание мелким шрифтом под ней.
fn spawn_row(parent: &mut ChildSpawnerCommands, row: usize, label: &str, value: Option<String>, note: Option<&str>) {
    parent
        .spawn((
            MenuRow(row),
            Hovered::default(),
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .with_children(|row_node| {
            let with_value = value.is_some();
            row_node
                .spawn(Node {
                    column_gap: Val::Px(14.0),
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|line| {
                    line.spawn(row_marker(">", row));
                    match value {
                        Some(value) => {
                            line.spawn((
                                Node {
                                    width: Val::Px(LABEL_WIDTH),
                                    ..default()
                                },
                                row_text(label, row),
                            ));
                            line.spawn((
                                RowValue(row),
                                Node {
                                    width: Val::Px(VALUE_WIDTH),
                                    justify_content: JustifyContent::Center,
                                    ..default()
                                },
                                TextLayout::no_wrap(),
                                row_text(value, row),
                            ));
                        }
                        None => {
                            line.spawn(row_text(label, row));
                        }
                    }
                    // У строки со значением стрелки «< >» уже стоят вокруг значения.
                    if !with_value {
                        line.spawn(row_marker("<", row));
                    }
                });
            if let Some(note) = note {
                row_node.spawn(run::screen_line(note, 16.0, HINT_COLOR));
            }
        });
}

/// Строка-переключатель «< ЗНАЧЕНИЕ >»: стрелки видны всегда, выделение — цветом.
fn spawn_switch_row(parent: &mut ChildSpawnerCommands, row: usize, label: &str) {
    parent
        .spawn((
            MenuRow(row),
            Hovered::default(),
            Node {
                column_gap: Val::Px(18.0),
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .with_children(|line| {
            line.spawn(run::screen_line("<", ROW_SIZE, TITLE_COLOR));
            line.spawn(row_text(label, row));
            line.spawn(run::screen_line(">", ROW_SIZE, TITLE_COLOR));
        });
}

/// Пустая строка-отступ.
fn spacer(parent: &mut ChildSpawnerCommands, height: f32) {
    parent.spawn(Node {
        height: Val::Px(height),
        ..default()
    });
}

/// Описание уровня сложности на экране «Сложность».
fn difficulty_note(difficulty: Difficulty) -> &'static str {
    match difficulty {
        Difficulty::Easy => "LESS DAMAGE, SLOWER WAVES, FAST SELF-REPAIR, STRONG GAMEPAD AIM ASSIST",
        Difficulty::Normal => "THE GAME AS DESIGNED",
        Difficulty::Hard => "MORE DAMAGE, FASTER WAVES, SLOW SELF-REPAIR, WEAK GAMEPAD AIM ASSIST",
    }
}

/// Значение строки настроек.
fn setting_value(settings: &Settings, setting: Setting) -> String {
    let percent = |value: u8| format!("{value}%");
    match setting {
        Setting::Quality => settings.quality.name().to_uppercase(),
        Setting::Master => percent(settings.master),
        Setting::Music => percent(settings.music),
        Setting::Effects => percent(settings.effects),
        Setting::Voice => percent(settings.voice),
        Setting::Subtitles => on_off(settings.subtitles),
        Setting::Vibration => on_off(settings.vibration),
        Setting::TouchSize => settings.touch_size.name().to_string(),
        Setting::TouchOpacity => percent(settings.touch_opacity),
    }
}

/// «Вкл» или «выкл».
fn on_off(on: bool) -> String {
    if on { "ON" } else { "OFF" }.to_string()
}

/// Подпись строки настроек.
fn setting_label(setting: Setting) -> &'static str {
    match setting {
        Setting::Quality => "QUALITY",
        Setting::Master => "VOLUME",
        Setting::Music => "MUSIC",
        Setting::Effects => "EFFECTS",
        Setting::Voice => "HQ VOICE",
        Setting::Subtitles => "SUBTITLES",
        Setting::Vibration => "VIBRATION",
        Setting::TouchSize => "CONTROLS",
        Setting::TouchOpacity => "OPACITY",
    }
}

/// Перерисовывает экран, когда сменилась страница или значение на ней.
fn draw_menu(
    mut commands: Commands,
    menu: Res<Menu>,
    settings: Res<Settings>,
    records: Res<RecordsTable>,
    gamepads: Query<(), With<Gamepad>>,
    device: Res<InputDevice>,
    online: Res<SignIn>,
    old: Query<Entity, With<MenuScreen>>,
    mut shown: Local<Option<(MenuPage, Settings, bool, bool, SignIn)>>,
) {
    let pad = !gamepads.is_empty();
    let touch = *device == InputDevice::Touch;
    let wanted = Some((menu.page, settings.clone(), pad, touch, online.clone()));
    if !old.is_empty() && *shown == wanted && !records.is_changed() {
        return;
    }
    // Первый показ проявляется, перерисовка — сразу.
    let appear = if old.is_empty() { Appear::over(run::MENU_APPEAR) } else { Appear::shown() };
    for screen in &old {
        commands.entity(screen).despawn();
    }
    *shown = wanted;

    let list = items(menu.page, pad, touch, &online);
    let index_of = |item: Item| list.iter().position(|i| *i == item).unwrap_or(0);
    commands
        .spawn((MenuScreen, run::screen_root(0.45), appear))
        .with_children(|screen| match menu.page {
            MenuPage::Main => {
                screen.spawn(run::screen_line("SPACE GUARD", 96.0, TITLE_COLOR));
                screen.spawn(run::screen_line("SURVIVE AS LONG AS YOU CAN", 26.0, Color::WHITE));
                let best = run::best_line(records.table(settings.difficulty).best());
                screen.spawn(run::screen_line(best.unwrap_or_default(), 22.0, TITLE_COLOR));
                spacer(screen, 4.0);
                for (row, item) in list.iter().enumerate() {
                    let (label, value) = match item {
                        Item::Single => ("SINGLE PLAYER", None),
                        Item::Online => ("MULTIPLAYER", None),
                        Item::OpenDifficulty => ("DIFFICULTY", Some(settings.difficulty.title().to_string())),
                        Item::OpenSettings => ("SETTINGS", None),
                        Item::OpenRecords => ("RECORDS", None),
                        Item::OpenAuthors => ("AUTHORS", None),
                        _ => ("QUIT", None),
                    };
                    match value {
                        // Название сложности — рядом с пунктом, одной строкой.
                        Some(value) => spawn_row(screen, row, &format!("{label}: {value}"), None, None),
                        None => spawn_row(screen, row, label, None, None),
                    }
                }
                spacer(screen, 4.0);
                screen.spawn(run::hint_line_touch(
                    "WASD - DRIVE     MOUSE - AIM     LEFT BUTTON - FIRE",
                    "LEFT STICK - DRIVE     RIGHT STICK - AIM     HOLD {CURSOR} - FREE AIM     {FIRE} - FIRE",
                    "LEFT THUMB - DRIVE     RIGHT THUMB - AIM AND FIRE     QUICK TAP - SHOOT NEAREST",
                    18.0,
                    HINT_COLOR,
                ));
                screen.spawn(run::hint_line_touch(
                    "1-6, WHEEL - WEAPON     HOLD SPACE - REPAIR     ESC - PAUSE",
                    "{BUMPERS} - WEAPON     HOLD {A} - REPAIR     {START} - PAUSE",
                    "TAP A WEAPON ICON - SWITCH     II - PAUSE",
                    18.0,
                    HINT_COLOR,
                ));
                screen.spawn(run::hint_line_touch(
                    "UP, DOWN - SELECT     ENTER - CHOOSE",
                    "D-PAD - SELECT     {A} - CHOOSE",
                    "TAP A LINE - CHOOSE",
                    18.0,
                    HINT_COLOR,
                ));
            }
            MenuPage::Difficulty => {
                screen.spawn(run::screen_line("DIFFICULTY", 56.0, TITLE_COLOR));
                spacer(screen, 4.0);
                for difficulty in Difficulty::ALL {
                    let label = if difficulty == settings.difficulty {
                        format!("{}  (SELECTED)", difficulty.title())
                    } else {
                        difficulty.title().to_string()
                    };
                    spawn_row(screen, index_of(Item::Choose(difficulty)), &label, None, Some(difficulty_note(difficulty)));
                }
                spacer(screen, 4.0);
                spawn_row(screen, index_of(Item::Back), "BACK", None, None);
                back_hint(
                    screen,
                    "UP, DOWN - SELECT     ENTER - CHOOSE     ESC - BACK",
                    "D-PAD - SELECT     {A} - CHOOSE     {B} - BACK",
                    "TAP A LINE - CHOOSE",
                );
            }
            MenuPage::Settings => {
                screen.spawn(run::screen_line("SETTINGS", 56.0, TITLE_COLOR));
                spacer(screen, 4.0);
                for (row, item) in list.iter().enumerate() {
                    if let Item::Setting(setting) = item {
                        let value = format!("< {} >", setting_value(&settings, *setting));
                        spawn_row(screen, row, setting_label(*setting), Some(value), None);
                    }
                }
                spacer(screen, 4.0);
                spawn_row(screen, index_of(Item::Back), "BACK", None, None);
                back_hint(
                    screen,
                    "UP, DOWN - SELECT     LEFT, RIGHT - CHANGE     ESC - BACK",
                    "D-PAD - SELECT     LEFT, RIGHT - CHANGE     {B} - BACK",
                    "TAP < OR > - CHANGE",
                );
            }
            MenuPage::Records(shown) => {
                screen.spawn(run::screen_line("RECORDS", 56.0, TITLE_COLOR));
                spawn_switch_row(screen, index_of(Item::RecordsTable), shown.title());
                screen
                    .spawn(Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(4.0),
                        min_height: Val::Px(250.0),
                        justify_content: JustifyContent::Center,
                        ..default()
                    })
                    .with_children(|table| {
                        let records = &records.table(shown).records;
                        if records.is_empty() {
                            table.spawn(run::screen_line("NO RECORDS YET", 22.0, HINT_COLOR));
                        }
                        for (place, record) in records.iter().enumerate() {
                            table.spawn(run::screen_line(run::table_row(place, record), 22.0, Color::WHITE));
                        }
                    });
                spawn_row(screen, index_of(Item::Back), "BACK", None, None);
                back_hint(
                    screen,
                    "LEFT, RIGHT - DIFFICULTY     ESC - BACK",
                    "LEFT, RIGHT - DIFFICULTY     {B} - BACK",
                    "SWIPE OR TAP THE TITLE - DIFFICULTY",
                );
            }
            MenuPage::Online => draw_online(screen, &online, &list),
            MenuPage::Authors => {
                screen.spawn(run::screen_line("AUTHORS", 56.0, TITLE_COLOR));
                spacer(screen, 4.0);
                screen.spawn(run::screen_line("MADE BY", 20.0, HINT_COLOR));
                for name in AUTHORS {
                    screen.spawn(run::screen_line(*name, 32.0, Color::WHITE));
                }
                spacer(screen, 8.0);
                spawn_row(screen, index_of(Item::Back), "BACK", None, None);
                back_hint(screen, "ENTER, ESC - BACK", "{A}, {B} - BACK", "TAP BACK");
            }
        });
}

/// Экран мультиплеера: состояние входа и его строки.
fn draw_online(screen: &mut ChildSpawnerCommands, online: &SignIn, list: &[Item]) {
    screen.spawn(run::screen_line("MULTIPLAYER", 56.0, TITLE_COLOR));
    spacer(screen, 4.0);
    let line = |screen: &mut ChildSpawnerCommands, text: &str, size: f32, color: Color| {
        screen.spawn(run::screen_line(text.to_string(), size, color));
    };
    match online {
        SignIn::Unsupported => {
            line(screen, "ONLINE PLAY IS ONLY IN THE APP", 26.0, Color::WHITE);
            line(screen, "ANDROID, IPHONE, IPAD OR COMPUTER", 18.0, HINT_COLOR);
        }
        SignIn::NotConfigured => {
            line(screen, "GOOGLE SIGN-IN IS NOT SET UP IN THIS BUILD", 26.0, Color::WHITE);
            line(screen, "SINGLE PLAYER WORKS AS USUAL", 18.0, HINT_COLOR);
        }
        SignIn::SignedOut => {
            line(screen, "SIGN IN WITH GOOGLE TO PLAY ONLINE", 26.0, Color::WHITE);
            line(screen, "SINGLE PLAYER NEEDS NO ACCOUNT", 18.0, HINT_COLOR);
        }
        SignIn::Requesting => line(screen, "CONTACTING GOOGLE...", 26.0, Color::WHITE),
        SignIn::Waiting { code, url, copied } => {
            let page = url.trim_start_matches("https://").trim_start_matches("www.");
            line(screen, &format!("OPEN {page} AND ENTER THE CODE"), 26.0, Color::WHITE);
            line(screen, code, 64.0, TITLE_COLOR);
            if *copied {
                line(screen, "THE CODE IS COPIED - PASTE IT ON THE PAGE", 18.0, HINT_COLOR);
            }
            line(screen, "THE GAME NOTICES BY ITSELF WHEN YOU CONFIRM", 18.0, HINT_COLOR);
        }
        SignIn::SignedIn(account) => {
            line(screen, "SIGNED IN AS", 18.0, HINT_COLOR);
            let name = if account.name.is_empty() { &account.email } else { &account.name };
            line(screen, name, 32.0, Color::WHITE);
            if !account.name.is_empty() && !account.email.is_empty() {
                line(screen, &account.email, 20.0, HINT_COLOR);
            }
            spacer(screen, 4.0);
            line(screen, "ONLINE PLAY IS COMING SOON", 22.0, TITLE_COLOR);
        }
        SignIn::Failed(reason) => {
            line(screen, reason, 26.0, TITLE_COLOR);
            if reason == crate::google_auth::NO_CONNECTION {
                line(screen, "CHECK THE INTERNET AND TRY AGAIN", 18.0, HINT_COLOR);
            }
        }
    }
    spacer(screen, 4.0);
    for (row, item) in list.iter().enumerate() {
        let label = match item {
            Item::Auth(AuthRequest::SignIn) if matches!(online, SignIn::Failed(_)) => "TRY AGAIN",
            Item::Auth(AuthRequest::SignIn) => "SIGN IN WITH GOOGLE",
            Item::Auth(AuthRequest::OpenPage) => "OPEN THE PAGE AGAIN",
            Item::Auth(AuthRequest::Cancel) => "CANCEL",
            Item::Auth(AuthRequest::SignOut) => "SIGN OUT",
            _ => "BACK",
        };
        spawn_row(screen, row, label, None, None);
    }
    back_hint(
        screen,
        "UP, DOWN - SELECT     ENTER - CHOOSE     ESC - BACK",
        "D-PAD - SELECT     {A} - CHOOSE     {B} - BACK",
        "TAP A LINE - CHOOSE",
    );
}

/// Подсказка внизу подэкрана.
fn back_hint(screen: &mut ChildSpawnerCommands, keys: &str, pad: &str, touch: &str) {
    spacer(screen, 4.0);
    screen.spawn(run::hint_line_touch(keys, pad, touch, 18.0, HINT_COLOR));
}

/// Выделенная строка белая со стрелками, остальные серые.
fn highlight_rows(
    menu: Res<Menu>,
    mut texts: Query<(&RowText, &mut TextColor)>,
    mut markers: Query<(&RowMarker, &mut Visibility)>,
) {
    for (row, mut color) in &mut texts {
        // Прозрачность не трогаем: ей управляет проявление экрана.
        let wanted = if row.0 == menu.selected { Color::WHITE } else { ROW_COLOR }.with_alpha(color.0.alpha());
        if color.0 != wanted {
            color.0 = wanted;
        }
    }
    for (marker, mut visibility) in &mut markers {
        let wanted = if marker.0 == menu.selected { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn трижды_вниз_и_enter_открывают_настройки() {
        let list = items(MenuPage::Main, false, false, &SignIn::SignedOut);
        let selected = moved(moved(moved(0, 1, list.len()), 1, list.len()), 1, list.len());
        assert_eq!(list[selected], Item::OpenSettings);
    }

    #[test]
    fn режимы_первыми_строками() {
        assert_eq!(main_items()[..2], [Item::Single, Item::Online]);
        assert_eq!(back_to_main(MenuPage::Online).selected, 1);
    }

    #[test]
    fn после_входа_выделено_назад() {
        let account = SignIn::SignedIn(crate::google_auth::Account {
            id: "1".into(),
            name: "ROVER".into(),
            email: String::new(),
        });
        assert_eq!(online_items(&account)[online_default(&account)], Item::Back);
        assert_eq!(online_items(&SignIn::NotConfigured), [Item::Back]);
        assert!(!online_items(&SignIn::Requesting).contains(&Item::Back));
    }

    #[test]
    fn список_пунктов_замкнут_в_кольцо() {
        let count = items(MenuPage::Main, false, false, &SignIn::SignedOut).len();
        assert_eq!(count, 7);
        assert_eq!(moved(0, -1, count), count - 1);
        assert_eq!(moved(count - 1, 1, count), 0);
    }

    #[test]
    fn громкость_шагами_по_10_и_по_кругу_с_enter() {
        assert_eq!(stepped_volume(100, -1, false), 90);
        assert_eq!(stepped_volume(100, 1, false), 100);
        assert_eq!(stepped_volume(0, -1, false), 0);
        assert_eq!(stepped_volume(100, 1, true), 0);
        let mut settings = Settings::default();
        for _ in 0..3 {
            change_setting(&mut settings, Setting::Music, -1, false);
        }
        assert_eq!(settings.music, 70);
        assert_eq!(setting_value(&settings, Setting::Music), "70%");
    }

    #[test]
    fn без_геймпада_строки_вибрации_нет() {
        let vibration = Item::Setting(Setting::Vibration);
        assert!(items(MenuPage::Settings, true, false, &SignIn::SignedOut).contains(&vibration));
        assert!(!items(MenuPage::Settings, false, false, &SignIn::SignedOut).contains(&vibration));
        assert_eq!(items(MenuPage::Settings, false, false, &SignIn::SignedOut).last(), Some(&Item::Back));
    }

    #[test]
    fn возврат_выделяет_пункт_подэкрана() {
        let menu = back_to_main(MenuPage::Authors);
        assert_eq!(menu.page, MenuPage::Main);
        assert_eq!(main_items()[menu.selected], Item::OpenAuthors);
    }

    #[test]
    fn авторы_по_алфавиту_фамилий() {
        assert_eq!(AUTHORS, ["Nidzhat Guseynov", "Ivan Tsarev", "Sergei Zotov"]);
        let surnames: Vec<_> = AUTHORS.iter().map(|name| name.rsplit(' ').next().unwrap()).collect();
        assert!(surnames.is_sorted());
    }
}
