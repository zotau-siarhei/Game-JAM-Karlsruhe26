//! След запуска: что игра делала последним. Пока идут тесты, игра пишет в файл рядом с настройками
//! этап загрузки, модель, собираемые конвейеры и реплику, а с ними свободную память системы. Если
//! процесс оборвался (падение драйвера, нехватка памяти, паника), при следующем запуске на экране
//! загрузки и в меню слева сверху видна строка «LAST LAUNCH STOPPED AT: …»: тестировщику без adb
//! достаточно прислать снимок экрана.
//!
//! Через [`MENU_SECONDS`] с в меню, в начале забега, при уходе в фон и при обычном выходе след
//! помечается чистым, и дальше отметки не пишутся. Причину выхода (паника, ошибка рендера) пишет
//! [`fail`] — и после чистой отметки. Включается флагом профиля `menu.version`.
//!
//! ```ignore
//! crate::trail::mark(format!("model {id}: simplify"));
//! ```

use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, OnceLock};

use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy::window::AppLifecycle;

use crate::frame::GameSet;
use crate::hud::{MARGIN, SafeAnchor};
use crate::run::{GameState, RunStarted};

/// Имя файла следа рядом с настройками.
const FILE_NAME: &str = "launch_trail.txt";
/// Начало строки чистого следа.
const CLEAN: &str = "ok";
/// Столько секунд в меню — и запуск считается удавшимся.
const MENU_SECONDS: f32 = 10.0;
/// Сдвиг строки вниз от верхнего поля: над ней в меню стоит подпись версии.
const BELOW_VERSION: f32 = 26.0;

/// Состояние следа.
struct Trail {
    /// Файл следа; `None` — писать некуда (браузер).
    path: Option<PathBuf>,
    /// Последняя отметка главного потока.
    stage: String,
    /// Конвейеры, которые сейчас собираются (рендер-поток): сборка идёт одновременно с загрузкой.
    pipelines: Option<String>,
    /// След уже помечен чистым или записана причина выхода: отметки больше не пишутся.
    finished: bool,
    /// Игра в фоне: след чистый, отметки копятся без записи.
    background: bool,
}

static TRAIL: Mutex<Option<Trail>> = Mutex::new(None);
/// Время запуска процесса: в строке следа — секунды от него.
static START: LazyLock<Instant> = LazyLock::new(Instant::now);
/// Прошлый след, если тот запуск оборвался.
static PREVIOUS: OnceLock<Option<String>> = OnceLock::new();

/// Начинает след: читает прошлый и пишет новый. Вызывается в `main` после выбора профиля.
pub fn begin() {
    LazyLock::force(&START);
    if !crate::profile::get().menu.version {
        let _ = PREVIOUS.set(None);
        return;
    }
    let path = crate::records::records_path().and_then(|records| Some(records.parent()?.join(FILE_NAME)));
    let previous = path
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty() && !text.starts_with(CLEAN));
    let _ = PREVIOUS.set(previous);
    *lock() = Some(Trail {
        path,
        stage: String::new(),
        pipelines: None,
        finished: false,
        background: false,
    });
    mark("start");
}

/// Прошлый запуск оборвался — его последняя запись.
pub fn previous() -> Option<&'static str> {
    PREVIOUS.get()?.as_deref()
}

/// Отмечает, что игра делает сейчас. Можно звать из любого потока; одинаковая отметка не пишется.
pub fn mark(stage: impl AsRef<str>) {
    let stage = stage.as_ref();
    let mut guard = lock();
    let Some(trail) = guard.as_mut() else {
        return;
    };
    if trail.finished || trail.stage == stage {
        return;
    }
    trail.stage = stage.to_owned();
    if !trail.background {
        write(trail.path.as_ref(), &trail.line());
    }
}

/// Отмечает конвейеры, которые собираются сейчас; `None` — сборка закончилась.
pub fn mark_pipelines(pipelines: Option<String>) {
    let mut guard = lock();
    let Some(trail) = guard.as_mut() else {
        return;
    };
    if trail.finished || trail.pipelines == pipelines {
        return;
    }
    trail.pipelines = pipelines;
    if !trail.background {
        write(trail.path.as_ref(), &trail.line());
    }
}

/// Записывает причину выхода: паника, ошибка рендера, молчаливый выход. Пишется и в чистый след.
/// Может звать обработчик паник, поэтому занятый замок не ждёт.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn fail(reason: &str) {
    let Ok(mut guard) = TRAIL.try_lock() else {
        return;
    };
    let Some(trail) = guard.as_mut() else {
        return;
    };
    let text = if trail.finished || trail.stage.is_empty() {
        line(reason)
    } else {
        format!("{reason} (after: {})", trail.line())
    };
    trail.finished = true;
    write(trail.path.as_ref(), &text);
}

/// Помечает след чистым: запуск удался или игра закрывается сама.
pub fn clean(reason: &str) {
    let mut guard = lock();
    let Some(trail) = guard.as_mut() else {
        return;
    };
    if trail.finished {
        return;
    }
    trail.finished = true;
    write(trail.path.as_ref(), &format!("{CLEAN}: {reason}"));
}

/// Игра ушла в фон или вернулась. В фоне система может закрыть процесс, и это не сбой.
fn set_background(background: bool) {
    let mut guard = lock();
    let Some(trail) = guard.as_mut() else {
        return;
    };
    if trail.finished || trail.background == background {
        return;
    }
    trail.background = background;
    let text = if background {
        format!("{CLEAN}: background (at: {})", trail.stage)
    } else {
        trail.line()
    };
    write(trail.path.as_ref(), &text);
}

impl Trail {
    /// Строка следа: отметка, конвейеры в сборке, свободная память, секунды от запуска.
    fn line(&self) -> String {
        match &self.pipelines {
            Some(pipelines) => line(&format!("{} | {pipelines}", self.stage)),
            None => line(&self.stage),
        }
    }
}

fn lock() -> std::sync::MutexGuard<'static, Option<Trail>> {
    TRAIL.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Строка следа: отметка, свободная память и секунды от запуска.
fn line(stage: &str) -> String {
    let seconds = START.elapsed().as_secs_f32();
    match free_memory_mb() {
        Some(free) => format!("{stage} · free {free} MB · {seconds:.1} s"),
        None => format!("{stage} · {seconds:.1} s"),
    }
}

fn write(path: Option<&PathBuf>, text: &str) {
    if let Some(path) = path {
        // Без fsync: запись уходит в кэш системы и переживает гибель процесса.
        let _ = std::fs::write(path, text);
    }
}

/// Свободная память системы, МБ (`MemAvailable` из `/proc/meminfo`).
#[cfg(any(target_os = "android", target_os = "linux"))]
fn free_memory_mb() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = text.lines().find(|line| line.starts_with("MemAvailable:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

#[cfg(not(any(target_os = "android", target_os = "linux")))]
fn free_memory_mb() -> Option<u64> {
    None
}

/// Показывает прошлый оборвавшийся запуск и помечает нынешний чистым.
pub struct TrailPlugin;

impl Plugin for TrailPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_note).add_systems(
            Update,
            (show_note, follow_run, follow_lifecycle).in_set(GameSet::Presentation),
        );
    }
}

/// Строка о прошлом запуске.
#[derive(Component)]
struct TrailNote;

fn spawn_note(mut commands: Commands) {
    let Some(previous) = previous() else {
        return;
    };
    warn!("Прошлый запуск оборвался: {previous}");
    commands.spawn((
        TrailNote,
        Text::new(format!("LAST LAUNCH STOPPED AT: {previous}")),
        TextFont {
            font_size: FontSize::Px(13.0),
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.75, 0.35)),
        Node {
            position_type: PositionType::Absolute,
            max_width: Val::Percent(70.0),
            padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
            ..default()
        },
        SafeAnchor {
            left: Some(MARGIN),
            top: Some(MARGIN + BELOW_VERSION),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
        // Экран загрузки стоит над занавесом, строка — с ним.
        GlobalZIndex(crate::fade::ABOVE_CURTAIN_Z + 1),
        Visibility::Hidden,
        Pickable::IGNORE,
    ));
}

/// Строка видна в загрузке и в меню.
fn show_note(state: Res<State<GameState>>, mut notes: Query<&mut Visibility, With<TrailNote>>) {
    let wanted = if matches!(state.get(), GameState::Loading | GameState::Menu) {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut visibility in &mut notes {
        visibility.set_if_neq(wanted);
    }
}

/// Этапы забега в следе: меню, а через [`MENU_SECONDS`] с в нём или в начале забега след чистый.
fn follow_run(
    state: Res<State<GameState>>,
    time: Res<Time<Real>>,
    mut in_menu: Local<f32>,
    mut started: MessageReader<RunStarted>,
) {
    if started.read().count() > 0 {
        clean("run started");
        return;
    }
    if *state.get() != GameState::Menu {
        return;
    }
    if *in_menu == 0.0 {
        mark("menu");
    }
    *in_menu += time.delta_secs();
    if *in_menu >= MENU_SECONDS {
        clean("menu");
    }
}

fn follow_lifecycle(mut lifecycle: MessageReader<AppLifecycle>) {
    for event in lifecycle.read() {
        match event {
            AppLifecycle::Suspended | AppLifecycle::WillSuspend => set_background(true),
            AppLifecycle::Running | AppLifecycle::WillResume => set_background(false),
            AppLifecycle::Idle => {}
        }
    }
}
